//! 版本清单：拉取 + 缓存到 `.mc\version_manifest.json`。
//!
//! M1 只做「拉清单、读版本列表」，完整下载器（资源索引、库、natives）留到 M2。

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::download::OFFICIAL_MANIFEST;
use crate::error::{AppError, Result};
use crate::paths::Paths;
use crate::sources::{map_origin, Origin};

/// 清单里的单条版本。
///
/// 前端契约：字段名必须是 versionType（Mojang 原始字段叫 type，读盘时用 alias 兼容），
/// 值一律小写（release / snapshot / old_beta / old_alpha），releaseTime 是 ISO8601。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionSummary {
    pub id: String,
    #[serde(rename = "versionType", alias = "type")]
    pub version_type: String,
    pub url: String,
    #[serde(rename = "releaseTime")]
    pub release_time: String,
}

impl VersionSummary {
    /// 类型统一小写（"Release" / "RELEASE" 都归一到 release）。
    pub fn normalized(mut self) -> Self {
        self.version_type = self.version_type.trim().to_ascii_lowercase();
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatestVersions {
    pub release: String,
    pub snapshot: String,
}

/// 官方清单原始结构（只取 M1 需要的字段，其余忽略）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawManifest {
    pub latest: LatestVersions,
    pub versions: Vec<VersionSummary>,
}

/// 缓存文件外层：带抓取时间，便于前端显示「数据来自几点」。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheEnvelope {
    #[serde(rename = "fetchedAt")]
    fetched_at: u64,
    manifest: RawManifest,
}

/// 给前端的视图（camelCase）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionManifestView {
    pub latest_release: String,
    pub latest_snapshot: String,
    pub versions: Vec<VersionSummary>,
    /// ISO8601（前端契约是 string | null，不给数字时间戳）
    pub fetched_at: Option<String>,
    pub from_cache: bool,
}

impl VersionManifestView {
    fn from_raw(raw: RawManifest, fetched_at: Option<u64>, from_cache: bool) -> Self {
        Self {
            latest_release: raw.latest.release,
            latest_snapshot: raw.latest.snapshot,
            versions: raw
                .versions
                .into_iter()
                .map(VersionSummary::normalized)
                .collect(),
            fetched_at: fetched_at.map(crate::util::iso8601),
            from_cache,
        }
    }
}

// ------------------------------------------------------------------ 加载器版本（M12）

/// 加载器版本的请求超时（每个源独立 8 秒，互不影响）。
pub const LOADER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(8);
/// 客户端能装的加载器。
pub const CLIENT_LOADERS: [&str; 3] = ["fabric", "forge", "neoforge"];

/// loader_versions 的元素。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoaderVersion {
    /// 装实例时要写进 versions 目录的版本号
    pub id: String,
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recommended: Option<bool>,
}

/// loaders_list 的一组：某个加载器到底能不能用。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoaderGroup {
    pub loader: String,
    pub ok: bool,
    pub versions: Vec<LoaderVersion>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

pub fn fabric_loader_url(mc_version: &str) -> String {
    format!("https://meta.fabricmc.net/v2/versions/loader/{mc_version}")
}

pub fn forge_promotions_url() -> String {
    "https://files.minecraftforge.net/net/minecraftforge/forge/promotions_slim.json".to_string()
}

pub fn neoforge_metadata_url() -> String {
    "https://maven.neoforged.net/releases/net/neoforged/neoforge/maven-metadata.xml".to_string()
}

/// Fabric：数组里每项有 loader.version / loader.stable，stable 的标成推荐。
pub fn parse_fabric_loaders(json: &str) -> Vec<LoaderVersion> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let Some(items) = value.as_array() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in items {
        let loader = item.get("loader").unwrap_or(item);
        let Some(version) = loader.get("version").and_then(|entry| entry.as_str()) else {
            continue;
        };
        let stable = loader
            .get("stable")
            .and_then(|entry| entry.as_bool())
            .unwrap_or(false);
        out.push(LoaderVersion {
            id: version.to_string(),
            version: version.to_string(),
            recommended: Some(stable),
        });
    }
    out
}

/// Forge：promotions_slim.json 里的  mc-recommended 与 mc-latest。
pub fn parse_forge_promotions(json: &str, mc_version: &str) -> Vec<LoaderVersion> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let Some(promos) = value.get("promos").and_then(|entry| entry.as_object()) else {
        return Vec::new();
    };
    let pick = |key: &str| {
        promos
            .get(&format!("{mc_version}-{key}"))
            .and_then(|entry| entry.as_str())
            .map(str::to_string)
    };
    let recommended = pick("recommended");
    let latest = pick("latest");
    let mut out = Vec::new();
    if let Some(version) = recommended.clone() {
        out.push(LoaderVersion {
            // Forge 的实例版本号要求 mc-loader
            id: format!("{mc_version}-{version}"),
            version,
            recommended: Some(true),
        });
    }
    if let Some(version) = latest {
        if Some(&version) != recommended.as_ref() {
            out.push(LoaderVersion {
                id: format!("{mc_version}-{version}"),
                version,
                recommended: Some(false),
            });
        }
    }
    out
}

/// NeoForge：maven-metadata.xml 里按 mc 去掉 1. 的前缀匹配（1.20.1 走 47.1.x 老编号）。
pub fn parse_neoforge_versions(xml: &str, mc_version: &str) -> Vec<LoaderVersion> {
    let mut prefixes: Vec<String> = Vec::new();
    if let Some(rest) = mc_version.strip_prefix("1.") {
        prefixes.push(rest.to_string());
    }
    prefixes.push(mc_version.to_string());
    if mc_version == "1.20.1" {
        prefixes.insert(0, "47.1".to_string());
    }
    let mut out: Vec<LoaderVersion> = Vec::new();
    for chunk in xml.split("<version>").skip(1) {
        let Some(end) = chunk.find("</version>") else {
            continue;
        };
        let version = chunk[..end].trim();
        if version.is_empty() || version.contains("-beta") {
            continue;
        }
        if !prefixes
            .iter()
            .any(|prefix| !prefix.is_empty() && version.starts_with(prefix))
        {
            continue;
        }
        out.push(LoaderVersion {
            id: version.to_string(),
            version: version.to_string(),
            recommended: None,
        });
    }
    out.sort_by(|left, right| right.version.cmp(&left.version));
    out.dedup_by(|left, right| left.version == right.version);
    out
}

async fn fetch_loader_text(client: &reqwest::Client, url: &str, loader: &str) -> Result<String> {
    let started = std::time::Instant::now();
    let response = tokio::time::timeout(LOADER_TIMEOUT, client.get(url).send())
        .await
        .map_err(|_| {
            tracing::warn!(loader, url, "加载器元数据请求超时");
            AppError::Net(format!(
                "{loader} 元数据请求超时（{} 秒）",
                LOADER_TIMEOUT.as_secs()
            ))
        })??
        .error_for_status()
        .map_err(|err| {
            tracing::warn!(loader, url, %err, "加载器元数据返回错误状态");
            AppError::Net(format!("{loader} 元数据不可用：{err}"))
        })?;
    let text = response
        .text()
        .await
        .map_err(|err| AppError::Net(format!("{loader} 元数据读取失败：{err}")))?;
    tracing::info!(
        loader,
        url,
        bytes = text.len(),
        elapsed_ms = started.elapsed().as_millis() as u64,
        "加载器元数据已取到"
    );
    Ok(text)
}

/// 单个加载器的可用版本列表。
pub async fn loader_versions(
    client: &reqwest::Client,
    loader: &str,
    mc_version: &str,
) -> Result<Vec<LoaderVersion>> {
    let loader = loader.trim().to_lowercase();
    let mc = mc_version.trim();
    if mc.is_empty() {
        return Err(AppError::Invalid("请先选择 Minecraft 版本".to_string()));
    }
    match loader.as_str() {
        "fabric" | "quilt" => {
            let text = fetch_loader_text(client, &fabric_loader_url(mc), "Fabric").await?;
            Ok(parse_fabric_loaders(&text))
        }
        "forge" => {
            let text = fetch_loader_text(client, &forge_promotions_url(), "Forge").await?;
            Ok(parse_forge_promotions(&text, mc))
        }
        "neoforge" => {
            let text = fetch_loader_text(client, &neoforge_metadata_url(), "NeoForge").await?;
            Ok(parse_neoforge_versions(&text, mc))
        }
        "" | "vanilla" => Ok(Vec::new()),
        other => Err(AppError::Invalid(format!("不支持的加载器：{other}"))),
    }
}

/// 三种加载器一次问完：任何一个失败都不影响其它。
pub async fn loaders_list(client: &reqwest::Client, mc_version: &str) -> Vec<LoaderGroup> {
    let mut handles = Vec::new();
    for loader in CLIENT_LOADERS {
        let client = client.clone();
        let mc = mc_version.to_string();
        handles.push(tokio::spawn(async move {
            (loader, loader_versions(&client, loader, &mc).await)
        }));
    }
    let mut out = Vec::new();
    for handle in handles {
        match handle.await {
            Ok((loader, Ok(versions))) => out.push(LoaderGroup {
                loader: loader.to_string(),
                ok: !versions.is_empty(),
                versions,
                reason: None,
            }),
            Ok((loader, Err(err))) => out.push(LoaderGroup {
                loader: loader.to_string(),
                ok: false,
                versions: Vec::new(),
                reason: Some(err.to_string()),
            }),
            Err(err) => tracing::warn!(%err, "加载器版本查询任务失败"),
        }
    }
    out
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 读缓存；文件缺失或损坏都返回 None（由调用方决定是否联网）。
pub fn read_cache(path: &Path) -> Option<(RawManifest, u64)> {
    let text = std::fs::read_to_string(path).ok()?;
    let envelope: CacheEnvelope = serde_json::from_str(&text).ok()?;
    Some((envelope.manifest, envelope.fetched_at))
}

pub fn write_cache(path: &Path, raw: &RawManifest) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let envelope = CacheEnvelope {
        fetched_at: now_secs(),
        manifest: raw.clone(),
    };
    let text = serde_json::to_string_pretty(&envelope)?;
    std::fs::write(path, text)?;
    Ok(())
}

/// 联网拉清单：按候选源顺序逐个尝试，第一个成功的就用。
pub async fn fetch(client: &reqwest::Client, origins: &[Origin]) -> Result<RawManifest> {
    let mut failures: Vec<String> = Vec::new();
    for origin in origins {
        let url = map_origin(origin, OFFICIAL_MANIFEST);
        match client.get(&url).send().await {
            Ok(response) if response.status().is_success() => {
                match response.json::<RawManifest>().await {
                    Ok(raw) => return Ok(raw),
                    Err(err) => failures.push(format!("{}: {err}", origin.id())),
                }
            }
            Ok(response) => failures.push(format!(
                "{}: HTTP {}",
                origin.id(),
                response.status().as_u16()
            )),
            Err(err) => failures.push(format!("{}: {err}", origin.id())),
        }
    }
    Err(AppError::Net(format!(
        "版本清单拉取失败：{}",
        failures.join("；")
    )))
}

/// 命令实现：force_refresh=false 时优先缓存，没有缓存或强制刷新才联网。
pub async fn load_with_origins(
    client: &reqwest::Client,
    paths: &Paths,
    origins: Vec<Origin>,
    force_refresh: bool,
) -> Result<VersionManifestView> {
    if !force_refresh {
        if let Some((raw, fetched_at)) = read_cache(&paths.version_manifest_file) {
            return Ok(VersionManifestView::from_raw(raw, Some(fetched_at), true));
        }
    }
    let raw = fetch(client, &origins).await?;
    write_cache(&paths.version_manifest_file, &raw)?;
    Ok(VersionManifestView::from_raw(raw, Some(now_secs()), false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fabric_loader_versions_are_parsed() {
        let json = r#"[
            {"loader":{"version":"0.15.11","stable":true},"intermediary":{"version":"1.20.1"}},
            {"loader":{"version":"0.16.0-beta.1","stable":false}}
        ]"#;
        let items = parse_fabric_loaders(json);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].version, "0.15.11");
        assert_eq!(items[0].recommended, Some(true));
        assert_eq!(items[1].recommended, Some(false));
        assert!(parse_fabric_loaders("{}").is_empty(), "不是数组就当空");
        assert!(parse_fabric_loaders("not json").is_empty());
        assert_eq!(
            fabric_loader_url("1.20.1"),
            "https://meta.fabricmc.net/v2/versions/loader/1.20.1"
        );
    }

    #[test]
    fn forge_promotions_give_recommended_and_latest() {
        let json = r#"{"promos":{"1.20.1-recommended":"47.2.0","1.20.1-latest":"47.3.12","1.19.2-recommended":"43.2.0"}}"#;
        let items = parse_forge_promotions(json, "1.20.1");
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].id, "1.20.1-47.2.0");
        assert_eq!(items[0].recommended, Some(true));
        assert_eq!(items[1].version, "47.3.12");
        let same = parse_forge_promotions(
            r#"{"promos":{"1.20.1-recommended":"47.2.0","1.20.1-latest":"47.2.0"}}"#,
            "1.20.1",
        );
        assert_eq!(same.len(), 1);
        assert!(parse_forge_promotions(json, "1.21.9").is_empty());
        assert!(forge_promotions_url().ends_with("promotions_slim.json"));
    }

    #[test]
    fn neoforge_versions_match_the_mc_prefix() {
        let xml = r#"<metadata><versioning><versions>
            <version>47.1.106</version>
            <version>20.4.237</version>
            <version>21.0.167</version>
            <version>21.1.9-beta</version>
        </versions></versioning></metadata>"#;
        let items = parse_neoforge_versions(xml, "1.20.1");
        assert_eq!(items.len(), 1, "{items:?}");
        assert_eq!(items[0].version, "47.1.106");
        let new = parse_neoforge_versions(xml, "1.20.4");
        assert_eq!(new.len(), 1);
        assert_eq!(new[0].version, "20.4.237");
        let latest = parse_neoforge_versions(xml, "1.21");
        assert_eq!(latest[0].version, "21.0.167");
        assert!(parse_neoforge_versions(xml, "1.19.4").is_empty());
        assert!(neoforge_metadata_url().contains("maven-metadata.xml"));
    }

    #[test]
    fn empty_mc_version_is_rejected_before_network() {
        let client = reqwest::Client::new();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let error = runtime
            .block_on(loader_versions(&client, "fabric", "  "))
            .unwrap_err()
            .to_string();
        assert!(error.contains("请先选择 Minecraft 版本"), "{error}");
        let error = runtime
            .block_on(loader_versions(&client, "不支持", "1.20.1"))
            .unwrap_err()
            .to_string();
        assert!(error.contains("不支持的加载器"), "{error}");
    }

    #[test]
    fn cache_roundtrip_keeps_versions() {
        let dir = std::env::temp_dir().join(format!("gugu-meta-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("version_manifest.json");

        let raw = RawManifest {
            latest: LatestVersions {
                release: "1.21.4".into(),
                snapshot: "25w02a".into(),
            },
            versions: vec![VersionSummary {
                id: "1.21.4".into(),
                version_type: "release".into(),
                url: "https://example.com/1.21.4.json".into(),
                release_time: "2025-01-08T10:00:00+00:00".into(),
            }],
        };
        write_cache(&path, &raw).unwrap();

        let (loaded, fetched_at) = read_cache(&path).expect("缓存应可读回");
        assert_eq!(loaded.latest.release, "1.21.4");
        assert_eq!(loaded.versions.len(), 1);
        assert!(fetched_at > 0);

        let view = VersionManifestView::from_raw(loaded, Some(fetched_at), true);
        assert!(view.from_cache);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn view_serializes_version_type_lowercase_and_fetched_at_iso() {
        let raw = RawManifest {
            latest: LatestVersions {
                release: "1.21.4".into(),
                snapshot: "25w02a".into(),
            },
            versions: vec![
                VersionSummary {
                    id: "1.21.4".into(),
                    version_type: "Release".into(),
                    url: "https://x/1.json".into(),
                    release_time: "2025-01-08T10:00:00+00:00".into(),
                },
                VersionSummary {
                    id: "25w02a".into(),
                    version_type: "snapshot".into(),
                    url: "https://x/2.json".into(),
                    release_time: "2025-01-15T10:00:00+00:00".into(),
                },
            ],
        };
        let view = VersionManifestView::from_raw(raw, Some(1_700_000_000), false);
        let json = serde_json::to_value(&view).unwrap();
        assert_eq!(json["versions"][0]["versionType"], "release", "类型必须是小写 release");
        assert!(json["versions"][0].get("type").is_none(), "不要同时发 type");
        assert_eq!(json["versions"][1]["versionType"], "snapshot");
        assert_eq!(json["versions"][0]["releaseTime"], "2025-01-08T10:00:00+00:00");
        assert_eq!(json["fetchedAt"], "2023-11-14T22:13:20Z");
        assert_eq!(json["fromCache"], false);

        // 读盘仍兼容 Mojang 的 type 字段
        let parsed: VersionSummary =
            serde_json::from_str(r#"{"id":"1.20.1","type":"release","url":"u","releaseTime":"t"}"#)
                .unwrap();
        assert_eq!(parsed.version_type, "release");
    }

    #[test]
    fn all_four_version_types_are_kept_and_mapped() {
        let json = r#"{"latest":{"release":"1.21.4","snapshot":"25w02a"},"versions":[
            {"id":"1.21.4","type":"release","url":"u1","releaseTime":"2025-01-08T10:00:00+00:00"},
            {"id":"25w02a","type":"snapshot","url":"u2","releaseTime":"2025-01-15T10:00:00+00:00"},
            {"id":"b1.7.3","type":"old_beta","url":"u3","releaseTime":"2011-07-08T10:00:00+00:00"},
            {"id":"a1.2.6","type":"old_alpha","url":"u4","releaseTime":"2010-12-02T10:00:00+00:00"}
        ]}"#;
        let raw: RawManifest = serde_json::from_str(json).unwrap();
        let view = VersionManifestView::from_raw(raw, Some(0), false);
        assert_eq!(view.versions.len(), 4, "四种类型都要留下");
        let value = serde_json::to_value(&view).unwrap();
        let types: Vec<String> = value["versions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["versionType"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(types, vec!["release", "snapshot", "old_beta", "old_alpha"]);
        assert_eq!(value["versions"][2]["id"], "b1.7.3", "原顺序不能变");
        assert!(value["versions"][0].get("type").is_none());

        // 空清单不 panic
        let empty: RawManifest = serde_json::from_str(r#"{"latest":{"release":"","snapshot":""},"versions":[]}"#).unwrap();
        let view = VersionManifestView::from_raw(empty, None, true);
        assert!(view.versions.is_empty());
        assert!(view.fetched_at.is_none());
    }

    #[test]
    fn cache_hit_returns_the_same_manifest() {
        let dir = std::env::temp_dir().join(format!("gugu-meta-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("version_manifest.json");
        let raw = RawManifest {
            latest: LatestVersions { release: "1.21.4".into(), snapshot: "25w02a".into() },
            versions: vec![VersionSummary {
                id: "b1.7.3".into(),
                version_type: "old_beta".into(),
                url: "u".into(),
                release_time: "2011-07-08T10:00:00+00:00".into(),
            }],
        };
        write_cache(&path, &raw).unwrap();
        let (first, at_first) = read_cache(&path).unwrap();
        let (second, at_second) = read_cache(&path).unwrap();
        assert_eq!(first.versions.len(), second.versions.len());
        assert_eq!(first.versions[0].version_type, "old_beta", "缓存不能只留 release");
        assert_eq!(at_first, at_second);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_cache_returns_none() {
        let path = std::env::temp_dir().join("gugu-meta-missing.json");
        let _ = std::fs::remove_file(&path);
        assert!(read_cache(&path).is_none());
    }
}
