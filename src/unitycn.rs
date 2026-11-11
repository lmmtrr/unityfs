use std::sync::RwLock;
use aes::Aes128;
use aes::cipher::{BlockCipherEncrypt, KeyInit};
use crate::reader::Reader;

const SIGNATURE: &[u8; 16] = b"#$unity3dchina!@";

const KEY_ENV: &str = "UNITYCN_KEY";

static DECRYPT_KEY: RwLock<Option<[u8; 16]>> = RwLock::new(None);

pub fn set_decrypt_key(key: &[u8]) -> Result<(), String> {
    let key = if key.is_empty() {
        None
    } else {
        Some(<[u8; 16]>::try_from(key).map_err(|_| format!("UnityCN key must be 16 bytes, got {}", key.len()))?)
    };
    *DECRYPT_KEY.write().unwrap() = key;
    Ok(())
}

fn decrypt_key() -> Option<[u8; 16]> {
    let configured = *DECRYPT_KEY.read().unwrap();
    configured.or_else(|| std::env::var(KEY_ENV).ok()?.as_bytes().try_into().ok())
}

fn read_vector(reader: &mut Reader) -> Result<([u8; 16], [u8; 16]), String> {
    let block = |bytes: Vec<u8>| <[u8; 16]>::try_from(bytes).map_err(|_| "Truncated UnityCN header".to_string());
    let data = block(reader.read_bytes(16))?;
    let key = block(reader.read_bytes(16))?;
    reader.pos += 1;
    Ok((data, key))
}

fn unmask(key: [u8; 16], data: [u8; 16], cipher: &Aes128) -> [u8; 16] {
    let mut block = key.into();
    cipher.encrypt_block(&mut block);
    let mask: [u8; 16] = block.into();
    std::array::from_fn(|i| data[i] ^ mask[i])
}

pub struct Decryptor {
    index: [u8; 16],
    substitute: [u8; 16],
}

impl Decryptor {
    pub fn read(reader: &mut Reader) -> Result<Self, String> {
        reader.read_u32();
        let (data, key) = read_vector(reader)?;
        let (data_sig, key_sig) = read_vector(reader)?;
        let secret = decrypt_key().ok_or_else(|| format!("Bundle is UnityCN encrypted and no key is set ({})", KEY_ENV))?;
        let cipher = Aes128::new(&secret.into());
        if &unmask(key_sig, data_sig, &cipher) != SIGNATURE {
            return Err("UnityCN key does not match this bundle".to_string());
        }
        let plain = unmask(key, data, &cipher);
        let nibbles: Vec<u8> = plain.iter().flat_map(|b| [b >> 4, b & 0xF]).collect();
        let index = std::array::from_fn(|i| nibbles[i]);
        let substitute = std::array::from_fn(|i| nibbles[16 + (i % 4) * 4 + i / 4]);
        Ok(Self { index, substitute })
    }

    fn decrypt_byte(&self, data: &mut [u8], offset: &mut usize, index: &mut usize) -> u8 {
        let at = *offset;
        *offset += 1;
        let i = *index;
        *index += 1;
        let Some(value) = data.get_mut(at) else { return 0 };
        let sub = |k: usize| self.substitute[k] as i32;
        let b = sub(((i >> 2) & 3) + 4) + sub(i & 3) + sub(((i >> 4) & 3) + 8) + sub(((i & 0xFF) >> 6) + 12);
        let low = self.index[(*value & 0xF) as usize] as i32 - b;
        let high = self.index[(*value >> 4) as usize] as i32 - b;
        *value = (low & 0xF | 0x10 * high) as u8;
        *value
    }

    fn decrypt_sequence(&self, data: &mut [u8], start: usize, mut index: usize) -> usize {
        let mut offset = start;
        let token = self.decrypt_byte(data, &mut offset, &mut index);
        let mut literals = (token >> 4) as usize;
        if literals == 0xF {
            loop {
                let b = self.decrypt_byte(data, &mut offset, &mut index);
                literals += b as usize;
                if b != 0xFF || offset >= data.len() {
                    break;
                }
            }
        }
        offset += literals;
        if offset < data.len() {
            self.decrypt_byte(data, &mut offset, &mut index);
            self.decrypt_byte(data, &mut offset, &mut index);
            if token & 0xF == 0xF {
                while self.decrypt_byte(data, &mut offset, &mut index) == 0xFF && offset < data.len() {}
            }
        }
        offset
    }

    pub fn decrypt_block(&self, data: &mut [u8], mut index: usize) {
        let mut offset = 0;
        while offset < data.len() {
            offset = self.decrypt_sequence(data, offset, index);
            index += 1;
        }
    }
}
