use crate::value::UnityValue;

pub fn font_data(value: &UnityValue) -> Option<&[u8]> {
    match value.get("m_FontData") {
        Some(UnityValue::Bytes(b)) if !b.is_empty() => Some(b),
        _ => None,
    }
}

pub fn extension(d: &[u8]) -> &'static str {
    match d.get(..4) {
        Some([0, 1, 0, 0]) | Some(b"true") => "ttf",
        Some(b"OTTO") => "otf",
        Some(b"ttcf") => "ttc",
        Some(b"wOFF") => "woff",
        Some(b"wOF2") => "woff2",
        _ => "bytes",
    }
}

pub fn mime(d: &[u8]) -> &'static str {
    match extension(d) {
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "ttc" => "font/collection",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        _ => "",
    }
}
