//! 字典管理：维护操作与食物的增改、停用/启用、删除与操作性质设置（票 04）。
//!
//! 全部函数只吃 `&Connection`，与 Tauri 解耦，可被 cargo test 直接覆盖；
//! Tauri command 只是薄包装（见 lib.rs）。
//!
//! 行为对齐 spec 评审附录规则 10：
//! - 改名只改显示名（id 不变），历史记录跟随新显示名；
//! - 未被引用可物理删；被引用只能停用（操作被 care_log / reminder_ledger /
//!   每窝周期行引用、食物被 log_food 引用时拒绝删除）；
//! - 停用项不出现在新建记录入口（log_care 已拒停用项，list 由前端过滤 enabled）；
//! - set_action_policy：切换 提醒/仅登记 性质 + 建议间隔；切到登记类时保留间隔值
//!   （spec：登记类也保留可编辑值以便日后切换）；is_feeding 位可编辑、不强制全局唯一。

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

/// 操作性质（schema CHECK 同款）。
/// - reminding：提醒类（超期标红可通知）
/// - log_only：登记类（只记录、永不催促）
/// - follow：跟随喂食（撤食预置专属，票 01）——由易腐喂食派生「该撤食」，不参与
///   提醒/登记切换、无建议间隔；用户界面不提供该性质的编辑入口。
pub const KINDS: [&str; 3] = ["reminding", "log_only", "follow"];

// ── DTO ──────────────────────────────────────────────────────────────────

/// 维护操作字典项（含停用的；referenced=被历史记录/提醒台账引用，只能停用不能删）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CareAction {
    pub id: i64,
    pub name: String,
    pub icon: Option<String>,
    pub kind: String, // reminding | log_only
    /// 是否喂食类操作（记账时带食物多选；可编辑、不强制全局唯一，界面提示自理）。
    pub is_feeding: bool,
    pub suggested_interval_days: Option<i64>,
    pub enabled: bool,
    pub sort: i64,
    /// 预置项禁删，可停用（反馈第二轮 F2）。
    pub is_preset: bool,
    pub referenced: bool,
}

/// 新增/修改操作的入参：id 为空=新增，否则改名+性质+间隔+排序（停用走单独命令）。
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ActionInput {
    pub id: Option<i64>,
    pub name: String,
    pub kind: String,
    pub is_feeding: bool,
    pub suggested_interval_days: Option<i64>,
    pub sort: i64,
}

/// set_action_policy 入参：性质 + 建议间隔（None=保留现值，切登记类不清空）
/// + 可选 is_feeding（None=不改）。
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ActionPolicyInput {
    pub kind: String,
    pub suggested_interval_days: Option<i64>,
    pub is_feeding: Option<bool>,
}

/// 食物大类合法域（ADR 0007）：固定三种，'seed'=种子 | 'protein'=蛋白质 | 'sugar'=糖水。
/// 大类不可增删改名，只作分组容器；可扩展的是大类之下的食物项。
pub const FOOD_CATEGORIES: [&str; 3] = ["seed", "protein", "sugar"];

/// 新增/修改食物的入参：id 为空=新增，否则改名+排序（停用走单独命令）。
/// v14（ADR 0008）起食物不再有各自的建议间隔——提醒粒度在大类，
/// 周期见 save_food_category_interval。
/// perishable = 易腐（票 01）；开易腐必须配 1–168 整数小时的撤食间隔（缺失/越界
/// 拒收），关易腐时间隔一律落 NULL（清空由前端触发，后端兜底归空）。
/// category = 食物大类 key（ADR 0007）：新增必填（None 拒收）；修改时 None =
/// 保留原大类（前端恒传当前值，缺省仅为兼容旧调用）。
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct FoodInput {
    pub id: Option<i64>,
    pub name: String,
    pub sort: i64,
    /// 兼容旧调用：缺省视为不易腐。
    #[serde(default)]
    pub perishable: bool,
    /// 撤食间隔（小时）；仅易腐有意义。
    #[serde(default)]
    pub retrieval_hours: Option<i64>,
    /// 食物大类 key：'seed' | 'protein' | 'sugar'（ADR 0007）。
    #[serde(default)]
    pub category: Option<String>,
}

fn db_err(e: rusqlite::Error) -> String {
    format!("数据库操作失败: {e}")
}

/// 唯一约束兜底报错转友好文案（与 colony.rs 同模式：按扩展错误码识别）。
fn friendly_unique_err(e: rusqlite::Error, what: &str, name: &str) -> String {
    if let rusqlite::Error::SqliteFailure(ffi, message) = &e {
        if ffi.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
            && message.as_deref().is_some_and(|m| m.contains(what))
        {
            return format!("「{name}」已存在");
        }
    }
    db_err(e)
}

fn validate_name(name: &str, what: &str) -> Result<String, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(format!("{what}名字不能为空"));
    }
    Ok(trimmed.to_string())
}

fn validate_kind(kind: &str) -> Result<(), String> {
    if KINDS.contains(&kind) {
        Ok(())
    } else {
        Err(format!(
            "无效的操作性质：{kind}（应为 reminding / log_only / follow）"
        ))
    }
}

fn validate_interval(days: Option<i64>) -> Result<(), String> {
    match days {
        None => Ok(()),
        Some(n) if n >= 1 => Ok(()),
        Some(n) => Err(format!("建议间隔应是不小于 1 的天数（收到 {n}）")),
    }
}

/// 食物大类守护（ADR 0007）：给了就必须是三种合法 key 之一；None 通过
/// （新增路径的必填检查在 save_food 里按 id 分支做）。
fn validate_food_category(category: Option<&str>) -> Result<(), String> {
    match category {
        None => Ok(()),
        Some(c) if FOOD_CATEGORIES.contains(&c) => Ok(()),
        Some(c) => Err(format!(
            "无效的食物大类：{c}（应为 seed / protein / sugar）"
        )),
    }
}

/// 撤食间隔守护（票 01）：开易腐必须给 1–168 的整数小时（缺失/0/负/越界拒收）；
/// 关易腐不校验——间隔值会被归空落库（前端清空、后端兜底，schema 允许 NULL）。
/// 合法值上限 168 = 一周（spec D 决策：撤食间隔按小时计，1–168 整数）。
fn validate_retrieval_hours(perishable: bool, hours: Option<i64>) -> Result<(), String> {
    if !perishable {
        return Ok(());
    }
    match hours {
        Some(h) if (1..=168).contains(&h) => Ok(()),
        Some(h) => Err(format!("撤食间隔应是 1–168 的整数小时（收到 {h}）")),
        None => Err("开易腐必须填写撤食间隔（1–168 的整数小时）".into()),
    }
}

// ── 操作 ─────────────────────────────────────────────────────────────────

const ACTION_SQL: &str = concat!(
    "SELECT a.id, a.name, a.icon, a.kind, a.is_feeding, a.suggested_interval_days, a.enabled, a.sort, a.is_preset, ",
    "(EXISTS(SELECT 1 FROM care_log l WHERE l.action_id = a.id) ",
    "OR EXISTS(SELECT 1 FROM reminder_ledger g WHERE g.action_id = a.id)) ",
    "FROM care_action a ",
);

fn row_to_action(row: &rusqlite::Row<'_>) -> rusqlite::Result<CareAction> {
    Ok(CareAction {
        id: row.get(0)?,
        name: row.get(1)?,
        icon: row.get(2)?,
        kind: row.get(3)?,
        is_feeding: row.get::<_, i64>(4)? != 0,
        suggested_interval_days: row.get(5)?,
        enabled: row.get::<_, i64>(6)? != 0,
        sort: row.get(7)?,
        is_preset: row.get::<_, i64>(8)? != 0,
        referenced: row.get::<_, i64>(9)? != 0,
    })
}

/// 全部操作（含停用的，与 list_locations 同口径；卡片/新建入口由调用方过滤 enabled）。
/// `referenced` = 被 care_log 或 reminder_ledger 引用：删除会被拒，只能停用（规则 10）。
pub fn list_actions(conn: &Connection) -> Result<Vec<CareAction>, String> {
    let mut stmt = conn
        .prepare(&format!("{ACTION_SQL}ORDER BY a.sort, a.id"))
        .map_err(db_err)?;
    let rows = stmt
        .query_map([], row_to_action)
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?;
    Ok(rows)
}

fn get_action(conn: &Connection, id: i64) -> Result<CareAction, String> {
    conn.query_row(
        &format!("{ACTION_SQL}WHERE a.id = ?1"),
        params![id],
        row_to_action,
    )
    .map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => "操作不存在".to_string(),
        other => db_err(other),
    })
}

/// 新增（enabled=1，icon 置空）或修改（改名+性质+间隔+排序；enabled/icon 不在编辑面）。
pub fn save_action(conn: &Connection, input: &ActionInput) -> Result<CareAction, String> {
    let name = validate_name(&input.name, "操作")?;
    validate_kind(&input.kind)?;
    validate_interval(input.suggested_interval_days)?;

    let dupes: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM care_action WHERE name = ?1 AND (?2 IS NULL OR id != ?2)",
            params![name, input.id],
            |row| row.get(0),
        )
        .map_err(db_err)?;
    if dupes > 0 {
        return Err(format!("操作名字「{name}」已存在"));
    }

    match input.id {
        None => {
            conn.execute(
                "INSERT INTO care_action (name, icon, kind, is_feeding, suggested_interval_days, enabled, sort)
                 VALUES (?1, NULL, ?2, ?3, ?4, 1, ?5)",
                params![name, input.kind, input.is_feeding, input.suggested_interval_days, input.sort],
            )
            .map_err(|e| friendly_unique_err(e, "care_action.name", &name))?;
            get_action(conn, conn.last_insert_rowid())
        }
        Some(id) => {
            let changed = conn
                .execute(
                    "UPDATE care_action
                     SET name = ?1, kind = ?2, is_feeding = ?3, suggested_interval_days = ?4, sort = ?5
                     WHERE id = ?6",
                    params![name, input.kind, input.is_feeding, input.suggested_interval_days, input.sort, id],
                )
                .map_err(|e| friendly_unique_err(e, "care_action.name", &name))?;
            if changed == 0 {
                return Err("操作不存在".into());
            }
            get_action(conn, id)
        }
    }
}

/// 停用/启用（enabled=false 即规则 10 的「停用」；停用项不进新建记录入口，log_care 已拒）。
pub fn set_action_enabled(conn: &Connection, id: i64, enabled: bool) -> Result<(), String> {
    let changed = conn
        .execute(
            "UPDATE care_action SET enabled = ?1 WHERE id = ?2",
            params![enabled, id],
        )
        .map_err(db_err)?;
    if changed == 0 {
        return Err("操作不存在".into());
    }
    Ok(())
}

/// 物理删；预置项禁删（反馈第二轮 F2）；被 care_log、reminder_ledger 或每窝周期
/// 行（票 05）引用则拒绝（友好文案，规则 10）。
pub fn erase_action(conn: &Connection, id: i64) -> Result<(), String> {
    let preset: i64 = conn
        .query_row(
            "SELECT is_preset FROM care_action WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => "操作不存在".to_string(),
            other => db_err(other),
        })?;
    if preset == 1 {
        return Err("预置操作不能删除；可改为停用".into());
    }
    let logs: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM care_log WHERE action_id = ?1",
            params![id],
            |row| row.get(0),
        )
        .map_err(db_err)?;
    if logs > 0 {
        return Err(format!(
            "该操作已被 {logs} 条记录使用，不能删除；可改为停用"
        ));
    }
    let ledger: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM reminder_ledger WHERE action_id = ?1",
            params![id],
            |row| row.get(0),
        )
        .map_err(db_err)?;
    if ledger > 0 {
        return Err(format!(
            "该操作仍被 {ledger} 条提醒台账引用，不能删除；可改为停用"
        ));
    }
    let intervals: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM colony_action_interval WHERE action_id = ?1",
            params![id],
            |row| row.get(0),
        )
        .map_err(db_err)?;
    if intervals > 0 {
        // 每窝周期票 05：复合主键保证一行=一窝，行数即设了该操作周期的窝数
        return Err(format!(
            "该操作仍被 {intervals} 个窝的每窝周期引用，不能删除；可先清各窝的每窝周期，或改为停用"
        ));
    }
    let changed = conn
        .execute("DELETE FROM care_action WHERE id = ?1", params![id])
        .map_err(db_err)?;
    if changed == 0 {
        return Err("操作不存在".into());
    }
    Ok(())
}

/// 操作性质设置：kind 必换；建议间隔 None=保留现值（切登记类不清空，便于日后切回）；
/// is_feeding None=不改、Some=覆盖（允许编辑、不强制全局唯一，界面提示用户自理）。
pub fn set_action_policy(
    conn: &Connection,
    id: i64,
    input: &ActionPolicyInput,
) -> Result<CareAction, String> {
    validate_kind(&input.kind)?;
    validate_interval(input.suggested_interval_days)?;
    let changed = conn
        .execute(
            "UPDATE care_action
             SET kind = ?1,
                 suggested_interval_days = COALESCE(?2, suggested_interval_days),
                 is_feeding = COALESCE(?3, is_feeding)
             WHERE id = ?4",
            params![
                input.kind,
                input.suggested_interval_days,
                input.is_feeding,
                id
            ],
        )
        .map_err(db_err)?;
    if changed == 0 {
        return Err("操作不存在".into());
    }
    get_action(conn, id)
}

// ── 食物 ─────────────────────────────────────────────────────────────────

/// 新增（enabled=1）或修改（改名+排序；enabled 不在编辑面）。
/// v14（ADR 0008）起食物无建议间隔列；易腐与撤食间隔走 validate_retrieval_hours
/// 守护（票 01）；大类走 validate_food_category 守护——新增必填、修改可改、
/// 缺省保留原值（ADR 0007）。
pub fn save_food(conn: &Connection, input: &FoodInput) -> Result<crate::care::Food, String> {
    let name = validate_name(&input.name, "食物")?;
    validate_retrieval_hours(input.perishable, input.retrieval_hours)?;
    validate_food_category(input.category.as_deref())?;
    let retrieval_hours = if input.perishable {
        input.retrieval_hours
    } else {
        None // 关易腐 → 间隔归空（前端清空，后端兜底）
    };
    let dupes: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM food WHERE name = ?1 AND (?2 IS NULL OR id != ?2)",
            params![name, input.id],
            |row| row.get(0),
        )
        .map_err(db_err)?;
    if dupes > 0 {
        return Err(format!("食物名字「{name}」已存在"));
    }

    match input.id {
        None => {
            let category = input
                .category
                .as_deref()
                .ok_or_else(|| "新食物必须指定大类（种子/蛋白质/糖水）".to_string())?;
            conn.execute(
                "INSERT INTO food (name, enabled, sort, perishable, retrieval_hours, category)
                 VALUES (?1, 1, ?2, ?3, ?4, ?5)",
                params![name, input.sort, input.perishable, retrieval_hours, category],
            )
            .map_err(|e| friendly_unique_err(e, "food.name", &name))?;
            crate::care::get_food(conn, conn.last_insert_rowid())
        }
        Some(id) => {
            let changed = conn
                .execute(
                    "UPDATE food SET name = ?1, sort = ?2,
                     perishable = ?3, retrieval_hours = ?4, category = COALESCE(?5, category)
                     WHERE id = ?6",
                    params![
                        name,
                        input.sort,
                        input.perishable,
                        retrieval_hours,
                        input.category,
                        id
                    ],
                )
                .map_err(|e| friendly_unique_err(e, "food.name", &name))?;
            if changed == 0 {
                return Err("食物不存在".into());
            }
            crate::care::get_food(conn, id)
        }
    }
}

// ── 大类周期（ADR 0008）──────────────────────────────────────────────────

/// 大类 key → 中文（后端只存英文 key，中文不落库、不受改名影响——与前端
/// foodCategories.ts 的映射同源，改动两处同步）。
pub const FOOD_CATEGORY_LABELS: [(&str, &str); 3] =
    [("seed", "种子"), ("protein", "蛋白质"), ("sugar", "糖水")];

/// 大类 key → 中文名（未知 key 原样返回，防御脏数据）。
pub fn food_category_label(key: &str) -> &str {
    FOOD_CATEGORY_LABELS
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, label)| *label)
        .unwrap_or(key)
}

/// 一条大类周期行。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FoodCategoryInterval {
    pub category: String,
    /// None = 未设（该大类不按周期提醒，只受统一周期/每窝周期管）。
    pub interval_days: Option<i64>,
}

/// 大类周期全表（恒三行、固定序 seed→protein→sugar；缺行视为未设）。
pub fn list_food_categories(conn: &Connection) -> Result<Vec<FoodCategoryInterval>, String> {
    let stored: Vec<(String, Option<i64>)> = {
        let mut stmt = conn
            .prepare("SELECT category, interval_days FROM food_category_interval")
            .map_err(db_err)?;
        let rows = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(db_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db_err)?;
        rows
    };
    Ok(FOOD_CATEGORIES
        .iter()
        .map(|key| FoodCategoryInterval {
            category: (*key).to_string(),
            interval_days: stored
                .iter()
                .find(|(c, _)| c == key)
                .and_then(|(_, d)| *d),
        })
        .collect())
}

/// 保存一条大类周期（None = 清空＝不按周期提醒）。行不存在则插入。
pub fn save_food_category_interval(
    conn: &Connection,
    category: &str,
    interval_days: Option<i64>,
) -> Result<FoodCategoryInterval, String> {
    if !FOOD_CATEGORIES.contains(&category) {
        return Err(format!(
            "无效的食物大类：{category}（应为 seed / protein / sugar）"
        ));
    }
    validate_interval(interval_days)?;
    conn.execute(
        "INSERT INTO food_category_interval (category, interval_days) VALUES (?1, ?2)
         ON CONFLICT(category) DO UPDATE SET interval_days = excluded.interval_days",
        params![category, interval_days],
    )
    .map_err(db_err)?;
    Ok(FoodCategoryInterval {
        category: category.to_string(),
        interval_days,
    })
}

/// 停用/启用（停用项不出现在喂食弹窗，规则 10）。
pub fn set_food_enabled(conn: &Connection, id: i64, enabled: bool) -> Result<(), String> {
    let changed = conn
        .execute(
            "UPDATE food SET enabled = ?1 WHERE id = ?2",
            params![enabled, id],
        )
        .map_err(db_err)?;
    if changed == 0 {
        return Err("食物不存在".into());
    }
    Ok(())
}

/// 物理删；预置项禁删（反馈第二轮 F2）；被 log_food 引用则拒绝（友好文案，
/// 规则 10）。v14 起台账不再按食物维度记（大类周期，ADR 0008），无需台账检查。
pub fn erase_food(conn: &Connection, id: i64) -> Result<(), String> {
    let preset: i64 = conn
        .query_row(
            "SELECT is_preset FROM food WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => "食物不存在".to_string(),
            other => db_err(other),
        })?;
    if preset == 1 {
        return Err("预置食物不能删除；可改为停用".into());
    }
    let used_by: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM log_food WHERE food_id = ?1",
            params![id],
            |row| row.get(0),
        )
        .map_err(db_err)?;
    if used_by > 0 {
        return Err(format!(
            "该食物已被 {used_by} 条记录使用，不能删除；可改为停用"
        ));
    }
    let changed = conn
        .execute("DELETE FROM food WHERE id = ?1", params![id])
        .map_err(db_err)?;
    if changed == 0 {
        return Err("食物不存在".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    /// 建内存库并迁移到最新 schema（票 01 地基）。
    fn mem_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("内存库打开失败");
        crate::db::migrate(&conn).expect("迁移失败");
        conn
    }

    const TODAY: &str = "2026-09-18";

    fn action_id(conn: &Connection, name: &str) -> i64 {
        conn.query_row(
            "SELECT id FROM care_action WHERE name = ?1",
            params![name],
            |r| r.get(0),
        )
        .expect("查操作失败")
    }

    fn food_id(conn: &Connection, name: &str) -> i64 {
        conn.query_row("SELECT id FROM food WHERE name = ?1", params![name], |r| {
            r.get(0)
        })
        .expect("查食物失败")
    }

    fn colony(conn: &Connection, name: &str) -> i64 {
        conn.execute(
            "INSERT INTO colony (name, start_date, status) VALUES (?1, '2026-01-20', 'active')",
            params![name],
        )
        .expect("建窝失败");
        conn.last_insert_rowid()
    }

    fn log(conn: &Connection, colony_id: i64, action: &str, happened_at: &str) {
        crate::care::log_care(
            conn,
            &crate::care::CareLogInput {
                colony_id,
                action_id: action_id(conn, action),
                happened_at: happened_at.into(),
                note: None,
                food_ids: vec![],
            },
            "2026-09-18 08:00:00",
        )
        .expect("记账失败");
    }

    fn log_feeding(conn: &Connection, colony_id: i64, happened_at: &str, foods: Vec<i64>) {
        crate::care::log_care(
            conn,
            &crate::care::CareLogInput {
                colony_id,
                action_id: action_id(conn, "喂食"),
                happened_at: happened_at.into(),
                note: None,
                food_ids: foods,
            },
            "2026-09-18 08:00:00",
        )
        .expect("记账失败");
    }

    fn count(conn: &Connection, sql: &str) -> i64 {
        conn.query_row(sql, [], |row| row.get(0))
            .expect("标量查询失败")
    }

    fn count_where_id(conn: &Connection, table: &str, id: i64) -> i64 {
        conn.query_row(
            &format!("SELECT COUNT(*) FROM {table} WHERE id = ?1"),
            params![id],
            |row| row.get(0),
        )
        .expect("标量查询失败")
    }

    fn by_name<'a>(items: &'a [CareAction], name: &str) -> &'a CareAction {
        items.iter().find(|a| a.name == name).expect("找不到操作")
    }

    fn tile<'a>(tiles: &'a [crate::care::ActionTile], name: &str) -> &'a crate::care::ActionTile {
        tiles.iter().find(|t| t.name == name).expect("找不到操作块")
    }

    // ── list_actions ──

    #[test]
    fn list_actions_returns_all_presets_including_disabled_in_dict_order() {
        let conn = mem_conn();
        conn.execute(
            "UPDATE care_action SET enabled = 0 WHERE name = '巢穴保湿'",
            [],
        )
        .unwrap();

        let actions = list_actions(&conn).unwrap();
        let names: Vec<&str> = actions.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["喂食", "撤食", "活动区换水", "巢穴保湿", "垃圾清理"],
            "含停用、按 sort 排"
        );

        let retrieval = by_name(&actions, "撤食");
        assert_eq!(retrieval.kind, "follow", "撤食预置为跟随喂食性质");
        assert!(retrieval.is_preset);
        assert_eq!(retrieval.suggested_interval_days, None);

        let feed = by_name(&actions, "喂食");
        assert_eq!(feed.kind, "reminding");
        assert!(feed.is_feeding, "预置喂食的 is_feeding 位为真");
        assert_eq!(feed.suggested_interval_days, Some(3));
        assert!(actions.iter().all(|a| a.enabled == (a.name != "巢穴保湿")));
        assert!(
            !actions.iter().any(|a| a.referenced),
            "无记录时 referenced 全 false"
        );
    }

    #[test]
    fn list_actions_flags_referenced_from_care_log_or_reminder_ledger() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        log(&conn, c, "喂食", "2026-09-17 20:00:00");
        conn.execute(
            "INSERT INTO reminder_ledger (colony_id, kind, action_id, base_date, sent_at)
             VALUES (?1, 'overdue', ?2, '2026-09-10', '2026-09-10 08:00:00')",
            params![c, action_id(&conn, "垃圾清理")],
        )
        .unwrap();

        let actions = list_actions(&conn).unwrap();
        assert!(by_name(&actions, "喂食").referenced);
        assert!(
            by_name(&actions, "垃圾清理").referenced,
            "提醒台账引用也算被引用"
        );
        assert!(!by_name(&actions, "活动区换水").referenced);
    }

    // ── save_action ──

    #[test]
    fn save_action_creates_new_action_enabled() {
        let conn = mem_conn();
        let created = save_action(
            &conn,
            &ActionInput {
                id: None,
                name: " 降温 ".into(),
                kind: "log_only".into(),
                is_feeding: false,
                suggested_interval_days: None,
                sort: 9,
            },
        )
        .unwrap();
        assert_eq!(created.name, "降温", "名字 trim 后落库");
        assert!(created.enabled, "新建默认启用");
        assert_eq!(created.sort, 9);
        assert!(!created.referenced);

        // 新操作立即可记、出现在卡片（字典可扩展，user story 7）
        let c = colony(&conn, "大头一号");
        log(&conn, c, "降温", "2026-09-17 20:00:00");
        let tiles = crate::care::tiles_for_colony(&conn, c, TODAY).unwrap();
        assert!(tiles.iter().any(|t| t.name == "降温"));
    }

    #[test]
    fn save_action_renames_keeps_id_and_history_follows_display_name() {
        // 验收 5：改名后历史记录跟随新显示名（id 不变）
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        let feed = action_id(&conn, "喂食");
        log(&conn, c, "喂食", "2026-09-17 20:00:00");

        let updated = save_action(
            &conn,
            &ActionInput {
                id: Some(feed),
                name: "投喂".into(),
                kind: "reminding".into(),
                is_feeding: true,
                suggested_interval_days: Some(3),
                sort: 1,
            },
        )
        .unwrap();

        assert_eq!(updated.id, feed, "改名不改 id");
        let recent = crate::care::recent_for_colony(&conn, c, 5).unwrap();
        assert_eq!(recent[0].action_name, "投喂", "历史跟随新显示名");
        assert_eq!(
            count(&conn, "SELECT COUNT(*) FROM care_action"),
            5,
            "改名不增行"
        );
    }

    #[test]
    fn save_action_update_keeps_enabled_state() {
        // 改名/排序不得顺手把停用的操作复活（enabled 只归 set_action_enabled 管）
        let conn = mem_conn();
        let water = action_id(&conn, "活动区换水");
        set_action_enabled(&conn, water, false).unwrap();

        let updated = save_action(
            &conn,
            &ActionInput {
                id: Some(water),
                name: "换水".into(),
                kind: "log_only".into(),
                is_feeding: false,
                suggested_interval_days: None,
                sort: 2,
            },
        )
        .unwrap();
        assert_eq!(updated.name, "换水");
        assert!(!updated.enabled, "改名不改启用位");
    }

    #[test]
    fn save_action_rejects_blank_duplicate_bad_kind_and_missing_id() {
        let conn = mem_conn();
        let input = |id: Option<i64>, name: &str, kind: &str| ActionInput {
            id,
            name: name.into(),
            kind: kind.into(),
            is_feeding: false,
            suggested_interval_days: None,
            sort: 0,
        };

        assert!(save_action(&conn, &input(None, "   ", "log_only"))
            .unwrap_err()
            .contains("不能为空"));
        let err = save_action(&conn, &input(None, "喂食", "reminding")).unwrap_err();
        assert!(err.contains("已存在"), "实际错误：{err}");
        assert!(save_action(&conn, &input(None, "糖水", "remind"))
            .unwrap_err()
            .contains("性质"));
        assert!(save_action(&conn, &input(Some(999), "幽灵", "log_only"))
            .unwrap_err()
            .contains("操作不存在"));
    }

    #[test]
    fn save_action_rejects_non_positive_interval() {
        let conn = mem_conn();
        let input = |days: Option<i64>| ActionInput {
            id: None,
            name: "糖水".into(),
            kind: "reminding".into(),
            is_feeding: false,
            suggested_interval_days: days,
            sort: 0,
        };
        assert!(save_action(&conn, &input(Some(0)))
            .unwrap_err()
            .contains("建议间隔"));
        assert!(save_action(&conn, &input(Some(-3)))
            .unwrap_err()
            .contains("建议间隔"));
        assert!(save_action(&conn, &input(None)).is_ok());
    }

    // ── set_action_enabled ──

    #[test]
    fn set_action_enabled_round_trip_hides_and_restores_tile() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        let hydrate = action_id(&conn, "巢穴保湿");

        set_action_enabled(&conn, hydrate, false).unwrap();
        let tiles = crate::care::tiles_for_colony(&conn, c, TODAY).unwrap();
        assert!(
            !tiles.iter().any(|t| t.name == "巢穴保湿"),
            "停用后不再出块"
        );

        set_action_enabled(&conn, hydrate, true).unwrap();
        let tiles = crate::care::tiles_for_colony(&conn, c, TODAY).unwrap();
        assert!(tiles.iter().any(|t| t.name == "巢穴保湿"), "启用后恢复");
    }

    #[test]
    fn set_action_enabled_missing_rejected() {
        let conn = mem_conn();
        assert!(set_action_enabled(&conn, 999, false)
            .unwrap_err()
            .contains("操作不存在"));
    }

    // ── erase_action（验收 3：未引用可删、被引用拒绝）──

    #[test]
    fn erase_action_unreferenced_succeeds() {
        let conn = mem_conn();
        let created = save_action(
            &conn,
            &ActionInput {
                id: None,
                name: "糖水".into(),
                kind: "log_only".into(),
                is_feeding: false,
                suggested_interval_days: None,
                sort: 5,
            },
        )
        .unwrap();
        erase_action(&conn, created.id).unwrap();
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM care_action"), 5);
    }

    #[test]
    fn erase_action_referenced_by_care_log_rejected_then_deactivate_works() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        // F2 起预置操作由预置守护先拒删；care_log 引用分支改用自建操作验证
        let custom = save_action(
            &conn,
            &ActionInput {
                id: None,
                name: "降温".into(),
                kind: "log_only".into(),
                is_feeding: false,
                suggested_interval_days: None,
                sort: 5,
            },
        )
        .unwrap();
        log(&conn, c, "降温", "2026-09-17 20:00:00");

        let err = erase_action(&conn, custom.id).unwrap_err();
        assert!(err.contains("停用"), "实际错误：{err}");
        assert_eq!(count_where_id(&conn, "care_action", custom.id), 1, "行保留");

        // 被引用删不掉，但可以停用（规则 10）
        set_action_enabled(&conn, custom.id, false).unwrap();
        let actions = list_actions(&conn).unwrap();
        assert!(!actions.iter().find(|a| a.id == custom.id).unwrap().enabled);
        // 历史记录展示不受影响（验收 2 的操作版：名字照常）
        let recent = crate::care::recent_for_colony(&conn, c, 5).unwrap();
        assert_eq!(recent[0].action_name, "降温");
    }

    #[test]
    fn erase_action_referenced_only_by_reminder_ledger_rejected() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        // F2 起预置操作由预置守护先拒删；提醒台账引用分支改用自建操作验证
        let custom = save_action(
            &conn,
            &ActionInput {
                id: None,
                name: "降温".into(),
                kind: "log_only".into(),
                is_feeding: false,
                suggested_interval_days: None,
                sort: 5,
            },
        )
        .unwrap();
        conn.execute(
            "INSERT INTO reminder_ledger (colony_id, kind, action_id, base_date, sent_at)
             VALUES (?1, 'overdue', ?2, '2026-09-10', '2026-09-10 08:00:00')",
            params![c, custom.id],
        )
        .unwrap();

        let err = erase_action(&conn, custom.id).unwrap_err();
        assert!(err.contains("台账"), "实际错误：{err}");
        assert_eq!(count_where_id(&conn, "care_action", custom.id), 1);
    }

    #[test]
    fn erase_action_referenced_by_per_colony_interval_rejected() {
        // 每窝周期票 05：操作被某窝的每窝周期引用时拒删（照 care_log/台账守卫同款），
        // 行保留，提示先清周期或改停用
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        let custom = save_action(
            &conn,
            &ActionInput {
                id: None,
                name: "降温".into(),
                kind: "reminding".into(),
                is_feeding: false,
                suggested_interval_days: Some(3),
                sort: 5,
            },
        )
        .unwrap();
        crate::colony::set_colony_action_interval(&conn, c, custom.id, Some(7)).unwrap();

        let err = erase_action(&conn, custom.id).unwrap_err();
        assert!(err.contains("周期"), "实际错误：{err}");
        assert!(err.contains("停用"), "实际错误：{err}");
        assert_eq!(count_where_id(&conn, "care_action", custom.id), 1, "行保留");
        let kept: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM colony_action_interval WHERE action_id = ?1",
                params![custom.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(kept, 1, "删除被拒时周期行不受影响");
    }

    #[test]
    fn erase_action_succeeds_after_per_colony_intervals_cleared() {
        // 清掉周期行后恢复可删（删行=未设，set None 幂等清除）
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        let custom = save_action(
            &conn,
            &ActionInput {
                id: None,
                name: "降温".into(),
                kind: "reminding".into(),
                is_feeding: false,
                suggested_interval_days: Some(3),
                sort: 5,
            },
        )
        .unwrap();
        crate::colony::set_colony_action_interval(&conn, c, custom.id, Some(7)).unwrap();
        crate::colony::set_colony_action_interval(&conn, c, custom.id, None).unwrap();

        erase_action(&conn, custom.id).unwrap();
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM care_action"), 5);
    }

    #[test]
    fn erase_missing_action_rejected() {
        let conn = mem_conn();
        assert!(erase_action(&conn, 999).unwrap_err().contains("操作不存在"));
    }

    // ── set_action_policy ──

    #[test]
    fn set_action_policy_switch_to_reminding_marks_tile_overdue() {
        // 验收 4：把「活动区换水」切成提醒类 + 3 天后，卡片相应块出现红绿态
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        let water = action_id(&conn, "活动区换水");
        log(&conn, c, "活动区换水", "2026-09-14 09:00:00"); // 4 天前

        let before = crate::care::tiles_for_colony(&conn, c, TODAY).unwrap();
        assert!(!tile(&before, "活动区换水").overdue, "登记类不红");
        assert_eq!(
            before.iter().find(|t| t.action_id == water).unwrap().kind,
            "log_only"
        );

        let updated = set_action_policy(
            &conn,
            water,
            &ActionPolicyInput {
                kind: "reminding".into(),
                suggested_interval_days: Some(3),
                is_feeding: None,
            },
        )
        .unwrap();
        assert_eq!(updated.kind, "reminding");
        assert_eq!(updated.suggested_interval_days, Some(3));

        let after = crate::care::tiles_for_colony(&conn, c, TODAY).unwrap();
        let t = after.iter().find(|t| t.action_id == water).unwrap();
        assert!(t.overdue, "切成提醒类 + 建议 3 天、距上次 4 天应超期");
    }

    #[test]
    fn set_action_policy_to_log_only_keeps_interval_value_for_later_switch_back() {
        // spec：登记类也保留可编辑值，以便日后切回提醒类
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        let feed = action_id(&conn, "喂食");
        log(&conn, c, "喂食", "2026-06-01 09:00:00"); // 远超 3 天

        let updated = set_action_policy(
            &conn,
            feed,
            &ActionPolicyInput {
                kind: "log_only".into(),
                suggested_interval_days: None, // 不带新值 → 保留现值，不清空
                is_feeding: None,
            },
        )
        .unwrap();
        assert_eq!(updated.kind, "log_only");
        assert_eq!(
            updated.suggested_interval_days,
            Some(3),
            "切登记类不清空间隔"
        );

        let tiles = crate::care::tiles_for_colony(&conn, c, TODAY).unwrap();
        assert!(!tile(&tiles, "喂食").overdue, "登记类永不红");
    }

    #[test]
    fn set_action_policy_can_toggle_is_feeding_flag() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        let water = action_id(&conn, "活动区换水");
        let seed = food_id(&conn, "种子");

        // 换水位标成喂食类 → 记账可带食物
        set_action_policy(
            &conn,
            water,
            &ActionPolicyInput {
                kind: "log_only".into(),
                suggested_interval_days: None,
                is_feeding: Some(true),
            },
        )
        .unwrap();
        log_feeding(&conn, c, "2026-09-17 09:00:00", vec![]); // 占位，确保窝有记录不碍事
        crate::care::log_care(
            &conn,
            &crate::care::CareLogInput {
                colony_id: c,
                action_id: water,
                happened_at: "2026-09-17 10:00:00".into(),
                note: None,
                food_ids: vec![seed],
            },
            "2026-09-18 08:00:00",
        )
        .expect("is_feeding=1 的操作应可带食物");

        // 喂食位取消 → 带食物被拒
        let feed = action_id(&conn, "喂食");
        set_action_policy(
            &conn,
            feed,
            &ActionPolicyInput {
                kind: "reminding".into(),
                suggested_interval_days: None,
                is_feeding: Some(false),
            },
        )
        .unwrap();
        let err = crate::care::log_care(
            &conn,
            &crate::care::CareLogInput {
                colony_id: c,
                action_id: feed,
                happened_at: "2026-09-17 11:00:00".into(),
                note: None,
                food_ids: vec![seed],
            },
            "2026-09-18 08:00:00",
        )
        .unwrap_err();
        assert!(err.contains("不是喂食"), "实际错误：{err}");
    }

    #[test]
    fn set_action_policy_rejects_bad_kind_negative_interval_and_missing_action() {
        let conn = mem_conn();
        let feed = action_id(&conn, "喂食");
        let input = |kind: &str, days: Option<i64>| ActionPolicyInput {
            kind: kind.into(),
            suggested_interval_days: days,
            is_feeding: None,
        };
        assert!(set_action_policy(&conn, feed, &input("remind", None))
            .unwrap_err()
            .contains("性质"));
        assert!(set_action_policy(&conn, feed, &input("reminding", Some(0)))
            .unwrap_err()
            .contains("建议间隔"));
        assert!(set_action_policy(&conn, 999, &input("reminding", None))
            .unwrap_err()
            .contains("操作不存在"));
    }

    // ── save_food / set_food_enabled / erase_food ──

    #[test]
    fn save_food_creates_and_renames() {
        let conn = mem_conn();
        let created = save_food(
            &conn,
            &FoodInput {
                id: None,
                name: " 糖水 ".into(),
                sort: 9,
                perishable: false,
                retrieval_hours: None,
                category: Some("sugar".into()),
            },
        )
        .unwrap();
        assert_eq!(created.name, "糖水");
        assert!(created.enabled);
        assert!(!created.referenced);

        let updated = save_food(
            &conn,
            &FoodInput {
                id: Some(created.id),
                name: "蜂蜜水".into(),
                sort: 0,
                perishable: false,
                retrieval_hours: None,
                category: None,
            },
        )
        .unwrap();
        assert_eq!(updated.name, "蜂蜜水");
        assert_eq!(updated.id, created.id, "改名不改 id");
    }

    #[test]
    fn save_food_rejects_blank_duplicate_and_missing_id() {
        let conn = mem_conn();
        assert!(save_food(
            &conn,
            &FoodInput {
                id: None,
                name: "  ".into(),
                sort: 0,
                perishable: false,
                retrieval_hours: None,
                category: None
            }
        )
        .unwrap_err()
        .contains("不能为空"));
        let err = save_food(
            &conn,
            &FoodInput {
                id: None,
                name: "种子".into(),
                sort: 0,
                perishable: false,
                retrieval_hours: None,
                category: None,
            },
        )
        .unwrap_err();
        assert!(err.contains("已存在"), "实际错误：{err}");
        assert!(save_food(
            &conn,
            &FoodInput {
                id: Some(999),
                name: "幽灵".into(),
                sort: 0,
                perishable: false,
                retrieval_hours: None,
                category: None
            }
        )
        .unwrap_err()
        .contains("食物不存在"));
    }

    #[test]
    fn food_category_interval_roundtrips_and_validates() {
        // ADR 0008：周期挂大类——设置/清空/非法值拒收；list 恒三行固定序
        let conn = mem_conn();
        // 迁移链播种的初值：大类内食物间隔最大值（seed=7 / protein=3 / sugar=3）
        assert_eq!(
            list_food_categories(&conn).unwrap(),
            vec![
                FoodCategoryInterval { category: "seed".into(), interval_days: Some(7) },
                FoodCategoryInterval { category: "protein".into(), interval_days: Some(3) },
                FoodCategoryInterval { category: "sugar".into(), interval_days: Some(3) },
            ]
        );

        // 改值 + 回读
        save_food_category_interval(&conn, "protein", Some(5)).unwrap();
        assert_eq!(
            list_food_categories(&conn).unwrap()[1],
            FoodCategoryInterval { category: "protein".into(), interval_days: Some(5) }
        );

        // 清空 = 不按周期提醒（None 落库，不是删行）
        save_food_category_interval(&conn, "protein", None).unwrap();
        assert_eq!(
            list_food_categories(&conn).unwrap()[1],
            FoodCategoryInterval { category: "protein".into(), interval_days: None }
        );

        // 非法：未知大类、0 天
        assert!(save_food_category_interval(&conn, "vitamin", Some(3))
            .unwrap_err()
            .contains("大类"));
        assert!(save_food_category_interval(&conn, "sugar", Some(0))
            .unwrap_err()
            .contains("建议间隔"));
    }

    #[test]
    fn food_category_label_maps_known_keys() {
        assert_eq!(food_category_label("seed"), "种子");
        assert_eq!(food_category_label("protein"), "蛋白质");
        assert_eq!(food_category_label("sugar"), "糖水");
        assert_eq!(food_category_label("junk"), "junk", "未知 key 原样返回");
    }

    #[test]
    fn save_food_rejects_perishable_without_valid_retrieval_hours() {
        // 票 01 验收：开易腐 + 空/0/负/越界 → 拒收；1–168 整数 → 通过
        let conn = mem_conn();
        let mealworm = food_id(&conn, "面包虫干");
        let input = |hours: Option<i64>| FoodInput {
            id: Some(mealworm),
            name: "面包虫干".into(),
            sort: 3,
            perishable: true,
            retrieval_hours: hours,
            category: None,
        };

        // 拒收矩阵：缺失 / 0 / 负 / 越界（169）
        for bad in [None, Some(0), Some(-3), Some(169)] {
            let err = save_food(&conn, &input(bad)).unwrap_err();
            assert!(
                err.contains("撤食间隔"),
                "hours={bad:?} 应拒收，实际：{err}"
            );
        }
        // 边界内通过：1 / 24 / 168
        for good in [Some(1), Some(24), Some(168)] {
            save_food(&conn, &input(good)).unwrap();
        }
    }

    #[test]
    fn save_food_persists_perishable_and_normalizes_hours_when_off() {
        let conn = mem_conn();
        let mealworm = food_id(&conn, "面包虫干");

        // 开易腐 + 24h → 落库
        save_food(
            &conn,
            &FoodInput {
                id: Some(mealworm),
                name: "面包虫干".into(),
                sort: 3,
                perishable: true,
                retrieval_hours: Some(24),
                category: None,
            },
        )
        .unwrap();
        let (perishable, hours): (i64, Option<i64>) = conn
            .query_row(
                "SELECT perishable, retrieval_hours FROM food WHERE id = ?1",
                params![mealworm],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((perishable, hours), (1, Some(24)));

        // 关易腐 → 后端兜底归空（前端清空触发，这里验证绕过前端的直调也安全）
        save_food(
            &conn,
            &FoodInput {
                id: Some(mealworm),
                name: "面包虫干".into(),
                sort: 3,
                perishable: false,
                retrieval_hours: Some(24),
                category: None,
            },
        )
        .unwrap();
        let (perishable, hours): (i64, Option<i64>) = conn
            .query_row(
                "SELECT perishable, retrieval_hours FROM food WHERE id = ?1",
                params![mealworm],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((perishable, hours), (0, None), "关易腐时间隔一律归空");

        // 新建自建食物可直接带易腐
        let created = save_food(
            &conn,
            &FoodInput {
                id: None,
                name: "鲜果".into(),
                sort: 9,
                perishable: true,
                retrieval_hours: Some(12),
                category: Some("protein".into()),
            },
        )
        .unwrap();
        let (perishable, hours): (i64, Option<i64>) = conn
            .query_row(
                "SELECT perishable, retrieval_hours FROM food WHERE id = ?1",
                params![created.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((perishable, hours), (1, Some(12)));
    }

    #[test]
    fn save_food_new_requires_category() {
        // ADR 0007：新增缺大类 → 拒收（前端新增行必选下拉，这里验后端兜底）
        let conn = mem_conn();
        let err = save_food(
            &conn,
            &FoodInput {
                id: None,
                name: "蟋蟀".into(),
                sort: 9,
                perishable: false,
                retrieval_hours: None,
                category: None,
            },
        )
        .unwrap_err();
        assert!(err.contains("大类"), "实际错误：{err}");
    }

    #[test]
    fn save_food_rejects_unknown_category() {
        let conn = mem_conn();
        let err = save_food(
            &conn,
            &FoodInput {
                id: None,
                name: "蟋蟀".into(),
                sort: 9,
                perishable: false,
                retrieval_hours: None,
                category: Some("fruit".into()),
            },
        )
        .unwrap_err();
        assert!(err.contains("无效的食物大类"), "实际错误：{err}");
    }

    #[test]
    fn save_food_inserts_updates_and_keeps_category() {
        // 新增带大类落库；行内可改大类；修改缺省 = 保留原值（旧调用兼容）
        let conn = mem_conn();
        let created = save_food(
            &conn,
            &FoodInput {
                id: None,
                name: "蟋蟀".into(),
                sort: 9,
                perishable: false,
                retrieval_hours: None,
                category: Some("protein".into()),
            },
        )
        .unwrap();
        assert_eq!(created.category, "protein");

        let moved = save_food(
            &conn,
            &FoodInput {
                id: Some(created.id),
                name: "蟋蟀".into(),
                sort: 9,
                perishable: false,
                retrieval_hours: None,
                category: Some("seed".into()),
            },
        )
        .unwrap();
        assert_eq!(moved.category, "seed", "行内可改大类");

        let kept = save_food(
            &conn,
            &FoodInput {
                id: Some(created.id),
                name: "蟋蟀".into(),
                sort: 9,
                perishable: false,
                retrieval_hours: None,
                category: None,
            },
        )
        .unwrap();
        assert_eq!(kept.category, "seed", "缺省保留原大类");
    }

    #[test]
    fn set_food_enabled_round_trip() {
        let conn = mem_conn();
        let seed = food_id(&conn, "种子");
        set_food_enabled(&conn, seed, false).unwrap();
        let foods = crate::care::list_foods(&conn).unwrap();
        assert!(!foods.iter().find(|f| f.id == seed).unwrap().enabled);
        set_food_enabled(&conn, seed, true).unwrap();
        let foods = crate::care::list_foods(&conn).unwrap();
        assert!(foods.iter().find(|f| f.id == seed).unwrap().enabled);
        assert!(set_food_enabled(&conn, 999, false)
            .unwrap_err()
            .contains("食物不存在"));
    }

    #[test]
    fn erase_food_unreferenced_succeeds() {
        let conn = mem_conn();
        let created = save_food(
            &conn,
            &FoodInput {
                id: None,
                name: "糖水".into(),
                sort: 9,
                perishable: false,
                retrieval_hours: None,
                category: Some("sugar".into()),
            },
        )
        .unwrap();
        erase_food(&conn, created.id).unwrap();
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM food"), 7);
    }

    #[test]
    fn erase_food_referenced_by_log_food_rejected_then_deactivate_works() {
        // 验收 3（食物）：被记录引用的删除被拒。
        // F2 起预置食物由预置守护先拒删；log_food 引用分支改用自建食物验证
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        let custom = save_food(
            &conn,
            &FoodInput {
                id: None,
                name: "糖水".into(),
                sort: 9,
                perishable: false,
                retrieval_hours: None,
                category: Some("sugar".into()),
            },
        )
        .unwrap();
        log_feeding(&conn, c, "2026-09-17 20:00:00", vec![custom.id]);

        let err = erase_food(&conn, custom.id).unwrap_err();
        assert!(err.contains("停用"), "实际错误：{err}");
        assert_eq!(count_where_id(&conn, "food", custom.id), 1, "行保留");

        set_food_enabled(&conn, custom.id, false).unwrap();
        let foods = crate::care::list_foods(&conn).unwrap();
        assert!(!foods.iter().find(|f| f.id == custom.id).unwrap().enabled);
        assert!(foods.iter().find(|f| f.id == custom.id).unwrap().referenced);
    }

    #[test]
    fn deactivated_food_keeps_history_name_and_is_hidden_from_new_entries() {
        // 验收 1 + 2：停用后喂食弹窗不再出现它（前端按 enabled 过滤 list_foods），
        // 引用过它的历史记录展示不受影响（名称保留）
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        let seed = food_id(&conn, "种子");
        log_feeding(&conn, c, "2026-09-17 20:00:00", vec![seed]);

        set_food_enabled(&conn, seed, false).unwrap();

        let foods = crate::care::list_foods(&conn).unwrap();
        let seed_row = foods.iter().find(|f| f.id == seed).unwrap();
        assert!(!seed_row.enabled, "停用位落库，前端据此从新建入口过滤");
        assert_eq!(seed_row.name, "种子", "行还在，名字不变");

        let recent = crate::care::recent_for_colony(&conn, c, 5).unwrap();
        assert_eq!(recent[0].food_names, vec!["种子"], "历史记录名称保留");
    }

    #[test]
    fn erase_food_referenced_only_by_old_ledger_now_succeeds() {
        // ADR 0008：台账不再按食物维度记（大类周期），旧 food_overdue 行已迁成
        // category_overdue——只被台账「引用」过的自建食物现在可以删（无外键可撞）；
        // referenced 位也只剩 log_food 一源
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        let custom = save_food(
            &conn,
            &FoodInput {
                id: None,
                name: "糖水".into(),
                sort: 9,
                perishable: false,
                retrieval_hours: None,
                category: Some("sugar".into()),
            },
        )
        .unwrap();
        conn.execute(
            "INSERT INTO reminder_ledger (colony_id, kind, action_id, category, base_date, sent_at)
             VALUES (?1, 'category_overdue', ?2, 'sugar', '2026-09-11', '2026-09-18 08:00:00')",
            params![c, action_id(&conn, "喂食")],
        )
        .unwrap();

        erase_food(&conn, custom.id).unwrap();
        assert_eq!(count_where_id(&conn, "food", custom.id), 0, "行已删");
        let foods = crate::care::list_foods(&conn).unwrap();
        assert!(
            foods.iter().all(|f| f.id != custom.id),
            "列表不再含该行"
        );
    }

    #[test]
    fn erase_missing_food_rejected() {
        let conn = mem_conn();
        assert!(erase_food(&conn, 999).unwrap_err().contains("食物不存在"));
    }

    // ── 预置项禁删（反馈第二轮 F2）──

    #[test]
    fn erase_action_rejects_all_five_presets_even_unreferenced() {
        let conn = mem_conn();
        for name in ["喂食", "撤食", "活动区换水", "巢穴保湿", "垃圾清理"] {
            let err = erase_action(&conn, action_id(&conn, name)).unwrap_err();
            assert!(err.contains("预置"), "{name} 应拒删，实际：{err}");
        }
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM care_action"), 5);
        // 改名/停用不受影响
        let water = action_id(&conn, "活动区换水");
        save_action(
            &conn,
            &ActionInput {
                id: Some(water),
                name: "换水".into(),
                kind: "log_only".into(),
                is_feeding: false,
                suggested_interval_days: None,
                sort: 2,
            },
        )
        .unwrap();
        set_action_enabled(&conn, water, false).unwrap();
    }

    #[test]
    fn erase_food_rejects_all_preset_foods() {
        let conn = mem_conn();
        for name in [
            "种子",
            "虾干",
            "面包虫干",
            "樱桃蟑螂",
            "蜂蜜",
            "冰糖水",
            "白糖水",
        ] {
            let err = erase_food(&conn, food_id(&conn, name)).unwrap_err();
            assert!(err.contains("预置"), "{name} 应拒删，实际：{err}");
        }
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM food"), 7);
    }

    #[test]
    fn list_actions_and_foods_expose_is_preset() {
        let conn = mem_conn();
        let created = save_action(
            &conn,
            &ActionInput {
                id: None,
                name: "降温".into(),
                kind: "log_only".into(),
                is_feeding: false,
                suggested_interval_days: None,
                sort: 5,
            },
        )
        .unwrap();
        let actions = list_actions(&conn).unwrap();
        assert!(by_name(&actions, "喂食").is_preset);
        assert!(
            !actions
                .iter()
                .find(|a| a.id == created.id)
                .unwrap()
                .is_preset
        );
        let foods = crate::care::list_foods(&conn).unwrap();
        assert!(foods.iter().find(|f| f.name == "种子").unwrap().is_preset);
        let custom = save_food(
            &conn,
            &FoodInput {
                id: None,
                name: "糖水".into(),
                sort: 9,
                perishable: false,
                retrieval_hours: None,
                category: Some("sugar".into()),
            },
        )
        .unwrap();
        assert!(!custom.is_preset);
    }
}
