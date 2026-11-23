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

pub fn serialize<S: serde::Serializer>(v: &DVec2, serializer: S) -> Result<S::Ok, S::Error> {
    F64XY { x: v.x, y: v.y }.serialize(serializer)
}
pub fn deserialize<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<DVec2, D::Error> {
    let xy = F64XY::deserialize(deserializer)?;
    Ok(DVec2::new(xy.x, xy.y))
}
