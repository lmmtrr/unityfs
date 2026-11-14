use std::path::Path;
use crate::assets::AssetManager;
use crate::value::UnityValue;

pub(crate) struct PosePartData {
    id: String,
    group_index: i32,
    link: Vec<String>,
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum ContentKind {
    FadeMotion,
    Expression,
    Other,
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

pub(crate) fn resolve_class_name(
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

pub(crate) fn convert_fade_motion_to_json(value: &UnityValue) -> Option<serde_json::Value> {
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

pub(crate) fn convert_expression_data_to_json(value: &UnityValue) -> Option<serde_json::Value> {
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

pub(crate) fn flatten_json_paths(value: &mut serde_json::Value) {
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

fn spine_string(data: &[u8], pos: &mut usize) -> Option<Vec<u8>> {
    let mut n = 0usize;
    for shift in 0..5 {
        let b = *data.get(*pos)?;
        *pos += 1;
        n |= ((b & 0x7F) as usize) << (shift * 7);
        if b & 0x80 == 0 {
            break;
        }
    }
    let len = n.checked_sub(1)?;
    let bytes = data.get(*pos..*pos + len)?.to_vec();
    *pos += len;
    Some(bytes)
}

fn is_spine_version(v: &[u8]) -> bool {
    matches!(v, [major, b'.', minor, ..] if (b'2'..=b'9').contains(major) && minor.is_ascii_digit()) && v.iter().all(|b| b.is_ascii_digit() || *b == b'.' || *b == b'-')
}

pub(crate) fn is_spine_binary(data: &[u8]) -> bool {
    if std::str::from_utf8(data).is_ok() {
        return false;
    }
    let mut pos = 0;
    let v3 = spine_string(data, &mut pos).and_then(|_| spine_string(data, &mut pos));
    let mut pos = 8;
    let v4 = spine_string(data, &mut pos);
    v3.is_some_and(|v| is_spine_version(&v)) || v4.is_some_and(|v| is_spine_version(&v))
}

pub(crate) fn is_json_bytes(data: &[u8]) -> bool {
    let data = data.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(data);
    if let Ok(s) = std::str::from_utf8(data) {
        let trimmed = s.trim_start();
        trimmed.starts_with('{') || trimmed.starts_with('[')
    } else {
        false
    }
}

pub(crate) fn criware_blobs(value: &UnityValue, key: &str, out: &mut Vec<(String, &'static str, Vec<u8>)>) {
    let child_key = |k: &str| if key.is_empty() { k.to_string() } else { format!("{}_{}", key, k) };
    match value {
        UnityValue::Bytes(bytes) if bytes.len() > 16 => {
            let ext = if bytes.starts_with(b"@UTF") {
                "acb"
            } else if bytes.starts_with(b"AFS2") || bytes.starts_with(b"AWBP") {
                "awb"
            } else if bytes.starts_with(b"CPK ") {
                "cpk"
            } else if bytes.starts_with(b"RIFF") {
                "wav"
            } else if bytes.starts_with(b"OggS") {
                "ogg"
            } else if &bytes[4..8] == b"ftyp" {
                "m4a"
            } else {
                return;
            };
            out.push((key.to_string(), ext, bytes.clone()));
        }
        UnityValue::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                criware_blobs(item, &child_key(&i.to_string()), out);
            }
        }
        UnityValue::Map(map) => {
            for (k, v) in map {
                criware_blobs(v, &child_key(k), out);
            }
        }
        _ => {}
    }
}

const RESERVED_NAMES: &[&str] = &["CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9"];

const MAX_NAME_CHARS: usize = 200;

pub(crate) fn safe_file_name(name: &str) -> String {
    let replaced: String = name.chars().take(MAX_NAME_CHARS).map(|c| if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') { '_' } else { c }).collect();
    let trimmed = replaced.trim().trim_end_matches(['.', ' ']);
    let stem = trimmed.split('.').next().unwrap_or_default().to_ascii_uppercase();
    if RESERVED_NAMES.contains(&stem.as_str()) { format!("_{}", trimmed) } else { trimmed.to_string() }
}

pub(crate) fn file_stem(name: &str, fallback: &str) -> String {
    Some(safe_file_name(name)).filter(|s| !s.is_empty()).unwrap_or_else(|| fallback.to_string())
}

pub(crate) fn criware_filename(base: &str, key: &str, ext: &str) -> String {
    let key = safe_file_name(key);
    if key.is_empty() { format!("{}.{}", base, ext) } else { format!("{}_{}.{}", base, key, ext) }
}

pub(crate) fn pose_part(value: &UnityValue, asset_manager: &AssetManager, asset_name: &str) -> Option<PosePartData> {
    let group_index = value.get("GroupIndex").and_then(|v| v.as_i32()).unwrap_or(0);
    let link = match value.get("Link") {
        Some(UnityValue::Array(items)) => items.iter().map(|v| v.as_str().unwrap_or("").to_string()).collect(),
        _ => Vec::new(),
    };
    let Some(UnityValue::PPtr { file_id, path_id }) = value.get("m_GameObject") else { return None };
    let go = asset_manager.read_object_value(asset_name, *file_id, *path_id).ok()?;
    let id = go.get("m_Name")?.as_str().filter(|n| !n.is_empty())?.to_string();
    Some(PosePartData { id, group_index, link })
}

pub(crate) fn pose_json(parts: Vec<PosePartData>) -> Option<Vec<u8>> {
    let mut groups: std::collections::BTreeMap<i32, Vec<serde_json::Value>> = std::collections::BTreeMap::new();
    for part in parts {
        groups.entry(part.group_index).or_default().push(serde_json::json!({ "Id": part.id, "Link": part.link }));
    }
    let groups: Vec<Vec<serde_json::Value>> = groups.into_values().collect();
    serde_json::to_vec_pretty(&serde_json::json!({ "Type": "Live2D Pose", "Groups": groups })).ok()
}

fn embedded_moc(raw: &[u8], endian: crate::reader::Endian) -> Option<Vec<u8>> {
    let pos = raw.windows(4).position(|w| w == b"MOC3")?;
    let size = raw.get(pos.checked_sub(4)?..pos).map(|b| {
        let b: [u8; 4] = b.try_into().unwrap();
        let n = match endian {
            crate::reader::Endian::Big => u32::from_be_bytes(b),
            crate::reader::Endian::Little => u32::from_le_bytes(b),
        };
        n as usize
    });
    let end = size.map(|n| pos + n).filter(|&end| end <= raw.len()).unwrap_or(raw.len());
    Some(raw[pos..end].to_vec())
}

pub(crate) fn monobehaviour_content(value: &UnityValue, asset_manager: &AssetManager, asset_name: &str, obj: &crate::objectreader::ObjectReader) -> Option<(Vec<u8>, ContentKind)> {
    let kind = match resolve_class_name(value, asset_manager, asset_name).as_deref() {
        Some("CubismFadeMotionData") => ContentKind::FadeMotion,
        Some("CubismExpressionData") => ContentKind::Expression,
        _ => ContentKind::Other,
    };
    let converted = match kind {
        ContentKind::FadeMotion => convert_fade_motion_to_json(value),
        ContentKind::Expression => convert_expression_data_to_json(value),
        ContentKind::Other => None,
    };
    if let Some(bytes) = converted.and_then(|json| serde_json::to_vec_pretty(&json).ok()) {
        return Some((bytes, kind));
    }
    let sf = asset_manager.files.get(asset_name)?;
    let raw = sf.data.get(obj.byte_start..obj.byte_start.checked_add(obj.byte_size)?);
    if let Some(moc) = raw.and_then(|raw| embedded_moc(raw, sf.endian)) {
        return Some((moc, kind));
    }
    let named = ["_bytes", "m_Bytes", "bytes", "m_Data", "_data"].iter().find_map(|k| match value.get(k) {
        Some(UnityValue::Bytes(b)) => Some(b.clone()),
        _ => None,
    });
    let moc = || match value {
        UnityValue::Map(map) => map.values().find_map(|v| match v {
            UnityValue::Bytes(b) if b.starts_with(b"MOC3") => Some(b.clone()),
            _ => None,
        }),
        _ => None,
    };
    named.or_else(moc).map(|data| (data, kind))
}

fn replace_suffix(name: &str, suffixes: &[&str], target: &str, force: bool) -> Option<String> {
    (force || suffixes.iter().any(|s| name.ends_with(s))).then(|| {
        let stem = suffixes.iter().fold(name.to_string(), |n, s| n.replace(s, ""));
        format!("{}{}", stem, target)
    })
}

pub(crate) fn content_filename(base: &str, data: &[u8], kind: ContentKind) -> String {
    let rules: [(&[&str], &str, bool); 7] = [
        (&[".exp3", ".exp"], ".exp3.json", kind == ContentKind::Expression),
        (&[".motion3", ".motion", ".fade"], ".motion3.json", kind == ContentKind::FadeMotion),
        (&[".model3", ".model"], ".model3.json", false),
        (&[".physics3", ".physics"], ".physics3.json", false),
        (&[".pose3", ".pose"], ".pose3.json", false),
        (&[".cdi3", ".cdi"], ".cdi3.json", false),
        (&[".userdata3", ".userdata"], ".userdata3.json", false),
    ];
    if let Some(name) = rules.iter().find_map(|(suffixes, target, force)| replace_suffix(base, suffixes, target, *force)) {
        return name;
    }
    if base.contains('.') {
        base.to_string()
    } else if is_json_bytes(data) {
        format!("{}.json", base)
    } else if data.starts_with(b"MOC3") {
        format!("{}.moc3", base)
    } else if is_spine_binary(data) {
        format!("{}.skel", base)
    } else {
        format!("{}.txt", base)
    }
}

pub(crate) fn finalize_content(filename: &str, data: Vec<u8>) -> Vec<u8> {
    if !filename.ends_with(".model3.json") {
        return data;
    }
    let Ok(mut json) = serde_json::from_slice::<serde_json::Value>(&data) else { return data };
    flatten_json_paths(&mut json);
    serde_json::to_vec_pretty(&json).unwrap_or(data)
}

pub(crate) fn numbered_name(filename: &str, counter: usize) -> String {
    let path = Path::new(filename);
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    match path.extension() {
        Some(extension) => format!("{} ({}).{}", stem, counter, extension.to_string_lossy()),
        None => format!("{} ({})", stem, counter),
    }
}

pub(crate) fn moc_stem(base: &str) -> String {
    base.replace(".moc3", "").replace(".moc", "")
}

pub(crate) fn texture_image(value: &UnityValue, asset_manager: &AssetManager) -> Vec<u8> {
    let streamed = value.get("m_StreamData").and_then(|s| {
        let field = |k: &str| s.get(k)?.as_i64().and_then(|v| usize::try_from(v).ok());
        let size = field("size").filter(|&n| n > 0)?;
        asset_manager.resource_slice(s.get("path")?.as_str()?, field("offset")?, size)
    });
    match streamed {
        Some(data) => data,
        None => match value.get("image_data").or_else(|| value.get("image data")) {
            Some(UnityValue::Bytes(b)) => b.clone(),
            _ => Vec::new(),
        },
    }
}
