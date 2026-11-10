use once_cell::sync::Lazy;
use std::collections::HashMap;

const IMA_STEPS: [i32; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66, 73, 80, 88, 97, 107, 118, 130, 143,
    157, 173, 190, 209, 230, 253, 279, 307, 337, 371, 408, 449, 494, 544, 598, 658, 724, 796, 876, 963, 1060, 1166, 1282, 1411,
    1552, 1707, 1878, 2066, 2272, 2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894, 6484, 7132, 7845, 8630, 9493, 10442,
    11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794, 32767,
];

const IMA_INDEX: [i32; 8] = [-1, -1, -1, -1, 2, 4, 6, 8];

const SAMPLE_RATES: [u32; 11] = [4000, 8000, 11000, 11025, 16000, 22050, 24000, 32000, 44100, 48000, 96000];

static VORBIS_SETUPS: Lazy<HashMap<u32, Vec<u8>>> = Lazy::new(|| {
    let mut raw = Vec::new();
    let mut map = HashMap::new();
    if lzma_rs::lzma_decompress(&mut &include_bytes!("vorbis_setup.bin")[..], &mut raw).is_err() {
        return map;
    }
    let mut pos = 0;
    while pos + 8 <= raw.len() {
        let crc = u32::from_le_bytes(raw[pos..pos + 4].try_into().unwrap());
        let len = u32::from_le_bytes(raw[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let Some(data) = raw.get(pos + 8..pos + 8 + len) else { break };
        map.insert(crc, data.to_vec());
        pos += 8 + len;
    }
    map
});

struct Stream<'a> {
    mode: u32,
    channels: u16,
    rate: u32,
    samples: u32,
    vorbis_crc: Option<u32>,
    data: &'a [u8],
}

fn u32_at(d: &[u8], pos: usize) -> Option<u32> {
    d.get(pos..pos + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()))
}

fn parse(d: &[u8]) -> Option<Stream<'_>> {
    if !d.starts_with(b"FSB5") {
        return None;
    }
    let version = u32_at(d, 4)?;
    let headers_size = u32_at(d, 12)? as usize;
    let names_size = u32_at(d, 16)? as usize;
    let data_size = u32_at(d, 20)? as usize;
    let mode = u32_at(d, 24)?;
    let base = if version == 0 { 64 } else { 60 };
    let raw = u64::from_le_bytes(d.get(base..base + 8)?.try_into().ok()?);
    let mut stream = Stream {
        mode,
        channels: [1, 2, 6, 8][((raw >> 5) & 3) as usize],
        rate: *SAMPLE_RATES.get(((raw >> 1) & 0xF) as usize)?,
        samples: ((raw >> 34) & 0x3FFF_FFFF) as u32,
        vorbis_crc: None,
        data: &[],
    };
    let mut pos = base + 8;
    let mut more = raw & 1 != 0;
    while more {
        let chunk = u32_at(d, pos)?;
        more = chunk & 1 != 0;
        let size = ((chunk >> 1) & 0xFF_FFFF) as usize;
        let body = pos + 4;
        match chunk >> 25 {
            1 => stream.channels = *d.get(body)? as u16,
            2 => stream.rate = u32_at(d, body)?,
            11 => stream.vorbis_crc = u32_at(d, body),
            14 => stream.channels *= u32_at(d, body)? as u16,
            _ => {}
        }
        pos = body + size;
    }
    let data_start = base + headers_size + names_size;
    let offset = (((raw >> 7) & 0x7FF_FFFF) * 32) as usize;
    let end = match u64::from_le_bytes(d.get(pos..pos + 8).unwrap_or(&[0; 8]).try_into().ok()?) {
        next if pos + 8 <= base + headers_size && next != 0 => ((next >> 7) & 0x7FF_FFFF) as usize * 32,
        _ => data_size,
    };
    stream.data = d.get(data_start + offset..(data_start + end.max(offset)).min(d.len()))?;
    (stream.channels > 0).then_some(stream)
}

fn wav(format: u16, channels: u16, rate: u32, bits: u16, pcm: &[u8]) -> Vec<u8> {
    let align = channels * bits / 8;
    let mut out = Vec::with_capacity(44 + pcm.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + pcm.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&format.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * align as u32).to_le_bytes());
    out.extend_from_slice(&align.to_le_bytes());
    out.extend_from_slice(&bits.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
    out.extend_from_slice(pcm);
    out
}

fn ima_nibble(nibble: u8, sample: &mut i32, index: &mut i32) -> i16 {
    let step = IMA_STEPS[*index as usize];
    let mut diff = step >> 3;
    if nibble & 1 != 0 {
        diff += step >> 2;
    }
    if nibble & 2 != 0 {
        diff += step >> 1;
    }
    if nibble & 4 != 0 {
        diff += step;
    }
    *sample = if nibble & 8 != 0 { *sample - diff } else { *sample + diff }.clamp(-32768, 32767);
    *index = (*index + IMA_INDEX[(nibble & 7) as usize]).clamp(0, 88);
    *sample as i16
}

fn decode_ima(s: &Stream) -> Vec<u8> {
    let ch = s.channels as usize;
    let block = 0x24 * ch;
    let mut out = Vec::new();
    for frame in s.data.chunks_exact(block) {
        let mut decoded = vec![[0i16; 64]; ch];
        for (c, samples) in decoded.iter_mut().enumerate() {
            let mut sample = i16::from_le_bytes([frame[c * 4], frame[c * 4 + 1]]) as i32;
            let mut index = (frame[c * 4 + 2] as i32).clamp(0, 88);
            samples[0] = sample as i16;
            for i in 0..63 {
                let byte = frame[4 * ch + (i / 8) * 4 * ch + c * 4 + (i % 8) / 2];
                samples[i + 1] = ima_nibble(if i % 2 == 0 { byte & 0xF } else { byte >> 4 }, &mut sample, &mut index);
            }
        }
        for i in 0..64 {
            for samples in &decoded {
                out.extend_from_slice(&samples[i].to_le_bytes());
            }
        }
    }
    if s.samples > 0 {
        out.truncate(s.samples as usize * ch * 2);
    }
    wav(1, s.channels, s.rate, 16, &out)
}

fn ogg_crc(data: &[u8]) -> u32 {
    static TABLE: Lazy<[u32; 256]> = Lazy::new(|| {
        let mut table = [0u32; 256];
        for (i, entry) in table.iter_mut().enumerate() {
            let mut r = (i as u32) << 24;
            for _ in 0..8 {
                r = if r & 0x8000_0000 != 0 { (r << 1) ^ 0x04C1_1DB7 } else { r << 1 };
            }
            *entry = r;
        }
        table
    });
    data.iter().fold(0u32, |crc, &b| (crc << 8) ^ TABLE[((crc >> 24) as u8 ^ b) as usize])
}

struct Ogg {
    out: Vec<u8>,
    serial: u32,
    seq: u32,
}

impl Ogg {
    fn packet(&mut self, data: &[u8], granule: i64, bos: bool, eos: bool) {
        let mut lacing = vec![255u8; data.len() / 255];
        lacing.push((data.len() % 255) as u8);
        let pages: Vec<&[u8]> = lacing.chunks(255).collect();
        let mut offset = 0;
        for (i, segments) in pages.iter().enumerate() {
            let last = i + 1 == pages.len();
            let size: usize = segments.iter().map(|&s| s as usize).sum();
            let flags = (i > 0) as u8 | ((bos && i == 0) as u8) << 1 | ((eos && last) as u8) << 2;
            let start = self.out.len();
            self.out.extend_from_slice(b"OggS\0");
            self.out.push(flags);
            self.out.extend_from_slice(&(if last { granule } else { -1 }).to_le_bytes());
            self.out.extend_from_slice(&self.serial.to_le_bytes());
            self.out.extend_from_slice(&self.seq.to_le_bytes());
            self.out.extend_from_slice(&[0; 4]);
            self.out.push(segments.len() as u8);
            self.out.extend_from_slice(segments);
            self.out.extend_from_slice(&data[offset..offset + size]);
            let crc = ogg_crc(&self.out[start..]);
            self.out[start + 22..start + 26].copy_from_slice(&crc.to_le_bytes());
            offset += size;
            self.seq += 1;
        }
    }
}

fn vorbis_packets(data: &[u8]) -> Vec<&[u8]> {
    let mut packets = Vec::new();
    let mut pos = 0;
    while let Some(size) = data.get(pos..pos + 2).map(|b| u16::from_le_bytes([b[0], b[1]]) as usize) {
        if size == 0 || size == 0xFFFF {
            break;
        }
        let Some(packet) = data.get(pos + 2..pos + 2 + size) else { break };
        packets.push(packet);
        pos += 2 + size;
    }
    packets
}

fn rebuild_vorbis(s: &Stream) -> Option<Vec<u8>> {
    let setup = VORBIS_SETUPS.get(&s.vorbis_crc?)?;
    let channels = u8::try_from(s.channels).ok()?;
    let mut ident = vec![1];
    ident.extend_from_slice(b"vorbis");
    ident.extend_from_slice(&0u32.to_le_bytes());
    ident.push(channels);
    ident.extend_from_slice(&s.rate.to_le_bytes());
    ident.extend_from_slice(&[0; 12]);
    ident.extend_from_slice(&[0xB8, 1]);
    let mut comment = vec![3];
    comment.extend_from_slice(b"vorbis");
    comment.extend_from_slice(&[0; 8]);
    comment.push(1);
    let ident_header = lewton::header::read_header_ident(&ident).ok()?;
    let setup_header = lewton::header::read_header_setup(setup, channels, (8, 11)).ok()?;
    let packets = vorbis_packets(s.data);
    let mut ogg = Ogg { out: Vec::new(), serial: s.vorbis_crc?, seq: 0 };
    ogg.packet(&ident, 0, true, false);
    ogg.packet(&comment, 0, false, false);
    ogg.packet(setup, 0, false, packets.is_empty());
    let mut granule = 0i64;
    for (i, packet) in packets.iter().enumerate() {
        let count = lewton::audio::get_decoded_sample_count(&ident_header, &setup_header, packet).ok()?;
        if i > 0 {
            granule += count as i64;
        }
        let last = i + 1 == packets.len();
        let pos = if last && s.samples > 0 { granule.min(s.samples as i64) } else { granule };
        ogg.packet(packet, pos, false, last);
    }
    Some(ogg.out)
}

pub fn convert(d: &[u8]) -> Option<(Vec<u8>, &'static str)> {
    let s = parse(d)?;
    let pcm_bits = |bits: u16| {
        let size = (s.samples as usize * s.channels as usize * bits as usize / 8).min(s.data.len());
        wav(if s.mode == 5 { 3 } else { 1 }, s.channels, s.rate, bits, &s.data[..if s.samples > 0 { size } else { s.data.len() }])
    };
    match s.mode {
        1 => Some((pcm_bits(8), "wav")),
        2 => Some((pcm_bits(16), "wav")),
        3 => Some((pcm_bits(24), "wav")),
        4 | 5 => Some((pcm_bits(32), "wav")),
        7 => Some((decode_ima(&s), "wav")),
        11 => Some((s.data.to_vec(), "mp3")),
        15 => rebuild_vorbis(&s).map(|o| (o, "ogg")),
        _ => None,
    }
}
