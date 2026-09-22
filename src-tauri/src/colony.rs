//! 窝与地点的业务逻辑（票 02）。
//!
//! 全部函数只吃 `&Connection`，与 Tauri 解耦，可被 cargo test 直接覆盖；
//! Tauri command 只是薄包装（见 lib.rs）。
//!
//! 行为对齐 spec：
//! - 窝名 trim 后全局唯一（schema UNIQUE 兜底 + 应用层友好报错）；
//! - 饲养天数 = 今天 − 开始饲养日期的自然日天数（含冬眠，评审附录规则 9）；
//! - deleteColony 仅无记录窝（无 care_log 行）；有记录只能置「已结束」（规则 10 精神）。

use std::collections::HashMap;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

/// 窝状态三选一（schema CHECK 同款）。
pub const STATUSES: [&str; 3] = ["active", "hibernating", "ended"];

// ── DTO ──────────────────────────────────────────────────────────────────

/// 窝（含首页展示数据：饲养天数、每个启用操作的距上次/超期态、最近记录摘要，Rust 算好）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Colony {
    pub id: i64,
    pub name: String,
    pub species: Option<String>,
    pub location_id: Option<i64>,
    pub start_date: String,
    pub status: String,
    /// 保湿方式（保湿方式票 01，spec D1）：'manual'=手动加水 | 'tower'=水塔 |
    /// None=未设。纯标签属性，不参与提醒判定（周期语义原样走每窝周期）。
    pub hydration_method: Option<String>,
    pub days_raised: i64,
    /// 每个启用中操作一块（care::ActionTile，按字典顺序）。
    pub actions: Vec<crate::care::ActionTile>,
    /// 最近 1-2 条记录摘要（care::RecentLog，发生时间倒序）。
    pub recent: Vec<crate::care::RecentLog>,
    /// 进行中的冬眠段摘要（冬眠卡片横幅数据：入眠日/预计出眠）；无开放段为 None。
    pub hibernation: Option<crate::hibernation::OpenSegment>,
    /// 巢况摘要（webui-checkin 票 02）：最新一组数 + 基线 + 距上次登记天数，
    /// Rust 算好；从未登记 latest/baseline_date/days_since_last 皆 None。
    pub checkin: crate::nest_checkin::CheckinDigest,
    /// 头像照片引用（窝头像票 01）：按排序契约派生的「最新一张巢况照片」
    /// （纯投影不落库，删登记自动回退，前端不重复推导）；无照片窝 None
    /// （显示占位）。
    pub avatar: Option<crate::nest_checkin::NestPhotoMeta>,
}

/// 新建/编辑窝的入参。
///
/// 保湿方式写入（保湿方式票 01，spec F2/F6 原子性）与基础字段同命令同事务：
/// - `hydration_method`：'manual' | 'tower' | None=未设。前端整窗提交（下拉框
///   恒有值），后端按「库内原值 vs 提交值」比较判定落库语义（不收前端旗标）；
/// - `interval_changes`：本次保存要增删的每窝周期行（只提交变化的行，未列出的
///   行零改动）。历史 create-then-set 两步 IPC（set_colony_action_interval）
///   不再用于窝表单的保湿写入，但该命令本身原样保留（其余读写维持现状）。
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ColonyInput {
    pub name: String,
    pub species: Option<String>,
    pub location_id: Option<i64>,
    pub start_date: String,
    pub status: String,
    #[serde(default)]
    pub hydration_method: Option<String>,
    #[serde(default)]
    pub interval_changes: Vec<ColonyIntervalChange>,
}

/// 整窗提交里的一行每窝周期增删：`interval_days = Some(1..=365)` 设/改一行，
/// `None` 删行（未设）。删行是否被净零语义拦截由后端按库内原值判定（F4）。
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ColonyIntervalChange {
    pub action_id: i64,
    pub interval_days: Option<i64>,
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
        "SELECT id, name, species, location_id, start_date, status, hydration_method
         FROM colony WHERE id = ?1",
        params![id],
        |row| {
            Ok(Colony {
                id: row.get(0)?,
                name: row.get(1)?,
                species: row.get(2)?,
                location_id: row.get(3)?,
                start_date: row.get(4)?,
                status: row.get(5)?,
                hydration_method: row.get(6)?,
                days_raised: 0,
                actions: Vec::new(),
                recent: Vec::new(),
                hibernation: None,
                checkin: crate::nest_checkin::CheckinDigest {
                    latest: None,
                    baseline_date: None,
                    days_since_last: None,
                },
                avatar: None,
            })
        },
    )
    .map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => "窝不存在".to_string(),
        other => db_err(other),
    })
    .and_then(|mut c| {
        c.days_raised = days_raised(&c.start_date, today)?;
        c.actions = crate::care::tiles_for_colony(conn, c.id, today)?;
        c.recent = crate::care::recent_for_colony(conn, c.id, 2)?;
        c.hibernation = crate::hibernation::open_segment(conn, c.id)?;
        c.checkin = crate::nest_checkin::digest_for_colony(conn, c.id, today)?;
        c.avatar = crate::nest_checkin::avatar_photo_for_colony(conn, c.id)?;
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

// ── 保湿方式写入（保湿方式票 01）─────────────────────────────────────────
// 状态矩阵落库语义（spec F1/F4，判定提示钉死）：方式是否被触碰**不由前端传
// 旗标**，后端按「库内原值 vs 提交值」比较——
// - 库内∈{manual,tower} 且 提交=未设 → 清回未设：方式置 NULL 并删该窝保湿
//   周期行（唯一由方式动作触发删周期的路径，与基础字段同事务）；
// - 库内=未设 且 提交=未设 → 净零：会话内「选了又改回」净效果为零，此时
//   丢弃提交里针对巢穴保湿行的清除，库内已有周期行原样保留（B 仍 B）；
// - 其余组合方式照抄提交值、不经由此路径删行（只清天数的 C→D 走
//   interval_changes 的删行通道，库内原值非未设时照常生效）。

/// 保湿方式合法域（spec：'manual'=手动加水 | 'tower'=水塔 | NULL=未设）。
const HYDRATION_METHODS: [&str; 2] = ["manual", "tower"];

/// 巢穴保湿操作的锚点名（预置名）。「清回未设删周期行」与净零守卫都要定位
/// 该窝的保湿周期行。改名后匹配不到为已知边界（同 v4/v6/v8 按名回填先例：
/// 单人工具可接受，此时清回未设不删行、方式照常切换）。
const HYDRATION_ACTION_NAME: &str = "巢穴保湿";

fn validate_hydration_method(method: Option<&str>) -> Result<Option<String>, String> {
    match method {
        None => Ok(None),
        Some(m) if HYDRATION_METHODS.contains(&m) => Ok(Some(m.to_string())),
        Some(other) => Err(format!(
            "无效的保湿方式：{other}（应为 未设/手动加水/水塔）"
        )),
    }
}

/// 按锚点名解析巢穴保湿操作的 id；改名/不存在为 None（调用方按边界降级）。
fn hydration_action_id(conn: &Connection) -> Result<Option<i64>, String> {
    conn.query_row(
        "SELECT id FROM care_action WHERE name = ?1",
        params![HYDRATION_ACTION_NAME],
        |row| row.get(0),
    )
    .optional()
    .map_err(db_err)
}

/// 整窗提交里的周期增删预检（写前人话报错，事务内写失败兜底回滚）：
/// 天数 1..=365（与 set_colony_action_interval 同口径）、操作必须存在。
/// 窝的存在性不在此查——新建时同事务先插窝、编辑时更新目标本就须存在。
fn validate_interval_changes(conn: &Connection, input: &ColonyInput) -> Result<(), String> {
    for change in &input.interval_changes {
        if let Some(days) = change.interval_days {
            if !INTERVAL_DAYS_RANGE.contains(&days) {
                return Err(format!("每窝周期应是 1–365 的整数天（收到 {days}）"));
            }
        }
        let found: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM care_action WHERE id = ?1",
                params![change.action_id],
                |row| row.get(0),
            )
            .map_err(db_err)?;
        if found == 0 {
            return Err(format!("操作不存在（id={}）", change.action_id));
        }
    }
    Ok(())
}

/// 在事务上应用整窗提交的每窝周期增删（写前校验见 [`validate_interval_changes`]）。
/// `drop_hydration_clears` = 净零守卫（库内与提交方式都为未设）：丢弃针对巢穴
/// 保湿行的清除，其余行照常处理。
fn apply_interval_changes(
    tx: &Connection,
    colony_id: i64,
    changes: &[ColonyIntervalChange],
    drop_hydration_clears: bool,
) -> Result<(), String> {
    if changes.is_empty() {
        return Ok(());
    }
    let hydration_id = if drop_hydration_clears {
        hydration_action_id(tx)?
    } else {
        None
    };
    for change in changes {
        match change.interval_days {
            Some(days) => {
                tx.execute(
                    "INSERT INTO colony_action_interval (colony_id, action_id, interval_days)
                     VALUES (?1, ?2, ?3)
                     ON CONFLICT(colony_id, action_id)
                     DO UPDATE SET interval_days = excluded.interval_days",
                    params![colony_id, change.action_id, days],
                )
                .map_err(db_err)?;
            }
            None => {
                if drop_hydration_clears && hydration_id == Some(change.action_id) {
                    continue; // 净零（F4）：未设往返不删库内已有周期行
                }
                tx.execute(
                    "DELETE FROM colony_action_interval
                     WHERE colony_id = ?1 AND action_id = ?2",
                    params![colony_id, change.action_id],
                )
                .map_err(db_err)?;
            }
        }
    }
    Ok(())
}

/// 清回未设：删该窝保湿周期行（锚点名定位；改名匹配不到时按已知边界跳过）。
fn delete_hydration_interval_row(tx: &Connection, colony_id: i64) -> Result<(), String> {
    if let Some(hydrate_id) = hydration_action_id(tx)? {
        tx.execute(
            "DELETE FROM colony_action_interval WHERE colony_id = ?1 AND action_id = ?2",
            params![colony_id, hydrate_id],
        )
        .map_err(db_err)?;
    }
    Ok(())
}

pub fn create_colony(conn: &Connection, input: &ColonyInput, today: &str) -> Result<Colony, String> {
    let (name, species, start_date) = normalize_colony_input(conn, input, None)?;
    let hydration_method = validate_hydration_method(input.hydration_method.as_deref())?;
    validate_interval_changes(conn, input)?;
    // 原子性（spec F2）：建窝 + 方式 + 初始周期同一条命令、同一事务，任一步
    // 失败整体回滚——绝无「窝建了但周期没落」的半截状态。
    let tx = conn.unchecked_transaction().map_err(db_err)?;
    tx.execute(
        "INSERT INTO colony (name, species, location_id, start_date, status, hydration_method)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![name, species, input.location_id, start_date, input.status, hydration_method],
    )
    .map_err(|e| friendly_unique_err(e, "colony.name", &name))?;
    let id = tx.last_insert_rowid();
    apply_interval_changes(&tx, id, &input.interval_changes, false)?;
    tx.commit().map_err(db_err)?;
    get_colony(conn, id, today)
}

pub fn update_colony(
    conn: &Connection,
    id: i64,
    input: &ColonyInput,
    today: &str,
) -> Result<Colony, String> {
    let (name, species, start_date) = normalize_colony_input(conn, input, Some(id))?;
    let hydration_method = validate_hydration_method(input.hydration_method.as_deref())?;
    validate_interval_changes(conn, input)?;
    // 库内方式原值（状态矩阵判定基准，spec 判定提示：不收前端旗标）
    let stored_method: Option<String> = conn
        .query_row(
            "SELECT hydration_method FROM colony WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => "窝不存在".to_string(),
            other => db_err(other),
        })?;
    // 清回未设：库内原值非未设（manual/tower）且提交=未设 → 删保湿周期行
    let clear_hydration = stored_method.is_some() && hydration_method.is_none();
    // 净零守卫：库内与提交都为未设 → 会话内「选了又改回」不删周期行（B 仍 B）
    let net_zero = stored_method.is_none() && hydration_method.is_none();

    // 整窗单事务（spec F6）：基础字段 + 方式变更 + 周期增删一次提交，任一步
    // 失败整体回滚——名字等基础字段绝不先落库。
    let tx = conn.unchecked_transaction().map_err(db_err)?;
    let changed = tx
        .execute(
            "UPDATE colony SET name = ?1, species = ?2, location_id = ?3, start_date = ?4,
             status = ?5, hydration_method = ?6 WHERE id = ?7",
            params![name, species, input.location_id, start_date, input.status, hydration_method, id],
        )
        .map_err(|e| friendly_unique_err(e, "colony.name", &name))?;
    if changed == 0 {
        // tx 在此 drop，自动回滚
        return Err("窝不存在".into());
    }
    apply_interval_changes(&tx, id, &input.interval_changes, net_zero)?;
    // 清回未设的删行放在周期增删之后（同事务）：方式动作是删行的最终裁决，
    // 矛盾提交（既清方式又带保湿 upsert）落成「未设+无周期」的 A 态。
    if clear_hydration {
        delete_hydration_interval_row(&tx, id)?;
    }
    tx.commit().map_err(db_err)?;
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

/// 仅无记录窝可删；有 care_log 行则拒绝。顺带清掉该窝的提醒台账、冬眠段、每窝
/// 周期行与巢况登记（webui-checkin 票 02，用户故事 11：删窝连带清巢况与照片元
/// 数据；照片文件清理随票 07 照片管线接管）。全部 DELETE 包在同一事务里，任一
/// 失败整体回滚，不留中间态。
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
    tx.execute(
        "DELETE FROM colony_action_interval WHERE colony_id = ?1",
        params![id],
    )
    .map_err(db_err)?;
    tx.execute(
        "DELETE FROM nest_photo WHERE checkin_id IN
             (SELECT id FROM nest_checkin WHERE colony_id = ?1)",
        params![id],
    )
    .map_err(db_err)?;
    tx.execute("DELETE FROM nest_checkin WHERE colony_id = ?1", params![id])
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

// ── 每窝周期（每窝周期票 01）─────────────────────────────────────────────
// 术语见 CONTEXT.md「每窝周期」：挂在窝上、按操作设的周期（天）。设了即提醒并
// 取代该操作层建议间隔，没设沿用操作层性质。本节只做存取与校验（存得进、读得
// 出、校验得住）；超期判定接线在票 02，前端在票 03/04。

/// 周期合法域（spec F4）：整数 1..=365；schema CHECK 同款兜底。
const INTERVAL_DAYS_RANGE: std::ops::RangeInclusive<i64> = 1..=365;

/// 窝与操作的存在性预检（外键裸 REFERENCES，应用层给人话报错）。
fn ensure_interval_targets(
    conn: &Connection,
    colony_id: i64,
    action_id: i64,
) -> Result<(), String> {
    let colony: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM colony WHERE id = ?1",
            params![colony_id],
            |row| row.get(0),
        )
        .map_err(db_err)?;
    if colony == 0 {
        return Err(format!("窝不存在（id={colony_id}）"));
    }
    let action: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM care_action WHERE id = ?1",
            params![action_id],
            |row| row.get(0),
        )
        .map_err(db_err)?;
    if action == 0 {
        return Err(format!("操作不存在（id={action_id}）"));
    }
    Ok(())
}

/// 设置/清除某窝某操作的每窝周期：`Some(天数)` upsert 一行（已设即改，立即
/// 生效）；`None` 删行 = 未设（未设时清除也成功，幂等）。天数须为 1..=365 的
/// 整数，窝或操作须存在，否则人话报错。与 Tauri 解耦，lib.rs 薄包装成桌面命令。
pub fn set_colony_action_interval(
    conn: &Connection,
    colony_id: i64,
    action_id: i64,
    interval_days: Option<i64>,
) -> Result<(), String> {
    if let Some(days) = interval_days {
        if !INTERVAL_DAYS_RANGE.contains(&days) {
            return Err(format!("每窝周期应是 1–365 的整数天（收到 {days}）"));
        }
    }
    ensure_interval_targets(conn, colony_id, action_id)?;
    match interval_days {
        Some(days) => conn
            .execute(
                "INSERT INTO colony_action_interval (colony_id, action_id, interval_days)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(colony_id, action_id)
                 DO UPDATE SET interval_days = excluded.interval_days",
                params![colony_id, action_id, days],
            )
            .map_err(db_err)?,
        None => conn
            .execute(
                "DELETE FROM colony_action_interval
                 WHERE colony_id = ?1 AND action_id = ?2",
                params![colony_id, action_id],
            )
            .map_err(db_err)?,
    };
    Ok(())
}

/// 某窝已设的全部每窝周期：`action_id → interval_days`。读侧内部函数，供票 02
/// 的 tiles/提醒接线取"窝 × 操作"覆盖值（本票不改 tiles_for_colony 行为）；
/// 未设任何周期的窝返回空表。
#[allow(dead_code)] // 每窝周期票 02（tiles/提醒接线）消费；本票先落读侧
pub fn intervals_for_colony(
    conn: &Connection,
    colony_id: i64,
) -> Result<HashMap<i64, i64>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT action_id, interval_days FROM colony_action_interval
             WHERE colony_id = ?1",
        )
        .map_err(db_err)?;
    let rows = stmt
        .query_map(params![colony_id], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(db_err)?
        .collect::<Result<HashMap<_, _>, _>>()
        .map_err(db_err)?;
    Ok(rows)
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

/// 启用/停用双向开关（票 04：补上停用后的恢复通道）。
pub fn set_location_enabled(conn: &Connection, id: i64, enabled: bool) -> Result<(), String> {
    let changed = conn
        .execute(
            "UPDATE location SET enabled = ?1 WHERE id = ?2",
            params![enabled, id],
        )
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
            hydration_method: None,
            interval_changes: Vec::new(),
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

    #[test]
    fn list_colonies_embeds_action_tiles_and_recent_summary() {
        let conn = mem_conn();
        let c = create_colony(&conn, &input("大头一号", Some(1)), TODAY).unwrap();
        // 喂食昨天（今天补录，距上次按发生时间 = 1）；保湿 3 天前
        conn.execute(
            "INSERT INTO care_log (colony_id, action_id, occurred_at, note, created_at)
             VALUES (?1, 1, '2026-09-17 20:00:00', '', '2026-09-18 08:00:00')",
            params![c.id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO care_log (colony_id, action_id, occurred_at, note, created_at)
             VALUES (?1, 3, '2026-09-15 09:00:00', '', '2026-09-15 09:00:00')",
            params![c.id],
        )
        .unwrap();

        let list = list_colonies(&conn, TODAY).unwrap();
        let colony = &list[0];

        // 每个启用操作一块，带距上次/超期态（v7 起撤食预置加入，5 块）
        assert_eq!(colony.actions.len(), 5);
        let feed = colony.actions.iter().find(|t| t.name == "喂食").unwrap();
        assert_eq!(feed.days_since_last, Some(1));
        assert!(!feed.overdue);
        let hydrate = colony.actions.iter().find(|t| t.name == "巢穴保湿").unwrap();
        assert_eq!(hydrate.days_since_last, Some(3));
        assert!(!hydrate.overdue, "登记类永不红");

        // 最近摘要按发生时间倒序，至多 2 条
        assert_eq!(colony.recent.len(), 2);
        assert_eq!(colony.recent[0].action_name, "喂食");
        assert_eq!(colony.recent[1].action_name, "巢穴保湿");
    }

    // ── 归档 ──

    #[test]
    fn list_colonies_embeds_open_hibernation_preview_for_banner() {
        // 票 05：冬眠卡片横幅数据（入眠日/预计出眠）由后端随窝一起给；
        // 只有开放段算数，闭合的历史段不算。
        let conn = mem_conn();
        let a = create_colony(&conn, &input("大头一号", Some(1)), TODAY).unwrap();
        let b = create_colony(&conn, &input("针毛一号", Some(1)), TODAY).unwrap();

        conn.execute(
            "INSERT INTO hibernation (colony_id, start_date, expected_end_date, actual_end_date)
             VALUES (?1, '2025-12-01', '2026-02-01', '2026-02-01')",
            params![a.id],
        )
        .unwrap();
        crate::hibernation::start_hibernation(&conn, b.id, "2026-09-01", "2026-12-01", TODAY)
            .unwrap();

        let list = list_colonies(&conn, TODAY).unwrap();
        let ha = list.iter().find(|c| c.id == a.id).unwrap();
        assert!(ha.hibernation.is_none(), "只有闭合历史段 → None");
        assert_eq!(ha.status, "active");

        let hb = list.iter().find(|c| c.id == b.id).unwrap();
        let seg = hb.hibernation.as_ref().expect("冬眠中的窝应带开放段");
        assert_eq!(seg.start_date, "2026-09-01");
        assert_eq!(seg.expected_end_date, "2026-12-01");
    }

    #[test]
    fn list_colonies_embeds_avatar_photo_reference() {
        // 窝头像票 01：头像照片引用直入窝列表载荷（纯投影，前端不重复推导）——
        // 有照片窝指向「第一条含照片登记的第一张」；无照片窝 None。
        let conn = mem_conn();
        let with_photos = create_colony(&conn, &input("大头一号", Some(1)), TODAY).unwrap();
        let bare = create_colony(&conn, &input("无照窝", None), TODAY).unwrap();

        // 早登记带照片；最新登记纯文字 → 头像跳过纯文字取早登记那张
        let older = crate::nest_checkin::save_checkin(
            &conn,
            &crate::nest_checkin::CheckinInput {
                colony_id: with_photos.id,
                date: "2026-09-10".into(),
                queen_count: None,
                worker_count: None,
                moved_nest: false,
                note: Some("带照片".into()),
            },
            TODAY,
            "2026-09-10 08:00:00",
        )
        .unwrap()
        .id;
        conn.execute(
            "INSERT INTO nest_photo (checkin_id, rel_path, original_name, note)
             VALUES (?1, '1/avatar.jpg', NULL, '')",
            params![older],
        )
        .unwrap();
        crate::nest_checkin::save_checkin(
            &conn,
            &crate::nest_checkin::CheckinInput {
                colony_id: with_photos.id,
                date: "2026-09-15".into(),
                queen_count: Some(2),
                worker_count: None,
                moved_nest: false,
                note: None,
            },
            TODAY,
            "2026-09-15 08:00:00",
        )
        .unwrap();

        let list = list_colonies(&conn, TODAY).unwrap();
        let a = list.iter().find(|c| c.id == with_photos.id).unwrap();
        let avatar = a.avatar.as_ref().expect("有照片窝应带头像引用");
        assert_eq!(avatar.rel_path, "1/avatar.jpg");
        assert_eq!(avatar.checkin_id, older, "跳过纯文字的最新登记");
        assert_eq!(avatar.crop, None, "未调过裁剪 = 居中（None）");
        let b = list.iter().find(|c| c.id == bare.id).unwrap();
        assert!(b.avatar.is_none(), "无照片窝头像为空");
    }

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
        // 造一条每窝周期行（每窝周期票 01）：删窝顺带清理，不留悬挂引用
        set_colony_action_interval(&conn, c.id, 1, Some(3)).unwrap();

        delete_colony(&conn, c.id).unwrap();
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM colony WHERE id = ?1", c.id), 0);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM hibernation WHERE colony_id = ?1", c.id), 0);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM reminder_ledger WHERE colony_id = ?1", c.id), 0);
        assert_eq!(
            count(&conn, "SELECT COUNT(*) FROM colony_action_interval WHERE colony_id = ?1", c.id),
            0,
            "删窝应连带清掉该窝全部周期行"
        );
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

    // ── 每窝周期（每窝周期票 01）──

    /// 查某窝某操作当前周期（未设 = None）。
    fn interval_days(conn: &Connection, colony_id: i64, action_id: i64) -> Option<i64> {
        conn.query_row(
            "SELECT interval_days FROM colony_action_interval
             WHERE colony_id = ?1 AND action_id = ?2",
            params![colony_id, action_id],
            |row| row.get(0),
        )
        .ok()
    }

    fn action_id_by_name(conn: &Connection, name: &str) -> i64 {
        conn.query_row(
            "SELECT id FROM care_action WHERE name = ?1",
            params![name],
            |row| row.get(0),
        )
        .expect("查预置操作失败")
    }

    #[test]
    fn set_interval_upserts_then_updates_single_row() {
        let conn = mem_conn();
        let c = create_colony(&conn, &input("大头一号", Some(1)), TODAY).unwrap();
        let hydrate = action_id_by_name(&conn, "巢穴保湿");

        set_colony_action_interval(&conn, c.id, hydrate, Some(2)).unwrap();
        assert_eq!(interval_days(&conn, c.id, hydrate), Some(2));

        // 改周期 = 原行更新，不新增
        set_colony_action_interval(&conn, c.id, hydrate, Some(7)).unwrap();
        assert_eq!(interval_days(&conn, c.id, hydrate), Some(7));
        assert_eq!(
            count(
                &conn,
                "SELECT COUNT(*) FROM colony_action_interval WHERE colony_id = ?1",
                c.id
            ),
            1
        );

        // 边界值 1 与 365 都合法
        set_colony_action_interval(&conn, c.id, hydrate, Some(1)).unwrap();
        assert_eq!(interval_days(&conn, c.id, hydrate), Some(1));
        set_colony_action_interval(&conn, c.id, hydrate, Some(365)).unwrap();
        assert_eq!(interval_days(&conn, c.id, hydrate), Some(365));
    }

    #[test]
    fn clear_interval_removes_row_and_unset_clear_is_ok() {
        let conn = mem_conn();
        let c = create_colony(&conn, &input("大头一号", Some(1)), TODAY).unwrap();
        let hydrate = action_id_by_name(&conn, "巢穴保湿");

        set_colony_action_interval(&conn, c.id, hydrate, Some(3)).unwrap();
        // 清除 = 删行（未设）
        set_colony_action_interval(&conn, c.id, hydrate, None).unwrap();
        assert_eq!(interval_days(&conn, c.id, hydrate), None);

        // 未设时再清除也成功（幂等）
        set_colony_action_interval(&conn, c.id, hydrate, None).unwrap();
        assert_eq!(
            count(
                &conn,
                "SELECT COUNT(*) FROM colony_action_interval WHERE colony_id = ?1",
                c.id
            ),
            0
        );
    }

    #[test]
    fn set_interval_rejects_out_of_range_days() {
        let conn = mem_conn();
        let c = create_colony(&conn, &input("大头一号", Some(1)), TODAY).unwrap();
        let hydrate = action_id_by_name(&conn, "巢穴保湿");

        for bad in [0, -1, -365, 366, 10000] {
            let err = set_colony_action_interval(&conn, c.id, hydrate, Some(bad)).unwrap_err();
            assert!(err.contains("1–365"), "interval_days={bad} 实际错误：{err}");
            assert!(err.contains(&bad.to_string()), "报错应带实际值 {bad}：{err}");
        }
        // 越界值一个都没落库
        assert_eq!(
            count(
                &conn,
                "SELECT COUNT(*) FROM colony_action_interval WHERE colony_id = ?1",
                c.id
            ),
            0
        );
    }

    #[test]
    fn set_interval_rejects_missing_colony_or_action() {
        let conn = mem_conn();
        let c = create_colony(&conn, &input("大头一号", Some(1)), TODAY).unwrap();
        let feed = action_id_by_name(&conn, "喂食");

        // 幽灵窝：设与清都拒绝
        let err = set_colony_action_interval(&conn, 999, feed, Some(3)).unwrap_err();
        assert!(err.contains("窝不存在"), "实际错误：{err}");
        let err = set_colony_action_interval(&conn, 999, feed, None).unwrap_err();
        assert!(err.contains("窝不存在"), "实际错误：{err}");

        // 幽灵操作：设与清都拒绝
        let err = set_colony_action_interval(&conn, c.id, 999, Some(3)).unwrap_err();
        assert!(err.contains("操作不存在"), "实际错误：{err}");
        let err = set_colony_action_interval(&conn, c.id, 999, None).unwrap_err();
        assert!(err.contains("操作不存在"), "实际错误：{err}");
    }

    #[test]
    fn intervals_for_colony_maps_actions_to_days() {
        let conn = mem_conn();
        let a = create_colony(&conn, &input("大头一号", Some(1)), TODAY).unwrap();
        let b = create_colony(&conn, &input("针毛一号", Some(1)), TODAY).unwrap();
        let feed = action_id_by_name(&conn, "喂食");
        let hydrate = action_id_by_name(&conn, "巢穴保湿");

        // 未设任何周期的窝 → 空表
        assert!(intervals_for_colony(&conn, a.id).unwrap().is_empty());

        set_colony_action_interval(&conn, a.id, feed, Some(5)).unwrap();
        set_colony_action_interval(&conn, a.id, hydrate, Some(2)).unwrap();
        set_colony_action_interval(&conn, b.id, hydrate, Some(9)).unwrap();

        let map = intervals_for_colony(&conn, a.id).unwrap();
        assert_eq!(map.len(), 2);
        assert_eq!(map.get(&feed), Some(&5));
        assert_eq!(map.get(&hydrate), Some(&2));

        // 窝之间互不串
        let map_b = intervals_for_colony(&conn, b.id).unwrap();
        assert_eq!(map_b.len(), 1);
        assert_eq!(map_b.get(&hydrate), Some(&9));
    }

    // ── 保湿方式（保湿方式票 01）：状态矩阵落库语义 + 写入原子性 ──

    /// 读某窝当前保湿方式（None = 未设）。
    fn hydration_method_of(conn: &Connection, colony_id: i64) -> Option<String> {
        conn.query_row(
            "SELECT hydration_method FROM colony WHERE id = ?1",
            params![colony_id],
            |row| row.get(0),
        )
        .expect("读保湿方式失败")
    }

    fn change(action_id: i64, interval_days: Option<i64>) -> ColonyIntervalChange {
        ColonyIntervalChange { action_id, interval_days }
    }

    #[test]
    fn create_colony_carries_method_and_initial_interval_atomically() {
        let conn = mem_conn();
        let hydrate = action_id_by_name(&conn, "巢穴保湿");
        let mut inp = input("水塔窝", Some(1));
        inp.hydration_method = Some("tower".into());
        inp.interval_changes = vec![change(hydrate, Some(15))];

        let c = create_colony(&conn, &inp, TODAY).unwrap();
        assert_eq!(c.hydration_method.as_deref(), Some("tower"));
        assert_eq!(interval_days(&conn, c.id, hydrate), Some(15));
    }

    #[test]
    fn create_colony_without_method_stays_unset_and_rowless() {
        let conn = mem_conn();
        let c = create_colony(&conn, &input("安静窝", None), TODAY).unwrap();
        assert_eq!(c.hydration_method, None);
        assert_eq!(
            count(&conn, "SELECT COUNT(*) FROM colony_action_interval WHERE colony_id = ?1", c.id),
            0
        );
        // 方式设了、周期不带 = D 态（有标签不提醒），合法落库
        let mut inp = input("空周期窝", None);
        inp.hydration_method = Some("manual".into());
        let d = create_colony(&conn, &inp, TODAY).unwrap();
        assert_eq!(d.hydration_method.as_deref(), Some("manual"));
        assert_eq!(
            count(&conn, "SELECT COUNT(*) FROM colony_action_interval WHERE colony_id = ?1", d.id),
            0
        );
    }

    #[test]
    fn create_colony_rejects_bad_interval_and_unknown_method_without_colony() {
        let conn = mem_conn();
        let hydrate = action_id_by_name(&conn, "巢穴保湿");

        // 非法天数：整体失败，窝不落库（验收 2）
        let mut inp = input("甲", None);
        inp.hydration_method = Some("manual".into());
        inp.interval_changes = vec![change(hydrate, Some(0))];
        assert!(create_colony(&conn, &inp, TODAY).is_err());
        // 幽灵操作同样整体失败
        let mut inp = input("乙", None);
        inp.interval_changes = vec![change(999, Some(7))];
        assert!(create_colony(&conn, &inp, TODAY).is_err());
        // 枚举外方式被拒
        let mut inp = input("丙", None);
        inp.hydration_method = Some("sprinkler".into());
        assert!(create_colony(&conn, &inp, TODAY).is_err());

        let total: i64 = conn
            .query_row("SELECT COUNT(*) FROM colony", [], |row| row.get(0))
            .unwrap();
        assert_eq!(total, 0, "任一步失败窝整体不落库");
    }

    #[test]
    fn create_colony_interval_write_failure_rolls_back_colony() {
        // 事务性硬验收：周期写入在事务内被打断（触发器模拟）→ 已插入的窝回滚
        let conn = mem_conn();
        let hydrate = action_id_by_name(&conn, "巢穴保湿");
        conn.execute(
            "CREATE TRIGGER block_interval_insert BEFORE INSERT ON colony_action_interval
             BEGIN SELECT RAISE(ABORT, 'blocked'); END",
            [],
        )
        .unwrap();
        let mut inp = input("倒霉窝", None);
        inp.hydration_method = Some("manual".into());
        inp.interval_changes = vec![change(hydrate, Some(7))];

        assert!(create_colony(&conn, &inp, TODAY).is_err());
        let total: i64 = conn
            .query_row("SELECT COUNT(*) FROM colony", [], |row| row.get(0))
            .unwrap();
        assert_eq!(total, 0, "周期写失败时窝必须整体回滚");
    }

    #[test]
    fn update_colony_full_window_single_transaction() {
        // 整窗单事务（spec F6）：基础字段 + 方式变更 + 周期增删一次提交全部生效
        let conn = mem_conn();
        let hydrate = action_id_by_name(&conn, "巢穴保湿");
        let feed = action_id_by_name(&conn, "喂食");
        let c = create_colony(&conn, &input("大头一号", Some(1)), TODAY).unwrap();
        set_colony_action_interval(&conn, c.id, feed, Some(3)).unwrap();

        let mut inp = input("大头壹号", Some(2));
        inp.hydration_method = Some("manual".into());
        inp.interval_changes = vec![change(hydrate, Some(7)), change(feed, None)];
        let updated = update_colony(&conn, c.id, &inp, TODAY).unwrap();

        assert_eq!(updated.name, "大头壹号");
        assert_eq!(updated.location_id, Some(2));
        assert_eq!(updated.hydration_method.as_deref(), Some("manual"));
        assert_eq!(interval_days(&conn, c.id, hydrate), Some(7));
        assert_eq!(interval_days(&conn, c.id, feed), None, "未列出的删行照常生效");
    }

    #[test]
    fn update_colony_hydration_failure_keeps_base_fields() {
        // 验收 3：保湿部分失败（周期写入被打断）→ 名字等基础字段也不落库
        let conn = mem_conn();
        let hydrate = action_id_by_name(&conn, "巢穴保湿");
        let mut seed = input("原名", Some(1));
        seed.hydration_method = Some("tower".into());
        let c = create_colony(&conn, &seed, TODAY).unwrap();

        conn.execute(
            "CREATE TRIGGER block_interval_insert BEFORE INSERT ON colony_action_interval
             BEGIN SELECT RAISE(ABORT, 'blocked'); END",
            [],
        )
        .unwrap();
        let mut inp = input("新名", Some(2));
        inp.hydration_method = Some("manual".into());
        inp.interval_changes = vec![change(hydrate, Some(7))];
        assert!(update_colony(&conn, c.id, &inp, TODAY).is_err());

        // 整窗回滚：名字/地点/方式/周期全部原样
        assert_eq!(hydration_method_of(&conn, c.id).as_deref(), Some("tower"));
        assert_eq!(interval_days(&conn, c.id, hydrate), None);
        let row: (String, Option<i64>) = conn
            .query_row(
                "SELECT name, location_id FROM colony WHERE id = ?1",
                params![c.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(row, ("原名".into(), Some(1)));
    }

    #[test]
    fn update_colony_clear_to_unset_deletes_hydration_interval_row() {
        // 验收 5 前半：库内=manual/tower 且保存清回未设 → 方式置 NULL 且周期行被删
        let conn = mem_conn();
        let hydrate = action_id_by_name(&conn, "巢穴保湿");
        let mut inp = input("水塔窝", None);
        inp.hydration_method = Some("tower".into());
        inp.interval_changes = vec![change(hydrate, Some(15))];
        let c = create_colony(&conn, &inp, TODAY).unwrap();

        let mut clear = input("水塔窝", None);
        clear.hydration_method = None; // 清回未设
        let updated = update_colony(&conn, c.id, &clear, TODAY).unwrap();

        assert_eq!(updated.hydration_method, None);
        assert_eq!(
            interval_days(&conn, c.id, hydrate),
            None,
            "清回未设必须删掉该窝保湿周期行"
        );
    }

    #[test]
    fn update_colony_net_zero_unset_journey_keeps_interval_row() {
        // 验收 4（净零语义，F4）：库内=未设+已有周期（存量 B 态），保存把方式
        // 「选了又改回未设」——即使前端把清行请求一并提交，库内周期行仍在
        let conn = mem_conn();
        let hydrate = action_id_by_name(&conn, "巢穴保湿");
        let c = create_colony(&conn, &input("存量窝", None), TODAY).unwrap();
        set_colony_action_interval(&conn, c.id, hydrate, Some(12)).unwrap();

        // 模拟会话内 未设→手动→未设：提交值=未设，且带上会话期间产生的清行
        let mut inp = input("存量窝", None);
        inp.hydration_method = None;
        inp.interval_changes = vec![change(hydrate, None)];
        let updated = update_colony(&conn, c.id, &inp, TODAY).unwrap();

        assert_eq!(updated.hydration_method, None, "方式仍为未设");
        assert_eq!(
            interval_days(&conn, c.id, hydrate),
            Some(12),
            "净零：库内已有周期行原样保留（B 仍 B）"
        );
    }

    #[test]
    fn update_colony_stored_unset_keeps_interval_row_on_any_edit_save() {
        // 验收 5 后半：库内=未设时任何编辑保存不删周期行（此处只改名字）
        let conn = mem_conn();
        let hydrate = action_id_by_name(&conn, "巢穴保湿");
        let c = create_colony(&conn, &input("存量窝", None), TODAY).unwrap();
        set_colony_action_interval(&conn, c.id, hydrate, Some(12)).unwrap();

        let mut inp = input("改名窝", Some(1));
        inp.hydration_method = None;
        update_colony(&conn, c.id, &inp, TODAY).unwrap();

        assert_eq!(hydration_method_of(&conn, c.id), None);
        assert_eq!(interval_days(&conn, c.id, hydrate), Some(12));
    }

    #[test]
    fn update_colony_days_only_clear_keeps_method() {
        // 规则 6（C→D）：只清天数删周期行，方式保留
        let conn = mem_conn();
        let hydrate = action_id_by_name(&conn, "巢穴保湿");
        let mut inp = input("手动窝", None);
        inp.hydration_method = Some("manual".into());
        inp.interval_changes = vec![change(hydrate, Some(7))];
        let c = create_colony(&conn, &inp, TODAY).unwrap();

        let mut clear_days = input("手动窝", None);
        clear_days.hydration_method = Some("manual".into());
        clear_days.interval_changes = vec![change(hydrate, None)];
        let updated = update_colony(&conn, c.id, &clear_days, TODAY).unwrap();

        assert_eq!(updated.hydration_method.as_deref(), Some("manual"), "方式保留");
        assert_eq!(interval_days(&conn, c.id, hydrate), None, "周期行被删（D 态）");
    }

    #[test]
    fn update_colony_method_switch_keeps_existing_interval_row() {
        // 规则 3 库内侧：手动↔水塔切换不动库内已有周期行
        let conn = mem_conn();
        let hydrate = action_id_by_name(&conn, "巢穴保湿");
        let mut inp = input("手动窝", None);
        inp.hydration_method = Some("manual".into());
        inp.interval_changes = vec![change(hydrate, Some(7))];
        let c = create_colony(&conn, &inp, TODAY).unwrap();

        let mut switch = input("手动窝", None);
        switch.hydration_method = Some("tower".into());
        let updated = update_colony(&conn, c.id, &switch, TODAY).unwrap();

        assert_eq!(updated.hydration_method.as_deref(), Some("tower"));
        assert_eq!(interval_days(&conn, c.id, hydrate), Some(7), "切方式不删不覆盖周期行");
    }

    #[test]
    fn update_colony_unset_to_method_keeps_existing_interval_untouched() {
        // 规则 2 库内侧：未设→选方式时已有周期非空不动（B→C，值原样）
        let conn = mem_conn();
        let hydrate = action_id_by_name(&conn, "巢穴保湿");
        let c = create_colony(&conn, &input("存量窝", None), TODAY).unwrap();
        set_colony_action_interval(&conn, c.id, hydrate, Some(12)).unwrap();

        let mut inp = input("存量窝", None);
        inp.hydration_method = Some("manual".into()); // 周期框非空 → 前端不提交该行
        let updated = update_colony(&conn, c.id, &inp, TODAY).unwrap();

        assert_eq!(updated.hydration_method.as_deref(), Some("manual"));
        assert_eq!(interval_days(&conn, c.id, hydrate), Some(12));
    }

    #[test]
    fn update_colony_untouched_round_trip_preserves_hydration_config() {
        // 规则 5：改名字等无关编辑绝不影响保湿配置（C 态往返）
        let conn = mem_conn();
        let hydrate = action_id_by_name(&conn, "巢穴保湿");
        let mut inp = input("原名", None);
        inp.hydration_method = Some("tower".into());
        inp.interval_changes = vec![change(hydrate, Some(15))];
        let c = create_colony(&conn, &inp, TODAY).unwrap();

        let mut rename = input("新名", None);
        rename.hydration_method = Some("tower".into());
        update_colony(&conn, c.id, &rename, TODAY).unwrap();

        assert_eq!(hydration_method_of(&conn, c.id).as_deref(), Some("tower"));
        assert_eq!(interval_days(&conn, c.id, hydrate), Some(15));
    }

    #[test]
    fn update_colony_rejects_unknown_method_value_without_changes() {
        let conn = mem_conn();
        let c = create_colony(&conn, &input("甲", None), TODAY).unwrap();
        let mut inp = input("甲", None);
        inp.hydration_method = Some("drip".into());
        assert!(update_colony(&conn, c.id, &inp, TODAY).is_err());
        assert_eq!(hydration_method_of(&conn, c.id), None, "拒绝时库内原样");
    }

    #[test]
    fn hydration_writes_do_not_disturb_interval_from_colony_semantics() {
        // 回归（验收 6）：interval_from_colony 既有语义原样——设了每窝周期时
        // effective_interval_days 即原始值；清回未设删行后回到操作层性质。
        let conn = mem_conn();
        let hydrate = action_id_by_name(&conn, "巢穴保湿");
        let mut inp = input("水塔窝", None);
        inp.hydration_method = Some("tower".into());
        inp.interval_changes = vec![change(hydrate, Some(15))];
        let c = create_colony(&conn, &inp, TODAY).unwrap();

        let tiles = crate::care::tiles_for_colony(&conn, c.id, TODAY).unwrap();
        let tile = tiles.iter().find(|t| t.name == "巢穴保湿").unwrap();
        assert!(tile.interval_from_colony, "设了每窝周期 → 标记来自每窝");
        assert_eq!(tile.effective_interval_days, Some(15), "有效周期即原始每窝周期值");

        // 清回未设：删行后该窝该操作回到「未设沿操作层」——登记类无建议间隔，
        // effective 回 None、标记回 false；独立设/清命令（其余读写维持现状）照常可用
        let mut clear = input("水塔窝", None);
        clear.hydration_method = None;
        update_colony(&conn, c.id, &clear, TODAY).unwrap();
        let tiles = crate::care::tiles_for_colony(&conn, c.id, TODAY).unwrap();
        let tile = tiles.iter().find(|t| t.name == "巢穴保湿").unwrap();
        assert!(!tile.interval_from_colony);
        assert_eq!(tile.effective_interval_days, None);

        set_colony_action_interval(&conn, c.id, hydrate, Some(30)).unwrap();
        let tiles = crate::care::tiles_for_colony(&conn, c.id, TODAY).unwrap();
        let tile = tiles.iter().find(|t| t.name == "巢穴保湿").unwrap();
        assert!(tile.interval_from_colony);
        assert_eq!(tile.effective_interval_days, Some(30));
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
    fn set_location_enabled_false_disables_but_keeps_row() {
        let conn = mem_conn();
        let locs = list_locations(&conn).unwrap();
        let home_id = locs[0].id;
        set_location_enabled(&conn, home_id, false).unwrap();
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

        set_location_enabled(&conn, home_id, false).unwrap();
        assert!(!list_locations(&conn).unwrap().iter().any(|l| l.id == home_id && l.enabled));
    }

    #[test]
    fn erase_missing_location_rejected() {
        let conn = mem_conn();
        assert!(erase_location(&conn, 999).is_err());
    }

    #[test]
    fn set_location_enabled_round_trip() {
        // 票 04：停用的地点可再启用（此前只有 deactivate 一条单行道）
        let conn = mem_conn();
        let home = list_locations(&conn).unwrap()[0].id;

        set_location_enabled(&conn, home, false).unwrap();
        assert!(!list_locations(&conn).unwrap().iter().find(|l| l.id == home).unwrap().enabled);

        set_location_enabled(&conn, home, true).unwrap();
        assert!(list_locations(&conn).unwrap().iter().find(|l| l.id == home).unwrap().enabled);

        assert!(set_location_enabled(&conn, 999, true).unwrap_err().contains("地点不存在"));
    }
}
