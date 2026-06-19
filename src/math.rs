use serde::{Serialize, Deserialize};
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub struct Vector2 {
    pub x: f32,
    pub y: f32,
}
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, Default, PartialEq)]
pub struct Vector3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, Default, PartialEq)]
pub struct Vector4 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub struct Quaternion {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub struct Matrix4x4 {
    pub e: [f32; 16],
}
