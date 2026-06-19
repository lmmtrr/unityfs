use crate::reader::Reader;
use crate::serializedfile::SerializedType;
use crate::typetree::TypeTreeReader;
use crate::value::UnityValue;
pub struct ObjectReader {
    pub path_id: i64,
    pub class_id: i32,
    pub byte_start: usize,
    pub byte_size: usize,
    pub serialized_type: Option<SerializedType>,
    pub version: u32,
}
impl ObjectReader {
    pub fn new(
        path_id: i64,
        class_id: i32,
        byte_start: usize,
        byte_size: usize,
        serialized_type: Option<SerializedType>,
        version: u32,
    ) -> Self {
        Self {
            path_id,
            class_id,
            byte_start,
            byte_size,
            serialized_type,
            version,
        }
    }
    pub fn read_typetree(&self, reader: &mut Reader, ref_types: &[crate::serializedfile::RefType]) -> Result<UnityValue, String> {
        let nodes = match &self.serialized_type {
            Some(t) if !t.nodes.is_empty() => &t.nodes,
            _ => return Err(format!("No TypeTree nodes found for class_id {}", self.class_id)),
        };
        let mut obj_reader = reader.absolute_reader(self.byte_start, self.byte_size);
        let mut tt_reader = TypeTreeReader::new(&mut obj_reader, self.byte_start + self.byte_size, ref_types, self.class_id);
        let res = tt_reader.read(nodes);
        Ok(res)
    }
    pub fn type_name(&self) -> &str {
        if let Some(ref t) = self.serialized_type {
            if let Some(node) = t.nodes.first() {
                return &node.m_Type;
            }
        }
        match self.class_id {
            1 => "GameObject",
            4 => "Transform",
            21 => "Material",
            23 => "MeshRenderer",
            28 => "Texture2D",
            33 => "MeshFilter",
            43 => "Mesh",
            48 => "Shader",
            49 => "TextAsset",
            83 => "AudioClip",
            90 => "Avatar",
            95 => "Animator",
            114 => "MonoBehaviour",
            115 => "MonoScript",
            137 => "SkinnedMeshRenderer",
            213 => "Sprite",
            _ => "Unknown",
        }
    }
    pub fn read_name(&self, reader: &mut Reader, ref_types: &[crate::serializedfile::RefType]) -> Option<String> {
        if let Ok(value) = self.read_typetree(reader, ref_types) {
            if let Some(name_val) = value.get("m_Name") {
                if let Some(name) = name_val.as_str() {
                    if !name.is_empty() {
                        return Some(name.to_string());
                    }
                }
            }
        }
        None
    }
}
