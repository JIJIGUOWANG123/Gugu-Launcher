//! 第三方账号（Yggdrasil / authlib-injector 兼容站点，例如 littleskin.cn）。
//!
//! 只做「登录 + 预览站点上正在用的皮肤」：authenticate / refresh / validate 走
//! <server>/authserver/*，皮肤走 <server>/sessionserver/session/minecraft/profile/<uuid>。

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::{AppError, Result};
use crate::paths::Paths;
use crate::skins;
use crate::util;

/// authlib-injector 的下载入口（返回 latest.json，里面有 jar 地址）。
pub const AUTHLIB_LATEST_URL: &str = "https://authlib-injector.yushi.moe/artifact/latest.json";
pub const AUTHLIB_DIR: &str = "authlib-injector";
pub const AUTHLIB_JAR: &str = "authlib-injector.jar";
pub const AUTH_TIMEOUT_SECS: u64 = 12;
pub const DOWNLOAD_USER_AGENT: &str = "Mozilla/5.0 GuguLauncher/1.0";

/// 站点根地址归一：补协议、去末尾斜杠。
pub fn normalize_server(server: &str) -> Result<String> {
    skins::normalize_server(server)
}

pub fn authserver_url(server: &str) -> String {
    format!("{server}/authserver")
}

pub fn authenticate_url(server: &str) -> String {
    format!("{}/authenticate", authserver_url(server))
}

pub fn refresh_url(server: &str) -> String {
    format!("{}/refresh", authserver_url(server))
}

pub fn validate_url(server: &str) -> String {
    format!("{}/validate", authserver_url(server))
}

pub fn profile_url(server: &str, uuid: &str) -> String {
    format!(
        "{server}/sessionserver/session/minecraft/profile/{}",
        uuid.trim().replace('-', "")
    )
}

pub fn authlib_dir(paths: &Paths) -> PathBuf {
    paths.root.join(AUTHLIB_DIR)
}

pub fn authlib_jar(paths: &Paths) -> PathBuf {
    authlib_dir(paths).join(AUTHLIB_JAR)
}

/// 启动参数里的 -javaagent（路径带空格也不用加引号：Java 自己按整串解析）。
pub fn javaagent_arg(jar: &Path, server: &str) -> String {
    let normalized = server.trim().trim_end_matches('/');
    format!("-javaagent:{}={normalized}", jar.display())
}

// ------------------------------------------------------------------ 解析

/// authenticate / refresh 拿到的会话。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct YggdrasilSession {
    pub access_token: String,
    pub client_token: String,
    pub profile_id: String,
    pub profile_name: String,
    /// 该账号可选的其它角色（id / name）
    pub available: Vec<(String, String)>,
}

/// 解析 authenticate 响应：必须带 selectedProfile。
pub fn parse_authenticate(json: &str) -> Result<YggdrasilSession> {
    let value: serde_json::Value = serde_json::from_str(json)
        .map_err(|err| AppError::Net(format!("认证响应格式不对：{err}")))?;
    if let Some(error) = value.get("error").and_then(|item| item.as_str()) {
        let message = value
            .get("errorMessage")
            .and_then(|item| item.as_str())
            .unwrap_or(error);
        return Err(AppError::Other(format!("认证失败：{message}")));
    }
    let access_token = value
        .get("accessToken")
        .and_then(|item| item.as_str())
        .filter(|token| !token.trim().is_empty())
        .ok_or_else(|| AppError::Other("认证失败：服务器没有返回 accessToken".to_string()))?;
    let client_token = value
        .get("clientToken")
        .and_then(|item| item.as_str())
        .unwrap_or_default()
        .to_string();
    let available = parse_available_profiles(&value);
    let Some(profile) = value.get("selectedProfile") else {
        // 有的站点只回 availableProfiles：拿第一个当选中角色
        if let Some((id, name)) = available.first() {
            return Ok(YggdrasilSession {
                access_token: access_token.to_string(),
                client_token,
                profile_id: id.clone(),
                profile_name: name.clone(),
                available,
            });
        }
        return Err(AppError::Other(
            "认证失败：该账号没有可用的角色（请先在皮肤站创建角色）".to_string(),
        ));
    };
    let profile_id = profile
        .get("id")
        .and_then(|item| item.as_str())
        .filter(|id| !id.trim().is_empty())
        .ok_or_else(|| AppError::Other("认证失败：角色缺少 id".to_string()))?;
    let profile_name = profile
        .get("name")
        .and_then(|item| item.as_str())
        .unwrap_or_default()
        .to_string();
    Ok(YggdrasilSession {
        access_token: access_token.to_string(),
        client_token,
        profile_id: profile_id.to_string(),
        profile_name,
        available,
    })
}

fn parse_available_profiles(value: &serde_json::Value) -> Vec<(String, String)> {
    value
        .get("availableProfiles")
        .and_then(|item| item.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|entry| {
                    let id = entry.get("id").and_then(|item| item.as_str())?;
                    let name = entry
                        .get("name")
                        .and_then(|item| item.as_str())
                        .unwrap_or_default();
                    Some((id.to_string(), name.to_string()))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// HTTP 状态码 → 中文（Yggdrasil 的 401/403 都是「密码或令牌不对」）。
pub fn map_auth_error(status: u16, message: &str) -> String {
    let detail = message.trim();
    match status {
        401 | 403 => {
            if detail.is_empty() {
                "用户名或密码不对（也可能是令牌已失效，请重新登录）".to_string()
            } else {
                format!("用户名或密码不对：{detail}")
            }
        }
        400 => {
            if detail.is_empty() {
                "登录请求被服务器拒绝（参数不合法）".to_string()
            } else {
                format!("登录请求被服务器拒绝：{detail}")
            }
        }
        404 => "该站点不支持 Yggdrasil 登录（认证接口不存在）".to_string(),
        429 => "请求太频繁，请稍后再试".to_string(),
        other => format!("皮肤站返回 HTTP {other}"),
    }
}

fn body_error(json: &str) -> String {
    serde_json::from_str::<serde_json::Value>(json)
        .ok()
        .and_then(|value| {
            value
                .get("errorMessage")
                .or_else(|| value.get("error"))
                .and_then(|item| item.as_str())
                .map(str::to_string)
        })
        .unwrap_or_default()
}

async fn post_json(
    client: &reqwest::Client,
    url: &str,
    body: &serde_json::Value,
) -> Result<serde_json::Value> {
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(AUTH_TIMEOUT_SECS),
        client
            .post(url)
            .header(reqwest::header::USER_AGENT, DOWNLOAD_USER_AGENT)
            .header(reqwest::header::ACCEPT, "application/json")
            .json(body)
            .send(),
    )
    .await
    .map_err(|_| {
        AppError::Net(format!(
            "连不上皮肤站（{AUTH_TIMEOUT_SECS} 秒超时）：{url}"
        ))
    })?
    .map_err(|err| AppError::Net(format!("连不上皮肤站：{err}")))?;
    let status = response.status().as_u16();
    let text = response
        .text()
        .await
        .map_err(|err| AppError::Net(format!("读取皮肤站响应失败：{err}")))?;
    if !(200..300).contains(&status) {
        return Err(AppError::Invalid(map_auth_error(status, &body_error(&text))));
    }
    serde_json::from_str(&text).map_err(|err| AppError::Net(format!("皮肤站响应不是 JSON：{err}")))
}

/// POST /authserver/authenticate。
pub async fn authenticate(
    client: &reqwest::Client,
    server: &str,
    username: &str,
    password: &str,
) -> Result<YggdrasilSession> {
    let server = normalize_server(server)?;
    if username.trim().is_empty() || password.is_empty() {
        return Err(AppError::Invalid("用户名和密码都不能为空".to_string()));
    }
    let body = serde_json::json!({
        "agent": { "name": "Minecraft", "version": 1 },
        "username": username.trim(),
        "password": password,
        "requestUser": false,
    });
    let value = post_json(client, &authenticate_url(&server), &body).await?;
    let session = parse_authenticate(&value.to_string())?;
    tracing::info!(
        server = %server,
        profile = %session.profile_name,
        profiles = session.available.len(),
        token = %util::mask_secret(&session.access_token),
        "第三方账号登录成功"
    );
    Ok(session)
}

/// POST /authserver/refresh。
pub async fn refresh(
    client: &reqwest::Client,
    server: &str,
    access_token: &str,
    client_token: &str,
) -> Result<YggdrasilSession> {
    let server = normalize_server(server)?;
    if access_token.trim().is_empty() {
        return Err(AppError::Invalid("没有可刷新的令牌，请重新登录".to_string()));
    }
    let mut body = serde_json::json!({
        "accessToken": access_token.trim(),
        "requestUser": false,
    });
    if !client_token.trim().is_empty() {
        body["clientToken"] = serde_json::json!(client_token.trim());
    }
    let value = post_json(client, &refresh_url(&server), &body).await?;
    parse_authenticate(&value.to_string())
}

/// POST /authserver/validate：204 有效，403 失效。
pub async fn validate(
    client: &reqwest::Client,
    server: &str,
    access_token: &str,
    client_token: &str,
) -> Result<bool> {
    let server = normalize_server(server)?;
    let mut body = serde_json::json!({ "accessToken": access_token.trim() });
    if !client_token.trim().is_empty() {
        body["clientToken"] = serde_json::json!(client_token.trim());
    }
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(AUTH_TIMEOUT_SECS),
        client
            .post(validate_url(&server))
            .header(reqwest::header::USER_AGENT, DOWNLOAD_USER_AGENT)
            .json(&body)
            .send(),
    )
    .await
    .map_err(|_| AppError::Net("校验令牌超时".to_string()))?
    .map_err(|err| AppError::Net(format!("校验令牌失败：{err}")))?;
    Ok(validate_outcome(response.status().as_u16()))
}

/// validate 的状态判定：2xx 有效，其余（403/404/5xx）一律无效。
pub fn validate_outcome(status: u16) -> bool {
    (200..300).contains(&status)
}

/// 启动前令牌方案：validate 通过就直接用，否则必须刷新。
pub const PLAN_USE_CURRENT: &str = "use-current";
pub const PLAN_REFRESH: &str = "refresh";

pub fn launch_token_plan(validate_ok: bool) -> &'static str {
    if validate_ok {
        PLAN_USE_CURRENT
    } else {
        PLAN_REFRESH
    }
}

/// 刷新之后必须再 validate 一次：不过就中止启动。
pub fn refreshed_token_ok(second_validate_ok: bool) -> Result<()> {
    if second_validate_ok {
        Ok(())
    } else {
        Err(AppError::Invalid(
            "登录已过期，请到账号页重新登录".to_string(),
        ))
    }
}

/// 启动前的令牌准备：validate → 需要就 refresh（并立刻持久化，旧值作废）→ 再 validate。
pub async fn prepare_launch_token(
    client: &reqwest::Client,
    server: &str,
    access_token: &str,
    client_token: &str,
    persist: impl FnOnce(&YggdrasilSession) -> Result<()>,
) -> Result<(String, bool)> {
    let server = normalize_server(server)?;
    let token = access_token.trim().to_string();
    let valid = validate(client, &server, &token, client_token)
        .await
        .unwrap_or(false);
    tracing::info!(
        token = %util::mask_secret(&token),
        valid,
        plan = launch_token_plan(valid),
        "启动前 validate"
    );
    if valid {
        return Ok((token, false));
    }
    let session = refresh(client, &server, &token, client_token)
        .await
        .map_err(|err| AppError::Invalid(format!("登录已过期，请到账号页重新登录（{err}）")))?;
    // 刷新会让旧 token 立刻失效：先把新值落盘，再用它启动
    persist(&session)?;
    tracing::info!(
        account = %session.profile_name,
        token = %util::mask_secret(&session.access_token),
        "第三方账号令牌已刷新并持久化"
    );
    let fresh = session.access_token.clone();
    let second = validate(client, &server, &fresh, &session.client_token)
        .await
        .unwrap_or(false);
    refreshed_token_ok(second)?;
    tracing::info!(
        account = %session.profile_name,
        token = %util::mask_secret(&fresh),
        "启动前 validate=ok（刷新后）"
    );
    Ok((fresh, true))
}



// ------------------------------------------------------------------ authlib-injector

/// authlib_ensure 的结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthlibInfo {
    pub path: String,
    pub version: String,
    pub installed: bool,
}

/// 本地已有的 authlib-injector（只看文件在不在）。
pub fn authlib_status(paths: &Paths) -> AuthlibInfo {
    let jar = authlib_jar(paths);
    AuthlibInfo {
        installed: jar.is_file(),
        path: jar.display().to_string(),
        version: read_authlib_version(paths).unwrap_or_default(),
    }
}

fn version_file(paths: &Paths) -> PathBuf {
    authlib_dir(paths).join("version.txt")
}

fn read_authlib_version(paths: &Paths) -> Option<String> {
    std::fs::read_to_string(version_file(paths))
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

/// 首次下载 authlib-injector.jar（已有就复用）。
pub async fn ensure_authlib(client: &reqwest::Client, paths: &Paths) -> Result<AuthlibInfo> {
    let jar = authlib_jar(paths);
    if jar.is_file() {
        return Ok(AuthlibInfo {
            installed: true,
            path: jar.display().to_string(),
            version: read_authlib_version(paths).unwrap_or_default(),
        });
    }
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        client
            .get(AUTHLIB_LATEST_URL)
            .header(reqwest::header::USER_AGENT, DOWNLOAD_USER_AGENT)
            .send(),
    )
    .await
    .map_err(|_| AppError::Net("下载 authlib-injector 超时".to_string()))?
    .map_err(|err| AppError::Net(format!("下载 authlib-injector 失败：{err}")))?;
    if !response.status().is_success() {
        return Err(AppError::Net(format!(
            "下载 authlib-injector 失败：HTTP {}",
            response.status().as_u16()
        )));
    }
    let text = response
        .text()
        .await
        .map_err(|err| AppError::Net(format!("读取 authlib-injector 版本信息失败：{err}")))?;
    let value: serde_json::Value = serde_json::from_str(&text)
        .map_err(|err| AppError::Net(format!("authlib-injector 版本信息不是 JSON：{err}")))?;
    let version = value
        .get("version")
        .and_then(|item| item.as_str())
        .unwrap_or_default()
        .to_string();
    let download = value
        .get("download_url")
        .and_then(|item| item.as_str())
        .ok_or_else(|| AppError::Net("authlib-injector 版本信息里没有下载地址".to_string()))?;
    let bytes = client
        .get(download)
        .header(reqwest::header::USER_AGENT, DOWNLOAD_USER_AGENT)
        .send()
        .await
        .map_err(|err| AppError::Net(format!("下载 authlib-injector 失败：{err}")))?
        .bytes()
        .await
        .map_err(|err| AppError::Net(format!("读取 authlib-injector 失败：{err}")))?;
    std::fs::create_dir_all(authlib_dir(paths))?;
    let tmp = jar.with_extension("jar.part");
    std::fs::write(&tmp, &bytes)?;
    std::fs::rename(&tmp, &jar)?;
    let _ = std::fs::write(version_file(paths), &version);
    tracing::info!(
        path = %jar.display(),
        bytes = bytes.len(),
        version = %version,
        "authlib-injector 已下载"
    );
    Ok(AuthlibInfo {
        installed: true,
        path: jar.display().to_string(),
        version,
    })
}

// ------------------------------------------------------------------ 第三方皮肤预览

/// thirdparty_skin 的返回：站点上正在用的那张皮肤（只读预览）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThirdPartySkinView {
    pub server: String,
    pub name: String,
    pub uuid: String,
    /// 没有皮肤时是空串
    pub skin_url: String,
    pub cape_url: String,
    pub variant: String,
    /// 缩略图 data URL（≤128KB 才给，超了只给 url）
    pub preview_data_url: String,
}

pub const PREVIEW_LIMIT: usize = 128 * 1024;

/// 解析 sessionserver 的 profile：properties 里 name=textures 的 base64 JSON。
pub fn parse_textures(json: &str) -> (Option<String>, Option<String>, Option<String>) {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return (None, None, None);
    };
    if let Some(properties) = value.get("properties").and_then(|item| item.as_array()) {
        for entry in properties {
            if entry.get("name").and_then(|item| item.as_str()) != Some("textures") {
                continue;
            }
            if let Some(encoded) = entry.get("value").and_then(|item| item.as_str()) {
                return skins::parse_textures_value(encoded);
            }
        }
    }
    (None, None, None)
}

/// 预览缓存文件名：thirdparty-<站点主机>-<角色名>.png（不参与本地皮肤列表）。
pub fn cache_file_name(server: &str, name: &str) -> String {
    let host = server
        .split("://")
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .unwrap_or(server)
        .replace(':', "_");
    format!(
        "thirdparty-{}-{}.png",
        util::sanitize_file_part(&host),
        skins::safe_name(name)
    )
}

/// 拉取站点上正在使用的皮肤（带缓存与缩略图）。
pub async fn thirdparty_skin(
    client: &reqwest::Client,
    paths: &Paths,
    server: &str,
    name: &str,
    uuid: &str,
) -> Result<ThirdPartySkinView> {
    let server = normalize_server(server)?;
    if uuid.trim().is_empty() {
        return Err(AppError::Other("这个账号没有角色 id，请重新登录".to_string()));
    }
    let url = profile_url(&server, uuid);
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(AUTH_TIMEOUT_SECS),
        client
            .get(&url)
            .header(reqwest::header::USER_AGENT, DOWNLOAD_USER_AGENT)
            .header(reqwest::header::ACCEPT, "application/json")
            .send(),
    )
    .await
    .map_err(|_| AppError::Net(format!("连不上皮肤站（{AUTH_TIMEOUT_SECS} 秒超时）")))?
    .map_err(|err| AppError::Net(format!("连不上皮肤站：{err}")))?;
    let status = response.status().as_u16();
    let text = response.text().await.unwrap_or_default();
    if !(200..300).contains(&status) {
        return Err(AppError::Net(map_auth_error(status, &body_error(&text))));
    }
    let (skin, cape, model) = parse_textures(&text);
    // 皮肤站多数给 http：先升级 https，下载失败再退回 http
    let skin_url = skin
        .as_deref()
        .map(skins::upgrade_to_https)
        .unwrap_or_default();
    let cape_url = cape
        .as_deref()
        .map(skins::upgrade_to_https)
        .unwrap_or_default();
    let variant = match model.as_deref() {
        Some("slim") => "slim".to_string(),
        _ => "classic".to_string(),
    };
    let mut preview = String::new();
    if !skin_url.is_empty() {
        let cache_dir = paths.skins.join("thirdparty-preview");
        let target = cache_dir.join(cache_file_name(&server, name));
        let raw = skin
            .as_deref()
            .map(str::to_string)
            .unwrap_or_else(|| skin_url.clone());
        match download_preview(client, &skin_url, &target).await {
            Ok(path) => preview = preview_of(&path),
            Err(err) => {
                tracing::warn!(%err, url = %skin_url, "皮肤站贴图 https 下载失败，回退原始地址");
                if let Ok(path) = download_preview(client, &raw, &target).await {
                    preview = preview_of(&path);
                }
            }
        }
    }
    tracing::info!(
        server = %server,
        player = %name,
        skin = !skin_url.is_empty(),
        cape = !cape_url.is_empty(),
        variant = %variant,
        "第三方皮肤预览已就绪"
    );
    Ok(ThirdPartySkinView {
        server,
        name: name.to_string(),
        uuid: uuid.to_string(),
        skin_url,
        cape_url,
        variant,
        preview_data_url: preview,
    })
}

async fn download_preview(
    client: &reqwest::Client,
    url: &str,
    target: &Path,
) -> Result<PathBuf> {
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        client
            .get(url)
            .header(reqwest::header::USER_AGENT, DOWNLOAD_USER_AGENT)
            .send(),
    )
    .await
    .map_err(|_| AppError::Net("下载皮肤超时".to_string()))?
    .map_err(|err| AppError::Net(format!("下载皮肤失败：{err}")))?;
    if !response.status().is_success() {
        return Err(AppError::Net(format!(
            "下载皮肤失败：HTTP {}",
            response.status().as_u16()
        )));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|err| AppError::Net(format!("读取皮肤内容失败：{err}")))?;
    skins::validate_skin_png(&bytes)?;
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(target, &bytes)?;
    Ok(target.to_path_buf())
}

fn preview_of(path: &Path) -> String {
    let Ok(size) = std::fs::metadata(path).map(|meta| meta.len()) else {
        return String::new();
    };
    if size == 0 || size as usize > PREVIEW_LIMIT {
        return String::new();
    }
    match std::fs::read(path) {
        Ok(bytes) => format!("data:image/png;base64,{}", util::base64_encode(&bytes)),
        Err(_) => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AUTH_OK: &str = r#"{
        "accessToken":"tok-1",
        "clientToken":"client-1",
        "availableProfiles":[{"id":"aaaabbbbccccddddeeeeffff00001111","name":"JiJi"}],
        "selectedProfile":{"id":"aaaabbbbccccddddeeeeffff00001111","name":"JiJi"}
    }"#;

    #[test]
    fn authenticate_response_is_parsed() {
        let session = parse_authenticate(AUTH_OK).unwrap();
        assert_eq!(session.access_token, "tok-1");
        assert_eq!(session.client_token, "client-1");
        assert_eq!(session.profile_id, "aaaabbbbccccddddeeeeffff00001111");
        assert_eq!(session.profile_name, "JiJi");
        assert_eq!(session.available.len(), 1);
    }

    #[test]
    fn authenticate_without_selected_profile_falls_back_to_available() {
        let json = r#"{"accessToken":"tok","availableProfiles":[{"id":"id-1","name":"Alt"}]}"#;
        let session = parse_authenticate(json).unwrap();
        assert_eq!(session.profile_id, "id-1");
        assert_eq!(session.profile_name, "Alt");
    }

    #[test]
    fn authenticate_without_any_profile_is_readable() {
        let error = parse_authenticate(r#"{"accessToken":"tok","availableProfiles":[]}"#)
            .unwrap_err()
            .to_string();
        assert!(error.contains("没有可用的角色"), "{error}");
        let error = parse_authenticate(r#"{"error":"FORBIDDEN","errorMessage":"bad token"}"#)
            .unwrap_err()
            .to_string();
        assert!(error.contains("bad token"), "{error}");
        assert!(parse_authenticate("not json").is_err());
    }

    #[test]
    fn auth_errors_map_to_chinese() {
        assert!(map_auth_error(401, "").contains("用户名或密码不对"));
        assert!(map_auth_error(403, "token expired").contains("token expired"));
        assert!(map_auth_error(404, "").contains("不支持 Yggdrasil"));
        assert!(map_auth_error(429, "").contains("太频繁"));
        assert!(map_auth_error(500, "").contains("HTTP 500"));
    }

    #[test]
    fn validate_outcome_has_three_branches() {
        assert!(validate_outcome(204), "204 = 令牌有效");
        assert!(validate_outcome(200));
        assert!(!validate_outcome(403), "403 = 令牌失效");
        assert!(!validate_outcome(500), "5xx = 服务器问题，按无效处理");
    }

    #[test]
    fn server_urls_are_normalized() {
        assert_eq!(
            normalize_server("https://littleskin.cn/api/yggdrasil/").unwrap(),
            "https://littleskin.cn/api/yggdrasil"
        );
        assert_eq!(
            authenticate_url("https://littleskin.cn/api/yggdrasil"),
            "https://littleskin.cn/api/yggdrasil/authserver/authenticate"
        );
        assert_eq!(
            refresh_url("https://x.cn/api/yggdrasil"),
            "https://x.cn/api/yggdrasil/authserver/refresh"
        );
        assert_eq!(
            validate_url("https://x.cn/api/yggdrasil"),
            "https://x.cn/api/yggdrasil/authserver/validate"
        );
        assert!(normalize_server("  ").is_err());
    }

    #[test]
    fn profile_url_strips_dashes() {
        assert_eq!(
            profile_url("https://littleskin.cn/api/yggdrasil", "aaaa-bbbb-cccc"),
            "https://littleskin.cn/api/yggdrasil/sessionserver/session/minecraft/profile/aaaabbbbcccc"
        );
    }

    #[test]
    fn javaagent_argument_keeps_spaces_and_normalizes_server() {
        let jar = std::path::Path::new(r"D:\Minecraft\咕咕 启动器\.mc\authlib-injector\authlib-injector.jar");
        let arg = javaagent_arg(jar, "https://littleskin.cn/api/yggdrasil/");
        assert!(arg.starts_with("-javaagent:"), "{arg}");
        assert!(arg.ends_with("=https://littleskin.cn/api/yggdrasil"), "{arg}");
        assert!(arg.contains("咕咕 启动器"), "路径里的空格要保留：{arg}");
    }

    #[test]
    fn textures_base64_decodes_skin_and_cape() {
        let textures = r#"{"textures":{"SKIN":{"url":"http://textures.minecraft.net/texture/s1","metadata":{"model":"slim"}},"CAPE":{"url":"http://textures.minecraft.net/texture/c1"}},"profileName":"JiJi"}"#;
        let encoded = util::base64_encode(textures.as_bytes());
        let json = format!(
            r#"{{"id":"aaaabbbbccccddddeeeeffff00001111","name":"JiJi","properties":[{{"name":"textures","value":"{encoded}"}}]}}"#
        );
        let (skin, cape, model) = parse_textures(&json);
        assert_eq!(skin.as_deref(), Some("http://textures.minecraft.net/texture/s1"));
        assert_eq!(cape.as_deref(), Some("http://textures.minecraft.net/texture/c1"));
        assert_eq!(model.as_deref(), Some("slim"));
        // http → https 升级
        assert_eq!(
            skins::upgrade_to_https(&skin.unwrap()),
            "https://textures.minecraft.net/texture/s1"
        );
    }

    #[test]
    fn textures_without_skin_branch_is_empty() {
        let textures = r#"{"textures":{"CAPE":{"url":"http://textures.minecraft.net/texture/c1"}}}"#;
        let encoded = util::base64_encode(textures.as_bytes());
        let json = format!(r#"{{"properties":[{{"name":"textures","value":"{encoded}"}}]}}"#);
        let (skin, cape, _) = parse_textures(&json);
        assert!(skin.is_none(), "没有 SKIN 就返回 None（前端显示默认皮肤）");
        assert_eq!(cape.as_deref(), Some("http://textures.minecraft.net/texture/c1"));
        // 没有 properties
        let (skin, cape, _) = parse_textures(r#"{"id":"x"}"#);
        assert!(skin.is_none() && cape.is_none());
    }

    #[test]
    fn preview_cache_name_is_sanitized() {
        let name = cache_file_name("https://littleskin.cn/api/yggdrasil", "JiJi/../坏 名");
        assert!(name.starts_with("thirdparty-littleskin.cn-"), "{name}");
        assert!(name.ends_with(".png"), "{name}");
        assert!(!name.contains('/'), "{name}");
        assert!(!name.contains(".."), "{name}");
        let with_port = cache_file_name("http://127.0.0.1:8080/api/yggdrasil", "Steve");
        assert!(with_port.contains("127.0.0.1_8080"), "{with_port}");
    }

#[test]
    fn launch_token_plan_decides_use_or_refresh() {
        assert_eq!(launch_token_plan(true), PLAN_USE_CURRENT);
        assert_eq!(launch_token_plan(false), PLAN_REFRESH);
    }

    #[test]
    fn refreshed_token_must_validate_again() {
        assert!(refreshed_token_ok(true).is_ok());
        let error = refreshed_token_ok(false).unwrap_err().to_string();
        assert!(error.contains("登录已过期，请到账号页重新登录"), "{error}");
    }

    #[test]
    fn authlib_status_reports_missing_jar() {
        let paths = crate::paths::Paths::from_root(std::env::temp_dir().join(format!(
            "gugu-authlib-{}",
            util::unique_suffix()
        )));
        let info = authlib_status(&paths);
        assert!(!info.installed);
        assert!(info.path.ends_with("authlib-injector.jar"));
        assert!(info.version.is_empty());
        // 放一个假 jar：状态要变成已安装
        std::fs::create_dir_all(authlib_dir(&paths)).unwrap();
        std::fs::write(authlib_jar(&paths), b"fake").unwrap();
        assert!(authlib_status(&paths).installed);
        let _ = std::fs::remove_dir_all(&paths.root);
    }
}
