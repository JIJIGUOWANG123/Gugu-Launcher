//! 极简 PNG 读写：只服务「64×32 老皮肤自动补齐成 64×64」这一件事。
//!
//! 不引入第三方 crate：inflate 按 RFC 1951 手写（stored / fixed / dynamic 三种块），
//! 输出侧只用 stored 块（体积大一点，但皮肤图几十 KB，无损且实现简单）。

use crate::error::{AppError, Result};

// ------------------------------------------------------------------ 校验和

/// PNG 每个 chunk 的 CRC32（多项式 0xEDB88320）。
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for (index, slot) in table.iter_mut().enumerate() {
        let mut value = index as u32;
        for _ in 0..8 {
            value = if value & 1 != 0 {
                0xEDB8_8320 ^ (value >> 1)
            } else {
                value >> 1
            };
        }
        *slot = value;
    }
    let mut crc = 0xFFFF_FFFFu32;
    for byte in bytes {
        crc = table[((crc ^ *byte as u32) & 0xFF) as usize] ^ (crc >> 8);
    }
    crc ^ 0xFFFF_FFFF
}

/// zlib 的 adler32。
pub fn adler32(bytes: &[u8]) -> u32 {
    let (mut low, mut high) = (1u32, 0u32);
    for byte in bytes {
        low = (low + *byte as u32) % 65521;
        high = (high + low) % 65521;
    }
    (high << 16) | low
}

// ------------------------------------------------------------------ inflate

struct BitReader<'a> {
    data: &'a [u8],
    /// 下一个待读字节下标
    position: usize,
    /// 位缓冲（低位在前）
    buffer: u32,
    count: u8,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            position: 0,
            buffer: 0,
            count: 0,
        }
    }

    fn need(&mut self, bits: u8) -> Result<()> {
        while self.count < bits {
            let byte = *self
                .data
                .get(self.position)
                .ok_or_else(|| AppError::Other("PNG 数据提前结束".to_string()))?;
            self.position += 1;
            self.buffer |= (byte as u32) << self.count;
            self.count += 8;
        }
        Ok(())
    }

    fn bits(&mut self, count: u8) -> Result<u32> {
        if count == 0 {
            return Ok(0);
        }
        self.need(count)?;
        let value = self.buffer & ((1u32 << count) - 1);
        self.buffer >>= count;
        self.count -= count;
        Ok(value)
    }

    /// 丢弃当前字节里剩下的位（stored 块对齐用）。
    fn align(&mut self) {
        let drop = self.count % 8;
        self.buffer >>= drop;
        self.count -= drop;
    }
}

struct Huffman {
    counts: [u16; 16],
    symbols: Vec<u16>,
}

impl Huffman {
    fn new(lengths: &[u8]) -> Result<Self> {
        let mut counts = [0u16; 16];
        for length in lengths {
            counts[*length as usize] += 1;
        }
        counts[0] = 0;
        let mut offsets = [0u16; 16];
        let mut total = 0u16;
        for length in 1..16 {
            offsets[length] = total;
            total += counts[length];
        }
        let mut symbols = vec![0u16; total as usize];
        for (symbol, length) in lengths.iter().enumerate() {
            if *length == 0 {
                continue;
            }
            let slot = &mut offsets[*length as usize];
            symbols[*slot as usize] = symbol as u16;
            *slot += 1;
        }
        Ok(Self { counts, symbols })
    }

    fn decode(&self, reader: &mut BitReader<'_>) -> Result<u16> {
        let mut code = 0i32;
        let mut first = 0i32;
        let mut index = 0i32;
        for length in 1..16 {
            code |= reader.bits(1)? as i32;
            let count = self.counts[length] as i32;
            if code - count < first {
                let slot = (index + (code - first)) as usize;
                return self
                    .symbols
                    .get(slot)
                    .copied()
                    .ok_or_else(|| AppError::Other("PNG 霍夫曼码越界".to_string()));
            }
            index += count;
            first += count;
            first <<= 1;
            code <<= 1;
        }
        Err(AppError::Other("PNG 霍夫曼码不完整".to_string()))
    }
}

const LENGTH_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LENGTH_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];
/// 动态霍夫曼里码长码表的传输顺序。
const CODE_LENGTH_ORDER: [usize; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

fn fixed_tables() -> (Huffman, Huffman) {
    let mut literals = [0u8; 288];
    for (index, slot) in literals.iter_mut().enumerate() {
        *slot = match index {
            0..=143 => 8,
            144..=255 => 9,
            256..=279 => 7,
            _ => 8,
        };
    }
    let distances = [5u8; 30];
    (
        Huffman::new(&literals).expect("固定表合法"),
        Huffman::new(&distances).expect("固定表合法"),
    )
}

fn inflate_block(
    reader: &mut BitReader<'_>,
    out: &mut Vec<u8>,
    literals: &Huffman,
    distances: &Huffman,
) -> Result<()> {
    loop {
        let symbol = literals.decode(reader)?;
        match symbol {
            0..=255 => out.push(symbol as u8),
            256 => return Ok(()),
            _ => {
                let index = (symbol as usize).saturating_sub(257);
                let length = *LENGTH_BASE
                    .get(index)
                    .ok_or_else(|| AppError::Other("PNG 长度码越界".to_string()))?
                    as usize
                    + reader.bits(LENGTH_EXTRA[index])? as usize;
                let dist_symbol = distances.decode(reader)? as usize;
                let distance = *DIST_BASE
                    .get(dist_symbol)
                    .ok_or_else(|| AppError::Other("PNG 距离码越界".to_string()))?
                    as usize
                    + reader.bits(DIST_EXTRA[dist_symbol])? as usize;
                if distance == 0 || distance > out.len() {
                    return Err(AppError::Other("PNG 回溯距离越界".to_string()));
                }
                let start = out.len() - distance;
                for offset in 0..length {
                    let byte = out[start + offset];
                    out.push(byte);
                }
            }
        }
    }
}

/// raw deflate 解压。
pub fn inflate_raw(data: &[u8]) -> Result<Vec<u8>> {
    let mut reader = BitReader::new(data);
    let mut out = Vec::new();
    loop {
        let last = reader.bits(1)? == 1;
        match reader.bits(2)? {
            0 => {
                reader.align();
                let length = reader.bits(16)? as usize;
                let inverse = reader.bits(16)? as usize;
                if length != (!inverse & 0xFFFF) {
                    return Err(AppError::Other("PNG stored 块长度校验失败".to_string()));
                }
                for _ in 0..length {
                    out.push(reader.bits(8)? as u8);
                }
            }
            1 => {
                let (literals, distances) = fixed_tables();
                inflate_block(&mut reader, &mut out, &literals, &distances)?;
            }
            2 => {
                let literal_count = reader.bits(5)? as usize + 257;
                let distance_count = reader.bits(5)? as usize + 1;
                let code_count = reader.bits(4)? as usize + 4;
                let mut code_lengths = [0u8; 19];
                for index in 0..code_count {
                    code_lengths[CODE_LENGTH_ORDER[index]] = reader.bits(3)? as u8;
                }
                let code_table = Huffman::new(&code_lengths)?;
                let total = literal_count + distance_count;
                let mut lengths = Vec::with_capacity(total);
                while lengths.len() < total {
                    let symbol = code_table.decode(&mut reader)?;
                    match symbol {
                        0..=15 => lengths.push(symbol as u8),
                        16 => {
                            let repeat = reader.bits(2)? as usize + 3;
                            let previous = *lengths
                                .last()
                                .ok_or_else(|| AppError::Other("PNG 码长重复缺前值".to_string()))?;
                            lengths.extend(std::iter::repeat(previous).take(repeat));
                        }
                        17 => {
                            let repeat = reader.bits(3)? as usize + 3;
                            lengths.extend(std::iter::repeat(0u8).take(repeat));
                        }
                        _ => {
                            let repeat = reader.bits(7)? as usize + 11;
                            lengths.extend(std::iter::repeat(0u8).take(repeat));
                        }
                    }
                }
                lengths.truncate(total);
                let literals = Huffman::new(&lengths[..literal_count])?;
                let distances = Huffman::new(&lengths[literal_count..])?;
                inflate_block(&mut reader, &mut out, &literals, &distances)?;
            }
            _ => return Err(AppError::Other("PNG 压缩块类型非法".to_string())),
        }
        if last {
            return Ok(out);
        }
    }
}

/// zlib（带 2 字节头 + adler32 尾）解压。
pub fn inflate_zlib(data: &[u8]) -> Result<Vec<u8>> {
    if data.len() < 6 {
        return Err(AppError::Other("PNG zlib 数据太短".to_string()));
    }
    let (cmf, flg) = (data[0], data[1]);
    if cmf & 0x0F != 8 {
        return Err(AppError::Other("PNG 只支持 deflate 压缩".to_string()));
    }
    if ((cmf as u16) << 8 | flg as u16) % 31 != 0 {
        return Err(AppError::Other("PNG zlib 头校验失败".to_string()));
    }
    if flg & 0x20 != 0 {
        return Err(AppError::Other("PNG 不支持预设字典".to_string()));
    }
    inflate_raw(&data[2..])
}

/// zlib 压缩（只用 stored 块，永远不会失败）。
pub fn deflate_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    let mut chunks = data.chunks(65535).peekable();
    if data.is_empty() {
        out.extend_from_slice(&[0x01, 0x00, 0x00, 0xFF, 0xFF]);
    }
    while let Some(chunk) = chunks.next() {
        let last = chunks.peek().is_none();
        out.push(if last { 1 } else { 0 });
        let length = chunk.len() as u16;
        out.extend_from_slice(&length.to_le_bytes());
        out.extend_from_slice(&(!length).to_le_bytes());
        out.extend_from_slice(chunk);
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

// ------------------------------------------------------------------ PNG 编解码

/// 解码后的位图：统一转成 RGBA8。
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

fn chunk(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 12);
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(payload);
    let mut crc_input = Vec::with_capacity(payload.len() + 4);
    crc_input.extend_from_slice(kind);
    crc_input.extend_from_slice(payload);
    out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
    out
}

fn paeth(left: u8, up: u8, up_left: u8) -> u8 {
    let base = left as i32 + up as i32 - up_left as i32;
    let (dl, du, dul) = (
        (base - left as i32).abs(),
        (base - up as i32).abs(),
        (base - up_left as i32).abs(),
    );
    if dl <= du && dl <= dul {
        left
    } else if du <= dul {
        up
    } else {
        up_left
    }
}

fn decode_png_inner(bytes: &[u8]) -> Result<Rgba> {
    const SIGNATURE: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];
    if bytes.len() < 8 || bytes[..8] != SIGNATURE {
        return Err(AppError::Invalid("不是 PNG 图片".to_string()));
    }
    let mut position = 8usize;
    let mut width = 0u32;
    let mut height = 0u32;
    let mut depth = 0u8;
    let mut color_type = 0u8;
    let mut interlace = 0u8;
    let mut data = Vec::new();
    while position + 8 <= bytes.len() {
        let length = u32::from_be_bytes([
            bytes[position],
            bytes[position + 1],
            bytes[position + 2],
            bytes[position + 3],
        ]) as usize;
        let kind = &bytes[position + 4..position + 8];
        let start = position + 8;
        let end = start
            .checked_add(length)
            .filter(|end| *end <= bytes.len())
            .ok_or_else(|| AppError::Other("PNG chunk 越界".to_string()))?;
        let payload = &bytes[start..end];
        match kind {
            b"IHDR" => {
                if payload.len() < 13 {
                    return Err(AppError::Other("PNG IHDR 长度不对".to_string()));
                }
                width = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
                height = u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]);
                depth = payload[8];
                color_type = payload[9];
                interlace = payload[12];
            }
            b"IDAT" => data.extend_from_slice(payload),
            b"IEND" => break,
            _ => {}
        }
        position = end + 4;
    }
    if width == 0 || height == 0 {
        return Err(AppError::Other("PNG 缺少 IHDR".to_string()));
    }
    if depth != 8 {
        return Err(AppError::Invalid(format!(
            "只支持 8 位深度的 PNG（当前 {depth} 位）"
        )));
    }
    if interlace != 0 {
        return Err(AppError::Invalid("不支持隔行扫描的 PNG".to_string()));
    }
    let channels = match color_type {
        0 => 1usize,
        2 => 3,
        4 => 2,
        6 => 4,
        other => {
            return Err(AppError::Invalid(format!(
                "不支持的 PNG 色彩类型：{other}"
            )))
        }
    };
    let raw = inflate_zlib(&data)?;
    let stride = width as usize * channels;
    if raw.len() < (stride + 1) * height as usize {
        return Err(AppError::Other("PNG 像素数据不完整".to_string()));
    }
    let mut pixels = vec![0u8; width as usize * height as usize * 4];
    let mut previous = vec![0u8; stride];
    let mut current = vec![0u8; stride];
    for row in 0..height as usize {
        let base = row * (stride + 1);
        let filter = raw[base];
        current.copy_from_slice(&raw[base + 1..base + 1 + stride]);
        for index in 0..stride {
            let left = if index >= channels {
                current[index - channels]
            } else {
                0
            };
            let up = previous[index];
            let up_left = if index >= channels {
                previous[index - channels]
            } else {
                0
            };
            let value = current[index];
            current[index] = match filter {
                0 => value,
                1 => value.wrapping_add(left),
                2 => value.wrapping_add(up),
                3 => value.wrapping_add(((left as u16 + up as u16) / 2) as u8),
                4 => value.wrapping_add(paeth(left, up, up_left)),
                other => {
                    return Err(AppError::Other(format!(
                        "PNG 行过滤器非法：{other}"
                    )))
                }
            };
        }
        for column in 0..width as usize {
            let source = column * channels;
            let target = (row * width as usize + column) * 4;
            match channels {
                1 => {
                    pixels[target] = current[source];
                    pixels[target + 1] = current[source];
                    pixels[target + 2] = current[source];
                    pixels[target + 3] = 255;
                }
                2 => {
                    pixels[target] = current[source];
                    pixels[target + 1] = current[source];
                    pixels[target + 2] = current[source];
                    pixels[target + 3] = current[source + 1];
                }
                3 => {
                    pixels[target] = current[source];
                    pixels[target + 1] = current[source + 1];
                    pixels[target + 2] = current[source + 2];
                    pixels[target + 3] = 255;
                }
                _ => {
                    pixels[target] = current[source];
                    pixels[target + 1] = current[source + 1];
                    pixels[target + 2] = current[source + 2];
                    pixels[target + 3] = current[source + 3];
                }
            }
        }
        std::mem::swap(&mut previous, &mut current);
    }
    Ok(Rgba {
        width,
        height,
        pixels,
    })
}

/// PNG 解码（8 位深度、非隔行、灰度/真彩/带 alpha）。
pub fn decode_png(bytes: &[u8]) -> Result<Rgba> {
    decode_png_inner(bytes)
}

/// PNG 编码（RGBA8，stored deflate）。
pub fn encode_png(width: u32, height: u32, pixels: &[u8]) -> Result<Vec<u8>> {
    let expected = width as usize * height as usize * 4;
    if pixels.len() != expected {
        return Err(AppError::Other("PNG 像素缓冲区大小不对".to_string()));
    }
    let mut raw = Vec::with_capacity(expected + height as usize);
    for row in 0..height as usize {
        raw.push(0); // 过滤器：None
        raw.extend_from_slice(&pixels[row * width as usize * 4..(row + 1) * width as usize * 4]);
    }
    let mut header = Vec::with_capacity(13);
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[8, 6, 0, 0, 0]);
    let mut out = vec![137, 80, 78, 71, 13, 10, 26, 10];
    out.extend_from_slice(&chunk(b"IHDR", &header));
    out.extend_from_slice(&chunk(b"IDAT", &deflate_stored(&raw)));
    out.extend_from_slice(&chunk(b"IEND", &[]));
    Ok(out)
}

/// 只读 IHDR 拿尺寸（不解压，校验上传时用）。
pub fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    const SIGNATURE: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];
    if bytes.len() < 24 || bytes[..8] != SIGNATURE || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    Some((width, height))
}

// ------------------------------------------------------------------ 老皮肤补齐

/// 64×32 → 64×64：上半部分原样搬，右腿/右臂由左侧对应区域补齐。
pub fn legacy_to_modern(bytes: &[u8]) -> Result<Vec<u8>> {
    let image = decode_png(bytes)?;
    if (image.width, image.height) != (64, 32) {
        return Err(AppError::Invalid(format!(
            "只有 64×32 的皮肤需要补齐（当前 {}×{}）",
            image.width, image.height
        )));
    }
    let mut out = vec![0u8; 64 * 64 * 4];
    let copy = |out: &mut Vec<u8>, sx: u32, sy: u32, dx: u32, dy: u32, w: u32, h: u32| {
        for row in 0..h {
            for column in 0..w {
                let source = (((sy + row) * 64 + sx + column) * 4) as usize;
                let target = (((dy + row) * 64 + dx + column) * 4) as usize;
                out[target..target + 4].copy_from_slice(&image.pixels[source..source + 4]);
            }
        }
    };
    // 原有内容（头部、身体、手臂、腿的上半截）
    copy(&mut out, 0, 0, 0, 0, 64, 32);
    // 右腿：老布局里左腿下半截在 (0,16)
    copy(&mut out, 0, 16, 16, 48, 16, 16);
    // 右臂：老布局里左臂下半截在 (40,16)
    copy(&mut out, 40, 16, 32, 48, 16, 16);
    encode_png(64, 64, &out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid_png(width: u32, height: u32, color: [u8; 4]) -> Vec<u8> {
        let mut pixels = vec![0u8; (width * height * 4) as usize];
        for chunk in pixels.chunks_mut(4) {
            chunk.copy_from_slice(&color);
        }
        encode_png(width, height, &pixels).unwrap()
    }

    #[test]
    fn checksums_match_reference_values() {
        assert_eq!(crc32(b"IEND"), 0xAE42_6082);
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    }

    #[test]
    fn encode_then_decode_round_trips() {
        let png = solid_png(8, 4, [10, 20, 30, 255]);
        let size = png_size(&png).unwrap();
        assert_eq!(size, (8, 4));
        let image = decode_png(&png).unwrap();
        assert_eq!((image.width, image.height), (8, 4));
        assert_eq!(image.pixels.len(), 8 * 4 * 4);
        assert_eq!(&image.pixels[..4], &[10, 20, 30, 255]);
    }

    #[test]
    fn inflate_handles_real_world_png_streams() {
        // 随包的 64×64 皮肤是真实工具压出来的（dynamic Huffman + 行过滤器）
        let image = decode_png(crate::skins::BUILTIN_STEVE).unwrap();
        assert_eq!((image.width, image.height), (64, 64));
        assert_eq!(image.pixels.len(), 64 * 64 * 4);
        // 皮肤不是全透明：一定存在不透明像素
        assert!(image.pixels.chunks(4).any(|pixel| pixel[3] > 0));
    }

    #[test]
    fn legacy_skin_is_padded_to_modern_layout() {
        let legacy = solid_png(64, 32, [200, 100, 50, 255]);
        let padded = legacy_to_modern(&legacy).unwrap();
        assert_eq!(png_size(&padded), Some((64, 64)));
        let image = decode_png(&padded).unwrap();
        // 上半部分原样
        assert_eq!(&image.pixels[..4], &[200, 100, 50, 255]);
        // 右腿/右臂区域被左侧内容补齐
        let right_leg = ((48 * 64 + 20) * 4) as usize;
        assert_eq!(&image.pixels[right_leg..right_leg + 4], &[200, 100, 50, 255]);
        let right_arm = ((48 * 64 + 40) * 4) as usize;
        assert_eq!(&image.pixels[right_arm..right_arm + 4], &[200, 100, 50, 255]);
        // 本来没内容的位置保持透明
        let empty = ((40 * 64 + 5) * 4) as usize;
        assert_eq!(image.pixels[empty + 3], 0);
    }

    #[test]
    fn rejects_non_png_and_unsupported_shapes() {
        assert!(decode_png(b"not a png").is_err());
        assert!(png_size(b"junk").is_none());
        assert!(legacy_to_modern(&solid_png(64, 64, [1, 2, 3, 4])).is_err());
    }

    #[test]
    fn stored_deflate_round_trips_large_payloads() {
        let data: Vec<u8> = (0..200_000u32).map(|index| (index % 251) as u8).collect();
        let packed = deflate_stored(&data);
        assert!(packed.len() > data.len());
        assert_eq!(inflate_zlib(&packed).unwrap(), data);
    }
}
