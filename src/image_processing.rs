use crate::constants::DEFAULT_FRAME_DELAY;
use crate::models::{ImageRotate::*, *};
use crate::progress::set_progress;
use crate::util::{
    JsCall, JsReflectGetSet, ResultJsExt, format_duration, now_ms, wasm_shared_memory,
};
use glam::{DVec2, UVec2, uvec2};
use image::{AnimationDecoder, imageops};
use std::{pin::Pin, sync::Arc, time::Duration};
use wasm_bindgen::prelude::*;

pub type FrameCallback = Box<
    dyn for<'a> Fn(
        &'a image::DynamicImage,
    ) -> Pin<Box<dyn Future<Output = Result<(), JsValue>> + 'a>>,
>;

pub struct FrameData<'a> {
    args: &'a ImageArgs,
    curr_n_us: u64,
    decoder: FrameDecoder<'a>,
    fps_timer: f64,
    in_dim: UVec2,
    in_frame_i: u32,
    is_first: bool,
    on_frame: Option<FrameCallback>,
    out_dim_raw: DVec2,
    out_frame_i: usize,
    out_n_frames: u32,
    prev_img: Option<(Arc<image::DynamicImage>, u64)>,
}

impl FrameData<'_> {
    /// Returns the dimensions of the output frames.
    ///
    /// Note: This is truncated from the raw dimensions instead of rounded.
    pub fn out_dim(&self) -> UVec2 {
        let dim = self.out_dim_raw();
        match self.args.rotate {
            Deg90 | Deg270 => uvec2(dim.y, dim.x),
            _ => dim,
        }
    }
    fn out_dim_raw(&self) -> UVec2 {
        let (w, h) = crate::image_util::resize_dimensions(
            self.in_dim.x,
            self.in_dim.y,
            self.out_dim_raw.x.round() as u32,
            self.out_dim_raw.y.round() as u32,
            false,
        );
        uvec2(w, h)
    }
    pub fn curr_duration_us(&self) -> u64 {
        self.decoder.curr_n_us
    }
    pub fn curr_total_frames(&self) -> u32 {
        self.in_frame_i
    }
    pub fn total_frames(&self) -> u32 {
        self.out_n_frames
    }
}
impl<'a> FrameData<'a> {
    pub fn new(
        reader: impl std::io::Read + 'a,
        image_args: &'a ImageArgs,
        on_frame: Option<FrameCallback>,
        js_next_frame: Option<JsVideoFrameCallback>,
    ) -> Result<Self, JsValue> {
        let frame_data = get_frames(reader, image_args, js_next_frame)?;
        let in_dim = frame_data.dimensions();
        let scale_factor = (image_args.max_size as f64 / in_dim.x as f64)
            .min(image_args.max_size as f64 / in_dim.y as f64)
            .min(1.0);

        fn expected_output_frames(n_ms: u32, fps: u32) -> u32 {
            ((n_ms as u64 * fps as u64) / 1000) as u32 + 1
        }

        Ok({
            let mut obj = Self {
                out_n_frames: match image_args.mode {
                    Mode::Static { .. } => 1,
                    _ => expected_output_frames(
                        (frame_data.info.duration.unwrap() * 1000.0) as u32,
                        image_args.target_fps.max(1),
                    ),
                },
                args: image_args,
                curr_n_us: 0,
                fps_timer: now_ms(),
                decoder: frame_data.decoder,
                in_dim,
                in_frame_i: 0,
                is_first: true,
                on_frame,
                out_dim_raw: scale_factor * DVec2::from(in_dim),
                out_frame_i: 0,
                prev_img: None,
            };
            obj.decoder.dim = obj.out_dim_raw();
            obj
        })
    }

    pub async fn next(&mut self) -> Result<Option<Arc<image::DynamicImage>>, JsValue> {
        let mut img: Option<Arc<image::DynamicImage>> = None;
        let use_js_decode = matches!(self.decoder.state, FrameDecoderState::JsFrames { .. });

        if matches!(self.args.mode, Mode::Static { .. }) {
            let Some(FrameWithTime(f, timestamp)) = self.decoder.next_frame().await? else {
                return Ok(None);
            };
            img = Some(Arc::new(self.convert_frame(f.into_buffer())?));
            self.curr_n_us = timestamp;
        } else if self.out_frame_i == 0 {
            self.fps_timer = now_ms();
        }

        loop {
            if self.out_t_us() <= self.curr_n_us
                && let Some(prev) = &self.prev_img
            {
                img = Some(Arc::clone(&prev.0)); // Emit duplicate frames if necessary
                self.out_frame_i += 1;
            }

            // Flush any remaining frames
            if let Some(frame) = img {
                if !use_js_decode && let Some(callback) = &self.on_frame {
                    callback(frame.as_ref()).await?;
                }
                return Ok(Some(frame));
            } else if self.out_t_us() >= self.curr_n_us {
                // If decoder is finished, we're done
                if self.decoder.is_finished() {
                    return Ok(None);
                }
                // On last input frame, continue in case there are remaining output frames
                let Some(FrameWithTime(frame, timestamp)) = self.decoder.next_frame().await? else {
                    continue;
                };

                self.curr_n_us = timestamp + Duration::from(frame.delay()).as_micros() as u64;

                let img = self.convert_frame(frame.into_buffer())?;
                self.prev_img = Some((Arc::new(img), self.curr_n_us));
                self.in_frame_i += 1;
            }

            if self.is_first {
                self.is_first = false;
            } else {
                let duration: f64;
                let progress: f64;
                let args = &self.args;

                if use_js_decode {
                    duration = args.duration.unwrap();
                    progress = (self.out_frame_i as f64 / args.target_fps as f64) / duration;
                } else {
                    duration = self.out_n_frames as f64 / args.target_fps as f64;
                    progress = self.out_frame_i as f64 / self.out_n_frames as f64;
                }

                set_progress(
                    0.10,
                    0.90,
                    progress,
                    &format!(
                        "Processing frame {}, {} / {}  ({:.1} FPS)",
                        self.out_frame_i,
                        format_duration((progress * duration * 1000.0) as u128),
                        format_duration((duration * 1000.0) as u128),
                        1000.0 * self.out_frame_i as f64 / (now_ms() - self.fps_timer)
                    ),
                );
            }
        }
    }

    //// Returns the current output time in microseconds.
    fn out_t_us(&self) -> u64 {
        (self.out_frame_i as u64 * 1000 * 1000) / self.args.target_fps as u64
    }

    /// Converts the input frame to the target output using the specified settings.
    pub fn convert_frame(&self, buf: image::RgbaImage) -> Result<image::DynamicImage, JsValue> {
        use crate::blueprint::models::ImageRotate::*;

        let mut img = image::DynamicImage::ImageRgba8(buf);
        let out_dim = self.out_dim_raw.round().as_uvec2();

        // Apply filters
        for f in self.args.filters.iter() {
            img = match *f {
                ImageFilter::Blur(sigma) => img.blur(sigma),
                ImageFilter::Brightness(value) => img.brighten(value),
                ImageFilter::Contrast(c) => img.adjust_contrast(c),
                ImageFilter::HueRotate(value) => img.huerotate(value),
                ImageFilter::Unsharpen(sigma, threshold) => img.unsharpen(sigma, threshold),
            };
        }

        if self.args.grayscale_bits > 0 {
            img = image::DynamicImage::ImageLuma8(img.to_luma8());
        }
        img = match self.args.resampling_filter {
            ResamplingFilter::Thumbnail => img.thumbnail(out_dim.x, out_dim.y),
            filter => img.resize(
                out_dim.x,
                out_dim.y,
                match filter {
                    ResamplingFilter::Catrom => imageops::FilterType::CatmullRom,
                    ResamplingFilter::Gaussian => imageops::FilterType::Gaussian,
                    ResamplingFilter::Lanczos3 => imageops::FilterType::Lanczos3,
                    ResamplingFilter::Nearest => imageops::FilterType::Nearest,
                    ResamplingFilter::Triangle => imageops::FilterType::Triangle,
                    _ => unreachable!(),
                },
            ),
        };
        if matches!(self.args.flip, ImageFlip::X | ImageFlip::Both) {
            imageops::flip_horizontal_in_place(&mut img);
        }
        if matches!(self.args.flip, ImageFlip::Y | ImageFlip::Both) {
            imageops::flip_vertical_in_place(&mut img);
        }
        img = match self.args.rotate {
            Deg90 => img.rotate90(),
            Deg180 => img.rotate180(),
            Deg270 => img.rotate270(),
            _ => img,
        };

        Ok(img)
    }
}

pub struct ImageFrameData<'a> {
    decoder: FrameDecoder<'a>,
    info: ImageArgs,
}
impl ImageFrameData<'_> {
    pub fn dimensions(&self) -> UVec2 {
        self.info.dimensions.unwrap()
    }
}

/// Decodes the provided image data into a vector of frames based on the image type.
///
/// Supported types: "gif" and "webp".
///
/// # Arguments
///
/// * `image_data` - A byte slice containing the image data.
/// * `image_type` - The type of the image ("gif" or "webp").
///
/// # Returns
///
/// An iterator of `Frame` objects or a JavaScript error.
pub fn get_frames<'a>(
    mut reader: impl std::io::Read + 'a,
    image_args: &'a ImageArgs,
    js_next_frame: Option<JsVideoFrameCallback>,
) -> Result<ImageFrameData<'a>, JsValue> {
    let mut info: ImageArgs = image_args.clone();
    let state = match image_args.mime_type.as_str() {
        "image/gif" => {
            let decoder = image::codecs::gif::GifDecoder::new(reader).map_js_err("decode err")?;
            FrameDecoderState::NativeFrames(decoder.into_frames())
        }
        "image/webp" => {
            let decoder = image::codecs::webp::WebPDecoder::new(reader).map_js_err("decode err")?;
            FrameDecoderState::NativeFrames(decoder.into_frames())
        }
        t if t.starts_with("image/") => {
            let use_combs = matches!(image_args.mode, Mode::Static { combs: true, .. });
            info.n_frames = Some(1);
            info.duration = Some(if use_combs { 1000.0 } else { 0.0 });

            let dim = image_args.dimensions.expect("No image size");
            let mut data = Vec::new();
            reader.read_to_end(&mut data).map_js_err("Read error")?;

            let frames = image::Frames::new(Box::new(std::iter::once(Ok(image::Frame::new(
                image::RgbaImage::from_raw(dim.x, dim.y, data)
                    .expect("buffer length must be width * height * 4"),
            ))))); // bruh
            FrameDecoderState::NativeFrames(frames)
        }
        t if t.starts_with("video/") => {
            info.n_frames = None;
            let get_frame = js_next_frame.expect("JS next frame must be provided for videos");
            FrameDecoderState::JsFrames(get_frame)
        }
        t => return Err(JsValue::from_str(&format!("Unsupported image type: {t}"))),
    };

    let decoder = FrameDecoder::new(state, info.dimensions.unwrap());
    Ok(ImageFrameData { decoder, info })
}

enum FrameDecoderState<'a> {
    NativeFrames(image::Frames<'a>),
    JsFrames(JsVideoFrameCallback),
}

#[derive(Clone)]
struct FrameWithTime(image::Frame, u64);
struct FrameDecoder<'a> {
    curr_n_us: u64,
    dim: UVec2,
    finished: bool,
    state: FrameDecoderState<'a>,
}
impl<'a> FrameDecoder<'a> {
    fn new(state: FrameDecoderState<'a>, dim: UVec2) -> FrameDecoder<'a> {
        FrameDecoder {
            curr_n_us: 0,
            dim,
            finished: false,
            state,
        }
    }

    fn is_finished(&self) -> bool {
        self.finished
    }

    async fn next_frame(&mut self) -> Result<Option<FrameWithTime>, JsValue> {
        Ok(match &mut self.state {
            FrameDecoderState::NativeFrames(frames) => {
                let frame = frames.next().transpose().map_js_err("image")?.map(|f| {
                    let delay = f.delay();
                    let output = FrameWithTime(f, self.curr_n_us);
                    let delay: std::time::Duration = if delay.numer_denom_ms().0 == 0 {
                        DEFAULT_FRAME_DELAY
                    } else {
                        delay.into()
                    };
                    self.curr_n_us += delay.as_micros() as u64;
                    output
                });
                self.finished = frame.is_none();
                frame
            }
            FrameDecoderState::JsFrames(get_next_frame) => {
                let args_obj = js_sys::Object::new();
                let in_buf = vec![0u8; self.dim.element_product() as usize * 4];
                args_obj.reflect_set("inBuffer", wasm_shared_memory(&in_buf[..])?)?;
                args_obj.reflect_set("dimensions", serde_wasm_bindgen::to_value(&self.dim)?)?;
                let result = get_next_frame.call_n((&args_obj.into(),))?.await?;
                let info: JsVideoFrameData = serde_wasm_bindgen::from_value(result)?;

                if info.is_finished {
                    self.finished = true;
                    return Ok(None);
                }

                let duration = Duration::from_secs_f64(info.duration.unwrap());
                let duration = image::Delay::from_saturating_duration(duration);
                let img = image::RgbaImage::from_raw(self.dim.x, self.dim.y, in_buf);
                let img = img.expect("Invalid RGBA frame dimensions");

                Some(FrameWithTime(
                    image::Frame::from_parts(img, 0, 0, duration),
                    (info.timestamp.unwrap() * 1000.0 * 1000.0) as u64,
                ))
            }
        })
    }
}

pub type JsVideoFrameCallback = js_sys::Function<fn(JsValue) -> js_sys::Promise>;

pub async fn encode_gif(
    get_next_frame: JsVideoFrameCallback,
    on_chunk: js_sys::Function<fn(js_sys::Uint8Array) -> js_sys::Promise>,
    args: &EncodeGifArgs,
) -> Result<(), JsValue> {
    use crate::ioutil::StreamingWriter;
    use image::codecs::gif::{GifEncoder, Repeat};
    use std::{cell::RefCell, rc::Rc};

    let promises = Rc::new(const { RefCell::new(Vec::<js_sys::Promise>::new()) });
    let promise_q = Rc::clone(&promises);

    // Custom writer that invokes a callback whenever chunk size is reached.
    let mut writer = StreamingWriter::new(1024 * 1024, |chunk| {
        let data = js_sys::Uint8Array::from(&chunk[..]);
        promise_q.borrow_mut().push(on_chunk.call_n((&data,))?);
        Ok(())
    });
    let mut encoder = GifEncoder::new_with_speed(&mut writer, args.speed as i32);
    (encoder.set_repeat(Repeat::Infinite)).map_js_err("gif encoding error")?;

    let in_buf = vec![0u8; args.dim.element_product() as usize * 4];
    let flush = async || -> Result<(), JsValue> {
        let pending = std::mem::take(&mut *promises.borrow_mut());
        for promise in pending {
            promise.await?;
        }
        Ok(())
    };

    loop {
        let info: JsVideoFrameData = serde_wasm_bindgen::from_value({
            let js_args = js_sys::Object::new();
            js_args.reflect_set("inBuffer", wasm_shared_memory(&in_buf[..])?)?;
            get_next_frame.call_n((&js_args.into(),))?.await?
        })?;
        if info.is_finished {
            break;
        }

        let duration = Duration::from_secs_f64(info.duration.unwrap());
        let duration = image::Delay::from_saturating_duration(duration);
        let img = image::RgbaImage::from_raw(args.dim.x, args.dim.y, in_buf.clone());
        let img = img.expect("Invalid RGBA frame dimensions");
        let f = image::Frame::from_parts(img, 0, 0, duration);
        encoder.encode_frame(f).map_js_err("Encode error")?;

        flush().await?;
    }

    Ok(())
}

/// Converts an RGB pixel to a single 24 bit integer (inside a u32, I know...).
///
/// Note: This is a fallback implementation for when the SIMD-accelerated version is not available.
///
/// # Arguments
///
/// * `r` - Red channel (0–255).
/// * `g` - Green channel (0–255).
/// * `b` - Blue channel (0–255).
///
/// # Returns
///
/// An integer representing the RGB value.
#[inline(always)]
#[allow(dead_code)]
pub fn rgb_to_int(r: u8, g: u8, b: u8) -> u32 {
    ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
}
