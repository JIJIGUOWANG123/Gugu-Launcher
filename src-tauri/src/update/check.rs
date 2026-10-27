//! 更新检查：GitHub Releases → 版本判定 → 资产选择 → 状态机。
//!
//! 1.0.0 的策略（用户确认）：启动后延迟 10s 静默查一次，之后每 6 小时一次；
//! 全程**不弹窗、不发系统通知**，结果只反映在「关于」页的一行状态文字上。
//! 设置 updateMode：
//!   auto   —— 发现新版本静默下载并安装（M2 补完下载失败的自动换节点与进度）；
//!   manual —— 只在关于页提示，用户点按钮才安装；
//!   off    —— 完全不请求 GitHub（连 releases API 都不发）。

use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::net::github::{self, ProbeCache, ProbeResult};
use crate::update::version::{self, AssetKind, ReleaseAsset, ReleaseInfo};

/// 启动后延迟多久做第一次检查（不阻塞首屏）。
pub const STARTUP_DELAY: Duration = Duration::from_secs(10);
/// 之后的检查间隔。
pub const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
/// Releases API（latest 只取最新一条）。
pub const RELEASES_API: &str = "https://api.github.com/repos/JIJIGUOWANG123/Gugu-Launcher/releases/latest";
/// 安装包固定文件名（资产选择只认它）。
pub const INSTALLER_NAME: &str = "GuguLauncher-Setup.exe";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    Idle,
    Checking,
    UpToDate,
    Available,
    Downloading,
    Installing,
    Failed,
}

/// 给「关于」页显示的一行状态。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    pub phase: Phase,
    pub current_version: String,
    pub remote_version: Option<String>,
    pub message: String,
    pub checked_at: Option<u64>,
    /// manual 模式下为 true：关于页出现「立即更新」按钮
    pub can_install: bool,
}

impl UpdateStatus {
    pub fn new(current: &str, phase: Phase, message: impl Into<String>) -> Self {
        Self {
            phase,
            current_version: current.to_string(),
            remote_version: None,
            message: message.into(),
            checked_at: None,
            can_install: false,
        }
    }
}

/// 纯函数：off 之外才允许发请求。
pub fn should_request(mode: &str) -> bool {
    !matches!(mode, "off")
}

/// 纯函数：拿到判定结果后该做什么。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// 不请求、不下载、不提示
    Skip,
    /// 只提示（manual）
    Notify,
    /// 直接静默下载安装（auto）
    Download,
}

pub fn plan_action(mode: &str, available: bool, has_installer: bool) -> Action {
    if !should_request(mode) {
        return Action::Skip;
    }
    if !available || !has_installer {
        return Action::Skip;
    }
    match mode {
        "auto" => Action::Download,
        "manual" => Action::Notify,
        _ => Action::Skip,
    }
}

/// Release 里与「要不要更新」相关的字段（GitHub API 结构子集）。
#[derive(Debug, Clone, Deserialize)]
pub struct ApiRelease {
    #[serde(default)]
    pub tag_name: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    /// RFC3339，例如 2026-10-27T05:30:34Z
    #[serde(default)]
    pub published_at: Option<String>,
    #[serde(default)]
    pub assets: Vec<ReleaseAsset>,
}

/// 一次判定的结果。
#[derive(Debug, Clone)]
pub enum Outcome {
    UpToDate {
        remote: Option<String>,
    },
    Available {
        remote: String,
        asset: ReleaseAsset,
    },
    /// 有新版本但没有可用安装包 → 跳过，不报错
    NoAsset {
        remote: Option<String>,
    },
}

/// 纯函数：解析 RFC3339 UTC 时间戳（只支持 ...Z 结尾的形式）。
pub fn parse_rfc3339_utc(text: &str) -> Option<i64> {
    let trimmed = text.trim();
    let core = trimmed.strip_suffix('Z')?;
    let (date, time) = core.split_once('T')?;
    let mut d = date.split('-');
    let year: i64 = d.next()?.parse().ok()?;
    let month: i64 = d.next()?.parse().ok()?;
    let day: i64 = d.next()?.parse().ok()?;
    let mut t = time.split(':');
    // 允许 "05:30:34" 与 "05:30:34.123" 两种写法（小数秒直接丢掉）
    let hour: i64 = t.next()?.split('.').next()?.parse().ok()?;
    let minute: i64 = t.next()?.split('.').next()?.parse().ok()?;
    let second: i64 = t.next()?.split('.').next()?.parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    // 天数换算：civil_from_days 的逆运算（Howard Hinnant 算法）
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + hour * 3_600 + minute * 60 + second)
}

/// 纯函数：版本判定 + 资产选择。
pub fn evaluate(current: &str, release: &ApiRelease, build_time: Option<i64>) -> Outcome {
    let info = ReleaseInfo {
        tag_name: release.tag_name.clone(),
        name: release.name.clone(),
        published_at: release.published_at.as_deref().and_then(parse_rfc3339_utc),
    };
    let decision = version::decide(current, &info, build_time);
    if !decision.available {
        return Outcome::UpToDate {
            remote: decision.version,
        };
    }
    match version::pick_asset(&release.assets) {
        Some((AssetKind::Installer, asset)) => Outcome::Available {
            remote: decision.version.unwrap_or_else(|| "未知版本".to_string()),
            asset: asset.clone(),
        },
        // 只发安装包：没有 GuguLauncher-Setup.exe 就当该版本不可用，跳过
        _ => Outcome::NoAsset {
            remote: decision.version,
        },
    }
}

/// API 候选地址顺序：先直连 api.github.com，再依次试加速节点（按测速结果升序）。
pub fn api_candidates(results: &[ProbeResult]) -> Vec<String> {
    let mut urls = vec![RELEASES_API.to_string()];
    for id in github::order_ok_by_latency(results) {
        if let Some(node) = github::find_node(id) {
            urls.push(format!("{}{}", node.base, RELEASES_API));
        }
    }
    urls
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 拉 Releases：直连失败就换加速节点，每次 8s 超时；全部失败返回 None（安静失败）。
pub async fn fetch_release(client: &reqwest::Client, probes: &Mutex<ProbeCache>) -> Option<ApiRelease> {
    // 注意：MutexGuard 不能跨 await 持有（否则整个 future 不是 Send），
    // 所以先取缓存并把锁释放，再 await 探测，最后重新加锁写回。
    let cached = probes.lock().ok().and_then(|cache| cache.get().map(|entries| entries.to_vec()));
    let results = match cached {
        Some(hit) => hit,
        None => {
            // 探测会发 6 个请求；失败也照常继续，直连仍然是首选
            let fresh = github::probe_all(client).await;
            if let Ok(mut cache) = probes.lock() {
                cache.set(fresh.clone());
            }
            fresh
        }
    };

    for url in api_candidates(&results) {
        match client
            .get(&url)
            .header("Accept", "application/vnd.github+json")
            .timeout(Duration::from_secs(8))
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => match response.json::<ApiRelease>().await {
                Ok(release) => return Some(release),
                Err(err) => tracing::warn!(%url, %err, "Releases JSON 解析失败，换下一个候选"),
            },
            Ok(response) => {
                // 404 = 仓库还没有 Release（1.0.0 的实际情况）：不是错误，直接判「已是最新」
                if response.status() == reqwest::StatusCode::NOT_FOUND {
                    tracing::info!(%url, "仓库暂无 Release（404），视为已是最新");
                    return None;
                }
                tracing::warn!(%url, status = %response.status(), "Releases 请求失败，换下一个候选");
            }
            Err(err) => tracing::warn!(%url, %err, "Releases 请求异常，换下一个候选"),
        }
    }
    tracing::warn!("所有候选节点都失败：本次更新检查安静失败");
    None
}

/// 状态机的一次完整检查（供后台任务与 update_check 命令共用）。
pub async fn run(
    client: &reqwest::Client,
    probes: &Mutex<ProbeCache>,
    status: &Mutex<UpdateStatus>,
    mode: &str,
    current: &str,
    build_time: Option<i64>,
) -> UpdateStatus {
    run_with_asset(client, probes, status, mode, current, build_time)
        .await
        .0
}

/// 同 run，但把「要装的那个安装包」一起返回，update_install 直接用，省一次 GitHub 请求。
pub async fn run_with_asset(
    client: &reqwest::Client,
    probes: &Mutex<ProbeCache>,
    status: &Mutex<UpdateStatus>,
    mode: &str,
    current: &str,
    build_time: Option<i64>,
) -> (UpdateStatus, Option<ReleaseAsset>) {
    let set = |next: UpdateStatus| -> UpdateStatus {
        if let Ok(mut guard) = status.lock() {
            *guard = next.clone();
        }
        next
    };

    if !should_request(mode) {
        return (
            set(UpdateStatus::new(current, Phase::Idle, "已关闭更新检查")),
            None,
        );
    }

    set(UpdateStatus::new(current, Phase::Checking, "正在检查更新…"));
    let Some(release) = fetch_release(client, probes).await else {
        // 没有 Release 或网络失败：都按「已是最新」安静处理，不显示错误
        let mut next = UpdateStatus::new(current, Phase::UpToDate, format!("已是最新（{current}）"));
        next.checked_at = Some(now_secs());
        return (set(next), None);
    };

    match evaluate(current, &release, build_time) {
        Outcome::UpToDate { remote } => {
            let mut next =
                UpdateStatus::new(current, Phase::UpToDate, format!("已是最新（{current}）"));
            next.remote_version = remote;
            next.checked_at = Some(now_secs());
            (set(next), None)
        }
        Outcome::NoAsset { remote } => {
            let mut next =
                UpdateStatus::new(current, Phase::UpToDate, "该版本没有可用资产，已跳过");
            next.remote_version = remote;
            next.checked_at = Some(now_secs());
            (set(next), None)
        }
        Outcome::Available { remote, asset } => {
            let manual = mode == "manual";
            let mut next = UpdateStatus::new(
                current,
                Phase::Available,
                if manual {
                    format!("发现 {remote}，可立即更新")
                } else {
                    format!("发现 {remote}，正在准备静默更新")
                },
            );
            next.remote_version = Some(remote);
            next.can_install = true;
            next.checked_at = Some(now_secs());
            next.message = format!("{}（{}）", next.message, asset.name);
            (set(next), Some(asset))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(name: &str) -> ReleaseAsset {
        ReleaseAsset {
            name: name.to_string(),
            browser_download_url: format!("https://example.com/{name}"),
            size: Some(10),
        }
    }

    fn release(tag: &str, assets: Vec<ReleaseAsset>) -> ApiRelease {
        ApiRelease {
            tag_name: Some(tag.to_string()),
            name: Some(tag.to_string()),
            published_at: Some("2026-10-27T05:30:34Z".to_string()),
            assets,
        }
    }

    #[test]
    fn newer_release_is_detected() {
        let rel = release("v1.0.1", vec![asset("GuguLauncher-Setup.exe")]);
        match evaluate("1.0.0", &rel, None) {
            Outcome::Available { remote, asset } => {
                assert_eq!(remote, "1.0.1");
                assert_eq!(asset.name, "GuguLauncher-Setup.exe");
            }
            other => panic!("应判定为有新版本，实际 {other:?}"),
        }
    }

    #[test]
    fn same_or_older_release_is_not_an_update() {
        let same = release("1.0.0", vec![asset("GuguLauncher-Setup.exe")]);
        assert!(matches!(evaluate("1.0.0", &same, None), Outcome::UpToDate { .. }));
        let older = release("v0.9.9", vec![asset("GuguLauncher-Setup.exe")]);
        assert!(matches!(evaluate("1.0.0", &older, None), Outcome::UpToDate { .. }));
    }

    #[test]
    fn missing_installer_asset_is_skipped() {
        let rel = release("1.0.1", vec![asset("source.zip"), asset("notes.txt")]);
        assert!(matches!(evaluate("1.0.0", &rel, None), Outcome::NoAsset { .. }));
        // 只有裸 exe / portable zip 也不算可用
        let rel = release("1.0.1", vec![asset("GuguLauncher.exe"), asset("GuguLauncher-portable-x64.zip")]);
        assert!(matches!(evaluate("1.0.0", &rel, None), Outcome::NoAsset { .. }));
    }

    #[test]
    fn no_update_never_triggers_download() {
        assert_eq!(plan_action("auto", false, false), Action::Skip);
        assert_eq!(plan_action("manual", false, true), Action::Skip);
        assert_eq!(plan_action("auto", true, false), Action::Skip, "缺安装包不下");
        assert_eq!(plan_action("auto", true, true), Action::Download);
        assert_eq!(plan_action("manual", true, true), Action::Notify);
    }

    #[test]
    fn off_mode_sends_no_request() {
        assert!(!should_request("off"));
        assert!(should_request("auto"));
        assert!(should_request("manual"));
        assert_eq!(plan_action("off", true, true), Action::Skip);
        assert_eq!(plan_action("off", false, false), Action::Skip);
    }

    #[test]
    fn rfc3339_parsing() {
        assert_eq!(parse_rfc3339_utc("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_rfc3339_utc("2026-10-27T05:30:34Z"), Some(1_793_079_034));
        assert_eq!(parse_rfc3339_utc("2026-10-27T05:30:34.123Z"), Some(1_793_079_034));
        assert_eq!(parse_rfc3339_utc("not-a-date"), None);
        assert_eq!(parse_rfc3339_utc("2026-13-01T00:00:00Z"), None);
    }

    #[test]
    fn api_candidates_try_direct_first_then_probed_nodes() {
        let results = vec![
            ProbeResult { node: "ghf", ok: true, elapsed_ms: 500 },
            ProbeResult { node: "tangbai", ok: true, elapsed_ms: 90 },
            ProbeResult { node: "xyz", ok: false, elapsed_ms: u64::MAX },
        ];
        let urls = api_candidates(&results);
        assert_eq!(urls[0], RELEASES_API, "先直连");
        assert_eq!(urls[1], format!("https://git.tangbai.cc/{RELEASES_API}"), "再按耗时升序");
        assert!(!urls.iter().any(|url| url.contains("30006000.xyz")), "测速失败的节点不参与");
    }
}
