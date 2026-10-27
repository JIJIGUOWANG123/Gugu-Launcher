//! 种子地图：用 cubiomes（C，MIT，见 third_party/cubiomes）算指定种子的
//! 生物群系与结构，输出给前端画地图。
//!
//! 三条硬约定：
//! 1. Generator 不能跨线程共享，所以每个命令都在 spawn_blocking 里现建一个
//!    句柄（seedmap_gen_new），用完立刻释放；
//! 2. Rust 只碰 C 的 int/指针，结构体布局全留在 C 里（seedmap_glue.c）；
//! 3. 坐标：像素 (i, j) 覆盖方块 [origin + i*scale, origin + (i+1)*scale)，
//!    原点先按 scale 向下对齐——cubiomes 的 Range 坐标单位就是 scale 个方块。

use std::ffi::{c_int, c_void, CStr};

use serde::{Deserialize, Serialize};

use crate::error::{AppError, Result};

/// 单边像素上限（512×512 已够铺满一大片视野）
pub const MAX_TILES: u32 = 512;
/// 允许的缩放范围（1 = 1px/方块）。前端会按视口算出连续值，这里不做离散限制
pub const MIN_SCALE: i32 = 1;
pub const MAX_SCALE: i32 = 64;
/// 1.17 及更早的分层生成只有这四档采样网格
pub const LAYER_SCALES: [i32; 4] = [1, 4, 16, 64];
/// 1.18 起是噪声生成，cubiomes 内部按 4 方块为步长采样
const MC_1_17: i32 = 21;
/// 一次最多返回的结构数量
pub const MAX_STRUCTURES: usize = 128;
/// 一次最多枚举的结构尝试点（防止结构区域特别小的类型被大半径拖死）
pub const MAX_STRUCTURE_SCANS: i64 = 65536;
/// 结构搜索半径上限（方块）
pub const MAX_RADIUS: i32 = 8192;

// ------------------------------------------------------------------ cubiomes FFI

// cubiomes 的 DIM 常量（biomes.h: DIM_NETHER=-1, DIM_OVERWORLD=0, DIM_END=+1）
const DIM_OVERWORLD: i32 = 0;
const DIM_NETHER: i32 = -1;
const DIM_END: i32 = 1;

/// 群系 id 的编号空间（cubiomes 用 0..256，127 = the_void，>=128 是变种）
const BIOME_SLOTS: usize = 256;

// 只在单测里用：拿 C 头文件里的真实枚举值交叉校验上面那张版本表
#[cfg(test)]
extern "C" {
    fn seedmap_mc_enum(name: *const std::os::raw::c_char) -> c_int;
}

extern "C" {
    fn seedmap_biome_name(mc: c_int, id: c_int) -> *const std::os::raw::c_char;
    fn seedmap_biome_colors(rgb: *mut u8);
    fn seedmap_gen_new(mc: c_int, dim: c_int, seed: u64) -> *mut c_void;
    fn seedmap_gen_free(gen: *mut c_void);
    fn seedmap_biomes(
        gen: *mut c_void,
        scale: c_int,
        x: c_int,
        z: c_int,
        w: c_int,
        h: c_int,
        y: c_int,
        out: *mut c_int,
    ) -> c_int;
    fn seedmap_biome_at(gen: *mut c_void, x: c_int, z: c_int) -> c_int;
    fn seedmap_spawn(mc: c_int, seed: u64, px: *mut c_int, pz: *mut c_int) -> c_int;
    fn seedmap_struct_config(
        stype: c_int,
        mc: c_int,
        region_size: *mut c_int,
        chunk_range: *mut c_int,
        dim: *mut c_int,
    ) -> c_int;
    fn seedmap_struct_pos(
        stype: c_int,
        mc: c_int,
        seed: u64,
        reg_x: c_int,
        reg_z: c_int,
        px: *mut c_int,
        pz: *mut c_int,
    ) -> c_int;
    fn seedmap_struct_viable(gen: *mut c_void, stype: c_int, bx: c_int, bz: c_int) -> c_int;
    fn seedmap_strongholds(
        mc: c_int,
        seed: u64,
        cx: c_int,
        cz: c_int,
        radius: c_int,
        cap: c_int,
        out: *mut c_int,
    ) -> c_int;
}

/// 生成器句柄：拥有 C 侧 calloc 出来的 Generator，Drop 时释放。
struct Gen(*mut c_void);

impl Gen {
    fn new(mc: i32, dim: i32, seed: u64) -> Result<Self> {
        let ptr = unsafe { seedmap_gen_new(mc, dim, seed) };
        if ptr.is_null() {
            return Err(AppError::Other("种子地图生成器初始化失败".to_string()));
        }
        Ok(Self(ptr))
    }
}

impl Drop for Gen {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { seedmap_gen_free(self.0) };
        }
    }
}

// ------------------------------------------------------------------ 版本表

/// 展示用版本号 → cubiomes 枚举名 → 枚举值。
///
/// 枚举值照抄 third_party/cubiomes/biomes.h 的 MCVersion 顺序；
/// 单测 version_table_matches_c_enum 会用 C 侧的 seedmap_mc_enum 交叉校验，
/// 头文件一变就会红。
const VERSIONS: &[(&str, &str, i32)] = &[
    ("1.7.10", "MC_1_7", 10),
    ("1.8.9", "MC_1_8", 11),
    ("1.9.4", "MC_1_9", 12),
    ("1.10.2", "MC_1_10", 13),
    ("1.11.2", "MC_1_11", 14),
    ("1.12.2", "MC_1_12", 15),
    ("1.13.2", "MC_1_13", 16),
    ("1.14.4", "MC_1_14", 17),
    ("1.15.2", "MC_1_15", 18),
    ("1.16.1", "MC_1_16_1", 19),
    ("1.16.5", "MC_1_16", 20),
    ("1.17.1", "MC_1_17", 21),
    ("1.18.2", "MC_1_18", 22),
    ("1.19.2", "MC_1_19_2", 23),
    ("1.19.4", "MC_1_19", 24),
    ("1.20.6", "MC_1_20", 25),
    ("1.21.1", "MC_1_21_1", 26),
    ("1.21.3", "MC_1_21_3", 27),
    ("1.21.4", "MC_1_21_4", 28),
    ("1.21.5", "MC_1_21_5", 29),
    ("1.21.6", "MC_1_21_6", 30),
    ("1.21.9", "MC_1_21_9", 31),
    ("1.21.11", "MC_1_21_11", 32),
    // 1.21.11 之后 Minecraft 改用年份版本号（cubiomes README：abandoned camp 26.3+）
    ("26.1", "MC_26_1", 33),
    ("26.2", "MC_26_2", 34),
    ("26.3", "MC_26_3", 35),
];

/// seedmap_versions 的一条。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VersionInfo {
    /// 前端回传给 seedmap_* 的版本号
    pub mc: &'static str,
    /// 下拉框里显示的文字
    pub label: &'static str,
}

/// 支持的版本列表（前端只显示这些）。**从新到旧**，首项就是最新版，
/// 前端拿 list[0] 当默认值即可。
pub fn versions() -> Vec<VersionInfo> {
    VERSIONS
        .iter()
        .rev()
        .map(|(mc, _, _)| VersionInfo { mc, label: mc })
        .collect()
}

/// 版本字符串 → cubiomes 枚举值。
///
/// 精确命中就用精确的那一档；否则按同一 major.minor 的候选里挑：
/// 给了补丁号就取「不超过它的最新一档」（1.21.7 → 1.21.6），太小/没给就取
/// 该小版本最高的一档（1.21 → 1.21.11，与 cubiomes 自己的 MC_1_21 别名一致）。
pub fn resolve_version(mc: &str) -> Result<i32> {
    let text = mc.trim();
    let text = text.strip_prefix('v').unwrap_or(text).trim();
    if text.is_empty() {
        return Err(unsupported_version(mc));
    }
    if let Some((_, _, value)) = VERSIONS.iter().find(|(name, _, _)| *name == text) {
        return Ok(*value);
    }
    let Some((major, minor, patch)) = split_version(text) else {
        return Err(unsupported_version(mc));
    };
    let mut candidates: Vec<(u32, i32)> = VERSIONS
        .iter()
        .filter_map(|(name, _, value)| match split_version(name) {
            Some((ma, mi, Some(pa))) if ma == major && mi == minor => Some((pa, *value)),
            _ => None,
        })
        .collect();
    if candidates.is_empty() {
        return Err(unsupported_version(mc));
    }
    candidates.sort_by_key(|(pa, _)| *pa);
    let chosen = match patch {
        Some(wanted) => candidates
            .iter()
            .rev()
            .find(|(pa, _)| *pa <= wanted)
            .or_else(|| candidates.first())
            .map(|(_, value)| *value),
        None => candidates.last().map(|(_, value)| *value),
    };
    chosen.ok_or_else(|| unsupported_version(mc))
}

/// "1.21.4" -> (1, 21, Some(4))；"26.3" -> (26, 3, None)；格式不对返回 None。
fn split_version(text: &str) -> Option<(u32, u32, Option<u32>)> {
    let mut parts = text.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = match parts.next() {
        Some(patch) => Some(patch.parse().ok()?),
        None => None,
    };
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

fn unsupported_version(mc: &str) -> AppError {
    AppError::Other(format!("该版本暂不支持种子地图（{mc}）"))
}

// ------------------------------------------------------------------ 参数校验

/// 前端维度 → cubiomes 的 DIM 常量。0=主世界，1=下界，2=末地。
pub fn dimension_to_cubiomes(dimension: i32) -> Result<i32> {
    match dimension {
        0 => Ok(DIM_OVERWORLD),
        1 => Ok(DIM_NETHER),
        2 => Ok(DIM_END),
        other => Err(AppError::Invalid(format!(
            "维度只支持 0=主世界 / 1=下界 / 2=末地（收到 {other}）"
        ))),
    }
}

/// 缩放比例范围校验：1..=64 的任意整数都接受（前端按视口算出来的连续值）。
pub fn check_scale(scale: i32) -> Result<i32> {
    if (MIN_SCALE..=MAX_SCALE).contains(&scale) {
        Ok(scale)
    } else {
        Err(AppError::Invalid(format!(
            "scale 需在 {MIN_SCALE} 到 {MAX_SCALE} 之间（收到 {scale}）"
        )))
    }
}

/// 把请求的 scale 落到 cubiomes 真正支持的采样网格上，并返回实际使用的 scale。
///
/// 原因：1.17- 的分层生成只有 1/4/16/64 四档；1.18+ 的噪声生成虽然不限档，
/// 但内部是按 4 方块为步长采样的（genBiomeNoise3D 用 scale/4）。
/// 所以一律向上取整到最近一档——细节只多不少，且能保证「像素 = 返回的 scale 个方块」。
pub fn normalize_scale(scale: i32, mc: i32) -> Result<i32> {
    let scale = check_scale(scale)?;
    if mc <= MC_1_17 {
        return Ok(*LAYER_SCALES
            .iter()
            .find(|step| **step >= scale)
            .unwrap_or(&MAX_SCALE));
    }
    if scale <= 1 {
        return Ok(1);
    }
    // scale >= 1，直接向上取整到 4 的倍数（有符号数的 div_ceil 还是 unstable，手写）
    Ok((scale + 3) / 4 * 4)
}

/// 像素尺寸校验（1..=512）。
pub fn check_tiles(tiles_x: u32, tiles_y: u32) -> Result<(u32, u32)> {
    if tiles_x == 0 || tiles_y == 0 {
        return Err(AppError::Invalid("地图宽高不能为 0".to_string()));
    }
    if tiles_x > MAX_TILES || tiles_y > MAX_TILES {
        return Err(AppError::Invalid(format!(
            "地图单边最多 {MAX_TILES} 像素（收到 {tiles_x}×{tiles_y}）"
        )));
    }
    Ok((tiles_x, tiles_y))
}

/// 地表采样高度（方块）：60 左右既不是洞穴也不会削掉山峰。
/// cubiomes 的 Range.y 在 scale>1 时按 1:4 解释，所以 60 方块 = 15。
pub const SURFACE_Y_BLOCK: i32 = 60;

/// 某个 scale 下传给 cubiomes 的 y（scale 1 用方块，其余用 1:4 单位）。
pub fn surface_y(scale: i32) -> i32 {
    if scale <= 1 {
        SURFACE_Y_BLOCK
    } else {
        SURFACE_Y_BLOCK / 4
    }
}

/// 把原点按 scale 向下对齐：否则整张图会整体偏半个格子。
fn snap(value: i32, scale: i32) -> i32 {
    value.div_euclid(scale) * scale
}

/// 左上角像素对应的方块坐标（已对齐到 scale 网格）。
pub fn snap_origin(x0: i32, z0: i32, scale: i32) -> (i32, i32) {
    (snap(x0, scale), snap(z0, scale))
}

// ------------------------------------------------------------------ 种子解析

/// 前端传来的种子：数字或字符串都收（Minecraft 里文本种子是按字符串哈希算的）。
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum SeedArg {
    Int(i64),
    Text(String),
}

impl SeedArg {
    pub fn to_seed(&self) -> Result<u64> {
        match self {
            SeedArg::Int(value) => Ok(*value as u64),
            SeedArg::Text(text) => parse_seed(text),
        }
    }
}

/// 字符串种子 → u64。
///
/// - 纯数字（可带负号）：按 i64 读，负种子用二进制补码当 u64；
/// - 超出 i64 但不超过 u64 的无符号数：按 u64 读；
/// - 其它文本：和原版一样用 Java String.hashCode()（UTF-16 码元，31 进制）。
pub fn parse_seed(text: &str) -> Result<u64> {
    let text = text.trim();
    if text.is_empty() {
        return Err(AppError::Invalid("种子不能为空".to_string()));
    }
    let digits = text.strip_prefix('-').unwrap_or(text);
    let numeric = !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit());
    if numeric {
        if let Ok(value) = text.parse::<i64>() {
            return Ok(value as u64);
        }
        if let Ok(value) = text.parse::<u64>() {
            return Ok(value);
        }
        return Err(AppError::Invalid(format!(
            "种子超出 64 位整数范围（{text}）"
        )));
    }
    Ok(java_string_hash(text) as u64)
}

/// Java String.hashCode()（int 溢出即回绕）。
fn java_string_hash(text: &str) -> i32 {
    let mut hash = 0i32;
    for unit in text.encode_utf16() {
        hash = hash.wrapping_mul(31).wrapping_add(unit as i32);
    }
    hash
}

// ------------------------------------------------------------------ 群系名

/// 群系 id → 中文名（1.13+ 的 id 编号，跨版本稳定）。
const BIOME_NAMES_ZH: &[(i32, &str)] = &[
    (0, "海洋"),
    (1, "平原"),
    (2, "沙漠"),
    (3, "山地"),
    (4, "森林"),
    (5, "针叶林"),
    (6, "沼泽"),
    (7, "河流"),
    (8, "下界荒地"),
    (9, "末地"),
    (10, "冻洋"),
    (11, "冰冻河流"),
    (12, "雪原"),
    (13, "雪山"),
    (14, "蘑菇岛"),
    (15, "蘑菇岛岸"),
    (16, "沙滩"),
    (17, "沙漠丘陵"),
    (18, "森林丘陵"),
    (19, "针叶林丘陵"),
    (20, "山地边缘"),
    (21, "丛林"),
    (22, "丛林丘陵"),
    (23, "丛林边缘"),
    (24, "深海"),
    (25, "石岸"),
    (26, "积雪沙滩"),
    (27, "桦木森林"),
    (28, "桦木森林丘陵"),
    (29, "黑森林"),
    (30, "积雪针叶林"),
    (31, "积雪针叶林丘陵"),
    (32, "巨型针叶林"),
    (33, "巨型针叶林丘陵"),
    (34, "疏林山地"),
    (35, "热带草原"),
    (36, "热带高原"),
    (37, "恶地"),
    (38, "森林恶地"),
    (39, "恶地高原"),
    (40, "末地小岛"),
    (41, "末地内陆"),
    (42, "末地高地"),
    (43, "末地荒地"),
    (44, "暖水海洋"),
    (45, "温水海洋"),
    (46, "冷水海洋"),
    (47, "暖水深海"),
    (48, "温水深海"),
    (49, "冷水深海"),
    (50, "冰冻深海"),
    (127, "虚空"),
    (129, "向日葵平原"),
    (130, "沙漠湖泊"),
    (131, "沙砾山地"),
    (132, "繁花森林"),
    (133, "针叶林山地"),
    (134, "沼泽丘陵"),
    (140, "冰刺之地"),
    (149, "丛林变种"),
    (151, "丛林边缘变种"),
    (155, "高大桦木森林"),
    (156, "高大桦木丘陵"),
    (157, "黑森林丘陵"),
    (158, "积雪针叶林山地"),
    (160, "巨型云杉针叶林"),
    (161, "巨型云杉针叶林丘陵"),
    (162, "被修改的沙砾山地"),
    (163, "破碎热带草原"),
    (164, "破碎热带高原"),
    (165, "侵蚀恶地"),
    (166, "被修改的森林恶地"),
    (167, "被修改的恶地高原"),
    (168, "竹林"),
    (169, "竹林丘陵"),
    (170, "灵魂沙峡谷"),
    (171, "绯红森林"),
    (172, "诡异森林"),
    (173, "玄武岩三角洲"),
    (174, "溶洞"),
    (175, "繁茂洞穴"),
    (177, "草甸"),
    (178, "雪林"),
    (179, "积雪山坡"),
    (180, "尖峭山峰"),
    (181, "冰封山峰"),
    (182, "裸岩山峰"),
    (183, "深暗之域"),
    (184, "红树林沼泽"),
    (185, "樱花树林"),
    (186, "苍白之园"),
];

/// 中文名（没收录的返回 None）。
pub fn biome_name_zh(id: i32) -> Option<&'static str> {
    BIOME_NAMES_ZH
        .iter()
        .find(|(key, _)| *key == id)
        .map(|(_, name)| *name)
}

/// cubiomes 的英文资源名（plains 之类），没有就返回 None。
pub fn biome_name_en(mc: i32, id: i32) -> Option<String> {
    let ptr = unsafe { seedmap_biome_name(mc, id) };
    if ptr.is_null() {
        return None;
    }
    let name = unsafe { CStr::from_ptr(ptr) }.to_string_lossy().into_owned();
    if name.is_empty() || name == "unknown" {
        None
    } else {
        Some(name)
    }
}

/// 中文优先，没收录回落到 cubiomes 的英文名，再不行给 未知群系(#id)。
pub fn biome_display_name(mc: i32, id: i32) -> String {
    if let Some(name) = biome_name_zh(id) {
        return name.to_string();
    }
    biome_name_en(mc, id).unwrap_or_else(|| format!("未知群系(#{id})"))
}

/// cubiomes 自带的地图配色（256 × RGB）。
fn biome_colors() -> Vec<u8> {
    let mut rgb = vec![0u8; BIOME_SLOTS * 3];
    unsafe { seedmap_biome_colors(rgb.as_mut_ptr()) };
    rgb
}

// ------------------------------------------------------------------ 位图

/// 群系 id 数组 → RGBA8（每像素 4 字节，alpha 固定 255）。
pub fn rgba_from_biomes(biomes: &[i32], colors: &[u8]) -> Vec<u8> {
    let mut rgba = vec![0u8; biomes.len() * 4];
    for (index, id) in biomes.iter().enumerate() {
        let base = index * 4;
        let valid = *id >= 0 && (*id as usize) < BIOME_SLOTS;
        if !valid {
            // 无效 id（-1 / 越界）：给一块中性灰，别让前端拿到透明像素
            rgba[base] = 128;
            rgba[base + 1] = 128;
            rgba[base + 2] = 128;
            rgba[base + 3] = 255;
            continue;
        }
        let slot = *id as usize;
        rgba[base] = colors[slot * 3];
        rgba[base + 1] = colors[slot * 3 + 1];
        rgba[base + 2] = colors[slot * 3 + 2];
        rgba[base + 3] = 255;
    }
    rgba
}

/// 标准 base64（带 = 填充）。自己写是为了不再多引一个 crate。
pub fn base64_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[(triple >> 18) as usize & 0x3F] as char);
        out.push(ALPHABET[(triple >> 12) as usize & 0x3F] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(triple >> 6) as usize & 0x3F] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[triple as usize & 0x3F] as char
        } else {
            '='
        });
    }
    out
}

// ------------------------------------------------------------------ 返回结构

/// seedmap_biomes 的返回。
///
/// rgba 是 PNG（RGBA8）的 base64（encoding = "png+base64"），前端拼成
/// data:image/png;base64,<rgba> 就能直接画；biomes 是逐像素群系 id
/// （索引 [z * width + x]），悬停查询用，不需要可以忽略。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BiomeMapView {
    pub width: u32,
    pub height: u32,
    pub scale: i32,
    pub origin_x: i32,
    pub origin_z: i32,
    pub rgba: String,
    pub encoding: &'static str,
    pub biomes: Vec<i32>,
}

/// seedmap_biome_at 的返回。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BiomeAtView {
    pub biome_id: i32,
    pub biome_name: String,
    pub x: i32,
    pub z: i32,
}

/// seedmap_spawn 的返回。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpawnView {
    pub x: i32,
    pub z: i32,
}

/// seedmap_structures 的一条。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StructureHit {
    pub kind: &'static str,
    pub x: i32,
    pub z: i32,
}

// ------------------------------------------------------------------ 结构表

/// 前端结构名 → cubiomes 的 StructureType 枚举值（finders.h）。
///
/// temple 是一组：沙漠神殿 / 丛林神庙 / 沼泽小屋 / 雪屋；
/// ruined_portal 含下界那一半；stronghold 不走区域枚举，单独迭代。
const STRUCTURE_GROUPS: &[(&str, &[i32])] = &[
    ("village", &[5]),
    ("temple", &[1, 2, 3, 4]),
    ("monument", &[8]),
    ("mansion", &[9]),
    ("stronghold", &[]),
    ("fortress", &[18]),
    ("bastion", &[19]),
    ("outpost", &[10]),
    ("ruined_portal", &[11, 12]),
    ("shipwreck", &[7]),
    ("ocean_ruin", &[6]),
    ("treasure", &[14]),
];

/// 结构枚举值 → 回给前端的 kind 文本。
const STRUCTURE_KIND_NAMES: &[(i32, &str)] = &[
    (1, "desert_pyramid"),
    (2, "jungle_temple"),
    (3, "swamp_hut"),
    (4, "igloo"),
    (5, "village"),
    (6, "ocean_ruin"),
    (7, "shipwreck"),
    (8, "monument"),
    (9, "mansion"),
    (10, "outpost"),
    (11, "ruined_portal"),
    (12, "ruined_portal_nether"),
    (14, "treasure"),
    (18, "fortress"),
    (19, "bastion"),
];

/// 结构名 → cubiomes 枚举值列表；不认识的返回错误。
pub fn structure_group(name: &str) -> Result<&'static [i32]> {
    let name = name.trim().to_ascii_lowercase();
    STRUCTURE_GROUPS
        .iter()
        .find(|(key, _)| *key == name)
        .map(|(_, ids)| *ids)
        .ok_or_else(|| {
            let known: Vec<&str> = STRUCTURE_GROUPS.iter().map(|(key, _)| *key).collect();
            AppError::Invalid(format!(
                "不支持的结构类型：{name}（支持 {}）",
                known.join("/")
            ))
        })
}

/// 结构枚举值 → 展示用 kind（temple 组要区分具体是哪一种）。
pub fn structure_kind_name(stype: i32) -> &'static str {
    STRUCTURE_KIND_NAMES
        .iter()
        .find(|(key, _)| *key == stype)
        .map(|(_, name)| *name)
        .unwrap_or("unknown")
}

// ------------------------------------------------------------------ 计算（阻塞）

/// seedmap_biomes 的入参（已换算成 cubiomes 的枚举/常量）。
#[derive(Debug, Clone, Copy)]
pub struct BiomeRequest {
    pub mc: i32,
    pub dim: i32,
    pub seed: u64,
    pub scale: i32,
    pub x0: i32,
    pub z0: i32,
    pub tiles_x: u32,
    pub tiles_y: u32,
}

/// 生成一张群系图。整块计算在调用线程内完成，命令层必须用 spawn_blocking。
pub fn biome_map_blocking(request: BiomeRequest) -> Result<BiomeMapView> {
    let (width, height) = check_tiles(request.tiles_x, request.tiles_y)?;
    let scale = normalize_scale(request.scale, request.mc)?;
    let (origin_x, origin_z) = snap_origin(request.x0, request.z0, scale);
    let gen = Gen::new(request.mc, request.dim, request.seed)?;

    let count = (width as usize) * (height as usize);
    let mut biomes = vec![0i32; count];
    let err = unsafe {
        seedmap_biomes(
            gen.0,
            scale,
            origin_x / scale,
            origin_z / scale,
            width as i32,
            height as i32,
            surface_y(scale),
            biomes.as_mut_ptr(),
        )
    };
    if err != 0 {
        return Err(AppError::Other(format!(
            "群系生成失败（cubiomes 返回 {err}），可能是不支持的维度/缩放组合"
        )));
    }

    let rgba = rgba_from_biomes(&biomes, &biome_colors());
    let png = crate::png::encode_png(width, height, &rgba)?;
    Ok(BiomeMapView {
        width,
        height,
        scale,
        origin_x,
        origin_z,
        rgba: base64_encode(&png),
        encoding: "png+base64",
        biomes,
    })
}

/// 单个坐标的群系。dim 是 cubiomes 的 DIM 常量。
pub fn biome_at_blocking(mc: i32, dim: i32, seed: u64, x: i32, z: i32) -> Result<BiomeAtView> {
    let gen = Gen::new(mc, dim, seed)?;
    let id = unsafe { seedmap_biome_at(gen.0, x, z) };
    Ok(BiomeAtView {
        biome_id: id,
        biome_name: biome_display_name(mc, id),
        x,
        z,
    })
}

/// 主世界出生点。
pub fn spawn_point_blocking(mc: i32, seed: u64) -> Result<SpawnView> {
    let (mut x, mut z) = (0i32, 0i32);
    let err = unsafe { seedmap_spawn(mc, seed, &mut x, &mut z) };
    if err != 0 {
        return Err(AppError::Other("出生点计算失败".to_string()));
    }
    Ok(SpawnView { x, z })
}

struct StructConfig {
    region_size: i32,
    dim: i32,
}

fn struct_config(stype: i32, mc: i32) -> Result<StructConfig> {
    let (mut region_size, mut chunk_range, mut dim) = (0i32, 0i32, 0i32);
    let ok = unsafe { seedmap_struct_config(stype, mc, &mut region_size, &mut chunk_range, &mut dim) };
    if ok == 0 || region_size <= 0 {
        return Err(AppError::Other(format!(
            "该版本没有结构类型 {stype}（或版本过旧）"
        )));
    }
    let _ = chunk_range;
    Ok(StructConfig { region_size, dim })
}

/// 结构位置：区域尝试点 + 群系校验，按距离排序后截断。
pub fn structures_blocking(
    mc: i32,
    seed: u64,
    structure: &str,
    x: i32,
    z: i32,
    radius: i32,
) -> Result<Vec<StructureHit>> {
    if radius <= 0 {
        return Err(AppError::Invalid("搜索半径必须大于 0".to_string()));
    }
    if radius > MAX_RADIUS {
        return Err(AppError::Invalid(format!(
            "搜索半径最多 {MAX_RADIUS} 方块（收到 {radius}）"
        )));
    }
    let name = structure.trim().to_ascii_lowercase();
    let ids = structure_group(&name)?;

    let mut hits: Vec<StructureHit> = Vec::new();
    if ids.is_empty() {
        // 要塞：cubiomes 用迭代器逐个算，从原点附近开始向外螺旋
        let cap = MAX_STRUCTURES as i32;
        let mut buffer = vec![0i32; cap as usize * 2];
        let found = unsafe { seedmap_strongholds(mc, seed, x, z, radius, cap, buffer.as_mut_ptr()) };
        for index in 0..found.max(0) as usize {
            hits.push(StructureHit {
                kind: "stronghold",
                x: buffer[index * 2],
                z: buffer[index * 2 + 1],
            });
        }
        return Ok(hits);
    }

    let mut scans = 0i64;
    for &stype in ids {
        let config = struct_config(stype, mc)?;
        // 区块 → 方块；区域网格就是 getStructurePos 的 (regX, regZ)
        let region_blocks = config.region_size as i64 * 16;
        let min_x = ((x as i64 - radius as i64).div_euclid(region_blocks)) as i32;
        let max_x = ((x as i64 + radius as i64).div_euclid(region_blocks)) as i32;
        let min_z = ((z as i64 - radius as i64).div_euclid(region_blocks)) as i32;
        let max_z = ((z as i64 + radius as i64).div_euclid(region_blocks)) as i32;
        scans += (max_x as i64 - min_x as i64 + 1) * (max_z as i64 - min_z as i64 + 1);
        if scans > MAX_STRUCTURE_SCANS {
            return Err(AppError::Invalid(format!(
                "搜索范围过大（{scans} 个结构区域），请缩小半径"
            )));
        }

        let gen = Gen::new(mc, config.dim, seed)?;
        for reg_x in min_x..=max_x {
            for reg_z in min_z..=max_z {
                let (mut px, mut pz) = (0i32, 0i32);
                let valid =
                    unsafe { seedmap_struct_pos(stype, mc, seed, reg_x, reg_z, &mut px, &mut pz) };
                if valid == 0 {
                    continue;
                }
                if (px - x).abs() > radius || (pz - z).abs() > radius {
                    continue;
                }
                // 群系校验：滤掉「落在海里 / 群系不支持」的候选点
                if unsafe { seedmap_struct_viable(gen.0, stype, px, pz) } == 0 {
                    continue;
                }
                hits.push(StructureHit {
                    kind: structure_kind_name(stype),
                    x: px,
                    z: pz,
                });
            }
        }
    }

    hits.sort_by_key(|hit| {
        let dx = (hit.x - x) as i64;
        let dz = (hit.z - z) as i64;
        dx * dx + dz * dz
    });
    hits.truncate(MAX_STRUCTURES);
    Ok(hits)
}

// ------------------------------------------------------------------ 异步包装

/// 命令层用的统一包装：计算全部丢给 spawn_blocking（Generator 不跨线程）。
async fn offload<T, F>(work: F) -> Result<T>
where
    F: FnOnce() -> Result<T> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|err| AppError::Other(format!("种子地图计算线程异常：{err}")))?
}

pub async fn biome_map(request: BiomeRequest) -> Result<BiomeMapView> {
    offload(move || biome_map_blocking(request)).await
}

pub async fn biome_at(mc: i32, dim: i32, seed: u64, x: i32, z: i32) -> Result<BiomeAtView> {
    offload(move || biome_at_blocking(mc, dim, seed, x, z)).await
}

pub async fn spawn_point(mc: i32, seed: u64) -> Result<SpawnView> {
    offload(move || spawn_point_blocking(mc, seed)).await
}

pub async fn structures(
    mc: i32,
    seed: u64,
    structure: String,
    x: i32,
    z: i32,
    radius: i32,
) -> Result<Vec<StructureHit>> {
    offload(move || structures_blocking(mc, seed, &structure, x, z, radius)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    const MC_1_21_4: i32 = 28;
    const SEED: u64 = 840_751_423_099_399;

    fn base64_decode(text: &str) -> Vec<u8> {
        let mut out = Vec::new();
        let mut buffer = 0u32;
        let mut bits = 0u32;
        for byte in text.bytes() {
            let value = match byte {
                b'A'..=b'Z' => byte - b'A',
                b'a'..=b'z' => byte - b'a' + 26,
                b'0'..=b'9' => byte - b'0' + 52,
                b'+' => 62,
                b'/' => 63,
                b'=' => break,
                _ => continue,
            } as u32;
            buffer = (buffer << 6) | value;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                out.push((buffer >> bits) as u8);
            }
        }
        out
    }

    #[test]
    fn version_table_matches_c_enum() {
        for (display, c_name, value) in VERSIONS {
            let name = std::ffi::CString::new(*c_name).unwrap();
            let from_c = unsafe { seedmap_mc_enum(name.as_ptr()) };
            assert_eq!(
                from_c, *value,
                "{display} / {c_name}: Rust 表 {value} 与 C 枚举 {from_c} 不一致"
            );
        }
    }

    #[test]
    fn version_lookup_accepts_exact_and_minor_forms() {
        assert_eq!(resolve_version("1.21.4").unwrap(), MC_1_21_4);
        assert_eq!(resolve_version(" 1.20.6 ").unwrap(), 25);
        // 只给小版本：取该小版本最高的一档（与 cubiomes 的 MC_1_21 别名一致）
        assert_eq!(resolve_version("1.21").unwrap(), 32);
        // 补丁号不在表里：取不超过它的最近一档
        assert_eq!(resolve_version("1.21.7").unwrap(), 30);
        assert_eq!(resolve_version("1.20.4").unwrap(), 25);
        assert_eq!(resolve_version("v1.18.2").unwrap(), 22);
        // 年份版本号
        assert_eq!(resolve_version("26.3").unwrap(), 35);
    }

    #[test]
    fn version_lookup_rejects_unsupported() {
        for bad in ["1.6.4", "1.22.0", "abc", "", "1.x"] {
            let message = resolve_version(bad).unwrap_err().to_string();
            assert!(
                message.contains("该版本暂不支持种子地图"),
                "{bad} -> {message}"
            );
        }
    }

    #[test]
    fn versions_list_is_newest_first_and_unique() {
        let list = versions();
        assert!(list.len() >= 15);
        let mut seen = std::collections::HashSet::new();
        for item in &list {
            assert!(seen.insert(item.mc), "重复版本 {}", item.mc);
            assert_eq!(item.mc, item.label);
        }
        // 顺序必须稳定：从新到旧，首项是节点里最新的那档
        assert_eq!(list.first().unwrap().mc, "26.3");
        assert_eq!(list.last().unwrap().mc, "1.7.10");
        for expected in [
            "1.21.4", "1.21.5", "1.21.6", "1.21.9", "1.21.11", "26.1", "26.2", "26.3",
        ] {
            assert!(list.iter().any(|item| item.mc == expected), "缺 {expected}");
        }
        // 降序：每一档在表里的序号都要比后一项大
        for window in list.windows(2) {
            let first = VERSIONS.iter().position(|(mc, _, _)| *mc == window[0].mc);
            let second = VERSIONS.iter().position(|(mc, _, _)| *mc == window[1].mc);
            assert!(first.unwrap() > second.unwrap(), "顺序不是从新到旧");
        }
    }

    #[test]
    fn dimension_maps_to_cubiomes_constants() {
        assert_eq!(dimension_to_cubiomes(0).unwrap(), DIM_OVERWORLD);
        assert_eq!(dimension_to_cubiomes(1).unwrap(), DIM_NETHER);
        assert_eq!(dimension_to_cubiomes(2).unwrap(), DIM_END);
        // cubiomes 的 DIM 实际取值（不是 1/2/3）
        assert_eq!(DIM_NETHER, -1);
        assert_eq!(DIM_OVERWORLD, 0);
        assert_eq!(DIM_END, 1);
        assert!(dimension_to_cubiomes(3).is_err());
        assert!(dimension_to_cubiomes(-1).is_err());
    }

    #[test]
    fn scale_accepts_any_integer_in_range() {
        for ok in [1, 4, 12, 16, 37, 64] {
            assert_eq!(check_scale(ok).unwrap(), ok);
        }
        for bad in [0, 65, -4, 512] {
            let message = check_scale(bad).unwrap_err().to_string();
            assert!(message.contains("scale 需在 1 到 64 之间"), "{bad} -> {message}");
        }
    }

    #[test]
    fn scale_is_snapped_onto_the_supported_sampling_grid() {
        // 1.18+：1 保持 1，其余向上取整到 4 的倍数
        for (requested, expected) in [(1, 1), (2, 4), (4, 4), (12, 12), (16, 16), (37, 40), (64, 64)] {
            assert_eq!(normalize_scale(requested, MC_1_21_4).unwrap(), expected, "1.21.4 scale {requested}");
        }
        // 1.17-：分层生成只有 1/4/16/64 四档
        for (requested, expected) in [(1, 1), (4, 4), (12, 16), (16, 16), (37, 64), (64, 64)] {
            assert_eq!(normalize_scale(requested, 21).unwrap(), expected, "1.17.1 scale {requested}");
        }
        assert!(normalize_scale(0, MC_1_21_4).is_err());
        assert!(normalize_scale(65, MC_1_21_4).is_err());
    }

    #[test]
    fn continuous_scale_renders_like_the_frontend_asks() {
        // 前端按视口算出来的连续值 12 必须能直接出图，且回传的 scale 是 12
        let view = biome_map_blocking(BiomeRequest {
            mc: MC_1_21_4,
            dim: DIM_OVERWORLD,
            seed: SEED,
            scale: 12,
            x0: -2048,
            z0: -2048,
            tiles_x: 8,
            tiles_y: 8,
        })
        .unwrap();
        assert_eq!(view.scale, 12);
        // 原点按 12 向下对齐：-2048 -> -2052（前端用回传的 originX/originZ 定位）
        assert_eq!((view.origin_x, view.origin_z), (-2052, -2052));
        assert_eq!(view.biomes.len(), 64);
        assert!(view.biomes.iter().all(|id| (0..256).contains(id)));

        // 1.17.1 请求 12 会被抬到 16，响应里的 scale 必须如实回报
        let legacy = biome_map_blocking(BiomeRequest {
            mc: 21,
            dim: DIM_OVERWORLD,
            seed: SEED,
            scale: 12,
            x0: -2048,
            z0: -2048,
            tiles_x: 3,
            tiles_y: 3,
        })
        .unwrap();
        assert_eq!(legacy.scale, 16);
    }

    #[test]
    fn tiles_upper_bound_is_512() {
        assert_eq!(check_tiles(1, 1).unwrap(), (1, 1));
        assert_eq!(check_tiles(512, 512).unwrap(), (512, 512));
        assert!(check_tiles(513, 64).is_err());
        assert!(check_tiles(64, 513).is_err());
        assert!(check_tiles(0, 16).is_err());
        assert!(check_tiles(16, 0).is_err());
    }

    #[test]
    fn surface_height_is_expressed_in_range_units() {
        // scale 1 是方块单位，其余是 1:4
        assert_eq!(surface_y(1), SURFACE_Y_BLOCK);
        for scale in [4, 16, 64] {
            assert_eq!(surface_y(scale), SURFACE_Y_BLOCK / 4);
        }
    }

    #[test]
    fn origin_is_snapped_to_scale_grid() {
        assert_eq!(snap_origin(-2048, -2048, 64), (-2048, -2048));
        assert_eq!(snap_origin(0, 0, 16), (0, 0));
        assert_eq!(snap_origin(-1, 15, 16), (-16, 0));
        assert_eq!(snap_origin(100, -100, 1), (100, -100));
        assert_eq!(snap_origin(-100, 63, 64), (-128, 0));
    }

    #[test]
    fn seed_parsing_handles_negative_and_huge_numbers() {
        assert_eq!(parse_seed("840751423099399").unwrap(), SEED);
        assert_eq!(parse_seed("-1").unwrap(), u64::MAX);
        assert_eq!(parse_seed("-757051213").unwrap(), (-757_051_213i64) as u64);
        // 超 i64 但没超 u64
        assert_eq!(parse_seed("18446744073709551615").unwrap(), u64::MAX);
        assert_eq!(SeedArg::Int(-2).to_seed().unwrap(), (-2i64) as u64);
        assert_eq!(SeedArg::Text("42".to_string()).to_seed().unwrap(), 42);
    }

    #[test]
    fn seed_parsing_rejects_empty_and_overflow() {
        assert!(parse_seed("").is_err());
        assert!(parse_seed("   ").is_err());
        assert!(parse_seed("99999999999999999999999").is_err());
    }

    #[test]
    fn seed_parsing_falls_back_to_java_string_hash() {
        // Java String.hashCode("hello") == 99162322
        assert_eq!(java_string_hash("hello"), 99_162_322);
        assert_eq!(parse_seed("hello").unwrap(), 99_162_322);
        assert_eq!(parse_seed("咕咕").unwrap(), java_string_hash("咕咕") as u64);
    }

    #[test]
    fn biome_chinese_names_cover_common_ids() {
        assert_eq!(biome_name_zh(1), Some("平原"));
        assert_eq!(biome_name_zh(2), Some("沙漠"));
        assert_eq!(biome_name_zh(6), Some("沼泽"));
        assert_eq!(biome_name_zh(21), Some("丛林"));
        assert_eq!(biome_name_zh(173), Some("玄武岩三角洲"));
        assert_eq!(biome_name_zh(183), Some("深暗之域"));
        assert!(BIOME_NAMES_ZH.len() >= 60, "中文表至少覆盖 60 个群系");
        assert_eq!(biome_display_name(MC_1_21_4, 1), "平原");
    }

    #[test]
    fn biome_name_falls_back_when_not_mapped() {
        assert!(biome_name_zh(176).is_none());
        assert_eq!(biome_display_name(MC_1_21_4, 176), "未知群系(#176)");
        // cubiomes 英文名可用的群系，中文表没收录时也要能拿到名字
        assert!(!biome_display_name(MC_1_21_4, 0).is_empty());
    }

    #[test]
    fn rgba_length_matches_tiles() {
        let colors = biome_colors();
        assert_eq!(colors.len(), BIOME_SLOTS * 3);

        let biomes = vec![1i32; 7 * 5];
        let rgba = rgba_from_biomes(&biomes, &colors);
        assert_eq!(rgba.len(), 7 * 5 * 4);
        assert_eq!(&rgba[..4], &[colors[3], colors[4], colors[5], 255]);

        let mut with_invalid = vec![1i32; 2 * 2];
        with_invalid[0] = -1;
        with_invalid[1] = 999;
        let rgba = rgba_from_biomes(&with_invalid, &colors);
        assert_eq!(rgba.len(), 2 * 2 * 4);
        assert_eq!(&rgba[..4], &[128, 128, 128, 255]);
        assert_eq!(&rgba[4..8], &[128, 128, 128, 255]);
    }

    #[test]
    fn base64_matches_reference_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64_decode(&base64_encode(b"foobar")), b"foobar");
    }

    #[test]
    fn structure_names_map_to_cubiomes_ids() {
        assert_eq!(structure_group("village").unwrap(), vec![5].as_slice());
        assert_eq!(
            structure_group("Temple").unwrap(),
            vec![1, 2, 3, 4].as_slice()
        );
        assert_eq!(structure_group("treasure").unwrap(), vec![14].as_slice());
        assert!(structure_group("stronghold").unwrap().is_empty());
        assert!(structure_group("pyramid").is_err());
        assert_eq!(structure_kind_name(5), "village");
        assert_eq!(structure_kind_name(2), "jungle_temple");
        assert_eq!(structure_kind_name(999), "unknown");
    }

    #[test]
    fn every_structure_id_exists_in_cubiomes() {
        for (name, ids) in STRUCTURE_GROUPS {
            for &stype in *ids {
                let (mut region, mut chunk_range, mut dim) = (0i32, 0i32, 0i32);
                let ok = unsafe {
                    seedmap_struct_config(stype, MC_1_21_4, &mut region, &mut chunk_range, &mut dim)
                };
                assert_eq!(ok, 1, "{name} 的 stype {stype} 在 1.21.4 不存在");
                assert!(region > 0, "{name} region_size 应为正");
                assert!(
                    STRUCTURE_KIND_NAMES.iter().any(|(key, _)| *key == stype),
                    "{name} 的 stype {stype} 缺少 kind 名称"
                );
            }
        }
    }

    #[test]
    fn biome_map_png_roundtrips_to_requested_size() {
        let view = biome_map_blocking(BiomeRequest {
            mc: MC_1_21_4,
            dim: DIM_OVERWORLD,
            seed: SEED,
            scale: 64,
            x0: -2048,
            z0: -2048,
            tiles_x: 4,
            tiles_y: 3,
        })
        .unwrap();
        assert_eq!((view.width, view.height), (4, 3));
        assert_eq!(view.scale, 64);
        assert_eq!((view.origin_x, view.origin_z), (-2048, -2048));
        assert_eq!(view.biomes.len(), 12);
        assert_eq!(view.encoding, "png+base64");
        assert!(view.biomes.iter().all(|id| (0..256).contains(id)));

        let png = base64_decode(&view.rgba);
        assert_eq!(crate::png::png_size(&png), Some((4, 3)));
        let image = crate::png::decode_png(&png).unwrap();
        assert_eq!(image.pixels.len(), 4 * 3 * 4);
        // 图上的像素颜色必须和 biomes 一致
        assert_eq!(image.pixels, rgba_from_biomes(&view.biomes, &biome_colors()));
    }

    #[test]
    fn biome_map_rejects_out_of_range_requests() {
        let base = BiomeRequest {
            mc: MC_1_21_4,
            dim: DIM_OVERWORLD,
            seed: SEED,
            scale: 64,
            x0: 0,
            z0: 0,
            tiles_x: 4,
            tiles_y: 4,
        };
        assert!(biome_map_blocking(BiomeRequest { scale: 0, ..base }).is_err());
        assert!(biome_map_blocking(BiomeRequest { scale: 65, ..base }).is_err());
        assert!(biome_map_blocking(BiomeRequest { tiles_x: 513, ..base }).is_err());
        assert!(biome_map_blocking(BiomeRequest { tiles_y: 513, ..base }).is_err(),
            "单边像素上限 512");
    }

    #[test]
    fn nether_and_end_render_too() {
        for dim in [DIM_NETHER, DIM_END] {
            let view = biome_map_blocking(BiomeRequest {
                mc: MC_1_21_4,
                dim,
                seed: 42,
                scale: 16,
                x0: 0,
                z0: 0,
                tiles_x: 2,
                tiles_y: 2,
            })
            .unwrap();
            assert_eq!(view.biomes.len(), 4);
            assert!(view.biomes.iter().all(|id| *id >= 0));
        }
    }

    #[test]
    fn biome_at_and_spawn_are_deterministic() {
        let first = biome_at_blocking(MC_1_21_4, DIM_OVERWORLD, SEED, 0, 0).unwrap();
        let again = biome_at_blocking(MC_1_21_4, DIM_OVERWORLD, SEED, 0, 0).unwrap();
        assert_eq!(first.biome_id, again.biome_id);
        assert!(!first.biome_name.is_empty());

        let spawn = spawn_point_blocking(MC_1_21_4, SEED).unwrap();
        assert!(spawn.x.abs() < 100_000 && spawn.z.abs() < 100_000);
    }

    #[test]
    fn structures_find_villages_within_radius() {
        let hits = structures_blocking(MC_1_21_4, SEED, "village", 0, 0, 512).unwrap();
        assert!(!hits.is_empty(), "1.21.4 主世界原点附近应有村庄");
        for hit in &hits {
            assert_eq!(hit.kind, "village");
            assert!(hit.x.abs() <= 512, "{} 超出半径", hit.x);
            assert!(hit.z.abs() <= 512);
        }
    }

    #[test]
    fn structures_reject_bad_requests() {
        assert!(structures_blocking(MC_1_21_4, 1, "village", 0, 0, 0).is_err());
        assert!(structures_blocking(MC_1_21_4, 1, "village", 0, 0, MAX_RADIUS + 1).is_err());
        assert!(structures_blocking(MC_1_21_4, 1, "castle", 0, 0, 128).is_err());
    }
}

