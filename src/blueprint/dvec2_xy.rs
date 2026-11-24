use glam::DVec2;
use serde::{Deserialize, Serialize};

#[derive(
    Clone, Copy, Debug, PartialEq, Serialize, Deserialize, bytemuck::Pod, bytemuck::Zeroable,
)]
#[repr(C)]
pub struct F64XY {
    pub x: f64,
    pub y: f64,
}

#[allow(clippy::allow_attributes, dead_code, reason = "May not be used")]
pub fn serialize<S: serde::Serializer>(v: &DVec2, serializer: S) -> Result<S::Ok, S::Error> {
    let xy: &F64XY = bytemuck::cast_ref(v);
    xy.serialize(serializer)
}
pub fn deserialize<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<DVec2, D::Error> {
    Ok(bytemuck::cast(F64XY::deserialize(deserializer)?))
}
