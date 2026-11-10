use crate::define_unity_class;

define_unity_class!(StreamedResource {
    m_Source: String,
    m_Offset: u64,
    m_Size: u64,
});

define_unity_class!(VideoClip {
    m_Name: String,
    m_OriginalPath: Option<String>,
    m_ProxyWidth: Option<u32>,
    m_ProxyHeight: Option<u32>,
    Width: Option<u32>,
    Height: Option<u32>,
    m_PixelAspecRatioNum: Option<u32>,
    m_PixelAspecRatioDen: Option<u32>,
    m_FrameRate: Option<f64>,
    m_FrameCount: Option<u64>,
    m_Format: Option<i32>,
    m_AudioChannelCount: Option<Vec<u16>>,
    m_AudioSampleRate: Option<Vec<u32>>,
    m_AudioLanguage: Option<Vec<String>>,
    m_ExternalResources: StreamedResource,
    m_HasSplitAlpha: Option<bool>,
    m_sRGB: Option<bool>,
});

pub fn video_bytes(value: &crate::value::UnityValue, am: &crate::assets::AssetManager) -> Vec<u8> {
    if let Some(data) = value.get("m_ExternalResources").and_then(|res| am.streamed_resource(res)) {
        return data;
    }
    match value.get("m_MovieData") {
        Some(crate::value::UnityValue::Bytes(b)) => b.clone(),
        _ => Vec::new(),
    }
}

pub fn extension(d: &[u8], value: &crate::value::UnityValue) -> String {
    let sniffed = if d.get(4..8) == Some(b"ftyp") {
        if d.get(8..12) == Some(b"qt  ") { "mov" } else { "mp4" }
    } else if d.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        "webm"
    } else if d.starts_with(b"OggS") {
        "ogv"
    } else if d.starts_with(b"RIFF") && d.get(8..12) == Some(b"AVI ") {
        "avi"
    } else {
        ""
    };
    if !sniffed.is_empty() {
        return sniffed.to_string();
    }
    value
        .get("m_OriginalPath")
        .and_then(|p| p.as_str())
        .and_then(|p| std::path::Path::new(p).extension())
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_else(|| "bytes".to_string())
}

pub fn mime(ext: &str) -> &'static str {
    match ext {
        "mp4" | "mov" => "video/mp4",
        "webm" => "video/webm",
        "ogv" => "video/ogg",
        _ => "",
    }
}
