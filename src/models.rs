use glam::UVec2;
use serde::{Deserialize, Serialize};
use tsify::Tsify;
use wasm_bindgen::prelude::*;

#[derive(Tsify, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(
    clippy::allow_attributes,
    clippy::struct_excessive_bools,
    reason = "API may change in the future"
)]
pub struct BlueprintArgs {
    pub custom_dim:              Option<UVec2>,
    pub detailed_descriptions:   bool,
    pub display_margin_y:        u8,
    pub dry_run:                 bool,
    pub image_args:              ImageArgs,
    pub include_power_poles:     bool,
    pub low_mem_usage:           bool,
    pub max_group_size:          Option<u8>,
    pub name:                    String,
    pub output_format:           OutputFormat,
    pub power_pole_name:         Option<String>,
    pub power_pole_quality:      Option<String>,
    pub prefer_green_wires:      bool,
    pub prefer_horizontal_wires: bool,
    pub signal_compression:      Option<SignalCompression>,
    pub signal_sorting:          SignalSorting,
    pub streaming_download:      bool,
    pub timing_signals:          TimingSignals,
    pub use_threads:             bool,
}

#[derive(Tsify, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageArgs {
    pub color_palette_size: u32,
    pub dimensions:         Option<UVec2>,
    pub duration:           Option<f64>,
    pub flip:               ImageFlip,
    pub grayscale_bits:     u8,
    pub filters:            Box<[ImageFilter]>,
    pub max_size:           u16,
    pub mime_type:          String,
    pub mode:               Mode,
    pub n_frames:           Option<u32>,
    pub resampling_filter:  ResamplingFilter,
    pub rotate:             ImageRotate,
    pub shading_speed:      u8,
    pub target_fps:         u8,
}
impl ImageArgs {
    #[must_use]
    pub const fn bits(&self) -> u32 {
        if self.grayscale_bits > 0 {
            self.grayscale_bits as u32
        } else if self.color_palette_size > 0 {
            let bits = usize::BITS - (self.color_palette_size - 1).leading_zeros();
            if bits == 0 { 1 } else { bits }
        } else {
            0
        }
    }
}

#[derive(Tsify, Debug, Serialize, Deserialize)]
pub struct TimingSignals {
    pub f: crate::blueprint::models::SignalID,
    pub p: crate::blueprint::models::SignalID,
    pub s: crate::blueprint::models::SignalID,
    pub t: crate::blueprint::models::SignalID,
}

#[derive(Tsify, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(untagged)]
pub enum ImageFilter {
    Blur(f32),
    Brightness(i32),
    Contrast(f32),
    HueRotate(i32),
    Unsharpen(f32, i32),
}

#[derive(Tsify, Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[repr(u16)]
pub enum ImageRotate {
    #[default]
    #[serde(rename = "none")]
    Deg0 = 0,
    Deg90 = 90,
    Deg180 = 180,
    Deg270 = 270,
}
impl ImageRotate {
    /// Returns `true` if `self` is `90` or `270`, otherwise `false`.
    #[must_use]
    #[inline]
    pub const fn swaps_dimensions(self) -> bool {
        matches!(self, Self::Deg90 | Self::Deg270)
    }
}

#[derive(Tsify, Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImageFlip {
    #[default]
    None = 0,
    X = 1,
    Y = 2,
    Both = 3,
}
impl ImageFlip {
    #[must_use]
    #[inline]
    pub const fn x(self) -> bool {
        matches!(self, Self::X | Self::Both)
    }

    #[must_use]
    #[inline]
    pub const fn y(self) -> bool {
        matches!(self, Self::Y | Self::Both)
    }
}

#[derive(Tsify, Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Mode {
    Full,
    Lamps,
    LampGrid,

    #[serde(rename = "staticImage")]
    Static {
        #[serde(rename = "useConstantCombinators")]
        const_cb: bool,

        #[serde(rename = "useCombinators")]
        combs: bool,

        #[serde(rename = "useTimer")]
        timer: bool,
    },
}

#[derive(Tsify, Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SignalSorting {
    None,
    Compression,
    Json,
}

#[derive(Tsify, Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResamplingFilter {
    Catrom,
    Gaussian,
    Lanczos3,
    Nearest,
    Thumbnail,
    Triangle,
}
impl ResamplingFilter {
    #[must_use]
    #[inline]
    pub const fn to_image_filter(self) -> Option<image::imageops::FilterType> {
        match self {
            Self::Catrom => Some(image::imageops::FilterType::CatmullRom),
            Self::Gaussian => Some(image::imageops::FilterType::Gaussian),
            Self::Lanczos3 => Some(image::imageops::FilterType::Lanczos3),
            Self::Nearest => Some(image::imageops::FilterType::Nearest),
            Self::Triangle => Some(image::imageops::FilterType::Triangle),
            _ => None,
        }
    }
}

#[derive(Tsify, Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SignalCompression {
    Delta,
    Temporal { window: u32 },
}

#[derive(Tsify, Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OutputFormat {
    Blueprint,
    Json,
    RawCompressed,
}

#[derive(Tsify, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JsVideoFrameData {
    #[serde(default)]
    pub is_finished: bool,

    #[serde(default)]
    pub timestamp: Option<f64>,

    #[serde(default)]
    pub duration: Option<f64>,
}

#[derive(Tsify, Deserialize)]
pub struct EncodeGifArgs {
    pub dim:   UVec2,
    pub speed: u8,
}

pub type RawJsonString = Box<serde_json::value::RawValue>;

#[wasm_bindgen(typescript_custom_section)]
const TS_TYPES: &str = r#"
export type DVec2 = [number, number];
export type UVec2 = [number, number];
"#;
