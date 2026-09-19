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
/// 网页端首启向导完成键（webui-checkin 票 03）：缺行/脏值 = 未做（false）。
pub const K_WEBUI_WIZARD_DONE: &str = "webui_wizard_done";
/// Pushover 应用内凭据键（webui-checkin 票 11；票 02 的 v7 不预置行）：
/// 缺行 = 应用内未填，发送侧回落环境变量。明文入库已明示接受（规格 G）。
pub const K_PUSHOVER_USER: &str = "pushover_user";
pub const K_PUSHOVER_TOKEN: &str = "pushover_token";

/// 临近出眠提前天数默认值（spec 设置节）。
pub const DEFAULT_WAKE_AHEAD_DAYS: i64 = 7;

/// 设置模型（spec：通知总开关/超期开关/冬眠开关/临近出眠提前天数/开机自启；
/// webui-checkin 票 11 增 Pushover 应用内凭据两键）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppSettings {
    pub notify_master_enabled: bool,
    pub notify_overdue_enabled: bool,
    pub notify_hibernation_enabled: bool,
    pub wake_remind_days_ahead: i64,
    pub autostart_enabled: bool,
    /// Pushover 用户键（空串 = 未填，发送侧回落环境变量）。明文入库已明示接受。
    pub pushover_user: String,
    /// Pushover 应用令牌（空串 = 未填）。明文入库已明示接受。
    pub pushover_token: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        AppSettings {
            notify_master_enabled: true,
            notify_overdue_enabled: true,
            notify_hibernation_enabled: true,
            wake_remind_days_ahead: DEFAULT_WAKE_AHEAD_DAYS,
            autostart_enabled: true,
            pushover_user: String::new(),
            pushover_token: String::new(),
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

/// 自由文本键（票 11 凭据）：缺行回默认空串；值原样返回（trim 在保存侧统一做）。
fn read_string(conn: &Connection, key: &str, default: &str) -> Result<String, String> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            params![key],
            |row| row.get(0),
        )
        .optional()
        .map_err(db_err)?;
    Ok(raw.unwrap_or_else(|| default.to_string()))
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

/// 读全部设置；缺行/脏值回退默认。凭据两键明文随本命令到前端（桌面专属
/// IPC，不入网页端白名单，规格 H）。
pub fn get_settings(conn: &Connection) -> Result<AppSettings, String> {
    let d = AppSettings::default();
    Ok(AppSettings {
        notify_master_enabled: read_bool(conn, K_MASTER, d.notify_master_enabled)?,
        notify_overdue_enabled: read_bool(conn, K_OVERDUE, d.notify_overdue_enabled)?,
        notify_hibernation_enabled: read_bool(conn, K_HIBERNATION, d.notify_hibernation_enabled)?,
        wake_remind_days_ahead: read_i64(conn, K_WAKE_AHEAD, d.wake_remind_days_ahead)?,
        autostart_enabled: read_bool(conn, K_AUTOSTART, d.autostart_enabled)?,
        pushover_user: read_string(conn, K_PUSHOVER_USER, "")?,
        pushover_token: read_string(conn, K_PUSHOVER_TOKEN, "")?,
    })
}

/// 应用内 Pushover 凭据（webui-checkin 票 11）：缺行回空串 = 未填。
/// 返回 (user, token)；发送侧三态优先级见 pushover::resolve_pushover_credentials。
pub fn get_pushover_credentials(conn: &Connection) -> Result<(String, String), String> {
    Ok((
        read_string(conn, K_PUSHOVER_USER, "")?,
        read_string(conn, K_PUSHOVER_TOKEN, "")?,
    ))
}

/// 保存全部设置（整体覆盖式 upsert），返回收敛后的生效值。
/// 提前天数收敛到 0–365（票 09 停靠 B：服务端兜底，前端已先行校验同区间）。
/// 提前 0 天 = 出眠日当天才发临近提醒，与出眠日提醒同日各一条——schema v3 的
/// 冬眠侧唯一键已按种类区分。
/// 票 11：凭据两键落库前去首尾空白（粘贴事故防御）；整体覆盖语义不变——
/// 前端表单回显保证未动的键原值回写，其他键（如向导键）走独立函数不受影响。
pub fn set_settings(conn: &Connection, s: &AppSettings) -> Result<AppSettings, String> {
    let effective = AppSettings {
        wake_remind_days_ahead: s.wake_remind_days_ahead.clamp(0, 365),
        pushover_user: s.pushover_user.trim().to_string(),
        pushover_token: s.pushover_token.trim().to_string(),
        ..s.clone()
    };
    upsert(conn, K_MASTER, bool_str(effective.notify_master_enabled))?;
    upsert(conn, K_OVERDUE, bool_str(effective.notify_overdue_enabled))?;
    upsert(conn, K_HIBERNATION, bool_str(effective.notify_hibernation_enabled))?;
    upsert(conn, K_WAKE_AHEAD, &effective.wake_remind_days_ahead.to_string())?;
    upsert(conn, K_AUTOSTART, bool_str(effective.autostart_enabled))?;
    upsert(conn, K_PUSHOVER_USER, &effective.pushover_user)?;
    upsert(conn, K_PUSHOVER_TOKEN, &effective.pushover_token)?;
    Ok(effective)
}

fn bool_str(b: bool) -> &'static str {
    if b {
        "1"
    } else {
        "0"
    }
}

/// 网页端首启向导做过没有（webui-checkin 票 03）：缺行/脏值回退未做（false），
/// 升级老库首次启动会弹一次向导。独立函数不进 AppSettings——向导键与通知设置
/// 互不相干，整体覆盖式 set_settings 不碰它。
pub fn get_webui_wizard_done(conn: &Connection) -> Result<bool, String> {
    read_bool(conn, K_WEBUI_WIZARD_DONE, false)
}

/// 标记向导已处理（完成或跳过都写）：幂等 upsert '1'。
pub fn mark_webui_wizard_done(conn: &Connection) -> Result<(), String> {
    upsert(conn, K_WEBUI_WIZARD_DONE, "1")
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
        // webui-checkin 票 11：凭据键缺省空串 = 应用内未填（回落环境变量）
        assert_eq!(s.pushover_user, "");
        assert_eq!(s.pushover_token, "");
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
            pushover_user: "u-往返".into(),
            pushover_token: "t-往返".into(),
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

    #[test]
    fn webui_wizard_done_defaults_false_and_marks_once() {
        // webui-checkin 票 03：向导键缺省=未做；标记后 true；set_settings 不碰它
        let conn = mem_conn();
        assert!(!get_webui_wizard_done(&conn).unwrap(), "缺省 = 未做（老库升级首次启动会弹向导）");
        // 脏值回退未做
        conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, 'yes')",
            params![K_WEBUI_WIZARD_DONE],
        )
        .unwrap();
        assert!(!get_webui_wizard_done(&conn).unwrap(), "脏值回退未做");
        mark_webui_wizard_done(&conn).unwrap();
        assert!(get_webui_wizard_done(&conn).unwrap(), "标记后 = 已做");
        mark_webui_wizard_done(&conn).unwrap(); // 幂等 upsert
        assert!(get_webui_wizard_done(&conn).unwrap());

        // 整体覆盖式 set_settings 不冲掉向导键（独立函数的取舍）
        set_settings(&conn, &AppSettings::default()).unwrap();
        assert!(get_webui_wizard_done(&conn).unwrap(), "通知设置保存不碰向导键");
    }

    #[test]
    fn pushover_credentials_round_trip_and_missing_rows_default_empty() {
        // webui-checkin 票 11：应用内凭据随 set_settings 落库；get_pushover_credentials
        // 独立读入口与 get_settings 同口径（缺行回空串）
        let conn = mem_conn();
        assert_eq!(
            get_pushover_credentials(&conn).unwrap(),
            (String::new(), String::new()),
            "票 02 v7 不预置行：缺键 = 未填"
        );

        let input = AppSettings {
            pushover_user: "u-应用内".into(),
            pushover_token: "t-应用内".into(),
            ..Default::default()
        };
        let saved = set_settings(&conn, &input).unwrap();
        assert_eq!(saved.pushover_user, "u-应用内");
        assert_eq!(get_settings(&conn).unwrap().pushover_token, "t-应用内");
        assert_eq!(
            get_pushover_credentials(&conn).unwrap(),
            ("u-应用内".into(), "t-应用内".into())
        );
        // 落库就是 settings 键值表普通行（随备份/恢复走）
        assert_eq!(raw(&conn, K_PUSHOVER_USER), "u-应用内");
        assert_eq!(raw(&conn, K_PUSHOVER_TOKEN), "t-应用内");
    }

    #[test]
    fn pushover_values_are_trimmed_on_save() {
        // 首尾空白几乎必是粘贴事故：保存时统一去掉
        let conn = mem_conn();
        let input = AppSettings {
            pushover_user: "  u-key  ".into(),
            pushover_token: "\tt-token\n".into(),
            ..Default::default()
        };
        let saved = set_settings(&conn, &input).unwrap();
        assert_eq!(saved.pushover_user, "u-key");
        assert_eq!(saved.pushover_token, "t-token");
        assert_eq!(get_settings(&conn).unwrap().pushover_user, "u-key");
    }

    #[test]
    fn set_settings_with_pushover_keys_does_not_clobber_other_keys() {
        // 票 11 扩键回归：整体覆盖式保存带上凭据后，既有键（通知开关/向导键）原样保留
        let conn = mem_conn();
        mark_webui_wizard_done(&conn).unwrap();
        let first = AppSettings {
            notify_master_enabled: false,
            wake_remind_days_ahead: 3,
            autostart_enabled: false,
            pushover_user: "u-一次".into(),
            pushover_token: "t-一次".into(),
            ..Default::default()
        };
        set_settings(&conn, &first).unwrap();

        // 第二次保存只改通知开关，凭据键随表单原值回写（前端表单回显保证），不误伤
        let second = AppSettings { notify_master_enabled: true, ..first.clone() };
        set_settings(&conn, &second).unwrap();

        let s = get_settings(&conn).unwrap();
        assert!(s.notify_master_enabled);
        assert!(!s.autostart_enabled, "未动的键不被保存冲掉");
        assert_eq!(s.wake_remind_days_ahead, 3);
        assert_eq!(s.pushover_user, "u-一次", "凭据随表单原值回写不丢失");
        assert_eq!(s.pushover_token, "t-一次");
        assert!(get_webui_wizard_done(&conn).unwrap(), "扩键保存不冲掉向导键");

        // 清空凭据（前端把输入框清空再保存）→ 落空串 = 应用内未填，回落环境变量
        let cleared = AppSettings { pushover_user: String::new(), pushover_token: String::new(), ..second };
        set_settings(&conn, &cleared).unwrap();
        assert_eq!(get_pushover_credentials(&conn).unwrap(), (String::new(), String::new()));
    }
}
