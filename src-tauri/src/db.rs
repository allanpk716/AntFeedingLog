//! SQLite 数据层：库文件打开 + 空→最新 schema 迁移。
//!
//! 设计约束（spec「架构与模块」节）：
//! - 数据层唯一属主是 Rust，前端一律经 Tauri command 读写；
//! - `journal_mode = DELETE`：无 -wal/-shm 伴生文件，备份 = 拷单个 .db（评审附录规则 11）；
//! - 迁移按 `PRAGMA user_version` 逐版本顺序执行，只升不降；
//! - 迁移入口只吃库路径，与 Tauri 解耦，可被 cargo test 直接覆盖。

use std::path::Path;

use rusqlite::Connection;

/// 当前 schema 版本。schema 变更时 +1，并在 `migrate` 的 match 里加对应分支。
/// v2：care_action 增加 is_feeding 标记位（R1 评审：喂食判定与名字解耦）。
pub const SCHEMA_VERSION: i64 = 2;

/// 库文件名，位于系统应用数据目录（Windows: `%APPDATA%\<identifier>\`）。
pub const DB_FILE_NAME: &str = "ant-feeding-log.db";

/// 数据层统一错误：目录/文件 IO 与 SQLite 两类。
#[derive(Debug)]
pub enum DbError {
    Io(std::io::Error),
    Sqlite(rusqlite::Error),
}

impl std::fmt::Display for DbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DbError::Io(e) => write!(f, "数据目录/文件 IO 失败: {e}"),
            DbError::Sqlite(e) => write!(f, "SQLite 错误: {e}"),
        }
    }
}

impl std::error::Error for DbError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            DbError::Io(e) => Some(e),
            DbError::Sqlite(e) => Some(e),
        }
    }
}

impl From<std::io::Error> for DbError {
    fn from(e: std::io::Error) -> Self {
        DbError::Io(e)
    }
}

impl From<rusqlite::Error> for DbError {
    fn from(e: rusqlite::Error) -> Self {
        DbError::Sqlite(e)
    }
}

pub type DbResult<T> = std::result::Result<T, DbError>;

/// 打开（必要时创建）库文件，设好连接级 PRAGMA，并迁移到最新 schema。
/// 父目录不存在会自动创建。
pub fn open_and_migrate(db_path: &Path) -> DbResult<Connection> {
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(db_path)?;
    // journal_mode 是查询型 PRAGMA（返回新模式），用 query_row 接住并断言生效。
    let mode: String =
        conn.query_row("PRAGMA journal_mode = DELETE", [], |row| row.get(0))?;
    if !mode.eq_ignore_ascii_case("delete") {
        return Err(rusqlite::Error::InvalidParameterName(format!(
            "journal_mode 应为 delete，实际为 {mode}"
        ))
        .into());
    }
    // 业务依赖外键语义（如"有记录的窝不可删"），SQLite 默认关闭，按连接开启。
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    migrate(&conn)?;
    Ok(conn)
}

/// 读取库的 schema 版本（`PRAGMA user_version`）。
pub fn schema_version_of(conn: &Connection) -> Result<i64, rusqlite::Error> {
    conn.query_row("PRAGMA user_version", [], |row| row.get(0))
}

/// 把库迁移到 [`SCHEMA_VERSION`]。幂等：已是最新则什么都不做；
/// 库版本高于应用支持时 fail-fast（报"请升级应用"，绝不静默放行，更不降级）。
pub fn migrate(conn: &Connection) -> DbResult<()> {
    let mut version = schema_version_of(conn)?;
    if version > SCHEMA_VERSION {
        return Err(rusqlite::Error::InvalidParameterName(format!(
            "库版本过新（schema v{version}，应用最高支持 v{SCHEMA_VERSION}），请升级应用后再打开"
        ))
        .into());
    }
    while version < SCHEMA_VERSION {
        match version {
            0 => migrate_v0_to_v1(conn)?,
            1 => migrate_v1_to_v2(conn)?,
            other => {
                return Err(rusqlite::Error::InvalidParameterName(format!(
                    "未知 schema 版本 v{other}"
                ))
                .into());
            }
        }
        version = schema_version_of(conn)?;
    }
    Ok(())
}

/// v0（空库）→ v1：建齐 spec「数据 schema」节全部表 + 预置数据，单事务原子完成。
fn migrate_v0_to_v1(conn: &Connection) -> Result<(), rusqlite::Error> {
    let tx = conn.unchecked_transaction()?;
    tx.execute_batch(V1_SCHEMA_SQL)?;
    seed_v1_presets(&tx)?;
    tx.pragma_update(None, "user_version", 1)?;
    tx.commit()
}

/// v1 → v2：care_action 增加 is_feeding 标记位（喂食判定与名字解耦，R1 评审方案 A），
/// 回填预置「喂食」行 = 1。单事务原子完成；v1 时期用户自建的操作回填为 0。
fn migrate_v1_to_v2(conn: &Connection) -> Result<(), rusqlite::Error> {
    let tx = conn.unchecked_transaction()?;
    tx.execute_batch(
        r#"
        ALTER TABLE care_action
            ADD COLUMN is_feeding INTEGER NOT NULL DEFAULT 0
            CHECK (is_feeding IN (0, 1));
        UPDATE care_action SET is_feeding = 1 WHERE name = '喂食';
        "#,
    )?;
    tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    tx.commit()
}

/// v1 预置数据：四操作（喂食/活动区换水/巢穴保湿/垃圾清理）、三食物、两地点、五项设置默认值。
fn seed_v1_presets(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        r#"
        INSERT INTO care_action (name, icon, kind, suggested_interval_days, enabled, sort) VALUES
            ('喂食',       NULL, 'reminding', 3,    1, 1),
            ('活动区换水', NULL, 'log_only',  NULL, 1, 2),
            ('巢穴保湿',   NULL, 'log_only',  NULL, 1, 3),
            ('垃圾清理',   NULL, 'reminding', 7,    1, 4);

        INSERT INTO food (name, enabled, sort) VALUES
            ('种子',   1, 1),
            ('干虾仁', 1, 2),
            ('面包虫', 1, 3);

        INSERT INTO location (name, enabled, sort) VALUES
            ('家',   1, 1),
            ('公司', 1, 2);

        INSERT INTO settings (key, value) VALUES
            ('notify_master_enabled',      '1'), -- 通知总开关（默认开）
            ('notify_overdue_enabled',     '1'), -- 超期提醒开关（默认开）
            ('notify_hibernation_enabled', '1'), -- 冬眠提醒开关（默认开）
            ('wake_remind_days_ahead',     '7'), -- 临近出眠提前天数（默认 7）
            ('autostart_enabled',          '1'); -- 开机自启（默认开）
        "#,
    )?;
    Ok(())
}

/// v1 全部建表 DDL。
const V1_SCHEMA_SQL: &str = r#"
-- 地点（预置：家、公司）
CREATE TABLE location (
    id      INTEGER PRIMARY KEY,
    name    TEXT    NOT NULL UNIQUE,
    enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    sort    INTEGER NOT NULL DEFAULT 0
);

-- 维护操作（性质：reminding=提醒 / log_only=仅登记）
CREATE TABLE care_action (
    id                      INTEGER PRIMARY KEY,
    name                    TEXT NOT NULL UNIQUE,
    icon                    TEXT,
    kind                    TEXT NOT NULL CHECK (kind IN ('reminding', 'log_only')),
    suggested_interval_days INTEGER, -- 仅提醒类有意义；登记类保留可编辑值以便日后切换
    enabled                 INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    sort                    INTEGER NOT NULL DEFAULT 0
);

-- 食物（喂食记录可多选）
CREATE TABLE food (
    id      INTEGER PRIMARY KEY,
    name    TEXT    NOT NULL UNIQUE,
    enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    sort    INTEGER NOT NULL DEFAULT 0
);

-- 窝（名字 trim 后全局唯一；trim 在应用层保证）
CREATE TABLE colony (
    id          INTEGER PRIMARY KEY,
    name        TEXT    NOT NULL UNIQUE,
    species     TEXT,
    location_id INTEGER REFERENCES location(id),
    start_date  TEXT    NOT NULL, -- ISO 日期
    status      TEXT    NOT NULL DEFAULT 'active'
                        CHECK (status IN ('active', 'hibernating', 'ended')),
    sort        INTEGER NOT NULL DEFAULT 0
);

-- 维护记录：发生时间与录入时间分开存（补录合法）
CREATE TABLE care_log (
    id          INTEGER PRIMARY KEY,
    colony_id   INTEGER NOT NULL REFERENCES colony(id),
    action_id   INTEGER NOT NULL REFERENCES care_action(id),
    occurred_at TEXT    NOT NULL, -- 发生时间（可补录过去）
    note        TEXT    NOT NULL DEFAULT '',
    created_at  TEXT    NOT NULL  -- 录入时间
);
CREATE INDEX idx_care_log_colony ON care_log(colony_id, occurred_at);
CREATE INDEX idx_care_log_action ON care_log(action_id);

-- 记录-食物多选关联（仅喂食记录有）
CREATE TABLE log_food (
    log_id  INTEGER NOT NULL REFERENCES care_log(id),
    food_id INTEGER NOT NULL REFERENCES food(id),
    PRIMARY KEY (log_id, food_id)
);

-- 冬眠期：actual_end_date 为空 = 开放段（回填即确认出眠）
CREATE TABLE hibernation (
    id                INTEGER PRIMARY KEY,
    colony_id         INTEGER NOT NULL REFERENCES colony(id),
    start_date        TEXT    NOT NULL,
    expected_end_date TEXT    NOT NULL,
    actual_end_date   TEXT, -- 空 = 开放段
    CHECK (expected_end_date >= start_date),
    CHECK (actual_end_date IS NULL OR actual_end_date >= start_date)
);
-- 每窝至多一段开放段
CREATE UNIQUE INDEX idx_hibernation_one_open_per_colony
    ON hibernation(colony_id) WHERE actual_end_date IS NULL;

-- 已发提醒台账：身份 = (窝, 种类, 操作, 基准日)
-- 超期类才有 action_id；SQLite 唯一约束视 NULL 互异，
-- 故用两条部分唯一索引分别覆盖"超期"与"冬眠类"两种去重。
CREATE TABLE reminder_ledger (
    id        INTEGER PRIMARY KEY,
    colony_id INTEGER NOT NULL REFERENCES colony(id),
    kind      TEXT    NOT NULL CHECK (kind IN ('overdue', 'approaching_wake', 'wake_day')),
    action_id INTEGER REFERENCES care_action(id), -- 仅 kind='overdue' 非空
    base_date TEXT    NOT NULL,
    sent_at   TEXT    NOT NULL
);
CREATE UNIQUE INDEX uq_ledger_overdue
    ON reminder_ledger(colony_id, action_id, base_date) WHERE kind = 'overdue';
CREATE UNIQUE INDEX uq_ledger_wake
    ON reminder_ledger(colony_id, base_date)
    WHERE kind IN ('approaching_wake', 'wake_day');

-- 设置（键值；预置行 = spec 默认配置）
CREATE TABLE settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
"#;

#[cfg(test)]
mod tests {
    use super::*;

    use rusqlite::{params, Connection};
    use tempfile::TempDir;

    /// 在临时目录的子路径里建全新库并迁移到最新。
    /// 刻意用两级子目录，顺带验证父目录不存在时会自动创建。
    fn fresh_conn() -> (Connection, TempDir) {
        let dir = TempDir::new().expect("创建临时目录失败");
        let conn = open_and_migrate(&dir.path().join("data").join(DB_FILE_NAME))
            .expect("空库迁移失败");
        (conn, dir)
    }

    fn scalar_i64(conn: &Connection, sql: &str) -> i64 {
        conn.query_row(sql, [], |row| row.get(0))
            .expect("标量查询失败")
    }

    fn table_names(conn: &Connection) -> Vec<String> {
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .expect("prepare 失败");
        stmt.query_map([], |row| row.get(0))
            .expect("query_map 失败")
            .collect::<Result<Vec<_>, _>>()
            .expect("读取表名失败")
    }

    fn settings_value(conn: &Connection, key: &str) -> String {
        conn.query_row(
            "SELECT value FROM settings WHERE key = ?1",
            params![key],
            |row| row.get(0),
        )
        .expect("读取设置失败")
    }

    #[test]
    fn empty_db_gets_all_tables() {
        let (conn, _dir) = fresh_conn();
        assert_eq!(
            table_names(&conn),
            vec![
                "care_action",
                "care_log",
                "colony",
                "food",
                "hibernation",
                "location",
                "log_food",
                "reminder_ledger",
                "settings",
            ]
        );
    }

    #[test]
    fn user_version_is_schema_version() {
        let (conn, _dir) = fresh_conn();
        assert_eq!(scalar_i64(&conn, "PRAGMA user_version"), SCHEMA_VERSION);
        assert_eq!(SCHEMA_VERSION, 2);
    }

    #[test]
    fn v2_fresh_db_has_is_feeding_with_preset_backfilled() {
        let (conn, _dir) = fresh_conn();
        // 列存在且 NOT NULL DEFAULT 0
        let cols: Vec<(String, i64, Option<String>)> = {
            let mut stmt = conn
                .prepare(
                    "SELECT name, \"notnull\", dflt_value FROM pragma_table_info('care_action')
                     WHERE name = 'is_feeding'",
                )
                .expect("prepare 失败");
            stmt.query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .expect("query_map 失败")
            .collect::<Result<Vec<_>, _>>()
            .expect("读取列信息失败")
        };
        assert_eq!(cols.len(), 1, "care_action 应有 is_feeding 列");
        assert_eq!(cols[0].1, 1, "is_feeding 应为 NOT NULL");

        // 回填：仅预置「喂食」=1，其余 =0
        let flagged: Vec<String> = {
            let mut stmt = conn
                .prepare("SELECT name FROM care_action WHERE is_feeding = 1")
                .expect("prepare 失败");
            stmt.query_map([], |row| row.get(0))
                .expect("query_map 失败")
                .collect::<Result<Vec<_>, _>>()
                .expect("读取失败")
        };
        assert_eq!(flagged, vec!["喂食"]);
        assert_eq!(
            scalar_i64(&conn, "SELECT COUNT(*) FROM care_action WHERE is_feeding = 0"),
            3
        );
    }

    #[test]
    fn v1_db_with_data_upgrades_to_v2_and_backfills() {
        // 手工搭一个 v1 库（旧 schema + 预置数据 + 用户在 v1 时期自建的操作 + 历史记录），
        // 版本停在 1，跑 migrate 应升到 v2 并正确回填 is_feeding。
        let conn = Connection::open_in_memory().expect("内存库打开失败");
        conn.execute_batch(V1_SCHEMA_SQL).expect("建 v1 schema 失败");
        seed_v1_presets(&conn).expect("灌 v1 预置数据失败");
        conn.execute(
            "INSERT INTO care_action (name, kind, enabled, sort) VALUES ('降温', 'log_only', 1, 5)",
            [],
        )
        .expect("自建操作失败");
        conn.execute(
            "INSERT INTO colony (name, start_date, status) VALUES ('大头一号', '2026-01-20', 'active')",
            [],
        )
        .expect("建窝失败");
        conn.execute(
            "INSERT INTO care_log (colony_id, action_id, occurred_at, note, created_at)
             VALUES (1, 1, '2026-09-01 08:00:00', '', '2026-09-01 08:00:00')",
            [],
        )
        .expect("建历史记录失败");
        conn.pragma_update(None, "user_version", 1).expect("置 v1 失败");

        migrate(&conn).expect("v1 → v2 升级失败");

        assert_eq!(scalar_i64(&conn, "PRAGMA user_version"), SCHEMA_VERSION);
        let flags: Vec<(String, i64)> = {
            let mut stmt = conn
                .prepare("SELECT name, is_feeding FROM care_action ORDER BY sort, id")
                .expect("prepare 失败");
            stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .expect("query_map 失败")
                .collect::<Result<Vec<_>, _>>()
                .expect("读取失败")
        };
        assert_eq!(
            flags,
            vec![
                ("喂食".into(), 1),
                ("活动区换水".into(), 0),
                ("巢穴保湿".into(), 0),
                ("垃圾清理".into(), 0),
                ("降温".into(), 0), // 用户自建操作不被误标
            ]
        );
        // 历史数据原样保留
        assert_eq!(scalar_i64(&conn, "SELECT COUNT(*) FROM care_log"), 1);
        // 升级后再 migrate 幂等
        migrate(&conn).expect("对 v2 库再次 migrate 不应失败");
    }

    #[test]
    fn journal_mode_is_delete() {
        let (conn, _dir) = fresh_conn();
        let mode: String = conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .expect("读取 journal_mode 失败");
        assert_eq!(mode.to_ascii_lowercase(), "delete");
    }

    #[test]
    fn seeds_four_care_actions_with_nature_and_interval() {
        let (conn, _dir) = fresh_conn();
        let mut stmt = conn
            .prepare("SELECT name, kind, suggested_interval_days FROM care_action ORDER BY sort")
            .expect("prepare 失败");
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                ))
            })
            .expect("query_map 失败")
            .collect::<Result<Vec<_>, _>>()
            .expect("读取操作失败");

        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0], ("喂食".into(), "reminding".into(), Some(3)));
        assert_eq!(rows[1], ("活动区换水".into(), "log_only".into(), None));
        assert_eq!(rows[2], ("巢穴保湿".into(), "log_only".into(), None));
        assert_eq!(rows[3], ("垃圾清理".into(), "reminding".into(), Some(7)));
    }

    #[test]
    fn seeds_three_foods() {
        let (conn, _dir) = fresh_conn();
        let mut stmt = conn
            .prepare("SELECT name FROM food ORDER BY sort")
            .expect("prepare 失败");
        let names = stmt
            .query_map([], |row| row.get(0))
            .expect("query_map 失败")
            .collect::<Result<Vec<String>, _>>()
            .expect("读取食物失败");
        assert_eq!(names, vec!["种子", "干虾仁", "面包虫"]);
    }

    #[test]
    fn seeds_two_locations() {
        let (conn, _dir) = fresh_conn();
        let mut stmt = conn
            .prepare("SELECT name FROM location ORDER BY sort")
            .expect("prepare 失败");
        let names = stmt
            .query_map([], |row| row.get(0))
            .expect("query_map 失败")
            .collect::<Result<Vec<String>, _>>()
            .expect("读取地点失败");
        assert_eq!(names, vec!["家", "公司"]);
    }

    #[test]
    fn seeds_settings_defaults() {
        let (conn, _dir) = fresh_conn();
        assert_eq!(scalar_i64(&conn, "SELECT COUNT(*) FROM settings"), 5);
        assert_eq!(settings_value(&conn, "notify_master_enabled"), "1");
        assert_eq!(settings_value(&conn, "notify_overdue_enabled"), "1");
        assert_eq!(settings_value(&conn, "notify_hibernation_enabled"), "1");
        assert_eq!(settings_value(&conn, "wake_remind_days_ahead"), "7");
        assert_eq!(settings_value(&conn, "autostart_enabled"), "1");
    }

    #[test]
    fn remigrate_is_noop() {
        let (conn, _dir) = fresh_conn();
        migrate(&conn).expect("对已迁移库再次 migrate 不应失败");
        assert_eq!(scalar_i64(&conn, "SELECT COUNT(*) FROM care_action"), 4);
        assert_eq!(scalar_i64(&conn, "SELECT COUNT(*) FROM food"), 3);
        assert_eq!(scalar_i64(&conn, "SELECT COUNT(*) FROM location"), 2);
        assert_eq!(scalar_i64(&conn, "SELECT COUNT(*) FROM settings"), 5);
        assert_eq!(scalar_i64(&conn, "PRAGMA user_version"), SCHEMA_VERSION);
    }

    #[test]
    fn db_newer_than_app_is_rejected_with_friendly_error() {
        // 停靠：库版本高于应用支持的 schema 版本时 fail-fast（报"请升级应用"），
        // 不再静默放行——旧应用读新库可能漏列/漏约束，写坏数据。
        let conn = Connection::open_in_memory().expect("内存库打开失败");
        conn.pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .expect("置未来版本失败");

        let err = migrate(&conn).unwrap_err().to_string();
        assert!(err.contains("过新"), "实际错误：{err}");
        assert!(err.contains("升级"), "实际错误：{err}");
        // 版本必须原样保留，绝不能被降级
        assert_eq!(
            scalar_i64(&conn, "PRAGMA user_version"),
            SCHEMA_VERSION + 1,
            "拒绝打开时不得改动库版本"
        );
    }

    #[test]
    fn reopening_existing_db_keeps_presets() {
        let dir = TempDir::new().expect("创建临时目录失败");
        let path = dir.path().join("data").join(DB_FILE_NAME);
        let _first = open_and_migrate(&path).expect("首次打开失败");
        let conn = open_and_migrate(&path).expect("二次打开失败");
        assert_eq!(scalar_i64(&conn, "SELECT COUNT(*) FROM care_action"), 4);
        assert_eq!(scalar_i64(&conn, "PRAGMA user_version"), SCHEMA_VERSION);
    }

    #[test]
    fn foreign_keys_are_enforced_on_opened_connections() {
        let (conn, _dir) = fresh_conn();
        let result = conn.execute(
            "INSERT INTO colony (name, species, location_id, start_date, status)
             VALUES (?1, NULL, 999, '2026-09-18', 'active')",
            params!["测试窝"],
        );
        assert!(result.is_err(), "引用不存在的地点应被外键约束拒绝");
    }
}
