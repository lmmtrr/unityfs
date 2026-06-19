use crate::define_unity_class;
define_unity_class!(GameObject {
    m_Name: String,
    m_IsActive: bool,
    m_Layer: u32,
    m_Component: Vec<crate::classes::ComponentPair>,
});
