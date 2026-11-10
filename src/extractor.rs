use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use crate::assets::AssetManager;
use crate::value::UnityValue;
use crate::export_common as common;
use crate::classes::TryFromUnityValue;

pub fn is_unity_bundle(data: &[u8]) -> bool {
    data.starts_with(b"UnityFS") || data.starts_with(b"UnityWeb") || data.starts_with(b"UnityRaw")
}

pub fn decompress_texture(width: usize, height: usize, format: i32, image_data: &[u8], version: &crate::unity_version::UnityVersion, flip_y: bool) -> Option<Vec<u8>> {
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
        28 | 29 if version.major() != 0 && *version < (2017, 3) => {
            success = texture2ddecoder::decode_crunch(image_data, width, height, &mut decompressed).is_ok();
        }
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
                                    decompressed[py * aligned_w + px] = u32::from_le_bytes([c[0], c[1], c[2], a8]);
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
        17 | 20 => {
            let bytes_per_channel = if format == 17 { 2 } else { 4 };
            let to_u8 = |c: &[u8]| -> u8 {
                let f = if bytes_per_channel == 2 {
                    crate::classes::mesh::f16_to_f32(u16::from_le_bytes([c[0], c[1]]))
                } else {
                    f32::from_le_bytes([c[0], c[1], c[2], c[3]])
                };
                (f.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
            };
            for (i, px) in safe_image_data.chunks_exact(bytes_per_channel * 4).enumerate().take(width * height) {
                let r = to_u8(&px[0..bytes_per_channel]);
                let g = to_u8(&px[bytes_per_channel..bytes_per_channel * 2]);
                let b = to_u8(&px[bytes_per_channel * 2..bytes_per_channel * 3]);
                let a = to_u8(&px[bytes_per_channel * 3..bytes_per_channel * 4]);
                decompressed[i] = u32::from_le_bytes([b, g, r, a]);
            }
            success = true;
        }
        13 => {
            for (i, chunk) in safe_image_data.chunks_exact(2).enumerate().take(width * height) {
                let val = u16::from_le_bytes([chunk[0], chunk[1]]);
                let r = ((val >> 12) & 0xF) as u8 * 17;
                let g = ((val >> 8) & 0xF) as u8 * 17;
                let b = ((val >> 4) & 0xF) as u8 * 17;
                let a = (val & 0xF) as u8 * 17;
                decompressed[i] = u32::from_le_bytes([b, g, r, a]);
            }
            success = true;
        }
        21 => {
            let yuv = |y: u8, u: u8, v: u8| -> [u8; 3] {
                let (c, d, e) = (y as i32 - 16, u as i32 - 128, v as i32 - 128);
                let clamp = |x: i32| ((x + 128) >> 8).clamp(0, 255) as u8;
                [clamp(298 * c + 409 * e), clamp(298 * c - 100 * d - 208 * e), clamp(298 * c + 516 * d)]
            };
            for (i, px) in safe_image_data.chunks_exact(4).enumerate() {
                for (k, y) in [px[0], px[2]].into_iter().enumerate() {
                    let at = i * 2 + k;
                    if at < width * height {
                        let [r, g, b] = yuv(y, px[1], px[3]);
                        decompressed[at] = u32::from_le_bytes([b, g, r, 255]);
                    }
                }
            }
            success = true;
        }
        22 => {
            for (i, px) in safe_image_data.chunks_exact(4).enumerate().take(width * height) {
                let n = u32::from_le_bytes([px[0], px[1], px[2], px[3]]);
                let scale = 2f32.powi((n >> 27) as i32 - 24);
                let to_u8 = |m: u32| (((m & 0x1FF) as f32 * scale).clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                decompressed[i] = u32::from_le_bytes([to_u8(n >> 18), to_u8(n >> 9), to_u8(n), 255]);
            }
            success = true;
        }
        9 | 15 | 16 | 18 | 19 | 62 | 63 | 72 | 73 | 74 | 75..=82 => {
            let (channels, channel_size) = match format {
                9 => (1, 2),
                15 => (1, 2),
                16 => (2, 2),
                18 => (1, 4),
                19 => (2, 4),
                62 => (2, 1),
                63 => (1, 1),
                72 => (2, 2),
                73 => (3, 2),
                74 => (4, 2),
                75 => (1, 1),
                76 => (2, 1),
                77 => (3, 1),
                78 => (4, 1),
                79 => (1, 2),
                80 => (2, 2),
                81 => (3, 2),
                _ => (4, 2),
            };
            let is_float = matches!(format, 15 | 16 | 18 | 19);
            let sign_bit = if format >= 75 { 0x80 } else { 0 };
            let to_u8 = |c: &[u8]| -> u8 {
                match (is_float, channel_size) {
                    (true, 2) => (crate::classes::mesh::f16_to_f32(u16::from_le_bytes([c[0], c[1]])).clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
                    (true, _) => (f32::from_le_bytes([c[0], c[1], c[2], c[3]]).clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
                    (false, _) => c[channel_size - 1] ^ sign_bit,
                }
            };
            for (i, px) in safe_image_data.chunks_exact(channels * channel_size).enumerate().take(width * height) {
                let mut rgba = [0u8, 0, 0, 255];
                for c in 0..channels {
                    rgba[c] = to_u8(&px[c * channel_size..(c + 1) * channel_size]);
                }
                decompressed[i] = u32::from_le_bytes([rgba[2], rgba[1], rgba[0], rgba[3]]);
            }
            success = true;
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
        _ => {}
    }
    if success {
        let mut bytes = Vec::with_capacity(width * height * 4);
        let mut has_strong_alpha = false;
        for row in 0..height {
            let y = if flip_y { height - 1 - row } else { row };
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
            }
        }
        if !has_strong_alpha {
            for chunk in bytes.chunks_exact_mut(4) {
                chunk[3] = 255;
            }
        }
        Some(bytes)
    } else {
        None
    }
}

fn extract_assets_internal(asset_manager: AssetManager, out_dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut extracted_files = Vec::new();
    let mut pose_parts = Vec::new();
    let mut moc_stem = None;
    for (asset_name, sf) in &asset_manager.files {
        for obj in &sf.objects {
            let class_id = obj.class_id;
            if class_id != 28 && class_id != 49 && class_id != 114 && class_id != 329 && class_id != 152 {
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
            if class_id == 114 && common::resolve_class_name(&value, &asset_manager, asset_name).as_deref() == Some("CubismPosePart") {
                pose_parts.extend(common::pose_part(&value, &asset_manager, asset_name));
                continue;
            }
            if class_id == 49 || class_id == 114 {
                let content = if class_id == 49 {
                    match value.get("m_Script") {
                        Some(UnityValue::String(s)) => Some((s.as_bytes().to_vec(), common::ContentKind::Other)),
                        Some(UnityValue::Bytes(b)) => Some((b.clone(), common::ContentKind::Other)),
                        _ => None,
                    }
                } else {
                    let mut blobs = Vec::new();
                    common::criware_blobs(&value, "", &mut blobs);
                    if !blobs.is_empty() {
                        let cri_dir = out_dir.join("CriWare");
                        let _ = std::fs::create_dir_all(&cri_dir);
                        for (key, ext, data) in blobs {
                            let dest = cri_dir.join(common::criware_filename(&sanitized_base, &key, ext));
                            if std::fs::write(&dest, &data).is_ok() {
                                extracted_files.push(dest);
                            }
                        }
                    }
                    common::monobehaviour_content(&value, &asset_manager, asset_name, obj)
                };
                if let Some((data, kind)) = content {
                    let final_name = common::content_filename(&sanitized_base, &data, kind);
                    let final_data = common::finalize_content(&final_name, data);
                    let dest = out_dir.join(&final_name);
                    if std::fs::write(&dest, &final_data).is_ok() {
                        extracted_files.push(dest);
                        if final_name.ends_with(".moc3") {
                            moc_stem = Some(common::moc_stem(&sanitized_base));
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
                let image_data = common::texture_image(&value, &asset_manager);
                if image_data.is_empty() {
                    continue;
                }
                let decompressed = decompress_texture(width, height, format, &image_data, &sf.unity_version, true);
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
            } else if class_id == 329 || class_id == 152 {
                let video_data = crate::classes::videoclip::video_bytes(&value, &asset_manager);
                if !video_data.is_empty() {
                    let dot_ext = format!(".{}", crate::classes::videoclip::extension(&video_data, &value));
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
    if !pose_parts.is_empty() {
        let dest = out_dir.join(format!("{}.pose3.json", moc_stem.unwrap_or_else(|| "model".to_string())));
        if let Some(serialized) = common::pose_json(pose_parts) {
            if std::fs::write(&dest, &serialized).is_ok() {
                extracted_files.push(dest);
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

pub fn is_unity_file(data: &[u8]) -> bool {
    is_unity_bundle(data) || crate::metadata::looks_like_serialized_file(data)
}

pub fn extract_unity_assets<P: AsRef<Path>>(bundle_bytes: &[u8], out_dir: P) -> Result<Vec<PathBuf>, String> {
    let out_dir = out_dir.as_ref();
    fs::create_dir_all(out_dir).map_err(|e| e.to_string())?;
    extract_unity_assets_with_resources(bundle_bytes, "data.assets", Vec::new(), out_dir)
}

pub fn extract_unity_assets_with_resources<P: AsRef<Path>>(data: &[u8], name: &str, resources: Vec<(String, Vec<u8>)>, out_dir: P) -> Result<Vec<PathBuf>, String> {
    let out_dir = out_dir.as_ref();
    fs::create_dir_all(out_dir).map_err(|e| e.to_string())?;
    let mut asset_manager = AssetManager::from_bytes(data.to_vec(), name).map_err(|e| format!("Failed to read file: {}", e))?;
    for (resource_name, resource_data) in resources {
        let resource_name = resource_name.rsplit(['/', '\\']).next().unwrap_or(&resource_name).to_string();
        asset_manager.add_raw_file(resource_name, resource_data);
    }
    extract_assets_internal(asset_manager, out_dir)
}

pub fn extract_unity_assets_from_path<P: AsRef<Path>, Q: AsRef<Path>>(bundle_path: P, out_dir: Q) -> Result<Vec<PathBuf>, String> {
    let out_dir = out_dir.as_ref();
    fs::create_dir_all(out_dir).map_err(|e| e.to_string())?;
    let asset_manager = AssetManager::open_path(bundle_path).map_err(|e| format!("Failed to read file: {}", e))?;
    extract_assets_internal(asset_manager, out_dir)
}

