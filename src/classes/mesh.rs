use crate::define_unity_class;
use crate::math::Matrix4x4;
define_unity_class!(ChannelInfo {
    stream: u8,
    offset: u8,
    format: u8,
    dimension: u8,
});
define_unity_class!(VertexData {
    m_VertexCount: u32,
    m_Channels: Vec<ChannelInfo>,
    m_DataSize: Option<crate::classes::TypelessData>,
    _typelessdata: Option<crate::classes::TypelessData>,
});
define_unity_class!(SubMesh {
    firstByte: u32,
    indexCount: u32,
    topology: u32,
    baseVertex: u32,
    firstVertex: u32,
    vertexCount: u32,
});
#[derive(Debug, serde::Serialize, serde::Deserialize, Clone)]
pub struct PackedData(pub Vec<u8>);
impl crate::classes::TryFromUnityValue for PackedData {
    fn try_from_unity_value(value: &crate::value::UnityValue) -> Result<Self, String> {
        match value {
            crate::value::UnityValue::Bytes(b) => Ok(PackedData(b.clone())),
            crate::value::UnityValue::Array(arr) => {
                let mut bytes = Vec::with_capacity(arr.len());
                for val in arr {
                    bytes.push(u8::try_from_unity_value(val)?);
                }
                Ok(PackedData(bytes))
            }
            _ => Err("Expected Bytes or Array for PackedData".to_string()),
        }
    }
}
define_unity_class!(PackedBitVector {
    m_NumItems: u32,
    m_Range: Option<f32>,
    m_Start: Option<f32>,
    m_Data: PackedData,
    m_BitSize: u8,
});
impl PackedBitVector {
    pub fn unpack_ints(&self, start: usize, count: usize) -> Vec<u32> {
        let bit_size = self.m_BitSize as usize;
        if bit_size == 0 {
            return vec![0; count];
        }
        let data = &self.m_Data.0;
        let mut bit_pos = bit_size * start;
        let mut index_pos = bit_pos / 8;
        bit_pos %= 8;
        let mut result = Vec::with_capacity(count);
        for _ in 0..count {
            let mut bits = 0;
            let mut value: u64 = 0;
            while bits < bit_size {
                if index_pos >= data.len() {
                    break;
                }
                value |= (data[index_pos] as u64 >> bit_pos) << bits;
                let num = std::cmp::min(bit_size - bits, 8 - bit_pos);
                bit_pos += num;
                bits += num;
                if bit_pos == 8 {
                    index_pos += 1;
                    bit_pos = 0;
                }
            }
            let mask = (1u64 << bit_size) - 1;
            result.push((value & mask) as u32);
        }
        result
    }
    pub fn unpack_floats(&self, start: usize, count: usize) -> Vec<f32> {
        let bit_size = self.m_BitSize as usize;
        let start_val = self.m_Start.unwrap_or(0.0);
        let range_val = self.m_Range.unwrap_or(1.0);
        if bit_size == 0 {
            return vec![start_val; count];
        }
        let quantized_ints = self.unpack_ints(start, count);
        let max_val = ((1u64 << bit_size) - 1) as f64;
        let scale = range_val as f64 / max_val;
        quantized_ints.into_iter()
            .map(|x| (x as f64 * scale + start_val as f64) as f32)
            .collect()
    }
}
define_unity_class!(CompressedMesh {
    m_Vertices: Option<PackedBitVector>,
    m_UV: Option<PackedBitVector>,
    m_Normals: Option<PackedBitVector>,
    m_Tangents: Option<PackedBitVector>,
    m_Weights: Option<PackedBitVector>,
    m_NormalSigns: Option<PackedBitVector>,
    m_TangentSigns: Option<PackedBitVector>,
    m_FloatColors: Option<PackedBitVector>,
    m_BoneIndices: Option<PackedBitVector>,
    m_Triangles: Option<PackedBitVector>,
    m_UVInfo: Option<u32>,
});
define_unity_class!(Mesh {
    m_Name: String,
    m_VertexData: VertexData,
    m_IndexFormat: Option<u32>,
    m_IndexBuffer: Vec<u8>,
    m_BindPose: Option<Vec<Matrix4x4>>,
    m_BoneNameHashes: Option<Vec<u32>>,
    m_SubMeshes: Vec<SubMesh>,
    m_StreamData: Option<crate::classes::StreamingInfo>,
    m_MeshCompression: Option<u8>,
    m_CompressedMesh: Option<CompressedMesh>,
});
impl Mesh {
    pub fn get_vertex_count(&self) -> usize {
        let count = self.m_VertexData.m_VertexCount as usize;
        if count == 0 {
            if let Some(ref compressed) = self.m_CompressedMesh {
                if let Some(ref vertices) = compressed.m_Vertices {
                    return (vertices.m_NumItems / 3) as usize;
                }
            }
        }
        count
    }
    pub fn get_format_size(format: u8) -> usize {
        match format {
            0 => 4,
            1 => 2,
            2 | 3 | 6 | 7 => 1,
            4 | 5 | 8 | 9 => 2,
            10 | 11 => 4,
            _ => 4,
        }
    }
    pub fn get_stream_stride(&self, stream: u8) -> usize {
        let mut stride = 0;
        for ch in &self.m_VertexData.m_Channels {
            if ch.stream == stream && (ch.dimension & 0xF) > 0 {
                let size = Self::get_format_size(ch.format) * ((ch.dimension & 0xF) as usize);
                let end = (ch.offset as usize) + size;
                if end > stride {
                    stride = end;
                }
            }
        }
        (stride + 3) & !3
    }
    pub fn get_stream_offset(&self, stream: u8) -> usize {
        let mut offset = 0;
        let v_count = self.m_VertexData.m_VertexCount as usize;
        for s in 0..stream {
            let stride = self.get_stream_stride(s);
            if stride > 0 {
                let size = v_count * stride;
                offset += (size + 15) & !15;
            }
        }
        offset
    }
    pub fn get_vertices(&self) -> Result<Vec<crate::math::Vector3>, String> {
        if let Some(ref compressed) = self.m_CompressedMesh {
            if let Some(ref vertices) = compressed.m_Vertices {
                if vertices.m_NumItems > 0 {
                    let floats = vertices.unpack_floats(0, vertices.m_NumItems as usize);
                    let mut result = Vec::with_capacity(floats.len() / 3);
                    for chunk in floats.chunks_exact(3) {
                        result.push(crate::math::Vector3 {
                            x: chunk[0],
                            y: chunk[1],
                            z: chunk[2],
                        });
                    }
                    return Ok(result);
                }
            }
        }
        self.extract_float_vector3(0)
    }
    pub fn get_normals(&self) -> Result<Vec<crate::math::Vector3>, String> {
        if let Some(ref compressed) = self.m_CompressedMesh {
            if let Some(ref normals_pb) = compressed.m_Normals {
                if normals_pb.m_NumItems > 0 {
                    let vertex_count = self.get_vertex_count();
                    let has_signs = compressed.m_NormalSigns.as_ref()
                        .map(|s| s.m_NumItems > 0)
                        .unwrap_or(false);
                    if has_signs {
                        let normal_data = normals_pb.unpack_floats(0, vertex_count * 2);
                        let signs = compressed.m_NormalSigns.as_ref().unwrap().unpack_ints(0, vertex_count);
                        let mut result = Vec::with_capacity(vertex_count);
                        for i in 0..vertex_count {
                            if i * 2 + 1 >= normal_data.len() || i >= signs.len() {
                                break;
                            }
                            let x = normal_data[i * 2];
                            let y = normal_data[i * 2 + 1];
                            let sign = signs[i];
                            let zsqr = 1.0 - x * x - y * y;
                            let (rx, ry, mut rz) = if zsqr >= 0.0 {
                                (x, y, zsqr.sqrt())
                            } else {
                                let len = (x * x + y * y).sqrt();
                                if len > 0.00001 {
                                    (x / len, y / len, 0.0)
                                } else {
                                    (0.0, 0.0, 0.0)
                                }
                            };
                            if sign == 0 {
                                rz = -rz;
                            }
                            result.push(crate::math::Vector3 { x: rx, y: ry, z: rz });
                        }
                        return Ok(result);
                    } else {
                        let normal_data = normals_pb.unpack_floats(0, normals_pb.m_NumItems as usize);
                        let mut result = Vec::with_capacity(normal_data.len() / 3);
                        for chunk in normal_data.chunks_exact(3) {
                            result.push(crate::math::Vector3 {
                                x: chunk[0],
                                y: chunk[1],
                                z: chunk[2],
                            });
                        }
                        return Ok(result);
                    }
                }
            }
        }
        self.extract_float_vector3(1)
    }
    pub fn get_indices(&self) -> Result<Vec<u32>, String> {
        if let Some(ref compressed) = self.m_CompressedMesh {
            if let Some(ref triangles) = compressed.m_Triangles {
                if triangles.m_NumItems > 0 {
                    return Ok(triangles.unpack_ints(0, triangles.m_NumItems as usize));
                }
            }
        }
        let is_32bit = self.m_IndexFormat.unwrap_or(0) == 1;
        let mut result = Vec::with_capacity(self.m_IndexBuffer.len() / 2);
        let buf = &self.m_IndexBuffer;
        if is_32bit {
            if buf.len() % 4 != 0 { return Err("Index buffer length not a multiple of 4".to_string()); }
            for i in (0..buf.len()).step_by(4) {
                let idx = u32::from_le_bytes(buf[i..i+4].try_into().unwrap());
                result.push(idx);
            }
        } else {
            if buf.len() % 2 != 0 { return Err(format!("Index buffer length {} not a multiple of 2", buf.len())); }
            for i in (0..buf.len()).step_by(2) {
                let idx = u16::from_le_bytes(buf[i..i+2].try_into().unwrap());
                result.push(idx as u32);
            }
        }
        Ok(result)
    }
    pub fn extract_float_vector3(&self, channel: usize) -> Result<Vec<crate::math::Vector3>, String> {
        if channel >= self.m_VertexData.m_Channels.len() {
            return Err("Channel not found".to_string());
        }
        let ch = &self.m_VertexData.m_Channels[channel];
        if (ch.dimension & 0xF) < 3 {
            return Err("Channel dimension < 3".to_string());
        }
        if ch.format != 0 {
            return Err("Channel format is not float".to_string());
        }
        let data = self.m_VertexData.m_DataSize.as_ref()
            .or(self.m_VertexData._typelessdata.as_ref())
            .ok_or("No vertex data")?;
        let data_bytes = &data.0;
        let stride = self.get_stream_stride(ch.stream);
        let stream_offset = self.get_stream_offset(ch.stream);
        let count = self.m_VertexData.m_VertexCount as usize;
        let mut result = Vec::with_capacity(count);
        for i in 0..count {
            let offset = stream_offset + i * stride + (ch.offset as usize);
            if offset + 12 > data_bytes.len() {
                return Err("Out of bounds".to_string());
            }
            let x = f32::from_le_bytes(data_bytes[offset..offset+4].try_into().unwrap());
            let y = f32::from_le_bytes(data_bytes[offset+4..offset+8].try_into().unwrap());
            let z = f32::from_le_bytes(data_bytes[offset+8..offset+12].try_into().unwrap());
            result.push(crate::math::Vector3 { x, y, z });
        }
        Ok(result)
    }
    pub fn extract_uvs(&self) -> Result<Vec<(f32, f32)>, String> {
        if let Some(ref compressed) = self.m_CompressedMesh {
            if let Some(ref uv_pb) = compressed.m_UV {
                if uv_pb.m_NumItems > 0 {
                    let vertex_count = self.get_vertex_count();
                    let uv_info = compressed.m_UVInfo.unwrap_or(0);
                    if uv_info != 0 {
                        let mut uv_src_offset = 0;
                        let mut uv0_floats = None;
                        let mut uv0_dim = 2;
                        for uv_channel in 0..8 {
                            let mut tex_coord_bits = uv_info >> (uv_channel * 4);
                            tex_coord_bits &= 15;
                            if (tex_coord_bits & 4) != 0 {
                                let uv_dim = 1 + (tex_coord_bits & 3) as usize;
                                if uv_channel == 0 {
                                    uv0_floats = Some(uv_pb.unpack_floats(uv_src_offset, vertex_count * uv_dim));
                                    uv0_dim = uv_dim;
                                    break;
                                }
                                uv_src_offset += uv_dim * vertex_count;
                            }
                        }
                        if let Some(floats) = uv0_floats {
                            let mut result = Vec::with_capacity(vertex_count);
                            for i in 0..vertex_count {
                                let u = if uv0_dim >= 1 && i * uv0_dim < floats.len() { floats[i * uv0_dim] } else { 0.0 };
                                let v = if uv0_dim >= 2 && i * uv0_dim + 1 < floats.len() { floats[i * uv0_dim + 1] } else { 0.0 };
                                result.push((u, v));
                            }
                            return Ok(result);
                        }
                    } else {
                        let floats = uv_pb.unpack_floats(0, vertex_count * 2);
                        let mut result = Vec::with_capacity(vertex_count);
                        for i in 0..vertex_count {
                            let u = if i * 2 < floats.len() { floats[i * 2] } else { 0.0 };
                            let v = if i * 2 + 1 < floats.len() { floats[i * 2 + 1] } else { 0.0 };
                            result.push((u, v));
                        }
                        return Ok(result);
                    }
                }
            }
        }
        let channel = 4;
        if channel >= self.m_VertexData.m_Channels.len() {
            return Err("UV Channel not found".to_string());
        }
        let ch = &self.m_VertexData.m_Channels[channel];
        if (ch.dimension & 0xF) < 2 {
            return Err("UV Channel dimension < 2".to_string());
        }
        let data = self.m_VertexData.m_DataSize.as_ref()
            .or(self.m_VertexData._typelessdata.as_ref())
            .ok_or("No vertex data")?;
        let data_bytes = &data.0;
        let stride = self.get_stream_stride(ch.stream);
        let stream_offset = self.get_stream_offset(ch.stream);
        let count = self.m_VertexData.m_VertexCount as usize;
        let mut result = Vec::with_capacity(count);
        for i in 0..count {
            let offset = stream_offset + i * stride + (ch.offset as usize);
            let (u, v) = if ch.format == 0 {
                if offset + 8 > data_bytes.len() { return Err("Out of bounds".to_string()); }
                let u = f32::from_le_bytes(data_bytes[offset..offset+4].try_into().unwrap());
                let v = f32::from_le_bytes(data_bytes[offset+4..offset+8].try_into().unwrap());
                (u, v)
            } else if ch.format == 1 {
                if offset + 4 > data_bytes.len() { return Err("Out of bounds".to_string()); }
                let u_raw = u16::from_le_bytes(data_bytes[offset..offset+2].try_into().unwrap());
                let v_raw = u16::from_le_bytes(data_bytes[offset+2..offset+4].try_into().unwrap());
                (f16_to_f32(u_raw), f16_to_f32(v_raw))
            } else {
                return Err(format!("Unsupported UV format {}", ch.format));
            };
            result.push((u, v));
        }
        Ok(result)
    }
    pub fn extract_bone_weights(&self) -> Result<Vec<crate::math::Vector4>, String> {
        if let Some(ref compressed) = self.m_CompressedMesh {
            if let Some(ref weights_pb) = compressed.m_Weights {
                if weights_pb.m_NumItems > 0 {
                    let vertex_count = self.get_vertex_count();
                    let weights_data = weights_pb.unpack_ints(0, weights_pb.m_NumItems as usize);
                    let bone_indices_data = if let Some(ref indices_pb) = compressed.m_BoneIndices {
                        indices_pb.unpack_ints(0, indices_pb.m_NumItems as usize)
                    } else {
                        Vec::new()
                    };
                    let mut bone_indices_iter = bone_indices_data.into_iter();
                    let mut weights_iter = weights_data.into_iter();
                    let mut result = vec![crate::math::Vector4 { x: 0.0, y: 0.0, z: 0.0, w: 0.0 }; vertex_count];
                    let mut vertex_index = 0;
                    let mut j = 0;
                    let mut sum = 0;
                    while vertex_index < vertex_count {
                        if let Some(weight) = weights_iter.next() {
                            let _bone_index = bone_indices_iter.next().unwrap_or(0);
                            let w_val = weight as f32 / 31.0;
                            match j {
                                0 => result[vertex_index].x = w_val,
                                1 => result[vertex_index].y = w_val,
                                2 => result[vertex_index].z = w_val,
                                3 => result[vertex_index].w = w_val,
                                _ => {}
                            }
                            j += 1;
                            sum += weight;
                            if sum >= 31 {
                                vertex_index += 1;
                                j = 0;
                                sum = 0;
                            } else if j == 3 {
                                let w_val_4 = (31.0 - sum as f32) / 31.0;
                                result[vertex_index].w = w_val_4;
                                let _bone_index_4 = bone_indices_iter.next().unwrap_or(0);
                                vertex_index += 1;
                                j = 0;
                                sum = 0;
                            }
                        } else {
                            break;
                        }
                    }
                    return Ok(result);
                }
            }
        }
        let channel = 12;
        if channel >= self.m_VertexData.m_Channels.len() {
            return Err("BoneWeight Channel not found".to_string());
        }
        let ch = &self.m_VertexData.m_Channels[channel];
        let count = self.m_VertexData.m_VertexCount as usize;
        if (ch.dimension & 0xF) == 0 {
            return Ok(vec![crate::math::Vector4 { x: 1.0, y: 0.0, z: 0.0, w: 0.0 }; count]);
        }
        if ch.format != 0 {
            return Err(format!("Unsupported BoneWeight format: dim={}, format={}", ch.dimension, ch.format));
        }
        let data = self.m_VertexData.m_DataSize.as_ref()
            .or(self.m_VertexData._typelessdata.as_ref())
            .ok_or("No vertex data")?;
        let data_bytes = &data.0;
        let stride = self.get_stream_stride(ch.stream);
        let stream_offset = self.get_stream_offset(ch.stream);
        let mut result = Vec::with_capacity(count);
        for i in 0..count {
            let offset = stream_offset + i * stride + (ch.offset as usize);
            let dim = (ch.dimension & 0xF) as usize;
            if offset + (dim * 4) > data_bytes.len() { return Err("Out of bounds".to_string()); }
            let x = if dim >= 1 { f32::from_le_bytes(data_bytes[offset..offset+4].try_into().unwrap()) } else { 1.0 };
            let y = if dim >= 2 { f32::from_le_bytes(data_bytes[offset+4..offset+8].try_into().unwrap()) } else { 0.0 };
            let z = if dim >= 3 { f32::from_le_bytes(data_bytes[offset+8..offset+12].try_into().unwrap()) } else { 0.0 };
            let w = if dim >= 4 { f32::from_le_bytes(data_bytes[offset+12..offset+16].try_into().unwrap()) } else { 0.0 };
            result.push(crate::math::Vector4 { x, y, z, w });
        }
        Ok(result)
    }
    pub fn extract_bone_indices(&self) -> Result<Vec<[u32; 4]>, String> {
        if let Some(ref compressed) = self.m_CompressedMesh {
            if let Some(ref weights_pb) = compressed.m_Weights {
                if weights_pb.m_NumItems > 0 {
                    let vertex_count = self.get_vertex_count();
                    let weights_data = weights_pb.unpack_ints(0, weights_pb.m_NumItems as usize);
                    let bone_indices_data = if let Some(ref indices_pb) = compressed.m_BoneIndices {
                        indices_pb.unpack_ints(0, indices_pb.m_NumItems as usize)
                    } else {
                        Vec::new()
                    };
                    let mut bone_indices_iter = bone_indices_data.into_iter();
                    let mut weights_iter = weights_data.into_iter();
                    let mut result = vec![[0, 0, 0, 0]; vertex_count];
                    let mut vertex_index = 0;
                    let mut j = 0;
                    let mut sum = 0;
                    while vertex_index < vertex_count {
                        if let Some(weight) = weights_iter.next() {
                            let bone_index = bone_indices_iter.next().unwrap_or(0);
                            if j < 4 {
                                result[vertex_index][j] = bone_index;
                            }
                            j += 1;
                            sum += weight;
                            if sum >= 31 {
                                vertex_index += 1;
                                j = 0;
                                sum = 0;
                            } else if j == 3 {
                                let bone_index_4 = bone_indices_iter.next().unwrap_or(0);
                                result[vertex_index][3] = bone_index_4;
                                vertex_index += 1;
                                j = 0;
                                sum = 0;
                            }
                        } else {
                            break;
                        }
                    }
                    return Ok(result);
                }
            }
        }
        let channel = 13;
        if channel >= self.m_VertexData.m_Channels.len() {
            return Err("BlendIndices Channel not found".to_string());
        }
        let ch = &self.m_VertexData.m_Channels[channel];
        let count = self.m_VertexData.m_VertexCount as usize;
        if (ch.dimension & 0xF) == 0 {
            return Ok(vec![[0, 0, 0, 0]; count]);
        }
        let data = self.m_VertexData.m_DataSize.as_ref()
            .or(self.m_VertexData._typelessdata.as_ref())
            .ok_or("No vertex data")?;
        let data_bytes = &data.0;
        let stride = self.get_stream_stride(ch.stream);
        let stream_offset = self.get_stream_offset(ch.stream);
        let mut result = Vec::with_capacity(count);
        let dim = (ch.dimension & 0xF) as usize;
        for i in 0..count {
            let offset = stream_offset + i * stride + (ch.offset as usize);
            if ch.format == 10 || ch.format == 11 {
                if offset + dim * 4 > data_bytes.len() { return Err("Out of bounds".to_string()); }
                let i0 = if dim >= 1 { u32::from_le_bytes(data_bytes[offset..offset+4].try_into().unwrap()) } else { 0 };
                let i1 = if dim >= 2 { u32::from_le_bytes(data_bytes[offset+4..offset+8].try_into().unwrap()) } else { 0 };
                let i2 = if dim >= 3 { u32::from_le_bytes(data_bytes[offset+8..offset+12].try_into().unwrap()) } else { 0 };
                let i3 = if dim >= 4 { u32::from_le_bytes(data_bytes[offset+12..offset+16].try_into().unwrap()) } else { 0 };
                result.push([i0, i1, i2, i3]);
            } else if ch.format == 2 || ch.format == 3 || ch.format == 6 || ch.format == 7 {
                if offset + dim > data_bytes.len() { return Err("Out of bounds".to_string()); }
                let i0 = if dim >= 1 { data_bytes[offset] as u32 } else { 0 };
                let i1 = if dim >= 2 { data_bytes[offset+1] as u32 } else { 0 };
                let i2 = if dim >= 3 { data_bytes[offset+2] as u32 } else { 0 };
                let i3 = if dim >= 4 { data_bytes[offset+3] as u32 } else { 0 };
                result.push([i0, i1, i2, i3]);
            } else if ch.format == 4 || ch.format == 5 || ch.format == 8 || ch.format == 9 {
                if offset + dim * 2 > data_bytes.len() { return Err("Out of bounds".to_string()); }
                let i0 = if dim >= 1 { u16::from_le_bytes(data_bytes[offset..offset+2].try_into().unwrap()) as u32 } else { 0 };
                let i1 = if dim >= 2 { u16::from_le_bytes(data_bytes[offset+2..offset+4].try_into().unwrap()) as u32 } else { 0 };
                let i2 = if dim >= 3 { u16::from_le_bytes(data_bytes[offset+4..offset+6].try_into().unwrap()) as u32 } else { 0 };
                let i3 = if dim >= 4 { u16::from_le_bytes(data_bytes[offset+6..offset+8].try_into().unwrap()) as u32 } else { 0 };
                result.push([i0, i1, i2, i3]);
            } else {
                return Err(format!("Unsupported BlendIndices format {}", ch.format));
            }
        }
        Ok(result)
    }
}
pub fn f16_to_f32(h: u16) -> f32 {
    let sign = ((h >> 15) & 1) as u32;
    let exp = ((h >> 10) & 0x1f) as u32;
    let frac = (h & 0x3ff) as u32;
    if exp == 0 {
        if frac == 0 {
            f32::from_bits(sign << 31)
        } else {
            f32::from_bits((sign << 31) | ((frac + 0x3f800) << 13))
        }
    } else if exp == 0x1f {
        f32::from_bits((sign << 31) | 0x7f800000 | (frac << 13))
    } else {
        f32::from_bits((sign << 31) | ((exp + 112) << 23) | (frac << 13))
    }
}
