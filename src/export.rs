use std::path::{Path, PathBuf};
use crate::assets::AssetManager;
use crate::classes::TryFromUnityValue;
use crate::classes::mesh::Mesh;
use crate::extractor::decompress_texture;
use crate::value::UnityValue;

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
    for (asset_name, sf) in &asset_manager.files {
        for obj in &sf.objects {
            let class_id = obj.class_id;
            let supported = match class_id {
                28 | 49 | 43 | 83 | 48 | 329 | 114 => true,
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
                28 => extract_texture2d(&unity_value, &bundle_output_dir, &asset_manager, flat, pb),
                49 => extract_text_asset(&unity_value, &bundle_output_dir, flat, pb),
                43 => extract_mesh(&unity_value, &bundle_output_dir, flat, pb),
                83 => extract_audioclip(&unity_value, &bundle_output_dir, &asset_manager, flat, pb),
                48 => extract_shader(&unity_value, &bundle_output_dir, flat, pb),
                329 => extract_videoclip(&unity_value, &bundle_output_dir, &asset_manager, flat, pb),
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
        let mut group_map: std::collections::BTreeMap<i32, Vec<serde_json::Value>> = std::collections::BTreeMap::new();
        for part in pose_parts_vec {
            let node = serde_json::json!({
                "Id": part.id,
                "Link": part.link,
            });
            group_map.entry(part.group_index).or_default().push(node);
        }
        let groups: Vec<Vec<serde_json::Value>> = group_map.into_values().collect();
        let pose_json = serde_json::json!({
            "Type": "Live2D Pose",
            "Groups": groups,
        });
        if let Ok(serialized) = serde_json::to_vec_pretty(&pose_json) {
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

fn find_and_extract_criware_bytes(
    value: &UnityValue,
    output_dir: &Path,
    base_name: &str,
    parent_key: &str,
    by_file: bool,
    pb: &Log,
) -> bool {
    match value {
        UnityValue::Bytes(bytes) => {
            if bytes.len() > 16 {
                let header = &bytes[..std::cmp::min(16, bytes.len())];
                let mut ext = None;
                if header.starts_with(b"@UTF") {
                    ext = Some("acb");
                } else if header.starts_with(b"AFS2") || header.starts_with(b"AWBP") {
                    ext = Some("awb");
                } else if header.starts_with(b"CPK ") {
                    ext = Some("cpk");
                } else if header.starts_with(b"RIFF") {
                    ext = Some("wav");
                } else if header.starts_with(b"OggS") {
                    ext = Some("ogg");
                } else if header.len() >= 8 && &header[4..8] == b"ftyp" {
                    ext = Some("m4a");
                }
                if let Some(extension) = ext {
                    let filename = if parent_key.is_empty() {
                        format!("{}.{}", base_name, extension)
                    } else {
                        format!("{}_{}.{}", base_name, parent_key, extension)
                    };
                    let cri_dir = if by_file {
                        output_dir.to_path_buf()
                    } else {
                        output_dir.join("CriWare")
                    };
                    let _ = std::fs::create_dir_all(&cri_dir);
                    return match get_unique_path(&cri_dir, &filename, Some(bytes)) {
                        UniquePathResult::New(dest) => {
                            if let Err(e) = std::fs::write(&dest, bytes) {
                                pb.println(format!("    Failed to write CriWare asset '{}': {}", dest.display(), e));
                                false
                            } else {
                                true
                            }
                        }
                        UniquePathResult::Exists(_) => true,
                    };
                }
            }
            false
        }
        UnityValue::Array(arr) => {
            if !arr.is_empty() && arr.len() > 16 {
                let mut bytes = Vec::with_capacity(arr.len());
                let mut is_byte_array = true;
                for item in arr {
                    match item {
                        UnityValue::UInt8(b) => bytes.push(*b),
                        UnityValue::Int8(b) => bytes.push(*b as u8),
                        UnityValue::UInt16(b) => bytes.push(*b as u8),
                        UnityValue::Int16(b) => bytes.push(*b as u8),
                        UnityValue::UInt32(b) => bytes.push(*b as u8),
                        UnityValue::Int32(b) => bytes.push(*b as u8),
                        UnityValue::UInt64(b) => bytes.push(*b as u8),
                        UnityValue::Int64(b) => bytes.push(*b as u8),
                        _ => {
                            is_byte_array = false;
                            break;
                        }
                    }
                }
                if is_byte_array {
                    return find_and_extract_criware_bytes(&UnityValue::Bytes(bytes), output_dir, base_name, parent_key, by_file, pb);
                }
            }
            let mut extracted = false;
            for (idx, item) in arr.iter().enumerate() {
                let item_key = if parent_key.is_empty() {
                    idx.to_string()
                } else {
                    format!("{}_{}", parent_key, idx)
                };
                if find_and_extract_criware_bytes(item, output_dir, base_name, &item_key, by_file, pb) {
                    extracted = true;
                }
            }
            extracted
        }
        UnityValue::Map(map) => {
            let mut extracted = false;
            for (k, v) in map {
                let item_key = if parent_key.is_empty() {
                    k.clone()
                } else {
                    format!("{}_{}", parent_key, k)
                };
                if find_and_extract_criware_bytes(v, output_dir, base_name, &item_key, by_file, pb) {
                    extracted = true;
                }
            }
            extracted
        }
        _ => false,
    }
}

struct PosePartData {
    id: String,
    group_index: i32,
    link: Vec<String>,
}

fn get_mono_behaviour_class_name(
    asset_manager: &AssetManager,
    asset_name: &str,
    m_script_pptr: &UnityValue,
) -> Option<String> {
    if let UnityValue::PPtr { file_id, path_id } = m_script_pptr {
        if let Ok(script_val) = asset_manager.read_object_value(asset_name, *file_id, *path_id) {
            if let Some(UnityValue::String(class_name)) = script_val.get("m_ClassName") {
                return Some(class_name.clone());
            }
        }
    }
    None
}

fn resolve_class_name(
    value: &UnityValue,
    asset_manager: &AssetManager,
    asset_name: &str,
) -> Option<String> {
    if let Some(script_pptr) = value.get("m_Script") {
        if let Some(class_name) = get_mono_behaviour_class_name(asset_manager, asset_name, script_pptr) {
            return Some(class_name);
        }
    }
    if value.get("ParameterIds").is_some() && value.get("ParameterCurves").is_some() {
        return Some("CubismFadeMotionData".to_string());
    }
    if value.get("Parameters").is_some() && value.get("FadeInTime").is_some() && value.get("FadeOutTime").is_some() {
        if value.get("ParameterIds").is_none() {
            return Some("CubismExpressionData".to_string());
        }
    }
    if value.get("GroupIndex").is_some() && value.get("Link").is_some() && value.get("m_GameObject").is_some() {
        return Some("CubismPosePart".to_string());
    }
    None
}

struct Keyframe {
    time: f32,
    value: f32,
    in_slope: f32,
    out_slope: f32,
}

fn parse_float(val: Option<&UnityValue>) -> Option<f32> {
    match val {
        Some(UnityValue::Float(f)) => Some(*f),
        Some(UnityValue::Double(d)) => Some(*d as f32),
        Some(UnityValue::Int8(i)) => Some(*i as f32),
        Some(UnityValue::UInt8(u)) => Some(*u as f32),
        Some(UnityValue::Int16(i)) => Some(*i as f32),
        Some(UnityValue::UInt16(u)) => Some(*u as f32),
        Some(UnityValue::Int32(i)) => Some(*i as f32),
        Some(UnityValue::UInt32(u)) => Some(*u as f32),
        Some(UnityValue::Int64(i)) => Some(*i as f32),
        Some(UnityValue::UInt64(u)) => Some(*u as f32),
        _ => None,
    }
}

fn parse_keyframe(val: &UnityValue) -> Option<Keyframe> {
    let map = match val {
        UnityValue::Map(m) => m,
        _ => return None,
    };
    let time = parse_float(map.get("time"))?;
    let value = parse_float(map.get("value"))?;
    let in_slope = parse_float(map.get("inSlope"))
        .or_else(|| parse_float(map.get("in_slope")))
        .unwrap_or(0.0);
    let out_slope = parse_float(map.get("outSlope"))
        .or_else(|| parse_float(map.get("out_slope")))
        .unwrap_or(0.0);
    Some(Keyframe { time, value, in_slope, out_slope })
}

fn add_segments(
    curve: &Keyframe,
    pre_curve: &Keyframe,
    next_curve: Option<&Keyframe>,
    segments: &mut Vec<f32>,
    _force_bezier: bool,
    total_point_count: &mut i32,
    total_segment_count: &mut i32,
    j: &mut usize,
) {
    if (curve.time - pre_curve.time - 0.01).abs() < 0.0001 {
        if let Some(next) = next_curve {
            if next.value == curve.value {
                segments.push(3.0);
                segments.push(next.time);
                segments.push(next.value);
                *j += 1;
                *total_point_count += 1;
                *total_segment_count += 1;
                return;
            }
        }
    }
    if curve.in_slope.is_infinite() && curve.in_slope.is_sign_positive() || curve.in_slope > 1000000.0 {
        segments.push(2.0);
        segments.push(curve.time);
        segments.push(curve.value);
        *total_point_count += 1;
    } else if pre_curve.out_slope == 0.0 && curve.in_slope.abs() < 0.0001 {
        segments.push(0.0);
        segments.push(curve.time);
        segments.push(curve.value);
        *total_point_count += 1;
    } else {
        let tangent_length = (curve.time - pre_curve.time) / 3.0;
        segments.push(1.0);
        segments.push(pre_curve.time + tangent_length);
        segments.push(pre_curve.out_slope * tangent_length + pre_curve.value);
        segments.push(curve.time - tangent_length);
        segments.push(curve.value - curve.in_slope * tangent_length);
        segments.push(curve.time);
        segments.push(curve.value);
        *total_point_count += 3;
    }
    *total_segment_count += 1;
}

fn convert_fade_motion_to_json(value: &UnityValue) -> Option<serde_json::Value> {
    let map = match value {
        UnityValue::Map(m) => m,
        _ => return None,
    };
    let motion_length = parse_float(map.get("MotionLength")).unwrap_or(0.0);
    let fade_in_time = parse_float(map.get("FadeInTime")).unwrap_or(0.0);
    let fade_out_time = parse_float(map.get("FadeOutTime")).unwrap_or(0.0);
    let parameter_ids = match map.get("ParameterIds") {
        Some(UnityValue::Array(arr)) => {
            arr.iter().map(|v| v.as_str().unwrap_or("").to_string()).collect::<Vec<_>>()
        }
        _ => Vec::new(),
    };
    let parameter_fade_in_times = match map.get("ParameterFadeInTimes") {
        Some(UnityValue::Array(arr)) => {
            arr.iter().map(|v| parse_float(Some(v)).unwrap_or(-1.0)).collect::<Vec<_>>()
        }
        _ => Vec::new(),
    };
    let parameter_fade_out_times = match map.get("ParameterFadeOutTimes") {
        Some(UnityValue::Array(arr)) => {
            arr.iter().map(|v| parse_float(Some(v)).unwrap_or(-1.0)).collect::<Vec<_>>()
        }
        _ => Vec::new(),
    };
    let parameter_curves = match map.get("ParameterCurves") {
        Some(UnityValue::Array(arr)) => arr,
        _ => return None,
    };
    let mut curves_json = Vec::new();
    let mut total_segment_count = 0;
    let mut total_point_count = 0;
    for i in 0..parameter_curves.len() {
        let curve_val = &parameter_curves[i];
        let curve_map = match curve_val {
            UnityValue::Map(m) => m,
            _ => continue,
        };
        let m_curve = match curve_map.get("m_Curve") {
            Some(UnityValue::Array(arr)) => arr,
            _ => continue,
        };
        if m_curve.is_empty() {
            continue;
        }
        let keyframes: Vec<Keyframe> = m_curve.iter().filter_map(parse_keyframe).collect();
        if keyframes.is_empty() {
            continue;
        }
        let param_id = parameter_ids.get(i).cloned().unwrap_or_else(|| "".to_string());
        if param_id.is_empty() {
            continue;
        }
        let target = match param_id.as_str() {
            "Opacity" | "EyeBlink" | "LipSync" => "Model",
            _ => {
                if param_id.to_lowercase().contains("part") {
                    "PartOpacity"
                } else {
                    "Parameter"
                }
            }
        };
        let curve_fade_in = parameter_fade_in_times.get(i).cloned().unwrap_or(-1.0);
        let curve_fade_out = parameter_fade_out_times.get(i).cloned().unwrap_or(-1.0);
        let mut segments = vec![keyframes[0].time, keyframes[0].value];
        let mut j = 1;
        while j < keyframes.len() {
            let curve = &keyframes[j];
            let pre_curve = &keyframes[j - 1];
            let next_curve = keyframes.get(j + 1);
            add_segments(
                curve,
                pre_curve,
                next_curve,
                &mut segments,
                false,
                &mut total_point_count,
                &mut total_segment_count,
                &mut j,
            );
            j += 1;
        }
        total_point_count += 1;
        curves_json.push(serde_json::json!({
            "Target": target,
            "Id": param_id,
            "FadeInTime": curve_fade_in,
            "FadeOutTime": curve_fade_out,
            "Segments": segments,
        }));
    }
    let curve_count = curves_json.len();
    let motion_json = serde_json::json!({
        "Version": 3,
        "Meta": {
            "Duration": motion_length,
            "Fps": 30.0,
            "Loop": true,
            "AreBeziersRestricted": true,
            "FadeInTime": fade_in_time,
            "FadeOutTime": fade_out_time,
            "CurveCount": curve_count as i32,
            "TotalSegmentCount": total_segment_count,
            "TotalPointCount": total_point_count,
            "UserDataCount": 0,
            "TotalUserDataSize": 0
        },
        "Curves": curves_json,
        "UserData": []
    });
    Some(motion_json)
}

fn convert_expression_data_to_json(value: &UnityValue) -> Option<serde_json::Value> {
    let map = match value {
        UnityValue::Map(m) => m,
        _ => return None,
    };
    let exp_type = match map.get("Type") {
        Some(UnityValue::String(s)) => s.clone(),
        _ => "Live2D Expression".to_string(),
    };
    let fade_in_time = parse_float(map.get("FadeInTime")).unwrap_or(1.0);
    let fade_out_time = parse_float(map.get("FadeOutTime")).unwrap_or(1.0);
    let parameters = match map.get("Parameters") {
        Some(UnityValue::Array(arr)) => {
            let mut params_json = Vec::new();
            for item in arr {
                if let UnityValue::Map(item_map) = item {
                    let id = match item_map.get("Id") {
                        Some(UnityValue::String(s)) => s.clone(),
                        _ => continue,
                    };
                    let val = parse_float(item_map.get("Value")).unwrap_or(0.0);
                    let blend = match item_map.get("Blend") {
                        Some(v) => v.as_i32().unwrap_or(0),
                        _ => 0,
                    };
                    params_json.push(serde_json::json!({
                        "Id": id,
                        "Value": val,
                        "Blend": blend
                    }));
                }
            }
            params_json
        }
        _ => Vec::new(),
    };
    Some(serde_json::json!({
        "Type": exp_type,
        "FadeInTime": fade_in_time,
        "FadeOutTime": fade_out_time,
        "Parameters": parameters
    }))
}

fn flatten_json_paths(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(s) => {
            if s.contains('/') || s.contains('\\') {
                if let Some(filename) = Path::new(s).file_name() {
                    *s = filename.to_string_lossy().into_owned();
                }
            }
        }
        serde_json::Value::Array(arr) => {
            for v in arr {
                flatten_json_paths(v);
            }
        }
        serde_json::Value::Object(map) => {
            for v in map.values_mut() {
                flatten_json_paths(v);
            }
        }
        _ => {}
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
    if find_and_extract_criware_bytes(val, output_dir, &sanitized_base, "", by_file, pb) {
        return true;
    }
    let mut is_cubism_pose_part = false;
    if let Some(class_name) = resolve_class_name(val, asset_manager, asset_name) {
        if class_name == "CubismPosePart" {
            is_cubism_pose_part = true;
        }
    }
    if is_cubism_pose_part {
        let group_index = val.get("GroupIndex").and_then(|v| v.as_i32()).unwrap_or(0);
        let link = val.get("Link").and_then(|v| match v {
            UnityValue::Array(arr) => Some(arr.iter().map(|item| item.as_str().unwrap_or("").to_string()).collect::<Vec<_>>()),
            _ => None
        }).unwrap_or_default();
        let mut go_name = String::new();
        if let Some(go_pptr) = val.get("m_GameObject") {
            if let UnityValue::PPtr { file_id, path_id } = go_pptr {
                if let Ok(go_val) = asset_manager.read_object_value(asset_name, *file_id, *path_id) {
                    if let Some(UnityValue::String(name)) = go_val.get("m_Name") {
                        go_name = name.clone();
                    }
                }
            }
        }
        if !go_name.is_empty() {
            let mut guard = pose_parts.lock().unwrap();
            guard.push(PosePartData {
                id: go_name,
                group_index,
                link,
            });
        }
        return true;
    }
    let mut content = None;
    let mut is_cubism_fade_motion = false;
    let mut is_cubism_expression = false;
    if let Some(class_name) = resolve_class_name(val, asset_manager, asset_name) {
        if class_name == "CubismFadeMotionData" {
            is_cubism_fade_motion = true;
        } else if class_name == "CubismExpressionData" {
            is_cubism_expression = true;
        }
    }
    if is_cubism_fade_motion {
        if let Some(json_val) = convert_fade_motion_to_json(val) {
            if let Ok(bytes) = serde_json::to_vec_pretty(&json_val) {
                content = Some(bytes);
            }
        }
    } else if is_cubism_expression {
        if let Some(json_val) = convert_expression_data_to_json(val) {
            if let Ok(bytes) = serde_json::to_vec_pretty(&json_val) {
                content = Some(bytes);
            }
        }
    }
    if content.is_none() {
        if let Some(sf) = asset_manager.files.get(asset_name) {
            let start = obj.byte_start;
            let size = obj.byte_size;
            if start + size <= sf.data.len() {
                let raw_bytes = &sf.data[start .. start + size];
                if let Some(moc_pos) = raw_bytes.windows(4).position(|window| window == b"MOC3") {
                    if moc_pos >= 4 {
                        let size_bytes = &raw_bytes[moc_pos - 4 .. moc_pos];
                        let is_big_endian = match sf.endian {
                            crate::reader::Endian::Big => true,
                            crate::reader::Endian::Little => false,
                        };
                        let moc_size = if is_big_endian {
                            u32::from_be_bytes([size_bytes[0], size_bytes[1], size_bytes[2], size_bytes[3]]) as usize
                        } else {
                            u32::from_le_bytes([size_bytes[0], size_bytes[1], size_bytes[2], size_bytes[3]]) as usize
                        };
                        if moc_pos + moc_size <= raw_bytes.len() {
                            content = Some(raw_bytes[moc_pos .. moc_pos + moc_size].to_vec());
                        } else {
                            content = Some(raw_bytes[moc_pos..].to_vec());
                        }
                    } else {
                        content = Some(raw_bytes[moc_pos..].to_vec());
                    }
                }
            }
        }
    }
    if content.is_none() {
        for key in &["_bytes", "m_Bytes", "bytes", "m_Data", "_data"] {
            if let Some(UnityValue::Bytes(b)) = val.get(*key) {
                content = Some(b.clone());
                break;
            }
        }
    }
    if content.is_none() {
        if let UnityValue::Map(map) = val {
            for (_, val_item) in map {
                if let UnityValue::Bytes(b) = val_item {
                    if b.starts_with(b"MOC3") {
                        content = Some(b.clone());
                        break;
                    }
                }
            }
        }
    }
    if let Some(data) = content {
        let mut final_name = sanitized_base.clone();
        if is_cubism_expression || final_name.ends_with(".exp") || final_name.ends_with(".exp3") {
            let name_without_exp = final_name.replace(".exp3", "").replace(".exp", "");
            final_name = format!("{}.exp3.json", name_without_exp);
        } else if is_cubism_fade_motion || final_name.ends_with(".fade") || final_name.ends_with(".motion") || final_name.ends_with(".motion3") {
            let name_without_motion = final_name
                .replace(".motion3", "")
                .replace(".motion", "")
                .replace(".fade", "");
            final_name = format!("{}.motion3.json", name_without_motion);
        } else if final_name.ends_with(".model") || final_name.ends_with(".model3") {
            let name_without = final_name.replace(".model3", "").replace(".model", "");
            final_name = format!("{}.model3.json", name_without);
        } else if final_name.ends_with(".physics") || final_name.ends_with(".physics3") {
            let name_without = final_name.replace(".physics3", "").replace(".physics", "");
            final_name = format!("{}.physics3.json", name_without);
        } else if final_name.ends_with(".pose") || final_name.ends_with(".pose3") {
            let name_without = final_name.replace(".pose3", "").replace(".pose", "");
            final_name = format!("{}.pose3.json", name_without);
        } else if final_name.ends_with(".cdi") || final_name.ends_with(".cdi3") {
            let name_without = final_name.replace(".cdi3", "").replace(".cdi", "");
            final_name = format!("{}.cdi3.json", name_without);
        } else if final_name.ends_with(".userdata") || final_name.ends_with(".userdata3") {
            let name_without = final_name.replace(".userdata3", "").replace(".userdata", "");
            final_name = format!("{}.userdata3.json", name_without);
        } else if !final_name.contains('.') {
            if data.starts_with(b"{") {
                final_name = format!("{}.json", final_name);
            } else if data.starts_with(b"MOC3") {
                final_name = format!("{}.moc3", final_name);
            } else {
                let check_len = std::cmp::min(data.len(), 256);
                let head_str = String::from_utf8_lossy(&data[..check_len]);
                let has_spine_version = head_str.contains("3.6") || head_str.contains("3.7") ||
                                        head_str.contains("3.8") || head_str.contains("4.0") ||
                                        head_str.contains("4.1") || head_str.contains("4.2");
                if has_spine_version {
                    final_name = format!("{}.skel", final_name);
                } else {
                    final_name = format!("{}.txt", final_name);
                }
            }
        }
        let monobehaviour_dir = if by_file {
            output_dir.to_path_buf()
        } else {
            output_dir.join("MonoBehaviour")
        };
        let _ = std::fs::create_dir_all(&monobehaviour_dir);
        let mut final_data = data;
        if final_name.ends_with(".model3.json") {
            if let Ok(mut json_val) = serde_json::from_slice::<serde_json::Value>(&final_data) {
                flatten_json_paths(&mut json_val);
                if let Ok(serialized) = serde_json::to_vec_pretty(&json_val) {
                    final_data = serialized;
                }
            }
        }
        if final_name.ends_with(".moc3") {
            let mut guard = moc_stem.lock().unwrap();
            *guard = Some(sanitized_base.replace(".moc3", "").replace(".moc", ""));
        }
        match get_unique_path(&monobehaviour_dir, &final_name, Some(&final_data)) {
            UniquePathResult::New(dest) => {
                if let Err(e) = std::fs::write(&dest, &final_data) {
                    pb.println(format!("    Failed to write MonoBehaviour asset '{}': {}", dest.display(), e));
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

fn is_json_bytes(data: &[u8]) -> bool {
    if let Ok(s) = std::str::from_utf8(data) {
        let trimmed = s.trim_start();
        trimmed.starts_with('{') || trimmed.starts_with('[')
    } else {
        false
    }
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
        } else if is_json_bytes(&data) {
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
                let check_len = std::cmp::min(data.len(), 256);
                let head_str = String::from_utf8_lossy(&data[..check_len]);
                let has_spine_version = head_str.contains("3.6") || head_str.contains("3.7") ||
                                        head_str.contains("3.8") || head_str.contains("4.0") ||
                                        head_str.contains("4.1") || head_str.contains("4.2");
                if has_spine_version {
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
    let mut image_data = Vec::new();
    let mut has_stream = false;
    if let Some(UnityValue::Map(stream_map)) = val.get("m_StreamData") {
        let offset = stream_map.get("offset").and_then(|v| match v {
            UnityValue::UInt64(o) => Some(*o),
            UnityValue::Int64(o) => Some(*o as u64),
            UnityValue::UInt32(o) => Some(*o as u64),
            UnityValue::Int32(o) => Some(*o as u64),
            _ => None,
        });
        let size = stream_map.get("size").and_then(|v| match v {
            UnityValue::UInt64(s) => Some(*s as u32),
            UnityValue::Int64(s) => Some(*s as u32),
            UnityValue::UInt32(s) => Some(*s),
            UnityValue::Int32(s) => Some(*s as u32),
            _ => None,
        });
        let path = stream_map.get("path").and_then(|v| match v {
            UnityValue::String(s) => Some(s.clone()),
            _ => None,
        });
        if let (Some(o), Some(s), Some(p)) = (offset, size, path) {
            if s > 0 {
                let stream_name = p.rsplit('/').next().unwrap_or(&p);
                if let Some(raw_data) = asset_manager.raw_files.get(stream_name) {
                    let start = o as usize;
                    let end = start + s as usize;
                    if end <= raw_data.len() {
                        image_data = raw_data[start..end].to_vec();
                        has_stream = true;
                    }
                }
            }
        }
    }
    if !has_stream {
        if let Some(UnityValue::Bytes(b)) = val.get("image_data").or_else(|| val.get("image data")) {
            image_data = b.clone();
        }
    }
    if image_data.is_empty() {
        return false;
    }
    if let Some(rgba_data) = decompress_texture(width, height, format, &image_data) {
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

fn extract_mesh(val: &UnityValue, output_dir: &Path, by_file: bool, pb: &Log) -> bool {
    let name = match val.get("m_Name") {
        Some(UnityValue::String(s)) if !s.is_empty() => s.clone(),
        _ => "mesh".to_string(),
    };
    if let Ok(mesh) = Mesh::try_from_unity_value(val) {
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
    by_file: bool,
    pb: &Log,
) -> bool {
    let name = match val.get("m_Name") {
        Some(UnityValue::String(s)) if !s.is_empty() => s.clone(),
        _ => "audio".to_string(),
    };
    let mut audio_data = Vec::new();
    let mut has_stream = false;
    if let Some(UnityValue::Map(res_map)) = val.get("m_Resource") {
        let offset = res_map.get("m_Offset").and_then(|v| match v {
            UnityValue::UInt64(o) => Some(*o),
            UnityValue::Int64(o) => Some(*o as u64),
            UnityValue::UInt32(o) => Some(*o as u64),
            UnityValue::Int32(o) => Some(*o as u64),
            _ => None,
        });
        let size = res_map.get("m_Size").and_then(|v| match v {
            UnityValue::UInt64(s) => Some(*s as u32),
            UnityValue::Int64(s) => Some(*s as u32),
            UnityValue::UInt32(s) => Some(*s),
            UnityValue::Int32(s) => Some(*s as u32),
            _ => None,
        });
        let path = res_map.get("m_Source").and_then(|v| match v {
            UnityValue::String(s) => Some(s.clone()),
            _ => None,
        });
        if let (Some(o), Some(s), Some(p)) = (offset, size, path) {
            if s > 0 {
                let stream_name = p.rsplit('/').next().unwrap_or(&p);
                if let Some(raw_data) = asset_manager.raw_files.get(stream_name) {
                    let start = o as usize;
                    let end = start + s as usize;
                    if end <= raw_data.len() {
                        audio_data = raw_data[start..end].to_vec();
                        has_stream = true;
                    }
                }
            }
        }
    }
    if !has_stream {
        if let Some(UnityValue::Bytes(b)) = val.get("m_AudioData") {
            audio_data = b.clone();
        }
    }
    if audio_data.is_empty() {
        return false;
    }
    let extension = if audio_data.starts_with(b"OggS") {
        "ogg"
    } else if audio_data.starts_with(b"RIFF") {
        "wav"
    } else if audio_data.len() >= 8 && &audio_data[4..8] == b"ftyp" {
        "m4a"
    } else if audio_data.starts_with(b"ID3") || audio_data.starts_with(&[0xFF, 0xFB]) {
        "mp3"
    } else if audio_data.starts_with(b"MThd") {
        "mid"
    } else if audio_data.starts_with(b"FSB5") {
        "fsb"
    } else {
        "bytes"
    };
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

fn extract_videoclip(
    val: &UnityValue,
    output_dir: &Path,
    asset_manager: &AssetManager,
    by_file: bool,
    pb: &Log,
) -> bool {
    let video_clip = match crate::classes::videoclip::VideoClip::from_value(val.clone()) {
        Ok(vc) => vc,
        Err(e) => {
            pb.println(format!("    Failed to parse VideoClip structure: {}", e));
            return false;
        }
    };
    if video_clip.m_ExternalResources.m_Size > 0 {
        let mut video_data = Vec::new();
        let source = &video_clip.m_ExternalResources.m_Source;
        if !source.is_empty() {
            let stream_name = source.rsplit('/').next().unwrap_or(source);
            if let Some(raw_data) = asset_manager.raw_files.get(stream_name) {
                let start = video_clip.m_ExternalResources.m_Offset as usize;
                let end = start + video_clip.m_ExternalResources.m_Size as usize;
                if end <= raw_data.len() {
                    video_data = raw_data[start..end].to_vec();
                }
            }
        }
        if !video_data.is_empty() {
            let ext = video_clip.m_OriginalPath.as_ref()
                .and_then(|p| Path::new(p).extension())
                .and_then(|s| s.to_str())
                .unwrap_or("mp4");
            let dot_ext = format!(".{}", ext);
            let sanitized_base = video_clip.m_Name.replace(|c: char| !c.is_alphanumeric() && c != '_' && c != '-', "");
            let filename = if sanitized_base.is_empty() {
                format!("video.{}", ext)
            } else if sanitized_base.ends_with(&dot_ext) {
                sanitized_base.clone()
            } else {
                format!("{}{}", sanitized_base, dot_ext)
            };
            let video_dir = if by_file {
                output_dir.to_path_buf()
            } else {
                output_dir.join("VideoClip")
            };
            let _ = std::fs::create_dir_all(&video_dir);
            match get_unique_path(&video_dir, &filename, Some(&video_data)) {
                UniquePathResult::New(dest) => {
                    if let Err(e) = std::fs::write(&dest, &video_data) {
                        pb.println(format!("    Failed to write VideoClip '{}': {}", dest.display(), e));
                        false
                    } else {
                        true
                    }
                }
                UniquePathResult::Exists(_) => true,
            }
        } else {
            pb.println(format!("    Failed to extract VideoClip raw bytes: resource data is missing or empty."));
            false
        }
    } else {
        false
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

fn unity_value_to_json(value: &UnityValue) -> serde_json::Value {
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
