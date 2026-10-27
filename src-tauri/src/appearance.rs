//! 外观背景：四种互斥方案（渐变 / 本地图 / API 图 / 壁纸引擎）的配置、扫描与探测。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{AppError, Result};
use crate::paths::Paths;
use crate::util;

pub const SCHEMES: [&str; 4] = ["gradient", "local", "api", "wallpaper"];
pub const MODES: [&str; 2] = ["single", "shuffle"];
/// 轮播间隔范围（秒）。
pub const INTERVAL_MIN: u16 = 5;
pub const INTERVAL_MAX: u16 = 3600;
/// 单张预览缩略图的 data URL 上限。
/// 单张缩略图上限（超过就不生成，避免几十 MB 的图进 IPC）。
pub const THUMB_LIMIT: usize = 128 * 1024;
/// 单次扫描最多枚举多少个目录项（超出就截断，绝不为了 total 读完整棵目录）。
pub const MAX_SCAN_ENTRIES: usize = 5000;
/// 只有当前页前这么多条才生成缩略图。
pub const THUMB_PER_PAGE: usize = 20;
/// 分页默认/上限。
pub const DEFAULT_PAGE_SIZE: u32 = 20;
pub const MAX_PAGE_SIZE: u32 = 60;
/// 列表缓存有效期（同一 (kind,page,pageSize,目录 mtime) 命中直接返回）。
pub const LIST_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(300);

/// 外观设置（四种方案互斥：scheme 决定谁生效）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Appearance {
    /// gradient / local / api / wallpaper（旧名 scheme，历史值 url→api、steam→wallpaper）
    #[serde(alias = "scheme")]
    pub scheme: String,
    #[serde(alias = "gradientTop", alias = "gradient_from")]
    pub gradient_from: String,
    #[serde(alias = "gradientBottom")]
    pub gradient_to: String,
    #[serde(alias = "localInterval")]
    pub local_interval_sec: u16,
    /// single / shuffle
    #[serde(alias = "localPlay")]
    pub local_mode: String,
    pub local_file: String,
    #[serde(alias = "url")]
    pub api_url: String,
    #[serde(alias = "urlPlay")]
    pub api_mode: String,
    #[serde(alias = "urlInterval")]
    pub api_interval_sec: u16,
    #[serde(alias = "steamDir")]
    pub wallpaper_dir: String,
    #[serde(alias = "steamMode", alias = "steamPlay")]
    pub wallpaper_mode: String,
    #[serde(alias = "steamFile")]
    pub wallpaper_file: String,
    #[serde(alias = "steamInterval")]
    pub wallpaper_interval_sec: u16,
}

/// 历史 scheme 值 → 现在的值（url→api、steam→wallpaper），未知值回落 gradient。
pub fn normalize_scheme(value: &str) -> (String, bool) {
    let lowered = value.trim().to_lowercase();
    match lowered.as_str() {
        "gradient" | "local" | "api" | "wallpaper" => (lowered, false),
        "url" => ("api".to_string(), false),
        "steam" => ("wallpaper".to_string(), false),
        _ => ("gradient".to_string(), true),
    }
}

/// background_list/apply 的 kind：兼容旧前端传 steam / url。
pub fn normalize_kind(kind: &str) -> crate::error::Result<&'static str> {
    match kind.trim().to_lowercase().as_str() {
        "local" | "url" => Ok("local"),
        "wallpaper" | "steam" => Ok("wallpaper"),
        other => Err(crate::error::AppError::Invalid(format!(
            "不支持的背景类型：{other}（允许：local / wallpaper，兼容旧值 url / steam）"
        ))),
    }
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            scheme: "gradient".to_string(),
            // 渐变固定「上面永远是白」，下面用主题色
            gradient_from: "#ffffff".to_string(),
            gradient_to: String::new(),
            local_interval_sec: 30,
            local_mode: "single".to_string(),
            local_file: String::new(),
            api_url: String::new(),
            api_mode: "single".to_string(),
            api_interval_sec: 30,
            wallpaper_dir: String::new(),
            wallpaper_mode: "single".to_string(),
            wallpaper_file: String::new(),
            wallpaper_interval_sec: 30,
        }
    }
}

impl Appearance {
    /// 读盘兜底：非法取值回落到默认，不报错。
    pub fn normalize(mut self) -> Self {
        let defaults = Appearance::default();
        let (scheme, fell_back) = normalize_scheme(&self.scheme);
        if fell_back {
            tracing::warn!(value = %self.scheme, "未知外观方案，回落 gradient");
        }
        self.scheme = scheme;
        if !MODES.contains(&self.local_mode.as_str()) {
            self.local_mode = defaults.local_mode;
        }
        if !MODES.contains(&self.api_mode.as_str()) {
            self.api_mode = defaults.api_mode;
        }
        if !MODES.contains(&self.wallpaper_mode.as_str()) {
            self.wallpaper_mode = defaults.wallpaper_mode;
        }
        if self.gradient_from.trim().is_empty() || !crate::settings::is_valid_accent(&self.gradient_from) {
            self.gradient_from = defaults.gradient_from;
        }
        if !self.gradient_to.trim().is_empty()
            && !crate::settings::is_valid_accent(&self.gradient_to)
        {
            self.gradient_to = String::new();
        }
        if !self.api_url.trim().is_empty() && !crate::settings::is_http_url(&self.api_url) {
            self.api_url = String::new();
        }
        self.local_interval_sec = self.local_interval_sec.clamp(INTERVAL_MIN, INTERVAL_MAX);
        self.api_interval_sec = self.api_interval_sec.clamp(INTERVAL_MIN, INTERVAL_MAX);
        self.wallpaper_interval_sec = self
            .wallpaper_interval_sec
            .clamp(INTERVAL_MIN, INTERVAL_MAX);
        self.normalize_paths();
        self
    }

    fn normalize_paths(&mut self) {
        self.local_file = self.local_file.trim().to_string();
        self.wallpaper_dir = self.wallpaper_dir.trim().to_string();
        self.wallpaper_file = self.wallpaper_file.trim().to_string();
        self.api_url = self.api_url.trim().to_string();
    }

    /// 写盘前校验。
    pub fn validate(&self) -> Result<()> {
        if !SCHEMES.contains(&self.scheme.as_str()) {
            return Err(AppError::Invalid(format!(
                "不支持的外观方案：{}（允许：gradient / local / api / wallpaper，兼容旧值 url / steam）",
                self.scheme
            )));
        }
        for (name, mode) in [
            ("localMode", &self.local_mode),
            ("apiMode", &self.api_mode),
            ("wallpaperMode", &self.wallpaper_mode),
        ] {
            if !MODES.contains(&mode.as_str()) {
                return Err(AppError::Invalid(format!("appearance.{name} 非法：{mode}")));
            }
        }
        if !crate::settings::is_valid_accent(&self.gradient_from) {
            return Err(AppError::Invalid(
                "appearance.gradientFrom 必须是 #rgb 或 #rrggbb".to_string(),
            ));
        }
        if !self.gradient_to.trim().is_empty() && !crate::settings::is_valid_accent(&self.gradient_to)
        {
            return Err(AppError::Invalid(
                "appearance.gradientTo 必须是 #rgb 或 #rrggbb 或留空".to_string(),
            ));
        }
        if self.scheme == "api" && !self.api_url.trim().is_empty()
            && !crate::settings::is_http_url(&self.api_url)
        {
            return Err(AppError::Invalid(
                "appearance.apiUrl 必须是 http(s) 地址".to_string(),
            ));
        }
        for (name, value) in [
            ("localIntervalSec", self.local_interval_sec),
            ("apiIntervalSec", self.api_interval_sec),
            ("wallpaperIntervalSec", self.wallpaper_interval_sec),
        ] {
            if !(INTERVAL_MIN..=INTERVAL_MAX).contains(&value) {
                return Err(AppError::Invalid(format!(
                    "appearance.{name} 必须在 {INTERVAL_MIN}..={INTERVAL_MAX} 之间"
                )));
            }
        }
        Ok(())
    }
}

/// background_list 的元素。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackgroundItem {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub is_video: bool,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub modified: String,
    /// 图片缩略图（视频没有；前端自己给视频出第一帧或占位）
    pub data_url: Option<String>,
}

/// 本地背景目录：.mc/backgrounds
pub fn backgrounds_dir(paths: &Paths) -> PathBuf {
    paths.root.join("backgrounds")
}

/// 支持的图片/视频扩展名。
pub const IMAGE_EXTS: [&str; 6] = ["png", "jpg", "jpeg", "webp", "gif", "bmp"];
pub const VIDEO_EXTS: [&str; 5] = ["mp4", "webm", "ogv", "mov", "mkv"];

fn extension_of(path: &Path) -> String {
    path.extension()
        .map(|ext| ext.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

pub fn is_video(path: &Path) -> bool {
    VIDEO_EXTS.contains(&extension_of(path).as_str())
}

pub fn is_supported(path: &Path) -> bool {
    let ext = extension_of(path);
    IMAGE_EXTS.contains(&ext.as_str()) || VIDEO_EXTS.contains(&ext.as_str())
}

/// PNG / JPEG 的尺寸（其它格式返回 None，前端按原始比例兜底）。
pub fn image_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() > 24 && bytes[..8] == [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a] {
        let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
        let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
        return Some((width, height));
    }
    // JPEG：扫描 SOFn 段
    if bytes.len() > 4 && bytes[0] == 0xff && bytes[1] == 0xd8 {
        let mut index = 2usize;
        while index + 9 < bytes.len() {
            if bytes[index] != 0xff {
                index += 1;
                continue;
            }
            let marker = bytes[index + 1];
            if (0xc0..=0xcf).contains(&marker)
                && marker != 0xc4
                && marker != 0xc8
                && marker != 0xcc
            {
                let height = u16::from_be_bytes([bytes[index + 5], bytes[index + 6]]) as u32;
                let width = u16::from_be_bytes([bytes[index + 7], bytes[index + 8]]) as u32;
                return Some((width, height));
            }
            let length = u16::from_be_bytes([bytes[index + 2], bytes[index + 3]]) as usize;
            index += 2 + length.max(2);
        }
    }
    None
}

/// 只看文件头（PNG/JPEG 的尺寸信息都在前几十 KB 里）。
pub const HEAD_LIMIT: usize = 64 * 1024;

fn read_head(path: &Path, limit: usize) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).ok()?;
    let mut buffer = vec![0u8; limit];
    let read = file.read(&mut buffer).ok()?;
    buffer.truncate(read);
    Some(buffer)
}

fn item_of(path: &Path, with_thumb: bool) -> Option<BackgroundItem> {
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() {
        return None;
    }
    let name = path.file_name()?.to_string_lossy().to_string();
    let video = is_video(path);
    // 尺寸只看文件头（最多 64KB），缩略图只在 <= THUMB_LIMIT 时整读：
    // 几十 GB 的目录也不会被扫描拖垮。
    let head = if video { None } else { read_head(path, HEAD_LIMIT) };
    let bytes = if !video && with_thumb && (metadata.len() as usize) <= THUMB_LIMIT {
        std::fs::read(path).ok()
    } else {
        None
    };
    let (width, height, data_url) = if video {
        (None, None, None)
    } else {
        let size = head.as_deref().and_then(image_size);
        let thumb = if with_thumb {
            bytes.and_then(|bytes| {
                if bytes.len() > THUMB_LIMIT {
                    None
                } else {
                    let mime = match extension_of(path).as_str() {
                        "png" => "image/png",
                        "jpg" | "jpeg" => "image/jpeg",
                        "webp" => "image/webp",
                        _ => "application/octet-stream",
                    };
                    Some(format!("data:{mime};base64,{}", util::base64_encode(&bytes)))
                }
            })
        } else {
            None
        };
        (
            size.map(|(width, _)| width),
            size.map(|(_, height)| height),
            thumb,
        )
    };
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| util::iso8601(duration.as_secs()))
        .unwrap_or_default();
    Some(BackgroundItem {
        name,
        path: path.display().to_string(),
        size: metadata.len(),
        is_video: video,
        width,
        height,
        modified,
        data_url,
    })
}

/// 扫描一个目录里的图片/视频（不递归，按文件名排序）。
pub fn scan_dir(dir: &Path, with_thumb: bool) -> Vec<BackgroundItem> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.filter_map(|entry| entry.ok()) {
        let path = entry.path();
        if path.is_dir() || !is_supported(&path) {
            continue;
        }
        if let Some(item) = item_of(&path, with_thumb) {
            out.push(item);
        }
    }
    out.sort_by(|left, right| left.name.cmp(&right.name));
    out
}


/// background_list 的返回（分页）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackgroundPage {
    pub items: Vec<BackgroundItem>,
    /// 本次枚举到的可用条目数（截断时是下限，不是全量统计）
    pub total: u64,
    pub page: u32,
    pub page_size: u32,
    /// 枚举到 MAX_SCAN_ENTRIES 就停了（剩下没数）
    pub truncated: bool,
}

impl BackgroundPage {
    pub fn empty(page: u32, page_size: u32) -> Self {
        Self {
            items: Vec::new(),
            total: 0,
            page,
            page_size,
            truncated: false,
        }
    }
}

/// 每页条数：默认 20，夹在 1..=60。
pub fn clamp_page_size(value: Option<u32>) -> u32 {
    value.unwrap_or(DEFAULT_PAGE_SIZE).clamp(1, MAX_PAGE_SIZE)
}

/// 枚举目录项（deep=true 时进一层子目录找壁纸引擎的媒体文件），最多 MAX_SCAN_ENTRIES 个。
pub fn collect_page_entries(dir: &Path, deep: bool) -> (Vec<PathBuf>, bool) {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return (out, false);
    };
    let mut scanned = 0usize;
    for entry in entries.filter_map(|entry| entry.ok()) {
        scanned += 1;
        if scanned > MAX_SCAN_ENTRIES {
            out.sort();
            return (out, true);
        }
        let path = entry.path();
        if path.is_dir() {
            if deep {
                if let Some(media) = media_in_wallpaper_item(&path) {
                    out.push(media);
                }
            }
            continue;
        }
        if is_supported(&path) {
            out.push(path);
        }
    }
    out.sort();
    (out, false)
}

/// 单页扫描（纯同步，调用方负责放到后台线程）：只读当前页的元数据，
/// 且只有前 THUMB_PER_PAGE 条、文件 <= THUMB_LIMIT 才生成缩略图。
pub fn scan_page(dir: &Path, deep: bool, page: u32, page_size: u32) -> BackgroundPage {
    let page = page.max(1);
    let page_size = clamp_page_size(Some(page_size));
    if !dir.is_dir() {
        return BackgroundPage::empty(page, page_size);
    }
    let (paths, truncated) = collect_page_entries(dir, deep);
    let total = paths.len() as u64;
    let offset = ((page - 1) as usize).saturating_mul(page_size as usize);
    let mut items = Vec::new();
    for (index, path) in paths.into_iter().skip(offset).take(page_size as usize).enumerate() {
        if let Some(item) = item_of(&path, index < THUMB_PER_PAGE) {
            items.push(item);
        }
    }
    BackgroundPage {
        items,
        total,
        page,
        page_size,
        truncated,
    }
}

// ---------------------------------------------------------------- 缓存与并发合并

struct PageCacheEntry {
    at: std::time::Instant,
    page: BackgroundPage,
}

fn page_cache() -> &'static std::sync::Mutex<std::collections::HashMap<String, PageCacheEntry>> {
    static CACHE: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, PageCacheEntry>>,
    > = std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

fn kind_locks() -> &'static std::sync::Mutex<std::collections::HashMap<String, std::sync::Arc<tokio::sync::Mutex<()>>>>
{
    static LOCKS: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, std::sync::Arc<tokio::sync::Mutex<()>>>>,
    > = std::sync::OnceLock::new();
    LOCKS.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// 每个目录的实际扫描次数（诊断 + 单测用）。
fn scan_counts() -> &'static std::sync::Mutex<std::collections::HashMap<String, usize>> {
    static COUNTS: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, usize>>> =
        std::sync::OnceLock::new();
    COUNTS.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

fn dir_key(dir: &Path) -> String {
    dir.display().to_string().to_lowercase()
}

pub fn scan_count(dir: &Path) -> usize {
    scan_counts()
        .lock()
        .ok()
        .and_then(|guard| guard.get(&dir_key(dir)).copied())
        .unwrap_or(0)
}

fn bump_scan_count(dir: &Path) {
    if let Ok(mut guard) = scan_counts().lock() {
        *guard.entry(dir_key(dir)).or_insert(0) += 1;
    }
}

/// 缓存键：(kind, 页码, 每页条数, 目录, 目录 mtime)。
pub fn cache_key(kind: &str, dir: &Path, page: u32, page_size: u32) -> String {
    let mtime = std::fs::metadata(dir)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    format!(
        "{}|{}|{}|{}|{}",
        kind.to_lowercase(),
        page,
        page_size,
        dir.display().to_string().to_lowercase(),
        mtime
    )
}

fn cached_page(key: &str) -> Option<BackgroundPage> {
    let guard = page_cache().lock().ok()?;
    let entry = guard.get(key)?;
    if entry.at.elapsed() < LIST_CACHE_TTL {
        Some(entry.page.clone())
    } else {
        None
    }
}

fn store_page(key: &str, page: &BackgroundPage) {
    if let Ok(mut guard) = page_cache().lock() {
        guard.insert(
            key.to_string(),
            PageCacheEntry {
                at: std::time::Instant::now(),
                page: page.clone(),
            },
        );
    }
}

fn kind_lock(kind: &str) -> std::sync::Arc<tokio::sync::Mutex<()>> {
    let key = kind.to_lowercase();
    if let Ok(guard) = kind_locks().lock() {
        if let Some(existing) = guard.get(&key) {
            return existing.clone();
        }
    }
    let created = std::sync::Arc::new(tokio::sync::Mutex::new(()));
    if let Ok(mut guard) = kind_locks().lock() {
        guard.insert(key, created.clone());
    }
    created
}

/// 清空列表缓存（设置里换了壁纸目录、或测试用）。
pub fn clear_list_cache() {
    if let Ok(mut guard) = page_cache().lock() {
        guard.clear();
    }
}

/// 分页取背景列表：扫描在后台线程跑，主线程（含状态锁）全程不被占用；
/// 同一 kind 的并发请求排队合并成一次扫描。
pub async fn list_page(
    kind: &str,
    dir: PathBuf,
    page: Option<u32>,
    page_size: Option<u32>,
) -> crate::error::Result<BackgroundPage> {
    let started = std::time::Instant::now();
    let page = page.unwrap_or(1).max(1);
    let page_size = clamp_page_size(page_size);
    if !dir.is_dir() {
        return Err(crate::error::AppError::Invalid(format!(
            "目录不存在或不可读：{}",
            dir.display()
        )));
    }
    let key = cache_key(kind, &dir, page, page_size);
    if let Some(hit) = cached_page(&key) {
        tracing::debug!(
            kind,
            page,
            page_size,
            elapsed_ms = started.elapsed().as_millis() as u64,
            "背景列表命中缓存"
        );
        return Ok(hit);
    }
    let lock = kind_lock(kind);
    let _guard = lock.lock().await;
    // 排队期间别人可能已经扫完了
    if let Some(hit) = cached_page(&key) {
        tracing::debug!(kind, page, page_size, "背景列表命中缓存（合并请求）");
        return Ok(hit);
    }
    let deep = kind != "local";
    let dir_for_count = dir.clone();
    let scan_dir_path = dir.clone();
    let scanned = tokio::task::spawn_blocking(move || scan_page(&scan_dir_path, deep, page, page_size))
        .await
        .map_err(|err| crate::error::AppError::Other(format!("扫描背景目录失败：{err}")))?;
    bump_scan_count(&dir_for_count);
    store_page(&key, &scanned);
    tracing::info!(
        kind,
        page,
        page_size,
        items = scanned.items.len(),
        total = scanned.total,
        truncated = scanned.truncated,
        elapsed_ms = started.elapsed().as_millis() as u64,
        "背景目录扫描完成"
    );
    Ok(scanned)
}

/// background_list(kind="local")：.mc/backgrounds
pub fn list_local(paths: &Paths, with_thumb: bool) -> Vec<BackgroundItem> {
    scan_dir(&backgrounds_dir(paths), with_thumb)
}

/// 壁纸引擎的一个创意工坊条目里，媒体文件可能叫 scene.pkg / project.json + 视频/图片，
/// 预览图固定叫 preview.gif|jpg|png。这里按优先级挑一个可渲染的文件。
pub fn media_in_wallpaper_item(item_dir: &Path) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    let entries = std::fs::read_dir(item_dir).ok()?;
    for entry in entries.filter_map(|entry| entry.ok()) {
        let path = entry.path();
        if !path.is_file() || !is_supported(&path) {
            continue;
        }
        // 预览图只是缩略图，优先选真正的媒体文件
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if name.starts_with("preview") {
            continue;
        }
        candidates.push(path);
    }
    candidates.sort();
    if let Some(found) = candidates.first() {
        return Some(found.clone());
    }
    // 只有 preview.* 也认（有些条目是 web 壁纸）
    let entries = std::fs::read_dir(item_dir).ok()?;
    for entry in entries.filter_map(|entry| entry.ok()) {
        let path = entry.path();
        if path.is_file() && is_supported(&path) {
            return Some(path);
        }
    }
    None
}

fn wallpaper_item(item_dir: &Path, with_thumb: bool) -> Option<BackgroundItem> {
    let media = media_in_wallpaper_item(item_dir)?;
    let mut item = item_of(&media, with_thumb)?;
    // 名字用创意工坊目录名（数字 id），前端好定位
    item.name = item_dir
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or(item.name);
    Some(item)
}

/// background_list(kind="wallpaper")：壁纸引擎创意工坊 431960 目录
///
/// 兼容两种布局：目录里直接放媒体文件，或每个创意工坊 id 一个子目录（真实布局）。
pub fn list_wallpaper(dir: &str, with_thumb: bool) -> Vec<BackgroundItem> {
    if dir.trim().is_empty() {
        return Vec::new();
    }
    let root = Path::new(dir.trim());
    let mut out = scan_dir(root, with_thumb);
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.filter_map(|entry| entry.ok()) {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            if let Some(item) = wallpaper_item(&path, with_thumb) {
                out.push(item);
            }
        }
    }
    out.sort_by(|left, right| left.name.cmp(&right.name));
    out
}

/// 解析 Steam 的 libraryfolders.vdf，取出所有库根目录。
pub fn parse_library_folders(vdf: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in vdf.lines() {
        let trimmed = line.trim();
        if !trimmed.contains("\"path\"") {
            continue;
        }
        let parts: Vec<&str> = trimmed.split('"').collect();
        // "path"  "D:\SteamLibrary"
        if let Some(value) = parts.iter().rev().find(|part| {
            let part = part.trim();
            part.len() > 2 && !part.eq_ignore_ascii_case("path")
        }) {
            let cleaned = value.trim().replace("\\\\", "\\");
            if !cleaned.is_empty() && !out.contains(&cleaned) {
                out.push(cleaned);
            }
        }
    }
    out
}

/// 从注册表读到 Steam 安装目录（读不到返回 None）。
pub fn steam_from_registry() -> Option<String> {
    let mut cmd = std::process::Command::new("reg");
    cmd.args([
        "query",
        "HKCU\\Software\\Valve\\Steam",
        "/v",
        "SteamPath",
    ]);
    util::hidden(&mut cmd);
    let output = cmd.output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout).to_string();
    for line in text.lines() {
        if let Some(index) = line.find("REG_SZ") {
            let value = line[index + "REG_SZ".len()..].trim();
            if !value.is_empty() {
                return Some(value.replace('/', "\\"));
            }
        }
    }
    None
}

/// 在库里定位创意工坊 431960 目录。
pub fn workshop_dir_of(library_root: &Path) -> PathBuf {
    library_root
        .join("steamapps")
        .join("workshop")
        .join("content")
        .join("431960")
}

/// Steam 目录探测结果的进程内缓存（10 分钟内不重复跑 reg）。
pub const STEAM_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(600);

fn steam_cache() -> &'static std::sync::Mutex<Option<(std::time::Instant, Option<String>)>> {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<Option<(std::time::Instant, Option<String>)>>> =
        std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(None))
}

/// 带缓存的默认壁纸目录（设置页反复进来不会一直跑 reg）。
pub fn wallpaper_dir_default() -> Option<String> {
    if let Ok(guard) = steam_cache().lock() {
        if let Some((at, value)) = guard.as_ref() {
            if at.elapsed() < STEAM_CACHE_TTL {
                return value.clone();
            }
        }
    }
    let probed = probe_wallpaper_dir();
    if let Ok(mut guard) = steam_cache().lock() {
        *guard = Some((std::time::Instant::now(), probed.clone()));
    }
    probed
}

/// 真正去读注册表 + libraryfolders.vdf 的那一步。
fn probe_wallpaper_dir() -> Option<String> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(steam) = steam_from_registry() {
        let root = PathBuf::from(&steam);
        for name in ["steamapps/libraryfolders.vdf", "config/libraryfolders.vdf"] {
            let vdf_path = root.join(name.replace('/', &std::path::MAIN_SEPARATOR.to_string()));
            if let Ok(text) = std::fs::read_to_string(&vdf_path) {
                for library in parse_library_folders(&text) {
                    roots.push(PathBuf::from(library));
                }
            }
        }
        roots.push(root);
    }
    for root in roots {
        let candidate = workshop_dir_of(&root);
        if candidate.is_dir() {
            return Some(candidate.display().to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_paths(tag: &str) -> Paths {
        let root = std::env::temp_dir().join(format!("gugu-bg-{tag}-{}", util::unique_suffix()));
        let paths = Paths::from_root(&root);
        paths.ensure().unwrap();
        paths
    }

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        bytes.extend_from_slice(&13u32.to_be_bytes());
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&[8, 6, 0, 0, 0]);
        bytes
    }

    #[test]
    fn legacy_field_aliases_parse() {
        // 旧名 scheme/gradientTop/gradientBottom/localPlay/url/urlPlay/steamDir/steamMode
        let json = r##"{
            "scheme": "local",
            "gradientTop": "#ffffff",
            "gradientBottom": "#f5c845",
            "localPlay": "shuffle",
            "localInterval": 45,
            "url": "https://api.example.com/bg",
            "urlPlay": "single",
            "urlInterval": 60,
            "steamDir": "D:\\Steam\\431960",
            "steamMode": "shuffle",
            "steamFile": "a.mp4",
            "steamInterval": 90
        }"##;
        let appearance: Appearance = serde_json::from_str(json).unwrap();
        assert_eq!(appearance.scheme, "local");
        assert_eq!(appearance.gradient_to, "#f5c845");
        assert_eq!(appearance.local_mode, "shuffle");
        assert_eq!(appearance.local_interval_sec, 45);
        assert_eq!(appearance.api_url, "https://api.example.com/bg");
        assert_eq!(appearance.api_interval_sec, 60);
        assert_eq!(appearance.wallpaper_dir, "D:\\Steam\\431960");
        assert_eq!(appearance.wallpaper_mode, "shuffle");
        assert_eq!(appearance.wallpaper_file, "a.mp4");
        assert_eq!(appearance.wallpaper_interval_sec, 90);
    }

    #[test]
    fn legacy_scheme_values_map_to_new_ones() {
        assert_eq!(normalize_scheme("url"), ("api".to_string(), false));
        assert_eq!(normalize_scheme("Steam"), ("wallpaper".to_string(), false));
        assert_eq!(normalize_scheme("api"), ("api".to_string(), false));
        let mut appearance = Appearance::default();
        appearance.scheme = "url".to_string();
        assert_eq!(appearance.normalize().scheme, "api");
        let mut steam = Appearance::default();
        steam.scheme = "steam".to_string();
        assert_eq!(steam.normalize().scheme, "wallpaper");
    }

    #[test]
    fn unknown_scheme_falls_back_with_warning() {
        let (value, fell_back) = normalize_scheme("video");
        assert_eq!(value, "gradient");
        assert!(fell_back);
        let mut appearance = Appearance::default();
        appearance.scheme = "video".to_string();
        assert_eq!(appearance.normalize().scheme, "gradient", "不报错，回落 gradient");
    }

    #[test]
    fn kind_aliases_accept_legacy_names() {
        assert_eq!(normalize_kind("local").unwrap(), "local");
        assert_eq!(normalize_kind("url").unwrap(), "local");
        assert_eq!(normalize_kind("wallpaper").unwrap(), "wallpaper");
        assert_eq!(normalize_kind("steam").unwrap(), "wallpaper");
        assert_eq!(normalize_kind(" Steam ").unwrap(), "wallpaper");
        let err = normalize_kind("gif").unwrap_err().to_string();
        assert!(err.contains("允许：local / wallpaper"), "{err}");
        assert!(err.contains("gif"), "错误里要带收到的值：{err}");
    }

    #[test]
    fn default_appearance_is_gradient_white_top() {
        let appearance = Appearance::default();
        assert_eq!(appearance.scheme, "gradient");
        assert_eq!(appearance.gradient_from, "#ffffff");
        appearance.validate().unwrap();
        let json = serde_json::to_value(&appearance).unwrap();
        assert_eq!(json["gradientFrom"], "#ffffff");
        assert_eq!(json["localMode"], "single");
    }

    #[test]
    fn invalid_appearance_falls_back_and_is_rejected() {
        let mut appearance = Appearance::default();
        appearance.scheme = "video".to_string();
        appearance.gradient_from = "white".to_string();
        appearance.local_interval_sec = 1;
        appearance.local_mode = "random".to_string();
        assert!(appearance.validate().is_err());
        let normalized = appearance.normalize();
        assert_eq!(normalized.scheme, "gradient");
        assert_eq!(normalized.gradient_from, "#ffffff");
        assert_eq!(normalized.local_mode, "single");
        assert_eq!(normalized.local_interval_sec, INTERVAL_MIN);

        let mut bad_api = Appearance::default();
        bad_api.scheme = "api".to_string();
        bad_api.api_url = "ftp://x/y".to_string();
        assert!(bad_api.validate().is_err());
        assert_eq!(bad_api.normalize().api_url, "");
    }

    #[test]
    fn schemes_are_mutually_exclusive_by_contract() {
        for scheme in SCHEMES {
            let mut appearance = Appearance::default();
            appearance.scheme = scheme.to_string();
            appearance.validate().unwrap();
        }
    }

    #[test]
    fn scans_backgrounds_and_reports_size_and_thumb() {
        let paths = temp_paths("scan");
        let dir = backgrounds_dir(&paths);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("bg1.png"), png(1920, 1080)).unwrap();
        std::fs::write(dir.join("bg2.png"), png(1280, 720)).unwrap();
        std::fs::write(dir.join("movie.mp4"), vec![0u8; 128]).unwrap();
        std::fs::write(dir.join("notes.txt"), b"ignored").unwrap();

        let items = list_local(&paths, true);
        assert_eq!(items.len(), 3, "两张图 + 一个视频");
        let first = items.iter().find(|item| item.name == "bg1.png").unwrap();
        assert_eq!(first.width, Some(1920));
        assert_eq!(first.height, Some(1080));
        assert!(first.size > 0);
        assert!(first.data_url.as_deref().unwrap_or_default().starts_with("data:image/png;base64,"));
        assert!(!first.is_video);

        let video = items.iter().find(|item| item.name == "movie.mp4").unwrap();
        assert!(video.is_video);
        assert!(video.data_url.is_none(), "视频不返回缩略图");
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn image_size_parses_png_and_jpeg() {
        assert_eq!(image_size(&png(64, 32)), Some((64, 32)));
        // 最小 JPEG 头：FFD8 + SOF0 段
        let jpeg: Vec<u8> = vec![
            0xff, 0xd8, 0xff, 0xc0, 0x00, 0x11, 0x08, 0x02, 0x58, 0x03, 0x20, 0x00, 0x00, 0x00,
            0x00,
        ];
        assert_eq!(image_size(&jpeg), Some((800, 600)));
        assert_eq!(image_size(b"not an image"), None);
    }

    #[test]
    fn library_folders_vdf_parsing() {
        let vdf = r#"
"libraryfolders"
{
	"0"
	{
		"path"		"C:\\Program Files (x86)\\Steam"
		"label"		""
	}
	"1"
	{
		"path"		"D:\\SteamLibrary"
		"apps" { "431960" "123456" }
	}
}
"#;
        let libs = parse_library_folders(vdf);
        assert_eq!(libs, vec!["C:\\Program Files (x86)\\Steam", "D:\\SteamLibrary"]);
        assert!(parse_library_folders("").is_empty());
    }

    #[test]
    fn workshop_dir_joins_steam_layout() {
        let dir = workshop_dir_of(Path::new("D:\\SteamLibrary"));
        assert!(dir.ends_with("steamapps/workshop/content/431960") || dir.ends_with("steamapps\\workshop\\content\\431960"));
    }

    fn temp_dir_with(tag: &str, count: usize) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("gugu-bg-{tag}-{}", util::unique_suffix()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for index in 0..count {
            std::fs::write(dir.join(format!("item-{index:04}.png")), b"x").unwrap();
        }
        dir
    }

    #[test]
    fn extension_whitelist_decides_video_and_image() {
        assert!(is_video(Path::new("a.mp4")));
        assert!(is_video(Path::new("a.WEBM")));
        assert!(is_video(Path::new("a.ogv")), "ogv 也算视频");
        assert!(!is_video(Path::new("a.png")));
        assert!(is_supported(Path::new("a.gif")));
        assert!(is_supported(Path::new("a.jpeg")));
        assert!(!is_supported(Path::new("a.txt")));
        assert!(!is_supported(Path::new("a.exe")));
    }

    #[test]
    fn page_size_and_boundaries() {
        assert_eq!(clamp_page_size(None), 20);
        assert_eq!(clamp_page_size(Some(20)), 20);
        assert_eq!(clamp_page_size(Some(40)), 40);
        assert_eq!(clamp_page_size(Some(60)), 60);
        assert_eq!(clamp_page_size(Some(0)), 1);
        assert_eq!(clamp_page_size(Some(1000)), MAX_PAGE_SIZE);

        let dir = temp_dir_with("pages", 45);
        let first = scan_page(&dir, false, 1, 20);
        assert_eq!(first.items.len(), 20);
        assert_eq!(first.total, 45);
        assert_eq!(first.page, 1);
        assert!(!first.truncated);
        let second = scan_page(&dir, false, 2, 20);
        assert_eq!(second.items.len(), 20);
        assert!(second.items[0].name > first.items[19].name, "第二页接着第一页");
        let last = scan_page(&dir, false, 3, 20);
        assert_eq!(last.items.len(), 5);
        // 超出末页：空列表但 total 还在
        let beyond = scan_page(&dir, false, 9, 20);
        assert!(beyond.items.is_empty());
        assert_eq!(beyond.total, 45);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_truncates_at_entry_cap() {
        let dir = std::env::temp_dir().join(format!("gugu-bg-cap-{}", util::unique_suffix()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // 造 MAX_SCAN_ENTRIES + 20 个条目（用空文件，够快）
        for index in 0..(MAX_SCAN_ENTRIES + 20) {
            std::fs::write(dir.join(format!("f-{index:05}.png")), b"").unwrap();
        }
        let page = scan_page(&dir, false, 1, 20);
        assert!(page.truncated, "超过上限要标记 truncated");
        assert_eq!(page.items.len(), 20);
        assert!(page.total <= MAX_SCAN_ENTRIES as u64);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn big_files_never_get_thumbnails_or_content_reads() {
        let dir = temp_dir_with("big", 1);
        let big = dir.join("huge.png");
        std::fs::write(&big, vec![0u8; THUMB_LIMIT + 10]).unwrap();
        let page = scan_page(&dir, false, 1, 20);
        let item = page.items.iter().find(|item| item.name == "huge.png").unwrap();
        assert!(item.data_url.is_none(), "超过 128KB 不生成缩略图");
        assert!(item.width.is_none(), "没读文件头就没有尺寸");
        assert!(item.size as usize > THUMB_LIMIT);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_directory_is_a_readable_error() {
        let missing = std::env::temp_dir().join("gugu-bg-missing-dir-xyz");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let err = runtime
            .block_on(list_page("local", missing, Some(1), Some(20)))
            .unwrap_err()
            .to_string();
        assert!(err.contains("目录不存在"), "{err}");
    }

    #[test]
    fn cache_hit_avoids_rescanning() {
        let dir = temp_dir_with("cache", 5);
        clear_list_cache();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let first = runtime
            .block_on(list_page("local", dir.clone(), Some(1), Some(20)))
            .unwrap();
        assert_eq!(scan_count(&dir), 1, "第一次要真的扫");
        let second = runtime
            .block_on(list_page("local", dir.clone(), Some(1), Some(20)))
            .unwrap();
        assert_eq!(scan_count(&dir), 1, "第二次命中缓存不再扫");
        assert_eq!(first.items.len(), second.items.len());
        assert_eq!(first.total, second.total);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn concurrent_requests_share_one_scan() {
        let dir = temp_dir_with("merge", 12);
        clear_list_cache();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let pages = runtime.block_on(async {
            let one = list_page("local", dir.clone(), Some(1), Some(20));
            let two = list_page("local", dir.clone(), Some(1), Some(20));
            let three = list_page("local", dir.clone(), Some(1), Some(20));
            tokio::join!(one, two, three)
        });
        assert!(pages.0.is_ok() && pages.1.is_ok() && pages.2.is_ok());
        assert_eq!(scan_count(&dir), 1, "三个并发请求只扫一次");
        assert_eq!(pages.0.unwrap().total, 12);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cache_key_includes_mtime_and_page() {
        let dir = temp_dir_with("key", 2);
        let a = cache_key("local", &dir, 1, 20);
        let b = cache_key("local", &dir, 2, 20);
        let c = cache_key("local", &dir, 1, 40);
        let d = cache_key("wallpaper", &dir, 1, 20);
        assert_ne!(a, b);
        assert_ne!(a, c);
        assert_ne!(a, d);
        assert_eq!(a, cache_key("LOCAL", &dir, 1, 20), "kind 大小写无关");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn wallpaper_scan_walks_workshop_item_subdirectories() {
        let dir = std::env::temp_dir().join(format!("gugu-ws-{}", util::unique_suffix()));
        let item1 = dir.join("3514403112");
        let item2 = dir.join("3565182130");
        std::fs::create_dir_all(&item1).unwrap();
        std::fs::create_dir_all(&item2).unwrap();
        std::fs::write(item1.join("preview.jpg"), png(100, 50)).unwrap();
        std::fs::write(item1.join("scene.pkg"), b"pkg").unwrap();
        std::fs::write(item1.join("bg.mp4"), vec![0u8; 64]).unwrap();
        std::fs::write(item2.join("preview.png"), png(64, 64)).unwrap();

        let items = list_wallpaper(&dir.display().to_string(), false);
        assert_eq!(items.len(), 2, "{items:?}");
        let video = items.iter().find(|item| item.name == "3514403112").unwrap();
        assert!(video.is_video, "该条目应当挑到 bg.mp4: {video:?}");
        let image = items.iter().find(|item| item.name == "3565182130").unwrap();
        assert!(!image.is_video, "只有 preview.png 时也认：{image:?}");
        assert_eq!(image.width, Some(64));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn wallpaper_list_handles_injected_directory() {
        let dir = std::env::temp_dir().join(format!("gugu-wp-{}", util::unique_suffix()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.png"), png(100, 100)).unwrap();
        std::fs::write(dir.join("b.mp4"), vec![1u8; 32]).unwrap();
        let items = list_wallpaper(&dir.display().to_string(), false);
        assert_eq!(items.len(), 2);
        assert!(items.iter().all(|item| item.data_url.is_none()));
        assert_eq!(list_wallpaper("", false).len(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
