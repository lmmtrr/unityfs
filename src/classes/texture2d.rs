use crate::define_unity_class;
define_unity_class!(Texture2D {
    m_Name: String,
    m_Width: i32,
    m_Height: i32,
    m_CompleteImageSize: u32,
    m_TextureFormat: i32,
    m_StreamData: Option<crate::classes::StreamingInfo>,
    image_data: Option<crate::classes::TypelessData>,
});
