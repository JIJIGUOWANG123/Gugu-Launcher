//! 内容服务：Modrinth / CurseForge / MCIM 的地址映射、搜索与详情解析。
//!
//! MCIM 兼容端点沿用旧项目规则：
//!   /modrinth/v2/...   搜 Modrinth
//!   /curseforge/v1/... 搜 CurseForge
//!   文件 CDN：/modrinth/data/<项目>/<版本>/<文件名>、/curseforge/files/<文件 id>/<文件名>

use serde::Serialize;

use crate::error::{AppError, Result};

pub const MODRINTH_API: &str = "https://api.modrinth.com/v2";
pub const CURSEFORGE_API: &str = "https://api.curseforge.com/v1";
pub const MCIM_BASE: &str = "https://mod.mcimirror.top";
pub const CURSEFORGE_GAME_ID: u32 = 432;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Modrinth,
    Curseforge,
    Mcim,
}

impl Provider {
    pub fn from_id(id: &str) -> Result<Self> {
        match id {
            "modrinth" => Ok(Provider::Modrinth),
            "curseforge" => Ok(Provider::Curseforge),
            "mcim" => Ok(Provider::Mcim),
            other => Err(AppError::Invalid(format!("不支持的内容源：{other}"))),
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            Provider::Modrinth => "modrinth",
            Provider::Curseforge => "curseforge",
            Provider::Mcim => "mcim",
        }
    }

    /// 搜 Modrinth 的接口（mcim 走镜像）。
    fn modrinth_root(&self) -> String {
        match self {
            Provider::Mcim => format!("{MCIM_BASE}/modrinth/v2"),
            _ => MODRINTH_API.to_string(),
        }
    }

    pub fn curseforge_root(&self) -> String {
        match self {
            Provider::Mcim => format!("{MCIM_BASE}/curseforge/v1"),
            _ => CURSEFORGE_API.to_string(),
        }
    }
}

/// 内容类型 → (Modrinth project_type, CurseForge classId)。
///
/// plugin 的 classId=5（Bukkit Plugins）是实测确认的：
/// GET /v1/categories?gameId=432 返回 id=5 name="Bukkit Plugins"（官方与 MCIM 镜像一致）。
pub fn kind_mapping(kind: &str) -> Result<(&'static str, u32)> {
    match kind {
        "mod" => Ok(("mod", 6)),
        // CurseForge 的 Modpacks classId 是 4471
        "modpack" => Ok(("modpack", 4471)),
        "resourcepack" => Ok(("resourcepack", 12)),
        "shader" => Ok(("shader", 6552)),
        "plugin" => Ok(("plugin", 5)),
        // 数据包只有 Modrinth/MCIM 支持；classId 0 表示「CurseForge 没有对应分类」
        "datapack" => Ok(("datapack", 0)),
        other => Err(AppError::Invalid(format!("不支持的内容类型：{other}"))),
    }
}

/// 插件加载器：Modrinth 用 categories 过滤，CurseForge 没有对应的 modLoaderType。
pub const PLUGIN_LOADERS: [&str; 7] = [
    "bukkit",
    "spigot",
    "paper",
    "folia",
    "purpur",
    "sponge",
    "pufferfish",
];

/// 排序档位（前端只给这四个）。
pub const SORT_IDS: [&str; 4] = ["relevance", "downloads", "updated", "newest"];

/// 排序归一：不认识的按 relevance。
pub fn normalize_sort(sort: Option<&str>) -> &'static str {
    match sort.map(str::trim).map(str::to_lowercase).as_deref() {
        Some("downloads") => "downloads",
        Some("updated") => "updated",
        Some("newest") => "newest",
        _ => "relevance",
    }
}

/// Modrinth 的 index 参数（与前端档位同名，保留映射函数便于单测）。
pub fn modrinth_index(sort: &str) -> &'static str {
    match sort {
        "downloads" => "downloads",
        "updated" => "updated",
        "newest" => "newest",
        _ => "relevance",
    }
}

/// CurseForge 的 sortField：2=Popularity 3=LastUpdated 6=TotalDownloads 11=ReleasedDate。
pub fn curseforge_sort_field(sort: &str) -> u32 {
    match sort {
        "downloads" => 6,
        "updated" => 3,
        "newest" => 11,
        _ => 2,
    }
}

/// CurseForge 的 modLoaderType：1=Forge 4=Fabric 5=Quilt 6=NeoForge。
pub fn curseforge_loader_type(loader: &str) -> Option<u32> {
    match loader.trim().to_lowercase().as_str() {
        "forge" => Some(1),
        "fabric" => Some(4),
        "quilt" => Some(5),
        "neoforge" => Some(6),
        _ => None,
    }
}

/// 该来源是否支持这种内容类型（CurseForge 没有 datapack 分类）。
pub fn kind_supported(provider: Provider, kind: &str) -> bool {
    let Ok((_, class_id)) = kind_mapping(kind) else {
        return false;
    };
    !(provider == Provider::Curseforge && class_id == 0)
}

/// 每页条数只允许 20/40/60（默认 20，非法值就近回落）。
pub const PAGE_SIZES: [u32; 3] = [20, 40, 60];
pub const DEFAULT_PAGE_SIZE: u32 = 20;
/// 每页最多翻多少条（避免翻译风暴）。
pub const TRANSLATE_PAGE_LIMIT: usize = 20;

pub fn clamp_page_size(value: Option<u32>) -> u32 {
    match value {
        Some(size) if PAGE_SIZES.contains(&size) => size,
        Some(size) => PAGE_SIZES
            .iter()
            .copied()
            .min_by_key(|allowed| allowed.abs_diff(size))
            .unwrap_or(DEFAULT_PAGE_SIZE),
        None => DEFAULT_PAGE_SIZE,
    }
}

/// 页码（从 1 开始）→ offset。
pub fn page_offset(page: u32, page_size: u32) -> u32 {
    page.saturating_sub(1).saturating_mul(page_size)
}

fn url_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            b' ' => out.push_str("%20"),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// 一次搜索的全部筛选条件。
#[derive(Debug, Clone)]
pub struct SearchQuery<'a> {
    pub provider: Provider,
    pub kind: &'a str,
    pub query: &'a str,
    pub mc_version: Option<&'a str>,
    pub loader: Option<&'a str>,
    pub categories: &'a [String],
    pub sort: Option<&'a str>,
    /// CurseForge 的 categoryIds（由 categories 的名字查表得到）
    pub category_ids: &'a [u32],
    pub offset: u32,
    pub limit: u32,
}

/// Modrinth facets：组之间 AND，组内数组 OR。
pub fn modrinth_facets(
    project_type: &str,
    mc_version: Option<&str>,
    loader: Option<&str>,
    categories: &[String],
) -> Vec<String> {
    fn group(values: &[String]) -> String {
        let items: Vec<String> = values
            .iter()
            .map(|value| format!("\"{}\"", value.replace('"', "")))
            .collect();
        format!("[{}]", items.join(","))
    }
    let mut facets = vec![group(&[format!("project_type:{project_type}")])];
    if let Some(version) = mc_version.map(str::trim).filter(|value| !value.is_empty()) {
        facets.push(group(&[format!("versions:{version}")]));
    }
    if let Some(loader) = loader.map(str::trim).filter(|value| !value.is_empty()) {
        facets.push(group(&[format!("categories:{}", loader.to_lowercase())]));
    }
    let picks: Vec<String> = categories
        .iter()
        .map(|value| value.trim().to_lowercase())
        .filter(|value| !value.is_empty())
        .map(|value| format!("categories:{value}"))
        .collect();
    if !picks.is_empty() {
        facets.push(group(&picks));
    }
    facets
}

/// 搜索地址。curseforge 走官方接口时要额外带 x-api-key（由命令层加头）。
pub fn search_url_with(query: &SearchQuery<'_>) -> Result<String> {
    let (project_type, class_id) = kind_mapping(query.kind)?;
    let limit = query.limit.clamp(1, 50);
    let sort = normalize_sort(query.sort);
    match query.provider {
        Provider::Modrinth | Provider::Mcim => {
            let facets = modrinth_facets(
                project_type,
                query.mc_version,
                query.loader,
                query.categories,
            );
            Ok(format!(
                "{}/search?query={}&limit={}&offset={}&index={}&facets={}",
                query.provider.modrinth_root(),
                url_encode(query.query),
                limit,
                query.offset,
                modrinth_index(sort),
                url_encode(&format!("[{}]", facets.join(",")))
            ))
        }
        Provider::Curseforge => {
            let mut url = format!(
                "{}/mods/search?gameId={CURSEFORGE_GAME_ID}&classId={class_id}&searchFilter={}&index={}&pageSize={}&sortField={}&sortOrder=desc",
                query.provider.curseforge_root(),
                url_encode(query.query),
                query.offset,
                limit,
                curseforge_sort_field(sort)
            );
            if let Some(version) = query.mc_version.map(str::trim).filter(|value| !value.is_empty()) {
                url.push_str(&format!("&gameVersion={}", url_encode(version)));
            }
            if let Some(loader) = query.loader.map(str::trim).filter(|value| !value.is_empty()) {
                if let Some(loader_type) = curseforge_loader_type(loader) {
                    url.push_str(&format!("&modLoaderType={loader_type}"));
                }
            }
            if !query.category_ids.is_empty() {
                let ids: Vec<String> = query.category_ids.iter().map(u32::to_string).collect();
                url.push_str(&format!("&categoryIds={}", ids.join(",")));
            }
            Ok(url)
        }
    }
}

/// 老签名（不带 categories/sort）的薄封装：等价于空筛选。
pub fn search_url(
    provider: Provider,
    kind: &str,
    query: &str,
    mc_version: Option<&str>,
    loader: Option<&str>,
    offset: u32,
    limit: u32,
) -> Result<String> {
    search_url_with(&SearchQuery {
        provider,
        kind,
        query,
        mc_version,
        loader,
        categories: &[],
        sort: None,
        category_ids: &[],
        offset,
        limit,
    })
}

// ------------------------------------------------------------------ 无 Key 走镜像

/// MCIM 的 CurseForge 通道（官方接口在没有 API Key 时的兜底）。
pub fn mcim_curseforge_root() -> String {
    format!("{MCIM_BASE}/curseforge/v1")
}

/// 官方 CurseForge 接口必须有 Key；没有 Key 就改走 MCIM 镜像。
pub fn effective_provider(provider: Provider, has_key: bool) -> Provider {
    match provider {
        Provider::Curseforge if !has_key => Provider::Mcim,
        other => other,
    }
}

/// CurseForge 实际要用的根地址：有 Key 用官方，没 Key 用镜像。
pub fn curseforge_root_for(provider: Provider, has_key: bool) -> String {
    match provider {
        Provider::Curseforge if !has_key => mcim_curseforge_root(),
        other => other.curseforge_root(),
    }
}

/// 与 search_url_with 相同，但 CurseForge 可以走镜像通道（无 Key 时）。
pub fn search_url_for(query: &SearchQuery<'_>, has_key: bool) -> Result<String> {
    let (_, class_id) = kind_mapping(query.kind)?;
    let limit = query.limit.clamp(1, 50);
    let sort = normalize_sort(query.sort);
    match query.provider {
        Provider::Modrinth | Provider::Mcim => search_url_with(query),
        Provider::Curseforge => {
            let mut url = format!(
                "{}/mods/search?gameId={CURSEFORGE_GAME_ID}&classId={class_id}&searchFilter={}&index={}&pageSize={}&sortField={}&sortOrder=desc",
                curseforge_root_for(query.provider, has_key),
                url_encode(query.query),
                query.offset,
                limit,
                curseforge_sort_field(sort)
            );
            if let Some(version) = query.mc_version.map(str::trim).filter(|value| !value.is_empty()) {
                url.push_str(&format!("&gameVersion={}", url_encode(version)));
            }
            if let Some(loader) = query.loader.map(str::trim).filter(|value| !value.is_empty()) {
                if let Some(loader_type) = curseforge_loader_type(loader) {
                    url.push_str(&format!("&modLoaderType={loader_type}"));
                }
            }
            if !query.category_ids.is_empty() {
                let ids: Vec<String> = query.category_ids.iter().map(u32::to_string).collect();
                url.push_str(&format!("&categoryIds={}", ids.join(",")));
            }
            Ok(url)
        }
    }
}

/// 详情地址（CurseForge 无 Key 时走镜像）。
pub fn detail_url_for(provider: Provider, has_key: bool, project_id: &str) -> Result<String> {
    if project_id.trim().is_empty() {
        return Err(AppError::Invalid("项目 id 不能为空".to_string()));
    }
    Ok(match provider {
        Provider::Modrinth | Provider::Mcim => format!(
            "{}/project/{}",
            provider.modrinth_root(),
            url_encode(project_id.trim())
        ),
        Provider::Curseforge => format!(
            "{}/mods/{}",
            curseforge_root_for(provider, has_key),
            url_encode(project_id.trim())
        ),
    })
}

/// 版本列表地址（CurseForge 无 Key 时走镜像）。
pub fn versions_url_for(
    provider: Provider,
    has_key: bool,
    project_id: &str,
    mc_version: Option<&str>,
    loader: Option<&str>,
) -> Result<String> {
    if project_id.trim().is_empty() {
        return Err(AppError::Invalid("项目 id 不能为空".to_string()));
    }
    match provider {
        Provider::Modrinth | Provider::Mcim => versions_url(provider, project_id, mc_version, loader),
        Provider::Curseforge => Ok(format!(
            "{}/mods/{}/files?pageSize=50",
            curseforge_root_for(provider, has_key),
            url_encode(project_id.trim())
        )),
    }
}

// ------------------------------------------------------------------ 下载通道

/// 把原始下载地址改写成 MCIM 镜像地址；改写不了返回 None。
pub fn mirror_download_url(url: &str) -> Option<String> {
    let url = url.trim();
    if url.is_empty() || url.contains("mod.mcimirror.top") {
        return None;
    }
    // Modrinth CDN：cdn.modrinth.com/data/<项目>/<版本>/<文件>
    if let Some(rest) = url
        .strip_prefix("https://cdn.modrinth.com/")
        .or_else(|| url.strip_prefix("http://cdn.modrinth.com/"))
    {
        return Some(format!("{MCIM_BASE}/modrinth/{rest}"));
    }
    // CurseForge CDN：edge/media.forgecdn.net/files/<head>/<tail4>/<文件> → 还原成文件 id
    for host in [
        "https://edge.forgecdn.net/files/",
        "http://edge.forgecdn.net/files/",
        "https://media.forgecdn.net/files/",
        "http://media.forgecdn.net/files/",
        "https://beta.curseforge.com/files/",
    ] {
        if let Some(rest) = url.strip_prefix(host) {
            let mut parts = rest.split('/');
            let (head, tail, file) = (parts.next()?, parts.next()?, parts.next()?);
            if head.is_empty() || tail.len() != 4 || file.is_empty() {
                return None;
            }
            let id: u64 = format!("{head}{tail}").parse().ok()?;
            return Some(format!(
                "{MCIM_BASE}/curseforge/files/{id}/{}",
                url_encode(file)
            ));
        }
    }
    None
}

/// 关掉翻译：标题/简介回落到原文，translated 恒为 false。
pub fn disable_translation(items: &mut [ContentItem]) {
    for item in items.iter_mut() {
        item.title = item.title_original.clone();
        item.summary = item.summary_original.clone();
        item.translated = false;
    }
}

/// 按设置选下载地址：via_mcim=true 时优先镜像。
pub fn preferred_download_url(url: &str, via_mcim: bool) -> Option<String> {
    if !via_mcim {
        return None;
    }
    mirror_download_url(url)
}

/// 下载一个文件内容：先试镜像（成功就直接用），失败（404/超时/网络错）回退原始地址。
pub async fn download_bytes(
    client: &reqwest::Client,
    url: &str,
    via_mcim: bool,
) -> Result<Vec<u8>> {
    let mirror = preferred_download_url(url, via_mcim);
    let mut last_error = String::new();
    if let Some(mirror) = mirror.as_deref() {
        match get_bytes(client, mirror).await {
            Ok(bytes) => {
                tracing::info!(url = %mirror, bytes = bytes.len(), "内容下载走 MCIM 镜像");
                return Ok(bytes);
            }
            Err(err) => {
                last_error = err.to_string();
                tracing::warn!(url = %mirror, %err, "MCIM 镜像下载失败，回退原始地址");
            }
        }
    }
    match get_bytes(client, url).await {
        Ok(bytes) => Ok(bytes),
        Err(err) => {
            if last_error.is_empty() {
                Err(err)
            } else {
                Err(AppError::Net(format!("{err}（镜像也不可用：{last_error}）")))
            }
        }
    }
}

async fn get_bytes(client: &reqwest::Client, url: &str) -> Result<Vec<u8>> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|err| AppError::Net(format!("下载失败：{err}")))?;
    if !response.status().is_success() {
        return Err(AppError::Net(format!(
            "下载失败：HTTP {}",
            response.status().as_u16()
        )));
    }
    response
        .bytes()
        .await
        .map(|bytes| bytes.to_vec())
        .map_err(|err| AppError::Net(format!("读取下载内容失败：{err}")))
}

// ------------------------------------------------------------------ CurseForge 分类缓存

/// 分类缓存有效期 24 小时。
pub const CATEGORY_TTL_SECS: u64 = 24 * 3600;

/// name/slug（小写）→ 分类 id。
pub type CategoryIds = std::collections::HashMap<String, u32>;

static CATEGORY_CACHE: std::sync::OnceLock<std::sync::Mutex<Option<(u64, CategoryIds)>>> =
    std::sync::OnceLock::new();

fn category_cache() -> &'static std::sync::Mutex<Option<(u64, CategoryIds)>> {
    CATEGORY_CACHE.get_or_init(|| std::sync::Mutex::new(None))
}

pub fn category_cache_is_fresh(fetched_at: u64, now: u64) -> bool {
    now.saturating_sub(fetched_at) < CATEGORY_TTL_SECS
}

/// /v1/categories?gameId=432 的地址。
pub fn categories_url(provider: Provider) -> String {
    format!(
        "{}/categories?gameId={CURSEFORGE_GAME_ID}",
        provider.curseforge_root()
    )
}

/// 解析分类响应：name 与 slug 都建索引（都小写）。
pub fn parse_categories(json: &str) -> Result<CategoryIds> {
    let value: serde_json::Value = serde_json::from_str(json)
        .map_err(|err| AppError::Net(format!("分类列表格式不对：{err}")))?;
    let data = value
        .get("data")
        .and_then(|item| item.as_array())
        .ok_or_else(|| AppError::Net("分类列表缺少 data".to_string()))?;
    let mut ids = CategoryIds::new();
    for entry in data {
        let Some(id) = entry.get("id").and_then(|item| item.as_u64()) else {
            continue;
        };
        for key in ["name", "slug"] {
            if let Some(text) = entry.get(key).and_then(|item| item.as_str()) {
                let trimmed = text.trim().to_lowercase();
                if !trimmed.is_empty() {
                    ids.insert(trimmed, id as u32);
                }
            }
        }
    }
    Ok(ids)
}

/// 从缓存里查名字 → id（缓存过期/缺失返回 None）。
pub fn cached_category_ids(names: &[String], now: u64) -> Option<Vec<u32>> {
    let guard = category_cache().lock().ok()?;
    let (fetched_at, ids) = guard.as_ref()?;
    if !category_cache_is_fresh(*fetched_at, now) {
        return None;
    }
    Some(lookup_category_ids(ids, names))
}

fn lookup_category_ids(ids: &CategoryIds, names: &[String]) -> Vec<u32> {
    let mut out = Vec::new();
    for name in names {
        let key = name.trim().to_lowercase();
        match ids.get(&key) {
            Some(id) => out.push(*id),
            None => tracing::warn!(category = %name, "CurseForge 分类名没查到 id，忽略"),
        }
    }
    out
}

/// 把名字换成 CurseForge 的 categoryIds（缓存 24h，过期重新拉一次）。
pub async fn resolve_category_ids(
    client: &reqwest::Client,
    provider: Provider,
    api_key: Option<&str>,
    names: &[String],
) -> Result<Vec<u32>> {
    if names.is_empty() {
        return Ok(Vec::new());
    }
    let now = crate::util::now_unix();
    if let Some(ids) = cached_category_ids(names, now) {
        return Ok(ids);
    }
    let text = http_text(client, &categories_url(provider), api_key).await?;
    let ids = parse_categories(&text)?;
    let resolved = lookup_category_ids(&ids, names);
    if let Ok(mut guard) = category_cache().lock() {
        *guard = Some((now, ids));
    }
    Ok(resolved)
}

pub fn detail_url(provider: Provider, project_id: &str) -> Result<String> {
    if project_id.trim().is_empty() {
        return Err(AppError::Invalid("项目 id 不能为空".to_string()));
    }
    Ok(match provider {
        Provider::Modrinth | Provider::Mcim => format!(
            "{}/project/{}",
            provider.modrinth_root(),
            url_encode(project_id.trim())
        ),
        Provider::Curseforge => format!(
            "{}/mods/{}",
            provider.curseforge_root(),
            url_encode(project_id.trim())
        ),
    })
}

pub fn versions_url(
    provider: Provider,
    project_id: &str,
    mc_version: Option<&str>,
    loader: Option<&str>,
) -> Result<String> {
    if project_id.trim().is_empty() {
        return Err(AppError::Invalid("项目 id 不能为空".to_string()));
    }
    Ok(match provider {
        Provider::Modrinth | Provider::Mcim => {
            let mut url = format!(
                "{}/project/{}/version",
                provider.modrinth_root(),
                url_encode(project_id.trim())
            );
            let mut query: Vec<String> = Vec::new();
            if let Some(version) = mc_version.filter(|value| !value.trim().is_empty()) {
                query.push(format!("game_versions={}", url_encode(&format!("[\"{}\"]", version.trim()))));
            }
            if let Some(loader) = loader.filter(|value| !value.trim().is_empty()) {
                query.push(format!("loaders={}", url_encode(&format!("[\"{}\"]", loader.trim().to_lowercase()))));
            }
            if !query.is_empty() {
                url.push('?');
                url.push_str(&query.join("&"));
            }
            url
        }
        Provider::Curseforge => format!(
            "{}/mods/{}/files?pageSize=50",
            provider.curseforge_root(),
            url_encode(project_id.trim())
        ),
    })
}

/// CurseForge 的 CDN 路径规则：文件 id 拆成 xxx/yyyy。
pub fn curseforge_cdn_path(file_id: u64, file_name: &str) -> String {
    let text = file_id.to_string();
    if text.len() <= 4 {
        return format!("{text}/{file_name}");
    }
    let head = &text[..text.len() - 4];
    let tail = &text[text.len() - 4..];
    format!("{head}/{tail}/{file_name}")
}

/// 版本文件下载地址。
pub fn file_url(
    provider: Provider,
    project_id: &str,
    version_id: &str,
    file_id: Option<u64>,
    file_name: &str,
) -> Result<String> {
    if file_name.trim().is_empty() {
        return Err(AppError::Invalid("文件名不能为空".to_string()));
    }
    Ok(match provider {
        Provider::Modrinth => format!(
            "https://cdn.modrinth.com/data/{}/{}/{}",
            url_encode(project_id.trim()),
            url_encode(version_id.trim()),
            url_encode(file_name.trim())
        ),
        Provider::Mcim => {
            if let Some(file_id) = file_id {
                format!(
                    "{MCIM_BASE}/curseforge/files/{file_id}/{}",
                    url_encode(file_name.trim())
                )
            } else {
                format!(
                    "{MCIM_BASE}/modrinth/data/{}/{}/{}",
                    url_encode(project_id.trim()),
                    url_encode(version_id.trim()),
                    url_encode(file_name.trim())
                )
            }
        }
        Provider::Curseforge => {
            if let Some(file_id) = file_id {
                format!(
                    "https://edge.forgecdn.net/files/{}",
                    curseforge_cdn_path(file_id, file_name.trim())
                )
            } else {
                return Err(AppError::Invalid(
                    "CurseForge 必须带文件 id 才能拼下载地址".to_string(),
                ));
            }
        }
    })
}

/// 内容卡片：字段一个都不能缺（前端直接渲染，缺了就是 undefined）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentItem {
    pub id: String,
    pub provider: String,
    pub kind: String,
    /// 译文（没翻出来就是原文）
    pub title: String,
    pub title_original: String,
    /// 作者（拿不到给空串，不给 undefined）
    pub author: String,
    pub downloads: u64,
    pub follows: Option<u64>,
    /// ISO8601（前端自己算相对时间）
    pub updated_at: String,
    pub summary: String,
    pub summary_original: String,
    pub categories: Vec<String>,
    pub loaders: Vec<String>,
    pub icon_url: Option<String>,
    pub translated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchPage {
    pub items: Vec<ContentItem>,
    pub total: u64,
    pub page: u32,
    pub page_size: u32,
}

fn string_list(value: Option<&serde_json::Value>) -> Vec<String> {
    value
        .and_then(|item| item.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(|text| text.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectDetail {
    pub id: String,
    pub name: String,
    pub description: String,
    pub body: Option<String>,
    pub icon_url: Option<String>,
    pub downloads: u64,
    pub provider: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionItem {
    pub id: String,
    pub name: String,
    pub version_number: String,
    pub mc_versions: Vec<String>,
    pub loaders: Vec<String>,
    pub download_url: Option<String>,
    pub file_name: Option<String>,
    pub sha1: Option<String>,
    pub size: Option<u64>,
    pub file_id: Option<u64>,
}

fn as_string(value: Option<&serde_json::Value>) -> Option<String> {
    value
        .and_then(|item| item.as_str())
        .map(|item| item.to_string())
        .filter(|item| !item.trim().is_empty())
}

fn as_u64(value: Option<&serde_json::Value>) -> u64 {
    value.and_then(|item| item.as_u64()).unwrap_or(0)
}

/// CurseForge 的毫秒时间戳 → ISO8601（拿不到就给空串）。
fn ms_to_iso(value: Option<&serde_json::Value>) -> String {
    let ms = value.and_then(|item| item.as_i64()).unwrap_or(0);
    if ms <= 0 {
        return String::new();
    }
    crate::util::iso8601((ms / 1000) as u64)
}

/// 搜索响应解析：modrinth 与 curseforge 两种形态，字段一律补齐。
pub fn parse_search(
    provider: Provider,
    json: &str,
    kind: &str,
    page: u32,
    page_size: u32,
) -> Result<SearchPage> {
    let (_, _class_id) = kind_mapping(kind)?;
    let value: serde_json::Value = serde_json::from_str(json)?;
    let provider_id = provider.id().to_string();
    let mut items = Vec::new();
    let total;
    if provider == Provider::Curseforge {
        total = as_u64(value.pointer("/pagination/totalCount"));
        if let Some(list) = value.get("data").and_then(|item| item.as_array()) {
            for entry in list {
                let title = as_string(entry.get("name")).unwrap_or_default();
                let summary = as_string(entry.get("summary")).unwrap_or_default();
                let author = entry
                    .get("authors")
                    .and_then(|item| item.as_array())
                    .and_then(|authors| authors.first())
                    .and_then(|author| as_string(author.get("name")))
                    .unwrap_or_default();
                let categories = entry
                    .get("categories")
                    .and_then(|item| item.as_array())
                    .map(|list| {
                        list.iter()
                            .filter_map(|item| {
                                as_string(item.get("name")).or_else(|| as_string(item.get("slug")))
                            })
                            .collect::<Vec<String>>()
                    })
                    .unwrap_or_default();
                let loaders = entry
                    .get("latestFilesIndexes")
                    .and_then(|item| item.as_array())
                    .map(|list| {
                        let mut out: Vec<String> = Vec::new();
                        for item in list {
                            if let Some(loader) = as_string(item.get("modLoader")) {
                                if !out.contains(&loader) {
                                    out.push(loader);
                                }
                            }
                        }
                        out
                    })
                    .unwrap_or_default();
                items.push(ContentItem {
                    id: entry
                        .get("id")
                        .map(|id| id.to_string().trim_matches('"').to_string())
                        .unor_default(),
                    provider: provider_id.clone(),
                    kind: kind.to_string(),
                    title: title.clone(),
                    title_original: title,
                    author,
                    downloads: as_u64(entry.get("downloadCount")),
                    follows: None,
                    updated_at: ms_to_iso(entry.get("dateModified")),
                    summary: summary.clone(),
                    summary_original: summary,
                    categories,
                    loaders,
                    icon_url: as_string(entry.pointer("/logo/thumbnailUrl")),
                    translated: false,
                });
            }
        }
    } else {
        total = as_u64(value.get("total_hits"));
        if let Some(list) = value.get("hits").and_then(|item| item.as_array()) {
            for entry in list {
                let title = as_string(entry.get("title")).unwrap_or_default();
                let summary = as_string(entry.get("description")).unwrap_or_default();
                let mut loaders = string_list(entry.get("loaders"));
                if loaders.is_empty() {
                    // 老响应里没有 loaders 字段：从 display_categories 兜底
                    loaders = string_list(entry.get("display_categories"));
                }
                items.push(ContentItem {
                    id: as_string(entry.get("project_id")).unwrap_or_default(),
                    provider: provider_id.clone(),
                    kind: kind.to_string(),
                    title: title.clone(),
                    title_original: title,
                    author: as_string(entry.get("author")).unwrap_or_default(),
                    downloads: as_u64(entry.get("downloads")),
                    follows: Some(as_u64(entry.get("follows"))),
                    updated_at: as_string(entry.get("date_modified")).unwrap_or_default(),
                    summary: summary.clone(),
                    summary_original: summary,
                    categories: string_list(entry.get("categories")),
                    loaders,
                    icon_url: as_string(entry.get("icon_url")),
                    translated: false,
                });
            }
        }
    }
    Ok(SearchPage {
        items,
        total,
        page,
        page_size,
    })
}

pub fn parse_detail(provider: Provider, json: &str) -> Result<ProjectDetail> {
    let value: serde_json::Value = serde_json::from_str(json)?;
    let provider_id = provider.id().to_string();
    if provider == Provider::Curseforge {
        let entry = value
            .get("data")
            .ok_or_else(|| AppError::Other("CurseForge 详情缺少 data 字段".to_string()))?;
        return Ok(ProjectDetail {
            id: entry
                .get("id")
                .map(|id| id.to_string().trim_matches('"').to_string())
                .unwrap_or_default(),
            name: as_string(entry.get("name")).unwrap_or_default(),
            description: as_string(entry.get("summary")).unwrap_or_default(),
            body: as_string(entry.pointer("/latestFilesIndexes")).or(None),
            icon_url: as_string(entry.pointer("/logo/thumbnailUrl")),
            downloads: as_u64(entry.get("downloadCount")),
            provider: provider_id,
        });
    }
    Ok(ProjectDetail {
        id: as_string(value.get("id")).unwrap_or_default(),
        name: as_string(value.get("title")).unwrap_or_default(),
        description: as_string(value.get("description")).unwrap_or_default(),
        body: as_string(value.get("body")),
        icon_url: as_string(value.get("icon_url")),
        downloads: as_u64(value.get("downloads")),
        provider: provider_id,
    })
}

/// 文字处理小工具（避免 Option 链写太丑）。
trait UnwrapDefault {
    fn unor_default(self) -> String;
}

impl UnwrapDefault for Option<String> {
    fn unor_default(self) -> String {
        self.unwrap_or_default()
    }
}

pub fn parse_versions(provider: Provider, json: &str) -> Result<Vec<VersionItem>> {
    let value: serde_json::Value = serde_json::from_str(json)?;
    let mut out = Vec::new();
    if provider == Provider::Curseforge {
        let list = value
            .get("data")
            .and_then(|item| item.as_array())
            .cloned()
            .unwrap_or_default();
        for entry in list {
            let file = entry.get("files").and_then(|files| files.as_array()).and_then(|files| files.first());
            let file_name = file.and_then(|file| as_string(file.get("fileName")));
            let file_id = file.and_then(|file| file.get("id")).and_then(|id| id.as_u64());
            let download_url = file.and_then(|file| as_string(file.get("downloadUrl"))).or_else(|| {
                match (file_id, file_name.clone()) {
                    (Some(id), Some(name)) => file_url(provider, "", "", Some(id), &name).ok(),
                    _ => None,
                }
            });
            let sha1 = file
                .and_then(|file| file.get("hashes").and_then(|hashes| hashes.as_array()))
                .and_then(|hashes| {
                    hashes.iter().find_map(|hash| {
                        if hash.get("algo").and_then(|algo| algo.as_u64()) == Some(1) {
                            as_string(hash.get("value"))
                        } else {
                            None
                        }
                    })
                });
            out.push(VersionItem {
                id: entry
                    .get("id")
                    .map(|id| id.to_string().trim_matches('"').to_string())
                    .unwrap_or_default(),
                name: as_string(entry.get("displayName")).unwrap_or_default(),
                version_number: as_string(entry.get("displayName")).unwrap_or_default(),
                mc_versions: entry
                    .get("gameVersions")
                    .and_then(|item| item.as_array())
                    .map(|versions| {
                        versions
                            .iter()
                            .filter_map(|version| version.as_str().map(|text| text.to_string()))
                            .collect()
                    })
                    .unwrap_or_default(),
                loaders: Vec::new(),
                download_url,
                file_name,
                sha1,
                size: file.and_then(|file| file.get("fileLength")).and_then(|size| size.as_u64()),
                file_id,
            });
        }
        return Ok(out);
    }

    let list = value.as_array().cloned().unwrap_or_default();
    for entry in list {
        let file = entry.get("files").and_then(|files| files.as_array()).and_then(|files| files.first());
        out.push(VersionItem {
            id: as_string(entry.get("id")).unwrap_or_default(),
            name: as_string(entry.get("name")).unwrap_or_default(),
            version_number: as_string(entry.get("version_number")).unwrap_or_default(),
            mc_versions: entry
                .get("game_versions")
                .and_then(|item| item.as_array())
                .map(|versions| {
                    versions
                        .iter()
                        .filter_map(|version| version.as_str().map(|text| text.to_string()))
                        .collect()
                })
                .unwrap_or_default(),
            loaders: entry
                .get("loaders")
                .and_then(|item| item.as_array())
                .map(|loaders| {
                    loaders
                        .iter()
                        .filter_map(|loader| loader.as_str().map(|text| text.to_string()))
                        .collect()
                })
                .unwrap_or_default(),
            download_url: file.and_then(|file| as_string(file.get("url"))),
            file_name: file.and_then(|file| as_string(file.get("filename"))),
            sha1: file.and_then(|file| as_string(file.pointer("/hashes/sha1"))),
            size: file.and_then(|file| file.get("size")).and_then(|size| size.as_u64()),
            file_id: None,
        });
    }
    Ok(out)
}

/// 带可选 API Key 的 GET（CurseForge 官方接口需要 x-api-key）。
pub async fn http_text(
    client: &reqwest::Client,
    url: &str,
    api_key: Option<&str>,
) -> Result<String> {
    let mut request = client.get(url);
    if let Some(key) = api_key.filter(|key| !key.trim().is_empty()) {
        request = request.header("x-api-key", key.trim());
    }
    let response = request
        .send()
        .await
        .map_err(|err| AppError::Net(format!("请求内容接口失败：{err}")))?;
    if !response.status().is_success() {
        return Err(AppError::Net(format!(
            "内容接口返回 HTTP {}",
            response.status().as_u16()
        )));
    }
    response
        .text()
        .await
        .map_err(|err| AppError::Net(format!("读取内容接口响应失败：{err}")))
}

/// mod_install 的实际动作：解析版本文件 → 下载 → 校验 sha1 → 落到实例 mods 目录。
pub async fn install_mod(
    client: &reqwest::Client,
    paths: &crate::paths::Paths,
    instance_name: &str,
    provider: Provider,
    project_id: &str,
    version_id: &str,
    api_key: Option<&str>,
    via_mcim: bool,
    task: &crate::tasks::TaskHandle,
) -> Result<std::path::PathBuf> {
    use crate::events::Phase;
    task.progress(Phase::Downloading, "正在读取版本文件列表", 0, 0);
    let url = versions_url_for(provider, api_key.is_some(), project_id, None, None)?;
    let text = http_text(client, &url, api_key).await?;
    let versions = parse_versions(provider, &text)?;
    let chosen = versions
        .iter()
        .find(|item| item.id == version_id)
        .or_else(|| versions.first())
        .ok_or_else(|| AppError::Other(format!("项目 {project_id} 没有可用的版本文件")))?;
    let file_name = chosen
        .file_name
        .clone()
        .ok_or_else(|| AppError::Other("版本文件缺少文件名".to_string()))?;
    let download_url = match &chosen.download_url {
        Some(url) => url.clone(),
        None => file_url(
            provider,
            project_id,
            &chosen.id,
            chosen.file_id,
            &file_name,
        )?,
    };

    let dir = crate::mods::kind_dir(paths, instance_name, crate::mods::ModKind::Mod);
    std::fs::create_dir_all(&dir)?;
    let target = dir.join(crate::mods::validate_file_name(&file_name).map(|_| file_name.clone())?);

    task.progress(Phase::Downloading, &format!("正在下载 {file_name}"), 0, 0);
    let bytes = download_bytes(client, &download_url, via_mcim).await?;
    let tmp = target.with_extension("part");
    std::fs::write(&tmp, &bytes)?;
    if let Some(expected) = &chosen.sha1 {
        let actual = crate::downloader::sha1_file(&tmp)?;
        if !actual.eq_ignore_ascii_case(expected) {
            let _ = std::fs::remove_file(&tmp);
            return Err(AppError::Net("模组 sha1 校验失败，已放弃写入".to_string()));
        }
    }
    std::fs::rename(&tmp, &target)?;
    task.progress(Phase::Finished, "模组安装完成", 1, 1);
    tracing::info!(file = %target.display(), bytes = bytes.len(), "模组已安装");
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_and_kind_validation() {
        assert_eq!(Provider::from_id("modrinth").unwrap(), Provider::Modrinth);
        assert_eq!(Provider::from_id("mcim").unwrap(), Provider::Mcim);
        assert!(Provider::from_id("curse").is_err());
        assert_eq!(kind_mapping("mod").unwrap(), ("mod", 6));
        assert_eq!(kind_mapping("resourcepack").unwrap(), ("resourcepack", 12));
        assert_eq!(kind_mapping("shader").unwrap(), ("shader", 6552));
        assert!(kind_mapping("world").is_err());
    }

    #[test]
    fn search_urls_for_each_provider() {
        let modrinth = search_url(Provider::Modrinth, "mod", "sodium", Some("1.20.1"), Some("fabric"), 0, 20).unwrap();
        assert!(modrinth.starts_with("https://api.modrinth.com/v2/search?query=sodium"));
        assert!(modrinth.contains("limit=20"));
        assert!(modrinth.contains("facets="));

        let mcim = search_url(Provider::Mcim, "mod", "sodium", None, None, 0, 20).unwrap();
        assert!(mcim.starts_with("https://mod.mcimirror.top/modrinth/v2/search"));

        let curse = search_url(Provider::Curseforge, "mod", "jei", Some("1.20.1"), Some("forge"), 0, 20).unwrap();
        assert!(curse.starts_with("https://api.curseforge.com/v1/mods/search?gameId=432&classId=6"));
        assert!(curse.contains("searchFilter=jei"));
        assert!(curse.contains("gameVersion=1.20.1"));
        assert!(curse.contains("modLoaderType=1"));

        let mcim_curse = search_url(Provider::Mcim, "resourcepack", "x", None, None, 10, 5).unwrap();
        assert!(mcim_curse.contains("/modrinth/v2/search"));
        assert!(mcim_curse.contains("classId") == false);
    }

    #[test]
    fn detail_and_version_urls() {
        assert_eq!(
            detail_url(Provider::Modrinth, "sodium").unwrap(),
            "https://api.modrinth.com/v2/project/sodium"
        );
        assert_eq!(
            detail_url(Provider::Curseforge, "123").unwrap(),
            "https://api.curseforge.com/v1/mods/123"
        );
        assert_eq!(
            detail_url(Provider::Mcim, "sodium").unwrap(),
            "https://mod.mcimirror.top/modrinth/v2/project/sodium"
        );
        let versions = versions_url(Provider::Modrinth, "sodium", Some("1.20.1"), Some("Fabric")).unwrap();
        assert!(versions.contains("/project/sodium/version?"));
        assert!(versions.contains("game_versions="));
        assert!(versions.contains("loaders="));
        assert!(versions_url(Provider::Curseforge, "123", None, None)
            .unwrap()
            .ends_with("/mods/123/files?pageSize=50"));
        assert!(detail_url(Provider::Modrinth, "  ").is_err());
    }

    #[test]
    fn cdn_urls_follow_mcim_rules() {
        assert_eq!(
            file_url(Provider::Modrinth, "sodium", "ver1", None, "sodium.jar").unwrap(),
            "https://cdn.modrinth.com/data/sodium/ver1/sodium.jar"
        );
        assert_eq!(
            file_url(Provider::Mcim, "sodium", "ver1", None, "sodium.jar").unwrap(),
            "https://mod.mcimirror.top/modrinth/data/sodium/ver1/sodium.jar"
        );
        assert_eq!(
            file_url(Provider::Mcim, "", "", Some(4242424), "jei.jar").unwrap(),
            "https://mod.mcimirror.top/curseforge/files/4242424/jei.jar"
        );
        assert_eq!(
            file_url(Provider::Curseforge, "123", "", Some(4242424), "jei.jar").unwrap(),
            "https://edge.forgecdn.net/files/424/2424/jei.jar"
        );
        assert!(file_url(Provider::Curseforge, "1", "", None, "a.jar").is_err());
        assert_eq!(curseforge_cdn_path(1234567, "a.jar"), "123/4567/a.jar");
        assert_eq!(curseforge_cdn_path(42, "a.jar"), "42/a.jar");
    }

    #[test]
    fn parses_modrinth_search_with_every_field() {
        let json = r#"{"hits":[{"project_id":"abc","title":"Sodium","description":"性能优化","icon_url":"https://x/i.png","downloads":123456,"author":"jellysquid3","follows":900,"date_modified":"2024-05-01T00:00:00Z","categories":["optimization"],"loaders":["fabric"],"versions":["1.20.1"]}],"total_hits":777}"#;
        let page = parse_search(Provider::Modrinth, json, "mod", 1, 20).unwrap();
        assert_eq!(page.total, 777);
        assert_eq!(page.page, 1);
        assert_eq!(page.page_size, 20);
        assert_eq!(page.items.len(), 1);
        let item = &page.items[0];
        assert_eq!(item.id, "abc");
        assert_eq!(item.title, "Sodium");
        assert_eq!(item.title_original, "Sodium");
        assert_eq!(item.author, "jellysquid3");
        assert_eq!(item.downloads, 123456);
        assert_eq!(item.follows, Some(900));
        assert_eq!(item.updated_at, "2024-05-01T00:00:00Z");
        assert_eq!(item.categories, vec!["optimization".to_string()]);
        assert_eq!(item.loaders, vec!["fabric".to_string()]);
        assert_eq!(item.provider, "modrinth");
        assert_eq!(item.kind, "mod");
        assert!(!item.translated);
    }

    #[test]
    fn parses_curseforge_search_with_every_field() {
        let json = r#"{"data":[{"id":238222,"name":"Just Enough Items","summary":"JEI","logo":{"thumbnailUrl":"https://x/t.png"},"downloadCount":999,"authors":[{"name":"mezz"}],"dateModified":1714521600000,"categories":[{"name":"Storage"}],"latestFilesIndexes":[{"modLoader":"Forge"},{"modLoader":"Forge"},{"modLoader":"Fabric"}]}],"pagination":{"totalCount":12}}"#;
        let page = parse_search(Provider::Curseforge, json, "mod", 2, 40).unwrap();
        assert_eq!(page.total, 12);
        assert_eq!(page.page, 2);
        assert_eq!(page.page_size, 40);
        let item = &page.items[0];
        assert_eq!(item.id, "238222");
        assert_eq!(item.title, "Just Enough Items");
        assert_eq!(item.author, "mezz");
        assert_eq!(item.downloads, 999);
        assert_eq!(item.icon_url.as_deref(), Some("https://x/t.png"));
        assert_eq!(item.categories, vec!["Storage".to_string()]);
        assert_eq!(item.loaders, vec!["Forge".to_string(), "Fabric".to_string()]);
        assert!(item.updated_at.starts_with("2024-05-01T"), "{}", item.updated_at);
        assert_eq!(item.provider, "curseforge");
    }

    #[test]
    fn missing_fields_fall_back_to_empty_not_undefined() {
        // Modrinth：只有 id 与标题
        let page = parse_search(Provider::Modrinth, r#"{"hits":[{"project_id":"x","title":"T"}],"total_hits":1}"#, "mod", 1, 20).unwrap();
        let item = &page.items[0];
        assert_eq!(item.author, "");
        assert_eq!(item.summary, "");
        assert_eq!(item.updated_at, "");
        assert!(item.categories.is_empty());
        assert!(item.loaders.is_empty());
        assert_eq!(item.downloads, 0);
        assert_eq!(item.follows, Some(0));

        // CurseForge：没有 authors / logo / dateModified
        let page = parse_search(Provider::Curseforge, r#"{"data":[{"id":7,"name":"N"}],"pagination":{"totalCount":1}}"#, "mod", 1, 20).unwrap();
        let item = &page.items[0];
        assert_eq!(item.author, "");
        assert_eq!(item.updated_at, "");
        assert!(item.icon_url.is_none());
        assert_eq!(item.follows, None);
    }

    #[test]
    fn page_math_is_one_based_and_never_repeats() {
        assert_eq!(page_offset(1, 20), 0);
        assert_eq!(page_offset(2, 20), 20);
        assert_eq!(page_offset(3, 40), 80);
        assert_eq!(page_offset(0, 20), 0, "页码小于 1 按 1 处理");
        // 同一 query 连续两页不重叠
        let first = page_offset(1, 20);
        let second = page_offset(2, 20);
        assert_eq!(second, first + 20);
        assert_eq!(clamp_page_size(Some(20)), 20);
        assert_eq!(clamp_page_size(Some(40)), 40);
        assert_eq!(clamp_page_size(Some(60)), 60);
        assert_eq!(clamp_page_size(None), 20);
        assert_eq!(clamp_page_size(Some(25)), 20, "非法值就近取 20");
        assert_eq!(clamp_page_size(Some(100)), 60);
    }

    #[test]
    fn all_frontend_kinds_map_to_a_class_id() {
        for kind in ["mod", "modpack", "resourcepack", "shader", "datapack"] {
            assert!(kind_mapping(kind).is_ok(), "{kind} 必须有映射");
        }
        assert_eq!(kind_mapping("modpack").unwrap(), ("modpack", 4471));
        assert!(kind_supported(Provider::Modrinth, "modpack"));
        assert!(kind_supported(Provider::Mcim, "modpack"));
        assert!(kind_supported(Provider::Curseforge, "modpack"), "CurseForge 有 4471 分类");
        assert!(kind_mapping("world").is_err());
    }

    #[test]
    fn modpack_search_urls_use_the_right_facets() {
        let modrinth = search_url(Provider::Modrinth, "modpack", "", None, None, 0, 20).unwrap();
        assert!(modrinth.contains("project_type%3Amodpack") || modrinth.contains("project_type:modpack"));
        let curseforge = search_url(Provider::Curseforge, "modpack", "", None, None, 0, 20).unwrap();
        assert!(curseforge.contains("classId=4471"), "{curseforge}");
        let mcim = search_url(Provider::Mcim, "modpack", "", None, None, 0, 20).unwrap();
        assert!(mcim.contains("project_type%3Amodpack") || mcim.contains("project_type:modpack"));
        assert!(mcim.contains("modrinth"));
    }

    #[test]
    fn datapack_kind_only_on_modrinth() {
        assert_eq!(kind_mapping("datapack").unwrap().0, "datapack");
        assert!(kind_supported(Provider::Modrinth, "datapack"));
        assert!(kind_supported(Provider::Mcim, "datapack"));
        assert!(!kind_supported(Provider::Curseforge, "datapack"));
        assert!(kind_supported(Provider::Curseforge, "mod"));
        assert!(!kind_supported(Provider::Modrinth, "world"));
    }

    #[test]
    fn parses_modrinth_versions() {
        let json = r#"[{"id":"v1","name":"1.0","version_number":"1.0.0","game_versions":["1.20.1"],"loaders":["fabric"],"files":[{"url":"https://cdn.modrinth.com/data/a/v1/a.jar","filename":"a.jar","hashes":{"sha1":"deadbeef"},"size":1024}]}]"#;
        let items = parse_versions(Provider::Modrinth, json).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].version_number, "1.0.0");
        assert_eq!(items[0].file_name.as_deref(), Some("a.jar"));
        assert_eq!(items[0].sha1.as_deref(), Some("deadbeef"));
        assert_eq!(items[0].size, Some(1024));
        assert!(items[0].download_url.as_deref().unwrap().ends_with("a.jar"));
    }

    #[test]
    fn parses_curseforge_versions_and_builds_url() {
        let json = r#"{"data":[{"id":55,"displayName":"JEI 1.0","gameVersions":["1.20.1","Forge"],"files":[{"id":4242424,"fileName":"jei.jar","fileLength":2048,"hashes":[{"value":"abc","algo":1}]}]}]}"#;
        let items = parse_versions(Provider::Curseforge, json).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].file_id, Some(4242424));
        assert_eq!(items[0].file_name.as_deref(), Some("jei.jar"));
        assert_eq!(items[0].sha1.as_deref(), Some("abc"));
        assert_eq!(items[0].size, Some(2048));
        assert_eq!(
            items[0].download_url.as_deref(),
            Some("https://edge.forgecdn.net/files/424/2424/jei.jar")
        );
    }

    // ------------------------------------------------ 筛选与插件类型（M10）

    fn cats(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    fn facets_of(query: SearchQuery<'_>) -> String {
        let url = search_url_with(&query).unwrap();
        url.split("facets=").nth(1).expect("没有 facets 参数").to_string()
    }

    fn modrinth_query<'a>(sort: &'a str, loader: Option<&'a str>, categories: &'a [String]) -> SearchQuery<'a> {
        SearchQuery {
            provider: Provider::Modrinth,
            kind: "mod",
            query: "sodium",
            mc_version: Some("1.20.1"),
            loader,
            categories,
            sort: Some(sort),
            category_ids: &[],
            offset: 0,
            limit: 20,
        }
    }

    #[test]
    fn modrinth_sort_maps_to_index_for_all_four_tiers() {
        assert_eq!(modrinth_index(normalize_sort(Some("relevance"))), "relevance");
        assert_eq!(modrinth_index(normalize_sort(Some("downloads"))), "downloads");
        assert_eq!(modrinth_index(normalize_sort(Some("updated"))), "updated");
        assert_eq!(modrinth_index(normalize_sort(Some("newest"))), "newest");
        // 不认识的档位回落到 relevance
        assert_eq!(normalize_sort(Some("follows")), "relevance");
        assert_eq!(normalize_sort(None), "relevance");
        assert_eq!(curseforge_sort_field("downloads"), 6);
        assert_eq!(curseforge_sort_field("updated"), 3);
        assert_eq!(curseforge_sort_field("newest"), 11);
        assert_eq!(curseforge_sort_field("relevance"), 2);
        let url = search_url_with(&modrinth_query("downloads", None, &[])).unwrap();
        assert!(url.contains("index=downloads"), "{url}");
        assert!(url.contains("offset=0"));
        assert!(url.contains("limit=20"));
    }

    #[test]
    fn modrinth_facets_put_each_filter_in_its_own_and_group() {
        let categories = cats(&["optimization"]);
        let facets = facets_of(modrinth_query("downloads", Some("Fabric"), &categories));
        let decoded = facets
            .replace("%5B", "[")
            .replace("%5D", "]")
            .replace("%22", "\"")
            .replace("%3A", ":")
            .replace("%2C", ",");
        assert_eq!(
            decoded,
            "[[\"project_type:mod\"],[\"versions:1.20.1\"],[\"categories:fabric\"],[\"categories:optimization\"]]"
        );
    }

    #[test]
    fn modrinth_facets_ors_multiple_categories() {
        let categories = cats(&["optimization", "storage", "technology"]);
        let facets = modrinth_facets("mod", Some("1.20.1"), None, &categories);
        assert_eq!(facets.len(), 3, "{facets:?}");
        assert_eq!(
            facets[2],
            "[\"categories:optimization\",\"categories:storage\",\"categories:technology\"]"
        );
    }

    #[test]
    fn empty_filters_are_not_appended() {
        let facets = modrinth_facets("mod", None, Some("  "), &[]);
        assert_eq!(facets, vec!["[\"project_type:mod\"]".to_string()]);
        let categories = cats(&["  "]);
        let facets = modrinth_facets("mod", Some(""), Some(""), &categories);
        assert_eq!(facets, vec!["[\"project_type:mod\"]".to_string()]);
    }

    #[test]
    fn plugin_kind_maps_to_modrinth_plugin_and_curseforge_bukkit() {
        // classId=5 实测自 GET /v1/categories?gameId=432（Bukkit Plugins）
        assert_eq!(kind_mapping("plugin").unwrap(), ("plugin", 5));
        assert!(kind_supported(Provider::Modrinth, "plugin"));
        assert!(kind_supported(Provider::Curseforge, "plugin"));
        assert!(kind_supported(Provider::Mcim, "plugin"));
        let url = search_url(Provider::Curseforge, "plugin", "", Some("1.20.1"), None, 0, 20).unwrap();
        assert!(url.contains("classId=5"), "{url}");
    }

    #[test]
    fn plugin_loaders_become_category_facets() {
        assert_eq!(PLUGIN_LOADERS.len(), 7);
        for loader in PLUGIN_LOADERS {
            let facets = modrinth_facets("plugin", Some("1.20.1"), Some(loader), &[]);
            assert_eq!(facets[0], "[\"project_type:plugin\"]");
            assert_eq!(facets[2], format!("[\"categories:{loader}\"]"));
            // CurseForge 没有插件加载器枚举，不能瞎塞 modLoaderType
            assert_eq!(curseforge_loader_type(loader), None);
        }
        assert_eq!(curseforge_loader_type("forge"), Some(1));
        assert_eq!(curseforge_loader_type("Fabric"), Some(4));
        assert_eq!(curseforge_loader_type("quilt"), Some(5));
        assert_eq!(curseforge_loader_type("neoforge"), Some(6));
    }

    #[test]
    fn curseforge_url_carries_every_filter() {
        let categories = cats(&["optimization"]);
        let ids = [1534u32];
        let url = search_url_with(&SearchQuery {
            provider: Provider::Curseforge,
            kind: "mod",
            query: "sodium",
            mc_version: Some("1.20.1"),
            loader: Some("fabric"),
            categories: &categories,
            sort: Some("downloads"),
            category_ids: &ids,
            offset: 40,
            limit: 20,
        })
        .unwrap();
        assert!(url.starts_with("https://api.curseforge.com/v1/mods/search?gameId=432&classId=6"));
        assert!(url.contains("searchFilter=sodium"));
        assert!(url.contains("index=40"));
        assert!(url.contains("pageSize=20"));
        assert!(url.contains("sortField=6"));
        assert!(url.contains("sortOrder=desc"));
        assert!(url.contains("gameVersion=1.20.1"));
        assert!(url.contains("modLoaderType=4"));
        assert!(url.contains("categoryIds=1534"), "{url}");
    }

    #[test]
    fn mcim_mirror_follows_the_same_two_paths() {
        let categories = cats(&["optimization"]);
        let modrinth = search_url_with(&SearchQuery {
            provider: Provider::Mcim,
            kind: "plugin",
            query: "essentialsx",
            mc_version: Some("1.20.1"),
            loader: Some("paper"),
            categories: &categories,
            sort: Some("newest"),
            category_ids: &[],
            offset: 0,
            limit: 20,
        })
        .unwrap();
        assert!(modrinth.starts_with("https://mod.mcimirror.top/modrinth/v2/search"));
        assert!(modrinth.contains("index=newest"));
        // mcim 的搜索永远走镜像的 Modrinth 路径（CurseForge 那边只有文件与分类接口）
        assert!(modrinth.contains("facets="));
        use_curseforge_mirror_for_categories();
    }

    /// mcim 的 CurseForge 侧确实存在（分类接口实测可用），搜索仍走 Modrinth 镜像。
    fn use_curseforge_mirror_for_categories() {
        assert_eq!(
            categories_url(Provider::Mcim),
            "https://mod.mcimirror.top/curseforge/v1/categories?gameId=432"
        );
    }

// ------------------------------------------------ 无 Key 走镜像 / 下载通道

    #[test]
    fn download_urls_are_rewritten_to_the_mcim_mirror() {
        assert_eq!(
            mirror_download_url("https://cdn.modrinth.com/data/AANobbMI/versions/abc/sodium.jar")
                .as_deref(),
            Some("https://mod.mcimirror.top/modrinth/data/AANobbMI/versions/abc/sodium.jar")
        );
        // CurseForge 的 CDN 路径要还原成文件 id
        assert_eq!(
            mirror_download_url("https://edge.forgecdn.net/files/424/2424/jei.jar").as_deref(),
            Some("https://mod.mcimirror.top/curseforge/files/4242424/jei.jar")
        );
        assert_eq!(
            mirror_download_url("https://media.forgecdn.net/files/5123/4567/a b.jar").as_deref(),
            Some("https://mod.mcimirror.top/curseforge/files/51234567/a%20b.jar")
        );
        // 认不出来的地址、已经镜像过的地址都不改写
        assert!(mirror_download_url("https://example.com/x.jar").is_none());
        assert!(mirror_download_url("https://mod.mcimirror.top/curseforge/files/1/a.jar").is_none());
        assert!(mirror_download_url("  ").is_none());
        // 关闭设置时一律不改写
        assert!(preferred_download_url("https://cdn.modrinth.com/data/a/b/c.jar", false).is_none());
        assert_eq!(
            preferred_download_url("https://cdn.modrinth.com/data/a/b/c.jar", true).as_deref(),
            Some("https://mod.mcimirror.top/modrinth/data/a/b/c.jar")
        );
    }

    #[test]
    fn curseforge_without_key_goes_through_the_mirror() {
        assert_eq!(mcim_curseforge_root(), "https://mod.mcimirror.top/curseforge/v1");
        assert_eq!(effective_provider(Provider::Curseforge, false), Provider::Mcim);
        assert_eq!(effective_provider(Provider::Curseforge, true), Provider::Curseforge);
        let categories = cats(&["optimization"]);
        let query = SearchQuery {
            provider: Provider::Curseforge,
            kind: "mod",
            query: "sodium",
            mc_version: Some("1.20.1"),
            loader: Some("fabric"),
            categories: &categories,
            sort: Some("downloads"),
            category_ids: &[1534],
            offset: 0,
            limit: 20,
        };
        let mirror = search_url_for(&query, false).unwrap();
        assert!(mirror.starts_with("https://mod.mcimirror.top/curseforge/v1/mods/search"), "{mirror}");
        assert!(mirror.contains("classId=6"));
        assert!(mirror.contains("categoryIds=1534"));
        let official = search_url_for(&query, true).unwrap();
        assert!(official.starts_with("https://api.curseforge.com/v1/mods/search"), "{official}");
        // 详情/版本同样处理
        let mirror_detail = detail_url_for(Provider::Curseforge, false, "238222").unwrap();
        assert_eq!(mirror_detail, "https://mod.mcimirror.top/curseforge/v1/mods/238222");
        let official_detail = detail_url_for(Provider::Curseforge, true, "238222").unwrap();
        assert_eq!(official_detail, "https://api.curseforge.com/v1/mods/238222");
        let mirror_versions = versions_url_for(Provider::Curseforge, false, "238222", None, None).unwrap();
        assert!(mirror_versions.starts_with("https://mod.mcimirror.top/curseforge/v1/mods/238222/files"));
        let official_versions = versions_url_for(Provider::Curseforge, true, "238222", None, None).unwrap();
        assert!(official_versions.starts_with("https://api.curseforge.com/v1/mods/238222/files"));
    }

    #[test]
    fn disabled_translation_returns_originals() {
        let json = r#"{"hits":[{"project_id":"abc","title":"Sodium","description":"A mod","downloads":1,"categories":["optimization"],"loaders":["fabric"]}],"total_hits":1}"#;
        let mut page = parse_search(Provider::Modrinth, json, "mod", 1, 20).unwrap();
        page.items[0].title = "钠".to_string();
        page.items[0].summary = "一个模组".to_string();
        page.items[0].translated = true;
        disable_translation(&mut page.items);
        assert_eq!(page.items[0].title, page.items[0].title_original);
        assert_eq!(page.items[0].summary, page.items[0].summary_original);
        assert!(!page.items[0].translated);
        assert_eq!(page.items[0].title, "Sodium");
    }

    #[test]
    fn categories_parse_into_lowercase_name_and_slug_index() {
        let json = r#"{"data":[
            {"id":5,"name":"Bukkit Plugins","slug":"bukkit-plugins"},
            {"id":6,"name":"Mods","slug":"mc-mods"},
            {"id":1534,"name":"Optimization","slug":"optimization"},
            {"name":"没有 id"}]}"#;
        let ids = parse_categories(json).unwrap();
        assert_eq!(ids.get("bukkit plugins"), Some(&5));
        assert_eq!(ids.get("bukkit-plugins"), Some(&5));
        assert_eq!(ids.get("optimization"), Some(&1534));
        assert_eq!(ids.get("mc-mods"), Some(&6));
        assert_eq!(ids.len(), 5);
        assert_eq!(categories_url(Provider::Curseforge), "https://api.curseforge.com/v1/categories?gameId=432");
        assert!(categories_url(Provider::Mcim).starts_with("https://mod.mcimirror.top/curseforge/v1/categories"));
        assert!(parse_categories("{}").is_err());
    }

    #[test]
    fn category_cache_expires_after_24_hours() {
        assert!(category_cache_is_fresh(1_000, 1_000));
        assert!(category_cache_is_fresh(1_000, 1_000 + CATEGORY_TTL_SECS - 1));
        assert!(!category_cache_is_fresh(1_000, 1_000 + CATEGORY_TTL_SECS));
        // 没拉过时缓存里查不到
        assert!(cached_category_ids(&cats(&["optimization"]), crate::util::now_unix()).is_none());
    }
}
