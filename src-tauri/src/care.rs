//! 维护操作字典、记账与「距上次」计算（票 03）。
//!
//! 全部函数只吃 `&Connection`，与 Tauri 解耦，可被 cargo test 直接覆盖；
//! Tauri command 只是薄包装（见 lib.rs）。
//!
//! 行为对齐 spec：
//! - 发生时间（occurred_at）与录入时间（created_at）分开存，补录合法；
//! - 「距上次」= 今天 − max(该窝该操作最近一次发生日期, 最近出眠日期)（自然日，
//!   评审附录规则 5：出眠当天 0 天、不全红；无出眠史即纯记录基线，票 05）；
//! - 超期判定仅提醒类且 > 建议间隔；登记类永不红；
//! - 喂食可一条记录挂多种食物（log_food 多选关联），与记录同事务写入。

use std::collections::HashSet;

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

// ── DTO ──────────────────────────────────────────────────────────────────

/// 食物（含停用的：前端新建入口过滤 enabled；referenced=被历史记录或提醒台账
/// （food 维度，F3）引用，只能停用不能删）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Food {
    pub id: i64,
    pub name: String,
    pub enabled: bool,
    pub sort: i64,
    /// 食物建议间隔（天，F3）：距上次喂「该食物」超过它就单独提醒；NULL = 未设，
    /// 该食物只受喂食操作统一周期管。
    pub suggested_interval_days: Option<i64>,
    /// 预置项禁删，可停用（反馈第二轮 F2）。
    pub is_preset: bool,
    pub referenced: bool,
}

/// 喂食块里单个食物的「距上次」明细（反馈第二轮 F3）。
/// 基线与操作层同口径：max(该窝该食物最近一次喂食日期, 最近出眠日期)。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FoodTileStatus {
    pub food_id: i64,
    pub name: String,
    pub suggested_interval_days: Option<i64>,
    /// 该食物从未被喂过且无出眠史为 None。
    pub days_since_last: Option<i64>,
    /// 仅操作 kind=reminding 且已设周期且 > 周期；否则恒 false。
    pub overdue: bool,
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
    /// 今天 − max(最近一次发生日期, 最近出眠日期)（自然日，规则 5）；从未记录且无出眠史为 None。
    pub days_since_last: Option<i64>,
    /// 仅提醒类且 > 建议间隔；喂食类任一设周期食物超期也算（F3，Q2 决议）；
    /// 登记类/从未记录恒 false。
    pub overdue: bool,
    /// 逐食物「距上次」明细（F3）；仅喂食类非空，其余操作恒空数组。
    pub foods: Vec<FoodTileStatus>,
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

/// 发生时间不得晚于当前时间（补录合法、预记未来不合法；同刻允许）。
/// 两串均为库内统一 `YYYY-MM-DD HH:MM:SS`，字典序 = 时间序（票 04 停靠②）。
pub fn ensure_not_future(happened_at: &str, now: &str) -> Result<(), String> {
    if happened_at > now {
        return Err(format!("发生时间不能晚于当前时间（{happened_at} 在未来）"));
    }
    Ok(())
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
    ensure_not_future(&happened_at, now)?;
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

// ── 记录列表 / 编辑 / 删除（票 08）──────────────────────────────────────

/// 记录流水一行（食物按字典顺序；停用操作/食物照常返回显示名，规则 10）。
/// created_at=录入时间（与发生时间分开存，补录合法；导出 CSV 的「录入时间」列）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LogRow {
    pub id: i64,
    pub colony_id: i64,
    pub colony_name: String,
    /// 窝所属地点名（交互第三轮 #5）；未分组 = None
    pub location_name: Option<String>,
    pub action_id: i64,
    pub action_name: String,
    pub occurred_at: String,
    pub created_at: String,
    pub note: String,
    pub food_ids: Vec<i64>,
    pub food_names: Vec<String>,
}

/// list_logs 返回体：一页行 + 命中总数（total 恒为全量命中数，不随分页变）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LogPage {
    pub total: i64,
    pub rows: Vec<LogRow>,
}

/// 记录流水筛选入参：各筛选项可空，组合生效（AND）。
/// start/end 为 ISO 日期，按 occurred_at 日期部分闭区间过滤；note_keyword 为
/// 备注子串匹配（LIKE 通配符转义）；limit 缺省 50、上限 500，offset 缺省 0。
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct LogFilter {
    /// 地点筛选（交互第三轮 #1）：窝的所属地点；与 colony_id 组合生效
    #[serde(default)]
    pub location_id: Option<i64>,
    #[serde(default)]
    pub colony_id: Option<i64>,
    #[serde(default)]
    pub action_id: Option<i64>,
    #[serde(default)]
    pub start: Option<String>,
    #[serde(default)]
    pub end: Option<String>,
    #[serde(default)]
    pub note_keyword: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub offset: Option<i64>,
}

/// 编辑入参：字段为 None = 保持原值；food_ids 传空数组 = 清空食物关联。
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct LogUpdateInput {
    #[serde(default)]
    pub occurred_at: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub action_id: Option<i64>,
    #[serde(default)]
    pub food_ids: Option<Vec<i64>>,
}

/// 筛选用日期（`YYYY-MM-DD`），非法格式给友好错误。
fn parse_filter_date(s: &str) -> Result<String, String> {
    chrono::NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d")
        .map(|d| d.format("%Y-%m-%d").to_string())
        .map_err(|_| format!("日期格式应为 YYYY-MM-DD：{s}"))
}

/// 记录流水查询（spec API 契约 listLogs）：按 occurred_at DESC（同刻 id DESC），
/// 分页返回；total = 组合筛选命中总数。LIKE 通配符按字面匹配（转义 `_`/`%`/`\`）。
pub fn list_logs(conn: &Connection, filter: &LogFilter) -> Result<LogPage, String> {
    let mut wheres: Vec<String> = Vec::new();
    let mut args: Vec<rusqlite::types::Value> = Vec::new();
    if let Some(location_id) = filter.location_id {
        args.push(location_id.into());
        wheres.push(format!("c.location_id = ?{}", args.len()));
    }
    if let Some(colony_id) = filter.colony_id {
        args.push(colony_id.into());
        wheres.push(format!("l.colony_id = ?{}", args.len()));
    }
    if let Some(action_id) = filter.action_id {
        args.push(action_id.into());
        wheres.push(format!("l.action_id = ?{}", args.len()));
    }
    if let Some(s) = filter.start.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        args.push(parse_filter_date(s)?.into());
        wheres.push(format!("substr(l.occurred_at, 1, 10) >= ?{}", args.len()));
    }
    if let Some(e) = filter.end.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        args.push(parse_filter_date(e)?.into());
        wheres.push(format!("substr(l.occurred_at, 1, 10) <= ?{}", args.len()));
    }
    if let Some(kw) = filter.note_keyword.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        let escaped = kw.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
        args.push(format!("%{escaped}%").into());
        wheres.push(format!("l.note LIKE ?{} ESCAPE '\\'", args.len()));
    }
    let where_sql = if wheres.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", wheres.join(" AND "))
    };
    let limit = filter.limit.unwrap_or(50).clamp(1, 500);
    let offset = filter.offset.unwrap_or(0).max(0);

    let total: i64 = conn
        .query_row(
            &format!("SELECT COUNT(*) FROM care_log l JOIN colony c ON c.id = l.colony_id {where_sql}"),
            rusqlite::params_from_iter(args.iter()),
            |row| row.get(0),
        )
        .map_err(db_err)?;

    let mut stmt = conn
        .prepare(&format!(
            "SELECT l.id, l.colony_id, c.name, lo.name, l.action_id, a.name, l.occurred_at, l.note, l.created_at
             FROM care_log l
             JOIN colony c ON c.id = l.colony_id
             LEFT JOIN location lo ON lo.id = c.location_id
             JOIN care_action a ON a.id = l.action_id
             {where_sql}
             ORDER BY l.occurred_at DESC, l.id DESC
             LIMIT {limit} OFFSET {offset}"
        ))
        .map_err(db_err)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(args.iter()), |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, String>(8)?,
            ))
        })
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?;

    let mut out = Vec::with_capacity(rows.len());
    for (id, colony_id, colony_name, location_name, action_id, action_name, occurred_at, note, created_at) in rows {
        let mut stmt_food = conn
            .prepare(
                "SELECT lf.food_id, f.name FROM log_food lf JOIN food f ON f.id = lf.food_id
                 WHERE lf.log_id = ?1 ORDER BY f.sort, f.id",
            )
            .map_err(db_err)?;
        let pairs = stmt_food
            .query_map(params![id], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(db_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db_err)?;
        out.push(LogRow {
            id,
            colony_id,
            colony_name,
            location_name,
            action_id,
            action_name,
            occurred_at,
            created_at,
            note,
            food_ids: pairs.iter().map(|(fid, _)| *fid).collect(),
            food_names: pairs.into_iter().map(|(_, name)| name).collect(),
        });
    }
    Ok(LogPage { total, rows: out })
}

/// 按窝按月的记录摘要行（交互第三轮 #8）：日历标记与重复提醒的数据源。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MonthDayRecords {
    pub day: i64,
    pub action_id: i64,
    pub count: i64,
    /// 该 (日, 操作) 最近一条的发生时刻（"YYYY-MM-DD HH:MM:SS"）
    pub last_time: String,
}

/// 某窝某月每天的逐操作计数（日历橙点/灰点、黄条「已有 N 条（HH:MM）」用）。
/// month 取 1–12；occurred_at 落在 [当月1日, 次月1日) 字典序区间内（库内格式定长，字典序即时间序）。
/// `exclude_log_id`：编辑场景传当前记录 id——正在编辑的这条不计入，防「只改备注也误报重复」。
pub fn colony_month_records(
    conn: &Connection,
    colony_id: i64,
    year: i64,
    month: i64,
    exclude_log_id: Option<i64>,
) -> Result<Vec<MonthDayRecords>, String> {
    if !(1..=12).contains(&month) {
        return Err(format!("月份应在 1–12：{year}-{month}"));
    }
    let (next_y, next_m) = if month == 12 { (year + 1, 1) } else { (year, month + 1) };
    let range_start = format!("{year:04}-{month:02}-01 00:00:00");
    let range_end = format!("{next_y:04}-{next_m:02}-01 00:00:00");

    // 动态 WHERE 照抄 list_logs 的 wheres/args 模式：固定三段 + 可选「排除自身」
    let mut wheres: Vec<String> = vec![
        "l.colony_id = ?1".into(),
        "l.occurred_at >= ?2".into(),
        "l.occurred_at < ?3".into(),
    ];
    let mut args: Vec<rusqlite::types::Value> =
        vec![colony_id.into(), range_start.into(), range_end.into()];
    if let Some(exclude) = exclude_log_id {
        args.push(exclude.into());
        wheres.push(format!("l.id != ?{}", args.len()));
    }

    let mut stmt = conn
        .prepare(&format!(
            "SELECT CAST(substr(l.occurred_at, 9, 2) AS INTEGER), l.action_id, COUNT(*), MAX(l.occurred_at)
             FROM care_log l
             WHERE {}
             GROUP BY substr(l.occurred_at, 9, 2), l.action_id
             ORDER BY substr(l.occurred_at, 9, 2), l.action_id",
            wheres.join(" AND ")
        ))
        .map_err(db_err)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(args.iter()), |row| {
            Ok(MonthDayRecords {
                day: row.get(0)?,
                action_id: row.get(1)?,
                count: row.get(2)?,
                last_time: row.get(3)?,
            })
        })
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?;
    Ok(rows)
}

/// 编辑一条记录（spec API 契约 updateLog）：走完整校验——时间规整 + 不许未来
/// （票 04 停靠②同口径）、is_feeding 位约束、字典引用规则——并与 log_food 关联
/// 同事务重写，任一失败整体不动。规则 10 裁定：该记录**原引用**的停用操作/食物
/// 可以原样保留；**新挂**的停用项一律拒绝。
pub fn update_log(conn: &Connection, id: i64, input: &LogUpdateInput, now: &str) -> Result<(), String> {
    let (cur_action_id, cur_occurred_at, cur_note): (i64, String, String) = conn
        .query_row(
            "SELECT action_id, occurred_at, note FROM care_log WHERE id = ?1",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => "记录不存在".to_string(),
            other => db_err(other),
        })?;

    // 目标操作：None = 保持原操作（停用中的原操作允许原样保留）；换了就必须启用
    let new_action_id = input.action_id.unwrap_or(cur_action_id);
    let (action_enabled, is_feeding, action_name): (i64, i64, String) = conn
        .query_row(
            "SELECT enabled, is_feeding, name FROM care_action WHERE id = ?1",
            params![new_action_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => "操作不存在".to_string(),
            other => db_err(other),
        })?;
    if new_action_id != cur_action_id && action_enabled == 0 {
        return Err(format!("操作「{action_name}」已停用，不能改挂"));
    }

    // 发生时间：None = 保持（库内已是规整格式）；给了就完整校验
    let occurred_at = match &input.occurred_at {
        Some(s) => {
            let normalized = normalize_happened_at(s)?;
            ensure_not_future(&normalized, now)?;
            normalized
        }
        None => cur_occurred_at,
    };
    let note = match &input.note {
        Some(s) => s.trim().to_string(),
        None => cur_note,
    };

    // 原引用食物 = 保留通道；food_ids: None = 原样保留，Some(空) = 清空
    let original_foods: HashSet<i64> = {
        let mut stmt = conn
            .prepare("SELECT food_id FROM log_food WHERE log_id = ?1")
            .map_err(db_err)?;
        let rows = stmt
            .query_map(params![id], |row| row.get::<_, i64>(0))
            .map_err(db_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db_err)?;
        rows.into_iter().collect()
    };
    let food_ids: Vec<i64> = match &input.food_ids {
        Some(list) => {
            let mut seen = HashSet::new();
            list.iter().filter(|fid| seen.insert(**fid)).copied().collect()
        }
        None => original_foods.iter().copied().collect(),
    };

    // is_feeding 位约束：非喂食不允许带食物（换到非喂食必须显式清空）
    if is_feeding == 0 && !food_ids.is_empty() {
        return Err(format!("操作「{action_name}」不是喂食，不能关联食物"));
    }

    // 食物：存在；启用；停用仅当为原引用（保留通道），新挂停用项拒绝
    for fid in &food_ids {
        let row: Option<(i64, String)> = conn
            .query_row(
                "SELECT enabled, name FROM food WHERE id = ?1",
                params![fid],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .ok();
        match row {
            None => return Err(format!("食物不存在（id={fid}）")),
            Some((0, name)) if !original_foods.contains(fid) => {
                return Err(format!("食物「{name}」已停用，不能新选"));
            }
            _ => {}
        }
    }

    let tx = conn.unchecked_transaction().map_err(db_err)?;
    tx.execute(
        "UPDATE care_log SET action_id = ?1, occurred_at = ?2, note = ?3 WHERE id = ?4",
        params![new_action_id, occurred_at, note, id],
    )
    .map_err(db_err)?;
    tx.execute("DELETE FROM log_food WHERE log_id = ?1", params![id]).map_err(db_err)?;
    for fid in &food_ids {
        tx.execute(
            "INSERT INTO log_food (log_id, food_id) VALUES (?1, ?2)",
            params![id, fid],
        )
        .map_err(db_err)?;
    }
    tx.commit().map_err(db_err)?;
    Ok(())
}

/// 删除一条记录（spec API 契约 deleteLog）：log_food 子行与主记录同事务删除。
pub fn delete_log(conn: &Connection, id: i64) -> Result<(), String> {
    let exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM care_log WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )
        .map_err(db_err)?;
    if exists == 0 {
        return Err("记录不存在".into());
    }
    let tx = conn.unchecked_transaction().map_err(db_err)?;
    tx.execute("DELETE FROM log_food WHERE log_id = ?1", params![id]).map_err(db_err)?;
    tx.execute("DELETE FROM care_log WHERE id = ?1", params![id]).map_err(db_err)?;
    tx.commit().map_err(db_err)?;
    Ok(())
}

// ── 卡片展示数据 ─────────────────────────────────────────────────────────

/// 某窝每个「启用中」操作一块，按 sort、id 排序（字典新增操作自动出现）。
/// 「距上次」基线（规则 5）：有出眠史的窝取 max(最近一次记录日期, 最近出眠日期)，
/// 出眠当天全窝 0 天、不会一睁眼全红；登记类显示同样基准（只影响文案，永不红的性质不变）。
/// 喂食块（F3）另带逐食物明细：基线同口径（max(该食物最近喂食, 出眠日)），
/// 任一设周期食物超期整块红（Q2）。
pub fn tiles_for_colony(
    conn: &Connection,
    colony_id: i64,
    today: &str,
) -> Result<Vec<ActionTile>, String> {
    let wake = crate::hibernation::latest_wake_date(conn, colony_id);
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

    // 启用中的食物（含周期，F3 食物层用）；全动作共用，取一次
    let food_rows: Vec<(i64, String, Option<i64>)> = {
        let mut stmt = conn
            .prepare(
                "SELECT id, name, suggested_interval_days FROM food WHERE enabled = 1 ORDER BY sort, id",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                ))
            })
            .map_err(db_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db_err)?;
        rows
    };

    let mut tiles = Vec::with_capacity(rows.len());
    for (action_id, name, icon, kind, is_feeding, interval) in rows {
        // 逐行取 occurred_at、跳过解析失败的单条（票 04 停靠①：一条脏行不得毒死整页），
        // 取剩余行里的最近日期；无可用行视同从未记录。
        let occurred_rows: Vec<String> = {
            let mut stmt = conn
                .prepare("SELECT occurred_at FROM care_log WHERE colony_id = ?1 AND action_id = ?2")
                .map_err(db_err)?;
            let rows = stmt
                .query_map(params![colony_id, action_id], |row| row.get(0))
                .map_err(db_err)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(db_err)?;
            rows
        };
        let mut latest: Option<chrono::NaiveDate> = None;
        for occurred in &occurred_rows {
            if let Ok(d) = chrono::NaiveDate::parse_from_str(occurred.get(0..10).unwrap_or(""), "%Y-%m-%d") {
                latest = Some(match latest {
                    Some(prev) if prev >= d => prev,
                    _ => d,
                });
            }
        }
        // 规则 5：出眠后基准 = max(最近一次记录, 出眠日期)
        let base = match (latest, wake) {
            (Some(log), Some(w)) => Some(log.max(w)),
            (log, w) => log.or(w),
        };
        let last = base.map(|d| d.format("%Y-%m-%d").to_string());
        let days = days_since_last(last.as_deref(), today)?;

        // F3 食物层：喂食类逐食物算「距上次该食物」，基线与操作层同口径（含出眠重置）；
        // 脏行同样逐食物跳过；食物超期仅提醒类且已设周期时判定
        let mut foods = Vec::with_capacity(if is_feeding != 0 { food_rows.len() } else { 0 });
        if is_feeding != 0 {
            for (food_id, food_name, food_interval) in &food_rows {
                let occurred: Vec<String> = {
                    let mut stmt = conn
                        .prepare(
                            "SELECT l.occurred_at FROM log_food lf
                             JOIN care_log l ON l.id = lf.log_id
                             JOIN care_action a ON a.id = l.action_id
                             WHERE lf.food_id = ?1 AND l.colony_id = ?2 AND a.is_feeding = 1",
                        )
                        .map_err(db_err)?;
                    let rows = stmt
                        .query_map(params![food_id, colony_id], |row| row.get(0))
                        .map_err(db_err)?
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(db_err)?;
                    rows
                };
                let food_latest = occurred
                    .iter()
                    .filter_map(|s| chrono::NaiveDate::parse_from_str(s.get(0..10).unwrap_or(""), "%Y-%m-%d").ok())
                    .max();
                let food_base = match (food_latest, wake) {
                    (Some(log), Some(w)) => Some(log.max(w)),
                    (log, w) => log.or(w),
                };
                let food_days = days_since_last(
                    food_base.map(|d| d.format("%Y-%m-%d").to_string()).as_deref(),
                    today,
                )?;
                let food_overdue = if kind == "reminding" {
                    is_overdue("reminding", food_days, *food_interval)
                } else {
                    false
                };
                foods.push(FoodTileStatus {
                    food_id: *food_id,
                    name: food_name.clone(),
                    suggested_interval_days: *food_interval,
                    days_since_last: food_days,
                    overdue: food_overdue,
                });
            }
        }
        let food_any = foods.iter().any(|f| f.overdue);
        let overdue = is_overdue(&kind, days, interval) || food_any;
        tiles.push(ActionTile {
            action_id,
            name,
            icon,
            kind,
            is_feeding: is_feeding != 0,
            suggested_interval_days: interval,
            days_since_last: days,
            overdue,
            foods,
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

const FOOD_SQL: &str = concat!(
    "SELECT f.id, f.name, f.enabled, f.sort, f.suggested_interval_days, f.is_preset, ",
    "(EXISTS(SELECT 1 FROM log_food lf WHERE lf.food_id = f.id) ",
    "OR EXISTS(SELECT 1 FROM reminder_ledger g WHERE g.food_id = f.id)) ",
    "FROM food f ",
);

fn row_to_food(row: &rusqlite::Row<'_>) -> rusqlite::Result<Food> {
    Ok(Food {
        id: row.get(0)?,
        name: row.get(1)?,
        enabled: row.get::<_, i64>(2)? != 0,
        sort: row.get(3)?,
        suggested_interval_days: row.get(4)?,
        is_preset: row.get::<_, i64>(5)? != 0,
        referenced: row.get::<_, i64>(6)? != 0,
    })
}

/// 单个食物（dict::save_food 保存后回读用）。
pub fn get_food(conn: &Connection, id: i64) -> Result<Food, String> {
    conn.query_row(
        &format!("{FOOD_SQL} WHERE f.id = ?1"),
        params![id],
        row_to_food,
    )
    .map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => "食物不存在".to_string(),
        other => db_err(other),
    })
}

/// 全部食物（含停用的，与 list_locations 同口径；新建入口由前端过滤 enabled）。
/// `referenced` = 被 log_food 或 reminder_ledger（food 维度，F3）引用：
/// 删除会被拒，只能停用（规则 10）。
pub fn list_foods(conn: &Connection) -> Result<Vec<Food>, String> {
    let mut stmt = conn
        .prepare(&format!("{FOOD_SQL} ORDER BY f.sort, f.id"))
        .map_err(db_err)?;
    let rows = stmt
        .query_map([], row_to_food)
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

    #[test]
    fn ensure_not_future_rejects_later_than_now_and_allows_same_or_earlier() {
        // 库内统一 `YYYY-MM-DD HH:MM:SS`，字典序 = 时间序（票 04 停靠②：拒绝未来时间）
        assert!(ensure_not_future("2026-09-18 08:00:01", "2026-09-18 08:00:00").is_err());
        assert!(ensure_not_future("2026-09-19 00:00:00", "2026-09-18 08:00:00").is_err());
        assert!(ensure_not_future("2026-09-18 08:00:00", "2026-09-18 08:00:00").is_ok(), "同刻允许");
        assert!(ensure_not_future("2026-09-17 23:59:59", "2026-09-18 08:00:00").is_ok());
    }

    #[test]
    fn log_care_rejects_future_happened_at_with_friendly_error() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        let feed = action_id(&conn, "喂食");

        let err = log_care(
            &conn,
            &CareLogInput {
                colony_id: c,
                action_id: feed,
                happened_at: "2026-09-19T09:00".into(),
                note: None,
                food_ids: vec![],
            },
            "2026-09-18 08:00:00",
        )
        .unwrap_err();
        assert!(err.contains("未来"), "实际错误：{err}");
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM care_log"), 0, "被拒的记账不落库");

        // 同刻与过去照常可记
        assert!(log_care(
            &conn,
            &CareLogInput {
                colony_id: c,
                action_id: feed,
                happened_at: "2026-09-18 08:00:00".into(),
                note: None,
                food_ids: vec![],
            },
            "2026-09-18 08:00:00",
        )
        .is_ok());
        assert!(log_care(
            &conn,
            &CareLogInput {
                colony_id: c,
                action_id: feed,
                happened_at: "2026-09-18T07:00".into(),
                note: None,
                food_ids: vec![],
            },
            "2026-09-18 08:00:00",
        )
        .is_ok());
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
            // 早于 now（08:00）：未来发生时间会被另行拒绝，这里只测字典引用校验
            happened_at: "2026-09-18T07:00".into(),
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
                happened_at: "2026-09-18T07:00".into(),
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
                happened_at: "2026-09-18T07:00".into(),
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
                happened_at: "2026-09-18T07:00".into(),
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
                happened_at: "2026-09-18T07:00".into(),
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
    fn tiles_skip_single_anomalous_occurred_at_instead_of_poisoning_the_page() {
        // 票 04 停靠①：单条异常 occurred_at 跳过该行，不让 ? 传播毒死整页
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        log(&conn, c, "喂食", "2026-09-15 20:00:00"); // 3 天前
        conn.execute(
            "INSERT INTO care_log (colony_id, action_id, occurred_at, note, created_at)
             VALUES (?1, ?2, 'garbage-time', '', '2026-09-18 08:00:00')",
            params![c, action_id(&conn, "喂食")],
        )
        .unwrap();

        let tiles = tiles_for_colony(&conn, c, TODAY).unwrap();
        assert_eq!(
            tile(&tiles, "喂食").days_since_last,
            Some(3),
            "异常行被跳过，距上次取其余行里的最近一条"
        );

        // 全部异常 → 视同从未记录，也不 Err
        let c2 = colony(&conn, "倒霉二号");
        conn.execute(
            "INSERT INTO care_log (colony_id, action_id, occurred_at, note, created_at)
             VALUES (?1, ?2, '???', '', '2026-09-18 08:00:00')",
            params![c2, action_id(&conn, "喂食")],
        )
        .unwrap();
        let tiles = tiles_for_colony(&conn, c2, TODAY).unwrap();
        assert_eq!(tile(&tiles, "喂食").days_since_last, None);
        assert!(!tile(&tiles, "喂食").overdue);
    }

    #[test]
    fn tiles_baseline_resets_to_wake_date_on_wake_day() {
        // 验收 3：出眠当天显示"距上次 0 天"——基准 = max(最近一次记录, 出眠日期)，
        // 不会一睁眼全红。入眠前 10 天喂过一次，冬眠期间没记账，今天（09-18）出眠。
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        log(&conn, c, "喂食", "2026-09-08 20:00:00");
        conn.execute(
            "INSERT INTO hibernation (colony_id, start_date, expected_end_date, actual_end_date)
             VALUES (?1, '2026-09-10', '2026-10-01', '2026-09-18')",
            params![c],
        )
        .unwrap();

        let tiles = tiles_for_colony(&conn, c, TODAY).unwrap();
        assert_eq!(
            tile(&tiles, "喂食").days_since_last,
            Some(0),
            "出眠当天基线 = 出眠日"
        );
        assert!(!tile(&tiles, "喂食").overdue, "出眠当天不超期");
        assert_eq!(
            tile(&tiles, "垃圾清理").days_since_last,
            Some(0),
            "从未记录的操作同样从出眠日起算（全窝同口径）"
        );
    }

    #[test]
    fn tiles_baseline_prefers_latest_log_when_newer_than_wake() {
        // 出眠 17 天后昨天刚喂过：喂食用记录日（1 天），没记过的操作仍从出眠日起算
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        conn.execute(
            "INSERT INTO hibernation (colony_id, start_date, expected_end_date, actual_end_date)
             VALUES (?1, '2026-08-01', '2026-09-01', '2026-09-01')",
            params![c],
        )
        .unwrap();
        log(&conn, c, "喂食", "2026-09-16 20:00:00");

        let tiles = tiles_for_colony(&conn, c, TODAY).unwrap();
        assert_eq!(
            tile(&tiles, "喂食").days_since_last,
            Some(2),
            "最近记录比出眠日新 → 用记录日"
        );
        assert_eq!(
            tile(&tiles, "垃圾清理").days_since_last,
            Some(17),
            "出眠后没记过 → 从出眠日起算"
        );
    }

    #[test]
    fn tiles_baseline_ignores_open_segment_and_dirty_wake_dates() {
        // 开放段没有实际结束日期：不改基线（出眠基线只看闭合段）
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        log(&conn, c, "喂食", "2026-09-14 20:00:00"); // 4 天前
        conn.execute(
            "INSERT INTO hibernation (colony_id, start_date, expected_end_date)
             VALUES (?1, '2026-09-10', '2026-10-01')",
            params![c],
        )
        .unwrap();
        let tiles = tiles_for_colony(&conn, c, TODAY).unwrap();
        assert_eq!(tile(&tiles, "喂食").days_since_last, Some(4), "开放段不改基线");

        // 脏数据：actual_end_date 非法 → 跳过，不毒死整页（票 04 停靠①口径）
        let c2 = colony(&conn, "倒霉二号");
        log(&conn, c2, "喂食", "2026-09-14 20:00:00");
        conn.execute(
            "INSERT INTO hibernation (colony_id, start_date, expected_end_date, actual_end_date)
             VALUES (?1, '2026-08-01', '2026-09-01', '???')",
            params![c2],
        )
        .unwrap();
        let tiles = tiles_for_colony(&conn, c2, TODAY).unwrap();
        assert_eq!(tile(&tiles, "喂食").days_since_last, Some(4), "脏出眠日被跳过");
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
        assert!(!foods.iter().find(|f| f.name == "面包虫").unwrap().enabled);
    }

    #[test]
    fn list_foods_flags_referenced_by_log_food() {
        // 票 04：referenced 供设置页禁用「删除」按钮（被引用只能停用，规则 10）
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        log_care(
            &conn,
            &CareLogInput {
                colony_id: c,
                action_id: action_id(&conn, "喂食"),
                happened_at: "2026-09-17 20:00:00".into(),
                note: None,
                food_ids: vec![food_id(&conn, "种子")],
            },
            "2026-09-18 08:00:00",
        )
        .unwrap();

        let foods = list_foods(&conn).unwrap();
        assert!(foods.iter().find(|f| f.name == "种子").unwrap().referenced);
        assert!(!foods.iter().find(|f| f.name == "干虾仁").unwrap().referenced);
    }

    #[test]
    fn list_foods_flags_referenced_by_reminder_ledger_food_dimension() {
        // F3：food_overdue 台账行也是引用——否则从未喂过的食物（被补发过提醒）
        // 会显示可删，DELETE 撞 reminder_ledger.food_id 外键报原始错误
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        conn.execute(
            "INSERT INTO reminder_ledger (colony_id, kind, action_id, food_id, base_date, sent_at)
             VALUES (?1, 'food_overdue', ?2, ?3, '2026-09-11', '2026-09-18 08:00:00')",
            params![c, action_id(&conn, "喂食"), food_id(&conn, "面包虫")],
        )
        .unwrap();

        let foods = list_foods(&conn).unwrap();
        assert!(foods.iter().find(|f| f.name == "面包虫").unwrap().referenced);
        assert!(!foods.iter().find(|f| f.name == "干虾仁").unwrap().referenced);
    }

    // ── 记录列表 / 编辑 / 删除（票 08）──

    /// 带备注与食物的喂食记录（搭列表/编辑场景用）。
    fn feed_log(
        conn: &Connection,
        colony_id: i64,
        happened_at: &str,
        note: Option<&str>,
        foods: &[&str],
    ) -> i64 {
        log_care(
            conn,
            &CareLogInput {
                colony_id,
                action_id: action_id(conn, "喂食"),
                happened_at: happened_at.into(),
                note: note.map(String::from),
                food_ids: foods.iter().map(|f| food_id(conn, f)).collect(),
            },
            "2026-09-18 08:00:00",
        )
        .expect("记账失败")
    }

    #[test]
    fn list_logs_orders_desc_and_each_filter_applies_alone() {
        let conn = mem_conn();
        let c1 = colony(&conn, "大头一号");
        let c2 = colony(&conn, "针毛一号");
        let feed_latest = feed_log(&conn, c1, "2026-09-17 20:00:00", Some("换了水盆"), &["种子"]);
        let water = log(&conn, c1, "活动区换水", "2026-09-10 09:00:00");
        let feed_old = feed_log(&conn, c1, "2026-09-05 08:00:00", Some("加餐面包虫"), &["面包虫"]);
        let c2_feed = log(&conn, c2, "喂食", "2026-09-16 21:00:00");

        // 无筛选：total 全量、occurred_at DESC（同刻再按 id DESC）
        let all = list_logs(&conn, &LogFilter::default()).unwrap();
        assert_eq!(all.total, 4);
        let occurred: Vec<&str> = all.rows.iter().map(|r| r.occurred_at.as_str()).collect();
        assert_eq!(
            occurred,
            [
                "2026-09-17 20:00:00",
                "2026-09-16 21:00:00",
                "2026-09-10 09:00:00",
                "2026-09-05 08:00:00",
            ]
        );
        let first = &all.rows[0];
        assert_eq!(first.id, feed_latest);
        assert_eq!(first.colony_id, c1);
        assert_eq!(first.colony_name, "大头一号");
        assert_eq!(first.action_id, action_id(&conn, "喂食"));
        assert_eq!(first.action_name, "喂食");
        assert_eq!(first.note, "换了水盆");
        assert_eq!(first.food_names, vec!["种子"]);
        assert_eq!(first.food_ids, vec![food_id(&conn, "种子")]);

        // 窝筛选
        let only_c2 = list_logs(&conn, &LogFilter { colony_id: Some(c2), ..Default::default() }).unwrap();
        assert_eq!(only_c2.total, 1);
        assert_eq!(only_c2.rows[0].id, c2_feed);

        // 操作筛选
        let water_only = list_logs(
            &conn,
            &LogFilter { action_id: Some(action_id(&conn, "活动区换水")), ..Default::default() },
        )
        .unwrap();
        assert_eq!(water_only.total, 1);
        assert_eq!(water_only.rows[0].id, water);
        assert!(water_only.rows[0].food_names.is_empty(), "非喂食记录无食物");

        // 时间范围：按 occurred_at 日期部分闭区间
        let ranged = list_logs(
            &conn,
            &LogFilter {
                start: Some("2026-09-10".into()),
                end: Some("2026-09-16".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(ranged.total, 2);
        let ids: Vec<i64> = ranged.rows.iter().map(|r| r.id).collect();
        assert_eq!(ids, vec![c2_feed, water]);

        // 只给一端
        let from_10 = list_logs(&conn, &LogFilter { start: Some("2026-09-10".into()), ..Default::default() }).unwrap();
        assert_eq!(from_10.total, 3);
        let to_16 = list_logs(&conn, &LogFilter { end: Some("2026-09-16".into()), ..Default::default() }).unwrap();
        assert_eq!(to_16.total, 3);

        // 备注关键词（子串）
        let kw = list_logs(
            &conn,
            &LogFilter { note_keyword: Some("面包虫".into()), ..Default::default() },
        )
        .unwrap();
        assert_eq!(kw.total, 1);
        assert_eq!(kw.rows[0].id, feed_old);
    }

    #[test]
    fn list_logs_combined_filters_narrow_together() {
        // 验收 1：三条件组合 + 备注搜索，全部 AND 叠加
        let conn = mem_conn();
        let c1 = colony(&conn, "大头一号");
        let c2 = colony(&conn, "针毛一号");
        feed_log(&conn, c1, "2026-09-17 20:00:00", Some("换了水盆"), &["种子"]);
        let target = feed_log(&conn, c1, "2026-09-05 08:00:00", Some("加餐面包虫"), &["面包虫"]);
        log(&conn, c1, "活动区换水", "2026-09-10 09:00:00");
        feed_log(&conn, c2, "2026-09-06 08:00:00", Some("面包虫大餐"), &["面包虫"]);

        let combined = list_logs(
            &conn,
            &LogFilter {
                colony_id: Some(c1),
                action_id: Some(action_id(&conn, "喂食")),
                start: Some("2026-09-01".into()),
                end: Some("2026-09-18".into()),
                note_keyword: Some("面包虫".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(combined.total, 1, "窝+操作+范围+关键词一起收口");
        assert_eq!(combined.rows[0].id, target);
        // 范围之外的关键词命中被组合条件排除（换水 09-10 不在喂食筛选里）
        assert!(combined.rows.iter().all(|r| r.action_name == "喂食"));
    }

    #[test]
    fn list_logs_keyword_escapes_like_wildcards() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        let underscore = feed_log(&conn, c, "2026-09-10 08:00:00", Some("配方 a_b 升级"), &[]);
        let _similar = feed_log(&conn, c, "2026-09-11 08:00:00", Some("配方 axb 试运行"), &[]);
        let pct = feed_log(&conn, c, "2026-09-12 08:00:00", Some("剩余 50% 量"), &[]);
        let _no_pct = feed_log(&conn, c, "2026-09-13 08:00:00", Some("投喂 50 只面包虫"), &[]);

        // `_`/`%` 按字面匹配：不转义时 LIKE 会把 axb / 50 只 也捞进来
        let kw1 = list_logs(&conn, &LogFilter { note_keyword: Some("a_b".into()), ..Default::default() }).unwrap();
        assert_eq!(kw1.total, 1);
        assert_eq!(kw1.rows[0].id, underscore);

        let kw2 = list_logs(&conn, &LogFilter { note_keyword: Some("50%".into()), ..Default::default() }).unwrap();
        assert_eq!(kw2.total, 1);
        assert_eq!(kw2.rows[0].id, pct);
    }

    #[test]
    fn list_logs_paginates_with_stable_total() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        for day in 10..=17 {
            log(&conn, c, "喂食", &format!("2026-09-{day} 08:00:00"));
        }

        let page1 = list_logs(
            &conn,
            &LogFilter { limit: Some(3), offset: Some(0), ..Default::default() },
        )
        .unwrap();
        assert_eq!(page1.total, 8, "total 恒为命中总数，不随分页变");
        assert_eq!(page1.rows.len(), 3);
        assert_eq!(page1.rows[0].occurred_at, "2026-09-17 08:00:00");

        let page2 = list_logs(
            &conn,
            &LogFilter { limit: Some(3), offset: Some(3), ..Default::default() },
        )
        .unwrap();
        assert_eq!(page2.rows.len(), 3);
        assert_ne!(page1.rows[0].id, page2.rows[0].id);

        let tail = list_logs(
            &conn,
            &LogFilter { limit: Some(3), offset: Some(6), ..Default::default() },
        )
        .unwrap();
        assert_eq!(tail.rows.len(), 2, "不足一页给余量");

        let past_end = list_logs(
            &conn,
            &LogFilter { limit: Some(3), offset: Some(99), ..Default::default() },
        )
        .unwrap();
        assert_eq!(past_end.rows.len(), 0);
        assert_eq!(past_end.total, 8);
    }

    #[test]
    fn list_logs_keeps_disabled_dict_display_names() {
        // 规则 10：停用操作/食物在历史记录里照常显示（显示名仍在）
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        let id = feed_log(&conn, c, "2026-09-15 08:00:00", None, &["种子"]);
        conn.execute("UPDATE care_action SET enabled = 0 WHERE name = '喂食'", []).unwrap();
        conn.execute("UPDATE food SET enabled = 0 WHERE name = '种子'", []).unwrap();

        let page = list_logs(&conn, &LogFilter::default()).unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.rows[0].id, id);
        assert_eq!(page.rows[0].action_name, "喂食");
        assert_eq!(page.rows[0].food_names, vec!["种子"]);
    }

    // ── 地点筛选 + 行带地点名（交互第三轮 #1/#5）──
    // （种子依据：db.rs seeds 预置 '家'/'公司'，loc_id 按名取不会踩空）

    fn colony_in_loc(conn: &Connection, name: &str, loc: Option<i64>) -> i64 {
        conn.execute(
            "INSERT INTO colony (name, location_id, start_date, status) VALUES (?1, ?2, '2026-01-01', 'active')",
            params![name, loc],
        ).expect("建窝失败");
        conn.last_insert_rowid()
    }
    fn loc_id(conn: &Connection, name: &str) -> i64 {
        conn.query_row("SELECT id FROM location WHERE name = ?1", params![name], |r| r.get(0)).expect("查地点失败")
    }

    #[test]
    fn list_logs_filters_by_location_and_joins_location_name() {
        let conn = mem_conn();
        let home = loc_id(&conn, "家");
        let c1 = colony_in_loc(&conn, "大头一号", Some(home));
        let c2 = colony_in_loc(&conn, "游民", None);
        log(&conn, c1, "喂食", "2026-09-17 20:00:00");
        log(&conn, c2, "喂食", "2026-09-16 20:00:00");

        let page = list_logs(&conn, &LogFilter { location_id: Some(home), ..Default::default() }).unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.rows[0].colony_name, "大头一号");
        assert_eq!(page.rows[0].location_name.as_deref(), Some("家"));

        // 未分组的窝：location_name = None；按「全部」查两行都在
        let all = list_logs(&conn, &LogFilter::default()).unwrap();
        assert_eq!(all.total, 2);
        let nomad = all.rows.iter().find(|r| r.colony_name == "游民").unwrap();
        assert_eq!(nomad.location_name, None);
    }

    #[test]
    fn list_logs_location_and_colony_filters_compose() {
        let conn = mem_conn();
        let home = loc_id(&conn, "家");
        let c1 = colony_in_loc(&conn, "家A", Some(home));
        let c2 = colony_in_loc(&conn, "家B", Some(home));
        log(&conn, c1, "喂食", "2026-09-17 20:00:00");
        log(&conn, c2, "喂食", "2026-09-16 20:00:00");

        let page = list_logs(&conn, &LogFilter {
            location_id: Some(home),
            colony_id: Some(c1),
            ..Default::default()
        }).unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.rows[0].colony_name, "家A");
    }

    #[test]
    fn update_log_swaps_foods_and_edits_fields_in_place() {
        // 验收 2：编辑喂食记录更换食物生效（log_food 关联同事务重写）
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        let id = feed_log(&conn, c, "2026-09-17 08:00:00", Some("  原备注  "), &["种子"]);

        update_log(
            &conn,
            id,
            &LogUpdateInput {
                occurred_at: Some("2026-09-10T08:30".into()),
                note: Some("  改投干虾仁和面包虫  ".into()),
                action_id: None,
                food_ids: Some(vec![food_id(&conn, "干虾仁"), food_id(&conn, "面包虫")]),
            },
            "2026-09-18 08:00:00",
        )
        .unwrap();

        let (action, occurred, note): (i64, String, String) = conn
            .query_row(
                "SELECT action_id, occurred_at, note FROM care_log WHERE id = ?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(action, action_id(&conn, "喂食"), "action_id None = 保持");
        assert_eq!(occurred, "2026-09-10 08:30:00", "发生时间规整后落库");
        assert_eq!(note, "改投干虾仁和面包虫", "备注 trim");

        let links: Vec<i64> = {
            let mut stmt = conn
                .prepare("SELECT food_id FROM log_food WHERE log_id = ?1 ORDER BY food_id")
                .unwrap();
            stmt.query_map(params![id], |r| r.get(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        assert_eq!(
            links,
            vec![food_id(&conn, "干虾仁"), food_id(&conn, "面包虫")],
            "旧关联删净、新关联写入（验收 2）"
        );

        // 列表读回也是新食物
        let page = list_logs(&conn, &LogFilter::default()).unwrap();
        assert_eq!(page.rows[0].food_names, vec!["干虾仁", "面包虫"]);
    }

    #[test]
    fn update_log_keeps_original_disabled_refs_and_rejects_new_ones() {
        // 规则 10 裁定：编辑时允许保存原引用的停用操作/食物，新挂停用项拒绝
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        let seed = food_id(&conn, "种子");
        let mealworm = food_id(&conn, "面包虫");
        let feed = action_id(&conn, "喂食");
        let water = action_id(&conn, "活动区换水");
        let id = feed_log(&conn, c, "2026-09-15 08:00:00", None, &["种子"]);

        conn.execute(
            "UPDATE food SET enabled = 0 WHERE id IN (?1, ?2)",
            params![seed, mealworm],
        )
        .unwrap();
        conn.execute(
            "UPDATE care_action SET enabled = 0 WHERE id IN (?1, ?2)",
            params![feed, water],
        )
        .unwrap();

        // ① 原引用的停用食物原样保留 → OK
        update_log(
            &conn,
            id,
            &LogUpdateInput {
                occurred_at: None,
                note: None,
                action_id: None,
                food_ids: Some(vec![seed]),
            },
            "2026-09-18 08:00:00",
        )
        .unwrap();
        assert_eq!(
            count(&conn, &format!("SELECT COUNT(*) FROM log_food WHERE log_id = {id}")),
            1
        );

        // ② 新挂停用食物 → 拒
        let err = update_log(
            &conn,
            id,
            &LogUpdateInput {
                occurred_at: None,
                note: None,
                action_id: None,
                food_ids: Some(vec![mealworm]),
            },
            "2026-09-18 08:00:00",
        )
        .unwrap_err();
        assert!(err.contains("停用"), "实际错误：{err}");

        // ③ 原操作（已停用）保持不动 → OK（food_ids 传空数组 = 清空食物）
        update_log(
            &conn,
            id,
            &LogUpdateInput {
                occurred_at: None,
                note: Some("改备注".into()),
                action_id: Some(feed),
                food_ids: Some(vec![]),
            },
            "2026-09-18 08:00:00",
        )
        .unwrap();

        // ④ 新挂停用操作 → 拒
        let err = update_log(
            &conn,
            id,
            &LogUpdateInput {
                occurred_at: None,
                note: None,
                action_id: Some(water),
                food_ids: None,
            },
            "2026-09-18 08:00:00",
        )
        .unwrap_err();
        assert!(err.contains("停用"), "实际错误：{err}");

        // ⑤ 启用后新挂 → OK
        conn.execute("UPDATE care_action SET enabled = 1 WHERE id = ?1", params![water]).unwrap();
        update_log(
            &conn,
            id,
            &LogUpdateInput {
                occurred_at: None,
                note: None,
                action_id: Some(water),
                food_ids: Some(vec![]),
            },
            "2026-09-18 08:00:00",
        )
        .unwrap();
        let action: i64 = conn
            .query_row("SELECT action_id FROM care_log WHERE id = ?1", params![id], |r| r.get(0))
            .unwrap();
        assert_eq!(action, water);
    }

    #[test]
    fn update_log_full_validation_rejects_and_leaves_row_untouched() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        let seed = food_id(&conn, "种子");
        let feed = action_id(&conn, "喂食");
        let water = action_id(&conn, "活动区换水");
        let id = feed_log(&conn, c, "2026-09-17 08:00:00", None, &["种子"]);

        let input = |occurred: Option<String>, action: Option<i64>, foods: Option<Vec<i64>>| {
            LogUpdateInput { occurred_at: occurred, note: None, action_id: action, food_ids: foods }
        };

        // 未来发生时间（票 04 停靠②同口径：编辑也不许预记未来）
        let err = update_log(
            &conn,
            id,
            &input(Some("2026-09-19T09:00".into()), None, None),
            "2026-09-18 08:00:00",
        )
        .unwrap_err();
        assert!(err.contains("未来"), "实际错误：{err}");

        // 非喂食操作带食物 → 拒
        let err = update_log(
            &conn,
            id,
            &input(None, Some(water), Some(vec![seed])),
            "2026-09-18 08:00:00",
        )
        .unwrap_err();
        assert!(err.contains("食物"), "实际错误：{err}");

        // 部分更新踩到 is_feeding 约束：原记录带食物、改成非喂食又不给 food_ids → 拒
        let err = update_log(
            &conn,
            id,
            &input(None, Some(water), None),
            "2026-09-18 08:00:00",
        )
        .unwrap_err();
        assert!(err.contains("食物"), "实际错误：{err}");

        // 不存在的食物
        let err = update_log(
            &conn,
            id,
            &input(None, None, Some(vec![999])),
            "2026-09-18 08:00:00",
        )
        .unwrap_err();
        assert!(err.contains("不存在"), "实际错误：{err}");

        // 不存在的记录
        let err = update_log(
            &conn,
            999,
            &input(Some("2026-09-10 08:00:00".into()), None, None),
            "2026-09-18 08:00:00",
        )
        .unwrap_err();
        assert!(err.contains("记录不存在"), "实际错误：{err}");

        // 一连串失败后原记录原样（校验全部在事务前，无部分落库）
        let (action, occurred): (i64, String) = conn
            .query_row(
                "SELECT action_id, occurred_at FROM care_log WHERE id = ?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(action, feed);
        assert_eq!(occurred, "2026-09-17 08:00:00");
        assert_eq!(count(&conn, &format!("SELECT COUNT(*) FROM log_food WHERE log_id = {id}")), 1);
    }

    #[test]
    fn update_log_backdate_recomputes_days_since_last_and_overdue() {
        // 验收 5：补录到过去时间后，首页红绿态按新发生时间重算（数据驱动）
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        let id = log(&conn, c, "喂食", "2026-09-18 08:00:00");
        let tiles = tiles_for_colony(&conn, c, TODAY).unwrap();
        assert_eq!(tile(&tiles, "喂食").days_since_last, Some(0));
        assert!(!tile(&tiles, "喂食").overdue);

        update_log(
            &conn,
            id,
            &LogUpdateInput {
                occurred_at: Some("2026-09-08 09:00:00".into()),
                note: None,
                action_id: None,
                food_ids: None,
            },
            "2026-09-18 08:00:00",
        )
        .unwrap();

        let tiles = tiles_for_colony(&conn, c, TODAY).unwrap();
        assert_eq!(tile(&tiles, "喂食").days_since_last, Some(10), "距上次跟随新发生时间");
        assert!(tile(&tiles, "喂食").overdue, "10 天 > 建议 3 天 → 超期红");
    }

    #[test]
    fn feeding_tile_lists_per_food_days_and_flags_food_overdue() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        // 2 天前喂了种子；8 天前喂过面包虫（周期 7 → 面包虫超期）
        feed_log(&conn, c, "2026-09-16 20:00:00", None, &["种子"]);
        feed_log(&conn, c, "2026-09-10 20:00:00", None, &["面包虫"]);

        let tiles = tiles_for_colony(&conn, c, TODAY).unwrap();
        let feed = tile(&tiles, "喂食");
        // 统一层 2 ≤ 3 自身不红，但面包虫食物层超期 → 任一层超期即红（Q2 决议；
        // 简报原文此处误写 !feed.overdue，与下方 Q2 测试同数据互斥，按 Q2 修正）
        assert!(feed.overdue, "面包虫食物层超期 → 整块红（Q2 决议）");
        assert_eq!(feed.days_since_last, Some(2));

        let foods: Vec<(String, Option<i64>, bool)> = feed.foods.iter()
            .map(|f| (f.name.clone(), f.days_since_last, f.overdue)).collect();
        assert_eq!(foods, vec![
            ("种子".into(), Some(2), false),
            ("干虾仁".into(), None, false),   // 从未喂过且设了周期 → None 不超期（同"从未记录"口径）
            ("面包虫".into(), Some(8), true), // 8 > 7 → 食物层超期
        ]);

        // 非喂食 tile 不带食物明细
        assert!(tile(&tiles, "垃圾清理").foods.is_empty());
    }

    #[test]
    fn feeding_tile_red_when_any_food_overdue_even_if_operation_layer_fresh() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        feed_log(&conn, c, "2026-09-16 20:00:00", None, &["种子"]);
        feed_log(&conn, c, "2026-09-10 20:00:00", None, &["面包虫"]);
        let tiles = tiles_for_colony(&conn, c, TODAY).unwrap();
        assert!(tile(&tiles, "喂食").overdue, "任一层超期即红（Q2 决议）");
    }

    #[test]
    fn food_without_interval_never_flags_and_wake_resets_food_clock() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        conn.execute("UPDATE food SET suggested_interval_days = NULL WHERE name = '种子'", []).unwrap();
        feed_log(&conn, c, "2026-08-01 20:00:00", None, &["种子"]); // 48 天前
        let tiles = tiles_for_colony(&conn, c, TODAY).unwrap();
        let seed = tile(&tiles, "喂食").foods.iter().find(|f| f.name == "种子").unwrap();
        assert!(!seed.overdue, "未设周期只受统一周期管（统一层 48>3 会红，但食物项自身不标）");

        // 出眠基线同样作用于食物层：出眠当天全部归零
        conn.execute(
            "INSERT INTO hibernation (colony_id, start_date, expected_end_date, actual_end_date)
             VALUES (?1, '2026-09-10', '2026-10-01', '2026-09-18')",
            params![c],
        )
        .unwrap();
        let tiles = tiles_for_colony(&conn, c, TODAY).unwrap();
        let worm = tile(&tiles, "喂食").foods.iter().find(|f| f.name == "面包虫").unwrap();
        assert_eq!(worm.days_since_last, Some(0), "从未喂过的食物从出眠日起算");
    }

    #[test]
    fn delete_log_removes_links_and_tile_returns_to_no_record() {
        // 验收 3：删除最后一条记录后，对应操作块「距上次」变为无记录态
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        let id = feed_log(&conn, c, "2026-09-18 08:00:00", None, &["种子"]);

        delete_log(&conn, id).unwrap();

        assert_eq!(count(&conn, "SELECT COUNT(*) FROM care_log"), 0);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM log_food"), 0, "食物关联一并删除");
        let tiles = tiles_for_colony(&conn, c, TODAY).unwrap();
        assert_eq!(tile(&tiles, "喂食").days_since_last, None, "回到无记录态");
        assert!(!tile(&tiles, "喂食").overdue);

        // 再删同一 id → 记录不存在
        let err = delete_log(&conn, id).unwrap_err();
        assert!(err.contains("不存在"), "实际错误：{err}");
    }

    // ── 按窝按月记录摘要（交互第三轮 #8：日历标记 + 重复黄条数据源）──

    fn month_rows(
        conn: &Connection,
        colony_id: i64,
        year: i64,
        month: i64,
        exclude_log_id: Option<i64>,
    ) -> Vec<MonthDayRecords> {
        colony_month_records(conn, colony_id, year, month, exclude_log_id).expect("按月查询失败")
    }

    #[test]
    fn colony_month_records_groups_by_day_and_action_with_last_time() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        log(&conn, c, "垃圾清理", "2026-09-12 19:40:00");
        log(&conn, c, "垃圾清理", "2026-09-12 08:30:00"); // 同日同操作第二条
        log(&conn, c, "巢穴保湿", "2026-09-12 21:00:00");
        log(&conn, c, "喂食", "2026-09-17 20:00:00");

        let rows = month_rows(&conn, c, 2026, 9, None);
        assert_eq!(rows.len(), 3, "3 个 (day, action) 组");
        let trash = rows.iter().find(|r| r.action_id == action_id(&conn, "垃圾清理")).unwrap();
        assert_eq!(trash.day, 12);
        assert_eq!(trash.count, 2);
        assert_eq!(trash.last_time, "2026-09-12 19:40:00"); // 最近一条的时刻
    }

    #[test]
    fn colony_month_records_excludes_other_months_and_colonies() {
        let conn = mem_conn();
        let c1 = colony(&conn, "大头一号");
        let c2 = colony(&conn, "大头二号");
        log(&conn, c1, "喂食", "2026-09-17 20:00:00");
        log(&conn, c1, "喂食", "2026-08-31 23:59:59"); // 上月
        // 次月：票 04 起写入层拒未来时间，log_care 写不进——查询边界直接落库验证
        conn.execute(
            "INSERT INTO care_log (colony_id, action_id, occurred_at, note, created_at)
             VALUES (?1, ?2, '2026-10-01 00:00:00', '', '2026-09-18 08:00:00')",
            params![c1, action_id(&conn, "喂食")],
        )
        .unwrap();
        log(&conn, c2, "喂食", "2026-09-18 07:00:00"); // 别窝

        let rows = month_rows(&conn, c1, 2026, 9, None);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].day, 17);
    }

    #[test]
    fn colony_month_records_exclude_log_id_drops_only_that_record() {
        // 编辑场景防自计数：正在编辑的这条不计入，其余照常聚合
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        let early = log(&conn, c, "喂食", "2026-09-12 08:30:00");
        let editing = log(&conn, c, "喂食", "2026-09-12 19:40:00");

        // 不排除：同日同操作 2 条，last_time 取最近
        let rows = month_rows(&conn, c, 2026, 9, None);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].count, 2);
        assert_eq!(rows[0].last_time, "2026-09-12 19:40:00");

        // 排除正在编辑的这条：剩 1 条，last_time 回退到早的那条
        let rows = month_rows(&conn, c, 2026, 9, Some(editing));
        assert_eq!(rows.len(), 1, "仍有另一条记录在");
        assert_eq!(rows[0].count, 1);
        assert_eq!(rows[0].last_time, "2026-09-12 08:30:00");

        // 排除另一条同理（对称校验）
        let rows = month_rows(&conn, c, 2026, 9, Some(early));
        assert_eq!(rows[0].count, 1);
        assert_eq!(rows[0].last_time, "2026-09-12 19:40:00");

        // 排除不存在的 id：不影响结果（WHERE 不命中任何行）
        let rows = month_rows(&conn, c, 2026, 9, Some(999_999));
        assert_eq!(rows[0].count, 2);
    }

    #[test]
    fn colony_month_records_rejects_bad_month() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        assert!(colony_month_records(&conn, c, 2026, 0, None).is_err());
        assert!(colony_month_records(&conn, c, 2026, 13, None).is_err());
    }
}
