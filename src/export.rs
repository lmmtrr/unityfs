use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use crate::assets::AssetManager;
use crate::classes::TryFromUnityValue;
use crate::extractor::decompress_unity_texture;
use crate::value::UnityValue;
use crate::export_common::{self as common, PosePartData};

#[derive(Clone, Debug, Default)]
pub struct ExportOptions {
    pub by_file: bool,
    pub metadata: bool,
    pub objects: Option<HashMap<String, HashSet<i64>>>,
    pub dependencies: Vec<PathBuf>,
    pub mono_behaviour_json: bool,
}

impl ExportOptions {
    fn wants(&self, file: &str, path_id: i64) -> bool {
        self.objects.as_ref().is_none_or(|o| o.get(file).is_some_and(|ids| ids.contains(&path_id)))
    }
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
    let mut asset_manager = AssetManager::open_path(file_path).map_err(|e| format!("{}: {}", file_path.display(), e))?;
    let own: HashSet<String> = asset_manager.files.keys().cloned().collect();
    let wants_sprites = asset_manager.files.iter().any(|(name, sf)| sf.objects.iter().any(|o| o.class_id == 213 && options.wants(name, o.path_id)));
    if wants_sprites {
        for dep in &options.dependencies {
            if let Err(e) = asset_manager.add_dependency(dep) {
                eprintln!("[{}]     Warning: dependency not loaded: {}", file_stem, e);
            }
        }
    }
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
    for (asset_name, sf) in asset_manager.files.iter().filter(|(name, _)| own.contains(*name)) {
        let mut objects: Vec<_> = sf.objects.iter().collect();
        objects.sort_by_key(|o| o.class_id == 213);
        for obj in objects {
            let class_id = obj.class_id;
            if !options.wants(asset_name, obj.path_id) {
                continue;
            }
            let supported = match class_id {
                28 | 49 | 83 | 48 | 128 | 152 | 329 | 114 | 213 => true,
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
            let fallback = format!("{}_{}", t_name.to_lowercase(), path_id);
            let success = match class_id {
                28 => extract_texture2d(&unity_value, &sf.unity_version, sf.target_platform, &bundle_output_dir, &asset_manager, flat, pb, &fallback),
                49 => extract_text_asset(&unity_value, &bundle_output_dir, flat, pb, &fallback),
                213 => extract_sprite(&unity_value, asset_name, &mut sprites, &bundle_output_dir, pb, &fallback),
                83 => extract_audioclip(&unity_value, &bundle_output_dir, &asset_manager, asset_name, flat, pb, &fallback),
                128 => extract_font(&unity_value, &bundle_output_dir, flat, pb, &fallback),
                48 => extract_shader(&unity_value, &bundle_output_dir, flat, pb, &fallback),
                329 => extract_videoclip(&unity_value, &bundle_output_dir, &asset_manager, "VideoClip", flat, pb, &fallback),
                152 => extract_videoclip(&unity_value, &bundle_output_dir, &asset_manager, "MovieTexture", flat, pb, &fallback),
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
                        options.mono_behaviour_json,
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
            if let UniquePathResult::New(dest) = get_unique_path(&bundle_output_dir, &format!("{}.pose3.json", stem), &serialized) {
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
    Exists,
}

fn same_file(path: &Path, data: &[u8]) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.len() == data.len() as u64) && std::fs::read(path).is_ok_and(|existing| existing == data)
}

fn get_unique_path(dir: &Path, filename: &str, data: &[u8]) -> UniquePathResult {
    let mut candidate = dir.join(filename);
    let mut counter = 2;
    while candidate.exists() {
        if same_file(&candidate, data) {
            return UniquePathResult::Exists;
        }
        candidate = dir.join(common::numbered_name(filename, counter));
        counter += 1;
    }
    UniquePathResult::New(candidate)
}

fn encode_png(rgba: &[u8], width: usize, height: usize) -> Result<Vec<u8>, image::ImageError> {
    let mut png = Vec::new();
    image::ImageEncoder::write_image(image::codecs::png::PngEncoder::new(&mut png), rgba, width as u32, height as u32, image::ExtendedColorType::Rgba8)?;
    Ok(png)
}

fn write_unique(dir: &Path, filename: &str, data: &[u8], what: &str, pb: &Log) -> bool {
    let _ = std::fs::create_dir_all(dir);
    match get_unique_path(dir, filename, data) {
        UniquePathResult::New(dest) => match std::fs::write(&dest, data) {
            Ok(()) => true,
            Err(e) => {
                pb.println(format!("    Failed to write {} '{}': {}", what, dest.display(), e));
                false
            }
        },
        UniquePathResult::Exists => true,
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
    json: bool,
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
    let sanitized_base = common::file_stem(&base_name, &format!("monobehaviour_{}", obj.path_id));
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
    let monobehaviour_dir = if by_file { output_dir.to_path_buf() } else { output_dir.join("MonoBehaviour") };
    let Some((data, kind)) = common::monobehaviour_content(val, asset_manager, asset_name, obj) else {
        if !json {
            return false;
        }
        let Ok(json) = serde_json::to_vec_pretty(&unity_value_to_json(val)) else { return false };
        return write_unique(&monobehaviour_dir, &format!("{}.json", sanitized_base), &json, "MonoBehaviour JSON", pb);
    };
    let final_name = common::content_filename(&sanitized_base, &data, kind);
    let final_data = common::finalize_content(&final_name, data);
    if final_name.ends_with(".moc3") {
        *moc_stem.lock().unwrap() = Some(common::moc_stem(&sanitized_base));
    }
    write_unique(&monobehaviour_dir, &final_name, &final_data, "MonoBehaviour asset", pb)
}

fn extract_text_asset(val: &UnityValue, output_dir: &Path, by_file: bool, pb: &Log, fallback: &str) -> bool {
    let name = match val.get("m_Name") {
        Some(UnityValue::String(s)) if !s.is_empty() => s.clone(),
        _ => "text_asset".to_string(),
    };
    let base = Path::new(&name).file_name().map_or_else(|| name.clone(), |f| f.to_string_lossy().to_string());
    let name = common::file_stem(&base, fallback);
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
        } else if name.contains('.') {
            name
        } else if common::is_spine_binary(&data) {
            format!("{}.skel", name)
        } else {
            format!("{}.txt", name)
        };
        let text_dir = if by_file {
            output_dir.to_path_buf()
        } else {
            output_dir.join("TextAsset")
        };
        let _ = std::fs::create_dir_all(&text_dir);
        match get_unique_path(&text_dir, &filename, &data) {
            UniquePathResult::New(dest) => {
                if let Err(e) = std::fs::write(&dest, &data) {
                    pb.println(format!("    Failed to write text asset '{}': {}", dest.display(), e));
                    false
                } else {
                    true
                }
            }
            UniquePathResult::Exists => true,
        }
    } else {
        false
    }
}

fn extract_texture2d(
    val: &UnityValue,
    version: &crate::unity_version::UnityVersion,
    platform: i32,
    output_dir: &Path,
    asset_manager: &AssetManager,
    by_file: bool,
    pb: &Log,
    fallback: &str,
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
    if let Some(rgba_data) = decompress_unity_texture(val, &image_data, version, platform, true) {
        let png = match encode_png(&rgba_data, width, height) {
            Ok(png) => png,
            Err(e) => {
                pb.println(format!("    Failed to encode PNG texture '{}': {}", name, e));
                return false;
            }
        };
        let filename = format!("{}.png", common::file_stem(&name, fallback));
        let texture_dir = if by_file {
            output_dir.to_path_buf()
        } else {
            output_dir.join("Texture2D")
        };
        write_unique(&texture_dir, &filename, &png, "PNG texture", pb)
    } else {
        pb.println(format!("    Failed to decompress texture '{}' (format={})", name, format));
        false
    }
}

fn extract_sprite(val: &UnityValue, asset_name: &str, sprites: &mut crate::classes::sprite::SpriteAtlases, output_dir: &Path, pb: &Log, fallback: &str) -> bool {
    let name = match val.get("m_Name") {
        Some(UnityValue::String(s)) if !s.is_empty() => s.clone(),
        _ => "sprite".to_string(),
    };
    let Some(image) = sprites.image(asset_name, val) else {
        pb.println(format!("    Failed to cut sprite '{}'", name));
        return false;
    };
    let png = match encode_png(&image.rgba, image.width, image.height) {
        Ok(png) => png,
        Err(e) => {
            pb.println(format!("    Failed to encode sprite '{}': {}", name, e));
            return false;
        }
    };
    let filename = format!("{}.png", common::file_stem(&name, fallback));
    write_unique(output_dir, &filename, &png, "sprite", pb)
}

fn extract_audioclip(
    val: &UnityValue,
    output_dir: &Path,
    asset_manager: &AssetManager,
    file_name: &str,
    by_file: bool,
    pb: &Log,
    fallback: &str,
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
    let filename = format!("{}.{}", common::file_stem(&name, fallback), extension);
    let audio_dir = if by_file {
        output_dir.to_path_buf()
    } else {
        output_dir.join("AudioClip")
    };
    let _ = std::fs::create_dir_all(&audio_dir);
    match get_unique_path(&audio_dir, &filename, &audio_data) {
        UniquePathResult::New(dest) => {
            if let Err(e) = std::fs::write(&dest, &audio_data) {
                pb.println(format!("    Failed to write AudioClip '{}': {}", dest.display(), e));
                false
            } else {
                true
            }
        }
        UniquePathResult::Exists => true,
    }
}

fn extract_font(val: &UnityValue, output_dir: &Path, by_file: bool, pb: &Log, fallback: &str) -> bool {
    let Some(data) = crate::classes::font::font_data(val) else { return false };
    let name = match val.get("m_Name") {
        Some(UnityValue::String(s)) if !s.is_empty() => s.clone(),
        _ => "font".to_string(),
    };
    let filename = format!("{}.{}", common::file_stem(&name, fallback), crate::classes::font::extension(data));
    let font_dir = if by_file { output_dir.to_path_buf() } else { output_dir.join("Font") };
    let _ = std::fs::create_dir_all(&font_dir);
    match get_unique_path(&font_dir, &filename, data) {
        UniquePathResult::New(dest) => match std::fs::write(&dest, data) {
            Ok(()) => true,
            Err(e) => {
                pb.println(format!("    Failed to write Font '{}': {}", dest.display(), e));
                false
            }
        },
        UniquePathResult::Exists => true,
    }
}

fn extract_videoclip(
    val: &UnityValue,
    output_dir: &Path,
    asset_manager: &AssetManager,
    folder: &str,
    by_file: bool,
    pb: &Log,
    fallback: &str,
) -> bool {
    let video_data = crate::classes::videoclip::video_bytes(val, asset_manager);
    if video_data.is_empty() {
        return false;
    }
    let ext = crate::classes::videoclip::extension(&video_data, val);
    let name = val.get("m_Name").and_then(UnityValue::as_str).unwrap_or("");
    let dot_ext = format!(".{}", ext);
    let sanitized_base = common::file_stem(name, fallback);
    let filename = if sanitized_base.ends_with(&dot_ext) {
        sanitized_base
    } else {
        format!("{}{}", sanitized_base, dot_ext)
    };
    let video_dir = if by_file { output_dir.to_path_buf() } else { output_dir.join(folder) };
    let _ = std::fs::create_dir_all(&video_dir);
    match get_unique_path(&video_dir, &filename, &video_data) {
        UniquePathResult::New(dest) => match std::fs::write(&dest, &video_data) {
            Ok(()) => true,
            Err(e) => {
                pb.println(format!("    Failed to write video '{}': {}", dest.display(), e));
                false
            }
        },
        UniquePathResult::Exists => true,
    }
}

fn extract_shader(val: &UnityValue, output_dir: &Path, by_file: bool, pb: &Log, fallback: &str) -> bool {
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
        let filename = format!("{}.shader", common::file_stem(&name, fallback));
        let shader_dir = if by_file {
            output_dir.to_path_buf()
        } else {
            output_dir.join("Shader")
        };
        let _ = std::fs::create_dir_all(&shader_dir);
        match get_unique_path(&shader_dir, &filename, &data) {
            UniquePathResult::New(dest) => {
                if let Err(e) = std::fs::write(&dest, &data) {
                    pb.println(format!("    Failed to write Shader '{}': {}", dest.display(), e));
                    false
                } else {
                    true
                }
            }
            UniquePathResult::Exists => true,
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
    let safe_name = common::safe_file_name(&name);
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
        match get_unique_path(&target_dir, &filename, json_str.as_bytes()) {
            UniquePathResult::New(dest) => std::fs::write(&dest, json_str).is_ok(),
            UniquePathResult::Exists => true,
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
