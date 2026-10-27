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
    pub kind: String,
    pub uuid: String,
    pub is_current: bool,
    pub skin_url: Option<String>,
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
    pub created_at: String,
}

impl StoredAccount {
    pub fn to_item(&self, current: Option<i64>) -> AccountItem {
        AccountItem {
            id: self.id,
            name: self.name.clone(),
            kind: self.kind.clone(),
            uuid: self.uuid.clone(),
            is_current: current == Some(self.id),
            skin_url: if self.skin_url.trim().is_empty() {
                None
            } else {
                Some(self.skin_url.clone())
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

/// 当前账号（启动时用）：返回 (用户名, uuid, access_token)。
pub fn current_credentials(paths: &Paths) -> Option<(String, String, String)> {
    let file = load(paths);
    let current = file.current?;
    let account = file
        .accounts
        .iter()
        .find(|account| account.id == current)?;
    Some((
        account.name.clone(),
        account.uuid.clone(),
        account.access_token.clone(),
    ))
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
        .unwrap_or(AccountItem {
            id,
            name,
            kind: "offline".to_string(),
            uuid: String::new(),
            is_current: true,
            skin_url: None,
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
    let id = match file.accounts.iter().position(|account| account.uuid == uuid) {
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
        account.uuid = uuid.to_string();
    }
    file.current = Some(id);
    save(paths, &file)?;
    tracing::info!(account = %name, token = %util::mask_secret(access_token), "微软账号已保存");
    Ok(list(paths)
        .into_iter()
        .find(|item| item.id == id)
        .unwrap_or(AccountItem {
            id,
            name: name.to_string(),
            kind: "ms".to_string(),
            uuid: uuid.to_string(),
            is_current: true,
            skin_url: None,
        }))
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
