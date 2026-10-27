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

    fn curseforge_root(&self) -> String {
        match self {
            Provider::Mcim => format!("{MCIM_BASE}/curseforge/v1"),
            _ => CURSEFORGE_API.to_string(),
        }
    }
}

/// 内容类型 → (Modrinth project_type, CurseForge classId)。
pub fn kind_mapping(kind: &str) -> Result<(&'static str, u32)> {
    match kind {
        "mod" => Ok(("mod", 6)),
        // CurseForge 的 Modpacks classId 是 4471
        "modpack" => Ok(("modpack", 4471)),
        "resourcepack" => Ok(("resourcepack", 12)),
        "shader" => Ok(("shader", 6552)),
        // 数据包只有 Modrinth/MCIM 支持；classId 0 表示「CurseForge 没有对应分类」
        "datapack" => Ok(("datapack", 0)),
        other => Err(AppError::Invalid(format!("不支持的内容类型：{other}"))),
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

/// 搜索地址。curseforge 走官方接口时要额外带 x-api-key（由命令层加头）。
pub fn search_url(
    provider: Provider,
    kind: &str,
    query: &str,
    mc_version: Option<&str>,
    loader: Option<&str>,
    offset: u32,
    limit: u32,
) -> Result<String> {
    let (project_type, class_id) = kind_mapping(kind)?;
    let limit = limit.clamp(1, 50);
    match provider {
        Provider::Modrinth | Provider::Mcim => {
            let mut facets = vec![format!("[\"project_type:{project_type}\"]")];
            if let Some(version) = mc_version.filter(|value| !value.trim().is_empty()) {
                facets.push(format!("[\"versions:{}\" ]", version.trim()).replace("\" ]", "\"]"));
            }
            if let Some(loader) = loader.filter(|value| !value.trim().is_empty()) {
                facets.push(format!("[\"categories:{}\" ]", loader.trim().to_lowercase()).replace("\" ]", "\"]"));
            }
            Ok(format!(
                "{}/search?query={}&limit={}&offset={}&facets={}",
                provider.modrinth_root(),
                url_encode(query),
                limit,
                offset,
                url_encode(&format!("[{}]", facets.join(",")))
            ))
        }
        Provider::Curseforge => {
            let mut url = format!(
                "{}/mods/search?gameId={CURSEFORGE_GAME_ID}&classId={class_id}&searchFilter={}&index={}&pageSize={}",
                provider.curseforge_root(),
                url_encode(query),
                offset,
                limit
            );
            if let Some(version) = mc_version.filter(|value| !value.trim().is_empty()) {
                url.push_str(&format!("&gameVersion={}", url_encode(version.trim())));
            }
            if let Some(loader) = loader.filter(|value| !value.trim().is_empty()) {
                // 4=Fabric 5=Forge 6=NeoForge 1=Forge(旧) —— 只区分 fabric/forge
                let loader_type = match loader.trim().to_lowercase().as_str() {
                    "fabric" | "quilt" => 4,
                    "forge" => 1,
                    "neoforge" => 6,
                    _ => 0,
                };
                if loader_type > 0 {
                    url.push_str(&format!("&modLoaderType={loader_type}"));
                }
            }
            Ok(url)
        }
    }
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
    task: &crate::tasks::TaskHandle,
) -> Result<std::path::PathBuf> {
    use crate::events::Phase;
    task.progress(Phase::Downloading, "正在读取版本文件列表", 0, 0);
    let url = versions_url(provider, project_id, None, None)?;
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
    let response = client
        .get(&download_url)
        .send()
        .await
        .map_err(|err| AppError::Net(format!("下载模组失败：{err}")))?;
    if !response.status().is_success() {
        return Err(AppError::Net(format!(
            "下载模组失败：HTTP {}",
            response.status().as_u16()
        )));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|err| AppError::Net(format!("读取模组内容失败：{err}")))?;
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
}
