//! 资源（模组/整合包）下载源：官方原站 / MCIM 镜像 / 自动选择 + 测速。

use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::settings::Settings;

/// 单个探测源超时。
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
/// 每个源探几次取中位数。
pub const PROBE_SAMPLES: usize = 3;
/// 探测结果缓存 10 分钟。
pub const PROBE_CACHE_TTL: Duration = Duration::from_secs(600);

/// resource_sources() 的条目。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceSource {
    pub id: String,
    pub label: String,
    pub description: String,
    pub base: String,
    pub selectable: bool,
}

/// 两个可选源（auto 是设置项，不在列表里）。
pub const OFFICIAL_SEARCH: &str = "https://api.modrinth.com/v2/tag/category";
pub const MCIM_SEARCH: &str = "https://mod.mcimirror.top/modrinth/v2/tag/category";
pub const OFFICIAL_BASE: &str = "https://api.modrinth.com";
pub const MCIM_BASE: &str = "https://mod.mcimirror.top";

pub fn resource_sources() -> Vec<ResourceSource> {
    vec![
        ResourceSource {
            id: "official".to_string(),
            label: "官方原站".to_string(),
            description: "直接连接 api.modrinth.com，权威但国内可能较慢".to_string(),
            base: OFFICIAL_BASE.to_string(),
            selectable: true,
        },
        ResourceSource {
            id: "mcim".to_string(),
            label: "MCIM 镜像".to_string(),
            description: "mod.mcimirror.top 的 Modrinth/CurseForge 镜像，国内更快".to_string(),
            base: MCIM_BASE.to_string(),
            selectable: true,
        },
        ResourceSource {
            id: "auto".to_string(),
            label: "自动选择".to_string(),
            description: "按实测延迟自动切换，跳过不可用镜像".to_string(),
            base: String::new(),
            selectable: true,
        },
    ]
}

/// resource_probe 的结果。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceProbe {
    pub source_id: String,
    pub ok: bool,
    pub latency_ms: u64,
    pub error: Option<String>,
}

type Cache = Mutex<std::collections::HashMap<String, (Instant, ResourceProbe)>>;

fn cache() -> &'static Cache {
    static CACHE: std::sync::OnceLock<Cache> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

pub fn cached(source_id: &str) -> Option<ResourceProbe> {
    let guard = cache().lock().ok()?;
    let (at, probe) = guard.get(source_id)?;
    if at.elapsed() < PROBE_CACHE_TTL {
        Some(probe.clone())
    } else {
        None
    }
}

fn store(probe: &ResourceProbe) {
    if let Ok(mut guard) = cache().lock() {
        guard.insert(probe.source_id.clone(), (Instant::now(), probe.clone()));
    }
}

pub fn clear_cache() {
    if let Ok(mut guard) = cache().lock() {
        guard.clear();
    }
}

/// 纯函数：这次探测要不要读缓存。
pub fn cache_hit(force: bool, cached: Option<&ResourceProbe>) -> bool {
    !force && cached.is_some()
}

/// 探测一个源（force=true 时不读缓存，结果写回缓存）。
pub async fn probe_one(client: &reqwest::Client, source_id: &str, force: bool) -> ResourceProbe {
    if !force {
        if let Some(hit) = cached(source_id) {
            tracing::debug!(source = source_id, latency_ms = hit.latency_ms, "资源源探测命中缓存");
            return hit;
        }
    } else {
        tracing::info!(source = source_id, "强制重新测速：忽略缓存");
    }
    let url = match source_id {
        "official" => OFFICIAL_SEARCH,
        _ => MCIM_SEARCH,
    };
    let mut samples: Vec<u64> = Vec::new();
    let mut last_error: Option<String> = None;
    for _ in 0..PROBE_SAMPLES {
        let started = Instant::now();
        let attempt = tokio::time::timeout(
            PROBE_TIMEOUT,
            client
                .get(url)
                .header(reqwest::header::ACCEPT, "application/json")
                .send(),
        )
        .await;
        match attempt {
            Ok(Ok(response)) if response.status().is_success() => {
                samples.push(started.elapsed().as_millis() as u64);
            }
            Ok(Ok(response)) => {
                last_error = Some(format!("HTTP {}", response.status().as_u16()));
            }
            Ok(Err(err)) => last_error = Some(err.to_string()),
            Err(_) => last_error = Some(format!("超时（{}s）", PROBE_TIMEOUT.as_secs())),
        }
    }
    let probe = if samples.is_empty() {
        ResourceProbe {
            source_id: source_id.to_string(),
            ok: false,
            latency_ms: 0,
            error: Some(last_error.unwrap_or_else(|| "没有响应".to_string())),
        }
    } else {
        samples.sort_unstable();
        ResourceProbe {
            source_id: source_id.to_string(),
            ok: true,
            latency_ms: samples[samples.len() / 2],
            error: None,
        }
    };
    if force {
        tracing::info!(
            source = %probe.source_id,
            latency_ms = probe.latency_ms,
            ok = probe.ok,
            "强制重新测速"
        );
    } else {
        tracing::info!(
            source = %probe.source_id,
            ok = probe.ok,
            latency_ms = probe.latency_ms,
            error = ?probe.error,
            "资源源判定结果"
        );
    }
    store(&probe);
    probe
}

/// 探测两个源（并发）。
pub async fn probe_all(client: &reqwest::Client, force: bool) -> Vec<ResourceProbe> {
    let a = probe_one(client, "official", force);
    let b = probe_one(client, "mcim", force);
    let (official, mcim) = tokio::join!(a, b);
    vec![official, mcim]
}

/// 设置里的三选一。
pub fn normalize_source(raw: &str) -> &'static str {
    match raw.trim().to_ascii_lowercase().as_str() {
        "official" => "official",
        "mcim" => "mcim",
        _ => "auto",
    }
}

/// 纯函数：按设置 + 最近一次探测结果决定走哪个通道。
/// 返回 (source_id, reason)：reason = manual | auto | no-key
pub fn route(settings: &Settings, probes: &[ResourceProbe]) -> (String, &'static str) {
    let selected = normalize_source(&settings.content_source);
    if selected != "auto" {
        return (selected.to_string(), "manual");
    }
    let fastest = probes
        .iter()
        .filter(|probe| probe.ok)
        .min_by_key(|probe| probe.latency_ms);
    match fastest {
        Some(probe) => (probe.source_id.clone(), "auto"),
        // 没有探测结果时默认 mcim（国内更快）
        None => ("mcim".to_string(), "auto"),
    }
}

/// 用缓存里的探测结果路由（不额外发请求），并打日志。
pub fn route_with_cache(settings: &Settings, provider: &str, has_key: bool) -> (String, &'static str) {
    // CurseForge 无 Key 时只能走 MCIM，优先于用户设置
    if provider == "curseforge" && !has_key {
        tracing::info!(
            source = "mcim",
            reason = "no-key",
            provider,
            "资源下载源命中选择"
        );
        return ("mcim".to_string(), "no-key");
    }
    let probes: Vec<ResourceProbe> = ["official", "mcim"]
        .iter()
        .filter_map(|id| cached(id))
        .collect();
    let (source, reason) = route(settings, &probes);
    tracing::info!(source = %source, reason, provider, "资源下载源命中选择");
    (source, reason)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe(id: &str, ok: bool, latency: u64) -> ResourceProbe {
        ResourceProbe {
            source_id: id.to_string(),
            ok,
            latency_ms: latency,
            error: if ok { None } else { Some("超时".to_string()) },
        }
    }

    #[test]
    fn three_way_routing_follows_the_setting() {
        let mut settings = Settings::default();
        assert_eq!(settings.content_source, "auto", "默认 auto");
        settings.content_source = "official".to_string();
        assert_eq!(route(&settings, &[]).0, "official");
        settings.content_source = "mcim".to_string();
        assert_eq!(route(&settings, &[]).0, "mcim");
        assert_eq!(route(&settings, &[]).1, "manual");
    }

    #[test]
    fn auto_picks_the_fastest_probed_source() {
        let mut settings = Settings::default();
        settings.content_source = "auto".to_string();
        let probes = vec![probe("official", true, 320), probe("mcim", true, 76)];
        assert_eq!(route(&settings, &probes).0, "mcim");
        let probes = vec![probe("official", true, 80), probe("mcim", true, 260)];
        assert_eq!(route(&settings, &probes).0, "official");
    }

    #[test]
    fn auto_without_probe_results_defaults_to_mcim() {
        let mut settings = Settings::default();
        settings.content_source = "auto".to_string();
        assert_eq!(route(&settings, &[]).0, "mcim", "没测过默认走镜像");
        assert_eq!(route(&settings, &[probe("official", false, 0)]).0, "mcim");
    }

    #[test]
    fn no_key_forces_mcim_even_when_official_selected() {
        let mut settings = Settings::default();
        settings.content_source = "official".to_string();
        let (source, reason) = route_with_cache(&settings, "curseforge", false);
        assert_eq!(source, "mcim");
        assert_eq!(reason, "no-key");
    }

    #[test]
    fn illegal_value_falls_back_to_auto() {
        assert_eq!(normalize_source("nonsense"), "auto");
        assert_eq!(normalize_source(""), "auto");
        assert_eq!(normalize_source(" MCIM "), "mcim");
        let mut settings = Settings::default();
        settings.content_source = "nonsense".to_string();
        assert_eq!(route(&settings, &[]).0, "mcim");
    }

    #[test]
    fn probe_cache_semantics_match_download_sources() {
        let hit = probe("mcim", true, 76);
        assert!(cache_hit(false, Some(&hit)));
        assert!(!cache_hit(true, Some(&hit)), "force 不读缓存");
        assert!(!cache_hit(false, None));
        let _guard = cache_lock();
        clear_cache();
        store(&probe("mcim", true, 70));
        assert_eq!(cached("mcim").unwrap().latency_ms, 70);
        store(&probe("mcim", true, 83));
        assert_eq!(cached("mcim").unwrap().latency_ms, 83, "force 后覆盖缓存");
        clear_cache();
    }

    #[test]
    fn resource_sources_expose_three_entries_including_auto() {
        let list = resource_sources();
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].id, "official");
        assert_eq!(list[1].id, "mcim");
        assert_eq!(list[2].id, "auto", "自动选择也要出现在列表里");
        assert!(list[2].base.is_empty(), "auto 没有固定 base");
        assert!(list[2].description.contains("自动"));
        assert_eq!(serde_json::to_value(&list).unwrap()[2]["id"], "auto");
        assert!(list.iter().all(|item| item.selectable));
        assert!(list[1].base.contains("mcimirror"));
        let json = serde_json::to_value(&list).unwrap();
        assert!(json[0].get("selectable").is_some());
    }

    fn cache_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(|err| err.into_inner())
    }
}
