//! 统计页数据聚合（票 07）。
//!
//! 全部函数只吃 `&Connection`，与 Tauri 解耦，可被 cargo test 直接覆盖；
//! Tauri command 只是薄包装（见 lib.rs）。
//!
//! 行为对齐 spec（评审附录规则 6/7/8）：
//! - 规则 6：相邻两次记录的间隔扣除与冬眠段重叠的自然日天数（多段各自适用，
//!   开放段视为延伸到间隔右端之外）。间隔按「同窝内相邻记录」成对——跨窝的
//!   两次记录不成对（各窝节奏独立，冬眠扣减也只属于各自的窝），窝内样本再按
//!   操作合并出 avg/min/max；
//! - 规则 7：频率分母 = 筛选范围自然日天数（含首尾、不扣冬眠），payload 携带
//!   `range_days` 供界面标注口径；
//! - 规则 8：食物占比给「出现次数」原始值（一次多选每种各计 1），分母与
//!   100% 归一化由前端做（lib/stats.ts）；
//! - 脏行防御：occurred_at 日期部分解析失败的行整行跳过（票 04 停靠①口径），
//!   一条脏行不得毒死整页。

use std::collections::BTreeMap;

use chrono::Datelike;
use rusqlite::{params, Connection};
use serde::Serialize;

// ── DTO ──────────────────────────────────────────────────────────────────

/// 按日操作数（范围内每天一条，无记录 = 0）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DailyCount {
    pub date: String,
    pub count: i64,
}

/// 热力图悬停明细里的一天内单条记录。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DayEntry {
    pub action_name: String,
    /// 喂食记录带食物名（字典顺序）；其他操作为空数组。
    pub food_names: Vec<String>,
}

/// 热力图悬停明细（只含有记录的天；按发生时间排）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DayDetail {
    pub date: String,
    pub entries: Vec<DayEntry>,
}

/// 食物出现次数（分母 = 各项合计，前端归一化 100%，规则 8）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FoodShare {
    pub food_name: String,
    pub occurrences: i64,
}

/// 每周操作数（周一为周首）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WeeklyCount {
    pub week_start: String,
    pub count: i64,
}

/// 单个操作的间隔统计（间隔已扣冬眠重叠天数，规则 6）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct IntervalStat {
    pub action_id: i64,
    pub name: String,
    pub kind: String, // reminding | log_only | follow（跟随喂食：无建议间隔口径，票 03）
    /// 仅提醒类有值（前端画建议刻度竖线）；登记类恒 None（界面标「仅登记」）。
    pub suggested_interval_days: Option<i64>,
    /// 有效间隔样本数（= 各窝样本数之和；0 = 记录不足）。
    pub sample_count: i64,
    pub avg_days: Option<f64>,
    pub min_days: Option<i64>,
    pub max_days: Option<i64>,
}

/// get_stats 返回体（spec API 契约：统计 → 按日/食物占比/每周/间隔）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StatsPayload {
    pub range_start: String,
    pub range_end: String,
    /// 规则 7：频率分母 = 范围自然日天数（含首尾，不扣冬眠）。
    pub range_days: i64,
    pub daily: Vec<DailyCount>,
    pub daily_detail: Vec<DayDetail>,
    pub food_share: Vec<FoodShare>,
    pub weekly: Vec<WeeklyCount>,
    pub intervals: Vec<IntervalStat>,
}

// ── 纯函数 ───────────────────────────────────────────────────────────────

/// 该日期所在周的周一（周首，每周分桶锚点）。
pub fn monday_of(date: chrono::NaiveDate) -> chrono::NaiveDate {
    let wd = date.weekday().number_from_monday() as i64; // 周一=1 … 周日=7
    date - chrono::Duration::days(wd - 1)
}

/// 相邻两次记录的间隔（自然日）扣除与冬眠段重叠天数后的有效间隔样本
/// （规则 6）。`dates` 须升序；样本数 = len-1，不足两条为空。
/// 调用方按窝分组后传入——跨窝的两次记录不成对。
pub fn interval_samples(
    dates: &[chrono::NaiveDate],
    segments: &[crate::hibernation::Segment],
) -> Vec<i64> {
    dates
        .windows(2)
        .map(|pair| {
            let (a, b) = (pair[0], pair[1]);
            let raw = (b - a).num_days();
            let overlap = crate::hibernation::hibernation_overlap_days(a, b, segments);
            raw - overlap
        })
        .collect()
}

/// 间隔样本汇总：(平均, 最短, 最长)。无样本 → 三个 None。
fn summarize(samples: &[i64]) -> (Option<f64>, Option<i64>, Option<i64>) {
    if samples.is_empty() {
        return (None, None, None);
    }
    let sum: i64 = samples.iter().sum();
    let avg = sum as f64 / samples.len() as f64;
    let min = *samples.iter().min().unwrap();
    let max = *samples.iter().max().unwrap();
    (Some(avg), Some(min), Some(max))
}

// ── 主入口 ───────────────────────────────────────────────────────────────

fn parse_iso(s: &str) -> Result<chrono::NaiveDate, String> {
    chrono::NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d")
        .map_err(|_| format!("日期格式应为 YYYY-MM-DD：{s}"))
}

fn db_err(e: rusqlite::Error) -> String {
    format!("数据库操作失败: {e}")
}

fn fmt_d(d: chrono::NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

/// 脏行防御：日期部分解析失败的行整行跳过（None），不毒死整页。
fn parse_date_part(s: &str) -> Option<chrono::NaiveDate> {
    chrono::NaiveDate::parse_from_str(s.get(0..10)?, "%Y-%m-%d").ok()
}

/// 全部记录里最早的 occurred_at 日期（`YYYY-MM-DD`）；无记录（或全是脏行）为 None。
/// 前端用它把「全部」范围下界压到 min(最早开始饲养日, 最早记录日期)：
/// 早于开始饲养日的补录不再从统计里消失（票 07 停靠①）。
pub fn earliest_log_date(conn: &Connection) -> Result<Option<String>, String> {
    let mut stmt = conn
        .prepare("SELECT substr(occurred_at, 1, 10) FROM care_log")
        .map_err(db_err)?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?;
    let mut min: Option<chrono::NaiveDate> = None;
    for date_part in rows {
        if let Some(day) = parse_date_part(&date_part) {
            min = Some(match min {
                Some(prev) if prev <= day => prev,
                _ => day,
            });
        }
    }
    Ok(min.map(fmt_d))
}

/// 统计页聚合：按日操作数（含无记录天 = 0）、悬停明细（含食物）、食物出现
/// 次数、每周操作数（周一为周首）、各操作间隔（扣冬眠，规则 6）。
/// `colony_id` 为 None = 全部窝合并。
pub fn get_stats(
    conn: &Connection,
    colony_id: Option<i64>,
    start_date: &str,
    end_date: &str,
) -> Result<StatsPayload, String> {
    let start = parse_iso(start_date)?;
    let end = parse_iso(end_date)?;
    if end < start {
        return Err(format!(
            "时间范围倒置：开始（{}）不能晚于结束（{}）",
            start_date.trim(),
            end_date.trim()
        ));
    }
    let range_days = (end - start).num_days() + 1;
    let start_s = start_date.trim();
    let end_s = end_date.trim();

    // 范围内全部记录（日期部分 + 操作名），按发生时间排——按日计数与悬停明细共用
    let mut stmt = conn
        .prepare(
            "SELECT l.id, substr(l.occurred_at, 1, 10), a.name
             FROM care_log l JOIN care_action a ON a.id = l.action_id
             WHERE substr(l.occurred_at, 1, 10) BETWEEN ?1 AND ?2
               AND (?3 IS NULL OR l.colony_id = ?3)
             ORDER BY l.occurred_at, l.id",
        )
        .map_err(db_err)?;
    let log_rows = stmt
        .query_map(params![start_s, end_s, colony_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?;

    // 各记录的食物名（一次多选各计 1，规则 8），按字典顺序
    let mut food_stmt = conn
        .prepare(
            "SELECT lf.log_id, f.name
             FROM log_food lf
             JOIN food f ON f.id = lf.food_id
             JOIN care_log l ON l.id = lf.log_id
             WHERE substr(l.occurred_at, 1, 10) BETWEEN ?1 AND ?2
               AND (?3 IS NULL OR l.colony_id = ?3)
             ORDER BY lf.log_id, f.sort, f.id",
        )
        .map_err(db_err)?;
    let mut foods_per_log: BTreeMap<i64, Vec<String>> = BTreeMap::new();
    let food_rows = food_stmt
        .query_map(params![start_s, end_s, colony_id], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(db_err)?;
    for row in food_rows {
        let (log_id, name) = row.map_err(db_err)?;
        foods_per_log.entry(log_id).or_default().push(name);
    }

    // 按日计数（脏行跳过）+ 悬停明细（只含有记录的天）
    let mut counts: BTreeMap<chrono::NaiveDate, i64> = BTreeMap::new();
    let mut detail_map: BTreeMap<chrono::NaiveDate, Vec<DayEntry>> = BTreeMap::new();
    for (log_id, date_part, action_name) in log_rows {
        let Some(day) = parse_date_part(&date_part) else {
            continue;
        };
        *counts.entry(day).or_insert(0) += 1;
        detail_map.entry(day).or_default().push(DayEntry {
            action_name,
            food_names: foods_per_log.get(&log_id).cloned().unwrap_or_default(),
        });
    }

    // 范围内每天一条（无记录 = 0），顺路按周一分桶出每周计数（零周也在）
    let mut daily = Vec::with_capacity(range_days as usize);
    let mut weekly_map: BTreeMap<chrono::NaiveDate, i64> = BTreeMap::new();
    let mut day = start;
    loop {
        let count = counts.get(&day).copied().unwrap_or(0);
        *weekly_map.entry(monday_of(day)).or_insert(0) += count;
        daily.push(DailyCount { date: fmt_d(day), count });
        if day == end {
            break;
        }
        day = day.succ_opt().ok_or_else(|| format!("日期越界：{day}"))?;
    }

    let daily_detail = detail_map
        .into_iter()
        .map(|(day, entries)| DayDetail { date: fmt_d(day), entries })
        .collect();

    let weekly = weekly_map
        .into_iter()
        .map(|(week_start, count)| WeeklyCount { week_start: fmt_d(week_start), count })
        .collect();

    // 食物出现次数（分母 = 合计，前端归一化），按次数降序、字典顺序破平
    let mut share_stmt = conn
        .prepare(
            "SELECT f.name, COUNT(*) AS occ
             FROM log_food lf
             JOIN food f ON f.id = lf.food_id
             JOIN care_log l ON l.id = lf.log_id
             WHERE substr(l.occurred_at, 1, 10) BETWEEN ?1 AND ?2
               AND (?3 IS NULL OR l.colony_id = ?3)
             GROUP BY lf.food_id
             ORDER BY occ DESC, f.sort, f.id",
        )
        .map_err(db_err)?;
    let food_share = share_stmt
        .query_map(params![start_s, end_s, colony_id], |row| {
            Ok(FoodShare {
                food_name: row.get(0)?,
                occurrences: row.get(1)?,
            })
        })
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?;

    // 间隔统计（规则 6）：按（窝, 操作）分组配对，各窝用自己的冬眠段扣减，
    // 样本按操作合并出 avg/min/max——跨窝的两次记录不成对。
    let mut pair_stmt = conn
        .prepare(
            "SELECT l.colony_id, l.action_id, substr(l.occurred_at, 1, 10)
             FROM care_log l
             WHERE substr(l.occurred_at, 1, 10) BETWEEN ?1 AND ?2
               AND (?3 IS NULL OR l.colony_id = ?3)",
        )
        .map_err(db_err)?;
    let mut dates_per_pair: BTreeMap<(i64, i64), Vec<chrono::NaiveDate>> = BTreeMap::new();
    let pair_rows = pair_stmt
        .query_map(params![start_s, end_s, colony_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(db_err)?;
    for row in pair_rows {
        let (colony, action, date_part) = row.map_err(db_err)?;
        let Some(day) = parse_date_part(&date_part) else {
            continue;
        };
        dates_per_pair.entry((colony, action)).or_default().push(day);
    }
    for dates in dates_per_pair.values_mut() {
        dates.sort_unstable();
    }

    // 各窝冬眠段（脏行整段跳过：入眠日非法扣无可扣，出眠日非法不当作开放段）
    let mut seg_stmt = conn
        .prepare("SELECT colony_id, start_date, actual_end_date FROM hibernation")
        .map_err(db_err)?;
    let mut segments_per_colony: BTreeMap<i64, Vec<crate::hibernation::Segment>> = BTreeMap::new();
    let seg_rows = seg_stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })
        .map_err(db_err)?;
    for row in seg_rows {
        let (colony, start, actual_end) = row.map_err(db_err)?;
        let Some(seg_start) = parse_date_part(&start) else {
            continue; // 入眠日非法 → 扣无可扣，整段跳过
        };
        let segment = match actual_end.as_deref() {
            None => crate::hibernation::Segment::open(seg_start), // 开放段（延伸到间隔右端之外）
            Some(s) => match parse_date_part(s) {
                Some(d) => crate::hibernation::Segment::closed(seg_start, d),
                None => continue, // 出眠日脏（非空但解析失败）→ 整段跳过
            },
        };
        segments_per_colony.entry(colony).or_default().push(segment);
    }

    // 行集 = 启用中的操作 ∪ 范围内有记录的操作（停用但历史有记录的照常显示，规则 10）
    let mut act_stmt = conn
        .prepare(
            "SELECT a.id, a.name, a.kind, a.suggested_interval_days
             FROM care_action a
             WHERE a.enabled = 1
                OR EXISTS(
                    SELECT 1 FROM care_log l
                    WHERE l.action_id = a.id
                      AND substr(l.occurred_at, 1, 10) BETWEEN ?1 AND ?2
                      AND (?3 IS NULL OR l.colony_id = ?3)
                )
             ORDER BY a.sort, a.id",
        )
        .map_err(db_err)?;
    let act_rows = act_stmt
        .query_map(params![start_s, end_s, colony_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<i64>>(3)?,
            ))
        })
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?;

    let mut intervals = Vec::with_capacity(act_rows.len());
    for (action_id, name, kind, suggested) in act_rows {
        let mut samples: Vec<i64> = Vec::new();
        for ((colony, act), dates) in &dates_per_pair {
            if *act != action_id {
                continue;
            }
            let segments = segments_per_colony.get(colony).map(Vec::as_slice).unwrap_or(&[]);
            samples.extend(interval_samples(dates, segments));
        }
        let (avg, min, max) = summarize(&samples);
        intervals.push(IntervalStat {
            action_id,
            name,
            suggested_interval_days: if kind == "reminding" { suggested } else { None },
            kind,
            sample_count: samples.len() as i64,
            avg_days: avg,
            min_days: min,
            max_days: max,
        });
    }

    Ok(StatsPayload {
        range_start: start_s.to_string(),
        range_end: end_s.to_string(),
        range_days,
        daily,
        daily_detail,
        food_share,
        weekly,
        intervals,
    })
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

    fn action_id(conn: &Connection, name: &str) -> i64 {
        conn.query_row("SELECT id FROM care_action WHERE name = ?1", params![name], |r| r.get(0))
            .expect("查操作失败")
    }

    fn food_id(conn: &Connection, name: &str) -> i64 {
        conn.query_row("SELECT id FROM food WHERE name = ?1", params![name], |r| r.get(0))
            .expect("查食物失败")
    }

    /// 记一笔（created_at 用远期固定值：录入时间不参与统计，只绕开“不能记未来”校验，
    /// 便于补录 2026 下半年与 2027 的发生时间）。
    fn log(conn: &Connection, colony_id: i64, action: &str, happened_at: &str) -> i64 {
        crate::care::log_care(
            conn,
            &crate::care::CareLogInput {
                colony_id,
                action_id: action_id(conn, action),
                happened_at: happened_at.into(),
                note: None,
                food_ids: vec![],
            },
            "2027-06-01 08:00:00",
        )
        .expect("记账失败")
    }

    fn log_with_foods(
        conn: &Connection,
        colony_id: i64,
        happened_at: &str,
        foods: &[&str],
    ) -> i64 {
        crate::care::log_care(
            conn,
            &crate::care::CareLogInput {
                colony_id,
                action_id: action_id(conn, "喂食"),
                happened_at: happened_at.into(),
                note: None,
                food_ids: foods.iter().map(|f| food_id(conn, f)).collect(),
            },
            "2027-06-01 08:00:00",
        )
        .expect("记账失败")
    }

    /// 直插一段冬眠（绕过应用层校验，用于搭场景）。
    fn seg(conn: &Connection, colony_id: i64, start: &str, actual_end: Option<&str>) {
        let expected = actual_end.unwrap_or("2099-01-01");
        conn.execute(
            "INSERT INTO hibernation (colony_id, start_date, expected_end_date, actual_end_date)
             VALUES (?1, ?2, ?3, ?4)",
            params![colony_id, start, expected, actual_end],
        )
        .expect("插冬眠段失败");
    }

    /// 取预置「撤食」行（kind='follow'）：票 01 v7 迁移起种子即含撤食（sort=2），
    /// 直接复用该行，不再自插（v6 时代用 PRAGMA 绕 CHECK 直插的写法已随迁移作废）。
    fn seed_retrieval_action(conn: &Connection) -> i64 {
        action_id(conn, "撤食")
    }

    fn d(s: &str) -> chrono::NaiveDate {
        chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").expect("测试日期应为合法 ISO")
    }

    // ── 纯函数 ──

    #[test]
    fn monday_of_snaps_back_to_monday() {
        assert_eq!(monday_of(d("2026-09-18")), d("2026-09-14"), "周五 → 本周一");
        assert_eq!(monday_of(d("2026-09-14")), d("2026-09-14"), "周一不动");
        assert_eq!(monday_of(d("2026-09-13")), d("2026-09-07"), "周日归上一周");
    }

    #[test]
    fn interval_samples_deduct_hibernation_overlap() {
        // 跨 30 天冬眠的两次喂食：68 − 30 = 38（与 hibernation 模块验收用例同数据）
        let samples = interval_samples(
            &[d("2026-11-01"), d("2027-01-08")],
            &[crate::hibernation::Segment::closed(d("2026-11-10"), d("2026-12-09"))],
        );
        assert_eq!(samples, vec![38]);
    }

    #[test]
    fn interval_samples_treats_open_segment_as_extending_over_the_gap() {
        // 冬眠中（开放段 09-10 起）：09-08 → 09-15 原始 7 天，扣 [09-10, 09-15) 的 5 天
        let samples = interval_samples(
            &[d("2026-09-08"), d("2026-09-15")],
            &[crate::hibernation::Segment::open(d("2026-09-10"))],
        );
        assert_eq!(samples, vec![2]);
    }

    #[test]
    fn interval_samples_needs_two_dates_and_multi_segments_each_apply() {
        assert!(interval_samples(&[], &[]).is_empty());
        assert!(interval_samples(&[d("2026-01-01")], &[]).is_empty());
        // 两段冬眠各自适用：182 − 30 − 22 = 130
        let samples = interval_samples(
            &[d("2026-10-01"), d("2027-04-01")],
            &[
                crate::hibernation::Segment::closed(d("2026-11-10"), d("2026-12-09")),
                crate::hibernation::Segment::closed(d("2027-01-20"), d("2027-02-10")),
            ],
        );
        assert_eq!(samples, vec![130]);
    }

    // ── get_stats：范围/按日/悬停明细 ──

    #[test]
    fn get_stats_rejects_bad_format_and_reversed_range() {
        let conn = mem_conn();
        assert!(get_stats(&conn, None, "2026/09/01", "2026-09-18").is_err());
        assert!(get_stats(&conn, None, "2026-09-18", "2026-09-01").is_err(), "end < start 拒绝");
        assert!(get_stats(&conn, None, "2026-09-01", "2026-09-01").is_ok(), "单天范围合法");
    }

    #[test]
    fn daily_covers_every_day_with_zeros_and_filters_by_colony_and_range() {
        let conn = mem_conn();
        let c1 = colony(&conn, "大头一号");
        let c2 = colony(&conn, "针毛一号");
        log(&conn, c1, "喂食", "2026-09-10 09:00:00");
        log(&conn, c1, "巢穴保湿", "2026-09-10 10:00:00"); // 同天第二次 → 当天 2
        log(&conn, c1, "喂食", "2026-09-05 09:00:00"); // 范围外
        log(&conn, c2, "喂食", "2026-09-12 09:00:00");

        let all = get_stats(&conn, None, "2026-09-08", "2026-09-14").unwrap();
        assert_eq!(all.range_days, 7, "含首尾 7 个自然日（规则 7 频率分母）");
        assert_eq!(all.daily.len(), 7, "范围内每天一条，含无记录天");
        assert_eq!(all.daily[0].date, "2026-09-08");
        assert_eq!(all.daily[0].count, 0, "无记录天 = 0");
        assert_eq!(all.daily[2].date, "2026-09-10");
        assert_eq!(all.daily[2].count, 2);
        assert_eq!(all.daily[4].count, 1, "09-12 针毛的记录在「全部」里");
        assert_eq!(all.daily[6].count, 0);

        let only1 = get_stats(&conn, Some(c1), "2026-09-08", "2026-09-14").unwrap();
        assert_eq!(only1.daily[2].count, 2);
        assert_eq!(only1.daily[4].count, 0, "筛选某窝后别窝的记录不计");
        assert_eq!(only1.range_days, 7, "分母不随记录数变（规则 7）");
    }

    #[test]
    fn daily_detail_hover_carries_food_names_and_skips_recordless_days() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        log_with_foods(&conn, c, "2026-09-10 09:00:00", &["种子", "干虾仁"]);
        log(&conn, c, "巢穴保湿", "2026-09-10 10:00:00");

        let stats = get_stats(&conn, None, "2026-09-08", "2026-09-14").unwrap();
        assert_eq!(stats.daily_detail.len(), 1, "只含有记录的天");
        let day = &stats.daily_detail[0];
        assert_eq!(day.date, "2026-09-10");
        assert_eq!(day.entries.len(), 2);
        assert_eq!(day.entries[0].action_name, "喂食");
        assert_eq!(
            day.entries[0].food_names,
            vec!["种子", "干虾仁"],
            "悬停明细含喂食的食物（验收 3），按字典顺序"
        );
        assert_eq!(day.entries[1].action_name, "巢穴保湿");
        assert!(day.entries[1].food_names.is_empty(), "非喂食记录无食物");
    }

    #[test]
    fn food_share_counts_each_selected_food_once_per_log() {
        // 规则 8：一次多选投喂每种所选食物各计 1 次
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        log_with_foods(&conn, c, "2026-09-10 09:00:00", &["种子", "干虾仁"]);
        log_with_foods(&conn, c, "2026-09-12 09:00:00", &["种子"]);
        log(&conn, c, "巢穴保湿", "2026-09-12 10:00:00"); // 非喂食不影响食物占比

        let stats = get_stats(&conn, None, "2026-09-01", "2026-09-30").unwrap();
        let seed = stats.food_share.iter().find(|f| f.food_name == "种子").unwrap();
        let shrimp = stats.food_share.iter().find(|f| f.food_name == "干虾仁").unwrap();
        assert_eq!(seed.occurrences, 2);
        assert_eq!(shrimp.occurrences, 1);
        assert!(
            stats.food_share.iter().all(|f| f.food_name != "面包虫"),
            "没投喂过的食物不出现（分母 = 出现过的食物合计，前端归一化）"
        );

        let only = get_stats(&conn, Some(999), "2026-09-01", "2026-09-30").unwrap();
        // 不存在的窝 = 无记录，不报错（筛选项动态来自窝清单，正常不会传 999；防崩即可）
        assert!(only.food_share.is_empty());
    }

    #[test]
    fn weekly_buckets_are_keyed_by_monday() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        // 2026-09-14 周一、09-17 周五、09-21 周二（下一周）
        log(&conn, c, "喂食", "2026-09-14 09:00:00");
        log(&conn, c, "喂食", "2026-09-17 09:00:00");
        log(&conn, c, "喂食", "2026-09-21 09:00:00");

        let stats = get_stats(&conn, None, "2026-09-14", "2026-09-21").unwrap();
        assert_eq!(stats.weekly.len(), 2);
        assert_eq!(stats.weekly[0].week_start, "2026-09-14", "周一为周首");
        assert_eq!(stats.weekly[0].count, 2);
        assert_eq!(stats.weekly[1].week_start, "2026-09-21");
        assert_eq!(stats.weekly[1].count, 1);
    }

    // ── get_stats：间隔统计（规则 6）──

    #[test]
    fn intervals_deduct_hibernation_and_expose_suggestion_only_for_reminding() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        // 喂食：跨 30 天冬眠，68 − 30 = 38
        log(&conn, c, "喂食", "2026-11-01 09:00:00");
        log(&conn, c, "喂食", "2027-01-08 09:00:00");
        // 垃圾清理：无冬眠交叉，09-01 → 09-08 = 7 天
        log(&conn, c, "垃圾清理", "2026-09-01 09:00:00");
        log(&conn, c, "垃圾清理", "2026-09-08 09:00:00");
        seg(&conn, c, "2026-11-10", Some("2026-12-09"));

        let stats = get_stats(&conn, Some(c), "2026-09-01", "2027-02-01").unwrap();
        let feed = stats.intervals.iter().find(|i| i.name == "喂食").unwrap();
        assert_eq!(feed.sample_count, 1);
        assert_eq!(feed.avg_days, Some(38.0), "间隔扣冬眠重叠天数（规则 6/验收 2）");
        assert_eq!(feed.min_days, Some(38));
        assert_eq!(feed.max_days, Some(38));
        assert_eq!(feed.suggested_interval_days, Some(3), "提醒类带建议间隔供画刻度");
        assert_eq!(feed.kind, "reminding");

        let trash = stats.intervals.iter().find(|i| i.name == "垃圾清理").unwrap();
        assert_eq!(trash.avg_days, Some(7.0), "无冬眠交叉不扣减");
        assert_eq!(trash.suggested_interval_days, Some(7));

        let water = stats.intervals.iter().find(|i| i.name == "活动区换水").unwrap();
        assert_eq!(water.sample_count, 0, "没记录的操作也给行，样本 0");
        assert_eq!(water.avg_days, None);
        assert_eq!(water.suggested_interval_days, None, "登记类不带建议间隔（前端标「仅登记」）");
        assert_eq!(water.kind, "log_only");
    }

    #[test]
    fn intervals_pair_within_colony_only_then_pool_across_colonies() {
        let conn = mem_conn();
        let c1 = colony(&conn, "大头一号");
        let c2 = colony(&conn, "针毛一号");
        // c1：09-01 → 09-11 = 10 天
        log(&conn, c1, "喂食", "2026-09-01 09:00:00");
        log(&conn, c1, "喂食", "2026-09-11 09:00:00");
        // c2：09-05 → 09-09 = 4 天；与 c1 的 09-11 不成对（跨窝不成对）
        log(&conn, c2, "喂食", "2026-09-05 09:00:00");
        log(&conn, c2, "喂食", "2026-09-09 09:00:00");

        let stats = get_stats(&conn, None, "2026-09-01", "2026-09-30").unwrap();
        let feed = stats.intervals.iter().find(|i| i.name == "喂食").unwrap();
        assert_eq!(feed.sample_count, 2, "两窝各出一对样本，共 2（不是 3）");
        assert_eq!(feed.avg_days, Some(7.0), "(10 + 4) / 2");
        assert_eq!(feed.min_days, Some(4));
        assert_eq!(feed.max_days, Some(10));
    }

    #[test]
    fn intervals_skip_dirty_dates_and_open_segments_apply() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        // 开放段：冬眠中。09-08 → 09-15 原始 7 天，扣 [09-10, 09-15) 的 5 天 = 2
        seg(&conn, c, "2026-09-10", None);
        log(&conn, c, "喂食", "2026-09-08 09:00:00");
        log(&conn, c, "喂食", "2026-09-15 09:00:00");
        // 脏行：occurred_at 解析失败 → 整行跳过，不毒死整页
        conn.execute(
            "INSERT INTO care_log (colony_id, action_id, occurred_at, note, created_at)
             VALUES (?1, ?2, 'garbage-time', '', '2026-09-18 08:00:00')",
            params![c, action_id(&conn, "喂食")],
        )
        .unwrap();

        let stats = get_stats(&conn, Some(c), "2026-09-01", "2026-09-30").unwrap();
        let feed = stats.intervals.iter().find(|i| i.name == "喂食").unwrap();
        assert_eq!(feed.sample_count, 1, "脏行不参与配对");
        assert_eq!(feed.avg_days, Some(2.0), "开放段视为延伸到间隔右端之外（规则 6）");

        // 脏行同样不进按日/明细/占比
        assert!(stats.daily.iter().all(|x| x.count <= 1));
        assert!(stats
            .daily_detail
            .iter()
            .all(|x| x.entries.iter().all(|e| e.action_name != "garbage-time")));
    }

    #[test]
    fn earliest_log_date_takes_min_across_logs_and_skips_dirty_rows() {
        // 票 07 停靠①：「全部」范围下界 = min(最早开始饲养日, 最早记录日期)。
        // 2025-12-01 的补录早于饲养日 2026-01-20，统计不该把它弄丢。
        let conn = mem_conn();
        assert_eq!(earliest_log_date(&conn).unwrap(), None, "空库无下界");

        let c = colony(&conn, "大头一号");
        log(&conn, c, "喂食", "2026-09-10 09:00:00");
        log(&conn, c, "喂食", "2025-12-01 09:00:00");
        // 脏行（occurred_at 解析失败）整行跳过，不当天花板也不当下界（票 04 停靠①口径）
        conn.execute(
            "INSERT INTO care_log (colony_id, action_id, occurred_at, note, created_at)
             VALUES (?1, ?2, 'garbage-time', '', '2027-06-01 08:00:00')",
            params![c, action_id(&conn, "喂食")],
        )
        .unwrap();

        assert_eq!(
            earliest_log_date(&conn).unwrap(),
            Some("2025-12-01".to_string())
        );
    }

    #[test]
    fn retrieval_logs_count_into_daily_weekly_hover_and_interval_without_suggestion() {
        // 票 03（撤食统计切片）：撤食作为普通维护操作进全部统计口径——
        // 按日计数、每周次数、悬停明细照常聚合；follow 性质不参与建议间隔
        // 口径（suggested_interval_days 恒 None），实际间隔照常成对计算。
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        seed_retrieval_action(&conn);

        log(&conn, c, "喂食", "2026-09-12 08:00:00");
        log(&conn, c, "撤食", "2026-09-12 09:00:00"); // 同天第二笔
        log(&conn, c, "撤食", "2026-09-15 09:00:00"); // 与上一条成对：3 天

        let stats = get_stats(&conn, Some(c), "2026-09-08", "2026-09-21").unwrap();

        // 按日：撤食与喂食各计 1，同天合并为 2
        let day = |date: &str| stats.daily.iter().find(|d| d.date == date).unwrap().count;
        assert_eq!(day("2026-09-12"), 2);
        assert_eq!(day("2026-09-15"), 1);

        // 每周（周一为周首）：09-12 落 09-07 周、09-15 落 09-14 周
        let week = |ws: &str| stats.weekly.iter().find(|w| w.week_start == ws).unwrap().count;
        assert_eq!(week("2026-09-07"), 2, "撤食进每周操作次数");
        assert_eq!(week("2026-09-14"), 1);

        // 悬停明细：撤食条目可见、无食物括注
        let day12 = stats.daily_detail.iter().find(|d| d.date == "2026-09-12").unwrap();
        assert_eq!(day12.entries.len(), 2);
        let retrieval_entry = day12.entries.iter().find(|e| e.action_name == "撤食").unwrap();
        assert!(retrieval_entry.food_names.is_empty(), "撤食无食物括注");

        // 间隔：follow 无建议间隔（不参与超期/建议口径），实际间隔照常算
        let retrieval = stats.intervals.iter().find(|i| i.name == "撤食").unwrap();
        assert_eq!(retrieval.kind, "follow");
        assert_eq!(retrieval.suggested_interval_days, None, "follow 无建议间隔口径");
        assert_eq!(retrieval.sample_count, 1);
        assert_eq!(retrieval.avg_days, Some(3.0), "09-12 → 09-15 = 3 天");

        // 回归守护：喂食的建议间隔不受新性质影响
        let feed = stats.intervals.iter().find(|i| i.name == "喂食").unwrap();
        assert_eq!(feed.suggested_interval_days, Some(3));
    }

    #[test]
    fn single_day_range_and_empty_db_do_not_crash() {
        let conn = mem_conn();
        // 空库单天
        let stats = get_stats(&conn, None, "2026-09-18", "2026-09-18").unwrap();
        assert_eq!(stats.range_days, 1);
        assert_eq!(stats.daily.len(), 1);
        assert_eq!(stats.daily[0].count, 0);
        assert_eq!(stats.weekly.len(), 1);
        assert!(stats.daily_detail.is_empty());
        assert!(stats.food_share.is_empty());
        assert_eq!(stats.intervals.len(), 5, "预置 5 个启用操作都给行（v7 起含撤食）");
        assert!(stats.intervals.iter().all(|i| i.sample_count == 0 && i.avg_days.is_none()));

        // 同一天两条记录：间隔样本 0 对（同天不成对）、当天计数 2
        let c = colony(&conn, "大头一号");
        log(&conn, c, "喂食", "2026-09-18 08:00:00");
        log(&conn, c, "喂食", "2026-09-18 20:00:00");
        let stats = get_stats(&conn, None, "2026-09-18", "2026-09-18").unwrap();
        assert_eq!(stats.daily[0].count, 2);
        let feed = stats.intervals.iter().find(|i| i.name == "喂食").unwrap();
        assert_eq!(feed.sample_count, 1, "同天两条记录也成对");
        assert_eq!(feed.avg_days, Some(0.0), "同天相邻间隔 = 0 天");
    }
}
