use std::collections::BTreeMap;
use serde::{Serialize, Deserialize};
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum UnityValue {
    Boolean(bool),
    Int8(i8),
    UInt8(u8),
    Int16(i16),
    UInt16(u16),
    Int32(i32),
    UInt32(u32),
    Int64(i64),
    UInt64(u64),
    Float(f32),
    Double(f64),
    String(String),
    Bytes(Vec<u8>),
    Array(Vec<UnityValue>),
    Map(BTreeMap<String, UnityValue>),
    PPtr {
        #[serde(rename = "m_FileID")]
        file_id: i32,
        #[serde(rename = "m_PathID")]
        path_id: i64,
    },
    Null,
}
impl UnityValue {
    pub fn as_i32(&self) -> Option<i32> {
        match self {
            UnityValue::Int8(v) => Some(*v as i32),
            UnityValue::UInt8(v) => Some(*v as i32),
            UnityValue::Int16(v) => Some(*v as i32),
            UnityValue::UInt16(v) => Some(*v as i32),
            UnityValue::Int32(v) => Some(*v),
            UnityValue::UInt32(v) => Some(*v as i32),
            UnityValue::Int64(v) => Some(*v as i32),
            UnityValue::UInt64(v) => Some(*v as i32),
            _ => None,
        }
    }
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            UnityValue::Int8(v) => Some(*v as i64),
            UnityValue::UInt8(v) => Some(*v as i64),
            UnityValue::Int16(v) => Some(*v as i64),
            UnityValue::UInt16(v) => Some(*v as i64),
            UnityValue::Int32(v) => Some(*v as i64),
            UnityValue::UInt32(v) => Some(*v as i64),
            UnityValue::Int64(v) => Some(*v),
            UnityValue::UInt64(v) => Some(*v as i64),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            UnityValue::String(s) => Some(s),
            _ => None,
        }
    }
    pub fn get(&self, key: &str) -> Option<&UnityValue> {
        match self {
            UnityValue::Map(map) => map.get(key),
            _ => None,
        }
    }
}
