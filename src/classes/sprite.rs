use serde::{Deserialize, Serialize};
use crate::value::UnityValue;
use crate::math::{Rect, Vector2};
#[allow(non_snake_case)]
#[derive(Debug, Serialize, Deserialize)]
pub struct Sprite {
    pub m_Name: String,
    pub m_Rect: Rect,
    pub m_Offset: Vector2,
    pub m_PixelsToUnits: f32,
    pub m_PhysicsShape: Option<Vec<Vec<Vector2>>>,
}
impl Sprite {
    pub fn from_value(value: UnityValue) -> Result<Self, String> {
        let map = match value {
            UnityValue::Map(m) => m,
            _ => return Err("Value is not a Map (Class/Struct)".to_string()),
        };
        let get_string = |key: &str| -> Result<String, String> {
            map.get(key).and_then(|v| match v {
                UnityValue::String(s) => Some(s.clone()),
                _ => None,
            }).ok_or_else(|| format!("Missing or invalid field: {}", key))
        };
        let get_f32 = |key: &str| -> Result<f32, String> {
            map.get(key).and_then(|v| match v {
                UnityValue::Float(f) => Some(*f),
                _ => None,
            }).ok_or_else(|| format!("Missing or invalid field: {}", key))
        };
        let get_vector2 = |key: &str| -> Result<Vector2, String> {
            let m = map.get(key).and_then(|v| match v {
                UnityValue::Map(m) => Some(m),
                _ => None,
            }).ok_or_else(|| format!("Missing or invalid field: {}", key))?;
            let x = m.get("x").and_then(|v| match v { UnityValue::Float(f) => Some(*f), _ => None }).unwrap_or(0.0);
            let y = m.get("y").and_then(|v| match v { UnityValue::Float(f) => Some(*f), _ => None }).unwrap_or(0.0);
            Ok(Vector2 { x, y })
        };
        let get_rect = |key: &str| -> Result<Rect, String> {
            let m = map.get(key).and_then(|v| match v {
                UnityValue::Map(m) => Some(m),
                _ => None,
            }).ok_or_else(|| format!("Missing or invalid field: {}", key))?;
            let x = m.get("x").and_then(|v| match v { UnityValue::Float(f) => Some(*f), _ => None }).unwrap_or(0.0);
            let y = m.get("y").and_then(|v| match v { UnityValue::Float(f) => Some(*f), _ => None }).unwrap_or(0.0);
            let w = m.get("m_Width").and_then(|v| match v { UnityValue::Float(f) => Some(*f), _ => None }).unwrap_or(0.0);
            let h = m.get("m_Height").and_then(|v| match v { UnityValue::Float(f) => Some(*f), _ => None }).unwrap_or(0.0);
            Ok(Rect { x, y, w, h })
        };
        Ok(Sprite {
            m_Name: get_string("m_Name")?,
            m_Rect: get_rect("m_Rect")?,
            m_Offset: get_vector2("m_Offset")?,
            m_PixelsToUnits: get_f32("m_PixelsToUnits")?,
            m_PhysicsShape: None,
        })
    }
}
