//! 网页端命令参数 schema 与服务端输入校验（webui-checkin 票 05，规格 A）。
//!
//! 白名单≠校验：HTTP 可绕过前端 UI，每个登记命令在本层定义**类型化参数
//! schema**——serde Deserialize 结构反序列化即拦「形状垃圾」（未知字段拒、
//! 类型错拒、缺字段拒），再过取值校验（ID 正整数、日期格式+范围、字符串
//! 长度、分页上限、数值范围）。非法一律 `Err(人话)`，HTTP 层回
//! `400 {"error":..}`，不进库。
//!
//! 键名映射（与桌面 Tauri invoke 同款）：**顶层参数键 camelCase**
//! （`#[serde(rename_all = "camelCase")]`，票 01 前端契约：前端直接传
//! camelCase，Tauri 侧自行映射，HTTP 派发层对齐同款映射）；**嵌套 input
//! 对象沿用各 DTO 的 snake_case 键**（桌面 invoke 对嵌套键不做映射，前端
//! types.ts 即 snake_case，两边同形）。
//!
//! 校验分层分工：schema 层挡形状（类型/长度/取值范围），语义校验（存在性、
//! 日期先后、不晚于今天、字典启用规则）权威在纯核，本层沿用不复制。

use chrono::Datelike;
use serde::de::DeserializeOwned;

use crate::care;
use crate::nest_checkin;

// ── 上限常量（规格 A：字符串长度/分页/数值范围服务端兜底）─────────────────

/// 备注长度上限（字符数）。
pub const MAX_NOTE_CHARS: usize = 2000;

/// 备注搜索关键词长度上限（字符数）。
pub const MAX_KEYWORD_CHARS: usize = 200;

/// 发生时间字符串长度上限（最宽合法形状 `YYYY-MM-DD HH:MM:SS` = 19 字符，留裕量挡垃圾）。
pub const MAX_DATETIME_CHARS: usize = 32;

/// 一条记录可挂食物种数上限。
pub const MAX_FOOD_IDS: usize = 50;

/// 分页大小上限（与纯核 `list_logs` 的 clamp 上限一致；HTTP 层直接拒超限）。
pub const MAX_PAGE_LIMIT: i64 = 500;

/// 蚁后/工蚁数上限（挡明显垃圾；下限 0 与纯核一致，负数即拒）。
pub const MAX_COUNT: i64 = 1_000_000;

/// ISO 日期合理年段（schema 层挡 `0001-01-01` 之类的范围垃圾；业务先后纯核管）。
pub const MIN_YEAR: i32 = 1970;
pub const MAX_YEAR: i32 = 2100;

// ── 取值校验器（人话错误，HTTP 层原样进 {"error"}）─────────────────────────

/// ID 必须是正整数（数据库 rowid 自 1 起；0/负数一定是绕过前端的垃圾）。
fn check_id(v: i64, field: &str) -> Result<(), String> {
    if v < 1 {
        return Err(format!("{field} 必须是正整数（收到 {v}）"));
    }
    Ok(())
}

/// ISO 日期（`YYYY-MM-DD`）格式 + 合理年段。
fn check_iso_date(s: &str, field: &str) -> Result<(), String> {
    let t = s.trim();
    let d = chrono::NaiveDate::parse_from_str(t, "%Y-%m-%d")
        .map_err(|_| format!("{field}格式应为 YYYY-MM-DD：{t}"))?;
    let y = d.year();
    if !(MIN_YEAR..=MAX_YEAR).contains(&y) {
        return Err(format!("{field}超出合理范围（{t}，年份应在 {MIN_YEAR}–{MAX_YEAR} 之间）"));
    }
    Ok(())
}

/// 发生/编辑时间：长度上限 + 日期前缀形状（接受 `YYYY-MM-DD` 或带 `HH:MM[:SS]`；
/// 完整格式权威在纯核 `care::normalize_happened_at`，本层先挡明显垃圾）。
fn check_happened_at(s: &str) -> Result<(), String> {
    let t = s.trim();
    if t.chars().count() > MAX_DATETIME_CHARS {
        return Err(format!("发生时间长度超过上限（{MAX_DATETIME_CHARS} 字符）"));
    }
    let head = t.split([' ', 'T']).next().unwrap_or("");
    check_iso_date(head, "发生时间")
}

/// 备注长度（Option 语义：None/空都放行，纯核自行 trim）。
fn check_note(s: &Option<String>) -> Result<(), String> {
    if let Some(n) = s {
        if n.chars().count() > MAX_NOTE_CHARS {
            return Err(format!("备注长度超过上限（{MAX_NOTE_CHARS} 字符）"));
        }
    }
    Ok(())
}

/// 关键词长度。
fn check_keyword(s: &Option<String>) -> Result<(), String> {
    if let Some(k) = s {
        if k.chars().count() > MAX_KEYWORD_CHARS {
            return Err(format!("搜索关键词长度超过上限（{MAX_KEYWORD_CHARS} 字符）"));
        }
    }
    Ok(())
}

/// 蚁后/工蚁数：非负 + 上限（负数纯核也拒，schema 层先挡）。
fn check_count(v: Option<i64>, field: &str) -> Result<(), String> {
    match v {
        Some(n) if n < 0 => Err(format!("{field}不能为负数（收到 {n}）")),
        Some(n) if n > MAX_COUNT => Err(format!("{field}超出合理范围（上限 {MAX_COUNT}）")),
        _ => Ok(()),
    }
}

/// 食物多选：种数上限 + 逐个正整数（存在/启用纯核管）。
fn check_food_ids(ids: &[i64]) -> Result<(), String> {
    if ids.len() > MAX_FOOD_IDS {
        return Err(format!("一次最多挂 {MAX_FOOD_IDS} 种食物"));
    }
    for (i, id) in ids.iter().enumerate() {
        check_id(*id, &format!("food_ids[{i}]"))?;
    }
    Ok(())
}

// ── 解析入口：serde 报错转人话 ────────────────────────────────────────────

/// 把 args JSON 解析进类型化 schema。serde 的英文报错翻成现场可读的人话
/// （未知字段/缺字段/类型错三类常见形状），其余原样带上细节。
pub fn parse<A: DeserializeOwned>(args: &serde_json::Value) -> Result<A, String> {
    serde_json::from_value(args.clone()).map_err(humanize_parse_error)
}

fn humanize_parse_error(e: serde_json::Error) -> String {
    let raw = e.to_string();
    if let Some((label, rest)) = take_quoted(&raw, "unknown field `") {
        let _ = rest;
        return format!("参数包含未知字段：{label}");
    }
    if let Some((label, _)) = take_quoted(&raw, "missing field `") {
        return format!("缺少必填参数：{label}");
    }
    if raw.contains("invalid type") || raw.contains("invalid value") {
        return format!("参数类型或取值不正确（{raw}）");
    }
    format!("请求参数不合法（{raw}）")
}

/// 从 `marker\`x\`` 形状里抠出反引号内的字段名。
fn take_quoted<'a>(raw: &'a str, marker: &str) -> Option<(String, &'a str)> {
    let idx = raw.find(marker)?;
    let rest = &raw[idx + marker.len()..];
    let end = rest.find('`')?;
    Some((rest[..end].to_string(), &rest[end + 1..]))
}

/// 命令参数 schema 的统一入口：反序列化 + 取值校验，两段都过才算合法。
pub trait ValidatedArgs: DeserializeOwned {
    fn validate(&self) -> Result<(), String>;
}

// ── 无参命令（list_colonies / list_locations / list_actions / list_foods）──

/// 无参命令的 args：`{}` 即可；多传字段即拒（deny_unknown_fields）。
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmptyArgs {}

impl ValidatedArgs for EmptyArgs {
    fn validate(&self) -> Result<(), String> {
        Ok(())
    }
}

// ── ID 形状复用 ──────────────────────────────────────────────────────────

/// `{ id }` 形命令（delete_log / delete_checkin）。
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdArgs {
    pub id: i64,
}

impl ValidatedArgs for IdArgs {
    fn validate(&self) -> Result<(), String> {
        check_id(self.id, "id")
    }
}

/// `{ colonyId }` 形命令（list_checkins / get_checkin_digest）。
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ColonyIdArgs {
    pub colony_id: i64,
}

impl ValidatedArgs for ColonyIdArgs {
    fn validate(&self) -> Result<(), String> {
        check_id(self.colony_id, "colonyId")
    }
}

/// `{ colonyId, year, month, excludeLogId? }` 形命令（colony_month_records，
/// 交互第三轮日历标记数据源；打卡面板/记录页/编辑弹窗在用，桌面与网页共用）。
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ColonyMonthRecordsArgs {
    pub colony_id: i64,
    pub year: i64,
    pub month: i64,
    pub exclude_log_id: Option<i64>,
}

impl ValidatedArgs for ColonyMonthRecordsArgs {
    fn validate(&self) -> Result<(), String> {
        check_id(self.colony_id, "colonyId")?;
        if !(1970..=2100).contains(&self.year) {
            return Err("年份需在 1970–2100 之间".into());
        }
        if !(1..=12).contains(&self.month) {
            return Err("月份需在 1–12 之间".into());
        }
        if let Some(id) = self.exclude_log_id {
            check_id(id, "excludeLogId")?;
        }
        Ok(())
    }
}

// ── 打卡：log_care ───────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogCareArgs {
    pub input: CareLogInputArgs,
}

impl ValidatedArgs for LogCareArgs {
    fn validate(&self) -> Result<(), String> {
        self.input.validate()
    }
}

/// `care::CareLogInput` 的服务端校验镜像（嵌套键 snake_case，与桌面同形）。
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CareLogInputArgs {
    pub colony_id: i64,
    pub action_id: i64,
    pub happened_at: String,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub food_ids: Vec<i64>,
}

impl CareLogInputArgs {
    fn validate(&self) -> Result<(), String> {
        check_id(self.colony_id, "colony_id")?;
        check_id(self.action_id, "action_id")?;
        check_happened_at(&self.happened_at)?;
        check_note(&self.note)?;
        check_food_ids(&self.food_ids)?;
        Ok(())
    }

    pub fn into_core(self) -> care::CareLogInput {
        care::CareLogInput {
            colony_id: self.colony_id,
            action_id: self.action_id,
            happened_at: self.happened_at,
            note: self.note,
            food_ids: self.food_ids,
        }
    }
}

// ── 历史：list_logs / update_log / delete_log ────────────────────────────

#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListLogsArgs {
    pub filter: LogFilterArgs,
}

impl ValidatedArgs for ListLogsArgs {
    fn validate(&self) -> Result<(), String> {
        self.filter.validate()
    }
}

/// `care::LogFilter` 的服务端校验镜像（嵌套键 snake_case）。纯核对空串日期
/// 按「未填」处理，本层同口径放行；非空则必须能按 ISO 解析。
#[derive(Debug, Clone, PartialEq, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogFilterArgs {
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

impl LogFilterArgs {
    fn validate(&self) -> Result<(), String> {
        if let Some(v) = self.location_id {
            check_id(v, "location_id")?;
        }
        if let Some(v) = self.colony_id {
            check_id(v, "colony_id")?;
        }
        if let Some(v) = self.action_id {
            check_id(v, "action_id")?;
        }
        if let Some(s) = &self.start {
            if !s.trim().is_empty() {
                check_iso_date(s, "开始日期")?;
            }
        }
        if let Some(s) = &self.end {
            if !s.trim().is_empty() {
                check_iso_date(s, "结束日期")?;
            }
        }
        check_keyword(&self.note_keyword)?;
        if let Some(l) = self.limit {
            if !(1..=MAX_PAGE_LIMIT).contains(&l) {
                return Err(format!("分页大小 limit 需在 1–{MAX_PAGE_LIMIT} 之间（收到 {l}）"));
            }
        }
        if let Some(o) = self.offset {
            if o < 0 {
                return Err(format!("分页偏移 offset 不能为负数（收到 {o}）"));
            }
        }
        Ok(())
    }

    pub fn into_core(self) -> care::LogFilter {
        care::LogFilter {
            location_id: self.location_id,
            colony_id: self.colony_id,
            action_id: self.action_id,
            start: self.start,
            end: self.end,
            note_keyword: self.note_keyword,
            limit: self.limit,
            offset: self.offset,
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateLogArgs {
    pub id: i64,
    pub input: LogUpdateInputArgs,
}

impl ValidatedArgs for UpdateLogArgs {
    fn validate(&self) -> Result<(), String> {
        check_id(self.id, "id")?;
        self.input.validate()
    }
}

/// `care::LogUpdateInput` 的服务端校验镜像（字段 None = 保持原值）。
#[derive(Debug, Clone, PartialEq, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogUpdateInputArgs {
    #[serde(default)]
    pub occurred_at: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub action_id: Option<i64>,
    #[serde(default)]
    pub food_ids: Option<Vec<i64>>,
}

impl LogUpdateInputArgs {
    fn validate(&self) -> Result<(), String> {
        if let Some(s) = &self.occurred_at {
            if !s.trim().is_empty() {
                check_happened_at(s)?;
            }
        }
        check_note(&self.note)?;
        if let Some(v) = self.action_id {
            check_id(v, "action_id")?;
        }
        if let Some(ids) = &self.food_ids {
            check_food_ids(ids)?;
        }
        Ok(())
    }

    pub fn into_core(self) -> care::LogUpdateInput {
        care::LogUpdateInput {
            occurred_at: self.occurred_at,
            note: self.note,
            action_id: self.action_id,
            food_ids: self.food_ids,
        }
    }
}

// ── 冬眠：start / confirm_wake / add_past / update_expected_end ──────────

#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartHibernationArgs {
    pub colony_id: i64,
    pub start_date: String,
    pub expected_end_date: String,
}

impl ValidatedArgs for StartHibernationArgs {
    fn validate(&self) -> Result<(), String> {
        check_id(self.colony_id, "colonyId")?;
        check_iso_date(&self.start_date, "开始日期")?;
        check_iso_date(&self.expected_end_date, "预计结束日期")
    }
}

#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfirmWakeArgs {
    pub colony_id: i64,
    pub actual_end_date: String,
}

impl ValidatedArgs for ConfirmWakeArgs {
    fn validate(&self) -> Result<(), String> {
        check_id(self.colony_id, "colonyId")?;
        check_iso_date(&self.actual_end_date, "实际结束日期")
    }
}

#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AddPastHibernationArgs {
    pub colony_id: i64,
    pub start_date: String,
    pub end_date: String,
}

impl ValidatedArgs for AddPastHibernationArgs {
    fn validate(&self) -> Result<(), String> {
        check_id(self.colony_id, "colonyId")?;
        check_iso_date(&self.start_date, "开始日期")?;
        check_iso_date(&self.end_date, "结束日期")
    }
}

#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateExpectedEndArgs {
    pub colony_id: i64,
    pub new_expected_end_date: String,
}

impl ValidatedArgs for UpdateExpectedEndArgs {
    fn validate(&self) -> Result<(), String> {
        check_id(self.colony_id, "colonyId")?;
        check_iso_date(&self.new_expected_end_date, "预计结束日期")
    }
}

// ── 巢况登记：save / update / delete / list / digest ─────────────────────

#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveCheckinArgs {
    pub input: CheckinInputArgs,
}

impl ValidatedArgs for SaveCheckinArgs {
    fn validate(&self) -> Result<(), String> {
        self.input.validate()
    }
}

/// `nest_checkin::CheckinInput` 的服务端校验镜像（至少一项非空属业务规则，
/// 纯核 ensure_something_filled 管，本层不复制）。
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckinInputArgs {
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

impl CheckinInputArgs {
    fn validate(&self) -> Result<(), String> {
        check_id(self.colony_id, "colony_id")?;
        check_iso_date(&self.date, "登记日期")?;
        check_count(self.queen_count, "蚁后数")?;
        check_count(self.worker_count, "工蚁数")?;
        check_note(&self.note)
    }

    pub fn into_core(self) -> nest_checkin::CheckinInput {
        nest_checkin::CheckinInput {
            colony_id: self.colony_id,
            date: self.date,
            queen_count: self.queen_count,
            worker_count: self.worker_count,
            moved_nest: self.moved_nest,
            note: self.note,
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateCheckinArgs {
    pub id: i64,
    pub input: CheckinUpdateInputArgs,
}

impl ValidatedArgs for UpdateCheckinArgs {
    fn validate(&self) -> Result<(), String> {
        check_id(self.id, "id")?;
        self.input.validate()
    }
}

/// `nest_checkin::CheckinUpdateInput` 的服务端校验镜像（全量覆盖语义）。
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckinUpdateInputArgs {
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

impl CheckinUpdateInputArgs {
    fn validate(&self) -> Result<(), String> {
        check_iso_date(&self.date, "登记日期")?;
        check_count(self.queen_count, "蚁后数")?;
        check_count(self.worker_count, "工蚁数")?;
        check_note(&self.note)
    }

    pub fn into_core(self) -> nest_checkin::CheckinUpdateInput {
        nest_checkin::CheckinUpdateInput {
            date: self.date,
            queen_count: self.queen_count,
            worker_count: self.worker_count,
            moved_nest: self.moved_nest,
            note: self.note,
        }
    }
}

// ── 测试：schema 层单测（形状垃圾全表；端到端真请求在 webui_server tests）──

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 解析 + 校验一步（与派发层同口径）。
    fn checked<A: ValidatedArgs>(args: &serde_json::Value) -> Result<A, String> {
        let a: A = parse(args)?;
        a.validate()?;
        Ok(a)
    }

    #[test]
    fn empty_args_accept_only_empty_object() {
        assert!(checked::<EmptyArgs>(&json!({})).is_ok());
        let err = checked::<EmptyArgs>(&json!({"foo": 1})).unwrap_err();
        assert_eq!(err, "参数包含未知字段：foo");
    }

    #[test]
    fn log_care_happy_path_parses_into_core_shape() {
        let a = checked::<LogCareArgs>(&json!({
            "input": {
                "colony_id": 3, "action_id": 1,
                "happened_at": "2026-09-19 08:30:00",
                "note": "喂了", "food_ids": [1, 2],
            }
        }))
        .unwrap();
        let core = a.input.into_core();
        assert_eq!(core.colony_id, 3);
        assert_eq!(core.happened_at, "2026-09-19 08:30:00");
        assert_eq!(core.food_ids, vec![1, 2]);
    }

    #[test]
    fn log_care_garbage_rejected_with_human_messages() {
        // 缺字段
        let err = checked::<LogCareArgs>(&json!({})).unwrap_err();
        assert_eq!(err, "缺少必填参数：input");
        // 嵌套缺字段
        let err = checked::<LogCareArgs>(&json!({"input": {"colony_id": 1}})).unwrap_err();
        assert!(err.contains("缺少必填参数"), "实际：{err}");
        // 错型
        let err = checked::<LogCareArgs>(&json!({
            "input": {"colony_id": "abc", "action_id": 1, "happened_at": "2026-09-19"}
        }))
        .unwrap_err();
        assert!(err.contains("类型或取值不正确"), "实际：{err}");
        // 负数 id
        let err = checked::<LogCareArgs>(&json!({
            "input": {"colony_id": -2, "action_id": 1, "happened_at": "2026-09-19"}
        }))
        .unwrap_err();
        assert_eq!(err, "colony_id 必须是正整数（收到 -2）");
        // 未知嵌套字段
        let err = checked::<LogCareArgs>(&json!({
            "input": {"colony_id": 1, "action_id": 1, "happened_at": "2026-09-19", "bogus": 1}
        }))
        .unwrap_err();
        assert_eq!(err, "参数包含未知字段：bogus");
        // 备注超长
        let err = checked::<LogCareArgs>(&json!({
            "input": {"colony_id": 1, "action_id": 1, "happened_at": "2026-09-19", "note": "超".repeat(MAX_NOTE_CHARS + 1)}
        }))
        .unwrap_err();
        assert!(err.contains("备注长度超过上限"), "实际：{err}");
        // 发生时间垃圾 / 超长
        let err = checked::<LogCareArgs>(&json!({
            "input": {"colony_id": 1, "action_id": 1, "happened_at": "不是时间"}
        }))
        .unwrap_err();
        assert!(err.contains("发生时间格式"), "实际：{err}");
        let err = checked::<LogCareArgs>(&json!({
            "input": {"colony_id": 1, "action_id": 1, "happened_at": "x".repeat(MAX_DATETIME_CHARS + 1)}
        }))
        .unwrap_err();
        assert!(err.contains("发生时间长度"), "实际：{err}");
        // 食物：负数与超种数
        let err = checked::<LogCareArgs>(&json!({
            "input": {"colony_id": 1, "action_id": 1, "happened_at": "2026-09-19", "food_ids": [0]}
        }))
        .unwrap_err();
        assert!(err.contains("food_ids[0]"), "实际：{err}");
        let err = checked::<LogCareArgs>(&json!({
            "input": {"colony_id": 1, "action_id": 1, "happened_at": "2026-09-19",
                      "food_ids": (1i64..=(MAX_FOOD_IDS as i64 + 1)).collect::<Vec<i64>>()}
        }))
        .unwrap_err();
        assert!(err.contains("最多挂"), "实际：{err}");
    }

    #[test]
    fn log_filter_pagination_and_dates() {
        let a = checked::<ListLogsArgs>(&json!({
            "filter": {"colony_id": 1, "limit": 50, "offset": 100,
                        "start": "2026-01-01", "end": "", "note_keyword": "喂"}
        }))
        .unwrap();
        // 空串结束日期 = 未填（纯核同口径放行）
        assert_eq!(a.filter.end.as_deref(), Some(""));
        // limit 下界外 / 上界外
        let err = checked::<ListLogsArgs>(&json!({"filter": {"limit": 0}})).unwrap_err();
        assert!(err.contains("limit"), "实际：{err}");
        let err = checked::<ListLogsArgs>(&json!({"filter": {"limit": MAX_PAGE_LIMIT + 1}})).unwrap_err();
        assert!(err.contains("limit"), "实际：{err}");
        // 负 offset
        let err = checked::<ListLogsArgs>(&json!({"filter": {"offset": -5}})).unwrap_err();
        assert!(err.contains("offset"), "实际：{err}");
        // 日期垃圾 / 超范围
        let err = checked::<ListLogsArgs>(&json!({"filter": {"start": "01/02/2026"}})).unwrap_err();
        assert!(err.contains("开始日期格式"), "实际：{err}");
        let err = checked::<ListLogsArgs>(&json!({"filter": {"end": "2150-01-01"}})).unwrap_err();
        assert!(err.contains("超出合理范围"), "实际：{err}");
        // 关键词超长
        let err = checked::<ListLogsArgs>(&json!({"filter": {"note_keyword": "词".repeat(MAX_KEYWORD_CHARS + 1)}}))
            .unwrap_err();
        assert!(err.contains("关键词长度"), "实际：{err}");
        // 缺 filter 整体
        let err = checked::<ListLogsArgs>(&json!({})).unwrap_err();
        assert_eq!(err, "缺少必填参数：filter");
    }

    #[test]
    fn update_log_none_means_keep_and_validates_given() {
        // 全 None（保持原值）合法
        assert!(checked::<UpdateLogArgs>(&json!({"id": 5, "input": {}})).is_ok());
        // 负 id / 负 action_id
        let err = checked::<UpdateLogArgs>(&json!({"id": 0, "input": {}})).unwrap_err();
        assert_eq!(err, "id 必须是正整数（收到 0）");
        let err = checked::<UpdateLogArgs>(&json!({"id": 5, "input": {"action_id": -1}})).unwrap_err();
        assert!(err.contains("action_id"), "实际：{err}");
        // occurred_at 给了就查形状（空串 = 纯核语义的保持原值，放行）
        assert!(checked::<UpdateLogArgs>(&json!({"id": 5, "input": {"occurred_at": ""}})).is_ok());
        let err = checked::<UpdateLogArgs>(&json!({"id": 5, "input": {"occurred_at": "垃圾"}}))
            .unwrap_err();
        assert!(err.contains("发生时间格式"), "实际：{err}");
        // food_ids Some(空) = 清空关联，合法形状
        assert!(checked::<UpdateLogArgs>(&json!({"id": 5, "input": {"food_ids": []}})).is_ok());
    }

    #[test]
    fn hibernation_args_use_camel_case_top_level_keys() {
        let a = checked::<StartHibernationArgs>(&json!({
            "colonyId": 3, "startDate": "2026-01-10", "expectedEndDate": "2026-03-01"
        }))
        .unwrap();
        assert_eq!(a.colony_id, 3);
        // snake_case 顶层键 = 未知字段（契约：顶层 camelCase，与桌面 Tauri 映射一致）
        let err = checked::<StartHibernationArgs>(&json!({
            "colony_id": 3, "start_date": "2026-01-10", "expected_end_date": "2026-03-01"
        }))
        .unwrap_err();
        assert_eq!(err, "参数包含未知字段：colony_id");
        // 缺字段 / 日期垃圾
        let err = checked::<StartHibernationArgs>(&json!({"colonyId": 3, "startDate": "2026-01-10"}))
            .unwrap_err();
        assert_eq!(err, "缺少必填参数：expectedEndDate");
        let err = checked::<ConfirmWakeArgs>(&json!({"colonyId": 3, "actualEndDate": "20261301"}))
            .unwrap_err();
        assert!(err.contains("实际结束日期格式"), "实际：{err}");
        // 补录冬眠期 / 改预计出眠日
        assert!(checked::<AddPastHibernationArgs>(&json!({
            "colonyId": 3, "startDate": "2025-12-01", "endDate": "2026-02-20"
        }))
        .is_ok());
        assert!(checked::<UpdateExpectedEndArgs>(&json!({
            "colonyId": 3, "newExpectedEndDate": "2026-04-01"
        }))
        .is_ok());
    }

    #[test]
    fn checkin_args_validate_shape_business_rules_stay_in_core() {
        // 只有日期（全空登记）形状合法——「至少填一项」是纯核业务规则
        let a = checked::<SaveCheckinArgs>(&json!({"input": {"colony_id": 1, "date": "2026-09-19"}}))
            .unwrap();
        assert!(!a.input.moved_nest, "缺省 moved_nest=false");
        // 负数蚁后数 / 超范围工蚁数
        let err = checked::<SaveCheckinArgs>(&json!({
            "input": {"colony_id": 1, "date": "2026-09-19", "queen_count": -3}
        }))
        .unwrap_err();
        assert!(err.contains("蚁后数不能为负数"), "实际：{err}");
        let err = checked::<SaveCheckinArgs>(&json!({
            "input": {"colony_id": 1, "date": "2026-09-19", "worker_count": MAX_COUNT + 1}
        }))
        .unwrap_err();
        assert!(err.contains("工蚁数超出合理范围"), "实际：{err}");
        // 日期垃圾 / 日期超年段
        let err = checked::<SaveCheckinArgs>(&json!({
            "input": {"colony_id": 1, "date": "2026-02-30"}
        }))
        .unwrap_err();
        assert!(err.contains("登记日期格式"), "实际：{err}");
        let err = checked::<SaveCheckinArgs>(&json!({
            "input": {"colony_id": 1, "date": "1500-01-01"}
        }))
        .unwrap_err();
        assert!(err.contains("超出合理范围"), "实际：{err}");
        // 编辑（无 colony_id 字段）
        let a = checked::<UpdateCheckinArgs>(&json!({
            "id": 7, "input": {"date": "2026-09-01", "queen_count": 1, "worker_count": null, "moved_nest": true}
        }))
        .unwrap();
        assert!(a.input.moved_nest);
        let core = a.input.into_core();
        assert_eq!(core.queen_count, Some(1));
        assert_eq!(core.worker_count, None);
    }

    #[test]
    fn id_and_colony_id_args() {
        assert!(checked::<IdArgs>(&json!({"id": 1})).is_ok());
        assert_eq!(
            checked::<IdArgs>(&json!({"id": -1})).unwrap_err(),
            "id 必须是正整数（收到 -1）"
        );
        let a = checked::<ColonyIdArgs>(&json!({"colonyId": 9})).unwrap();
        assert_eq!(a.colony_id, 9);
        assert!(checked::<ColonyIdArgs>(&json!({})).unwrap_err().contains("缺少必填参数"));
    }

    #[test]
    fn humanize_covers_type_and_fallback_shapes() {
        // 直接压 humanize 的分支（经 parse 入口）
        let err = parse::<IdArgs>(&json!({"id": "abc"})).unwrap_err();
        assert!(err.contains("类型或取值不正确"), "实际：{err}");
        // args 不是对象形状（长度不符的序列）也按人话拒绝，绝不 panic
        let err = parse::<IdArgs>(&json!([])).unwrap_err();
        assert!(err.contains("不合法"), "实际：{err}");
    }
}
