//! Release 版本识别规则（按用户上传习惯定制）。
//!
//! 用户的 GitHub Release：**标题是纯数字**，或直接用 tag 判断版本。
//! 取版本号优先级：tag_name → name（标题）→ published_at 时间兜底。
//!
//! 允许的写法（去空白、可选 v/V 前缀、下划线当点）：
//! 1.0.1 / v1.0.1 / V1.0.1 / 1.0 / 1 / 1.0.1.2 / 纯数字 105。
//! 带后缀的（1.0.1-beta、1.0.1 (正式版)）取第一段能解析的数字，其余忽略并记 warn。
//!
//! 比较规则：绝不用字符串比较（1.10.0 必须 > 1.9.0）。两边都是元组→逐段补 0 比较；
//! 两边都是纯整数→整数比较；一边元组一边纯整数→把纯整数当 (n,) 补 0 比较并 warn 一次。

use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

/// 解析出来的版本号：带点的元组，或纯整数。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VersionNumber {
    Tuple(Vec<u64>),
    Single(u64),
}

impl VersionNumber {
    fn as_tuple(&self) -> Vec<u64> {
        match self {
            VersionNumber::Tuple(parts) => parts.clone(),
            VersionNumber::Single(value) => vec![*value],
        }
    }

    fn is_single(&self) -> bool {
        matches!(self, VersionNumber::Single(_))
    }

    pub fn to_text(&self) -> String {
        match self {
            VersionNumber::Tuple(parts) => parts
                .iter()
                .map(|part| part.to_string())
                .collect::<Vec<_>>()
                .join("."),
            VersionNumber::Single(value) => value.to_string(),
        }
    }
}

/// 解析版本号；解析不出来返回 None（调用方回落到 published_at）。
pub fn parse_version(text: &str) -> Option<VersionNumber> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let without_prefix = trimmed
        .strip_prefix('v')
        .or_else(|| trimmed.strip_prefix('V'))
        .unwrap_or(trimmed)
        .trim();
    let normalized = without_prefix.replace('_', ".");

    // 找到第一个数字，从这里开始取「数字 + 点」的最长前缀，其余（-beta、(正式版) 等）忽略
    let start = normalized.find(|c: char| c.is_ascii_digit())?;
    let tail = &normalized[start..];
    let mut end = 0usize;
    for (index, ch) in tail.char_indices() {
        if ch.is_ascii_digit() || ch == '.' {
            end = index + ch.len_utf8();
        } else {
            break;
        }
    }
    let numeric = tail[..end].trim_end_matches('.');
    if numeric.is_empty() {
        return None;
    }
    if !numeric.contains('.') {
        return numeric.parse::<u64>().ok().map(VersionNumber::Single);
    }

    let mut parts = Vec::new();
    for segment in numeric.split('.') {
        // "1..2" 这类脏输入按解析失败处理
        if segment.is_empty() {
            return None;
        }
        parts.push(segment.parse::<u64>().ok()?);
    }
    if parts.is_empty() {
        None
    } else {
        Some(VersionNumber::Tuple(parts))
    }
}

/// 逐段比较，缺段补 0；格式不一致时把纯整数当 (n,) 并警告一次。
pub fn compare(left: &VersionNumber, right: &VersionNumber) -> Ordering {
    if left.is_single() != right.is_single() {
        tracing::warn!(
            left = %left.to_text(),
            right = %right.to_text(),
            "版本号格式不一致（元组 vs 纯整数），按补 0 规则比较"
        );
    }
    let mut a = left.as_tuple();
    let mut b = right.as_tuple();
    let len = a.len().max(b.len());
    a.resize(len, 0);
    b.resize(len, 0);
    a.cmp(&b)
}

/// 版本号来源，测试里要断言「tag 优先于标题」。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum VersionSource {
    Tag,
    Name,
    PublishedAt,
}

/// Release 里与判断更新相关的字段。
#[derive(Debug, Clone, Default)]
pub struct ReleaseInfo {
    pub tag_name: Option<String>,
    pub name: Option<String>,
    /// Unix 秒
    pub published_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateDecision {
    pub available: bool,
    pub version: Option<String>,
    pub source: Option<VersionSource>,
    pub reason: String,
}

/// 判断是否有更新。build_time 是当前安装包的构建时间（Unix 秒），只用于时间兜底。
pub fn decide(current: &str, release: &ReleaseInfo, build_time: Option<i64>) -> UpdateDecision {
    let current_version = parse_version(current);

    let remote = release
        .tag_name
        .as_deref()
        .and_then(parse_version)
        .map(|version| (version, VersionSource::Tag))
        .or_else(|| {
            release
                .name
                .as_deref()
                .and_then(parse_version)
                .map(|version| (version, VersionSource::Name))
        });

    if let Some((remote_version, source)) = remote {
        let Some(current_version) = current_version else {
            return UpdateDecision {
                available: false,
                version: Some(remote_version.to_text()),
                source: Some(source),
                reason: "当前版本号无法解析，不做更新判断".to_string(),
            };
        };
        return match compare(&remote_version, &current_version) {
            Ordering::Greater => UpdateDecision {
                available: true,
                version: Some(remote_version.to_text()),
                source: Some(source),
                reason: format!("远端 {} 大于当前 {}", remote_version.to_text(), current_version.to_text()),
            },
            Ordering::Equal => UpdateDecision {
                available: false,
                version: Some(remote_version.to_text()),
                source: Some(source),
                reason: "版本号相同".to_string(),
            },
            Ordering::Less => UpdateDecision {
                available: false,
                version: Some(remote_version.to_text()),
                source: Some(source),
                reason: format!("远端 {} 比当前 {} 更旧", remote_version.to_text(), current_version.to_text()),
            },
        };
    }

    // 都解析不出来 → 用发布时间兜底，避免漏更
    match (release.published_at, build_time) {
        (Some(published), Some(built)) if published > built => UpdateDecision {
            available: true,
            version: None,
            source: Some(VersionSource::PublishedAt),
            reason: "版本号无法解析，按发布时间判定有更新".to_string(),
        },
        (Some(_), Some(_)) => UpdateDecision {
            available: false,
            version: None,
            source: Some(VersionSource::PublishedAt),
            reason: "版本号无法解析，发布时间不晚于当前构建".to_string(),
        },
        _ => UpdateDecision {
            available: false,
            version: None,
            source: None,
            reason: "无法解析版本号，也缺少发布时间".to_string(),
        },
    }
}

// ------------------------------------------------------------------ 资产

/// Release 资产。
#[derive(Debug, Clone, Deserialize)]
pub struct ReleaseAsset {
    pub name: String,
    pub browser_download_url: String,
    #[serde(default)]
    pub size: Option<u64>,
}

/// 资产类型：只发布安装包，所以只有一种。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AssetKind {
    /// GuguLauncher-Setup.exe：NSIS 安装包，静默 /S 装回原目录
    Installer,
}

/// 安装方式：只有「安装版静默安装」与「不更新」两种。
///
/// 明确不做 portable 自替换（0.5.6 里 swapScriptBody / PORTABLE_EXECUTABLE_FILE 那套不移植）：
/// 单文件自替换在 Windows 上要临时脚本、要处理杀软误报，收益不值。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum InstallMode {
    /// 下载 GuguLauncher-Setup.exe，静默 /S 装回原目录，装完自动重启
    Setup,
    /// 没有更新可装（或用户关掉了自动更新）
    None,
}

/// 从资产列表里选出安装包：只认 GuguLauncher-Setup.exe（大小写不敏感、允许前后空格）。
/// 不看版本号，也不 contains(version)；名字里带版本号的（容错）同样识别。
/// 找不到就返回 None，调用方报「该版本没有可用资产」并跳过 —— 绝不装 zip 或裸 exe。
pub fn pick_asset(assets: &[ReleaseAsset]) -> Option<(AssetKind, &ReleaseAsset)> {
    let normalized = |asset: &ReleaseAsset| asset.name.trim().to_ascii_lowercase();

    assets
        .iter()
        .find(|asset| {
            let name = normalized(asset);
            name.contains("gugulauncher") && name.ends_with(".exe") && name.contains("setup")
        })
        .map(|asset| (AssetKind::Installer, asset))
}

/// 下载后用 Release 给的 size 校验（有就校验，没有就跳过）。文件名里的版本号不参与任何判断。
pub fn verify_size(expected: Option<u64>, actual: u64) -> Result<(), String> {
    match expected {
        Some(size) if size != actual => Err(format!("大小不匹配：期望 {size} 字节，实际 {actual} 字节")),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tuple(parts: &[u64]) -> VersionNumber {
        VersionNumber::Tuple(parts.to_vec())
    }

    #[test]
    fn parses_all_supported_spellings() {
        assert_eq!(parse_version("1.0.1"), Some(tuple(&[1, 0, 1])));
        assert_eq!(parse_version("v1.0.1"), Some(tuple(&[1, 0, 1])));
        assert_eq!(parse_version("V1.0.1"), Some(tuple(&[1, 0, 1])));
        assert_eq!(parse_version("  1.0  "), Some(tuple(&[1, 0])));
        assert_eq!(parse_version("1"), Some(VersionNumber::Single(1)));
        assert_eq!(parse_version("1.0.1.2"), Some(tuple(&[1, 0, 1, 2])));
        assert_eq!(parse_version("1_0_1"), Some(tuple(&[1, 0, 1])));
        // 纯数字按单个整数，不猜成 1.0.5
        assert_eq!(parse_version("105"), Some(VersionNumber::Single(105)));
        // 后缀忽略
        assert_eq!(parse_version("1.0.1-beta"), Some(tuple(&[1, 0, 1])));
        assert_eq!(parse_version("1.0.1 (正式版)"), Some(tuple(&[1, 0, 1])));
        assert_eq!(parse_version("正式版 v1.2.3"), Some(tuple(&[1, 2, 3])));
        assert_eq!(parse_version(""), None);
        assert_eq!(parse_version("正式版"), None);
    }

    #[test]
    fn numeric_comparison_not_string_comparison() {
        assert_eq!(compare(&tuple(&[1, 10, 0]), &tuple(&[1, 9, 0])), Ordering::Greater);
        assert_eq!(
            compare(&parse_version("v1.0.1").unwrap(), &parse_version("1.0.1").unwrap()),
            Ordering::Equal
        );
        assert_eq!(
            compare(&parse_version("1.0").unwrap(), &parse_version("1.0.0").unwrap()),
            Ordering::Equal
        );
        assert_eq!(
            compare(&parse_version("105").unwrap(), &parse_version("104").unwrap()),
            Ordering::Greater
        );
        // 格式不一致也不 panic
        assert_eq!(
            compare(&parse_version("105").unwrap(), &parse_version("1.0.5").unwrap()),
            Ordering::Greater
        );
        assert_eq!(compare(&tuple(&[1, 0]), &VersionNumber::Single(1)), Ordering::Equal);
    }

    #[test]
    fn tag_wins_over_title() {
        let release = ReleaseInfo {
            tag_name: Some("v1.0.1".to_string()),
            name: Some("9.9.9".to_string()),
            published_at: None,
        };
        let decision = decide("1.0.0", &release, Some(0));
        assert!(decision.available);
        assert_eq!(decision.version.as_deref(), Some("1.0.1"));
        assert_eq!(decision.source, Some(VersionSource::Tag));
    }

    #[test]
    fn equal_or_older_means_no_update() {
        let same = ReleaseInfo {
            tag_name: Some("1.0.1".to_string()),
            name: None,
            published_at: None,
        };
        assert!(!decide("v1.0.1", &same, None).available);

        let older = ReleaseInfo {
            tag_name: None,
            name: Some("1.0.0".to_string()),
            published_at: None,
        };
        assert!(!decide("1.0.1", &older, None).available);
        assert_eq!(decide("1.0.1", &older, None).source, Some(VersionSource::Name));
    }

    #[test]
    fn published_at_is_the_fallback_when_versions_are_unparseable() {
        let release = ReleaseInfo {
            tag_name: Some("正式版".to_string()),
            name: Some("说明".to_string()),
            published_at: Some(2_000),
        };
        assert!(decide("1.0.0", &release, Some(1_000)).available);
        assert!(!decide("1.0.0", &release, Some(3_000)).available);
        assert_eq!(
            decide("1.0.0", &release, Some(1_000)).source,
            Some(VersionSource::PublishedAt)
        );
        // 缺发布时间 → 不猜
        assert!(!decide("1.0.0", &release, None).available);
    }

    fn asset(name: &str, size: u64) -> ReleaseAsset {
        ReleaseAsset {
            name: name.to_string(),
            browser_download_url: format!("https://example.com/{name}"),
            size: Some(size),
        }
    }

    #[test]
    fn installer_is_the_only_accepted_asset() {
        let assets = vec![
            asset("GuguLauncher.exe", 100),
            asset("GuguLauncher-Setup.exe", 200),
            asset("GuguLauncher-portable-x64.zip", 300),
        ];
        let (kind, picked) = pick_asset(&assets).unwrap();
        assert_eq!(kind, AssetKind::Installer);
        assert_eq!(picked.name, "GuguLauncher-Setup.exe");

        // 只发安装包：裸 exe 与 portable zip 一律忽略
        assert!(pick_asset(&assets[..1]).is_none(), "单文件 exe 不再支持");
        assert!(pick_asset(&assets[2..]).is_none(), "portable zip 不再支持");
    }

    #[test]
    fn version_in_asset_name_is_still_recognized_but_not_required() {
        let assets = vec![asset("GuguLauncher-1.0.1-Setup.exe", 200)];
        assert_eq!(pick_asset(&assets).unwrap().0, AssetKind::Installer);

        // 前后空格、大小写不影响
        let assets = vec![asset("  gugulauncher-setup.EXE ", 200)];
        assert_eq!(pick_asset(&assets).unwrap().0, AssetKind::Installer);
    }

    #[test]
    fn unrelated_names_report_no_usable_asset() {
        let noise = vec![asset("source.zip", 1), asset("notes.txt", 2)];
        assert!(pick_asset(&noise).is_none(), "找不到就报没有可用资产，不能瞎装");
        assert!(pick_asset(&[]).is_none());
    }

    #[test]
    fn size_verification_uses_release_metadata_only() {
        assert!(verify_size(Some(100), 100).is_ok());
        assert!(verify_size(None, 12345).is_ok());
        assert!(verify_size(Some(100), 99).is_err());
    }
}
