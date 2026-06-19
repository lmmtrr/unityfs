use crate::define_unity_class;
use crate::math::{Vector3, Quaternion};
use crate::classes::PPtr;
define_unity_class!(Transform {
    m_LocalRotation: Quaternion,
    m_LocalPosition: Vector3,
    m_LocalScale: Vector3,
    m_Children: Vec<PPtr>,
    m_Father: PPtr,
    m_GameObject: PPtr,
});
