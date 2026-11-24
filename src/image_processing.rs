use crate::constants;
use crate::util::wasm_shared_memory;
use crate::{constants::DEFAULT_FRAME_DELAY, util::format_duration};
use crate::{prelude::*, ts};
use alloc::{rc::Rc, sync::Arc};
use core::{pin::Pin, time::Duration};
use glam::{DVec2, UVec2, uvec2};
use image::{AnimationDecoder as _, GenericImageView as _, imageops};
use wasm_bindgen::JsValue;

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
    progress_msg: String,
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
    ) -> Result<Self, JsError> {
        #[must_use]
        #[inline]
        const fn expected_output_frames(n_ms: u64, fps: u32) -> usize {
            1 + ((n_ms * fps as u64).div_euclid(1000)) as usize
        }

        let in_dim = frame_data.info.dimensions.ok_or_js_err("Missing dimensions!")?;
        let scale_factor = (f64::from(image_args.max_size) / f64::from(in_dim.x))
            .min(f64::from(image_args.max_size) / f64::from(in_dim.y))
            .min(1.0);

        let mut obj = Self {
            out_n_frames: match image_args.mode {
                Mode::Static { .. } => 1,
                _ => expected_output_frames(
                    (frame_data.info.duration.ok_or_js_err("FrameData missing 'duration' field")?
                        * 1000.0) as u64,
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
            progress_msg: String::new(),
        };
        obj.decoder.dim = obj.out_dim_raw();

        Ok(obj)
    }

    pub fn with_progress_message(mut self, msg: impl Into<String>) -> Self {
        self.progress_msg = msg.into();
        self
    }

    pub async fn next(&mut self) -> Result<Option<Arc<image::DynamicImage>>, JsError> {
        let mut img: Option<Arc<image::DynamicImage>> = None;
        let use_js_decode = matches!(self.decoder.state, FrameDecoderState::JsFrames { .. });

        if unlikely(matches!(self.args.mode, Mode::Static { .. })) {
            let Some(FrameWithTime(f, timestamp)) = self.decoder.next_frame().await? else {
                return Ok(None);
            };
            img = Some(Arc::new(self.convert_frame(f.into_buffer())?));
            self.curr_n_us = timestamp;
        } else if unlikely(self.out_frame_i == 0) {
            self.fps_timer = now_ms();
        }

        loop {
            if self.out_t_us() <= self.curr_n_us
                && let Some(ref prev) = self.prev_img
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

                let img = self.convert_frame(frame.into_buffer())?;
                self.prev_img = Some((Arc::new(img), self.curr_n_us));
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

                if !self.progress_msg.is_empty() {
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
    pub fn get_palette_colors(&self) -> Option<&[[u8; 3]]> {
        if let Some(ref palette) = self.palette {
            return Some(&palette.colors);
        }
        None
    }

    //// Returns the current output time in microseconds.
    #[must_use]
    #[inline]
    fn out_t_us(&self) -> u64 {
        (self.out_frame_i as u64 * 1000 * 1000).div_euclid(u64::from(self.args.target_fps))
    }

    /// Converts the input frame to the target output using the specified settings.
    pub fn convert_frame(&self, buf: image::RgbaImage) -> Result<image::DynamicImage, JsError> {
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
            filter => img.resize(out_dim.x, out_dim.y, filter.to_image_filter()?),
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

            match img {
                image::DynamicImage::ImageRgba8(ref mut inner) => {
                    for (pixel, v) in inner.pixels_mut().zip(indexes) {
                        let [r, g, b] = palette.colors[v as usize];
                        *pixel = image::Rgba([r, g, b, 255]);
                    }
                }
                image::DynamicImage::ImageLuma8(ref mut inner) => {
                    for (pixel, v) in inner.iter_mut().zip(indexes) {
                        *pixel = palette.colors[v as usize][0];
                    }
                }
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
    pub fn palette_frames_to_outputs(
        &self,
        frames: &[image::SubImage<&image::DynamicImage>],
    ) -> Result<Box<[i32]>, JsError> {
        let Some(ref palette) = self.palette else {
            err_js!("palette_frames_to_outputs() called without a palette!");
        };

        assert!(!frames.is_empty(), "No frames provided for packing");
        assert!(!palette.colors.is_empty(), "Palette cannot be empty");

        let offsets = frames[0].offsets();
        let (width, height) = frames[0].dimensions();
        assert!(
            (frames.iter()).all(|f| f.dimensions() == (width, height) && f.offsets() == offsets),
            "All frames must have identical dimensions"
        );

        let bits_per_index = (usize::BITS - (palette.colors.len() - 1).leading_zeros()).max(1);
        assert!(bits_per_index <= 32, "Palette too large to pack into i32");

        let frames = (frames.iter().map(|f| match *f.inner() {
            image::DynamicImage::ImageRgba8(ref f) => Ok(f),
            _ => err_js!("Expected all frames to be RGBA8"),
        }))
        .collect::<Result<Box<_>, _>>()?;

        let frames_per_output = (32_u32.div_euclid(bits_per_index)) as usize;
        let n_groups = frames.len().div_ceil(frames_per_output);
        let n_pixels = (width * height) as usize;

        let mut outputs = boxed_slice![0_i32; n_pixels * n_groups];

        for (group_i, frame_group) in frames.chunks(frames_per_output).enumerate() {
            let group_offset = group_i * n_pixels;

            for (y, x) in (0..height).flat_map(|y| (0..width).map(move |x| (y, x))) {
                let pixel_idx = (y * width + x) as usize;
                let mut packed = 0_u32;

                for (frame_i, frame) in frame_group.iter().enumerate() {
                    let [r, g, b, ..] = frame.get_pixel(x + offsets.0, y + offsets.1).0;
                    packed |= palette.nearest_index([r, g, b]) << (frame_i as u32 * bits_per_index);
                }

                outputs[group_offset + pixel_idx] = packed.cast_signed();
            }
        }

        Ok(outputs)
    }
}

pub struct ImageFrameData<'a> {
    decoder: FrameDecoder<'a>,
    info:    ImageArgs,
}
impl<'a> ImageFrameData<'a> {
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

                let dim = image_args.dimensions.ok_or_js_err("No image size")?;
                let mut data = Vec::new();
                reader.read_to_end(&mut data)?;

                let frames = image::Frames::new(Box::new(core::iter::once(Ok(image::Frame::new(
                    image::RgbaImage::from_raw(dim.x, dim.y, data)
                        .ok_or_js_err("buffer length must be width * height * 4")?,
                ))))); // bruh
                FrameDecoderState::NativeFrames(frames)
            }
            t if t.starts_with("video/") => {
                info.n_frames = None;
                FrameDecoderState::JsFrames(
                    js_next_frame.ok_or_js_err("JS next frame must be provided for videos")?,
                )
            }
            t => err_js!("Unsupported image type: '{t}'"),
        };

        let decoder =
            FrameDecoder::new(state, info.dimensions.ok_or_js_err("Missing dimensions!")?);
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
    use imagequant::RGBA;

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
                image::DynamicImage::ImageLuma8(luma) => {
                    let pixels = luma.into_raw();
                    let f = move |row: &mut [core::mem::MaybeUninit<RGBA>], y: usize| {
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
                }
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

    ColorPalette::new(colors)
}

/// Converts an RGB pixel to an integer using a utility function.
///
/// # Arguments
///
/// * `r` - Red channel.
/// * `g` - Green channel.
/// * `b` - Blue channel.
///
/// # Returns
///
/// A vector of ``CombinatorOutputs`` for the frame
#[cfg_attr(target_arch = "wasm32", target_feature(enable = "simd128"))]
pub fn color_frame_to_outputs(
    view: &image::SubImage<&image::DynamicImage>,
) -> Result<Box<[i32]>, JsError> {
    #[cfg(target_arch = "wasm32")]
    {
        use core::arch::wasm32::{i8x16_shuffle, i8x16_splat, v128, v128_load, v128_store};
        use image::GenericImageView as _;

        let Some(rgba) = view.inner().as_rgba8() else {
            err_js!("Expected RGBA8, got '{:?}'", view.inner().color())
        };

        let (x, y) = view.offsets();
        let (width, height) = (view.width() as usize, view.height() as usize);
        let mut output: Vec<i32> = Vec::with_capacity(width * height);

        let stride = rgba.width() as usize * 4;
        let raw = rgba.as_raw();
        let zero = i8x16_splat(0);

        for row in 0..height {
            let start = (y as usize + row) * stride + x as usize * 4;
            let bytes = &raw[start..start + width * 4];

            let mut i = 0;

            // SAFETY: `output` has capacity for `width * height` elements. Since
            // `row < height`, `row * width` is within that allocation.
            let output_ptr = unsafe { output.as_mut_ptr().add(row * width) };

            #[expect(clippy::cast_ptr_alignment, reason = "v128_load uses unaligned memory")]
            #[expect(clippy::ptr_as_ptr, reason = "Cast to pass byte buf to v128_load")]
            while i + 4 <= width {
                // SAFETY: `i + 4 <= width` guarantees at least 16 bytes remain in `bytes`.
                let input_ptr = unsafe { bytes.as_ptr().add(i * 4) as *const v128 };

                // SAFETY: WebAssembly SIMD loads permit unaligned memory accesses.
                let input = unsafe { v128_load(input_ptr) };
                let packed = i8x16_shuffle::<2, 1, 0, 16, 6, 5, 4, 16, 10, 9, 8, 16, 14, 13, 12, 16>(
                    input, zero,
                );

                // SAFETY: `i + 4 <= width` guarantees four remaining output elements.
                let output_ptr = unsafe { output_ptr.add(i) as *mut v128 };

                // SAFETY: WebAssembly SIMD stores permit unaligned memory accesses.
                unsafe { v128_store(output_ptr, packed) };

                i += 4;
            }
            while i < width {
                let j = i * 4;

                // SAFETY: `i < width` guarantees the pointer is within the allocated output capacity.
                let pixel_ptr = unsafe { output_ptr.add(i) };

                // SAFETY: `pixel_ptr` points to a valid, properly aligned, writable
                // `i32` slot within `output`'s allocated capacity. This slot has not
                // yet been initialized, and the assignment initializes it.
                unsafe {
                    *pixel_ptr = (i32::from(bytes[j]) << 16)
                        | (i32::from(bytes[j + 1]) << 8)
                        | i32::from(bytes[j + 2]);
                };
                i += 1;
            }

            // SAFETY: Every element in the current row has been initialized by
            // the SIMD and scalar loops. All previous rows were initialized
            // before their lengths were committed. `(row + 1) * width` does
            // not exceed the vector's allocated capacity.
            unsafe { output.set_len((row + 1) * width) }; // Mark this row's output as initialized.
        }

        Ok(output.into_boxed_slice())
    }

    #[cfg(not(target_arch = "wasm32"))] // scalar fallback
    {
        Ok((rgba.as_raw().as_chunks::<4>().0.iter())
            .map(|&[r, g, b, ..]| crate::image_processing::rgb_to_int(r, g, b).cast_signed())
            .collect())
    }
}

/// Packs grayscale frames into output signals by bit-packing pixel values.
///
/// A single output represents the value of a single pixel across multiple frames.
///
/// # Arguments
///
/// * `frames` - A slice of grayscale image frames.
/// * `prev_outputs` - Previous output values, for delta compression.
/// * `grayscale_bits` - Number of bits for grayscale conversion.
///
/// # Returns
///
/// A vector of JSON objects representing output filters.
pub fn grayscale_frames_to_outputs(
    frames: &[image::SubImage<&image::DynamicImage>],
    bits: u32,
) -> Result<Box<[i32]>, JsError> {
    assert!(!frames.is_empty(), "No frames provided for packing");

    let (width, height) = frames[0].dimensions();
    let num_pixels = (width * height) as usize;
    let mut outputs = boxed_slice![0_i32; num_pixels];

    match bits {
        1 | 4 | 8 => {
            for (i, output) in outputs.iter_mut().enumerate() {
                let mut packed = 0_u32;

                for (j, img) in frames.iter().enumerate() {
                    let x = (i as u32).rem_euclid(width);
                    let y = (i as u32).div_euclid(width);

                    let pixel = img.get_pixel(x, y)[0];
                    let value = match bits {
                        1 => u32::from(pixel >= constants::GRAYSCALE_THRESH),
                        4 => u32::from(pixel >> 4),
                        8 => u32::from(pixel),
                        _ => unreachable!(),
                    };

                    packed |= value << (j as u32 * bits);
                }

                *output = packed.cast_signed();
            }
        }
        _ => err_js!("Unsupported grayscale bit depth '{bits}'"),
    }

    Ok(outputs)
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
) -> Box<[u8]> {
    let (x, y) = frame.offsets();
    let (width, height) = (frame.width() as usize, frame.height() as usize);
    let inner = frame.inner();
    let mut outputs = Vec::<u8>::with_capacity(width * height);
    let stride = frame.width() as usize;

    match *inner {
        image::DynamicImage::ImageRgba8(ref img) => {
            let rgba = img.as_raw();
            for row in 0..height {
                let start = ((y as usize + row) * stride + x as usize) * 4;
                let end = start + width * 4;
                outputs.extend(
                    (rgba[start..end].as_chunks::<4>().0.iter())
                        .map(|&[r, g, b, ..]| palette.nearest_index([r, g, b]) as u8),
                );
            }
        }
        image::DynamicImage::ImageLuma8(ref img) => {
            let luma = img.as_raw();
            for row in 0..height {
                let start = (y as usize + row) * stride + x as usize;
                let end = start + width;
                outputs.extend(
                    luma[start..end].iter().map(|&l| palette.nearest_index([l, l, l]) as u8),
                );
            }
        }
        _ => unreachable!("Expected RGBA8 or Luma8"),
    }

    outputs.into_boxed_slice()
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
        Ok(match self.state {
            FrameDecoderState::NativeFrames(ref mut frames) => {
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
            FrameDecoderState::JsFrames(ref mut get_next_frame) => {
                let args_obj = js_sys::Object::new();
                let in_buf = vec![0_u8; self.dim.element_product() as usize * 4];
                args_obj.reflect_set("inBuffer", wasm_shared_memory(&in_buf)?)?;
                args_obj.reflect_set("dimensions", serde_wasm_bindgen::to_value(&self.dim)?)?;

                let result = get_next_frame.call_n((&args_obj.into(),))?.await?;
                let info: JsVideoFrameData = serde_wasm_bindgen::from_value(result)?;

                if unlikely(info.is_finished) {
                    self.finished = true;
                    return Ok(None);
                }

                let duration =
                    Duration::from_secs_f64(info.duration.ok_or_js_err("Missing duration")?);
                let duration = image::Delay::from_saturating_duration(duration);
                let img = image::RgbaImage::from_raw(self.dim.x, self.dim.y, in_buf)
                    .ok_or_js_err("Invalid RGBA frame dimensions")?;

                Some(FrameWithTime(
                    image::Frame::from_parts(img, 0, 0, duration),
                    (info.timestamp.ok_or_js_err("Missing timestamp")? * 1000.0 * 1000.0) as u64,
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
    use alloc::rc::Rc;
    use core::cell::RefCell;
    use image::codecs::gif::{GifEncoder, Repeat};

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

    let in_buf = vec![0_u8; args.dim.element_product() as usize * 4];
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

        let duration = Duration::from_secs_f64(info.duration.ok_or_js_err("Missing duration")?);
        let duration = image::Delay::from_saturating_duration(duration);
        let img = image::RgbaImage::from_raw(args.dim.x, args.dim.y, in_buf.clone())
            .ok_or_js_err("Invalid RGBA frame dimensions")?;
        let f = image::Frame::from_parts(img, 0, 0, duration);
        encoder.encode_frame(f)?;

        flush().await?;
    }

    Ok(())
}

#[inline]
#[must_use]
pub const fn rgb_to_int(r: u8, g: u8, b: u8) -> u32 {
    ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
}
