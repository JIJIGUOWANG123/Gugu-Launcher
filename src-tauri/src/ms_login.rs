//! 微软登录：OAuth 2.0 设备码流程 + XBL / XSTS / Minecraft 令牌链。
//!
//! client id 只能来自 ms_auth::client_id()（打包时注入）；没有就返回 configured=false，
//! 绝不写死任何 id。令牌存 .mc/accounts.json，日志与错误信息里一律打码。

use serde::Serialize;
use serde_json::{json, Value};

use crate::accounts::{self, AccountItem};
use crate::error::{AppError, Result};
use crate::ms_auth;
use crate::paths::Paths;
use crate::util;

pub const DEVICE_CODE_URL: &str =
    "https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode";
pub const TOKEN_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/token";
pub const XBL_URL: &str = "https://user.auth.xboxlive.com/user/authenticate";
pub const XSTS_URL: &str = "https://xsts.auth.xboxlive.com/xsts/authorize";
pub const MC_LOGIN_URL: &str =
    "https://api.minecraftservices.com/authentication/login_with_xbox";
pub const MC_PROFILE_URL: &str = "https://api.minecraftservices.com/minecraft/profile";
pub const SCOPE: &str = "XboxLive.signin offline_access";

/// ms_login_start 的返回。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceCodeStart {
    pub configured: bool,
    pub user_code: Option<String>,
    pub verify_url: Option<String>,
    pub device_code: Option<String>,
    pub expires_in: Option<u64>,
    pub message: String,
}

/// ms_login_poll 的返回。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginPoll {
    /// pending / done / error
    pub state: String,
    pub account: Option<AccountItem>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceCodeParsed {
    pub user_code: String,
    pub device_code: String,
    pub verify_url: String,
    pub expires_in: u64,
    pub interval: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenOutcome {
    /// 用户还没完成授权
    Pending(String),
    Ready {
        access_token: String,
        refresh_token: String,
    },
}

pub fn parse_device_code(json: &str) -> Result<DeviceCodeParsed> {
    let value: Value = serde_json::from_str(json)?;
    if let Some(error) = value.get("error").and_then(|item| item.as_str()) {
        let description = value
            .get("error_description")
            .and_then(|item| item.as_str())
            .unwrap_or(error);
        return Err(AppError::Other(format!("微软返回错误：{description}")));
    }
    let pick = |key: &str| {
        value
            .get(key)
            .and_then(|item| item.as_str())
            .map(|item| item.to_string())
            .filter(|item| !item.is_empty())
    };
    Ok(DeviceCodeParsed {
        user_code: pick("user_code")
            .ok_or_else(|| AppError::Other("微软返回缺少 user_code".to_string()))?,
        device_code: pick("device_code")
            .ok_or_else(|| AppError::Other("微软返回缺少 device_code".to_string()))?,
        verify_url: pick("verification_uri")
            .or_else(|| pick("verification_url"))
            .ok_or_else(|| AppError::Other("微软返回缺少 verification_uri".to_string()))?,
        expires_in: value
            .get("expires_in")
            .and_then(|item| item.as_u64())
            .unwrap_or(900),
        interval: value
            .get("interval")
            .and_then(|item| item.as_u64())
            .unwrap_or(5),
    })
}

pub fn parse_token_response(json: &str) -> TokenOutcome {
    let Ok(value) = serde_json::from_str::<Value>(json) else {
        return TokenOutcome::Pending("令牌返回不是合法 JSON".to_string());
    };
    if let Some(access_token) = value
        .get("access_token")
        .and_then(|item| item.as_str())
        .filter(|item| !item.is_empty())
    {
        return TokenOutcome::Ready {
            access_token: access_token.to_string(),
            refresh_token: value
                .get("refresh_token")
                .and_then(|item| item.as_str())
                .unwrap_or("")
                .to_string(),
        };
    }
    let code = value
        .get("error")
        .and_then(|item| item.as_str())
        .unwrap_or("unknown_error");
    let description = value
        .get("error_description")
        .and_then(|item| item.as_str())
        .unwrap_or("");
    TokenOutcome::Pending(format!("{code}：{description}").trim_end_matches("：").to_string())
}

/// XBL / XSTS 响应：Token + DisplayClaims.xui[0].uhs
pub fn parse_xbox_token(json: &str) -> Result<(String, String)> {
    let value: Value = serde_json::from_str(json)?;
    if let Some(error) = value.get("error").and_then(|item| item.as_str()) {
        let message = value
            .get("error_description")
            .and_then(|item| item.as_str())
            .unwrap_or(error);
        return Err(AppError::Other(format!("Xbox 认证失败：{message}")));
    }
    let token = value
        .get("Token")
        .and_then(|item| item.as_str())
        .ok_or_else(|| AppError::Other("Xbox 返回缺少 Token".to_string()))?
        .to_string();
    let uhs = value
        .pointer("/DisplayClaims/xui/0/uhs")
        .and_then(|item| item.as_str())
        .ok_or_else(|| AppError::Other("Xbox 返回缺少 uhs".to_string()))?
        .to_string();
    Ok((token, uhs))
}

/// XSTS 的常见错误码 → 中文说明。
pub fn explain_xsts_error(xerr: u64) -> String {
    match xerr {
        2148916233 => "该微软账号还没有 Xbox 账号，请先在 xbox.com 注册".to_string(),
        2148916235 => "该账号所在地区不支持 Xbox Live".to_string(),
        2148916236 | 2148916237 => "该账号需要先完成年龄验证".to_string(),
        2148916238 => "未成年账号需要家长同意".to_string(),
        other => format!("XSTS 认证失败（XErr {other}）"),
    }
}

pub fn parse_xsts_error(json: &str) -> Option<String> {
    let value: Value = serde_json::from_str(json).ok()?;
    let xerr = value.get("XErr").and_then(|item| item.as_u64())?;
    Some(explain_xsts_error(xerr))
}

pub fn parse_mc_token(json: &str) -> Result<String> {
    let value: Value = serde_json::from_str(json)?;
    value
        .get("access_token")
        .and_then(|item| item.as_str())
        .map(|item| item.to_string())
        .filter(|item| !item.is_empty())
        .ok_or_else(|| AppError::Other("Minecraft 登录没有返回 access_token".to_string()))
}

/// 档案：{ id, name, skins:[{url}] }
pub fn parse_profile(json: &str) -> Result<(String, String, Option<String>)> {
    let value: Value = serde_json::from_str(json)?;
    if let Some(error) = value.get("error").and_then(|item| item.as_str()) {
        let message = value
            .get("errorMessage")
            .and_then(|item| item.as_str())
            .unwrap_or(error);
        return Err(AppError::Other(format!(
            "读取 Minecraft 档案失败：{message}（可能还没有购买游戏）"
        )));
    }
    let name = value
        .get("name")
        .and_then(|item| item.as_str())
        .ok_or_else(|| AppError::Other("档案缺少玩家名".to_string()))?
        .to_string();
    let id = value
        .get("id")
        .and_then(|item| item.as_str())
        .ok_or_else(|| AppError::Other("档案缺少 uuid".to_string()))?
        .to_string();
    let skin = value
        .get("skins")
        .and_then(|item| item.as_array())
        .and_then(|skins| skins.first())
        .and_then(|skin| skin.get("url"))
        .and_then(|url| url.as_str())
        .map(|url| url.to_string());
    Ok((name, id, skin))
}

/// 第一步：申请设备码。
pub async fn start_device_code(
    client: &reqwest::Client,
    settings: &crate::settings::Settings,
) -> Result<DeviceCodeStart> {
    let Some(client_id) = ms_auth::client_id(settings) else {
        return Ok(DeviceCodeStart {
            configured: false,
            user_code: None,
            verify_url: None,
            device_code: None,
            expires_in: None,
            message: "请先在「设置 → 内容服务」里填写微软登录 Client ID".to_string(),
        });
    };
    let response = client
        .post(DEVICE_CODE_URL)
        .form(&[("client_id", client_id), ("scope", SCOPE)])
        .send()
        .await
        .map_err(|err| AppError::Net(format!("申请设备码失败：{err}")))?;
    let text = response
        .text()
        .await
        .map_err(|err| AppError::Net(format!("读取设备码响应失败：{err}")))?;
    let parsed = parse_device_code(&text)?;
    tracing::info!(
        user_code = %parsed.user_code,
        verify = %parsed.verify_url,
        "已申请微软登录设备码"
    );
    Ok(DeviceCodeStart {
        configured: true,
        user_code: Some(parsed.user_code),
        verify_url: Some(parsed.verify_url),
        device_code: Some(parsed.device_code),
        expires_in: Some(parsed.expires_in),
        message: "请在浏览器里打开验证地址并输入代码".to_string(),
    })
}

/// 第二步：轮询一次；完成时把账号写进 accounts.json。
pub async fn poll_once(
    client: &reqwest::Client,
    paths: &Paths,
    settings: &crate::settings::Settings,
    device_code: &str,
) -> Result<LoginPoll> {
    let Some(client_id) = ms_auth::client_id(settings) else {
        return Ok(LoginPoll {
            state: "error".to_string(),
            account: None,
            message: Some("请先在「设置 → 内容服务」里填写微软登录 Client ID".to_string()),
        });
    };
    let response = client
        .post(TOKEN_URL)
        .form(&[
            ("client_id", client_id),
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ("device_code", device_code),
            ("scope", SCOPE),
        ])
        .send()
        .await
        .map_err(|err| AppError::Net(format!("轮询令牌失败：{err}")))?;
    let text = response
        .text()
        .await
        .map_err(|err| AppError::Net(format!("读取令牌响应失败：{err}")))?;

    let (ms_token, refresh_token) = match parse_token_response(&text) {
        TokenOutcome::Pending(reason) => {
            let expired = reason.contains("expired_token") || reason.contains("code_expired");
            return Ok(LoginPoll {
                state: if expired { "error" } else { "pending" }.to_string(),
                account: None,
                message: Some(if expired {
                    "设备码已过期，请重新开始登录".to_string()
                } else {
                    reason
                }),
            });
        }
        TokenOutcome::Ready {
            access_token,
            refresh_token,
        } => (access_token, refresh_token),
    };
    tracing::info!(
        token = %util::mask_secret(&ms_token),
        "已拿到微软访问令牌"
    );

    let (xbl_token, uhs) = xbox_authenticate(client, &ms_token).await?;
    let (xsts_token, xsts_uhs) = xsts_authorize(client, &xbl_token, &uhs).await?;
    let mc_token = minecraft_login(client, &xsts_uhs, &xsts_token).await?;
    let (name, uuid, skin) = minecraft_profile(client, &mc_token).await?;
    let account = accounts::upsert_ms(paths, &name, &uuid, &mc_token, &refresh_token)?;
    tracing::info!(player = %name, "微软账号登录成功");
    let mut account = account;
    if let Some(skin) = skin {
        account.skin_url = Some(skin);
    }
    Ok(LoginPoll {
        state: "done".to_string(),
        account: Some(account),
        message: Some("登录成功".to_string()),
    })
}

async fn xbox_authenticate(client: &reqwest::Client, ms_token: &str) -> Result<(String, String)> {
    let body = json!({
        "Properties": {
            "AuthMethod": "RPS",
            "SiteName": "user.auth.xboxlive.com",
            "RpsTicket": format!("d={ms_token}"),
        },
        "RelyingParty": "http://auth.xboxlive.com",
        "TokenType": "JWT",
    });
    let response = client
        .post(XBL_URL)
        .json(&body)
        .send()
        .await
        .map_err(|err| AppError::Net(format!("Xbox 认证请求失败：{err}")))?;
    let text = response
        .text()
        .await
        .map_err(|err| AppError::Net(format!("读取 Xbox 认证响应失败：{err}")))?;
    parse_xbox_token(&text)
}

async fn xsts_authorize(
    client: &reqwest::Client,
    xbl_token: &str,
    uhs: &str,
) -> Result<(String, String)> {
    let body = json!({
        "Properties": {
            "SandboxId": "RETAIL",
            "UserTokens": [xbl_token],
        },
        "RelyingParty": "rp://api.minecraftservices.com/",
        "TokenType": "JWT",
    });
    let response = client
        .post(XSTS_URL)
        .json(&body)
        .send()
        .await
        .map_err(|err| AppError::Net(format!("XSTS 认证请求失败：{err}")))?;
    let text = response
        .text()
        .await
        .map_err(|err| AppError::Net(format!("读取 XSTS 响应失败：{err}")))?;
    if let Some(message) = parse_xsts_error(&text) {
        return Err(AppError::Other(message));
    }
    let (token, returned_uhs) = parse_xbox_token(&text)?;
    let uhs = if returned_uhs.is_empty() {
        uhs.to_string()
    } else {
        returned_uhs
    };
    Ok((token, uhs))
}

async fn minecraft_login(client: &reqwest::Client, uhs: &str, xsts_token: &str) -> Result<String> {
    let body = json!({
        "identityToken": format!("XBL3.0 x={uhs};{xsts_token}"),
    });
    let response = client
        .post(MC_LOGIN_URL)
        .json(&body)
        .send()
        .await
        .map_err(|err| AppError::Net(format!("Minecraft 登录请求失败：{err}")))?;
    let text = response
        .text()
        .await
        .map_err(|err| AppError::Net(format!("读取 Minecraft 登录响应失败：{err}")))?;
    parse_mc_token(&text)
}

async fn minecraft_profile(
    client: &reqwest::Client,
    mc_token: &str,
) -> Result<(String, String, Option<String>)> {
    let response = client
        .get(MC_PROFILE_URL)
        .bearer_auth(mc_token)
        .send()
        .await
        .map_err(|err| AppError::Net(format!("读取 Minecraft 档案失败：{err}")))?;
    let text = response
        .text()
        .await
        .map_err(|err| AppError::Net(format!("读取 Minecraft 档案内容失败：{err}")))?;
    parse_profile(&text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_code_parsing() {
        let json = r#"{"user_code":"ABCD-EFGH","device_code":"dev-123","verification_uri":"https://microsoft.com/devicelogin","expires_in":900,"interval":5}"#;
        let parsed = parse_device_code(json).unwrap();
        assert_eq!(parsed.user_code, "ABCD-EFGH");
        assert_eq!(parsed.device_code, "dev-123");
        assert_eq!(parsed.verify_url, "https://microsoft.com/devicelogin");
        assert_eq!(parsed.expires_in, 900);
        assert_eq!(parsed.interval, 5);

        let error = parse_device_code(r#"{"error":"invalid_client","error_description":"bad client id"}"#)
            .unwrap_err();
        assert!(error.to_string().contains("bad client id"));
        assert!(parse_device_code(r#"{"user_code":"x"}"#).is_err());
    }

    #[test]
    fn token_response_pending_and_ready() {
        let pending = parse_token_response(
            r#"{"error":"authorization_pending","error_description":"waiting"}"#,
        );
        assert!(matches!(pending, TokenOutcome::Pending(reason) if reason.contains("authorization_pending")));

        let ready = parse_token_response(
            r#"{"access_token":"eyJhbGciOi.payload.sig","refresh_token":"rt-1","expires_in":3600}"#,
        );
        match ready {
            TokenOutcome::Ready { access_token, refresh_token } => {
                assert_eq!(access_token, "eyJhbGciOi.payload.sig");
                assert_eq!(refresh_token, "rt-1");
            }
            other => panic!("应当就绪：{other:?}"),
        }
        assert!(matches!(parse_token_response("not json"), TokenOutcome::Pending(_)));
    }

    #[test]
    fn xbox_token_parsing_and_errors() {
        let json = r#"{"Token":"xbl-token","DisplayClaims":{"xui":[{"uhs":"user-hash"}]}}"#;
        let (token, uhs) = parse_xbox_token(json).unwrap();
        assert_eq!(token, "xbl-token");
        assert_eq!(uhs, "user-hash");
        assert!(parse_xbox_token(r#"{"Token":"only"}"#).is_err());

        // XSTS 常见错误码给中文说明
        assert!(explain_xsts_error(2148916233).contains("还没有 Xbox 账号"));
        assert!(explain_xsts_error(2148916238).contains("家长"));
        assert!(explain_xsts_error(999).contains("999"));
        assert_eq!(
            parse_xsts_error(r#"{"Identity":"0","XErr":2148916233,"Message":""}"#)
                .unwrap()
                .contains("Xbox"),
            true
        );
        assert!(parse_xsts_error(r#"{"Token":"ok"}"#).is_none());
    }

    #[test]
    fn mc_token_and_profile_parsing() {
        assert_eq!(
            parse_mc_token(r#"{"access_token":"mc-token","username":"abc"}"#).unwrap(),
            "mc-token"
        );
        assert!(parse_mc_token(r#"{"error":"x"}"#).is_err());

        let profile = r#"{"id":"069a79f444e94726a5befca90e38aaf5","name":"Steve","skins":[{"id":"x","state":"ACTIVE","url":"https://textures.minecraft.net/texture/abc"}]}"#;
        let (name, uuid, skin) = parse_profile(profile).unwrap();
        assert_eq!(name, "Steve");
        assert_eq!(uuid, "069a79f444e94726a5befca90e38aaf5");
        assert_eq!(skin.as_deref(), Some("https://textures.minecraft.net/texture/abc"));

        let error = parse_profile(r#"{"error":"NOT_FOUND","errorMessage":"not purchased"}"#).unwrap_err();
        assert!(error.to_string().contains("not purchased"));
        // 没有皮肤字段也要能解析
        let (name, _, skin) = parse_profile(r#"{"id":"abc","name":"Alex"}"#).unwrap();
        assert_eq!(name, "Alex");
        assert!(skin.is_none());
    }

    #[test]
    fn token_is_masked_in_logs() {
        let token = "eyJhbGciOiJSUzI1NiJ9.long.payload";
        assert_eq!(util::mask_secret(token), "***");
        assert!(!util::redact_secret(&format!("token={token}"), token).contains(token));
    }
}
