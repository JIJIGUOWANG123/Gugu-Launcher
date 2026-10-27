//! 整合包安装：Modrinth ".mrpack" 与 CurseForge zip → 直接建一个新实例并装进去。
//!
//! 流程：版本详情 → 整包下载到 "<数据根>/cache/packs/" → 解 index/manifest 定加载器与文件清单
//! → 建实例（先写 packState=incomplete）→ 逐条下载文件 → 解 overrides/client-overrides
//! → 写 "<实例目录>/pack.json" → 标 packState=installed 并置为最近游玩。
//!
//! 网络动作全部走 downloader::Fetcher（多源回退 + 分片），进度走 tasks::TaskHandle
//! （taskId 前缀 modpack），实例落库通过 InstanceSink 注入，所以纯逻辑部分可以单测。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;

use crate::content::{self, Provider};
use crate::downloader::Fetcher;
use crate::error::{AppError, Result};
use crate::events::Phase;
use crate::instance::InstanceCreate;
use crate::paths::Paths;
use crate::settings::{MEMORY_MB_MAX, MEMORY_MB_MIN};
use crate::sources::Origin;
use crate::tasks::TaskHandle;
use crate::util;

/// 整合包没传内存时的默认值（任务书约定 2048）。
pub const DEFAULT_MEMORY_MB: u32 = 2048;
/// 下载任务 kind（下载中心按这个字段分组）。
pub const TASK_KIND: &str = "modpack";
/// 下载任务 id 前缀：前端靠 "modpack-" 区分整合包任务。
pub const TASK_PREFIX: &str = "modpack";
/// 实例目录下的整合包元数据文件。
pub const PACK_FILE: &str = "pack.json";
/// 实例名清洗后为空时的兜底名。
pub const FALLBACK_NAME: &str = "整合包实例";
/// 实例名上限（Windows 目录名不能太长）。
pub const NAME_MAX: usize = 48;
pub const PACK_STATE_INSTALLED: &str = "installed";
pub const PACK_STATE_INCOMPLETE: &str = "incomplete";

/// mrpack 的索引文件名（在 zip 根目录）。
const MRPACK_INDEX: &str = "modrinth.index.json";
/// CurseForge 的清单文件名（在 zip 根目录）。
const CF_MANIFEST: &str = "manifest.json";
/// mrpack 的覆盖目录：后者优先，覆盖同路径文件。
const OVERRIDE_ROOTS: [&str; 2] = ["overrides", "client-overrides"];
/// CurseForge 清单没写 overrides 字段时的默认目录。
const CF_OVERRIDES_DEFAULT: &str = "overrides";
/// 失败信息里最多列几条原因。
const FAILURE_PREVIEW: usize = 3;
/// CurseForge 文件下载地址的 MCIM 镜像前缀。
const MCIM_CURSEFORGE: &str = "/curseforge/v1/mods";

// ------------------------------------------------------------------ 纯逻辑：命名 / 内存

/// 整合包标题 → 合法实例名：去掉非法字符与控制字符，压掉多余空格，截断长度。
pub fn sanitize_instance_name(raw: &str) -> String {
    let mut cleaned = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if ch.is_control() {
            continue;
        }
        if matches!(ch, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
            cleaned.push(' ');
            continue;
        }
        cleaned.push(ch);
    }
    let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    // Windows 上目录名不能以点或空格结尾
    let trimmed = collapsed.trim_end_matches(['.', ' ']).trim();
    let name: String = trimmed.chars().take(NAME_MAX).collect();
    let name = name.trim().to_string();
    if name.is_empty() {
        FALLBACK_NAME.to_string()
    } else {
        name
    }
}

/// 实例重名时追加序号（不区分大小写，与 sqlite 的 UNIQUE 语义一致）。
pub fn unique_instance_name(base: &str, taken: &[String]) -> String {
    let base = if base.trim().is_empty() {
        FALLBACK_NAME
    } else {
        base.trim()
    };
    let exists = |candidate: &str| taken.iter().any(|name| name.eq_ignore_ascii_case(candidate));
    if !exists(base) {
        return base.to_string();
    }
    for index in 2..=99 {
        let candidate = format!("{base} ({index})");
        if !exists(&candidate) {
            return candidate;
        }
    }
    format!("{base} ({})", util::unique_suffix())
}

/// 内存：只接受合法区间，越界或没传一律回落 2048。
pub fn normalize_memory(memory_mb: Option<u32>) -> u32 {
    match memory_mb {
        Some(value) if (MEMORY_MB_MIN..=MEMORY_MB_MAX).contains(&value) => value,
        _ => DEFAULT_MEMORY_MB,
    }
}

// ------------------------------------------------------------------ 纯逻辑：加载器

/// 整合包声明的「游戏版本 + 加载器」。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoaderInfo {
    pub mc_version: String,
    pub loader: String,
    pub loader_version: String,
}

/// Modrinth dependencies → 加载器（fabric-loader / quilt-loader / forge / neoforge）。
pub fn loader_from_dependencies(deps: &BTreeMap<String, String>) -> Result<LoaderInfo> {
    let mc_version = deps
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("minecraft"))
        .map(|(_, value)| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| AppError::Invalid("整合包没有声明 minecraft 版本".to_string()))?;
    for (key, loader) in [
        ("fabric-loader", "Fabric"),
        ("quilt-loader", "Quilt"),
        ("forge", "Forge"),
        ("neoforge", "NeoForge"),
    ] {
        if let Some((_, version)) = deps
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(key))
        {
            return Ok(LoaderInfo {
                mc_version,
                loader: loader.to_string(),
                loader_version: version.trim().to_string(),
            });
        }
    }
    Ok(LoaderInfo {
        mc_version,
        loader: "Vanilla".to_string(),
        loader_version: String::new(),
    })
}

/// CurseForge 的 modLoaders[].id（如 forge-47.2.0、fabric-0.15.11）→ (加载器, 版本)。
pub fn loader_from_cf_id(id: &str) -> Result<(String, String)> {
    let text = id.trim();
    if text.is_empty() || text.eq_ignore_ascii_case("vanilla") {
        return Ok(("Vanilla".to_string(), String::new()));
    }
    let lower = text.to_lowercase();
    for (prefix, loader) in [
        ("neoforge", "NeoForge"),
        ("fabric-loader", "Fabric"),
        ("fabric", "Fabric"),
        ("quilt-loader", "Quilt"),
        ("quilt", "Quilt"),
        ("forge", "Forge"),
    ] {
        if let Some(rest) = lower.strip_prefix(prefix) {
            let version = rest.trim_start_matches('-').trim().to_string();
            return Ok((loader.to_string(), version));
        }
    }
    Err(AppError::Invalid(format!("无法识别的整合包加载器：{id}")))
}

// ------------------------------------------------------------------ 纯逻辑：路径安全

/// 整合包里的相对路径 → 安全相对路径：拒绝绝对路径、盘符与 ".."。
pub fn safe_rel_path(raw: &str) -> Result<PathBuf> {
    let text = raw.trim();
    if text.is_empty() {
        return Err(AppError::Invalid("整合包文件路径为空".to_string()));
    }
    if text.starts_with('/') || text.starts_with('\\') {
        return Err(AppError::Invalid(format!(
            "整合包文件路径不能是绝对路径：{raw}"
        )));
    }
    let bytes = text.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' {
        return Err(AppError::Invalid(format!("整合包文件路径不能带盘符：{raw}")));
    }
    let mut out = PathBuf::new();
    for part in text.split(|ch| ch == '/' || ch == '\\') {
        let part = part.trim();
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            return Err(AppError::Invalid(format!(
                "整合包文件路径不能含 ..：{raw}"
            )));
        }
        if part.contains(':') {
            return Err(AppError::Invalid(format!(
                "整合包文件路径不能含冒号：{raw}"
            )));
        }
        out.push(part);
    }
    if out.as_os_str().is_empty() {
        return Err(AppError::Invalid(format!("整合包文件路径无效：{raw}")));
    }
    Ok(out)
}

/// PathBuf → 用 "/" 拼的相对路径（写进 pack.json / 日志时跨平台一致）。
fn slash_path(path: &Path) -> String {
    path.components()
        .map(|part| part.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join("/")
}

// ------------------------------------------------------------------ 纯逻辑：Modrinth index

#[derive(Debug, Clone, Deserialize, Default)]
pub struct MrpackEnv {
    #[serde(default)]
    pub client: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct MrpackFile {
    pub path: String,
    #[serde(default)]
    pub hashes: BTreeMap<String, String>,
    #[serde(default)]
    pub env: Option<MrpackEnv>,
    #[serde(default)]
    pub downloads: Vec<String>,
    #[serde(default, rename = "fileSize")]
    pub file_size: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct MrpackIndex {
    #[serde(default, rename = "formatVersion")]
    pub format_version: i64,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default, rename = "versionId")]
    pub version_id: Option<String>,
    #[serde(default)]
    pub dependencies: BTreeMap<String, String>,
    #[serde(default)]
    pub files: Vec<MrpackFile>,
}

pub fn parse_mrpack_index(text: &str) -> Result<MrpackIndex> {
    let index: MrpackIndex = serde_json::from_str(text)
        .map_err(|err| AppError::Other(format!("modrinth.index.json 解析失败：{err}")))?;
    // 提前校验 dependencies：定位不到游戏版本就不该继续建实例
    loader_from_dependencies(&index.dependencies)?;
    Ok(index)
}

/// env.client == "unsupported" 才跳过；optional 与缺省都照装。
pub fn client_installs(file: &MrpackFile) -> bool {
    match file.env.as_ref().and_then(|env| env.client.as_deref()) {
        Some(value) => !value.trim().eq_ignore_ascii_case("unsupported"),
        None => true,
    }
}

// ------------------------------------------------------------------ 纯逻辑：CurseForge manifest

#[derive(Debug, Clone, Deserialize, Default)]
pub struct CfModLoader {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub primary: bool,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct CfMinecraft {
    #[serde(default)]
    pub version: String,
    #[serde(default, rename = "modLoaders")]
    pub mod_loaders: Vec<CfModLoader>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct CfManifestFile {
    #[serde(default, rename = "projectID")]
    pub project_id: u64,
    #[serde(default, rename = "fileID")]
    pub file_id: u64,
    #[serde(default)]
    pub required: bool,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct CfManifest {
    #[serde(default)]
    pub minecraft: CfMinecraft,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub files: Vec<CfManifestFile>,
    #[serde(default)]
    pub overrides: Option<String>,
}

pub fn parse_cf_manifest(text: &str) -> Result<CfManifest> {
    let manifest: CfManifest = serde_json::from_str(text)
        .map_err(|err| AppError::Other(format!("manifest.json 解析失败：{err}")))?;
    if manifest.minecraft.version.trim().is_empty() {
        return Err(AppError::Invalid(
            "manifest.json 没有 minecraft.version".to_string(),
        ));
    }
    Ok(manifest)
}

/// primary=true 的加载器优先，没有就取第一个。
pub fn primary_loader_id(loaders: &[CfModLoader]) -> Option<&str> {
    loaders
        .iter()
        .find(|loader| loader.primary)
        .or_else(|| loaders.first())
        .map(|loader| loader.id.as_str())
}

/// CurseForge 文件直链接口（有 Key 走官方，无 Key 走 MCIM 镜像）。
pub fn curseforge_download_url_request(api_key: Option<&str>, project_id: u64, file_id: u64) -> String {
    match api_key.map(str::trim).filter(|key| !key.is_empty()) {
        Some(_) => format!(
            "{}/mods/{project_id}/files/{file_id}/download-url",
            content::CURSEFORGE_API
        ),
        None => format!(
            "{}{MCIM_CURSEFORGE}/{project_id}/files/{file_id}/download-url",
            content::MCIM_BASE
        ),
    }
}

pub fn parse_curseforge_download_url(text: &str) -> Result<String> {
    let value: serde_json::Value = serde_json::from_str(text)?;
    value
        .get("data")
        .and_then(|data| data.as_str())
        .map(|url| url.trim().to_string())
        .filter(|url| !url.is_empty())
        .ok_or_else(|| AppError::Net("CurseForge 没有返回文件直链".to_string()))
}

/// 直链 → 文件名（去掉查询串，拒绝非法片段）。
pub fn file_name_from_url(url: &str) -> Option<String> {
    let clean = url.split(|ch| ch == '?' || ch == '#').next().unwrap_or(url);
    let last = clean.rsplit('/').next()?.trim();
    if last.is_empty() || last.contains(':') || last.contains("..") {
        return None;
    }
    Some(last.to_string())
}

// ------------------------------------------------------------------ 纯逻辑：安装计划

/// 一个待安装文件：mrpack 直接给 url，CurseForge 只给 (projectID, fileID)，下载前再换直链。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackFile {
    pub rel_path: String,
    pub url: String,
    pub sha1: Option<String>,
    pub size: Option<u64>,
    pub required: bool,
    pub project_id: Option<u64>,
    pub file_id: Option<u64>,
}

/// 解析出来的安装计划（创建实例与下载都按它走）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackPlan {
    pub name: String,
    pub mc_version: String,
    pub loader: String,
    pub loader_version: String,
    pub files: Vec<PackFile>,
    pub overrides: Vec<String>,
    pub skipped: usize,
}

pub fn plan_from_mrpack(index: &MrpackIndex) -> Result<PackPlan> {
    let info = loader_from_dependencies(&index.dependencies)?;
    let mut files = Vec::new();
    let mut skipped = 0usize;
    for entry in &index.files {
        if !client_installs(entry) {
            skipped += 1;
            continue;
        }
        let rel = safe_rel_path(&entry.path)?;
        let url = entry
            .downloads
            .iter()
            .map(|url| url.trim().to_string())
            .find(|url| !url.is_empty())
            .ok_or_else(|| {
                AppError::Invalid(format!("整合包文件没有下载地址：{}", entry.path))
            })?;
        files.push(PackFile {
            rel_path: slash_path(&rel),
            url,
            sha1: entry
                .hashes
                .get("sha1")
                .map(|hash| hash.trim().to_string())
                .filter(|hash| !hash.is_empty()),
            size: entry.file_size,
            required: true,
            project_id: None,
            file_id: None,
        });
    }
    Ok(PackPlan {
        name: index.name.clone().unwrap_or_default(),
        mc_version: info.mc_version,
        loader: info.loader,
        loader_version: info.loader_version,
        files,
        overrides: OVERRIDE_ROOTS.iter().map(|name| name.to_string()).collect(),
        skipped,
    })
}

pub fn plan_from_curseforge(manifest: &CfManifest) -> Result<PackPlan> {
    let (loader, loader_version) = loader_from_cf_id(
        primary_loader_id(&manifest.minecraft.mod_loaders).unwrap_or_default(),
    )?;
    let mc_version = manifest.minecraft.version.trim().to_string();
    if mc_version.is_empty() {
        return Err(AppError::Invalid(
            "manifest.json 没有 minecraft.version".to_string(),
        ));
    }
    let mut files = Vec::new();
    let mut skipped = 0usize;
    for entry in &manifest.files {
        if entry.project_id == 0 || entry.file_id == 0 {
            skipped += 1;
            continue;
        }
        files.push(PackFile {
            rel_path: String::new(),
            url: String::new(),
            sha1: None,
            size: None,
            required: entry.required,
            project_id: Some(entry.project_id),
            file_id: Some(entry.file_id),
        });
    }
    Ok(PackPlan {
        name: manifest.name.clone().unwrap_or_default(),
        mc_version,
        loader,
        loader_version,
        files,
        overrides: vec![manifest
            .overrides
            .clone()
            .filter(|dir| !dir.trim().is_empty())
            .unwrap_or_else(|| CF_OVERRIDES_DEFAULT.to_string())],
        skipped,
    })
}

/// 整包格式：zip 根目录有 modrinth.index.json 就是 mrpack，否则看 manifest.json。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackFormat {
    Mrpack,
    Curseforge,
}

pub fn detect_format(entry_names: &[String]) -> Result<PackFormat> {
    let has = |wanted: &str| {
        entry_names
            .iter()
            .any(|name| name.trim_start_matches('/').eq_ignore_ascii_case(wanted))
    };
    if has(MRPACK_INDEX) {
        return Ok(PackFormat::Mrpack);
    }
    if has(CF_MANIFEST) {
        return Ok(PackFormat::Curseforge);
    }
    Err(AppError::Invalid(
        "整合包既没有 modrinth.index.json 也没有 manifest.json".to_string(),
    ))
}

// ------------------------------------------------------------------ 纯逻辑：失败汇总

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackFailure {
    pub path: String,
    pub reason: String,
}

/// 失败信息：先报总数，再列前 3 条原因（前端直接显示这段文字）。
pub fn failure_message(failures: &[PackFailure]) -> String {
    let preview = failures
        .iter()
        .take(FAILURE_PREVIEW)
        .map(|item| format!("{}（{}）", item.path, item.reason))
        .collect::<Vec<_>>()
        .join("；");
    format!(
        "整合包安装未完成：{} 个必装文件失败。前 {} 条原因：{preview}",
        failures.len(),
        failures.len().min(FAILURE_PREVIEW)
    )
}

// ------------------------------------------------------------------ pack.json

/// "<实例目录>/pack.json" 的内容。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackInfo {
    pub provider: String,
    pub project_id: String,
    pub version_id: String,
    pub name: String,
    pub mc_version: String,
    pub loader: String,
    pub loader_version: String,
    pub installed_at: String,
    pub files_count: usize,
}

pub fn pack_json_path(instance_dir: &Path) -> PathBuf {
    instance_dir.join(PACK_FILE)
}

pub fn write_pack_json(instance_dir: &Path, info: &PackInfo) -> Result<()> {
    let path = pack_json_path(instance_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(info)?;
    std::fs::write(&path, text)?;
    Ok(())
}

pub fn read_pack_json(instance_dir: &Path) -> Result<PackInfo> {
    let path = pack_json_path(instance_dir);
    let text = std::fs::read_to_string(&path)
        .map_err(|err| AppError::Other(format!("读取 {} 失败：{err}", path.display())))?;
    let info: PackInfo = serde_json::from_str(&text)?;
    Ok(info)
}

// ------------------------------------------------------------------ zip 读取与解压

fn open_zip(zip_path: &Path) -> Result<zip::ZipArchive<std::fs::File>> {
    let file = std::fs::File::open(zip_path).map_err(|err| {
        AppError::Other(format!("打开整合包失败（{}）：{err}", zip_path.display()))
    })?;
    zip::ZipArchive::new(file).map_err(|err| {
        AppError::Other(format!("整合包不是有效的 zip（{}）：{err}", zip_path.display()))
    })
}

fn zip_entry_names(zip_path: &Path) -> Result<Vec<String>> {
    let mut archive = open_zip(zip_path)?;
    let mut names = Vec::with_capacity(archive.len());
    for index in 0..archive.len() {
        if let Ok(entry) = archive.by_index(index) {
            names.push(entry.name().replace('\\', "/"));
        }
    }
    Ok(names)
}

fn read_zip_text(zip_path: &Path, name: &str) -> Result<String> {
    let mut archive = open_zip(zip_path)?;
    let mut entry = archive
        .by_name(name)
        .map_err(|_| AppError::Invalid(format!("整合包里没有 {name}")))?;
    let mut text = String::new();
    std::io::Read::read_to_string(&mut entry, &mut text)?;
    Ok(text)
}

/// overrides 解压结果。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OverrideStats {
    pub files: usize,
    pub client_files: usize,
}

/// 按 prefixes 顺序解压覆盖目录：后面的（client-overrides）覆盖前面的同路径文件。
pub fn extract_overrides(zip_path: &Path, game_dir: &Path, prefixes: &[String]) -> Result<OverrideStats> {
    let mut archive = open_zip(zip_path)?;
    let mut stats = OverrideStats::default();
    for prefix in prefixes {
        let root = prefix.trim().trim_matches('/');
        if root.is_empty() {
            continue;
        }
        let key = format!("{root}/");
        let is_client = root.eq_ignore_ascii_case("client-overrides");
        for index in 0..archive.len() {
            let Ok(mut entry) = archive.by_index(index) else {
                continue;
            };
            if entry.is_dir() {
                continue;
            }
            let name = entry.name().replace('\\', "/");
            let Some(rest) = name.strip_prefix(&key) else {
                continue;
            };
            let rel = safe_rel_path(rest)?;
            let target = game_dir.join(&rel);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut out = std::fs::File::create(&target)?;
            std::io::copy(&mut entry, &mut out)?;
            stats.files += 1;
            if is_client {
                stats.client_files += 1;
            }
        }
    }
    Ok(stats)
}

// ------------------------------------------------------------------ 下载

/// 整合包缓存放哪：<数据根>/cache/packs/<安全名>。
pub fn cache_pack_path(paths: &Paths, file_name: &str) -> PathBuf {
    let mut safe = util::sanitize_file_part(file_name);
    let lower = safe.to_lowercase();
    if !lower.ends_with(".mrpack") && !lower.ends_with(".zip") {
        safe.push_str(".zip");
    }
    paths.cache.join("packs").join(safe)
}

/// 下载文件的 sha1 校验；不匹配就删掉并报可读错误。
pub fn verify_download(dest: &Path, sha1: Option<&str>) -> Result<()> {
    let Some(expected) = sha1.map(str::trim).filter(|hash| !hash.is_empty()) else {
        return Ok(());
    };
    let actual = crate::downloader::sha1_file(dest)?;
    if actual.eq_ignore_ascii_case(expected) {
        return Ok(());
    }
    let _ = std::fs::remove_file(dest);
    let name = dest
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| dest.display().to_string());
    Err(AppError::Net(format!(
        "{name} sha1 校验失败（期望 {expected}，实际 {actual}）"
    )))
}

async fn fetch_once(
    fetcher: &Fetcher,
    url: &str,
    dest: &Path,
    sha1: Option<&str>,
    size: Option<u64>,
) -> Result<u64> {
    let outcome = fetcher.fetch(url, dest, None, size).await?;
    verify_download(dest, sha1)?;
    Ok(outcome.bytes)
}

/// 传输错误的尝试次数（Modrinth CDN 偶发连接被重置，多试几次比直接失败好）。
const TRANSPORT_RETRY_LIMIT: usize = 4;
/// 哈希校验失败只重下 1 次（任务书约定）。
const HASH_RETRY_LIMIT: usize = 2;

/// 下载 + 校验：传输失败最多试 4 次（退避 0.5s/1s/1.5s），sha1 对不上只重下 1 次。
async fn fetch_with_retry(
    fetcher: &Fetcher,
    url: &str,
    dest: &Path,
    sha1: Option<&str>,
    size: Option<u64>,
) -> Result<u64> {
    let mut attempt = 0usize;
    let mut first_error: Option<String> = None;
    loop {
        attempt += 1;
        match fetch_once(fetcher, url, dest, sha1, size).await {
            Ok(bytes) => return Ok(bytes),
            Err(AppError::Cancelled) => return Err(AppError::Cancelled),
            Err(err) => {
                let message = err.to_string();
                let hash_mismatch = message.contains("sha1 校验失败");
                let limit = if hash_mismatch {
                    HASH_RETRY_LIMIT
                } else {
                    TRANSPORT_RETRY_LIMIT
                };
                let _ = std::fs::remove_file(dest);
                if attempt >= limit {
                    return Err(AppError::Other(match first_error {
                        Some(first) => format!("{message}（共尝试 {attempt} 次；首次错误：{first}）"),
                        None => message,
                    }));
                }
                first_error.get_or_insert(message);
                tokio::time::sleep(std::time::Duration::from_millis(500 * attempt as u64)).await;
            }
        }
    }
}

// ------------------------------------------------------------------ 实例落库

/// 整合包安装需要的实例操作（命令层用 sqlite 实现，单测可替身）。
pub trait InstanceSink: Send + Sync {
    fn existing_names(&self) -> Result<Vec<String>>;
    fn create(&self, payload: &InstanceCreate) -> Result<i64>;
    /// 写 packName + packState（installed / incomplete）。
    fn set_pack(&self, id: i64, pack_name: &str, state: &str) -> Result<()>;
    /// 置为最近游玩（instances_recent 能取到）。
    fn touch_recent(&self, id: i64) -> Result<()>;
}

/// 真实实现：每次调用新开一条 sqlite 连接（rusqlite 不是 Sync）。
pub struct DbInstanceSink {
    paths: Paths,
}

impl DbInstanceSink {
    pub fn new(paths: &Paths) -> Self {
        Self {
            paths: paths.clone(),
        }
    }

    fn conn(&self) -> Result<rusqlite::Connection> {
        crate::db::open(&self.paths.db_file)
    }
}

impl InstanceSink for DbInstanceSink {
    fn existing_names(&self) -> Result<Vec<String>> {
        let conn = self.conn()?;
        Ok(crate::instance::list(&conn)?
            .into_iter()
            .map(|item| item.name)
            .collect())
    }

    fn create(&self, payload: &InstanceCreate) -> Result<i64> {
        let conn = self.conn()?;
        Ok(crate::instance::create(&conn, &self.paths, payload)?.id)
    }

    fn set_pack(&self, id: i64, pack_name: &str, state: &str) -> Result<()> {
        let conn = self.conn()?;
        crate::instance::set_pack(&conn, id, pack_name, state)
    }

    fn touch_recent(&self, id: i64) -> Result<()> {
        let conn = self.conn()?;
        crate::instance::touch_last_played(&conn, id)
    }
}

// ------------------------------------------------------------------ 安装编排

/// 安装参数（命令层已把 provider 解析好、名字清洗好）。
#[derive(Debug, Clone)]
pub struct PackRequest {
    pub provider: Provider,
    pub project_id: String,
    pub version_id: String,
    pub instance_name: String,
    pub memory_mb: u32,
}

/// 运行期依赖：网络客户端、路径、下载源与并发。
pub struct PackContext {
    pub client: reqwest::Client,
    pub paths: Paths,
    pub origins: Vec<Origin>,
    pub connections: usize,
    pub concurrency: usize,
    pub curseforge_api_key: String,
    /// 内容文件优先走 MCIM 镜像下载（失败自动回退原始地址）
    pub via_mcim: bool,
}

impl PackContext {
    fn fetcher(&self, task: &TaskHandle) -> Fetcher {
        Fetcher::new(
            self.client.clone(),
            self.origins.clone(),
            task.cancel_flag(),
            self.connections.max(1),
        )
    }

    fn curseforge_key(&self) -> Option<&str> {
        let key = self.curseforge_api_key.trim();
        if key.is_empty() {
            None
        } else {
            Some(key)
        }
    }
}

/// 安装结果（命令层的返回值与日志都用它）。
#[derive(Debug, Clone)]
pub struct PackOutcome {
    pub instance_id: i64,
    pub instance_name: String,
    pub pack_name: String,
    pub mc_version: String,
    pub loader: String,
    pub loader_version: String,
    pub files_count: usize,
    pub skipped_files: usize,
    pub override_files: usize,
    pub client_override_files: usize,
    pub elapsed_secs: u64,
}

impl PackOutcome {
    pub fn summary(&self) -> String {
        format!(
            "整合包 {} → 实例 {}（{}）：{} 个文件、overrides {} 个（client {} 个）、跳过 {} 个，耗时 {}s",
            self.pack_name,
            self.instance_name,
            self.loader,
            self.files_count,
            self.override_files,
            self.client_override_files,
            self.skipped_files,
            self.elapsed_secs
        )
    }
}

/// 版本详情里挑出要装的整合包文件。
async fn pick_pack_version(ctx: &PackContext, request: &PackRequest) -> Result<content::VersionItem> {
    let url = content::versions_url(request.provider, &request.project_id, None, None)?;
    let key = if request.provider == Provider::Curseforge {
        ctx.curseforge_key()
    } else {
        None
    };
    let text = content::http_text(&ctx.client, &url, key).await?;
    let mut versions = content::parse_versions(request.provider, &text)?;
    let wanted = request.version_id.trim();
    let chosen = if wanted.is_empty() {
        versions.into_iter().next()
    } else {
        let position = versions.iter().position(|item| item.id == wanted).or_else(|| {
            let needle = wanted.to_lowercase();
            versions.iter().position(|item| {
                item.file_name
                    .as_deref()
                    .map(|name| name.to_lowercase().contains(&needle))
                    .unwrap_or(false)
            })
        });
        position.map(|index| versions.remove(index))
    };
    chosen.ok_or_else(|| {
        AppError::Other(format!(
            "项目 {} 没有可用的整合包版本（请求 versionId={}）",
            request.project_id, request.version_id
        ))
    })
}

/// CurseForge 条目：先换直链，再把文件名补成 mods/<文件名>。
async fn resolve_pack_file(
    client: &reqwest::Client,
    api_key: Option<&str>,
    file: &PackFile,
) -> Result<(String, String)> {
    if !file.url.trim().is_empty() {
        return Ok((file.rel_path.clone(), file.url.clone()));
    }
    let (Some(project_id), Some(file_id)) = (file.project_id, file.file_id) else {
        return Err(AppError::Invalid("整合包文件缺少下载地址".to_string()));
    };
    let url = curseforge_download_url_request(api_key, project_id, file_id);
    let text = content::http_text(client, &url, api_key).await?;
    let direct = parse_curseforge_download_url(&text)?;
    let name = file_name_from_url(&direct)
        .map(|name| slash_path(&safe_rel_path(&name).unwrap_or_else(|_| PathBuf::from(format!("{file_id}.jar")))))
        .unwrap_or_else(|| format!("{file_id}.jar"));
    Ok((format!("mods/{name}"), direct))
}

fn failure_label(file: &PackFile) -> String {
    if !file.rel_path.is_empty() {
        return file.rel_path.clone();
    }
    match (file.project_id, file.file_id) {
        (Some(project_id), Some(file_id)) => format!("curseforge:{project_id}/{file_id}"),
        _ => "未知文件".to_string(),
    }
}

#[derive(Debug, Default)]
struct InstallStats {
    files: usize,
    skipped: usize,
    failures: Vec<PackFailure>,
    optional_failures: usize,
    overrides: OverrideStats,
}

/// 把计划里的文件下载到实例目录（并发 + 失败汇总）。
async fn download_plan(
    ctx: &PackContext,
    task: &TaskHandle,
    fetcher: &Fetcher,
    plan: &PackPlan,
    game_dir: &Path,
) -> Result<InstallStats> {
    let total = plan.files.len();
    let semaphore = Arc::new(Semaphore::new(ctx.concurrency.max(1)));
    let mut set = tokio::task::JoinSet::new();

    for (index, file) in plan.files.iter().cloned().enumerate() {
        task.check_cancelled()?;
        let permit = semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| AppError::Other("下载并发控制已失效".to_string()))?;
        let client = ctx.client.clone();
        let api_key = ctx.curseforge_api_key.trim().to_string();
        let via_mcim = ctx.via_mcim;
        let fetcher = fetcher.clone();
        let game_dir = game_dir.to_path_buf();
        set.spawn(async move {
            let _permit = permit;
            let key = if api_key.is_empty() {
                None
            } else {
                Some(api_key.as_str())
            };
            let resolved = match resolve_pack_file(&client, key, &file).await {
                Ok(value) => value,
                Err(err) => return (index, file, Err(err)),
            };
            let (rel_path, url) = resolved;
            // 默认改写走 MCIM 镜像；下载失败时 fetch_with_retry 会回退原始地址
            let url = crate::content::preferred_download_url(&url, via_mcim).unwrap_or(url);
            let dest = game_dir.join(Path::new(&rel_path.replace('/', std::path::MAIN_SEPARATOR_STR)));
            if crate::downloader::should_skip(&dest, file.size) {
                return (index, file, Ok(true));
            }
            match fetch_with_retry(&fetcher, &url, &dest, file.sha1.as_deref(), file.size).await {
                Ok(_) => (index, file, Ok(false)),
                Err(err) => (index, file, Err(err)),
            }
        });
    }

    let mut stats = InstallStats::default();
    let mut done = 0usize;
    while let Some(joined) = set.join_next().await {
        done += 1;
        let (_, file, result) =
            joined.map_err(|err| AppError::Other(format!("下载任务异常：{err}")))?;
        match result {
            Ok(skipped) => {
                stats.files += 1;
                if skipped {
                    stats.skipped += 1;
                }
            }
            Err(AppError::Cancelled) => return Err(AppError::Cancelled),
            Err(err) => {
                let label = failure_label(&file);
                if file.required {
                    stats.failures.push(PackFailure {
                        path: label,
                        reason: err.to_string(),
                    });
                } else {
                    stats.optional_failures += 1;
                    tracing::warn!(path = %label, %err, "整合包可选文件下载失败，已跳过");
                }
            }
        }
        task.progress(
            Phase::Downloading,
            &format!("正在安装整合包文件 {done}/{total}"),
            done as u64,
            total as u64,
        );
    }
    Ok(stats)
}

/// 准备阶段的产物：计划 + 已建好的实例 + 落地的整包。
///
/// 拆成两段是为了让 examples 能跑「改坏 sha1 → 实例保留并标 incomplete」这条失败路径。
pub struct PreparedPack {
    pub plan: PackPlan,
    pub zip_path: PathBuf,
    pub instance_id: i64,
    pub instance_name: String,
    pub pack_name: String,
    pub format: PackFormat,
}

/// 第一步：读版本详情 → 下载整包 → 解析计划 → 建实例（先标 incomplete）。
pub async fn prepare(
    ctx: &PackContext,
    request: &PackRequest,
    sink: &dyn InstanceSink,
    task: &TaskHandle,
) -> Result<PreparedPack> {
    ctx.paths.ensure()?;
    let fetcher = ctx.fetcher(task);

    task.progress_forced(Phase::Downloading, "正在读取整合包版本信息", 0, 0);
    let chosen = pick_pack_version(ctx, request).await?;
    let file_name = chosen
        .file_name
        .clone()
        .filter(|name| !name.trim().is_empty())
        .ok_or_else(|| AppError::Other("整合包版本没有文件名".to_string()))?;
    let url = match chosen.download_url.clone().filter(|url| !url.trim().is_empty()) {
        Some(url) => url,
        None => content::file_url(
            request.provider,
            &request.project_id,
            &chosen.id,
            chosen.file_id,
            &file_name,
        )?,
    };
    let zip_path = cache_pack_path(&ctx.paths, &file_name);
    task.progress_forced(
        Phase::Downloading,
        &format!("正在下载整合包 {file_name}"),
        0,
        0,
    );
    fetch_with_retry(&fetcher, &url, &zip_path, chosen.sha1.as_deref(), chosen.size).await?;

    task.progress_forced(Phase::Verifying, "正在解析整合包结构", 0, 0);
    let names = zip_entry_names(&zip_path)?;
    let format = detect_format(&names)?;
    let plan = match format {
        PackFormat::Mrpack => {
            let index = parse_mrpack_index(&read_zip_text(&zip_path, MRPACK_INDEX)?)?;
            plan_from_mrpack(&index)?
        }
        PackFormat::Curseforge => {
            let manifest = parse_cf_manifest(&read_zip_text(&zip_path, CF_MANIFEST)?)?;
            plan_from_curseforge(&manifest)?
        }
    };

    let pack_name = if plan.name.trim().is_empty() {
        request.project_id.trim().to_string()
    } else {
        plan.name.trim().to_string()
    };
    let wanted = if request.instance_name.trim().is_empty() {
        pack_name.clone()
    } else {
        request.instance_name.clone()
    };
    let instance_name =
        unique_instance_name(&sanitize_instance_name(&wanted), &sink.existing_names()?);

    task.progress_forced(Phase::Extracting, "正在创建实例", 0, 0);
    let instance_id = sink.create(&InstanceCreate {
        name: instance_name.clone(),
        mc_version: plan.mc_version.clone(),
        loader: Some(plan.loader.clone()),
        loader_version: Some(plan.loader_version.clone()),
        memory_mb: Some(request.memory_mb),
    })?;
    // 先标 incomplete：中途崩溃/取消时实例卡上会显示「整合包未装完」
    sink.set_pack(instance_id, &pack_name, PACK_STATE_INCOMPLETE)?;

    Ok(PreparedPack {
        plan,
        zip_path,
        instance_id,
        instance_name,
        pack_name,
        format,
    })
}

/// 第二步：装文件 → 解 overrides → 写 pack.json → 标 installed 并置为最近游玩。
pub async fn install_prepared(
    ctx: &PackContext,
    request: &PackRequest,
    sink: &dyn InstanceSink,
    task: &TaskHandle,
    prepared: PreparedPack,
) -> Result<PackOutcome> {
    let started = std::time::Instant::now();
    let fetcher = ctx.fetcher(task);
    let game_dir = crate::instance::game_dir(&ctx.paths, &prepared.instance_name);
    std::fs::create_dir_all(&game_dir)?;

    let outcome = finish_install(
        ctx,
        task,
        &fetcher,
        request,
        &prepared,
        &game_dir,
        started,
    )
    .await;
    // 失败（含取消）一律留 incomplete，实例本身不删
    if outcome.is_err() {
        let _ = sink.set_pack(prepared.instance_id, &prepared.pack_name, PACK_STATE_INCOMPLETE);
        return outcome;
    }
    let outcome = outcome?;
    sink.set_pack(prepared.instance_id, &prepared.pack_name, PACK_STATE_INSTALLED)?;
    sink.touch_recent(prepared.instance_id)?;
    Ok(outcome)
}

/// 安装整合包：准备（建实例）+ 安装两步一次跑完。
///
/// 任何失败都会把实例留在那里并写 packState=incomplete，错误信息里带失败文件与前 3 条原因。
pub async fn install(
    ctx: &PackContext,
    request: &PackRequest,
    sink: &dyn InstanceSink,
    task: &TaskHandle,
) -> Result<PackOutcome> {
    let prepared = prepare(ctx, request, sink, task).await?;
    install_prepared(ctx, request, sink, task, prepared).await
}

async fn finish_install(
    ctx: &PackContext,
    task: &TaskHandle,
    fetcher: &Fetcher,
    request: &PackRequest,
    prepared: &PreparedPack,
    game_dir: &Path,
    started: std::time::Instant,
) -> Result<PackOutcome> {
    let plan = &prepared.plan;
    let mut stats = download_plan(ctx, task, fetcher, plan, game_dir).await?;
    if !stats.failures.is_empty() {
        return Err(AppError::Other(failure_message(&stats.failures)));
    }
    if stats.optional_failures > 0 {
        tracing::warn!(count = stats.optional_failures, "整合包可选文件有缺失");
    }

    task.progress_forced(Phase::Extracting, "正在解压 overrides", 0, 0);
    stats.overrides = extract_overrides(&prepared.zip_path, game_dir, &plan.overrides)?;

    let info = PackInfo {
        provider: request.provider.id().to_string(),
        project_id: request.project_id.trim().to_string(),
        version_id: request.version_id.trim().to_string(),
        name: prepared.pack_name.clone(),
        mc_version: plan.mc_version.clone(),
        loader: plan.loader.clone(),
        loader_version: plan.loader_version.clone(),
        installed_at: util::iso8601(util::now_unix()),
        files_count: stats.files,
    };
    write_pack_json(
        &crate::instance::instance_dir(&ctx.paths, &prepared.instance_name),
        &info,
    )?;

    Ok(PackOutcome {
        instance_id: prepared.instance_id,
        instance_name: prepared.instance_name.clone(),
        pack_name: prepared.pack_name.clone(),
        mc_version: plan.mc_version.clone(),
        loader: plan.loader.clone(),
        loader_version: plan.loader_version.clone(),
        files_count: stats.files,
        skipped_files: stats.skipped,
        override_files: stats.overrides.files,
        client_override_files: stats.overrides.client_files,
        elapsed_secs: started.elapsed().as_secs(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "gugu-modpack-{tag}-{}-{}",
            std::process::id(),
            util::unique_suffix()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
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

    fn mrpack(files: &str, dependencies: &str, name: &str) -> String {
        format!(
            r#"{{"formatVersion":1,"game":"minecraft","versionId":"1.0.0","name":"{name}",
            "dependencies":{dependencies},"files":[{files}]}}"#
        )
    }

    #[test]
    fn mrpack_index_fabric_loader() {
        let json = mrpack(
            r#"{"path":"mods/sodium.jar","hashes":{"sha1":"aa","sha512":"bb"},
                "env":{"client":"required"},"downloads":["https://cdn.example.com/sodium.jar"],"fileSize":2048}"#,
            r#"{"minecraft":"1.20.1","fabric-loader":"0.15.11"}"#,
            "Fabulously Optimized",
        );
        let plan = plan_from_mrpack(&parse_mrpack_index(&json).unwrap()).unwrap();
        assert_eq!(plan.mc_version, "1.20.1");
        assert_eq!(plan.loader, "Fabric");
        assert_eq!(plan.loader_version, "0.15.11");
        assert_eq!(plan.name, "Fabulously Optimized");
        assert_eq!(plan.files.len(), 1);
        assert_eq!(plan.files[0].rel_path, "mods/sodium.jar");
        assert_eq!(plan.files[0].sha1.as_deref(), Some("aa"));
        assert_eq!(plan.files[0].size, Some(2048));
        assert!(plan.files[0].required);
        assert_eq!(plan.overrides, vec!["overrides".to_string(), "client-overrides".to_string()]);
    }

    #[test]
    fn mrpack_index_forge_loader() {
        let json = mrpack("", r#"{"minecraft":"1.20.1","forge":"47.2.0"}"#, "Forge Pack");
        let plan = plan_from_mrpack(&parse_mrpack_index(&json).unwrap()).unwrap();
        assert_eq!(plan.loader, "Forge");
        assert_eq!(plan.loader_version, "47.2.0");
        assert!(plan.files.is_empty());
    }

    #[test]
    fn mrpack_index_neoforge_loader() {
        let json = mrpack("", r#"{"minecraft":"1.21.1","neoforge":"21.1.72"}"#, "Neo Pack");
        let plan = plan_from_mrpack(&parse_mrpack_index(&json).unwrap()).unwrap();
        assert_eq!(plan.mc_version, "1.21.1");
        assert_eq!(plan.loader, "NeoForge");
        assert_eq!(plan.loader_version, "21.1.72");
    }

    #[test]
    fn mrpack_index_quilt_loader_and_missing_minecraft() {
        let quilt = mrpack("", r#"{"minecraft":"1.20.1","quilt-loader":"0.23.1"}"#, "Quilt");
        let plan = plan_from_mrpack(&parse_mrpack_index(&quilt).unwrap()).unwrap();
        assert_eq!(plan.loader, "Quilt");
        assert_eq!(plan.loader_version, "0.23.1");

        // 没有 minecraft 版本必须报错
        let broken = mrpack("", r#"{"fabric-loader":"0.15.11"}"#, "x");
        assert!(parse_mrpack_index(&broken).is_err());
        // 只有 minecraft、没有加载器 ⇒ Vanilla
        let vanilla = mrpack("", r#"{"minecraft":"1.20.1"}"#, "x");
        let plan = plan_from_mrpack(&parse_mrpack_index(&vanilla).unwrap()).unwrap();
        assert_eq!(plan.loader, "Vanilla");
        assert!(plan.loader_version.is_empty());
    }

    #[test]
    fn unsupported_client_files_are_skipped_and_optional_installed() {
        let json = mrpack(
            r#"{"path":"mods/serveronly.jar","hashes":{"sha1":"a"},"env":{"client":"unsupported"},"downloads":["https://x/a.jar"]},
               {"path":"mods/optional.jar","hashes":{"sha1":"b"},"env":{"client":"optional"},"downloads":["https://x/b.jar"]},
               {"path":"mods/noenv.jar","hashes":{"sha1":"c"},"downloads":["https://x/c.jar"]}"#,
            r#"{"minecraft":"1.20.1","fabric-loader":"0.15.11"}"#,
            "env pack",
        );
        let plan = plan_from_mrpack(&parse_mrpack_index(&json).unwrap()).unwrap();
        assert_eq!(plan.skipped, 1, "client=unsupported 要跳过");
        let paths: Vec<&str> = plan.files.iter().map(|file| file.rel_path.as_str()).collect();
        assert_eq!(paths, vec!["mods/optional.jar", "mods/noenv.jar"]);
    }

    #[test]
    fn client_overrides_take_priority() {
        let dir = temp_dir("overrides");
        let zip_path = dir.join("pack.mrpack");
        build_zip(
            &zip_path,
            &[
                ("overrides/config/a.txt", b"base"),
                ("overrides/mods/x.jar", b"jar"),
                ("client-overrides/config/a.txt", b"client"),
                ("unrelated/file.txt", b"nope"),
            ],
        );
        let game_dir = dir.join("game");
        std::fs::create_dir_all(&game_dir).unwrap();
        let stats = extract_overrides(
            &zip_path,
            &game_dir,
            &["overrides".to_string(), "client-overrides".to_string()],
        )
        .unwrap();
        assert_eq!(stats.files, 3);
        assert_eq!(stats.client_files, 1);
        assert_eq!(
            std::fs::read_to_string(game_dir.join("config").join("a.txt")).unwrap(),
            "client",
            "client-overrides 必须覆盖 overrides 的同路径文件"
        );
        assert!(game_dir.join("mods").join("x.jar").is_file());
        assert!(!game_dir.join("unrelated").exists(), "overrides 之外的条目不能解压");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unsafe_pack_paths_are_rejected() {
        assert!(safe_rel_path("mods/ok.jar").is_ok());
        assert!(safe_rel_path("config\\nested\\a.toml").is_ok());
        for bad in [
            "../escape.jar",
            "mods/../../escape.jar",
            "/etc/passwd",
            "\\windows\\system32\\evil.dll",
            "C:\\evil.jar",
            "",
            "   ",
        ] {
            assert!(safe_rel_path(bad).is_err(), "{bad} 应当被拒绝");
        }
        // 计划阶段也要拒绝越界路径
        let json = mrpack(
            r#"{"path":"../../evil.jar","hashes":{"sha1":"a"},"downloads":["https://x/a.jar"]}"#,
            r#"{"minecraft":"1.20.1"}"#,
            "evil",
        );
        assert!(plan_from_mrpack(&parse_mrpack_index(&json).unwrap()).is_err());
    }

    #[test]
    fn cf_manifest_forge_loader() {
        let json = r#"{"minecraft":{"version":"1.20.1","modLoaders":[{"id":"forge-47.2.0","primary":true}]},
            "manifestType":"minecraftModpack","manifestVersion":1,"name":"Forge Pack","version":"1.0.0",
            "files":[{"projectID":238222,"fileID":4011984,"required":true},
                     {"projectID":306612,"fileID":4000000,"required":false}],
            "overrides":"overrides"}"#;
        let plan = plan_from_curseforge(&parse_cf_manifest(json).unwrap()).unwrap();
        assert_eq!(plan.mc_version, "1.20.1");
        assert_eq!(plan.loader, "Forge");
        assert_eq!(plan.loader_version, "47.2.0");
        assert_eq!(plan.name, "Forge Pack");
        assert_eq!(plan.files.len(), 2);
        assert!(plan.files[0].required);
        assert!(!plan.files[1].required, "required=false 是可选文件");
        assert_eq!(plan.files[0].project_id, Some(238222));
        assert_eq!(plan.files[0].file_id, Some(4011984));
        assert_eq!(plan.overrides, vec!["overrides".to_string()]);
    }

    #[test]
    fn cf_manifest_fabric_loader() {
        let json = r#"{"minecraft":{"version":"1.20.1","modLoaders":[{"id":"fabric-0.15.11","primary":true}]},
            "name":"Fabric Pack","files":[],"overrides":"custom_overrides"}"#;
        let manifest = parse_cf_manifest(json).unwrap();
        let plan = plan_from_curseforge(&manifest).unwrap();
        assert_eq!(plan.loader, "Fabric");
        assert_eq!(plan.loader_version, "0.15.11");
        assert_eq!(plan.overrides, vec!["custom_overrides".to_string()]);
        // 没有 overrides 字段 ⇒ 默认 overrides
        let plain = parse_cf_manifest(
            r#"{"minecraft":{"version":"1.19.2","modLoaders":[{"id":"neoforge-20.4.1"}]},"files":[]}"#,
        )
        .unwrap();
        let plan = plan_from_curseforge(&plain).unwrap();
        assert_eq!(plan.loader, "NeoForge");
        assert_eq!(plan.overrides, vec!["overrides".to_string()]);
        // 缺 minecraft.version 直接报错
        assert!(parse_cf_manifest(r#"{"minecraft":{"modLoaders":[]},"files":[]}"#).is_err());
        assert!(loader_from_cf_id("weird-1.0").is_err());
    }

    #[test]
    fn sha1_mismatch_deletes_file_and_reports() {
        let dir = temp_dir("sha1");
        let file = dir.join("a.jar");
        std::fs::write(&file, b"abc").unwrap();
        // abc 的 sha1 = a9993e364706816aba3e25717850c26c9cd0d89d
        assert!(verify_download(&file, Some("a9993e364706816aba3e25717850c26c9cd0d89d")).is_ok());
        assert!(file.is_file(), "校验通过的文件要留着");
        let err = verify_download(&file, Some("deadbeef")).unwrap_err().to_string();
        assert!(err.contains("sha1 校验失败"), "{err}");
        assert!(!file.is_file(), "校验失败要删掉坏文件");
        // 没给哈希就不校验
        std::fs::write(&file, b"abc").unwrap();
        assert!(verify_download(&file, None).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn instance_name_is_sanitized() {
        assert_eq!(
            sanitize_instance_name("a/b\\c:d*e?f\"g<h>i|j"),
            "a b c d e f g h i j"
        );
        assert_eq!(sanitize_instance_name("  Mod Pack / 1.20  "), "Mod Pack 1.20");
        assert_eq!(sanitize_instance_name("坏\u{7}名\u{1}字"), "坏名字");
        assert_eq!(sanitize_instance_name("   "), FALLBACK_NAME);
        assert_eq!(sanitize_instance_name("尾巴..."), "尾巴");
        let long = "安".repeat(80);
        assert_eq!(sanitize_instance_name(&long).chars().count(), NAME_MAX);
        // 清洗后的名字必须能通过实例名校验
        let cleaned = sanitize_instance_name("Fabulously Optimized (1.20.1)");
        assert!(crate::instance::validate_name(&cleaned).is_ok(), "{cleaned}");
    }

    #[test]
    fn memory_out_of_range_falls_back() {
        assert_eq!(normalize_memory(None), DEFAULT_MEMORY_MB);
        assert_eq!(normalize_memory(Some(0)), DEFAULT_MEMORY_MB);
        assert_eq!(normalize_memory(Some(128)), DEFAULT_MEMORY_MB, "低于下限回落 2048");
        assert_eq!(normalize_memory(Some(999_999)), DEFAULT_MEMORY_MB, "高于上限回落 2048");
        assert_eq!(normalize_memory(Some(4096)), 4096);
        assert_eq!(normalize_memory(Some(MEMORY_MB_MIN)), MEMORY_MB_MIN);
    }

    #[test]
    fn pack_json_roundtrip() {
        let dir = temp_dir("packjson");
        let info = PackInfo {
            provider: "modrinth".to_string(),
            project_id: "fabulously-optimized".to_string(),
            version_id: "abc123".to_string(),
            name: "Fabulously Optimized".to_string(),
            mc_version: "1.20.1".to_string(),
            loader: "Fabric".to_string(),
            loader_version: "0.15.11".to_string(),
            installed_at: util::iso8601(util::now_unix()),
            files_count: 42,
        };
        write_pack_json(&dir, &info).unwrap();
        assert_eq!(pack_json_path(&dir), dir.join(PACK_FILE));
        let back = read_pack_json(&dir).unwrap();
        assert_eq!(back, info);
        let json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(pack_json_path(&dir)).unwrap()).unwrap();
        for key in [
            "provider",
            "projectId",
            "versionId",
            "name",
            "mcVersion",
            "loader",
            "loaderVersion",
            "installedAt",
            "filesCount",
        ] {
            assert!(json.get(key).is_some(), "pack.json 缺字段 {key}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn failure_message_lists_first_three_reasons() {
        let failures: Vec<PackFailure> = (0..5)
            .map(|index| PackFailure {
                path: format!("mods/{index}.jar"),
                reason: format!("原因{index}"),
            })
            .collect();
        let message = failure_message(&failures);
        assert!(message.contains("5 个必装文件失败"), "{message}");
        assert!(message.contains("mods/0.jar（原因0）"));
        assert!(message.contains("mods/2.jar（原因2）"));
        assert!(!message.contains("mods/3.jar"), "只列前 3 条：{message}");
    }

    #[test]
    fn unique_instance_name_appends_suffix() {
        let taken = vec!["整合包".to_string(), "整合包 (2)".to_string()];
        assert_eq!(unique_instance_name("新包", &taken), "新包");
        assert_eq!(unique_instance_name("整合包", &taken), "整合包 (3)");
        // 大小写不敏感（sqlite UNIQUE 是二进制，但目录名在 Windows 上不区分大小写）
        assert_eq!(unique_instance_name("新包", &["新包".to_string()]), "新包 (2)");
        assert_eq!(unique_instance_name("  ", &[]), FALLBACK_NAME);
    }

    #[test]
    fn format_detection_reads_root_entries() {
        let mrpack_names = vec!["overrides/a.txt".to_string(), MRPACK_INDEX.to_string()];
        assert_eq!(detect_format(&mrpack_names).unwrap(), PackFormat::Mrpack);
        let cf_names = vec![CF_MANIFEST.to_string(), "overrides/x".to_string()];
        assert_eq!(detect_format(&cf_names).unwrap(), PackFormat::Curseforge);
        assert!(detect_format(&["README.md".to_string()]).is_err());
        // 嵌套的同名文件不算根目录
        assert!(detect_format(&["sub/manifest.json".to_string()]).is_err());
    }

    #[test]
    fn curseforge_download_url_uses_key_or_mirror() {
        let mirror = curseforge_download_url_request(None, 238222, 4011984);
        assert_eq!(
            mirror,
            "https://mod.mcimirror.top/curseforge/v1/mods/238222/files/4011984/download-url"
        );
        let official = curseforge_download_url_request(Some("  key-1  "), 238222, 4011984);
        assert_eq!(
            official,
            "https://api.curseforge.com/v1/mods/238222/files/4011984/download-url"
        );
        assert_eq!(curseforge_download_url_request(Some("   "), 1, 2), curseforge_download_url_request(None, 1, 2));

        assert_eq!(
            parse_curseforge_download_url(r#"{"data":"https://edge.forgecdn.net/files/1/2/x.jar"}"#).unwrap(),
            "https://edge.forgecdn.net/files/1/2/x.jar"
        );
        assert!(parse_curseforge_download_url(r#"{"data":null}"#).is_err());
        assert_eq!(
            file_name_from_url("https://edge.forgecdn.net/files/401/1984/jei.jar?token=1").as_deref(),
            Some("jei.jar")
        );
        assert!(file_name_from_url("https://x/").is_none());
    }

    #[test]
    fn cache_path_keeps_extension_and_sanitizes() {
        let paths = Paths::from_root("D:\\gugu-test");
        let mrpack_path = cache_pack_path(&paths, "Fabulously Optimized.mrpack");
        assert_eq!(mrpack_path.file_name().unwrap(), "Fabulously Optimized.mrpack");
        assert!(mrpack_path.starts_with(paths.cache.join("packs")));
        let zip_path = cache_pack_path(&paths, "pack:1.zip");
        assert_eq!(zip_path.file_name().unwrap(), "pack_1.zip");
        let no_ext = cache_pack_path(&paths, "noext");
        assert_eq!(no_ext.file_name().unwrap(), "noext.zip");
    }
}
