use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use crate::bundle::BundleFile;
use crate::container::FileKind;
use crate::objectreader::ObjectReader;
use crate::reader::{ByteSource, Endian, Reader};
use crate::serializedfile::SerializedFile;
use crate::unity_version::UnityVersion;
use crate::value::UnityValue;

pub struct Metadata {
    source: Source,
    pub files: Vec<MetadataFile>,
}

enum Source {
    Bundle(BundleFile),
    File(ByteSource),
    Entries(Vec<ByteSource>),
}

pub struct MetadataFile {
    pub name: String,
    pub file: SerializedFile,
    node: usize,
}

impl MetadataFile {
    pub fn externals(&self) -> impl Iterator<Item = &str> {
        self.file.externals.iter().map(|e| e.path_name.as_str())
    }

    pub fn type_counts(&self) -> BTreeMap<String, u32> {
        let mut types = BTreeMap::new();
        for obj in &self.file.objects {
            *types.entry(obj.type_name().to_string()).or_insert(0) += 1;
        }
        types
    }

    pub fn script_fields(&self) -> Vec<Vec<&str>> {
        self.file
            .types
            .iter()
            .filter(|t| t.class_id == 114)
            .map(|t| t.nodes.iter().filter(|n| n.m_Level == 1).map(|n| n.m_Name.as_str()).collect())
            .collect()
    }

    pub fn has_class(&self, class_id: i32) -> bool {
        self.file.objects.iter().any(|o| o.class_id == class_id)
    }
}

impl Metadata {
    pub fn open(path: &Path) -> Result<Self, String> {
        let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let mmap = unsafe { memmap2::Mmap::map(&file).map_err(|e| e.to_string())? };
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if crate::container::is_web_file(&mmap) {
            return Self::open_expanded(ByteSource::Mmap(Arc::new(mmap)), &name);
        }
        if crate::extractor::is_unity_bundle(&mmap) {
            let source = ByteSource::Mmap(Arc::new(mmap));
            let bundle = BundleFile::open(source.clone())?;
            let mut files = Vec::new();
            for (i, node) in bundle.nodes.iter().enumerate() {
                if node.is_raw_stream() || node.size < 20 {
                    continue;
                }
                let head = bundle.read_node_range(i, 0, 256)?;
                match crate::container::kind_of(&head, node.size, &node.name) {
                    FileKind::Bundle | FileKind::Web => return Self::open_expanded(source, &name),
                    FileKind::Raw => continue,
                    FileKind::Assets => {}
                }
                let Some(extent) = SerializedFile::metadata_extent(&head) else { continue };
                let bytes = bundle.read_node_range(i, 0, extent)?;
                let file = SerializedFile::read(&mut Reader::new(bytes, bundle.engine_version.clone()));
                files.push(MetadataFile { name: node.name.clone(), file, node: i });
            }
            Ok(Self { source: Source::Bundle(bundle), files })
        } else {
            let extent = SerializedFile::metadata_extent(&mmap).ok_or("not a SerializedFile")? as usize;
            let bytes = mmap[..extent.min(mmap.len())].to_vec();
            let file = SerializedFile::read(&mut Reader::new(bytes, UnityVersion::default()));
            Ok(Self { source: Source::File(ByteSource::Mmap(Arc::new(mmap))), files: vec![MetadataFile { name, file, node: 0 }] })
        }
    }

    fn open_expanded(data: ByteSource, name: &str) -> Result<Self, String> {
        let mut sources = Vec::new();
        let mut files = Vec::new();
        for entry in crate::container::expand(data, name)? {
            if entry.kind != FileKind::Assets {
                continue;
            }
            let Some(extent) = SerializedFile::metadata_extent(&entry.data) else { continue };
            let bytes = entry.data[..(extent as usize).min(entry.data.len())].to_vec();
            let file = SerializedFile::read(&mut Reader::new(bytes, entry.version));
            files.push(MetadataFile { name: entry.name, file, node: sources.len() });
            sources.push(entry.data);
        }
        Ok(Self { source: Source::Entries(sources), files })
    }

    pub fn is_bundle(&self) -> bool {
        !matches!(self.source, Source::File(_))
    }

    fn read_range(&self, entry: &MetadataFile, offset: u64, len: u64) -> Result<Vec<u8>, String> {
        match &self.source {
            Source::Bundle(bundle) => bundle.read_node_range(entry.node, offset, len),
            Source::File(data) => Ok(slice_range(data, offset, len)),
            Source::Entries(sources) => Ok(slice_range(&sources[entry.node], offset, len)),
        }
    }

    fn unity_version(&self, entry: &MetadataFile) -> UnityVersion {
        match &self.source {
            Source::Bundle(bundle) => bundle.engine_version.clone(),
            Source::File(_) | Source::Entries(_) => entry.file.unity_version.clone(),
        }
    }

    pub fn container(&self, entry: &MetadataFile) -> Vec<String> {
        let Some(ab) = entry.file.objects.iter().find(|o| o.class_id == 142) else { return Vec::new() };
        let end = (ab.byte_start + ab.byte_size) as u64;
        let Ok(bytes) = self.read_range(entry, 0, end) else { return Vec::new() };
        let sf = SerializedFile::read(&mut Reader::new(bytes, self.unity_version(entry)));
        let Some(ab) = sf.objects.iter().find(|o| o.class_id == 142) else { return Vec::new() };
        let mut r = Reader { data: sf.data.clone(), pos: 0, endian: sf.endian, unity_version: sf.unity_version.clone() };
        let Ok(v) = ab.read_typetree(&mut r, &sf.ref_types) else { return Vec::new() };
        let Some(UnityValue::Array(items)) = v.get("m_Container") else { return Vec::new() };
        items
            .iter()
            .filter_map(|it| match it.get("first") {
                Some(UnityValue::String(s)) => Some(s.clone()),
                _ => None,
            })
            .collect()
    }

    fn read_object(&self, entry: &MetadataFile, obj: &ObjectReader, len: usize) -> Result<UnityValue, String> {
        let sf = &entry.file;
        let bytes = self.read_range(entry, obj.byte_start as u64, len as u64)?;
        let local = ObjectReader { byte_start: 0, byte_size: bytes.len(), serialized_type: obj.serialized_type.clone(), ..*obj };
        let mut r = Reader { data: ByteSource::Heap(bytes.into()), pos: 0, endian: sf.endian, unity_version: sf.unity_version.clone() };
        local.read_typetree(&mut r, &sf.ref_types)
    }

    pub fn object_name(&self, entry: &MetadataFile, obj: &ObjectReader) -> Option<String> {
        let sf = &entry.file;
        let first_field = obj.serialized_type.as_ref()?.nodes.iter().find(|n| n.m_Level == 1)?;
        let name = if obj.class_id == 1 {
            self.read_object(entry, obj, obj.byte_size).ok()?.get("m_Name")?.as_str()?.to_string()
        } else if first_field.m_Name == "m_Name" {
            let b = self.read_range(entry, obj.byte_start as u64, obj.byte_size.min(4 + 512) as u64).ok()?;
            let len = b.get(..4)?.try_into().ok().map(|h| match sf.endian {
                Endian::Big => u32::from_be_bytes(h),
                Endian::Little => u32::from_le_bytes(h),
            })? as usize;
            String::from_utf8_lossy(b.get(4..4 + len)?).into_owned()
        } else {
            return None;
        };
        (!name.is_empty()).then_some(name)
    }

    pub fn object_names(&self, entry: &MetadataFile, class_id: i32) -> Vec<String> {
        entry.file.objects.iter().filter(|o| o.class_id == class_id).filter_map(|o| self.object_name(entry, o)).collect()
    }

    pub fn main_asset_name(&self, entry: &MetadataFile) -> Option<String> {
        let sf = &entry.file;
        let ab = sf.objects.iter().find(|o| o.class_id == 142)?;
        let v = self.read_object(entry, ab, ab.byte_size).ok()?;
        let Some(UnityValue::Array(items)) = v.get("m_Container") else { return None };
        let assets: Vec<&ObjectReader> = items
            .iter()
            .filter_map(|it| match it.get("second").and_then(|s| s.get("asset")) {
                Some(UnityValue::PPtr { file_id: 0, path_id }) => sf.object_map.get(path_id).map(|&i| &sf.objects[i]),
                _ => None,
            })
            .collect();
        let (objects, others): (Vec<_>, Vec<_>) = assets.into_iter().partition(|o| o.class_id == 1);
        objects.into_iter().chain(others).find_map(|o| self.object_name(entry, o))
    }
}

fn slice_range(data: &[u8], offset: u64, len: u64) -> Vec<u8> {
    let start = (offset as usize).min(data.len());
    let end = (offset.saturating_add(len) as usize).min(data.len());
    data[start..end].to_vec()
}

pub fn looks_like_old_serialized_file(head: &[u8], file_len: u64) -> bool {
    if head.len() < 16 {
        return false;
    }
    let be32 = |at: usize| u32::from_be_bytes([head[at], head[at + 1], head[at + 2], head[at + 3]]) as u64;
    let (meta, file_size, version, data_offset) = (be32(0), be32(4), be32(8), be32(12));
    (1..=8).contains(&version) && file_size == file_len && meta > 0 && meta < file_size && data_offset <= file_size
}

pub fn looks_like_serialized_file(head: &[u8]) -> bool {
    if head.len() < 20 {
        return false;
    }
    let be32 = |at: usize| u32::from_be_bytes([head[at], head[at + 1], head[at + 2], head[at + 3]]) as u64;
    let version = be32(8);
    if !(9..=50).contains(&version) {
        return false;
    }
    let endian = head[16];
    if endian > 1 || head[17..20] != [0, 0, 0] {
        return false;
    }
    if version >= 22 {
        if head.len() < 48 {
            return false;
        }
        let be64 = |at: usize| u64::from_be_bytes(head[at..at + 8].try_into().unwrap());
        let (file_size, data_offset) = (be64(24), be64(32));
        data_offset <= file_size && be32(20) < file_size
    } else {
        let (meta, file_size, data_offset) = (be32(0), be32(4), be32(12));
        meta > 0 && meta < file_size && data_offset <= file_size
    }
}
