use std::path::{Path, PathBuf};
use crate::assets::AssetManager;
use crate::classes::TryFromUnityValue;
use crate::classes::mesh::Mesh;
use crate::extractor::decompress_texture;
use crate::value::UnityValue;
use crate::export_common::{self as common, PosePartData};

#[derive(Clone, Debug, Default)]
pub struct ExportOptions {
    pub by_file: bool,
    pub metadata: bool,
}

struct Log;

impl Log {
    fn println(&self, msg: String) {
        eprintln!("{}", msg);
    }
}

pub fn export_bundle_file(file_path: &Path, out_dir: &Path, options: &ExportOptions) -> Result<usize, String> {
    let pb = &Log;
    let file_stem = file_path
        .file_stem()
        .unwrap_or_else(|| std::ffi::OsStr::new("bundle"))
        .to_string_lossy();
    let asset_manager = AssetManager::open_path(file_path).map_err(|e| format!("{}: {}", file_path.display(), e))?;
    let bundle_output_dir = if options.by_file {
        out_dir.join(file_stem.as_ref())
    } else {
        out_dir.to_path_buf()
    };
    let flat = true;
    let pose_parts = std::sync::Mutex::new(Vec::new());
    let moc_stem = std::sync::Mutex::new(None);
    let mut count = 0;
    let mut sprites = crate::classes::sprite::SpriteAtlases::open(&asset_manager);
    for (asset_name, sf) in &asset_manager.files {
        let mut objects: Vec<_> = sf.objects.iter().collect();
        objects.sort_by_key(|o| o.class_id == 213);
        for obj in objects {
            let class_id = obj.class_id;
            let supported = match class_id {
                28 | 49 | 43 | 83 | 48 | 128 | 152 | 329 | 114 | 213 => true,
                1 | 4 | 21 | 74 | 115 => options.metadata,
                _ => false,
            };
            if !supported {
                continue;
            }
            let path_id = obj.path_id;
            let t_name = obj.type_name();
            let unity_value = match asset_manager.read_object_value(asset_name, 0, path_id) {
                Ok(v) => v,
                Err(e) => {
                    pb.println(format!("[{}]     Warning: Failed to read object value for path_id {}: {}", file_stem, path_id, e));
                    continue;
                }
            };
            let success = match class_id {
                28 => extract_texture2d(&unity_value, &sf.unity_version, &bundle_output_dir, &asset_manager, flat, pb),
                49 => extract_text_asset(&unity_value, &bundle_output_dir, flat, pb),
                213 => extract_sprite(&unity_value, asset_name, &mut sprites, &bundle_output_dir, pb),
                43 => extract_mesh(&unity_value, &sf.unity_version, &bundle_output_dir, flat, pb),
                83 => extract_audioclip(&unity_value, &bundle_output_dir, &asset_manager, asset_name, flat, pb),
                128 => extract_font(&unity_value, &bundle_output_dir, flat, pb),
                48 => extract_shader(&unity_value, &bundle_output_dir, flat, pb),
                329 => extract_videoclip(&unity_value, &bundle_output_dir, &asset_manager, "VideoClip", flat, pb),
                152 => extract_videoclip(&unity_value, &bundle_output_dir, &asset_manager, "MovieTexture", flat, pb),
                114 => {
                    let success = extract_monobehaviour(
                        &unity_value,
                        &bundle_output_dir,
                        &asset_manager,
                        asset_name,
                        obj,
                        flat,
                        pb,
                        &pose_parts,
                        &moc_stem,
                    );
                    if options.metadata {
                        dump_asset_as_json(class_id, t_name, &unity_value, &bundle_output_dir, flat, path_id);
                    }
                    success
                }
                _ => dump_asset_as_json(class_id, t_name, &unity_value, &bundle_output_dir, flat, path_id),
            };
            if success {
                count += 1;
            }
        }
    }
    let pose_parts_vec = pose_parts.into_inner().unwrap();
    if !pose_parts_vec.is_empty() {
        let stem = moc_stem.into_inner().unwrap().unwrap_or_else(|| file_stem.to_string());
        if let Some(serialized) = common::pose_json(pose_parts_vec) {
            let _ = std::fs::create_dir_all(&bundle_output_dir);
            if let UniquePathResult::New(dest) = get_unique_path(&bundle_output_dir, &format!("{}.pose3.json", stem), Some(&serialized)) {
                if let Err(e) = std::fs::write(&dest, &serialized) {
                    pb.println(format!("    Failed to write pose asset '{}': {}", dest.display(), e));
                }
            }
        }
    }
    Ok(count)
}

#[derive(Debug)]
enum UniquePathResult {
    New(PathBuf),
    Exists(PathBuf),
}

fn get_unique_path(dir: &Path, filename: &str, data: Option<&[u8]>) -> UniquePathResult {
    let base_path = dir.join(filename);
    if !base_path.exists() {
        return UniquePathResult::New(base_path);
    }
    if let Some(bytes) = data {
        if let Ok(metadata) = std::fs::metadata(&base_path) {
            if metadata.len() == bytes.len() as u64 {
                return UniquePathResult::Exists(base_path);
            }
        }
    }
    let stem = base_path.file_stem().unwrap_or_default().to_string_lossy();
    let extension = base_path.extension().unwrap_or_default().to_string_lossy();
    let mut counter = 2;
    loop {
        let new_filename = if extension.is_empty() {
            format!("{} ({})", stem, counter)
        } else {
            format!("{} ({}).{}", stem, counter, extension)
        };
        let new_path = dir.join(new_filename);
        if !new_path.exists() {
            return UniquePathResult::New(new_path);
        }
        if let Some(bytes) = data {
            if let Ok(metadata) = std::fs::metadata(&new_path) {
                if metadata.len() == bytes.len() as u64 {
                    return UniquePathResult::Exists(new_path);
                }
            }
        }
        counter += 1;
    }
}

fn write_unique(dir: &Path, filename: &str, data: &[u8], what: &str, pb: &Log) -> bool {
    let _ = std::fs::create_dir_all(dir);
    match get_unique_path(dir, filename, Some(data)) {
        UniquePathResult::New(dest) => match std::fs::write(&dest, data) {
            Ok(()) => true,
            Err(e) => {
                pb.println(format!("    Failed to write {} '{}': {}", what, dest.display(), e));
                false
            }
        },
        UniquePathResult::Exists(_) => true,
    }
}

fn extract_monobehaviour(
    val: &UnityValue,
    output_dir: &Path,
    asset_manager: &AssetManager,
    asset_name: &str,
    obj: &crate::objectreader::ObjectReader,
    by_file: bool,
    pb: &Log,
    pose_parts: &std::sync::Mutex<Vec<PosePartData>>,
    moc_stem: &std::sync::Mutex<Option<String>>,
) -> bool {
    let name = match val.get("m_Name") {
        Some(UnityValue::String(s)) if !s.is_empty() => s.clone(),
        _ => "".to_string(),
    };
    let base_name = if !name.is_empty() {
        Path::new(&name)
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or(name)
    } else {
        format!("monobehaviour_{}", obj.path_id)
    };
    let sanitized_base = base_name.replace(|c: char| !c.is_alphanumeric() && c != '_' && c != '-' && c != '.', "");
    if sanitized_base.is_empty() {
        return false;
    }
    let mut blobs = Vec::new();
    common::criware_blobs(val, "", &mut blobs);
    if !blobs.is_empty() {
        let cri_dir = if by_file { output_dir.to_path_buf() } else { output_dir.join("CriWare") };
        return blobs.iter().fold(false, |ok, (key, ext, data)| write_unique(&cri_dir, &common::criware_filename(&sanitized_base, key, ext), data, "CriWare asset", pb) || ok);
    }
    if common::resolve_class_name(val, asset_manager, asset_name).as_deref() == Some("CubismPosePart") {
        if let Some(part) = common::pose_part(val, asset_manager, asset_name) {
            pose_parts.lock().unwrap().push(part);
        }
        return true;
    }
    let Some((data, kind)) = common::monobehaviour_content(val, asset_manager, asset_name, obj) else { return false };
    let final_name = common::content_filename(&sanitized_base, &data, kind);
    let final_data = common::finalize_content(&final_name, data);
    if final_name.ends_with(".moc3") {
        *moc_stem.lock().unwrap() = Some(common::moc_stem(&sanitized_base));
    }
    let monobehaviour_dir = if by_file { output_dir.to_path_buf() } else { output_dir.join("MonoBehaviour") };
    write_unique(&monobehaviour_dir, &final_name, &final_data, "MonoBehaviour asset", pb)
}

fn extract_text_asset(val: &UnityValue, output_dir: &Path, by_file: bool, pb: &Log) -> bool {
    let name = match val.get("m_Name") {
        Some(UnityValue::String(s)) if !s.is_empty() => s.clone(),
        _ => "text_asset".to_string(),
    };
    let content = match val.get("m_Script") {
        Some(UnityValue::String(s)) => Some(s.as_bytes().to_vec()),
        Some(UnityValue::Bytes(b)) => Some(b.clone()),
        _ => None,
    };
    if let Some(data) = content {
        let is_moc3 = data.starts_with(b"MOC3");
        let name_lower = name.to_lowercase();
        let filename = if is_moc3 {
            let mut stem = name.clone();
            if stem.to_lowercase().ends_with(".moc3") {
                stem = stem[..stem.len() - 5].to_string();
            } else if stem.to_lowercase().ends_with(".moc") {
                stem = stem[..stem.len() - 4].to_string();
            } else if stem.to_lowercase().ends_with("_moc3") {
                stem = stem[..stem.len() - 5].to_string();
            } else if stem.to_lowercase().ends_with("_moc") {
                stem = stem[..stem.len() - 4].to_string();
            }
            format!("{}.moc3", stem)
        } else if common::is_json_bytes(&data) {
            let mut base = name.clone();
            if base.to_lowercase().ends_with(".json") {
                base = base[..base.len() - 5].to_string();
            }
            if name_lower.contains("physics") {
                if base.to_lowercase().ends_with(".physics3") || base.to_lowercase().ends_with("_physics3") {
                    let stem = base[..base.len() - 9].to_string();
                    format!("{}.physics3.json", stem)
                } else if base.to_lowercase().ends_with(".physics") || base.to_lowercase().ends_with("_physics") {
                    let stem = base[..base.len() - 8].to_string();
                    format!("{}.physics.json", stem)
                } else {
                    format!("{}.physics3.json", base)
                }
            } else if name_lower.contains("motion") {
                if base.to_lowercase().ends_with(".motion3") || base.to_lowercase().ends_with("_motion3") {
                    let stem = base[..base.len() - 8].to_string();
                    format!("{}.motion3.json", stem)
                } else if base.to_lowercase().ends_with(".motion") || base.to_lowercase().ends_with("_motion") {
                    let stem = base[..base.len() - 7].to_string();
                    format!("{}.motion.json", stem)
                } else {
                    format!("{}.motion3.json", base)
                }
            } else if name_lower.contains("pose") {
                if base.to_lowercase().ends_with(".pose3") || base.to_lowercase().ends_with("_pose3") {
                    let stem = base[..base.len() - 6].to_string();
                    format!("{}.pose3.json", stem)
                } else if base.to_lowercase().ends_with(".pose") || base.to_lowercase().ends_with("_pose") {
                    let stem = base[..base.len() - 5].to_string();
                    format!("{}.pose.json", stem)
                } else {
                    format!("{}.pose.json", base)
                }
            } else if name_lower.contains("cdi") {
                if base.to_lowercase().ends_with(".cdi3") || base.to_lowercase().ends_with("_cdi3") {
                    let stem = base[..base.len() - 5].to_string();
                    format!("{}.cdi3.json", stem)
                } else {
                    format!("{}.cdi3.json", base)
                }
            } else if name_lower.contains("userdata") {
                if base.to_lowercase().ends_with(".userdata3") || base.to_lowercase().ends_with("_userdata3") {
                    let stem = base[..base.len() - 10].to_string();
                    format!("{}.userdata3.json", stem)
                } else {
                    format!("{}.userdata3.json", base)
                }
            } else if name_lower.contains("exp") {
                if base.to_lowercase().ends_with(".exp3") || base.to_lowercase().ends_with("_exp3") {
                    let stem = base[..base.len() - 5].to_string();
                    format!("{}.exp3.json", stem)
                } else if base.to_lowercase().ends_with(".exp") || base.to_lowercase().ends_with("_exp") {
                    let stem = base[..base.len() - 4].to_string();
                    format!("{}.exp.json", stem)
                } else {
                    format!("{}.exp3.json", base)
                }
            } else {
                format!("{}.json", base)
            }
        } else {
            let safe_name = name.replace(|c: char| !c.is_alphanumeric() && c != '_' && c != '-' && c != '.', "");
            if safe_name.is_empty() {
                "text_asset.txt".to_string()
            } else if safe_name.contains('.') {
                safe_name
            } else {
                if common::is_spine_binary(&data) {
                    format!("{}.skel", safe_name)
                } else {
                    format!("{}.txt", safe_name)
                }
            }
        };
        let text_dir = if by_file {
            output_dir.to_path_buf()
        } else {
            output_dir.join("TextAsset")
        };
        let _ = std::fs::create_dir_all(&text_dir);
        match get_unique_path(&text_dir, &filename, Some(&data)) {
            UniquePathResult::New(dest) => {
                if let Err(e) = std::fs::write(&dest, &data) {
                    pb.println(format!("    Failed to write text asset '{}': {}", dest.display(), e));
                    false
                } else {
                    true
                }
            }
            UniquePathResult::Exists(_) => true,
        }
    } else {
        false
    }
}

fn extract_texture2d(
    val: &UnityValue,
    version: &crate::unity_version::UnityVersion,
    output_dir: &Path,
    asset_manager: &AssetManager,
    by_file: bool,
    pb: &Log,
) -> bool {
    let name = match val.get("m_Name") {
        Some(UnityValue::String(s)) if !s.is_empty() => s.clone(),
        _ => "texture".to_string(),
    };
    let width = match val.get("m_Width") {
        Some(v) => i32::try_from_unity_value(v).unwrap_or(0) as usize,
        _ => 0,
    };
    let height = match val.get("m_Height") {
        Some(v) => i32::try_from_unity_value(v).unwrap_or(0) as usize,
        _ => 0,
    };
    let format = match val.get("m_TextureFormat") {
        Some(v) => i32::try_from_unity_value(v).unwrap_or(0),
        _ => 0,
    };
    if width == 0 || height == 0 {
        return false;
    }
    let image_data = common::texture_image(val, asset_manager);
    if image_data.is_empty() {
        return false;
    }
    if let Some(rgba_data) = decompress_texture(width, height, format, &image_data, version, true) {
        let safe_name = name.replace(|c: char| !c.is_alphanumeric() && c != '_' && c != '-', "");
        let filename = if safe_name.is_empty() {
            "texture.png".to_string()
        } else {
            format!("{}.png", safe_name)
        };
        let texture_dir = if by_file {
            output_dir.to_path_buf()
        } else {
            output_dir.join("Texture2D")
        };
        let _ = std::fs::create_dir_all(&texture_dir);
        let dest = match get_unique_path(&texture_dir, &filename, None) {
            UniquePathResult::New(dest) | UniquePathResult::Exists(dest) => dest,
        };
        if let Err(e) = image::save_buffer(
            &dest,
            &rgba_data,
            width as u32,
            height as u32,
            image::ExtendedColorType::Rgba8,
        ) {
            pb.println(format!("    Failed to save PNG texture '{}': {}", dest.display(), e));
            false
        } else {
            true
        }
    } else {
        pb.println(format!("    Failed to decompress texture '{}' (format={})", name, format));
        false
    }
}

fn extract_sprite(val: &UnityValue, asset_name: &str, sprites: &mut crate::classes::sprite::SpriteAtlases, output_dir: &Path, pb: &Log) -> bool {
    let name = match val.get("m_Name") {
        Some(UnityValue::String(s)) if !s.is_empty() => s.clone(),
        _ => "sprite".to_string(),
    };
    let Some(image) = sprites.image(asset_name, val) else {
        pb.println(format!("    Failed to cut sprite '{}'", name));
        return false;
    };
    let mut png = Vec::new();
    let encoded = image::ImageEncoder::write_image(image::codecs::png::PngEncoder::new(&mut png), &image.rgba, image.width as u32, image.height as u32, image::ExtendedColorType::Rgba8);
    if let Err(e) = encoded {
        pb.println(format!("    Failed to encode sprite '{}': {}", name, e));
        return false;
    }
    let safe_name = name.replace(|c: char| !c.is_alphanumeric() && c != '_' && c != '-', "");
    let filename = if safe_name.is_empty() { "sprite.png".to_string() } else { format!("{}.png", safe_name) };
    write_unique(output_dir, &filename, &png, "sprite", pb)
}

fn extract_mesh(val: &UnityValue, version: &crate::unity_version::UnityVersion, output_dir: &Path, by_file: bool, pb: &Log) -> bool {
    let name = match val.get("m_Name") {
        Some(UnityValue::String(s)) if !s.is_empty() => s.clone(),
        _ => "mesh".to_string(),
    };
    if let Ok(mesh) = Mesh::read(val, version) {
        let vertices = mesh.get_vertices().unwrap_or_default();
        let indices = mesh.get_indices().unwrap_or_default();
        let uvs = mesh.extract_uvs().unwrap_or_default();
        let normals = mesh.get_normals().unwrap_or_default();
        if vertices.is_empty() {
            return false;
        }
        let mut obj_content = String::new();
        obj_content.push_str(&format!("# Mesh Exporter: {}\n", name));
        obj_content.push_str(&format!("o {}\n\n", name));
        for v in &vertices {
            obj_content.push_str(&format!("v {} {} {}\n", v.x, v.y, v.z));
        }
        obj_content.push_str("\n");
        for uv in &uvs {
            obj_content.push_str(&format!("vt {} {}\n", uv.0, uv.1));
        }
        obj_content.push_str("\n");
        for n in &normals {
            obj_content.push_str(&format!("vn {} {} {}\n", n.x, n.y, n.z));
        }
        obj_content.push_str("\n");
        let has_uv = !uvs.is_empty();
        let has_normal = !normals.is_empty();
        for chunk in indices.chunks_exact(3) {
            let i1 = chunk[0] + 1;
            let i2 = chunk[1] + 1;
            let i3 = chunk[2] + 1;
            match (has_uv, has_normal) {
                (true, true) => {
                    obj_content.push_str(&format!("f {}/{}/{} {}/{}/{} {}/{}/{}\n", i1, i1, i1, i2, i2, i2, i3, i3, i3));
                }
                (true, false) => {
                    obj_content.push_str(&format!("f {}/{} {}/{} {}/{}\n", i1, i1, i2, i2, i3, i3));
                }
                (false, true) => {
                    obj_content.push_str(&format!("f {}//{} {}//{} {}//{}\n", i1, i1, i2, i2, i3, i3));
                }
                (false, false) => {
                    obj_content.push_str(&format!("f {} {} {}\n", i1, i2, i3));
                }
            }
        }
        let safe_name = name.replace(|c: char| !c.is_alphanumeric() && c != '_' && c != '-', "");
        let filename = if safe_name.is_empty() {
            "mesh.obj".to_string()
        } else {
            format!("{}.obj", safe_name)
        };
        let mesh_dir = if by_file {
            output_dir.to_path_buf()
        } else {
            output_dir.join("Mesh")
        };
        let _ = std::fs::create_dir_all(&mesh_dir);
        match get_unique_path(&mesh_dir, &filename, Some(obj_content.as_bytes())) {
            UniquePathResult::New(dest) => {
                if let Err(e) = std::fs::write(&dest, obj_content) {
                    pb.println(format!("    Failed to write Mesh OBJ '{}': {}", dest.display(), e));
                    false
                } else {
                    true
                }
            }
            UniquePathResult::Exists(_) => true,
        }
    } else {
        false
    }
}

fn extract_audioclip(
    val: &UnityValue,
    output_dir: &Path,
    asset_manager: &AssetManager,
    file_name: &str,
    by_file: bool,
    pb: &Log,
) -> bool {
    let name = match val.get("m_Name") {
        Some(UnityValue::String(s)) if !s.is_empty() => s.clone(),
        _ => "audio".to_string(),
    };
    let audio_data = crate::classes::audioclip::playable(crate::classes::audioclip::audio_bytes(val, asset_manager, file_name));
    if audio_data.is_empty() {
        return false;
    }
    let extension = crate::classes::audioclip::extension(&audio_data);
    let safe_name = name.replace(|c: char| !c.is_alphanumeric() && c != '_' && c != '-', "");
    let filename = if safe_name.is_empty() {
        format!("audio.{}", extension)
    } else {
        format!("{}.{}", safe_name, extension)
    };
    let audio_dir = if by_file {
        output_dir.to_path_buf()
    } else {
        output_dir.join("AudioClip")
    };
    let _ = std::fs::create_dir_all(&audio_dir);
    match get_unique_path(&audio_dir, &filename, Some(&audio_data)) {
        UniquePathResult::New(dest) => {
            if let Err(e) = std::fs::write(&dest, &audio_data) {
                pb.println(format!("    Failed to write AudioClip '{}': {}", dest.display(), e));
                false
            } else {
                true
            }
        }
        UniquePathResult::Exists(_) => true,
    }
}

fn extract_font(val: &UnityValue, output_dir: &Path, by_file: bool, pb: &Log) -> bool {
    let Some(data) = crate::classes::font::font_data(val) else { return false };
    let name = match val.get("m_Name") {
        Some(UnityValue::String(s)) if !s.is_empty() => s.clone(),
        _ => "font".to_string(),
    };
    let safe_name = name.replace(|c: char| !c.is_alphanumeric() && c != '_' && c != '-', "");
    let filename = format!("{}.{}", if safe_name.is_empty() { "font" } else { &safe_name }, crate::classes::font::extension(data));
    let font_dir = if by_file { output_dir.to_path_buf() } else { output_dir.join("Font") };
    let _ = std::fs::create_dir_all(&font_dir);
    match get_unique_path(&font_dir, &filename, Some(data)) {
        UniquePathResult::New(dest) => match std::fs::write(&dest, data) {
            Ok(()) => true,
            Err(e) => {
                pb.println(format!("    Failed to write Font '{}': {}", dest.display(), e));
                false
            }
        },
        UniquePathResult::Exists(_) => true,
    }
}

fn extract_videoclip(
    val: &UnityValue,
    output_dir: &Path,
    asset_manager: &AssetManager,
    folder: &str,
    by_file: bool,
    pb: &Log,
) -> bool {
    let video_data = crate::classes::videoclip::video_bytes(val, asset_manager);
    if video_data.is_empty() {
        return false;
    }
    let ext = crate::classes::videoclip::extension(&video_data, val);
    let name = val.get("m_Name").and_then(UnityValue::as_str).unwrap_or("");
    let dot_ext = format!(".{}", ext);
    let sanitized_base = name.replace(|c: char| !c.is_alphanumeric() && c != '_' && c != '-', "");
    let filename = if sanitized_base.is_empty() {
        format!("video{}", dot_ext)
    } else if sanitized_base.ends_with(&dot_ext) {
        sanitized_base
    } else {
        format!("{}{}", sanitized_base, dot_ext)
    };
    let video_dir = if by_file { output_dir.to_path_buf() } else { output_dir.join(folder) };
    let _ = std::fs::create_dir_all(&video_dir);
    match get_unique_path(&video_dir, &filename, Some(&video_data)) {
        UniquePathResult::New(dest) => match std::fs::write(&dest, &video_data) {
            Ok(()) => true,
            Err(e) => {
                pb.println(format!("    Failed to write video '{}': {}", dest.display(), e));
                false
            }
        },
        UniquePathResult::Exists(_) => true,
    }
}

fn extract_shader(val: &UnityValue, output_dir: &Path, by_file: bool, pb: &Log) -> bool {
    let name = match val.get("m_Name") {
        Some(UnityValue::String(s)) if !s.is_empty() => s.clone(),
        _ => "shader".to_string(),
    };
    let content = match val.get("m_Script") {
        Some(UnityValue::String(s)) => Some(s.as_bytes().to_vec()),
        Some(UnityValue::Bytes(b)) => Some(b.clone()),
        _ => None,
    };
    if let Some(data) = content {
        let safe_name = name.replace(|c: char| !c.is_alphanumeric() && c != '_' && c != '-', "");
        let filename = if safe_name.is_empty() {
            "shader.shader".to_string()
        } else {
            format!("{}.shader", safe_name)
        };
        let shader_dir = if by_file {
            output_dir.to_path_buf()
        } else {
            output_dir.join("Shader")
        };
        let _ = std::fs::create_dir_all(&shader_dir);
        match get_unique_path(&shader_dir, &filename, Some(&data)) {
            UniquePathResult::New(dest) => {
                if let Err(e) = std::fs::write(&dest, &data) {
                    pb.println(format!("    Failed to write Shader '{}': {}", dest.display(), e));
                    false
                } else {
                    true
                }
            }
            UniquePathResult::Exists(_) => true,
        }
    } else {
        false
    }
}

fn dump_asset_as_json(
    class_id: i32,
    type_name: &str,
    val: &UnityValue,
    output_dir: &Path,
    by_file: bool,
    path_id: i64,
) -> bool {
    let name = match val.get("m_Name") {
        Some(UnityValue::String(s)) if !s.is_empty() => s.clone(),
        _ => type_name.to_lowercase(),
    };
    let json_val = unity_value_to_json(val);
    let safe_name = name.replace(|c: char| !c.is_alphanumeric() && c != '_' && c != '-', "");
    let filename = if safe_name.is_empty() {
        format!("{}_{}_{}.json", type_name.to_lowercase(), class_id, path_id)
    } else {
        format!("{}_{}.json", safe_name, path_id)
    };
    let sub_dir_name = match class_id {
        21 => "materials",
        74 => "animationclips",
        114 => "monobehaviours",
        1 => "gameobjects",
        4 => "transforms",
        _ => "meta",
    };
    let target_dir = if by_file {
        output_dir.to_path_buf()
    } else {
        output_dir.join(sub_dir_name)
    };
    let _ = std::fs::create_dir_all(&target_dir);
    if let Ok(json_str) = serde_json::to_string_pretty(&json_val) {
        match get_unique_path(&target_dir, &filename, Some(json_str.as_bytes())) {
            UniquePathResult::New(dest) => std::fs::write(&dest, json_str).is_ok(),
            UniquePathResult::Exists(_) => true,
        }
    } else {
        false
    }
}

pub fn unity_value_to_json(value: &UnityValue) -> serde_json::Value {
    match value {
        UnityValue::Boolean(b) => serde_json::Value::Bool(*b),
        UnityValue::Int8(i) => serde_json::Value::Number(serde_json::Number::from(*i)),
        UnityValue::UInt8(u) => serde_json::Value::Number(serde_json::Number::from(*u)),
        UnityValue::Int16(i) => serde_json::Value::Number(serde_json::Number::from(*i)),
        UnityValue::UInt16(u) => serde_json::Value::Number(serde_json::Number::from(*u)),
        UnityValue::Int32(i) => serde_json::Value::Number(serde_json::Number::from(*i)),
        UnityValue::UInt32(u) => serde_json::Value::Number(serde_json::Number::from(*u)),
        UnityValue::Int64(i) => serde_json::Value::Number(serde_json::Number::from(*i)),
        UnityValue::UInt64(u) => serde_json::Value::Number(serde_json::Number::from(*u)),
        UnityValue::Float(f) => serde_json::Value::Number(serde_json::Number::from_f64(*f as f64).unwrap_or_else(|| serde_json::Number::from(0))),
        UnityValue::Double(d) => serde_json::Value::Number(serde_json::Number::from_f64(*d).unwrap_or_else(|| serde_json::Number::from(0))),
        UnityValue::String(s) => serde_json::Value::String(s.clone()),
        UnityValue::Bytes(b) => {
            let arr = b.iter().map(|&x| serde_json::Value::Number(serde_json::Number::from(x))).collect();
            serde_json::Value::Array(arr)
        }
        UnityValue::Array(arr) => {
            let json_arr = arr.iter().map(unity_value_to_json).collect();
            serde_json::Value::Array(json_arr)
        }
        UnityValue::Map(map) => {
            let mut json_map = serde_json::Map::new();
            for (k, v) in map {
                let key = if k == "m_Name" { "name".to_string() } else { k.clone() };
                json_map.insert(key, unity_value_to_json(v));
            }
            serde_json::Value::Object(json_map)
        }
        UnityValue::PPtr { file_id, path_id } => {
            let mut json_map = serde_json::Map::new();
            json_map.insert("file_id".to_string(), serde_json::Value::Number(serde_json::Number::from(*file_id)));
            json_map.insert("path_id".to_string(), serde_json::Value::String(path_id.to_string()));
            json_map.insert("m_FileID".to_string(), serde_json::Value::Number(serde_json::Number::from(*file_id)));
            json_map.insert("m_PathID".to_string(), serde_json::Value::String(path_id.to_string()));
            serde_json::Value::Object(json_map)
        }
        UnityValue::Null => serde_json::Value::Null,
    }
}
