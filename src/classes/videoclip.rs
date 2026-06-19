use crate::define_unity_class;
define_unity_class!(StreamedResource {
    m_Source: String,
    m_Offset: u64,
    m_Size: u64,
});
define_unity_class!(VideoClip {
    m_Name: String,
    m_OriginalPath: Option<String>,
    m_ProxyWidth: Option<u32>,
    m_ProxyHeight: Option<u32>,
    Width: Option<u32>,
    Height: Option<u32>,
    m_PixelAspecRatioNum: Option<u32>,
    m_PixelAspecRatioDen: Option<u32>,
    m_FrameRate: Option<f64>,
    m_FrameCount: Option<u64>,
    m_Format: Option<i32>,
    m_AudioChannelCount: Option<Vec<u16>>,
    m_AudioSampleRate: Option<Vec<u32>>,
    m_AudioLanguage: Option<Vec<String>>,
    m_ExternalResources: StreamedResource,
    m_HasSplitAlpha: Option<bool>,
    m_sRGB: Option<bool>,
});
