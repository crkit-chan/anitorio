use crate::constants;
use crate::util::wasm_shared_memory;
use crate::{constants::DEFAULT_FRAME_DELAY, util::format_duration};
use crate::{prelude::*, ts};
use glam::{DVec2, UVec2, uvec2};
use image::{AnimationDecoder, GenericImageView, imageops};
use std::{pin::Pin, rc::Rc, sync::Arc, time::Duration};
use wasm_bindgen::JsValue;

const MAX_CALLBACKS: usize = 4;

pub type FrameCallback = dyn for<'a> Fn(
    &'a image::DynamicImage,
) -> Pin<Box<dyn Future<Output = Result<(), JsError>> + 'a>>;

pub struct FrameData<'a> {
    args:         &'a ImageArgs,
    curr_n_us:    u64,
    decoder:      FrameDecoder<'a>,
    fps_timer:    f64,
    in_dim:       UVec2,
    in_frame_i:   usize,
    is_first:     bool,
    on_frame:     Option<Rc<FrameCallback>>,
    out_dim_raw:  DVec2,
    out_frame_i:  usize,
    out_n_frames: usize,
    palette:      Option<ColorPalette>,
    prev_img:     Option<(Arc<image::DynamicImage>, u64)>,
}
impl FrameData<'_> {
    /// Returns the dimensions of the output frames.
    ///
    /// Note: This is truncated from the raw dimensions instead of rounded.
    #[must_use]
    #[inline]
    pub const fn out_dim(&self) -> UVec2 {
        let dim = self.out_dim_raw();
        if self.args.rotate.swaps_dimensions() {
            uvec2(dim.y, dim.x)
        } else {
            dim
        }
    }

    #[must_use]
    #[inline]
    const fn out_dim_raw(&self) -> UVec2 {
        let (w, h) = crate::image_util::resize_dimensions(
            self.in_dim.x,
            self.in_dim.y,
            self.out_dim_raw.x.round() as u32,
            self.out_dim_raw.y.round() as u32,
            false,
        );
        uvec2(w, h)
    }

    #[must_use]
    #[inline]
    pub const fn curr_total_frames(&self) -> usize {
        self.in_frame_i
    }

    #[must_use]
    #[inline]
    pub const fn total_frames(&self) -> usize {
        self.out_n_frames
    }
}
impl<'a> FrameData<'a> {
    pub fn new(
        frame_data: ImageFrameData<'a>,
        image_args: &'a ImageArgs,
        on_frame: Option<Rc<FrameCallback>>,
    ) -> Self {
        #[must_use]
        #[inline]
        const fn expected_output_frames(n_ms: u64, fps: u32) -> usize {
            1 + ((n_ms * fps as u64) / 1000) as usize
        }

        let in_dim = frame_data.dimensions();
        let scale_factor = (f64::from(image_args.max_size) / f64::from(in_dim.x))
            .min(f64::from(image_args.max_size) / f64::from(in_dim.y))
            .min(1.0);

        let mut obj = Self {
            out_n_frames: match image_args.mode {
                Mode::Static { .. } => 1,
                _ => expected_output_frames(
                    (frame_data.info.duration.unwrap() * 1000.0) as u64,
                    u32::from(image_args.target_fps.max(1)),
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
            palette: None,
            prev_img: None,
        };
        obj.decoder.dim = obj.out_dim_raw();
        obj
    }

    pub async fn next(&mut self) -> Result<Option<Arc<image::DynamicImage>>, JsError> {
        let mut img: Option<Arc<image::DynamicImage>> = None;
        let use_js_decode = matches!(self.decoder.state, FrameDecoderState::JsFrames { .. });

        if unlikely(matches!(self.args.mode, Mode::Static { .. })) {
            let Some(FrameWithTime(f, timestamp)) = self.decoder.next_frame().await? else {
                return Ok(None);
            };
            img = Some(Arc::new(self.convert_frame(f.into_buffer())));
            self.curr_n_us = timestamp;
        } else if unlikely(self.out_frame_i == 0) {
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
                if let Some(callback) = self.on_frame.clone() {
                    callback(frame.as_ref()).await?;
                }
                return Ok(Some(frame));
            } else if self.out_t_us() >= self.curr_n_us {
                // If decoder is finished, we're done
                if unlikely(self.decoder.is_finished()) {
                    return Ok(None);
                }

                let Some(FrameWithTime(frame, timestamp)) = self.decoder.next_frame().await? else {
                    continue; // On last input frame, continue in case there are remaining output frames
                };
                self.curr_n_us = timestamp + Duration::from(frame.delay()).as_micros() as u64;

                let img = self.convert_frame(frame.into_buffer());
                self.prev_img = Some((Arc::new(img), self.curr_n_us));
                self.in_frame_i += 1;
            }

            if unlikely(self.is_first) {
                self.is_first = false;
            } else {
                let (duration, progress): (f64, f64);
                let args = &self.args;

                if use_js_decode {
                    duration = args.duration.unwrap();
                    progress = (self.out_frame_i as f64 / f64::from(args.target_fps)) / duration;
                } else {
                    duration = self.out_n_frames as f64 / f64::from(args.target_fps);
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

    #[must_use]
    pub fn with_palette(mut self, palette: Option<ColorPalette>) -> Self {
        self.palette = palette;
        self
    }

    //// Returns the current output time in microseconds.
    #[must_use]
    #[inline]
    fn out_t_us(&self) -> u64 {
        (self.out_frame_i as u64 * 1000 * 1000) / u64::from(self.args.target_fps)
    }

    /// Converts the input frame to the target output using the specified settings.
    #[must_use]
    pub fn convert_frame(&self, buf: image::RgbaImage) -> image::DynamicImage {
        let mut img = image::DynamicImage::ImageRgba8(buf);
        let out_dim = self.out_dim_raw.round().as_uvec2();

        // 1. Apply custom filters.
        for f in &self.args.filters {
            img = match *f {
                ImageFilter::Blur(sigma) => img.blur(sigma),
                ImageFilter::Brightness(value) => img.brighten(value),
                ImageFilter::Contrast(c) => img.adjust_contrast(c),
                ImageFilter::HueRotate(value) => img.huerotate(value),
                ImageFilter::Unsharpen(sigma, threshold) => img.unsharpen(sigma, threshold),
            };
        }

        // 2. Resample after filters, but before grayscale to preserve detail.
        img = match self.args.resampling_filter {
            ResamplingFilter::Thumbnail => img.thumbnail(out_dim.x, out_dim.y),
            filter => img.resize(out_dim.x, out_dim.y, filter.to_image_filter().unwrap()),
        };

        // 3. Grayscale after resampling
        if self.args.grayscale_bits > 0 {
            let mut luma = img.into_luma8(); // Luma after resize
            match self.args.grayscale_bits {
                1 => {
                    for p in luma.as_mut() {
                        *p = 255 * u8::from(*p >= constants::GRAYSCALE_THRESH);
                    }
                }
                4 => {
                    for p in luma.as_mut() {
                        *p &= 0xF0;
                    }
                }
                8 => {}
                _ => unreachable!(),
            }
            img = image::DynamicImage::ImageLuma8(luma);
        }

        // 4. Lossless operations
        if self.args.flip.x() {
            imageops::flip_horizontal_in_place(&mut img);
        }
        if self.args.flip.y() {
            imageops::flip_vertical_in_place(&mut img);
        }
        match self.args.rotate as u16 {
            90 => img = img.rotate90(),
            180 => imageops::rotate180_in_place(&mut img),
            270 => img = img.rotate270(),
            _ => (),
        }

        // 5. Palette at the very end
        if let Some(palette) = self.palette.as_ref() {
            let indexes =
                quantized_frame_to_outputs(&img.view(0, 0, img.width(), img.height()), palette);

            match &mut img {
                image::DynamicImage::ImageRgba8(inner) => {
                    for (pixel, v) in inner.pixels_mut().zip(indexes) {
                        let [r, g, b] = palette.colors[v as usize];
                        *pixel = image::Rgba([r, g, b, 255]);
                    }
                }
                image::DynamicImage::ImageLuma8(inner) => {
                    for (pixel, v) in inner.iter_mut().zip(indexes) {
                        *pixel = palette.colors[v as usize][0];
                    }
                }
                _ => unimplemented!(),
            }
        }
        img
    }
}

pub struct ImageFrameData<'a> {
    decoder: FrameDecoder<'a>,
    info:    ImageArgs,
}
impl<'a> ImageFrameData<'a> {
    #[must_use]
    #[inline]
    pub const fn dimensions(&self) -> UVec2 {
        self.info.dimensions.unwrap()
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
    pub fn new(
        mut reader: impl std::io::Read + 'a,
        image_args: &'a ImageArgs,
        js_next_frame: Option<JsVideoFrameCallback>,
    ) -> Result<Self, JsError> {
        let mut info: ImageArgs = image_args.clone();
        let state = match image_args.mime_type.as_str() {
            "image/gif" => {
                let decoder = image::codecs::gif::GifDecoder::new(reader)?;
                FrameDecoderState::NativeFrames(decoder.into_frames())
            }
            "image/webp" => {
                let decoder = image::codecs::webp::WebPDecoder::new(reader)?;
                FrameDecoderState::NativeFrames(decoder.into_frames())
            }
            t if t.starts_with("image/") || matches!(image_args.mode, Mode::Static { .. }) => {
                let use_combs = matches!(image_args.mode, Mode::Static { combs: true, .. });
                info.n_frames = Some(1);
                info.duration = Some(if use_combs { 1000.0 } else { 0.0 });

                let dim = image_args.dimensions.expect("No image size");
                let mut data = Vec::new();
                reader.read_to_end(&mut data)?;

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
            t => panic!("Unsupported image type: '{t}'"),
        };

        let decoder = FrameDecoder::new(state, info.dimensions.unwrap());
        Ok(ImageFrameData { decoder, info })
    }
}

/// Picks one palette of `size` colors for all frames. Index 0 is always black, since black
/// pixels are left out of the blueprint entirely.
///
/// # Arguments
///
/// * `frames` - The processed frames.
/// * `size` - Number of palette entries, including black.
///
/// # Returns
///
/// The palette colors as RGB triples.
pub async fn build_palette(
    mut generator: FrameData<'_>,
    size: u32,
    speed: i32,
) -> Result<ColorPalette, JsError> {
    let mut attr = imagequant::Attributes::new();
    attr.set_max_colors(size)?;
    attr.set_speed(speed)?;

    let mut result = {
        let mut hist = imagequant::Histogram::new(&attr);

        while let Some(frame) = generator.next().await? {
            let (w, h) = (frame.width() as usize, frame.height() as usize);
            let mut qimg = match Arc::unwrap_or_clone(frame) {
                image::DynamicImage::ImageRgba8(rgba) => {
                    attr.new_image(bytemuck::cast_slice(&rgba.into_raw()), w, h, 0.0)?
                }
                image::DynamicImage::ImageLuma8(luma) => unsafe {
                    let pixels = luma.into_raw();
                    let f = move |row: &mut [std::mem::MaybeUninit<imagequant::RGBA>], y: usize| {
                        let start = y * w;
                        for (x, pixel) in row.iter_mut().enumerate() {
                            let l = pixels[start + x];
                            pixel.write(imagequant::RGBA::new(l, l, l, 255));
                        }
                    };
                    imagequant::Image::new_fn(&attr, f, w, h, 0.0)?
                },
                _ => unreachable!("Expected RGBA8 or LumaA8"),
            };
            hist.add_image(&attr, &mut qimg)?;
        }
        hist.quantize(&attr)?
    };
    let mut colors = result.palette_vec().into_iter().map(|c| [c.r, c.g, c.b]).collect::<Box<_>>();

    if let Some(darkest) = (0..colors.len()).min_by_key(|&i| {
        let c = colors[i];
        c[0].saturating_add(c[1]).saturating_add(c[2])
    }) {
        colors.swap(0, darkest);
    }

    ColorPalette::new(&colors)
}

/// Finds the palette entry closest to a color.9
///
/// # Arguments
///
/// * `palette` - The palette colors.
/// * `color` - The RGB color to match.
///
/// # Returns
///
/// The index of the closest palette entry.
pub fn quantized_frame_to_outputs(
    frame: &image::SubImage<&image::DynamicImage>,
    palette: &ColorPalette,
) -> Vec<u32> {
    let (x, y) = frame.offsets();
    let (width, height) = (frame.width() as usize, frame.height() as usize);
    let inner = frame.inner();
    let mut outputs = Vec::with_capacity(width * height);
    let stride = frame.width() as usize;

    match inner {
        image::DynamicImage::ImageRgba8(img) => {
            let rgba = img.as_raw();
            for row in 0..height {
                let start = ((y as usize + row) * stride + x as usize) * 4;
                let end = start + width * 4;
                outputs.extend(
                    (rgba[start..end].as_chunks::<4>().0.iter())
                        .map(|&[r, g, b, ..]| palette.nearest_index([r, g, b])),
                );
            }
        }
        image::DynamicImage::ImageLuma8(img) => {
            let luma = img.as_raw();
            for row in 0..height {
                let start = (y as usize + row) * stride + x as usize;
                let end = start + width;
                outputs.extend(luma[start..end].iter().map(|&l| palette.nearest_index([l, l, l])));
            }
        }
        _ => unreachable!("Expected RGBA8 or Luma8"),
    }

    outputs
}
enum FrameDecoderState<'a> {
    NativeFrames(image::Frames<'a>),
    JsFrames(JsVideoFrameCallback),
}

#[derive(Clone)]
struct FrameWithTime(image::Frame, u64);
struct FrameDecoder<'a> {
    curr_n_us: u64,
    dim:       UVec2,
    finished:  bool,
    state:     FrameDecoderState<'a>,
}
impl<'a> FrameDecoder<'a> {
    const fn new(state: FrameDecoderState<'a>, dim: UVec2) -> Self {
        FrameDecoder { curr_n_us: 0, dim, finished: false, state }
    }

    #[must_use]
    #[inline]
    const fn is_finished(&self) -> bool {
        self.finished
    }

    async fn next_frame(&mut self) -> Result<Option<FrameWithTime>, JsError> {
        Ok(match &mut self.state {
            FrameDecoderState::NativeFrames(frames) => {
                let frame = frames.next().transpose()?.map(|f| {
                    let delay = f.delay();
                    let output = FrameWithTime(f, self.curr_n_us);
                    let delay: core::time::Duration = if unlikely(delay.numer_denom_ms().0 == 0) {
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
                args_obj.reflect_set("inBuffer", wasm_shared_memory(&in_buf)?)?;
                args_obj.reflect_set("dimensions", serde_wasm_bindgen::to_value(&self.dim)?)?;

                let result = get_next_frame.call_n((&args_obj.into(),))?.await?;
                let info: JsVideoFrameData = serde_wasm_bindgen::from_value(result)?;

                if unlikely(info.is_finished) {
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

pub type JsVideoFrameCallback = js_sys::Function<fn(JsValue) -> ts::TsFut<JsValue>>;

pub async fn encode_gif(
    get_next_frame: JsVideoFrameCallback,
    on_chunk: js_sys::Function<fn(js_sys::Uint8Array) -> ts::TsFut>,
    args: &EncodeGifArgs,
) -> Result<(), JsError> {
    use crate::ioutil::CallbackWriter;
    use core::cell::RefCell;
    use image::codecs::gif::{GifEncoder, Repeat};
    use std::rc::Rc;

    let promises = Rc::new(const { RefCell::new(Vec::<ts::TsFut>::new()) });
    let promise_q = Rc::clone(&promises);

    // Custom writer that invokes a callback whenever chunk size is reached.
    let writer = CallbackWriter::new(|chunk| {
        let js_promise = on_chunk.call_n((&(&*chunk).into(),));
        promise_q.borrow_mut().push(js_promise?);
        Ok(())
    });
    let mut writer = std::io::BufWriter::with_capacity(1024 * 1024, writer);
    let mut encoder = GifEncoder::new_with_speed(&mut writer, i32::from(args.speed));
    (encoder.set_repeat(Repeat::Infinite))?;

    let in_buf = vec![0u8; args.dim.element_product() as usize * 4];
    let flush = async || -> Result<(), JsError> {
        let pending = core::mem::take(&mut *promises.borrow_mut());
        for promise in pending {
            promise.await?;
        }
        Ok(())
    };

    loop {
        let info: JsVideoFrameData = serde_wasm_bindgen::from_value({
            let js_args = js_sys::Object::new();
            js_args.reflect_set("inBuffer", wasm_shared_memory(&in_buf)?)?;
            get_next_frame.call_n((&js_args.into(),))?.await?
        })?;
        if unlikely(info.is_finished) {
            break;
        }

        let duration = Duration::from_secs_f64(info.duration.unwrap());
        let duration = image::Delay::from_saturating_duration(duration);
        let img = image::RgbaImage::from_raw(args.dim.x, args.dim.y, in_buf.clone());
        let img = img.expect("Invalid RGBA frame dimensions");
        let f = image::Frame::from_parts(img, 0, 0, duration);
        encoder.encode_frame(f)?;

        flush().await?;
    }

    Ok(())
}

#[must_use]
#[inline]
pub const fn rgb_to_int(r: u8, g: u8, b: u8) -> u32 {
    ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
}
