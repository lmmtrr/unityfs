use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use crate::reader::{ByteSource, Endian, Reader};
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
        let bundle = BundleFile::open(reader.data.clone())?;
        let mut files = Vec::with_capacity(bundle.nodes.len());
        for i in 0..bundle.nodes.len() {
            files.push(BundleEntry {
                name: bundle.nodes[i].name.clone(),
                data: bundle.read_node(i)?,
            });
        }
        Ok(Self { engine_version: bundle.engine_version, files })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct StorageBlock {
    pub uncompressed_size: u32,
    pub compressed_size: u32,
    pub flags: u16,
    pub uncompressed_offset: u64,
    pub compressed_offset: u64,
}

#[derive(Clone, Debug)]
pub struct BundleNode {
    pub offset: u64,
    pub size: u64,
    pub flags: u32,
    pub name: String,
}

impl BundleNode {
    pub fn is_raw_stream(&self) -> bool {
        self.name.ends_with(".resS") || self.name.ends_with(".resource")
    }
}

pub struct BundleFile {
    pub source: ByteSource,
    pub engine_version: UnityVersion,
    pub blocks: Vec<StorageBlock>,
    pub nodes: Vec<BundleNode>,
    cache: Mutex<HashMap<usize, Arc<Vec<u8>>>>,
}

impl BundleFile {
    pub fn open(source: ByteSource) -> Result<Self, String> {
        let mut reader = Reader {
            data: source.clone(),
            pos: 0,
            endian: Endian::Big,
            unity_version: UnityVersion::default(),
        };
        let signature = reader.read_bytes(8);
        if signature.len() < 7 || &signature[..7] != b"UnityFS" {
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
        let info_at_end = flags & 0x80 != 0;
        let info_pos = if info_at_end {
            (total_size as usize).saturating_sub(compressed_size as usize)
        } else {
            reader.pos
        };
        let header_end = reader.pos;
        let info_end = info_pos + compressed_size as usize;
        if info_end > source.len() {
            return Err("Truncated bundle: blocks info out of range".to_string());
        }
        let blocks_info_data = &source[info_pos..info_end];
        let is_old_dataflags = engine_version.major() < 2020
            || (engine_version.major() == 2020 && engine_version < (2020, 3, 34))
            || (engine_version.major() == 2021 && engine_version < (2021, 3, 2))
            || (engine_version.major() == 2022 && engine_version < (2022, 1, 1));
        let need_padding = !is_old_dataflags && (flags & 0x200 != 0);
        let mut data_start = if info_at_end { header_end } else { info_end };
        if !info_at_end && need_padding {
            data_start = (data_start + 15) & !15;
        }
        let info = decompress_block(blocks_info_data, flags as u16, uncompressed_size as usize)
            .map_err(|e| format!("blocks info: {}", e))?;
        let mut br = Reader::new(info, engine_version.clone());
        br.endian = Endian::Big;
        let _hash = br.read_bytes(16);
        let block_count = br.read_i32().max(0) as usize;
        if block_count > br.data.len().saturating_sub(br.pos) / 10 {
            return Err("Truncated bundle: block table out of range".to_string());
        }
        let mut blocks = Vec::with_capacity(block_count);
        let (mut upos, mut cpos) = (0u64, data_start as u64);
        for _ in 0..block_count {
            let u = br.read_u32();
            let c = br.read_u32();
            let f = br.read_u16();
            blocks.push(StorageBlock {
                uncompressed_size: u,
                compressed_size: c,
                flags: f,
                uncompressed_offset: upos,
                compressed_offset: cpos,
            });
            upos += u as u64;
            cpos += c as u64;
        }
        if cpos as usize > source.len() {
            return Err("Truncated bundle: block data out of range".to_string());
        }
        let node_count = br.read_i32().max(0) as usize;
        if node_count > br.data.len().saturating_sub(br.pos) / 21 {
            return Err("Truncated bundle: node table out of range".to_string());
        }
        let mut nodes = Vec::with_capacity(node_count);
        for _ in 0..node_count {
            nodes.push(BundleNode {
                offset: br.read_i64() as u64,
                size: br.read_i64() as u64,
                flags: br.read_u32(),
                name: br.read_string_null(),
            });
        }
        Ok(Self { source, engine_version, blocks, nodes, cache: Mutex::new(HashMap::new()) })
    }

    pub fn open_path<P: AsRef<std::path::Path>>(path: P) -> Result<Self, String> {
        let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let mmap = unsafe { memmap2::Mmap::map(&file).map_err(|e| e.to_string())? };
        Self::open(ByteSource::Mmap(Arc::new(mmap)))
    }

    fn block_data(&self, index: usize) -> Result<Arc<Vec<u8>>, String> {
        if let Some(b) = self.cache.lock().unwrap().get(&index) {
            return Ok(b.clone());
        }
        let b = &self.blocks[index];
        let start = b.compressed_offset as usize;
        let raw = &self.source[start..start + b.compressed_size as usize];
        let data = Arc::new(
            decompress_block(raw, b.flags, b.uncompressed_size as usize)
                .map_err(|e| format!("block {}: {}", index, e))?,
        );
        self.cache.lock().unwrap().insert(index, data.clone());
        Ok(data)
    }

    pub fn read_range(&self, offset: u64, len: u64) -> Result<Vec<u8>, String> {
        let total = self.blocks.last().map_or(0, |b| b.uncompressed_offset + b.uncompressed_size as u64);
        let end = offset.checked_add(len).filter(|&e| e <= total).ok_or_else(|| format!("range {}+{} out of bundle data", offset, len))?;
        let mut out = Vec::with_capacity(len as usize);
        let first = self.blocks.partition_point(|b| b.uncompressed_offset + b.uncompressed_size as u64 <= offset);
        for i in first..self.blocks.len() {
            let b = self.blocks[i];
            if b.uncompressed_offset >= end {
                break;
            }
            let lo = offset.saturating_sub(b.uncompressed_offset) as usize;
            let hi = (end - b.uncompressed_offset).min(b.uncompressed_size as u64) as usize;
            if b.flags & 0x3F == 0 {
                let s = b.compressed_offset as usize;
                out.extend_from_slice(self.source.get(s + lo..s + hi).ok_or_else(|| format!("block {} out of bundle data", i))?);
            } else {
                out.extend_from_slice(self.block_data(i)?.get(lo..hi).ok_or_else(|| format!("block {} shorter than declared", i))?);
            }
        }
        if out.len() as u64 != len {
            return Err(format!("range {}+{} out of bundle data", offset, len));
        }
        Ok(out)
    }

    pub fn read_node(&self, index: usize) -> Result<Vec<u8>, String> {
        let n = &self.nodes[index];
        self.read_range(n.offset, n.size)
    }

    pub fn read_node_range(&self, index: usize, offset: u64, len: u64) -> Result<Vec<u8>, String> {
        let n = &self.nodes[index];
        let len = len.min(n.size.saturating_sub(offset));
        self.read_range(n.offset.saturating_add(offset), len)
    }

    pub fn clear_cache(&self) {
        self.cache.lock().unwrap().clear();
    }
}

pub fn decompress_block(data: &[u8], flags: u16, uncompressed_size: usize) -> Result<Vec<u8>, String> {
    match flags & 0x3F {
        0 => Ok(data.to_vec()),
        1 => decompress_lzma(data, uncompressed_size),
        2 | 3 => lz4_flex::decompress(data, uncompressed_size)
            .map_err(|e| format!("LZ4: {} (compressed={}, uncompressed={})", e, data.len(), uncompressed_size)),
        c => Err(format!("unsupported compression type {}", c)),
    }
}

fn decompress_lzma(data: &[u8], uncompressed_size: usize) -> Result<Vec<u8>, String> {
    if data.len() < 5 {
        return Err("LZMA: data too short".to_string());
    }
    let opts = lzma_rs::decompress::Options {
        unpacked_size: lzma_rs::decompress::UnpackedSize::UseProvided(Some(uncompressed_size as u64)),
        ..Default::default()
    };
    let mut out = Vec::with_capacity(uncompressed_size);
    lzma_rs::lzma_decompress_with_options(&mut &data[..], &mut out, &opts)
        .map_err(|e| format!("LZMA: {}", e))?;
    Ok(out)
}
