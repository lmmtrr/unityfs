use crate::define_unity_class;
use crate::classes::PPtr;
define_unity_class!(SkinnedMeshRenderer {
    m_GameObject: PPtr,
    m_Mesh: PPtr,
    m_Bones: Vec<PPtr>,
    m_RootBone: PPtr,
});
