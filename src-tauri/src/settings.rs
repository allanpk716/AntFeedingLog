//! 设置（键值）读写（票 06）。
//!
//! 全部函数只吃 `&Connection`，与 Tauri 解耦，可被 cargo test 直接覆盖；
//! Tauri command 只是薄包装（见 lib.rs）。
//!
//! 键名与预置行来自 schema v1（db.rs `seed_v1_presets`）；读取遇到缺失行或脏值
//! 回退默认值，绝不毒死提醒调度。布尔以 '1'/'0' 落库。

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

/// 设置键名（settings 表 key 列）。
pub const K_MASTER: &str = "notify_master_enabled";
pub const K_OVERDUE: &str = "notify_overdue_enabled";
pub const K_HIBERNATION: &str = "notify_hibernation_enabled";
pub const K_WAKE_AHEAD: &str = "wake_remind_days_ahead";
pub const K_AUTOSTART: &str = "autostart_enabled";

/// 临近出眠提前天数默认值（spec 设置节）。
pub const DEFAULT_WAKE_AHEAD_DAYS: i64 = 7;

/// 设置模型（spec：通知总开关/超期开关/冬眠开关/临近出眠提前天数/开机自启）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppSettings {
    pub notify_master_enabled: bool,
    pub notify_overdue_enabled: bool,
    pub notify_hibernation_enabled: bool,
    pub wake_remind_days_ahead: i64,
    pub autostart_enabled: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        AppSettings {
            notify_master_enabled: true,
            notify_overdue_enabled: true,
            notify_hibernation_enabled: true,
            wake_remind_days_ahead: DEFAULT_WAKE_AHEAD_DAYS,
            autostart_enabled: true,
        }
    }
}

fn db_err(e: rusqlite::Error) -> String {
    format!("数据库操作失败: {e}")
}

fn read_bool(conn: &Connection, key: &str, default: bool) -> Result<bool, String> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            params![key],
            |row| row.get(0),
        )
        .optional()
        .map_err(db_err)?;
    match raw.as_deref() {
        Some("1") => Ok(true),
        Some("0") => Ok(false),
        _ => Ok(default), // 缺行或脏值回退默认
    }
}

fn read_i64(conn: &Connection, key: &str, default: i64) -> Result<i64, String> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            params![key],
            |row| row.get(0),
        )
        .optional()
        .map_err(db_err)?;
    match raw {
        Some(s) => Ok(s.trim().parse::<i64>().unwrap_or(default)),
        None => Ok(default),
    }
}

fn upsert(conn: &Connection, key: &str, value: &str) -> Result<(), String> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )
    .map_err(db_err)?;
    Ok(())
}

/// 读全部设置；缺行/脏值回退默认。
pub fn get_settings(conn: &Connection) -> Result<AppSettings, String> {
    let d = AppSettings::default();
    Ok(AppSettings {
        notify_master_enabled: read_bool(conn, K_MASTER, d.notify_master_enabled)?,
        notify_overdue_enabled: read_bool(conn, K_OVERDUE, d.notify_overdue_enabled)?,
        notify_hibernation_enabled: read_bool(conn, K_HIBERNATION, d.notify_hibernation_enabled)?,
        wake_remind_days_ahead: read_i64(conn, K_WAKE_AHEAD, d.wake_remind_days_ahead)?,
        autostart_enabled: read_bool(conn, K_AUTOSTART, d.autostart_enabled)?,
    })
}

/// 保存全部设置（整体覆盖式 upsert），返回收敛后的生效值。
/// 提前天数收敛到 0–365（票 09 停靠 B：服务端兜底，前端已先行校验同区间）。
/// 提前 0 天 = 出眠日当天才发临近提醒，与出眠日提醒同日各一条——schema v3 的
/// 冬眠侧唯一键已按种类区分。
pub fn set_settings(conn: &Connection, s: &AppSettings) -> Result<AppSettings, String> {
    let effective = AppSettings {
        wake_remind_days_ahead: s.wake_remind_days_ahead.clamp(0, 365),
        ..s.clone()
    };
    upsert(conn, K_MASTER, bool_str(effective.notify_master_enabled))?;
    upsert(conn, K_OVERDUE, bool_str(effective.notify_overdue_enabled))?;
    upsert(conn, K_HIBERNATION, bool_str(effective.notify_hibernation_enabled))?;
    upsert(conn, K_WAKE_AHEAD, &effective.wake_remind_days_ahead.to_string())?;
    upsert(conn, K_AUTOSTART, bool_str(effective.autostart_enabled))?;
    Ok(effective)
}

fn bool_str(b: bool) -> &'static str {
    if b {
        "1"
    } else {
        "0"
    }
}

// ── 测试：只测外部行为（spec「Testing Decisions」）────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// 建内存库并迁移到最新 schema（票 01 地基）。
    fn mem_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("内存库打开失败");
        crate::db::migrate(&conn).expect("迁移失败");
        conn
    }

    fn raw(conn: &Connection, key: &str) -> String {
        conn.query_row(
            "SELECT value FROM settings WHERE key = ?1",
            params![key],
            |r| r.get(0),
        )
        .expect("读原始值失败")
    }

    #[test]
    fn fresh_db_defaults_match_preset_rows() {
        let conn = mem_conn();
        let s = get_settings(&conn).unwrap();
        assert_eq!(s, AppSettings::default());
        assert!(s.notify_master_enabled);
        assert!(s.notify_overdue_enabled);
        assert!(s.notify_hibernation_enabled);
        assert_eq!(s.wake_remind_days_ahead, 7);
        assert!(s.autostart_enabled);
    }

    #[test]
    fn set_then_get_round_trips_all_fields() {
        let conn = mem_conn();
        let input = AppSettings {
            notify_master_enabled: false,
            notify_overdue_enabled: false,
            notify_hibernation_enabled: true,
            wake_remind_days_ahead: 3,
            autostart_enabled: false,
        };
        let saved = set_settings(&conn, &input).unwrap();
        assert_eq!(saved, input);
        assert_eq!(get_settings(&conn).unwrap(), input);
        // 落库格式：'1'/'0'
        assert_eq!(raw(&conn, K_MASTER), "0");
        assert_eq!(raw(&conn, K_WAKE_AHEAD), "3");
    }

    #[test]
    fn negative_days_ahead_clamped_to_zero() {
        let conn = mem_conn();
        let saved = set_settings(&conn, &AppSettings { wake_remind_days_ahead: -2, ..Default::default() }).unwrap();
        assert_eq!(saved.wake_remind_days_ahead, 0);
        assert_eq!(get_settings(&conn).unwrap().wake_remind_days_ahead, 0);
    }

    #[test]
    fn days_ahead_clamped_into_0_365_range() {
        // 票 09 停靠 B：服务端收敛 0–365（前端已校验，这里兜底绕过前端的直调）
        let conn = mem_conn();
        let saved = set_settings(&conn, &AppSettings { wake_remind_days_ahead: 999, ..Default::default() }).unwrap();
        assert_eq!(saved.wake_remind_days_ahead, 365);
        assert_eq!(get_settings(&conn).unwrap().wake_remind_days_ahead, 365);

        let saved = set_settings(&conn, &AppSettings { wake_remind_days_ahead: -5, ..Default::default() }).unwrap();
        assert_eq!(saved.wake_remind_days_ahead, 0);

        // 边界值原样保留
        let saved = set_settings(&conn, &AppSettings { wake_remind_days_ahead: 365, ..Default::default() }).unwrap();
        assert_eq!(saved.wake_remind_days_ahead, 365);
    }

    #[test]
    fn missing_or_dirty_rows_fall_back_to_defaults() {
        // 手工删行/写脏值：读取不报错、回退默认（调度健壮性）
        let conn = mem_conn();
        conn.execute("DELETE FROM settings WHERE key = ?1", params![K_MASTER])
            .unwrap();
        conn.execute(
            "UPDATE settings SET value = 'yes' WHERE key = ?1",
            params![K_HIBERNATION],
        )
        .unwrap();
        conn.execute(
            "UPDATE settings SET value = 'abc' WHERE key = ?1",
            params![K_WAKE_AHEAD],
        )
        .unwrap();

        let s = get_settings(&conn).unwrap();
        assert!(s.notify_master_enabled, "缺行回退默认开");
        assert!(s.notify_hibernation_enabled, "脏值回退默认开");
        assert_eq!(s.wake_remind_days_ahead, 7, "脏值回退默认 7");
    }
}
