pub mod sprite;
pub mod gameobject;
pub mod transform;
pub mod texture2d;
pub mod mesh;
pub mod skinned_mesh_renderer;
pub mod videoclip;
use crate::value::UnityValue;
use std::collections::BTreeMap;
pub fn map_field<T>(map: &BTreeMap<String, UnityValue>, key: &str) -> Result<T, String>
where T: TryFromUnityValue {
    if let Some(v) = map.get(key) {
        return T::try_from_unity_value(v).or_else(|_| T::try_from_missing_field().map_err(|_| format!("Field '{}' could not be mapped", key)));
    }
    let spaced_key = key.replace("_", " ");
    if let Some(v) = map.get(&spaced_key) {
        return T::try_from_unity_value(v).or_else(|_| T::try_from_missing_field().map_err(|_| format!("Field '{}' could not be mapped", spaced_key)));
    }
    T::try_from_missing_field().map_err(|_| format!("Missing field: {}", key))
}
pub trait TryFromUnityValue: Sized {
    fn try_from_unity_value(value: &UnityValue) -> Result<Self, String>;
    fn try_from_missing_field() -> Result<Self, ()> {
        Err(())
    }
}
impl<T: TryFromUnityValue> TryFromUnityValue for Option<T> {
    fn try_from_unity_value(value: &UnityValue) -> Result<Self, String> {
        match T::try_from_unity_value(value) {
            Ok(v) => Ok(Some(v)),
            Err(_) => Ok(None),
        }
    }
    fn try_from_missing_field() -> Result<Self, ()> {
        Ok(None)
    }
}
impl TryFromUnityValue for String {
    fn try_from_unity_value(value: &UnityValue) -> Result<Self, String> {
        match value {
            UnityValue::String(s) => Ok(s.clone()),
            _ => Err("Expected String".to_string()),
        }
    }
}
impl TryFromUnityValue for bool {
    fn try_from_unity_value(value: &UnityValue) -> Result<Self, String> {
        match value {
            UnityValue::Boolean(b) => Ok(*b),
            _ => Err("Expected Boolean".to_string()),
        }
    }
}
impl TryFromUnityValue for i32 {
    fn try_from_unity_value(value: &UnityValue) -> Result<Self, String> {
        match value {
            UnityValue::Int32(i) => Ok(*i),
            UnityValue::UInt32(u) => Ok(*u as i32),
            UnityValue::Int64(i) => Ok(*i as i32),
            UnityValue::UInt64(u) => Ok(*u as i32),
            UnityValue::Int16(i) => Ok(*i as i32),
            UnityValue::UInt16(u) => Ok(*u as i32),
            UnityValue::Int8(i) => Ok(*i as i32),
            UnityValue::UInt8(u) => Ok(*u as i32),
            _ => Err("Expected integer for i32".to_string()),
        }
    }
}
impl TryFromUnityValue for f32 {
    fn try_from_unity_value(value: &UnityValue) -> Result<Self, String> {
        match value {
            UnityValue::Float(f) => Ok(*f),
            _ => Err("Expected Float".to_string()),
        }
    }
}
impl TryFromUnityValue for f64 {
    fn try_from_unity_value(value: &UnityValue) -> Result<Self, String> {
        match value {
            UnityValue::Double(d) => Ok(*d),
            UnityValue::Float(f) => Ok(*f as f64),
            _ => Err("Expected Double or Float".to_string()),
        }
    }
}
#[derive(Debug, serde::Serialize, serde::Deserialize, Clone)]
pub struct TypelessData(pub Vec<u8>);
impl TryFromUnityValue for TypelessData {
    fn try_from_unity_value(value: &UnityValue) -> Result<Self, String> {
        match value {
            UnityValue::Bytes(b) => Ok(TypelessData(b.clone())),
            _ => Err("Expected Bytes (TypelessData)".to_string()),
        }
    }
}
impl TryFromUnityValue for u32 {
    fn try_from_unity_value(value: &UnityValue) -> Result<Self, String> {
        match value {
            UnityValue::UInt32(u) => Ok(*u),
            UnityValue::Int32(i) => Ok(*i as u32),
            UnityValue::UInt64(u) => Ok(*u as u32),
            UnityValue::Int64(i) => Ok(*i as u32),
            UnityValue::UInt16(u) => Ok(*u as u32),
            UnityValue::Int16(i) => Ok(*i as u32),
            UnityValue::UInt8(u) => Ok(*u as u32),
            UnityValue::Int8(i) => Ok(*i as u32),
            _ => Err("Expected integer for u32".to_string()),
        }
    }
}
impl TryFromUnityValue for u16 {
    fn try_from_unity_value(value: &UnityValue) -> Result<Self, String> {
        match value {
            UnityValue::UInt16(u) => Ok(*u),
            UnityValue::Int16(i) => Ok(*i as u16),
            UnityValue::Int32(i) => Ok(*i as u16),
            UnityValue::UInt32(u) => Ok(*u as u16),
            UnityValue::Int64(i) => Ok(*i as u16),
            UnityValue::UInt64(u) => Ok(*u as u16),
            UnityValue::Int8(i) => Ok(*i as u16),
            UnityValue::UInt8(u) => Ok(*u as u16),
            _ => Err("Expected integer for u16".to_string()),
        }
    }
}
impl TryFromUnityValue for u8 {
    fn try_from_unity_value(value: &UnityValue) -> Result<Self, String> {
        match value {
            UnityValue::UInt8(u) => Ok(*u),
            UnityValue::Int8(i) => Ok(*i as u8),
            UnityValue::Int32(i) => Ok(*i as u8),
            UnityValue::UInt32(u) => Ok(*u as u8),
            UnityValue::Int16(i) => Ok(*i as u8),
            UnityValue::UInt16(u) => Ok(*u as u8),
            UnityValue::Int64(i) => Ok(*i as u8),
            UnityValue::UInt64(u) => Ok(*u as u8),
            _ => Err("Expected integer for u8".to_string()),
        }
    }
}
impl<T: TryFromUnityValue> TryFromUnityValue for Vec<T> {
    fn try_from_unity_value(value: &UnityValue) -> Result<Self, String> {
        match value {
            UnityValue::Array(arr) => {
                arr.iter().map(|v| T::try_from_unity_value(v)).collect()
            }
            _ => Err("Expected Array".to_string()),
        }
    }
}
impl TryFromUnityValue for crate::math::Vector3 {
    fn try_from_unity_value(value: &UnityValue) -> Result<Self, String> {
        match value {
            UnityValue::Map(m) => {
                let x = map_field(m, "x")?;
                let y = map_field(m, "y")?;
                let z = map_field(m, "z")?;
                Ok(crate::math::Vector3 { x, y, z })
            }
            _ => Err("Expected Vector3 Map".to_string()),
        }
    }
}
impl TryFromUnityValue for crate::math::Quaternion {
    fn try_from_unity_value(value: &UnityValue) -> Result<Self, String> {
        match value {
            UnityValue::Map(m) => {
                let x = map_field(m, "x")?;
                let y = map_field(m, "y")?;
                let z = map_field(m, "z")?;
                let w = map_field(m, "w")?;
                Ok(crate::math::Quaternion { x, y, z, w })
            }
            _ => Err("Expected Quaternion Map".to_string()),
        }
    }
}
impl TryFromUnityValue for crate::math::Matrix4x4 {
    fn try_from_unity_value(value: &UnityValue) -> Result<Self, String> {
        match value {
            UnityValue::Map(m) => {
                let mut e = [0.0; 16];
                for i in 0..16 {
                    let row = i / 4;
                    let col = i % 4;
                    e[i] = map_field(m, &format!("e{}{}", row, col))?;
                }
                Ok(crate::math::Matrix4x4 { e })
            }
            _ => Err("Expected Matrix4x4 Map".to_string()),
        }
    }
}
#[derive(Debug, serde::Serialize, serde::Deserialize, Clone)]
pub struct PPtr {
    pub file_id: i32,
    pub path_id: i64,
}
impl TryFromUnityValue for PPtr {
    fn try_from_unity_value(value: &UnityValue) -> Result<Self, String> {
        match value {
            UnityValue::PPtr { file_id, path_id } => Ok(PPtr { file_id: *file_id, path_id: *path_id }),
            UnityValue::Map(m) => {
                let file_id = map_field(m, "m_FileID")?;
                let path_id = map_field(m, "m_PathID")?;
                Ok(PPtr { file_id, path_id })
            }
            _ => Err("Expected PPtr or Map".to_string()),
        }
    }
}
impl TryFromUnityValue for i64 {
    fn try_from_unity_value(value: &UnityValue) -> Result<Self, String> {
        match value {
            UnityValue::Int64(i) => Ok(*i),
            UnityValue::UInt64(u) => Ok(*u as i64),
            UnityValue::Int32(i) => Ok(*i as i64),
            UnityValue::UInt32(u) => Ok(*u as i64),
            UnityValue::Int16(i) => Ok(*i as i64),
            UnityValue::UInt16(u) => Ok(*u as i64),
            UnityValue::Int8(i) => Ok(*i as i64),
            UnityValue::UInt8(u) => Ok(*u as i64),
            _ => Err("Expected integer for i64".to_string()),
        }
    }
}
#[derive(Debug, serde::Serialize, serde::Deserialize, Clone)]
pub struct ComponentPair {
    pub component: PPtr,
}
impl TryFromUnityValue for ComponentPair {
    fn try_from_unity_value(value: &UnityValue) -> Result<Self, String> {
        match value {
            UnityValue::Map(m) => {
                let component = map_field(m, "component")?;
                Ok(ComponentPair { component })
            }
            _ => Err("Expected ComponentPair Map".to_string()),
        }
    }
}
#[macro_export]
macro_rules! define_unity_class {
    ($name:ident { $($field_name:ident : $field_type:ty),* $(,)? }) => {
        #[allow(non_snake_case)]
        #[derive(Debug, serde::Serialize, serde::Deserialize)]
        pub struct $name {
            $( pub $field_name : $field_type ),*
        }
        impl $name {
            pub fn from_value(value: $crate::value::UnityValue) -> Result<Self, String> {
                use $crate::classes::TryFromUnityValue;
                Self::try_from_unity_value(&value)
            }
        }
        impl $crate::classes::TryFromUnityValue for $name {
            fn try_from_unity_value(value: &$crate::value::UnityValue) -> Result<Self, String> {
                let map = match value {
                    $crate::value::UnityValue::Map(m) => m,
                    _ => return Err(format!("Value is not a Map for {}", stringify!($name))),
                };
                Ok($name {
                    $(
                        $field_name: $crate::classes::map_field(map, stringify!($field_name))?,
                    )*
                })
            }
        }
    };
}
define_unity_class!(StreamingInfo {
    offset: u64,
    size: u32,
    path: String,
});
impl TryFromUnityValue for u64 {
    fn try_from_unity_value(value: &UnityValue) -> Result<Self, String> {
        match value {
            UnityValue::UInt64(u) => Ok(*u),
            UnityValue::Int64(i) => Ok(*i as u64),
            UnityValue::UInt32(u) => Ok(*u as u64),
            UnityValue::Int32(i) => Ok(*i as u64),
            UnityValue::UInt16(u) => Ok(*u as u64),
            UnityValue::Int16(i) => Ok(*i as u64),
            UnityValue::UInt8(u) => Ok(*u as u64),
            UnityValue::Int8(i) => Ok(*i as u64),
            _ => Err("Expected integer for u64".to_string()),
        }
    }
}
