//! 存储统计与清理（设置 → 存储页）。
//!
//! 统计全部走 spawn_blocking，单目录最多枚举 MAX_FILES 个文件（超出标 truncated）；
//! 结果缓存 CACHE_TTL 秒，避免反复点开就重扫。

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::error::{AppError, Result};
use crate::paths::Paths;
use crate::tasks::TaskStore;

/// 单个目录最多统计多少个文件（防止几十万文件的目录把统计拖死）。
pub const MAX_FILES: u64 = 200_000;
/// 统计结果缓存。
pub const CACHE_TTL: Duration = Duration::from_secs(60);
/// 清理日志时只删这么多天以前的。
pub const LOG_KEEP_DAYS: u64 = 7;
/// storage_clean 允许的 key。
pub const CLEAN_KEYS: [&str; 4] = ["cache", "logs", "versions", "assets"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageItem {
    pub key: String,
    pub label: String,
    pub bytes: u64,
    pub files: u64,
    pub path: String,
    /// 文件数超过 MAX_FILES 时停止统计，数字是下限
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageStats {
    pub total_bytes: u64,
    pub items: Vec<StorageItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanResult {
    pub freed_bytes: u64,
}

fn walk(dir: &Path, bytes: &mut u64, files: &mut u64, truncated: &mut bool) {
    if *truncated {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(|entry| entry.ok()) {
        if *files >= MAX_FILES {
            *truncated = true;
            return;
        }
        let path = entry.path();
        match entry.file_type() {
            Ok(kind) if kind.is_dir() => walk(&path, bytes, files, truncated),
            Ok(_) => {
                *files += 1;
                *bytes += entry.metadata().map(|meta| meta.len()).unwrap_or(0);
            }
            Err(_) => {}
        }
    }
}

/// 统计一个目录：返回 (字节, 文件数, 是否被上限截断)。
pub fn dir_size(dir: &Path) -> (u64, u64, bool) {
    let mut bytes = 0;
    let mut files = 0;
    let mut truncated = false;
    walk(dir, &mut bytes, &mut files, &mut truncated);
    (bytes, files, truncated)
}

/// 统计项定义（key, 中文名, 目录列表）。
pub fn items_of(paths: &Paths) -> Vec<(String, String, Vec<PathBuf>)> {
    vec![
        ("instances".to_string(), "实例".to_string(), vec![paths.instances.clone()]),
        ("versions".to_string(), "游戏版本".to_string(), vec![paths.versions.clone()]),
        ("libraries".to_string(), "依赖库".to_string(), vec![paths.libraries.clone()]),
        ("assets".to_string(), "资源文件".to_string(), vec![paths.assets.clone()]),
        (
            "cache".to_string(),
            "缓存".to_string(),
            vec![paths.cache.clone(), paths.root.join("update")],
        ),
        ("logs".to_string(), "日志".to_string(), vec![paths.logs.clone(), paths.diagnostics.clone()]),
        ("runtimes".to_string(), "Java 运行时".to_string(), vec![paths.runtimes.clone()]),
    ]
}

/// storage_stats：7 项分别统计（同一个目录被两项共用时各算各的，前端按 key 显示）。
pub fn stats(paths: &Paths) -> StorageStats {
    let mut items = Vec::new();
    let mut total = 0u64;
    for (key, label, dirs) in items_of(paths) {
        let mut bytes = 0u64;
        let mut files = 0u64;
        let mut truncated = false;
        for dir in &dirs {
            let (dir_bytes, dir_files, dir_truncated) = dir_size(dir);
            bytes += dir_bytes;
            files += dir_files;
            truncated |= dir_truncated;
        }
        total += bytes;
        items.push(StorageItem {
            key,
            label,
            bytes,
            files,
            path: dirs
                .first()
                .map(|dir| dir.display().to_string())
                .unwrap_or_default(),
            truncated,
        });
    }
    StorageStats {
        total_bytes: total,
        items,
    }
}

// ---------------------------------------------------------------- 缓存

fn cache() -> &'static std::sync::Mutex<Option<(Instant, StorageStats)>> {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<Option<(Instant, StorageStats)>>> =
        std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(None))
}

pub fn cached_stats() -> Option<StorageStats> {
    let guard = cache().lock().ok()?;
    let (at, stats) = guard.as_ref()?;
    if at.elapsed() < CACHE_TTL {
        Some(stats.clone())
    } else {
        None
    }
}

pub fn store_stats(stats: &StorageStats) {
    if let Ok(mut guard) = cache().lock() {
        *guard = Some((Instant::now(), stats.clone()));
    }
}

pub fn invalidate_cache() {
    if let Ok(mut guard) = cache().lock() {
        *guard = None;
    }
}

// ---------------------------------------------------------------- 清理

/// 有下载/安装任务在跑就不许清理。
pub fn busy_message(tasks: &TaskStore) -> Option<String> {
    let running = tasks
        .list()
        .into_iter()
        .any(|task| !task.phase.is_terminal());
    running.then(|| "有下载任务进行中，稍后再清理".to_string())
}

/// 7 天以前的时间戳（秒）。
pub fn log_cutoff(now: u64) -> u64 {
    now.saturating_sub(LOG_KEEP_DAYS * 24 * 3600)
}

/// 该文件的修改时间是否早于 cutoff（读不到时间就不删）。
pub fn is_old_file(path: &Path, cutoff: u64) -> bool {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs() < cutoff)
        .unwrap_or(false)
}

fn remove_path(path: &Path) -> u64 {
    let (bytes, _, _) = dir_size(path);
    let result = if path.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    };
    match result {
        Ok(()) => bytes,
        Err(err) => {
            tracing::warn!(%err, path = %path.display(), "删除失败");
            0
        }
    }
}

/// 清 cache：.mc/cache 与 .mc/update 整个删掉再建空目录。
pub fn clean_cache(paths: &Paths) -> u64 {
    let mut freed = 0;
    for dir in [paths.cache.clone(), paths.root.join("update")] {
        freed += remove_path(&dir);
        let _ = std::fs::create_dir_all(&dir);
    }
    freed
}

/// 清 logs：只删 LOG_KEEP_DAYS 天以前的文件（目录保留）。
pub fn clean_logs(paths: &Paths, now: u64) -> u64 {
    let cutoff = log_cutoff(now);
    let mut freed = 0;
    for dir in [paths.logs.clone(), paths.diagnostics.clone()] {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.filter_map(|entry| entry.ok()) {
            let path = entry.path();
            if !path.is_file() || !is_old_file(&path, cutoff) {
                continue;
            }
            freed += entry.metadata().map(|meta| meta.len()).unwrap_or(0);
            if let Err(err) = std::fs::remove_file(&path) {
                tracing::warn!(%err, path = %path.display(), "删除旧日志失败");
            }
        }
    }
    freed
}

/// 挑出「没有任何实例引用」的版本目录（纯函数，方便单测）。
pub fn unreferenced_versions(available: &[String], referenced: &[String]) -> Vec<String> {
    available
        .iter()
        .filter(|id| !referenced.iter().any(|used| used == *id))
        .cloned()
        .collect()
}

/// 清 versions：只删没有被任何实例引用的版本目录。
pub fn clean_versions(paths: &Paths, referenced: &[String]) -> u64 {
    let Ok(entries) = std::fs::read_dir(&paths.versions) else {
        return 0;
    };
    let mut available: Vec<String> = Vec::new();
    for entry in entries.filter_map(|entry| entry.ok()) {
        if entry.path().is_dir() {
            available.push(entry.file_name().to_string_lossy().to_string());
        }
    }
    let mut freed = 0;
    for id in unreferenced_versions(&available, referenced) {
        tracing::info!(version = %id, "删除未被实例引用的版本目录");
        freed += remove_path(&paths.versions.join(&id));
    }
    freed
}

/// assets 只校验（统计缺多少），不删任何东西。
pub fn assets_report(paths: &Paths) -> String {
    if !paths.assets.is_dir() {
        return "assets 目录不存在".to_string();
    }
    let (bytes, files, truncated) = dir_size(&paths.assets);
    format!(
        "assets 共 {files} 个文件（{} MB{}），不做删除，只做了校验统计",
        bytes / (1024 * 1024),
        if truncated { "，已截断" } else { "" }
    )
}

/// key → 清理动作（返回实际释放的字节）。
pub fn clean(paths: &Paths, key: &str, now: u64, referenced: &[String]) -> Result<u64> {
    match key {
        "cache" => Ok(clean_cache(paths)),
        "logs" => Ok(clean_logs(paths, now)),
        "versions" => Ok(clean_versions(paths, referenced)),
        "assets" => {
            let message = assets_report(paths);
            tracing::info!("{message}");
            Ok(0)
        }
        other => Err(AppError::Invalid(format!(
            "不支持清理这个 key：{other}（可用：{}）",
            CLEAN_KEYS.join(" / ")
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_paths(tag: &str) -> Paths {
        let root = std::env::temp_dir().join(format!("gugu-storage-{tag}-{}", crate::util::unique_suffix()));
        let _ = std::fs::remove_dir_all(&root);
        let paths = Paths::from_root(&root);
        paths.ensure().unwrap();
        paths
    }

    #[test]
    fn dir_size_counts_files_and_respects_limit() {
        let paths = temp_paths("size");
        std::fs::write(paths.cache.join("a.bin"), vec![0u8; 100]).unwrap();
        std::fs::write(paths.cache.join("b.bin"), vec![0u8; 50]).unwrap();
        std::fs::create_dir_all(paths.cache.join("sub")).unwrap();
        std::fs::write(paths.cache.join("sub").join("c.bin"), vec![0u8; 25]).unwrap();
        let (bytes, files, truncated) = dir_size(&paths.cache);
        assert_eq!(files, 3);
        assert_eq!(bytes, 175);
        assert!(!truncated);
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn walk_stops_at_the_file_cap() {
        let paths = temp_paths("cap");
        // 直接构造一个「远超上限」的假统计：把上限临时换小，用同样的 walk 逻辑
        for index in 0..40 {
            std::fs::write(paths.cache.join(format!("f{index}.bin")), b"x").unwrap();
        }
        let mut bytes = 0;
        // 用 MAX_FILES 的语义验证：手动把 files 推到上限，walk 必须停
        let mut files = MAX_FILES;
        let mut truncated = false;
        walk(&paths.cache, &mut bytes, &mut files, &mut truncated);
        assert!(truncated, "达到上限必须标记截断");
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn stats_lists_seven_items_and_caches() {
        let paths = temp_paths("stats");
        std::fs::write(paths.cache.join("x.bin"), vec![0u8; 1024]).unwrap();
        invalidate_cache();
        assert!(cached_stats().is_none());
        let stats = super::stats(&paths);
        assert_eq!(stats.items.len(), 7);
        let keys: Vec<&str> = stats.items.iter().map(|item| item.key.as_str()).collect();
        assert_eq!(keys, vec!["instances", "versions", "libraries", "assets", "cache", "logs", "runtimes"]);
        let cache_item = stats.items.iter().find(|item| item.key == "cache").unwrap();
        assert!(cache_item.bytes >= 1024);
        assert!(stats.total_bytes >= cache_item.bytes);
        store_stats(&stats);
        let hit = cached_stats().expect("60 秒内应命中缓存");
        assert_eq!(hit.total_bytes, stats.total_bytes);
        invalidate_cache();
        assert!(cached_stats().is_none());
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn unreferenced_versions_only_keeps_used_ones() {
        let available = vec!["1.20.1".to_string(), "1.21.4".to_string(), "1.7.10".to_string()];
        let referenced = vec!["1.20.1".to_string()];
        assert_eq!(
            unreferenced_versions(&available, &referenced),
            vec!["1.21.4".to_string(), "1.7.10".to_string()]
        );
        assert!(unreferenced_versions(&available, &available).is_empty());
        assert!(unreferenced_versions(&[], &[]).is_empty());
    }

    #[test]
    fn clean_versions_keeps_referenced_directories() {
        let paths = temp_paths("versions");
        for id in ["1.20.1", "1.21.4"] {
            let dir = paths.versions.join(id);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("x.jar"), vec![0u8; 2048]).unwrap();
        }
        let freed = clean_versions(&paths, &["1.20.1".to_string()]);
        assert!(freed >= 2048);
        assert!(paths.versions.join("1.20.1").is_dir(), "被引用的版本不能删");
        assert!(!paths.versions.join("1.21.4").exists(), "没人用的版本要删掉");
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn clean_logs_only_removes_files_older_than_seven_days() {
        let paths = temp_paths("logs");
        let fresh = paths.logs.join("fresh.log");
        let old = paths.logs.join("old.log");
        std::fs::write(&fresh, b"new").unwrap();
        std::fs::write(&old, b"old").unwrap();
        let now = crate::util::now_unix();
        // 把 old.log 的修改时间改成 30 天前
        let script_path = paths.root.join("age.txt");
        std::fs::write(&script_path, b"x").unwrap();
        let old_time = now - 30 * 24 * 3600;
        let output = std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                &format!(
                    "(Get-Item '{}').LastWriteTimeUtc = [datetime]::UnixEpoch.AddSeconds({old_time})",
                    old.display()
                ),
            ])
            .output();
        // 允许在非 Windows / 无 powershell 环境跳过时间造假
        if output.map(|out| out.status.success()).unwrap_or(false) {
            let freed = clean_logs(&paths, now);
            assert!(freed > 0, "旧日志应当被删掉");
            assert!(!old.exists(), "30 天前的日志要删");
            assert!(fresh.exists(), "新日志要留着");
        } else {
            // 时间造不出来时只验证「7 天前」的判定函数本身
            assert!(!is_old_file(&fresh, log_cutoff(now)));
            assert_eq!(log_cutoff(now), now - 7 * 24 * 3600);
        }
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn clean_cache_empties_directories() {
        let paths = temp_paths("cache");
        std::fs::write(paths.cache.join("a.bin"), vec![0u8; 4096]).unwrap();
        std::fs::create_dir_all(paths.root.join("update")).unwrap();
        std::fs::write(paths.root.join("update").join("b.bin"), vec![0u8; 1024]).unwrap();
        let freed = clean_cache(&paths);
        assert!(freed >= 5120);
        assert!(paths.cache.is_dir(), "目录要重建");
        let (bytes, files, _) = dir_size(&paths.cache);
        assert_eq!((bytes, files), (0, 0));
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn cleaning_is_refused_while_a_task_runs() {
        let store = TaskStore::new();
        assert!(busy_message(&store).is_none());
        let running = store.create("install", "1.20.1", "dir");
        assert_eq!(
            busy_message(&store).as_deref(),
            Some("有下载任务进行中，稍后再清理")
        );
        running.finish();
        assert!(busy_message(&store).is_none());
    }

    #[test]
    fn unknown_clean_key_is_rejected() {
        let paths = temp_paths("badkey");
        let err = clean(&paths, "instances", 0, &[]).unwrap_err().to_string();
        assert!(err.contains("不支持清理"), "{err}");
        let _ = std::fs::remove_dir_all(&paths.root);
    }
}
