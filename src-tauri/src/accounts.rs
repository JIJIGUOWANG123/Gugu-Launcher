//! 账号：文件存储（.mc/accounts.json）+ 离线账号 + 微软设备码登录状态。
//!
//! 令牌只落盘，日志与错误信息里一律打码。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{AppError, Result};
use crate::paths::Paths;
use crate::util;

/// 离线 UUID：Java 的 UUID.nameUUIDFromBytes("OfflinePlayer:<name>")。
pub fn offline_uuid(name: &str) -> String {
    let mut hasher_input = String::from("OfflinePlayer:");
    hasher_input.push_str(name);
    let mut bytes = util::md5(hasher_input.as_bytes());
    bytes[6] = (bytes[6] & 0x0f) | 0x30;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    util::hex(&bytes)
}

/// 32 位十六进制 → 带连字符的标准 UUID 写法。
pub fn dashed_uuid(uuid: &str) -> String {
    let clean: String = uuid.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if clean.len() != 32 {
        return uuid.to_string();
    }
    format!(
        "{}-{}-{}-{}-{}",
        &clean[0..8],
        &clean[8..12],
        &clean[12..16],
        &clean[16..20],
        &clean[20..32]
    )
}

/// accounts_list() 的条目。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountItem {
    pub id: i64,
    pub name: String,
    /// 前端契约字段：microsoft / offline
    #[serde(rename = "type")]
    pub account_type: String,
    /// 内部 kind：ms / offline（前端也认这个字段）
    pub kind: String,
    pub uuid: String,
    /// 是否当前账号（前端字段名是 current）
    #[serde(rename = "current")]
    pub is_current: bool,
    /// 账号皮肤：优先本地已下载的皮肤（data URL），其次正版贴图地址
    pub skin_data_url: Option<String>,
    /// 正版贴图地址（只给后端用，不进 JSON）
    #[serde(skip)]
    pub skin_url: Option<String>,
    /// 第三方站点根（离线/微软账号为空）
    pub server: Option<String>,
}

impl AccountItem {
    /// kind → 前端 type：只有 ms 算微软账号，其余都是离线。
    pub fn type_of(kind: &str) -> &'static str {
        match kind {
            "ms" => "microsoft",
            "thirdparty" => "thirdparty",
            _ => "offline",
        }
    }
}

/// 落盘的账号（含令牌，绝不直接序列化给前端）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredAccount {
    pub id: i64,
    pub name: String,
    /// ms / offline
    pub kind: String,
    pub uuid: String,
    #[serde(default)]
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: String,
    #[serde(default)]
    pub skin_url: String,
    /// 登录后下载到 .mc/skins 的皮肤文件绝对路径（空串表示没有）
    #[serde(default)]
    pub local_skin: String,
    /// 第三方（Yggdrasil）站点根地址；官方/离线账号为空串
    #[serde(default)]
    pub server: String,
    pub created_at: String,
}

impl StoredAccount {
    pub fn to_item(&self, current: Option<i64>) -> AccountItem {
        let local = std::path::Path::new(self.local_skin.trim());
        let skin_data_url = local
            .is_file()
            .then(|| crate::skins::png_data_url(local, crate::skins::MAX_DATA_URL_BYTES))
            .flatten()
            .or_else(|| {
                if self.skin_url.trim().is_empty() {
                    None
                } else {
                    Some(self.skin_url.clone())
                }
            });
        AccountItem {
            id: self.id,
            name: self.name.clone(),
            account_type: AccountItem::type_of(&self.kind).to_string(),
            kind: self.kind.clone(),
            uuid: self.uuid.clone(),
            is_current: current == Some(self.id),
            skin_data_url,
            skin_url: if self.skin_url.trim().is_empty() {
                None
            } else {
                Some(self.skin_url.clone())
            },
            server: if self.server.trim().is_empty() {
                None
            } else {
                Some(self.server.trim().to_string())
            },
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountsFile {
    #[serde(default)]
    pub accounts: Vec<StoredAccount>,
    #[serde(default)]
    pub current: Option<i64>,
    #[serde(default)]
    pub next_id: i64,
}

pub fn accounts_path(paths: &Paths) -> PathBuf {
    paths.root.join("accounts.json")
}

/// 读账号文件；不存在或损坏都返回空表（不报错，避免挡住启动）。
pub fn load(paths: &Paths) -> AccountsFile {
    let path = accounts_path(paths);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return AccountsFile::default();
    };
    match serde_json::from_str::<AccountsFile>(&text) {
        Ok(mut file) => {
            if file.next_id <= 0 {
                file.next_id = file
                    .accounts
                    .iter()
                    .map(|account| account.id)
                    .max()
                    .unwrap_or(0)
                    + 1;
            }
            file
        }
        Err(err) => {
            tracing::warn!(%err, "accounts.json 解析失败，按空账号表处理");
            AccountsFile::default()
        }
    }
}

pub fn save(paths: &Paths, file: &AccountsFile) -> Result<()> {
    let path = accounts_path(paths);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(file)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

/// 名字校验：3..=16 个字符（与正版一致），不允许带颜色等特殊前缀。
pub fn validate_offline_name(name: &str) -> Result<String> {
    let trimmed = name.trim();
    let count = trimmed.chars().count();
    if !(3..=16).contains(&count) {
        return Err(AppError::Invalid(
            "离线用户名必须是 3 到 16 个字符".to_string(),
        ));
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Err(AppError::Invalid(
            "离线用户名只能包含字母、数字与下划线".to_string(),
        ));
    }
    Ok(trimmed.to_string())
}

pub fn list(paths: &Paths) -> Vec<AccountItem> {
    let file = load(paths);
    file.accounts
        .iter()
        .map(|account| account.to_item(file.current))
        .collect()
}

/// 登录后把这条账号的外观信息写回（皮肤地址 / 本地皮肤文件）。
pub fn set_appearance(
    paths: &Paths,
    id: i64,
    skin_url: Option<&str>,
    local_skin: Option<&str>,
) -> Result<()> {
    let mut file = load(paths);
    let account = file
        .accounts
        .iter_mut()
        .find(|account| account.id == id)
        .ok_or_else(|| AppError::Invalid(format!("账号不存在：{id}")))?;
    if let Some(url) = skin_url {
        account.skin_url = url.to_string();
    }
    if let Some(local) = local_skin {
        account.local_skin = local.to_string();
    }
    account.kind = "ms".to_string();
    save(paths, &file)
}

/// 当前账号（启动时用）：返回 (用户名, uuid, access_token)。
pub fn current_credentials(paths: &Paths) -> Option<(String, String, String)> {
    let file = load(paths);
    let current = file.current?;
    let account = file
        .accounts
        .iter()
        .find(|account| account.id == current)?;
    // uuid 缺失（档案没读到）时按离线 uuid 兜底，启动参数里不会出现空 uuid
    let uuid = if account.uuid.trim().is_empty() {
        offline_uuid(&account.name)
    } else {
        account.uuid.clone()
    };
    Some((account.name.clone(), uuid, account.access_token.clone()))
}

pub fn add_offline(paths: &Paths, name: &str) -> Result<AccountItem> {
    let name = validate_offline_name(name)?;
    let mut file = load(paths);
    if file
        .accounts
        .iter()
        .any(|account| account.name.eq_ignore_ascii_case(&name))
    {
        return Err(AppError::Invalid(format!("账号已存在：{name}")));
    }
    let id = file.next_id.max(1);
    file.next_id = id + 1;
    let account = StoredAccount {
        id,
        name: name.clone(),
        kind: "offline".to_string(),
        uuid: offline_uuid(&name),
        access_token: "0".to_string(),
        refresh_token: String::new(),
        skin_url: String::new(),
        local_skin: String::new(),
        server: String::new(),
        created_at: util::iso8601(util::now_unix()),
    };
    file.accounts.push(account);
    if file.current.is_none() {
        file.current = Some(id);
    }
    save(paths, &file)?;
    Ok(load(paths)
        .accounts
        .iter()
        .find(|item| item.id == id)
        .map(|item| item.to_item(file.current))
        .unwrap_or_else(|| AccountItem {
            id,
            name,
            account_type: AccountItem::type_of("offline").to_string(),
            kind: "offline".to_string(),
            uuid: String::new(),
            is_current: true,
            skin_data_url: None,
            skin_url: None,
            server: None,
        }))
}

/// 写入（或更新同名）微软账号，成功后设为当前账号。
pub fn upsert_ms(
    paths: &Paths,
    name: &str,
    uuid: &str,
    access_token: &str,
    refresh_token: &str,
) -> Result<AccountItem> {
    let mut file = load(paths);
    let uuid_known = !uuid.trim().is_empty();
    // 先按 uuid 认账号；uuid 缺失（档案读不出来）时退化成按名字找同名微软账号
    let id = match file
        .accounts
        .iter()
        .position(|account| uuid_known && account.uuid == uuid)
        .or_else(|| {
            file.accounts.iter().position(|account| {
                account.kind == "ms" && account.name.eq_ignore_ascii_case(name)
            })
        })
    {
        Some(index) => file.accounts[index].id,
        None => {
            let id = file.next_id.max(1);
            file.next_id = id + 1;
            file.accounts.push(StoredAccount {
                id,
                name: name.to_string(),
                kind: "ms".to_string(),
                uuid: uuid.to_string(),
                access_token: String::new(),
                refresh_token: String::new(),
                skin_url: String::new(),
                local_skin: String::new(),
                server: String::new(),
                created_at: util::iso8601(util::now_unix()),
            });
            id
        }
    };
    if let Some(account) = file.accounts.iter_mut().find(|account| account.id == id) {
        account.name = name.to_string();
        account.access_token = access_token.to_string();
        account.refresh_token = refresh_token.to_string();
        account.kind = "ms".to_string();
        if uuid_known {
            account.uuid = uuid.to_string();
        }
    }
    file.current = Some(id);
    save(paths, &file)?;
    tracing::info!(account = %name, token = %util::mask_secret(access_token), "微软账号已保存");
    Ok(list(paths)
        .into_iter()
        .find(|item| item.id == id)
        .unwrap_or_else(|| AccountItem {
            id,
            name: name.to_string(),
            account_type: AccountItem::type_of("ms").to_string(),
            kind: "ms".to_string(),
            uuid: uuid.to_string(),
            is_current: true,
            skin_data_url: None,
            skin_url: None,
            server: None,
        }))
}

/// 写入（或更新）第三方（Yggdrasil）账号，成功后设为当前账号。
pub fn upsert_thirdparty(
    paths: &Paths,
    server: &str,
    session: &crate::yggdrasil::YggdrasilSession,
) -> Result<AccountItem> {
    if session.profile_id.trim().is_empty() {
        return Err(AppError::Invalid(
            "这个账号没有角色 id，无法保存（请先在皮肤站创建角色）".to_string(),
        ));
    }
    let name = if session.profile_name.trim().is_empty() {
        session.profile_id.clone()
    } else {
        session.profile_name.clone()
    };
    let mut file = load(paths);
    let id = match file
        .accounts
        .iter()
        .position(|account| account.uuid == session.profile_id && account.kind == "thirdparty")
    {
        Some(index) => file.accounts[index].id,
        None => {
            let id = file.next_id.max(1);
            file.next_id = id + 1;
            file.accounts.push(StoredAccount {
                id,
                name: name.clone(),
                kind: "thirdparty".to_string(),
                uuid: session.profile_id.clone(),
                access_token: String::new(),
                refresh_token: String::new(),
                skin_url: String::new(),
                local_skin: String::new(),
                server: server.to_string(),
                created_at: util::iso8601(util::now_unix()),
            });
            id
        }
    };
    if let Some(account) = file.accounts.iter_mut().find(|account| account.id == id) {
        account.name = name;
        account.kind = "thirdparty".to_string();
        account.uuid = session.profile_id.clone();
        account.access_token = session.access_token.clone();
        account.refresh_token = session.client_token.clone();
        account.server = server.trim().to_string();
    }
    file.current = Some(id);
    save(paths, &file)?;
    tracing::info!(
        server = %server,
        account = %session.profile_name,
        token = %util::mask_secret(&session.access_token),
        "第三方账号已保存"
    );
    Ok(list(paths)
        .into_iter()
        .find(|item| item.id == id)
        .unwrap_or_else(|| AccountItem {
            id,
            name: session.profile_name.clone(),
            account_type: AccountItem::type_of("thirdparty").to_string(),
            kind: "thirdparty".to_string(),
            uuid: session.profile_id.clone(),
            is_current: true,
            skin_data_url: None,
            skin_url: None,
            server: Some(server.trim().to_string()),
        }))
}

/// 当前账号（含令牌，给启动链路用）。
pub fn current_stored(paths: &Paths) -> Option<StoredAccount> {
    let file = load(paths);
    let current = file.current?;
    file.accounts.into_iter().find(|account| account.id == current)
}

/// 按 id 取账号；id=None 时取当前账号。
pub fn stored_by_id(paths: &Paths, id: Option<i64>) -> Option<StoredAccount> {
    let file = load(paths);
    match id {
        Some(id) => file.accounts.into_iter().find(|account| account.id == id),
        None => {
            let current = file.current?;
            file.accounts.into_iter().find(|account| account.id == current)
        }
    }
}

/// 刷新后回写令牌：只动令牌字段，不动当前账号。
pub fn update_tokens(
    paths: &Paths,
    id: i64,
    access_token: &str,
    refresh_token: &str,
) -> Result<()> {
    let mut file = load(paths);
    let account = file
        .accounts
        .iter_mut()
        .find(|account| account.id == id)
        .ok_or_else(|| AppError::Invalid(format!("账号不存在：{id}")))?;
    account.access_token = access_token.to_string();
    if !refresh_token.trim().is_empty() {
        account.refresh_token = refresh_token.to_string();
    }
    save(paths, &file)?;
    tracing::info!(id, token = %util::mask_secret(access_token), "微软令牌已更新");
    Ok(())
}

pub fn apply(paths: &Paths, id: i64) -> Result<()> {
    let mut file = load(paths);
    if !file.accounts.iter().any(|account| account.id == id) {
        return Err(AppError::Invalid(format!("账号不存在：{id}")));
    }
    file.current = Some(id);
    save(paths, &file)
}

pub fn delete(paths: &Paths, id: i64) -> Result<()> {
    let mut file = load(paths);
    let before = file.accounts.len();
    // 登出要干净：连同登录时下载的皮肤文件一起删掉
    if let Some(account) = file.accounts.iter().find(|account| account.id == id) {
        let local = account.local_skin.trim().to_string();
        if !local.is_empty() {
            if let Err(err) = std::fs::remove_file(&local) {
                if err.kind() != std::io::ErrorKind::NotFound {
                    tracing::warn!(%err, path = %local, "删除账号皮肤文件失败");
                }
            }
        }
    }
    file.accounts.retain(|account| account.id != id);
    if file.accounts.len() == before {
        return Err(AppError::Invalid(format!("账号不存在：{id}")));
    }
    if file.current == Some(id) {
        file.current = file.accounts.first().map(|account| account.id);
    }
    save(paths, &file)
}

/// 供示例/测试用：把账号文件指到临时目录。
pub fn ensure_dir(paths: &Paths) -> Result<()> {
    let path = accounts_path(paths);
    if let Some(parent) = Path::new(&path).parent() {
        std::fs::create_dir_all(parent)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_paths(tag: &str) -> Paths {
        let root = std::env::temp_dir().join(format!(
            "gugu-acc-{tag}-{}-{}",
            std::process::id(),
            util::unique_suffix()
        ));
        let paths = Paths::from_root(&root);
        paths.ensure().unwrap();
        paths
    }

    #[test]
    fn upsert_thirdparty_persists_refreshed_token_and_overwrites_old() {
        let paths = temp_paths("thirdparty-refresh");
        let session = crate::yggdrasil::YggdrasilSession {
            access_token: "old-token".to_string(),
            client_token: "client-1".to_string(),
            profile_id: "964b99c006e24f67bbc78cc74e71a5d4".to_string(),
            profile_name: "JiJi_GuoWang".to_string(),
            available: Vec::new(),
        };
        let server = "https://littleskin.cn/api/yggdrasil";
        let item = upsert_thirdparty(&paths, server, &session).unwrap();
        assert_eq!(item.kind, "thirdparty");
        assert_eq!(item.account_type, "thirdparty");
        assert_eq!(item.server.as_deref(), Some(server));
        assert_eq!(stored_by_id(&paths, None).unwrap().access_token, "old-token");

        let refreshed = crate::yggdrasil::YggdrasilSession {
            access_token: "new-token".to_string(),
            client_token: "client-2".to_string(),
            profile_id: session.profile_id.clone(),
            profile_name: session.profile_name.clone(),
            available: Vec::new(),
        };
        upsert_thirdparty(&paths, server, &refreshed).unwrap();
        let stored = stored_by_id(&paths, None).unwrap();
        assert_eq!(stored.access_token, "new-token", "启动只认最新 token");
        assert_eq!(stored.refresh_token, "client-2");
        assert_eq!(list(&paths).len(), 1, "同一个角色只有一条账号记录");
        let text = std::fs::read_to_string(accounts_path(&paths)).unwrap();
        assert!(!text.contains("password"), "accounts.json 不应有密码字段");
        let _ = std::fs::remove_dir_all(&paths.root);
    }

#[test]
    fn ms_account_serializes_as_microsoft_even_without_uuid() {
        let paths = temp_paths("ms-kind");
        // 档案读不出来时 uuid 为空：仍然必须是微软账号
        let item = upsert_ms(&paths, "微软账号", "", "token-1", "refresh-1").unwrap();
        assert_eq!(item.kind, "ms");
        assert_eq!(item.account_type, "microsoft");
        assert_eq!(item.uuid, "");
        // 再登录一次（这次有 uuid）：按名字复用同一条，不新增
        let same = upsert_ms(
            &paths,
            "微软账号",
            "0123456789abcdef0123456789abcdef",
            "token-2",
            "refresh-2",
        )
        .unwrap();
        assert_eq!(same.id, item.id);
        assert_eq!(same.uuid, "0123456789abcdef0123456789abcdef");
        assert_eq!(list(&paths).len(), 1);
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn account_item_json_follows_the_frontend_contract() {
        let paths = temp_paths("account-json");
        let offline = add_offline(&paths, "Steve").unwrap();
        let value = serde_json::to_value(&offline).unwrap();
        assert_eq!(value["type"], "offline");
        assert_eq!(value["kind"], "offline");
        assert_eq!(value["current"], true);
        assert!(value.get("skinDataUrl").is_some());
        assert!(value.get("isCurrent").is_none(), "前端只认 current");
        assert!(value.get("accessToken").is_none(), "令牌不能出后端");
        let ms = upsert_ms(&paths, "Notch", "0123456789abcdef0123456789abcdef", "t", "r").unwrap();
        let value = serde_json::to_value(&ms).unwrap();
        assert_eq!(value["type"], "microsoft");
        assert_eq!(value["kind"], "ms");
        assert_eq!(value["current"], true);
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn deleting_account_removes_downloaded_skin_file() {
        let paths = temp_paths("account-skin-clean");
        let item = upsert_ms(&paths, "Notch", "0123456789abcdef0123456789abcdef", "t", "r").unwrap();
        std::fs::create_dir_all(&paths.skins).unwrap();
        let skin = paths.skins.join("Notch.png");
        std::fs::write(
            &skin,
            crate::png::encode_png(64, 64, &vec![0u8; 64 * 64 * 4]).unwrap(),
        )
        .unwrap();
        set_appearance(&paths, item.id, Some("https://t/skin.png"), skin.to_str()).unwrap();
        let stored = list(&paths)
            .into_iter()
            .find(|entry| entry.id == item.id)
            .unwrap();
        assert!(stored
            .skin_data_url
            .as_deref()
            .map(|url| url.starts_with("data:image/png;base64,"))
            .unwrap_or(false));
        delete(&paths, item.id).unwrap();
        assert!(!skin.exists(), "登出后皮肤文件要一起删掉");
        assert!(list(&paths).is_empty());
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn offline_uuid_follows_java_rule() {
        let uuid = offline_uuid("Steve");
        assert_eq!(uuid.len(), 32);
        // 版本位必须是 3（nameUUIDFromBytes 是 v3）
        assert_eq!(uuid.as_bytes()[12] as char, '3');
        // 变体位必须是 8/9/a/b
        assert!("89ab".contains(uuid.as_bytes()[16] as char));
        assert_eq!(uuid, offline_uuid("Steve"));
        assert_ne!(uuid, offline_uuid("Alex"));
        // 固定输入固定输出（与 Java 端一致）
        assert_eq!(
            uuid,
            util::hex(&{
                let mut bytes = util::md5(b"OfflinePlayer:Steve");
                bytes[6] = (bytes[6] & 0x0f) | 0x30;
                bytes[8] = (bytes[8] & 0x3f) | 0x80;
                bytes
            })
        );
        assert_eq!(
            dashed_uuid(&uuid),
            format!(
                "{}-{}-{}-{}-{}",
                &uuid[0..8],
                &uuid[8..12],
                &uuid[12..16],
                &uuid[16..20],
                &uuid[20..32]
            )
        );
    }

    #[test]
    fn offline_name_validation() {
        assert_eq!(validate_offline_name("  Steve ").unwrap(), "Steve");
        assert!(validate_offline_name("ab").is_err());
        assert!(validate_offline_name("12345678901234567").is_err());
        assert!(validate_offline_name("bad name").is_err());
        assert!(validate_offline_name("中文名").is_err());
        assert!(validate_offline_name("ok_name").is_ok());
    }

    #[test]
    fn add_apply_delete_roundtrip() {
        let paths = temp_paths("crud");
        let first = add_offline(&paths, "Steve").unwrap();
        assert!(first.is_current, "第一个账号自动设为当前");
        let second = add_offline(&paths, "Alex").unwrap();
        assert!(!second.is_current);
        assert!(add_offline(&paths, "steve").is_err(), "重名（忽略大小写）要拒绝");

        let items = list(&paths);
        assert_eq!(items.len(), 2);
        apply(&paths, second.id).unwrap();
        assert!(list(&paths).iter().find(|item| item.id == second.id).unwrap().is_current);

        delete(&paths, second.id).unwrap();
        let items = list(&paths);
        assert_eq!(items.len(), 1);
        assert!(items[0].is_current, "删掉当前账号后要自动接管");
        assert!(delete(&paths, 999).is_err());
        assert_eq!(current_credentials(&paths).unwrap().0, "Steve");
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn ms_account_upsert_keeps_single_row() {
        let paths = temp_paths("ms");
        let uuid = offline_uuid("MsPlayer");
        let first = upsert_ms(&paths, "MsPlayer", &uuid, "token-1", "refresh-1").unwrap();
        assert_eq!(first.kind, "ms");
        assert!(first.is_current);
        let second = upsert_ms(&paths, "MsPlayer", &uuid, "token-2", "refresh-2").unwrap();
        assert_eq!(second.id, first.id, "同一 uuid 只更新不新增");
        assert_eq!(list(&paths).len(), 1);

        // 令牌只落盘，不进 AccountItem（前端拿不到）
        let items = serde_json::to_value(list(&paths)).unwrap();
        assert!(items[0].get("accessToken").is_none());
        let stored = std::fs::read_to_string(accounts_path(&paths)).unwrap();
        assert!(stored.contains("token-2"));
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn token_masking_hides_value() {
        let masked = util::mask_secret("eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.payload.sig");
        assert_eq!(masked, "***");
        let redacted = util::redact_secret("Bearer eyJabc.def.ghi", "eyJabc.def.ghi");
        assert!(!redacted.contains("eyJabc.def.ghi"));
    }

    #[test]
    fn broken_file_does_not_panic() {
        let paths = temp_paths("broken");
        std::fs::write(accounts_path(&paths), "{ not json").unwrap();
        assert!(list(&paths).is_empty());
        assert!(current_credentials(&paths).is_none());
        ensure_dir(&paths).unwrap();
        let _ = std::fs::remove_dir_all(&paths.root);
    }
}