//! 小工具：时间戳格式化（不引入 chrono）与文件名清洗。
//!
//! 日志文件名、实例 createdAt、任务 createdAt 都用这里的纯函数，
//! 这样测试可以直接断言固定时间点的输出。

/// 当前 Unix 秒。
pub fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 当前毫秒时间戳（测速、事件节流用）。
pub fn now_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// civil_from_days（Howard Hinnant 算法）：Unix 天数 → (年, 月, 日)。
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { y + 1 } else { y }, month, day)
}

fn split(secs: u64) -> ((i64, u32, u32), u64, u64, u64) {
    let secs = secs as i64;
    let days = secs.div_euclid(86_400);
    let rest = secs.rem_euclid(86_400) as u64;
    (
        civil_from_days(days),
        rest / 3600,
        (rest % 3600) / 60,
        rest % 60,
    )
}

/// 2025-02-01T00:00:00Z（前端 createdAt / lastPlayedAt 的形态）。
pub fn iso8601(secs: u64) -> String {
    let ((y, m, d), hh, mm, ss) = split(secs);
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

/// 2025-02-01（日志文件名 launcher-2025-02-01.log）。
pub fn date_stamp(secs: u64) -> String {
    let ((y, m, d), _, _, _) = split(secs);
    format!("{y:04}-{m:02}-{d:02}")
}

/// 20250201-120000（游戏日志文件名 game-<实例>-20250201-120000.log）。
pub fn compact_stamp(secs: u64) -> String {
    let ((y, m, d), hh, mm, ss) = split(secs);
    format!("{y:04}{m:02}{d:02}-{hh:02}{mm:02}{ss:02}")
}

/// 把任意字符串压成安全的文件名片段（实例名可能带空格或中文）。
pub fn sanitize_file_part(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        "instance".to_string()
    } else {
        trimmed.chars().take(60).collect()
    }
}

const MD5_K: [u32; 64] = [
    0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613, 0xfd469501,
    0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193, 0xa679438e, 0x49b40821,
    0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d, 0x02441453, 0xd8a1e681, 0xe7d3fbc8,
    0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed, 0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a,
    0xfffa3942, 0x8771f681, 0x6d9d6122, 0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70,
    0x289b7ec6, 0xeaa127fa, 0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665,
    0xf4292244, 0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
    0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb, 0xeb86d391,
];
const MD5_S: [u32; 64] = [
    7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9,
    14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15,
    21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
];

/// MD5（离线 UUID 与旧实现一致，没有引额外依赖）。
pub fn md5(bytes: &[u8]) -> [u8; 16] {
    let mut message = bytes.to_vec();
    let bit_len = (bytes.len() as u64).wrapping_mul(8);
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_len.to_le_bytes());

    let mut a0: u32 = 0x6745_2301;
    let mut b0: u32 = 0xefcd_ab89;
    let mut c0: u32 = 0x98ba_dcfe;
    let mut d0: u32 = 0x1032_5476;

    for chunk in message.chunks(64) {
        let mut words = [0u32; 16];
        for (index, slot) in words.iter_mut().enumerate() {
            *slot = u32::from_le_bytes([
                chunk[index * 4],
                chunk[index * 4 + 1],
                chunk[index * 4 + 2],
                chunk[index * 4 + 3],
            ]);
        }
        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        for index in 0..64 {
            let (f, g) = match index / 16 {
                0 => ((b & c) | (!b & d), index),
                1 => ((d & b) | (!d & c), (5 * index + 1) % 16),
                2 => (b ^ c ^ d, (3 * index + 5) % 16),
                _ => (c ^ (b | !d), (7 * index) % 16),
            };
            let temp = f
                .wrapping_add(a)
                .wrapping_add(MD5_K[index])
                .wrapping_add(words[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(temp.rotate_left(MD5_S[index]));
        }
        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }

    let mut out = [0u8; 16];
    out[0..4].copy_from_slice(&a0.to_le_bytes());
    out[4..8].copy_from_slice(&b0.to_le_bytes());
    out[8..12].copy_from_slice(&c0.to_le_bytes());
    out[12..16].copy_from_slice(&d0.to_le_bytes());
    out
}

/// SHA-256（更新包校验用，不引额外依赖）。
pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut message = bytes.to_vec();
    let bit_len = (bytes.len() as u64).wrapping_mul(8);
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in message.chunks(64) {
        let mut w = [0u32; 64];
        for index in 0..16 {
            w[index] = u32::from_be_bytes([
                chunk[index * 4],
                chunk[index * 4 + 1],
                chunk[index * 4 + 2],
                chunk[index * 4 + 3],
            ]);
        }
        for index in 16..64 {
            let s0 = w[index - 15].rotate_right(7)
                ^ w[index - 15].rotate_right(18)
                ^ (w[index - 15] >> 3);
            let s1 = w[index - 2].rotate_right(17)
                ^ w[index - 2].rotate_right(19)
                ^ (w[index - 2] >> 10);
            w[index] = w[index - 16]
                .wrapping_add(s0)
                .wrapping_add(w[index - 7])
                .wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
        for index in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[index])
                .wrapping_add(w[index]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }

    let mut out = [0u8; 32];
    for (index, word) in h.iter().enumerate() {
        out[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

/// 文件 SHA-256（十六进制小写）。
/// 内存里字节的 sha256（导入去重用）。
pub fn sha256_file_of_bytes(bytes: &[u8]) -> String {
    hex(&sha256(bytes))
}

pub fn sha256_file(path: &std::path::Path) -> std::io::Result<String> {
    let bytes = std::fs::read(path)?;
    Ok(hex(&sha256(&bytes)))
}

/// CREATE_NO_WINDOW：启动控制台程序但不弹黑窗（Windows）。
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 让 std::process::Command 不弹控制台窗口（非 Windows 上是空操作）。
pub fn hidden(cmd: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    {
        let _ = cmd;
    }
}

/// tokio 版本的同名助手（java -version / reg query 这类异步调用用）。
pub fn hidden_async(cmd: &mut tokio::process::Command) {
    #[cfg(windows)]
    {
        // tokio::process::Command 自带同签名 creation_flags（内部转发到 std 的实现）
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    {
        let _ = cmd;
    }
}

/// SHA-1 十六进制（稳定 id、缓存键用）。
pub fn sha1_hex(bytes: &[u8]) -> String {
    use sha1::{Digest, Sha1};
    let mut hasher = Sha1::new();
    hasher.update(bytes);
    hex(&hasher.finalize())
}

/// 字节数组 → 小写十六进制。
pub fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// 宽松 base64 解码（皮肤上传与 Mojang textures 字段用；不认 URL-safe 变体的补位差异）。
pub fn base64_decode(input: &str) -> Option<Vec<u8>> {
    fn value_of(byte: u8) -> Option<u8> {
        match byte {
            b'A'..=b'Z' => Some(byte - b'A'),
            b'a'..=b'z' => Some(byte - b'a' + 26),
            b'0'..=b'9' => Some(byte - b'0' + 52),
            b'+' | b'-' => Some(62),
            b'/' | b'_' => Some(63),
            _ => None,
        }
    }
    let cleaned: Vec<u8> = input
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace() && *byte != b'=')
        .collect();
    if cleaned.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(cleaned.len() * 3 / 4);
    for chunk in cleaned.chunks(4) {
        let mut buffer = 0u32;
        for (index, byte) in chunk.iter().enumerate() {
            buffer |= u32::from(value_of(*byte)?) << (18 - index * 6);
        }
        let bytes = buffer.to_be_bytes();
        out.push(bytes[1]);
        if chunk.len() > 2 {
            out.push(bytes[2]);
        }
        if chunk.len() > 3 {
            out.push(bytes[3]);
        }
    }
    Some(out)
}

/// base64 编码（皮肤读取与测试用）。
pub fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let mut buffer = 0u32;
        for (index, byte) in chunk.iter().enumerate() {
            buffer |= u32::from(*byte) << (16 - index * 8);
        }
        for index in 0..4 {
            if index <= chunk.len() {
                let shift = 18 - index * 6;
                out.push(TABLE[((buffer >> shift) & 0x3f) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// API Key 打码：sk-abcd...wxyz → sk-***（日志与错误信息里一律用它）。
pub fn mask_secret(secret: &str) -> String {
    let trimmed = secret.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if let Some(index) = trimmed.find('-') {
        let prefix = &trimmed[..index];
        if !prefix.is_empty() && prefix.len() <= 8 && prefix.chars().all(|c| c.is_ascii_alphanumeric())
        {
            return format!("{prefix}-***");
        }
    }
    "***".to_string()
}

/// 把文本里出现的密钥字面量换成打码形式（错误信息回传前过一遍）。
pub fn redact_secret(text: &str, secret: &str) -> String {
    let secret = secret.trim();
    if secret.is_empty() || secret.len() < 4 {
        return text.to_string();
    }
    text.replace(secret, &mask_secret(secret))
}

/// 进程内唯一后缀（任务 id 用，避免同一毫秒内撞车）。
pub fn unique_suffix() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{}-{}", now_millis(), seq)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_and_known_instants_format_correctly() {
        assert_eq!(iso8601(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso8601(1_738_368_000), "2025-02-01T00:00:00Z");
        // 闰日 2024-02-29T12:34:56Z
        assert_eq!(iso8601(1_709_210_096), "2024-02-29T12:34:56Z");
    }

    #[test]
    fn date_and_compact_stamps_share_the_same_day() {
        let secs = 1_738_368_000;
        assert_eq!(date_stamp(secs), "2025-02-01");
        assert_eq!(compact_stamp(secs), "20250201-000000");
    }

    #[test]
    fn sanitize_replaces_forbidden_and_keeps_chinese() {
        assert_eq!(sanitize_file_part("26.3 Fabric"), "26.3 Fabric");
        assert_eq!(sanitize_file_part("a/b:c"), "a_b_c");
        assert_eq!(sanitize_file_part("   "), "instance");
        assert_eq!(sanitize_file_part("咕咕 实例"), "咕咕 实例");
    }

    #[test]
    fn mask_secret_keeps_only_prefix() {
        assert_eq!(mask_secret("sk-abcd1234wxyz"), "sk-***");
        assert_eq!(mask_secret("  sk-123  "), "sk-***");
        assert_eq!(mask_secret("plainkey"), "***");
        assert_eq!(mask_secret(""), "");
        assert_eq!(mask_secret("   "), "");
        // 子串替换：错误文本里不能再出现原始密钥
        assert_eq!(
            redact_secret("请求失败：key=sk-abcd1234wxyz 无效", "sk-abcd1234wxyz"),
            "请求失败：key=sk-*** 无效"
        );
        assert_eq!(redact_secret("没有密钥", ""), "没有密钥");
    }

    #[test]
    fn md5_matches_known_vectors() {
        assert_eq!(hex(&md5(b"")), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(hex(&md5(b"abc")), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(
            hex(&md5(b"The quick brown fox jumps over the lazy dog")),
            "9e107d9d372bb6826bd81d3542a419d6"
        );
    }

    #[test]
    fn sha256_matches_known_vectors() {
        assert_eq!(
            hex(&sha256(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hex(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let dir = std::env::temp_dir().join(format!("gugu-sha256-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.bin");
        std::fs::write(&file, b"abc").unwrap();
        assert_eq!(
            sha256_file(&file).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn base64_roundtrips_bytes() {
        for sample in [&b""[..], b"f", b"fo", b"foo", b"foob", b"hello world!"] {
            let encoded = base64_encode(sample);
            assert_eq!(base64_decode(&encoded).unwrap(), sample.to_vec());
        }
        assert_eq!(base64_decode("aGVsbG8=").unwrap(), b"hello".to_vec());
        assert!(base64_decode("!!!!").is_none());
    }

    #[test]
    fn unique_suffix_is_distinct() {
        assert_ne!(unique_suffix(), unique_suffix());
    }
}
