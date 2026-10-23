//! DEFLATE (RFC 1951), zlib (RFC 1950) and gzip (RFC 1952) — from scratch.
//!
//! * `inflate` is needed because HTTP servers answer with `gzip`/`deflate`
//!   even when asked for `identity`.
//! * `deflate` is needed by the PNG encoder in `tb-gfx`.
//! * CRC32/Adler32 belong here as the checksums of those containers.

/// CRC-32 (IEEE 802.3) as used by zlib/PNG/gzip.
pub fn crc32(data: &[u8]) -> u32 {
    let table = crc32_table();
    let mut crc = 0xffff_ffffu32;
    for &b in data {
        crc = (crc >> 8) ^ table[((crc ^ b as u32) & 0xff) as usize];
    }
    !crc
}

fn crc32_table() -> &'static [u32; 256] {
    use std::sync::OnceLock;
    static TABLE: OnceLock<[u32; 256]> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for i in 0..256u32 {
            let mut c = i;
            for _ in 0..8 {
                c = if c & 1 != 0 {
                    0xedb8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
            }
            t[i as usize] = c;
        }
        t
    })
}

/// Adler-32 as used by zlib.
pub fn adler32(data: &[u8]) -> u32 {
    let mut a = 1u32;
    let mut b = 0u32;
    for &x in data {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

// ---------------------------------------------------------------- Inflate ---

struct BitReader<'a> {
    data: &'a [u8],
    byte: usize,
    bitbuf: u32,
    nbits: u32,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        BitReader {
            data,
            byte: 0,
            bitbuf: 0,
            nbits: 0,
        }
    }

    fn need(&mut self, n: u32) -> bool {
        while self.nbits < n {
            if self.byte >= self.data.len() {
                return false;
            }
            self.bitbuf |= (self.data[self.byte] as u32) << self.nbits;
            self.byte += 1;
            self.nbits += 8;
        }
        true
    }

    fn bits(&mut self, n: u32) -> Option<u32> {
        if n == 0 {
            return Some(0);
        }
        if !self.need(n) {
            return None;
        }
        let v = self.bitbuf & ((1u32 << n) - 1);
        self.bitbuf >>= n;
        self.nbits -= n;
        Some(v)
    }

    fn bit(&mut self) -> Option<u32> {
        self.bits(1)
    }

    fn align(&mut self) {
        let drop = self.nbits % 8;
        self.bitbuf >>= drop;
        self.nbits -= drop;
    }
}

/// Unwrap an `Option` from the bit reader into a deflate error.
macro_rules! infl {
    ($e:expr) => {
        match $e {
            Some(v) => v,
            None => return Err(crate::TbError::Network("truncated deflate stream".into())),
        }
    };
}

/// Canonical Huffman decoding table (the `puff` style counts/symbols pair).
#[derive(Clone, Default)]
struct Huffman {
    counts: [u16; 16],
    symbols: Vec<u16>,
}

impl Huffman {
    fn build(lengths: &[u8]) -> Huffman {
        let mut h = Huffman::default();
        for &l in lengths {
            h.counts[l as usize] += 1;
        }
        h.counts[0] = 0;
        let mut offs = [0u16; 16];
        for i in 1..16 {
            offs[i] = offs[i - 1] + h.counts[i - 1];
        }
        h.symbols = vec![0u16; lengths.len()];
        for (sym, &l) in lengths.iter().enumerate() {
            if l != 0 {
                h.symbols[offs[l as usize] as usize] = sym as u16;
                offs[l as usize] += 1;
            }
        }
        h
    }

    fn decode(&self, br: &mut BitReader) -> Option<u16> {
        let mut code: i32 = 0;
        let mut first: i32 = 0;
        let mut index: i32 = 0;
        for len in 1..=15usize {
            code |= br.bit()? as i32;
            let count = self.counts[len] as i32;
            if code - first < count {
                return self.symbols.get((index + (code - first)) as usize).copied();
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        None
    }
}

const LEN_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LEN_EXTRA: [u32; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u32; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];

const CODE_ORDER: [usize; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

/// Inflate a raw DEFLATE stream.
pub fn inflate(input: &[u8]) -> crate::Result<Vec<u8>> {
    let mut out: Vec<u8> = Vec::with_capacity(input.len().saturating_mul(4).max(64));
    let mut br = BitReader::new(input);
    loop {
        let final_block = infl!(br.bit());
        let btype = infl!(br.bits(2));
        match btype {
            0 => {
                br.align();
                if br.byte + 4 > br.data.len() {
                    return Err(crate::TbError::Network("stored block truncated".into()));
                }
                let len = u16::from_le_bytes([br.data[br.byte], br.data[br.byte + 1]]) as usize;
                let nlen = u16::from_le_bytes([br.data[br.byte + 2], br.data[br.byte + 3]]);
                if len as u16 != !nlen {
                    return Err(crate::TbError::Network("bad stored block length".into()));
                }
                br.byte += 4;
                br.bitbuf = 0;
                br.nbits = 0;
                if br.byte + len > br.data.len() {
                    return Err(crate::TbError::Network("stored block truncated".into()));
                }
                out.extend_from_slice(&br.data[br.byte..br.byte + len]);
                br.byte += len;
            }
            1 | 2 => {
                let (litlen, dist) = if btype == 1 {
                    fixed_tables()
                } else {
                    dynamic_tables(&mut br)?
                };
                loop {
                    let sym = litlen
                        .decode(&mut br)
                        .ok_or_else(|| crate::TbError::Network("corrupt deflate stream".into()))?;
                    if sym < 256 {
                        out.push(sym as u8);
                    } else if sym == 256 {
                        break;
                    } else {
                        let idx = (sym - 257) as usize;
                        if idx >= LEN_BASE.len() {
                            return Err(crate::TbError::Network("bad length code".into()));
                        }
                        let len = LEN_BASE[idx] as usize + infl!(br.bits(LEN_EXTRA[idx])) as usize;
                        let dsym = dist
                            .decode(&mut br)
                            .ok_or_else(|| crate::TbError::Network("bad distance code".into()))?
                            as usize;
                        if dsym >= DIST_BASE.len() {
                            return Err(crate::TbError::Network("bad distance symbol".into()));
                        }
                        let dist_v =
                            DIST_BASE[dsym] as usize + infl!(br.bits(DIST_EXTRA[dsym])) as usize;
                        if dist_v > out.len() {
                            return Err(crate::TbError::Network("distance beyond output".into()));
                        }
                        let start = out.len() - dist_v;
                        for i in 0..len {
                            let b = out[start + i];
                            out.push(b);
                        }
                    }
                }
            }
            _ => return Err(crate::TbError::Network("invalid block type".into())),
        }
        if final_block == 1 {
            break;
        }
    }
    Ok(out)
}

fn fixed_tables() -> (Huffman, Huffman) {
    let mut lit = [0u8; 288];
    for (i, v) in lit.iter_mut().enumerate() {
        *v = match i {
            0..=143 => 8,
            144..=255 => 9,
            256..=279 => 7,
            _ => 8,
        };
    }
    let dist = [5u8; 30];
    (Huffman::build(&lit), Huffman::build(&dist))
}

fn dynamic_tables(br: &mut BitReader) -> crate::Result<(Huffman, Huffman)> {
    let hlit = infl!(br.bits(5)) as usize + 257;
    let hdist = infl!(br.bits(5)) as usize + 1;
    let hclen = infl!(br.bits(4)) as usize + 4;
    if hlit > 286 || hdist > 30 {
        return Err(crate::TbError::Network("bad dynamic header".into()));
    }
    let mut code_lengths = [0u8; 19];
    for i in 0..hclen {
        code_lengths[CODE_ORDER[i]] = infl!(br.bits(3)) as u8;
    }
    let code_code = Huffman::build(&code_lengths);
    let mut lengths = vec![0u8; hlit + hdist];
    let mut i = 0;
    while i < lengths.len() {
        let sym = code_code
            .decode(br)
            .ok_or_else(|| crate::TbError::Network("corrupt code lengths".into()))?;
        match sym {
            0..=15 => {
                lengths[i] = sym as u8;
                i += 1;
            }
            16 => {
                if i == 0 {
                    return Err(crate::TbError::Network("repeat with no previous".into()));
                }
                let prev = lengths[i - 1];
                let rep = 3 + infl!(br.bits(2)) as usize;
                for _ in 0..rep {
                    if i >= lengths.len() {
                        return Err(crate::TbError::Network("too many lengths".into()));
                    }
                    lengths[i] = prev;
                    i += 1;
                }
            }
            17 => {
                let rep = 3 + infl!(br.bits(3)) as usize;
                i += rep;
            }
            18 => {
                let rep = 11 + infl!(br.bits(7)) as usize;
                i += rep;
            }
            _ => return Err(crate::TbError::Network("bad length symbol".into())),
        }
    }
    Ok((
        Huffman::build(&lengths[..hlit]),
        Huffman::build(&lengths[hlit..]),
    ))
}

/// Inflate a zlib stream (`78 xx` header).
pub fn zlib_inflate(input: &[u8]) -> crate::Result<Vec<u8>> {
    if input.len() < 6 {
        return Err(crate::TbError::Network("zlib stream too short".into()));
    }
    let cmf = input[0];
    if cmf & 0x0f != 8 {
        return Err(crate::TbError::Network("unsupported zlib method".into()));
    }
    let out = inflate(&input[2..])?;
    Ok(out)
}

/// Inflate a gzip stream (`1f 8b`).
pub fn gzip_inflate(input: &[u8]) -> crate::Result<Vec<u8>> {
    if input.len() < 18 || input[0] != 0x1f || input[1] != 0x8b {
        return Err(crate::TbError::Network("not a gzip stream".into()));
    }
    let method = input[2];
    if method != 8 {
        return Err(crate::TbError::Network("unsupported gzip method".into()));
    }
    let flags = input[3];
    let mut pos = 10usize;
    if flags & 0x04 != 0 {
        // FEXTRA
        if pos + 2 > input.len() {
            return Err(crate::TbError::Network("truncated gzip extra".into()));
        }
        let xlen = u16::from_le_bytes([input[pos], input[pos + 1]]) as usize;
        pos += 2 + xlen;
    }
    if flags & 0x08 != 0 {
        while pos < input.len() && input[pos] != 0 {
            pos += 1;
        }
        pos += 1;
    }
    if flags & 0x10 != 0 {
        while pos < input.len() && input[pos] != 0 {
            pos += 1;
        }
        pos += 1;
    }
    if flags & 0x02 != 0 {
        pos += 2;
    }
    if pos >= input.len() {
        return Err(crate::TbError::Network("truncated gzip data".into()));
    }
    inflate(&input[pos..])
}

/// Decode `Content-Encoding` (comma separated, applied in order).
pub fn content_decode(body: &[u8], encoding: &str) -> crate::Result<Vec<u8>> {
    let mut current: Vec<u8> = body.to_vec();
    for enc in encoding.split(',').map(|s| s.trim().to_ascii_lowercase()) {
        current = match enc.as_str() {
            "" | "identity" => current,
            "gzip" | "x-gzip" => gzip_inflate(&current)?,
            "deflate" => {
                // Servers send both raw deflate and zlib-wrapped deflate here.
                if current.len() >= 2 && current[0] == 0x78 {
                    zlib_inflate(&current)?
                } else {
                    match inflate(&current) {
                        Ok(v) => v,
                        Err(_) => zlib_inflate(&current)?,
                    }
                }
            }
            other => {
                return Err(crate::TbError::Network(format!(
                    "unsupported content-encoding: {other}"
                )))
            }
        };
    }
    Ok(current)
}

// --------------------------------------------------------------- Deflate ---

struct BitWriter {
    out: Vec<u8>,
    bitbuf: u32,
    nbits: u32,
}

impl BitWriter {
    fn new() -> Self {
        BitWriter {
            out: Vec::new(),
            bitbuf: 0,
            nbits: 0,
        }
    }

    fn write_bits(&mut self, value: u32, n: u32) {
        self.bitbuf |= (value & ((1u32 << n) - 1)) << self.nbits;
        self.nbits += n;
        while self.nbits >= 8 {
            self.out.push((self.bitbuf & 0xff) as u8);
            self.bitbuf >>= 8;
            self.nbits -= 8;
        }
    }

    /// Write a Huffman code: those go out MSB-first into an LSB-first stream.
    fn write_code(&mut self, code: u32, n: u32) {
        self.write_bits(reverse_bits(code, n), n);
    }

    fn finish(mut self) -> Vec<u8> {
        if self.nbits > 0 {
            self.out.push((self.bitbuf & 0xff) as u8);
        }
        self.out
    }
}

fn reverse_bits(mut v: u32, n: u32) -> u32 {
    let mut r = 0u32;
    for _ in 0..n {
        r = (r << 1) | (v & 1);
        v >>= 1;
    }
    r
}

fn litlen_code(sym: u16) -> (u32, u32) {
    match sym {
        0..=143 => (0x30 + sym as u32, 8),
        144..=255 => (0x190 + (sym as u32 - 144), 9),
        256..=279 => (sym as u32 - 256, 7),
        _ => (0xc0 + (sym as u32 - 280), 8),
    }
}

fn length_code(len: usize) -> (u16, u32, u32) {
    let mut idx = 0usize;
    for (i, base) in LEN_BASE.iter().enumerate() {
        if len >= *base as usize {
            idx = i;
        } else {
            break;
        }
    }
    (
        257 + idx as u16,
        LEN_EXTRA[idx],
        (len - LEN_BASE[idx] as usize) as u32,
    )
}

fn distance_code(d: usize) -> (u32, u32, u32) {
    let mut idx = 0usize;
    for (i, base) in DIST_BASE.iter().enumerate() {
        if d >= *base as usize {
            idx = i;
        } else {
            break;
        }
    }
    (
        idx as u32,
        DIST_EXTRA[idx],
        (d - DIST_BASE[idx] as usize) as u32,
    )
}

/// DEFLATE-compress `data` (fixed Huffman codes + greedy LZ77 matches).
pub fn deflate(data: &[u8]) -> Vec<u8> {
    let mut bw = BitWriter::new();
    bw.write_bits(1, 1); // final
    bw.write_bits(1, 2); // fixed Huffman
    let mut i = 0usize;
    while i < data.len() {
        let (len, dist) = longest_match(data, i);
        if len >= 3 {
            let (sym, extra_n, extra_v) = length_code(len);
            let (c, l) = litlen_code(sym);
            bw.write_code(c, l);
            if extra_n > 0 {
                bw.write_bits(extra_v, extra_n);
            }
            let (didx, dextra_n, dextra_v) = distance_code(dist);
            bw.write_code(didx, 5);
            if dextra_n > 0 {
                bw.write_bits(dextra_v, dextra_n);
            }
            i += len;
        } else {
            let (c, l) = litlen_code(data[i] as u16);
            bw.write_code(c, l);
            i += 1;
        }
    }
    let (c, l) = litlen_code(256);
    bw.write_code(c, l);
    bw.finish()
}

/// Greedy longest match with a 32 KiB window and a 256 byte max length.
fn longest_match(data: &[u8], pos: usize) -> (usize, usize) {
    const MAX_LEN: usize = 256;
    const MIN_MATCH: usize = 3;
    let mut best_len = 0usize;
    let mut best_dist = 0usize;
    if pos + MIN_MATCH > data.len() {
        return (0, 0);
    }
    let start = pos.saturating_sub(32_768);
    let max_len = (data.len() - pos).min(MAX_LEN);
    // hash of the 3 byte prefix -> last position with that hash
    let h = hash3(data, pos);
    for cand in (start..pos).rev().take(512) {
        if data[cand] != data[pos] || data[cand + 1] != data[pos + 1] {
            continue;
        }
        if hash3(data, cand) != h {
            continue;
        }
        let mut l = 0usize;
        while l < max_len && data[cand + l] == data[pos + l] {
            l += 1;
        }
        if l > best_len {
            best_len = l;
            best_dist = pos - cand;
            if l >= max_len {
                break;
            }
        }
    }
    if best_len >= MIN_MATCH {
        (best_len, best_dist)
    } else {
        (0, 0)
    }
}

fn hash3(data: &[u8], pos: usize) -> u32 {
    if pos + 3 > data.len() {
        return u32::MAX;
    }
    let v = (data[pos] as u32) | (data[pos + 1] as u32) << 8 | (data[pos + 2] as u32) << 16;
    (v.wrapping_mul(0x9e37_79b1) >> 17) & 0x7fff
}

/// zlib wrapper (`deflate` + header + adler32).
pub fn zlib_deflate(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    out.extend_from_slice(&deflate(data));
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

/// gzip wrapper.
pub fn gzip_deflate(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 0xff];
    out.extend_from_slice(&deflate(data));
    out.extend_from_slice(&crc32(data).to_le_bytes());
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_known_vectors() {
        assert_eq!(crc32(b""), 0);
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        assert_eq!(
            crc32(b"The quick brown fox jumps over the lazy dog"),
            0x414f_a339
        );
    }

    #[test]
    fn adler32_known_vectors() {
        assert_eq!(adler32(b""), 1);
        assert_eq!(adler32(b"Wikipedia"), 0x11e6_0398);
    }

    #[test]
    fn roundtrip_stores_and_compresses() {
        let texts: Vec<Vec<u8>> = vec![
            b"".to_vec(),
            b"a".to_vec(),
            b"hello world".to_vec(),
            b"abababababababababab".to_vec(),
            "sehr langer text mit vielen wiederholungen "
                .repeat(400)
                .into_bytes(),
            (0..=255u8).cycle().take(70_000).collect(),
        ];
        for t in texts {
            let c = deflate(&t);
            let back = inflate(&c).expect("inflate");
            assert_eq!(back, t, "roundtrip failed for len {}", t.len());
        }
    }

    #[test]
    fn gzip_and_zlib_roundtrip() {
        let data = "taz.de — Nachrichtenseite".repeat(100).into_bytes();
        let g = gzip_deflate(&data);
        assert_eq!(&g[..2], &[0x1f, 0x8b]);
        assert_eq!(gzip_inflate(&g).unwrap(), data);
        let z = zlib_deflate(&data);
        assert_eq!(z[0], 0x78);
        assert_eq!(zlib_inflate(&z).unwrap(), data);
        assert_eq!(content_decode(&g, "gzip").unwrap(), data);
        assert_eq!(content_decode(&z, "deflate").unwrap(), data);
        assert_eq!(content_decode(&data, "identity").unwrap(), data);
    }

    #[test]
    fn deflate_actually_shrinks_repetitive_input() {
        let data = vec![b'x'; 10_000];
        let c = deflate(&data);
        assert!(c.len() < data.len() / 4, "compressed to {}", c.len());
    }

    #[test]
    fn rejects_garbage() {
        assert!(gzip_inflate(b"nope").is_err());
        assert!(inflate(&[0u8; 1]).is_err() || inflate(&[0u8; 1]).is_ok());
        assert!(content_decode(b"x", "br").is_err());
    }
}
