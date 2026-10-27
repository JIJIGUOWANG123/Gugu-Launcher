//! 正版披风与皮肤上传（api.minecraftservices.com）。
//!
//! 只有当前账号是微软账号、且令牌可用时才有这些能力；其余情况一律返回
//! supported=false + 一句给用户看的中文 reason，界面直接显示。

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::accounts::{self, StoredAccount};
use crate::error::{AppError, Result};
use crate::paths::Paths;
use crate::settings::Settings;
use crate::skins;
use crate::util;

pub const PROFILE_URL: &str = "https://api.minecraftservices.com/minecraft/profile";
pub const SKIN_UPLOAD_URL: &str = "https://api.minecraftservices.com/minecraft/profile/skins";
pub const CAPE_ACTIVE_URL: &str = "https://api.minecraftservices.com/minecraft/profile/capes/active";
/// 装披风接口只认 PUT（POST 会 405）。
pub const CAPE_ACTIVE_METHOD: &str = "PUT";

/// 披风预览图上限：单张不超过 32KB 才做成 data URL 回给前端。
pub const CAPE_PREVIEW_LIMIT: usize = 32 * 1024;
/// 皮肤文件上限 1MB。
pub const SKIN_UPLOAD_LIMIT: usize = 1024 * 1024;

/// multipart 的两个字段名（Mojang 认死的名字，不能改）。
pub const SKIN_FIELD_VARIANT: &str = "variant";
pub const SKIN_FIELD_FILE: &str = "file";
pub const SKIN_FILE_NAME: &str = "skin.png";

/// 披风不支持时的四句原因（前端原样显示）。
pub const REASON_OFFLINE: &str = "离线账号不支持披风，登录微软账号后可用";
pub const REASON_THIRDPARTY: &str = "第三方（皮肤站）账号不支持披风";
pub const REASON_NO_LOGIN: &str = "请先登录微软账号";
pub const REASON_EXPIRED: &str = "登录已过期，请重新登录";

/// capes_list 的元素。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapeListItem {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    pub url: String,
    /// ACTIVE / INACTIVE
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview_data_url: Option<String>,
}

/// capes_list 的结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapesListResult {
    pub supported: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub active: Option<String>,
    pub items: Vec<CapeListItem>,
}

impl CapesListResult {
    fn unsupported(reason: &str) -> Self {
        Self {
            supported: false,
            reason: Some(reason.to_string()),
            active: None,
            items: Vec::new(),
        }
    }
}

/// cape_set_active 的结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapeState {
    pub active: Option<String>,
}

/// ms_skin_upload 的结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MsSkinUploadResult {
    pub ok: bool,
    pub variant: String,
    pub uploaded_at: String,
    pub cape_synced: bool,
}

// ------------------------------------------------------------------ 纯函数（可单测）

/// 当前账号能不能用披风；能用返回 None，不能用返回给用户看的那句话。
pub fn unsupported_reason(account: Option<&StoredAccount>) -> Option<&'static str> {
    match account {
        None => Some(REASON_NO_LOGIN),
        Some(account) if account.kind == "offline" => Some(REASON_OFFLINE),
        Some(account) if account.kind != "ms" => Some(REASON_THIRDPARTY),
        Some(account) if account.access_token.trim().is_empty() => Some(REASON_NO_LOGIN),
        Some(_) => None,
    }
}

/// 已知披风的中文名；表里没有就用 alias，再没有就用 id。
pub fn cape_display_name(alias: Option<&str>, id: &str) -> String {
    if let Some(alias) = alias.map(str::trim).filter(|value| !value.is_empty()) {
        let known = match alias.to_lowercase().as_str() {
            "migrator" => Some("迁移者披风"),
            "vanilla" => Some("原版披风"),
            "cherry blossom" => Some("樱花披风"),
            "15th anniversary" => Some("15 周年披风"),
            "founder's" | "founders" => Some("创始人披风"),
            "pan" => Some("潘披风"),
            "common" => Some("平民披风"),
            "purple heart" => Some("紫心披风"),
            "copper" => Some("铜披风"),
            "turtle" => Some("海龟披风"),
            "mojang" => Some("Mojang 披风"),
            "mojang classic" => Some("Mojang 经典披风"),
            "mojang studios" => Some("Mojang Studios 披风"),
            "realms" | "realms mapmaker" => Some("Realms 披风"),
            "translator" => Some("翻译者披风"),
            "chinese translator" => Some("中文翻译者披风"),
            "cobalt" => Some("钴披风"),
            "scrolls" => Some("卷轴披风"),
            "snake" => Some("蛇年披风"),
            "birthday" => Some("生日披风"),
            "mcc 15th year" => Some("MCC 15 周年披风"),
            "minecon 2011" => Some("Minecon 2011 披风"),
            "minecon 2012" => Some("Minecon 2012 披风"),
            "minecon 2013" => Some("Minecon 2013 披风"),
            "minecon 2015" => Some("Minecon 2015 披风"),
            "minecon 2016" => Some("Minecon 2016 披风"),
            "millionth customer" => Some("第一百万位顾客披风"),
            _ => None,
        };
        if let Some(name) = known {
            return name.to_string();
        }
        return alias.to_string();
    }
    id.to_string()
}

/// 解析 /minecraft/profile：返回 (披风列表, 当前披风的 id)。
pub fn parse_profile_capes(json: &str) -> Result<(Vec<CapeListItem>, Option<String>)> {
    let value: serde_json::Value = serde_json::from_str(json)
        .map_err(|err| AppError::Net(format!("账号资料格式不对：{err}")))?;
    if let Some(error) = value.get("error").and_then(|item| item.as_str()) {
        if value.get("capes").is_none() {
            return Err(AppError::Net(format!("账号资料读取失败：{error}")));
        }
    }
    let mut items = Vec::new();
    let mut active = None;
    if let Some(capes) = value.get("capes").and_then(|item| item.as_array()) {
        for cape in capes {
            let Some(id) = cape.get("id").and_then(|item| item.as_str()) else {
                continue;
            };
            let alias = cape
                .get("alias")
                .and_then(|item| item.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            let state = cape
                .get("state")
                .and_then(|item| item.as_str())
                .unwrap_or("INACTIVE")
                .to_uppercase();
            if state == "ACTIVE" {
                active = Some(id.to_string());
            }
            let url = cape
                .get("url")
                .and_then(|item| item.as_str())
                .unwrap_or_default()
                .to_string();
            items.push(CapeListItem {
                id: id.to_string(),
                name: cape_display_name(alias.as_deref(), id),
                alias,
                url,
                state,
                preview_data_url: None,
            });
        }
    }
    Ok((items, active))
}

/// cape_set_active 的请求体：null 表示不使用披风（不传 capeId 字段）。
pub fn cape_body(cape_id: Option<&str>) -> serde_json::Value {
    match cape_id.map(str::trim).filter(|value| !value.is_empty()) {
        Some(id) => serde_json::json!({ "capeId": id }),
        None => serde_json::json!({}),
    }
}

/// 装披风接口的 HTTP 状态码 → 中文。
pub fn map_cape_error(status: u16) -> String {
    match status {
        400 | 404 => "该披风不属于此账号".to_string(),
        401 | 403 => REASON_EXPIRED.to_string(),
        429 => "操作太频繁，请稍后再试".to_string(),
        other => format!("设置披风失败：HTTP {other}"),
    }
}

/// 上传皮肤接口的 HTTP 状态码 → 中文。
pub fn map_upload_error(status: u16) -> String {
    match status {
        400 => "皮肤图片不符合要求（需要 64×64 或 64×32 的 PNG）".to_string(),
        401 | 403 => REASON_EXPIRED.to_string(),
        429 => "上传太频繁，请稍后再试".to_string(),
        other => format!("上传皮肤失败：HTTP {other}"),
    }
}

/// 皮肤变体白名单。
pub fn validate_variant(variant: &str) -> Result<&'static str> {
    match variant.trim().to_lowercase().as_str() {
        "classic" | "default" | "wide" => Ok("classic"),
        "slim" => Ok("slim"),
        other => Err(AppError::Invalid(format!(
            "皮肤模型只支持 classic / slim（收到 {other}）"
        ))),
    }
}

/// 校验通过、可以直接上传的皮肤。
#[derive(Debug, Clone)]
pub struct PreparedSkin {
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// 由 64×32 补齐成 64×64
    pub padded: bool,
    pub variant: &'static str,
}

/// 64×32 自动补齐成 64×64；其余尺寸一律拒绝。
pub fn prepare_skin(bytes: &[u8], variant: &str) -> Result<PreparedSkin> {
    let variant = validate_variant(variant)?;
    if bytes.len() > SKIN_UPLOAD_LIMIT {
        return Err(AppError::Invalid(format!(
            "皮肤文件不能超过 1MB（当前 {} KB）",
            bytes.len() / 1024
        )));
    }
    let (width, height) = crate::png::png_size(bytes)
        .ok_or_else(|| AppError::Invalid("皮肤必须是 PNG 图片".to_string()))?;
    match (width, height) {
        (64, 64) => Ok(PreparedSkin {
            bytes: bytes.to_vec(),
            width,
            height,
            padded: false,
            variant,
        }),
        (64, 32) => {
            let padded = crate::png::legacy_to_modern(bytes)?;
            Ok(PreparedSkin {
                bytes: padded,
                width: 64,
                height: 64,
                padded: true,
                variant,
            })
        }
        (width, height) => Err(AppError::Invalid(format!(
            "皮肤尺寸必须是 64×64 或 64×32（当前 {width}×{height}）"
        ))),
    }
}

/// multipart 里的字段（单测直接核对名字）。
pub fn multipart_fields(variant: &str) -> Vec<(&'static str, String)> {
    vec![
        (SKIN_FIELD_VARIANT, variant.to_string()),
        (SKIN_FIELD_FILE, SKIN_FILE_NAME.to_string()),
    ]
}

/// 手写 multipart/form-data 请求体：两个字段（variant + file），不额外引 reqwest 的 multipart 特性。
pub fn build_multipart(boundary: &str, variant: &str, png: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(png.len() + 512);
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        format!("Content-Disposition: form-data; name=\"{SKIN_FIELD_VARIANT}\"\r\n\r\n").as_bytes(),
    );
    body.extend_from_slice(variant.as_bytes());
    body.extend_from_slice(format!("\r\n--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        format!(
            "Content-Disposition: form-data; name=\"{SKIN_FIELD_FILE}\"; filename=\"{SKIN_FILE_NAME}\"\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(b"Content-Type: image/png\r\n\r\n");
    body.extend_from_slice(png);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    body
}

/// 上传完皮肤后同步披风：成功 True，失败 False（只 warn，不影响上传结果）。
pub fn cape_sync_flag(active: Option<&str>, outcome: Result<()>) -> bool {
    match outcome {
        Ok(()) => active.is_some(),
        Err(err) => {
            tracing::warn!(%err, "上传皮肤后同步披风失败（忽略）");
            false
        }
    }
}

// ------------------------------------------------------------------ 会话

/// 当前微软账号 + 可用令牌（必要时刷新一次）。
pub struct MsSession {
    pub account: StoredAccount,
    pub token: String,
}

fn current_account(paths: &Paths) -> Option<StoredAccount> {
    let file = accounts::load(paths);
    let id = file.current?;
    file.accounts.into_iter().find(|account| account.id == id)
}

/// 拿一份能用的 MC 令牌：先试现有的，401 再刷新一次。
pub async fn require_session(
    client: &reqwest::Client,
    paths: &Paths,
    settings: &Settings,
) -> Result<MsSession> {
    let account = current_account(paths);
    if let Some(reason) = unsupported_reason(account.as_ref()) {
        return Err(AppError::Invalid(reason.to_string()));
    }
    let account = account.expect("unsupported_reason 已排除 None");
    let token = account.access_token.trim().to_string();
    if !token.is_empty() {
        match fetch_profile(client, &token).await? {
            ProfileOutcome::Ok(_) => {
                return Ok(MsSession {
                    token,
                    account,
                })
            }
            ProfileOutcome::Expired => {}
        }
    }
    let refreshed = crate::ms_login::refresh_access_token(client, paths, settings, &account).await?;
    match refreshed {
        Some(token) => Ok(MsSession { account, token }),
        None => Err(AppError::Invalid(REASON_EXPIRED.to_string())),
    }
}

pub enum ProfileOutcome {
    Ok(String),
    Expired,
}

/// GET /minecraft/profile；401/403 折算成 Expired。
pub async fn fetch_profile(client: &reqwest::Client, token: &str) -> Result<ProfileOutcome> {
    let response = client
        .get(PROFILE_URL)
        .bearer_auth(token)
        .send()
        .await
        .map_err(|err| AppError::Net(format!("读取账号资料失败：{err}")))?;
    let status = response.status().as_u16();
    let text = response
        .text()
        .await
        .map_err(|err| AppError::Net(format!("读取账号资料失败：{err}")))?;
    match status {
        200 => Ok(ProfileOutcome::Ok(text)),
        401 | 403 => Ok(ProfileOutcome::Expired),
        other => Err(AppError::Net(format!("账号资料接口返回 HTTP {other}"))),
    }
}

// ------------------------------------------------------------------ 命令实现

/// capes_list：微软账号才有披风，离线/第三方/过期都返回 supported=false。
pub async fn capes_list(
    client: &reqwest::Client,
    paths: &Paths,
    settings: &Settings,
) -> Result<CapesListResult> {
    let account = current_account(paths);
    if let Some(reason) = unsupported_reason(account.as_ref()) {
        return Ok(CapesListResult::unsupported(reason));
    }
    let session = match require_session(client, paths, settings).await {
        Ok(session) => session,
        Err(AppError::Invalid(message)) => return Ok(CapesListResult::unsupported(&message)),
        Err(err) => return Err(err),
    };
    let text = match fetch_profile(client, &session.token).await? {
        ProfileOutcome::Ok(text) => text,
        ProfileOutcome::Expired => return Ok(CapesListResult::unsupported(REASON_EXPIRED)),
    };
    let (mut items, active) = parse_profile_capes(&text)?;
    let dir = skins::capes_dir(paths);
    for item in items.iter_mut() {
        item.preview_data_url = cache_cape_preview(client, &dir, &item.id, &item.url).await;
    }
    tracing::info!(
        account = %session.account.name,
        capes = items.len(),
        active = ?active,
        "披风列表读取完成"
    );
    Ok(CapesListResult {
        supported: true,
        reason: None,
        active,
        items,
    })
}

/// 下载披风贴图到 .mc/capes/<id>.png（有缓存就不重复拉），返回预览 data URL。
async fn cache_cape_preview(
    client: &reqwest::Client,
    dir: &Path,
    id: &str,
    url: &str,
) -> Option<String> {
    if url.trim().is_empty() {
        return None;
    }
    let safe: String = id
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .collect();
    if safe.is_empty() {
        return None;
    }
    let path = dir.join(format!("{safe}.png"));
    if !path.is_file() {
        let response = client.get(url).send().await.ok()?;
        if !response.status().is_success() {
            tracing::warn!(cape = %safe, status = response.status().as_u16(), "披风贴图下载失败");
            return None;
        }
        let bytes = response.bytes().await.ok()?;
        if std::fs::create_dir_all(dir).is_err() {
            return None;
        }
        if std::fs::write(&path, &bytes).is_err() {
            return None;
        }
    }
    preview_of(&path)
}

/// 只有 ≤32KB 的披风图才做成 data URL。
pub fn preview_of(path: &PathBuf) -> Option<String> {
    let size = std::fs::metadata(path).ok()?.len() as usize;
    if size == 0 || size > CAPE_PREVIEW_LIMIT {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    Some(format!(
        "data:image/png;base64,{}",
        util::base64_encode(&bytes)
    ))
}

/// 装披风的请求：PUT + Bearer + JSON（Mojang 只认 PUT）。
pub fn cape_request(client: &reqwest::Client, token: &str, cape_id: Option<&str>) -> reqwest::RequestBuilder {
    client
        .put(CAPE_ACTIVE_URL)
        .bearer_auth(token)
        .json(&cape_body(cape_id))
}

/// cape_set_active：null = 不使用披风。
pub async fn cape_set_active(
    client: &reqwest::Client,
    paths: &Paths,
    settings: &Settings,
    cape_id: Option<String>,
) -> Result<CapeState> {
    let session = require_session(client, paths, settings).await?;
    let response = cape_request(client, &session.token, cape_id.as_deref())
        .send()
        .await
        .map_err(|err| AppError::Net(format!("设置披风失败：{err}")))?;
    let status = response.status().as_u16();
    let text = response.text().await.unwrap_or_default();
    if !(200..300).contains(&status) {
        tracing::warn!(status, method = CAPE_ACTIVE_METHOD, "设置披风被拒绝");
        return Err(AppError::Net(map_cape_error(status)));
    }
    let active = cape_id
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty());
    tracing::info!(
        account = %session.account.name,
        active = ?active,
        bytes = text.len(),
        "披风已切换"
    );
    Ok(CapeState { active })
}

/// 当前生效的披风 id（上传皮肤后同步用）。
async fn active_cape_id(client: &reqwest::Client, token: &str) -> Result<Option<String>> {
    match fetch_profile(client, token).await? {
        ProfileOutcome::Ok(text) => {
            let (_, active) = parse_profile_capes(&text)?;
            Ok(active)
        }
        ProfileOutcome::Expired => Err(AppError::Invalid(REASON_EXPIRED.to_string())),
    }
}

/// 去掉 data URL 前缀（带不带都行）。
pub fn strip_data_url_prefix(raw: &str) -> &str {
    let text = raw.trim();
    if let Some(rest) = text.strip_prefix("data:") {
        if let Some(index) = rest.find("base64,") {
            return rest[index + 7..].trim();
        }
    }
    text
}

/// 解码 base64 皮肤数据。
pub fn decode_skin_base64(raw: &str) -> Result<Vec<u8>> {
    util::base64_decode(strip_data_url_prefix(raw))
        .ok_or_else(|| AppError::Invalid("皮肤数据不是有效的 PNG".to_string()))
}

/// 皮肤数据来源：base64 优先，其次磁盘路径（内置皮肤没有真实路径）。
pub fn read_skin_source(png_path: Option<&str>, png_base64: Option<&str>) -> Result<Vec<u8>> {
    if let Some(raw) = png_base64.map(str::trim).filter(|value| !value.is_empty()) {
        return decode_skin_base64(raw);
    }
    if let Some(raw) = png_path.map(str::trim).filter(|value| !value.is_empty()) {
        let path = Path::new(raw);
        if !path.is_file() {
            return Err(AppError::Invalid(format!(
                "皮肤文件不存在：{}",
                path.display()
            )));
        }
        return Ok(std::fs::read(path)?);
    }
    Err(AppError::Invalid(
        "缺少皮肤数据（pngPath 与 pngBase64 都没有）".to_string(),
    ))
}

/// ms_skin_upload：校验 → 上传 → 同步披风。
pub async fn skin_upload(
    client: &reqwest::Client,
    paths: &Paths,
    settings: &Settings,
    png_path: Option<&str>,
    png_base64: Option<&str>,
    variant: &str,
) -> Result<MsSkinUploadResult> {
    let variant = validate_variant(variant)?;
    let bytes = read_skin_source(png_path, png_base64)?;
    let prepared = prepare_skin(&bytes, variant)?;
    let session = require_session(client, paths, settings).await?;

    let boundary = format!("----GuguBoundary{}", util::unique_suffix());
    let body = build_multipart(&boundary, prepared.variant, &prepared.bytes);
    let response = client
        .post(SKIN_UPLOAD_URL)
        .bearer_auth(&session.token)
        .header(
            "Content-Type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(body)
        .send()
        .await
        .map_err(|err| AppError::Net(format!("上传皮肤失败：{err}")))?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        tracing::warn!(status, "上传皮肤被拒绝");
        return Err(AppError::Net(map_upload_error(status)));
    }
    tracing::info!(
        account = %session.account.name,
        variant = prepared.variant,
        padded = prepared.padded,
        bytes = prepared.bytes.len(),
        token = %util::mask_secret(&session.token),
        "皮肤已上传"
    );

    // 上传会顶掉当前披风：重新选一次
    let active = active_cape_id(client, &session.token).await.unwrap_or(None);
    let synced = match active.as_deref() {
        Some(id) => {
            let outcome = async {
                let response = cape_request(client, &session.token, Some(id))
                    .send()
                    .await
                    .map_err(|err| AppError::Net(format!("同步披风失败：{err}")))?;
                if !(200..300).contains(&response.status().as_u16()) {
                    return Err(AppError::Net(map_cape_error(response.status().as_u16())));
                }
                Ok(())
            }
            .await;
            cape_sync_flag(Some(id), outcome)
        }
        None => false,
    };

    Ok(MsSkinUploadResult {
        ok: true,
        variant: prepared.variant.to_string(),
        uploaded_at: util::iso8601(util::now_unix()),
        cape_synced: synced,
    })
}

// ------------------------------------------------------------------ 账号外观

/// /minecraft/profile 解析出来的账号外观。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountProfile {
    /// 恒为 ms：能读到 profile 就说明令牌是微软的
    pub kind: String,
    pub name: String,
    pub uuid: String,
    /// 没有皮肤时是空串（前端按空串判空）
    pub skin_url: String,
    /// 没有生效披风时是空串
    pub cape_url: String,
    /// classic / slim（缺失按 classic）
    pub variant: String,
}

/// 解析 /minecraft/profile：skins[0].{url,variant} 与 capes 里的 ACTIVE 项。
pub fn parse_account_profile(json: &str) -> Result<AccountProfile> {
    let value: serde_json::Value = serde_json::from_str(json)
        .map_err(|err| AppError::Net(format!("账号资料格式不对：{err}")))?;
    if let Some(error) = value.get("error").and_then(|item| item.as_str()) {
        let message = value
            .get("errorMessage")
            .and_then(|item| item.as_str())
            .unwrap_or(error);
        return Err(AppError::Other(format!("读取 Minecraft 档案失败：{message}")));
    }
    let name = value
        .get("name")
        .and_then(|item| item.as_str())
        .unwrap_or_default()
        .to_string();
    let uuid = value
        .get("id")
        .and_then(|item| item.as_str())
        .unwrap_or_default()
        .to_string();
    let skin = value
        .get("skins")
        .and_then(|item| item.as_array())
        .and_then(|skins| skins.first());
    let skin_url = skin
        .and_then(|entry| entry.get("url"))
        .and_then(|item| item.as_str())
        .unwrap_or_default()
        .to_string();
    let variant = skin
        .and_then(|entry| entry.get("variant"))
        .and_then(|item| item.as_str())
        .map(|value| value.to_lowercase())
        .filter(|value| value == "slim" || value == "classic")
        .unwrap_or_else(|| "classic".to_string());
    let (capes, _) = parse_profile_capes(json)?;
    let cape_url = capes
        .iter()
        .find(|cape| cape.state == "ACTIVE")
        .map(|cape| cape.url.clone())
        .filter(|url| !url.trim().is_empty())
        .unwrap_or_default();
    Ok(AccountProfile {
        kind: "ms".to_string(),
        name,
        uuid,
        skin_url,
        cape_url,
        variant,
    })
}

/// ms_profile：读一次账号资料（失败只报错，不动账号状态）。
pub async fn ms_profile(
    client: &reqwest::Client,
    paths: &Paths,
    settings: &Settings,
) -> Result<AccountProfile> {
    let session = require_session(client, paths, settings).await?;
    match fetch_profile(client, &session.token).await? {
        ProfileOutcome::Ok(text) => parse_account_profile(&text),
        ProfileOutcome::Expired => Err(AppError::Invalid(REASON_EXPIRED.to_string())),
    }
}

/// 皮肤落盘：<skins>/<sanitize(name)>.png（同名覆盖，登录同步与手动下载共用）。
pub fn save_skin_bytes(paths: &Paths, name: &str, bytes: &[u8]) -> Result<PathBuf> {
    if crate::png::png_size(bytes).is_none() {
        return Err(AppError::Invalid("下载到的皮肤不是 PNG".to_string()));
    }
    std::fs::create_dir_all(&paths.skins)?;
    let file = paths
        .skins
        .join(format!("{}.png", skins::safe_name(name)));
    std::fs::write(&file, bytes)?;
    Ok(file)
}

/// 清掉 .mc/capes 里的贴图缓存（重新登录后强制重新拉）。
pub fn invalidate_cape_cache(paths: &Paths) -> usize {
    let dir = skins::capes_dir(paths);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("png")
            && std::fs::remove_file(&path).is_ok()
        {
            removed += 1;
        }
    }
    removed
}

/// 登录/手动刷新后：拉资料 → 皮肤落盘 → 回写账号 → 清披风缓存。
///
/// 皮肤下载失败不影响登录成功：只是账号上没有本地皮肤。
pub async fn refresh_appearance(
    client: &reqwest::Client,
    paths: &Paths,
    token: &str,
    account_id: i64,
) -> Result<AccountProfile> {
    let text = match fetch_profile(client, token).await? {
        ProfileOutcome::Ok(text) => text,
        ProfileOutcome::Expired => return Err(AppError::Invalid(REASON_EXPIRED.to_string())),
    };
    let profile = parse_account_profile(&text)?;
    let mut local: Option<String> = None;
    if let Some(url) = Some(profile.skin_url.clone()).filter(|url| !url.trim().is_empty()) {
        match download_skin(client, paths, &profile.name, &url).await {
            Ok(path) => match path.to_str() {
                Some(text) => local = Some(text.to_string()),
                None => tracing::warn!("皮肤文件路径不是合法 UTF-8，忽略"),
            },
            Err(err) => tracing::warn!(%err, "登录后下载皮肤失败（不影响登录）"),
        }
    }
    if let Err(err) = accounts::set_appearance(
        paths,
        account_id,
        Some(profile.skin_url.as_str()),
        local.as_deref(),
    ) {
        tracing::warn!(%err, "回写账号外观失败（不影响登录）");
    }
    let removed = invalidate_cape_cache(paths);
    tracing::info!(
        player = %profile.name,
        skin = if profile.skin_url.is_empty() { "无" } else { "有" },
        cape = if profile.cape_url.is_empty() { "无" } else { "有" },
        variant = %profile.variant,
        capes_cleared = removed,
        "账号外观已同步"
    );
    Ok(profile)
}

async fn download_skin(
    client: &reqwest::Client,
    paths: &Paths,
    name: &str,
    url: &str,
) -> Result<PathBuf> {
    let response = client
        .get(url)
        .send()
        .await
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
    save_skin_bytes(paths, name, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(kind: &str, token: &str) -> StoredAccount {
        StoredAccount {
            id: 1,
            name: "Steve".to_string(),
            kind: kind.to_string(),
            uuid: "0".repeat(32),
            access_token: token.to_string(),
            refresh_token: String::new(),
            skin_url: String::new(),
            local_skin: String::new(),
            created_at: String::new(),
        }
    }

    fn png(width: u32, height: u32) -> Vec<u8> {
        let pixels = vec![7u8; (width * height * 4) as usize];
        crate::png::encode_png(width, height, &pixels).unwrap()
    }

    #[test]
    fn parses_profile_with_active_cape() {
        let json = r#"{"id":"uuid","name":"Steve","capes":[
            {"id":"aaa","state":"INACTIVE","url":"https://t/a.png","alias":"Migrator"},
            {"id":"bbb","state":"ACTIVE","url":"https://t/b.png","alias":"Vanilla"}]}"#;
        let (items, active) = parse_profile_capes(json).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(active.as_deref(), Some("bbb"));
        assert_eq!(items[0].name, "迁移者披风");
        assert_eq!(items[0].alias.as_deref(), Some("Migrator"));
        assert_eq!(items[1].state, "ACTIVE");
        assert_eq!(items[1].url, "https://t/b.png");
    }

    #[test]
    fn parses_profile_without_capes() {
        let (items, active) = parse_profile_capes(r#"{"id":"uuid","name":"Alex"}"#).unwrap();
        assert!(items.is_empty());
        assert!(active.is_none());
    }

    #[test]
    fn cape_without_alias_falls_back_to_id() {
        let json = r#"{"capes":[{"id":"deadbeef","state":"INACTIVE","url":"https://t/x.png"}]}"#;
        let (items, _) = parse_profile_capes(json).unwrap();
        assert_eq!(items[0].name, "deadbeef");
        assert!(items[0].alias.is_none());
        assert_eq!(cape_display_name(Some("Cherry Blossom"), "x"), "樱花披风");
        assert_eq!(cape_display_name(Some("Unknown Cape"), "x"), "Unknown Cape");
        assert_eq!(cape_display_name(Some("   "), "x"), "x");
    }

    #[test]
    fn offline_account_is_unsupported_with_chinese_reason() {
        let value = unsupported_reason(Some(&account("offline", "")));
        assert_eq!(value, Some("离线账号不支持披风，登录微软账号后可用"));
    }

    #[test]
    fn thirdparty_account_is_unsupported() {
        assert_eq!(
            unsupported_reason(Some(&account("thirdparty", "tok"))),
            Some("第三方（皮肤站）账号不支持披风")
        );
        assert_eq!(
            unsupported_reason(Some(&account("yggdrasil", "tok"))),
            Some("第三方（皮肤站）账号不支持披风")
        );
    }

    #[test]
    fn no_account_and_empty_token_ask_for_login() {
        assert_eq!(unsupported_reason(None), Some("请先登录微软账号"));
        assert_eq!(
            unsupported_reason(Some(&account("ms", "  "))),
            Some("请先登录微软账号")
        );
        assert_eq!(unsupported_reason(Some(&account("ms", "token"))), None);
    }

    #[test]
    fn cape_body_omits_id_for_none() {
        assert_eq!(cape_body(None), serde_json::json!({}));
        assert_eq!(cape_body(Some("   ")), serde_json::json!({}));
        assert_eq!(
            cape_body(Some("abc")),
            serde_json::json!({"capeId": "abc"})
        );
    }

    #[test]
    fn cape_errors_map_to_chinese() {
        assert_eq!(map_cape_error(400), "该披风不属于此账号");
        assert_eq!(map_cape_error(401), "登录已过期，请重新登录");
        assert_eq!(map_cape_error(403), "登录已过期，请重新登录");
        assert_eq!(map_cape_error(429), "操作太频繁，请稍后再试");
        assert!(map_cape_error(500).contains("500"));
    }

    #[test]
    fn upload_errors_map_to_chinese() {
        assert!(map_upload_error(400).contains("64×64"));
        assert_eq!(map_upload_error(401), "登录已过期，请重新登录");
        assert_eq!(map_upload_error(429), "上传太频繁，请稍后再试");
        assert!(map_upload_error(502).contains("502"));
    }

    #[test]
    fn upload_accepts_64x64_unchanged() {
        let prepared = prepare_skin(&png(64, 64), "slim").unwrap();
        assert_eq!((prepared.width, prepared.height), (64, 64));
        assert!(!prepared.padded);
        assert_eq!(prepared.variant, "slim");
    }

    #[test]
    fn upload_pads_64x32_to_64x64() {
        let prepared = prepare_skin(&png(64, 32), "classic").unwrap();
        assert!(prepared.padded);
        assert_eq!(crate::png::png_size(&prepared.bytes), Some((64, 64)));
    }

    #[test]
    fn upload_rejects_bad_size_and_non_png() {
        let error = prepare_skin(&png(32, 32), "classic").unwrap_err().to_string();
        assert!(error.contains("64×64 或 64×32"), "{error}");
        let error = prepare_skin(b"not a png", "classic").unwrap_err().to_string();
        assert!(error.contains("PNG"), "{error}");
    }

    #[test]
    fn upload_rejects_oversized_file() {
        let mut bytes = png(64, 64);
        bytes.resize(SKIN_UPLOAD_LIMIT + 1, 0);
        let error = prepare_skin(&bytes, "classic").unwrap_err().to_string();
        assert!(error.contains("1MB"), "{error}");
    }

    #[test]
    fn variant_whitelist_is_enforced() {
        assert_eq!(validate_variant("classic").unwrap(), "classic");
        assert_eq!(validate_variant("Slim").unwrap(), "slim");
        assert_eq!(validate_variant("default").unwrap(), "classic");
        assert!(validate_variant("fat").is_err());
        assert_eq!(multipart_fields("slim")[0], ("variant", "slim".to_string()));
        assert_eq!(multipart_fields("slim")[1], ("file", "skin.png".to_string()));
        assert_eq!(SKIN_FIELD_FILE, "file");
    }

// ------------------------------------------------ 账号外观（微软登录修复）

    fn profile_json(skin: bool, cape: bool) -> String {
        let skins = if skin {
            r#""skins":[{"id":"s1","state":"ACTIVE","url":"https://t/skin.png","variant":"SLIM"}]"#
        } else {
            r#""skins":[]"#
        };
        let capes = if cape {
            r#""capes":[{"id":"c1","state":"INACTIVE","url":"https://t/c1.png"},{"id":"c2","state":"ACTIVE","url":"https://t/c2.png"}]"#
        } else {
            r#""capes":[]"#
        };
        format!(
            r#"{{"id":"069a79f444e94726a5befca90e38aaf5","name":"Notch",{skins},{capes}}}"#
        )
    }

    #[test]
    fn profile_with_skin_and_active_cape() {
        let profile = parse_account_profile(&profile_json(true, true)).unwrap();
        assert_eq!(profile.kind, "ms");
        assert_eq!(profile.name, "Notch");
        assert_eq!(profile.uuid, "069a79f444e94726a5befca90e38aaf5");
        assert_eq!(profile.skin_url, "https://t/skin.png");
        assert_eq!(profile.variant, "slim");
        // 只有 ACTIVE 的那张算当前披风
        assert_eq!(profile.cape_url, "https://t/c2.png");
        // 前端按空串判空，这里给的是字符串
        assert!(!profile.skin_url.is_empty());
    }

    #[test]
    fn profile_without_skin_or_cape() {
        let profile = parse_account_profile(&profile_json(false, false)).unwrap();
        // 没有皮肤/披风时是空串，variant 回落 classic
        assert_eq!(profile.skin_url, "");
        assert_eq!(profile.cape_url, "");
        assert_eq!(profile.variant, "classic");
        assert_eq!(profile.name, "Notch");
    }

    #[test]
    fn profile_without_skin_but_with_cape() {
        let profile = parse_account_profile(&profile_json(false, true)).unwrap();
        assert_eq!(profile.skin_url, "");
        assert_eq!(profile.cape_url, "https://t/c2.png");
        assert_eq!(profile.variant, "classic");
    }

    #[test]
    fn profile_error_body_is_readable() {
        let error = parse_account_profile(
            r#"{"error":"NOT_FOUND","errorMessage":"The server has not found anything"}"#,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("读取 Minecraft 档案失败"), "{error}");
    }

    #[test]
    fn skin_save_rejects_non_png_and_writes_png() {
        let paths = crate::paths::Paths::from_root(std::env::temp_dir().join(format!(
            "gugu-skin-save-{}",
            crate::util::unique_suffix()
        )));
        assert!(save_skin_bytes(&paths, "Notch", b"not a png").is_err());
        let png = crate::png::encode_png(64, 64, &vec![1u8; 64 * 64 * 4]).unwrap();
        let file = save_skin_bytes(&paths, "Notch", &png).unwrap();
        assert!(file.is_file());
        assert_eq!(file.file_name().and_then(|name| name.to_str()), Some("Notch.png"));
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn cape_cache_invalidation_removes_png_only() {
        let paths = crate::paths::Paths::from_root(std::env::temp_dir().join(format!(
            "gugu-cape-cache-{}",
            crate::util::unique_suffix()
        )));
        let dir = skins::capes_dir(&paths);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("migrator.png"), b"x").unwrap();
        std::fs::write(dir.join("keep.txt"), b"x").unwrap();
        assert_eq!(invalidate_cape_cache(&paths), 1);
        assert!(!dir.join("migrator.png").exists());
        assert!(dir.join("keep.txt").exists());
        assert_eq!(invalidate_cape_cache(&paths), 0);
        let _ = std::fs::remove_dir_all(&paths.root);
    }

#[test]
    fn cape_request_uses_put_with_json_body() {
        let client = reqwest::Client::new();
        let request = cape_request(&client, "token-abc", Some("cape-1"))
            .build()
            .unwrap();
        assert_eq!(request.method(), reqwest::Method::PUT);
        assert_eq!(request.url().as_str(), CAPE_ACTIVE_URL);
        assert_eq!(CAPE_ACTIVE_METHOD, "PUT");
        assert_eq!(
            request.headers().get(reqwest::header::AUTHORIZATION).map(|v| v.to_str().unwrap()),
            Some("Bearer token-abc")
        );
        let body = request.body().and_then(|body| body.as_bytes()).unwrap();
        let text = std::str::from_utf8(body).unwrap();
        assert!(text.contains("\"capeId\":\"cape-1\""), "{text}");
        // 取消披风：不带 capeId
        let cleared = cape_request(&client, "token-abc", None).build().unwrap();
        let body = cleared.body().and_then(|body| body.as_bytes()).unwrap();
        assert_eq!(std::str::from_utf8(body).unwrap(), "{}");
    }

    #[test]
    fn cape_405_and_400_map_to_readable_chinese() {
        assert!(map_cape_error(405).contains("405"), "405 要有可读文案");
        assert_eq!(map_cape_error(400), "该披风不属于此账号");
        assert_eq!(map_cape_error(401), "登录已过期，请重新登录");
    }

    #[test]
    fn skin_base64_accepts_data_url_and_plain() {
        let big = png(64, 64);
        let plain = util::base64_encode(&big);
        let with_prefix = format!("data:image/png;base64,{plain}");
        assert_eq!(decode_skin_base64(&plain).unwrap(), big);
        assert_eq!(decode_skin_base64(&with_prefix).unwrap(), big);
        assert_eq!(strip_data_url_prefix(&with_prefix), plain.as_str());
        assert_eq!(strip_data_url_prefix(&plain), plain.as_str());
        // 走同一套校验：尺寸不对要报错
        let small = util::base64_encode(&png(32, 32));
        let error = prepare_skin(&decode_skin_base64(&small).unwrap(), "classic")
            .unwrap_err()
            .to_string();
        assert!(error.contains("64×64 或 64×32"), "{error}");
    }

    #[test]
    fn skin_source_prefers_base64_over_path() {
        let paths = crate::paths::Paths::from_root(std::env::temp_dir().join(format!(
            "gugu-skin-source-{}",
            util::unique_suffix()
        )));
        std::fs::create_dir_all(&paths.root).unwrap();
        let file = paths.root.join("on-disk.png");
        std::fs::write(&file, png(32, 32)).unwrap();
        let base64 = util::base64_encode(&png(64, 64));
        // 两者都给：以 base64 为准（磁盘那张是 32×32，会被拒）
        let bytes = read_skin_source(Some(file.to_str().unwrap()), Some(&base64)).unwrap();
        assert_eq!(crate::png::png_size(&bytes), Some((64, 64)));
        // 只给路径
        let bytes = read_skin_source(Some(file.to_str().unwrap()), None).unwrap();
        assert_eq!(crate::png::png_size(&bytes), Some((32, 32)));
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn skin_source_errors_are_readable() {
        let error = read_skin_source(None, None).unwrap_err().to_string();
        assert!(error.contains("缺少皮肤数据"), "{error}");
        let error = read_skin_source(None, Some("data:image/png;base64,@@@不是base64"))
            .unwrap_err()
            .to_string();
        assert!(error.contains("不是有效的 PNG"), "{error}");
        let error = read_skin_source(Some("D:\\nope\\missing.png"), None)
            .unwrap_err()
            .to_string();
        assert!(error.contains("皮肤文件不存在"), "{error}");
    }

    #[test]
    fn multipart_body_carries_both_fields() {
        let body = build_multipart("XYZ", "slim", b"PNGDATA");
        let text = String::from_utf8_lossy(&body);
        assert!(text.starts_with("--XYZ\r\n"), "{text}");
        assert!(text.contains("name=\"variant\"\r\n\r\nslim\r\n"), "{text}");
        assert!(
            text.contains("name=\"file\"; filename=\"skin.png\""),
            "{text}"
        );
        assert!(text.contains("Content-Type: image/png"));
        assert!(text.ends_with("--XYZ--\r\n"));
        assert!(body.windows(7).any(|window| window == b"PNGDATA"));
    }

    #[test]
    fn cape_sync_flag_reports_success_and_failure() {
        assert!(cape_sync_flag(Some("cape"), Ok(())));
        assert!(!cape_sync_flag(None, Ok(())));
        assert!(!cape_sync_flag(Some("cape"), Err(AppError::Net("boom".to_string()))));
    }
}
