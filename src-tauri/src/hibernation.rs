//! 冬眠段业务层（票 05）。
//!
//! 全部函数只吃 `&Connection`，与 Tauri 解耦，可被 cargo test 直接覆盖；
//! Tauri command 只是薄包装（见 lib.rs）。
//!
//! 行为对齐 spec：
//! - 开始冬眠：落开放段 + 窝状态置冬眠（同事务）；
//! - 确认出眠：回填实际结束日期 + 状态回活跃（同事务），实际结束可与预计不同；
//! - 补录历史段：两端闭合、不碰窝当前状态（活跃/冬眠中/已结束的窝都可补过去的时间段）；
//! - 约束全组：同窝各段不重叠（schema 无防护，应用层在此补齐票 01 硬性欠账）、
//!   结束日期 ≥ 开始日期（schema CHECK 兜底 + 应用层友好报错）、
//!   每窝至多一段开放段（部分唯一索引兜底 + 应用层预检）、
//!   已结束的窝不可开始冬眠；
//! - `hibernation_overlap_days`：区间与冬眠段的重叠自然日数（半开日空间），
//!   间隔统计扣减（行为规则 6）与统计页（票 07）共用。

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

// ── DTO ──────────────────────────────────────────────────────────────────

/// 一段冬眠期（actual_end_date 为空 = 开放段）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Hibernation {
    pub id: i64,
    pub colony_id: i64,
    pub start_date: String,
    pub expected_end_date: String,
    pub actual_end_date: Option<String>,
}

/// 首页卡片横幅用的开放段摘要（窝状态=冬眠时应恰有一段）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OpenSegment {
    pub id: i64,
    pub start_date: String,
    pub expected_end_date: String,
}

/// 纯函数入参的一段冬眠期。`end` 为 None = 开放段（尚未出眠，视为延伸到 +∞）。
// 本纯函数 API 随票 05 交付、票 06（提醒调度静音窗口）/票 07（间隔统计）接线，
// 接线后可去掉 allow。
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Segment {
    pub start: chrono::NaiveDate,
    pub end: Option<chrono::NaiveDate>,
}

impl Segment {
    /// 闭合段（入眠日与出眠日都算冬眠日）。
    #[allow(dead_code)]
    pub fn closed(start: chrono::NaiveDate, end: chrono::NaiveDate) -> Self {
        Segment { start, end: Some(end) }
    }

    /// 开放段（尚未出眠）。
    #[allow(dead_code)]
    pub fn open(start: chrono::NaiveDate) -> Self {
        Segment { start, end: None }
    }
}

// ── 日期 ─────────────────────────────────────────────────────────────────

fn parse_iso(s: &str) -> Result<chrono::NaiveDate, String> {
    chrono::NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d")
        .map_err(|_| format!("日期格式应为 YYYY-MM-DD：{s}"))
}

fn db_err(e: rusqlite::Error) -> String {
    format!("数据库操作失败: {e}")
}

// ── 重叠天数（纯函数，规则 6 的计算核心）────────────────────────────────

/// 区间 [range_start, range_end)（半开，按自然日）与冬眠段并集的重叠天数。
/// 开放段（end=None）视为延伸到区间右端之外；段先裁剪进区间再并集去重，
/// 即便输入段彼此重叠（应用层约束本不允许）也不重复计数；空区间返回 0。
// 统计页间隔扣减（规则 6）的共用入口：`cargo test hibernation_overlap` 有用例。
#[allow(dead_code)]
pub fn hibernation_overlap_days(
    range_start: chrono::NaiveDate,
    range_end: chrono::NaiveDate,
    segments: &[Segment],
) -> i64 {
    if range_end <= range_start {
        return 0;
    }
    // 每段裁剪进区间：闭合段按半开日空间 [start, end+1)（出眠日当天仍算冬眠日），
    // 开放段右端取区间右端。
    let mut clipped: Vec<(chrono::NaiveDate, chrono::NaiveDate)> = segments
        .iter()
        .filter_map(|s| {
            let lo = s.start.max(range_start);
            let hi = match s.end {
                Some(e) => e.succ_opt().unwrap_or(range_end).min(range_end),
                None => range_end,
            };
            (lo < hi).then_some((lo, hi))
        })
        .collect();
    clipped.sort();

    // 合并相接/重叠的段后累加，避免共享日重复计数
    let mut total = 0i64;
    let mut cur: Option<(chrono::NaiveDate, chrono::NaiveDate)> = None;
    for (lo, hi) in clipped {
        match cur {
            Some((cs, ce)) if lo <= ce => cur = Some((cs, ce.max(hi))),
            Some((cs, ce)) => {
                total += (ce - cs).num_days();
                cur = Some((lo, hi));
            }
            None => cur = Some((lo, hi)),
        }
    }
    if let Some((cs, ce)) = cur {
        total += (ce - cs).num_days();
    }
    total
}

// ── 约束校验 ─────────────────────────────────────────────────────────────

/// 应用层重叠预检：[new_start, new_end]（new_end 为 None = 开放段）与该窝既有
/// 任一段共享哪怕一个自然日即拒绝（相邻两段须至少隔一天）。exclude_id 供将来
/// 编辑段时排除自身，当前入口均为新建、恒传 None。
fn ensure_no_overlap(
    conn: &Connection,
    colony_id: i64,
    new_start: &str,
    new_end: Option<&str>,
    exclude_id: Option<i64>,
) -> Result<(), String> {
    let clashes: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM hibernation
             WHERE colony_id = ?1
               AND (?4 IS NULL OR id != ?4)
               AND (actual_end_date IS NULL OR actual_end_date >= ?2)
               AND (?3 IS NULL OR start_date <= ?3)",
            params![colony_id, new_start, new_end, exclude_id],
            |row| row.get(0),
        )
        .map_err(db_err)?;
    if clashes > 0 {
        return Err("与该窝既有冬眠段时间重叠（相邻两段至少要隔一天）".into());
    }
    Ok(())
}

// ── 开始 / 出眠 / 补录 / 列表 ────────────────────────────────────────────

/// 开始冬眠：插入开放段并把窝状态置冬眠（同事务）。
/// 仅活跃窝可入眠；结束日期不得早于开始日期；开始日期不得晚于今天
/// （预计结束日期是计划，允许在未来——spec 用户故事 10/11）；与既有段不重叠。
pub fn start_hibernation(
    conn: &Connection,
    colony_id: i64,
    start_date: &str,
    expected_end_date: &str,
    today: &str,
) -> Result<Hibernation, String> {
    let status: String = conn
        .query_row(
            "SELECT status FROM colony WHERE id = ?1",
            params![colony_id],
            |row| row.get(0),
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => "窝不存在".to_string(),
            other => db_err(other),
        })?;
    match status.as_str() {
        "ended" => return Err("已结束的窝不可开始冬眠".into()),
        "hibernating" => return Err("该窝已在冬眠中，请先确认出眠".into()),
        _ => {}
    }

    let start = parse_iso(start_date)?;
    let expected_end = parse_iso(expected_end_date)?;
    if expected_end < start {
        return Err(format!(
            "预计结束日期（{}）不能早于开始日期（{}）",
            expected_end_date.trim(),
            start_date.trim()
        ));
    }
    // 开始日期是"已发生的事实"，不得晚于今天（预计结束是计划，允许未来）
    let today_date = parse_iso(today).map_err(|_| format!("今天日期异常：{today}"))?;
    if start > today_date {
        return Err(format!(
            "开始日期（{}）不能晚于今天（{}）",
            start_date.trim(),
            today.trim()
        ));
    }

    let tx = conn.unchecked_transaction().map_err(db_err)?;
    ensure_no_overlap(&tx, colony_id, start_date.trim(), None, None)?;
    // 开放段预检（部分唯一索引兜底；防状态被手工改脏时重复落开放段）
    let open: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM hibernation WHERE colony_id = ?1 AND actual_end_date IS NULL",
            params![colony_id],
            |row| row.get(0),
        )
        .map_err(db_err)?;
    if open > 0 {
        return Err("该窝已有一段进行中的冬眠，请先确认出眠".into());
    }

    let start = start_date.trim().to_string();
    let expected_end = expected_end_date.trim().to_string();
    tx.execute(
        "INSERT INTO hibernation (colony_id, start_date, expected_end_date) VALUES (?1, ?2, ?3)",
        params![colony_id, start, expected_end],
    )
    .map_err(db_err)?;
    let id = tx.last_insert_rowid();
    // 状态联动与插段同事务：任一失败整体回滚
    tx.execute(
        "UPDATE colony SET status = 'hibernating' WHERE id = ?1",
        params![colony_id],
    )
    .map_err(db_err)?;
    tx.commit().map_err(db_err)?;

    Ok(Hibernation { id, colony_id, start_date: start, expected_end_date: expected_end, actual_end_date: None })
}

/// 确认出眠：回填该窝开放段的实际结束日期并把状态回活跃（同事务）。
/// 实际结束可与预计不同；不得早于该段入眠日，也不得晚于今天
/// （出眠是已发生的事实，care::ensure_not_future 的日期版，票 05 停靠②）。
pub fn confirm_wake(
    conn: &Connection,
    colony_id: i64,
    actual_end_date: &str,
    today: &str,
) -> Result<Hibernation, String> {
    let actual_end = parse_iso(actual_end_date)?;
    let (seg_id, start_date, expected_end_date): (i64, String, String) = conn
        .query_row(
            "SELECT id, start_date, expected_end_date FROM hibernation
             WHERE colony_id = ?1 AND actual_end_date IS NULL
             ORDER BY id DESC LIMIT 1",
            params![colony_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => "该窝没有进行中的冬眠段".to_string(),
            other => db_err(other),
        })?;
    let seg_start = parse_iso(&start_date)?;
    if actual_end < seg_start {
        return Err(format!(
            "实际结束日期（{}）不能早于入眠日期（{start_date}）",
            actual_end_date.trim()
        ));
    }
    // 出眠是"已发生的事实"，不得晚于今天（care::ensure_not_future 的日期版）
    let today_date = parse_iso(today).map_err(|_| format!("今天日期异常：{today}"))?;
    if actual_end > today_date {
        return Err(format!(
            "实际结束日期（{}）不能晚于今天（{}）",
            actual_end_date.trim(),
            today.trim()
        ));
    }

    let tx = conn.unchecked_transaction().map_err(db_err)?;
    let actual_end = actual_end_date.trim().to_string();
    tx.execute(
        "UPDATE hibernation SET actual_end_date = ?1 WHERE id = ?2",
        params![actual_end, seg_id],
    )
    .map_err(db_err)?;
    tx.execute(
        "UPDATE colony SET status = 'active' WHERE id = ?1",
        params![colony_id],
    )
    .map_err(db_err)?;
    tx.commit().map_err(db_err)?;

    Ok(Hibernation {
        id: seg_id,
        colony_id,
        start_date,
        expected_end_date,
        actual_end_date: Some(actual_end),
    })
}

/// 补录一段已闭合的历史冬眠（两端都填）：不碰窝当前状态；
/// 同样做日期先后与重叠校验。
pub fn add_past_hibernation(
    conn: &Connection,
    colony_id: i64,
    start_date: &str,
    end_date: &str,
) -> Result<Hibernation, String> {
    let exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM colony WHERE id = ?1",
            params![colony_id],
            |row| row.get(0),
        )
        .map_err(db_err)?;
    if exists == 0 {
        return Err("窝不存在".into());
    }

    let start = parse_iso(start_date)?;
    let end = parse_iso(end_date)?;
    if end < start {
        return Err(format!(
            "结束日期（{}）不能早于开始日期（{}）",
            end_date.trim(),
            start_date.trim()
        ));
    }
    ensure_no_overlap(conn, colony_id, start_date.trim(), Some(end_date.trim()), None)?;

    let start = start_date.trim().to_string();
    let end = end_date.trim().to_string();
    conn.execute(
        "INSERT INTO hibernation (colony_id, start_date, expected_end_date, actual_end_date)
         VALUES (?1, ?2, ?3, ?3)",
        params![colony_id, start, end],
    )
    .map_err(db_err)?;
    let id = conn.last_insert_rowid();
    Ok(Hibernation {
        id,
        colony_id,
        start_date: start,
        expected_end_date: end.clone(),
        actual_end_date: Some(end),
    })
}

/// 某窝全部冬眠段，按开始日期升序（同日按 id）。
pub fn list_hibernations(conn: &Connection, colony_id: i64) -> Result<Vec<Hibernation>, String> {
    let exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM colony WHERE id = ?1",
            params![colony_id],
            |row| row.get(0),
        )
        .map_err(db_err)?;
    if exists == 0 {
        return Err("窝不存在".into());
    }

    let mut stmt = conn
        .prepare(
            "SELECT id, colony_id, start_date, expected_end_date, actual_end_date
             FROM hibernation WHERE colony_id = ?1 ORDER BY start_date, id",
        )
        .map_err(db_err)?;
    let rows = stmt
        .query_map(params![colony_id], |row| {
            Ok(Hibernation {
                id: row.get(0)?,
                colony_id: row.get(1)?,
                start_date: row.get(2)?,
                expected_end_date: row.get(3)?,
                actual_end_date: row.get(4)?,
            })
        })
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?;
    Ok(rows)
}

/// 某窝的开放段摘要（首页冬眠横幅用）；无开放段返回 None。
pub fn open_segment(conn: &Connection, colony_id: i64) -> Result<Option<OpenSegment>, String> {
    conn.query_row(
        "SELECT id, start_date, expected_end_date FROM hibernation
         WHERE colony_id = ?1 AND actual_end_date IS NULL LIMIT 1",
        params![colony_id],
        |row| {
            Ok(OpenSegment {
                id: row.get(0)?,
                start_date: row.get(1)?,
                expected_end_date: row.get(2)?,
            })
        },
    )
    .optional()
    .map_err(db_err)
}

/// 某窝最近一次出眠日期：逐行解析 actual_end_date、跳过脏行后取最大合法值。
/// 旧实现 `MAX(actual_end_date)` 按文本取最大——脏行（如 'not-a-date'）字典序往往
/// 最大，一条脏行会把合法出眠日整个吞掉（票 05 停靠①，票 04 停靠①同口径）。
/// 无闭合段或全部脏行返回 None。
pub fn latest_wake_date(conn: &Connection, colony_id: i64) -> Option<chrono::NaiveDate> {
    let mut stmt = conn
        .prepare(
            "SELECT actual_end_date FROM hibernation
             WHERE colony_id = ?1 AND actual_end_date IS NOT NULL",
        )
        .ok()?;
    let rows = stmt
        .query_map(params![colony_id], |row| row.get::<_, String>(0))
        .ok()?;
    let mut best: Option<chrono::NaiveDate> = None;
    for row in rows.flatten() {
        if let Ok(day) = parse_iso(&row) {
            best = Some(match best {
                Some(prev) if prev >= day => prev,
                _ => day,
            });
        }
    }
    best
}

// ── 测试：只测外部行为（spec「Testing Decisions」）────────────────────────

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

    fn colony(conn: &Connection, name: &str) -> i64 {
        conn.execute(
            "INSERT INTO colony (name, start_date, status) VALUES (?1, '2026-01-20', 'active')",
            params![name],
        )
        .expect("建窝失败");
        conn.last_insert_rowid()
    }

    fn set_status(conn: &Connection, id: i64, status: &str) {
        conn.execute(
            "UPDATE colony SET status = ?1 WHERE id = ?2",
            params![status, id],
        )
        .expect("改状态失败");
    }

    fn status_of(conn: &Connection, id: i64) -> String {
        conn.query_row("SELECT status FROM colony WHERE id = ?1", params![id], |r| r.get(0))
            .expect("查状态失败")
    }

    /// 直插一段冬眠（绕过应用层校验，用于搭场景）。
    fn seg(
        conn: &Connection,
        colony_id: i64,
        start: &str,
        expected_end: &str,
        actual_end: Option<&str>,
    ) -> i64 {
        conn.execute(
            "INSERT INTO hibernation (colony_id, start_date, expected_end_date, actual_end_date)
             VALUES (?1, ?2, ?3, ?4)",
            params![colony_id, start, expected_end, actual_end],
        )
        .expect("插冬眠段失败");
        conn.last_insert_rowid()
    }

    fn hiber_count(conn: &Connection, colony_id: i64) -> i64 {
        conn.query_row(
            "SELECT COUNT(*) FROM hibernation WHERE colony_id = ?1",
            params![colony_id],
            |r| r.get(0),
        )
        .expect("计数失败")
    }

    fn d(s: &str) -> chrono::NaiveDate {
        chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").expect("测试日期应为合法 ISO")
    }

    // ── 开始冬眠：状态联动 + 约束全组（验收 1、2）──

    #[test]
    fn start_hibernation_creates_open_segment_and_sets_status_hibernating() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");

        let h = start_hibernation(&conn, c, "2026-12-01", "2027-03-01", "2026-12-01").unwrap();

        assert_eq!(h.colony_id, c);
        assert_eq!(h.start_date, "2026-12-01");
        assert_eq!(h.expected_end_date, "2027-03-01");
        assert_eq!(h.actual_end_date, None, "开始冬眠落的是开放段");
        assert_eq!(status_of(&conn, c), "hibernating", "开始冬眠 → 状态联动置冬眠");
        assert_eq!(hiber_count(&conn, c), 1);
    }

    #[test]
    fn start_hibernation_rejects_any_overlap_with_existing_segments() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        seg(&conn, c, "2025-12-01", "2026-03-01", Some("2026-03-01"));

        // 新段起点与既有段终点同天：共享 03-01 一天也算重叠
        let err = start_hibernation(&conn, c, "2026-03-01", "2026-04-01", "2026-03-01").unwrap_err();
        assert!(err.contains("重叠"), "实际错误：{err}");
        // 新段终点与既有段起点同天，同样重叠
        assert!(start_hibernation(&conn, c, "2025-11-01", "2025-12-01", "2026-03-01").is_err());
        // 完全包住既有段
        assert!(start_hibernation(&conn, c, "2025-11-01", "2026-04-01", "2026-03-01").is_err());
        // 落在既有段内部
        assert!(start_hibernation(&conn, c, "2025-12-15", "2026-01-15", "2026-03-01").is_err());

        // 隔一天即可（约束只禁共享自然日）
        assert!(start_hibernation(&conn, c, "2026-03-03", "2026-04-01", "2026-03-03").is_ok());
        assert_eq!(status_of(&conn, c), "hibernating", "后落的开放段同样联动状态");
    }

    #[test]
    fn start_hibernation_rejects_second_open_segment_even_when_status_is_dirty() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        let open_id = seg(&conn, c, "2025-12-01", "2026-03-01", None);
        // 脏数据：开放段在但状态被手工改回活跃（schema 只挡唯一开放段，这里测应用层预检）
        set_status(&conn, c, "active");

        // 与开放段重叠的新段 → 重叠拒绝
        let err = start_hibernation(&conn, c, "2026-12-01", "2027-03-01", "2026-12-01").unwrap_err();
        assert!(err.contains("重叠"), "实际错误：{err}");

        // 不重叠的过去段也拒：每窝至多一段开放段（部分唯一索引兜底 + 预检）
        let err = start_hibernation(&conn, c, "2025-01-01", "2025-02-01", "2025-01-01").unwrap_err();
        assert!(err.contains("冬眠"), "实际错误：{err}");
        assert_eq!(hiber_count(&conn, c), 1, "被拒不落库");
        assert_eq!(
            conn.query_row::<i64, _, _>(
                "SELECT COUNT(*) FROM hibernation WHERE id = ?1 AND actual_end_date IS NULL",
                params![open_id],
                |r| r.get(0),
            )
            .unwrap(),
            1,
            "既有开放段不受影响"
        );
    }

    #[test]
    fn start_hibernation_rejects_ended_and_hibernating_colonies() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");

        // 已结束的窝不可开始冬眠（spec schema 约束）
        set_status(&conn, c, "ended");
        let err = start_hibernation(&conn, c, "2026-12-01", "2027-03-01", "2026-12-01").unwrap_err();
        assert!(err.contains("已结束"), "实际错误：{err}");

        // 已在冬眠中的窝先出眠再入眠
        set_status(&conn, c, "hibernating");
        let err = start_hibernation(&conn, c, "2026-12-01", "2027-03-01", "2026-12-01").unwrap_err();
        assert!(err.contains("冬眠"), "实际错误：{err}");
    }

    #[test]
    fn start_hibernation_rejects_bad_order_bad_format_unknown_colony_and_leaves_no_row() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");

        // 结束日期早于开始日期（验收 1：日期先后拒绝）
        let err = start_hibernation(&conn, c, "2026-12-01", "2026-11-01", "2026-12-01").unwrap_err();
        assert!(err.contains("不能早于"), "实际错误：{err}");
        // 日期格式
        let err = start_hibernation(&conn, c, "2026/12/01", "2027-03-01", "2026-12-01").unwrap_err();
        assert!(err.contains("YYYY-MM-DD"), "实际错误：{err}");
        // 窝不存在
        let err = start_hibernation(&conn, 999, "2026-12-01", "2027-03-01", "2026-12-01").unwrap_err();
        assert!(err.contains("窝不存在"), "实际错误：{err}");

        // 同一天开始并结束（一天冬眠）允许；格式允许首尾空格
        assert!(start_hibernation(&conn, c, " 2026-12-01 ", "2026-12-01", "2026-12-01").is_ok());
        // 全部被拒的调用不落库（此时应只有最后成功那 1 条）
        assert_eq!(hiber_count(&conn, c), 1);
    }

    // ── 确认出眠（验收 2：状态回活跃 + 实际结束回填）──

    #[test]
    fn confirm_wake_backfills_actual_end_and_sets_status_active() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        start_hibernation(&conn, c, "2025-12-01", "2026-03-01", "2025-12-01").unwrap();

        // 实际出眠 02-15，与预计 03-01 不同没关系
        let h = confirm_wake(&conn, c, "2026-02-15", "2026-02-15").unwrap();
        assert_eq!(h.actual_end_date, Some("2026-02-15".into()));
        assert_eq!(h.expected_end_date, "2026-03-01");
        assert_eq!(status_of(&conn, c), "active", "确认出眠 → 状态回活跃");
        let (actual, expected): (Option<String>, String) = conn
            .query_row(
                "SELECT actual_end_date, expected_end_date FROM hibernation WHERE colony_id = ?1",
                params![c],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(actual.as_deref(), Some("2026-02-15"), "实际结束回填落库");
        assert_eq!(expected, "2026-03-01", "预计结束保持原值");
    }

    #[test]
    fn confirm_wake_allows_same_day_and_rejects_before_start_or_without_open_segment() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");

        // 没有开放段
        let err = confirm_wake(&conn, c, "2026-02-15", "2026-02-15").unwrap_err();
        assert!(err.contains("没有进行中的冬眠段"), "实际错误：{err}");

        start_hibernation(&conn, c, "2026-02-15", "2026-03-01", "2026-02-15").unwrap();
        // 实际结束早于入眠日
        let err = confirm_wake(&conn, c, "2026-02-14", "2026-02-15").unwrap_err();
        assert!(err.contains("不能早于"), "实际错误：{err}");
        // 出眠日 = 入眠日允许（当天进当天出）
        assert!(confirm_wake(&conn, c, "2026-02-15", "2026-02-15").is_ok());
        // 出眠后再确认：已无开放段
        let err = confirm_wake(&conn, c, "2026-02-16", "2026-02-16").unwrap_err();
        assert!(err.contains("没有进行中的冬眠段"), "实际错误：{err}");
    }

    #[test]
    fn start_hibernation_rejects_start_date_in_the_future() {
        // 票 05 停靠②：开始日期不得晚于今天（care::ensure_not_future 的日期版）。
        // 预计结束日期是计划，允许在未来（spec 用户故事 10/11 出眠提醒以此为锚）。
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");

        let err = start_hibernation(&conn, c, "2026-09-19", "2027-03-01", "2026-09-18").unwrap_err();
        assert!(err.contains("不能晚于今天"), "实际错误：{err}");
        assert_eq!(hiber_count(&conn, c), 0, "被拒不落库");

        // 今天开始 + 预计结束在未来：合法
        assert!(start_hibernation(&conn, c, "2026-09-18", "2027-03-01", "2026-09-18").is_ok());
        assert_eq!(status_of(&conn, c), "hibernating");
    }

    #[test]
    fn confirm_wake_rejects_actual_end_in_the_future() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        start_hibernation(&conn, c, "2026-02-15", "2026-03-01", "2026-02-15").unwrap();

        // 实际出眠日在未来 → 拒绝，状态与开放段不动
        let err = confirm_wake(&conn, c, "2026-02-20", "2026-02-19").unwrap_err();
        assert!(err.contains("不能晚于今天"), "实际错误：{err}");
        assert_eq!(status_of(&conn, c), "hibernating");

        // 今天出眠合法
        let h = confirm_wake(&conn, c, "2026-02-19", "2026-02-19").unwrap();
        assert_eq!(h.actual_end_date, Some("2026-02-19".into()));
        assert_eq!(status_of(&conn, c), "active");
    }

    // ── 补录历史段（不碰状态）──

    #[test]
    fn add_past_hibernation_backfills_closed_segment_without_touching_status() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");

        let h = add_past_hibernation(&conn, c, "2025-12-01", "2026-02-01").unwrap();
        assert_eq!(h.start_date, "2025-12-01");
        assert_eq!(h.expected_end_date, "2026-02-01", "历史段预计=实际（已闭合）");
        assert_eq!(h.actual_end_date, Some("2026-02-01".into()));
        assert_eq!(status_of(&conn, c), "active", "补录不改当前状态");

        // 冬眠中的窝同样可以补更早的历史段
        start_hibernation(&conn, c, "2026-12-01", "2027-03-01", "2026-12-01").unwrap();
        assert!(add_past_hibernation(&conn, c, "2024-12-01", "2025-01-31").is_ok());
    }

    #[test]
    fn add_past_hibernation_rejects_overlap_bad_order_unknown_colony() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        seg(&conn, c, "2025-12-01", "2026-02-01", Some("2026-02-01"));

        // 与闭合段共享一天
        let err = add_past_hibernation(&conn, c, "2026-02-01", "2026-03-01").unwrap_err();
        assert!(err.contains("重叠"), "实际错误：{err}");
        // 落在既有段内部
        assert!(add_past_hibernation(&conn, c, "2025-12-15", "2026-01-15").is_err());
        // 结束早于开始
        let err = add_past_hibernation(&conn, c, "2026-03-01", "2026-02-01").unwrap_err();
        assert!(err.contains("不能早于"), "实际错误：{err}");
        // 窝不存在
        let err = add_past_hibernation(&conn, 999, "2025-12-01", "2026-02-01").unwrap_err();
        assert!(err.contains("窝不存在"), "实际错误：{err}");

        assert_eq!(hiber_count(&conn, c), 1, "被拒不落库");
    }

    // ── 列表 ──

    #[test]
    fn list_hibernations_orders_by_start_and_marks_open_segment() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        seg(&conn, c, "2025-12-01", "2026-02-01", Some("2026-02-01"));
        seg(&conn, c, "2024-12-01", "2025-02-01", Some("2025-02-01"));
        seg(&conn, c, "2026-12-01", "2027-03-01", None);

        let list = list_hibernations(&conn, c).unwrap();
        let starts: Vec<&str> = list.iter().map(|h| h.start_date.as_str()).collect();
        assert_eq!(starts, vec!["2024-12-01", "2025-12-01", "2026-12-01"], "按开始日期升序");
        assert_eq!(list[0].actual_end_date.as_deref(), Some("2025-02-01"));
        assert_eq!(list[2].actual_end_date, None, "开放段 actual_end 为空");

        let err = list_hibernations(&conn, 999).unwrap_err();
        assert!(err.contains("窝不存在"), "实际错误：{err}");
    }

    #[test]
    fn latest_wake_date_takes_max_of_valid_rows_skipping_dirty() {
        // 票 05 停靠①：脏行（解析失败的 actual_end_date）被跳过，不丢合法出眠日。
        // 旧实现 MAX(actual_end_date) 取整列最大——脏行字典序往往最大（'x' > '2'），
        // 一条脏行会把合法出眠日整个吞掉。
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        seg(&conn, c, "2024-12-01", "2025-02-01", Some("2025-01-15"));
        seg(&conn, c, "2025-12-01", "2026-02-01", Some("not-a-date"));

        assert_eq!(
            latest_wake_date(&conn, c),
            Some(d("2025-01-15")),
            "脏出眠日跳过，取剩余合法行的最大值"
        );

        // 全部脏 / 无闭合段 → None
        let c2 = colony(&conn, "倒霉二号");
        seg(&conn, c2, "2025-12-01", "2026-02-01", Some("???"));
        assert_eq!(latest_wake_date(&conn, c2), None);
        let c3 = colony(&conn, " Fresh 三号");
        assert_eq!(latest_wake_date(&conn, c3), None);
    }

    // ── 重叠天数纯函数（规则 6，供统计/间隔扣减复用）──

    #[test]
    fn overlap_days_counts_natural_days_inside_range_with_clipping() {
        // 完整落在区间内：01-03..01-06 共 4 个自然日
        assert_eq!(
            hibernation_overlap_days(
                d("2026-01-01"),
                d("2026-01-11"),
                &[Segment::closed(d("2026-01-03"), d("2026-01-06"))]
            ),
            4
        );
        // 段跨区间左边界：只数区间内部分（01-01..01-05 共 5 天）
        assert_eq!(
            hibernation_overlap_days(
                d("2026-01-01"),
                d("2026-01-11"),
                &[Segment::closed(d("2025-12-20"), d("2026-01-05"))]
            ),
            5
        );
        // 开放段从区间中部延伸到右端之外：01-05..01-10 共 6 天
        assert_eq!(
            hibernation_overlap_days(
                d("2026-01-01"),
                d("2026-01-11"),
                &[Segment::open(d("2026-01-05"))]
            ),
            6
        );
        // 开放段完全罩住区间：整个区间 10 天
        assert_eq!(
            hibernation_overlap_days(
                d("2026-01-01"),
                d("2026-01-11"),
                &[Segment::open(d("2025-11-01"))]
            ),
            10
        );
        // 段在区间之外 / 空区间
        assert_eq!(
            hibernation_overlap_days(
                d("2026-01-01"),
                d("2026-01-11"),
                &[Segment::closed(d("2026-02-01"), d("2026-02-10"))]
            ),
            0
        );
        assert_eq!(
            hibernation_overlap_days(d("2026-01-11"), d("2026-01-01"), &[Segment::open(d("2026-01-01"))]),
            0,
            "range_end <= range_start 视为空区间"
        );
        assert_eq!(hibernation_overlap_days(d("2026-01-01"), d("2026-01-01"), &[]), 0);
    }

    #[test]
    fn overlap_days_unions_touching_or_overlapping_segments_without_double_count() {
        // 相接两段（共享 01-06 一天）：并集 01-03..01-09 共 7 天
        assert_eq!(
            hibernation_overlap_days(
                d("2026-01-01"),
                d("2026-01-11"),
                &[
                    Segment::closed(d("2026-01-03"), d("2026-01-06")),
                    Segment::closed(d("2026-01-06"), d("2026-01-09")),
                ]
            ),
            7
        );
        // 部分重叠两段（应用层约束本不允许，函数防御性去重）：并集 01-03..01-08 共 6 天
        assert_eq!(
            hibernation_overlap_days(
                d("2026-01-01"),
                d("2026-01-11"),
                &[
                    Segment::closed(d("2026-01-03"), d("2026-01-06")),
                    Segment::closed(d("2026-01-05"), d("2026-01-08")),
                ]
            ),
            6
        );
        // 不相交两段：各自计入
        assert_eq!(
            hibernation_overlap_days(
                d("2026-01-01"),
                d("2026-01-31"),
                &[
                    Segment::closed(d("2026-01-03"), d("2026-01-06")),
                    Segment::closed(d("2026-01-20"), d("2026-01-24")),
                ]
            ),
            4 + 5
        );
    }

    #[test]
    fn overlap_days_supports_interval_deduction_across_one_hibernation() {
        // 验收 4：跨 30 天冬眠的两次喂食，间隔 = 实际天数 − 重叠天数
        // 喂食 2026-11-01 → 2027-01-08：实际间隔 68 天；冬眠 2026-11-10..2026-12-09（30 天）
        let interval = (d("2027-01-08") - d("2026-11-01")).num_days();
        assert_eq!(interval, 68);
        let overlap = hibernation_overlap_days(
            d("2026-11-01"),
            d("2027-01-08"),
            &[Segment::closed(d("2026-11-10"), d("2026-12-09"))],
        );
        assert_eq!(overlap, 30, "喂食间隔与冬眠段重叠 30 天");
        assert_eq!(interval - overlap, 38, "扣冬眠后的有效间隔");
    }

    #[test]
    fn overlap_days_sums_each_hibernation_segment_within_one_interval() {
        // 验收 4：两段冬眠各自适用——喂食 2026-10-01 → 2027-04-01（182 天），
        // 冬眠 A 2026-11-10..2026-12-09（30 天）+ B 2027-01-20..2027-02-10（22 天）
        let interval = (d("2027-04-01") - d("2026-10-01")).num_days();
        assert_eq!(interval, 182);
        let overlap = hibernation_overlap_days(
            d("2026-10-01"),
            d("2027-04-01"),
            &[
                Segment::closed(d("2026-11-10"), d("2026-12-09")),
                Segment::closed(d("2027-01-20"), d("2027-02-10")),
            ],
        );
        assert_eq!(overlap, 52);
        assert_eq!(interval - overlap, 130);
    }
}
