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
