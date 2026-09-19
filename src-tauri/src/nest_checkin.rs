//! 巢况登记（蚁口/换巢/备注时间线，webui-checkin 票 02）。
//!
//! 全部函数只吃 `&Connection`，与 Tauri 解耦，可被 cargo test 直接覆盖；
//! Tauri command 只是薄包装（见 lib.rs）。
//!
//! 行为对齐规格 Implementation Decisions D/E 与 User Stories 7/8/10/17：
//! - 日期可补录（过去合法），未来日期拒绝（沿用 care.rs「不预记未来」口径，当天允许）；
//! - 至少一项非空才可提交（蚁后数/工蚁数/换巢/备注全空拒绝）；
//! - 基线 = 最早一条登记的日期字段，纯查询侧投影（首条日期可改；删首条后
//!   下一条最早者自然接任）；
//! - 照片元数据（nest_photo）本票只读不写（写入随票 07 照片管线），读取侧照常
//!   JOIN，无照片即空数组；
//! - 巢况永不参与提醒/催促（reminder.rs 不引用本模块，不进维护操作清单）。

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

// ── DTO ──────────────────────────────────────────────────────────────────

/// 照片元数据一行（本票恒空：无写入路径，票 07 接管线后生效）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NestPhotoMeta {
    pub id: i64,
    pub checkin_id: i64,
    /// photos/ 下相对路径 `<colonyId>/<uuid>.jpg`（落盘文件名一律服务端 UUID）。
    pub rel_path: String,
    /// 客户端原始文件名，仅备注、不参与路径。
    pub original_name: Option<String>,
    pub note: String,
}

/// 巢况登记一行（时间线/列表通用）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NestCheckin {
    pub id: i64,
    pub colony_id: i64,
    /// 登记日期 `YYYY-MM-DD`（可补录过去）。
    pub date: String,
    pub queen_count: Option<i64>,
    pub worker_count: Option<i64>,
    pub moved_nest: bool,
    pub note: String,
    pub created_at: String,
    pub photos: Vec<NestPhotoMeta>,
}

/// 新增登记入参（spec 用户故事 7：日期、蚁后数、工蚁数都可空、换巢标记、备注；
/// 至少一项非空才可提交——本票无照片字段，照片随票 07）。
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct CheckinInput {
    pub colony_id: i64,
    pub date: String,
    #[serde(default)]
    pub queen_count: Option<i64>,
    #[serde(default)]
    pub worker_count: Option<i64>,
    #[serde(default)]
    pub moved_nest: bool,
    #[serde(default)]
    pub note: Option<String>,
}

/// 编辑入参：全量覆盖（编辑弹窗整单提交；数可清回「未数」，日期必填）。
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct CheckinUpdateInput {
    pub date: String,
    #[serde(default)]
    pub queen_count: Option<i64>,
    #[serde(default)]
    pub worker_count: Option<i64>,
    #[serde(default)]
    pub moved_nest: bool,
    #[serde(default)]
    pub note: Option<String>,
}

/// 窝卡片/窝详情的巢况摘要（Rust 算好展示值，挂在 Colony.checkin 上）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CheckinDigest {
    /// 最新一组数（按日期最新、同日按录入最新）；从未登记为 None。
    pub latest: Option<NestCheckin>,
    /// 基线 = 最早一条登记的日期字段；从未登记为 None。
    pub baseline_date: Option<String>,
    /// 距上次登记 = 今天 − 最新登记日期（自然日，当天 0）；从未登记为 None。
    pub days_since_last: Option<i64>,
}

// ── 校验 ─────────────────────────────────────────────────────────────────

fn db_err(e: rusqlite::Error) -> String {
    format!("数据库操作失败: {e}")
}

/// 日期规整：`YYYY-MM-DD`（trim 后解析回写），沿用 care.rs 的日期规约风格。
fn parse_checkin_date(s: &str) -> Result<String, String> {
    chrono::NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d")
        .map(|d| d.format("%Y-%m-%d").to_string())
        .map_err(|_| format!("日期格式应为 YYYY-MM-DD：{s}"))
}

/// 登记日期不得晚于今天（补录合法、预记未来不合法；当天允许）。
fn ensure_not_future_date(date: &str, today: &str) -> Result<(), String> {
    if date > today {
        return Err(format!("登记日期不能晚于今天（{date} 在未来）"));
    }
    Ok(())
}

fn validate_counts(queen_count: Option<i64>, worker_count: Option<i64>) -> Result<(), String> {
    if queen_count.is_some_and(|n| n < 0) {
        return Err("蚁后数不能为负数".into());
    }
    if worker_count.is_some_and(|n| n < 0) {
        return Err("工蚁数不能为负数".into());
    }
    Ok(())
}

/// 至少一项非空：蚁后数/工蚁数任一填了、换巢勾了、备注非空，四者取一即可。
fn ensure_something_filled(
    queen_count: Option<i64>,
    worker_count: Option<i64>,
    moved_nest: bool,
    note: &str,
) -> Result<(), String> {
    if queen_count.is_none() && worker_count.is_none() && !moved_nest && note.is_empty() {
        return Err("至少填一项：蚁后数 / 工蚁数 / 换巢 / 备注".into());
    }
    Ok(())
}

fn ensure_colony_exists(conn: &Connection, colony_id: i64) -> Result<(), String> {
    let found: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM colony WHERE id = ?1",
            params![colony_id],
            |row| row.get(0),
        )
        .map_err(db_err)?;
    if found == 0 {
        return Err("窝不存在".into());
    }
    Ok(())
}

/// 保存前共通校验：返回（规整日期、trim 后备注）。
fn validate_fields(
    date: &str,
    queen_count: Option<i64>,
    worker_count: Option<i64>,
    moved_nest: bool,
    note: Option<&str>,
    today: &str,
) -> Result<(String, String), String> {
    let date = parse_checkin_date(date)?;
    ensure_not_future_date(&date, today)?;
    validate_counts(queen_count, worker_count)?;
    let note = note.map(str::trim).unwrap_or("").to_string();
    ensure_something_filled(queen_count, worker_count, moved_nest, &note)?;
    Ok((date, note))
}

// ── 增查改删 ─────────────────────────────────────────────────────────────

/// 记一条巢况：插入 nest_checkin，返回带照片元数据的完整行（本票 photos 恒空）。
pub fn save_checkin(
    conn: &Connection,
    input: &CheckinInput,
    today: &str,
    now: &str,
) -> Result<NestCheckin, String> {
    ensure_colony_exists(conn, input.colony_id)?;
    let (date, note) = validate_fields(
        &input.date,
        input.queen_count,
        input.worker_count,
        input.moved_nest,
        input.note.as_deref(),
        today,
    )?;
    conn.execute(
        "INSERT INTO nest_checkin
             (colony_id, date, queen_count, worker_count, moved_nest, note, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            input.colony_id,
            date,
            input.queen_count,
            input.worker_count,
            input.moved_nest,
            note,
            now
        ],
    )
    .map_err(db_err)?;
    get_checkin(conn, conn.last_insert_rowid())
}

/// 照片元数据（按 id 序；本票恒空数组）。
fn load_photos(conn: &Connection, checkin_id: i64) -> Result<Vec<NestPhotoMeta>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, checkin_id, rel_path, original_name, note
             FROM nest_photo WHERE checkin_id = ?1 ORDER BY id",
        )
        .map_err(db_err)?;
    let rows = stmt
        .query_map(params![checkin_id], |row| {
            Ok(NestPhotoMeta {
                id: row.get(0)?,
                checkin_id: row.get(1)?,
                rel_path: row.get(2)?,
                original_name: row.get(3)?,
                note: row.get(4)?,
            })
        })
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?;
    Ok(rows)
}

fn row_to_checkin(row: &rusqlite::Row<'_>) -> rusqlite::Result<(i64, i64, String, Option<i64>, Option<i64>, bool, String, String)> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get::<_, i64>(5)? != 0,
        row.get(6)?,
        row.get(7)?,
    ))
}

/// 登记是否存在（webui-checkin 票 08 照片上传端点的快速失败预检——不存在的
/// 登记不烧解码重编码的 CPU；权威归属查询仍是 photo::attach_photos 的反查）。
pub fn checkin_exists(conn: &Connection, id: i64) -> Result<bool, String> {
    let found: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM nest_checkin WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| format!("数据库操作失败: {e}"))?;
    Ok(found.is_some())
}

/// 单条登记（保存/编辑后回读用）。
pub fn get_checkin(conn: &Connection, id: i64) -> Result<NestCheckin, String> {
    let (id, colony_id, date, queen_count, worker_count, moved_nest, note, created_at) = conn
        .query_row(
            "SELECT id, colony_id, date, queen_count, worker_count, moved_nest, note, created_at
             FROM nest_checkin WHERE id = ?1",
            params![id],
            row_to_checkin,
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => "登记不存在".to_string(),
            other => db_err(other),
        })?;
    let photos = load_photos(conn, id)?;
    Ok(NestCheckin {
        id,
        colony_id,
        date,
        queen_count,
        worker_count,
        moved_nest,
        note,
        created_at,
        photos,
    })
}

/// 某窝巢况时间线：日期倒序（同日按录入 id 倒序），带照片元数据占位。
pub fn list_checkins(conn: &Connection, colony_id: i64) -> Result<Vec<NestCheckin>, String> {
    ensure_colony_exists(conn, colony_id)?;
    let ids: Vec<i64> = {
        let mut stmt = conn
            .prepare(
                "SELECT id FROM nest_checkin WHERE colony_id = ?1 ORDER BY date DESC, id DESC",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map(params![colony_id], |row| row.get(0))
            .map_err(db_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db_err)?;
        rows
    };
    ids.iter().map(|&id| get_checkin(conn, id)).collect()
}

/// 编辑一条登记（全量覆盖；首条日期可改 → 基线随之顺延，见 digest_for_colony）。
pub fn update_checkin(
    conn: &Connection,
    id: i64,
    input: &CheckinUpdateInput,
    today: &str,
) -> Result<NestCheckin, String> {
    let exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM nest_checkin WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )
        .map_err(db_err)?;
    if exists == 0 {
        return Err("登记不存在".into());
    }
    let (date, note) = validate_fields(
        &input.date,
        input.queen_count,
        input.worker_count,
        input.moved_nest,
        input.note.as_deref(),
        today,
    )?;
    conn.execute(
        "UPDATE nest_checkin
         SET date = ?1, queen_count = ?2, worker_count = ?3, moved_nest = ?4, note = ?5
         WHERE id = ?6",
        params![date, input.queen_count, input.worker_count, input.moved_nest, note, id],
    )
    .map_err(db_err)?;
    get_checkin(conn, id)
}

/// 删除一条登记：照片元数据行同事务一并删（照片文件清理随票 07 接管，本票无文件）。
pub fn delete_checkin(conn: &Connection, id: i64) -> Result<(), String> {
    let exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM nest_checkin WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )
        .map_err(db_err)?;
    if exists == 0 {
        return Err("登记不存在".into());
    }
    let tx = conn.unchecked_transaction().map_err(db_err)?;
    tx.execute("DELETE FROM nest_photo WHERE checkin_id = ?1", params![id])
        .map_err(db_err)?;
    tx.execute("DELETE FROM nest_checkin WHERE id = ?1", params![id])
        .map_err(db_err)?;
    tx.commit().map_err(db_err)?;
    Ok(())
}

/// 巢况摘要（基线投影）：latest = 最新一组数，baseline_date = 最早登记日期，
/// days_since_last = 今天 − 最新登记日期。从未登记三者皆 None。
/// 基线是纯查询投影：首条日期可改（改完基线跟走），删首条后 MIN 自然落到下一条。
pub fn digest_for_colony(
    conn: &Connection,
    colony_id: i64,
    today: &str,
) -> Result<CheckinDigest, String> {
    let latest_id: Option<i64> = conn
        .query_row(
            "SELECT id FROM nest_checkin WHERE colony_id = ?1 ORDER BY date DESC, id DESC LIMIT 1",
            params![colony_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(db_err)?;
    let baseline_date: Option<String> = conn
        .query_row(
            "SELECT MIN(date) FROM nest_checkin WHERE colony_id = ?1",
            params![colony_id],
            |row| row.get(0),
        )
        .map_err(db_err)?;
    let latest = match latest_id {
        Some(id) => Some(get_checkin(conn, id)?),
        None => None,
    };
    let days_since_last = match &latest {
        Some(c) => crate::care::days_since_last(Some(&c.date), today)?,
        None => None,
    };
    Ok(CheckinDigest {
        latest,
        baseline_date,
        days_since_last,
    })
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
    const NOW: &str = "2026-09-18 21:00:00";

    fn colony(conn: &Connection, name: &str) -> i64 {
        conn.execute(
            "INSERT INTO colony (name, start_date, status) VALUES (?1, '2026-01-20', 'active')",
            params![name],
        )
        .expect("建窝失败");
        conn.last_insert_rowid()
    }

    /// 全空入参（至少一项非空的拒绝基线）。
    fn empty_input(colony_id: i64, date: &str) -> CheckinInput {
        CheckinInput {
            colony_id,
            date: date.into(),
            queen_count: None,
            worker_count: None,
            moved_nest: false,
            note: None,
        }
    }

    fn count(conn: &Connection, sql: &str) -> i64 {
        conn.query_row(sql, [], |row| row.get(0)).expect("标量查询失败")
    }

    // ── 新增 ──

    #[test]
    fn save_checkin_persists_all_fields_and_normalizes() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");

        let saved = save_checkin(
            &conn,
            &CheckinInput {
                colony_id: c,
                date: " 2026-09-15 ".into(),
                queen_count: Some(2),
                worker_count: Some(3000),
                moved_nest: true,
                note: Some("  搬进新塔  ".into()),
            },
            TODAY,
            NOW,
        )
        .unwrap();

        assert_eq!(saved.colony_id, c);
        assert_eq!(saved.date, "2026-09-15", "日期 trim 后落库");
        assert_eq!(saved.queen_count, Some(2));
        assert_eq!(saved.worker_count, Some(3000));
        assert!(saved.moved_nest);
        assert_eq!(saved.note, "搬进新塔", "备注 trim");
        assert_eq!(saved.created_at, NOW, "录入时间与登记日期分开存");
        assert!(saved.photos.is_empty(), "本票无照片字段，恒空数组");

        assert_eq!(count(&conn, "SELECT COUNT(*) FROM nest_checkin"), 1);
    }

    #[test]
    fn save_checkin_rejects_all_empty() {
        // 验收：全空登记被拒（数/换巢/备注全空），不落库
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");

        let err = save_checkin(&conn, &empty_input(c, "2026-09-18"), TODAY, NOW).unwrap_err();
        assert!(err.contains("至少填一项"), "实际错误：{err}");
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM nest_checkin"), 0, "被拒的登记不落库");
    }

    #[test]
    fn save_checkin_accepts_any_single_field() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");

        // 只填备注
        let mut note_only = empty_input(c, "2026-09-18");
        note_only.note = Some("状态不错".into());
        assert!(save_checkin(&conn, &note_only, TODAY, NOW).is_ok());

        // 只填蚁后数
        let mut queen_only = empty_input(c, "2026-09-17");
        queen_only.queen_count = Some(1);
        assert!(save_checkin(&conn, &queen_only, TODAY, NOW).is_ok());

        // 只勾换巢
        let mut moved_only = empty_input(c, "2026-09-16");
        moved_only.moved_nest = true;
        assert!(save_checkin(&conn, &moved_only, TODAY, NOW).is_ok());

        // 只填工蚁数
        let mut worker_only = empty_input(c, "2026-09-15");
        worker_only.worker_count = Some(500);
        assert!(save_checkin(&conn, &worker_only, TODAY, NOW).is_ok());

        assert_eq!(count(&conn, "SELECT COUNT(*) FROM nest_checkin"), 4);
    }

    #[test]
    fn save_checkin_rejects_future_date_and_allows_backdate() {
        // 验收：日期补录（早于今天）合法；未来拒绝（沿用 care.rs 口径）
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");

        let err = save_checkin(&conn, &empty_input(c, "2026-09-19"), TODAY, NOW)
            .map(|_| ())
            .unwrap_err();
        assert!(err.contains("未来"), "实际错误：{err}");

        let mut past = empty_input(c, "2026-09-01");
        past.note = Some("补录月初".into());
        assert!(save_checkin(&conn, &past, TODAY, NOW).is_ok(), "补录过去合法");

        // 当天（今天）合法
        let mut today_itself = empty_input(c, "2026-09-18");
        today_itself.moved_nest = true;
        assert!(save_checkin(&conn, &today_itself, TODAY, NOW).is_ok());

        // 未来那笔没落库
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM nest_checkin"), 2);
    }

    #[test]
    fn save_checkin_rejects_bad_date_negative_counts_and_unknown_colony() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");

        assert!(save_checkin(&conn, &empty_input(c, "09/18/2026"), TODAY, NOW)
            .unwrap_err()
            .contains("YYYY-MM-DD"));

        let mut neg_queen = empty_input(c, "2026-09-18");
        neg_queen.queen_count = Some(-1);
        assert!(save_checkin(&conn, &neg_queen, TODAY, NOW)
            .unwrap_err()
            .contains("蚁后数"));

        let mut neg_worker = empty_input(c, "2026-09-18");
        neg_worker.worker_count = Some(-5);
        assert!(save_checkin(&conn, &neg_worker, TODAY, NOW)
            .unwrap_err()
            .contains("工蚁数"));

        assert!(save_checkin(&conn, &empty_input(999, "2026-09-18"), TODAY, NOW)
            .unwrap_err()
            .contains("窝不存在"));
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM nest_checkin"), 0);
    }

    // ── 时间线 ──

    #[test]
    fn list_checkins_orders_date_desc_then_id_desc() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        let mk = |date: &str, note: &str| CheckinInput {
            colony_id: c,
            date: date.into(),
            queen_count: None,
            worker_count: None,
            moved_nest: false,
            note: Some(note.into()),
        };
        let first = save_checkin(&conn, &mk("2026-09-10", "甲"), TODAY, NOW).unwrap();
        let second = save_checkin(&conn, &mk("2026-09-15", "乙"), TODAY, NOW).unwrap();
        let third = save_checkin(&conn, &mk("2026-09-10", "丙"), TODAY, NOW).unwrap();

        let rows = list_checkins(&conn, c).unwrap();
        let ids: Vec<i64> = rows.iter().map(|r| r.id).collect();
        assert_eq!(ids, vec![second.id, third.id, first.id], "日期倒序，同日按录入 id 倒序");
        assert_eq!(rows[0].note, "乙");
        assert_eq!(rows[1].note, "丙");
        assert_eq!(rows[2].note, "甲");
    }

    #[test]
    fn list_checkins_rejects_unknown_colony() {
        let conn = mem_conn();
        assert!(list_checkins(&conn, 999).unwrap_err().contains("窝不存在"));
    }

    // ── 编辑 ──

    #[test]
    fn update_checkin_overwrites_fields_and_can_clear_counts() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        let mut input = empty_input(c, "2026-09-15");
        input.queen_count = Some(3);
        input.moved_nest = true;
        let saved = save_checkin(&conn, &input, TODAY, NOW).unwrap();

        // 全量覆盖：蚁后清回「未数」、工蚁填上、换巢取消、改日期改备注
        let updated = update_checkin(
            &conn,
            saved.id,
            &CheckinUpdateInput {
                date: "2026-09-17".into(),
                queen_count: None,
                worker_count: Some(800),
                moved_nest: false,
                note: Some("  复盘  ".into()),
            },
            TODAY,
        )
        .unwrap();

        assert_eq!(updated.date, "2026-09-17");
        assert_eq!(updated.queen_count, None, "数可清回未数");
        assert_eq!(updated.worker_count, Some(800));
        assert!(!updated.moved_nest);
        assert_eq!(updated.note, "复盘");
        assert_eq!(updated.created_at, NOW, "录入时间保持首次录入");
    }

    #[test]
    fn update_checkin_rejects_resulting_all_empty_future_date_and_missing_row() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        let mut input = empty_input(c, "2026-09-15");
        input.queen_count = Some(3);
        let saved = save_checkin(&conn, &input, TODAY, NOW).unwrap();

        // 编辑后全空（蚁后清掉、其余不填）→ 拒，原行不动
        let err = update_checkin(
            &conn,
            saved.id,
            &CheckinUpdateInput {
                date: "2026-09-15".into(),
                queen_count: None,
                worker_count: None,
                moved_nest: false,
                note: None,
            },
            TODAY,
        )
        .unwrap_err();
        assert!(err.contains("至少填一项"), "实际错误：{err}");

        // 编辑成未来日期 → 拒
        let err = update_checkin(
            &conn,
            saved.id,
            &CheckinUpdateInput {
                date: "2026-09-19".into(),
                queen_count: Some(1),
                worker_count: None,
                moved_nest: false,
                note: None,
            },
            TODAY,
        )
        .unwrap_err();
        assert!(err.contains("未来"), "实际错误：{err}");

        // 不存在的登记
        let err = update_checkin(
            &conn,
            999,
            &CheckinUpdateInput {
                date: "2026-09-15".into(),
                queen_count: Some(1),
                worker_count: None,
                moved_nest: false,
                note: None,
            },
            TODAY,
        )
        .unwrap_err();
        assert!(err.contains("不存在"), "实际错误：{err}");

        // 一连串失败后原行原样
        let (date, queen): (String, Option<i64>) = conn
            .query_row(
                "SELECT date, queen_count FROM nest_checkin WHERE id = ?1",
                params![saved.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(date, "2026-09-15");
        assert_eq!(queen, Some(3));
    }

    // ── 删除与基线顺延 ──

    #[test]
    fn delete_checkin_removes_row_and_second_delete_errors() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        let mut input = empty_input(c, "2026-09-15");
        input.note = Some("待删".into());
        let saved = save_checkin(&conn, &input, TODAY, NOW).unwrap();

        delete_checkin(&conn, saved.id).unwrap();
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM nest_checkin"), 0);

        let err = delete_checkin(&conn, saved.id).unwrap_err();
        assert!(err.contains("不存在"), "实际错误：{err}");
    }

    #[test]
    fn baseline_is_earliest_date_edits_follow_and_delete_hands_over() {
        // 验收（基线语义）：基线 = 最早登记日期；首条日期可改；删首条后下一条最早者接任
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        let mk = |date: &str| CheckinInput {
            colony_id: c,
            date: date.into(),
            queen_count: Some(1),
            worker_count: None,
            moved_nest: false,
            note: None,
        };
        let a = save_checkin(&conn, &mk("2026-09-10"), TODAY, NOW).unwrap();
        let _b = save_checkin(&conn, &mk("2026-09-15"), TODAY, NOW).unwrap();
        let _c3 = save_checkin(&conn, &mk("2026-09-17"), TODAY, NOW).unwrap();

        let digest = digest_for_colony(&conn, c, TODAY).unwrap();
        assert_eq!(digest.baseline_date, Some("2026-09-10".into()));

        // 首条（09-10）日期改到 09-16 → 基线顺延到 09-15
        update_checkin(
            &conn,
            a.id,
            &CheckinUpdateInput {
                date: "2026-09-16".into(),
                queen_count: Some(1),
                worker_count: None,
                moved_nest: false,
                note: None,
            },
            TODAY,
        )
        .unwrap();
        let digest = digest_for_colony(&conn, c, TODAY).unwrap();
        assert_eq!(digest.baseline_date, Some("2026-09-15".into()), "改首条日期后基线跟走");

        // 删掉当前的基线条目（09-15）→ 基线落到余下最早的 09-16（即改期后的原首条）
        let rows = list_checkins(&conn, c).unwrap();
        let baseline_row = rows.iter().find(|r| r.date == "2026-09-15").unwrap();
        delete_checkin(&conn, baseline_row.id).unwrap();
        let digest = digest_for_colony(&conn, c, TODAY).unwrap();
        assert_eq!(digest.baseline_date, Some("2026-09-16".into()), "删首条后下一条最早者接任");
    }

    // ── 摘要投影 ──

    #[test]
    fn digest_reports_latest_counts_days_and_baseline() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        let mut old = empty_input(c, "2026-09-10");
        old.worker_count = Some(100);
        save_checkin(&conn, &old, TODAY, NOW).unwrap();
        let mut new = empty_input(c, "2026-09-15");
        new.queen_count = Some(2);
        save_checkin(&conn, &new, TODAY, NOW).unwrap();

        let digest = digest_for_colony(&conn, c, TODAY).unwrap();
        let latest = digest.latest.expect("有登记必有 latest");
        assert_eq!(latest.date, "2026-09-15", "latest 取最新一组数");
        assert_eq!(latest.queen_count, Some(2));
        assert_eq!(digest.days_since_last, Some(3), "今天(09-18) − 09-15");
        assert_eq!(digest.baseline_date, Some("2026-09-10".into()));
    }

    #[test]
    fn digest_is_all_none_for_never_registered_colony() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        let digest = digest_for_colony(&conn, c, TODAY).unwrap();
        assert!(digest.latest.is_none());
        assert_eq!(digest.baseline_date, None);
        assert_eq!(digest.days_since_last, None);
    }

    #[test]
    fn deleting_colony_cleans_up_its_checkins() {
        // spec 用户故事 11：删窝时该窝全部巢况一并清理（照片随票 07 一并删文件）
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        let mut input = empty_input(c, "2026-09-15");
        input.note = Some("随窝走".into());
        save_checkin(&conn, &input, TODAY, NOW).unwrap();

        crate::colony::delete_colony(&conn, c).unwrap();
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM nest_checkin"), 0, "删窝连带清巢况");
    }
}
