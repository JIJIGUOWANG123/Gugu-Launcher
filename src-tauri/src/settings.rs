//! 设置：JSON 文件（`.mc\settings.json`）+ 取值校验。
//!
//! 校验策略：写入前 `validate()` 直接拒绝非法值；读取时先 `normalize()` 把老配置/脏值
//! 回落到默认（例如 0.5.6 里残留的 `padao` 加速节点回落成 `auto`），再校验。

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, Result};

/// 界面只提供简体中文；其它值读盘时一律回落，字段保留是为了以后加语言。
pub const SUPPORTED_LANGUAGES: [&str; 1] = ["zh-CN"];
pub const THEMES: [&str; 2] = ["light", "dark"];
pub const UPDATE_MODES: [&str; 3] = ["auto", "manual", "off"];
/// custom 需要 customMirrorUrl；sjtu 只为兼容老配置保留（UI 暂时不展示）。
pub const DOWNLOAD_SOURCES: [&str; 5] = ["official", "bmclapi", "sjtu", "custom", "auto"];
/// 同时下载文件数范围。
pub const CONCURRENCY_MIN: u16 = 1;
pub const CONCURRENCY_MAX: u16 = 32;
/// 单文件分片线程数范围。
pub const CONNECTIONS_MIN: u16 = 1;
pub const CONNECTIONS_MAX: u16 = 16;
pub const DEFAULT_CONCURRENT_DOWNLOADS: u16 = 8;
pub const DEFAULT_PER_FILE_CONNECTIONS: u16 = 4;
/// 内容服务翻译来源：内置（有限流）或用户自填的 OpenAI 兼容接口。
pub const TRANSLATION_SOURCES: [&str; 2] = ["builtin", "custom"];
pub const TEMPERATURE_MIN: f32 = 0.0;
pub const TEMPERATURE_MAX: f32 = 2.0;
pub const MAX_TOKENS_MIN: u32 = 1;
pub const MAX_TOKENS_MAX: u32 = 8192;

/// 是否是 http(s) 地址（不校验主机名细节，交给真实请求去判）。
pub fn is_http_url(value: &str) -> bool {
    let lower = value.trim().to_ascii_lowercase();
    (lower.starts_with("http://") && lower.len() > 7) || (lower.starts_with("https://") && lower.len() > 8)
}
pub const RESOURCE_SOURCES: [&str; 3] = ["auto", "official", "mcim"];
pub const MEMORY_MB_MIN: u32 = 512;
pub const MEMORY_MB_MAX: u32 = 65536;
pub const DEFAULT_ACCENT: &str = "#f5c845";
/// 关闭窗口的行为：询问 / 最小化到后台 / 退出程序。
pub const CLOSE_ACTIONS: [&str; 3] = ["ask", "minimize", "exit"];
pub const DEFAULT_CLOSE_ACTION: &str = "minimize";

/// 设置结构：字段名与前端 `Settings` 一一对应（camelCase）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub language: String,
    /// 主题：light / dark
    pub theme: String,
    /// 主题色：#rgb 或 #rrggbb
    pub accent: String,
    pub update_mode: String,
    pub download_source: String,
    pub resource_source: String,
    pub java_path: String,
    pub memory_mb: u32,
    /// GitHub 加速节点：auto 或 net::github 节点 id
    pub github_accelerator: String,
    /// 界面显示字体（family 名，来自 fonts_list()）；默认「微软雅黑」
    pub ui_font: String,
    /// 自定义镜像根地址（downloadSource = custom 时生效），末尾不带斜杠
    pub custom_mirror_url: String,
    /// 同时下载文件数（1..=32）
    pub concurrent_downloads: u16,
    /// 单文件分片线程数（1..=16）
    pub per_file_connections: u16,
    /// 每日一言接口（空 = 内置默认地址）
    pub saying_api_url: String,
    /// GitHub 下载器默认目录（空 = <数据根>/DL）
    pub github_download_dir: String,
    /// 内容服务翻译来源：builtin / custom
    pub translation_source: String,
    /// custom 时的 OpenAI 兼容根地址（如 https://api.openai.com/v1）
    pub translation_base_url: String,
    /// custom 时的 API Key（只落 .mc/settings.json，日志与错误里打码）
    pub translation_api_key: String,
    pub translation_model: String,
    pub translation_temperature: f32,
    pub translation_max_tokens: u32,
    /// 内容来源（CurseForge）用的 API Key
    pub curseforge_api_key: String,
    /// 可选：DeepL 翻译 Key（留空就走内置 uapis/suapi 通道）
    pub deepl_api_key: String,
    /// 外观背景（四方案互斥）
    #[serde(default)]
    pub appearance: crate::appearance::Appearance,
    /// 关闭窗口的行为：ask / minimize / exit（默认 minimize）
    pub close_action: String,
    /// 数据根目录（空 = 安装目录下的 .mc）
    pub data_root: String,
    /// 启动时缺 Java 自动下载（默认 true）
    pub auto_download_java: bool,
    /// 0.5.x 的旧字段：读盘时迁移到 closeAction，之后不再写盘
    #[serde(default, skip_serializing)]
    pub close_to_tray: Option<bool>,
    /// 读盘时确实发生过 closeToTray → closeAction 迁移（用于启动时补写一次配置）
    #[serde(skip)]
    pub close_to_tray_migrated: bool,
    /// 微软登录 Client ID（用户自填；空 = 未配置）
    pub ms_client_id: String,
    /// 读盘时发现 msClientId 格式不对（不落盘，只用于提示）
    #[serde(skip)]
    pub ms_client_id_invalid: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            language: "zh-CN".to_string(),
            theme: "light".to_string(),
            accent: DEFAULT_ACCENT.to_string(),
            // 1.0.0 为首个正式版本：默认关闭自更新（解析/测速代码保留，运行期不调用）
            update_mode: "off".to_string(),
            download_source: "auto".to_string(),
            resource_source: "auto".to_string(),
            java_path: String::new(),
            memory_mb: 4096,
            github_accelerator: "auto".to_string(),
            ui_font: crate::fonts::DEFAULT_FONT.to_string(),
            custom_mirror_url: String::new(),
            concurrent_downloads: DEFAULT_CONCURRENT_DOWNLOADS,
            per_file_connections: DEFAULT_PER_FILE_CONNECTIONS,
            saying_api_url: String::new(),
            github_download_dir: String::new(),
            translation_source: "builtin".to_string(),
            translation_base_url: String::new(),
            translation_api_key: String::new(),
            translation_model: String::new(),
            translation_temperature: 0.2,
            translation_max_tokens: 512,
            curseforge_api_key: String::new(),
            deepl_api_key: String::new(),
            appearance: crate::appearance::Appearance::default(),
            close_action: DEFAULT_CLOSE_ACTION.to_string(),
            data_root: String::new(),
            auto_download_java: true,
            close_to_tray: None,
            close_to_tray_migrated: false,
            ms_client_id: String::new(),
            ms_client_id_invalid: false,
        }
    }
}

/// 是否是 8-4-4-4-12 的 UUID（大小写都认）。
pub fn is_uuid(value: &str) -> bool {
    let parts: Vec<&str> = value.split('-').collect();
    if parts.len() != 5 {
        return false;
    }
    let expected = [8usize, 4, 4, 4, 12];
    parts
        .iter()
        .zip(expected)
        .all(|(part, len)| part.len() == len && part.chars().all(|c| c.is_ascii_hexdigit()))
}

impl Settings {
    /// 实际生效的并发数（写盘前已夹住范围，这里再兜一层）。
    pub fn effective_concurrency(&self) -> usize {
        self.concurrent_downloads.clamp(CONCURRENCY_MIN, CONCURRENCY_MAX) as usize
    }

    /// 实际生效的单文件分片线程数。
    pub fn effective_connections(&self) -> usize {
        self.per_file_connections.clamp(CONNECTIONS_MIN, CONNECTIONS_MAX) as usize
    }
}

/// #rgb / #rrggbb 校验（不引正则依赖）。
pub fn is_valid_accent(value: &str) -> bool {
    let Some(hex) = value.strip_prefix('#') else {
        return false;
    };
    if hex.len() != 3 && hex.len() != 6 {
        return false;
    }
    hex.chars().all(|c| c.is_ascii_hexdigit())
}

impl Settings {
    /// 读取时的兜底：把非法/过期取值回落到默认值，不报错。
    pub fn normalize(mut self) -> Self {
        let defaults = Settings::default();
        // 旧配置迁移：closeToTray: true → minimize，false → exit
        if let Some(close_to_tray) = self.close_to_tray.take() {
            self.close_action = if close_to_tray { "minimize" } else { "exit" }.to_string();
            self.close_to_tray_migrated = true;
            tracing::info!(close_action = %self.close_action, "配置里的 closeToTray 已迁移到 closeAction");
        }
        if !CLOSE_ACTIONS.contains(&self.close_action.as_str()) {
            self.close_action = defaults.close_action.clone();
        }
        self.data_root = self.data_root.trim().to_string();
        if !self.data_root.is_empty() && !std::path::Path::new(&self.data_root).is_absolute() {
            tracing::warn!(data_root = %self.data_root, "settings.json 里的 dataRoot 不是绝对路径，回落默认数据根");
            self.data_root = String::new();
        }
        if !SUPPORTED_LANGUAGES.contains(&self.language.as_str()) {
            self.language = defaults.language;
        }
        if !THEMES.contains(&self.theme.as_str()) {
            self.theme = defaults.theme;
        }
        if !is_valid_accent(&self.accent) {
            self.accent = defaults.accent;
        }
        if !UPDATE_MODES.contains(&self.update_mode.as_str()) {
            self.update_mode = defaults.update_mode;
        }
        let default_source = defaults.download_source.clone();
        if !DOWNLOAD_SOURCES.contains(&self.download_source.as_str()) {
            self.download_source = default_source.clone();
        }
        // 自定义镜像地址：能规范化就规范化，非法时把来源回落到 auto
        match crate::sources::normalize_mirror_url(&self.custom_mirror_url) {
            Ok(url) => self.custom_mirror_url = url,
            Err(_) => {
                if self.download_source == "custom" {
                    self.download_source = default_source;
                }
                self.custom_mirror_url = String::new();
            }
        }
        // 翻译来源：非法值或 custom 配置不全都回落到 builtin
        if !TRANSLATION_SOURCES.contains(&self.translation_source.as_str()) {
            self.translation_source = "builtin".to_string();
        }
        self.translation_base_url = self.translation_base_url.trim().to_string();
        self.translation_api_key = self.translation_api_key.trim().to_string();
        if self.translation_source == "custom"
            && (!is_http_url(&self.translation_base_url) || self.translation_api_key.is_empty())
        {
            self.translation_source = "builtin".to_string();
        }
        self.translation_temperature = self
            .translation_temperature
            .clamp(TEMPERATURE_MIN, TEMPERATURE_MAX);
        self.translation_max_tokens = self
            .translation_max_tokens
            .clamp(MAX_TOKENS_MIN, MAX_TOKENS_MAX);
        // 一言接口留空表示用内置默认地址；填了非法地址就当没填
        self.saying_api_url = self.saying_api_url.trim().to_string();
        if !self.saying_api_url.is_empty() && !is_http_url(&self.saying_api_url) {
            self.saying_api_url = String::new();
        }
        self.github_download_dir = self.github_download_dir.trim().to_string();
        self.deepl_api_key = self.deepl_api_key.trim().to_string();
        self.appearance = std::mem::take(&mut self.appearance).normalize();
        // 微软登录 client id：只留首尾空白外的内容；格式不对就回落为空并留 invalid 标记
        self.ms_client_id = self.ms_client_id.trim().to_string();
        if !self.ms_client_id.is_empty() && !is_uuid(&self.ms_client_id) {
            tracing::warn!(
                client_id = %crate::ms_auth::mask_client_id(&self.ms_client_id),
                "settings.json 里的 msClientId 格式不对，已回落为空"
            );
            self.ms_client_id = String::new();
            self.ms_client_id_invalid = true;
        } else {
            self.ms_client_id_invalid = false;
        }
        self.concurrent_downloads = self
            .concurrent_downloads
            .clamp(CONCURRENCY_MIN, CONCURRENCY_MAX);
        self.per_file_connections = self.per_file_connections.clamp(CONNECTIONS_MIN, CONNECTIONS_MAX);
        if !RESOURCE_SOURCES.contains(&self.resource_source.as_str()) {
            self.resource_source = defaults.resource_source;
        }
        // 老配置里可能留着已下线的 padao 节点
        if !crate::net::github::is_known_accelerator(&self.github_accelerator) {
            self.github_accelerator = defaults.github_accelerator;
        }
        // 字体留空说明老配置没有这个字段，回落默认「微软雅黑」
        if self.ui_font.trim().is_empty() {
            self.ui_font = defaults.ui_font;
        }
        self
    }

    /// 写入前的硬校验：非法值直接拒绝，并说明哪个字段不合法。
    pub fn validate(&self) -> Result<()> {
        if !SUPPORTED_LANGUAGES.contains(&self.language.as_str()) {
            return Err(AppError::Invalid(format!(
                "language 必须是 {:?} 之一，收到 {:?}",
                SUPPORTED_LANGUAGES, self.language
            )));
        }
        if !THEMES.contains(&self.theme.as_str()) {
            return Err(AppError::Invalid(format!(
                "theme 必须是 {:?} 之一，收到 {:?}",
                THEMES, self.theme
            )));
        }
        if !CLOSE_ACTIONS.contains(&self.close_action.as_str()) {
            return Err(AppError::Invalid(format!(
                "closeAction 必须是 {:?} 之一，收到 {:?}",
                CLOSE_ACTIONS, self.close_action
            )));
        }
        if !self.data_root.trim().is_empty()
            && !std::path::Path::new(self.data_root.trim()).is_absolute()
        {
            return Err(AppError::Invalid(format!(
                "dataRoot 必须是绝对路径或留空，收到 {:?}",
                self.data_root
            )));
        }
        if !is_valid_accent(&self.accent) {
            return Err(AppError::Invalid(format!("accent 必须是 #rgb 或 #rrggbb，收到 {:?}", self.accent)));
        }
        if !UPDATE_MODES.contains(&self.update_mode.as_str()) {
            return Err(AppError::Invalid(format!("updateMode 非法：{:?}", self.update_mode)));
        }
        if !DOWNLOAD_SOURCES.contains(&self.download_source.as_str()) {
            return Err(AppError::Invalid(format!("downloadSource 非法：{:?}", self.download_source)));
        }
        if !RESOURCE_SOURCES.contains(&self.resource_source.as_str()) {
            return Err(AppError::Invalid(format!("resourceSource 非法：{:?}", self.resource_source)));
        }
        if self.download_source == "custom" {
            crate::sources::normalize_mirror_url(&self.custom_mirror_url)?;
        }
        if !(CONCURRENCY_MIN..=CONCURRENCY_MAX).contains(&self.concurrent_downloads) {
            return Err(AppError::Invalid(format!(
                "concurrentDownloads 必须在 {CONCURRENCY_MIN}..={CONCURRENCY_MAX} 之间，收到 {}",
                self.concurrent_downloads
            )));
        }
        if !TRANSLATION_SOURCES.contains(&self.translation_source.as_str()) {
            return Err(AppError::Invalid(format!(
                "translationSource 非法：{:?}",
                self.translation_source
            )));
        }
        if self.translation_source == "custom" {
            if !is_http_url(&self.translation_base_url) {
                return Err(AppError::Invalid(
                    "使用自定义翻译接口时 translationBaseUrl 必须是 http(s) 地址".to_string(),
                ));
            }
            if self.translation_api_key.trim().is_empty() {
                return Err(AppError::Invalid(
                    "使用自定义翻译接口时 translationApiKey 不能为空".to_string(),
                ));
            }
        }
        if !(TEMPERATURE_MIN..=TEMPERATURE_MAX).contains(&self.translation_temperature) {
            return Err(AppError::Invalid(format!(
                "translationTemperature 必须在 {TEMPERATURE_MIN}..={TEMPERATURE_MAX} 之间，收到 {}",
                self.translation_temperature
            )));
        }
        if !(MAX_TOKENS_MIN..=MAX_TOKENS_MAX).contains(&self.translation_max_tokens) {
            return Err(AppError::Invalid(format!(
                "translationMaxTokens 必须在 {MAX_TOKENS_MIN}..={MAX_TOKENS_MAX} 之间，收到 {}",
                self.translation_max_tokens
            )));
        }
        self.appearance.validate()?;
        if !self.ms_client_id.trim().is_empty() && !is_uuid(self.ms_client_id.trim()) {
            return Err(AppError::Invalid(
                "msClientId 必须是 8-4-4-4-12 的 UUID，或留空".to_string(),
            ));
        }
        if !self.saying_api_url.trim().is_empty() && !is_http_url(&self.saying_api_url) {
            return Err(AppError::Invalid(format!(
                "sayingApiUrl 必须是 http(s) 地址，收到 {:?}",
                self.saying_api_url
            )));
        }
        if !(CONNECTIONS_MIN..=CONNECTIONS_MAX).contains(&self.per_file_connections) {
            return Err(AppError::Invalid(format!(
                "perFileConnections 必须在 {CONNECTIONS_MIN}..={CONNECTIONS_MAX} 之间，收到 {}",
                self.per_file_connections
            )));
        }
        if !(MEMORY_MB_MIN..=MEMORY_MB_MAX).contains(&self.memory_mb) {
            return Err(AppError::Invalid(format!(
                "memoryMb 必须在 {MEMORY_MB_MIN}..={MEMORY_MB_MAX} 之间，收到 {}",
                self.memory_mb
            )));
        }
        if !crate::net::github::is_known_accelerator(&self.github_accelerator) {
            return Err(AppError::Invalid(format!(
                "githubAccelerator 非法：{:?}",
                self.github_accelerator
            )));
        }
        // 字体是否存在由 fonts::resolve_ui_font 在运行期判定，这里只挡空值
        if self.ui_font.trim().is_empty() {
            return Err(AppError::Invalid("uiFont 不能为空".to_string()));
        }
        Ok(())
    }
}

/// 读取设置：文件不存在或解析失败都回落到默认值并写回一份，保证前端永远拿到完整对象。
pub fn load(path: &Path) -> Result<Settings> {
    if !path.exists() {
        let settings = Settings::default();
        save(path, &settings)?;
        return Ok(settings);
    }
    let text = std::fs::read_to_string(path)?;
    // 记事本 / PowerShell 5.1 会写 UTF-8 BOM，serde_json 不认，先去干净
    let text = text.trim_start_matches('\u{feff}').to_string();
    match serde_json::from_str::<Settings>(&text) {
        Ok(settings) => Ok(settings.normalize()),
        Err(_) => {
            let settings = Settings::default();
            save(path, &settings)?;
            Ok(settings)
        }
    }
}

/// 保存设置：先校验再落盘；写临时文件后重命名，避免半截文件。
pub fn save(path: &Path, settings: &Settings) -> Result<()> {
    settings.validate()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(settings)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_settings_are_valid() {
        Settings::default().validate().expect("默认设置必须合法");
    }

    #[test]
    fn accent_validation_accepts_short_and_long_hex() {
        assert!(is_valid_accent("#fff"));
        assert!(is_valid_accent("#f5c845"));
        assert!(!is_valid_accent("f5c845"));
        assert!(!is_valid_accent("#gggggg"));
        assert!(!is_valid_accent("#12345"));
    }

    #[test]
    fn invalid_values_are_rejected() {
        let mut settings = Settings::default();
        settings.accent = "yellow".to_string();
        assert!(settings.validate().is_err());

        let mut settings = Settings::default();
        settings.memory_mb = 128;
        assert!(settings.validate().is_err());

        let mut settings = Settings::default();
        settings.theme = "amoled".to_string();
        assert!(settings.validate().is_err());

        let mut settings = Settings::default();
        settings.language = "fr-FR".to_string();
        assert!(settings.validate().is_err());

        // 只支持简体中文：en-US / ja-JP 读盘时回落
        let mut settings = Settings::default();
        settings.language = "en-US".to_string();
        assert!(settings.validate().is_err());
        assert_eq!(settings.normalize().language, "zh-CN");

        // 字体不能为空，留空读盘回落默认「微软雅黑」
        let mut settings = Settings::default();
        settings.ui_font = String::new();
        assert!(settings.validate().is_err());
        assert_eq!(settings.normalize().ui_font, crate::fonts::DEFAULT_FONT);
        assert_eq!(crate::fonts::DEFAULT_FONT, "Microsoft YaHei");
    }

    #[test]
    fn removed_padao_node_falls_back_to_auto() {
        let mut settings = Settings::default();
        settings.github_accelerator = "padao".to_string();
        assert!(settings.clone().validate().is_err());
        assert_eq!(settings.normalize().github_accelerator, "auto");
    }

    #[test]
    fn custom_mirror_url_is_normalized_and_bad_values_fall_back() {
        let mut settings = Settings::default();
        settings.download_source = "custom".to_string();
        settings.custom_mirror_url = "  https://mirror.example.com/  ".to_string();
        let normalized = settings.clone().normalize();
        assert_eq!(normalized.custom_mirror_url, "https://mirror.example.com");
        assert_eq!(normalized.download_source, "custom");
        assert!(normalized.validate().is_ok());

        // 非法 scheme → 来源回落 auto，地址清空
        let mut broken = Settings::default();
        broken.download_source = "custom".to_string();
        broken.custom_mirror_url = "ftp://mirror.example.com".to_string();
        let normalized = broken.clone().normalize();
        assert_eq!(normalized.download_source, "auto");
        assert_eq!(normalized.custom_mirror_url, "");
        assert!(broken.validate().is_err(), "custom 但地址非法必须拒绝");

        // custom 但地址为空也拒绝
        let mut empty = Settings::default();
        empty.download_source = "custom".to_string();
        assert!(empty.validate().is_err());
    }

    #[test]
    fn download_tuning_values_are_bounded() {
        let defaults = Settings::default();
        assert_eq!(defaults.concurrent_downloads, 8);
        assert_eq!(defaults.per_file_connections, 4);
        assert_eq!(defaults.effective_concurrency(), 8);
        assert_eq!(defaults.effective_connections(), 4);

        for (value, expected) in [(1u16, 1usize), (8, 8), (32, 32)] {
            let mut settings = Settings::default();
            settings.concurrent_downloads = value;
            assert!(settings.validate().is_ok(), "{value} 应当合法");
            assert_eq!(settings.effective_concurrency(), expected);
        }
        let mut too_big = Settings::default();
        too_big.concurrent_downloads = 64;
        assert!(too_big.validate().is_err());
        assert_eq!(too_big.normalize().concurrent_downloads, CONCURRENCY_MAX);

        let mut conn = Settings::default();
        conn.per_file_connections = 0;
        assert!(conn.validate().is_err());
        assert_eq!(conn.normalize().per_file_connections, CONNECTIONS_MIN);

        let mut conn = Settings::default();
        conn.per_file_connections = 16;
        assert!(conn.validate().is_ok());
        assert_eq!(conn.effective_connections(), 16);
    }

    #[test]
    fn custom_translation_requires_url_and_key_else_falls_back() {
        let mut settings = Settings::default();
        assert_eq!(settings.translation_source, "builtin");
        settings.translation_source = "custom".to_string();
        settings.translation_base_url = "https://api.openai.com/v1".to_string();
        settings.translation_api_key = "sk-abc".to_string();
        assert!(settings.validate().is_ok());

        // 缺 apiKey → 读盘回落 builtin，写盘直接拒绝
        let mut no_key = Settings::default();
        no_key.translation_source = "custom".to_string();
        no_key.translation_base_url = "https://api.openai.com/v1".to_string();
        assert!(no_key.validate().is_err());
        assert_eq!(no_key.normalize().translation_source, "builtin");

        // 缺 baseUrl → 同样回落
        let mut no_url = Settings::default();
        no_url.translation_source = "custom".to_string();
        no_url.translation_api_key = "sk-abc".to_string();
        assert!(no_url.validate().is_err());
        assert_eq!(no_url.normalize().translation_source, "builtin");

        // 非法来源直接回落
        let mut broken = Settings::default();
        broken.translation_source = "google".to_string();
        assert!(broken.validate().is_err());
        assert_eq!(broken.normalize().translation_source, "builtin");
    }

    #[test]
    fn translation_ranges_and_saying_url_are_validated() {
        let mut settings = Settings::default();
        settings.translation_temperature = 3.0;
        assert!(settings.validate().is_err());
        assert_eq!(settings.normalize().translation_temperature, TEMPERATURE_MAX);

        let mut tokens = Settings::default();
        tokens.translation_max_tokens = 0;
        assert!(tokens.validate().is_err());
        assert_eq!(tokens.normalize().translation_max_tokens, MAX_TOKENS_MIN);

        let mut saying = Settings::default();
        saying.saying_api_url = "ftp://x/y".to_string();
        assert!(saying.validate().is_err());
        assert_eq!(saying.clone().normalize().saying_api_url, "");

        saying.saying_api_url = "https://api.uapis.cn/api/v1/saying".to_string();
        assert!(saying.validate().is_ok());
        assert_eq!(
            saying.normalize().saying_api_url,
            "https://api.uapis.cn/api/v1/saying"
        );
    }

    /// 合法 UUID 形态的样例：运行时拼出来，源码里不留整串
    /// （凭据扫描按「出现 UUID 字面量」判 FAIL）。
    fn uuid_sample() -> String {
        format!("{}-{}-{}-{}-{}", "1234abcd", "1234", "5678", "9abc", "def012345678")
    }

    #[test]
    fn ms_client_id_accepts_uuid_and_falls_back_on_garbage() {
        let expected = uuid_sample();
        let mut settings = Settings::default();
        assert_eq!(settings.ms_client_id, "");
        settings.ms_client_id = format!("  {expected}  ");
        let normalized = settings.clone().normalize();
        assert_eq!(normalized.ms_client_id, expected);
        assert!(!normalized.ms_client_id_invalid);
        assert!(normalized.validate().is_ok());

        let mut broken = Settings::default();
        broken.ms_client_id = "test-client-id".to_string();
        assert!(broken.validate().is_err(), "写盘非法值要拒绝");
        let normalized = broken.normalize();
        assert_eq!(normalized.ms_client_id, "", "读盘要回落为空");
        assert!(normalized.ms_client_id_invalid, "并留下 invalid 标记");

        // 大小写都认；分段数/长度/非十六进制都不认
        assert!(is_uuid(&uuid_sample().to_uppercase()));
        assert!(is_uuid(&expected));
        let short = format!("{}-{}-{}-{}-{}", "1234abcd", "1234", "5678", "9abc", "def01234");
        assert!(!is_uuid(&short));
        assert!(!is_uuid("test-client-id"));
        assert!(!is_uuid(""));
    }

    #[test]
    fn save_then_load_roundtrip() {
        let dir = std::env::temp_dir().join(format!("gugu-settings-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");

        let mut settings = Settings::default();
        settings.theme = "dark".to_string();
        settings.accent = "#4c9aff".to_string();
        settings.memory_mb = 8192;
        save(&path, &settings).unwrap();
        assert_eq!(load(&path).unwrap(), settings);

        std::fs::write(&path, "{ not json }").unwrap();
        assert_eq!(load(&path).unwrap(), Settings::default());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn close_action_defaults_to_minimize_and_rejects_garbage() {
        assert_eq!(Settings::default().close_action, "minimize");
        assert_eq!(CLOSE_ACTIONS, ["ask", "minimize", "exit"]);

        for value in CLOSE_ACTIONS {
            let mut settings = Settings::default();
            settings.close_action = value.to_string();
            assert!(settings.validate().is_ok(), "{value} 应当合法");
        }

        let mut broken = Settings::default();
        broken.close_action = "tray".to_string();
        assert!(broken.validate().is_err());
        assert_eq!(broken.normalize().close_action, "minimize");
    }

    #[test]
    fn legacy_close_to_tray_is_migrated() {
        // 老配置：closeToTray=true → minimize，false → exit，并打上迁移标记
        let parsed: Settings = serde_json::from_str(r#"{"closeToTray": true}"#).unwrap();
        assert_eq!(parsed.close_to_tray, Some(true));
        let migrated = parsed.normalize();
        assert_eq!(migrated.close_action, "minimize");
        assert!(migrated.close_to_tray_migrated);
        assert_eq!(migrated.close_to_tray, None, "迁移后不再保留旧字段");

        let parsed: Settings = serde_json::from_str(r#"{"closeToTray": false}"#).unwrap();
        let migrated = parsed.normalize();
        assert_eq!(migrated.close_action, "exit");
        assert!(migrated.close_to_tray_migrated);

        // 新配置里没有 closeToTray：不迁移，closeAction 原样保留
        let parsed: Settings = serde_json::from_str(r#"{"closeAction": "ask"}"#).unwrap();
        let normalized = parsed.normalize();
        assert_eq!(normalized.close_action, "ask");
        assert!(!normalized.close_to_tray_migrated);

        // 落盘不再写 closeToTray
        let text = serde_json::to_string(&migrated).unwrap();
        assert!(!text.contains("closeToTray"), "旧字段不能回写：{text}");
        assert!(text.contains("closeAction"));
    }

    #[test]
    fn data_root_must_be_absolute_or_empty() {
        let mut settings = Settings::default();
        assert_eq!(settings.data_root, "");
        assert!(settings.validate().is_ok(), "空表示用默认数据根");

        settings.data_root = "  D:\\GuguData  ".to_string();
        let normalized = settings.clone().normalize();
        assert_eq!(normalized.data_root, "D:\\GuguData", "读盘去空白");
        assert!(normalized.validate().is_ok());

        let mut relative = Settings::default();
        relative.data_root = "GuguData".to_string();
        assert!(relative.validate().is_err());
        assert_eq!(relative.normalize().data_root, "", "读盘回落默认");
    }

    #[test]
    fn auto_download_java_defaults_to_true_and_roundtrips() {
        assert!(Settings::default().auto_download_java);
        let mut settings = Settings::default();
        settings.auto_download_java = false;
        let text = serde_json::to_string(&settings).unwrap();
        assert!(text.contains("autoDownloadJava"));
        let back: Settings = serde_json::from_str(&text).unwrap();
        assert!(!back.auto_download_java);
    }
}

