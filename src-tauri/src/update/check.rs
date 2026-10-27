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

/// API 候选地址顺序：先直连 api.github.com，再测速通过的节点（按延迟升序），
/// 最后是**其余所有节点**——测速通过只代表首页能通，Releases 可能还是 403，
/// 全列上才不会因为「只挑到一个坏节点」而整次检查失败。
pub fn api_candidates(results: &[ProbeResult]) -> Vec<String> {
    let mut urls = vec![RELEASES_API.to_string()];
    let mut seen: Vec<&'static str> = Vec::new();
    for id in github::order_ok_by_latency(results) {
        if let Some(node) = github::find_node(id) {
            urls.push(format!("{}{}", node.base, RELEASES_API));
            seen.push(node.id);
        }
    }
    for node in github::GITHUB_NODES.iter() {
        if seen.contains(&node.id) {
            continue;
        }
        urls.push(format!("{}{}", node.base, RELEASES_API));
    }
    urls
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 一次候选尝试的结果（把网络细节抽出来，便于纯函数判定）。
#[derive(Debug, Clone)]
pub enum Attempt {
    /// 拿到并解析出 Release
    Json(ApiRelease),
    /// 仓库确实没有 Release（404）
    NotFound,
    /// HTTP 非 2xx（非 404）
    Status(u16),
    /// 网络异常（超时/连接失败）
    Error(String),
    /// 2xx 但 JSON 解析失败
    Parse(String),
}

/// 拉取结论：区分「仓库没发布」与「网络失败」。
#[derive(Debug, Clone)]
pub enum FetchOutcome {
    Found(ApiRelease),
    /// 仓库还没有 Release（404）：不是错误
    NoRelease,
    /// 所有候选节点都失败
    Failed { reason: String },
}

/// 纯函数：把一串候选尝试折成一个结论。
pub fn decide_fetch(attempts: Vec<Attempt>) -> FetchOutcome {
    let mut last_error = String::new();
    let mut saw_not_found = false;
    for attempt in attempts {
        match attempt {
            Attempt::Json(release) => return FetchOutcome::Found(release),
            Attempt::NotFound => saw_not_found = true,
            Attempt::Status(status) => {
                last_error = format!("HTTP {status}");
            }
            Attempt::Error(err) => last_error = err,
            Attempt::Parse(err) => last_error = format!("响应解析失败：{err}"),
        }
    }
    // 只要有一个候选明确回 404（仓库没发布），就不算网络失败
    if saw_not_found && last_error.is_empty() {
        FetchOutcome::NoRelease
    } else if saw_not_found {
        // 既有 404 也有别的错误：按「没发布」处理更贴近事实
        FetchOutcome::NoRelease
    } else if last_error.is_empty() {
        FetchOutcome::Failed {
            reason: "没有任何可用候选节点".to_string(),
        }
    } else {
        FetchOutcome::Failed {
            reason: last_error,
        }
    }
}

/// 拉 Releases 并给出区分过的结论（日志逐节点打点）。
pub async fn fetch_release_detailed(
    client: &reqwest::Client,
    probes: &Mutex<ProbeCache>,
) -> FetchOutcome {
    let cached = probes
        .lock()
        .ok()
        .and_then(|cache| cache.get().map(|entries| entries.to_vec()));
    let results = match cached {
        Some(hit) => hit,
        None => {
            let fresh = github::probe_all(client).await;
            if let Ok(mut cache) = probes.lock() {
                cache.set(fresh.clone());
            }
            fresh
        }
    };
    let candidates = api_candidates(&results);
    tracing::info!(candidates = candidates.len(), "开始拉取 GitHub Releases");
    let mut attempts: Vec<Attempt> = Vec::new();
    for url in candidates {
        let started = std::time::Instant::now();
        let outcome = match client
            .get(&url)
            .header("Accept", "application/vnd.github+json")
            .timeout(Duration::from_secs(8))
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => {
                let status = response.status().as_u16();
                match response.json::<ApiRelease>().await {
                    Ok(release) => {
                        tracing::info!(
                            %url,
                            status,
                            elapsed_ms = started.elapsed().as_millis() as u64,
                            tag = ?release.tag_name,
                            "Releases 拉取成功"
                        );
                        Attempt::Json(release)
                    }
                    Err(err) => {
                        tracing::warn!(
                            %url,
                            status,
                            elapsed_ms = started.elapsed().as_millis() as u64,
                            %err,
                            "Releases JSON 解析失败"
                        );
                        Attempt::Parse(err.to_string())
                    }
                }
            }
            Ok(response) => {
                let status = response.status().as_u16();
                let attempt = if response.status() == reqwest::StatusCode::NOT_FOUND {
                    tracing::info!(
                        %url,
                        status,
                        elapsed_ms = started.elapsed().as_millis() as u64,
                        "仓库还没有 Release（404）"
                    );
                    Attempt::NotFound
                } else {
                    tracing::warn!(
                        %url,
                        status,
                        elapsed_ms = started.elapsed().as_millis() as u64,
                        "Releases 返回非 2xx"
                    );
                    Attempt::Status(status)
                };
                attempt
            }
            Err(err) => {
                tracing::warn!(
                    %url,
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    %err,
                    "Releases 请求异常"
                );
                Attempt::Error(err.to_string())
            }
        };
        if matches!(outcome, Attempt::Json(_)) {
            attempts.push(outcome);
            return decide_fetch(attempts);
        }
        attempts.push(outcome);
    }
    let verdict = decide_fetch(attempts);
    match &verdict {
        FetchOutcome::Found(release) => tracing::info!(tag = ?release.tag_name, "Releases 结论：已取到"),
        FetchOutcome::NoRelease => tracing::info!("Releases 结论：仓库还没有发布版本"),
        FetchOutcome::Failed { reason } => tracing::warn!(%reason, "Releases 结论：所有候选都失败"),
    }
    verdict
}

/// 拉 Releases：直连失败就换加速节点，每次 8s 超时；全部失败返回 None（只有「没发布」或「取到了」才可能返回 Some/None 组合，见 fetch_release_detailed）。
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

    tracing::info!(current, mode, "更新检查开始（current=<版本> mode=<模式>）");
    set(UpdateStatus::new(current, Phase::Checking, "正在检查更新…"));
    let release = match fetch_release_detailed(client, probes).await {
        FetchOutcome::Found(release) => release,
        FetchOutcome::NoRelease => {
            // 仓库确实没发布：这才是「已是最新」，但文案要说清原因
            let mut next = UpdateStatus::new(
                current,
                Phase::UpToDate,
                "仓库还没有发布版本".to_string(),
            );
            next.checked_at = Some(now_secs());
            tracing::info!(
                current,
                phase = "upToDate",
                message = %next.message,
                "更新检查结论"
            );
            return (set(next), None);
        }
        FetchOutcome::Failed { reason } => {
            // 网络/限流/节点全挂：必须报失败，绝不能伪装成「已是最新」
            let mut next = UpdateStatus::new(
                current,
                Phase::Failed,
                format!("检查更新失败：{reason}（可重试）"),
            );
            next.can_install = false;
            next.checked_at = Some(now_secs());
            tracing::warn!(
                current,
                reason = %reason,
                message = %next.message,
                "更新检查结论"
            );
            return (set(next), None);
        }
    };

    match evaluate(current, &release, build_time) {
        Outcome::UpToDate { remote } => {
            let mut next =
                UpdateStatus::new(current, Phase::UpToDate, format!("已是最新（{current}）"));
            next.remote_version = remote;
            next.checked_at = Some(now_secs());
            tracing::info!(
                current,
                phase = "upToDate",
                remote = ?next.remote_version,
                message = %next.message,
                "更新检查结论"
            );
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
            tracing::info!(
                current,
                phase = "available",
                remote = ?next.remote_version,
                asset = %asset.name,
                message = %next.message,
                "更新检查结论"
            );
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
    fn api_candidates_cover_every_node() {
        let urls = api_candidates(&[]);
        assert_eq!(urls[0], RELEASES_API, "直连永远排第一");
        assert_eq!(urls.len(), 1 + github::GITHUB_NODES.len(), "直连 + 全部节点");
        assert!(urls.iter().skip(1).all(|url| url.ends_with(RELEASES_API)));
        let probes = vec![
            ProbeResult {
                node: "tangbai",
                ok: true,
                elapsed_ms: 120,
            },
            ProbeResult {
                node: "ghf",
                ok: true,
                elapsed_ms: 40,
            },
        ];
        let urls = api_candidates(&probes);
        assert!(urls[1].contains("ghf"), "最快排第一：{}", urls[1]);
        assert!(urls[2].contains("tangbai"));
        assert_eq!(urls.len(), 1 + github::GITHUB_NODES.len(), "其余节点仍要作为兜底");
        let unique: std::collections::HashSet<&String> = urls.iter().collect();
        assert_eq!(unique.len(), urls.len(), "不能有重复候选");
    }

    #[test]
    fn fetch_404_means_no_release_not_latest() {
        let verdict = decide_fetch(vec![Attempt::NotFound]);
        assert!(matches!(verdict, FetchOutcome::NoRelease), "{verdict:?}");
    }

    #[test]
    fn fetch_all_candidates_failing_is_an_error() {
        let verdict = decide_fetch(vec![
            Attempt::Error("error sending request for url".to_string()),
            Attempt::Status(502),
        ]);
        match verdict {
            FetchOutcome::Failed { reason } => {
                assert!(reason.contains("502"), "{reason}");
            }
            other => panic!("必须报失败，实际 {other:?}"),
        }
        // 一个候选都没有
        assert!(matches!(decide_fetch(Vec::new()), FetchOutcome::Failed { .. }));
    }

    #[test]
    fn fetch_first_json_wins() {
        let rel = release("1.0.1", vec![asset("GuguLauncher-Setup.exe")]);
        let verdict = decide_fetch(vec![
            Attempt::Error("timeout".to_string()),
            Attempt::Json(rel),
        ]);
        assert!(matches!(verdict, FetchOutcome::Found(_)), "{verdict:?}");
    }

    #[test]
    fn status_messages_distinguish_the_four_cases() {
        // 用与 run_with_asset 相同的文案规则做纯函数级校验
        let no_release = UpdateStatus::new("1.0.0", Phase::UpToDate, "仓库还没有发布版本");
        assert_eq!(no_release.message, "仓库还没有发布版本");
        let failed = UpdateStatus::new(
            "1.0.0",
            Phase::Failed,
            format!("检查更新失败：{}（可重试）", "HTTP 502"),
        );
        assert!(failed.message.contains("可重试"));
        assert_eq!(failed.current_version, "1.0.0");
        let up = UpdateStatus::new("1.0.0", Phase::UpToDate, format!("已是最新（{}）", "1.0.0"));
        assert_eq!(up.message, "已是最新（1.0.0）");
        // Available 判定仍走 evaluate
        let rel = release("1.0.1", vec![asset("GuguLauncher-Setup.exe")]);
        match evaluate("1.0.0", &rel, None) {
            Outcome::Available { remote, .. } => assert_eq!(remote, "1.0.1"),
            other => panic!("{other:?}"),
        }
        let same = release("1.0.1", vec![asset("GuguLauncher-Setup.exe")]);
        assert!(matches!(evaluate("1.0.1", &same, None), Outcome::UpToDate { .. }));
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
        // 测速失败的节点也保留在候选末尾：测速只测首页，Releases 可能反而能用
        let xyz = urls.iter().position(|url| url.contains("30006000.xyz"));
        assert!(xyz.is_some(), "测速失败的节点仍要作为兜底候选");
        assert!(xyz.unwrap() > urls.iter().position(|url| url.contains("ghf")).unwrap());
    }
}
