//! 每日一言：主进程代发，避免前端跨域。
//!
//! 目标地址取设置里的 sayingApiUrl，留空用内置默认地址；
//! 8 秒超时，解析顺序与 0.5.6 一致：text / saying / hitokoto / content /
//! data.text / data / msg。网络失败不报错，交给前端兜底文案。

use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

use crate::error::{AppError, Result};
use crate::settings::Settings;

/// 内置默认接口。
pub const DEFAULT_SAYING_URL: &str = "https://api.uapis.cn/api/v1/saying";
/// 超时（任务书要求 8 秒）。
pub const SAYING_TIMEOUT: Duration = Duration::from_secs(8);

/// fun_saying() 的返回。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SayingResult {
    pub ok: bool,
    pub text: String,
    pub source: String,
}

/// 解析目标地址：留空用内置默认，填了必须是 http(s)。
pub fn resolve_url(settings_url: &str) -> Result<String> {
    let trimmed = settings_url.trim();
    if trimmed.is_empty() {
        return Ok(DEFAULT_SAYING_URL.to_string());
    }
    if !crate::settings::is_http_url(trimmed) {
        return Err(AppError::Invalid(format!(
            "每日一言接口必须是 http(s) 地址：{trimmed}"
        )));
    }
    Ok(trimmed.to_string())
}

/// 从响应 JSON 里按既定优先级取一句。
pub fn parse_saying(json: &str) -> Option<String> {
    let value: Value = serde_json::from_str(json).ok()?;
    let pick = |text: Option<&str>| text.map(|text| text.trim().to_string()).filter(|text| !text.is_empty());

    for key in ["text", "saying", "hitokoto", "content"] {
        if let Some(found) = pick(value.get(key).and_then(|item| item.as_str())) {
            return Some(found);
        }
    }
    // data 可能是对象（含 text），也可能直接是字符串
    if let Some(data) = value.get("data") {
        if let Some(found) = pick(data.get("text").and_then(|item| item.as_str())) {
            return Some(found);
        }
        if let Some(found) = pick(data.as_str()) {
            return Some(found);
        }
    }
    if let Some(found) = pick(value.get("msg").and_then(|item| item.as_str())) {
        return Some(found);
    }
    None
}

/// 联网取一句；任何失败都返回 ok=false（前端用兜底文案）。
pub async fn fetch(client: &reqwest::Client, settings: &Settings) -> SayingResult {
    let url = match resolve_url(&settings.saying_api_url) {
        Ok(url) => url,
        Err(err) => {
            tracing::warn!(%err, "每日一言接口地址非法");
            return SayingResult {
                ok: false,
                text: err.to_string(),
                source: "设置".to_string(),
            };
        }
    };
    let source = if settings.saying_api_url.trim().is_empty() {
        "内置".to_string()
    } else {
        "设置".to_string()
    };
    let started = std::time::Instant::now();
    let response = client.get(&url).timeout(SAYING_TIMEOUT).send().await;
    tracing::info!(
        url = %url,
        elapsed_ms = started.elapsed().as_millis() as u64,
        "每日一言请求完成"
    );
    match response {
        Ok(response) if response.status().is_success() => match response.text().await {
            Ok(text) => match parse_saying(&text) {
                Some(saying) => SayingResult {
                    ok: true,
                    text: saying,
                    source,
                },
                None => SayingResult {
                    ok: false,
                    text: "接口返回里没有可取的一句话".to_string(),
                    source,
                },
            },
            Err(err) => SayingResult {
                ok: false,
                text: format!("读取响应失败：{err}"),
                source,
            },
        },
        Ok(response) => SayingResult {
            ok: false,
            text: format!("接口返回 HTTP {}", response.status().as_u16()),
            source,
        },
        Err(err) => SayingResult {
            ok: false,
            text: if err.is_timeout() {
                format!("请求超时（{} 秒）", SAYING_TIMEOUT.as_secs())
            } else {
                format!("网络请求失败：{err}")
            },
            source,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_url_defaults_and_validates() {
        assert_eq!(resolve_url("").unwrap(), DEFAULT_SAYING_URL);
        assert_eq!(
            resolve_url("  https://example.com/saying  ").unwrap(),
            "https://example.com/saying"
        );
        assert!(resolve_url("ftp://example.com").is_err());
        assert!(resolve_url("example.com").is_err());
    }

    #[test]
    fn parse_prefers_text_then_other_fields() {
        assert_eq!(
            parse_saying(r#"{"text":"句子A","saying":"句子B","hitokoto":"句子C","content":"句子D","msg":"句子E"}"#).unwrap(),
            "句子A"
        );
        assert_eq!(
            parse_saying(r#"{"saying":"句子B","hitokoto":"句子C","content":"句子D","msg":"句子E"}"#).unwrap(),
            "句子B"
        );
        assert_eq!(
            parse_saying(r#"{"hitokoto":"句子C","content":"句子D"}"#).unwrap(),
            "句子C"
        );
        assert_eq!(parse_saying(r#"{"content":"句子D"}"#).unwrap(), "句子D");
        assert_eq!(
            parse_saying(r#"{"data":{"text":"句子F"}}"#).unwrap(),
            "句子F"
        );
        assert_eq!(parse_saying(r#"{"data":"句子G"}"#).unwrap(), "句子G");
        assert_eq!(parse_saying(r#"{"msg":"句子E"}"#).unwrap(), "句子E");
    }

    #[test]
    fn parse_ignores_blank_and_broken_payloads() {
        assert_eq!(parse_saying(r#"{"text":"   "}"#), None);
        assert_eq!(parse_saying("不是 JSON"), None);
        assert_eq!(parse_saying(r#"{"code":200}"#), None);
        assert_eq!(parse_saying(""), None);
    }

    #[tokio::test]
    async fn fetch_reports_bad_url_without_network() {
        let mut settings = Settings::default();
        settings.saying_api_url = "not-a-url".to_string();
        let client = reqwest::Client::new();
        let result = fetch(&client, &settings).await;
        assert!(!result.ok);
        assert!(result.text.contains("http"));
        assert_eq!(result.source, "设置");
    }
}
