//! 应用内更新日志：以内嵌的仓库 CHANGELOG.md 为准（与官网/仓库三处一致）。

use serde::Serialize;

/// 编译期内嵌的变更日志（仓库根 CHANGELOG.md）。
pub const BUNDLED_CHANGELOG: &str = include_str!("../../CHANGELOG.md");

/// app_changelog() 的返回。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Changelog {
    pub version: String,
    pub markdown: String,
    /// bundled = 来自内嵌 CHANGELOG.md；fallback = 取不到时的空内容
    pub source: String,
}

/// 解析「最新一个 ## x.y.z 段落」（多个版本时只取第一个）。
pub fn parse_latest(text: &str, current_version: &str) -> Changelog {
    let mut version = String::new();
    let mut body: Vec<&str> = Vec::new();
    let mut started = false;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("## ") {
            if started {
                break;
            }
            // 标题形如 "1.0.1 — 2026-10-27" / "v1.0.1"：取第一个 token
            let candidate = rest
                .trim()
                .split(|ch: char| ch.is_whitespace() || ch == '—' || ch == '-')
                .next()
                .unwrap_or_default()
                .trim()
                .trim_start_matches('v')
                .to_string();
            // 只认形如 1.0.1 的版本标题
            if !candidate.is_empty()
                && candidate
                    .chars()
                    .all(|ch| ch.is_ascii_digit() || ch == '.')
            {
                version = candidate;
                started = true;
                continue;
            }
            // 不是版本标题（例如 "## 未发布"）：跳过，继续找
            continue;
        }
        if started {
            body.push(line);
        }
    }
    if version.is_empty() {
        return Changelog {
            version: current_version.to_string(),
            markdown: String::new(),
            source: "fallback".to_string(),
        };
    }
    Changelog {
        version,
        markdown: body.join("\n").trim().to_string(),
        source: "bundled".to_string(),
    }
}

/// app_changelog()：内嵌 CHANGELOG.md 的最新段落；解析失败回落空内容。
pub fn latest() -> Changelog {
    let current = env!("CARGO_PKG_VERSION");
    let parsed = parse_latest(BUNDLED_CHANGELOG, current);
    if parsed.source == "bundled" {
        if parsed.version != current {
            // 发版时 CHANGELOG 可能落后一个补丁版本：只提示，不阻塞
            tracing::warn!(
                changelog = %parsed.version,
                current,
                "内嵌 CHANGELOG 的最新版本与当前版本不一致"
            );
        }
        tracing::debug!(
            version = %parsed.version,
            bytes = parsed.markdown.len(),
            "内嵌 CHANGELOG 最新段落"
        );
    } else {
        tracing::warn!("内嵌 CHANGELOG.md 里没有版本段落，返回空内容");
    }
    parsed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takes_the_first_version_section_only() {
        let text = "# 更新日志\n\n## 1.0.2\n\n- 新的东西\n- 另一个改动\n\n## 1.0.1\n\n- 旧的东西\n";
        let parsed = parse_latest(text, "1.0.2");
        assert_eq!(parsed.version, "1.0.2");
        assert_eq!(parsed.source, "bundled");
        assert!(parsed.markdown.contains("新的东西"));
        assert!(!parsed.markdown.contains("旧的东西"), "只取最新一段");
    }

    #[test]
    fn skips_non_version_headings() {
        let text = "## 未发布\n\n- 进行中\n\n## v1.0.1\n\n- 修好了\n";
        let parsed = parse_latest(text, "1.0.1");
        assert_eq!(parsed.version, "1.0.1", "v 前缀要去掉");
        assert!(parsed.markdown.contains("修好了"));
    }

    #[test]
    fn empty_file_falls_back() {
        let parsed = parse_latest("", "1.0.2");
        assert_eq!(parsed.version, "1.0.2");
        assert!(parsed.markdown.is_empty());
        assert_eq!(parsed.source, "fallback");
        assert_eq!(parsed, parse_latest("# 只有标题\n", "1.0.2"));
    }

    #[test]
    fn bundled_changelog_matches_the_compiled_version() {
        let parsed = latest();
        assert_eq!(parsed.source, "bundled", "内嵌 CHANGELOG 必须能解析出版本段落");
        assert!(!parsed.markdown.is_empty());
        // 只校验同一发布线（major.minor）：发版时 CHANGELOG 可能落后一个补丁号
        let major_minor = |version: &str| {
            version
                .split('.')
                .take(2)
                .collect::<Vec<_>>()
                .join(".")
        };
        assert_eq!(
            major_minor(&parsed.version),
            major_minor(env!("CARGO_PKG_VERSION")),
            "CHANGELOG 最新段落要与 CARGO_PKG_VERSION 同一发布线（实际 {} vs {}）",
            parsed.version,
            env!("CARGO_PKG_VERSION")
        );
    }
}
