//! 服务端内容（mods/plugins）、存档备份、server.properties 与日志。
//!
//! jar.disabled 视为禁用（靠改文件名切换），安装记录落在 <server>/.gugu-content.json，
//! 检查更新只认记录：没记录的文件进 failed（服务端没有客户端那套指纹库）。

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::content::{self, Provider};
use crate::error::{AppError, Result};
use crate::events::Phase;
use crate::server::PropertySet;
use crate::tasks::TaskHandle;
use crate::util;

pub const RECORD_FILE: &str = ".gugu-content.json";
pub const BACKUP_DIR: &str = "backups";
pub const DISABLED_SUFFIX: &str = ".disabled";
/// 备份任务的 install://progress 前缀
pub const BACKUP_PREFIX: &str = "server-backup:";
pub const DEFAULT_LEVEL_NAME: &str = "world";
pub const PROPERTIES_FILE: &str = "server.properties";

/// 服务端内容种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerContentKind {
    Mod,
    Plugin,
}

impl ServerContentKind {
    pub fn from_id(id: &str) -> Result<Self> {
        match id.trim().to_lowercase().as_str() {
            "mod" => Ok(Self::Mod),
            "plugin" => Ok(Self::Plugin),
            other => Err(AppError::Invalid(format!(
                "服务端内容只支持 mod / plugin（收到 {other}）"
            ))),
        }
    }

    pub fn dir_name(&self) -> &'static str {
        match self {
            Self::Mod => "mods",
            Self::Plugin => "plugins",
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            Self::Mod => "mod",
            Self::Plugin => "plugin",
        }
    }

    /// 查更新时要用的加载器（插件一律按 paper 找；模组跟着服务端核心）。
    pub fn loader_for(&self, core: &str) -> String {
        match self {
            Self::Plugin => "paper".to_string(),
            Self::Mod => match core.trim().to_lowercase().as_str() {
                "fabric" => "fabric".to_string(),
                "forge" => "forge".to_string(),
                "neoforge" => "neoforge".to_string(),
                "purpur" | "paper" => "paper".to_string(),
                _ => "fabric".to_string(),
            },
        }
    }
}

/// server_content_install 的返回。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallStarted {
    pub task_id: String,
}

/// server_saves_backup 的返回。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupResult {
    pub path: String,
}

// ------------------------------------------------------------------ 安装记录

/// 一个 jar 的来源记录。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentRecord {
    pub provider: String,
    pub project_id: String,
    pub version_id: String,
    #[serde(default)]
    pub version_number: Option<String>,
    #[serde(default)]
    pub file_name: String,
}

/// <server>/.gugu-content.json
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordFile {
    #[serde(default)]
    pub files: BTreeMap<String, ContentRecord>,
}

pub fn record_path(dir: &Path) -> PathBuf {
    dir.join(RECORD_FILE)
}

pub fn load_records(dir: &Path) -> RecordFile {
    let path = record_path(dir);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return RecordFile::default();
    };
    serde_json::from_str(&text).unwrap_or_else(|err| {
        tracing::warn!(%err, path = %path.display(), "安装记录损坏，按空记录处理");
        RecordFile::default()
    })
}

pub fn save_records(dir: &Path, records: &RecordFile) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let path = record_path(dir);
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(records)?)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

/// 记下一个 jar 的来源（覆盖同名记录）。
pub fn upsert_record(dir: &Path, file_name: &str, record: ContentRecord) -> Result<()> {
    let mut records = load_records(dir);
    records.files.insert(file_name.to_string(), record);
    save_records(dir, &records)
}

/// 删掉某个文件的记录（文件被删除/替换后避免脏数据）。
pub fn drop_record(dir: &Path, file_name: &str) -> Result<()> {
    let mut records = load_records(dir);
    if records.files.remove(file_name).is_some() {
        save_records(dir, &records)?;
    }
    Ok(())
}

// ------------------------------------------------------------------ 目录

pub fn kind_dir(dir: &Path, kind: ServerContentKind) -> PathBuf {
    dir.join(kind.dir_name())
}

/// jar / jar.disabled → (真实文件名, 是否启用)。
pub fn split_enabled(file_name: &str) -> (String, bool) {
    match file_name.strip_suffix(DISABLED_SUFFIX) {
        Some(base) if !base.trim().is_empty() => (base.to_string(), false),
        _ => (file_name.to_string(), true),
    }
}

/// 只关心 jar 与 jar.disabled。
pub fn is_content_jar(file_name: &str) -> bool {
    let (base, _) = split_enabled(file_name);
    base.to_lowercase().ends_with(".jar")
}

pub fn validate_file_name(file_name: &str) -> Result<String> {
    let name = file_name.trim();
    if name.is_empty() || name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err(AppError::Invalid(format!("文件名不合法：{file_name}")));
    }
    if !is_content_jar(name) {
        return Err(AppError::Invalid(format!("不是 jar 文件：{file_name}")));
    }
    Ok(name.to_string())
}

/// server_content_list 的元素。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerContentItem {
    pub file_name: String,
    pub size_bytes: u64,
    pub enabled: bool,
    pub provider: Option<String>,
    pub project_id: Option<String>,
}

pub fn list_content(dir: &Path, kind: ServerContentKind) -> Result<Vec<ServerContentItem>> {
    let target = kind_dir(dir, kind);
    let records = load_records(dir);
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(&target) else {
        return Ok(out);
    };
    for entry in entries.filter_map(|entry| entry.ok()) {
        let file_name = entry.file_name().to_string_lossy().to_string();
        if !is_content_jar(&file_name) {
            continue;
        }
        let (base, enabled) = split_enabled(&file_name);
        let record = records.files.get(&file_name).or_else(|| records.files.get(&base));
        out.push(ServerContentItem {
            file_name,
            size_bytes: entry.metadata().map(|meta| meta.len()).unwrap_or(0),
            enabled,
            provider: record.map(|item| item.provider.clone()),
            project_id: record.map(|item| item.project_id.clone()),
        });
    }
    out.sort_by(|left, right| left.file_name.to_lowercase().cmp(&right.file_name.to_lowercase()));
    Ok(out)
}

/// 启用/禁用：靠改文件名实现（jar ↔ jar.disabled）。
pub fn toggle_content(
    dir: &Path,
    kind: ServerContentKind,
    file_name: &str,
    enabled: bool,
) -> Result<()> {
    let name = validate_file_name(file_name)?;
    let target = kind_dir(dir, kind);
    let (base, currently_enabled) = split_enabled(&name);
    let current = target.join(if currently_enabled {
        base.clone()
    } else {
        format!("{base}{DISABLED_SUFFIX}")
    });
    if !current.is_file() {
        return Err(AppError::Invalid(format!("找不到文件：{file_name}")));
    }
    if currently_enabled == enabled {
        return Ok(());
    }
    let next = if enabled {
        target.join(&base)
    } else {
        target.join(format!("{base}{DISABLED_SUFFIX}"))
    };
    if next.exists() {
        return Err(AppError::Invalid(format!(
            "目标文件已存在：{}",
            next.file_name().unwrap_or_default().to_string_lossy()
        )));
    }
    std::fs::rename(&current, &next)?;
    // 记录跟着文件走
    let records = load_records(dir);
    if let Some(record) = records
        .files
        .get(&current.file_name().unwrap_or_default().to_string_lossy().to_string())
        .cloned()
    {
        let previous_name = current.file_name().unwrap_or_default().to_string_lossy().to_string();
        drop_record(dir, &previous_name)?;
        upsert_record(
            dir,
            &next.file_name().unwrap_or_default().to_string_lossy().to_string(),
            record,
        )?;
    }
    tracing::info!(
        kind = kind.id(),
        from = %current.display(),
        to = %next.display(),
        "服务端内容已切换启用状态"
    );
    Ok(())
}

// ------------------------------------------------------------------ 检查/应用更新

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FailedFile {
    pub file_name: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentUpdateItem {
    pub file_name: String,
    pub current_version: Option<String>,
    pub latest_version: String,
    pub download_url: String,
    pub sha1: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentCheckResult {
    pub checked: usize,
    pub updates: Vec<ContentUpdateItem>,
    pub failed: Vec<FailedFile>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentApplyResult {
    pub updated: usize,
    pub failed: Vec<FailedFile>,
}

/// 检查更新：只认安装记录，没记录的直接进 failed。
pub async fn check_content(
    client: &reqwest::Client,
    dir: &Path,
    kind: ServerContentKind,
    mc_version: &str,
    loader: &str,
    api_key: Option<&str>,
) -> Result<ContentCheckResult> {
    let items = list_content(dir, kind)?;
    let records = load_records(dir);
    let mut result = ContentCheckResult {
        checked: items.len(),
        ..Default::default()
    };
    for item in items {
        let (base, _) = split_enabled(&item.file_name);
        let Some(record) = records.files.get(&item.file_name).or_else(|| records.files.get(&base))
        else {
            result.failed.push(FailedFile {
                file_name: item.file_name.clone(),
                reason: "没有安装记录（本地文件无法确认来源）".to_string(),
            });
            continue;
        };
        let provider = match Provider::from_id(&record.provider) {
            Ok(provider) => provider,
            Err(err) => {
                result.failed.push(FailedFile {
                    file_name: item.file_name.clone(),
                    reason: err.to_string(),
                });
                continue;
            }
        };
        let url = match content::versions_url(provider, &record.project_id, Some(mc_version), Some(loader))
        {
            Ok(url) => url,
            Err(err) => {
                result.failed.push(FailedFile {
                    file_name: item.file_name.clone(),
                    reason: err.to_string(),
                });
                continue;
            }
        };
        let text = match content::http_text(client, &url, api_key).await {
            Ok(text) => text,
            Err(err) => {
                result.failed.push(FailedFile {
                    file_name: item.file_name.clone(),
                    reason: err.to_string(),
                });
                continue;
            }
        };
        let versions = match content::parse_versions(provider, &text) {
            Ok(versions) => versions,
            Err(err) => {
                result.failed.push(FailedFile {
                    file_name: item.file_name.clone(),
                    reason: err.to_string(),
                });
                continue;
            }
        };
        let Some(latest) = versions.first() else {
            result.failed.push(FailedFile {
                file_name: item.file_name.clone(),
                reason: "该版本没有可用文件".to_string(),
            });
            continue;
        };
        if latest.id == record.version_id {
            continue;
        }
        let file_name = latest
            .file_name
            .clone()
            .unwrap_or_else(|| item.file_name.clone());
        let download_url = match latest.download_url.clone() {
            Some(url) => url,
            None => match content::file_url(
                provider,
                &record.project_id,
                &latest.id,
                latest.file_id,
                &file_name,
            ) {
                Ok(url) => url,
                Err(err) => {
                    result.failed.push(FailedFile {
                        file_name: item.file_name.clone(),
                        reason: err.to_string(),
                    });
                    continue;
                }
            },
        };
        result.updates.push(ContentUpdateItem {
            file_name: item.file_name.clone(),
            current_version: record.version_number.clone(),
            latest_version: latest.version_number.clone(),
            download_url,
            sha1: latest.sha1.clone(),
        });
    }
    Ok(result)
}

/// 应用更新：下载 → 校验 → 原子替换（失败保留原文件）。
pub async fn apply_updates(
    client: &reqwest::Client,
    dir: &Path,
    kind: ServerContentKind,
    items: &[ContentUpdateItem],
    via_mcim: bool,
) -> Result<ContentApplyResult> {
    let target = kind_dir(dir, kind);
    std::fs::create_dir_all(&target)?;
    let mut result = ContentApplyResult::default();
    for item in items {
        match replace_one(client, &target, item, via_mcim).await {
            Ok(()) => result.updated += 1,
            Err(err) => result.failed.push(FailedFile {
                file_name: item.file_name.clone(),
                reason: err.to_string(),
            }),
        }
    }
    Ok(result)
}

/// 单个文件的「下载 → 校验 → 替换」。
pub async fn replace_one(
    client: &reqwest::Client,
    target_dir: &Path,
    item: &ContentUpdateItem,
    via_mcim: bool,
) -> Result<()> {
    let file_name = validate_file_name(&item.file_name)?;
    let bytes = content::download_bytes(client, &item.download_url, via_mcim).await?;
    if let Some(expected) = item.sha1.as_deref().filter(|value| !value.trim().is_empty()) {
        let actual = util::sha1_hex(&bytes);
        if !actual.eq_ignore_ascii_case(expected) {
            return Err(AppError::Net("sha1 校验失败，已放弃写入".to_string()));
        }
    }
    let destination = target_dir.join(&file_name);
    let tmp = target_dir.join(format!("{file_name}.part"));
    std::fs::write(&tmp, &bytes)?;
    let backup = target_dir.join(format!("{file_name}.bak"));
    if destination.is_file() {
        std::fs::copy(&destination, &backup)?;
    }
    match std::fs::rename(&tmp, &destination) {
        Ok(()) => {
            let _ = std::fs::remove_file(&backup);
            Ok(())
        }
        Err(err) => {
            let _ = std::fs::remove_file(&tmp);
            if backup.is_file() {
                let _ = std::fs::rename(&backup, &destination);
            }
            Err(AppError::Io(err))
        }
    }
}

// ------------------------------------------------------------------ 存档

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerSaveItem {
    pub name: String,
    pub size_bytes: u64,
    pub updated_at: String,
}

/// server.properties 里的 level-name（读不到就用 world）。
pub fn level_name(dir: &Path) -> String {
    let text = std::fs::read_to_string(dir.join(PROPERTIES_FILE)).unwrap_or_default();
    PropertySet::parse(&text)
        .get("level-name")
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_LEVEL_NAME)
        .to_string()
}

/// 存档列表：level-name 本体 + _nether / _the_end + 同级其它 world* 目录。
pub fn list_saves(dir: &Path) -> Result<Vec<ServerSaveItem>> {
    let level = level_name(dir);
    let mut names: Vec<String> = vec![
        level.clone(),
        format!("{level}_nether"),
        format!("{level}_the_end"),
    ];
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.filter_map(|entry| entry.ok()) {
            if !entry.path().is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("world") && !names.contains(&name) {
                names.push(name);
            }
        }
    }
    let mut out = Vec::new();
    for name in names {
        let path = dir.join(&name);
        if !path.is_dir() {
            continue;
        }
        let meta = std::fs::metadata(&path).ok();
        let updated = meta
            .as_ref()
            .and_then(|meta| meta.modified().ok())
            .map(|time| {
                time.duration_since(std::time::UNIX_EPOCH)
                    .map(|delta| util::iso8601(delta.as_secs()))
                    .unwrap_or_default()
            })
            .unwrap_or_default();
        out.push(ServerSaveItem {
            name,
            size_bytes: dir_size(&path),
            updated_at: updated,
        });
    }
    Ok(out)
}

fn dir_size(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut total = 0u64;
    for entry in entries.filter_map(|entry| entry.ok()) {
        match entry.metadata() {
            Ok(meta) if meta.is_dir() => total += dir_size(&entry.path()),
            Ok(meta) => total += meta.len(),
            Err(_) => {}
        }
    }
    total
}

pub fn validate_save_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() || name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err(AppError::Invalid(format!("存档名不合法：{name}")));
    }
    Ok(name.to_string())
}

/// 备份文件名：<存档>-<时间戳>.zip
pub fn backup_file_name(name: &str, stamp: u64) -> String {
    format!("{}-{}.zip", util::sanitize_file_part(name), util::compact_stamp(stamp))
}

/// 把世界目录打成 zip：<server>/backups/<name>-<时间戳>.zip
pub fn backup_save(dir: &Path, name: &str, task: Option<&TaskHandle>) -> Result<PathBuf> {
    let name = validate_save_name(name)?;
    let source = dir.join(&name);
    if !source.is_dir() {
        return Err(AppError::Invalid(format!("存档不存在：{name}")));
    }
    let backups = dir.join(BACKUP_DIR);
    std::fs::create_dir_all(&backups)?;
    let target = backups.join(backup_file_name(&name, util::now_unix()));
    if let Some(task) = task {
        task.progress(
            Phase::Extracting,
            &format!("正在备份 {name} → {}", target.display()),
            0,
            0,
        );
    }
    let file = std::fs::File::create(&target)?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let count = add_dir(&mut zip, &source, &source, options, task)?;
    zip.finish()
        .map_err(|err| AppError::Other(format!("关闭压缩包失败：{err}")))?;
    if let Some(task) = task {
        task.progress(Phase::Finished, "存档备份完成", count as u64, count as u64);
    }
    tracing::info!(path = %target.display(), files = count, "服务端存档已备份");
    Ok(target)
}

fn add_dir<W: std::io::Write + std::io::Seek>(
    zip: &mut zip::ZipWriter<W>,
    root: &Path,
    current: &Path,
    options: zip::write::SimpleFileOptions,
    task: Option<&TaskHandle>,
) -> Result<usize> {
    let mut count = 0usize;
    let Ok(entries) = std::fs::read_dir(current) else {
        return Ok(0);
    };
    for entry in entries.filter_map(|entry| entry.ok()) {
        let path = entry.path();
        if path.is_dir() {
            count += add_dir(zip, root, &path, options, task)?;
        } else {
            let relative = path
                .strip_prefix(root)
                .map(|item| item.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();
            zip.start_file(relative, options)
                .map_err(|err| AppError::Other(format!("写入压缩包失败：{err}")))?;
            zip.write_all(&std::fs::read(&path)?)?;
            count += 1;
            if let Some(task) = task {
                if count % 64 == 0 {
                    task.progress(Phase::Extracting, "正在写入压缩包", count as u64, 0);
                }
            }
        }
    }
    Ok(count)
}

// ------------------------------------------------------------------ 配置

/// server_config_read 的返回。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerConfigResult {
    pub text: String,
    pub fields: BTreeMap<String, String>,
}

/// 界面上会显示的常用项（顺序固定）。
pub const CONFIG_FIELDS: [&str; 11] = [
    "motd",
    "server-port",
    "max-players",
    "level-name",
    "gamemode",
    "difficulty",
    "online-mode",
    "view-distance",
    "simulation-distance",
    "pvp",
    "white-list",
];

pub fn read_config(dir: &Path) -> Result<ServerConfigResult> {
    let path = dir.join(PROPERTIES_FILE);
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let set = PropertySet::parse(&text);
    let mut fields = BTreeMap::new();
    for key in CONFIG_FIELDS {
        if let Some(value) = set.get(key) {
            fields.insert(key.to_string(), value.to_string());
        }
    }
    Ok(ServerConfigResult { text, fields })
}

/// 只改 patch 里的 key：注释、顺序、未识别的行原样保留，写前留 .bak。
pub fn write_config(dir: &Path, patch: &BTreeMap<String, String>) -> Result<()> {
    if patch.is_empty() {
        return Ok(());
    }
    std::fs::create_dir_all(dir)?;
    let path = dir.join(PROPERTIES_FILE);
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    if path.is_file() {
        std::fs::copy(&path, dir.join(format!("{PROPERTIES_FILE}.bak")))?;
    }
    let mut set = PropertySet::parse(&text);
    set.patch(patch);
    std::fs::write(&path, set.render())?;
    tracing::info!(keys = patch.len(), path = %path.display(), "服务端配置已写回");
    Ok(())
}

// ------------------------------------------------------------------ 日志

/// 日志尾部：优先 logs/latest.log，其次 logs 里最新的 .log。
pub fn logs_tail(dir: &Path, lines: usize) -> Result<Vec<String>> {
    let logs = dir.join("logs");
    let latest = logs.join("latest.log");
    let file = if latest.is_file() {
        Some(latest)
    } else {
        let mut candidates: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(&logs)
            .map(|entries| {
                entries
                    .filter_map(|entry| entry.ok())
                    .map(|entry| entry.path())
                    .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("log"))
                    .filter_map(|path| {
                        let time = std::fs::metadata(&path).ok()?.modified().ok()?;
                        Some((time, path))
                    })
                    .collect()
            })
            .unwrap_or_default();
        candidates.sort_by(|left, right| left.0.cmp(&right.0));
        candidates.pop().map(|(_, path)| path)
    };
    let Some(file) = file else {
        return Ok(Vec::new());
    };
    let text = std::fs::read_to_string(&file).unwrap_or_default();
    let all: Vec<&str> = text.lines().collect();
    let tail = all.len().saturating_sub(lines);
    Ok(all[tail..].iter().map(|line| line.to_string()).collect())
}

// ------------------------------------------------------------------ 安装

/// server_content_install 的实际动作：解析版本文件 → 下载 → 校验 → 落盘 → 记来源。
pub async fn install(
    client: &reqwest::Client,
    dir: &Path,
    kind: ServerContentKind,
    provider: Provider,
    project_id: &str,
    version_id: &str,
    mc_version: &str,
    loader: &str,
    api_key: Option<&str>,
    via_mcim: bool,
    task: &TaskHandle,
) -> Result<PathBuf> {
    task.progress(Phase::Downloading, "正在读取版本文件列表", 0, 0);
    let url = content::versions_url(provider, project_id, Some(mc_version), Some(loader))?;
    let text = content::http_text(client, &url, api_key).await?;
    let versions = content::parse_versions(provider, &text)?;
    let chosen = versions
        .iter()
        .find(|item| item.id == version_id)
        .or_else(|| versions.first())
        .ok_or_else(|| AppError::Other(format!("项目 {project_id} 没有可用的版本文件")))?;
    let file_name = chosen
        .file_name
        .clone()
        .ok_or_else(|| AppError::Other("版本文件缺少文件名".to_string()))?;
    let download_url = match chosen.download_url.clone() {
        Some(url) => url,
        None => content::file_url(provider, project_id, &chosen.id, chosen.file_id, &file_name)?,
    };
    let target_dir = kind_dir(dir, kind);
    std::fs::create_dir_all(&target_dir)?;
    let item = ContentUpdateItem {
        file_name: file_name.clone(),
        current_version: None,
        latest_version: chosen.version_number.clone(),
        download_url,
        sha1: chosen.sha1.clone(),
    };
    task.progress(Phase::Downloading, &format!("正在下载 {file_name}"), 0, 0);
    replace_one(client, &target_dir, &item, via_mcim).await?;
    upsert_record(
        dir,
        &file_name,
        ContentRecord {
            provider: provider.id().to_string(),
            project_id: project_id.to_string(),
            version_id: chosen.id.clone(),
            version_number: Some(chosen.version_number.clone()),
            file_name: file_name.clone(),
        },
    )?;
    task.progress(Phase::Finished, "安装完成", 1, 1);
    tracing::info!(file = %file_name, kind = kind.id(), "服务端内容已安装");
    Ok(target_dir.join(file_name))
}
#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "gugu-server-content-{tag}-{}",
            util::unique_suffix()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn disabled_suffix_is_recognised() {
        assert_eq!(split_enabled("EssentialsX.jar"), ("EssentialsX.jar".to_string(), true));
        assert_eq!(
            split_enabled("EssentialsX.jar.disabled"),
            ("EssentialsX.jar".to_string(), false)
        );
        assert!(is_content_jar("a.jar"));
        assert!(is_content_jar("a.jar.disabled"));
        assert!(!is_content_jar("a.jar.bak"));
        assert!(!is_content_jar("README.txt"));
        assert!(!is_content_jar(".disabled"));
        assert!(validate_file_name("../evil.jar").is_err());
        assert!(validate_file_name("a.jar").is_ok());
        assert!(ServerContentKind::from_id("plugin").is_ok());
        assert!(ServerContentKind::from_id("datapack").is_err());
        assert_eq!(ServerContentKind::Plugin.dir_name(), "plugins");
        assert_eq!(ServerContentKind::Mod.loader_for("fabric"), "fabric");
        assert_eq!(ServerContentKind::Plugin.loader_for("paper"), "paper");
    }

    #[test]
    fn list_reports_enabled_state_and_record_source() {
        let dir = temp_dir("list");
        let plugins = kind_dir(&dir, ServerContentKind::Plugin);
        std::fs::create_dir_all(&plugins).unwrap();
        std::fs::write(plugins.join("EssentialsX.jar"), b"jar").unwrap();
        std::fs::write(plugins.join("Vault.jar.disabled"), b"jar").unwrap();
        std::fs::write(plugins.join("config.yml"), b"x").unwrap();
        upsert_record(
            &dir,
            "EssentialsX.jar",
            ContentRecord {
                provider: "modrinth".to_string(),
                project_id: "essentialsx".to_string(),
                version_id: "v1".to_string(),
                version_number: Some("2.20.1".to_string()),
                file_name: "EssentialsX.jar".to_string(),
            },
        )
        .unwrap();
        let items = list_content(&dir, ServerContentKind::Plugin).unwrap();
        assert_eq!(items.len(), 2, "只列 jar，不列 config.yml");
        let enabled = items.iter().find(|item| item.file_name == "EssentialsX.jar").unwrap();
        assert!(enabled.enabled);
        assert_eq!(enabled.provider.as_deref(), Some("modrinth"));
        assert_eq!(enabled.project_id.as_deref(), Some("essentialsx"));
        assert_eq!(enabled.size_bytes, 3);
        let disabled = items
            .iter()
            .find(|item| item.file_name == "Vault.jar.disabled")
            .unwrap();
        assert!(!disabled.enabled);
        assert!(disabled.provider.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn toggle_moves_between_jar_and_jar_disabled() {
        let dir = temp_dir("toggle");
        let plugins = kind_dir(&dir, ServerContentKind::Plugin);
        std::fs::create_dir_all(&plugins).unwrap();
        std::fs::write(plugins.join("EssentialsX.jar"), b"jar").unwrap();
        // 禁用：文件改名成 .disabled
        toggle_content(&dir, ServerContentKind::Plugin, "EssentialsX.jar", false).unwrap();
        assert!(!plugins.join("EssentialsX.jar").exists());
        assert!(plugins.join("EssentialsX.jar.disabled").exists());
        // 再启用：改回来
        toggle_content(&dir, ServerContentKind::Plugin, "EssentialsX.jar.disabled", true).unwrap();
        assert!(plugins.join("EssentialsX.jar").exists());
        assert!(!plugins.join("EssentialsX.jar.disabled").exists());
        // 重复设置同一状态是无操作
        toggle_content(&dir, ServerContentKind::Plugin, "EssentialsX.jar", true).unwrap();
        // 不存在的文件报错
        assert!(toggle_content(&dir, ServerContentKind::Plugin, "Nope.jar", false).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn config_write_only_touches_patched_keys() {
        let dir = temp_dir("config");
        let original = "#Minecraft server properties\r\n#Mon Sep 25 12:00:00 CST 2023\r\nmotd=A Minecraft Server\r\nserver-port=25565\r\nenable-command-block=false\r\n#自定义注释\r\nmax-players=20\r\n";
        std::fs::write(dir.join(PROPERTIES_FILE), original).unwrap();
        let read = read_config(&dir).unwrap();
        assert_eq!(read.fields.get("motd").map(String::as_str), Some("A Minecraft Server"));
        assert_eq!(read.fields.get("server-port").map(String::as_str), Some("25565"));

        let mut patch = std::collections::BTreeMap::new();
        patch.insert("motd".to_string(), "咕咕服".to_string());
        write_config(&dir, &patch).unwrap();
        let after = std::fs::read_to_string(dir.join(PROPERTIES_FILE)).unwrap();
        assert!(after.contains("motd=咕咕服"));
        // 注释、顺序、未识别的行全部保留
        assert!(after.contains("#Minecraft server properties"));
        assert!(after.contains("#自定义注释"));
        assert!(after.contains("enable-command-block=false"));
        assert_eq!(
            after.lines().position(|line| line.starts_with("motd=")),
            original.lines().position(|line| line.starts_with("motd="))
        );
        // 只改了一行
        let before: Vec<&str> = original.lines().collect();
        let now: Vec<&str> = after.lines().collect();
        assert_eq!(before.len(), now.len());
        let changed: Vec<usize> = (0..before.len()).filter(|index| before[*index] != now[*index]).collect();
        assert_eq!(changed.len(), 1, "只有 motd 那一行变了：{changed:?}");
        // 写前留了备份
        assert!(dir.join(format!("{PROPERTIES_FILE}.bak")).is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn config_write_appends_unknown_key_at_the_end() {
        let dir = temp_dir("config-append");
        std::fs::write(dir.join(PROPERTIES_FILE), "motd=x\n").unwrap();
        let mut patch = std::collections::BTreeMap::new();
        patch.insert("allow-flight".to_string(), "true".to_string());
        write_config(&dir, &patch).unwrap();
        let after = std::fs::read_to_string(dir.join(PROPERTIES_FILE)).unwrap();
        assert!(after.contains("motd=x"));
        assert!(after.contains("allow-flight=true"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn level_name_comes_from_properties() {
        let dir = temp_dir("level");
        assert_eq!(level_name(&dir), DEFAULT_LEVEL_NAME);
        std::fs::write(dir.join(PROPERTIES_FILE), "level-name=我的世界\nmotd=x\n").unwrap();
        assert_eq!(level_name(&dir), "我的世界");
        std::fs::write(dir.join(PROPERTIES_FILE), "level-name=\n").unwrap();
        assert_eq!(level_name(&dir), DEFAULT_LEVEL_NAME);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn saves_list_covers_world_siblings() {
        let dir = temp_dir("saves");
        std::fs::write(dir.join(PROPERTIES_FILE), "level-name=world\n").unwrap();
        for name in ["world", "world_nether", "world_the_end", "world_backup", "plugins"] {
            std::fs::create_dir_all(dir.join(name)).unwrap();
            std::fs::write(dir.join(name).join("level.dat"), vec![0u8; 10]).unwrap();
        }
        let saves = list_saves(&dir).unwrap();
        let names: Vec<&str> = saves.iter().map(|item| item.name.as_str()).collect();
        assert!(names.contains(&"world"));
        assert!(names.contains(&"world_nether"));
        assert!(names.contains(&"world_the_end"));
        assert!(names.contains(&"world_backup"));
        assert!(!names.contains(&"plugins"), "非 world* 目录不算存档");
        let world = saves.iter().find(|item| item.name == "world").unwrap();
        assert_eq!(world.size_bytes, 10);
        assert!(world.updated_at.starts_with("20"), "{}", world.updated_at);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn backup_zip_is_named_with_timestamp() {
        let dir = temp_dir("backup");
        std::fs::create_dir_all(dir.join("world/region")).unwrap();
        std::fs::write(dir.join("world/level.dat"), b"level").unwrap();
        std::fs::write(dir.join("world/region/r.0.0.mca"), b"region").unwrap();
        let name = backup_file_name("world", 1_700_000_000);
        assert!(name.starts_with("world-"), "{name}");
        assert!(name.ends_with(".zip"), "{name}");
        let path = backup_save(&dir, "world", None).unwrap();
        assert!(path.is_file());
        let parent = path.parent().unwrap();
        assert_eq!(parent.file_name().unwrap().to_string_lossy(), BACKUP_DIR);
        assert!(path.file_name().unwrap().to_string_lossy().starts_with("world-"));
        // zip 里有两份文件
        let file = std::fs::File::open(&path).unwrap();
        let zip = zip::ZipArchive::new(file).unwrap();
        assert_eq!(zip.len(), 2);
        assert!(backup_save(&dir, "nope", None).is_err());
        assert!(backup_save(&dir, "../escape", None).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn check_puts_files_without_record_into_failed() {
        let dir = temp_dir("check");
        let plugins = kind_dir(&dir, ServerContentKind::Plugin);
        std::fs::create_dir_all(&plugins).unwrap();
        std::fs::write(plugins.join("Unknown.jar"), b"jar").unwrap();
        let client = reqwest::Client::new();
        let result = check_content(&client, &dir, ServerContentKind::Plugin, "1.20.1", "paper", None)
            .await
            .unwrap();
        assert_eq!(result.checked, 1);
        assert!(result.updates.is_empty());
        assert_eq!(result.failed.len(), 1);
        assert_eq!(result.failed[0].file_name, "Unknown.jar");
        assert!(result.failed[0].reason.contains("没有安装记录"), "{}", result.failed[0].reason);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn records_roundtrip_and_survive_corruption() {
        let dir = temp_dir("records");
        let record = ContentRecord {
            provider: "modrinth".to_string(),
            project_id: "essentialsx".to_string(),
            version_id: "v9".to_string(),
            version_number: None,
            file_name: "EssentialsX.jar".to_string(),
        };
        upsert_record(&dir, "EssentialsX.jar", record.clone()).unwrap();
        let loaded = load_records(&dir);
        assert_eq!(loaded.files.get("EssentialsX.jar"), Some(&record));
        drop_record(&dir, "EssentialsX.jar").unwrap();
        assert!(load_records(&dir).files.is_empty());
        std::fs::write(record_path(&dir), b"{ not json").unwrap();
        assert!(load_records(&dir).files.is_empty(), "坏记录按空处理，不 panic");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn logs_tail_reads_last_lines() {
        let dir = temp_dir("logs");
        assert!(logs_tail(&dir, 10).unwrap().is_empty());
        std::fs::create_dir_all(dir.join("logs")).unwrap();
        let text: String = (1..=50).map(|index| format!("line{index}\n")).collect();
        std::fs::write(dir.join("logs/latest.log"), text).unwrap();
        let tail = logs_tail(&dir, 5).unwrap();
        assert_eq!(tail.len(), 5);
        assert_eq!(tail[0], "line46");
        assert_eq!(tail[4], "line50");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
