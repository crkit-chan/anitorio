use image::GenericImageView;

use super::prelude::*;
use crate::boxed_slice;
use crate::constants;
use crate::prelude::*;
use std::io;

pub fn write_to(writer: &mut impl io::Write, buf: &[u8]) -> Result<(), JsError> {
    writer.write_all(buf).map_js_err("Write error")
}
pub fn write_json_trimmed_to<T: ?Sized + serde::Serialize>(
    writer: &mut impl io::Write,
    value: &T,
) -> Result<(), JsError> {
    let json_bytes = serde_json::to_vec(value).map_js_err("Write error")?;
    write_to(writer, &json_bytes[1..json_bytes.len() - 1]) // Remove braces
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
pub fn color_frame_to_outputs(rgba: &image::SubImage<&image::DynamicImage>) -> Box<[i32]> {
    #[cfg(target_arch = "wasm32")]
    {
        use core::arch::wasm32::{i8x16_shuffle, i8x16_splat, v128, v128_load, v128_store};
        use image::GenericImageView;

        let (width, height) = (rgba.width() as usize, rgba.height() as usize);
        let mut output: Vec<i32> = Vec::with_capacity(width * height);

        let inner = rgba.inner();
        let (x, y) = rgba.offsets();

        let Some(rgba) = inner.as_rgba8() else {
            panic!("Expected RGBA8, got '{:?}'", inner.color())
        };
        let rgba = rgba.as_raw();
        let stride = inner.width() as usize * 4;

        let zero = i8x16_splat(0);

        for row in 0..height {
            let start = (y as usize + row) * stride + x as usize * 4;
            let bytes = &rgba[start..start + width * 4];

            let mut i = 0;

            #[expect(clippy::cast_ptr_alignment, reason = "v128_load uses unaligned memory")]
            #[expect(clippy::ptr_as_ptr, reason = "Cast to pass byte buf to v128_load")]
            unsafe {
                let output_ptr = output.as_mut_ptr().add(row * width);
                while i + 4 <= width {
                    let input = v128_load(bytes.as_ptr().add(i * 4) as *const v128);
                    let packed =
                        i8x16_shuffle::<2, 1, 0, 16, 6, 5, 4, 16, 10, 9, 8, 16, 14, 13, 12, 16>(
                            input, zero,
                        );
                    v128_store(output_ptr.add(i) as *mut v128, packed);
                    i += 4;
                }
                while i < width {
                    let j = i * 4;
                    *output_ptr.add(i) = (i32::from(bytes[j]) << 16)
                        | (i32::from(bytes[j + 1]) << 8)
                        | i32::from(bytes[j + 2]);
                    i += 1;
                }
            }

            unsafe {
                output.set_len((row + 1) * width);
            } // Mark this row's output as initialized.
        }

        output.into_boxed_slice()
    }

    #[cfg(not(target_arch = "wasm32"))] // scalar fallback
    {
        (rgba.as_raw().as_chunks::<4>().0.iter())
            .map(|&[r, g, b, ..]| crate::image_processing::rgb_to_int(r, g, b).cast_signed())
            .collect()
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
    grayscale_bits: u32,
) -> Box<[i32]> {
    assert!(!frames.is_empty(), "No frames provided for packing");

    let num_pixels = (frames[0].width() * frames[0].height()) as usize;
    let mut outputs = boxed_slice![0i32; num_pixels];

    match grayscale_bits {
        1 | 4 | 8 => {
            let bits = grayscale_bits;

            for (i, output) in outputs.iter_mut().enumerate() {
                let mut packed = 0u32;

                for (j, img) in frames.iter().enumerate() {
                    let x = (i as u32) % frames[0].width();
                    let y = (i as u32) / frames[0].width();

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
        _ => panic!("Unsupported grayscale bit depth '{grayscale_bits}'"),
    }
    outputs
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
    frames: &[image::SubImage<&image::DynamicImage>],
    palette: &ColorPalette,
) -> Box<[i32]> {
    use image::GenericImageView;

    assert!(!frames.is_empty(), "No frames provided for packing");
    assert!(!palette.colors.is_empty(), "Palette cannot be empty");

    let (width, height) = frames[0].dimensions();
    assert!(
        !frames.iter().any(|f| f.dimensions() != (width, height)),
        "All frames must have identical dimensions"
    );

    let bits_per_index = (usize::BITS - (palette.colors.len() - 1).leading_zeros()).max(1);
    assert!(bits_per_index <= 32, "Palette too large to pack into i32");

    let frames_per_output = (32 / bits_per_index) as usize;
    let n_groups = frames.len().div_ceil(frames_per_output);
    let n_pixels = (width * height) as usize;

    let mut outputs = boxed_slice![0i32; n_pixels * n_groups];

    for (group_idx, frame_group) in frames.chunks(frames_per_output).enumerate() {
        let group_offset = group_idx * n_pixels;
        for (y, x) in (0..height).flat_map(|y| (0..width).map(move |x| (y, x))) {
            let pixel_idx = (y * width + x) as usize;
            let mut packed = 0u32;

            for (frame_idx, frame) in frame_group.iter().enumerate() {
                let [r, g, b, ..] = frame.get_pixel(x, y).0;
                packed |= palette.nearest_index([r, g, b]) << (frame_idx as u32 * bits_per_index);
            }

            outputs[group_offset + pixel_idx] = packed.cast_signed();
        }
    }

    outputs
}
