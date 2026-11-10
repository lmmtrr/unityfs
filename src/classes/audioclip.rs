use crate::assets::AssetManager;
use crate::value::UnityValue;

fn raw_slice(am: &AssetManager, source: &str, offset: usize, size: usize) -> Option<Vec<u8>> {
    let name = source.rsplit('/').next().unwrap_or(source);
    let raw = am.raw_files.get(name).or_else(|| am.raw_files.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v))?;
    raw.get(offset..offset.checked_add(size)?).map(<[u8]>::to_vec)
}

fn field_usize(value: &UnityValue, key: &str) -> Option<usize> {
    value.get(key)?.as_i64().and_then(|v| usize::try_from(v).ok())
}

pub fn audio_bytes(value: &UnityValue, am: &AssetManager, file_name: &str) -> Vec<u8> {
    if let Some(res) = value.get("m_Resource")
        && let (Some(source), Some(offset), Some(size)) = (res.get("m_Source").and_then(UnityValue::as_str), field_usize(res, "m_Offset"), field_usize(res, "m_Size"))
        && size > 0
        && let Some(data) = raw_slice(am, source, offset, size)
    {
        return data;
    }
    match value.get("m_AudioData") {
        Some(UnityValue::Bytes(b)) => b.clone(),
        Some(UnityValue::Array(items)) => items.iter().filter_map(|v| v.as_i64().map(|b| b as u8)).collect(),
        Some(streamed @ UnityValue::Map(_)) => {
            let source = format!("{}.resS", file_name.rsplit(['/', '\\']).next().unwrap_or(file_name));
            match (field_usize(streamed, "m_Offset"), field_usize(streamed, "m_Size")) {
                (Some(offset), Some(size)) => raw_slice(am, &source, offset, size).unwrap_or_default(),
                _ => Vec::new(),
            }
        }
        _ => Vec::new(),
    }
}

pub fn playable(data: Vec<u8>) -> Vec<u8> {
    match crate::fsb5::convert(&data) {
        Some((converted, _)) => converted,
        None => data,
    }
}

pub fn extension(d: &[u8]) -> &'static str {
    if d.starts_with(b"OggS") {
        "ogg"
    } else if d.starts_with(b"RIFF") {
        "wav"
    } else if d.get(4..8) == Some(b"ftyp") {
        "m4a"
    } else if d.starts_with(b"ID3") || (d.len() > 1 && d[0] == 0xFF && d[1] & 0xE0 == 0xE0) {
        "mp3"
    } else if d.starts_with(b"MThd") {
        "mid"
    } else if d.starts_with(b"FSB5") {
        "fsb"
    } else {
        "bytes"
    }
}
