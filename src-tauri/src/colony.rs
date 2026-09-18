//! 窝与地点的业务逻辑（票 02）。
//!
//! 全部函数只吃 `&Connection`，与 Tauri 解耦，可被 cargo test 直接覆盖；
//! Tauri command 只是薄包装（见 lib.rs）。
//!
//! 行为对齐 spec：
//! - 窝名 trim 后全局唯一（schema UNIQUE 兜底 + 应用层友好报错）；
//! - 饲养天数 = 今天 − 开始饲养日期的自然日天数（含冬眠，评审附录规则 9）；
//! - deleteColony 仅无记录窝（无 care_log 行）；有记录只能置「已结束」（规则 10 精神）。

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

/// 窝状态三选一（schema CHECK 同款）。
pub const STATUSES: [&str; 3] = ["active", "hibernating", "ended"];

// ── DTO ──────────────────────────────────────────────────────────────────

/// 窝（含给首页展示的饲养天数，Rust 算好直接给前端）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Colony {
    pub id: i64,
    pub name: String,
    pub species: Option<String>,
    pub location_id: Option<i64>,
    pub start_date: String,
    pub status: String,
    pub days_raised: i64,
}

/// 新建/编辑窝的入参。
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ColonyInput {
    pub name: String,
    pub species: Option<String>,
    pub location_id: Option<i64>,
    pub start_date: String,
    pub status: String,
}

/// 地点。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Location {
    pub id: i64,
    pub name: String,
    pub enabled: bool,
    pub sort: i64,
}

/// 新增/修改地点的入参：id 为空=新增，否则改名+排序（停用走单独 command）。
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct LocationInput {
    pub id: Option<i64>,
    pub name: String,
    pub sort: i64,
}

// ── 日期 ─────────────────────────────────────────────────────────────────

/// 本机今天的 ISO 日期（命令层取"今天"用；测试里直接传固定日期，不碰时钟）。
pub fn today_iso() -> String {
    chrono::Local::now().date_naive().format("%Y-%m-%d").to_string()
}

/// 饲养天数 = today − start_date 的自然日天数（含冬眠）。同一天为 0。
pub fn days_raised(start_date: &str, today: &str) -> Result<i64, String> {
    let start = parse_iso(start_date)
        .map_err(|_| "开始饲养日期格式应为 YYYY-MM-DD".to_string())?;
    let today = parse_iso(today).map_err(|_| "日期格式应为 YYYY-MM-DD".to_string())?;
    Ok((today - start).num_days())
}

fn parse_iso(s: &str) -> Result<chrono::NaiveDate, chrono::ParseError> {
    chrono::NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d")
}

// ── 窝 ───────────────────────────────────────────────────────────────────

/// 名字 trim 后非空且全局唯一（exclude_id 用于编辑时排除自己）。返回 trim 后的名字。
fn validate_colony_name(
    conn: &Connection,
    name: &str,
    exclude_id: Option<i64>,
) -> Result<String, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("窝名字不能为空".into());
    }
    let dupes: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM colony WHERE name = ?1 AND (?2 IS NULL OR id != ?2)",
            params![trimmed, exclude_id],
            |row| row.get(0),
        )
        .map_err(db_err)?;
    if dupes > 0 {
        return Err(format!("窝名字「{trimmed}」已存在，换个名字吧"));
    }
    Ok(trimmed.to_string())
}

fn validate_start_date(s: &str) -> Result<String, String> {
    parse_iso(s)
        .map(|_| s.trim().to_string())
        .map_err(|_| "开始饲养日期格式应为 YYYY-MM-DD".to_string())
}

fn validate_status(s: &str) -> Result<(), String> {
    if STATUSES.contains(&s) {
        Ok(())
    } else {
        Err(format!("无效的窝状态：{s}（应为 活跃/冬眠/已结束）"))
    }
}

fn validate_location_id(conn: &Connection, location_id: Option<i64>) -> Result<(), String> {
    if let Some(id) = location_id {
        let found: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM location WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .map_err(db_err)?;
        if found == 0 {
            return Err(format!("所选地点不存在（id={id}）"));
        }
    }
    Ok(())
}

fn db_err(e: rusqlite::Error) -> String {
    format!("数据库操作失败: {e}")
}

/// 唯一约束兜底报错转友好文案（应用层已预检，这里防并发/边界）。
/// 用扩展错误码 2067（SQLITE_CONSTRAINT_UNIQUE）识别唯一约束；
/// 文案匹配只用于区分撞的是哪条唯一索引，不再是识别手段。
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

fn get_colony(conn: &Connection, id: i64, today: &str) -> Result<Colony, String> {
    conn.query_row(
        "SELECT id, name, species, location_id, start_date, status FROM colony WHERE id = ?1",
        params![id],
        |row| {
            Ok(Colony {
                id: row.get(0)?,
                name: row.get(1)?,
                species: row.get(2)?,
                location_id: row.get(3)?,
                start_date: row.get(4)?,
                status: row.get(5)?,
                days_raised: 0,
            })
        },
    )
    .map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => "窝不存在".to_string(),
        other => db_err(other),
    })
    .and_then(|mut c| {
        c.days_raised = days_raised(&c.start_date, today)?;
        Ok(c)
    })
}

/// 校验并规整入参，返回 (trim 后名字, trim 非空物种或 NULL, 校验过的开始日期)。
fn normalize_colony_input(
    conn: &Connection,
    input: &ColonyInput,
    exclude_id: Option<i64>,
) -> Result<(String, Option<String>, String), String> {
    let name = validate_colony_name(conn, &input.name, exclude_id)?;
    let species = input
        .species
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let start_date = validate_start_date(&input.start_date)?;
    validate_status(&input.status)?;
    validate_location_id(conn, input.location_id)?;
    Ok((name, species, start_date))
}

/// 首页数据源：全部窝（已结束的也返回，前端归入底部折叠区），按 sort、id 排序。
pub fn list_colonies(conn: &Connection, today: &str) -> Result<Vec<Colony>, String> {
    let ids: Vec<i64> = {
        let mut stmt = conn
            .prepare("SELECT id FROM colony ORDER BY sort, id")
            .map_err(db_err)?;
        let rows = stmt
            .query_map([], |row| row.get(0))
            .map_err(db_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db_err)?;
        rows
    };
    ids.iter().map(|&id| get_colony(conn, id, today)).collect()
}

pub fn create_colony(conn: &Connection, input: &ColonyInput, today: &str) -> Result<Colony, String> {
    let (name, species, start_date) = normalize_colony_input(conn, input, None)?;
    conn.execute(
        "INSERT INTO colony (name, species, location_id, start_date, status) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![name, species, input.location_id, start_date, input.status],
    )
    .map_err(|e| friendly_unique_err(e, "colony.name", &name))?;
    get_colony(conn, conn.last_insert_rowid(), today)
}

pub fn update_colony(
    conn: &Connection,
    id: i64,
    input: &ColonyInput,
    today: &str,
) -> Result<Colony, String> {
    let (name, species, start_date) = normalize_colony_input(conn, input, Some(id))?;
    let changed = conn
        .execute(
            "UPDATE colony SET name = ?1, species = ?2, location_id = ?3, start_date = ?4, status = ?5 WHERE id = ?6",
            params![name, species, input.location_id, start_date, input.status, id],
        )
        .map_err(|e| friendly_unique_err(e, "colony.name", &name))?;
    if changed == 0 {
        return Err("窝不存在".into());
    }
    get_colony(conn, id, today)
}

/// 置为「已结束」（送人/死亡走这里，历史保留）。
pub fn archive_colony(conn: &Connection, id: i64, today: &str) -> Result<Colony, String> {
    let changed = conn
        .execute("UPDATE colony SET status = 'ended' WHERE id = ?1", params![id])
        .map_err(db_err)?;
    if changed == 0 {
        return Err("窝不存在".into());
    }
    get_colony(conn, id, today)
}

/// 仅无记录窝可删；有 care_log 行则拒绝。顺带清掉该窝的提醒台账与冬眠段（它们不是记录）。
/// 三条 DELETE 包在同一事务里，任一失败整体回滚，不留中间态。
pub fn delete_colony(conn: &Connection, id: i64) -> Result<(), String> {
    let logs: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM care_log WHERE colony_id = ?1",
            params![id],
            |row| row.get(0),
        )
        .map_err(db_err)?;
    if logs > 0 {
        return Err(format!(
            "该窝已有 {logs} 条记录，不能删除；可改为置为「已结束」"
        ));
    }
    let tx = conn.unchecked_transaction().map_err(db_err)?;
    tx.execute("DELETE FROM reminder_ledger WHERE colony_id = ?1", params![id])
        .map_err(db_err)?;
    tx.execute("DELETE FROM hibernation WHERE colony_id = ?1", params![id])
        .map_err(db_err)?;
    let changed = tx
        .execute("DELETE FROM colony WHERE id = ?1", params![id])
        .map_err(db_err)?;
    if changed == 0 {
        // tx 在此 drop，自动回滚前两条 DELETE
        return Err("窝不存在".into());
    }
    tx.commit().map_err(db_err)
}

// ── 地点 ─────────────────────────────────────────────────────────────────

fn row_to_location(row: &rusqlite::Row<'_>) -> rusqlite::Result<Location> {
    Ok(Location {
        id: row.get(0)?,
        name: row.get(1)?,
        enabled: row.get::<_, i64>(2)? != 0,
        sort: row.get(3)?,
    })
}

/// 全部地点（含停用的：首页分组还要按它排，只有新建/编辑入口过滤 enabled）。
pub fn list_locations(conn: &Connection) -> Result<Vec<Location>, String> {
    let mut stmt = conn
        .prepare("SELECT id, name, enabled, sort FROM location ORDER BY sort, id")
        .map_err(db_err)?;
    let rows = stmt
        .query_map([], row_to_location)
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?;
    Ok(rows)
}

pub fn save_location(conn: &Connection, input: &LocationInput) -> Result<Location, String> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err("地点名字不能为空".into());
    }
    let dupes: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM location WHERE name = ?1 AND (?2 IS NULL OR id != ?2)",
            params![name, input.id],
            |row| row.get(0),
        )
        .map_err(db_err)?;
    if dupes > 0 {
        return Err(format!("地点名字「{name}」已存在"));
    }

    match input.id {
        None => {
            conn.execute(
                "INSERT INTO location (name, enabled, sort) VALUES (?1, 1, ?2)",
                params![name, input.sort],
            )
            .map_err(|e| friendly_unique_err(e, "location.name", name))?;
            let id = conn.last_insert_rowid();
            Ok(Location { id, name: name.into(), enabled: true, sort: input.sort })
        }
        Some(id) => {
            let changed = conn
                .execute(
                    "UPDATE location SET name = ?1, sort = ?2 WHERE id = ?3",
                    params![name, input.sort, id],
                )
                .map_err(|e| friendly_unique_err(e, "location.name", name))?;
            if changed == 0 {
                return Err("地点不存在".into());
            }
            conn.query_row(
                "SELECT id, name, enabled, sort FROM location WHERE id = ?1",
                params![id],
                row_to_location,
            )
            .map_err(db_err)
        }
    }
}

/// 停用（规则 10：被引用的只能停用，不能物理删）。
pub fn deactivate_location(conn: &Connection, id: i64) -> Result<(), String> {
    let changed = conn
        .execute("UPDATE location SET enabled = 0 WHERE id = ?1", params![id])
        .map_err(db_err)?;
    if changed == 0 {
        return Err("地点不存在".into());
    }
    Ok(())
}

/// 物理删；仍被窝引用则拒绝。
pub fn erase_location(conn: &Connection, id: i64) -> Result<(), String> {
    let used_by: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM colony WHERE location_id = ?1",
            params![id],
            |row| row.get(0),
        )
        .map_err(db_err)?;
    if used_by > 0 {
        return Err(format!("该地点仍被 {used_by} 个窝使用，不能删除；可改为停用"));
    }
    let changed = conn
        .execute("DELETE FROM location WHERE id = ?1", params![id])
        .map_err(db_err)?;
    if changed == 0 {
        return Err("地点不存在".into());
    }
    Ok(())
}

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

    fn input(name: &str, location_id: Option<i64>) -> ColonyInput {
        ColonyInput {
            name: name.into(),
            species: Some("大头收获蚁".into()),
            location_id,
            start_date: "2026-01-20".into(),
            status: "active".into(),
        }
    }

    fn count(conn: &Connection, sql: &str, id: i64) -> i64 {
        conn.query_row(sql, params![id], |row| row.get(0))
            .expect("标量查询失败")
    }

    fn insert_care_log(conn: &Connection, colony_id: i64) {
        conn.execute(
            "INSERT INTO care_log (colony_id, action_id, occurred_at, note, created_at)
             VALUES (?1, 1, '2026-09-17 20:00:00', '', '2026-09-17 20:00:00')",
            params![colony_id],
        )
        .expect("插入记录失败");
    }

    // ── 饲养天数 ──

    #[test]
    fn days_raised_same_day_is_zero() {
        assert_eq!(days_raised("2026-09-18", "2026-09-18").unwrap(), 0);
    }

    #[test]
    fn days_raised_matches_natural_day_span() {
        // 与 spec 示例一致：2026-01-20 → 2026-09-18 = 241
        assert_eq!(days_raised("2026-01-20", "2026-09-18").unwrap(), 241);
    }

    #[test]
    fn days_raised_spans_leap_day() {
        assert_eq!(days_raised("2028-02-28", "2028-03-01").unwrap(), 2);
    }

    #[test]
    fn days_raised_future_start_is_negative() {
        assert_eq!(days_raised("2026-09-19", "2026-09-18").unwrap(), -1);
    }

    #[test]
    fn days_raised_rejects_bad_format() {
        let err = days_raised("2026/01/20", "2026-09-18").unwrap_err();
        assert!(err.contains("YYYY-MM-DD"));
    }

    // ── 新建窝 ──

    #[test]
    fn create_colony_trims_name_and_persists() {
        let conn = mem_conn();
        let c = create_colony(&conn, &input("  大头一号  ", Some(1)), TODAY).unwrap();
        assert_eq!(c.name, "大头一号");
        assert_eq!(c.location_id, Some(1));
        assert_eq!(c.days_raised, 241);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM colony WHERE id = ?1", c.id), 1);
    }

    #[test]
    fn create_colony_trims_species_and_keeps_null_location() {
        let conn = mem_conn();
        let mut inp = input("无家窝", None);
        inp.species = Some("  针毛收获蚁  ".into());
        let c = create_colony(&conn, &inp, TODAY).unwrap();
        assert_eq!(c.species.as_deref(), Some("针毛收获蚁"));
        assert_eq!(c.location_id, None);
    }

    #[test]
    fn create_colony_rejects_duplicate_name_ignoring_spaces() {
        let conn = mem_conn();
        create_colony(&conn, &input("大头一号", Some(1)), TODAY).unwrap();
        // 首尾空格差异也算重名
        let err = create_colony(&conn, &input("  大头一号 ", Some(2)), TODAY).unwrap_err();
        assert!(err.contains("已存在"), "实际错误：{err}");
    }

    #[test]
    fn create_colony_rejects_blank_name() {
        let conn = mem_conn();
        assert!(create_colony(&conn, &input("   ", None), TODAY)
            .unwrap_err()
            .contains("不能为空"));
    }

    #[test]
    fn create_colony_rejects_bad_date_status_and_location() {
        let conn = mem_conn();
        let mut bad_date = input("甲", None);
        bad_date.start_date = "01/20/2026".into();
        assert!(create_colony(&conn, &bad_date, TODAY).is_err());

        let mut bad_status = input("乙", None);
        bad_status.status = "sleeping".into();
        assert!(create_colony(&conn, &bad_status, TODAY).is_err());

        let bad_location = input("丙", Some(999));
        let err = create_colony(&conn, &bad_location, TODAY).unwrap_err();
        assert!(err.contains("地点"), "实际错误：{err}");

        // 三个都没写进库
        let total: i64 = conn
            .query_row("SELECT COUNT(*) FROM colony", [], |row| row.get(0))
            .unwrap();
        assert_eq!(total, 0);
    }

    // ── 编辑窝 ──

    #[test]
    fn update_colony_changes_fields() {
        let conn = mem_conn();
        let c = create_colony(&conn, &input("大头一号", Some(1)), TODAY).unwrap();
        let mut inp = input("大头壹号", Some(2));
        inp.status = "hibernating".into();
        let updated = update_colony(&conn, c.id, &inp, TODAY).unwrap();
        assert_eq!(updated.name, "大头壹号");
        assert_eq!(updated.location_id, Some(2));
        assert_eq!(updated.status, "hibernating");
    }

    #[test]
    fn update_colony_keeps_own_name_and_rejects_foreign_name() {
        let conn = mem_conn();
        let a = create_colony(&conn, &input("大头一号", Some(1)), TODAY).unwrap();
        let _b = create_colony(&conn, &input("针毛一号", Some(2)), TODAY).unwrap();

        // 自己的名字（带空格）不算重名
        assert!(update_colony(&conn, a.id, &input(" 大头一号 ", Some(1)), TODAY).is_ok());
        // 改成别人的名字（含空格差异）被拒
        let err = update_colony(&conn, a.id, &input(" 针毛一号", Some(1)), TODAY).unwrap_err();
        assert!(err.contains("已存在"), "实际错误：{err}");
    }

    #[test]
    fn update_colony_missing_id_rejected() {
        let conn = mem_conn();
        assert!(update_colony(&conn, 42, &input("幽灵", None), TODAY).is_err());
    }

    // ── 列表 ──

    #[test]
    fn list_colonies_orders_by_sort_then_id_and_computes_days() {
        let conn = mem_conn();
        let a = create_colony(&conn, &input("窝A", Some(1)), TODAY).unwrap();
        let b = create_colony(&conn, &input("窝B", Some(1)), TODAY).unwrap();
        let c = create_colony(&conn, &input("窝C", None), TODAY).unwrap();
        // 让 B 排到 A 前面
        conn.execute("UPDATE colony SET sort = -1 WHERE id = ?1", params![b.id])
            .unwrap();

        let list = list_colonies(&conn, TODAY).unwrap();
        let ids: Vec<i64> = list.iter().map(|c| c.id).collect();
        assert_eq!(ids, vec![b.id, a.id, c.id]);
        assert!(list.iter().all(|c| c.days_raised == 241));
    }

    // ── 归档 ──

    #[test]
    fn archive_colony_sets_status_ended() {
        let conn = mem_conn();
        let c = create_colony(&conn, &input("大头一号", Some(1)), TODAY).unwrap();
        let archived = archive_colony(&conn, c.id, TODAY).unwrap();
        assert_eq!(archived.status, "ended");
        assert_eq!(archived.days_raised, 241, "已结束的窝仍显示饲养天数");
    }

    #[test]
    fn archive_colony_missing_id_rejected() {
        let conn = mem_conn();
        assert!(archive_colony(&conn, 42, TODAY).is_err());
    }

    // ── 删除 ──

    #[test]
    fn delete_colony_without_logs_succeeds_and_cleans_side_tables() {
        let conn = mem_conn();
        let c = create_colony(&conn, &input("短命窝", None), TODAY).unwrap();
        // 造一条冬眠段 + 一条台账（不是 care_log 记录），删除应连带清掉
        conn.execute(
            "INSERT INTO hibernation (colony_id, start_date, expected_end_date)
             VALUES (?1, '2026-12-01', '2027-03-01')",
            params![c.id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO reminder_ledger (colony_id, kind, action_id, base_date, sent_at)
             VALUES (?1, 'overdue', 1, '2026-09-10', '2026-09-10 08:00:00')",
            params![c.id],
        )
        .unwrap();

        delete_colony(&conn, c.id).unwrap();
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM colony WHERE id = ?1", c.id), 0);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM hibernation WHERE colony_id = ?1", c.id), 0);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM reminder_ledger WHERE colony_id = ?1", c.id), 0);
    }

    #[test]
    fn delete_colony_with_log_rejected_and_row_kept() {
        let conn = mem_conn();
        let c = create_colony(&conn, &input("有账窝", None), TODAY).unwrap();
        insert_care_log(&conn, c.id);

        let err = delete_colony(&conn, c.id).unwrap_err();
        assert!(err.contains("记录"), "实际错误：{err}");
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM colony WHERE id = ?1", c.id), 1);
    }

    #[test]
    fn delete_colony_missing_id_rejected() {
        let conn = mem_conn();
        assert!(delete_colony(&conn, 42).is_err());
    }

    #[test]
    fn delete_colony_failure_rolls_back_side_table_deletes() {
        let conn = mem_conn();
        let c = create_colony(&conn, &input("倒楣窝", None), TODAY).unwrap();
        conn.execute(
            "INSERT INTO hibernation (colony_id, start_date, expected_end_date)
             VALUES (?1, '2026-12-01', '2027-03-01')",
            params![c.id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO reminder_ledger (colony_id, kind, action_id, base_date, sent_at)
             VALUES (?1, 'overdue', 1, '2026-09-10', '2026-09-10 08:00:00')",
            params![c.id],
        )
        .unwrap();
        // 让最后的 colony DELETE 失败，模拟中途崩溃
        conn.execute(
            "CREATE TRIGGER block_colony_delete BEFORE DELETE ON colony
             BEGIN SELECT RAISE(ABORT, 'blocked'); END",
            [],
        )
        .unwrap();

        assert!(delete_colony(&conn, c.id).is_err());
        // 三张表必须原样都在：台账/冬眠段不能被删掉一半
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM reminder_ledger WHERE colony_id = ?1", c.id), 1);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM hibernation WHERE colony_id = ?1", c.id), 1);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM colony WHERE id = ?1", c.id), 1);
    }

    // ── 地点 ──

    #[test]
    fn list_locations_orders_by_sort_and_includes_presets() {
        let conn = mem_conn();
        let locs = list_locations(&conn).unwrap();
        let names: Vec<&str> = locs.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, vec!["家", "公司"]);
        assert!(locs.iter().all(|l| l.enabled));
    }

    #[test]
    fn save_location_creates_updates_and_rejects_duplicate() {
        let conn = mem_conn();
        let created = save_location(&conn, &LocationInput { id: None, name: "阳台".into(), sort: 5 }).unwrap();
        assert_eq!((created.name.as_str(), created.sort, created.enabled), ("阳台", 5, true));

        let updated = save_location(&conn, &LocationInput { id: Some(created.id), name: " 阳台B ".into(), sort: 0 }).unwrap();
        assert_eq!(updated.name, "阳台B");
        assert_eq!(updated.sort, 0);

        // 与预置「家」重名被拒
        let err = save_location(&conn, &LocationInput { id: None, name: "家".into(), sort: 9 }).unwrap_err();
        assert!(err.contains("已存在"), "实际错误：{err}");
        // 自己改名时保留自己的名字
        assert!(save_location(&conn, &LocationInput { id: Some(created.id), name: "阳台B".into(), sort: 1 }).is_ok());
    }

    #[test]
    fn save_location_blank_name_and_missing_id_rejected() {
        let conn = mem_conn();
        assert!(save_location(&conn, &LocationInput { id: None, name: "  ".into(), sort: 0 }).is_err());
        assert!(save_location(&conn, &LocationInput { id: Some(999), name: "不存在".into(), sort: 0 }).is_err());
    }

    #[test]
    fn deactivate_location_disables_but_keeps_row() {
        let conn = mem_conn();
        let locs = list_locations(&conn).unwrap();
        let home_id = locs[0].id;
        deactivate_location(&conn, home_id).unwrap();
        let locs = list_locations(&conn).unwrap();
        let home = locs.iter().find(|l| l.id == home_id).unwrap();
        assert!(!home.enabled);
    }

    #[test]
    fn erase_location_without_colony_succeeds() {
        let conn = mem_conn();
        let created = save_location(&conn, &LocationInput { id: None, name: "阳台".into(), sort: 5 }).unwrap();
        erase_location(&conn, created.id).unwrap();
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM location WHERE id = ?1", created.id), 0);
    }

    #[test]
    fn erase_location_referenced_by_colony_rejected_then_deactivate_works() {
        let conn = mem_conn();
        let home_id = list_locations(&conn).unwrap()[0].id;
        create_colony(&conn, &input("大头一号", Some(home_id)), TODAY).unwrap();

        let err = erase_location(&conn, home_id).unwrap_err();
        assert!(err.contains("停用"), "实际错误：{err}");
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM location WHERE id = ?1", home_id), 1);

        deactivate_location(&conn, home_id).unwrap();
        assert!(!list_locations(&conn).unwrap().iter().any(|l| l.id == home_id && l.enabled));
    }

    #[test]
    fn erase_missing_location_rejected() {
        let conn = mem_conn();
        assert!(erase_location(&conn, 999).is_err());
    }
}
