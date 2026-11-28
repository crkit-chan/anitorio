use std::collections::HashMap;
use std::io;
use std::pin::Pin;
use std::rc::Rc;
use std::time::Duration;

use glam::{DVec2, UVec2, uvec2};
use image::{GenericImageView as _, imageops};
use wasm_bindgen::JsValue;

use super::image_util;
use crate::constants::DEFAULT_FRAME_DELAY;
use crate::hash::IdentityHasherBuilder;
use crate::image_processing::image_util::LzwHistogramGenerator;
use crate::palette::ColorPalettePixelSlice;
use crate::prelude::*;
use crate::util::{format_duration, wasm_shared_memory};
use crate::{GetNextFrameArgs, constants, ts};

const MAX_PREVIEW_FPS: f64 = 60.0;

pub type FrameCallback = dyn for<'a> Fn(
    &'a image::DynamicImage,
) -> Pin<Box<dyn Future<Output = Result<(), JsError>> + 'a>>;

pub type GetJsFrameCallback =
    js_sys::Function<fn(ts::TsIntoRust<GetNextFrameArgs>) -> ts::TsFut<JsValue>>;

pub struct FrameData<'a> {
    args:                ImageArgs,
    curr_n_us:           u64,
    decoder:             FrameDecoder<'a>,
    for_palette:         bool,
    fps_timer:           f64,
    histogram_generator: Option<Box<LzwHistogramGenerator>>,
    in_dim:              UVec2,
    in_frame_i:          usize,
    is_first:            bool,
    on_frame:            Option<Rc<FrameCallback>>,
    out_dim_raw:         DVec2,
    out_frame_i:         usize,
    out_n_frames:        usize,
    palette:             Option<ColorPalette>,
    progress_msg:        String,
    prev_img:            Option<(RC<image::DynamicImage>, u64)>,
}
impl<'a> FrameData<'a> {
    /// Decodes the provided image data into a vector of frames based on the
    /// image type.
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
    pub fn new<R: BufReadSeek + 'a>(
        mut reader: R, args: &ImageArgs, js_next_frame: Option<GetJsFrameCallback>,
        on_frame: Option<Rc<FrameCallback>>, for_palette: bool,
    ) -> Result<Self, JsError> {
        #[inline]
        #[must_use]
        const fn expected_output_frames(n_ms: u64, fps: u32) -> usize {
            1 + ((n_ms * fps as u64).div_euclid(1000)) as usize
        }

        let mut args = args.clone();
        let state = match args.mime_type.as_str() {
            "image/gif" => {
                let mut options = gif::DecodeOptions::new();
                options.set_color_output(gif::ColorOutput::Indexed);
                options.skip_frame_decoding(for_palette);

                let decoder = options.read_info(Box::new(reader) as Box<dyn BufReadSeek>)?;
                let screen = gif_dispose::Screen::new_decoder(&decoder);

                FrameDecoderState::Gif(Box::new(GifData { decoder, screen }))
            },
            "image/webp" => {
                struct FramesInner<R: BufReadSeek> {
                    decoder: image_webp::WebPDecoder<R>,
                    current: u32,
                }
                impl<R: BufReadSeek> Iterator for FramesInner<R> {
                    type Item = Result<DynamicFrame, JsError>;

                    fn next(&mut self) -> Option<Self::Item> {
                        if self.current == self.decoder.num_frames() {
                            return None;
                        }

                        self.current += 1;
                        let (width, height) = self.decoder.dimensions();
                        let (img, delay) = if self.decoder.has_alpha() {
                            let mut img = image::RgbaImage::new(width, height);
                            match self.decoder.read_frame(&mut img) {
                                Ok(delay) => (image::DynamicImage::ImageRgba8(img), delay),
                                Err(image_webp::DecodingError::NoMoreFrames) => return None,
                                Err(e) => return Some(Err(js_err!("{e}"))),
                            }
                        } else {
                            let mut img = image::RgbImage::new(width, height);
                            match self.decoder.read_frame(&mut img) {
                                Ok(delay) => (image::DynamicImage::ImageRgb8(img), delay),
                                Err(image_webp::DecodingError::NoMoreFrames) => return None,
                                Err(e) => return Some(Err(js_err!("{e}"))),
                            }
                        };
                        let delay = image::Delay::from_numer_denom_ms(delay, 1);
                        Some(Ok(DynamicFrame::new(img, delay)))
                    }
                }

                let decoder = image_webp::WebPDecoder::new(reader)?;
                let frames = DynamicFrames::new(Box::new(FramesInner { decoder, current: 0 }));
                FrameDecoderState::Frames(frames)
            },
            t if t.starts_with("image/") || matches!(args.mode, Mode::Static { .. }) => {
                let use_combs = matches!(args.mode, Mode::Static { combs: true, .. });
                args.duration = Some(1000.0 * f64::from(use_combs));
                args.n_frames = Some(1);

                let dim = args.dimensions.ok_or_js_err("No image size")?;
                let mut data = Vec::new();
                reader.read_to_end(&mut data)?;

                let img = image::RgbaImage::from_raw(dim.x, dim.y, data)
                    .ok_or_js_err("buffer length must be width * height * 4")?;
                let frames = DynamicFrames::new(Box::new(std::iter::once(Ok(DynamicFrame::new(
                    image::DynamicImage::ImageRgba8(img),
                    image::Delay::from_numer_denom_ms(0, 1),
                ))))); // bruh
                FrameDecoderState::Frames(frames)
            },
            t if t.starts_with("video/") => {
                args.n_frames = None;
                FrameDecoderState::JsFrames(
                    js_next_frame.ok_or_js_err("JS next frame must be provided for videos")?,
                )
            },
            t => err_js!("Unsupported image type: '{t}'"),
        };

        let mut decoder =
            FrameDecoder::new(state, args.dimensions.ok_or_js_err("Missing dimensions!")?);
        decoder = decoder.with_use_sparse_decoding((args.target_fps <= 10).then_some(true));

        let in_dim = args.dimensions.ok_or_js_err("Missing dimensions!")?;
        let scale_factor = (f64::from(args.max_size) / f64::from(in_dim.x))
            .min(f64::from(args.max_size) / f64::from(in_dim.y))
            .min(1.0);

        let mut obj = Self {
            out_n_frames: match args.mode {
                Mode::Static { .. } => 1,
                _ => expected_output_frames(
                    (args.duration.ok_or_js_err("FrameData missing 'duration' field")? * 1000.0)
                        as u64,
                    u32::from(args.target_fps.max(1)),
                ),
            },
            args,
            curr_n_us: 0,
            for_palette,
            fps_timer: now_ms(),
            decoder,
            histogram_generator: for_palette.then(|| Box::new(LzwHistogramGenerator::new())),
            in_dim,
            in_frame_i: 0,
            is_first: true,
            on_frame,
            out_dim_raw: scale_factor * DVec2::from(in_dim),
            out_frame_i: 0,
            palette: None,
            prev_img: None,
            progress_msg: String::new(),
        };
        obj.decoder.dim = obj.out_dim_raw();

        Ok(obj)
    }

    /// Returns the dimensions of the output frames.
    ///
    /// Note: This is truncated from the raw dimensions instead of rounded.
    #[inline]
    #[must_use]
    pub const fn out_dim(&self) -> UVec2 {
        let dim = self.out_dim_raw();
        if self.args.rotate.swaps_dimensions() {
            uvec2(dim.y, dim.x)
        } else {
            dim
        }
    }

    #[inline]
    #[must_use]
    const fn out_dim_raw(&self) -> UVec2 {
        let (w, h) = image_util::resize_dimensions(
            self.in_dim.x,
            self.in_dim.y,
            self.out_dim_raw.x.round() as u32,
            self.out_dim_raw.y.round() as u32,
            false,
        );
        uvec2(w, h)
    }

    #[inline]
    #[must_use]
    pub const fn curr_total_frames(&self) -> usize {
        self.in_frame_i
    }

    #[inline]
    #[must_use]
    pub const fn total_frames(&self) -> usize {
        self.out_n_frames
    }

    #[must_use]
    pub fn with_progress_message(mut self, msg: impl Into<String>) -> Self {
        self.progress_msg = msg.into();
        self
    }

    // #[crate::perf("FrameData::next()")]
    pub async fn next(&mut self) -> Result<Option<RC<image::DynamicImage>>, JsError> {
        let mut img: Option<RC<image::DynamicImage>> = None;
        let use_js_decode = matches!(self.decoder.state, FrameDecoderState::JsFrames { .. });

        if unlikely(matches!(self.args.mode, Mode::Static { .. })) {
            let Some(FrameWithTime(f, timestamp)) = self.decoder.next_frame().await? else {
                return Ok(None);
            };
            img = Some(RC::new(self.convert_frame(f.into_buffer())?));
            self.curr_n_us = timestamp;
        } else if unlikely(self.out_frame_i == 0) {
            self.fps_timer = now_ms();
        }

        loop {
            if self.out_t_us() <= self.curr_n_us
                && let Some(ref prev) = self.prev_img
            {
                img = Some(RC::clone(&prev.0)); // Emit duplicate frames if necessary
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

                let img = self.convert_frame(frame.into_buffer())?;
                self.prev_img = Some((RC::new(img), self.curr_n_us));
                self.in_frame_i += 1;
            }

            if unlikely(self.is_first) {
                self.is_first = false;
            } else {
                let (duration, progress): (f64, f64);
                let args = &self.args;

                if use_js_decode {
                    duration = args.duration.ok_or_js_err("Duration must be provided")?;
                    progress = (self.out_frame_i as f64 / f64::from(args.target_fps)) / duration;
                } else {
                    duration = self.out_n_frames as f64 / f64::from(args.target_fps);
                    progress = self.out_frame_i as f64 / self.out_n_frames as f64;
                }

                if likely(!self.progress_msg.is_empty()) {
                    set_progress(
                        0.10,
                        0.90,
                        progress,
                        &format!(
                            "{} {}, {} / {}  ({:.1} FPS)",
                            self.progress_msg,
                            self.out_frame_i,
                            format_duration((progress * duration * 1000.0) as u128),
                            format_duration((duration * 1000.0) as u128),
                            1000.0 * self.out_frame_i as f64 / (now_ms() - self.fps_timer)
                        ),
                    );
                }
            }
        }
    }

    #[inline]
    #[must_use]
    pub fn with_palette(mut self, palette: Option<ColorPalette>) -> Self {
        self.palette = palette;
        self
    }

    #[inline]
    #[must_use]
    pub fn get_palette_colors(&self) -> Option<&ColorPalettePixelSlice> {
        self.palette.as_ref().map(ColorPalette::get_colors)
    }

    //// Returns the current output time in microseconds.
    #[inline]
    #[must_use]
    const fn out_t_us(&self) -> u64 {
        (self.out_frame_i as u64 * 1000 * 1000).div_euclid(u64::from(self.args.target_fps))
    }

    /// Converts the input frame to the target output using the specified
    /// settings.
    // #[crate::perf("")]
    pub fn convert_frame(&self, buf: image::DynamicImage) -> Result<image::DynamicImage, JsError> {
        let mut img = image::DynamicImage::ImageRgba8(buf.into_rgba8());
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
            filter => img.resize(out_dim.x, out_dim.y, match filter {
                ResamplingFilter::Catrom => image::imageops::CatmullRom,
                ResamplingFilter::Gaussian => image::imageops::Gaussian,
                ResamplingFilter::Lanczos3 => image::imageops::Lanczos3,
                ResamplingFilter::Nearest => image::imageops::Nearest,
                ResamplingFilter::Triangle => image::imageops::Triangle,
                _ => err_js!("Invalid image filter '{filter:?}'"),
            }),
        };

        // 3. Grayscale after resampling
        if self.args.grayscale_bits > 0 {
            let mut luma = img.into_luma8(); // Luma after resize
            match self.args.grayscale_bits {
                1 => {
                    for p in luma.as_mut() {
                        *p = 255 * u8::from(*p >= constants::GRAYSCALE_THRESH);
                    }
                },
                4 => {
                    for p in luma.as_mut() {
                        *p &= 0xF0;
                    }
                },
                8 => {},
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
            let indexes = image_util::quantized_frame_to_outputs(
                &img.view(0, 0, img.width(), img.height()),
                palette,
            );

            match img {
                image::DynamicImage::ImageRgba8(ref mut inner) => {
                    let colors = palette.get_colors();
                    for (pixel, v) in inner.pixels_mut().zip(indexes) {
                        let [r, g, b] = colors[v as usize];
                        *pixel = image::Rgba([r, g, b, 255]);
                    }
                },
                image::DynamicImage::ImageLuma8(ref mut inner) => {
                    let colors = palette.get_colors();
                    for (pixel, v) in inner.iter_mut().zip(indexes) {
                        *pixel = colors[v as usize][0];
                    }
                },
                _ => err_js!("Expected Rgba8 or Luma8, got '{:?}'", img.color()),
            }
        }
        Ok(img)
    }

    /// Packs palette frames into output signals by bit-packing palette indices.
    ///
    /// A single output represents the palette indices of a single pixel across
    /// multiple frames.
    ///
    /// Each palette index uses the minimum number of bits required to represent
    /// the palette.
    ///
    /// # Arguments
    ///
    /// * `frames` - A slice of image frames.
    /// * `palette` - The RGBA palette used to identify pixel indices.
    ///
    /// # Returns
    ///
    /// A vector of packed palette indices. Each group of frames produces
    /// `width * height` outputs, in row-major order.
    #[inline]
    pub fn palette_frames_to_outputs(
        &self, frames: &[image::SubImage<&image::DynamicImage>],
    ) -> Result<Box<[i32]>, JsError> {
        let Some(ref palette) = self.palette else {
            err_js!("palette_frames_to_outputs() called without a palette!");
        };
        image_util::palette_frames_to_outputs(palette, frames)
    }

    /// Picks one palette of `size` colors for all frames. Index 0 is always
    /// black, since black pixels are left out of the blueprint entirely.
    ///
    /// # Arguments
    ///
    /// * `frames` - The processed frames.
    /// * `size` - Number of palette entries, including black.
    ///
    /// # Returns
    ///
    /// The palette colors as RGB triples.
    pub async fn into_palette(mut self, size: u32, speed: i32) -> Result<ColorPalette, JsError> {
        use imagequant::RGBA;

        let mut attr = imagequant::Attributes::new();
        attr.set_max_colors(size)?;
        attr.set_speed(speed)?;

        let mut result = {
            let mut hist = imagequant::Histogram::new(&attr);

            while let Some(frame) = self.next().await? {
                let (w, h) = (frame.width() as usize, frame.height() as usize);
                let mut qimg = match RC::unwrap_or_clone(frame) {
                    image::DynamicImage::ImageRgba8(rgba) => {
                        attr.new_image(bytemuck::cast_slice(&rgba.into_raw()), w, h, 0.0)?
                    },
                    image::DynamicImage::ImageLuma8(luma) => {
                        let pixels = luma.into_raw();
                        let f = move |row: &mut [std::mem::MaybeUninit<RGBA>], y: usize| {
                            let start = y * w;
                            for (x, pixel) in row.iter_mut().enumerate() {
                                let l = pixels[start + x];
                                pixel.write(RGBA::new(l, l, l, 255));
                            }
                        };

                        // SAFETY: `Image::new_fn` requires the callback to initialize every
                        // `MaybeUninit<RGBA>` element in each row before returning. The row
                        // length is `w`, and `pixels` contains exactly `w * h` elements.
                        // `y` is a valid row index, so `start + x` is in bounds for every
                        // pixel written.
                        unsafe { imagequant::Image::new_fn(&attr, f, w, h, 0.0)? }
                    },
                    _ => unreachable!("Expected RGBA8 or LumaA8"),
                };
                hist.add_image(&attr, &mut qimg)?;
            }
            hist.quantize(&attr)?
        };
        let mut colors: Box<_> =
            result.palette_vec().into_iter().map(|c| [c.r, c.g, c.b]).collect();

        if let Some(darkest) = (0..colors.len()).min_by_key(|&i| {
            let c = colors[i];
            c[0].saturating_add(c[1]).saturating_add(c[2])
        }) {
            colors.swap(0, darkest);
        }

        ColorPalette::new(colors)
    }

    #[crate::perf("")]
    pub fn into_gif_color_count_map(mut self) -> Result<ColorCountMap, JsError> {
        let FrameDecoderState::Gif(ref mut decoder) = self.decoder.state else {
            err_js!("Expected self.decoder to be a GIF decoder!");
        };
        let decoder = &mut decoder.decoder;
        let total_duration = self.args.duration;

        let mut colors: ColorCountMap = HashMap::with_hasher(IdentityHasherBuilder::default());
        let mut curr_total_duration = 0_u64;
        let mut frame_i = 0_usize;

        while let Some(frame) = decoder.read_next_frame()? {
            let duration_ms = u64::from(frame.delay) * 10; // `frame.delay` is in units of 10ms
            let transparent = frame.transparent;

            if frame_i.is_multiple_of(10)
                && let Some(total_duration) = total_duration
            {
                set_progress(
                    0.0,
                    1.0,
                    curr_total_duration as f64 / (1000.0 * total_duration),
                    "Finding GIF color palette...",
                );
            }

            // Efficiently compute the number of occurences of each colour index
            let counts = if self.for_palette
                && let Some(ref mut histogram) = self.histogram_generator
            {
                // Decode the rest of the chunks
                (histogram.decode_bytes(&frame.buffer))
                    .map_err(|e| js_err!("GIF LZW decoding failed: {e:?}"))?
            } else {
                crate::util::u8_histogram_8chunks(&frame.buffer)
            };
            let palette = decoder.palette()?;

            for (i, count) in counts.into_iter().enumerate() {
                if count == 0 || transparent == Some(i as u8) {
                    continue;
                }
                let i = i * 3;
                let rgba = [palette[i], palette[i + 1], palette[i + 2], 255];

                *colors.entry(rgba).or_default() += count;
            }

            curr_total_duration += duration_ms;
            frame_i += 1;
        }

        Ok(colors)
    }
}

pub trait BufReadSeek: io::BufRead + io::Seek {}
impl<T: io::BufRead + io::Seek> BufReadSeek for T {}

enum FrameDecoderState<'a> {
    Frames(DynamicFrames<'a>),
    Gif(Box<GifData<'a>>),
    JsFrames(GetJsFrameCallback),
}

/// A single animation frame
#[derive(Clone)]
pub struct DynamicFrame {
    buffer: image::DynamicImage,
    /// Delay between the frames in milliseconds
    delay:  image::Delay,
}
impl DynamicFrame {
    #[inline]
    #[must_use]
    pub const fn new(buffer: image::DynamicImage, delay: image::Delay) -> Self {
        Self { buffer, delay }
    }

    #[inline]
    #[must_use]
    pub fn into_buffer(self) -> image::DynamicImage {
        self.buffer
    }

    #[inline]
    #[must_use]
    pub const fn delay(&self) -> image::Delay {
        self.delay
    }
}

pub struct DynamicFrames<'a> {
    /// Delay between the frames in milliseconds
    iterator: Box<dyn Iterator<Item = Result<DynamicFrame, JsError>> + 'a>,
}
impl<'a> DynamicFrames<'a> {
    #[inline]
    #[must_use]
    pub const fn new(
        iterator: Box<dyn Iterator<Item = Result<DynamicFrame, JsError>> + 'a>,
    ) -> Self {
        Self { iterator }
    }
}
impl std::iter::Iterator for DynamicFrames<'_> {
    type Item = Result<DynamicFrame, JsError>;
    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.iterator.next()
    }
}

struct GifData<'a> {
    decoder: gif::Decoder<Box<dyn BufReadSeek + 'a>>,
    screen:  gif_dispose::Screen,
}

pub type ColorCountMap = HashMap<[u8; 4], u32, IdentityHasherBuilder>;

#[derive(Clone)]
struct FrameWithTime(DynamicFrame, u64);

struct FrameDecoder<'a> {
    curr_n_us:           u64,
    dim:                 UVec2,
    finished:            bool,
    state:               FrameDecoderState<'a>,
    use_sparse_decoding: Option<bool>,
}
impl<'a> FrameDecoder<'a> {
    const fn new(state: FrameDecoderState<'a>, dim: UVec2) -> Self {
        FrameDecoder {
            curr_n_us: 0,
            dim,
            finished: false,
            state,
            use_sparse_decoding: None,
        }
    }

    #[inline]
    #[must_use]
    const fn with_use_sparse_decoding(mut self, use_sparse_decoding: Option<bool>) -> Self {
        self.use_sparse_decoding = use_sparse_decoding;
        self
    }

    #[inline]
    #[must_use]
    const fn is_finished(&self) -> bool {
        self.finished
    }

    async fn next_frame(&mut self) -> Result<Option<FrameWithTime>, JsError> {
        Ok(match self.state {
            FrameDecoderState::Frames(ref mut frames) => {
                let frame = frames.next().transpose()?.map(|f| {
                    let delay = f.delay();
                    let output = FrameWithTime(f, self.curr_n_us);
                    let delay: std::time::Duration = if unlikely(delay.numer_denom_ms().0 == 0) {
                        DEFAULT_FRAME_DELAY
                    } else {
                        delay.into()
                    };
                    self.curr_n_us += delay.as_micros() as u64;
                    output
                });
                self.finished = frame.is_none();
                frame
            },
            FrameDecoderState::Gif(ref mut data) => {
                let Some(frame) = data.decoder.read_next_frame()? else {
                    self.finished = true;
                    return Ok(None);
                };

                let duration_ms = u32::from(frame.delay) * 10; // frame.delay is in units of 10ms
                let duration_microseconds = u64::from(duration_ms) * 1000;
                data.screen.blit_frame(frame)?; // Advance the screen by 1 frame

                let (pixels, width, height) = data.screen.pixels_rgba().to_contiguous_buf();
                let bytes: &[u8] = bytemuck::cast_slice(&pixels);
                let img = image::RgbaImage::from_raw(width as u32, height as u32, bytes.to_vec())
                    .ok_or_js_err("Invalid RGBA frame dimensions")?;

                let img = image::DynamicImage::ImageRgba8(img);
                let duration = image::Delay::from_numer_denom_ms(duration_ms, 1);
                let frame = DynamicFrame::new(img, duration);
                let output = FrameWithTime(frame, self.curr_n_us);
                self.curr_n_us += duration_microseconds;

                Some(output)
            },
            FrameDecoderState::JsFrames(ref mut get_next_frame) => {
                let in_buf = vec![0_u8; self.dim.element_product() as usize * 4];
                let js_args = ts::TsIntoRust::new(&GetNextFrameArgs {
                    dimensions:          Some(self.dim),
                    in_buffer:           wasm_shared_memory(&in_buf)?,
                    use_sparse_decoding: self.use_sparse_decoding,
                })?;
                let result = get_next_frame.call_n((&js_args,))?.await?;
                let info: JsVideoFrameData = serde_wasm_bindgen::from_value(result)?;

                if unlikely(info.is_finished) {
                    self.finished = true;
                    return Ok(None);
                }

                let duration =
                    Duration::from_secs_f64(info.duration.ok_or_js_err("Missing duration")?);
                let delay = image::Delay::from_saturating_duration(duration);
                let img = image::RgbaImage::from_raw(self.dim.x, self.dim.y, in_buf)
                    .ok_or_js_err("Invalid RGBA frame dimensions")?;

                Some(FrameWithTime(
                    DynamicFrame::new(image::DynamicImage::ImageRgba8(img), delay),
                    (info.timestamp.ok_or_js_err("Missing timestamp")? * 1000.0 * 1000.0) as u64,
                ))
            },
        })
    }
}
