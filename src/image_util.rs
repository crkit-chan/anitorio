use std::collections::HashMap;
use std::io;

use image::GenericImageView as _;

use crate::constants;
use crate::hash::IdentityHasherBuilder;
use crate::image_processing::FrameData;
use crate::prelude::*;

/// Copied from image crate
#[must_use]
pub const fn resize_dimensions(
    width: u32, height: u32, nwidth: u32, nheight: u32, fill: bool,
) -> (u32, u32) {
    let wratio = nwidth as f64 / width as f64;
    let hratio = nheight as f64 / height as f64;

    let ratio = if fill {
        wratio.max(hratio)
    } else {
        wratio.min(hratio)
    };

    let nw = 1.max((width as f64 * ratio).round() as u64);
    let nh = 1.max((height as f64 * ratio).round() as u64);

    if nw > u32::MAX as u64 {
        let ratio = u32::MAX as f64 / width as f64;
        (u32::MAX, 1.max((height as f64 * ratio).round() as u32))
    } else if nh > u32::MAX as u64 {
        let ratio = u32::MAX as f64 / height as f64;
        (1.max((width as f64 * ratio).round() as u32), u32::MAX)
    } else {
        (nw as u32, nh as u32)
    }
}

/// Picks one palette of `size` colors for all frames. Index 0 is always black,
/// since black pixels are left out of the blueprint entirely.
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
    mut generator: FrameData<'_>, size: u32, speed: i32,
) -> Result<ColorPalette, JsError> {
    use imagequant::RGBA;

    let mut attr = imagequant::Attributes::new();
    attr.set_max_colors(size)?;
    attr.set_speed(speed)?;

    let mut result = {
        let mut hist = imagequant::Histogram::new(&attr);

        while let Some(frame) = generator.next().await? {
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
    let mut colors: Box<_> = result.palette_vec().into_iter().map(|c| [c.r, c.g, c.b]).collect();

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
        use std::arch::wasm32::{i8x16_shuffle, i8x16_splat, v128, v128_load, v128_store};

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

                // SAFETY: `i < width` guarantees the pointer is within the allocated output
                // capacity.
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
/// A single output represents the value of a single pixel across multiple
/// frames.
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
    frames: &[image::SubImage<&image::DynamicImage>], bits: u32,
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
        },
        _ => err_js!("Unsupported grayscale bit depth '{bits}'"),
    }

    Ok(outputs)
}

pub fn palette_frames_to_outputs(
    palette: &ColorPalette, frames: &[image::SubImage<&image::DynamicImage>],
) -> Result<Box<[i32]>, JsError> {
    assert!(!frames.is_empty(), "No frames provided for packing");
    assert!(!palette.get_colors().is_empty(), "Palette cannot be empty");

    let offsets = frames[0].offsets();
    let (width, height) = (frames[0].width() as usize, frames[0].height() as usize);
    assert!(
        (frames.iter())
            .all(|f| f.dimensions() == (width as u32, height as u32) && f.offsets() == offsets),
        "All frames must have identical dimensions"
    );

    let bits_per_index = (usize::BITS - (palette.get_colors().len() - 1).leading_zeros()).max(1);
    assert!(bits_per_index <= 32, "Palette too large to pack into i32");

    let frames_per_output = (32_u32.div_euclid(bits_per_index)) as usize;
    let n_groups = frames.len().div_ceil(frames_per_output);
    let n_pixels = width * height;

    let mut outputs = boxed_slice![0_i32; n_pixels * n_groups];

    let offset_x = offsets.0 as usize * 4;
    let offset_y = offsets.1 as usize;

    for (frame_i, &frame) in frames.iter().enumerate() {
        let group_i = frame_i.div_euclid(frames_per_output);
        let frame_i = frame_i.rem_euclid(frames_per_output);

        let image::DynamicImage::ImageRgba8(ref img) = *frame.inner() else {
            err_js!("Expected all frames to be RGBA8");
        };

        let raw = img.as_raw();
        let stride = img.width() as usize * 4;

        let group_offset = group_i * n_pixels;

        for y in 0..height {
            let pixel_row = y * width;
            let source_row = (y + offset_y) * stride + offset_x;

            for x in 0..width {
                let pixel_i = pixel_row + x;
                let i = source_row + x * 4;
                let palette_i = palette.exact_index([raw[i], raw[i + 1], raw[i + 2]]);

                outputs[group_offset + pixel_i] |=
                    (palette_i << (frame_i as u32 * bits_per_index)).cast_signed();
            }
        }
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
    frame: &image::SubImage<&image::DynamicImage>, palette: &ColorPalette,
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
        },
        image::DynamicImage::ImageLuma8(ref img) => {
            let luma = img.as_raw();
            for row in 0..height {
                let start = (y as usize + row) * stride + x as usize;
                let end = start + width;
                outputs.extend(
                    luma[start..end].iter().map(|&l| palette.nearest_index([l, l, l]) as u8),
                );
            }
        },
        _ => unreachable!("Expected RGBA8 or Luma8"),
    }

    outputs.into_boxed_slice()
}

pub type GifColorCountMap = HashMap<[u8; 4], u32, IdentityHasherBuilder>;

pub fn count_gif_colors<R: io::Read>(
    reader: R, total_duration: Option<f64>,
) -> Result<GifColorCountMap, JsError> {
    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::Indexed);

    let mut curr_total_duration = 0_u64;
    let mut decoder = options.read_info(reader)?;
    let mut colors: GifColorCountMap = HashMap::with_hasher(IdentityHasherBuilder::default());

    while let Some(frame) = decoder.read_next_frame()? {
        let duration_ms = u64::from(frame.delay) * 10; // `frame.delay` is in units of 10ms
        let mut counts = [0_u64; 256];

        if let Some(total_duration) = total_duration {
            set_progress(
                0.0,
                1.0,
                curr_total_duration as f64 / (1000.0 * total_duration),
                "Finding GIF color palette...",
            );
        }

        for &i in frame.buffer.iter() {
            if likely(frame.transparent != Some(i)) {
                counts[i as usize] += 1;
            }
        }

        let palette = decoder.palette()?;
        for (i, count) in counts.into_iter().enumerate() {
            if count == 0 {
                continue;
            }
            let i = i * 3;
            let rgba = [palette[i], palette[i + 1], palette[i + 2], 255];

            *colors.entry(rgba).or_default() += count as u32;
        }

        curr_total_duration += duration_ms;
    }

    Ok(colors)
}

pub fn color_counts_to_palette(
    color_counts: GifColorCountMap, size: u32, speed: i32,
) -> Result<ColorPalette, JsError> {
    let mut attr = imagequant::Attributes::new();
    attr.set_max_colors(size)?;
    attr.set_speed(speed)?;

    let mut result = {
        let mut hist = imagequant::Histogram::new(&attr);
        let color_counts_iter = (color_counts.into_iter()).map(|(color, count)| {
            imagequant::HistogramEntry { color: bytemuck::cast(color), count }
        });
        hist.add_colors(&color_counts_iter.collect::<Box<_>>(), 0.0)?;
        hist.quantize(&attr)?
    };
    let mut colors: Box<_> = result.palette_vec().into_iter().map(|c| [c.r, c.g, c.b]).collect();

    if let Some(darkest) = (0..colors.len()).min_by_key(|&i| {
        let c = colors[i];
        c[0].saturating_add(c[1]).saturating_add(c[2])
    }) {
        colors.swap(0, darkest);
    }

    ColorPalette::new(colors)
}

#[inline]
#[must_use]
pub const fn rgb_to_int(r: u8, g: u8, b: u8) -> u32 {
    ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
}
