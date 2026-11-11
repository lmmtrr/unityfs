use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use crate::assets::AssetManager;
use crate::value::UnityValue;
use crate::export_common as common;
use crate::classes::TryFromUnityValue;

const SWITCH_PLATFORM: i32 = 38;

const SWITCH_GOB_WIDTH: usize = 4;

const SWITCH_GOB_HEIGHT: usize = 8;

const SWITCH_TEXEL_SIZE: usize = 16;

const EAC_MODIFIERS: [[i32; 8]; 16] = [
    [-3, -6, -9, -15, 2, 5, 8, 14],
    [-3, -7, -10, -13, 2, 6, 9, 12],
    [-2, -5, -8, -13, 1, 4, 7, 12],
    [-2, -4, -6, -13, 1, 3, 5, 12],
    [-3, -6, -8, -12, 2, 5, 7, 11],
    [-3, -7, -9, -11, 2, 6, 8, 10],
    [-4, -7, -8, -11, 3, 6, 7, 10],
    [-3, -5, -8, -11, 2, 4, 7, 10],
    [-2, -6, -8, -10, 1, 5, 7, 9],
    [-2, -5, -8, -10, 1, 4, 7, 9],
    [-2, -4, -8, -10, 1, 3, 7, 9],
    [-2, -5, -7, -10, 1, 4, 6, 9],
    [-3, -4, -7, -10, 2, 3, 6, 9],
    [-1, -2, -3, -10, 0, 1, 2, 9],
    [-4, -6, -8, -9, 3, 5, 7, 8],
    [-3, -5, -7, -9, 2, 4, 6, 8],
];

const EAC_TEXEL_ORDER: [usize; 16] = [15, 11, 7, 3, 14, 10, 6, 2, 13, 9, 5, 1, 12, 8, 4, 0];

pub fn is_unity_bundle(data: &[u8]) -> bool {
    data.starts_with(b"UnityFS") || data.starts_with(b"UnityWeb") || data.starts_with(b"UnityRaw")
}

pub fn decompress_texture(width: usize, height: usize, format: i32, image_data: &[u8], version: &crate::unity_version::UnityVersion, flip_y: bool) -> Option<Vec<u8>> {
    std::panic::catch_unwind(|| decode_texture(width, height, format, image_data, version, flip_y)).ok().flatten()
}

fn expand_rgb565(color: u16) -> [u32; 3] {
    let (r, g, b) = ((color >> 11) as u32 & 0x1F, (color >> 5) as u32 & 0x3F, color as u32 & 0x1F);
    [(r << 3) | (r >> 2), (g << 2) | (g >> 4), (b << 3) | (b >> 2)]
}

fn restore_four_color_blocks(data: &[u8], stride: usize, pixels: &mut [u32]) {
    let columns = (stride / 4).max(1);
    for (index, block) in data.chunks_exact(16).enumerate() {
        let c0 = u16::from_le_bytes([block[8], block[9]]);
        let c1 = u16::from_le_bytes([block[10], block[11]]);
        if c0 > c1 {
            continue;
        }
        let (a, b) = (expand_rgb565(c0), expand_rgb565(c1));
        let palette = [a, b, std::array::from_fn(|i| (2 * a[i] + b[i]) / 3), std::array::from_fn(|i| (a[i] + 2 * b[i]) / 3)];
        let selectors = u32::from_le_bytes([block[12], block[13], block[14], block[15]]);
        for texel in 0..16 {
            let at = (index / columns * 4 + texel / 4) * stride + index % columns * 4 + texel % 4;
            let Some(pixel) = pixels.get_mut(at) else { continue };
            let [r, g, b] = palette[(selectors >> (texel * 2)) as usize & 3];
            *pixel = (*pixel & 0xFF000000) | (r << 16) | (g << 8) | b;
        }
    }
}

fn restore_atc_middle_colors(data: &[u8], stride: usize, block_size: usize, pixels: &mut [u32]) {
    let columns = (stride / 4).max(1);
    let expand = |value: u32, bits: u32| {
        let shifted = value << (8 - bits);
        shifted | (shifted >> bits)
    };
    for (index, block) in data.chunks_exact(block_size).enumerate() {
        let color = &block[block_size - 8..];
        let c0 = u16::from_le_bytes([color[0], color[1]]) as u32;
        let c1 = u16::from_le_bytes([color[2], color[3]]) as u32;
        if c0 & 0x8000 == 0 {
            continue;
        }
        let high = [expand(c0 & 0x1F, 5), expand((c0 >> 5) & 0x1F, 5), expand((c0 >> 10) & 0x1F, 5)];
        let low = [expand(c1 & 0x1F, 5), expand((c1 >> 5) & 0x3F, 6), expand((c1 >> 11) & 0x1F, 5)];
        let middle: [u32; 3] = std::array::from_fn(|i| high[i].saturating_sub(low[i] / 4));
        let selectors = u32::from_le_bytes([color[4], color[5], color[6], color[7]]);
        for texel in 0..16 {
            if (selectors >> (texel * 2)) & 3 != 1 {
                continue;
            }
            let at = (index / columns * 4 + texel / 4) * stride + index % columns * 4 + texel % 4;
            if let Some(pixel) = pixels.get_mut(at) {
                *pixel = (*pixel & 0xFF000000) | (middle[2] << 16) | (middle[1] << 8) | middle[0];
            }
        }
    }
}

fn decode_eac_channel(block: &[u8], signed: bool) -> [u32; 16] {
    let multiplier = match (block[1] >> 1) & 0x78 {
        0 => 1,
        value => value as i32,
    };
    let modifiers = EAC_MODIFIERS[(block[1] & 0xF) as usize];
    let base = if signed { block[0] as i8 as i32 * 8 + 1023 } else { block[0] as i32 * 8 + 4 };
    let mut bits = u64::from_be_bytes([block[0], block[1], block[2], block[3], block[4], block[5], block[6], block[7]]);
    let mut channel = [0; 16];
    for texel in EAC_TEXEL_ORDER {
        channel[texel] = ((base + multiplier * modifiers[bits as usize & 7]).clamp(0, 2047) >> 3) as u32;
        bits >>= 3;
    }
    channel
}

fn decode_eac(data: &[u8], width: usize, height: usize, two_channels: bool, signed: bool, pixels: &mut [u32]) -> bool {
    let block_size = if two_channels { 16 } else { 8 };
    let columns = (width / 4).max(1);
    let blocks = columns * (height / 4);
    if data.len() < blocks * block_size {
        return false;
    }
    for (index, block) in data.chunks_exact(block_size).take(blocks).enumerate() {
        let red = decode_eac_channel(&block[..8], signed);
        let green = if two_channels { decode_eac_channel(&block[8..], signed) } else { [0; 16] };
        for texel in 0..16 {
            let at = (index / columns * 4 + texel / 4) * width + index % columns * 4 + texel % 4;
            if let Some(pixel) = pixels.get_mut(at) {
                *pixel = 0xFF000000 | (red[texel] << 16) | (green[texel] << 8);
            }
        }
    }
    true
}

fn decode_texture(width: usize, height: usize, format: i32, image_data: &[u8], version: &crate::unity_version::UnityVersion, flip_y: bool) -> Option<Vec<u8>> {
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
                decompressed[i] = (a as u32) << 24;
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
                let r8 = (r as u32 * 255 / 31) as u8;
                let g8 = (g as u32 * 255 / 63) as u8;
                let b8 = (b as u32 * 255 / 31) as u8;
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
            success = texture2ddecoder::decode_bc1a(safe_image_data, aligned_w, aligned_h, &mut decompressed).is_ok();
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
        6 | 17 | 20 => {
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
                decompressed[i] = u32::from_le_bytes(if format == 6 { [r, g, b, a] } else { [b, g, r, a] });
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
            for pixel in &mut decompressed {
                *pixel = 0xFF000000 | (((*pixel >> 16) & 0xFF) * 0x010101);
            }
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
            success = decode_eac(safe_image_data, aligned_w, aligned_h, false, false, &mut decompressed);
        }
        42 => {
            success = decode_eac(safe_image_data, aligned_w, aligned_h, false, true, &mut decompressed);
        }
        43 => {
            success = decode_eac(safe_image_data, aligned_w, aligned_h, true, false, &mut decompressed);
        }
        44 => {
            success = decode_eac(safe_image_data, aligned_w, aligned_h, true, true, &mut decompressed);
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
    if success && matches!(format, 11 | 12) {
        restore_four_color_blocks(safe_image_data, aligned_w, &mut decompressed);
    }
    if success && matches!(format, 35 | 36) {
        restore_atc_middle_colors(safe_image_data, aligned_w, if format == 35 { 8 } else { 16 }, &mut decompressed);
    }
    if success {
        let mut bytes = Vec::with_capacity(width * height * 4);
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
            }
        }
        Some(bytes)
    } else {
        None
    }
}

fn switch_gobs_per_block(platform: i32, value: &UnityValue) -> Option<usize> {
    if platform != SWITCH_PLATFORM {
        return None;
    }
    let blob: Vec<u8> = match value.get("m_PlatformBlob")? {
        UnityValue::Bytes(bytes) => bytes.clone(),
        UnityValue::Array(items) => items.iter().filter_map(|v| v.as_i64().map(|b| b as u8)).collect(),
        _ => return None,
    };
    let shift = u32::from_le_bytes(blob.get(8..12)?.try_into().ok()?);
    (1..=5).contains(&shift).then_some(1 << shift)
}

fn switch_block_size(format: i32) -> Option<(usize, usize)> {
    Some(match format {
        1 | 63 => (16, 1),
        2 | 7 | 9 | 13 | 62 => (8, 1),
        4 | 5 | 14 => (4, 1),
        6 => (1, 1),
        10 | 26 => (8, 4),
        12 | 24 | 25 | 27 | 48 | 54 => (4, 4),
        49 | 55 => (5, 5),
        50 | 56 => (6, 6),
        51 | 57 => (8, 8),
        52 | 58 => (10, 10),
        53 | 59 => (12, 12),
        _ => return None,
    })
}

fn switch_deswizzle(data: &[u8], width: usize, height: usize, block: (usize, usize), gobs_per_block: usize) -> Vec<u8> {
    let block_count_x = width.div_ceil(block.0);
    let block_count_y = height.div_ceil(block.1);
    let mut out = vec![0; data.len()];
    let mut texels = data.chunks_exact(SWITCH_TEXEL_SIZE);
    for i in 0..block_count_y / SWITCH_GOB_HEIGHT / gobs_per_block {
        for j in 0..block_count_x / SWITCH_GOB_WIDTH {
            for k in 0..gobs_per_block {
                let base_y = (i * gobs_per_block + k) * SWITCH_GOB_HEIGHT;
                for v in 0..SWITCH_GOB_WIDTH * SWITCH_GOB_HEIGHT {
                    let gob_x = ((v >> 3) & 0b10) | ((v >> 1) & 0b1);
                    let gob_y = ((v >> 1) & 0b110) | (v & 0b1);
                    let Some(texel) = texels.next() else { return out };
                    let dst = ((base_y + gob_y) * block_count_x + j * SWITCH_GOB_WIDTH + gob_x) * SWITCH_TEXEL_SIZE;
                    if let Some(slot) = out.get_mut(dst..dst + SWITCH_TEXEL_SIZE) {
                        slot.copy_from_slice(texel);
                    }
                }
            }
        }
    }
    out
}

pub fn decompress_unity_texture(value: &UnityValue, image_data: &[u8], version: &crate::unity_version::UnityVersion, platform: i32, flip_y: bool) -> Option<Vec<u8>> {
    let field = |key: &str| value.get(key).and_then(|v| v.as_i32()).unwrap_or(0);
    let (width, height, format) = (field("m_Width") as usize, field("m_Height") as usize, field("m_TextureFormat"));
    let Some(gobs_per_block) = switch_gobs_per_block(platform, value) else {
        return decompress_texture(width, height, format, image_data, version, flip_y);
    };
    let format = match format {
        3 => 4,
        8 => 14,
        other => other,
    };
    let block = switch_block_size(format)?;
    let padded_width = width.div_ceil(block.0 * SWITCH_GOB_WIDTH) * block.0 * SWITCH_GOB_WIDTH;
    let padded_height = height.div_ceil(block.1 * SWITCH_GOB_HEIGHT * gobs_per_block) * block.1 * SWITCH_GOB_HEIGHT * gobs_per_block;
    let deswizzled = switch_deswizzle(image_data, padded_width, padded_height, block, gobs_per_block);
    let pixels = decompress_texture(padded_width, padded_height, format, &deswizzled, version, false)?;
    let mut out = Vec::with_capacity(width * height * 4);
    for row in 0..height {
        let y = if flip_y { height - 1 - row } else { row };
        let start = y * padded_width * 4;
        out.extend_from_slice(&pixels[start..start + width * 4]);
    }
    Some(out)
}

pub(crate) fn sanitized_base_name(name: &str) -> String {
    let base_name = Path::new(name).file_name().map_or_else(|| name.to_string(), |f| f.to_string_lossy().to_string());
    base_name.replace(|c: char| !c.is_alphanumeric() && c != '_' && c != '-' && c != '.', "")
}

pub fn texture_file_name(name: &str) -> Option<String> {
    let base = sanitized_base_name(name);
    if base.is_empty() {
        None
    } else if base.ends_with(".png") {
        Some(base)
    } else {
        Some(format!("{}.png", base))
    }
}

#[derive(Default)]
struct ExtractedFiles {
    claimed: HashSet<String>,
    files: Vec<PathBuf>,
}

impl ExtractedFiles {
    fn write(&mut self, dir: &Path, name: &str, data: &[u8]) -> bool {
        let mut dest = dir.join(name);
        let mut counter = 2;
        while !self.claimed.insert(dest.to_string_lossy().to_lowercase()) {
            if fs::read(&dest).is_ok_and(|existing| existing == data) {
                return true;
            }
            dest = dir.join(common::numbered_name(name, counter));
            counter += 1;
        }
        let written = fs::write(&dest, data).is_ok();
        if written {
            self.files.push(dest);
        }
        written
    }
}

fn extract_assets_internal(asset_manager: AssetManager, out_dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut extracted = ExtractedFiles::default();
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
            let sanitized_base = sanitized_base_name(&name);
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
                            extracted.write(&cri_dir, &common::criware_filename(&sanitized_base, &key, ext), &data);
                        }
                    }
                    common::monobehaviour_content(&value, &asset_manager, asset_name, obj)
                };
                if let Some((data, kind)) = content {
                    let final_name = common::content_filename(&sanitized_base, &data, kind);
                    let final_data = common::finalize_content(&final_name, data);
                    if extracted.write(out_dir, &final_name, &final_data) && final_name.ends_with(".moc3") {
                        moc_stem = Some(common::moc_stem(&sanitized_base));
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
                if width == 0 || height == 0 {
                    continue;
                }
                let image_data = common::texture_image(&value, &asset_manager);
                if image_data.is_empty() {
                    continue;
                }
                let decompressed = decompress_unity_texture(&value, &image_data, &sf.unity_version, sf.target_platform, true);
                if let (Some(rgba_data), Some(final_name)) = (decompressed, texture_file_name(&name)) {
                    if let Ok(png) = encode_png(&rgba_data, width as u32, height as u32) {
                        extracted.write(out_dir, &final_name, &png);
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
                    extracted.write(out_dir, &final_name, &video_data);
                }
            }
        }
    }
    if !pose_parts.is_empty() {
        let name = format!("{}.pose3.json", moc_stem.unwrap_or_else(|| "model".to_string()));
        if let Some(serialized) = common::pose_json(pose_parts) {
            extracted.write(out_dir, &name, &serialized);
        }
    }
    Ok(extracted.files)
}

fn encode_png(data: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(data).map_err(|e| e.to_string())?;
    writer.finish().map_err(|e| e.to_string())?;
    Ok(out)
}

pub fn is_unity_file(data: &[u8]) -> bool {
    is_unity_bundle(data) || crate::metadata::looks_like_serialized_file(data) || crate::metadata::looks_like_old_serialized_file(data, data.len() as u64)
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

