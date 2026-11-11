use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use crate::assets::AssetManager;
use crate::classes::mesh::{topology_triangles, Mesh};
use crate::export_common::texture_image;
use crate::extractor::decompress_unity_texture;
use crate::unity_version::UnityVersion;
use crate::value::UnityValue;
use crate::math::{Rect, Vector2};

const SPRITE_ATLAS_CLASS_ID: i32 = 687078895;

pub const PACKING_ROTATE_90: u32 = 4;

type RenderDataKey = ([u32; 4], i64);

type TextureRef = (String, i64);

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
    pub texture: Option<TextureRef>,
    pub alpha_texture: Option<TextureRef>,
    pub texture_rect: Option<Rect>,
    pub texture_rect_offset: Option<Vector2>,
    pub packing_rotation: u32,
    pub tight: bool,
}

pub struct SpriteImage {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

impl SpriteImage {
    fn new(width: usize, height: usize) -> Self {
        Self { width, height, rgba: vec![0; width * height * 4] }
    }

    fn pixel(&self, x: i64, y: i64) -> [u8; 4] {
        if x < 0 || y < 0 || x >= self.width as i64 || y >= self.height as i64 {
            return [0; 4];
        }
        let at = (y as usize * self.width + x as usize) * 4;
        [self.rgba[at], self.rgba[at + 1], self.rgba[at + 2], self.rgba[at + 3]]
    }

    fn set(&mut self, x: i64, y: i64, pixel: [u8; 4]) {
        if x < 0 || y < 0 || x >= self.width as i64 || y >= self.height as i64 {
            return;
        }
        let at = (y as usize * self.width + x as usize) * 4;
        self.rgba[at..at + 4].copy_from_slice(&pixel);
    }

    fn remap(&self, width: usize, height: usize, source: impl Fn(usize, usize) -> (usize, usize)) -> Self {
        let mut out = Self::new(width, height);
        for y in 0..height {
            for x in 0..width {
                let (sx, sy) = source(x, y);
                out.set(x as i64, y as i64, self.pixel(sx as i64, sy as i64));
            }
        }
        out
    }
}

struct SpriteMesh {
    vertices: Vec<[f64; 3]>,
    uvs: Option<Vec<[f64; 2]>>,
    submeshes: Vec<Vec<[usize; 3]>>,
}

impl SpriteMesh {
    fn read(render_data: &UnityValue, version: &UnityVersion) -> Option<Self> {
        let mesh = Mesh::from_render_data(render_data, version).ok()?;
        let vertices: Vec<[f64; 3]> = mesh.get_vertices().ok()?.iter().map(|v| [v.x as f64, v.y as f64, v.z as f64]).collect();
        if vertices.is_empty() {
            return None;
        }
        let uvs = mesh.extract_uvs().ok().map(|uvs| uvs.iter().map(|uv| [uv.0 as f64, uv.1 as f64]).collect());
        let indices = mesh.get_indices().ok()?;
        let Some(UnityValue::Array(submeshes)) = render_data.get("m_SubMeshes") else { return None };
        let submeshes = submeshes.iter().map(|s| submesh_triangles(s, &indices, vertices.len())).collect();
        Some(Self { vertices, uvs, submeshes })
    }

    fn textured_uvs(&self) -> Option<&Vec<[f64; 2]>> {
        self.uvs.as_ref().filter(|uvs| uvs.len() == self.vertices.len() && uvs.iter().any(|uv| uv[0] != 0.0 || uv[1] != 0.0))
    }
}

struct LoadedAtlas {
    file: String,
    name: String,
    path_id: i64,
    entries: HashMap<RenderDataKey, RenderData>,
}

pub struct SpriteAtlases<'a> {
    asset_manager: &'a AssetManager,
    atlases: Vec<LoadedAtlas>,
    textures: HashMap<(TextureRef, Option<TextureRef>), SpriteImage>,
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
                let name = value.get("m_Name").and_then(|v| v.as_str()).unwrap_or_default().to_string();
                atlases.push(LoadedAtlas { file: file.clone(), name, path_id: obj.path_id, entries });
            }
        }
        Self { asset_manager, atlases, textures: HashMap::new() }
    }

    pub fn render_data(&self, file: &str, sprite: &UnityValue) -> Option<RenderData> {
        let own = sprite.get("m_RD").map(|rd| read_render_data(self.asset_manager, file, rd));
        let Some(key) = sprite.get("m_RenderDataKey").and_then(render_data_key) else { return own };
        let entry = self.linked_atlas(file, sprite).and_then(|a| a.entries.get(&key));
        entry.filter(|e| e.texture.is_some()).cloned().or(own)
    }

    pub fn image(&mut self, file: &str, sprite: &UnityValue) -> Option<SpriteImage> {
        let data = self.render_data(file, sprite)?;
        let key = (data.texture.clone()?, data.alpha_texture.clone());
        if !self.textures.contains_key(&key) {
            let decoded = decode_source(self.asset_manager, &key.0, key.1.as_ref())?;
            self.textures.insert(key.clone(), decoded);
        }
        let source = self.textures.get(&key)?;
        let cut = apply_packing_rotation(crop(source, data.texture_rect?), data.packing_rotation);
        let version = self.asset_manager.files.get(file)?.unity_version;
        let mesh = if data.tight { sprite.get("m_RD").and_then(|rd| SpriteMesh::read(rd, &version)) } else { None };
        let pixels_to_units = float(sprite, "m_PixelsToUnits") as f64;
        let image = match mesh {
            Some(mesh) if mesh.textured_uvs().is_some() => render_mesh(&mesh, pixels_to_units, source).unwrap_or(cut),
            Some(mesh) => mask_image(&mesh, pixels_to_units, &cut),
            None => cut,
        };
        if image.width == 0 || image.height == 0 {
            return None;
        }
        let (width, height) = (image.width, image.height);
        Some(image.remap(width, height, |x, y| (x, height - 1 - y)))
    }

    fn linked_atlas(&self, file: &str, sprite: &UnityValue) -> Option<&LoadedAtlas> {
        if let Some(UnityValue::PPtr { file_id, path_id }) = sprite.get("m_SpriteAtlas") {
            if *path_id != 0 {
                let (name, _) = self.asset_manager.resolve_pptr(file, *file_id, *path_id)?;
                return self.atlases.iter().find(|a| a.file == name && a.path_id == *path_id);
            }
        }
        let Some(UnityValue::Array(tags)) = sprite.get("m_AtlasTags") else { return None };
        let tag = tags.first()?.as_str()?;
        self.atlases.iter().find(|a| a.file == file && a.name == tag)
    }
}

fn read_rect(value: &UnityValue) -> Rect {
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
    let texture_ref = |key: &str| match data.get(key) {
        Some(UnityValue::PPtr { file_id, path_id }) if *path_id != 0 => asset_manager.resolve_pptr(file, *file_id, *path_id).map(|(name, _)| (name.to_string(), *path_id)),
        _ => None,
    };
    let settings = data.get("settingsRaw").and_then(|v| v.as_i64()).unwrap_or(0) as u32;
    RenderData {
        texture: texture_ref("texture"),
        alpha_texture: texture_ref("alphaTexture"),
        texture_rect: data.get("textureRect").map(read_rect),
        texture_rect_offset: data.get("textureRectOffset").map(read_vector2),
        packing_rotation: if settings & 1 != 0 { (settings >> 2) & 0xF } else { 0 },
        tight: (settings >> 1) & 1 == 0,
    }
}

fn decode_texture(asset_manager: &AssetManager, texture: &TextureRef) -> Option<SpriteImage> {
    let sf = asset_manager.files.get(&texture.0)?;
    let value = asset_manager.read_object_value(&texture.0, 0, texture.1).ok()?;
    let width = value.get("m_Width")?.as_i32()? as usize;
    let height = value.get("m_Height")?.as_i32()? as usize;
    let data = texture_image(&value, asset_manager);
    if width == 0 || height == 0 || data.is_empty() {
        return None;
    }
    let rgba = decompress_unity_texture(&value, &data, &sf.unity_version, sf.target_platform, false)?;
    Some(SpriteImage { width, height, rgba })
}

fn decode_source(asset_manager: &AssetManager, texture: &TextureRef, alpha: Option<&TextureRef>) -> Option<SpriteImage> {
    let mut image = decode_texture(asset_manager, texture)?;
    if let Some(alpha) = alpha.and_then(|a| decode_texture(asset_manager, a)) {
        if alpha.width == image.width && alpha.height == image.height {
            for (pixel, mask) in image.rgba.chunks_exact_mut(4).zip(alpha.rgba.chunks_exact(4)) {
                pixel[3] = mask[0];
            }
        }
    }
    Some(image)
}

fn crop(source: &SpriteImage, rect: Rect) -> SpriteImage {
    let (left, top) = (rect.x as f64, rect.y as f64);
    let x0 = left.round_ties_even() as i64;
    let y0 = top.round_ties_even() as i64;
    let x1 = (left + rect.w as f64).round_ties_even() as i64;
    let y1 = (top + rect.h as f64).round_ties_even() as i64;
    let mut out = SpriteImage::new((x1 - x0).max(0) as usize, (y1 - y0).max(0) as usize);
    for y in 0..out.height as i64 {
        for x in 0..out.width as i64 {
            out.set(x, y, source.pixel(x0 + x, y0 + y));
        }
    }
    out
}

fn apply_packing_rotation(image: SpriteImage, rotation: u32) -> SpriteImage {
    let (width, height) = (image.width, image.height);
    match rotation {
        1 => image.remap(width, height, |x, y| (width - 1 - x, y)),
        2 => image.remap(width, height, |x, y| (x, height - 1 - y)),
        3 => image.remap(width, height, |x, y| (width - 1 - x, height - 1 - y)),
        PACKING_ROTATE_90 => image.remap(height, width, |x, y| (y, height - 1 - x)),
        _ => image,
    }
}

fn submesh_triangles(submesh: &UnityValue, indices: &[u32], vertex_count: usize) -> Vec<[usize; 3]> {
    let field = |key: &str| submesh.get(key).and_then(|v| v.as_i64()).unwrap_or(0) as usize;
    let first = field("firstByte") / 2;
    let end = (first + field("indexCount")).min(indices.len());
    let Some(range) = indices.get(first..end) else { return Vec::new() };
    topology_triangles(range, field("topology") as u32).into_iter().filter(|t| t.iter().all(|&i| (i as usize) < vertex_count)).map(|t| t.map(|i| i as usize)).collect()
}

fn fill_triangle(triangle: [(i64, i64); 3], width: i64, height: i64, mut plot: impl FnMut(i64, i64)) {
    let top = triangle.iter().map(|p| p.1).min().unwrap_or(0).max(0);
    let bottom = triangle.iter().map(|p| p.1).max().unwrap_or(-1).min(height - 1);
    for y in top..=bottom {
        let (mut low, mut high) = (f64::MAX, f64::MIN);
        for i in 0..3 {
            let (x0, y0) = triangle[i];
            let (x1, y1) = triangle[(i + 1) % 3];
            if y < y0.min(y1) || y > y0.max(y1) {
                continue;
            }
            let (a, b) = if y0 == y1 {
                (x0 as f64, x1 as f64)
            } else {
                let x = x0 as f64 + (y - y0) as f64 * (x1 - x0) as f64 / (y1 - y0) as f64;
                (x, x)
            };
            low = low.min(a).min(b);
            high = high.max(a).max(b);
        }
        if low > high {
            continue;
        }
        let start = ((low + 0.5).floor() as i64).max(0);
        let end = ((high - 0.5).ceil() as i64).min(width - 1);
        for x in start..=end {
            plot(x, y);
        }
    }
}

fn mask_image(mesh: &SpriteMesh, pixels_to_units: f64, image: &SpriteImage) -> SpriteImage {
    let min_x = mesh.vertices.iter().map(|v| v[0]).fold(f64::MAX, f64::min);
    let min_y = mesh.vertices.iter().map(|v| v[1]).fold(f64::MAX, f64::min);
    let point = |i: usize| (((mesh.vertices[i][0] - min_x) * pixels_to_units) as i64, ((mesh.vertices[i][1] - min_y) * pixels_to_units) as i64);
    let mut out = SpriteImage::new(image.width, image.height);
    for triangle in mesh.submeshes.iter().flatten() {
        fill_triangle(triangle.map(point), image.width as i64, image.height as i64, |x, y| out.set(x, y, image.pixel(x, y)));
    }
    out
}

fn render_mesh(mesh: &SpriteMesh, pixels_to_units: f64, texture: &SpriteImage) -> Option<SpriteImage> {
    let triangles = mesh.submeshes.first()?;
    let uvs = mesh.textured_uvs()?;
    let flat_axis = (0..3).rev().find(|&axis| mesh.vertices.iter().all(|v| v[axis] == mesh.vertices[0][axis]))?;
    let axes: Vec<usize> = (0..3).filter(|&axis| axis != flat_axis).collect();
    let extent = |axis: usize| {
        let values = mesh.vertices.iter().map(|v| v[axis]);
        (values.clone().fold(f64::MAX, f64::min), values.fold(f64::MIN, f64::max))
    };
    let ((x_min, x_max), (y_min, y_max)) = (extent(axes[0]), extent(axes[1]));
    let rounded = |value: f64| value.round_ties_even() as i64;
    let positions: Vec<(i64, i64)> = mesh.vertices.iter().map(|v| (rounded((v[axes[0]] - x_min) * pixels_to_units), rounded((v[axes[1]] - y_min) * pixels_to_units))).collect();
    let texels: Vec<(i64, i64)> = uvs.iter().map(|uv| (rounded(uv[0] * texture.width as f64), rounded(uv[1] * texture.height as f64))).collect();
    let mut out = SpriteImage::new(rounded((x_max - x_min) * pixels_to_units) as usize, rounded((y_max - y_min) * pixels_to_units) as usize);
    for triangle in triangles {
        copy_triangle(texture, triangle.map(|i| texels[i]), &mut out, triangle.map(|i| positions[i]));
    }
    Some(out)
}

fn copy_triangle(source: &SpriteImage, from: [(i64, i64); 3], target: &mut SpriteImage, to: [(i64, i64); 3]) {
    let offsets = |t: [(i64, i64); 3]| [(t[1].0 - t[0].0, t[1].1 - t[0].1), (t[2].0 - t[0].0, t[2].1 - t[0].1)];
    if offsets(from) == offsets(to) {
        let corner = |t: [(i64, i64); 3]| (t.iter().map(|p| p.0).min().unwrap_or(0), t.iter().map(|p| p.1).min().unwrap_or(0));
        let (from_x, from_y) = corner(from);
        let (to_x, to_y) = corner(to);
        let span_x = from.iter().map(|p| p.0).max().unwrap_or(0) - from_x;
        let span_y = from.iter().map(|p| p.1).max().unwrap_or(0) - from_y;
        fill_triangle(from.map(|p| (p.0 - from_x, p.1 - from_y)), span_x, span_y, |x, y| target.set(to_x + x, to_y + y, source.pixel(from_x + x, from_y + y)));
        return;
    }
    let det3 = |a: [f64; 3], b: [f64; 3], c: [f64; 3]| a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0]) + a[2] * (b[0] * c[1] - b[1] * c[0]);
    let xs = to.map(|p| p.0 as f64);
    let ys = to.map(|p| p.1 as f64);
    let ones = [1.0; 3];
    let det = det3(xs, ys, ones);
    if det == 0.0 {
        return;
    }
    let solve = |values: [f64; 3]| [det3(values, ys, ones) / det, det3(xs, values, ones) / det, det3(xs, ys, values) / det];
    let u = solve(from.map(|p| p.0 as f64));
    let v = solve(from.map(|p| p.1 as f64));
    let (width, height) = (target.width as i64, target.height as i64);
    fill_triangle(to, width, height, |x, y| {
        let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
        let sx = (u[0] * px + u[1] * py + u[2]).floor() as i64;
        let sy = (v[0] * px + v[1] * py + v[2]).floor() as i64;
        target.set(x, y, source.pixel(sx, sy));
    });
}
