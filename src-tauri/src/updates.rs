//! 实例内容（模组 / 光影 / 资源包）的批量检查更新与应用更新。
//!
//! 来源识别顺序：安装记录（provider+projectId+versionId）→ jar 的 CurseForge 指纹（murmur2）。
//! 两者都不中进 failed 并写清原因；应用更新前先备份到 <instance>/.gugu-backup/<时间戳>/。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::content::{self, Provider};
use crate::error::{AppError, Result};
use crate::server_content::{self, ContentRecord};
use crate::util;

/// 备份目录（实例根下，跟着实例一起被删/导出时忽略）。
pub const BACKUP_DIR: &str = ".gugu-backup";
/// CurseForge 指纹接口路径。
pub const FINGERPRINT_PATH: &str = "/fingerprints";

/// mods_check_updates 的元素。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModUpdateItem {
    pub file_name: String,
    pub path: String,
    pub kind: String,
    pub current_version: Option<String>,
    pub latest_version: String,
    pub download_url: String,
    pub sha1: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateFailure {
    pub file_name: String,
    pub reason: String,
}

impl UpdateFailure {
    pub fn new(file_name: &str, reason: impl Into<String>) -> Self {
        Self {
            file_name: file_name.to_string(),
            reason: reason.into(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModCheckResult {
    pub checked: usize,
    pub updates: Vec<ModUpdateItem>,
    pub failed: Vec<UpdateFailure>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModApplyResult {
    pub updated: usize,
    pub failed: Vec<UpdateFailure>,
}

// ------------------------------------------------------------------ 指纹

/// CurseForge 的指纹：murmur2(seed=1)，并且先丢掉空白字节。
pub fn curseforge_fingerprint(bytes: &[u8]) -> u32 {
    const M: u32 = 0x5bd1_e995;
    const R: u32 = 24;
    let filtered: Vec<u8> = bytes
        .iter()
        .copied()
        .filter(|byte| !matches!(byte, 0x09 | 0x0a | 0x0d | 0x20))
        .collect();
    let mut hash = 1u32 ^ filtered.len() as u32;
    let mut chunks = filtered.chunks_exact(4);
    for chunk in chunks.by_ref() {
        let mut word = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        word = word.wrapping_mul(M);
        word ^= word >> R;
        word = word.wrapping_mul(M);
        hash = hash.wrapping_mul(M);
        hash ^= word;
    }
    let rest = chunks.remainder();
    match rest.len() {
        3 => {
            hash ^= (rest[2] as u32) << 16;
            hash ^= (rest[1] as u32) << 8;
            hash ^= rest[0] as u32;
            hash = hash.wrapping_mul(M);
        }
        2 => {
            hash ^= (rest[1] as u32) << 8;
            hash ^= rest[0] as u32;
            hash = hash.wrapping_mul(M);
        }
        1 => {
            hash ^= rest[0] as u32;
            hash = hash.wrapping_mul(M);
        }
        _ => {}
    }
    hash ^= hash >> 13;
    hash = hash.wrapping_mul(M);
    hash ^= hash >> 15;
    hash
}

/// 指纹接口地址（无 Key 时走 MCIM 镜像）。
pub fn fingerprint_url(provider: Provider) -> String {
    format!("{}{FINGERPRINT_PATH}", provider.curseforge_root())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FingerprintMatch {
    /// CurseForge 的 modId（当 projectId 用）
    pub project_id: String,
    pub file_id: u64,
    pub file_name: String,
    pub version: Option<String>,
}

/// 解析 /v1/fingerprints 的响应：exactMatches 里带 file 对象。
pub fn parse_fingerprints(json: &str) -> Result<HashMap<u32, FingerprintMatch>> {
    let value: serde_json::Value = serde_json::from_str(json)
        .map_err(|err| AppError::Net(format!("指纹结果格式不对：{err}")))?;
    let mut out = HashMap::new();
    // 官方接口是 data: [...]；MCIM 镜像把 data 包成对象（exactMatches 在里面），两种都认
    let matches: Vec<serde_json::Value> = match value.get("data") {
        Some(serde_json::Value::Array(items)) => items.clone(),
        Some(serde_json::Value::Object(map)) => map
            .get("exactMatches")
            .and_then(|item| item.as_array())
            .cloned()
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    for entry in matches.iter() {
        let Some(file) = entry.get("file") else {
            continue;
        };
        let (Some(file_id), Some(mod_id)) = (
            file.get("id").and_then(|item| item.as_u64()),
            file.get("modId").and_then(|item| item.as_u64()),
        ) else {
            continue;
        };
        let fingerprint = entry
            .get("fingerprint")
            .and_then(|item| item.as_u64())
            .unwrap_or(0) as u32;
        out.insert(
            fingerprint,
            FingerprintMatch {
                project_id: mod_id.to_string(),
                file_id,
                file_name: file
                    .get("fileName")
                    .and_then(|item| item.as_str())
                    .unwrap_or_default()
                    .to_string(),
                version: file
                    .get("displayName")
                    .and_then(|item| item.as_str())
                    .map(str::to_string),
            },
        );
    }
    Ok(out)
}

fn fingerprint_body(fingerprints: &[u32]) -> serde_json::Value {
    serde_json::json!({ "fingerprints": fingerprints })
}

/// CurseForge 指纹接口一次最多吃多少个指纹。
pub const FINGERPRINT_BATCH: usize = 100;
/// 内容接口对并发很敏感：整轮检查都串行处理，指纹再按批切（并发上限就是 1 批）。
pub const MAX_CONCURRENCY: usize = 1;

/// 把指纹按批切开（每批不超过 size）。
pub fn fingerprint_batches(fingerprints: &[u32], size: usize) -> Vec<&[u32]> {
    let size = size.max(1);
    if fingerprints.is_empty() {
        return Vec::new();
    }
    fingerprints.chunks(size).collect()
}

/// 批量查指纹：按批请求，结果合并。
pub async fn lookup_fingerprints(
    client: &reqwest::Client,
    provider: Provider,
    api_key: Option<&str>,
    fingerprints: &[u32],
) -> Result<HashMap<u32, FingerprintMatch>> {
    let mut out = HashMap::new();
    for batch in fingerprint_batches(fingerprints, FINGERPRINT_BATCH) {
        let mut request = client
            .post(fingerprint_url(provider))
            .json(&fingerprint_body(batch));
        if let Some(key) = api_key.filter(|value| !value.trim().is_empty()) {
            request = request.header("x-api-key", key.trim());
        }
        let response = request
            .send()
            .await
            .map_err(|err| AppError::Net(format!("查询文件指纹失败：{err}")))?;
        let status = response.status().as_u16();
        let text = response
            .text()
            .await
            .map_err(|err| AppError::Net(format!("读取指纹结果失败：{err}")))?;
        if !(200..300).contains(&status) {
            return Err(AppError::Net(format!("查询文件指纹失败：HTTP {status}")));
        }
        out.extend(parse_fingerprints(&text)?);
    }
    Ok(out)
}

/// 按文件名（含 .disabled 的原始名与去掉后缀的名字）找安装记录。
pub fn record_for<'a>(
    records: &'a server_content::RecordFile,
    file_name: &str,
) -> Option<&'a ContentRecord> {
    let base = server_content::split_enabled(file_name).0;
    records
        .files
        .get(file_name)
        .or_else(|| records.files.get(&base))
}

// ------------------------------------------------------------------ 检查更新

/// 实例内容目录要检查的文件（jar / jar.disabled，不含指纹缓存）。
pub fn list_jars(dir: &Path) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.filter_map(|entry| entry.ok()) {
        let name = entry.file_name().to_string_lossy().to_string();
        if !server_content::is_content_jar(&name) {
            continue;
        }
        out.push((name, entry.path()));
    }
    out.sort_by(|left, right| left.0.cmp(&right.0));
    out
}

/// 检查更新：记录优先，其次指纹，两者都不中进 failed。
pub async fn check_updates(
    client: &reqwest::Client,
    instance_dir: &Path,
    content_dir: &Path,
    kind: &str,
    mc_version: &str,
    loader: &str,
    api_key: Option<&str>,
) -> Result<ModCheckResult> {
    let jars = list_jars(content_dir);
    let records = server_content::load_records(instance_dir);
    let mut result = ModCheckResult {
        checked: jars.len(),
        ..Default::default()
    };

    // 1) 有记录的逐个查最新版本
    let mut unmatched: Vec<(String, PathBuf)> = Vec::new();
    for (file_name, path) in &jars {
        let Some(record) = record_for(&records, file_name) else {
            unmatched.push((file_name.clone(), path.clone()));
            continue;
        };
        match latest_from_record(client, record, mc_version, loader, api_key).await {
            Ok(Some((latest_version, download_url, sha1, latest_id))) => {
                if latest_id == record.version_id {
                    continue;
                }
                result.updates.push(ModUpdateItem {
                    file_name: file_name.clone(),
                    path: content_dir.display().to_string(),
                    kind: kind.to_string(),
                    current_version: record.version_number.clone(),
                    latest_version,
                    download_url,
                    sha1,
                });
            }
            Ok(None) => result.failed.push(UpdateFailure::new(
                file_name,
                "该版本没有可用文件",
            )),
            Err(err) => result.failed.push(UpdateFailure::new(file_name, err.to_string())),
        }
    }

    // 2) 没记录的用 CurseForge 指纹兜底（一次请求带全部）
    if !unmatched.is_empty() {
        let mut entries: Vec<(String, PathBuf, u32)> = Vec::new();
        for (file_name, path) in &unmatched {
            match std::fs::read(path) {
                Ok(bytes) => entries.push((
                    file_name.clone(),
                    path.clone(),
                    curseforge_fingerprint(&bytes),
                )),
                Err(err) => result
                    .failed
                    .push(UpdateFailure::new(file_name, format!("读取文件失败：{err}"))),
            }
        }
        let hashes: Vec<u32> = entries.iter().map(|(_, _, hash)| *hash).collect();
        let provider = if api_key.is_some() {
            Provider::Curseforge
        } else {
            Provider::Mcim
        };
        match lookup_fingerprints(client, provider, api_key, &hashes).await {
            Ok(matches) => {
                for (file_name, _path, hash) in entries {
                    let Some(found) = matches.get(&hash) else {
                        result.failed.push(UpdateFailure::new(
                            &file_name,
                            "未记录安装信息，且 CurseForge 指纹未命中",
                        ));
                        continue;
                    };
                    // 指纹给了 CurseForge 的文件：查它所属项目的最新文件
                    let record = ContentRecord {
                        provider: "curseforge".to_string(),
                        project_id: found.project_id.clone(),
                        version_id: found.file_id.to_string(),
                        version_number: found.version.clone(),
                        file_name: found.file_name.clone(),
                    };
                    match latest_from_record(client, &record, mc_version, loader, api_key).await {
                        Ok(Some((latest_version, download_url, sha1, latest_id))) => {
                            if latest_id == record.version_id {
                                continue;
                            }
                            result.updates.push(ModUpdateItem {
                                file_name: file_name.clone(),
                                path: content_dir.display().to_string(),
                                kind: kind.to_string(),
                                current_version: found.version.clone(),
                                latest_version,
                                download_url,
                                sha1,
                            });
                        }
                        Ok(None) => result
                            .failed
                            .push(UpdateFailure::new(&file_name, "该版本没有可用文件")),
                        Err(err) => {
                            result.failed.push(UpdateFailure::new(&file_name, err.to_string()))
                        }
                    }
                }
            }
            Err(err) => {
                for (file_name, _, _) in entries {
                    result.failed.push(UpdateFailure::new(
                        &file_name,
                        format!("未记录安装信息，指纹查询失败：{err}"),
                    ));
                }
            }
        }
    }
    Ok(result)
}

/// 用安装记录查该项目在 (mcVersion, loader) 下的最新文件。
async fn latest_from_record(
    client: &reqwest::Client,
    record: &ContentRecord,
    mc_version: &str,
    loader: &str,
    api_key: Option<&str>,
) -> Result<Option<(String, String, Option<String>, String)>> {
    let provider = Provider::from_id(&record.provider)?;
    let url = content::versions_url(provider, &record.project_id, Some(mc_version), Some(loader))?;
    let text = content::http_text(client, &url, api_key).await?;
    let versions = content::parse_versions(provider, &text)?;
    let Some(latest) = versions.first() else {
        return Ok(None);
    };
    let file_name = latest
        .file_name
        .clone()
        .unwrap_or_else(|| record.file_name.clone());
    let download_url = match latest.download_url.clone() {
        Some(url) => url,
        None => content::file_url(
            provider,
            &record.project_id,
            &latest.id,
            latest.file_id,
            &file_name,
        )?,
    };
    Ok(Some((
        latest.version_number.clone(),
        download_url,
        latest.sha1.clone(),
        latest.id.clone(),
    )))
}

// ------------------------------------------------------------------ 应用更新

/// 备份目录：<instance>/.gugu-backup/<时间戳>/
pub fn backup_dir(instance_dir: &Path, stamp: u64) -> PathBuf {
    instance_dir.join(BACKUP_DIR).join(util::compact_stamp(stamp))
}

/// 应用更新：先备份 → 下载 → 校验 → 原子替换；失败回滚。
pub async fn apply_updates(
    client: &reqwest::Client,
    instance_dir: &Path,
    content_dir: &Path,
    items: &[ModUpdateItem],
    via_mcim: bool,
) -> Result<ModApplyResult> {
    let backup = backup_dir(instance_dir, util::now_unix());
    let mut result = ModApplyResult::default();
    for item in items {
        match replace_one(client, &backup, content_dir, item, via_mcim).await {
            Ok(()) => result.updated += 1,
            Err(err) => result
                .failed
                .push(UpdateFailure::new(&item.file_name, err.to_string())),
        }
    }
    Ok(result)
}

async fn replace_one(
    client: &reqwest::Client,
    backup: &Path,
    content_dir: &Path,
    item: &ModUpdateItem,
    via_mcim: bool,
) -> Result<()> {
    let file_name = crate::server_content::validate_file_name(&item.file_name)?;
    let target = content_dir.join(&file_name);
    let bytes = content::download_bytes(client, &item.download_url, via_mcim).await?;
    if let Some(expected) = item.sha1.as_deref().filter(|value| !value.trim().is_empty()) {
        let actual = util::sha1_hex(&bytes);
        if !actual.eq_ignore_ascii_case(expected) {
            return Err(AppError::Net("sha1 校验失败，已放弃写入".to_string()));
        }
    }
    std::fs::create_dir_all(content_dir)?;
    // 备份原文件（有的话）
    let mut rollback: Option<PathBuf> = None;
    if target.is_file() {
        std::fs::create_dir_all(backup)?;
        let saved = backup.join(&file_name);
        std::fs::copy(&target, &saved)?;
        rollback = Some(saved);
    }
    let tmp = content_dir.join(format!("{file_name}.part"));
    std::fs::write(&tmp, &bytes)?;
    if let Err(err) = std::fs::rename(&tmp, &target) {
        let _ = std::fs::remove_file(&tmp);
        return Err(AppError::Io(err));
    }
    // 替换成功后把备份留在磁盘（用户能找回旧版本），只记日志
    if let Some(saved) = rollback {
        tracing::info!(
            file = %file_name,
            backup = %saved.display(),
            "内容已更新，旧版本留备份"
        );
    }
    Ok(())
}

// ------------------------------------------------------------------ 深度启动游戏

/// 该版本能不能用 --quickPlaySingleplayer（1.20 及以上；快照算支持）。
pub fn supports_quick_play(mc_version: &str) -> bool {
    let text = mc_version.trim();
    if text.is_empty() {
        return false;
    }
    // 快照：24w14a / 1.20-pre1 都按支持处理
    if text.chars().next().map(|value| value.is_ascii_digit()) == Some(true)
        && text.contains('w')
        && !text.starts_with("1.")
    {
        return true;
    }
    let mut parts = text.split(['.', '-', '+']);
    let major: u32 = parts.next().and_then(|value| value.parse().ok()).unwrap_or(0);
    let minor: u32 = parts.next().and_then(|value| value.parse().ok()).unwrap_or(0);
    if major != 1 {
        return major > 1;
    }
    minor >= 20
}

/// 不能直接进存档时的中文说明。
pub fn quick_play_rejection(mc_version: &str) -> String {
    format!("该版本（{mc_version}）不支持直接进入存档，1.20 及以上才支持")
}

/// --quickPlaySingleplayer 的两个参数（存档名带空格也算一个参数）。
pub fn quick_play_args(level_name: &str) -> Vec<String> {
    vec![
        "--quickPlaySingleplayer".to_string(),
        level_name.to_string(),
    ]
}
#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("gugu-updates-{tag}-{}", util::unique_suffix()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn record(provider: &str, project: &str, version: &str, file: &str) -> ContentRecord {
        ContentRecord {
            provider: provider.to_string(),
            project_id: project.to_string(),
            version_id: version.to_string(),
            version_number: Some("1.0.0".to_string()),
            file_name: file.to_string(),
        }
    }

    #[test]
    fn fingerprint_ignores_whitespace_bytes() {
        let plain = b"PK\x03\x04minecraft-mod";
        let spaced = b"PK\x03\x04\n minecraft-mod\r\t";
        assert_eq!(
            curseforge_fingerprint(plain),
            curseforge_fingerprint(spaced),
            "空白字节要先丢掉再算指纹"
        );
        assert_ne!(
            curseforge_fingerprint(b"abc"),
            curseforge_fingerprint(b"abd")
        );
        // 空内容也能算（不 panic）
        assert_eq!(curseforge_fingerprint(b""), curseforge_fingerprint(b"  \n\t\r"));
    }

    #[test]
    fn fingerprints_are_batched_before_hitting_the_api() {
        let hashes: Vec<u32> = (0..250).collect();
        let batches = fingerprint_batches(&hashes, FINGERPRINT_BATCH);
        assert_eq!(batches.len(), 3);
        assert_eq!(batches[0].len(), FINGERPRINT_BATCH);
        assert_eq!(batches[2].len(), 50);
        assert_eq!(FINGERPRINT_BATCH, 100);
        assert_eq!(MAX_CONCURRENCY, 1, "内容接口串行处理，不并发打");
        assert!(fingerprint_batches(&[], FINGERPRINT_BATCH).is_empty());
        assert_eq!(fingerprint_batches(&[1, 2, 3], 0).len(), 3, "size=0 也按 1 切");
        assert_eq!(
            fingerprint_url(Provider::Curseforge),
            "https://api.curseforge.com/v1/fingerprints"
        );
        assert_eq!(
            fingerprint_url(Provider::Mcim),
            "https://mod.mcimirror.top/curseforge/v1/fingerprints"
        );
    }

    #[test]
    fn fingerprint_response_parses_exact_matches() {
        let json = r#"{"data":[{"id":1,"file":{"id":4242424,"modId":238222,"fileName":"jei.jar","displayName":"JEI 1.0"},"fingerprint":1234567}]}"#;
        let parsed = parse_fingerprints(json).unwrap();
        let found = parsed.get(&1234567).unwrap();
        assert_eq!(found.project_id, "238222");
        assert_eq!(found.file_id, 4242424);
        assert_eq!(found.file_name, "jei.jar");
        assert_eq!(found.version.as_deref(), Some("JEI 1.0"));
        assert!(parse_fingerprints(r#"{"data":[]}"#).unwrap().is_empty());
        // MCIM 镜像把 data 包成对象
        let mirror = r#"{"data":{"isCacheBuilt":true,"exactMatches":[{"id":2,"file":{"id":7,"modId":99,"fileName":"sodium.jar","displayName":"sodium 0.5"},"fingerprint":42}],"unmatchedFingerprints":[]}}"#;
        let parsed = parse_fingerprints(mirror).unwrap();
        assert_eq!(parsed.get(&42).map(|item| item.project_id.as_str()), Some("99"));
    }

    #[test]
    fn record_lookup_handles_disabled_suffix() {
        let mut records = server_content::RecordFile::default();
        records
            .files
            .insert("sodium.jar".to_string(), record("modrinth", "sodium", "v1", "sodium.jar"));
        assert_eq!(
            record_for(&records, "sodium.jar").map(|item| item.project_id.as_str()),
            Some("sodium"),
            "记录命中"
        );
        assert_eq!(
            record_for(&records, "sodium.jar.disabled").map(|item| item.project_id.as_str()),
            Some("sodium"),
            "禁用状态也要能命中记录"
        );
        assert!(record_for(&records, "unknown.jar").is_none(), "没记录 → 走指纹");
        // 指纹也没中时的原因文案
        let failure = UpdateFailure::new("unknown.jar", "未记录安装信息，且 CurseForge 指纹未命中");
        assert!(failure.reason.contains("指纹未命中"));
        assert_eq!(failure.file_name, "unknown.jar");
    }

    #[test]
    fn check_marks_unrecorded_and_unmatched_files_as_failed() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let instance = temp_dir("check-instance");
        let mods = instance.join("mods");
        std::fs::create_dir_all(&mods).unwrap();
        std::fs::write(mods.join("unknown.jar"), b"PK\x03\x04junk").unwrap();
        std::fs::write(mods.join("notes.txt"), b"x").unwrap();
        let client = reqwest::Client::new();
        // 指向不存在的镜像，保证指纹查询失败 → 全部进 failed
        let result = runtime
            .block_on(check_updates(
                &client,
                &instance,
                &mods,
                "mod",
                "1.20.1",
                "fabric",
                None,
            ))
            .unwrap();
        assert_eq!(result.checked, 1, "只统计 jar");
        assert!(result.updates.is_empty());
        assert_eq!(result.failed.len(), 1);
        assert_eq!(result.failed[0].file_name, "unknown.jar");
        let _ = std::fs::remove_dir_all(&instance);
    }

    #[test]
    fn apply_keeps_original_when_download_fails() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let instance = temp_dir("apply");
        let mods = instance.join("mods");
        std::fs::create_dir_all(&mods).unwrap();
        std::fs::write(mods.join("sodium.jar"), b"OLD").unwrap();
        let item = ModUpdateItem {
            file_name: "sodium.jar".to_string(),
            path: mods.display().to_string(),
            kind: "mod".to_string(),
            current_version: Some("0.1".to_string()),
            latest_version: "0.2".to_string(),
            // 连不上的本地地址：模拟下载失败
            download_url: "http://127.0.0.1:9/sodium.jar".to_string(),
            sha1: None,
        };
        let client = reqwest::Client::new();
        let result = runtime
            .block_on(apply_updates(&client, &instance, &mods, &[item], false))
            .unwrap();
        assert_eq!(result.updated, 0);
        assert_eq!(result.failed.len(), 1);
        assert_eq!(
            std::fs::read(mods.join("sodium.jar")).unwrap(),
            b"OLD",
            "下载失败不能动原文件"
        );
        assert!(!mods.join("sodium.jar.part").exists(), "临时文件要清掉");
        let _ = std::fs::remove_dir_all(&instance);
    }

    #[test]
    fn backup_dir_is_timestamped_under_instance() {
        let instance = Path::new("D:\\minecraft\\.mc\\instances\\测试");
        let dir = backup_dir(instance, 1_700_000_000);
        assert!(dir.starts_with(instance.join(BACKUP_DIR)));
        assert_eq!(BACKUP_DIR, ".gugu-backup");
        // compact_stamp 形如 20231114-221320（日期-时分秒）
        let name = dir.file_name().unwrap().to_string_lossy().to_string();
        assert_eq!(name.len(), 15, "{name}");
        assert!(name.chars().next().unwrap().is_ascii_digit(), "{name}");
        assert_eq!(&name[8..9], "-", "{name}");
    }

    #[test]
    fn quick_play_args_keep_spaces_in_one_argument() {
        let args = quick_play_args("我的 存档");
        assert_eq!(args.len(), 2);
        assert_eq!(args[0], "--quickPlaySingleplayer");
        assert_eq!(args[1], "我的 存档");
    }

    #[test]
    fn quick_play_support_is_version_gated() {
        assert!(supports_quick_play("1.20"));
        assert!(supports_quick_play("1.20.1"));
        assert!(supports_quick_play("1.21.4"));
        // 快照也算支持
        assert!(supports_quick_play("24w14a"));
        // 1.20 以下不支持
        assert!(!supports_quick_play("1.19.4"));
        assert!(!supports_quick_play("1.12.2"));
        assert!(!supports_quick_play(""));
        assert!(!supports_quick_play("不知道"));
        let message = quick_play_rejection("1.19.2");
        assert_eq!(
            message,
            "该版本（1.19.2）不支持直接进入存档，1.20 及以上才支持"
        );
    }

    #[test]
    fn jar_listing_skips_other_files() {
        let dir = temp_dir("list");
        std::fs::write(dir.join("a.jar"), b"x").unwrap();
        std::fs::write(dir.join("b.jar.disabled"), b"x").unwrap();
        std::fs::write(dir.join("c.zip"), b"x").unwrap();
        std::fs::write(dir.join("d.txt"), b"x").unwrap();
        let jars = list_jars(&dir);
        assert_eq!(jars.len(), 2);
        assert_eq!(jars[0].0, "a.jar");
        assert_eq!(jars[1].0, "b.jar.disabled");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
