//! 更新下载用的 GitHub 加速节点表 + 探针 + 选最快节点。
//!
//! 节点清单来自 0.5.6 实机测速日志（2026-10-26 当天，每节点 32 次探针）：
//!```text
//!node         ok fail avg(ms) failRate
//!ghf          29    3   591      9%
//!ghfast       32    0   311      0%
//!cnxiaobai    32    0   484      0%
//!tangbai      32    0    98      0%
//!chenc        32    0   258      0%
//!xyz          32    0   258      0%
//!```
//! `padao`(https://gh.padao.fun/) 0.5.6 实测 31/32 次失败，已移除。

use std::time::{Duration, Instant};

use serde::Serialize;

/// 一个加速节点。
#[derive(Debug, Clone)]
pub struct GithubNode {
    pub id: &'static str,
    pub label: &'static str,
    pub base: &'static str,
}

/// 节点表（顺序固定：UI 顺序与全部失败时的兜底都依赖它）。
pub const GITHUB_NODES: [GithubNode; 6] = [
    GithubNode { id: "ghf", label: "GHF", base: "https://ghf.无名氏.top/" },
    GithubNode { id: "ghfast", label: "GHFast", base: "https://ghfast.top/" },
    GithubNode { id: "cnxiaobai", label: "CnxiaoBai", base: "https://github.cnxiaobai.com/" },
    GithubNode { id: "tangbai", label: "TangBai", base: "https://git.tangbai.cc/" },
    GithubNode { id: "chenc", label: "Chenc", base: "https://github.chenc.dev/" },
    GithubNode { id: "xyz", label: "XYZ", base: "https://30006000.xyz/" },
];

/// 探针目标：用自己仓库的 releases 页面，避免依赖第三方文件。
pub const PROBE_TARGET: &str = "https://github.com/JIJIGUOWANG123/Gugu-Launcher";

/// 每个节点探几次（取最小耗时）。
pub const PROBE_ATTEMPTS: usize = 2;
/// 单次探针超时。
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
/// 探针结果缓存时长；更新下载前会强制重测，忽略这个缓存。
pub const PROBE_CACHE_TTL: Duration = Duration::from_secs(300);

pub fn find_node(id: &str) -> Option<&'static GithubNode> {
    GITHUB_NODES.iter().find(|node| node.id == id)
}

/// 设置项校验：`auto` 或节点表里的 id。
pub fn is_known_accelerator(value: &str) -> bool {
    value == "auto" || find_node(value).is_some()
}

/// 加速 URL 规则（与 0.5.6 一致）：节点 base + 原 GitHub 完整 URL。
pub fn accelerated_url(base: &str, github_url: &str) -> String {
    format!("{base}{github_url}")
}

/// 一次探针的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeResult {
    pub node: &'static str,
    pub ok: bool,
    pub elapsed_ms: u64,
}

/// 选出最快且测通的节点：只考虑 ok，按耗时升序、同耗时按声明顺序。
/// 全部失败时回落到声明顺序里第一个出现在结果中的节点（结果为空则回落首个节点）。
pub fn pick_fastest(results: &[ProbeResult]) -> Option<&'static str> {
    let order_of = |id: &str| GITHUB_NODES.iter().position(|node| node.id == id).unwrap_or(usize::MAX);

    let mut best: Option<(&'static str, u64, usize)> = None;
    for result in results.iter().filter(|result| result.ok) {
        let order = order_of(result.node);
        best = match best {
            None => Some((result.node, result.elapsed_ms, order)),
            Some((_, best_ms, best_order)) => {
                if (result.elapsed_ms, order) < (best_ms, best_order) {
                    Some((result.node, result.elapsed_ms, order))
                } else {
                    best
                }
            }
        };
    }
    if let Some((id, _, _)) = best {
        return Some(id);
    }

    GITHUB_NODES
        .iter()
        .map(|node| node.id)
        .find(|id| results.iter().any(|result| result.node == *id))
        .or(Some(GITHUB_NODES[0].id))
}

/// 检查 Releases API 时的节点顺序：只保留测通的，按耗时升序。
/// 直连 api.github.com 永远是首选（加速节点不一定代理 api 子域），失败的节点不参与。
pub fn order_ok_by_latency(results: &[ProbeResult]) -> Vec<&'static str> {
    let mut ok: Vec<&ProbeResult> = results.iter().filter(|result| result.ok).collect();
    ok.sort_by_key(|result| {
        let order = GITHUB_NODES
            .iter()
            .position(|node| node.id == result.node)
            .unwrap_or(usize::MAX);
        (result.elapsed_ms, order)
    });
    ok.into_iter().map(|result| result.node).collect()
}

/// 探针结果缓存（5 分钟）。更新下载前要强制重测，所以必须能 `clear()`。
#[derive(Debug, Default)]
pub struct ProbeCache {
    entries: Vec<ProbeResult>,
    at: Option<Instant>,
}

impl ProbeCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// 命中且未过期才返回缓存。
    pub fn get(&self) -> Option<&[ProbeResult]> {
        let at = self.at?;
        if at.elapsed() > PROBE_CACHE_TTL {
            return None;
        }
        if self.entries.is_empty() {
            return None;
        }
        Some(&self.entries)
    }

    pub fn set(&mut self, results: Vec<ProbeResult>) {
        self.entries = results;
        self.at = Some(Instant::now());
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.at = None;
    }
}

/// 单节点探针：最多试 `PROBE_ATTEMPTS` 次，取最小耗时。
pub async fn probe_node(client: &reqwest::Client, node: &'static GithubNode, target: &str) -> ProbeResult {
    let url = accelerated_url(node.base, target);
    let mut best: Option<u64> = None;
    for _ in 0..PROBE_ATTEMPTS {
        let started = Instant::now();
        let ok = tokio::time::timeout(PROBE_TIMEOUT, async {
            client
                .head(&url)
                .timeout(PROBE_TIMEOUT)
                .send()
                .await
                .map(|response| response.status().is_success() || response.status().is_redirection())
                .unwrap_or(false)
        })
        .await
        .unwrap_or(false);
        if ok {
            let elapsed = started.elapsed().as_millis() as u64;
            best = Some(best.map_or(elapsed, |current: u64| current.min(elapsed)));
        }
    }
    ProbeResult {
        node: node.id,
        ok: best.is_some(),
        elapsed_ms: best.unwrap_or(u64::MAX),
    }
}

/// 并发测所有节点（每节点 2 次取最小）。失败节点排在最后（`elapsed_ms = u64::MAX`）。
pub async fn probe_all(client: &reqwest::Client) -> Vec<ProbeResult> {
    let mut set = tokio::task::JoinSet::new();
    for node in GITHUB_NODES.iter() {
        let client = client.clone();
        set.spawn(async move { probe_node(&client, node, PROBE_TARGET).await });
    }
    let mut results = Vec::with_capacity(GITHUB_NODES.len());
    while let Some(joined) = set.join_next().await {
        if let Ok(result) = joined {
            results.push(result);
        }
    }
    results.sort_by_key(|result| {
        let order = GITHUB_NODES
            .iter()
            .position(|node| node.id == result.node)
            .unwrap_or(usize::MAX);
        (if result.ok { 0 } else { 1 }, result.elapsed_ms, order)
    });
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_table_is_exactly_six_and_without_padao() {
        assert_eq!(GITHUB_NODES.len(), 6);
        let ids: Vec<&str> = GITHUB_NODES.iter().map(|node| node.id).collect();
        assert_eq!(ids, vec!["ghf", "ghfast", "cnxiaobai", "tangbai", "chenc", "xyz"]);
        assert!(!ids.contains(&"padao"));
        assert!(GITHUB_NODES.iter().all(|node| !node.base.contains("padao")));
        assert!(GITHUB_NODES.iter().all(|node| node.base.ends_with('/')));
    }

    #[test]
    fn settings_guard_accepts_auto_and_known_ids_only() {
        assert!(is_known_accelerator("auto"));
        assert!(is_known_accelerator("ghfast"));
        assert!(!is_known_accelerator("padao"));
        assert!(!is_known_accelerator(""));
    }

    #[test]
    fn pick_fastest_prefers_ok_and_lowest_latency() {
        let results = vec![
            ProbeResult { node: "ghf", ok: true, elapsed_ms: 591 },
            ProbeResult { node: "ghfast", ok: true, elapsed_ms: 311 },
            ProbeResult { node: "tangbai", ok: true, elapsed_ms: 98 },
            ProbeResult { node: "chenc", ok: false, elapsed_ms: u64::MAX },
            ProbeResult { node: "xyz", ok: true, elapsed_ms: 9999 },
            ProbeResult { node: "cnxiaobai", ok: true, elapsed_ms: 484 },
        ];
        assert_eq!(pick_fastest(&results), Some("tangbai"));

        // 同耗时按声明顺序
        let tie = vec![
            ProbeResult { node: "xyz", ok: true, elapsed_ms: 258 },
            ProbeResult { node: "chenc", ok: true, elapsed_ms: 258 },
        ];
        assert_eq!(pick_fastest(&tie), Some("chenc"));
    }

    #[test]
    fn pick_fastest_falls_back_to_declaration_order_when_all_fail() {
        let results = vec![
            ProbeResult { node: "xyz", ok: false, elapsed_ms: u64::MAX },
            ProbeResult { node: "ghf", ok: false, elapsed_ms: u64::MAX },
        ];
        assert_eq!(pick_fastest(&results), Some("ghf"), "必须按节点声明顺序兜底");
        assert_eq!(pick_fastest(&[]), Some(GITHUB_NODES[0].id));
    }

    #[test]
    fn accelerated_url_is_base_plus_original_url() {
        let url = "https://github.com/JIJIGUOWANG123/Gugu-Launcher/releases/download/v1.0.1/GuguLauncher-Setup.exe";
        assert_eq!(
            accelerated_url("https://ghfast.top/", url),
            format!("https://ghfast.top/{url}")
        );
        assert!(accelerated_url(find_node("tangbai").unwrap().base, url).starts_with("https://git.tangbai.cc/https://github.com/"));
    }

    #[test]
    fn api_order_keeps_only_ok_nodes_sorted_by_latency() {
        let results = vec![
            ProbeResult { node: "ghf", ok: true, elapsed_ms: 591 },
            ProbeResult { node: "tangbai", ok: true, elapsed_ms: 98 },
            ProbeResult { node: "xyz", ok: false, elapsed_ms: u64::MAX },
        ];
        assert_eq!(order_ok_by_latency(&results), vec!["tangbai", "ghf"]);
        assert!(order_ok_by_latency(&[]).is_empty());
    }

    #[test]
    fn cache_expires() {
        let mut cache = ProbeCache::new();
        assert!(cache.get().is_none());
        cache.set(vec![ProbeResult { node: "ghf", ok: true, elapsed_ms: 10 }]);
        assert!(cache.get().is_some());
        cache.clear();
        assert!(cache.get().is_none());
    }
}
