//! 版本安装：版本 json（含 inheritsFrom）→ client jar + libraries + natives + assets。
//!
//! 所有网络动作都经过 downloader::Fetcher（多源回退），进度走 tasks::TaskHandle，
//! 事件由调用方注入的 events::Events 发出，因此这套逻辑在 examples 里也能直接跑。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use serde::Serialize;
use tokio::sync::Semaphore;

use crate::downloader::Fetcher;
use crate::error::{AppError, Result};
use crate::events::{Events, Phase};
use crate::meta;
use crate::paths::Paths;
use crate::sources::Origin;
use crate::tasks::TaskHandle;
use crate::version::{self, MergedVersion, Platform, VersionJson};
use crate::util;

/// 没设置时的并发兜底（任务书默认 8）。
pub const DEFAULT_CONCURRENCY: usize = 8;
/// inheritsFrom 链的最大深度，防止环。
const MAX_CHAIN_DEPTH: usize = 5;

pub fn version_dir(paths: &Paths, version_id: &str) -> PathBuf {
    paths.versions.join(version_id)
}

pub fn version_json_path(paths: &Paths, version_id: &str) -> PathBuf {
    version_dir(paths, version_id).join(format!("{version_id}.json"))
}

pub fn client_jar_path(paths: &Paths, version_id: &str) -> PathBuf {
    version_dir(paths, version_id).join(format!("{version_id}.jar"))
}

pub fn natives_dir(paths: &Paths, version_id: &str) -> PathBuf {
    version_dir(paths, version_id).join("natives")
}

pub fn assets_index_path(paths: &Paths, index_id: &str) -> PathBuf {
    paths.assets.join("indexes").join(format!("{index_id}.json"))
}

/// 安装结果（version_install 的返回值）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallReport {
    pub version_id: String,
    pub client_jar: Option<String>,
    pub libraries: usize,
    pub natives: usize,
    pub assets_total: usize,
    pub assets_downloaded: usize,
    pub assets_skipped: usize,
    pub java_major: Option<u32>,
    pub assets_index: String,
    pub natives_dir: String,
    pub chain: Vec<String>,
}

/// 一次安装的执行体。字段都是克隆出来的便宜值，能安全跨 await。
pub struct Installer {
    pub client: reqwest::Client,
    pub paths: Paths,
    /// 候选下载源（auto 已测速排序好）
    pub origins: Vec<Origin>,
    pub events: Arc<dyn Events>,
    /// 同时下载文件数（来自设置 concurrentDownloads）
    pub concurrency: usize,
    /// 单文件分片线程数（来自设置 perFileConnections）
    pub connections: usize,
}

impl Installer {
    pub fn new(
        client: reqwest::Client,
        paths: Paths,
        origins: Vec<Origin>,
        events: Arc<dyn Events>,
        concurrency: usize,
        connections: usize,
    ) -> Self {
        Self {
            client,
            paths,
            origins,
            events,
            concurrency: concurrency.max(1),
            connections: connections.max(1),
        }
    }

    fn fetcher(&self, task: &TaskHandle) -> Fetcher {
        // 取消标志与任务共享：cancel 一按，在飞的请求也会尽快退出
        Fetcher::new(
            self.client.clone(),
            self.origins.clone(),
            task.cancel_flag(),
            self.connections,
        )
    }

    /// 安装（或补齐）指定版本。
    pub async fn install(&self, version_id: &str, task: &TaskHandle) -> Result<InstallReport> {
        let version_id = version_id.trim();
        if version_id.is_empty() {
            return Err(AppError::Invalid("游戏版本不能为空".to_string()));
        }
        self.paths.ensure()?;
        let fetcher = self.fetcher(task);

        // 1) 版本 json 链
        task.progress(Phase::Downloading, "正在获取版本清单", 0, 0);
        self.ensure_version_json(&fetcher, version_id).await?;
        let chain = load_chain(&self.paths, version_id)?;
        let merged = version::merge_chain(&chain)?;
        let platform = Platform::current();
        let features = version::default_features();
        let libraries = version::resolve_libraries(&merged.libraries, &platform, &features)?;

        // 2) 客户端 jar + libraries
        let mut pending: Vec<PendingFile> = Vec::new();
        if let Some(client) = &merged.client {
            let dest = client_jar_path(&self.paths, version_id);
            if !crate::downloader::should_skip(&dest, client.size) {
                pending.push(PendingFile {
                    url: client.url.clone(),
                    dest,
                    sha1: client.sha1.clone(),
                    size: client.size,
                    label: format!("{version_id}.jar"),
                });
            }
        }
        for lib in &libraries {
            let dest = self.paths.libraries.join(lib.path.replace('/', &std::path::MAIN_SEPARATOR.to_string()));
            if crate::downloader::should_skip(&dest, lib.size) {
                continue;
            }
            pending.push(PendingFile {
                url: lib.url.clone(),
                dest,
                sha1: lib.sha1.clone(),
                size: lib.size,
                label: lib.path.clone(),
            });
        }
        let lib_total = pending.len();
        task.progress_forced(
            Phase::Downloading,
            &format!("正在下载客户端与库（{lib_total} 个待下载）"),
            0,
            lib_total as u64,
        );
        let (lib_done, _) = download_batch(
            &fetcher,
            self.events.as_ref(),
            pending,
            self.concurrency,
            task,
            "正在下载客户端与库",
        )
        .await?;

        // 3) assets
        let mut assets_total = 0usize;
        let mut assets_downloaded = 0usize;
        let mut assets_skipped = 0usize;
        let mut index_id = merged.assets.clone();
        if let Some(index_ref) = &merged.asset_index {
            index_id = index_ref.id.clone();
            let index_path = assets_index_path(&self.paths, &index_ref.id);
            if !crate::downloader::should_skip(&index_path, index_ref.size) {
                let url = if index_ref.url.trim().is_empty() {
                    format!("https://launchermeta.mojang.com/v1/packages/{}/{}.json", index_ref.id, index_ref.id)
                } else {
                    index_ref.url.clone()
                };
                task.progress(Phase::Downloading, "正在下载资源索引", 0, 0);
                fetcher
                    .fetch(&url, &index_path, index_ref.sha1.as_deref(), index_ref.size)
                    .await?;
            }
            let text = std::fs::read_to_string(&index_path).map_err(|err| {
                AppError::Other(format!("资源索引读取失败（{}）：{err}", index_path.display()))
            })?;
            let index = version::AssetIndex::parse(&text)?;
            assets_total = index.objects.len();

            let mut pending_assets: Vec<PendingFile> = Vec::new();
            for object in index.objects.values() {
                let dest = version::asset_object_path(&self.paths.assets, &object.hash)?;
                if dest.is_file() {
                    assets_skipped += 1;
                    continue;
                }
                let url = version::asset_object_url(&object.hash)?;
                pending_assets.push(PendingFile {
                    url,
                    dest,
                    sha1: Some(object.hash.clone()),
                    size: if object.size > 0 { Some(object.size) } else { None },
                    label: object.hash.clone(),
                });
            }
            assets_downloaded = pending_assets.len();
            task.progress_forced(
                Phase::Downloading,
                &format!("正在下载资源文件（{assets_downloaded} 个待下载）"),
                0,
                assets_downloaded as u64,
            );
            download_batch(
                &fetcher,
                self.events.as_ref(),
                pending_assets,
                self.concurrency,
                task,
                "正在下载资源文件",
            )
            .await?;
        }

        // 4) natives 解压
        let natives_target = natives_dir(&self.paths, version_id);
        let natives = libraries
            .iter()
            .filter(|lib| lib.kind == version::LibKind::Natives)
            .collect::<Vec<_>>();
        if !natives.is_empty() {
            task.progress_forced(Phase::Extracting, "正在解压 natives", 0, natives.len() as u64);
            if natives_target.is_dir() {
                // 旧架构的 dll 留着会让 JVM 加载错的版本，直接清空重解
                let _ = std::fs::remove_dir_all(&natives_target);
            }
            std::fs::create_dir_all(&natives_target)?;
            for (index, lib) in natives.iter().enumerate() {
                task.check_cancelled()?;
                let jar = self
                    .paths
                    .libraries
                    .join(lib.path.replace('/', &std::path::MAIN_SEPARATOR.to_string()));
                extract_natives_jar(&jar, &natives_target, &lib.exclude)?;
                task.progress(
                    Phase::Extracting,
                    &format!("正在解压 natives：{}", lib.path),
                    index as u64 + 1,
                    natives.len() as u64,
                );
            }
        } else if natives_target.is_dir() {
            // 无 natives 的版本（极旧/极新）留空目录没意义
            let _ = std::fs::remove_dir_all(&natives_target);
        }

        // 5) 自检：缺什么就报什么，不带病启动
        task.progress(Phase::Verifying, "正在校验安装结果", 0, 0);
        let problems = inspect_ready(&self.paths, version_id, &merged, &platform, &features)?;
        if !problems.is_empty() {
            return Err(AppError::Other(format!(
                "安装后自检未通过：{}",
                problems.join("；")
            )));
        }

        let report = InstallReport {
            version_id: version_id.to_string(),
            client_jar: merged
                .client
                .as_ref()
                .map(|_| client_jar_path(&self.paths, version_id).display().to_string()),
            libraries: libraries.len(),
            natives: natives.len(),
            assets_total,
            assets_downloaded,
            assets_skipped,
            java_major: merged.java_major,
            assets_index: index_id,
            natives_dir: natives_target.display().to_string(),
            chain: merged.chain.clone(),
        };
        task.progress_forced(Phase::Finished, "安装完成", lib_done as u64, lib_done as u64);
        tracing::info!(
            version = version_id,
            libraries = report.libraries,
            assets = assets_total,
            "版本安装完成"
        );
        Ok(report)
    }

    /// 启动前自检：齐全直接返回，缺东西就补齐；补不齐给可读原因。
    pub async fn ensure_ready(&self, version_id: &str, task: &TaskHandle) -> Result<InstallReport> {
        let platform = Platform::current();
        let features = version::default_features();
        match load_chain(&self.paths, version_id) {
            Ok(chain) => {
                let merged = version::merge_chain(&chain)?;
                let problems = inspect_ready(&self.paths, version_id, &merged, &platform, &features)?;
                if problems.is_empty() {
                    let libraries =
                        version::resolve_libraries(&merged.libraries, &platform, &features)?;
                    return Ok(InstallReport {
                        version_id: version_id.to_string(),
                        client_jar: Some(client_jar_path(&self.paths, version_id).display().to_string()),
                        libraries: libraries.len(),
                        natives: libraries
                            .iter()
                            .filter(|lib| lib.kind == version::LibKind::Natives)
                            .count(),
                        assets_total: 0,
                        assets_downloaded: 0,
                        assets_skipped: 0,
                        java_major: merged.java_major,
                        assets_index: merged.assets.clone(),
                        natives_dir: natives_dir(&self.paths, version_id).display().to_string(),
                        chain: merged.chain.clone(),
                    });
                }
                tracing::info!(version = version_id, problems = problems.len(), "依赖不全，开始补齐");
            }
            Err(err) => {
                tracing::info!(version = version_id, %err, "版本未安装，开始安装");
            }
        }
        self.install(version_id, task).await
    }

    /// 版本 json 落盘（含父版本）。
    async fn ensure_version_json(&self, fetcher: &Fetcher, version_id: &str) -> Result<()> {
        let mut current = version_id.to_string();
        let mut depth = 0usize;
        loop {
            if depth > MAX_CHAIN_DEPTH {
                return Err(AppError::Other(format!(
                    "版本继承链过深（超过 {MAX_CHAIN_DEPTH} 层）：{version_id}"
                )));
            }
            depth += 1;
            let path = version_json_path(&self.paths, &current);
            let json: VersionJson = if path.is_file() {
                let text = std::fs::read_to_string(&path)?;
                serde_json::from_str(&text)?
            } else {
                let url = version_url(fetcher, &self.paths, &current).await?;
                let text = fetcher.fetch_text(&url).await?;
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&path, &text)?;
                serde_json::from_str(&text)?
            };
            match json.inherits_from.clone() {
                Some(parent) if !parent.trim().is_empty() => current = parent,
                _ => break,
            }
        }
        Ok(())
    }
}

/// 一个待下载文件。
#[derive(Debug, Clone)]
pub struct PendingFile {
    pub url: String,
    pub dest: PathBuf,
    pub sha1: Option<String>,
    pub size: Option<u64>,
    pub label: String,
}

/// 并发下载一批文件；返回 (成功数, 跳过数)。
async fn download_batch(
    fetcher: &Fetcher,
    _events: &dyn Events,
    files: Vec<PendingFile>,
    concurrency: usize,
    task: &TaskHandle,
    stage: &str,
) -> Result<(usize, usize)> {
    if files.is_empty() {
        return Ok((0, 0));
    }
    let total = files.len() as u64;
    let semaphore = Arc::new(Semaphore::new(concurrency.max(1)));
    let done = Arc::new(AtomicU64::new(0));
    let bytes = Arc::new(AtomicU64::new(0));
    let started = Instant::now();

    let mut set = tokio::task::JoinSet::new();
    for file in files {
        task.check_cancelled()?;
        let permit = semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| AppError::Other("下载并发控制已关闭".to_string()))?;
        let fetcher = fetcher.clone();
        let done = done.clone();
        let bytes = bytes.clone();
        set.spawn(async move {
            let _permit = permit;
            let outcome = fetcher
                .fetch(&file.url, &file.dest, file.sha1.as_deref(), file.size)
                .await;
            match outcome {
                Ok(outcome) => {
                    done.fetch_add(1, Ordering::Relaxed);
                    if !outcome.skipped {
                        bytes.fetch_add(outcome.bytes, Ordering::Relaxed);
                    }
                    Ok(outcome.skipped)
                }
                Err(err) => Err((file.label.clone(), err)),
            }
        });
    }

    let mut downloaded = 0usize;
    let mut skipped = 0usize;
    let mut failures: Vec<String> = Vec::new();
    let mut last_logged_done = 0u64;
    let mut last_logged_bytes = 0u64;
    tracing::info!(stage, files = total, "阶段开始：下载文件");
    while let Some(joined) = set.join_next().await {
        let result = joined.map_err(|err| AppError::Other(format!("下载任务异常退出：{err}")))?;
        match result {
            Ok(was_skipped) => {
                if was_skipped {
                    skipped += 1;
                } else {
                    downloaded += 1;
                }
                let current = done.load(Ordering::Relaxed);
                let received = bytes.load(Ordering::Relaxed);
                let elapsed = started.elapsed().as_secs_f64().max(0.001);
                let speed = (received as f64 / elapsed) as u64;
                task.set_speed(speed);
                task.progress(Phase::Downloading, stage, current, total);
                if should_log_progress(current, received, last_logged_done, last_logged_bytes) {
                    last_logged_done = current;
                    last_logged_bytes = received;
                    tracing::info!(
                        stage,
                        done = current,
                        total,
                        received_bytes = received,
                        "下载进度"
                    );
                }
            }
            Err((label, err)) => {
                // 单个文件失败不静默：先记 WARN 汇总，全部结束后一起报
                failures.push(format!("{label}：{err}"));
                tracing::warn!(stage, file = %label, %err, "文件下载失败（继续其它文件）");
                let current = done.load(Ordering::Relaxed);
                task.progress(Phase::Downloading, stage, current, total);
                if failures.len() >= MAX_FILE_FAILURES {
                    let summary = summarize_failures(&failures);
                    return Err(AppError::Other(format!(
                        "{stage} 有 {} 个文件下载失败：{summary}",
                        failures.len()
                    )));
                }
            }
        }
    }
    if !failures.is_empty() {
        let summary = summarize_failures(&failures);
        return Err(AppError::Other(format!(
            "{stage} 有 {} 个文件下载失败：{summary}",
            failures.len()
        )));
    }
    tracing::info!(
        stage,
        downloaded,
        skipped,
        elapsed_ms = started.elapsed().as_millis() as u64,
        "阶段结束：下载文件"
    );
    Ok((downloaded, skipped))
}

/// 进度日志节流：每 20 个文件或每 5 MB 打一条 INFO。
pub const LOG_EVERY_FILES: u64 = 20;
pub const LOG_EVERY_BYTES: u64 = 5 * 1024 * 1024;

/// 单批里允许的最大失败文件数（超过就直接报错，避免无限拖）。
pub const MAX_FILE_FAILURES: usize = 8;

/// 纯函数：这次进度要不要打日志。
pub fn should_log_progress(done: u64, bytes: u64, last_done: u64, last_bytes: u64) -> bool {
    done.saturating_sub(last_done) >= LOG_EVERY_FILES
        || bytes.saturating_sub(last_bytes) >= LOG_EVERY_BYTES
}

/// 失败汇总：只取前 3 条，附带总数。
pub fn summarize_failures(failures: &[String]) -> String {
    if failures.is_empty() {
        return String::new();
    }
    let head = failures
        .iter()
        .take(3)
        .map(|item| item.as_str())
        .collect::<Vec<_>>()
        .join("；");
    if failures.len() > 3 {
        format!("{head}（还有其他 {} 条失败）", failures.len() - 3)
    } else {
        head
    }
}

/// 纯函数：total 未知时进度必须标 indeterminate（前端按字节显示）。
pub fn progress_indeterminate(total: u64) -> bool {
    total == 0
}

/// 下载器里查清单用：读缓存优先，缓存里没有该版本才联网重拉。
async fn version_url(fetcher: &Fetcher, paths: &Paths, version_id: &str) -> Result<String> {
    if let Some((raw, _)) = meta::read_cache(&paths.version_manifest_file) {
        if let Some(found) = raw.versions.iter().find(|item| item.id == version_id) {
            return Ok(found.url.clone());
        }
    }
    let text = fetcher.fetch_text(crate::download::OFFICIAL_MANIFEST).await?;
    let raw: meta::RawManifest = serde_json::from_str(&text)?;
    meta::write_cache(&paths.version_manifest_file, &raw)?;
    raw.versions
        .iter()
        .find(|item| item.id == version_id)
        .map(|item| item.url.clone())
        .ok_or_else(|| {
            AppError::Invalid(format!("版本清单里没有 {version_id}，请确认版本号是否正确"))
        })
}

/// 读磁盘上的版本链（子 → 父）。
pub fn load_chain(paths: &Paths, version_id: &str) -> Result<Vec<VersionJson>> {
    let mut chain = Vec::new();
    let mut current = version_id.to_string();
    let mut depth = 0usize;
    loop {
        if depth > MAX_CHAIN_DEPTH {
            return Err(AppError::Other(format!("版本继承链过深：{version_id}")));
        }
        depth += 1;
        let path = version_json_path(paths, &current);
        if !path.is_file() {
            return Err(AppError::Invalid(format!(
                "版本 {current} 尚未安装（缺少 {}）",
                path.display()
            )));
        }
        let text = std::fs::read_to_string(&path)?;
        let json: VersionJson = serde_json::from_str(&text)?;
        let parent = json.inherits_from.clone();
        chain.push(json);
        match parent {
            Some(parent) if !parent.trim().is_empty() => current = parent,
            _ => break,
        }
    }
    Ok(chain)
}

/// 从磁盘读合并后的版本（启动流程用）。
pub fn load_merged(paths: &Paths, version_id: &str) -> Result<MergedVersion> {
    let chain = load_chain(paths, version_id)?;
    version::merge_chain(&chain)
}

/// 依赖自检：返回缺失项的中文描述（空 = 齐全）。
pub fn inspect_ready(
    paths: &Paths,
    version_id: &str,
    merged: &MergedVersion,
    platform: &Platform,
    features: &BTreeMap<String, bool>,
) -> Result<Vec<String>> {
    let mut problems = Vec::new();

    let jar = client_jar_path(paths, version_id);
    if !jar.is_file() {
        problems.push(format!("缺少客户端 jar（{}）", jar.display()));
    }

    let libraries = version::resolve_libraries(&merged.libraries, platform, features)?;
    let mut missing_libs = 0usize;
    let mut first_missing: Option<String> = None;
    for lib in &libraries {
        let dest = paths.libraries.join(lib.path.replace('/', &std::path::MAIN_SEPARATOR.to_string()));
        if !dest.is_file() {
            missing_libs += 1;
            if first_missing.is_none() {
                first_missing = Some(lib.path.clone());
            }
        }
    }
    if missing_libs > 0 {
        problems.push(format!(
            "缺少 {missing_libs} 个库文件（例如 {}）",
            first_missing.unwrap_or_default()
        ));
    }

    // natives 目录：有 natives 库时不该是空的
    let natives_expected = libraries
        .iter()
        .any(|lib| lib.kind == version::LibKind::Natives);
    if natives_expected {
        let dir = natives_dir(paths, version_id);
        let has_file = std::fs::read_dir(&dir)
            .map(|entries| {
                entries
                    .filter_map(|entry| entry.ok())
                    .any(|entry| entry.path().is_file())
            })
            .unwrap_or(false);
        if !has_file {
            problems.push(format!("缺少 natives 解压文件（{}）", dir.display()));
        }
    }

    // assets：索引文件 + 对象抽样
    if let Some(index_ref) = &merged.asset_index {
        let index_path = assets_index_path(paths, &index_ref.id);
        if !index_path.is_file() {
            problems.push(format!("缺少资源索引 {}.json", index_ref.id));
        } else {
            let text = std::fs::read_to_string(&index_path)?;
            let index = version::AssetIndex::parse(&text)?;
            let missing = version::missing_assets_sample(&index, &paths.assets, 5);
            if !missing.is_empty() {
                problems.push(format!(
                    "缺少资源文件：{} 等（共 {} 个对象）",
                    missing.join("、"),
                    index.objects.len()
                ));
            }
        }
    }

    Ok(problems)
}

/// 解压 natives jar 到目标目录（跳过目录、META-INF/ 与 extract.exclude）。
pub fn extract_natives_jar(jar: &Path, dest: &Path, excludes: &[String]) -> Result<usize> {
    let file = std::fs::File::open(jar)
        .map_err(|err| AppError::Other(format!("打开 natives 包失败（{}）：{err}", jar.display())))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|err| AppError::Other(format!("natives 包损坏（{}）：{err}", jar.display())))?;
    std::fs::create_dir_all(dest)?;

    let mut count = 0usize;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|err| AppError::Other(format!("读取 natives 条目失败：{err}")))?;
        if entry.is_dir() {
            continue;
        }
        let Some(name) = entry.enclosed_name() else {
            continue;
        };
        let name_text = name.to_string_lossy().replace('\\', "/");
        if name_text.starts_with("META-INF/") {
            continue;
        }
        if excludes
            .iter()
            .any(|prefix| !prefix.is_empty() && name_text.starts_with(prefix))
        {
            continue;
        }
        let file_name = name
            .file_name()
            .map(|file| file.to_os_string())
            .unwrap_or_else(|| std::ffi::OsString::from("native.bin"));
        let target = dest.join(file_name);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = std::fs::File::create(&target)?;
        std::io::copy(&mut entry, &mut out)?;
        count += 1;
    }
    Ok(count)
}

/// 生成一份可读的安装摘要（日志用）。
pub fn summarize(report: &InstallReport) -> String {
    format!(
        "{}：库 {} 个、natives {} 个、资源 {}/{}（已存在 {}）、Java {}",
        report.version_id,
        report.libraries,
        report.natives,
        report.assets_downloaded,
        report.assets_total,
        report.assets_skipped,
        report
            .java_major
            .map(|major| major.to_string())
            .unwrap_or_else(|| "未知".to_string())
    )
}

/// 给日志文件名用：版本 id 里可能有 / 之类的字符。
pub fn safe_version_dir_name(version_id: &str) -> String {
    util::sanitize_file_part(version_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::NoopEvents;
    use crate::tasks::TaskStore;

    #[test]
    fn progress_log_throttle_fires_every_20_files_or_5mb() {
        assert!(!should_log_progress(5, 1024, 0, 0), "5 个文件、1KB 都不够");
        assert!(should_log_progress(20, 1024, 0, 0), "满 20 个文件要打");
        assert!(!should_log_progress(19, 1024, 0, 0));
        assert!(should_log_progress(3, 5 * 1024 * 1024, 0, 0), "满 5MB 也要打");
        assert!(!should_log_progress(21, 1024, 20, 1024), "刚打过不再打");
    }

    #[test]
    fn failure_summary_keeps_first_three() {
        assert!(summarize_failures(&[]).is_empty());
        let failures: Vec<String> = (1..=6).map(|index| format!("文件{index} 失败")).collect();
        let summary = summarize_failures(&failures);
        assert!(summary.contains("文件1"));
        assert!(summary.contains("文件3"));
        assert!(!summary.contains("文件4"), "只取前 3 条：{summary}");
        assert!(summary.contains("还有其他 3 条失败"), "{summary}");
        let two = summarize_failures(&["a 失败".to_string(), "b 失败".to_string()]);
        assert!(!two.contains("还有其他"));
    }

    #[test]
    fn unknown_total_marks_indeterminate() {
        assert!(progress_indeterminate(0));
        assert!(!progress_indeterminate(1));
        assert!(!progress_indeterminate(1024));
    }

    fn temp_root(tag: &str) -> Paths {
        let root = std::env::temp_dir().join(format!("gugu-install-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let paths = Paths::from_root(&root);
        paths.ensure().unwrap();
        paths
    }

    fn write_version_json(paths: &Paths, json: &str) {
        let id: serde_json::Value = serde_json::from_str(json).unwrap();
        let version_id = id["id"].as_str().unwrap().to_string();
        let path = version_json_path(paths, &version_id);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, json).unwrap();
    }

    fn build_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let file = std::fs::File::create(path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, body) in entries {
            writer.start_file(*name, options).unwrap();
            std::io::Write::write_all(&mut writer, body).unwrap();
        }
        writer.finish().unwrap();
    }

    #[test]
    fn extract_natives_skips_meta_inf_and_excludes() {
        let paths = temp_root("natives");
        let jar = paths.cache.join("lwjgl-natives.jar");
        build_zip(
            &jar,
            &[
                ("META-INF/MANIFEST.MF", b"manifest"),
                ("META-INF/versions.txt", b"x"),
                ("lwjgl.dll", b"dll"),
                ("libglfw.so", b"so"),
                ("skip/readme.txt", b"nope"),
            ],
        );
        let dest = paths.cache.join("natives-out");
        let count = extract_natives_jar(&jar, &dest, &["skip/".to_string()]).unwrap();
        assert_eq!(count, 2, "只该解出 lwjgl.dll 与 libglfw.so");
        assert!(dest.join("lwjgl.dll").is_file());
        assert!(dest.join("libglfw.so").is_file());
        assert!(!dest.join("META-INF").exists());
        assert!(!dest.join("readme.txt").exists());
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn load_chain_reads_child_then_parent() {
        let paths = temp_root("chain");
        write_version_json(
            &paths,
            r#"{"id": "1.20.1", "mainClass": "net.minecraft.client.main.Main", "assets": "5"}"#,
        );
        write_version_json(
            &paths,
            r#"{"id": "1.20.1-forge", "inheritsFrom": "1.20.1", "mainClass": "cpw.mods.Boot"}"#,
        );
        let chain = load_chain(&paths, "1.20.1-forge").unwrap();
        assert_eq!(chain.len(), 2);
        assert_eq!(chain[0].id, "1.20.1-forge");
        assert_eq!(chain[1].id, "1.20.1");

        let merged = load_merged(&paths, "1.20.1-forge").unwrap();
        assert_eq!(merged.main_class.as_deref(), Some("cpw.mods.Boot"));
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn load_chain_reports_missing_version_readably() {
        let paths = temp_root("missing");
        let err = load_chain(&paths, "9.9.9").unwrap_err().to_string();
        assert!(err.contains("尚未安装"), "错误文案要可读：{err}");
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn inspect_ready_lists_every_missing_piece() {
        let paths = temp_root("inspect");
        write_version_json(
            &paths,
            r#"{
                "id": "1.20.1",
                "mainClass": "net.minecraft.client.main.Main",
                "assets": "5",
                "assetIndex": {"id": "5", "url": "https://example.com/5.json"},
                "downloads": {"client": {"url": "https://example.com/client.jar", "size": 3}},
                "libraries": [
                    {"name": "com.mojang:logging:1.1.1", "downloads": {"artifact": {"path": "com/mojang/logging/1.1.1/logging-1.1.1.jar", "url": "https://example.com/l.jar"}}},
                    {"name": "org.lwjgl:lwjgl:3.3.1:natives-windows", "rules": [{"action": "allow", "os": {"name": "windows"}}], "downloads": {"artifact": {"path": "org/lwjgl/lwjgl/3.3.1/lwjgl-3.3.1-natives-windows.jar", "url": "https://example.com/n.jar"}}}
                ]
            }"#,
        );
        let merged = load_merged(&paths, "1.20.1").unwrap();
        let problems = inspect_ready(
            &paths,
            "1.20.1",
            &merged,
            &Platform::windows_x64(),
            &version::default_features(),
        )
        .unwrap();
        let joined = problems.join("；");
        assert!(joined.contains("客户端 jar"), "{joined}");
        assert!(joined.contains("库文件"), "{joined}");
        assert!(joined.contains("资源索引"), "{joined}");
        assert!(joined.contains("natives"), "{joined}");

        // 补齐缺失项后问题应逐条消失
        std::fs::write(client_jar_path(&paths, "1.20.1"), b"jar").unwrap();
        let logging = paths
            .libraries
            .join("com/mojang/logging/1.1.1/logging-1.1.1.jar".replace('/', &std::path::MAIN_SEPARATOR.to_string()));
        std::fs::create_dir_all(logging.parent().unwrap()).unwrap();
        std::fs::write(&logging, b"lib").unwrap();
        let natives_jar = paths
            .libraries
            .join("org/lwjgl/lwjgl/3.3.1/lwjgl-3.3.1-natives-windows.jar".replace('/', &std::path::MAIN_SEPARATOR.to_string()));
        std::fs::create_dir_all(natives_jar.parent().unwrap()).unwrap();
        std::fs::write(&natives_jar, b"nat").unwrap();
        let natives = natives_dir(&paths, "1.20.1");
        std::fs::create_dir_all(&natives).unwrap();
        std::fs::write(natives.join("lwjgl.dll"), b"dll").unwrap();
        let index_path = assets_index_path(&paths, "5");
        std::fs::create_dir_all(index_path.parent().unwrap()).unwrap();
        std::fs::write(&index_path, r#"{"objects": {}}"#).unwrap();

        let problems = inspect_ready(
            &paths,
            "1.20.1",
            &merged,
            &Platform::windows_x64(),
            &version::default_features(),
        )
        .unwrap();
        assert!(problems.is_empty(), "补齐后不该再有问题：{problems:?}");
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn report_summary_is_readable() {
        let report = InstallReport {
            version_id: "1.20.1".into(),
            client_jar: Some("D:\\minecraft\\.mc\\versions\\1.20.1\\1.20.1.jar".into()),
            libraries: 40,
            natives: 6,
            assets_total: 100,
            assets_downloaded: 12,
            assets_skipped: 88,
            java_major: Some(17),
            assets_index: "5".into(),
            natives_dir: "natives".into(),
            chain: vec!["1.20.1".into()],
        };
        let text = summarize(&report);
        assert!(text.contains("1.20.1"));
        assert!(text.contains("Java 17"));
        assert!(text.contains("100"));
    }

    #[test]
    fn safe_version_dir_name_replaces_path_separators() {
        assert_eq!(safe_version_dir_name("1.20.1"), "1.20.1");
        assert_eq!(safe_version_dir_name("a/b"), "a_b");
    }

    #[tokio::test]
    async fn install_rejects_empty_version_without_network() {
        let paths = temp_root("empty");
        let store = TaskStore::new();
        let task = store.create("install", "空版本", &paths.root.display().to_string());
        let installer = Installer::new(
            reqwest::Client::new(),
            paths.clone(),
            vec![Origin::Builtin(crate::download::DownloadSource::Auto)],
            Arc::new(NoopEvents),
            DEFAULT_CONCURRENCY,
            1,
        );
        let err = installer.install("   ", &task).await.unwrap_err();
        assert!(err.to_string().contains("游戏版本不能为空"));
        let _ = std::fs::remove_dir_all(&paths.root);
    }
}
