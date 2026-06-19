use crate::reader::{Reader, Endian};
use crate::unity_version::UnityVersion;
pub struct Bundle {
    pub engine_version: UnityVersion,
    pub files: Vec<BundleEntry>,
}
pub struct BundleEntry {
    pub name: String,
    pub data: Vec<u8>,
}
impl Bundle {
    pub fn read(reader: &mut Reader) -> Result<Self, String> {
        let signature = reader.read_bytes(8);
        if &signature[..7] != b"UnityFS" {
            return Err("Invalid bundle signature".to_string());
        }
        let version = reader.read_u32();
        let _version_player = reader.read_string_null();
        let engine_version_str = reader.read_string_null();
        let mut engine_version = engine_version_str.parse::<UnityVersion>().unwrap_or_default();
        if engine_version.is_empty() {
            engine_version = UnityVersion::new(2020, 3, 34, crate::unity_version::UnityVersionType::Final, 1);
        }
        let total_size = reader.read_i64();
        let compressed_size = reader.read_u32();
        let uncompressed_size = reader.read_u32();
        let flags = reader.read_u32();
        if version >= 7 || (engine_version.major() == 2019 && engine_version >= (2019, 4, 15)) {
            reader.align(16);
        }
        let info_pos = if flags & 0x80 != 0 {
            (total_size as usize).saturating_sub(compressed_size as usize)
        } else {
            reader.pos
        };
        let old_pos = reader.pos;
        reader.pos = info_pos;
        let blocks_info_data = reader.read_bytes(compressed_size as usize);
        let is_old_dataflags = engine_version.major() < 2020
            || (engine_version.major() == 2020 && engine_version < (2020, 3, 34))
            || (engine_version.major() == 2021 && engine_version < (2021, 3, 2))
            || (engine_version.major() == 2022 && engine_version < (2022, 1, 1));
        let need_padding = !is_old_dataflags && (flags & 0x200 != 0);
        if flags & 0x80 == 0 {
            if need_padding {
                reader.align(16);
            }
        } else {
            reader.pos = old_pos;
        }
        let compression_type = flags & 0x3F;
        let decompressed_blocks_info = match compression_type {
            1 => {
                let mut decompressed = Vec::new();
                lzma_rs::lzma_decompress(&mut &blocks_info_data[..], &mut decompressed)
                    .map_err(|e| format!("LZMA blocks info error: {}", e))?;
                decompressed
            }
            2 | 3 => {
                lz4_flex::decompress(&blocks_info_data, uncompressed_size as usize)
                    .map_err(|e| format!("LZ4 blocks info error: {}, compressed_size={}, uncompressed_size={}", e, compressed_size, uncompressed_size))?
            }
            _ => blocks_info_data,
        };
        let mut blocks_reader = Reader::new(decompressed_blocks_info, engine_version.clone());
        blocks_reader.endian = Endian::Big;
        let _hash = blocks_reader.read_bytes(16);
        let block_count = blocks_reader.read_i32();
        let mut blocks = Vec::new();
        for _i in 0..block_count {
            let u = blocks_reader.read_u32();
            let c = blocks_reader.read_u32();
            let f = blocks_reader.read_u16();
            blocks.push((u, c, f));
        }
        let entry_count = blocks_reader.read_i32();
        let mut entries = Vec::new();
        for _ in 0..entry_count {
            entries.push((
                blocks_reader.read_i64(),
                blocks_reader.read_i64(),
                blocks_reader.read_u32(),
                blocks_reader.read_string_null(),
            ));
        }
        let mut raw_data = Vec::with_capacity(uncompressed_size as usize);
        for (i, (u, c, f)) in blocks.iter().copied().enumerate() {
            let compressed_data = reader.read_bytes(c as usize);
            let block_compression = f & 0x3F;
            match block_compression {
                2 | 3 => {
                    let decompressed = lz4_flex::decompress(&compressed_data, u as usize)
                        .map_err(|e| format!("LZ4 block {} data error: {}, compressed_size={}, uncompressed_size={}", i, e, c, u))?;
                    raw_data.extend(decompressed);
                }
                _ => raw_data.extend(compressed_data),
            }
        }
        let mut bundle_entries = Vec::new();
        for (offset, size, _, name) in entries {
            let start = offset as usize;
            let end = start + size as usize;
            if end <= raw_data.len() {
                bundle_entries.push(BundleEntry {
                    name,
                    data: raw_data[start..end].to_vec(),
                });
            }
        }
        Ok(Self {
            engine_version,
            files: bundle_entries,
        })
    }
}
