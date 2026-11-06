use glam::Vec2;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct XY {
    x: f32,
    y: f32,
}

pub fn serialize<S: serde::Serializer>(v: &Vec2, serializer: S) -> Result<S::Ok, S::Error> {
    XY { x: v.x, y: v.y }.serialize(serializer)
}
pub fn deserialize<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Vec2, D::Error> {
    let xy = XY::deserialize(deserializer)?;
    Ok(Vec2::new(xy.x, xy.y))
}
