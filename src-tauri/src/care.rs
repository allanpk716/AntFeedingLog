//! 维护操作字典、记账与「距上次」计算（票 03）。
//!
//! 全部函数只吃 `&Connection`，与 Tauri 解耦，可被 cargo test 直接覆盖；
//! Tauri command 只是薄包装（见 lib.rs）。
//!
//! 行为对齐 spec：
//! - 发生时间（occurred_at）与录入时间（created_at）分开存，补录合法；
//! - 「距上次」= 今天 − 该窝该操作最近一次发生日期（自然日，评审附录规则 5 的无冬眠基线，
//!   出眠重算属票 05）；
//! - 超期判定仅提醒类且 > 建议间隔；登记类永不红；
//! - 喂食可一条记录挂多种食物（log_food 多选关联），与记录同事务写入。

use std::collections::HashSet;

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

// ── DTO ──────────────────────────────────────────────────────────────────

/// 食物（含停用的：前端新建入口过滤 enabled）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Food {
    pub id: i64,
    pub name: String,
    pub enabled: bool,
    pub sort: i64,
}

/// 窝卡片上单个操作块的后端算好的展示数据。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ActionTile {
    pub action_id: i64,
    pub name: String,
    pub icon: Option<String>,
    pub kind: String, // reminding | log_only
    /// 是否喂食类操作（决定记账时是否带食物多选；schema 标记位，与名字无关）。
    pub is_feeding: bool,
    pub suggested_interval_days: Option<i64>,
    /// 今天 − 最近一次发生日期（自然日）；从未记录为 None。
    pub days_since_last: Option<i64>,
    /// 仅提醒类且 > 建议间隔；登记类/从未记录恒 false。
    pub overdue: bool,
}

/// 最近记录摘要的一行（前端拼成「最近：09-17 喂食（种子）· …」）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RecentLog {
    pub happened_at: String,
    pub action_name: String,
    pub food_names: Vec<String>,
}

/// 记账入参：喂食才带 food_ids（其余操作传空数组）。
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct CareLogInput {
    pub colony_id: i64,
    pub action_id: i64,
    /// 发生时间，可补录过去。接受 `YYYY-MM-DD`、`YYYY-MM-DD HH:MM[:SS]`（T 或空格分隔）。
    pub happened_at: String,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub food_ids: Vec<i64>,
}

// ── 时间 ─────────────────────────────────────────────────────────────────

/// 本机当前时间，`YYYY-MM-DD HH:MM:SS`（command 层取"现在"用；测试传固定值，不碰时钟）。
pub fn now_local() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

/// 规整发生时间为 `YYYY-MM-DD HH:MM:SS`（库内统一格式，保证文本比较 = 时间比较）。
pub fn normalize_happened_at(s: &str) -> Result<String, String> {
    let trimmed = s.trim();
    for fmt in ["%Y-%m-%dT%H:%M:%S", "%Y-%m-%dT%H:%M", "%Y-%m-%d %H:%M:%S", "%Y-%m-%d %H:%M"] {
        if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(trimmed, fmt) {
            return Ok(dt.format("%Y-%m-%d %H:%M:%S").to_string());
        }
    }
    if let Ok(d) = chrono::NaiveDate::parse_from_str(trimmed, "%Y-%m-%d") {
        return Ok(d.format("%Y-%m-%d 00:00:00").to_string());
    }
    Err("发生时间格式应为 YYYY-MM-DD 或 YYYY-MM-DD HH:MM".into())
}

/// ISO 日期解析（`YYYY-MM-DD`）。与 colony.rs 的 parse_iso 同口径，本模块自持不外依赖。
fn parse_iso_date(s: &str) -> Result<chrono::NaiveDate, chrono::ParseError> {
    chrono::NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d")
}

/// 「距上次」：today − 最近一次发生日期（occurred_at 的日期部分）的自然日天数。
/// 同一天为 0；从未记录（None）为 None。
pub fn days_since_last(last_occurred_at: Option<&str>, today: &str) -> Result<Option<i64>, String> {
    let Some(last) = last_occurred_at else {
        return Ok(None);
    };
    let last = parse_iso_date(last.get(0..10).unwrap_or(""))
        .map_err(|_| "记录发生时间格式异常".to_string())?;
    let today =
        parse_iso_date(today).map_err(|_| "日期格式应为 YYYY-MM-DD".to_string())?;
    Ok(Some((today - last).num_days()))
}

/// 超期判定：仅提醒类且距上次严格大于建议间隔。登记类与从未记录永不超期。
pub fn is_overdue(kind: &str, days: Option<i64>, suggested_interval_days: Option<i64>) -> bool {
    match (kind, days, suggested_interval_days) {
        ("reminding", Some(d), Some(interval)) => d > interval,
        _ => false,
    }
}

// ── 记账 ─────────────────────────────────────────────────────────────────

fn db_err(e: rusqlite::Error) -> String {
    format!("数据库操作失败: {e}")
}

/// 记一笔：插入 care_log（occurred_at 规整后落库，created_at = now），喂食多选食物
/// 写 log_food，与主记录同事务，任一失败整体回滚。返回新记录 id。
pub fn log_care(conn: &Connection, input: &CareLogInput, now: &str) -> Result<i64, String> {
    let colony_exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM colony WHERE id = ?1",
            params![input.colony_id],
            |row| row.get(0),
        )
        .map_err(db_err)?;
    if colony_exists == 0 {
        return Err("窝不存在".into());
    }

    let (action_name, enabled, is_feeding): (String, i64, i64) = conn
        .query_row(
            "SELECT name, enabled, is_feeding FROM care_action WHERE id = ?1",
            params![input.action_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => "操作不存在".to_string(),
            other => db_err(other),
        })?;
    if enabled == 0 {
        return Err(format!("操作「{action_name}」已停用，不能新记"));
    }

    let happened_at = normalize_happened_at(&input.happened_at)?;
    let note = input.note.as_deref().map(str::trim).unwrap_or("").to_string();

    // 食物关联仅喂食类操作可带（is_feeding 标记位，与名字无关）
    if is_feeding == 0 && !input.food_ids.is_empty() {
        return Err(format!("操作「{action_name}」不是喂食，不能关联食物"));
    }

    // 食物：去重（保序）+ 逐个校验存在且启用
    let mut seen = HashSet::new();
    let food_ids: Vec<i64> = input.food_ids.iter().filter(|id| seen.insert(**id)).copied().collect();
    for food_id in &food_ids {
        let food_enabled: Option<i64> = conn
            .query_row(
                "SELECT enabled FROM food WHERE id = ?1",
                params![food_id],
                |row| row.get(0),
            )
            .ok();
        match food_enabled {
            None => return Err(format!("食物不存在（id={food_id}）")),
            Some(0) => return Err(format!("食物（id={food_id}）已停用，不能选用")),
            _ => {}
        }
    }

    let tx = conn.unchecked_transaction().map_err(db_err)?;
    tx.execute(
        "INSERT INTO care_log (colony_id, action_id, occurred_at, note, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![input.colony_id, input.action_id, happened_at, note, now],
    )
    .map_err(db_err)?;
    let log_id = tx.last_insert_rowid();
    for food_id in &food_ids {
        tx.execute(
            "INSERT INTO log_food (log_id, food_id) VALUES (?1, ?2)",
            params![log_id, food_id],
        )
        .map_err(db_err)?;
    }
    tx.commit().map_err(db_err)?;
    Ok(log_id)
}

// ── 卡片展示数据 ─────────────────────────────────────────────────────────

/// 某窝每个「启用中」操作一块，按 sort、id 排序（字典新增操作自动出现）。
pub fn tiles_for_colony(
    conn: &Connection,
    colony_id: i64,
    today: &str,
) -> Result<Vec<ActionTile>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, name, icon, kind, is_feeding, suggested_interval_days
             FROM care_action WHERE enabled = 1 ORDER BY sort, id",
        )
        .map_err(db_err)?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, Option<i64>>(5)?,
            ))
        })
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?;

    let mut tiles = Vec::with_capacity(rows.len());
    for (action_id, name, icon, kind, is_feeding, interval) in rows {
        let last: Option<String> = conn
            .query_row(
                "SELECT MAX(occurred_at) FROM care_log WHERE colony_id = ?1 AND action_id = ?2",
                params![colony_id, action_id],
                |row| row.get(0),
            )
            .map_err(db_err)?;
        let days = days_since_last(last.as_deref(), today)?;
        let overdue = is_overdue(&kind, days, interval);
        tiles.push(ActionTile {
            action_id,
            name,
            icon,
            kind,
            is_feeding: is_feeding != 0,
            suggested_interval_days: interval,
            days_since_last: days,
            overdue,
        });
    }
    Ok(tiles)
}

/// 某窝最近 `limit` 条记录摘要（按发生时间倒序；同刻按 id 倒序），带食物名。
pub fn recent_for_colony(
    conn: &Connection,
    colony_id: i64,
    limit: i64,
) -> Result<Vec<RecentLog>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT l.id, l.occurred_at, a.name
             FROM care_log l JOIN care_action a ON a.id = l.action_id
             WHERE l.colony_id = ?1
             ORDER BY l.occurred_at DESC, l.id DESC LIMIT ?2",
        )
        .map_err(db_err)?;
    let rows = stmt
        .query_map(params![colony_id, limit], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?;

    let mut recent = Vec::with_capacity(rows.len());
    for (log_id, happened_at, action_name) in rows {
        let mut stmt_food = conn
            .prepare(
                "SELECT f.name FROM log_food lf JOIN food f ON f.id = lf.food_id
                 WHERE lf.log_id = ?1 ORDER BY f.sort, f.id",
            )
            .map_err(db_err)?;
        let foods = stmt_food
            .query_map(params![log_id], |row| row.get::<_, String>(0))
            .map_err(db_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db_err)?;
        recent.push(RecentLog { happened_at, action_name, food_names: foods });
    }
    Ok(recent)
}

// ── 字典查询 ─────────────────────────────────────────────────────────────

/// 全部食物（含停用的，与 list_locations 同口径；新建入口由前端过滤 enabled）。
pub fn list_foods(conn: &Connection) -> Result<Vec<Food>, String> {
    let mut stmt = conn
        .prepare("SELECT id, name, enabled, sort FROM food ORDER BY sort, id")
        .map_err(db_err)?;
    let rows = stmt
        .query_map([], |row| {
            Ok(Food {
                id: row.get(0)?,
                name: row.get(1)?,
                enabled: row.get::<_, i64>(2)? != 0,
                sort: row.get(3)?,
            })
        })
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?;
    Ok(rows)
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

    const TODAY: &str = "2026-09-18";

    fn colony(conn: &Connection, name: &str) -> i64 {
        conn.execute(
            "INSERT INTO colony (name, start_date, status) VALUES (?1, '2026-01-20', 'active')",
            params![name],
        )
        .expect("建窝失败");
        conn.last_insert_rowid()
    }

    fn action_id(conn: &Connection, name: &str) -> i64 {
        conn.query_row("SELECT id FROM care_action WHERE name = ?1", params![name], |r| r.get(0))
            .expect("查操作失败")
    }

    fn food_id(conn: &Connection, name: &str) -> i64 {
        conn.query_row("SELECT id FROM food WHERE name = ?1", params![name], |r| r.get(0))
            .expect("查食物失败")
    }

    fn log(conn: &Connection, colony_id: i64, action: &str, happened_at: &str) -> i64 {
        log_care(
            conn,
            &CareLogInput {
                colony_id,
                action_id: action_id(conn, action),
                happened_at: happened_at.into(),
                note: None,
                food_ids: vec![],
            },
            "2026-09-18 08:00:00",
        )
        .expect("记账失败")
    }

    fn count(conn: &Connection, sql: &str) -> i64 {
        conn.query_row(sql, [], |row| row.get(0)).expect("标量查询失败")
    }

    fn tile<'a>(tiles: &'a [ActionTile], name: &str) -> &'a ActionTile {
        tiles.iter().find(|t| t.name == name).expect("找不到操作块")
    }

    // ── 发生时间规整 ──

    #[test]
    fn normalize_happened_at_accepts_common_formats() {
        assert_eq!(normalize_happened_at("2026-09-18T20:00").unwrap(), "2026-09-18 20:00:00");
        assert_eq!(normalize_happened_at("2026-09-18T20:00:05").unwrap(), "2026-09-18 20:00:05");
        assert_eq!(normalize_happened_at(" 2026-09-18 20:00 ").unwrap(), "2026-09-18 20:00:00");
        assert_eq!(normalize_happened_at("2026-09-18").unwrap(), "2026-09-18 00:00:00");
        assert!(normalize_happened_at("2026/09/18").is_err());
        assert!(normalize_happened_at("昨天晚上").is_err());
    }

    // ── 距上次 / 超期判定（纯函数） ──

    #[test]
    fn days_since_last_counts_natural_days_from_occurrence() {
        // 补录昨天 → 距上次按发生时间算 = 1（验收 3）
        assert_eq!(days_since_last(Some("2026-09-17 21:30:00"), TODAY).unwrap(), Some(1));
        // 今天记的 → 0
        assert_eq!(days_since_last(Some("2026-09-18 08:00:00"), TODAY).unwrap(), Some(0));
        // 4 天前（验收 4 的数据）
        assert_eq!(days_since_last(Some("2026-09-14 20:00:00"), TODAY).unwrap(), Some(4));
        // 从未记录
        assert_eq!(days_since_last(None, TODAY).unwrap(), None);
    }

    #[test]
    fn is_overdue_only_for_reminding_beyond_interval() {
        // 喂食 4 天前、建议 3 天 → 超期（验收 4）
        assert!(is_overdue("reminding", Some(4), Some(3)));
        // 恰好 3 天不超期
        assert!(!is_overdue("reminding", Some(3), Some(3)));
        // 垃圾清理 4 天、建议 7 天 → 不超期
        assert!(!is_overdue("reminding", Some(4), Some(7)));
        // 登记类无论多少天都不超期（验收 5）
        assert!(!is_overdue("log_only", Some(100), Some(3)));
        assert!(!is_overdue("log_only", Some(100), None));
        // 从未记录不超期
        assert!(!is_overdue("reminding", None, Some(3)));
    }

    // ── 记账 ──

    #[test]
    fn feeding_with_two_foods_creates_one_log_and_two_links() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        let seed = food_id(&conn, "种子");
        let shrimp = food_id(&conn, "干虾仁");

        let id = log_care(
            &conn,
            &CareLogInput {
                colony_id: c,
                action_id: action_id(&conn, "喂食"),
                happened_at: "2026-09-18T20:00".into(),
                note: Some("  换新批次  ".into()),
                food_ids: vec![seed, shrimp],
            },
            "2026-09-18 21:00:00",
        )
        .unwrap();

        // 一条记录、两个食物关联（验收 2）
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM care_log"), 1);
        assert_eq!(count(&conn, &format!("SELECT COUNT(*) FROM log_food WHERE log_id = {id}")), 2);
        let (note, occurred, created): (String, String, String) = conn
            .query_row(
                "SELECT note, occurred_at, created_at FROM care_log WHERE id = ?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(note, "换新批次");
        // 发生时间与录入时间分开存，且规整为统一格式（验收 7）
        assert_eq!(occurred, "2026-09-18 20:00:00");
        assert_eq!(created, "2026-09-18 21:00:00");
    }

    #[test]
    fn backdated_log_ages_from_happened_time() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        // 今天录入，但补录昨天发生（录入时间今天、发生时间昨天）
        log_care(
            &conn,
            &CareLogInput {
                colony_id: c,
                action_id: action_id(&conn, "喂食"),
                happened_at: "2026-09-17T21:00".into(),
                note: None,
                food_ids: vec![],
            },
            "2026-09-18 09:00:00",
        )
        .unwrap();

        let tiles = tiles_for_colony(&conn, c, TODAY).unwrap();
        assert_eq!(tile(&tiles, "喂食").days_since_last, Some(1), "距上次按发生时间算");
        assert!(!tile(&tiles, "喂食").overdue);
    }

    #[test]
    fn log_care_rejects_unknown_colony_action_and_disabled_entries() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        let feed = action_id(&conn, "喂食");

        let input = |colony_id: i64, act: i64, foods: Vec<i64>| CareLogInput {
            colony_id,
            action_id: act,
            happened_at: "2026-09-18T20:00".into(),
            note: None,
            food_ids: foods,
        };

        assert!(log_care(&conn, &input(999, feed, vec![]), "2026-09-18 08:00:00")
            .unwrap_err()
            .contains("窝不存在"));
        assert!(log_care(&conn, &input(c, 999, vec![]), "2026-09-18 08:00:00")
            .unwrap_err()
            .contains("操作不存在"));

        // 停用的操作/食物不能进新记录（规则 10：停用项不出现在新建记录入口）
        conn.execute("UPDATE care_action SET enabled = 0 WHERE id = ?1", params![feed]).unwrap();
        assert!(log_care(&conn, &input(c, feed, vec![]), "2026-09-18 08:00:00")
            .unwrap_err()
            .contains("停用"));
        conn.execute("UPDATE care_action SET enabled = 1 WHERE id = ?1", params![feed]).unwrap();

        let water = food_id(&conn, "种子");
        conn.execute("UPDATE food SET enabled = 0 WHERE id = ?1", params![water]).unwrap();
        assert!(log_care(&conn, &input(c, feed, vec![water]), "2026-09-18 08:00:00")
            .unwrap_err()
            .contains("停用"));
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM care_log"), 0, "被拒的记账不落库");
    }

    #[test]
    fn duplicate_food_ids_are_stored_once() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        let seed = food_id(&conn, "种子");
        let id = log_care(
            &conn,
            &CareLogInput {
                colony_id: c,
                action_id: action_id(&conn, "喂食"),
                happened_at: "2026-09-18T20:00".into(),
                note: None,
                food_ids: vec![seed, seed],
            },
            "2026-09-18 08:00:00",
        )
        .unwrap();
        assert_eq!(count(&conn, &format!("SELECT COUNT(*) FROM log_food WHERE log_id = {id}")), 1);
    }

    #[test]
    fn log_care_rejects_foods_for_non_feeding_action() {
        // 食物关联仅喂食类操作可带（is_feeding 位，与名字无关）：
        // 「活动区换水」不是喂食 → 带食物拒收
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        let water = action_id(&conn, "活动区换水");
        let seed = food_id(&conn, "种子");

        let err = log_care(
            &conn,
            &CareLogInput {
                colony_id: c,
                action_id: water,
                happened_at: "2026-09-18T20:00".into(),
                note: None,
                food_ids: vec![seed],
            },
            "2026-09-18 08:00:00",
        )
        .unwrap_err();
        assert!(err.contains("食物"), "实际错误：{err}");
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM care_log"), 0, "被拒的记账不落库");

        // 不带食物照常可记
        assert!(log_care(
            &conn,
            &CareLogInput {
                colony_id: c,
                action_id: water,
                happened_at: "2026-09-18T20:00".into(),
                note: None,
                food_ids: vec![],
            },
            "2026-09-18 08:00:00",
        )
        .is_ok());
    }

    #[test]
    fn log_care_failure_rolls_back_the_log_row() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        // 让食物关联写入失败，模拟中途崩溃：主记录必须一并回滚
        conn.execute(
            "CREATE TRIGGER block_log_food BEFORE INSERT ON log_food
             BEGIN SELECT RAISE(ABORT, 'blocked'); END",
            [],
        )
        .unwrap();

        let result = log_care(
            &conn,
            &CareLogInput {
                colony_id: c,
                action_id: action_id(&conn, "喂食"),
                happened_at: "2026-09-18T20:00".into(),
                note: None,
                food_ids: vec![food_id(&conn, "种子")],
            },
            "2026-09-18 08:00:00",
        );
        assert!(result.is_err());
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM care_log"), 0, "主记录应随事务回滚");
    }

    // ── 卡片展示数据 ──

    #[test]
    fn tiles_report_days_since_last_and_overdue_per_action() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        // 喂食 4 天前（建议 3 → 红）；垃圾清理 4 天前（建议 7 → 不红）；
        // 换水 100 天前（登记类 → 永不红）；保湿从未记录
        log(&conn, c, "喂食", "2026-09-14 20:00:00");
        log(&conn, c, "垃圾清理", "2026-09-14 20:00:00");
        log(&conn, c, "活动区换水", "2026-06-10 09:00:00");

        let tiles = tiles_for_colony(&conn, c, TODAY).unwrap();
        let feed = tile(&tiles, "喂食");
        assert_eq!(feed.days_since_last, Some(4));
        assert!(feed.overdue, "喂食 4 天 > 建议 3 应超期");
        assert_eq!(feed.kind, "reminding");
        assert_eq!(feed.suggested_interval_days, Some(3));
        assert!(feed.is_feeding, "预置喂食操作的 is_feeding 标记位为真");

        let trash = tile(&tiles, "垃圾清理");
        assert_eq!(trash.days_since_last, Some(4));
        assert!(!trash.overdue, "垃圾清理 4 天 ≤ 建议 7 不超期");
        assert!(!trash.is_feeding);

        let water = tile(&tiles, "活动区换水");
        assert_eq!(water.days_since_last, Some(100));
        assert!(!water.overdue, "登记类无论多少天都不红");
        assert_eq!(water.kind, "log_only");

        let hydrate = tile(&tiles, "巢穴保湿");
        assert_eq!(hydrate.days_since_last, None, "从未记录为 None");
        assert!(!hydrate.overdue);
    }

    #[test]
    fn tiles_include_only_enabled_actions_in_dict_order() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");

        let tiles = tiles_for_colony(&conn, c, TODAY).unwrap();
        let names: Vec<&str> = tiles.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["喂食", "活动区换水", "巢穴保湿", "垃圾清理"], "按 sort 排");

        // 停用的操作不再出现（验收 6）
        conn.execute("UPDATE care_action SET enabled = 0 WHERE name = '巢穴保湿'", []).unwrap();
        let tiles = tiles_for_colony(&conn, c, TODAY).unwrap();
        let names: Vec<&str> = tiles.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["喂食", "活动区换水", "垃圾清理"]);
    }

    #[test]
    fn latest_log_wins_for_days_since_last() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        log(&conn, c, "喂食", "2026-09-08 20:00:00");
        log(&conn, c, "喂食", "2026-09-16 20:00:00");

        let tiles = tiles_for_colony(&conn, c, TODAY).unwrap();
        assert_eq!(tile(&tiles, "喂食").days_since_last, Some(2), "取最近一次发生");
    }

    #[test]
    fn recent_lists_latest_two_with_food_names() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        let seed = food_id(&conn, "种子");
        let shrimp = food_id(&conn, "干虾仁");
        log(&conn, c, "喂食", "2026-09-17 20:00:00");
        log_care(
            &conn,
            &CareLogInput {
                colony_id: c,
                action_id: action_id(&conn, "喂食"),
                happened_at: "2026-09-18T08:00".into(),
                note: None,
                food_ids: vec![shrimp, seed],
            },
            "2026-09-18 08:05:00",
        )
        .unwrap();
        log(&conn, c, "巢穴保湿", "2026-09-12 09:00:00");

        let recent = recent_for_colony(&conn, c, 2).unwrap();
        assert_eq!(recent.len(), 2, "只取最近 1-2 条");
        assert_eq!(recent[0].action_name, "喂食");
        assert_eq!(recent[0].happened_at, "2026-09-18 08:00:00");
        assert_eq!(recent[0].food_names, vec!["种子", "干虾仁"], "log_food 无顺序列，按字典顺序展示");
        assert_eq!(recent[1].action_name, "喂食");
        assert_eq!(recent[1].food_names, Vec::<String>::new(), "无食物关联为空数组");
    }

    #[test]
    fn list_foods_returns_all_presets_including_disabled() {
        let conn = mem_conn();
        conn.execute("UPDATE food SET enabled = 0 WHERE name = '面包虫'", []).unwrap();
        let foods = list_foods(&conn).unwrap();
        let names: Vec<&str> = foods.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, vec!["种子", "干虾仁", "面包虫"]);
        assert_eq!(foods.iter().find(|f| f.name == "面包虫").unwrap().enabled, false);
    }
}
