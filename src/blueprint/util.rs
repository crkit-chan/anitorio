use super::prelude::*;
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

pub fn invert_wires(ents: &mut [BlueprintEntity], wires: &mut [Wire]) {
    const fn get_swap(wire_id: u32) -> u32 {
        match wire_id {
            1 => 2,
            2 => 1,
            3 => 4,
            4 => 3,
            _ => wire_id,
        }
    }
    for e in ents.iter_mut() {
        if let Some(ControlBehavior::DeciderCombinator { decider_conditions }) =
            e.control_behavior.as_mut()
        {
            decider_conditions.swap_networks();
        }
    }
    for wire in wires.iter_mut() {
        wire[1] = get_swap(wire[1]);
        wire[3] = get_swap(wire[3]);
    }
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
pub fn color_frame_to_outputs(frame: &image::DynamicImage) -> Vec<i32> {
    #[cfg(target_arch = "wasm32")]
    {
        use core::arch::wasm32::{i8x16_shuffle, i8x16_splat, v128, v128_load, v128_store};

        let rgba = frame.to_rgba8();
        let rgba: &[u8] = rgba.as_raw();

        debug_assert_eq!(rgba.len() % 4, 0);

        let pixel_count = rgba.len() / 4;
        let zero = i8x16_splat(0);
        let mut output = Vec::<i32>::with_capacity(pixel_count);
        let mut i = 0;

        while i + 16 <= rgba.len() {
            #[expect(
                clippy::cast_ptr_alignment,
                reason = "v128_load supports unaligned memory accesses"
            )]
            #[expect(
                clippy::ptr_as_ptr,
                reason = "Pointer is cast only to pass the byte buffer to v128_load"
            )]
            unsafe {
                let input = v128_load(rgba.as_ptr().add(i) as *const v128); // Load 4 RGBA pixels: [R0 G0 B0 A0 R1 G1 B1 A1 R2 G2 B2 A2 R3 G3 B3 A3]
                let packed = i8x16_shuffle::<2, 1, 0, 16, 6, 5, 4, 16, 10, 9, 8, 16, 14, 13, 12, 16>(
                    input, zero, // Indices 0..15 = input. Indices 16..31 = zero
                ); // Little-endian output needs: [B0 G0 R0 00 B1 G1 R1 00 B2 G2 R2 00 B3 G3 R3 00]
                v128_store(output.as_mut_ptr().add(i / 4) as *mut v128, packed); // Write the 4 resulting i32s directly.
            }
            i += 16;
        }

        unsafe { output.set_len(pixel_count) };

        // Handle remaining pixels.
        while i < rgba.len() {
            let r = rgba[i];
            let g = rgba[i + 1];
            let b = rgba[i + 2];

            output[i / 4] = (i32::from(r) << 16) | (i32::from(g) << 8) | i32::from(b);
            i += 4;
        }

        output
    }

    #[cfg(not(target_arch = "wasm32"))] // scalar fallback
    {
        use crate::image_processing::rgb_to_int;
        (frame.to_rgb8().as_raw().as_chunks::<3>().0)
            .iter()
            .map(|&[r, g, b]| rgb_to_int(r, g, b).cast_signed())
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
    frames: &[image::DynamicImage],
    grayscale_bits: u32,
) -> Result<Vec<i32>, JsError> {
    if frames.is_empty() {
        return Err(JsError::new("No frames provided for packing"));
    }
    let num_pixels = (frames[0].width() * frames[0].height()) as usize;
    let luma_images: Vec<_> = frames.iter().map(image::DynamicImage::to_luma8).collect();
    let mut outputs: Vec<i32> = vec![0i32; num_pixels];

    match grayscale_bits {
        1 => {
            for (i, output) in outputs.iter_mut().enumerate() {
                let mut packed: u32 = 0;
                for (j, img) in luma_images.iter().enumerate() {
                    packed |= u32::from(img.as_raw()[i] >= constants::GRAYSCALE_THRESH) << j;
                }
                *output = packed.cast_signed();
            }
        }
        4 => {
            for (i, output) in outputs.iter_mut().enumerate() {
                let mut packed: u32 = 0;
                for (j, img) in luma_images.iter().enumerate() {
                    packed |= u32::from(img.as_raw()[i] >> 4) << (j * 4);
                }
                *output = packed.cast_signed();
            }
        }
        8 => {
            for (i, output) in outputs.iter_mut().enumerate() {
                let mut packed: u32 = 0;
                for (j, img) in luma_images.iter().enumerate() {
                    packed |= u32::from(img.as_raw()[i]) << (j * 8);
                }
                *output = packed.cast_signed();
            }
        }
        _ => return Err(JsError::new("Unsupported grayscale bit depth")),
    }
    Ok(outputs)
}
