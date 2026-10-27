//! 实例 CRUD 与实例设置：落 sqlite + 建出 instances\<名字>\ 目录。
//!
//! 前端契约（M2-B）：
//! - instances_list() 每条带 playTimeSecs / lastPlayed / iconPath；
//! - instance_settings_get / set 用 InstanceSettings（含 note、memoryMb、窗口宽高、javaPath、iconPath）；
//! - 改名要同步改目录名。

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, Result};
use crate::paths::Paths;
use crate::settings::{MEMORY_MB_MAX, MEMORY_MB_MIN};
use crate::util;

pub const DEFAULT_WINDOW_WIDTH: u32 = 854;
pub const DEFAULT_WINDOW_HEIGHT: u32 = 480;
pub const WINDOW_MIN: u32 = 320;
pub const WINDOW_MAX: u32 = 7680;

/// 实例（序列化给前端，camelCase）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Instance {
    pub id: i64,
    pub name: String,
    pub mc_version: String,
    pub loader: String,
    pub loader_version: String,
    /// 实例图标（等价于 iconPath，老前端只读这个字段）
    pub icon: String,
    pub pinned_home: bool,
    pub play_time_secs: i64,
    pub created_at: String,
    /// 前端统计条按 lastPlayed 读，老字段 lastPlayedAt 同时保留
    pub last_played: Option<String>,
    pub last_played_at: Option<String>,
    pub note: String,
    pub memory_mb: u32,
    pub window_width: u32,
    pub window_height: u32,
    pub java_path: String,
    pub icon_path: String,
    /// 整合包名（modpack_install 写入，普通实例为空串）
    pub pack_name: String,
    /// 整合包状态：installed / incomplete（普通实例为 None）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pack_state: Option<String>,
}

/// 新建实例的入参。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceCreate {
    pub name: String,
    pub mc_version: String,
    #[serde(default)]
    pub loader: Option<String>,
    #[serde(default)]
    pub loader_version: Option<String>,
    #[serde(default)]
    pub memory_mb: Option<u32>,
}

/// instance_settings_get 的返回 / instance_settings_set 的入参。
///
/// 窗口尺寸同时给 windowWidth/windowHeight 与 width/height 两套字段名，
/// javaPath/iconPath 允许 null（前端类型是 string | null）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceSettingsView {
    pub name: String,
    pub note: String,
    pub memory_mb: u32,
    pub window_width: u32,
    pub window_height: u32,
    pub width: u32,
    pub height: u32,
    pub java_path: Option<String>,
    pub icon_path: Option<String>,
    /// 这个版本能不能「直接进入存档」（1.20 及以上）
    pub supports_quick_play: bool,
}

/// 设置补丁：只改传进来的字段（null 也表示不改，空字符串表示清空）。
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceSettingsPatch {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub memory_mb: Option<u32>,
    #[serde(default, alias = "width")]
    pub window_width: Option<u32>,
    #[serde(default, alias = "height")]
    pub window_height: Option<u32>,
    #[serde(default)]
    pub java_path: Option<String>,
    #[serde(default)]
    pub icon_path: Option<String>,
}

impl Instance {
    pub fn settings_view(&self) -> InstanceSettingsView {
        InstanceSettingsView {
            name: self.name.clone(),
            note: self.note.clone(),
            memory_mb: self.memory_mb,
            window_width: self.window_width,
            window_height: self.window_height,
            width: self.window_width,
            height: self.window_height,
            java_path: if self.java_path.is_empty() {
                None
            } else {
                Some(self.java_path.clone())
            },
            icon_path: if self.icon_path.is_empty() {
                None
            } else {
                Some(self.icon_path.clone())
            },
            supports_quick_play: crate::updates::supports_quick_play(&self.mc_version),
        }
    }
}

/// 实例名直接当目录名用，必须挡掉路径分隔符和 Windows 保留字符。
pub fn validate_name(name: &str) -> Result<()> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(AppError::Invalid("实例名不能为空".into()));
    }
    if trimmed.chars().count() > 64 {
        return Err(AppError::Invalid("实例名不能超过 64 个字符".into()));
    }
    const FORBIDDEN: [char; 9] = ['\\', '/', ':', '*', '?', '"', '<', '>', '|'];
    if let Some(bad) = trimmed.chars().find(|c| FORBIDDEN.contains(c)) {
        return Err(AppError::Invalid(format!("实例名不能包含 {bad:?}")));
    }
    if trimmed.starts_with('.') {
        return Err(AppError::Invalid("实例名不能以 . 开头".into()));
    }
    Ok(())
}

/// 窗口宽高：夹到合理范围并取偶数（MC 要求窗口尺寸是偶数）。
pub fn normalize_window(value: u32, fallback: u32) -> u32 {
    let value = if value == 0 { fallback } else { value };
    let clamped = value.clamp(WINDOW_MIN, WINDOW_MAX);
    clamped - clamped % 2
}

/// 内存：0 表示跟随全局设置，其余夹到合法区间。
pub fn normalize_memory(memory_mb: u32) -> u32 {
    if memory_mb == 0 {
        0
    } else {
        memory_mb.clamp(MEMORY_MB_MIN, MEMORY_MB_MAX)
    }
}

/// 纯函数：把补丁合到实例上（目录改名由调用方负责）。
pub fn apply_patch(current: &Instance, patch: &InstanceSettingsPatch) -> Result<Instance> {
    let mut next = current.clone();
    if let Some(name) = &patch.name {
        validate_name(name)?;
        next.name = name.trim().to_string();
    }
    if let Some(note) = &patch.note {
        next.note = note.chars().take(500).collect();
    }
    if let Some(memory) = patch.memory_mb {
        next.memory_mb = normalize_memory(memory);
    }
    if let Some(width) = patch.window_width {
        next.window_width = normalize_window(width, DEFAULT_WINDOW_WIDTH);
    }
    if let Some(height) = patch.window_height {
        next.window_height = normalize_window(height, DEFAULT_WINDOW_HEIGHT);
    }
    if let Some(java_path) = &patch.java_path {
        let trimmed = java_path.trim();
        if !trimmed.is_empty() && !std::path::Path::new(trimmed).is_file() {
            return Err(AppError::Invalid(format!("指定的 Java 不存在：{trimmed}")));
        }
        next.java_path = trimmed.to_string();
    }
    if let Some(icon_path) = &patch.icon_path {
        next.icon_path = icon_path.trim().to_string();
        next.icon = next.icon_path.clone();
    }
    Ok(next)
}

fn row_to_instance(row: &rusqlite::Row<'_>) -> rusqlite::Result<Instance> {
    let icon: String = row.get("icon")?;
    let last_played_at: Option<String> = row.get("last_played_at")?;
    Ok(Instance {
        id: row.get("id")?,
        name: row.get("name")?,
        mc_version: row.get("mc_version")?,
        loader: row.get("loader")?,
        loader_version: row.get("loader_version")?,
        icon: icon.clone(),
        pinned_home: row.get::<_, i64>("pinned_home")? != 0,
        play_time_secs: row.get("play_time_secs")?,
        created_at: row.get("created_at")?,
        last_played: last_played_at.clone(),
        last_played_at,
        note: row.get("note")?,
        memory_mb: row.get::<_, i64>("memory_mb")? as u32,
        window_width: row.get::<_, i64>("window_width")? as u32,
        window_height: row.get::<_, i64>("window_height")? as u32,
        java_path: row.get("java_path")?,
        icon_path: icon,
        pack_name: row.get("pack_name")?,
        pack_state: {
            let state: String = row.get("pack_state")?;
            if state.trim().is_empty() {
                None
            } else {
                Some(state)
            }
        },
    })
}

const SELECT_COLUMNS: &str = "id, name, mc_version, loader, loader_version, icon, pinned_home,
                              play_time_secs, created_at, last_played_at, note, memory_mb,
                              window_width, window_height, java_path, pack_name, pack_state";

pub fn list(conn: &Connection) -> Result<Vec<Instance>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {SELECT_COLUMNS} FROM instances ORDER BY pinned_home DESC, id ASC"
    ))?;
    let rows = stmt.query_map([], row_to_instance)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// 最近游玩：按最后游玩时间倒序（没玩过的排最后，按 id 升序兜底）。
pub fn recent(conn: &Connection, limit: usize) -> Result<Vec<Instance>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {SELECT_COLUMNS} FROM instances
         ORDER BY (last_played_at IS NULL) ASC, last_played_at DESC, id ASC
         LIMIT ?1"
    ))?;
    let rows = stmt.query_map([limit as i64], row_to_instance)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub fn get(conn: &Connection, id: i64) -> Result<Instance> {
    conn.query_row(
        &format!("SELECT {SELECT_COLUMNS} FROM instances WHERE id = ?1"),
        [id],
        row_to_instance,
    )
    .map_err(|err| match err {
        rusqlite::Error::QueryReturnedNoRows => {
            AppError::Invalid(format!("实例不存在：id={id}"))
        }
        other => AppError::Db(other),
    })
}

/// 实例目录：<数据根>\instances\<名字>\；游戏目录是里面的 .minecraft。
pub fn instance_dir(paths: &Paths, name: &str) -> std::path::PathBuf {
    paths.instances.join(name)
}

pub fn game_dir(paths: &Paths, name: &str) -> std::path::PathBuf {
    instance_dir(paths, name).join(".minecraft")
}

/// 实例图标文件（实例目录下的 icon.png）。
pub fn icon_file(paths: &Paths, name: &str) -> std::path::PathBuf {
    instance_dir(paths, name).join("icon.png")
}

pub fn create(conn: &Connection, paths: &Paths, payload: &InstanceCreate) -> Result<Instance> {
    validate_name(&payload.name)?;
    if payload.mc_version.trim().is_empty() {
        return Err(AppError::Invalid("游戏版本不能为空".into()));
    }

    let name = payload.name.trim().to_string();
    let loader = payload.loader.clone().unwrap_or_else(|| "Vanilla".to_string());
    let loader_version = payload.loader_version.clone().unwrap_or_default();
    let created_at = util::iso8601(util::now_unix());
    let memory_mb = normalize_memory(payload.memory_mb.unwrap_or(0));

    let dir = instance_dir(paths, &name);
    if dir.exists() {
        return Err(AppError::Invalid(format!("实例目录已存在：{}", dir.display())));
    }

    conn.execute(
        "INSERT INTO instances (name, mc_version, loader, loader_version, icon, pinned_home,
                                play_time_secs, created_at, note, memory_mb, window_width,
                                window_height, java_path)
         VALUES (?1, ?2, ?3, ?4, '', 0, 0, ?5, '', ?6, ?7, ?8, '')",
        params![
            name,
            payload.mc_version.trim(),
            loader,
            loader_version,
            created_at,
            memory_mb,
            DEFAULT_WINDOW_WIDTH,
            DEFAULT_WINDOW_HEIGHT,
        ],
    )
    .map_err(|err| match err {
        rusqlite::Error::SqliteFailure(_, Some(msg)) if msg.contains("UNIQUE") => {
            AppError::Invalid(format!("实例名已存在：{name}"))
        }
        other => AppError::Db(other),
    })?;

    std::fs::create_dir_all(game_dir(paths, &name))?;

    let id = conn.last_insert_rowid();
    get(conn, id)
}

/// 删除实例：先删目录再删行；目录不存在不算失败。
pub fn delete(conn: &Connection, paths: &Paths, id: i64) -> Result<()> {
    let instance = get(conn, id)?;
    let dir = instance_dir(paths, &instance.name);
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    conn.execute("DELETE FROM instances WHERE id = ?1", [id])?;
    Ok(())
}

/// 更新实例设置；改名时同步改目录名。
pub fn update_settings(
    conn: &Connection,
    paths: &Paths,
    id: i64,
    patch: &InstanceSettingsPatch,
) -> Result<Instance> {
    let current = get(conn, id)?;
    let next = apply_patch(&current, patch)?;

    if next.name != current.name {
        let from = instance_dir(paths, &current.name);
        let to = instance_dir(paths, &next.name);
        if to.exists() {
            return Err(AppError::Invalid(format!(
                "目标实例目录已存在：{}",
                to.display()
            )));
        }
        if from.is_dir() {
            std::fs::rename(&from, &to)?;
        } else {
            std::fs::create_dir_all(game_dir(paths, &next.name))?;
        }
    }

    // 图标路径变了就把文件落到实例目录下（前端传进来的可能是任意 png）
    if next.icon_path != current.icon_path && !next.icon_path.is_empty() {
        let source = std::path::Path::new(&next.icon_path);
        let target = icon_file(paths, &next.name);
        if source.is_file() && source != target {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(source, &target)?;
        }
    }

    conn.execute(
        "UPDATE instances SET name = ?2, note = ?3, memory_mb = ?4, window_width = ?5,
                window_height = ?6, java_path = ?7, icon = ?8
         WHERE id = ?1",
        params![
            id,
            next.name,
            next.note,
            next.memory_mb as i64,
            next.window_width as i64,
            next.window_height as i64,
            next.java_path,
            if next.icon_path.is_empty() {
                icon_file(paths, &next.name).display().to_string()
            } else {
                next.icon_path.clone()
            },
        ],
    )
    .map_err(|err| match err {
        rusqlite::Error::SqliteFailure(_, Some(msg)) if msg.contains("UNIQUE") => {
            AppError::Invalid(format!("实例名已存在：{}", next.name))
        }
        other => AppError::Db(other),
    })?;

    get(conn, id)
}

/// 记录实例图标（instance_pick_icon 用）。
pub fn set_icon(conn: &Connection, id: i64, icon_path: &str) -> Result<()> {
    conn.execute(
        "UPDATE instances SET icon = ?2 WHERE id = ?1",
        params![id, icon_path],
    )?;
    Ok(())
}

/// 整合包安装写 packName + packState（installed / incomplete）。
pub fn set_pack(conn: &Connection, id: i64, pack_name: &str, state: &str) -> Result<()> {
    conn.execute(
        "UPDATE instances SET pack_name = ?2, pack_state = ?3 WHERE id = ?1",
        params![id, pack_name, state],
    )?;
    Ok(())
}

/// 只刷新最近游玩时间（整合包装完把自己顶到"最近游玩"第一位）。
pub fn touch_last_played(conn: &Connection, id: i64) -> Result<()> {
    conn.execute(
        "UPDATE instances SET last_played_at = ?2 WHERE id = ?1",
        params![id, util::iso8601(util::now_unix())],
    )?;
    Ok(())
}

/// 离线账号：账号表里的第一条（没有就用默认离线名）。
pub fn first_account(conn: &Connection) -> Option<(String, String)> {
    conn.query_row(
        "SELECT username, uuid FROM accounts ORDER BY id ASC LIMIT 1",
        [],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
    )
    .ok()
    .filter(|(username, _)| !username.trim().is_empty())
}

/// 关掉游戏后累加游玩时长并刷新最近游玩时间。
pub fn add_playtime(conn: &Connection, id: i64, seconds: i64) -> Result<()> {
    let seconds = seconds.max(0);
    conn.execute(
        "UPDATE instances SET play_time_secs = play_time_secs + ?2, last_played_at = ?3
         WHERE id = ?1",
        params![id, seconds, util::iso8601(util::now_unix())],
    )?;
    Ok(())
}

/// 供测试与后续命令复用的实例目录是否存在的判断。
pub fn dir_exists(paths: &Paths, name: &str) -> bool {
    instance_dir(paths, name).is_dir()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    fn setup() -> (Connection, Paths) {
        // 每个测试一个独立数据根：同一进程里并行跑也不会互相看到对方的实例
        let root = std::env::temp_dir().join(format!(
            "gugu-instance-test-{}-{}",
            std::process::id(),
            crate::util::unique_suffix()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let paths = Paths::from_root(&root);
        paths.ensure().unwrap();
        let conn = db::open(&paths.db_file).unwrap();
        (conn, paths)
    }

    fn payload(name: &str) -> InstanceCreate {
        InstanceCreate {
            name: name.into(),
            mc_version: "1.20.1".into(),
            loader: Some("Vanilla".into()),
            loader_version: None,
            memory_mb: None,
        }
    }

    #[test]
    fn create_list_delete_roundtrip() {
        let (conn, paths) = setup();
        let created = create(&conn, &paths, &payload("26.3 Fabric")).unwrap();
        assert_eq!(created.name, "26.3 Fabric");
        assert_eq!(created.window_width, DEFAULT_WINDOW_WIDTH);
        assert_eq!(created.memory_mb, 0);
        assert!(dir_exists(&paths, "26.3 Fabric"));
        assert!(game_dir(&paths, "26.3 Fabric").is_dir());

        assert!(create(&conn, &paths, &payload("26.3 Fabric")).is_err(), "重名必须被拒绝");

        let all = list(&conn).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].play_time_secs, 0);
        assert!(all[0].last_played.is_none());

        delete(&conn, &paths, created.id).unwrap();
        assert!(list(&conn).unwrap().is_empty());
        assert!(!dir_exists(&paths, "26.3 Fabric"));

        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn invalid_names_are_rejected() {
        assert!(validate_name("").is_err());
        assert!(validate_name("a/b").is_err());
        assert!(validate_name("a:b").is_err());
        assert!(validate_name(".hidden").is_err());
        assert!(validate_name("ok-name_1").is_ok());
    }

    #[test]
    fn patch_merges_only_given_fields() {
        let (conn, paths) = setup();
        let created = create(&conn, &paths, &payload("原名")).unwrap();

        let patch = InstanceSettingsPatch {
            note: Some("备注一下".into()),
            memory_mb: Some(8192),
            window_width: Some(1921),
            window_height: Some(1080),
            ..Default::default()
        };
        let next = apply_patch(&created, &patch).unwrap();
        assert_eq!(next.name, "原名");
        assert_eq!(next.note, "备注一下");
        assert_eq!(next.memory_mb, 8192);
        assert_eq!(next.window_width, 1920, "奇数要取偶数");
        assert_eq!(next.window_height, 1080);

        // 内存夹到合法区间；0 表示跟随全局
        let clamped = apply_patch(
            &created,
            &InstanceSettingsPatch {
                memory_mb: Some(64),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(clamped.memory_mb, MEMORY_MB_MIN);
        let follow = apply_patch(
            &created,
            &InstanceSettingsPatch {
                memory_mb: Some(0),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(follow.memory_mb, 0);

        // 非法名字 / 不存在的 Java 都要报错
        assert!(apply_patch(
            &created,
            &InstanceSettingsPatch {
                name: Some("a/b".into()),
                ..Default::default()
            }
        )
        .is_err());
        assert!(apply_patch(
            &created,
            &InstanceSettingsPatch {
                java_path: Some("C:\\nope\\javaw.exe".into()),
                ..Default::default()
            }
        )
        .is_err());

        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn update_settings_renames_directory_and_persists() {
        let (conn, paths) = setup();
        let created = create(&conn, &paths, &payload("旧名字")).unwrap();
        std::fs::write(game_dir(&paths, "旧名字").join("options.txt"), b"fov:70").unwrap();

        let updated = update_settings(
            &conn,
            &paths,
            created.id,
            &InstanceSettingsPatch {
                name: Some("新名字".into()),
                note: Some("改了名".into()),
                memory_mb: Some(4096),
                window_width: Some(1600),
                window_height: Some(900),
                java_path: Some(String::new()),
                icon_path: None,
            },
        )
        .unwrap();

        assert_eq!(updated.name, "新名字");
        assert_eq!(updated.note, "改了名");
        assert!(dir_exists(&paths, "新名字"));
        assert!(!dir_exists(&paths, "旧名字"));
        assert!(game_dir(&paths, "新名字").join("options.txt").is_file());

        let reloaded = get(&conn, created.id).unwrap();
        assert_eq!(reloaded.name, "新名字");
        assert_eq!(reloaded.window_height, 900);
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn playtime_accumulates_and_updates_last_played() {
        let (conn, paths) = setup();
        let created = create(&conn, &paths, &payload("时长")).unwrap();
        add_playtime(&conn, created.id, 90).unwrap();
        add_playtime(&conn, created.id, 30).unwrap();
        let reloaded = get(&conn, created.id).unwrap();
        assert_eq!(reloaded.play_time_secs, 120);
        assert!(reloaded.last_played.is_some());
        assert!(reloaded.last_played_at.is_some());
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn settings_view_returns_both_window_names() {
        let (conn, paths) = setup();
        let created = create(&conn, &paths, &payload("视图")).unwrap();
        let view = created.settings_view();
        assert_eq!(view.window_width, view.width);
        assert_eq!(view.window_height, view.height);
        assert!(view.java_path.is_none());
        let json = serde_json::to_value(&view).unwrap();
        assert!(json.get("windowWidth").is_some());
        assert!(json.get("width").is_some());
        assert_eq!(json["javaPath"], serde_json::Value::Null);
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn pack_state_is_written_and_defaults_to_none() {
        let (conn, paths) = setup();
        let created = create(&conn, &paths, &payload("整合包实例")).unwrap();
        assert!(created.pack_state.is_none(), "普通实例没有整合包状态");
        assert!(created.pack_name.is_empty());

        set_pack(&conn, created.id, "Fabulously Optimized", "incomplete").unwrap();
        let marked = get(&conn, created.id).unwrap();
        assert_eq!(marked.pack_name, "Fabulously Optimized");
        assert_eq!(marked.pack_state.as_deref(), Some("incomplete"));

        set_pack(&conn, created.id, "Fabulously Optimized", "installed").unwrap();
        let installed = get(&conn, created.id).unwrap();
        assert_eq!(installed.pack_state.as_deref(), Some("installed"));
        // 序列化给前端时字段名是 packName / packState
        let json = serde_json::to_value(&installed).unwrap();
        assert_eq!(json["packName"], "Fabulously Optimized");
        assert_eq!(json["packState"], "installed");
        // 普通实例不出 packState（前端 optional 类型）
        let plain = create(&conn, &paths, &payload("普通实例")).unwrap();
        let plain_json = serde_json::to_value(&plain).unwrap();
        assert!(plain_json.get("packState").is_none());

        touch_last_played(&conn, created.id).unwrap();
        let recent = recent(&conn, 10).unwrap();
        assert_eq!(recent[0].id, created.id, "刚装完的整合包要排最近游玩第一位");
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn missing_instance_reports_readable_error() {
        let (conn, paths) = setup();
        let err = get(&conn, 4242).unwrap_err().to_string();
        assert!(err.contains("实例不存在"), "{err}");
        let _ = std::fs::remove_dir_all(&paths.root);
    }
}
