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
//! - 照片元数据（nest_photo）：写入走 photo::attach_photos（票 07），本模块管
//!   读取与裁剪更新（update_photo_crop，窝头像票 01）；无照片即空数组；
//! - 头像投影（窝头像票 01）：avatar_photo_for_colony 按排序契约取「第一条含
//!   照片的登记的第一张照片」，纯查询不落库；photo_wall 照片墙只读载荷同契约；
//! - 巢况永不参与提醒/催促（reminder.rs 不引用本模块，不进维护操作清单）。

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

// ── DTO ──────────────────────────────────────────────────────────────────

/// 头像裁剪（窝头像票 01）：归一化方形区域——`x`/`y` = 左上角坐标、`size` = 边长，
/// 各 ∈ [0,1] 且 x+size ≤ 1、y+size ≤ 1（校验权威 [`validate_crop`]）。
/// None（库内三列 NULL）= 默认居中（未调过/存量行）；圆形显示只是方形区域挖角，
/// 裁剪数据与显示形状无关。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PhotoCrop {
    pub x: f64,
    pub y: f64,
    pub size: f64,
}

/// 照片元数据一行（写入走 photo::attach_photos；裁剪列 NULL = 居中）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NestPhotoMeta {
    pub id: i64,
    pub checkin_id: i64,
    /// photos/ 下相对路径 `<colonyId>/<uuid>.jpg`（落盘文件名一律服务端 UUID）。
    pub rel_path: String,
    /// 客户端原始文件名，仅备注、不参与路径。
    pub original_name: Option<String>,
    pub note: String,
    /// 头像裁剪（归一化区域）；None = 默认居中。
    pub crop: Option<PhotoCrop>,
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

/// 照片元数据（按 id 序）。
fn load_photos(conn: &Connection, checkin_id: i64) -> Result<Vec<NestPhotoMeta>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, checkin_id, rel_path, original_name, note, crop_x, crop_y, crop_size
             FROM nest_photo WHERE checkin_id = ?1 ORDER BY id",
        )
        .map_err(db_err)?;
    let rows = stmt
        .query_map(params![checkin_id], row_to_photo_meta)
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?;
    Ok(rows)
}

/// 一行 nest_photo → 照片元数据（读取链路共用：时间线 / 上传回读 / 头像 / 照片墙）。
fn row_to_photo_meta(row: &rusqlite::Row<'_>) -> rusqlite::Result<NestPhotoMeta> {
    Ok(NestPhotoMeta {
        id: row.get(0)?,
        checkin_id: row.get(1)?,
        rel_path: row.get(2)?,
        original_name: row.get(3)?,
        note: row.get(4)?,
        crop: crop_from_cols(row.get(5)?, row.get(6)?, row.get(7)?),
    })
}

/// 裁剪三列 → Option：写入侧（update_photo_crop）三列同进同出；残缺行按
/// 「未调过」的居中语义处理（None），不做半截裁剪。
fn crop_from_cols(x: Option<f64>, y: Option<f64>, size: Option<f64>) -> Option<PhotoCrop> {
    match (x, y, size) {
        (Some(x), Some(y), Some(size)) => Some(PhotoCrop { x, y, size }),
        _ => None,
    }
}

/// 单张照片元数据（按 id；update_photo_crop 成功后的回读）。
fn get_photo_meta(conn: &Connection, photo_id: i64) -> Result<NestPhotoMeta, String> {
    conn.query_row(
        "SELECT id, checkin_id, rel_path, original_name, note, crop_x, crop_y, crop_size
         FROM nest_photo WHERE id = ?1",
        params![photo_id],
        row_to_photo_meta,
    )
    .map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => "照片不存在".to_string(),
        other => db_err(other),
    })
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

// ── 裁剪更新 / 头像投影 / 照片墙（窝头像票 01）──────────────────────────────
// 排序契约（spec Implementation Decisions，全链唯一依据，与既有实现一致）：
// 登记全序 = 登记日期倒序、同日期按登记创建序号倒序（后建的在前）；
// 照片列表全序 = 按照片序号正序（先传的在前）。头像与照片墙必须复用同一契约。

/// 裁剪校验（权威在纯核；桌面 IPC 与网页端镜像同口径调用）：
/// x/y/边长各 ∈ [0,1]（NaN/∞ 落在范围判定外自然被拒），且 x+边长 ≤ 1、y+边长 ≤ 1。
pub fn validate_crop(crop: &PhotoCrop) -> Result<(), String> {
    for (label, v) in [("x", crop.x), ("y", crop.y), ("边长", crop.size)] {
        if !(0.0..=1.0).contains(&v) {
            return Err(format!("裁剪{label}应在 0–1 之间（收到 {v}）"));
        }
    }
    if crop.x + crop.size > 1.0 {
        return Err(format!(
            "裁剪区域超出照片（x+边长 = {}，最大 1）",
            crop.x + crop.size
        ));
    }
    if crop.y + crop.size > 1.0 {
        return Err(format!(
            "裁剪区域超出照片（y+边长 = {}，最大 1）",
            crop.y + crop.size
        ));
    }
    Ok(())
}

/// 更新照片裁剪：`Some(合法区域)` 落库；`None` = 重置居中（三列置 NULL）。
/// 校验失败或照片不存在时不落库；成功回读返回完整照片元数据（裁剪随之可读回）。
pub fn update_photo_crop(
    conn: &Connection,
    photo_id: i64,
    crop: Option<PhotoCrop>,
) -> Result<NestPhotoMeta, String> {
    if let Some(c) = crop.as_ref() {
        validate_crop(c)?;
    }
    let (cx, cy, cs) = match crop {
        Some(c) => (Some(c.x), Some(c.y), Some(c.size)),
        None => (None, None, None),
    };
    let changed = conn
        .execute(
            "UPDATE nest_photo SET crop_x = ?1, crop_y = ?2, crop_size = ?3 WHERE id = ?4",
            params![cx, cy, cs, photo_id],
        )
        .map_err(db_err)?;
    if changed == 0 {
        return Err("照片不存在".into());
    }
    get_photo_meta(conn, photo_id)
}

/// 头像照片引用（纯投影，不落库）：按排序契约取**第一条含照片的登记**的第一张
/// 照片——纯文字登记无照片行自然跳过（spec F6）；该窝从无照片则 None（前端显示
/// 占位）。删登记后投影重算自然回退到更早照片，裁剪随照片行原样生效。
pub fn avatar_photo_for_colony(
    conn: &Connection,
    colony_id: i64,
) -> Result<Option<NestPhotoMeta>, String> {
    conn.query_row(
        "SELECT np.id, np.checkin_id, np.rel_path, np.original_name, np.note,
                np.crop_x, np.crop_y, np.crop_size
         FROM nest_photo np
         JOIN nest_checkin nc ON np.checkin_id = nc.id
         WHERE nc.colony_id = ?1
         ORDER BY nc.date DESC, nc.id DESC, np.id ASC
         LIMIT 1",
        params![colony_id],
        row_to_photo_meta,
    )
    .optional()
    .map_err(db_err)
}

/// 照片墙一窝载荷：窝 id/名 + 按登记日期倒序的日期分组（无照片的窝不占分组）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PhotoWallColony {
    pub colony_id: i64,
    pub colony_name: String,
    pub groups: Vec<PhotoWallDayGroup>,
}

/// 照片墙一个日期分组：该日期的全部登记（按登记全序，创建序号倒序）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PhotoWallDayGroup {
    pub date: String,
    pub checkins: Vec<PhotoWallCheckin>,
}

/// 分组内一条登记：登记 id + 该登记的照片（按上传序号正序）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PhotoWallCheckin {
    pub checkin_id: i64,
    pub photos: Vec<NestPhotoMeta>,
}

/// 照片墙只读载荷：全部窝的照片——窝按 sort/id（与首页窝列表同序）、窝内按
/// 登记日期倒序、同日期按登记全序、登记内照片按上传序（排序契约全链同源）。
/// 照片带完整元数据（含裁剪）；只读，无任何写路径。
pub fn photo_wall(conn: &Connection) -> Result<Vec<PhotoWallColony>, String> {
    let colony_ids: Vec<(i64, String)> = {
        let mut stmt = conn
            .prepare("SELECT id, name FROM colony ORDER BY sort, id")
            .map_err(db_err)?;
        let rows = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(db_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db_err)?;
        rows
    };
    let mut out = Vec::new();
    for (colony_id, colony_name) in colony_ids {
        // 行序即分组序：日期倒序、同日期登记全序、登记内照片序号正序——
        // 相邻同日期/同登记的行在 Rust 侧折叠成分组与登记条目。
        // 列序：照片元数据占 0..7（与 row_to_photo_meta 对齐），date 在末位。
        let rows: Vec<(String, NestPhotoMeta)> = {
            let mut stmt = conn
                .prepare(
                    "SELECT np.id, np.checkin_id, np.rel_path, np.original_name, np.note,
                            np.crop_x, np.crop_y, np.crop_size, nc.date
                     FROM nest_checkin nc
                     JOIN nest_photo np ON np.checkin_id = nc.id
                     WHERE nc.colony_id = ?1
                     ORDER BY nc.date DESC, nc.id DESC, np.id ASC",
                )
                .map_err(db_err)?;
            let rows = stmt
                .query_map(params![colony_id], |row| {
                    Ok((row.get::<_, String>(8)?, row_to_photo_meta(row)?))
                })
                .map_err(db_err)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(db_err)?;
            rows
        };
        if rows.is_empty() {
            continue; // 无照片的窝不进照片墙
        }
        let mut groups: Vec<PhotoWallDayGroup> = Vec::new();
        for (date, meta) in rows {
            let group = match groups.last_mut() {
                Some(g) if g.date == date => g,
                _ => {
                    groups.push(PhotoWallDayGroup {
                        date: date.clone(),
                        checkins: Vec::new(),
                    });
                    groups.last_mut().expect("刚 push 必有")
                }
            };
            match group.checkins.last_mut() {
                Some(c) if c.checkin_id == meta.checkin_id => c.photos.push(meta),
                _ => group.checkins.push(PhotoWallCheckin {
                    checkin_id: meta.checkin_id,
                    photos: vec![meta],
                }),
            }
        }
        out.push(PhotoWallColony {
            colony_id,
            colony_name,
            groups,
        });
    }
    Ok(out)
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

    // ── 裁剪更新 / 头像投影 / 照片墙（窝头像票 01）───────────────────────────

    /// 直插一行照片元数据（读取/投影测试夹具；写入管线本身在 photo.rs 有专测）。
    fn photo(conn: &Connection, checkin_id: i64, rel: &str) -> i64 {
        conn.execute(
            "INSERT INTO nest_photo (checkin_id, rel_path, original_name, note)
             VALUES (?1, ?2, NULL, '')",
            params![checkin_id, rel],
        )
        .expect("插照片元数据失败");
        conn.last_insert_rowid()
    }

    /// 一条只有备注的合法登记（照片挂靠点），返回登记 id。
    fn checkin_on(conn: &Connection, colony_id: i64, date: &str, note: &str) -> i64 {
        save_checkin(
            conn,
            &CheckinInput {
                colony_id,
                date: date.into(),
                queen_count: None,
                worker_count: None,
                moved_nest: false,
                note: Some(note.into()),
            },
            TODAY,
            NOW,
        )
        .expect("建登记失败")
        .id
    }

    #[test]
    fn update_photo_crop_persists_legal_and_resets_to_center() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        let ck = checkin_on(&conn, c, "2026-09-18", "带照片");
        let p = photo(&conn, ck, "1/a.jpg");

        // 合法区域落库并可读回
        let updated =
            update_photo_crop(&conn, p, Some(PhotoCrop { x: 0.25, y: 0.5, size: 0.5 })).unwrap();
        assert_eq!(updated.crop, Some(PhotoCrop { x: 0.25, y: 0.5, size: 0.5 }));
        assert_eq!(updated.id, p);
        assert_eq!(updated.rel_path, "1/a.jpg");

        // 元数据贯通：时间线/摘要读到的同一份照片也带裁剪
        let row = get_checkin(&conn, ck).unwrap();
        assert_eq!(row.photos[0].crop, Some(PhotoCrop { x: 0.25, y: 0.5, size: 0.5 }));

        // 边界：整幅（0,0,1）合法
        update_photo_crop(&conn, p, Some(PhotoCrop { x: 0.0, y: 0.0, size: 1.0 })).unwrap();

        // None 重置 = 居中语义（三列回 NULL），幂等
        let reset = update_photo_crop(&conn, p, None).unwrap();
        assert_eq!(reset.crop, None, "重置后读回 None（居中）");
        update_photo_crop(&conn, p, None).unwrap();
    }

    #[test]
    fn update_photo_crop_rejects_out_of_range_and_missing_photo() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        let ck = checkin_on(&conn, c, "2026-09-18", "带照片");
        let p = photo(&conn, ck, "1/a.jpg");

        // 越界/非法：单值出 [0,1]、x+边长>1、y+边长>1
        for bad in [
            PhotoCrop { x: -0.1, y: 0.0, size: 0.5 },
            PhotoCrop { x: 0.0, y: 1.5, size: 0.5 },
            PhotoCrop { x: 0.6, y: 0.0, size: 0.5 }, // x+边长 = 1.1
            PhotoCrop { x: 0.0, y: 0.7, size: 0.4 }, // y+边长 = 1.1
            PhotoCrop { x: 0.0, y: 0.0, size: 1.2 },
        ] {
            let err = update_photo_crop(&conn, p, Some(bad)).unwrap_err();
            assert!(err.contains("裁剪"), "crop={bad:?} 实际错误：{err}");
        }
        // 越界值一个都没落库
        let stored: Option<f64> = conn
            .query_row(
                "SELECT crop_x FROM nest_photo WHERE id = ?1",
                params![p],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stored, None);

        // 幽灵照片：设与重置都报不存在（不静默成功）
        let err = update_photo_crop(&conn, 999, Some(PhotoCrop { x: 0.1, y: 0.1, size: 0.5 }))
            .unwrap_err();
        assert!(err.contains("照片不存在"), "实际错误：{err}");
        assert!(update_photo_crop(&conn, 999, None)
            .unwrap_err()
            .contains("照片不存在"));
    }

    #[test]
    fn avatar_takes_first_photo_of_latest_checkin_with_photos() {
        // 排序契约：最新登记的第一张（序号最小）照片
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        let older = checkin_on(&conn, c, "2026-09-10", "早");
        let latest = checkin_on(&conn, c, "2026-09-15", "晚");
        let _older_photo = photo(&conn, older, "1/old.jpg");
        let p1 = photo(&conn, latest, "1/new-1.jpg");
        let _p2 = photo(&conn, latest, "1/new-2.jpg");

        let avatar = avatar_photo_for_colony(&conn, c)
            .unwrap()
            .expect("最新登记有照片，应有头像");
        assert_eq!(avatar.id, p1, "最新登记的照片里取序号最小者");
        assert_eq!(avatar.rel_path, "1/new-1.jpg");
    }

    #[test]
    fn avatar_skips_text_only_latest_checkin() {
        // 最新登记是纯文字（无照片）→ 跳过取更早含照片登记（spec F6）
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        let older = checkin_on(&conn, c, "2026-09-10", "有照片");
        let older_photo = photo(&conn, older, "1/old.jpg");
        checkin_on(&conn, c, "2026-09-15", "纯文字");

        let avatar = avatar_photo_for_colony(&conn, c)
            .unwrap()
            .expect("跳过纯文字登记后应回退到更早照片");
        assert_eq!(avatar.id, older_photo);
    }

    #[test]
    fn avatar_same_date_takes_latest_created_checkin() {
        // 同日期多条登记 → 登记全序取创建序号最大者（后建的在前）
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        let first = checkin_on(&conn, c, "2026-09-15", "先建");
        let second = checkin_on(&conn, c, "2026-09-15", "后建");
        let _fp = photo(&conn, first, "1/first.jpg");
        let sp = photo(&conn, second, "1/second.jpg");

        let avatar = avatar_photo_for_colony(&conn, c).unwrap().unwrap();
        assert_eq!(avatar.id, sp, "同日期取创建序号最大者的照片");
    }

    #[test]
    fn avatar_falls_back_after_delete_and_is_none_without_photos() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        let older = checkin_on(&conn, c, "2026-09-10", "早");
        let older_photo = photo(&conn, older, "1/old.jpg");
        let latest = checkin_on(&conn, c, "2026-09-15", "晚");
        let lp = photo(&conn, latest, "1/new.jpg");

        assert_eq!(avatar_photo_for_colony(&conn, c).unwrap().unwrap().id, lp);

        // 更早照片先调过裁剪 → 删最新登记后投影回退，裁剪仍随照片生效
        update_photo_crop(&conn, older_photo, Some(PhotoCrop { x: 0.1, y: 0.1, size: 0.4 }))
            .unwrap();
        delete_checkin(&conn, latest).unwrap();
        let avatar = avatar_photo_for_colony(&conn, c).unwrap().unwrap();
        assert_eq!(avatar.id, older_photo, "删登记后投影重算自然回退");
        assert_eq!(
            avatar.crop,
            Some(PhotoCrop { x: 0.1, y: 0.1, size: 0.4 }),
            "回退照片调过的裁剪仍生效"
        );

        // 无照片窝 → None；从未登记/幽灵窝同样 None（与 digest 的空语义同口径）
        let bare = colony(&conn, "无照窝");
        checkin_on(&conn, bare, "2026-09-15", "纯文字");
        assert!(avatar_photo_for_colony(&conn, bare).unwrap().is_none());
        assert!(avatar_photo_for_colony(&conn, 999).unwrap().is_none());
    }

    #[test]
    fn photo_wall_groups_by_colony_date_and_checkin_order() {
        // 排序契约验收：跨窝（按 sort/id）、窝内日期倒序、同日期多登记按登记全序、
        // 登记内多照片按上传序；载荷带窝名与日期；照片带裁剪。
        let conn = mem_conn();
        let a = colony(&conn, "大头一号");
        let b = colony(&conn, "针毛一号");
        let bare = colony(&conn, "无照窝"); // 无照片窝不进照片墙
        checkin_on(&conn, bare, "2026-09-18", "纯文字");

        // 窝 A：09-10 一条两照片（上传序）、09-15 同日两条登记（登记全序）
        let a_old = checkin_on(&conn, a, "2026-09-10", "早");
        let a_1st = checkin_on(&conn, a, "2026-09-15", "同日先建");
        let a_2nd = checkin_on(&conn, a, "2026-09-15", "同日后建");
        let a_old_p1 = photo(&conn, a_old, "1/old-1.jpg");
        let _a_old_p2 = photo(&conn, a_old, "1/old-2.jpg");
        let _a_1st_p = photo(&conn, a_1st, "1/same-1.jpg");
        let _a_2nd_p1 = photo(&conn, a_2nd, "1/same-2a.jpg");
        let _a_2nd_p2 = photo(&conn, a_2nd, "1/same-2b.jpg");

        // 窝 B：日期更晚（验证窝间按窝序而非日期；不得晚于 TODAY）
        let b_ck = checkin_on(&conn, b, "2026-09-17", "B 窝");
        let _b_p = photo(&conn, b_ck, "2/b.jpg");

        update_photo_crop(&conn, a_old_p1, Some(PhotoCrop { x: 0.2, y: 0.2, size: 0.6 })).unwrap();

        let wall = photo_wall(&conn).unwrap();
        assert_eq!(wall.len(), 2, "无照片窝不占分组，共两窝");
        assert_eq!(wall[0].colony_id, a, "窝按 sort/id 序（先建的在前）");
        assert_eq!(wall[0].colony_name, "大头一号");
        assert_eq!(wall[1].colony_id, b);
        assert_eq!(wall[1].colony_name, "针毛一号");

        let a_groups = &wall[0].groups;
        let dates: Vec<&str> = a_groups.iter().map(|g| g.date.as_str()).collect();
        assert_eq!(dates, vec!["2026-09-15", "2026-09-10"], "窝内按登记日期倒序");

        // 同日两条登记按登记全序（创建序号倒序：后建在前）
        let same_day = &a_groups[0];
        let ck_ids: Vec<i64> = same_day.checkins.iter().map(|c| c.checkin_id).collect();
        assert_eq!(ck_ids, vec![a_2nd, a_1st]);
        // 登记内照片按上传序（序号正序）
        let second_paths: Vec<&str> = same_day.checkins[0]
            .photos
            .iter()
            .map(|p| p.rel_path.as_str())
            .collect();
        assert_eq!(second_paths, vec!["1/same-2a.jpg", "1/same-2b.jpg"]);

        // 早日期分组：单登记两照片按上传序，首张带刚调过的裁剪
        let old_group = &a_groups[1];
        assert_eq!(old_group.checkins.len(), 1);
        let old_paths: Vec<&str> = old_group.checkins[0]
            .photos
            .iter()
            .map(|p| p.rel_path.as_str())
            .collect();
        assert_eq!(old_paths, vec!["1/old-1.jpg", "1/old-2.jpg"]);
        assert_eq!(
            old_group.checkins[0].photos[0].crop,
            Some(PhotoCrop { x: 0.2, y: 0.2, size: 0.6 }),
            "照片墙照片带裁剪元数据"
        );

        // 窝 B：单组单登记
        assert_eq!(wall[1].groups.len(), 1);
        assert_eq!(wall[1].groups[0].date, "2026-09-17");
        assert_eq!(wall[1].groups[0].checkins[0].checkin_id, b_ck);
    }
}
