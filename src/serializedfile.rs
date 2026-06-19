use crate::reader::{Reader, Endian};
use crate::unity_version::UnityVersion;
use crate::objectreader::ObjectReader;
#[allow(non_snake_case)]
#[derive(Clone, Debug, serde::Serialize)]
pub struct TypeTreeNode {
    pub m_Version: i16,
    pub m_Level: i8,
    pub m_IsArray: bool,
    pub m_TypeStrOffset: u32,
    pub m_NameStrOffset: u32,
    pub m_ByteSize: i32,
    pub m_Index: i32,
    pub m_MetaFlag: i32,
    pub m_RefTypeHash: u64,
    pub m_Type: String,
    pub m_Name: String,
}
#[derive(Clone, Debug)]
pub struct SerializedType {
    pub class_id: i32,
    pub nodes: std::sync::Arc<[TypeTreeNode]>,
}
#[derive(Clone, Debug)]
pub struct FileIdentifier {
    pub guid: [u8; 16],
    pub type_: i32,
    pub path_name: String,
}
#[derive(Clone, Debug)]
pub struct RefType {
    pub class_name: String,
    pub namespace: String,
    pub assembly_name: String,
    pub nodes: std::sync::Arc<[TypeTreeNode]>,
}
pub struct SerializedFile {
    pub data: crate::reader::ByteSource,
    pub version: u32,
    pub endian: Endian,
    pub unity_version: UnityVersion,
    pub target_platform: i32,
    pub enable_type_tree: bool,
    pub types: Vec<SerializedType>,
    pub objects: Vec<ObjectReader>,
    pub externals: Vec<FileIdentifier>,
    pub object_map: std::collections::HashMap<i64, usize>,
    pub ref_types: Vec<RefType>,
}
impl SerializedFile {
    pub fn read(reader: &mut Reader) -> Self {
        let _original_endian = reader.endian;
        reader.endian = Endian::Big;
        let metadata_size_base = reader.read_u32() as u64;
        let file_size_base = reader.read_u32() as u64;
        let version = reader.read_u32();
        let data_offset_base = reader.read_u32() as u64;
        let metadata_size = metadata_size_base;
        let file_size = file_size_base;
        let mut data_offset = data_offset_base;
        let endian;
        if version >= 9 {
            let endian_byte = reader.read_u8();
            endian = if endian_byte == 1 { Endian::Big } else { Endian::Little };
            reader.read_bytes(3);
            if version >= 22 {
                let _metadata_size = reader.read_u32() as u64;
                let _file_size = reader.read_i64() as u64;
                data_offset = reader.read_i64() as u64;
                let _unknown = reader.read_i64();
            }
        } else {
            let pos = (file_size as i64).saturating_sub(metadata_size as i64);
            reader.pos = pos.max(0) as usize;
            let endian_byte = reader.read_u8();
            endian = if endian_byte == 1 { Endian::Big } else { Endian::Little };
        }
        reader.endian = endian;
        let unity_version_str = if version >= 7 {
            let s = reader.read_string_null();
            s
        } else {
            String::new()
        };
        let head_pos = reader.pos;
        let _head_dump = reader.read_bytes(64);
        reader.pos = head_pos;
        let mut unity_version = unity_version_str.parse::<UnityVersion>().unwrap_or_default();
        if unity_version.is_empty() {
            unity_version = UnityVersion::guess_from_format(version);
        }
        reader.unity_version = unity_version.clone();
        let target_platform = if version >= 8 {
            reader.read_i32()
        } else { 0 };
        let enable_type_tree = if version >= 13 { reader.read_u8() != 0 } else { true };
        let type_count = reader.read_i32();
        let mut types = Vec::new();
        if type_count > 0 && type_count < 100_000 {
            for i in 0..type_count {
                let (typ, _) = Self::read_type(reader, version, enable_type_tree, false);
                types.push(typ);
                if i % 100 == 0 || i == type_count - 1 {
                }
            }
        }
        let big_id_enabled = if version >= 7 && version < 14 { reader.read_i32() } else { 0 };
        let object_count = reader.read_i32();
        let mut objects = Vec::new();
        if object_count > 0 && object_count < 1_000_000 {
            let mut local_fallback_cache = std::collections::HashMap::new();
            for _ in 0..object_count {
                objects.push(Self::read_object_inner(reader, version, big_id_enabled, &types, data_offset as usize, &mut local_fallback_cache));
            }
        }
        if version >= 11 {
            let script_count = reader.read_i32();
            for _ in 0..script_count {
                reader.read_i32();
                if version < 14 {
                    reader.read_i32();
                } else {
                    reader.align(4);
                    reader.read_i64();
                }
            }
        }
        let externals_count = reader.read_i32();
        let mut externals = Vec::with_capacity(externals_count as usize);
        for _ in 0..externals_count {
            let mut guid = [0u8; 16];
            let mut type_ = 0;
            if version >= 6 {
                reader.read_string_null();
            }
            if version >= 5 {
                let bytes = reader.read_bytes(16);
                guid.copy_from_slice(&bytes);
                type_ = reader.read_i32();
            }
            let path_name = reader.read_string_null();
            externals.push(FileIdentifier {
                guid,
                type_,
                path_name,
            });
        }
        let mut ref_types = Vec::new();
        if version >= 20 {
            let ref_type_count = reader.read_i32();
            ref_types = Vec::with_capacity(ref_type_count as usize);
            for _ in 0..ref_type_count {
                let (typ, ref_info) = Self::read_type(reader, version, enable_type_tree, true);
                if let Some((class_name, namespace, assembly_name)) = ref_info {
                    ref_types.push(RefType {
                        class_name,
                        namespace,
                        assembly_name,
                        nodes: typ.nodes,
                    });
                }
            }
        }
        if version >= 5 {
            let _user_info = reader.read_string_null();
        }
        let mut object_map = std::collections::HashMap::with_capacity(objects.len());
        for (idx, obj) in objects.iter().enumerate() {
            object_map.insert(obj.path_id, idx);
        }
        Self {
            data: reader.data.clone(),
            version,
            endian,
            unity_version,
            target_platform,
            enable_type_tree,
            types,
            objects,
            externals,
            object_map,
            ref_types,
        }
    }
    fn read_type(reader: &mut Reader, version: u32, enable_type_tree: bool, is_ref_type: bool) -> (SerializedType, Option<(String, String, String)>) {
        let class_id = reader.read_i32();
        if version >= 16 {
            reader.read_u8();
        }
        let mut script_type_index = -1i16;
        if version >= 17 {
            script_type_index = reader.read_i16();
        }
        if version >= 13 {
            if (is_ref_type && script_type_index >= 0) || (version < 16 && class_id < 0) || (version >= 16 && class_id == 114) {
                reader.read_bytes(16);
            }
            reader.read_bytes(16);
        }
        let mut nodes = Vec::new();
        let mut ref_type_info = None;
        if enable_type_tree {
            nodes = if version >= 12 || version == 10 {
                Self::read_type_tree_blob(reader, version)
            } else {
                Self::read_type_tree_recursive(reader, 0)
            };
            if version >= 21 {
                if is_ref_type {
                    let class_name = reader.read_string_null();
                    let namespace = reader.read_string_null();
                    let assembly_name = reader.read_string_null();
                    ref_type_info = Some((class_name, namespace, assembly_name));
                } else {
                    let count = reader.read_i32();
                    for _ in 0..count { reader.read_i32(); }
                }
            }
        }
        (SerializedType { class_id, nodes: nodes.into() }, ref_type_info)
    }
    fn read_type_tree_recursive(reader: &mut Reader, level: i8) -> Vec<TypeTreeNode> {
        let mut type_tree = Vec::new();
        let m_type = reader.read_string_null();
        let m_name = reader.read_string_null();
        let m_byte_size = reader.read_i32();
        let m_index = reader.read_i32();
        let m_is_array = reader.read_i32() != 0;
        let m_version = reader.read_i32() as i16;
        let m_meta_flag = reader.read_i32();
        type_tree.push(TypeTreeNode {
            m_Version: m_version,
            m_Level: level,
            m_IsArray: m_is_array,
            m_TypeStrOffset: 0,
            m_NameStrOffset: 0,
            m_ByteSize: m_byte_size,
            m_Index: m_index,
            m_MetaFlag: m_meta_flag,
            m_RefTypeHash: 0,
            m_Type: m_type,
            m_Name: m_name,
        });
        let num_fields = reader.read_u32();
        for _ in 0..num_fields {
            type_tree.extend(Self::read_type_tree_recursive(reader, level + 1));
        }
        type_tree
    }
    fn read_type_tree_blob(reader: &mut Reader, version: u32) -> Vec<TypeTreeNode> {
        let node_count = reader.read_i32();
        let string_buffer_size = reader.read_i32();
        if node_count <= 0 || node_count > 100_000 { return Vec::new(); }
        let node_struct_size = if version >= 19 { 32 } else { 24 };
        let base_pos = reader.pos + (node_count as usize * node_struct_size);
        let mut raw_nodes = Vec::with_capacity(node_count as usize);
        for _ in 0..node_count {
            raw_nodes.push(Self::read_type_tree_node(reader, version));
        }
        reader.pos = base_pos;
        let string_buffer = reader.read_bytes(string_buffer_size as usize);
        raw_nodes.into_iter().map(|raw| {
            TypeTreeNode {
                m_Version: raw.version,
                m_Level: raw.level,
                m_IsArray: raw.is_array,
                m_TypeStrOffset: raw.type_offset,
                m_NameStrOffset: raw.name_offset,
                m_ByteSize: raw.byte_size,
                m_Index: raw.index,
                m_MetaFlag: raw.meta_flag,
                m_RefTypeHash: raw.ref_hash,
                m_Type: Self::get_string(&string_buffer, raw.type_offset),
                m_Name: Self::get_string(&string_buffer, raw.name_offset),
            }
        }).collect()
    }
    fn read_type_tree_node(reader: &mut Reader, version: u32) -> RawTypeTreeNode {
        if version >= 19 {
            let v = reader.read_u16();
            let l = reader.read_i8();
            let a = reader.read_u8() != 0;
            let to = reader.read_u32();
            let no = reader.read_u32();
            let bs = reader.read_i32();
            let idx = reader.read_i32();
            let mf = reader.read_i32();
            let rh = reader.read_u64();
            RawTypeTreeNode {
                version: v as i16, level: l, is_array: a, type_offset: to, name_offset: no,
                byte_size: bs, index: idx, meta_flag: mf, ref_hash: rh
            }
        } else {
            let v = reader.read_i16();
            let l = reader.read_i8();
            let a = reader.read_u8() != 0;
            let to = reader.read_u32();
            let no = reader.read_u32();
            let bs = reader.read_i32();
            let idx = reader.read_i32();
            let mf = reader.read_i32();
            RawTypeTreeNode {
                version: v, level: l, is_array: a, type_offset: to, name_offset: no,
                byte_size: bs, index: idx, meta_flag: mf, ref_hash: 0
            }
        }
    }
    fn get_string(buffer: &[u8], offset: u32) -> String {
        if offset & 0x80000000 != 0 {
            let id = offset & 0x7FFFFFFF;
            match crate::common_string::get_common_string(id) {
                Some(s) => s.to_string(),
                None => format!("CommonString_{}", id),
            }
        } else {
            let start = offset as usize;
            let mut end = start;
            while end < buffer.len() && buffer[end] != 0 {
                end += 1;
            }
            String::from_utf8_lossy(&buffer[start..end.min(buffer.len())]).into_owned()
        }
    }
    fn read_object_inner(
        reader: &mut Reader,
        version: u32,
        big_id_enabled: i32,
        types: &[SerializedType],
        data_offset: usize,
        local_fallback_cache: &mut std::collections::HashMap<i32, std::sync::Arc<[TypeTreeNode]>>,
    ) -> ObjectReader {
        let path_id = if big_id_enabled != 0 {
            reader.read_i64()
        } else if version < 14 {
            reader.read_i32() as i64
        } else {
            reader.align(4);
            reader.read_i64()
        };
        let byte_start = if version >= 22 {
            reader.read_u64() as usize
        } else {
            reader.read_u32() as usize
        } + data_offset;
        let byte_size = reader.read_u32() as usize;
        let type_id = reader.read_i32();
        let (class_id, mut nodes) = if version < 16 {
            let class_id = reader.read_u16() as i32;
            (class_id, std::sync::Arc::from(Vec::new()))
        } else {
            match types.get(type_id as usize) {
                Some(typ) => (typ.class_id, typ.nodes.clone()),
                None => (-1, std::sync::Arc::from(Vec::new())),
            }
        };
        if nodes.is_empty() {
            if let Some(cached_nodes) = local_fallback_cache.get(&class_id) {
                nodes = cached_nodes.clone();
            } else {
                let mut raw_nodes = crate::fallback_typetree::get_fallback_nodes(class_id);
                if class_id == 43 && reader.unity_version < (2019, 1) {
                    let mut filtered = Vec::new();
                    let mut skip_until_level_1 = false;
                    for node in raw_nodes.iter() {
                        if skip_until_level_1 {
                            if node.m_Level == 1 {
                                skip_until_level_1 = false;
                            } else {
                                continue;
                            }
                        }
                        if node.m_Level == 1 && (node.m_Name == "m_BonesAABB" || node.m_Name == "m_VariableBoneCountWeights") {
                            skip_until_level_1 = true;
                            continue;
                        }
                        filtered.push(node.clone());
                    }
                    raw_nodes = filtered.into();
                }
                nodes = raw_nodes;
                local_fallback_cache.insert(class_id, nodes.clone());
            }
        }
        if version < 11 {
            reader.read_u16();
        }
        if version >= 11 && version < 17 {
            reader.read_i16();
        }
        if version == 15 || version == 16 {
            reader.read_i8();
        }
        let mut obj = ObjectReader::new(path_id, class_id, byte_start, byte_size, None, version);
        obj.serialized_type = Some(SerializedType { class_id, nodes });
        obj
    }
}
struct RawTypeTreeNode {
    version: i16,
    level: i8,
    is_array: bool,
    type_offset: u32,
    name_offset: u32,
    byte_size: i32,
    index: i32,
    meta_flag: i32,
    ref_hash: u64,
}
