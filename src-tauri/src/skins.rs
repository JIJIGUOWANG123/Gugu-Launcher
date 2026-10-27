//! 皮肤与披风：本地库（.mc/skins、.mc/capes）+ 应用/上传 + 正版皮肤查询。

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::{AppError, Result};
use crate::paths::Paths;
use crate::util;

const CURRENT_SKIN: &str = "current.txt";
const CURRENT_CAPE: &str = "cape-current.txt";
/// 正版皮肤允许的尺寸。
pub const SKIN_SIZES: [(u32, u32); 2] = [(64, 64), (64, 32)];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkinItem {
    pub name: String,
    pub path: String,
    pub is_current: bool,
    pub size: u64,
    /// data:image/png;base64,…（前端用 canvas 直接画，不走 asset 协议）
    pub data_url: Option<String>,
    /// 随包内置的 Steve / Alex（不写盘、不能删）
    pub builtin: bool,
    /// 正版玩家的 uuid（下载来的皮肤才有）
    pub uuid: Option<String>,
    /// 来源：builtin / local / mojang / thirdparty
    pub source: Option<String>,
    /// 文件是否真的在磁盘上（current.txt 指向的文件可能已被删）
    pub exists: bool,
}

/// 随包内置的两张皮肤（编译期嵌入，安装包 resources 里也带一份）。
pub const BUILTIN_STEVE: &[u8] = include_bytes!("../resources/skins/steve.png");
pub const BUILTIN_ALEX: &[u8] = include_bytes!("../resources/skins/alex.png");
pub const BUILTIN_NAMES: [&str; 2] = ["Steve", "Alex"];

fn builtin_bytes(model: &str) -> (&'static str, &'static [u8]) {
    match model.trim().to_lowercase().as_str() {
        "slim" | "alex" => ("Alex", BUILTIN_ALEX),
        _ => ("Steve", BUILTIN_STEVE),
    }
}

fn builtin_item(model: &str, is_current: bool) -> SkinItem {
    let (name, bytes) = builtin_bytes(model);
    SkinItem {
        name: name.to_string(),
        path: format!("builtin://{name}"),
        is_current,
        size: bytes.len() as u64,
        data_url: Some(format!("data:image/png;base64,{}", util::base64_encode(bytes))),
        builtin: true,
        uuid: None,
        source: Some("builtin".to_string()),
        exists: true,
    }
}


/// 随包的 9 个官方角色（每个都是 wide=经典 + slim=纤细两张）。
pub const BUILTIN_SKINS: [(&str, &[u8], &[u8]); 9] = [
    (
        "ari",
        include_bytes!("../resources/skins/ari-wide.png"),
        include_bytes!("../resources/skins/ari-slim.png"),
    ),
    (
        "efe",
        include_bytes!("../resources/skins/efe-wide.png"),
        include_bytes!("../resources/skins/efe-slim.png"),
    ),
    (
        "kai",
        include_bytes!("../resources/skins/kai-wide.png"),
        include_bytes!("../resources/skins/kai-slim.png"),
    ),
    (
        "makena",
        include_bytes!("../resources/skins/makena-wide.png"),
        include_bytes!("../resources/skins/makena-slim.png"),
    ),
    (
        "noor",
        include_bytes!("../resources/skins/noor-wide.png"),
        include_bytes!("../resources/skins/noor-slim.png"),
    ),
    (
        "steve",
        include_bytes!("../resources/skins/steve-wide.png"),
        include_bytes!("../resources/skins/steve-slim.png"),
    ),
    (
        "sunny",
        include_bytes!("../resources/skins/sunny-wide.png"),
        include_bytes!("../resources/skins/sunny-slim.png"),
    ),
    (
        "zuri",
        include_bytes!("../resources/skins/zuri-wide.png"),
        include_bytes!("../resources/skins/zuri-slim.png"),
    ),
    (
        "alex",
        include_bytes!("../resources/skins/alex-wide.png"),
        include_bytes!("../resources/skins/alex-slim.png"),
    ),
];

/// 内置角色名表（第一项是默认皮肤）。
pub const BUILTIN_CHARACTERS: [&str; 9] = [
    "steve", "alex", "ari", "efe", "kai", "makena", "noor", "sunny", "zuri",
];

/// 模型名归一：classic/default/wide → wide，slim → slim。
fn model_key(model: &str) -> &'static str {
    match model.trim().to_lowercase().as_str() {
        "slim" | "alex" => "slim",
        _ => "wide",
    }
}

fn character_key(name: &str) -> String {
    name.trim().to_lowercase()
}

/// 取一个内置角色的字节；名字不认识返回 None。
pub fn builtin_skin(name: &str, model: &str) -> Option<(&'static str, &'static [u8])> {
    let key = character_key(name);
    let entry = BUILTIN_SKINS
        .iter()
        .find(|(candidate, _, _)| *candidate == key)?;
    let bytes = if model_key(model) == "slim" {
        entry.2
    } else {
        entry.1
    };
    Some((entry.0, bytes))
}

fn display_name(key: &str) -> String {
    let mut chars = key.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => key.to_string(),
    }
}

/// 内置角色条目（不落盘、不能删）。
pub fn builtin_skin_item(name: &str, model: &str, is_current: bool) -> Option<(SkinItem, String)> {
    let (key, bytes) = builtin_skin(name, model)?;
    let item = SkinItem {
        name: display_name(key),
        path: format!("builtin://{key}-{}", model_key(model)),
        is_current,
        size: bytes.len() as u64,
        data_url: Some(format!("data:image/png;base64,{}", util::base64_encode(bytes))),
        builtin: true,
        uuid: None,
        source: Some("builtin".to_string()),
        exists: true,
    };
    Some((item, util::hex(&util::sha256(bytes))))
}

/// skin_default({name?, model?})：9 个官方角色 × classic/slim。
pub fn default_skin_named(name: Option<&str>, model: &str) -> Result<(SkinItem, String)> {
    let key = match name.map(character_key).filter(|value| !value.is_empty()) {
        Some(key) => key,
        // 没给名字时沿用老的 Steve / Alex
        None => {
            if model_key(model) == "slim" {
                "alex".to_string()
            } else {
                "steve".to_string()
            }
        }
    };
    builtin_skin_item(&key, model, false)
        .ok_or_else(|| AppError::Invalid(format!("没有这个内置皮肤：{key}")))
}

/// skin_default({model})：没给角色名时返回内置 Steve / Alex（带 sha256 便于校验）。
pub fn default_skin(model: &str) -> (SkinItem, String) {
    default_skin_named(None, model).expect("默认内置皮肤一定存在")
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapeItem {
    pub name: String,
    pub path: String,
    pub is_current: bool,
    pub size: u64,
    pub data_url: Option<String>,
}

/// data URL 上限：皮肤/披风都是几十 KB，2MB 足够且不会把 IPC 撑爆。
pub const MAX_DATA_URL_BYTES: usize = 2 * 1024 * 1024;

/// 读成 data URL；文件太大或读不到就返回 None（前端回落到占位图）。
pub fn png_data_url(path: &Path, limit: usize) -> Option<String> {
    let metadata = std::fs::metadata(path).ok()?;
    if metadata.len() as usize > limit {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    Some(format!(
        "data:image/png;base64,{}",
        util::base64_encode(&bytes)
    ))
}

fn size_and_data_url(path: &Path) -> (u64, Option<String>) {
    let size = std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0);
    (size, png_data_url(path, MAX_DATA_URL_BYTES))
}

pub fn skins_dir(paths: &Paths) -> PathBuf {
    paths.skins.clone()
}

pub fn capes_dir(paths: &Paths) -> PathBuf {
    paths.root.join("capes")
}

/// 文件名安全化（皮肤名可能带空格/中文）。
pub fn safe_name(name: &str) -> String {
    let cleaned = util::sanitize_file_part(name).replace('.', "_");
    if cleaned.trim().is_empty() {
        "skin".to_string()
    } else {
        cleaned
    }
}

fn read_current(dir: &Path, marker: &str) -> Option<String> {
    std::fs::read_to_string(dir.join(marker))
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

fn write_current(dir: &Path, marker: &str, name: &str) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join(marker), name)?;
    Ok(())
}

fn list_png(dir: &Path) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.filter_map(|entry| entry.ok()) {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default();
        if name.to_lowercase().ends_with(".png") {
            out.push((name, path));
        }
    }
    out.sort_by(|left, right| left.0.cmp(&right.0));
    out
}

pub fn list_skins(paths: &Paths) -> Vec<SkinItem> {
    let dir = skins_dir(paths);
    let current = read_current(&dir, CURRENT_SKIN);
    // 内置 Steve / Alex 永远在列表最前面；没选过皮肤时 Steve 就是当前
    let mut items = vec![
        builtin_item("classic", current.is_none()),
        builtin_item("slim", false),
    ];
    items.extend(local_skins(&dir, current.as_deref()));
    items
}

fn local_skins(dir: &Path, current: Option<&str>) -> Vec<SkinItem> {
    list_png(dir)
        .into_iter()
        .map(|(name, path)| {
            let (size, data_url) = size_and_data_url(&path);
            SkinItem {
                is_current: current == Some(name.as_str()),
                name,
                path: path.display().to_string(),
                size,
                data_url,
                builtin: false,
                uuid: None,
                source: Some("local".to_string()),
                exists: path.is_file(),
            }
        })
        .collect()
}

pub fn list_capes(paths: &Paths) -> Vec<CapeItem> {
    let dir = capes_dir(paths);
    let current = read_current(&dir, CURRENT_CAPE);
    list_png(&dir)
        .into_iter()
        .map(|(name, path)| {
            let (size, data_url) = size_and_data_url(&path);
            CapeItem {
                is_current: current.as_deref() == Some(name.as_str()),
                name,
                path: path.display().to_string(),
                size,
                data_url,
            }
        })
        .collect()
}

/// skin_current()：当前皮肤；离线账号没选过就是内置 Steve（永远不为 None）。
pub fn current_skin(paths: &Paths) -> Option<SkinItem> {
    let items = list_skins(paths);
    if let Some(item) = items.iter().find(|item| item.is_current) {
        return Some(item.clone());
    }
    let dir = skins_dir(paths);
    // current.txt 指向的文件没了：报 exists=false，别偷偷回落 Steve
    if let Some(name) = read_current(&dir, CURRENT_SKIN) {
        let path = dir.join(&name);
        return Some(SkinItem {
            exists: path.is_file(),
            name,
            path: path.display().to_string(),
            is_current: true,
            size: std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0),
            data_url: png_data_url(&path, MAX_DATA_URL_BYTES),
            builtin: false,
            uuid: None,
            source: Some("local".to_string()),
        });
    }
    items.first().cloned()
}

/// cape_current()：当前披风（没选就是 None）。
pub fn current_cape(paths: &Paths) -> Option<CapeItem> {
    list_capes(paths).into_iter().find(|item| item.is_current)
}

/// 只允许删 .mc/skins 下的 png：禁止分隔符、.. 与绝对路径。
pub fn validate_skin_file_name(name: &str) -> Result<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(AppError::Invalid("皮肤名不能为空".to_string()));
    }
    if trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.contains("..")
        || Path::new(trimmed).is_absolute()
    {
        return Err(AppError::Invalid(format!("皮肤名不合法：{name}")));
    }
    if !trimmed.to_lowercase().ends_with(".png") {
        return Err(AppError::Invalid(format!("只能删除 png 皮肤：{name}")));
    }
    Ok(trimmed.to_string())
}

/// skins_delete：删掉本地皮肤；删的是当前皮肤时回落内置 Steve。
pub fn delete_local_skin(paths: &Paths, name: &str) -> Result<()> {
    let file = validate_skin_file_name(name)?;
    let dir = skins_dir(paths);
    let path = dir.join(&file);
    if !path.is_file() {
        return Err(AppError::Invalid(
            "该皮肤不存在（可能已被删除）".to_string(),
        ));
    }
    std::fs::remove_file(&path)?;
    if read_current(&dir, CURRENT_SKIN).as_deref() == Some(file.as_str()) {
        let marker = dir.join(CURRENT_SKIN);
        let _ = std::fs::remove_file(marker);
    }
    tracing::info!(name = %file, "删除本地皮肤");
    Ok(())
}

/// 校验 PNG 头并取尺寸；只允许 64x64 / 64x32。
pub fn validate_skin_png(bytes: &[u8]) -> Result<(u32, u32)> {
    const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    if bytes.len() < 24 || bytes[..8] != SIGNATURE {
        return Err(AppError::Invalid("不是合法的 PNG 文件".to_string()));
    }
    if &bytes[12..16] != b"IHDR" {
        return Err(AppError::Invalid("PNG 缺少 IHDR 块".to_string()));
    }
    let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    if !SKIN_SIZES.contains(&(width, height)) {
        return Err(AppError::Invalid(format!(
            "皮肤尺寸必须是 64x64 或 64x32，收到 {width}x{height}"
        )));
    }
    Ok((width, height))
}

/// 应用一个本地 png 作为当前皮肤（复制进 .mc/skins）。
pub fn apply_skin(paths: &Paths, source: &str) -> Result<SkinItem> {
    let source_path = Path::new(source.trim());
    if !source_path.is_file() {
        return Err(AppError::Invalid(format!("皮肤文件不存在：{source}")));
    }
    let bytes = std::fs::read(source_path)?;
    validate_skin_png(&bytes)?;
    let dir = skins_dir(paths);
    std::fs::create_dir_all(&dir)?;
    let name = source_path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "skin.png".to_string());
    let name = if name.to_lowercase().ends_with(".png") {
        name
    } else {
        format!("{name}.png")
    };
    let target = dir.join(&name);
    if source_path != target {
        std::fs::copy(source_path, &target)?;
    }
    write_current(&dir, CURRENT_SKIN, &name)?;
    let (size, data_url) = size_and_data_url(&target);
    Ok(SkinItem {
        name,
        path: target.display().to_string(),
        is_current: true,
        size,
        data_url,
        builtin: false,
        uuid: None,
        source: Some("local".to_string()),
        exists: true,
    })
}

/// 前端上传的 base64 png：校验尺寸后落盘并设为当前皮肤。
pub fn upload_skin(paths: &Paths, name: &str, png_base64: &str) -> Result<SkinItem> {
    let data = util::base64_decode(png_base64)
        .ok_or_else(|| AppError::Invalid("皮肤数据不是合法的 base64".to_string()))?;
    validate_skin_png(&data)?;
    let dir = skins_dir(paths);
    std::fs::create_dir_all(&dir)?;
    let base = safe_name(name);
    let file_name = if base.to_lowercase().ends_with(".png") {
        base
    } else {
        format!("{base}.png")
    };
    let target = dir.join(&file_name);
    std::fs::write(&target, &data)?;
    write_current(&dir, CURRENT_SKIN, &file_name)?;
    let (size, data_url) = size_and_data_url(&target);
    Ok(SkinItem {
        name: file_name,
        path: target.display().to_string(),
        is_current: true,
        size,
        data_url,
        builtin: false,
        uuid: None,
        source: Some("local".to_string()),
        exists: true,
    })
}

pub fn apply_cape(paths: &Paths, name: &str) -> Result<CapeItem> {
    let dir = capes_dir(paths);
    let file_name = safe_name(name);
    let file_name = if file_name.to_lowercase().ends_with(".png") {
        file_name
    } else {
        format!("{file_name}.png")
    };
    let target = dir.join(&file_name);
    if !target.is_file() {
        return Err(AppError::Invalid(format!("披风不存在：{file_name}")));
    }
    write_current(&dir, CURRENT_CAPE, &file_name)?;
    let (size, data_url) = size_and_data_url(&target);
    Ok(CapeItem {
        name: file_name,
        path: target.display().to_string(),
        is_current: true,
        size,
        data_url,
    })
}

pub fn clear_cape(paths: &Paths) -> Result<()> {
    let dir = capes_dir(paths);
    let marker = dir.join(CURRENT_CAPE);
    if marker.is_file() {
        std::fs::remove_file(marker)?;
    }
    Ok(())
}

/// 解析 Mojang 会话服务器返回的皮肤/披风地址。
/// textures 是 base64 后的 JSON：{"textures":{"SKIN":{"url":...},"CAPE":{"url":...}}}
pub fn parse_mojang_profile(json: &str) -> Result<(Option<String>, Option<String>)> {
    let value: serde_json::Value = serde_json::from_str(json)
        .map_err(|err| AppError::Json(err))?;
    if let Some(error) = value.get("error").and_then(|item| item.as_str()) {
        let message = value
            .get("errorMessage")
            .and_then(|item| item.as_str())
            .unwrap_or(error);
        return Err(AppError::Other(format!("皮肤查询失败：{message}")));
    }
    let encoded = value
        .get("properties")
        .and_then(|properties| properties.as_array())
        .and_then(|items| {
            items.iter().find_map(|item| {
                if item.get("name").and_then(|name| name.as_str()) == Some("textures") {
                    item.get("value").and_then(|value| value.as_str())
                } else {
                    None
                }
            })
        });
    let Some(encoded) = encoded else {
        return Err(AppError::Other("该玩家没有公开皮肤".to_string()));
    };
    let decoded = util::base64_decode(encoded)
        .ok_or_else(|| AppError::Other("皮肤数据不是合法 base64".to_string()))?;
    let textures: serde_json::Value = serde_json::from_slice(&decoded)
        .map_err(|_| AppError::Other("皮肤数据不是合法 JSON".to_string()))?;
    let pick = |key: &str| {
        textures
            .pointer(&format!("/textures/{key}/url"))
            .and_then(|url| url.as_str())
            .map(|url| url.to_string())
    };
    Ok((pick("SKIN"), pick("CAPE")))
}


// ------------------------------------------------------------ 第三方皮肤站（Yggdrasil 查询）

/// 皮肤站的 Yggdrasil 根地址规范化：必须 http(s)、末尾斜杠去掉。
pub fn normalize_server(server: &str) -> Result<String> {
    let trimmed = server.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err(AppError::Invalid("皮肤站地址不能为空".to_string()));
    }
    if !crate::settings::is_http_url(trimmed) {
        return Err(AppError::Invalid(format!(
            "皮肤站地址必须是 http(s)：{trimmed}"
        )));
    }
    Ok(trimmed.to_string())
}

/// skin_lookup_thirdparty 的返回。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ThirdPartySkin {
    pub uuid: String,
    pub name: String,
    pub skin_url: Option<String>,
    pub cape_url: Option<String>,
    /// default / slim（皮肤站用 metadata.model 表示）
    pub model: Option<String>,
}

/// 把皮肤站返回的相对地址补成绝对地址。
pub fn absolute_texture_url(url: &str, server: &str) -> String {
    let trimmed = url.trim();
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return trimmed.to_string();
    }
    if let Some(rest) = trimmed.strip_prefix("//") {
        return format!("https://{rest}");
    }
    if trimmed.starts_with("textures/") || trimmed.starts_with("texture/") {
        // 官方 textures 域名的相对写法
        return format!("https://textures.minecraft.net/{trimmed}");
    }
    // 皮肤站自己的相对路径：取 server 的 scheme://host
    if trimmed.starts_with('/') {
        if let Some(index) = server.find("://") {
            let rest = &server[index + 3..];
            let host = rest.split('/').next().unwrap_or(rest);
            return format!("{}://{host}{trimmed}", &server[..index]);
        }
    }
    trimmed.to_string()
}

/// textures 属性（base64 后的 JSON）→ (皮肤 URL, 披风 URL, 模型)。
pub fn parse_textures_value(value: &str) -> (Option<String>, Option<String>, Option<String>) {
    let Some(bytes) = util::base64_decode(value) else {
        return (None, None, None);
    };
    let Ok(json) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return (None, None, None);
    };
    let skin = json.pointer("/textures/SKIN/url").and_then(|i| i.as_str());
    let cape = json.pointer("/textures/CAPE/url").and_then(|i| i.as_str());
    let model = json
        .pointer("/textures/SKIN/metadata/model")
        .and_then(|i| i.as_str());
    (
        skin.map(|s| s.to_string()),
        cape.map(|s| s.to_string()),
        model.map(|s| s.to_string()),
    )
}

/// 解析皮肤站 profile 响应（name 查询与 session 查询两种形态都支持）。
pub fn parse_thirdparty_profile(json: &str, server: &str) -> Result<ThirdPartySkin> {
    let value: serde_json::Value = serde_json::from_str(json).map_err(AppError::Json)?;
    if let Some(error) = value.get("error").and_then(|i| i.as_str()) {
        let message = value
            .get("errorMessage")
            .and_then(|i| i.as_str())
            .unwrap_or(error);
        return Err(AppError::Other(format!("皮肤站返回错误：{message}")));
    }
    let name = value
        .get("name")
        .and_then(|i| i.as_str())
        .unwrap_or("")
        .to_string();
    let uuid = value
        .get("id")
        .and_then(|i| i.as_str())
        .unwrap_or("")
        .to_string();
    // 1) skins 数组（LittleSkin / Blessing Skin 的 name 查询形态）
    let (mut skin, mut cape, mut model) = (None, None, None);
    if let Some(skins) = value.get("skins").and_then(|i| i.as_array()) {
        for entry in skins {
            let state = entry.get("state").and_then(|i| i.as_str()).unwrap_or("ACTIVE");
            if !state.eq_ignore_ascii_case("ACTIVE") {
                continue;
            }
            if let Some(url) = entry.get("url").and_then(|i| i.as_str()) {
                if skin.is_none() {
                    skin = Some(absolute_texture_url(url, server));
                }
            }
            if let Some(url) = entry.get("cape").and_then(|i| i.as_str()).or_else(|| {
                entry.get("capeUrl").and_then(|i| i.as_str())
            }) {
                cape = Some(absolute_texture_url(url, server));
            }
            if let Some(kind) = entry.get("variant").and_then(|i| i.as_str()) {
                model = Some(kind.to_string());
            } else if let Some(kind) = entry.get("model").and_then(|i| i.as_str()) {
                model = Some(kind.to_string());
            }
        }
    }
    // 2) properties[].value 里的 base64 textures（官方 / session 形态）
    if skin.is_none() || cape.is_none() {
        if let Some(properties) = value.get("properties").and_then(|i| i.as_array()) {
            for entry in properties {
                let is_textures = entry.get("name").and_then(|i| i.as_str()) == Some("textures");
                let Some(encoded) = entry.get("value").and_then(|i| i.as_str()) else {
                    continue;
                };
                if !is_textures && skin.is_none() {
                    continue;
                }
                let (found_skin, found_cape, found_model) = parse_textures_value(encoded);
                if skin.is_none() {
                    skin = found_skin.map(|url| absolute_texture_url(&url, server));
                }
                if cape.is_none() {
                    cape = found_cape.map(|url| absolute_texture_url(&url, server));
                }
                if model.is_none() {
                    model = found_model;
                }
            }
        }
    }
    if name.is_empty() && uuid.is_empty() {
        return Err(AppError::Other("该玩家不存在".to_string()));
    }
    if skin.is_none() {
        return Err(AppError::Other(format!("该玩家没有设置皮肤（{}）", name)));
    }
    Ok(ThirdPartySkin {
        uuid,
        name,
        skin_url: skin,
        cape_url: cape,
        model,
    })
}

/// 导入结果：既有条目 + 是否复用（不新建文件）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkinImportResult {
    #[serde(flatten)]
    pub item: SkinItem,
    pub reused: bool,
}

/// 在 .mc/skins 里找 sha256 相同的文件（导入去重）。
pub fn find_skin_by_sha256(dir: &Path, hash: &str) -> Option<(String, PathBuf)> {
    if hash.trim().is_empty() {
        return None;
    }
    for (name, path) in list_png(dir) {
        if let Ok(current) = util::sha256_file(&path) {
            if current.eq_ignore_ascii_case(hash.trim()) {
                return Some((name, path));
            }
        }
    }
    None
}

/// 下载皮肤用的 UA（textures.minecraft.net 与各家皮肤站都认这个）。
pub const SKIN_DOWNLOAD_USER_AGENT: &str = "Mozilla/5.0 GuguLauncher/1.0";

/// 导入落盘时的文件名：清洗掉空格与路径分隔符，统一补 .png。
pub fn skin_file_name_for(name: Option<&str>) -> String {
    let base = match name.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => safe_name(value).trim().to_string(),
        None => String::new(),
    };
    let base = if base.is_empty() {
        "skin".to_string()
    } else {
        base
    };
    if base.to_lowercase().ends_with(".png") {
        base
    } else {
        format!("{base}.png")
    }
}

/// 把状态码 / 网络错误翻成中文。
pub fn map_lookup_error(status: Option<u16>, message: &str, server: &str) -> String {
    match status {
        Some(404) => "该玩家不存在".to_string(),
        Some(401) | Some(403) => "皮肤站拒绝了请求（可能需要登录）".to_string(),
        Some(other) => format!("皮肤站返回 HTTP {other}"),
        None => {
            if message.to_lowercase().contains("timed out")
                || message.contains("timeout")
                || message.contains("超时")
            {
                format!("皮肤站无响应（{server}）")
            } else {
                format!("无法连接皮肤站（{server}）：{message}")
            }
        }
    }
}

fn host_of(server: &str) -> String {
    server
        .split("://")
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .unwrap_or(server)
        .to_string()
}

async fn get_json(
    client: &reqwest::Client,
    url: &str,
    server: &str,
) -> Result<serde_json::Value> {
    let response = client
        .get(url)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|err| AppError::Other(map_lookup_error(None, &err.to_string(), server)))?;
    if !response.status().is_success() {
        return Err(AppError::Other(map_lookup_error(
            Some(response.status().as_u16()),
            "",
            server,
        )));
    }
    response
        .json::<serde_json::Value>()
        .await
        .map_err(|err| AppError::Other(format!("皮肤站返回不是 JSON：{err}")))
}

/// 第三方皮肤查询：先按名字查，失败再把入参当 UUID 查 session 端点。
pub async fn lookup_thirdparty(
    client: &reqwest::Client,
    server: &str,
    name_or_uuid: &str,
) -> Result<ThirdPartySkin> {
    let server = normalize_server(server)?;
    let query = name_or_uuid.trim();
    if query.is_empty() {
        return Err(AppError::Invalid("玩家名或 UUID 不能为空".to_string()));
    }
    let by_name = format!(
        "{server}/minecraft/profile/lookup/name/{}",
        percent_encode(query)
    );
    match get_json(client, &by_name, &server).await {
        Ok(value) => {
            let json = serde_json::to_string(&value)?;
            return parse_thirdparty_profile(&json, &server);
        }
        Err(err) => tracing::info!(
            %err,
            host = %host_of(&server),
            "按名字查皮肤失败，改用 UUID 查询"
        ),
    }
    let uuid: String = query.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if uuid.len() != 32 {
        return Err(AppError::Other(format!(
            "该玩家不存在（{query}，皮肤站 {}）",
            host_of(&server)
        )));
    }
    let dashed = crate::accounts::dashed_uuid(&uuid);
    let by_uuid = format!(
        "{server}/sessionserver/session/minecraft/profile/{}",
        percent_encode(&dashed)
    );
    let value = get_json(client, &by_uuid, &server).await?;
    parse_thirdparty_profile(&serde_json::to_string(&value)?, &server)
}

fn percent_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// 皮肤重名时追加 (1)(2)…
fn unique_skin_target(dir: &Path, file_name: &str) -> PathBuf {
    let candidate = dir.join(file_name);
    if !candidate.exists() {
        return candidate;
    }
    let stem = file_name.trim_end_matches(".png");
    for index in 1..1000 {
        let next = dir.join(format!("{stem} ({index}).png"));
        if !next.exists() {
            return next;
        }
    }
    dir.join(format!("{stem}-{}.png", util::unique_suffix()))
}

/// 从绝对 URL 下载皮肤进 .mc/skins（不改当前选中，前端再调 skin_apply）。
pub async fn import_from_url(
    client: &reqwest::Client,
    paths: &Paths,
    url: &str,
    name: Option<&str>,
) -> Result<SkinImportResult> {
    if !crate::settings::is_http_url(url) {
        return Err(AppError::Invalid(format!(
            "皮肤地址必须是 http(s)：{url}"
        )));
    }
    let response = client
        .get(url)
        .timeout(std::time::Duration::from_secs(15))
        // traits 会拒绝空 UA/非浏览器 UA；官方贴图只有 1–2KB，一样照收
        .header(reqwest::header::USER_AGENT, SKIN_DOWNLOAD_USER_AGENT)
        .header(reqwest::header::ACCEPT, "image/png,image/*;q=0.8,*/*;q=0.5")
        .send()
        .await
        .map_err(|err| AppError::Other(format!("下载皮肤失败：{err}")))?;
    if !response.status().is_success() {
        return Err(AppError::Other(format!(
            "下载皮肤失败：HTTP {}",
            response.status().as_u16()
        )));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|err| AppError::Other(format!("读取皮肤内容失败：{err}")))?;
    validate_skin_png(&bytes)?;
    let dir = skins_dir(paths);
    std::fs::create_dir_all(&dir)?;
    // 同一张图重复导入：直接复用既有文件，别再攒新名字
    let hash = util::sha256_file_of_bytes(&bytes);
    if let Some((existing, path)) = find_skin_by_sha256(&dir, &hash) {
        let (size, data_url) = size_and_data_url(&path);
        tracing::info!(
            name = %existing,
            sha256 = %&hash[..12.min(hash.len())],
            "皮肤已存在，复用"
        );
        return Ok(SkinImportResult {
            item: SkinItem {
                name: existing,
                path: path.display().to_string(),
                is_current: false,
                size,
                data_url,
                builtin: false,
                uuid: None,
                source: Some("thirdparty".to_string()),
                exists: true,
            },
            reused: true,
        });
    }
    let file_name = skin_file_name_for(name);
    let target = unique_skin_target(&dir, &file_name);
    std::fs::write(&target, &bytes)?;
    let (size, data_url) = size_and_data_url(&target);
    let name = target
        .file_name()
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or(file_name);
    tracing::info!(
        name = %name,
        sha256 = %&hash[..12.min(hash.len())],
        "导入新皮肤"
    );
    Ok(SkinImportResult {
        item: SkinItem {
            name,
            path: target.display().to_string(),
            is_current: false,
            size,
            data_url,
            builtin: false,
            uuid: None,
            source: Some("thirdparty".to_string()),
            exists: true,
        },
        reused: false,
    })
}


// ------------------------------------------------------------ 正版玩家皮肤（uapis 接口）

pub const UAPIS_USERINFO: &str = "https://uapis.cn/api/v1/game/minecraft/userinfo";
pub const UAPIS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(8);

/// uapis 的响应：{username, uuid（32 位无连字符）, skin_url}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UapisUserInfo {
    pub username: String,
    pub uuid: String,
    pub skin_url: Option<String>,
}

/// 32 位无连字符 → 标准 8-4-4-4-12。
pub fn normalize_uuid(raw: &str) -> String {
    crate::accounts::dashed_uuid(&raw.chars().filter(|c| c.is_ascii_hexdigit()).collect::<String>())
}

/// http → https（textures.minecraft.net 支持 https）。
pub fn upgrade_to_https(url: &str) -> String {
    if let Some(rest) = url.strip_prefix("http://") {
        return format!("https://{rest}");
    }
    url.to_string()
}

/// 解析 uapis 的 userinfo 响应。
pub fn parse_userinfo(json: &str) -> Result<UapisUserInfo> {
    let value: serde_json::Value = serde_json::from_str(json).map_err(AppError::Json)?;
    if let Some(error) = value.get("error").and_then(|i| i.as_str()) {
        return Err(AppError::Other(match error {
            "username query parameter is required" => "玩家名不能为空".to_string(),
            other => format!("皮肤服务返回错误：{other}"),
        }));
    }
    let username = value
        .get("username")
        .and_then(|i| i.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if username.is_empty() {
        return Err(AppError::Other("没有这个正版玩家".to_string()));
    }
    let uuid = value
        .get("uuid")
        .and_then(|i| i.as_str())
        .map(normalize_uuid)
        .unwrap_or_default();
    let skin_url = value
        .get("skin_url")
        .and_then(|i| i.as_str())
        .map(|url| url.trim().to_string())
        .filter(|url| !url.is_empty())
        .map(|url| upgrade_to_https(&url));
    Ok(UapisUserInfo {
        username,
        uuid,
        skin_url,
    })
}

/// 状态码 → 中文错误。
pub fn map_userinfo_error(status: Option<u16>, message: &str) -> String {
    match status {
        Some(400) => "玩家名不能为空".to_string(),
        Some(404) => "没有这个正版玩家".to_string(),
        Some(other) if other >= 500 => format!("皮肤服务暂时不可用（HTTP {other}）"),
        Some(other) => format!("皮肤服务返回 HTTP {other}"),
        None => {
            if message.to_lowercase().contains("timed out") || message.contains("timeout") {
                "无法连接皮肤服务（超时）".to_string()
            } else {
                format!("无法连接皮肤服务：{message}")
            }
        }
    }
}

fn encode_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for byte in name.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// skin_fetch_mojang({username})：从 uapis 查正版玩家皮肤并下载落盘。
pub async fn fetch_mojang_skin(
    client: &reqwest::Client,
    paths: &Paths,
    username: &str,
) -> Result<SkinItem> {
    let name = username.trim();
    if name.is_empty() {
        return Err(AppError::Invalid("玩家名不能为空".to_string()));
    }
    let url = format!("{UAPIS_USERINFO}?username={}", encode_name(name));
    let response = client
        .get(&url)
        .timeout(UAPIS_TIMEOUT)
        .send()
        .await
        .map_err(|err| AppError::Other(map_userinfo_error(None, &err.to_string())))?;
    if !response.status().is_success() {
        return Err(AppError::Other(map_userinfo_error(
            Some(response.status().as_u16()),
            "",
        )));
    }
    let text = response
        .text()
        .await
        .map_err(|err| AppError::Other(format!("读取皮肤服务响应失败：{err}")))?;
    let info = parse_userinfo(&text)?;
    let Some(skin_url) = info.skin_url.clone() else {
        return Err(AppError::Other("该玩家没有公开皮肤".to_string()));
    };

    let image = client
        .get(&skin_url)
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .map_err(|err| AppError::Other(format!("下载皮肤失败：{err}")))?;
    if !image.status().is_success() {
        return Err(AppError::Other(format!(
            "下载皮肤失败：HTTP {}",
            image.status().as_u16()
        )));
    }
    let bytes = image
        .bytes()
        .await
        .map_err(|err| AppError::Other(format!("读取皮肤内容失败：{err}")))?;
    if validate_skin_png(&bytes).is_err() {
        return Err(AppError::Other(format!(
            "这不是有效的皮肤文件（{} 字节）",
            bytes.len()
        )));
    }
    let dir = skins_dir(paths);
    std::fs::create_dir_all(&dir)?;
    let file_name = format!("{}.png", safe_name(&info.username));
    let target = unique_skin_target(&dir, &file_name);
    std::fs::write(&target, &bytes)?;
    let (size, data_url) = size_and_data_url(&target);
    let name = target
        .file_name()
        .map(|v| v.to_string_lossy().to_string())
        .unwrap_or(file_name);
    Ok(SkinItem {
        name,
        path: target.display().to_string(),
        is_current: false,
        size,
        data_url,
        builtin: false,
        uuid: if info.uuid.is_empty() {
            None
        } else {
            Some(info.uuid)
        },
        source: Some("mojang".to_string()),
        exists: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_paths(tag: &str) -> Paths {
        let root = std::env::temp_dir().join(format!(
            "gugu-skin-{tag}-{}-{}",
            std::process::id(),
            util::unique_suffix()
        ));
        let paths = Paths::from_root(&root);
        paths.ensure().unwrap();
        std::fs::create_dir_all(capes_dir(&paths)).unwrap();
        paths
    }

    fn png_bytes(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        bytes.extend_from_slice(&13u32.to_be_bytes());
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&[8, 6, 0, 0, 0]);
        bytes.extend_from_slice(&[0, 0, 0, 0]);
        bytes
    }

    #[test]
    fn png_size_validation() {
        assert_eq!(validate_skin_png(&png_bytes(64, 64)).unwrap(), (64, 64));
        assert_eq!(validate_skin_png(&png_bytes(64, 32)).unwrap(), (64, 32));
        assert!(validate_skin_png(&png_bytes(128, 128)).is_err());
        assert!(validate_skin_png(b"not a png").is_err());
        let mut broken = png_bytes(64, 64);
        broken[12] = b'X';
        assert!(validate_skin_png(&broken).is_err());
    }

    #[test]
    fn apply_and_upload_skin_marks_current() {
        let paths = temp_paths("apply");
        let source = paths.root.join("incoming.png");
        std::fs::write(&source, png_bytes(64, 64)).unwrap();
        let applied = apply_skin(&paths, &source.display().to_string()).unwrap();
        assert!(applied.is_current);
        assert!(Path::new(&applied.path).is_file());
        assert!(applied.size > 0);
        let data_url = applied.data_url.clone().unwrap_or_default();
        assert!(data_url.starts_with("data:image/png;base64,"), "{data_url}");
        assert_eq!(current_skin(&paths).map(|item| item.name), Some(applied.name.clone()));

        let uploaded = upload_skin(&paths, "我的皮肤", &util::base64_encode(&png_bytes(64, 32))).unwrap();
        assert!(uploaded.name.starts_with("我的皮肤"));
        let items = list_skins(&paths);
        assert_eq!(items.len(), 4, "内置 Steve/Alex + 两个本地皮肤");
        assert_eq!(items.iter().filter(|item| item.is_current).count(), 1);
        assert!(items
            .iter()
            .any(|item| item.is_current && item.name == uploaded.name));

        // 非法 base64 / 尺寸要报错
        assert!(upload_skin(&paths, "bad", "!!!!").is_err());
        assert!(upload_skin(&paths, "big", &util::base64_encode(&png_bytes(128, 128))).is_err());
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn capes_apply_and_clear() {
        let paths = temp_paths("cape");
        let cape = capes_dir(&paths).join("我的披风.png");
        std::fs::write(&cape, png_bytes(64, 32)).unwrap();
        let applied = apply_cape(&paths, "我的披风").unwrap();
        assert!(applied.is_current);
        assert!(applied.data_url.unwrap_or_default().starts_with("data:image/png"));
        assert!(list_capes(&paths).iter().any(|item| item.is_current));
        assert_eq!(current_cape(&paths).map(|item| item.name), Some("我的披风.png".to_string()));
        clear_cape(&paths).unwrap();
        assert!(list_capes(&paths).iter().all(|item| !item.is_current));
        assert!(current_cape(&paths).is_none());
        assert!(apply_cape(&paths, "不存在").is_err());
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn builtin_steve_and_alex_are_always_listed() {
        // 内置资源必须是真的 64x64 PNG
        assert_eq!(validate_skin_png(BUILTIN_STEVE).unwrap(), (64, 64));
        assert_eq!(validate_skin_png(BUILTIN_ALEX).unwrap(), (64, 64));
        let paths = temp_paths("builtin");
        let items = list_skins(&paths);
        assert_eq!(items.len(), 2, "没本地皮肤时只列内置两张");
        assert!(items.iter().all(|item| item.builtin));
        assert_eq!(items[0].name, "Steve");
        assert_eq!(items[1].name, "Alex");
        assert!(items[0].is_current, "没选过皮肤时 Steve 是当前");
        assert!(items[0].path.starts_with("builtin://"));
        let current = current_skin(&paths).unwrap();
        assert_eq!(current.name, "Steve");

        let (item, sha256) = default_skin("classic");
        assert_eq!(item.name, "Steve");
        assert_eq!(item.size as usize, BUILTIN_STEVE.len());
        assert_eq!(sha256.len(), 64);
        let (slim, _) = default_skin("slim");
        assert_eq!(slim.name, "Alex");
        // 本地皮肤排在后面，且不会被误标内置
        std::fs::write(paths.skins.join("自定义.png"), png_bytes(64, 32)).unwrap();
        let items = list_skins(&paths);
        assert_eq!(items.len(), 3);
        assert!(!items[2].builtin);
        let _ = std::fs::remove_dir_all(&paths.root);
    }

#[test]
    fn textures_minecraft_net_urls_are_allowed() {
        assert!(crate::settings::is_http_url(
            "https://textures.minecraft.net/texture/292009a49bba793b6e2b4a1a8bb0f7c8d3a2f4b1"
        ));
        assert!(crate::settings::is_http_url("http://textures.minecraft.net/texture/abc"));
        assert!(crate::settings::is_http_url("https://mod.mcimirror.top/curseforge/files/1/a.png"));
        assert!(crate::settings::is_http_url("https://littleskin.cn/textures/abc"));
        assert!(!crate::settings::is_http_url("file:///C:/x.png"));
        assert!(!crate::settings::is_http_url("ftp://textures.minecraft.net/x.png"));
        assert_eq!(SKIN_DOWNLOAD_USER_AGENT, "Mozilla/5.0 GuguLauncher/1.0");
    }

    #[test]
    fn import_file_name_is_sanitized() {
        assert_eq!(skin_file_name_for(Some("my skin")), "my skin.png");
        assert_eq!(skin_file_name_for(Some("a/b\\c")), "a_b_c.png");
        assert_eq!(skin_file_name_for(Some("  ")), "skin.png");
        assert_eq!(skin_file_name_for(None), "skin.png");
        assert!(!skin_file_name_for(Some("名字")).is_empty());
        assert!(skin_file_name_for(Some("x.png")).ends_with(".png"));
        assert!(!skin_file_name_for(Some("x.png")).ends_with(".png.png"));
    }

    #[test]
    fn small_official_skin_textures_are_accepted() {
        assert_eq!(validate_skin_png(&png_bytes(64, 32)).unwrap(), (64, 32));
        assert_eq!(validate_skin_png(&png_bytes(64, 64)).unwrap(), (64, 64));
        assert!(png_bytes(64, 64).len() < 8 * 1024, "官方贴图 1-2KB 也要接受");
    }

    #[test]
    fn non_png_import_payload_is_rejected() {
        let error = validate_skin_png(b"<html>403 Forbidden</html>")
            .unwrap_err()
            .to_string();
        assert!(error.contains("不是合法的 PNG"), "{error}");
        let error = validate_skin_png(&png_bytes(32, 32)).unwrap_err().to_string();
        assert!(error.contains("64x64") || error.contains("尺寸"), "{error}");
    }

    #[test]
    fn delete_local_skin_removes_file() {
        let paths = temp_paths("skin-delete");
        std::fs::write(paths.skins.join("自定义.png"), png_bytes(64, 64)).unwrap();
        assert!(delete_local_skin(&paths, " 自定义.png ").is_ok());
        assert!(!paths.skins.join("自定义.png").exists());
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn delete_local_skin_rejects_path_traversal() {
        let paths = temp_paths("skin-delete-trav");
        for bad in ["../x.png", "a\\b.png", "sub/dir.png", "C:\\Windows\\x.png", "..\\x.png"] {
            let error = delete_local_skin(&paths, bad).unwrap_err().to_string();
            assert!(error.contains("皮肤名不合法"), "{bad} => {error}");
        }
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn delete_local_skin_rejects_non_png() {
        let paths = temp_paths("skin-delete-ext");
        std::fs::write(paths.skins.join("readme.txt"), b"x").unwrap();
        let error = delete_local_skin(&paths, "readme.txt").unwrap_err().to_string();
        assert!(error.contains("只能删除 png 皮肤"), "{error}");
        assert!(paths.skins.join("readme.txt").exists());
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn delete_local_skin_missing_file_message() {
        let paths = temp_paths("skin-delete-missing");
        let error = delete_local_skin(&paths, "ghost.png").unwrap_err().to_string();
        assert!(error.contains("该皮肤不存在（可能已被删除）"), "{error}");
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn deleting_current_skin_falls_back_to_builtin_steve() {
        let paths = temp_paths("skin-delete-current");
        let file = paths.skins.join("mine.png");
        std::fs::write(&file, png_bytes(64, 64)).unwrap();
        let applied = apply_skin(&paths, &file.display().to_string()).unwrap();
        assert!(applied.is_current);
        assert_eq!(
            current_skin(&paths).map(|item| item.name),
            Some("mine.png".to_string())
        );
        delete_local_skin(&paths, "mine.png").unwrap();
        let current = current_skin(&paths).expect("永远不为 None");
        assert_eq!(current.name, "Steve");
        assert!(current.builtin);
        assert!(!paths.skins.join("mine.png").exists());
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn nine_bundled_characters_read_in_both_models() {
        assert_eq!(BUILTIN_CHARACTERS.len(), 9);
        assert_eq!(BUILTIN_SKINS.len(), 9);
        let mut seen = Vec::new();
        for key in BUILTIN_CHARACTERS {
            for model in ["classic", "slim"] {
                let (item, sha256) = default_skin_named(Some(key), model).unwrap();
                assert!(item.builtin, "{key}/{model} 必须标记为内置");
                let expected = if model == "slim" { "slim" } else { "wide" };
                assert_eq!(item.path, format!("builtin://{key}-{expected}"));
                assert_eq!(
                    item.data_url
                        .as_deref()
                        .map(|url| url.starts_with("data:image/png;base64,")),
                    Some(true)
                );
                // 每张都是真的 64×64 PNG，且 sha256 与文件字节一致
                let bytes = builtin_skin(key, model).unwrap().1;
                assert_eq!(validate_skin_png(bytes).unwrap(), (64, 64));
                assert_eq!(sha256, util::hex(&util::sha256(bytes)));
                assert_eq!(item.size as usize, bytes.len());
                seen.push(sha256);
            }
        }
        // 18 张互不相同（每位角色 wide/slim 也不同）
        let unique: std::collections::HashSet<&String> = seen.iter().collect();
        assert_eq!(unique.len(), 18);
        assert_eq!(default_skin_named(Some("Ari"), "classic").unwrap().0.name, "Ari");
        assert_eq!(default_skin_named(Some("alex"), "slim").unwrap().0.name, "Alex");
        assert!(default_skin_named(Some("nobody"), "classic").is_err());
    }

    /// 32 位十六进制（不带横线，不是 UUID 形态）。
    fn sample_uuid_plain() -> String {
        "0123456789abcdef0123456789abcdef".to_string()
    }

    /// 带横线的 UUID 形态：运行时拼，源码里不留整串。
    fn sample_uuid() -> String {
        format!("{}-{}-{}-{}-{}", "01234567", "89ab", "cdef", "0123", "456789abcdef")
    }

    #[test]
    fn userinfo_response_parsing() {
        let json = format!(
            r#"{{"username":"Player","uuid":"{}","skin_url":"http://textures.minecraft.net/texture/292009a4"}}"#,
            sample_uuid_plain()
        );
        let info = parse_userinfo(&json).unwrap();
        assert_eq!(info.username, "Player");
        assert_eq!(info.uuid, sample_uuid());
        assert_eq!(
            info.skin_url.as_deref(),
            Some("https://textures.minecraft.net/texture/292009a4"),
            "http 要升级成 https"
        );

        // 没有 skin_url
        let no_skin = parse_userinfo(r#"{"username":"X","uuid":"abcdabcdabcdabcdabcdabcdabcdabcd"}"#).unwrap();
        assert!(no_skin.skin_url.is_none());
        // 400 语义
        let err = parse_userinfo(r#"{"error":"username query parameter is required"}"#)
            .unwrap_err()
            .to_string();
        assert!(err.contains("玩家名不能为空"), "{err}");
        assert!(parse_userinfo("not json").is_err());
    }

    #[test]
    fn uuid_and_https_normalization() {
        assert_eq!(normalize_uuid(&sample_uuid_plain()), sample_uuid());
        assert_eq!(normalize_uuid(&sample_uuid()), sample_uuid());
        assert_eq!(upgrade_to_https("http://a/b"), "https://a/b");
        assert_eq!(upgrade_to_https("https://a/b"), "https://a/b");
        assert_eq!(upgrade_to_https("//a/b"), "//a/b");
    }

    #[test]
    fn userinfo_error_mapping() {
        assert_eq!(map_userinfo_error(Some(400), ""), "玩家名不能为空");
        assert_eq!(map_userinfo_error(Some(404), ""), "没有这个正版玩家");
        assert!(map_userinfo_error(Some(500), "").contains("暂时不可用"));
        assert!(map_userinfo_error(Some(418), "").contains("HTTP 418"));
        assert!(map_userinfo_error(None, "operation timed out").contains("超时"));
        assert!(map_userinfo_error(None, "dns error").contains("无法连接皮肤服务"));
    }

    #[test]
    fn name_encoding_handles_chinese_and_spaces() {
        assert_eq!(encode_name("Notch"), "Notch");
        assert_eq!(encode_name("a b"), "a%20b");
        assert_eq!(encode_name("小明"), "%E5%B0%8F%E6%98%8E");
    }

    #[tokio::test]
    async fn fetch_mojang_skin_requires_name() {
        let paths = temp_paths("mojang-empty");
        let client = reqwest::Client::new();
        let err = fetch_mojang_skin(&client, &paths, "   ").await.unwrap_err();
        assert!(err.to_string().contains("玩家名不能为空"));
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn thirdparty_server_normalization() {
        assert_eq!(
            normalize_server("https://littleskin.cn/api/yggdrasil/").unwrap(),
            "https://littleskin.cn/api/yggdrasil"
        );
        assert!(normalize_server("").is_err());
        assert!(normalize_server("littleskin.cn").is_err());
        assert!(normalize_server("ftp://x/y").is_err());
    }

    #[test]
    fn thirdparty_by_name_response_parsing() {
        let json = r#"{
            "id": "12345678123412341234123456789abc",
            "name": "Gugu",
            "skins": [
                {"id": "s1", "state": "ACTIVE", "url": "/textures/abc123", "variant": "slim"},
                {"id": "s2", "state": "ACTIVE", "url": "https://textures.minecraft.net/texture/zzz"}
            ]
        }"#;
        let skin = parse_thirdparty_profile(json, "https://littleskin.cn/api/yggdrasil").unwrap();
        assert_eq!(skin.name, "Gugu");
        assert_eq!(
            skin.skin_url.as_deref(),
            Some("https://littleskin.cn/textures/abc123"),
            "相对路径要补成皮肤站绝对地址"
        );
        assert_eq!(skin.model.as_deref(), Some("slim"));
        assert!(skin.cape_url.is_none());
    }

    #[test]
    fn thirdparty_textures_base64_parsing() {
        let textures = r#"{"textures":{"SKIN":{"url":"http://textures.minecraft.net/texture/s1","metadata":{"model":"slim"}},"CAPE":{"url":"http://textures.minecraft.net/texture/c1"}}}"#;
        let encoded = util::base64_encode(textures.as_bytes());
        let json = format!(
            r#"{{"id":"abcdabcdabcdabcdabcdabcdabcdabcd","name":"Alex","properties":[{{"name":"textures","value":"{encoded}"}}]}}"#
        );
        let skin = parse_thirdparty_profile(&json, "https://littleskin.cn/api/yggdrasil").unwrap();
        assert_eq!(
            skin.skin_url.as_deref(),
            Some("http://textures.minecraft.net/texture/s1")
        );
        assert_eq!(
            skin.cape_url.as_deref(),
            Some("http://textures.minecraft.net/texture/c1")
        );
        assert_eq!(skin.model.as_deref(), Some("slim"));

        // 直接解 textures：非 base64 / 非 JSON 都要安全返回 None
        assert_eq!(parse_textures_value("!!!!"), (None, None, None));
        assert_eq!(
            parse_textures_value(&util::base64_encode(b"not json")),
            (None, None, None)
        );
    }

    #[test]
    fn thirdparty_no_skin_and_missing_player() {
        let no_skin = r#"{"id":"abcdabcdabcdabcdabcdabcdabcdabcd","name":"NoSkin","skins":[]}"#;
        let err = parse_thirdparty_profile(no_skin, "https://littleskin.cn/api/yggdrasil")
            .unwrap_err()
            .to_string();
        assert!(err.contains("没有设置皮肤"), "{err}");

        let err = parse_thirdparty_profile(r#"{"error":"NotFound","errorMessage":"player not found"}"#, "https://x/api")
            .unwrap_err()
            .to_string();
        assert!(err.contains("player not found"), "{err}");
    }

    #[test]
    fn thirdparty_error_mapping() {
        let server = "https://littleskin.cn/api/yggdrasil";
        assert_eq!(map_lookup_error(Some(404), "", server), "该玩家不存在");
        assert!(map_lookup_error(Some(401), "", server).contains("拒绝"));
        assert!(map_lookup_error(Some(500), "", server).contains("HTTP 500"));
        assert!(map_lookup_error(None, "operation timed out", server).contains("无响应"));
        assert!(map_lookup_error(None, "dns error", server).contains("无法连接皮肤站"));
    }

    #[test]
    fn texture_url_absolutization() {
        assert_eq!(
            absolute_texture_url("textures/abc", "https://littleskin.cn/api/yggdrasil"),
            "https://textures.minecraft.net/textures/abc"
        );
        assert_eq!(
            absolute_texture_url("//cdn.example.com/a.png", "https://x/api"),
            "https://cdn.example.com/a.png"
        );
        assert_eq!(
            absolute_texture_url("/textures/a.png", "https://littleskin.cn/api/yggdrasil"),
            "https://littleskin.cn/textures/a.png"
        );
        assert_eq!(
            absolute_texture_url("https://cdn/a.png?x=1", "https://y"),
            "https://cdn/a.png?x=1"
        );
    }

    #[tokio::test]
    async fn import_from_url_validates_and_dedupes_names() {
        let paths = temp_paths("import");
        // 用本地 HTTP 服务太麻烦：直接起一个最小服务返回 PNG 字节
        let png = png_bytes(64, 64);
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            for _ in 0..3 {
                let Ok((mut socket, _)) = listener.accept().await else { break };
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let mut buffer = [0u8; 1024];
                let _ = socket.read(&mut buffer).await;
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    png.len()
                );
                let _ = socket.write_all(header.as_bytes()).await;
                let _ = socket.write_all(&png).await;
                let _ = socket.flush().await;
            }
        });

        let client = reqwest::Client::new();
        let url = format!("http://127.0.0.1:{}/skin.png", addr.port());
        let first = import_from_url(&client, &paths, &url, Some("我的皮肤")).await.unwrap();
        assert_eq!(first.item.name, "我的皮肤.png");
        assert!(!first.item.is_current, "导入不自动应用");
        assert!(!first.reused, "第一次是新导入");
        assert!(first
            .item
            .data_url
            .as_ref()
            .map(|value| value.starts_with("data:image/png"))
            .unwrap_or(false));
        // 同一张图（同 sha256）再导入一次：复用既有文件，不新增、不改名
        let second = import_from_url(&client, &paths, &url, Some("我的皮肤")).await.unwrap();
        assert!(second.reused, "重复导入要复用");
        assert_eq!(second.item.name, "我的皮肤.png", "名字保持不变");
        assert_eq!(
            list_skins(&paths).len(),
            3,
            "内置 Steve/Alex + 一个导入皮肤（没有 (1)）"
        );

        assert!(import_from_url(&client, &paths, "ftp://x/y.png", None).await.is_err());
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn mojang_profile_parsing() {
        let texture_json = r#"{"textures":{"SKIN":{"url":"http://textures.minecraft.net/texture/aaa"},"CAPE":{"url":"http://textures.minecraft.net/texture/bbb"}}}"#;
        let encoded = util::base64_encode(texture_json.as_bytes());
        let profile = format!(
            r#"{{"id":"abc","name":"Steve","properties":[{{"name":"textures","value":"{encoded}"}}]}}"#
        );
        let (skin, cape) = parse_mojang_profile(&profile).unwrap();
        assert_eq!(skin.as_deref(), Some("http://textures.minecraft.net/texture/aaa"));
        assert_eq!(cape.as_deref(), Some("http://textures.minecraft.net/texture/bbb"));

        // 只有皮肤没有披风
        let skin_only = r#"{"textures":{"SKIN":{"url":"http://x/s"}}}"#;
        let profile = format!(
            r#"{{"properties":[{{"name":"textures","value":"{}"}}]}}"#,
            util::base64_encode(skin_only.as_bytes())
        );
        let (skin, cape) = parse_mojang_profile(&profile).unwrap();
        assert_eq!(skin.as_deref(), Some("http://x/s"));
        assert!(cape.is_none());

        // 错误响应要可读
        let err = parse_mojang_profile(r#"{"error":"NOT_FOUND","errorMessage":"player not found"}"#)
            .unwrap_err()
            .to_string();
        assert!(err.contains("player not found"));
        assert!(parse_mojang_profile(r#"{"properties":[]}"#).is_err());
    }

    #[test]
    fn current_skin_is_persisted_after_apply() {
        let paths = temp_paths("current-apply");
        let source = paths.root.join("incoming.png");
        std::fs::write(&source, png_bytes(64, 64)).unwrap();
        apply_skin(&paths, &source.display().to_string()).unwrap();
        let current = current_skin(&paths).unwrap();
        assert_eq!(current.name, "incoming.png");
        assert!(current.exists);
        assert!(current.is_current);
        assert!(!current.builtin);
        assert_eq!(current.source.as_deref(), Some("local"));
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn current_skin_is_persisted_after_upload() {
        let paths = temp_paths("current-upload");
        let base64 = util::base64_encode(&png_bytes(64, 64));
        let uploaded = upload_skin(&paths, "我的皮肤", &base64).unwrap();
        let current = current_skin(&paths).unwrap();
        assert_eq!(current.name, uploaded.name);
        assert!(current.exists);
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn current_skin_is_persisted_after_import() {
        let paths = temp_paths("current-import");
        std::fs::create_dir_all(&paths.skins).unwrap();
        let file = paths.skins.join("from-url.png");
        std::fs::write(&file, png_bytes(64, 32)).unwrap();
        apply_skin(&paths, &file.display().to_string()).unwrap();
        let current = current_skin(&paths).unwrap();
        assert_eq!(current.name, "from-url.png");
        assert!(current.exists);
        assert_eq!(
            validate_skin_png(&std::fs::read(&file).unwrap()).unwrap(),
            (64, 32)
        );
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn missing_current_file_reports_exists_false() {
        let paths = temp_paths("current-missing");
        let file = paths.skins.join("gone.png");
        std::fs::write(&file, png_bytes(64, 64)).unwrap();
        apply_skin(&paths, &file.display().to_string()).unwrap();
        std::fs::remove_file(&file).unwrap();
        let current = current_skin(&paths).expect("不能偷偷回落 Steve");
        assert_eq!(current.name, "gone.png", "名字要保留");
        assert!(!current.exists, "exists 必须是 false");
        assert!(current.data_url.is_none());
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn builtin_fallback_has_exists_true() {
        let paths = temp_paths("current-builtin");
        let current = current_skin(&paths).unwrap();
        assert_eq!(current.name, "Steve");
        assert!(current.builtin);
        assert!(current.exists);
        let _ = std::fs::remove_dir_all(&paths.root);
    }
}
