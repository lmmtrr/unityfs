use std::collections::BTreeMap;
use crate::reader::Reader;
use crate::value::UnityValue;
use crate::serializedfile::TypeTreeNode;
pub struct TypeTreeReader<'a> {
    pub reader: &'a mut Reader,
    pub obj_end: usize,
    pub ref_types: &'a [crate::serializedfile::RefType],
    pub class_id: i32,
}
fn should_ignore_field(name: &str) -> bool {
    name == "m_Shapes" || name == "m_BakedConvexCollisionMesh" || name == "m_BakedTriangleCollisionMesh" || name == "m_VariableBoneCountWeights"
}
impl<'a> TypeTreeReader<'a> {
    pub fn new(reader: &'a mut Reader, obj_end: usize, ref_types: &'a [crate::serializedfile::RefType], class_id: i32) -> Self {
        Self { reader, obj_end, ref_types, class_id }
    }
    pub fn read(&mut self, nodes: &[TypeTreeNode]) -> UnityValue {
        if nodes.is_empty() {
            return UnityValue::Null;
        }
        let mut index = 0;
        self.read_node(nodes, &mut index)
    }
    fn read_node(&mut self, nodes: &[TypeTreeNode], index: &mut usize) -> UnityValue {
        let node = &nodes[*index];
        if should_ignore_field(node.m_Name.as_str()) {
            self.skip_node(nodes, index);
            return UnityValue::Null;
        }
        let align = (node.m_MetaFlag & 0x4000) != 0;
        let res = match node.m_Type.as_str() {
            "bool" => UnityValue::Boolean(self.reader.read_u8() != 0),
            "SInt8" => UnityValue::Int8(self.reader.read_i8()),
            "UInt8" | "char" => UnityValue::UInt8(self.reader.read_u8()),
            "SInt16" | "short" => UnityValue::Int16(self.reader.read_i16()),
            "UInt16" | "unsigned short" => UnityValue::UInt16(self.reader.read_u16()),
            "int" | "SInt32" => UnityValue::Int32(self.reader.read_i32()),
            "unsigned int" | "UInt32" | "Type*" => UnityValue::UInt32(self.reader.read_u32()),
            "long long" | "SInt64" => UnityValue::Int64(self.reader.read_i64()),
            "unsigned long long" | "UInt64" | "FileSize" => UnityValue::UInt64(self.reader.read_u64()),
            "float" => UnityValue::Float(self.reader.read_f32()),
            "double" => UnityValue::Double(self.reader.read_f64()),
            "ReferencedObject" => self.read_referenced_object(nodes, index),
            "string" => {
                if self.class_id == 49 && node.m_Name == "m_Script" {
                    let len = self.reader.read_i32();
                    let bytes = if len > 0 && len <= 100_000_000 {
                        let b = self.reader.read_bytes(len as usize);
                        self.reader.align(4);
                        b
                    } else {
                        Vec::new()
                    };
                    *index = self.get_node_end(nodes, *index) - 1;
                    UnityValue::Bytes(bytes)
                } else {
                    let s = self.reader.read_string();
                    *index = self.get_node_end(nodes, *index) - 1;
                    UnityValue::String(s)
                }
            },
            "TypelessData" => {
                let size = self.reader.read_i32();
                let bytes = self.reader.read_bytes(size as usize);
                *index = self.get_node_end(nodes, *index) - 1;
                UnityValue::Bytes(bytes)
            },
            "StreamingInfo" => {
                let start_pos = self.reader.pos;
                let end = self.get_node_end(nodes, *index);
                let is_32bit = if start_pos + 12 <= self.reader.data.len() {
                    let path_len_32 = u32::from_le_bytes(self.reader.data[start_pos + 8..start_pos + 12].try_into().unwrap()) as usize;
                    if path_len_32 > 0 && path_len_32 < 1024 && start_pos + 12 + path_len_32 <= self.reader.data.len() {
                        let path_bytes = &self.reader.data[start_pos + 12..start_pos + 12 + path_len_32];
                        path_bytes.iter().all(|&b| b >= 32 && b <= 126)
                    } else {
                        false
                    }
                } else {
                    false
                };
                let (offset, size, path) = if is_32bit {
                    let offset = self.reader.read_u32() as u64;
                    let size = self.reader.read_u32();
                    let path = self.reader.read_string();
                    (offset, size, path)
                } else {
                    let offset = self.reader.read_u64();
                    let size = self.reader.read_u32();
                    let path = self.reader.read_string();
                    (offset, size, path)
                };
                *index = end - 1;
                let mut map = BTreeMap::new();
                map.insert("offset".to_string(), UnityValue::UInt64(offset));
                map.insert("size".to_string(), UnityValue::UInt32(size));
                map.insert("path".to_string(), UnityValue::String(path));
                UnityValue::Map(map)
            },
            _ => {
                if *index + 1 < nodes.len() && nodes[*index + 1].m_Type == "Array" {
                    self.read_array(nodes, index)
                } else if node.m_Type.starts_with("PPtr<") {
                    self.read_pptr(nodes, index)
                } else {
                    self.read_class(nodes, index)
                }
            }
        };
        if align {
            self.reader.align(4);
        }
        res
    }
    fn read_array(&mut self, nodes: &[TypeTreeNode], index: &mut usize) -> UnityValue {
        let container_index = *index;
        let array_node_index = container_index + 1;
        let end = self.get_node_end(nodes, container_index);
        let align_array = (nodes[array_node_index].m_MetaFlag & 0x4000) != 0;
        *index += 2;
        let size = self.reader.read_i32();
        if size < 0 || size > 10_000_000 {
            panic!("Invalid array size: {} at pos {}. This indicates a parsing error.", size, self.reader.pos);
        }
        *index += 1;
        let element_node_start = *index;
        if nodes[element_node_start].m_Type == "string" && size > 0 {
            let peek_pos = self.reader.pos;
            if peek_pos + 4 <= self.reader.data.len() {
                let first_str_len = self.reader.read_i32();
                self.reader.pos = peek_pos;
                let remaining_obj_bytes = self.obj_end.saturating_sub(peek_pos);
                if first_str_len < 0 || (first_str_len as usize + 4) > remaining_obj_bytes {
                    let bytes = self.reader.read_bytes(size as usize);
                    let s = String::from_utf8_lossy(&bytes).into_owned();
                    if align_array {
                        self.reader.align(4);
                    }
                    *index = end - 1;
                    return UnityValue::Array(vec![UnityValue::String(s)]);
                }
            }
        }
        let mut arr = Vec::with_capacity(size.min(1_000_000) as usize);
        for _ in 0..size {
            let mut element_index = element_node_start;
            arr.push(self.read_node(nodes, &mut element_index));
        }
        *index = end - 1;
        if align_array {
            self.reader.align(4);
        }
        UnityValue::Array(arr)
    }
    fn read_pptr(&mut self, nodes: &[TypeTreeNode], index: &mut usize) -> UnityValue {
        let container_index = *index;
        let end = self.get_node_end(nodes, container_index);
        let mut map = BTreeMap::new();
        *index += 1;
        while *index < end {
            let child_name = nodes[*index].m_Name.clone();
            map.insert(child_name, self.read_node(nodes, index));
            *index += 1;
        }
        *index = end - 1;
        UnityValue::PPtr {
            file_id: map.get("m_FileID").and_then(|v| v.as_i32()).unwrap_or(0),
            path_id: map.get("m_PathID").and_then(|v| v.as_i64()).unwrap_or(0),
        }
    }
    fn read_class(&mut self, nodes: &[TypeTreeNode], index: &mut usize) -> UnityValue {
        let container_index = *index;
        let end = self.get_node_end(nodes, container_index);
        let mut map = BTreeMap::new();
        *index += 1;
        while *index < end {
            let child_node = &nodes[*index];
            if child_node.m_Name == "m_StreamData" && self.reader.pos + 16 > self.obj_end {
                *index = self.get_node_end(nodes, *index) - 1;
                *index += 1;
                continue;
            }
            if self.reader.pos >= self.obj_end {
                *index = end;
                break;
            }
            let child_name = child_node.m_Name.clone();
            map.insert(child_name, self.read_node(nodes, index));
            *index += 1;
        }
        *index = end - 1;
        UnityValue::Map(map)
    }
    fn read_referenced_object(&mut self, nodes: &[TypeTreeNode], index: &mut usize) -> UnityValue {
        let container_index = *index;
        let end = self.get_node_end(nodes, container_index);
        let mut map = BTreeMap::new();
        *index += 1;
        while *index < end {
            let child_node = &nodes[*index];
            if child_node.m_Type == "ReferencedObjectData" {
                let mut found_nodes = None;
                if let Some(UnityValue::Map(type_map)) = map.get("type") {
                    let class_name = type_map.get("class").and_then(|v| v.as_str()).unwrap_or("");
                    let namespace = type_map.get("ns").and_then(|v| v.as_str()).unwrap_or("");
                    let assembly_name = type_map.get("asm").and_then(|v| v.as_str()).unwrap_or("");
                    if !class_name.is_empty() {
                        for ref_type in self.ref_types {
                            if ref_type.class_name == class_name && ref_type.namespace == namespace && ref_type.assembly_name == assembly_name {
                                found_nodes = Some(&ref_type.nodes);
                                break;
                            }
                        }
                        if found_nodes.is_none() {
                            println!("[WARNING] Referenced type not found in ref_types: class={}, ns={}, asm={}", class_name, namespace, assembly_name);
                        }
                    }
                }
                let val = if let Some(sub_nodes) = found_nodes {
                    let mut sub_reader = TypeTreeReader::new(self.reader, self.obj_end, self.ref_types, self.class_id);
                    sub_reader.read(sub_nodes)
                } else {
                    UnityValue::Null
                };
                map.insert(child_node.m_Name.clone(), val);
                *index = self.get_node_end(nodes, *index) - 1;
            } else {
                let child_name = child_node.m_Name.clone();
                map.insert(child_name, self.read_node(nodes, index));
            }
            *index += 1;
        }
        *index = end - 1;
        UnityValue::Map(map)
    }
    fn get_node_end(&self, nodes: &[TypeTreeNode], start_index: usize) -> usize {
        let level = nodes[start_index].m_Level;
        for i in (start_index + 1)..nodes.len() {
            if nodes[i].m_Level <= level {
                return i;
            }
        }
        nodes.len()
    }
    fn has_any_align_flag(&self, nodes: &[TypeTreeNode], start_index: usize) -> bool {
        let end = self.get_node_end(nodes, start_index);
        for i in start_index..end {
            if (nodes[i].m_MetaFlag & 0x4000) != 0 {
                return true;
            }
        }
        false
    }
    fn get_subtree_byte_size(&self, nodes: &[TypeTreeNode], start_index: usize) -> Option<usize> {
        if self.has_any_align_flag(nodes, start_index) {
            return None;
        }
        let root_node = &nodes[start_index];
        if root_node.m_ByteSize > 0 {
            return Some(root_node.m_ByteSize as usize);
        }
        let end = self.get_node_end(nodes, start_index);
        let mut total_size = 0;
        let mut i = start_index + 1;
        while i < end {
            let node = &nodes[i];
            if node.m_ByteSize > 0 {
                total_size += node.m_ByteSize as usize;
                i = self.get_node_end(nodes, i);
            } else {
                match node.m_Type.as_str() {
                    "string" | "TypelessData" | "StreamingInfo" => return None,
                    _ => {
                        if i + 1 < nodes.len() && nodes[i + 1].m_Type == "Array" {
                            return None;
                        } else {
                            i += 1;
                        }
                    }
                }
            }
        }
        Some(total_size)
    }
    fn skip_node(&mut self, nodes: &[TypeTreeNode], index: &mut usize) {
        let node = &nodes[*index];
        let align = (node.m_MetaFlag & 0x4000) != 0;
        match node.m_Type.as_str() {
            "bool" | "SInt8" | "UInt8" | "char" => {
                self.reader.pos += 1;
            }
            "SInt16" | "short" | "UInt16" | "unsigned short" => {
                self.reader.pos += 2;
            }
            "int" | "SInt32" | "unsigned int" | "UInt32" | "Type*" | "float" => {
                self.reader.pos += 4;
            }
            "long long" | "SInt64" | "unsigned long long" | "UInt64" | "FileSize" | "double" => {
                self.reader.pos += 8;
            }
            "string" => {
                let len = self.reader.read_i32();
                if len > 0 && len < 10_000_000 {
                    self.reader.pos += len as usize;
                }
                self.reader.align(4);
                *index = self.get_node_end(nodes, *index) - 1;
            }
            "TypelessData" => {
                let len = self.reader.read_i32();
                if len > 0 && len < 100_000_000 {
                    self.reader.pos += len as usize;
                }
                *index = self.get_node_end(nodes, *index) - 1;
            }
            "StreamingInfo" => {
                let start_pos = self.reader.pos;
                let end = self.get_node_end(nodes, *index);
                let is_32bit = if start_pos + 12 <= self.reader.data.len() {
                    let path_len_32 = u32::from_le_bytes(self.reader.data[start_pos + 8..start_pos + 12].try_into().unwrap()) as usize;
                    if path_len_32 > 0 && path_len_32 < 1024 && start_pos + 12 + path_len_32 <= self.reader.data.len() {
                        let path_bytes = &self.reader.data[start_pos + 12..start_pos + 12 + path_len_32];
                        path_bytes.iter().all(|&b| b >= 32 && b <= 126)
                    } else {
                        false
                    }
                } else {
                    false
                };
                if is_32bit {
                    self.reader.pos += 8;
                    let len = self.reader.read_i32();
                    if len > 0 && len < 10_000_000 {
                        self.reader.pos += len as usize;
                    }
                    self.reader.align(4);
                } else {
                    self.reader.pos += 12;
                    let len = self.reader.read_i32();
                    if len > 0 && len < 10_000_000 {
                        self.reader.pos += len as usize;
                    }
                    self.reader.align(4);
                }
                *index = end - 1;
            }
            _ => {
                if *index + 1 < nodes.len() && nodes[*index + 1].m_Type == "Array" {
                    self.skip_array(nodes, index);
                } else {
                    self.skip_class(nodes, index);
                }
            }
        }
        if align {
            self.reader.align(4);
        }
    }
    fn skip_array(&mut self, nodes: &[TypeTreeNode], index: &mut usize) {
        let container_index = *index;
        let array_node_index = container_index + 1;
        let end = self.get_node_end(nodes, container_index);
        let align_array = (nodes[array_node_index].m_MetaFlag & 0x4000) != 0;
        *index += 2;
        let size = self.reader.read_i32();
        if size < 0 || size > 10_000_000 {
            panic!("Invalid array size: {} in skip_array", size);
        }
        *index += 1;
        let element_node_start = *index;
        if nodes[element_node_start].m_Type == "string" && size > 0 {
            let peek_pos = self.reader.pos;
            if peek_pos + 4 <= self.reader.data.len() {
                let first_str_len = self.reader.read_i32();
                self.reader.pos = peek_pos;
                let remaining_obj_bytes = self.obj_end.saturating_sub(peek_pos);
                if first_str_len < 0 || (first_str_len as usize + 4) > remaining_obj_bytes {
                    self.reader.pos += size as usize;
                    if align_array {
                        self.reader.align(4);
                    }
                    *index = end - 1;
                    return;
                }
            }
        }
        if size > 0 {
            if let Some(element_size) = self.get_subtree_byte_size(nodes, element_node_start) {
                self.reader.pos += element_size * size as usize;
            } else {
                for _ in 0..size {
                    let mut element_index = element_node_start;
                    self.skip_node(nodes, &mut element_index);
                }
            }
        }
        *index = end - 1;
        if align_array {
            self.reader.align(4);
        }
    }
    fn skip_class(&mut self, nodes: &[TypeTreeNode], index: &mut usize) {
        let container_index = *index;
        let end = self.get_node_end(nodes, container_index);
        *index += 1;
        while *index < end {
            if self.reader.pos >= self.obj_end {
                *index = end;
                break;
            }
            self.skip_node(nodes, index);
            *index += 1;
        }
        *index = end - 1;
    }
}
