use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::assets::AssetManager;
use crate::serializedfile::SerializedFile;
use crate::value::UnityValue;
use crate::Reader;
use crate::Bundle;
use crate::classes::TryFromUnityValue;

pub fn is_unity_bundle(data: &[u8]) -> bool {
    data.starts_with(b"UnityFS") || data.starts_with(b"UnityWeb") || data.starts_with(b"UnityRaw")
}
pub fn decompress_texture(width: usize, height: usize, format: i32, image_data: &[u8]) -> Option<Vec<u8>> {
    let (block_w, block_h) = match format {
        10 | 11 | 12 | 24 | 25 | 26 | 27 | 34 | 35 | 36 | 41 | 42 | 43 | 44 | 45 | 46 | 47 => (4, 4),
        30 | 31 => (8, 4),
        32 | 33 => (4, 4),
        48 | 54 | 66 => (4, 4),
        49 | 55 | 67 => (5, 5),
        50 | 56 | 68 => (6, 6),
        51 | 57 | 69 => (8, 8),
        52 | 58 | 70 => (10, 10),
        53 | 59 | 71 => (12, 12),
        _ => (1, 1),
    };
    let aligned_w = ((width + block_w - 1) / block_w) * block_w;
    let aligned_h = ((height + block_h - 1) / block_h) * block_h;
    let aligned_size = aligned_w * aligned_h;
    let is_crunch = matches!(format, 28 | 29 | 64 | 65);
    let buffer_size = if is_crunch {
        width * height * 2
    } else {
        std::cmp::max(width * height, aligned_size)
    };
    let mut decompressed = vec![0u32; buffer_size];
    let mut success = false;
    let expected_input_size = match format {
        10 | 34 | 45 | 46 | 60 | 61 => {
            Some(((width + 3) / 4) * ((height + 3) / 4) * 8)
        }
        12 | 25 | 27 | 47 => {
            Some(((width + 3) / 4) * ((height + 3) / 4) * 16)
        }
        26 => {
            Some(((width + 3) / 4) * ((height + 3) / 4) * 8)
        }
        48 | 54 | 66 => {
            Some(((width + 3) / 4) * ((height + 3) / 4) * 16)
        }
        49 | 55 | 67 => {
            Some(((width + 4) / 5) * ((height + 4) / 5) * 16)
        }
        50 | 56 | 68 => {
            Some(((width + 5) / 6) * ((height + 5) / 6) * 16)
        }
        51 | 57 | 69 => {
            Some(((width + 7) / 8) * ((height + 7) / 8) * 16)
        }
        52 | 58 | 70 => {
            Some(((width + 9) / 10) * ((height + 9) / 10) * 16)
        }
        53 | 59 | 71 => {
            Some(((width + 11) / 12) * ((height + 11) / 12) * 16)
        }
        _ => None,
    };
    let safe_image_data = match expected_input_size {
        Some(size) if image_data.len() >= size => &image_data[0..size],
        _ => image_data,
    };
    match format {
        28 | 29 | 64 | 65 => {
            success = texture2ddecoder::decode_unity_crunch(image_data, width, height, &mut decompressed).is_ok();
        }
        1 => {
            for (i, &a) in safe_image_data.iter().enumerate().take(width * height) {
                decompressed[i] = ((a as u32) << 24) | 0x00FFFFFF;
            }
            success = true;
        }
        2 => {
            for (i, chunk) in safe_image_data.chunks_exact(2).enumerate().take(width * height) {
                let val = u16::from_le_bytes([chunk[0], chunk[1]]);
                let a = ((val >> 12) & 0xF) as u8 * 17;
                let r = ((val >> 8) & 0xF) as u8 * 17;
                let g = ((val >> 4) & 0xF) as u8 * 17;
                let b = (val & 0xF) as u8 * 17;
                decompressed[i] = u32::from_le_bytes([b, g, r, a]);
            }
            success = true;
        }
        3 => {
            for (i, chunk) in safe_image_data.chunks_exact(3).enumerate().take(width * height) {
                decompressed[i] = u32::from_le_bytes([chunk[2], chunk[1], chunk[0], 255]);
            }
            success = true;
        }
        4 => {
            for (i, chunk) in safe_image_data.chunks_exact(4).enumerate().take(width * height) {
                decompressed[i] = u32::from_le_bytes([chunk[2], chunk[1], chunk[0], chunk[3]]);
            }
            success = true;
        }
        5 => {
            for (i, chunk) in safe_image_data.chunks_exact(4).enumerate().take(width * height) {
                decompressed[i] = u32::from_le_bytes([chunk[3], chunk[2], chunk[1], chunk[0]]);
            }
            success = true;
        }
        7 => {
            for (i, chunk) in safe_image_data.chunks_exact(2).enumerate().take(width * height) {
                let val = u16::from_le_bytes([chunk[0], chunk[1]]);
                let r = ((val >> 11) & 0x1F) as u8;
                let g = ((val >> 5) & 0x3F) as u8;
                let b = (val & 0x1F) as u8;
                let r8 = (r << 3) | (r >> 2);
                let g8 = (g << 2) | (g >> 4);
                let b8 = (b << 3) | (b >> 2);
                decompressed[i] = u32::from_le_bytes([b8, g8, r8, 255]);
            }
            success = true;
        }
        8 => {
            for (i, chunk) in safe_image_data.chunks_exact(3).enumerate().take(width * height) {
                decompressed[i] = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], 255]);
            }
            success = true;
        }
        10 => {
            success = texture2ddecoder::decode_bc1(safe_image_data, aligned_w, aligned_h, &mut decompressed).is_ok();
        }
        11 => {
            let bw = (width + 3) / 4;
            let bh = (height + 3) / 4;
            if safe_image_data.len() >= bw * bh * 16 {
                for by in 0..bh {
                    for bx in 0..bw {
                        let offset = (by * bw + bx) * 16;
                        let alpha = &safe_image_data[offset..offset + 8];
                        let color = &safe_image_data[offset + 8..offset + 16];
                        let mut block = [0u32; 16];
                        if texture2ddecoder::decode_bc1(color, 4, 4, &mut block).is_ok() {
                            for i in 0..16 {
                                let px = bx * 4 + (i % 4);
                                let py = by * 4 + (i / 4);
                                if px < width && py < height {
                                    let a = (alpha[i / 2] >> ((i % 2) * 4)) & 0xF;
                                    let a8 = a | (a << 4);
                                    let c = block[i].to_le_bytes();
                                    decompressed[py * width + px] = u32::from_le_bytes([c[0], c[1], c[2], a8]);
                                }
                            }
                        }
                    }
                }
                success = true;
            }
        }
        12 => {
            success = texture2ddecoder::decode_bc3(safe_image_data, aligned_w, aligned_h, &mut decompressed).is_ok();
        }
        14 => {
            for (i, chunk) in safe_image_data.chunks_exact(4).enumerate().take(width * height) {
                decompressed[i] = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            }
            success = true;
        }
        24 => {
            success = texture2ddecoder::decode_bc6(safe_image_data, aligned_w, aligned_h, &mut decompressed, false).is_ok();
        }
        25 => {
            success = texture2ddecoder::decode_bc7(safe_image_data, aligned_w, aligned_h, &mut decompressed).is_ok();
        }
        26 => {
            success = texture2ddecoder::decode_bc4(safe_image_data, aligned_w, aligned_h, &mut decompressed).is_ok();
        }
        27 => {
            success = texture2ddecoder::decode_bc5(safe_image_data, aligned_w, aligned_h, &mut decompressed).is_ok();
        }
        30 | 31 => {
            success = texture2ddecoder::decode_pvrtc_2bpp(safe_image_data, aligned_w, aligned_h, &mut decompressed).is_ok();
        }
        32 | 33 => {
            success = texture2ddecoder::decode_pvrtc_4bpp(safe_image_data, aligned_w, aligned_h, &mut decompressed).is_ok();
        }
        34 | 60 | 61 => {
            success = texture2ddecoder::decode_etc1(safe_image_data, aligned_w, aligned_h, &mut decompressed).is_ok();
        }
        35 => {
            success = texture2ddecoder::decode_atc_rgb4(safe_image_data, aligned_w, aligned_h, &mut decompressed).is_ok();
        }
        36 => {
            success = texture2ddecoder::decode_atc_rgba8(safe_image_data, aligned_w, aligned_h, &mut decompressed).is_ok();
        }
        41 => {
            success = texture2ddecoder::decode_eacr(safe_image_data, aligned_w, aligned_h, &mut decompressed).is_ok();
        }
        42 => {
            success = texture2ddecoder::decode_eacr_signed(safe_image_data, aligned_w, aligned_h, &mut decompressed).is_ok();
        }
        43 => {
            success = texture2ddecoder::decode_eacrg(safe_image_data, aligned_w, aligned_h, &mut decompressed).is_ok();
        }
        44 => {
            success = texture2ddecoder::decode_eacrg_signed(safe_image_data, aligned_w, aligned_h, &mut decompressed).is_ok();
        }
        45 => {
            success = texture2ddecoder::decode_etc2_rgb(safe_image_data, aligned_w, aligned_h, &mut decompressed).is_ok();
        }
        46 => {
            success = texture2ddecoder::decode_etc2_rgba1(safe_image_data, aligned_w, aligned_h, &mut decompressed).is_ok();
        }
        47 => {
            success = texture2ddecoder::decode_etc2_rgba8(safe_image_data, aligned_w, aligned_h, &mut decompressed).is_ok();
        }
        48 | 54 | 66 => {
            success = texture2ddecoder::decode_astc(safe_image_data, aligned_w, aligned_h, 4, 4, &mut decompressed).is_ok();
        }
        49 | 55 | 67 => {
            success = texture2ddecoder::decode_astc(safe_image_data, aligned_w, aligned_h, 5, 5, &mut decompressed).is_ok();
        }
        50 | 56 | 68 => {
            success = texture2ddecoder::decode_astc(safe_image_data, aligned_w, aligned_h, 6, 6, &mut decompressed).is_ok();
        }
        51 | 57 | 69 => {
            success = texture2ddecoder::decode_astc(safe_image_data, aligned_w, aligned_h, 8, 8, &mut decompressed).is_ok();
        }
        52 | 58 | 70 => {
            success = texture2ddecoder::decode_astc(safe_image_data, aligned_w, aligned_h, 10, 10, &mut decompressed).is_ok();
        }
        53 | 59 | 71 => {
            success = texture2ddecoder::decode_astc(safe_image_data, aligned_w, aligned_h, 12, 12, &mut decompressed).is_ok();
        }
        _ => {
            if safe_image_data.len() == width * height * 4 {
                for (i, chunk) in safe_image_data.chunks_exact(4).enumerate() {
                    decompressed[i] = u32::from_le_bytes([chunk[2], chunk[1], chunk[0], chunk[3]]);
                }
                success = true;
            }
        }
    }
    if success {
        let mut bytes = Vec::with_capacity(width * height * 4);
        let mut has_strong_alpha = false;
        let mut non_zero_count = 0;
        for y in (0..height).rev() {
            for x in 0..width {
                let idx = y * aligned_w + x;
                let p = decompressed[idx];
                let b = p.to_le_bytes();
                bytes.push(b[2]);
                bytes.push(b[1]);
                bytes.push(b[0]);
                bytes.push(b[3]);
                if b[3] > 50 {
                    has_strong_alpha = true;
                }
                if b[3] > 15 {
                    non_zero_count += 1;
                }
            }
        }
        let total_pixels = width * height;
        let threshold = total_pixels / 200;
        if !has_strong_alpha || non_zero_count < threshold {
            for chunk in bytes.chunks_exact_mut(4) {
                chunk[3] = 255;
            }
        }
        Some(bytes)
    } else {
        None
    }
}
struct PosePartData {
    id: String,
    group_index: i32,
    link: Vec<String>,
}
fn extract_assets_internal(asset_manager: AssetManager, out_dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut extracted_files = Vec::new();
    let mut pose_parts = Vec::new();
    let mut moc_stem = None;
    for (asset_name, sf) in &asset_manager.files {
        for obj in &sf.objects {
            let class_id = obj.class_id;
            if class_id != 28 && class_id != 49 && class_id != 114 && class_id != 329 {
                continue;
            }
            let value = match asset_manager.read_object_value(asset_name, 0, obj.path_id) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let name = match value.get("m_Name") {
                Some(UnityValue::String(s)) if !s.is_empty() => s.clone(),
                _ => continue,
            };
            let base_name = Path::new(&name)
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or(name);
            let sanitized_base = base_name.replace(|c: char| !c.is_alphanumeric() && c != '_' && c != '-' && c != '.', "");
            if sanitized_base.is_empty() {
                continue;
            }
            let mut is_cubism_pose_part = false;
            if class_id == 114 {
                if let Some(class_name) = resolve_class_name(&value, &asset_manager, asset_name) {
                    if class_name == "CubismPosePart" {
                        is_cubism_pose_part = true;
                    }
                }
            }
            if is_cubism_pose_part {
                let group_index = match value.get("GroupIndex") {
                    Some(v) => v.as_i32().unwrap_or(0),
                    _ => 0,
                };
                let link = match value.get("Link") {
                    Some(UnityValue::Array(arr)) => {
                        arr.iter().map(|v| v.as_str().unwrap_or("").to_string()).collect::<Vec<_>>()
                    }
                    _ => Vec::new(),
                };
                let mut go_name = String::new();
                if let Some(go_pptr) = value.get("m_GameObject") {
                    if let UnityValue::PPtr { file_id, path_id } = go_pptr {
                        if let Ok(go_val) = asset_manager.read_object_value(asset_name, *file_id, *path_id) {
                            if let Some(UnityValue::String(name)) = go_val.get("m_Name") {
                                go_name = name.clone();
                            }
                        }
                    }
                }
                if !go_name.is_empty() {
                    pose_parts.push(PosePartData {
                        id: go_name,
                        group_index,
                        link,
                    });
                }
                continue;
            }
            if class_id == 49 || class_id == 114 {
                let mut content = None;
                let mut is_cubism_fade_motion = false;
                let mut is_cubism_expression = false;
                if class_id == 49 {
                    content = match value.get("m_Script") {
                        Some(UnityValue::String(s)) => Some(s.as_bytes().to_vec()),
                        Some(UnityValue::Bytes(b)) => Some(b.clone()),
                        _ => None,
                    };
                } else {
                    if let Some(class_name) = resolve_class_name(&value, &asset_manager, asset_name) {
                        if class_name == "CubismFadeMotionData" {
                            is_cubism_fade_motion = true;
                        } else if class_name == "CubismExpressionData" {
                            is_cubism_expression = true;
                        }
                    }

                    if is_cubism_fade_motion {
                        if let Some(json_val) = convert_fade_motion_to_json(&value) {
                            if let Ok(bytes) = serde_json::to_vec_pretty(&json_val) {
                                content = Some(bytes);
                            }
                        }
                    } else if is_cubism_expression {
                        if let Some(json_val) = convert_expression_data_to_json(&value) {
                            if let Ok(bytes) = serde_json::to_vec_pretty(&json_val) {
                                content = Some(bytes);
                            }
                        }
                    }
                    if content.is_none() {
                        let raw_bytes = &sf.data[obj.byte_start .. obj.byte_start + obj.byte_size];
                        if let Some(moc_pos) = raw_bytes.windows(4).position(|window| window == b"MOC3") {
                            if moc_pos >= 4 {
                                let size_bytes = &raw_bytes[moc_pos - 4 .. moc_pos];
                                let size = if sf.endian == crate::reader::Endian::Big {
                                    u32::from_be_bytes([size_bytes[0], size_bytes[1], size_bytes[2], size_bytes[3]]) as usize
                                } else {
                                    u32::from_le_bytes([size_bytes[0], size_bytes[1], size_bytes[2], size_bytes[3]]) as usize
                                };
                                if moc_pos + size <= raw_bytes.len() {
                                    content = Some(raw_bytes[moc_pos .. moc_pos + size].to_vec());
                                } else {
                                    content = Some(raw_bytes[moc_pos..].to_vec());
                                }
                            } else {
                                content = Some(raw_bytes[moc_pos..].to_vec());
                            }
                        }
                    }
                    if content.is_none() {
                        for key in &["_bytes", "m_Bytes", "bytes", "m_Data", "_data"] {
                            if let Some(val) = value.get(*key) {
                                if let UnityValue::Bytes(b) = val {
                                    content = Some(b.clone());
                                    break;
                                }
                            }
                        }
                    }
                    if content.is_none() {
                        if let UnityValue::Map(map) = &value {
                            for (_, val) in map {
                                if let UnityValue::Bytes(b) = val {
                                    if b.starts_with(b"MOC3") {
                                        content = Some(b.clone());
                                        break;
                                    }
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
                    let dest = out_dir.join(&final_name);
                    let mut final_data = data;
                    if final_name.ends_with(".model3.json") {
                        if let Ok(mut json_val) = serde_json::from_slice::<serde_json::Value>(&final_data) {
                            flatten_json_paths(&mut json_val);
                            if let Ok(serialized) = serde_json::to_vec_pretty(&json_val) {
                                final_data = serialized;
                            }
                        }
                    }
                    if let Ok(mut f) = File::create(&dest) {
                        if f.write_all(&final_data).is_ok() {
                            extracted_files.push(dest.clone());
                            if final_name.ends_with(".moc3") {
                                moc_stem = Some(sanitized_base.replace(".moc3", "").replace(".moc", ""));
                            }
                        }
                    }
                }
            } else if class_id == 28 {
                let width = match value.get("m_Width") {
                    Some(v) => i32::try_from_unity_value(v).unwrap_or(0) as usize,
                    _ => 0,
                };
                let height = match value.get("m_Height") {
                    Some(v) => i32::try_from_unity_value(v).unwrap_or(0) as usize,
                    _ => 0,
                };
                let format = match value.get("m_TextureFormat") {
                    Some(v) => i32::try_from_unity_value(v).unwrap_or(0),
                    _ => 0,
                };
                if width == 0 || height == 0 {
                    continue;
                }
                let mut image_data = Vec::new();
                let mut has_stream = false;
                if let Some(UnityValue::Map(stream_map)) = value.get("m_StreamData") {
                    let offset = stream_map.get("offset").and_then(|v| match v {
                        UnityValue::UInt64(o) => Some(*o),
                        UnityValue::Int64(o) => Some(*o as u64),
                        UnityValue::UInt32(o) => Some(*o as u64),
                        UnityValue::Int32(o) => Some(*o as u64),
                        _ => None,
                    });
                    let size = stream_map.get("size").and_then(|v| match v {
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
                    if let Some(UnityValue::Bytes(b)) = value.get("image_data").or_else(|| value.get("image data")) {
                        image_data = b.clone();
                    }
                }
                if image_data.is_empty() {
                    continue;
                }
                let decompressed = decompress_texture(width, height, format, &image_data);
                if let Some(rgba_data) = decompressed {
                    let final_name = if sanitized_base.ends_with(".png") {
                        sanitized_base.clone()
                    } else {
                        format!("{}.png", sanitized_base)
                    };
                    let dest = out_dir.join(&final_name);
                    if save_png(&dest, &rgba_data, width as u32, height as u32).is_ok() {
                        extracted_files.push(dest);
                    }
                }
            } else if class_id == 329 {
                if let Ok(video_clip) = crate::classes::videoclip::VideoClip::from_value(value.clone()) {
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
                                .and_then(|p| std::path::Path::new(p).extension())
                                .and_then(|s| s.to_str())
                                .unwrap_or("mp4");
                            let dot_ext = format!(".{}", ext);
                            let final_name = if sanitized_base.ends_with(&dot_ext) {
                                sanitized_base.clone()
                            } else {
                                format!("{}{}", sanitized_base, dot_ext)
                            };
                            let dest = out_dir.join(&final_name);
                            if let Ok(mut f) = File::create(&dest) {
                                if f.write_all(&video_data).is_ok() {
                                    extracted_files.push(dest);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    if !pose_parts.is_empty() {
        let stem = moc_stem.unwrap_or_else(|| "model".to_string());
        let dest = out_dir.join(format!("{}.pose3.json", stem));
        let mut group_map: std::collections::BTreeMap<i32, Vec<serde_json::Value>> = std::collections::BTreeMap::new();
        for part in pose_parts {
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
            if let Ok(mut f) = File::create(&dest) {
                if f.write_all(&serialized).is_ok() {
                    extracted_files.push(dest);
                }
            }
        }
    }
    Ok(extracted_files)
}
fn save_png<P: AsRef<Path>>(path: P, data: &[u8], width: u32, height: u32) -> Result<(), String> {
    let file = File::create(path).map_err(|e| e.to_string())?;
    let w = std::io::BufWriter::new(file);
    let mut encoder = png::Encoder::new(w, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(data).map_err(|e| e.to_string())?;
    Ok(())
}
pub fn extract_unity_assets<P: AsRef<Path>>(bundle_bytes: &[u8], out_dir: P) -> Result<Vec<PathBuf>, String> {
    let out_dir = out_dir.as_ref();
    fs::create_dir_all(out_dir).map_err(|e| e.to_string())?;
    let mut reader = Reader::new(bundle_bytes.to_vec(), crate::unity_version::UnityVersion::default());
    let bundle = Bundle::read(&mut reader).map_err(|e| format!("Failed to read bundle: {}", e))?;
    let mut asset_manager = AssetManager::new();
    for entry in bundle.files {
        if entry.name.ends_with(".resS") || entry.name.ends_with(".resource") {
            asset_manager.add_raw_file(entry.name, entry.data);
        } else if entry.data.len() > 20 {
            let mut sf_reader = Reader::new(entry.data, bundle.engine_version.clone());
            let sf = SerializedFile::read(&mut sf_reader);
            asset_manager.add_file(entry.name, sf);
        }
    }
    extract_assets_internal(asset_manager, out_dir)
}
pub fn extract_unity_assets_from_path<P: AsRef<Path>, Q: AsRef<Path>>(bundle_path: P, out_dir: Q) -> Result<Vec<PathBuf>, String> {
    let out_dir = out_dir.as_ref();
    fs::create_dir_all(out_dir).map_err(|e| e.to_string())?;
    let file = File::open(bundle_path).map_err(|e| format!("Failed to open bundle file: {}", e))?;
    let mmap = unsafe { memmap2::Mmap::map(&file).map_err(|e| format!("Failed to mmap bundle file: {}", e))? };
    let mut reader = Reader::new_mmap(mmap, crate::unity_version::UnityVersion::default());
    let bundle = Bundle::read(&mut reader).map_err(|e| format!("Failed to read bundle: {}", e))?;
    let mut asset_manager = AssetManager::new();
    for entry in bundle.files {
        if entry.name.ends_with(".resS") || entry.name.ends_with(".resource") {
            asset_manager.add_raw_file(entry.name, entry.data);
        } else if entry.data.len() > 20 {
            let mut sf_reader = Reader::new(entry.data, bundle.engine_version.clone());
            let sf = SerializedFile::read(&mut sf_reader);
            asset_manager.add_file(entry.name, sf);
        }
    }
    extract_assets_internal(asset_manager, out_dir)
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
fn get_mono_behaviour_class_name(asset_manager: &AssetManager, source_file_name: &str, m_script_pptr: &UnityValue) -> Option<String> {
    if let UnityValue::PPtr { file_id, path_id } = m_script_pptr {
        if let Ok(script_val) = asset_manager.read_object_value(source_file_name, *file_id, *path_id) {
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
    force_bezier: bool,
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
    } else if pre_curve.out_slope == 0.0 && curve.in_slope.abs() < 0.0001 && !force_bezier {
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
