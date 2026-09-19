//! 数据元信息（键值表 data_meta，webui-checkin 票 02）。
//!
//! 跨表元数据的落点：数据版本计数器（票 06 接线）、未来同类元信息。
//! 与 settings 的分工：settings 是「用户可见的应用设置」（随设置 UI 读写），
//! data_meta 是「程序内部簿记」，永不进设置页。
//! 读取缺键回 None，绝不毒死调用方。

use rusqlite::{params, Connection, OptionalExtension};

fn db_err(e: rusqlite::Error) -> String {
    format!("数据库操作失败: {e}")
}

/// 读一个键；缺键为 None。
pub fn get(conn: &Connection, key: &str) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT value FROM data_meta WHERE key = ?1",
        params![key],
        |row| row.get(0),
    )
    .optional()
    .map_err(db_err)
}

/// 写一个键（upsert 覆盖）。
pub fn set(conn: &Connection, key: &str, value: &str) -> Result<(), String> {
    conn.execute(
        "INSERT INTO data_meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )
    .map_err(db_err)?;
    Ok(())
}

// ── 测试：只测外部行为（spec「Testing Decisions」）────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn mem_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("内存库打开失败");
        crate::db::migrate(&conn).expect("迁移失败");
        conn
    }

    #[test]
    fn missing_key_reads_as_none() {
        let conn = mem_conn();
        assert_eq!(get(&conn, "data_version").unwrap(), None);
    }

    #[test]
    fn set_then_get_round_trips_and_overwrites() {
        let conn = mem_conn();
        set(&conn, "data_version", "1").unwrap();
        assert_eq!(get(&conn, "data_version").unwrap(), Some("1".into()));

        // 同键再写 = 覆盖（计数器递增场景）
        set(&conn, "data_version", "2").unwrap();
        assert_eq!(get(&conn, "data_version").unwrap(), Some("2".into()));
        assert_eq!(get(&conn, "other_key").unwrap(), None, "键互不串");
    }
}
