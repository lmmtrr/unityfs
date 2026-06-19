use std::sync::Arc;
use crate::unity_version::UnityVersion;
use crate::math::*;

#[derive(Debug, Clone)]
pub enum ByteSource {
    Heap(Arc<[u8]>),
    Mmap(Arc<memmap2::Mmap>),
}
impl std::ops::Deref for ByteSource {
    type Target = [u8];
    fn deref(&self) -> &Self::Target {
        match self {
            ByteSource::Heap(arc) => arc,
            ByteSource::Mmap(mmap) => mmap,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Endian {
    Big,
    Little,
}
#[derive(Debug, Clone)]
pub struct Reader {
    pub data: ByteSource,
    pub pos: usize,
    pub endian: Endian,
    pub unity_version: UnityVersion,
}
impl Reader {
    pub fn new(data: Vec<u8>, unity_version: UnityVersion) -> Self {
        Self {
            data: ByteSource::Heap(Arc::from(data)),
            pos: 0,
            endian: Endian::Big,
            unity_version,
        }
    }
    pub fn new_mmap(mmap: memmap2::Mmap, unity_version: UnityVersion) -> Self {
        Self {
            data: ByteSource::Mmap(Arc::new(mmap)),
            pos: 0,
            endian: Endian::Big,
            unity_version,
        }
    }
    pub fn sub_reader(&self, offset: usize, size: usize) -> Self {
        self.absolute_reader(self.pos + offset, size)
    }
    pub fn absolute_reader(&self, pos: usize, size: usize) -> Self {
        let pos = pos.min(self.data.len());
        let _end = (pos + size).min(self.data.len());
        Self {
            data: self.data.clone(),
            pos,
            endian: self.endian,
            unity_version: self.unity_version.clone(),
        }
    }
    pub fn align(&mut self, alignment: usize) {
        if alignment == 0 { return; }
        self.pos = (self.pos + alignment - 1) & !(alignment - 1);
        if self.pos > self.data.len() {
            self.pos = self.data.len();
        }
    }
    pub fn read_bytes(&mut self, size: usize) -> Vec<u8> {
        if size > 100 * 1024 * 1024 {
            panic!("Extremely large read requested: {} bytes at pos {}. This is likely a parsing error.", size, self.pos);
        }
        if self.pos + size > self.data.len() {
            panic!("Out of bounds read of {} bytes at pos {} (len {})", size, self.pos, self.data.len());
        }
        let end = self.pos + size;
        let res = self.data[self.pos..end].to_vec();
        self.pos = end;
        res
    }
    pub fn read_u8(&mut self) -> u8 {
        if self.pos >= self.data.len() {
            panic!("Out of bounds read at pos {} (len {})", self.pos, self.data.len());
        }
        let val = self.data[self.pos];
        self.pos += 1;
        val
    }
    pub fn read_i8(&mut self) -> i8 {
        self.read_u8() as i8
    }
    fn read_fixed<const N: usize>(&mut self) -> [u8; N] {
        let mut buf = [0u8; N];
        if self.pos + N <= self.data.len() {
            buf.copy_from_slice(&self.data[self.pos..self.pos + N]);
            self.pos += N;
        } else {
            panic!("Out of bounds read of {} bytes at pos {} (len {})", N, self.pos, self.data.len());
        }
        buf
    }
    pub fn read_i16(&mut self) -> i16 {
        let buf = self.read_fixed::<2>();
        match self.endian {
            Endian::Big => i16::from_be_bytes(buf),
            Endian::Little => i16::from_le_bytes(buf),
        }
    }
    pub fn read_u16(&mut self) -> u16 {
        let buf = self.read_fixed::<2>();
        match self.endian {
            Endian::Big => u16::from_be_bytes(buf),
            Endian::Little => u16::from_le_bytes(buf),
        }
    }
    pub fn read_i32(&mut self) -> i32 {
        let buf = self.read_fixed::<4>();
        match self.endian {
            Endian::Big => i32::from_be_bytes(buf),
            Endian::Little => i32::from_le_bytes(buf),
        }
    }
    pub fn read_u32(&mut self) -> u32 {
        let buf = self.read_fixed::<4>();
        match self.endian {
            Endian::Big => u32::from_be_bytes(buf),
            Endian::Little => u32::from_le_bytes(buf),
        }
    }
    pub fn read_i64(&mut self) -> i64 {
        let buf = self.read_fixed::<8>();
        match self.endian {
            Endian::Big => i64::from_be_bytes(buf),
            Endian::Little => i64::from_le_bytes(buf),
        }
    }
    pub fn read_u64(&mut self) -> u64 {
        let buf = self.read_fixed::<8>();
        match self.endian {
            Endian::Big => u64::from_be_bytes(buf),
            Endian::Little => u64::from_le_bytes(buf),
        }
    }
    pub fn read_f32(&mut self) -> f32 {
        let buf = self.read_fixed::<4>();
        match self.endian {
            Endian::Big => f32::from_be_bytes(buf),
            Endian::Little => f32::from_le_bytes(buf),
        }
    }
    pub fn read_f64(&mut self) -> f64 {
        let buf = self.read_fixed::<8>();
        match self.endian {
            Endian::Big => f64::from_be_bytes(buf),
            Endian::Little => f64::from_le_bytes(buf),
        }
    }
    pub fn read_string_null(&mut self) -> String {
        let start = self.pos;
        let mut end = start;
        while end < self.data.len() && self.data[end] != 0 {
            end += 1;
        }
        let res = String::from_utf8_lossy(&self.data[start..end]).into_owned();
        self.pos = if end < self.data.len() { end + 1 } else { end };
        res
    }
    pub fn read_string(&mut self) -> String {
        let len = self.read_i32();
        if self.data.len() == 37352 {
        }
        if len <= 0 || len > 10_000_000 { return String::new(); }
        let bytes = self.read_bytes(len as usize);
        self.align(4);
        String::from_utf8_lossy(&bytes).into_owned()
    }
    pub fn read_vector2(&mut self) -> Vector2 {
        Vector2 { x: self.read_f32(), y: self.read_f32() }
    }
}
