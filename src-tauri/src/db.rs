//! SQLite：`.mc\gugu.db`，最小 schema + 版本表（PRAGMA user_version）迁移。
//!
//! 迁移只向前：每个版本一个 `apply_vN`，启动时按 user_version 依次补齐。
//! 命令层每次调用新开连接（rusqlite::Connection 不是 Sync，这样做也不用到处加锁）。

use std::path::Path;

use rusqlite::Connection;

use crate::error::Result;

/// 当前 schema 版本。v2 给实例表加上设置列（备注/内存/窗口/Java），
/// v3 加上整合包列（pack_name / pack_state）。
pub const SCHEMA_VERSION: i64 = 4;

/// 打开（必要时创建）数据库并跑迁移。
pub fn open(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut conn = Connection::open(path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    migrate(&mut conn)?;
    Ok(conn)
}

/// 按 user_version 逐级迁移，幂等。
pub fn migrate(conn: &mut Connection) -> Result<()> {
    let mut version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    while version < SCHEMA_VERSION {
        version += 1;
        let tx = conn.transaction()?;
        match version {
            1 => apply_v1(&tx)?,
            2 => apply_v2(&tx)?,
            3 => apply_v3(&tx)?,
            4 => apply_v4(&tx)?,
            other => {
                return Err(crate::error::AppError::Other(format!(
                    "缺少 schema v{other} 的迁移实现"
                )))
            }
        }
        tx.pragma_update(None, "user_version", version)?;
        tx.commit()?;
    }
    Ok(())
}

fn apply_v1(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS instances (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            name            TEXT    NOT NULL UNIQUE,
            mc_version      TEXT    NOT NULL,
            loader          TEXT    NOT NULL DEFAULT 'Vanilla',
            loader_version  TEXT    NOT NULL DEFAULT '',
            icon            TEXT    NOT NULL DEFAULT '',
            pinned_home     INTEGER NOT NULL DEFAULT 0,
            play_time_secs  INTEGER NOT NULL DEFAULT 0,
            created_at      TEXT    NOT NULL,
            last_played_at  TEXT
        );

        CREATE TABLE IF NOT EXISTS accounts (
            id            INTEGER PRIMARY KEY AUTOINCREMENT,
            username      TEXT    NOT NULL,
            uuid          TEXT    NOT NULL DEFAULT '',
            kind          TEXT    NOT NULL DEFAULT 'offline',
            access_token  TEXT    NOT NULL DEFAULT '',
            created_at    TEXT    NOT NULL
        );

        CREATE TABLE IF NOT EXISTS tasks (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            kind        TEXT    NOT NULL,
            target      TEXT    NOT NULL DEFAULT '',
            state       TEXT    NOT NULL DEFAULT 'pending',
            progress    REAL    NOT NULL DEFAULT 0,
            message     TEXT    NOT NULL DEFAULT '',
            created_at  TEXT    NOT NULL,
            updated_at  TEXT    NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_instances_pinned ON instances(pinned_home);
        CREATE INDEX IF NOT EXISTS idx_tasks_state ON tasks(state);
        "#,
    )?;
    Ok(())
}

/// v2：实例设置列（备注、内存上限、窗口宽高、实例级 Java）。
fn apply_v2(conn: &Connection) -> Result<()> {
    // SQLite 不支持 ALTER TABLE ... IF NOT EXISTS，先查一下列是否已在
    let mut existing: Vec<String> = Vec::new();
    {
        let mut stmt = conn.prepare("PRAGMA table_info(instances)")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
        for row in rows {
            existing.push(row?);
        }
    }
    let additions = [
        ("note", "TEXT NOT NULL DEFAULT ''"),
        ("memory_mb", "INTEGER NOT NULL DEFAULT 0"),
        ("window_width", "INTEGER NOT NULL DEFAULT 854"),
        ("window_height", "INTEGER NOT NULL DEFAULT 480"),
        ("java_path", "TEXT NOT NULL DEFAULT ''"),
    ];
    for (column, definition) in additions {
        if existing.iter().any(|name| name == column) {
            continue;
        }
        conn.execute_batch(&format!("ALTER TABLE instances ADD COLUMN {column} {definition};"))?;
    }
    Ok(())
}

/// v3：整合包列（packName 与 packState，普通实例保持空串）。
fn apply_v3(conn: &Connection) -> Result<()> {
    let mut existing: Vec<String> = Vec::new();
    {
        let mut stmt = conn.prepare("PRAGMA table_info(instances)")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
        for row in rows {
            existing.push(row?);
        }
    }
    for (column, definition) in [
        ("pack_name", "TEXT NOT NULL DEFAULT ''"),
        ("pack_state", "TEXT NOT NULL DEFAULT ''"),
    ] {
        if existing.iter().any(|name| name == column) {
            continue;
        }
        conn.execute_batch(&format!(
            "ALTER TABLE instances ADD COLUMN {column} {definition};"
        ))?;
    }
    Ok(())
}

/// v4：实例图标 spec（JSON 字符串，空串表示没设过）。
fn apply_v4(conn: &Connection) -> Result<()> {
    let mut existing: Vec<String> = Vec::new();
    {
        let mut stmt = conn.prepare("PRAGMA table_info(instances)")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
        for row in rows {
            existing.push(row?);
        }
    }
    if !existing.iter().any(|name| name == "icon_spec") {
        conn.execute_batch("ALTER TABLE instances ADD COLUMN icon_spec TEXT NOT NULL DEFAULT '';")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("gugu-db-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("gugu.db")
    }

    #[test]
    fn migrate_creates_schema_and_is_idempotent() {
        let path = temp_db();
        let _ = std::fs::remove_file(&path);

        let mut conn = open(&path).unwrap();
        let version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0)).unwrap();
        assert_eq!(version, SCHEMA_VERSION);

        for table in ["instances", "accounts", "tasks"] {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                    [table],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 1, "表 {table} 未创建");
        }

        // v2/v3 的实例列都要在
        for column in [
            "note",
            "memory_mb",
            "window_width",
            "window_height",
            "java_path",
            "pack_name",
            "pack_state",
        ] {
            let found: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM pragma_table_info('instances') WHERE name = ?1",
                    [column],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(found, 1, "列 {column} 未创建");
        }

        // 再跑一次不应报错、也不应重复建表
        migrate(&mut conn).unwrap();
        assert_eq!(
            conn.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0)).unwrap(),
            SCHEMA_VERSION
        );

        drop(conn);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }
}
