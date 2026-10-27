//! 下载源解析：内置源 + 自定义镜像 + auto 测速。
//!
//! 约定：
//! - official / bmclapi / sjtu 的 URL 映射沿用 download::map_url；
//! - custom 用与 BMCLAPI 相同的路径规则映射到用户填的根地址；
//! - auto 先并发测速再排序，全挂回落 official。

use std::time::{Duration, Instant};

use serde::Serialize;

use crate::download::{map_url, DownloadSource, OFFICIAL_MANIFEST};
use crate::error::{AppError, Result};
use crate::settings::Settings;

/// 单个源的测速超时。
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(8);
/// 每个源测几次取中位数。
pub const PROBE_SAMPLES: usize = 3;

/// 运行期的一个下载源：内置四种之一，或用户自定义镜像根。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    Builtin(DownloadSource),
    /// 已经规范化过的镜像根（末尾不带斜杠）
    Custom(String),
}

impl Origin {
    pub fn id(&self) -> &str {
        match self {
            Origin::Builtin(source) => source.id(),
            Origin::Custom(_) => "custom",
        }
    }

    pub fn base(&self) -> String {
        match self {
            Origin::Builtin(source) => source.base().to_string(),
            Origin::Custom(root) => root.clone(),
        }
    }

    pub fn is_custom(&self) -> bool {
        matches!(self, Origin::Custom(_))
    }
}

/// 自定义镜像的映射表（与 BMCLAPI 兼容：根 + 官方路径尾部）。
const CUSTOM_RULES: [(&str, &str); 5] = [
    ("https://launchermeta.mojang.com/", ""),
    ("https://piston-meta.mojang.com/", ""),
    ("https://piston-data.mojang.com/", ""),
    ("https://resources.download.minecraft.net/", "assets/"),
    ("https://libraries.minecraft.net/", "maven/"),
];

/// 把一个官方 URL 映射到指定源。
pub fn map_origin(origin: &Origin, original: &str) -> String {
    match origin {
        Origin::Builtin(source) => map_url(*source, original),
        Origin::Custom(root) => {
            for (prefix, suffix) in CUSTOM_RULES {
                if let Some(rest) = original.strip_prefix(prefix) {
                    return format!("{root}/{suffix}{rest}");
                }
            }
            original.to_string()
        }
    }
}

/// 校验并规范化自定义镜像根地址（去空白、去末尾斜杠、只收 http(s)）。
pub fn normalize_mirror_url(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(AppError::Invalid("自定义镜像地址不能为空".to_string()));
    }
    if !trimmed.is_ascii() {
        // 域名必须是 ASCII；带中文的一定是用户填错了
        return Err(AppError::Invalid(
            "自定义镜像地址只能是 ASCII（域名不能含中文）".to_string(),
        ));
    }
    let lower = trimmed.to_ascii_lowercase();
    let rest = if let Some(rest) = lower.strip_prefix("https://") {
        rest
    } else if let Some(rest) = lower.strip_prefix("http://") {
        rest
    } else {
        return Err(AppError::Invalid(
            "自定义镜像地址必须以 http:// 或 https:// 开头".to_string(),
        ));
    };
    if rest.is_empty() || rest.starts_with('/') || rest.starts_with('.') {
        return Err(AppError::Invalid("自定义镜像地址缺少主机名".to_string()));
    }
    if rest.contains(char::is_whitespace) {
        return Err(AppError::Invalid("自定义镜像地址不能包含空格".to_string()));
    }
    Ok(trimmed.trim_end_matches('/').to_string())
}

/// 内置源在 auto 之外的选择顺序（第一个是首选）。
pub fn builtin_order(source_id: &str) -> Vec<Origin> {
    let order = match source_id {
        "bmclapi" => vec![DownloadSource::Bmclapi, DownloadSource::Official],
        "sjtu" => vec![
            DownloadSource::Sjtu,
            DownloadSource::Official,
            DownloadSource::Bmclapi,
        ],
        // official / auto / 未知值都以官方打头
        _ => vec![
            DownloadSource::Official,
            DownloadSource::Bmclapi,
            DownloadSource::Sjtu,
        ],
    };
    order.into_iter().map(Origin::Builtin).collect()
}

/// 同步解析（不含测速）：custom 非法时回落到 official 打头。
pub fn resolve(settings: &Settings) -> Vec<Origin> {
    match settings.download_source.as_str() {
        "custom" => match normalize_mirror_url(&settings.custom_mirror_url) {
            Ok(root) => vec![Origin::Custom(root), Origin::Builtin(DownloadSource::Official)],
            Err(err) => {
                tracing::warn!(%err, "自定义镜像地址非法，回落到官方源");
                builtin_order("official")
            }
        },
        other => builtin_order(other),
    }
}

/// auto 的候选：官方、BMCLAPI，外加配好的自定义镜像。
pub fn auto_candidates(custom_url: &str) -> Vec<Origin> {
    let mut out = vec![
        Origin::Builtin(DownloadSource::Official),
        Origin::Builtin(DownloadSource::Bmclapi),
    ];
    if let Ok(root) = normalize_mirror_url(custom_url) {
        out.push(Origin::Custom(root));
    }
    out
}

/// 测速结果（sources_probe 的返回值）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceProbe {
    pub source_id: String,
    pub ok: bool,
    pub latency_ms: u64,
    pub error: Option<String>,
}

/// 一组延迟取中位数（偶数个时取中间两个的平均）。
pub fn median_of(mut samples: Vec<u64>) -> u64 {
    if samples.is_empty() {
        return 0;
    }
    samples.sort_unstable();
    let middle = samples.len() / 2;
    if samples.len() % 2 == 0 {
        (samples[middle - 1] + samples[middle]) / 2
    } else {
        samples[middle]
    }
}

/// 纯函数：挑延迟最低的可用源。
pub fn pick_fastest(probes: &[SourceProbe]) -> Option<String> {
    probes
        .iter()
        .filter(|probe| probe.ok)
        .min_by_key(|probe| probe.latency_ms)
        .map(|probe| probe.source_id.clone())
}

/// 按测速结果把候选源排序（可用且快的前面，失败的后置，官方永远在最后兜底）。
pub fn order_by_probes(candidates: Vec<Origin>, probes: &[SourceProbe]) -> Vec<Origin> {
    let latency_of = |origin: &Origin| -> Option<u64> {
        probes
            .iter()
            .find(|probe| probe.source_id == origin.id())
            .and_then(|probe| if probe.ok { Some(probe.latency_ms) } else { None })
    };
    let mut sorted = candidates;
    sorted.sort_by_key(|origin| latency_of(origin).unwrap_or(u64::MAX));
    sorted
}

/// 单源测速：连发 PROBE_SAMPLES 次 HEAD，取中位数。
pub async fn probe_one(client: &reqwest::Client, origin: &Origin) -> SourceProbe {
    let url = map_origin(origin, OFFICIAL_MANIFEST);
    let mut samples: Vec<u64> = Vec::new();
    let mut last_error: Option<String> = None;
    for _ in 0..PROBE_SAMPLES {
        let started = Instant::now();
        let attempt = tokio::time::timeout(PROBE_TIMEOUT, client.head(&url).send()).await;
        match attempt {
            Ok(Ok(response)) if response.status().is_success() => {
                samples.push(started.elapsed().as_millis() as u64);
            }
            Ok(Ok(response)) => {
                last_error = Some(format!("HTTP {}", response.status().as_u16()));
            }
            Ok(Err(err)) => {
                last_error = Some(format!("{err}"));
            }
            Err(_) => {
                last_error = Some(format!("超时（{} 秒）", PROBE_TIMEOUT.as_secs()));
            }
        }
    }
    if samples.is_empty() {
        return SourceProbe {
            source_id: origin.id().to_string(),
            ok: false,
            latency_ms: 0,
            error: Some(last_error.unwrap_or_else(|| "没有响应".to_string())),
        };
    }
    SourceProbe {
        source_id: origin.id().to_string(),
        ok: true,
        latency_ms: median_of(samples),
        error: None,
    }
}

/// 并发测一组源。
pub async fn probe_all(client: &reqwest::Client, origins: &[Origin]) -> Vec<SourceProbe> {
    let mut set = tokio::task::JoinSet::new();
    for origin in origins {
        let client = client.clone();
        let origin = origin.clone();
        set.spawn(async move { probe_one(&client, &origin).await });
    }
    let mut out = Vec::new();
    while let Some(joined) = set.join_next().await {
        if let Ok(probe) = joined {
            out.push(probe);
        }
    }
    // 顺序与候选一致，前端好对齐
    out.sort_by_key(|probe| {
        origins
            .iter()
            .position(|origin| origin.id() == probe.source_id)
            .unwrap_or(usize::MAX)
    });
    out
}

/// 真正干活用的解析：auto 会先测速。
pub async fn resolve_with_probe(client: &reqwest::Client, settings: &Settings) -> Vec<Origin> {
    match settings.download_source.as_str() {
        "auto" => {
            let candidates = auto_candidates(&settings.custom_mirror_url);
            let probes = probe_all(client, &candidates).await;
            for probe in &probes {
                tracing::info!(
                    source = %probe.source_id,
                    ok = probe.ok,
                    latency_ms = probe.latency_ms,
                    "下载源测速"
                );
            }
            match pick_fastest(&probes) {
                Some(_) => order_by_probes(candidates, &probes),
                None => {
                    tracing::warn!("所有下载源都测不通，回落到官方源");
                    builtin_order("official")
                }
            }
        }
        _ => resolve(settings),
    }
}

/// download_sources() 的条目（新旧字段名都给，前端哪种都能读）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceInfo {
    pub id: String,
    pub label: String,
    pub name: String,
    pub description: String,
    pub base: String,
    pub base_url: String,
    pub selectable: bool,
}

pub fn sources_table(settings: &Settings) -> Vec<SourceInfo> {
    let custom_base = normalize_mirror_url(&settings.custom_mirror_url).unwrap_or_default();
    let entry = |id: &str, description: &str, base: String, selectable: bool| SourceInfo {
        id: id.to_string(),
        label: id.to_string(),
        name: id.to_string(),
        description: description.to_string(),
        base: base.clone(),
        base_url: base,
        selectable,
    };
    vec![
        entry(
            "official",
            "launchermeta.mojang.com，最权威但国内可能较慢",
            DownloadSource::Official.base().to_string(),
            true,
        ),
        entry(
            "bmclapi",
            "bangbang93 提供的国内镜像，稳定首选",
            DownloadSource::Bmclapi.base().to_string(),
            true,
        ),
        entry(
            "sjtu",
            "上海交通大学镜像站（旧配置兼容，界面暂不展示）",
            DownloadSource::Sjtu.base().to_string(),
            false,
        ),
        entry(
            "custom",
            "自定义镜像：按 BMCLAPI 的路径规则映射到自己的根地址",
            custom_base,
            true,
        ),
        entry("auto", "按测速结果在官方、镜像与自定义源之间自动选择", String::new(), true),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe(id: &str, ok: bool, latency: u64) -> SourceProbe {
        SourceProbe {
            source_id: id.to_string(),
            ok,
            latency_ms: latency,
            error: if ok { None } else { Some("超时".to_string()) },
        }
    }

    #[test]
    fn mirror_url_normalization() {
        assert_eq!(
            normalize_mirror_url("  https://mirror.example.com/  ").unwrap(),
            "https://mirror.example.com"
        );
        assert_eq!(
            normalize_mirror_url("http://10.0.0.2:8080/mc/").unwrap(),
            "http://10.0.0.2:8080/mc"
        );
        assert!(normalize_mirror_url("").is_err());
        assert!(normalize_mirror_url("   ").is_err());
        assert!(normalize_mirror_url("ftp://mirror.example.com").is_err());
        assert!(normalize_mirror_url("mirror.example.com").is_err());
        assert!(normalize_mirror_url("https://").is_err());
        assert!(normalize_mirror_url("https:///path").is_err());
        assert!(normalize_mirror_url("https://镜 像.example.com").is_err());
    }

    #[test]
    fn custom_mapping_follows_bmclapi_rules() {
        let origin = Origin::Custom("https://mirror.example.com".to_string());
        assert_eq!(
            map_origin(&origin, OFFICIAL_MANIFEST),
            "https://mirror.example.com/mc/game/version_manifest_v2.json"
        );
        assert_eq!(
            map_origin(&origin, "https://libraries.minecraft.net/org/lwjgl/a.jar"),
            "https://mirror.example.com/maven/org/lwjgl/a.jar"
        );
        assert_eq!(
            map_origin(&origin, "https://resources.download.minecraft.net/ab/abcdef"),
            "https://mirror.example.com/assets/ab/abcdef"
        );
        assert_eq!(
            map_origin(&origin, "https://piston-data.mojang.com/v1/objects/x/client.jar"),
            "https://mirror.example.com/v1/objects/x/client.jar"
        );
        // 未知域名原样返回
        assert_eq!(
            map_origin(&origin, "https://example.com/x.jar"),
            "https://example.com/x.jar"
        );
    }

    #[test]
    fn builtin_origins_match_download_module() {
        let origin = Origin::Builtin(DownloadSource::Bmclapi);
        assert_eq!(
            map_origin(&origin, OFFICIAL_MANIFEST),
            map_url(DownloadSource::Bmclapi, OFFICIAL_MANIFEST)
        );
        assert_eq!(origin.id(), "bmclapi");
        assert!(!origin.is_custom());
        assert_eq!(origin.base(), "https://bmclapi2.bangbang93.com");
    }

    #[test]
    fn resolve_orders_and_falls_back_to_official() {
        let mut settings = Settings::default();
        settings.download_source = "bmclapi".to_string();
        let origins = resolve(&settings);
        assert_eq!(origins[0].id(), "bmclapi");
        assert_eq!(origins[1].id(), "official");

        settings.download_source = "custom".to_string();
        settings.custom_mirror_url = "https://mirror.example.com/".to_string();
        let origins = resolve(&settings);
        assert_eq!(origins[0].id(), "custom");
        assert!(origins[0].is_custom());
        assert_eq!(origins[1].id(), "official");

        // 非法 custom → 官方打头
        settings.custom_mirror_url = "not a url".to_string();
        let origins = resolve(&settings);
        assert_eq!(origins[0].id(), "official");

        // sjtu 仍在枚举里
        settings.download_source = "sjtu".to_string();
        assert_eq!(resolve(&settings)[0].id(), "sjtu");
    }

    #[test]
    fn auto_candidates_include_custom_only_when_valid() {
        assert_eq!(auto_candidates("").len(), 2);
        assert_eq!(auto_candidates("https://mirror.example.com").len(), 3);
        assert_eq!(auto_candidates("nope").len(), 2);
    }

    #[test]
    fn fastest_source_is_picked_by_latency() {
        let probes = vec![
            probe("official", true, 900),
            probe("bmclapi", true, 120),
            probe("custom", false, 0),
        ];
        assert_eq!(pick_fastest(&probes), Some("bmclapi".to_string()));
        assert_eq!(pick_fastest(&[probe("official", false, 0)]), None);

        let candidates = auto_candidates("https://mirror.example.com");
        let ordered = order_by_probes(candidates, &probes);
        assert_eq!(ordered[0].id(), "bmclapi");
        assert_eq!(ordered[1].id(), "official");
        assert_eq!(ordered[2].id(), "custom", "失败的源排最后");
    }

    #[test]
    fn median_handles_even_and_odd() {
        assert_eq!(median_of(vec![10, 20, 30]), 20);
        assert_eq!(median_of(vec![30, 10, 20, 40]), 25);
        assert_eq!(median_of(vec![]), 0);
        assert_eq!(median_of(vec![7]), 7);
    }

    #[test]
    fn sources_table_exposes_custom_and_selectable() {
        let mut settings = Settings::default();
        settings.custom_mirror_url = "https://mirror.example.com/".to_string();
        let table = sources_table(&settings);
        assert_eq!(table.len(), 5);
        let custom = table.iter().find(|item| item.id == "custom").unwrap();
        assert_eq!(custom.base_url, "https://mirror.example.com");
        assert!(custom.selectable);
        let sjtu = table.iter().find(|item| item.id == "sjtu").unwrap();
        assert!(!sjtu.selectable);
        let json = serde_json::to_value(&table).unwrap();
        assert!(json[0].get("baseUrl").is_some());
        assert!(json[0].get("selectable").is_some());
    }
}
