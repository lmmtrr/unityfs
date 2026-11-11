use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use crate::assets::AssetManager;
use crate::value::UnityValue;
use crate::math::{Rect, Vector2};

const SPRITE_ATLAS_CLASS_ID: i32 = 687078895;

pub const PACKING_ROTATE_90: u32 = 4;

type RenderDataKey = ([u32; 4], i64);

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
        let get_map = |key: &str| -> Result<&UnityValue, String> {
            map.get(key).filter(|v| matches!(v, UnityValue::Map(_))).ok_or_else(|| format!("Missing or invalid field: {}", key))
        };
        Ok(Sprite {
            m_Name: get_string("m_Name")?,
            m_Rect: read_rect(get_map("m_Rect")?),
            m_Offset: read_vector2(get_map("m_Offset")?),
            m_PixelsToUnits: get_f32("m_PixelsToUnits")?,
            m_PhysicsShape: None,
        })
    }
}

#[derive(Debug, Clone)]
pub struct RenderData {
    pub texture: Option<(String, i64)>,
    pub texture_rect: Option<Rect>,
    pub texture_rect_offset: Option<Vector2>,
    pub packing_rotation: u32,
}

struct LoadedAtlas {
    file: String,
    path_id: i64,
    entries: HashMap<RenderDataKey, RenderData>,
}

pub struct SpriteAtlases<'a> {
    asset_manager: &'a AssetManager,
    atlases: Vec<LoadedAtlas>,
}

impl<'a> SpriteAtlases<'a> {
    pub fn open(asset_manager: &'a AssetManager) -> Self {
        let mut atlases = Vec::new();
        for (file, sf) in &asset_manager.files {
            for obj in sf.objects.iter().filter(|o| o.class_id == SPRITE_ATLAS_CLASS_ID) {
                let Ok(value) = asset_manager.read_object_value(file, 0, obj.path_id) else { continue };
                let Some(UnityValue::Array(render_data_map)) = value.get("m_RenderDataMap") else { continue };
                let entries = render_data_map
                    .iter()
                    .filter_map(|entry| Some((render_data_key(entry.get("first")?)?, read_render_data(asset_manager, file, entry.get("second")?))))
                    .collect();
                atlases.push(LoadedAtlas { file: file.clone(), path_id: obj.path_id, entries });
            }
        }
        Self { asset_manager, atlases }
    }

    pub fn render_data(&self, file: &str, sprite: &UnityValue) -> Option<RenderData> {
        let own = sprite.get("m_RD").map(|rd| read_render_data(self.asset_manager, file, rd));
        let Some(key) = sprite.get("m_RenderDataKey").and_then(render_data_key) else { return own };
        let linked = match sprite.get("m_SpriteAtlas") {
            Some(UnityValue::PPtr { file_id, path_id }) if *path_id != 0 => self.asset_manager.resolve_pptr(file, *file_id, *path_id).map(|(name, _)| (name, *path_id)),
            _ => None,
        };
        let entry = match linked {
            Some((name, path_id)) => self.atlases.iter().find(|a| a.file == name && a.path_id == path_id).and_then(|a| a.entries.get(&key)),
            None if own.as_ref().is_some_and(|o| o.texture.is_some()) => None,
            None => self.atlases.iter().find_map(|a| a.entries.get(&key)),
        };
        entry.filter(|e| e.texture.is_some()).cloned().or(own)
    }
}

pub fn read_rect(value: &UnityValue) -> Rect {
    Rect { x: float(value, "x"), y: float(value, "y"), w: float(value, "width"), h: float(value, "height") }
}

fn read_vector2(value: &UnityValue) -> Vector2 {
    Vector2 { x: float(value, "x"), y: float(value, "y") }
}

fn float(value: &UnityValue, key: &str) -> f32 {
    match value.get(key) {
        Some(UnityValue::Float(f)) => *f,
        Some(UnityValue::Double(d)) => *d as f32,
        _ => 0.0,
    }
}

fn render_data_key(key: &UnityValue) -> Option<RenderDataKey> {
    let guid = key.get("first")?;
    let mut data = [0u32; 4];
    for (i, slot) in data.iter_mut().enumerate() {
        *slot = guid.get(&format!("data[{}]", i))?.as_i32()? as u32;
    }
    Some((data, key.get("second")?.as_i64()?))
}

fn read_render_data(asset_manager: &AssetManager, file: &str, data: &UnityValue) -> RenderData {
    let texture = match data.get("texture") {
        Some(UnityValue::PPtr { file_id, path_id }) if *path_id != 0 => asset_manager.resolve_pptr(file, *file_id, *path_id).map(|(name, _)| (name.to_string(), *path_id)),
        _ => None,
    };
    let settings = data.get("settingsRaw").and_then(|v| v.as_i64()).unwrap_or(0) as u32;
    RenderData {
        texture,
        texture_rect: data.get("textureRect").map(read_rect),
        texture_rect_offset: data.get("textureRectOffset").map(read_vector2),
        packing_rotation: if settings & 1 != 0 { (settings >> 2) & 0xF } else { 0 },
    }
}
