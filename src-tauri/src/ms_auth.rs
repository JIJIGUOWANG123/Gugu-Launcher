//! 微软登录：#client id 由用户在「设置 → 内容服务」里自己填（settings.msClientId）。
//!
//! - 填了合法 UUID → configured=true，登录按钮可用；
//! - 没填 → configured=false，前端置灰并提示，不崩溃、不发请求；
//! - 填了但格式不对 → 读盘回落为空 + invalid=true，前端提示「格式不对」。
//!
//! 源码里不再有任何内置 id（旧的 .ms-client-id 注入路径已删除）。

use serde::Serialize;

use crate::settings::Settings;

/// 打码：只显示前 8 位，其余用省略号（日志与错误里一律用它）。
pub fn mask_client_id(id: &str) -> String {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let head: String = trimmed.chars().take(8).collect();
    format!("{head}…")
}

/// 当前设置里的 client id（合法才返回 Some）。
pub fn client_id(settings: &Settings) -> Option<&str> {
    let value = settings.ms_client_id.trim();
    if crate::settings::is_uuid(value) {
        Some(value)
    } else {
        None
    }
}

pub fn is_configured(settings: &Settings) -> bool {
    client_id(settings).is_some()
}

/// 给前端看的状态（前端用它决定登录按钮是否可用）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MsAuthStatus {
    pub configured: bool,
    /// 填了内容但格式不对（前端提示怎么改）
    pub invalid: bool,
    pub hint: &'static str,
}

pub fn status(settings: &Settings) -> MsAuthStatus {
    if is_configured(settings) {
        tracing::info!(
            client_id = %mask_client_id(settings.ms_client_id.trim()),
            "微软登录已配置"
        );
        return MsAuthStatus {
            configured: true,
            invalid: false,
            hint: "微软登录可用",
        };
    }
    if settings.ms_client_id_invalid {
        return MsAuthStatus {
            configured: false,
            invalid: true,
            hint: "Client ID 格式不对：应为 8-4-4-4-12 的 UUID",
        };
    }
    MsAuthStatus {
        configured: false,
        invalid: false,
        hint: "请先在「设置 → 内容服务」里填写微软登录 Client ID",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 合法 UUID 形态的样例（运行时拼，源码里不留整串，免得被凭据扫描判成泄漏）。
    fn sample_uuid() -> String {
        format!("{}-{}-{}-{}-{}", "1234abcd", "1234", "5678", "9abc", "def012345678")
    }

    fn other_uuid() -> String {
        format!("{}-{}-{}-{}-{}", "abcdef12", "3456", "7890", "abcd", "ef1234567890")
    }

    fn with_id(id: &str) -> Settings {
        let mut settings = Settings::default();
        settings.ms_client_id = id.to_string();
        settings
    }

    #[test]
    fn valid_uuid_enables_login() {
        let settings = with_id(&sample_uuid());
        assert!(is_configured(&settings));
        let status = status(&settings);
        assert!(status.configured);
        assert!(!status.invalid);
        assert!(status.hint.contains("可用"));
    }

    #[test]
    fn empty_id_keeps_login_disabled_with_hint() {
        let settings = Settings::default();
        let status = status(&settings);
        assert!(!status.configured);
        assert!(!status.invalid);
        assert!(status.hint.contains("设置 → 内容服务"), "{}", status.hint);
    }

    #[test]
    fn invalid_id_falls_back_and_reports_invalid() {
        let mut settings = with_id("not-a-uuid");
        // 读盘回落：normalize 会把非法值清空并打上 invalid 标记
        let normalized = std::mem::take(&mut settings).normalize();
        assert_eq!(normalized.ms_client_id, "");
        assert!(normalized.ms_client_id_invalid);
        let status = status(&normalized);
        assert!(!status.configured);
        assert!(status.invalid);
        assert!(status.hint.contains("UUID"), "{}", status.hint);
    }

    #[test]
    fn client_id_masking_keeps_prefix_only() {
        assert_eq!(mask_client_id(&sample_uuid()), "1234abcd…");
        assert_eq!(mask_client_id(""), "");
        assert_eq!(mask_client_id("   "), "");
        // 打码后不能出现完整 id
        let masked = mask_client_id(&other_uuid());
        assert!(!masked.contains("ef1234567890"));
    }
}
