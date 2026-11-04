use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use crate::bundle::BundleFile;
use crate::serializedfile::SerializedFile;
use crate::objectreader::ObjectReader;
use crate::reader::{ByteSource, Endian, Reader};
use crate::unity_version::UnityVersion;

pub struct AssetManager {
    pub files: HashMap<String, SerializedFile>,
    pub raw_files: HashMap<String, ByteSource>,
    pub simplified_files: HashMap<String, String>,
}

impl AssetManager {
    pub fn new() -> Self {
        Self {
            files: HashMap::new(),
            raw_files: HashMap::new(),
            simplified_files: HashMap::new(),
        }
    }

    pub fn add_file(&mut self, name: String, file: SerializedFile) {
        let simple = simplify_name(&name);
        self.simplified_files.insert(simple, name.clone());
        self.files.insert(name, file);
    }

    pub fn add_raw_file(&mut self, name: String, data: impl Into<ByteSource>) {
        self.raw_files.insert(name, data.into());
    }

    pub fn from_source(data: ByteSource, name: &str) -> Result<Self, String> {
        let mut manager = Self::new();
        if data.starts_with(b"UnityFS") {
            let bundle = BundleFile::open(data)?;
            for (i, node) in bundle.nodes.iter().enumerate() {
                let bytes = bundle.read_node(i)?;
                if node.is_raw_stream() {
                    manager.add_raw_file(node.name.clone(), bytes);
                } else if bytes.len() > 20 {
                    let sf = SerializedFile::read(&mut Reader::new(bytes, bundle.engine_version.clone()));
                    manager.add_file(node.name.clone(), sf);
                }
            }
        } else if crate::metadata::looks_like_serialized_file(&data) {
            manager.add_serialized_file(name.to_string(), data);
        } else {
            return Err("not a Unity bundle or SerializedFile".to_string());
        }
        Ok(manager)
    }

    pub fn from_bytes(data: Vec<u8>, name: &str) -> Result<Self, String> {
        Self::from_source(data.into(), name)
    }

    pub fn open_path<P: AsRef<Path>>(path: P) -> Result<Self, String> {
        let path = path.as_ref();
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let mut manager = Self::from_source(map_file(path)?, &name)?;
        if !manager.files.is_empty() && manager.raw_files.is_empty() {
            if let Some(dir) = path.parent() {
                manager.add_resource_siblings(dir, &name);
            }
        }
        Ok(manager)
    }

    pub fn add_serialized_file(&mut self, name: String, data: ByteSource) {
        let mut reader = Reader { data, pos: 0, endian: Endian::Big, unity_version: UnityVersion::default() };
        let sf = SerializedFile::read(&mut reader);
        self.add_file(name, sf);
    }

    pub fn add_resource_siblings(&mut self, dir: &Path, name: &str) {
        let stem = name.split('.').next().unwrap_or(name).to_lowercase();
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let file_name = entry.file_name().to_string_lossy().into_owned();
            let lower = file_name.to_lowercase();
            if !(lower.ends_with(".ress") || lower.ends_with(".resource")) || lower.split('.').next() != Some(stem.as_str()) {
                continue;
            }
            if let Ok(data) = map_file(&entry.path()) {
                self.add_raw_file(file_name, data);
            }
        }
    }

    pub fn get_object(&self, file_name: &str, path_id: i64) -> Option<&ObjectReader> {
        self.files.get(file_name).and_then(|f| {
            f.object_map.get(&path_id).map(|&idx| &f.objects[idx])
        })
    }

    fn get_file_by_name(&self, name: &str) -> Option<(&String, &SerializedFile)> {
        if let Some(kv) = self.files.get_key_value(name) {
            return Some(kv);
        }
        let simple_name = simplify_name(name);
        if let Some(original_name) = self.simplified_files.get(&simple_name) {
            return self.files.get_key_value(original_name);
        }
        None
    }

    pub fn resolve_pptr<'a>(&'a self, source_file_name: &str, file_id: i32, path_id: i64) -> Option<(&'a str, &'a ObjectReader)> {
        if file_id == 0 {
            let (name, sf) = self.get_file_by_name(source_file_name)?;
            let idx = *sf.object_map.get(&path_id)?;
            let obj = &sf.objects[idx];
            Some((name.as_str(), obj))
        } else {
            let sf = self.get_file_by_name(source_file_name)?.1;
            let external = sf.externals.get((file_id - 1) as usize)?;
            let target_name = &external.path_name;
            let (name, target_sf) = self.get_file_by_name(target_name)?;
            let idx = *target_sf.object_map.get(&path_id)?;
            let obj = &target_sf.objects[idx];
            Some((name.as_str(), obj))
        }
    }

    pub fn read_object_value(&self, source_file_name: &str, file_id: i32, path_id: i64) -> Result<crate::value::UnityValue, String> {
        let (file_name, obj) = self.resolve_pptr(source_file_name, file_id, path_id).ok_or("Failed to resolve PPtr")?;
        let sf = self.get_file_by_name(file_name).unwrap().1;
        let mut reader = crate::reader::Reader {
            data: sf.data.clone(),
            pos: 0,
            endian: sf.endian,
            unity_version: sf.unity_version.clone(),
        };
        obj.read_typetree(&mut reader, &sf.ref_types)
    }
}

pub struct GameObjectNode<'a> {
    pub manager: &'a AssetManager,
    pub file_name: &'a str,
    pub path_id: i64,
    pub go: crate::GameObject,
    pub transform: crate::Transform,
}

impl<'a> GameObjectNode<'a> {
    pub fn new(manager: &'a AssetManager, file_name: &'a str, path_id: i64) -> Result<Self, String> {
        let go_val = manager.read_object_value(file_name, 0, path_id)?;
        let go = crate::GameObject::from_value(go_val)?;
        let mut transform = None;
        for comp_pair in &go.m_Component {
            if let Ok(comp_val) = manager.read_object_value(file_name, comp_pair.component.file_id, comp_pair.component.path_id) {
                if let Ok(t) = crate::Transform::from_value(comp_val) {
                    transform = Some(t);
                    break;
                }
            }
        }
        let transform = transform.ok_or("GameObject has no Transform")?;
        Ok(Self {
            manager,
            file_name,
            path_id,
            go,
            transform,
        })
    }

    pub fn children(&self) -> Vec<GameObjectNode<'a>> {
        let mut children = Vec::new();
        for child_pptr in &self.transform.m_Children {
            if let Some((target_file, _)) = self.manager.resolve_pptr(self.file_name, child_pptr.file_id, child_pptr.path_id) {
                if let Ok(child_transform_val) = self.manager.read_object_value(self.file_name, child_pptr.file_id, child_pptr.path_id) {
                    if let Ok(child_transform) = crate::Transform::from_value(child_transform_val) {
                        let go_pptr = &child_transform.m_GameObject;
                        if let Some((go_target_file, _)) = self.manager.resolve_pptr(target_file, go_pptr.file_id, go_pptr.path_id) {
                            if let Ok(child_node) = GameObjectNode::new(self.manager, go_target_file, go_pptr.path_id) {
                                children.push(child_node);
                            }
                        }
                    }
                }
            }
        }
        children
    }
}

fn map_file(path: &Path) -> Result<ByteSource, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {}", path.display(), e))?;
    if file.metadata().map(|m| m.len() == 0).unwrap_or(false) {
        return Ok(Vec::new().into());
    }
    let mmap = unsafe { memmap2::Mmap::map(&file) }.map_err(|e| format!("{}: {}", path.display(), e))?;
    Ok(ByteSource::Mmap(Arc::new(mmap)))
}

fn simplify_name(name: &str) -> String {
    let name = name.replace('\\', "/");
    let basename = name.rsplit('/').next().unwrap_or(&name);
    basename.to_lowercase()
}
