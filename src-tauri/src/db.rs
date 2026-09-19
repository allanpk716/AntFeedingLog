//! SQLite 数据层：库文件打开 + 空→最新 schema 迁移。
//!
//! 设计约束（spec「架构与模块」节）：
//! - 数据层唯一属主是 Rust，前端一律经 Tauri command 读写；
//! - `journal_mode = DELETE`：无 -wal/-shm 伴生文件，备份 = 拷单个 .db（评审附录规则 11）；
//! - 迁移按 `PRAGMA user_version` 逐版本顺序执行，只升不降；
//! - 迁移入口只吃库路径，与 Tauri 解耦，可被 cargo test 直接覆盖。

use std::path::Path;

use rusqlite::{params, Connection};

/// 当前 schema 版本。schema 变更时 +1，并在 `migrate` 的 match 里加对应分支。
/// v2：care_action 增加 is_feeding 标记位（R1 评审：喂食判定与名字解耦）。
/// v3：reminder_ledger 冬眠侧唯一键把种类并入（窝,种类,基准日）——v2 是
///     (窝,基准日)，提前天数设 0 时临近/出眠日两种提醒同日互斥（票 01 停靠）。
/// v4：字典预置项保护位（反馈第二轮 F2，Q5）——care_action/food 各加 is_preset。
/// v5：提醒台账推送列（反馈第二轮 F4，Q3/Q4/Q8）——push_title/push_body = 发送当时
///     的通知文案快照（补发直接用、不重算），pushover_done = 手机侧已了结。
/// v6：双层喂食周期（反馈第二轮 F3）——food 加 suggested_interval_days（预置按名
///     回填）；reminder_ledger 整表重建加 food_id 维度（v1 的 kind CHECK 不含
///     'food_overdue'，重建时顺带去掉该 CHECK、改由应用层保证）。
/// v7：易腐撤食地基（票 01）——food 加 perishable/retrieval_hours（预置按名回填
///     24h）；care_action 重建把 'follow'（跟随喂食）加进 kind CHECK 并插入预置
///     「撤食」（撞名则原位升格，F1/F7）；升级库写 retrieval_baseline_at 存量基线
///     （全新安装不写，F4）；reminder_ledger 加 retrieval_due 每窝每日唯一索引。
pub const SCHEMA_VERSION: i64 = 7;

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
    let start = schema_version_of(conn)?;
    if start > SCHEMA_VERSION {
        return Err(rusqlite::Error::InvalidParameterName(format!(
            "库版本过新（schema v{start}，应用最高支持 v{SCHEMA_VERSION}），请升级应用后再打开"
        ))
        .into());
    }
    let mut version = start;
    while version < SCHEMA_VERSION {
        match version {
            0 => migrate_v0_to_v1(conn)?,
            1 => migrate_v1_to_v2(conn)?,
            2 => migrate_v2_to_v3(conn)?,
            3 => migrate_v3_to_v4(conn)?,
            4 => migrate_v4_to_v5(conn)?,
            5 => migrate_v5_to_v6(conn)?,
            // 存量基线只属于"升级库"：全新安装从 v0 起步，不写 retrieval_baseline_at（F4）
            6 => migrate_v6_to_v7(conn, start == 0)?,
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
/// 版本号钉死为 2（每级迁移只盖自己的章，测试可用它搭出真实的 v2 库）。
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
    tx.pragma_update(None, "user_version", 2)?;
    tx.commit()
}

/// v2 → v3：reminder_ledger 冬眠侧唯一键把种类并入（窝,种类,基准日）。
/// v2 的 uq_ledger_wake 是 (窝,基准日)——提前天数设 0 时临近/出眠日两种提醒同日、
/// 第二条被拒（票 01 停靠）；v3 改为 (窝,种类,基准日)，同日各一条，旧台账行原样
/// 保留。超期侧 (窝,操作,基准日) 本就正确，不动。单事务原子完成。
fn migrate_v2_to_v3(conn: &Connection) -> Result<(), rusqlite::Error> {
    let tx = conn.unchecked_transaction()?;
    tx.execute_batch(
        r#"
        DROP INDEX IF EXISTS uq_ledger_wake;
        CREATE UNIQUE INDEX uq_ledger_wake
            ON reminder_ledger(colony_id, kind, base_date) WHERE kind != 'overdue';
        "#,
    )?;
    tx.pragma_update(None, "user_version", 3)?;
    tx.commit()
}

/// v3 → v4：字典预置项保护位（反馈第二轮 F2，Q5）。care_action/food 各加 is_preset，
/// 按名字回填预置行（喂食/活动区换水/巢穴保湿/垃圾清理、种子/干虾仁/面包虫）。
/// 已改名且从未被引用的预置回填不到——已知边界（单人工具可接受，见共识文档）。
/// 新库走同一条路：v0→v1 seed 不带该列，v4 统一回填。
fn migrate_v3_to_v4(conn: &Connection) -> Result<(), rusqlite::Error> {
    let tx = conn.unchecked_transaction()?;
    tx.execute_batch(
        r#"
        ALTER TABLE care_action
            ADD COLUMN is_preset INTEGER NOT NULL DEFAULT 0 CHECK (is_preset IN (0, 1));
        UPDATE care_action SET is_preset = 1
            WHERE name IN ('喂食', '活动区换水', '巢穴保湿', '垃圾清理');

        ALTER TABLE food
            ADD COLUMN is_preset INTEGER NOT NULL DEFAULT 0 CHECK (is_preset IN (0, 1));
        UPDATE food SET is_preset = 1 WHERE name IN ('种子', '干虾仁', '面包虫');
        "#,
    )?;
    tx.pragma_update(None, "user_version", 4)?;
    tx.commit()
}

/// v5 → v?（F4）：台账加推送列。push_title/push_body = 发送当时的通知文案快照
/// （补发时直接用，不重算）；pushover_done = 手机侧已了结（已送达或超窗放弃）。
/// 历史行升级即了结（它们的桌面通知在当天已发过，不补手机）。
fn migrate_v4_to_v5(conn: &Connection) -> Result<(), rusqlite::Error> {
    let tx = conn.unchecked_transaction()?;
    tx.execute_batch(
        r#"
        ALTER TABLE reminder_ledger ADD COLUMN push_title TEXT;
        ALTER TABLE reminder_ledger ADD COLUMN push_body TEXT;
        ALTER TABLE reminder_ledger ADD COLUMN pushover_done INTEGER NOT NULL DEFAULT 0;
        UPDATE reminder_ledger SET pushover_done = 1;
        "#,
    )?;
    tx.pragma_update(None, "user_version", 5)?;
    tx.commit()
}

/// v6（F3）：① food 加 suggested_interval_days（按名回填 种子3/干虾仁7/面包虫7）；
/// ② reminder_ledger 整表重建——v1 的 kind CHECK 不含 'food_overdue' 且 SQLite 不能
/// 改列约束，顺带加 food_id 维度；kind 合法性改由应用层（compute 只产四种）保证。
/// 数据、推送列（v5）、既有唯一键语义全部原样保留。
fn migrate_v5_to_v6(conn: &Connection) -> Result<(), rusqlite::Error> {
    let tx = conn.unchecked_transaction()?;
    tx.execute_batch(
        r#"
        ALTER TABLE food ADD COLUMN suggested_interval_days INTEGER;
        UPDATE food SET suggested_interval_days = CASE name
            WHEN '种子' THEN 3
            WHEN '干虾仁' THEN 7
            WHEN '面包虫' THEN 7
        END
        WHERE name IN ('种子', '干虾仁', '面包虫');

        CREATE TABLE reminder_ledger_v6 (
            id             INTEGER PRIMARY KEY,
            colony_id      INTEGER NOT NULL REFERENCES colony(id),
            kind           TEXT    NOT NULL,
            action_id      INTEGER REFERENCES care_action(id),
            food_id        INTEGER REFERENCES food(id),
            base_date      TEXT    NOT NULL,
            sent_at        TEXT    NOT NULL,
            push_title     TEXT,
            push_body      TEXT,
            pushover_done  INTEGER NOT NULL DEFAULT 0
        );
        INSERT INTO reminder_ledger_v6
            (id, colony_id, kind, action_id, base_date, sent_at, push_title, push_body, pushover_done)
            SELECT id, colony_id, kind, action_id, base_date, sent_at, push_title, push_body, pushover_done
            FROM reminder_ledger;
        DROP TABLE reminder_ledger;
        ALTER TABLE reminder_ledger_v6 RENAME TO reminder_ledger;

        CREATE UNIQUE INDEX uq_ledger_overdue
            ON reminder_ledger(colony_id, action_id, base_date) WHERE kind = 'overdue';
        CREATE UNIQUE INDEX uq_ledger_food
            ON reminder_ledger(colony_id, action_id, food_id, base_date) WHERE kind = 'food_overdue';
        CREATE UNIQUE INDEX uq_ledger_wake
            ON reminder_ledger(colony_id, kind, base_date)
            WHERE kind IN ('approaching_wake', 'wake_day');
        "#,
    )?;
    tx.pragma_update(None, "user_version", 6)?;
    tx.commit()
}

/// 库内时间戳统一格式（与 care_log.occurred_at/created_at 同款，文本序 = 时间序）。
fn local_timestamp_now() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

/// v7（易腐撤食，票 01）：① food 加 perishable（0/1，默认 0）与 retrieval_hours
/// （可空；合法值 1–168 整数，0/负/非整数由应用层拒收），预置按名回填
/// 干虾仁/面包虫 → 易腐 24h（改名匹配不到为已知边界，同 v4 先例）；
/// ② care_action 整表重建——kind CHECK 加入第三性质 'follow'（跟随喂食；
/// SQLite 不能改列约束，v6 重建 reminder_ledger 同款先例），随后插入预置「撤食」
/// (sort=2，原 sort≥2 顺延 +1)；库中已有同名自建「撤食」时改为原位升格
/// （kind/is_preset/sort=2，id 与历史引用、停用态原样保留，不重复插入，F1/F7）；
/// ③ 升级库写 settings 键 retrieval_baseline_at = 迁移执行时刻（F4 存量基线）；
/// 全新安装（迁移起点 v0）不写——新库走同一条迁移链，由 fresh 分支跳过；
/// ④ reminder_ledger 加部分唯一索引 uq_ledger_retrieval（每窝每天至多一条
/// retrieval_due；v6 已去掉 kind CHECK，无需重建表）。
///
/// care_action 被 care_log / reminder_ledger 外键引用，父表重建须临时关外键
/// （PRAGMA foreign_keys 在事务内是 no-op，故在开事务前切换、提交后还原）；
/// 行按 id 原样拷贝，拷贝语义保证子表引用在还原后仍然成立。
fn migrate_v6_to_v7(
    conn: &Connection,
    fresh_install: bool,
) -> Result<(), rusqlite::Error> {
    migrate_v6_to_v7_at(conn, fresh_install, &local_timestamp_now())
}

/// [`migrate_v6_to_v7`] 的可注入时钟版（测试断言基线值 = 指定迁移时刻）。
fn migrate_v6_to_v7_at(
    conn: &Connection,
    fresh_install: bool,
    now: &str,
) -> Result<(), rusqlite::Error> {
    let fk_before: i64 = conn.query_row("PRAGMA foreign_keys", [], |row| row.get(0))?;
    conn.execute_batch("PRAGMA foreign_keys = OFF;")?;
    let tx = conn.unchecked_transaction()?;
    let result = (|| {
        tx.execute_batch(
            r#"
            ALTER TABLE food ADD COLUMN perishable INTEGER NOT NULL DEFAULT 0 CHECK (perishable IN (0, 1));
            ALTER TABLE food ADD COLUMN retrieval_hours INTEGER;
            UPDATE food SET perishable = 1, retrieval_hours = 24 WHERE name IN ('干虾仁', '面包虫');

            CREATE TABLE care_action_v7 (
                id                      INTEGER PRIMARY KEY,
                name                    TEXT NOT NULL UNIQUE,
                icon                    TEXT,
                kind                    TEXT NOT NULL CHECK (kind IN ('reminding', 'log_only', 'follow')),
                suggested_interval_days INTEGER, -- 仅提醒类有意义；登记类保留可编辑值以便日后切换
                enabled                 INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
                sort                    INTEGER NOT NULL DEFAULT 0,
                is_feeding              INTEGER NOT NULL DEFAULT 0 CHECK (is_feeding IN (0, 1)),
                is_preset               INTEGER NOT NULL DEFAULT 0 CHECK (is_preset IN (0, 1))
            );
            INSERT INTO care_action_v7
                (id, name, icon, kind, suggested_interval_days, enabled, sort, is_feeding, is_preset)
                SELECT id, name, icon, kind, suggested_interval_days, enabled, sort, is_feeding, is_preset
                FROM care_action;
            DROP TABLE care_action;
            ALTER TABLE care_action_v7 RENAME TO care_action;

            CREATE UNIQUE INDEX uq_ledger_retrieval
                ON reminder_ledger(colony_id, base_date) WHERE kind = 'retrieval_due';
            "#,
        )?;

        // 撞名分支（F1）：同名「撤食」已存在 → 原位升格（id 不变、停用态保持）；
        // 排序处理与预置插入分支同款（F7）：撤食归位 sort=2，其余 sort≥2 顺延 +1。
        let existing: i64 = tx.query_row(
            "SELECT COUNT(*) FROM care_action WHERE name = '撤食'",
            [],
            |row| row.get(0),
        )?;
        if existing > 0 {
            tx.execute(
                "UPDATE care_action SET kind = 'follow', is_preset = 1, sort = 2
                 WHERE name = '撤食'",
                [],
            )?;
            tx.execute(
                "UPDATE care_action SET sort = sort + 1 WHERE sort >= 2 AND name != '撤食'",
                [],
            )?;
        } else {
            tx.execute(
                "UPDATE care_action SET sort = sort + 1 WHERE sort >= 2",
                [],
            )?;
            tx.execute(
                "INSERT INTO care_action
                    (name, icon, kind, suggested_interval_days, enabled, is_feeding, is_preset, sort)
                 VALUES ('撤食', NULL, 'follow', NULL, 1, 0, 1, 2)",
                [],
            )?;
        }

        // 存量基线（F4）：升级库记迁移时刻；全新安装不写（视为无非存量喂食）。
        if !fresh_install {
            tx.execute(
                "INSERT INTO settings (key, value) VALUES ('retrieval_baseline_at', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![now],
            )?;
        }
        tx.pragma_update(None, "user_version", 7)
    })();
    match result {
        Ok(()) => tx.commit()?,
        Err(e) => {
            tx.rollback()?;
            conn.execute_batch(&format!("PRAGMA foreign_keys = {fk_before};"))?;
            return Err(e);
        }
    }
    conn.execute_batch(&format!("PRAGMA foreign_keys = {fk_before};"))?;
    Ok(())
}

/// v1 预置数据：四操作（喂食/活动区换水/巢穴保湿/垃圾清理）、三食物、两地点、五项设置默认值。
/// pub(crate)：restore.rs 的旧 schema 备份测试要搭真实 v1 库（票 04 验收 3）。
pub(crate) fn seed_v1_presets(conn: &Connection) -> Result<(), rusqlite::Error> {
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

/// v1 全部建表 DDL。pub(crate)：restore.rs 的旧 schema 备份测试要搭真实 v1 库（票 04 验收 3）。
pub(crate) const V1_SCHEMA_SQL: &str = r#"
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
        assert_eq!(SCHEMA_VERSION, 7);
    }

    /// 按真实迁移函数链手工搭到 v6 的库（票 01 v7 迁移测试地基）。
    fn v6_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("内存库打开失败");
        conn.execute_batch(V1_SCHEMA_SQL).expect("建 v1 schema 失败");
        seed_v1_presets(&conn).expect("灌 v1 预置数据失败");
        migrate_v1_to_v2(&conn).expect("升 v2 失败");
        migrate_v2_to_v3(&conn).expect("升 v3 失败");
        migrate_v3_to_v4(&conn).expect("升 v4 失败");
        migrate_v4_to_v5(&conn).expect("升 v5 失败");
        migrate_v5_to_v6(&conn).expect("升 v6 失败");
        conn
    }

    #[test]
    fn v5_adds_push_columns_and_settles_legacy_rows() {
        // 真实迁移链搭到 v4（v1→v2→v3→v4），再由 migrate() 升到最新
        let conn = Connection::open_in_memory().expect("内存库打开失败");
        conn.execute_batch(V1_SCHEMA_SQL).unwrap();
        seed_v1_presets(&conn).unwrap();
        migrate_v1_to_v2(&conn).unwrap();
        migrate_v2_to_v3(&conn).unwrap();
        migrate_v3_to_v4(&conn).unwrap();
        conn.execute("INSERT INTO colony (name, start_date) VALUES ('大头一号', '2026-01-20')", []).unwrap();
        conn.execute(
            "INSERT INTO reminder_ledger (colony_id, kind, action_id, base_date, sent_at)
             VALUES (1, 'overdue', 1, '2026-09-15', '2026-09-15 08:00:00')",
            [],
        )
        .unwrap();
        migrate_v4_to_v5(&conn).unwrap(); // 停在 v5，让 migrate() 接力 v6→v7

        migrate(&conn).unwrap();
        // 历史行只走过桌面通道：直接视为已了结，不参与补发
        let done: i64 = conn.query_row("SELECT pushover_done FROM reminder_ledger", [], |r| r.get(0)).unwrap();
        assert_eq!(done, 1);
        // 新列存在且可为空
        let (title, body): (Option<String>, Option<String>) = conn
            .query_row("SELECT push_title, push_body FROM reminder_ledger", [], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap();
        assert_eq!((title, body), (None, None));
    }

    #[test]
    fn v6_adds_food_interval_and_rebuilds_ledger_with_food_dimension() {
        // 真实迁移链搭到 v5（含 v4 的 is_preset 列），灌 v5 时期台账行，migrate() 升 v6→v7
        let conn = Connection::open_in_memory().expect("内存库打开失败");
        conn.execute_batch(V1_SCHEMA_SQL).unwrap();
        seed_v1_presets(&conn).unwrap();
        migrate_v1_to_v2(&conn).unwrap();
        migrate_v2_to_v3(&conn).unwrap();
        migrate_v3_to_v4(&conn).unwrap();
        migrate_v4_to_v5(&conn).unwrap();
        conn.execute(
            "INSERT INTO colony (name, start_date) VALUES ('大头一号', '2026-01-20')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO reminder_ledger (colony_id, kind, action_id, base_date, sent_at, push_title, push_body, pushover_done)
             VALUES (1, 'overdue', 1, '2026-09-15', '2026-09-15 08:00:00', '喂食超期', '正文', 1)",
            [],
        )
        .unwrap();

        migrate(&conn).unwrap();

        // 食物周期：预置三样按名回填，自建为 NULL
        let intervals: Vec<(String, Option<i64>)> = {
            let mut stmt = conn.prepare("SELECT name, suggested_interval_days FROM food ORDER BY sort").unwrap();
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().collect::<Result<Vec<_>, _>>().unwrap()
        };
        assert_eq!(intervals, vec![
            ("种子".into(), Some(3)), ("干虾仁".into(), Some(7)), ("面包虫".into(), Some(7)),
        ]);

        // 台账重建后：旧行与推送列原样保留 + 新 food_id 列（NULL）
        let row: (Option<i64>, Option<String>, i64) = conn
            .query_row("SELECT food_id, push_title, pushover_done FROM reminder_ledger", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap();
        assert_eq!(row, (None, Some("喂食超期".into()), 1));

        // food_overdue 维度的唯一键生效：同窝同操作同日、不同食物共存；同食物重复被拒
        let insert = |food: Option<i64>| {
            conn.execute(
                "INSERT INTO reminder_ledger (colony_id, kind, action_id, food_id, base_date, sent_at)
                 VALUES (1, 'food_overdue', 1, ?1, '2026-09-15', '2026-09-15 08:00:00')",
                params![food],
            )
        };
        assert_eq!(insert(Some(1)).unwrap(), 1);
        assert_eq!(insert(Some(2)).unwrap(), 1);
        assert!(insert(Some(1)).is_err());
    }

    #[test]
    fn v3_ledger_identity_is_four_tuple_allowing_same_day_wake_kinds() {
        // 票 01 停靠：v2 里临近/出眠日共用 (colony, base_date) 唯一键——提前天数设 0 时
        // 两种提醒同日、第二条被拒。v3 改为 spec 契约的四元组唯一（窝,种类,操作,基准日）。
        let (conn, _dir) = fresh_conn();
        conn.execute(
            "INSERT INTO colony (name, start_date) VALUES ('大头一号', '2026-01-20')",
            [],
        )
        .expect("建窝失败");

        let insert = |kind: &str, action: Option<i64>, base: &str| {
            conn.execute(
                "INSERT INTO reminder_ledger (colony_id, kind, action_id, base_date, sent_at)
                 VALUES (1, ?1, ?2, ?3, '2026-09-18 08:00:00')",
                params![kind, action, base],
            )
        };

        // 提前天数=0：临近与出眠日同日（同窝同基准日、种类不同）→ 两条共存
        assert_eq!(insert("approaching_wake", None, "2026-10-01").unwrap(), 1);
        assert_eq!(insert("wake_day", None, "2026-10-01").unwrap(), 1);

        // 同四元组重复仍被拒（去重兜底在库层）
        assert!(insert("wake_day", None, "2026-10-01").is_err());
        assert!(insert("approaching_wake", None, "2026-10-01").is_err());

        // 超期类：同窝同操作同基准日唯一；不同操作互不干扰
        assert_eq!(insert("overdue", Some(1), "2026-09-15").unwrap(), 1);
        assert!(insert("overdue", Some(1), "2026-09-15").is_err());
        assert_eq!(insert("overdue", Some(4), "2026-09-15").unwrap(), 1);

        // 出眠日改期后按新基准日重发：同种类不同基准日共存（重算的前提）
        assert_eq!(insert("wake_day", None, "2026-10-10").unwrap(), 1);
    }

    #[test]
    fn v2_ledger_rows_survive_migration_to_v3() {
        // 手工搭 v2 库（真实迁移链 v0→v1→v2），灌 v2 时期合法的台账行，升 v3 后原样保留。
        let conn = Connection::open_in_memory().expect("内存库打开失败");
        conn.execute_batch(V1_SCHEMA_SQL).expect("建 v1 schema 失败");
        seed_v1_presets(&conn).expect("灌 v1 预置数据失败");
        migrate_v1_to_v2(&conn).expect("升 v2 失败");
        conn.execute(
            "INSERT INTO colony (name, start_date) VALUES ('大头一号', '2026-01-20')",
            [],
        )
        .expect("建窝失败");
        conn.execute_batch(
            r#"
            INSERT INTO reminder_ledger (colony_id, kind, action_id, base_date, sent_at) VALUES
                (1, 'overdue',          1,    '2026-09-15', '2026-09-15 08:00:00'),
                (1, 'approaching_wake', NULL, '2026-10-01', '2026-09-24 08:00:00');
            "#,
        )
        .expect("灌台账失败");

        migrate(&conn).expect("v2 → v3 升级失败");

        assert_eq!(scalar_i64(&conn, "PRAGMA user_version"), SCHEMA_VERSION);
        assert_eq!(
            scalar_i64(&conn, "SELECT COUNT(*) FROM reminder_ledger"),
            2,
            "旧台账数据迁移后原样保留"
        );
        // 新索引生效：同日临近+出眠可共存（v2 索引下这组会互斥）
        conn.execute(
            "INSERT INTO reminder_ledger (colony_id, kind, action_id, base_date, sent_at)
             VALUES (1, 'wake_day', NULL, '2026-10-01', '2026-10-01 08:00:00')",
            [],
        )
        .expect("v3 下同日出眠日台账应可写入");
        // 旧的两条部分唯一索引已被替换：冬眠侧唯一键现在含种类列
        let wake_index_sql: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'index' AND name = 'uq_ledger_wake'",
                [],
                |row| row.get(0),
            )
            .expect("查 uq_ledger_wake 失败");
        assert!(
            wake_index_sql.contains("kind"),
            "v3 的 uq_ledger_wake 应把种类并入唯一键，实际：{wake_index_sql}"
        );
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
            4,
            "撤食预置同为非喂食类"
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
                ("撤食".into(), 0), // v7 预置撤食：非喂食类
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
    fn v4_flags_preset_actions_and_foods() {
        let (conn, _dir) = fresh_conn();
        let action_names: Vec<String> = {
            let mut stmt = conn.prepare("SELECT name FROM care_action WHERE is_preset = 1 ORDER BY sort").unwrap();
            stmt.query_map([], |r| r.get(0)).unwrap().collect::<Result<Vec<_>, _>>().unwrap()
        };
        assert_eq!(action_names, vec!["喂食", "撤食", "活动区换水", "巢穴保湿", "垃圾清理"]);
        let food_names: Vec<String> = {
            let mut stmt = conn.prepare("SELECT name FROM food WHERE is_preset = 1 ORDER BY sort").unwrap();
            stmt.query_map([], |r| r.get(0)).unwrap().collect::<Result<Vec<_>, _>>().unwrap()
        };
        assert_eq!(food_names, vec!["种子", "干虾仁", "面包虫"]);
    }

    #[test]
    fn v3_db_with_custom_rows_upgrades_to_v4_and_backfills() {
        // 真实 v3 库 + 用户自建行 + 一个改过名的预置（回填盲区，见共识文档边界）
        let conn = Connection::open_in_memory().expect("内存库打开失败");
        conn.execute_batch(V1_SCHEMA_SQL).unwrap();
        seed_v1_presets(&conn).unwrap();
        migrate_v1_to_v2(&conn).unwrap();
        migrate_v2_to_v3(&conn).unwrap();
        conn.execute("UPDATE care_action SET name = '换水' WHERE name = '活动区换水'", []).unwrap();
        conn.execute("INSERT INTO care_action (name, kind, enabled, sort) VALUES ('降温', 'log_only', 1, 5)", []).unwrap();
        conn.pragma_update(None, "user_version", 3).unwrap();

        migrate(&conn).unwrap();
        assert_eq!(scalar_i64(&conn, "PRAGMA user_version"), SCHEMA_VERSION);
        let flagged: Vec<String> = {
            let mut stmt = conn.prepare("SELECT name FROM care_action WHERE is_preset = 1 ORDER BY sort").unwrap();
            stmt.query_map([], |r| r.get(0)).unwrap().collect::<Result<Vec<_>, _>>().unwrap()
        };
        // 已改名的预置匹配不到（已知边界，接受）；自建项不误标；
        // 撤食预置由 v7 迁移插入（sort=2，列在喂食之后）
        assert_eq!(flagged, vec!["喂食", "撤食", "巢穴保湿", "垃圾清理"]);
    }

    #[test]
    fn v7_upgrade_preserves_rows_and_backfills_perishable_presets() {
        // 真实 v6 库：历史记录 + 台账 + 自建操作/食物齐全，升 v7 后数据原样、
        // 预置食物按名回填易腐 24h、撤食预置插入且其余顺延（票 01 验收 1）
        let conn = v6_conn();
        conn.execute(
            "INSERT INTO colony (name, start_date) VALUES ('大头一号', '2026-01-20')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO care_action (name, icon, kind, is_feeding, suggested_interval_days, enabled, is_preset, sort)
             VALUES ('降温', NULL, 'log_only', 0, NULL, 1, 0, 5)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO food (name, enabled, sort, suggested_interval_days, is_preset)
             VALUES ('糖水', 1, 4, NULL, 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO care_log (colony_id, action_id, occurred_at, note, created_at)
             VALUES (1, 1, '2026-09-01 08:00:00', '备注', '2026-09-01 08:00:00')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO log_food (log_id, food_id) VALUES (1, 2)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO reminder_ledger (colony_id, kind, action_id, base_date, sent_at)
             VALUES (1, 'overdue', 1, '2026-09-10', '2026-09-10 08:00:00')",
            [],
        )
        .unwrap();
        conn.pragma_update(None, "user_version", 6).unwrap();

        migrate(&conn).unwrap();
        assert_eq!(scalar_i64(&conn, "PRAGMA user_version"), SCHEMA_VERSION);

        // 数据原样保留
        assert_eq!(scalar_i64(&conn, "SELECT COUNT(*) FROM care_log"), 1);
        assert_eq!(scalar_i64(&conn, "SELECT COUNT(*) FROM log_food"), 1);
        assert_eq!(scalar_i64(&conn, "SELECT COUNT(*) FROM reminder_ledger"), 1);
        assert_eq!(
            scalar_i64(&conn, "SELECT COUNT(*) FROM settings WHERE key = 'retrieval_baseline_at'"),
            1,
            "升级库写存量基线键"
        );

        // 食物按名回填：预置两样易腐 24h，种子/自建不动
        let foods: Vec<(String, i64, Option<i64>)> = {
            let mut stmt = conn
                .prepare("SELECT name, perishable, retrieval_hours FROM food ORDER BY sort")
                .unwrap();
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        assert_eq!(
            foods,
            vec![
                ("种子".into(), 0, None),
                ("干虾仁".into(), 1, Some(24)),
                ("面包虫".into(), 1, Some(24)),
                ("糖水".into(), 0, None),
            ]
        );

        // 撤食预置插入（follow、sort=2、预置守护），原 sort≥2 顺延 +1
        let actions: Vec<(String, String, Option<i64>, i64, i64, i64)> = {
            let mut stmt = conn
                .prepare(
                    "SELECT name, kind, suggested_interval_days, enabled, is_preset, sort
                     FROM care_action ORDER BY sort",
                )
                .unwrap();
            stmt.query_map([], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
        };
        assert_eq!(
            actions,
            vec![
                ("喂食".into(), "reminding".into(), Some(3), 1, 1, 1),
                ("撤食".into(), "follow".into(), None, 1, 1, 2),
                ("活动区换水".into(), "log_only".into(), None, 1, 1, 3),
                ("巢穴保湿".into(), "log_only".into(), None, 1, 1, 4),
                ("垃圾清理".into(), "reminding".into(), Some(7), 1, 1, 5),
                ("降温".into(), "log_only".into(), None, 1, 0, 6),
            ]
        );

        // 撤食预置不可删（沿用 is_preset 守护——字典层同一条 SQL，这里验证行标记）
        assert_eq!(
            scalar_i64(&conn, "SELECT is_preset FROM care_action WHERE name = '撤食'"),
            1
        );
    }

    #[test]
    fn v7_renamed_preset_food_misses_perishable_backfill() {
        // 已改名的预置匹配不到（已知边界，同 v4/v6 按名回填先例）
        let conn = v6_conn();
        conn.execute("UPDATE food SET name = '黄粉虫' WHERE name = '面包虫'", []).unwrap();
        conn.pragma_update(None, "user_version", 6).unwrap();

        migrate(&conn).unwrap();
        let (perishable, hours): (i64, Option<i64>) = conn
            .query_row(
                "SELECT perishable, retrieval_hours FROM food WHERE name = '黄粉虫'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((perishable, hours), (0, None), "改名预置不回填");
    }

    #[test]
    fn v7_collision_with_custom_retrieval_upgrades_row_in_place() {
        // 验收 2：旧库已自建「撤食」→ 原位升格（id 不变、历史引用不断、无重复行、
        // 停用态保持、sort=2 归位且其它操作顺延）
        let conn = v6_conn();
        conn.execute(
            "INSERT INTO colony (name, start_date) VALUES ('大头一号', '2026-01-20')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO care_action (name, icon, kind, is_feeding, suggested_interval_days, enabled, is_preset, sort)
             VALUES ('撤食', NULL, 'log_only', 0, NULL, 0, 0, 5)",
            [],
        )
        .unwrap();
        let custom_id: i64 = conn
            .query_row("SELECT id FROM care_action WHERE name = '撤食'", [], |r| r.get(0))
            .unwrap();
        conn.execute(
            "INSERT INTO care_log (colony_id, action_id, occurred_at, note, created_at)
             VALUES (1, 1, '2026-09-01 08:00:00', '', '2026-09-01 08:00:00')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO care_log (colony_id, action_id, occurred_at, note, created_at)
             VALUES (1, ?1, '2026-09-02 08:00:00', '', '2026-09-02 08:00:00')",
            params![custom_id],
        )
        .unwrap();
        conn.pragma_update(None, "user_version", 6).unwrap();

        migrate(&conn).unwrap();

        // 无重复行，同一 id 原位升格
        assert_eq!(
            scalar_i64(&conn, "SELECT COUNT(*) FROM care_action WHERE name = '撤食'"),
            1
        );
        let row: (i64, String, i64, i64, i64) = conn
            .query_row(
                "SELECT id, kind, is_preset, enabled, sort FROM care_action WHERE name = '撤食'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .unwrap();
        assert_eq!(row.0, custom_id, "id 不变，历史引用不断");
        assert_eq!(row.1, "follow");
        assert_eq!(row.2, 1, "升格为预置");
        assert_eq!(row.3, 0, "原停用态保持");
        assert_eq!(row.4, 2, "sort=2 归位");

        // 历史引用仍然成立 + 其它操作顺延
        assert_eq!(scalar_i64(&conn, "SELECT COUNT(*) FROM care_log"), 2);
        let sorts: Vec<(String, i64)> = {
            let mut stmt = conn
                .prepare("SELECT name, sort FROM care_action ORDER BY sort")
                .unwrap();
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        assert_eq!(
            sorts,
            vec![
                ("喂食".into(), 1),
                ("撤食".into(), 2),
                ("活动区换水".into(), 3),
                ("巢穴保湿".into(), 4),
                ("垃圾清理".into(), 5),
            ]
        );
    }

    #[test]
    fn v7_retrieval_due_ledger_unique_per_colony_per_day() {
        let conn = v6_conn();
        conn.execute(
            "INSERT INTO colony (name, start_date) VALUES ('大头一号', '2026-01-20')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO colony (name, start_date) VALUES ('针毛一号', '2026-02-01')",
            [],
        )
        .unwrap();
        conn.pragma_update(None, "user_version", 6).unwrap();
        migrate(&conn).unwrap();

        let insert = |colony: i64, base: &str| {
            conn.execute(
                "INSERT INTO reminder_ledger (colony_id, kind, base_date, sent_at)
                 VALUES (?1, 'retrieval_due', ?2, '2026-09-19 08:00:00')",
                params![colony, base],
            )
        };
        assert_eq!(insert(1, "2026-09-19").unwrap(), 1);
        // 同窝同日第二条被拒（部分唯一索引，库层去重兜底）
        assert!(insert(1, "2026-09-19").is_err());
        // 不同窝、不同基准日照常各一条
        assert_eq!(insert(2, "2026-09-19").unwrap(), 1);
        assert_eq!(insert(1, "2026-09-20").unwrap(), 1);
    }

    #[test]
    fn v7_follow_kind_accepted_but_nonsense_rejected() {
        let conn = v6_conn();
        conn.pragma_update(None, "user_version", 6).unwrap();
        migrate(&conn).unwrap();

        // follow 通过重建后的 CHECK
        conn.execute(
            "INSERT INTO care_action (name, kind, enabled, sort) VALUES ('跟随样例', 'follow', 1, 9)",
            [],
        )
        .unwrap();
        // CHECK 仍拦非法值
        assert!(conn
            .execute(
                "INSERT INTO care_action (name, kind, enabled, sort) VALUES ('坏性质', 'remind', 1, 10)",
                [],
            )
            .is_err());
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
    fn seeds_five_care_actions_with_retrieval_preset_in_place() {
        let (conn, _dir) = fresh_conn();
        let mut stmt = conn
            .prepare("SELECT name, kind, suggested_interval_days, is_preset FROM care_action ORDER BY sort")
            .expect("prepare 失败");
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })
            .expect("query_map 失败")
            .collect::<Result<Vec<_>, _>>()
            .expect("读取操作失败");

        assert_eq!(rows.len(), 5);
        assert_eq!(rows[0], ("喂食".into(), "reminding".into(), Some(3), 1));
        assert_eq!(
            rows[1],
            ("撤食".into(), "follow".into(), None, 1),
            "撤食预置 sort=2、跟随喂食性质"
        );
        assert_eq!(rows[2], ("活动区换水".into(), "log_only".into(), None, 1));
        assert_eq!(rows[3], ("巢穴保湿".into(), "log_only".into(), None, 1));
        assert_eq!(rows[4], ("垃圾清理".into(), "reminding".into(), Some(7), 1));
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
    fn seeds_perishable_defaults_on_preset_foods() {
        // 票 01：种子不易腐；干虾仁/面包虫易腐 + 24h（全新安装走迁移链回填，同一条路）
        let (conn, _dir) = fresh_conn();
        let mut stmt = conn
            .prepare("SELECT name, perishable, retrieval_hours FROM food ORDER BY sort")
            .expect("prepare 失败");
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                ))
            })
            .expect("query_map 失败")
            .collect::<Result<Vec<_>, _>>()
            .expect("读取食物失败");
        assert_eq!(
            rows,
            vec![
                ("种子".into(), 0, None),
                ("干虾仁".into(), 1, Some(24)),
                ("面包虫".into(), 1, Some(24)),
            ]
        );
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
        assert_eq!(scalar_i64(&conn, "SELECT COUNT(*) FROM care_action"), 5);
        assert_eq!(scalar_i64(&conn, "SELECT COUNT(*) FROM food"), 3);
        assert_eq!(scalar_i64(&conn, "SELECT COUNT(*) FROM location"), 2);
        // 全新安装不写存量基线键（F4）
        assert_eq!(scalar_i64(&conn, "SELECT COUNT(*) FROM settings"), 5);
        assert!(conn
            .query_row(
                "SELECT COUNT(*) FROM settings WHERE key = 'retrieval_baseline_at'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map(|n| n == 0)
            .expect("查基线键失败"));
        assert_eq!(scalar_i64(&conn, "PRAGMA user_version"), SCHEMA_VERSION);
    }

    #[test]
    fn upgraded_db_writes_retrieval_baseline() {
        // v6 旧库升级 → 写入存量基线键；值 = 迁移执行时刻（注入时钟断言精确值）
        let conn = v6_conn();
        conn.pragma_update(None, "user_version", 6).unwrap();
        migrate_v6_to_v7_at(&conn, false, "2026-09-19 12:34:56").unwrap();
        assert_eq!(
            settings_value(&conn, "retrieval_baseline_at"),
            "2026-09-19 12:34:56"
        );
        // 幂等：已是 v7 后再走完整 migrate 不重跑迁移、基线值不被改动
        migrate(&conn).unwrap();
        assert_eq!(
            settings_value(&conn, "retrieval_baseline_at"),
            "2026-09-19 12:34:56"
        );
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
        assert_eq!(scalar_i64(&conn, "SELECT COUNT(*) FROM care_action"), 5);
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
