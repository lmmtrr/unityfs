use std::borrow::Cow;
use std::io::Read;
use crate::bundle::BundleFile;
use crate::extractor::is_unity_bundle;
use crate::reader::ByteSource;
use crate::unity_version::UnityVersion;

const MAX_DEPTH: usize = 8;

const GZIP_MAGIC: &[u8] = b"\x1f\x8b";

const BROTLI_MAGIC: &[u8] = b"brotli";

const BROTLI_MAGIC_OFFSET: usize = 0x20;

const WEB_SIGNATURES: [&[u8]; 2] = [b"UnityWebData", b"TuanjieWebData"];

const NON_ASSET_SUFFIXES: [&str; 5] = [".resS", ".resource", ".config", ".xml", ".dat"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Bundle,
    Web,
    Assets,
    Raw,
}

pub struct Entry {
    pub name: String,
    pub data: ByteSource,
    pub version: UnityVersion,
    pub kind: FileKind,
}

fn has_web_signature(data: &[u8]) -> bool {
    WEB_SIGNATURES.iter().any(|signature| data.starts_with(signature))
}

fn is_brotli_web(head: &[u8]) -> bool {
    head.get(BROTLI_MAGIC_OFFSET..BROTLI_MAGIC_OFFSET + BROTLI_MAGIC.len()) == Some(BROTLI_MAGIC)
}

fn is_gzip_web(head: &[u8]) -> bool {
    if !head.starts_with(GZIP_MAGIC) {
        return false;
    }
    let mut out = Vec::new();
    let _ = flate2::read::GzDecoder::new(head).take(20).read_to_end(&mut out);
    has_web_signature(&out)
}

pub fn is_web_file(head: &[u8]) -> bool {
    has_web_signature(head) || is_gzip_web(head) || is_brotli_web(head)
}

fn looks_like_assets(head: &[u8], len: u64) -> bool {
    if head.len() < 16 {
        return false;
    }
    let be32 = |at: usize| u32::from_be_bytes([head[at], head[at + 1], head[at + 2], head[at + 3]]) as u64;
    let (mut metadata_size, mut file_size, version, mut data_offset) = (be32(0), be32(4), be32(8), be32(12));
    if version > 100 {
        return false;
    }
    if version >= 22 {
        if head.len() < 40 {
            return false;
        }
        let be64 = |at: usize| u64::from_be_bytes(head[at..at + 8].try_into().unwrap());
        (metadata_size, file_size, data_offset) = (be32(20), be64(24), be64(32));
    }
    [file_size, metadata_size, version, data_offset].iter().all(|&value| value <= len) && file_size >= metadata_size && file_size >= data_offset
}

pub fn kind_of(head: &[u8], len: u64, name: &str) -> FileKind {
    if len < 20 {
        FileKind::Raw
    } else if is_unity_bundle(head) {
        FileKind::Bundle
    } else if has_web_signature(head) {
        FileKind::Web
    } else if len < 128 {
        FileKind::Raw
    } else if is_gzip_web(head) || is_brotli_web(head) {
        FileKind::Web
    } else if looks_like_assets(head, len) && !NON_ASSET_SUFFIXES.iter().any(|suffix| name.ends_with(suffix)) {
        FileKind::Assets
    } else {
        FileKind::Raw
    }
}

fn decompress_web(data: &[u8]) -> Result<Cow<'_, [u8]>, String> {
    if data.starts_with(GZIP_MAGIC) {
        let mut out = Vec::new();
        flate2::read::GzDecoder::new(data).read_to_end(&mut out).map_err(|e| format!("WebFile gzip: {}", e))?;
        Ok(Cow::Owned(out))
    } else if is_brotli_web(data) {
        let mut out = Vec::new();
        brotli_decompressor::Decompressor::new(data, 4096).read_to_end(&mut out).map_err(|e| format!("WebFile brotli: {}", e))?;
        Ok(Cow::Owned(out))
    } else {
        Ok(Cow::Borrowed(data))
    }
}

fn read_web_entries(data: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let plain = decompress_web(data)?;
    if !has_web_signature(&plain) {
        return Err("Invalid WebFile signature".to_string());
    }
    let mut pos = plain.iter().position(|&b| b == 0).ok_or("Truncated WebFile header")? + 1;
    let read_i32 = |at: usize| plain.get(at..at + 4).map(|b| i32::from_le_bytes(b.try_into().unwrap()) as usize).ok_or("Truncated WebFile header".to_string());
    let header_end = read_i32(pos)?;
    pos += 4;
    let mut entries = Vec::new();
    while pos < header_end {
        let (offset, length, name_length) = (read_i32(pos)?, read_i32(pos + 4)?, read_i32(pos + 8)?);
        pos += 12;
        let name = plain.get(pos..pos.saturating_add(name_length)).ok_or("Truncated WebFile header")?;
        pos += name_length;
        let body = plain.get(offset..offset.saturating_add(length)).ok_or("Truncated WebFile entry")?;
        entries.push((String::from_utf8_lossy(name).into_owned(), body.to_vec()));
    }
    Ok(entries)
}

fn visit(data: ByteSource, name: &str, version: UnityVersion, depth: usize, out: &mut Vec<Entry>) -> Result<(), String> {
    let kind = if depth > MAX_DEPTH { FileKind::Raw } else { kind_of(&data, data.len() as u64, name) };
    match kind {
        FileKind::Bundle => {
            let bundle = BundleFile::open(data)?;
            for (i, node) in bundle.nodes.iter().enumerate() {
                let bytes: ByteSource = bundle.read_node(i)?.into();
                if node.is_raw_stream() {
                    out.push(Entry { name: node.name.clone(), data: bytes, version: bundle.engine_version.clone(), kind: FileKind::Raw });
                } else {
                    visit(bytes, &node.name, bundle.engine_version.clone(), depth + 1, out)?;
                }
            }
        }
        FileKind::Web => {
            for (entry_name, bytes) in read_web_entries(&data)? {
                visit(bytes.into(), &entry_name, version.clone(), depth + 1, out)?;
            }
        }
        kind => out.push(Entry { name: name.to_string(), data, version, kind }),
    }
    Ok(())
}

pub fn expand(data: ByteSource, name: &str) -> Result<Vec<Entry>, String> {
    let mut out = Vec::new();
    visit(data, name, UnityVersion::default(), 0, &mut out)?;
    Ok(out)
}
