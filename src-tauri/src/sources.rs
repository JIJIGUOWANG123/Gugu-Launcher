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
/// 探测结果缓存 10 分钟（设置页反复打开不必重复打）。
pub const PROBE_CACHE_TTL: Duration = Duration::from_secs(600);

type ProbeCache = std::sync::Mutex<
    std::collections::HashMap<String, (Instant, SourceProbe)>,
>;

fn probe_cache() -> &'static ProbeCache {
    static CACHE: std::sync::OnceLock<ProbeCache> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// 取缓存里的探测结果（过期就丢掉）。
pub fn cached_probe(source_id: &str) -> Option<SourceProbe> {
    let guard = probe_cache().lock().ok()?;
    let (at, probe) = guard.get(source_id)?;
    if at.elapsed() < PROBE_CACHE_TTL {
        Some(probe.clone())
    } else {
        None
    }
}

fn store_probe(probe: &SourceProbe) {
    if let Ok(mut guard) = probe_cache().lock() {
        guard.insert(probe.source_id.clone(), (Instant::now(), probe.clone()));
    }
}

/// 清空探测缓存（设置里换了自定义镜像、或手动重测）。
pub fn clear_probe_cache() {
    if let Ok(mut guard) = probe_cache().lock() {
        guard.clear();
    }
}

/// 探测判定：200/206/3xx 算可用；405/416 也算（这些镜像对 Range 支持差，但文件本身能下）。
pub fn probe_status_usable(status: u16) -> bool {
    (200..400).contains(&status) || status == 405 || status == 416
}

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
        // 8 个 BMCLAPI 协议镜像：自身优先，然后官方
        other if DownloadSource::from_id(other).map(|source| source.is_bmclapi_mirror()).unwrap_or(false) => {
            match DownloadSource::from_id(other) {
                Some(source) => vec![source, DownloadSource::Official],
                None => vec![DownloadSource::Official],
            }
        }
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

/// auto 的候选：官方 + 全部 BMCLAPI 镜像（bmclapi 官方 + 8 个国内镜像）+ 配好的自定义镜像。
pub fn auto_candidates(custom_url: &str) -> Vec<Origin> {
    let mut out: Vec<Origin> = DownloadSource::MIRRORS
        .iter()
        .map(|source| Origin::Builtin(*source))
        .collect();
    out.push(Origin::Builtin(DownloadSource::Official));
    if let Ok(root) = normalize_mirror_url(custom_url) {
        out.push(Origin::Custom(root));
    }
    out
}

/// 把「其它已测可用的镜像」补进候选链（延迟升序），官方永远最后。
/// 只读探测缓存，不额外发请求。
pub fn augment_with_mirrors(mut origins: Vec<Origin>) -> Vec<Origin> {
    let mut extras: Vec<(u64, Origin)> = Vec::new();
    for source in DownloadSource::MIRRORS {
        let origin = Origin::Builtin(source);
        if origins.iter().any(|item| item.id() == origin.id()) {
            continue;
        }
        if let Some(probe) = cached_probe(origin.id()) {
            if probe.ok {
                extras.push((probe.latency_ms, origin));
            }
        }
    }
    extras.sort_by_key(|(latency, _)| *latency);
    origins.extend(extras.into_iter().map(|(_, origin)| origin));
    if let Some(index) = origins.iter().position(|origin| origin.id() == "official") {
        let official = origins.remove(index);
        origins.push(official);
    }
    origins
}

/// 缓存里现有探测结果的快照（不额外发请求）。
pub fn probe_snapshot() -> Vec<SourceProbe> {
    match probe_cache().lock() {
        Ok(guard) => guard
            .values()
            .filter(|(at, _)| at.elapsed() < PROBE_CACHE_TTL)
            .map(|(_, probe)| probe.clone())
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// 纯函数：给定「首选链」与探测结果，排出真正的候选链。
/// 规则：首选排第一（不支持该 URL 的部分镜像直接剔除）→ 其余已测可用镜像按延迟升序
/// → 未测过的按声明顺序 → official 永远最后。
pub fn order_candidates(base: Vec<Origin>, probes: &[SourceProbe], original_url: &str) -> Vec<Origin> {
    let latency_of = |id: &str| probes.iter().find(|probe| probe.source_id == id);
    let mut head: Option<Origin> = None;
    let mut rest: Vec<Origin> = Vec::new();
    let push_unique = |list: &mut Vec<Origin>, origin: Origin| {
        if !list.iter().any(|item| item.id() == origin.id()) {
            list.push(origin);
        }
    };
    for origin in base {
        if !origin_supports(&origin, original_url) {
            continue;
        }
        match &head {
            None => head = Some(origin),
            Some(_) => push_unique(&mut rest, origin),
        }
    }
    // 其余内置镜像：只在探测确认可用时才加入（避免浪费一次 404）
    for source in DownloadSource::MIRRORS {
        let origin = Origin::Builtin(source);
        if !origin_supports(&origin, original_url) {
            continue;
        }
        if latency_of(origin.id()).map(|probe| probe.ok).unwrap_or(false) {
            push_unique(&mut rest, origin);
        }
    }
    rest.sort_by_key(|origin| match latency_of(origin.id()) {
        Some(probe) if probe.ok => (0u8, probe.latency_ms),
        Some(_) => (2u8, u64::MAX),
        None => (1u8, u64::MAX),
    });
    let mut chain: Vec<Origin> = head.into_iter().collect();
    chain.extend(rest);
    if let Some(index) = chain.iter().position(|origin| origin.id() == "official") {
        let official = chain.remove(index);
        chain.push(official);
    }
    chain
}

/// 单次下载最多尝试几个候选源（避免长时间卡住）。
pub const MAX_DOWNLOAD_ATTEMPTS: usize = 4;

/// 这个源能不能下这个官方 URL：部分镜像只代理 libraries / assets。
pub fn origin_supports(origin: &Origin, original: &str) -> bool {
    match origin {
        // 自定义镜像按 BMCLAPI 规则映射，认为支持全部路径
        Origin::Custom(_) => true,
        Origin::Builtin(source) => {
            if !source.partial_mirror() {
                return true;
            }
            original.starts_with("https://libraries.minecraft.net/")
                || original.starts_with("https://resources.download.minecraft.net/")
        }
    }
}

/// 候选链：首选的源 → 其余可用镜像（延迟升序，剔除不支持该 URL 的）→ official 兜底。
///
/// 只读 10 分钟探测缓存，不额外发探测请求。
pub fn candidate_chain(settings: &Settings, original_url: &str) -> Vec<Origin> {
    order_candidates(resolve(settings), &probe_snapshot(), original_url)
}

/// 纯函数：这次探测要不要用缓存（force 时永远不用）。
pub fn probe_cache_hit(force: bool, cached: Option<&SourceProbe>) -> bool {
    !force && cached.is_some()
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

/// 单源测速：对**已知存在的真实文件**发 GET Range(0-0)，只读 1 字节。
///
/// HEAD 在 CDN 上经常被 405/403 挡掉，所以不用 HEAD。
pub async fn probe_one(client: &reqwest::Client, origin: &Origin) -> SourceProbe {
    probe_one_with(client, origin, false).await
}

/// force=true 时不读缓存（前端点「测速」就要新数字）。
pub async fn probe_one_with(client: &reqwest::Client, origin: &Origin, force: bool) -> SourceProbe {
    if !force {
        if let Some(hit) = cached_probe(origin.id()) {
            tracing::debug!(source = %origin.id(), latency_ms = hit.latency_ms, "下载源探测命中缓存");
            return hit;
        }
    } else {
        tracing::info!(source = %origin.id(), "强制重新测速：忽略缓存");
    }
    // 内置源用它自己支持的探测地址（部分大学镜像没有 /mc/ 端点，只镜像 maven/assets）
    let url = match origin {
        Origin::Builtin(source) => source.probe_url(),
        Origin::Custom(_) => map_origin(origin, OFFICIAL_MANIFEST),
    };
    let mut samples: Vec<u64> = Vec::new();
    let mut last_error: Option<String> = None;
    let mut last_status: Option<u16> = None;
    for _ in 0..PROBE_SAMPLES {
        let started = Instant::now();
        let attempt = tokio::time::timeout(
            PROBE_TIMEOUT,
            client
                .get(&url)
                .header(reqwest::header::RANGE, "bytes=0-0")
                .send(),
        )
        .await;
        let elapsed = started.elapsed().as_millis() as u64;
        match attempt {
            Ok(Ok(response)) => {
                let status = response.status().as_u16();
                last_status = Some(status);
                if probe_status_usable(status) {
                    samples.push(elapsed);
                    tracing::info!(
                        source = %origin.id(),
                        url = %url,
                        status,
                        elapsed_ms = elapsed,
                        "下载源探测成功"
                    );
                    continue;
                }
                last_error = Some(format!("HTTP {status}"));
                tracing::warn!(
                    source = %origin.id(),
                    url = %url,
                    status,
                    elapsed_ms = elapsed,
                    "下载源探测状态码不可用"
                );
            }
            Ok(Err(err)) => {
                last_error = Some(format!("{err}"));
                tracing::warn!(
                    source = %origin.id(),
                    url = %url,
                    elapsed_ms = elapsed,
                    error = %err,
                    "下载源探测连接失败"
                );
            }
            Err(_) => {
                last_error = Some(format!("超时（{} 秒）", PROBE_TIMEOUT.as_secs()));
                tracing::warn!(
                    source = %origin.id(),
                    url = %url,
                    elapsed_ms = elapsed,
                    "下载源探测超时"
                );
            }
        }
    }
    let probe = if samples.is_empty() {
        SourceProbe {
            source_id: origin.id().to_string(),
            ok: false,
            latency_ms: 0,
            error: Some(last_error.unwrap_or_else(|| "没有响应".to_string())),
        }
    } else {
        SourceProbe {
            source_id: origin.id().to_string(),
            ok: true,
            latency_ms: median_of(samples),
            error: None,
        }
    };
    if force {
        tracing::info!(
            source = %probe.source_id,
            latency_ms = probe.latency_ms,
            status = ?last_status,
            "强制重新测速"
        );
    } else {
        tracing::info!(
            source = %probe.source_id,
            ok = probe.ok,
            latency_ms = probe.latency_ms,
            status = ?last_status,
            error = ?probe.error,
            "下载源判定结果"
        );
    }
    store_probe(&probe);
    probe
}

/// 并发测一组源。
pub async fn probe_all(client: &reqwest::Client, origins: &[Origin]) -> Vec<SourceProbe> {
    probe_all_with(client, origins, false).await
}

/// force=true：全部重新探测并刷新缓存。
pub async fn probe_all_with(
    client: &reqwest::Client,
    origins: &[Origin],
    force: bool,
) -> Vec<SourceProbe> {
    let mut set = tokio::task::JoinSet::new();
    for origin in origins {
        let client = client.clone();
        let origin = origin.clone();
        set.spawn(async move { probe_one_with(&client, &origin, force).await });
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
                Some(_) => augment_with_mirrors(order_by_probes(candidates, &probes)),
                None => {
                    tracing::warn!("所有下载源都测不通，回落到官方源");
                    builtin_order("official")
                }
            }
        }
        // 指定源：它自己优先，其余已测可用镜像按延迟补在后面，官方兜底
        _ => augment_with_mirrors(resolve(settings)),
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
            "minguncle",
            "个人镜像 mirror.minguncle.cn（BMCLAPI 协议）",
            DownloadSource::Minguncle.base().to_string(),
            true,
        ),
        entry(
            "nju",
            "南京大学镜像 mirror.nju.edu.cn（BMCLAPI 协议）",
            DownloadSource::Nju.base().to_string(),
            true,
        ),
        entry(
            "lzu",
            "兰州大学镜像（BMCLAPI 协议）",
            DownloadSource::Lzu.base().to_string(),
            true,
        ),
        entry(
            "qlu",
            "齐鲁工业大学镜像（BMCLAPI 协议）",
            DownloadSource::Qlu.base().to_string(),
            true,
        ),
        entry(
            "hit",
            "哈尔滨工业大学镜像（BMCLAPI 协议）",
            DownloadSource::Hit.base().to_string(),
            true,
        ),
        entry(
            "ha",
            "ha.edu.cn 镜像（BMCLAPI 协议）",
            DownloadSource::Ha.base().to_string(),
            true,
        ),
        entry(
            "ustc",
            "中国科学技术大学镜像（BMCLAPI 协议）",
            DownloadSource::Ustc.base().to_string(),
            true,
        ),
        entry(
            "nyist",
            "南阳理工学院镜像（BMCLAPI 协议）",
            DownloadSource::Nyist.base().to_string(),
            true,
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

/// 探测缓存是全局的：涉及缓存的测试必须串行，否则互相清空。
    fn cache_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(|err| err.into_inner())
    }

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
        // 9 个 BMCLAPI 镜像 + 官方
        let base = DownloadSource::MIRRORS.len() + 1;
        assert_eq!(auto_candidates("").len(), base);
        assert_eq!(auto_candidates("https://mirror.example.com").len(), base + 1);
        assert_eq!(auto_candidates("nope").len(), base, "非法自定义源不参与");
        let first = auto_candidates("");
        assert!(first.iter().any(|origin| origin.id() == "lzu"));
        assert_eq!(first.last().unwrap().id(), "official", "官方永远兜底排最后");
    }

    #[test]
    fn force_probe_overwrites_cache() {
        let _guard = cache_lock();
        clear_probe_cache();
        store_probe(&probe("lzu", true, 61));
        assert_eq!(cached_probe("lzu").unwrap().latency_ms, 61);
        // force 之后写回的是新值
        store_probe(&probe("lzu", true, 74));
        assert_eq!(cached_probe("lzu").unwrap().latency_ms, 74, "新结果覆盖旧缓存");
        clear_probe_cache();
    }

    #[test]
    fn force_ignores_probe_cache() {
        let cached = probe("lzu", true, 60);
        assert!(probe_cache_hit(false, Some(&cached)), "不 force 时命中缓存");
        assert!(!probe_cache_hit(true, Some(&cached)), "force 时不用缓存");
        assert!(!probe_cache_hit(false, None));
    }

    #[test]
    fn candidate_chain_puts_selected_first_and_official_last() {
        let _guard = cache_lock();
        clear_probe_cache();
        let mut settings = Settings::default();
        settings.download_source = "bmclapi".to_string();
        let chain = candidate_chain(&settings, OFFICIAL_MANIFEST);
        assert_eq!(chain[0].id(), "bmclapi", "指定的源排第一");
        assert_eq!(chain.last().unwrap().id(), "official", "官方永远兜底");
    }

    #[test]
    fn partial_mirrors_are_dropped_for_unsupported_urls() {
        let _guard = cache_lock();
        clear_probe_cache();
        let mut settings = Settings::default();
        settings.download_source = "hit".to_string();
        // 版本清单：部分镜像不支持 → 从候选里剔除
        let manifest_chain = candidate_chain(&settings, OFFICIAL_MANIFEST);
        assert!(
            !manifest_chain.iter().any(|origin| origin.id() == "hit"),
            "部分镜像不能出现在清单候选里：{:?}",
            manifest_chain.iter().map(|origin| origin.id()).collect::<Vec<_>>()
        );
        // libraries：部分镜像支持
        let library_chain = candidate_chain(
            &settings,
            "https://libraries.minecraft.net/com/google/gson/2.10/gson-2.10.jar",
        );
        assert_eq!(library_chain[0].id(), "hit");
        assert!(origin_supports(
            &Origin::Builtin(DownloadSource::Minguncle),
            OFFICIAL_MANIFEST
        ));
        assert!(!origin_supports(
            &Origin::Builtin(DownloadSource::Ustc),
            OFFICIAL_MANIFEST
        ));
        assert!(origin_supports(
            &Origin::Custom("https://mirror.example.com".to_string()),
            OFFICIAL_MANIFEST
        ));
    }

    #[test]
    fn cached_latency_orders_the_fallback_tail() {
        let probes = vec![
            probe("lzu", true, 60),
            probe("ustc", true, 900),
            probe("ha", false, 0),
        ];
        // 用 libraries 路径：部分镜像只代理这类文件
        let chain = order_candidates(builtin_order("official"), &probes, "https://libraries.minecraft.net/com/google/gson/2.10/gson-2.10.jar");
        let ids: Vec<&str> = chain.iter().map(|origin| origin.id()).collect();
        let lzu = ids.iter().position(|id| *id == "lzu").unwrap();
        let ustc = ids.iter().position(|id| *id == "ustc").unwrap();
        assert!(lzu < ustc, "延迟低的排前面：{ids:?}");
        assert!(!ids.contains(&"ha"), "探测不可用的不参与：{ids:?}");
        assert_eq!(*ids.last().unwrap(), "official");
    }

    #[test]
    fn augment_adds_probed_mirrors_and_keeps_official_last() {
        let probes = vec![probe("qlu", true, 30), probe("nyist", true, 200)];
        let chain = order_candidates(builtin_order("official"), &probes, "https://libraries.minecraft.net/com/google/gson/2.10/gson-2.10.jar");
        let ids: Vec<&str> = chain.iter().map(|origin| origin.id()).collect();
        assert!(ids.contains(&"qlu"), "{ids:?}");
        assert!(ids.contains(&"nyist"), "{ids:?}");
        let qlu = ids.iter().position(|id| *id == "qlu").unwrap();
        let nyist = ids.iter().position(|id| *id == "nyist").unwrap();
        assert!(qlu < nyist);
        assert_eq!(*ids.last().unwrap(), "official");
    }

    #[test]
    fn auto_prefers_lowest_latency_in_chain() {
        let probes = vec![
            probe("lzu", true, 55),
            probe("bmclapi", true, 480),
            probe("official", true, 120),
        ];
        // auto 走 order_by_probes 之后的链，这里直接验排序规则
        let ordered = order_by_probes(auto_candidates(""), &probes);
        let chain = order_candidates(ordered, &probes, "https://libraries.minecraft.net/com/google/gson/2.10/gson-2.10.jar");
        let ids: Vec<&str> = chain.iter().map(|origin| origin.id()).collect();
        assert_eq!(ids[0], "lzu", "auto 首选延迟最低：{ids:?}");
        assert_eq!(*ids.last().unwrap(), "official");
    }

    #[test]
    fn download_attempt_cap_is_four() {
        assert_eq!(MAX_DOWNLOAD_ATTEMPTS, 4);
        clear_probe_cache();
        for (index, source) in DownloadSource::MIRRORS.iter().enumerate() {
            store_probe(&probe(source.id(), true, 10 + index as u64));
        }
        let mut settings = Settings::default();
        settings.download_source = "bmclapi".to_string();
        let chain = candidate_chain(&settings, "https://libraries.minecraft.net/a/b.jar");
        assert!(chain.len() > MAX_DOWNLOAD_ATTEMPTS, "候选比上限多，下载器才会截断");
        clear_probe_cache();
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
        assert_eq!(ordered[0].id(), "bmclapi", "可用且最快排第一");
        assert_eq!(ordered[1].id(), "official");
        let custom_at = ordered.iter().position(|origin| origin.id() == "custom").unwrap();
        let tail = ordered.len() - 1;
        assert_eq!(custom_at, tail, "失败的源排最后");
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
        assert_eq!(table.len(), 5 + DownloadSource::MIRRORS.len() - 1, "官方+bmclapi+8 镜像+sjtu+custom+auto");
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