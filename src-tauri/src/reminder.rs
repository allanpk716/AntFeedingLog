//! 提醒引擎（票 06）：算「今天该发什么」+ 台账去重 + 补发窗口。
//!
//! 纯函数核心只吃 `&Connection`、today 可注入，cargo test 直接覆盖：
//! - [`compute_due_reminders`]：按窝状态 × 启用中的提醒类操作，算出「今天应发」清单（不落库）；
//! - [`run_check`]：读设置 → 算 → 总开关过滤 → 台账去重落库，返回真正要发的
//!   桌面条目与待发手机推送（发送/了结在锁外由调用方驱动）；
//!
//! 通知发送、30 分钟调度与托盘接线是薄封装（lib.rs 调度器，系统行为不进单测）。
//!
//! 行为对齐 spec 评审附录规则 1-4：
//! - 超期：仅活跃窝、仅提醒类、距上次 > 建议间隔才发；基准日 = 今天 − 建议间隔，
//!   随今天逐日推进 → 天然「每个超期日最多一条」，同日重查被台账唯一键挡住；
//! - 食物超期（反馈第二轮 F3）：同超期口径，但按「该食物」自己的周期与喂食史
//!   各算各的（基准日 = 今天 − 食物周期），台账加 food 维度去重；冬眠同样静音；
//! - 临近出眠：冬眠中的窝、今天 ≥ 预计出眠日 − 提前天数，基准日 = 预计出眠日，一次；
//! - 出眠日：冬眠中的窝、今天 ≥ 预计出眠日，基准日 = 预计出眠日，一次；
//! - 补发上限（规则 3）：临近/出眠的基准日在近 7 天内才补，更旧不补；
//!   超期的基准日锚定今天，重开后天然只发「当前这一天」的一条；
//! - 提前出眠后状态回活跃，冬眠类提醒随之停发（规则 4）；
//! - 改预计出眠日：未发的按新日期自然重算（基准日跟着新出眠日走、不撞旧台账），
//!   已发的不追回（旧行保留、也不重发）；
//! - 开关关 = 完全静默（不发送也不写台账），重开后按去重规则正常发。

use chrono::Duration;
use rusqlite::{params, Connection};
use serde::Serialize;

use crate::settings::{self, AppSettings};

/// 补发窗口：基准日在近 7 天内才补发，更旧不补（评审附录规则 3）。
pub const BACKFILL_WINDOW_DAYS: i64 = 7;

/// 手机推送补发窗口（天）：登记起两天内没发成功就放弃（Q8=A，超窗防陈年补发）。
pub const PUSHOVER_RETRY_DAYS: i64 = 2;

/// 一个待发推送任务：IO 在锁外做，成功后拿 ledger_id 去 settle。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PushJob {
    pub ledger_id: i64,
    pub title: String,
    pub body: String,
}

/// run_check 返回体：toasts = 本轮新登记（该发桌面通知）；push_jobs = 待发/补发的
/// 手机推送（新登记 + 窗口内未了结）。发送与了结都在锁外由调用方驱动。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CheckOutcome {
    pub toasts: Vec<Reminder>,
    pub push_jobs: Vec<PushJob>,
}

// ── DTO ──────────────────────────────────────────────────────────────────

/// 提醒种类（reminder_ledger.kind 同款）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReminderKind {
    /// 提醒类操作超期（仅活跃窝）
    Overdue,
    /// 食物超期：距上次喂「该食物」超过它的建议间隔（仅活跃窝，F3）
    FoodOverdue,
    /// 临近出眠（预计出眠日前 N 天起，一次）
    ApproachingWake,
    /// 出眠日当天（一次）
    WakeDay,
}

impl ReminderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ReminderKind::Overdue => "overdue",
            ReminderKind::FoodOverdue => "food_overdue",
            ReminderKind::ApproachingWake => "approaching_wake",
            ReminderKind::WakeDay => "wake_day",
        }
    }
}

/// 一条「该发的提醒」。台账身份 = (colony_id, kind, action_id, food_id, base_date)。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Reminder {
    pub colony_id: i64,
    pub colony_name: String,
    pub kind: ReminderKind,
    /// 仅超期类（操作层/食物层）非空。
    pub action_id: Option<i64>,
    pub action_name: Option<String>,
    /// 仅食物层（FoodOverdue）非空（F3）。
    pub food_id: Option<i64>,
    pub food_name: Option<String>,
    /// 去重基准日：超期 = 今天 − 建议间隔；临近/出眠日 = 预计出眠日。
    pub base_date: String,
    /// 超期类：距上次天数（通知文案用）。
    pub days_since_last: Option<i64>,
    pub suggested_interval_days: Option<i64>,
}

impl Reminder {
    /// 通知文案（纯函数）：（标题，正文）。
    pub fn notification_text(&self) -> (String, String) {
        match self.kind {
            ReminderKind::Overdue => {
                let action = self.action_name.as_deref().unwrap_or("操作");
                let mut body = format!("「{}」已 ", self.colony_name);
                match self.days_since_last {
                    Some(d) => body.push_str(&format!("{d} 天")),
                    None => body.push_str("有一阵子"),
                }
                body.push_str(&format!("没{action}"));
                if let Some(n) = self.suggested_interval_days {
                    body.push_str(&format!("（建议 {n} 天一次）"));
                }
                (format!("{action}超期"), body)
            }
            ReminderKind::FoodOverdue => {
                let food = self.food_name.as_deref().unwrap_or("食物");
                let mut body = format!("「{}」已 ", self.colony_name);
                match self.days_since_last {
                    Some(d) => body.push_str(&format!("{d} 天")),
                    None => body.push_str("有一阵子"),
                }
                body.push_str(&format!("没喂{food}"));
                if let Some(n) = self.suggested_interval_days {
                    body.push_str(&format!("（建议 {n} 天一次）"));
                }
                (format!("该喂{food}了"), body)
            }
            ReminderKind::ApproachingWake => (
                "临近出眠".into(),
                format!(
                    "「{}」预计 {} 出眠，快恢复照顾吧",
                    self.colony_name, self.base_date
                ),
            ),
            ReminderKind::WakeDay => (
                "出眠日".into(),
                format!(
                    "「{}」的预计出眠日 {} 到了，该恢复照顾了",
                    self.colony_name, self.base_date
                ),
            ),
        }
    }
}

// ── 核心：今天该发什么（不落库）──────────────────────────────────────────

fn parse_iso(s: &str) -> Result<chrono::NaiveDate, String> {
    chrono::NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d")
        .map_err(|_| format!("日期格式应为 YYYY-MM-DD：{s}"))
}

fn db_err(e: rusqlite::Error) -> String {
    format!("数据库操作失败: {e}")
}

/// 算出「今天应发」的提醒清单。`days_ahead` = 临近出眠提前天数（负值按 0）。
/// 已结束的窝全不发；冬眠中的窝不发超期；活跃窝不发冬眠类。
/// 脏数据（冬眠状态无开放段 / 出眠日解析失败）按窝跳过，不毒死整轮。
pub fn compute_due_reminders(
    conn: &Connection,
    today: &str,
    days_ahead: i64,
) -> Result<Vec<Reminder>, String> {
    let today = parse_iso(today)?;
    let ahead = Duration::days(days_ahead.max(0));
    let mut due = Vec::new();

    let colonies: Vec<(i64, String, String)> = {
        let mut stmt = conn
            .prepare("SELECT id, name, status FROM colony ORDER BY id")
            .map_err(db_err)?;
        let rows = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .map_err(db_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db_err)?;
        rows
    };

    for (colony_id, colony_name, status) in colonies {
        match status.as_str() {
            "active" => {
                // F3：喂食 tile 即使统一层不红也要扫食物层（食物层可独立超期）
                for tile in crate::care::tiles_for_colony(conn, colony_id, &today.to_string())?
                    .into_iter()
                    .filter(|t| t.overdue || t.is_feeding)
                {
                    // 统一层只看操作层自身的超期（tile.overdue 已含食物层，不能用它判定，
                    // 否则食物层顶红时会把"距上次 2 天 ≤ 3"也当超期发出去）
                    if crate::care::is_overdue(&tile.kind, tile.days_since_last, tile.suggested_interval_days) {
                        let interval = tile.suggested_interval_days.unwrap_or_default();
                        let base = (today - Duration::days(interval.max(0)))
                            .format("%Y-%m-%d")
                            .to_string();
                        due.push(Reminder {
                            colony_id,
                            colony_name: colony_name.clone(),
                            kind: ReminderKind::Overdue,
                            action_id: Some(tile.action_id),
                            action_name: Some(tile.name.clone()),
                            food_id: None,
                            food_name: None,
                            base_date: base,
                            days_since_last: tile.days_since_last,
                            suggested_interval_days: tile.suggested_interval_days,
                        });
                    }
                    // 食物层（F3）：与统一层各记各的，台账按 food 维度去重
                    for f in tile.foods.iter().filter(|f| f.overdue) {
                        let Some(interval) = f.suggested_interval_days else {
                            continue;
                        };
                        let base = (today - Duration::days(interval.max(0))).format("%Y-%m-%d").to_string();
                        due.push(Reminder {
                            colony_id,
                            colony_name: colony_name.clone(),
                            kind: ReminderKind::FoodOverdue,
                            action_id: Some(tile.action_id),
                            action_name: Some(tile.name.clone()),
                            food_id: Some(f.food_id),
                            food_name: Some(f.name.clone()),
                            base_date: base,
                            days_since_last: f.days_since_last,
                            suggested_interval_days: f.suggested_interval_days,
                        });
                    }
                }
            }
            "hibernating" => {
                // 冬眠中的窝只发冬眠类；无开放段（脏状态）静默跳过
                let Some(seg) = crate::hibernation::open_segment(conn, colony_id)? else {
                    continue;
                };
                let Ok(end) = parse_iso(&seg.expected_end_date) else {
                    continue; // 脏出眠日跳过该窝
                };
                let base = end.format("%Y-%m-%d").to_string();
                let since_end = (today - end).num_days();
                // 临近：预计出眠日前 N 天起（基准日=出眠日，台账去重保证一次）
                if today >= end - ahead && since_end <= BACKFILL_WINDOW_DAYS {
                    due.push(Reminder {
                        colony_id,
                        colony_name: colony_name.clone(),
                        kind: ReminderKind::ApproachingWake,
                        action_id: None,
                        action_name: None,
                        food_id: None,
                        food_name: None,
                        base_date: base.clone(),
                        days_since_last: None,
                        suggested_interval_days: None,
                    });
                }
                // 出眠日：当天一次；错过 ≤ 7 天补发一条
                if today >= end && since_end <= BACKFILL_WINDOW_DAYS {
                    due.push(Reminder {
                        colony_id,
                        colony_name: colony_name.clone(),
                        kind: ReminderKind::WakeDay,
                        action_id: None,
                        action_name: None,
                        food_id: None,
                        food_name: None,
                        base_date: base,
                        days_since_last: None,
                        suggested_interval_days: None,
                    });
                }
            }
            _ => {} // ended 与未知状态：全不发
        }
    }
    Ok(due)
}

// ── 核心：一整轮检查（总开关 + 台账去重落库 + 推送补发）─────────────────

/// 一轮完整检查：读设置 → 算应发 → 台账去重落库，返回真正要发的条目与待发推送。
/// 总开关关 = 完全静默（不发也不写台账）。`now` 供台账 sent_at。
/// 反馈第二轮 Q7/Q9：分类子开关作废，总开关是唯一闸门；本函数零网络 IO，
/// 手机推送的发送与了结都由调用方在锁外做（settle_pushover）。
pub fn run_check(conn: &Connection, today: &str, now: &str) -> Result<CheckOutcome, String> {
    let s: AppSettings = settings::get_settings(conn)?;
    if !s.notify_master_enabled {
        return Ok(CheckOutcome::default());
    }
    let due = compute_due_reminders(conn, today, s.wake_remind_days_ahead)?;

    let mut outcome = CheckOutcome::default();
    if !due.is_empty() {
        let tx = conn.unchecked_transaction().map_err(db_err)?;
        for r in &due {
            let (title, body) = r.notification_text();
            // 同身份已发过 → OR IGNORE 跳过（rowcount=0），不重发；
            // 通知文案随行落库（快照），补发时不重算
            let inserted = tx
                .execute(
                    "INSERT OR IGNORE INTO reminder_ledger (colony_id, kind, action_id, food_id, base_date, sent_at, push_title, push_body)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![r.colony_id, r.kind.as_str(), r.action_id, r.food_id, r.base_date, now, &title, &body],
                )
                .map_err(db_err)?;
            if inserted == 1 {
                outcome.toasts.push(r.clone());
                outcome.push_jobs.push(PushJob {
                    ledger_id: tx.last_insert_rowid(),
                    title,
                    body,
                });
            }
        }
        tx.commit().map_err(db_err)?;
    }

    // 事务提交后跑补发扫描（新登记的行已在上面收编，这里按 id 去重不重复收）
    for job in collect_retry_jobs(conn, today)? {
        if !outcome.push_jobs.iter().any(|j| j.ledger_id == job.ledger_id) {
            outcome.push_jobs.push(job);
        }
    }
    Ok(outcome)
}

fn collect_retry_jobs(conn: &Connection, today: &str) -> Result<Vec<PushJob>, String> {
    // 先了结超窗/无文案的历史行，再捞窗口内未了结的
    conn.execute(
        "UPDATE reminder_ledger SET pushover_done = 1
         WHERE pushover_done = 0
           AND date(sent_at) < date(?1, ?2)",
        params![today, format!("-{PUSHOVER_RETRY_DAYS} day")],
    )
    .map_err(db_err)?;
    let mut stmt = conn
        .prepare(
            "SELECT id, push_title, push_body FROM reminder_ledger
             WHERE pushover_done = 0 AND push_title IS NOT NULL",
        )
        .map_err(db_err)?;
    let jobs = stmt
        .query_map([], |row| {
            Ok(PushJob {
                ledger_id: row.get(0)?,
                title: row.get(1)?,
                body: row.get(2)?,
            })
        })
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?;
    Ok(jobs)
}

/// 手机侧了结（送达或放弃）——由调用方在发送拿到结果后调用。
pub fn settle_pushover(conn: &Connection, ids: &[i64]) -> Result<(), String> {
    for id in ids {
        conn.execute("UPDATE reminder_ledger SET pushover_done = 1 WHERE id = ?1", params![id])
            .map_err(db_err)?;
    }
    Ok(())
}

// ── 托盘 tooltip 一句话概要（纯函数）─────────────────────────────────────

/// 托盘 tooltip 一句话概要（spec 界面节示例："2 窝活跃 · 大头一号喂食超期 1 天"）。
/// 无窝 → "暂无窝"；否则 = 活跃数 +（冬眠数 > 0 时）+ 逐条超期摘要（仅活跃窝，
/// 冬眠窝静音）。F3：设周期食物各自超期再补「某窝该喂某食物了」一条，与统一层并存。
/// Windows tooltip 上限 128 字符，超长按字符截断。
pub fn tray_summary(colonies: &[crate::colony::Colony]) -> String {
    if colonies.is_empty() {
        return "暂无窝".into();
    }
    let mut parts: Vec<String> = Vec::new();
    let active = colonies.iter().filter(|c| c.status == "active").count();
    parts.push(format!("{active} 窝活跃"));
    let hibernating = colonies.iter().filter(|c| c.status == "hibernating").count();
    if hibernating > 0 {
        parts.push(format!("{hibernating} 窝冬眠中"));
    }
    for c in colonies.iter().filter(|c| c.status == "active") {
        for t in c.actions.iter().filter(|t| t.overdue) {
            parts.push(format!(
                "{}{}超期 {} 天",
                c.name,
                t.name,
                t.days_since_last.unwrap_or(0)
            ));
        }
        // 食物层超期（F3）：统一层超期维持原句式，两者可并存
        for t in c.actions.iter() {
            for f in t.foods.iter().filter(|f| f.overdue) {
                parts.push(format!("{}该喂{}了", c.name, f.name));
            }
        }
    }
    let text = parts.join(" · ");
    if text.chars().count() > 128 {
        text.chars().take(128).collect()
    } else {
        text
    }
}

// ── 薄封装：通知发送 / 调度 / 托盘（Tauri 侧，系统行为不进单测）─────────

use tauri::Manager;
use tauri_plugin_notification::NotificationExt;

/// 主托盘 id（refresh_tray_tooltip 按它找回托盘句柄）。
pub const TRAY_ID: &str = "ant-main-tray";

/// 发一条系统通知。失败静默忽略：通知是副产物，不应打断调度循环。
pub fn send_notification(handle: &tauri::AppHandle, r: &Reminder) {
    let (title, body) = r.notification_text();
    let _ = handle.notification().builder().title(title).body(body).show();
}

/// 测试通知结果（分渠道回显；pushover=None 表示未配置）。
#[derive(Debug, Clone, Serialize)]
pub struct PushoverTestResult {
    pub ok: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TestNotifyOutcome {
    pub desktop_ok: bool,
    pub desktop_error: Option<String>,
    pub pushover: Option<PushoverTestResult>,
}

pub fn send_test_notification_dual(handle: &tauri::AppHandle) -> TestNotifyOutcome {
    let desktop = handle
        .notification()
        .builder()
        .title("测试通知")
        .body("蚂蚁饲养记录：桌面通道正常。")
        .show()
        .map_err(|e| format!("发送测试通知失败: {e}"));
    let pushover = crate::pushover::PushoverConfig::from_env().map(|cfg| {
        match crate::pushover::send(&cfg, "测试通知", "蚂蚁饲养记录：手机通道正常。") {
            Ok(()) => PushoverTestResult { ok: true, error: None },
            Err(e) => PushoverTestResult { ok: false, error: Some(e) },
        }
    });
    TestNotifyOutcome {
        desktop_ok: desktop.is_ok(),
        desktop_error: desktop.err(),
        pushover,
    }
}

/// 一轮「检查 → 发通知 → 刷新托盘概要」。启动首查与调度线程共用。
/// 单轮失败不致命，静默等下一个 30 分钟周期。
/// 锁纪律（反馈第二轮 F4）：run_check 在锁内零网络 IO，桌面通知与 Pushover
/// 发送、推送了结（settle_pushover）全在锁外。
pub fn check_and_notify(handle: &tauri::AppHandle) {
    let Some(state) = handle.try_state::<crate::DbState>() else {
        return;
    };
    let today = crate::colony::today_iso();
    let now = crate::care::now_local();
    let outcome = {
        let Ok(conn) = state.0.lock() else { return };
        match run_check(&conn, &today, &now) {
            Ok(outcome) => outcome,
            Err(_) => return,
        }
    };
    for r in &outcome.toasts {
        send_notification(handle, r);
    }
    if !outcome.push_jobs.is_empty() {
        let cfg = crate::pushover::PushoverConfig::from_env();
        let mut settled: Vec<i64> = Vec::new();
        for job in &outcome.push_jobs {
            if let Some(cfg) = &cfg {
                match crate::pushover::send(cfg, &job.title, &job.body) {
                    Ok(()) => settled.push(job.ledger_id),
                    Err(e) => eprintln!("[pushover] 发送失败（下轮重试）: {e}"),
                }
            }
        }
        if !settled.is_empty() {
            if let Ok(conn) = state.0.lock() {
                let _ = settle_pushover(&conn, &settled);
            }
        }
    }
    refresh_tray_tooltip(handle);
}

/// 托盘 tooltip 概要与首页同源：list_colonies → tray_summary。
pub fn refresh_tray_tooltip(handle: &tauri::AppHandle) {
    let Some(state) = handle.try_state::<crate::DbState>() else {
        return;
    };
    let today = crate::colony::today_iso();
    let text = {
        let Ok(conn) = state.0.lock() else { return };
        crate::colony::list_colonies(&conn, &today)
            .map(|colonies| tray_summary(&colonies))
            .unwrap_or_else(|_| "蚂蚁饲养记录".to_string())
    };
    if let Some(tray) = handle.tray_by_id(TRAY_ID) {
        let _ = tray.set_tooltip(Some(text));
    }
}

/// 启动调度：启动即查一次，此后每 30 分钟一轮（评审附录规则 1）。
/// 独立 std 线程 + sleep：锁竞争每半小时一次，不值得占 async 运行时。
/// 每轮包 catch_unwind（票 09 停靠 A）：单轮 panic 打日志后继续下一轮，
/// 调度线程不再无声死掉。
pub fn spawn_scheduler(handle: tauri::AppHandle) {
    std::thread::spawn(move || loop {
        let h = handle.clone();
        let round = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            check_and_notify(&h);
        }));
        if let Err(panic) = round {
            eprintln!("[reminder] 本轮提醒检查 panic（已跳过，下个 30 分钟周期重试）: {panic:?}");
        }
        std::thread::sleep(std::time::Duration::from_secs(30 * 60));
    });
}

/// 建托盘：悬停看概要 tooltip；右键菜单 显示主窗口 / 退出（spec 用户故事 19）。
pub fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::TrayIconBuilder;

    let show = MenuItem::with_id(app, "show", "显示主窗口", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;

    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "quit" => app.exit(0),
            "show" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.unminimize();
                    let _ = window.set_focus();
                }
            }
            _ => {}
        })
        .tooltip("蚂蚁饲养记录");
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

// ── 测试：只测外部行为（spec「Testing Decisions」）────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::AppSettings;

    /// 建内存库并迁移到最新 schema（票 01 地基）。
    fn mem_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("内存库打开失败");
        crate::db::migrate(&conn).expect("迁移失败");
        conn
    }

    const TODAY: &str = "2026-09-18";
    const NOW: &str = "2026-09-18 08:00:00";

    fn colony(conn: &Connection, name: &str, status: &str) -> i64 {
        conn.execute(
            "INSERT INTO colony (name, start_date, status) VALUES (?1, '2026-01-20', ?2)",
            params![name, status],
        )
        .expect("建窝失败");
        conn.last_insert_rowid()
    }

    fn feed(conn: &Connection, colony_id: i64, action: &str, happened_at: &str) {
        crate::care::log_care(
            conn,
            &crate::care::CareLogInput {
                colony_id,
                action_id: action_id(conn, action),
                happened_at: happened_at.into(),
                note: None,
                food_ids: vec![],
            },
            NOW,
        )
        .expect("记账失败");
    }

    /// 喂食并挂食物（F3 食物层场景用；参考 dict.rs 的 log_feeding）。
    fn feed_foods(conn: &Connection, colony_id: i64, happened_at: &str, foods: &[&str]) {
        crate::care::log_care(
            conn,
            &crate::care::CareLogInput {
                colony_id,
                action_id: action_id(conn, "喂食"),
                happened_at: happened_at.into(),
                note: None,
                food_ids: foods.iter().map(|f| food_id(conn, f)).collect(),
            },
            NOW,
        )
        .expect("记账失败");
    }

    fn food_id(conn: &Connection, name: &str) -> i64 {
        conn.query_row("SELECT id FROM food WHERE name = ?1", params![name], |r| r.get(0))
            .expect("查食物失败")
    }

    fn action_id(conn: &Connection, name: &str) -> i64 {
        conn.query_row("SELECT id FROM care_action WHERE name = ?1", params![name], |r| {
            r.get(0)
        })
        .expect("查操作失败")
    }

    /// 直插开放冬眠段（绕过应用层校验，搭场景用）。
    fn open_seg(conn: &Connection, colony_id: i64, expected_end: &str) {
        conn.execute(
            "INSERT INTO hibernation (colony_id, start_date, expected_end_date)
             VALUES (?1, '2026-08-01', ?2)",
            params![colony_id, expected_end],
        )
        .expect("插开放段失败");
    }

    fn set_ahead(conn: &Connection, days: i64) {
        settings::set_settings(
            conn,
            &AppSettings {
                wake_remind_days_ahead: days,
                ..Default::default()
            },
        )
        .unwrap();
    }

    fn ledger_rows(conn: &Connection) -> Vec<(String, Option<i64>, String, Option<i64>)> {
        let mut stmt = conn
            .prepare("SELECT kind, action_id, base_date, food_id FROM reminder_ledger ORDER BY id")
            .unwrap();
        stmt.query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
    }

    fn ledger_count(conn: &Connection) -> i64 {
        conn.query_row("SELECT COUNT(*) FROM reminder_ledger", [], |r| r.get(0))
            .unwrap()
    }

    fn day(s: &str) -> chrono::NaiveDate {
        parse_iso(s).unwrap()
    }

    fn fmt(d: chrono::NaiveDate) -> String {
        d.format("%Y-%m-%d").to_string()
    }

    // ── 超期：一天一条 + 逐日推进 ──

    #[test]
    fn overdue_fires_once_per_day_and_repeats_next_day() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号", "active");
        // 喂食 4 天前（建议 3 → 超 1 天）；垃圾清理 4 天前（建议 7 → 不超）
        feed(&conn, c, "喂食", "2026-09-14 20:00:00");
        feed(&conn, c, "垃圾清理", "2026-09-14 20:00:00");

        let sent = run_check(&conn, TODAY, NOW).unwrap().toasts;
        assert_eq!(sent.len(), 1, "只喂食超期");
        assert_eq!(sent[0].kind, ReminderKind::Overdue);
        assert_eq!(sent[0].action_id, Some(action_id(&conn, "喂食")));
        assert_eq!(sent[0].base_date, fmt(day(TODAY) - Duration::days(3)));
        assert_eq!(sent[0].days_since_last, Some(4));

        // 同日再查：台账身份相同 → 不重发
        let again = run_check(&conn, TODAY, "2026-09-18 12:00:00").unwrap().toasts;
        assert!(again.is_empty(), "同日不重发");
        assert_eq!(ledger_count(&conn), 1);

        // 第二天仍超期：新基准日 → 再发一条（每个超期日最多一条）
        let tomorrow = fmt(day(TODAY) + Duration::days(1));
        let sent = run_check(&conn, &tomorrow, "2026-09-19 08:00:00").unwrap().toasts;
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].base_date, fmt(day(TODAY) - Duration::days(2)));
        assert_eq!(ledger_count(&conn), 2);
        assert_eq!(ledger_rows(&conn)[0].1, Some(action_id(&conn, "喂食")));
    }

    #[test]
    fn overdue_only_for_active_reminding_beyond_interval() {
        let conn = mem_conn();
        // 冬眠窝（有开放段）100 天没喂：不发超期（出眠日远在天边，冬眠类也不发）
        let hiber = colony(&conn, "冬眠一号", "hibernating");
        open_seg(&conn, hiber, "2027-03-01");
        feed(&conn, hiber, "喂食", "2026-06-01 08:00:00");
        // 已结束的窝 100 天没喂：全不发
        let ended = colony(&conn, "结束一号", "ended");
        feed(&conn, ended, "喂食", "2026-06-01 08:00:00");
        // 活跃窝只记了登记类（换水 100 天前）：永不催
        let logonly = colony(&conn, "登记一号", "active");
        feed(&conn, logonly, "活动区换水", "2026-06-01 08:00:00");
        // 活跃窝从未记录：不发
        let _fresh = colony(&conn, "新窝一号", "active");

        let due = compute_due_reminders(&conn, TODAY, 7).unwrap();
        assert!(
            due.is_empty(),
            "冬眠不发超期、已结束不发、登记类永不催、从未记录不发：{due:?}"
        );
    }

    // ── 临近 / 出眠日 ──

    #[test]
    fn approaching_then_wake_day_fire_once_each() {
        let conn = mem_conn();
        let c = colony(&conn, "冬眠一号", "hibernating");
        let end = day(TODAY) + Duration::days(10);
        open_seg(&conn, c, &fmt(end));

        // 还有 10 天（提前 7 天窗之外）→ 不发
        let sent = run_check(&conn, TODAY, NOW).unwrap().toasts;
        assert!(sent.is_empty());

        // 进入提前窗（E−7）→ 临近一次
        let d7 = fmt(end - Duration::days(7));
        let sent = run_check(&conn, &d7, NOW).unwrap().toasts;
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].kind, ReminderKind::ApproachingWake);
        assert_eq!(sent[0].base_date, fmt(end));

        // 窗内第二天 → 不重发
        let d6 = fmt(end - Duration::days(6));
        assert!(run_check(&conn, &d6, NOW).unwrap().toasts.is_empty());

        // 出眠日当天 → 出眠日一条
        let sent = run_check(&conn, &fmt(end), NOW).unwrap().toasts;
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].kind, ReminderKind::WakeDay);
        // 同日再查不重发
        assert!(run_check(&conn, &fmt(end), "2026-09-28 20:00:00").unwrap().toasts.is_empty());
        assert_eq!(ledger_count(&conn), 2, "临近一条 + 出眠日一条");
    }

    #[test]
    fn zero_days_ahead_lets_approaching_and_wake_day_share_a_day() {
        // 票 01 停靠的兑现：提前天数=0 → 两种提醒同日、各发一条（v2 唯一键会互斥）
        let conn = mem_conn();
        let c = colony(&conn, "冬眠一号", "hibernating");
        open_seg(&conn, c, TODAY); // 今天就是预计出眠日
        set_ahead(&conn, 0);

        let sent = run_check(&conn, TODAY, NOW).unwrap().toasts;
        let kinds: Vec<ReminderKind> = sent.iter().map(|r| r.kind).collect();
        assert_eq!(
            kinds,
            vec![ReminderKind::ApproachingWake, ReminderKind::WakeDay],
            "同日两条：临近 + 出眠日"
        );
        assert_eq!(ledger_count(&conn), 2);
    }

    #[test]
    fn changing_expected_end_recomputes_unsent_and_keeps_sent() {
        let conn = mem_conn();
        let c = colony(&conn, "冬眠一号", "hibernating");
        // E1 = 今天+3：已在提前窗内 → 临近已发（基准日 = E1）
        let e1 = day(TODAY) + Duration::days(3);
        open_seg(&conn, c, &fmt(e1));
        let sent = run_check(&conn, TODAY, NOW).unwrap().toasts;
        assert_eq!(sent.len(), 1, "临近先发");
        assert_eq!(sent[0].base_date, fmt(e1));

        // 改预计出眠日 → E2 = 今天+9（提前窗外）：已发的 E1 行保留（不追回）
        let e2 = day(TODAY) + Duration::days(9);
        conn.execute(
            "UPDATE hibernation SET expected_end_date = ?1 WHERE colony_id = ?2",
            params![fmt(e2), c],
        )
        .unwrap();
        assert!(run_check(&conn, TODAY, NOW).unwrap().toasts.is_empty(), "新日期未到窗内，不发");

        // 到新窗内（E2−7）→ 按新日期发（未发的按新日期重算）
        let sent = run_check(&conn, &fmt(e2 - Duration::days(7)), NOW).unwrap().toasts;
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].base_date, fmt(e2), "按新出眠日发");

        // 旧行仍在：已发不删不追回；两行不同基准日
        let rows = ledger_rows(&conn);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].2, fmt(e1));
        assert_eq!(rows[1].2, fmt(e2));
    }

    #[test]
    fn early_wake_stops_hibernation_reminders() {
        // 规则 4：提前出眠后临近/出眠日不再触发
        let conn = mem_conn();
        let c = colony(&conn, "冬眠一号", "hibernating");
        let end = day(TODAY) + Duration::days(5);
        open_seg(&conn, c, &fmt(end));
        let sent = run_check(&conn, TODAY, NOW).unwrap().toasts;
        assert_eq!(sent.len(), 1, "提前窗内，临近先发一条");

        // 提前出眠（状态回活跃）
        crate::hibernation::confirm_wake(&conn, c, TODAY, TODAY).unwrap();

        // 推进到预计出眠日当天：临近/出眠日不再触发（规则 4）。
        // 出眠后「距上次」从出眠日重新起算（5 天没喂 → 统一层超期；
        // F3：种子（周期 3）同样从出眠日起算 5 > 3，食物层各自再发一条）。
        let due = compute_due_reminders(&conn, &fmt(end), 7).unwrap();
        assert_eq!(due.len(), 2, "统一超期 + 种子食物层各一条，实际：{due:?}");
        assert_eq!(due[0].kind, ReminderKind::Overdue);
        assert_eq!(due[0].base_date, fmt(end - Duration::days(3)));
        assert_eq!(due[1].kind, ReminderKind::FoodOverdue);
        assert_eq!(due[1].food_name.as_deref(), Some("种子"));
        assert_eq!(due[1].base_date, fmt(end - Duration::days(3)));
    }

    // ── 补发上限（规则 3）──

    #[test]
    fn backfill_window_blocks_wake_reminders_older_than_seven_days() {
        let conn = mem_conn();
        let stale = colony(&conn, "过号一号", "hibernating");
        let fresh = colony(&conn, "刚好一号", "hibernating");
        // 出眠日 10 天前（窗外）→ 临近/出眠日都不补
        open_seg(&conn, stale, &fmt(day(TODAY) - Duration::days(10)));
        // 出眠日 3 天前（窗内）→ 各补一条
        open_seg(&conn, fresh, &fmt(day(TODAY) - Duration::days(3)));

        let sent = run_check(&conn, TODAY, NOW).unwrap().toasts;
        let stale_ids: Vec<i64> = sent.iter().map(|r| r.colony_id).collect();
        assert!(!stale_ids.contains(&stale), "窗外不补");

        let fresh_rows: Vec<Reminder> = sent.into_iter().filter(|r| r.colony_id == fresh).collect();
        assert_eq!(fresh_rows.len(), 2, "窗内临近+出眠日各补一条");
        for r in &fresh_rows {
            assert_eq!(r.base_date, fmt(day(TODAY) - Duration::days(3)));
        }
        // 重查不重发：每窝每种只补一次
        assert!(run_check(&conn, TODAY, "2026-09-18 21:00:00").unwrap().toasts.is_empty());
        assert_eq!(ledger_count(&conn), 2);
    }

    #[test]
    fn backfill_produces_only_latest_single_overdue_per_action() {
        // 模拟 10 天未启动：喂食 13 天前（建议 3）→ 只补「今天这一天」的一条
        let conn = mem_conn();
        let c = colony(&conn, "大头一号", "active");
        feed(&conn, c, "喂食", "2026-09-05 08:00:00");

        let sent = run_check(&conn, TODAY, NOW).unwrap().toasts;
        assert_eq!(sent.len(), 1, "只补最新一条，不倒灌 10 天");
        assert_eq!(sent[0].base_date, fmt(day(TODAY) - Duration::days(3)));
        assert_eq!(ledger_count(&conn), 1);
    }

    // ── 开关（规则 1）──

    #[test]
    fn master_switch_off_is_full_silence_and_backlog_fires_when_reenabled() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号", "active");
        feed(&conn, c, "喂食", "2026-09-10 08:00:00"); // 8 天前，超期

        // 总开关关：不发也不写台账
        settings::set_settings(&conn, &AppSettings { notify_master_enabled: false, ..Default::default() }).unwrap();
        let sent = run_check(&conn, TODAY, NOW).unwrap().toasts;
        assert!(sent.is_empty());
        assert_eq!(ledger_count(&conn), 0, "静默期不写台账");

        // 重开后：按去重规则正常发（此刻应发的照样发出）
        settings::set_settings(&conn, &AppSettings::default()).unwrap();
        let sent = run_check(&conn, TODAY, NOW).unwrap().toasts;
        assert_eq!(sent.len(), 1);
        assert_eq!(ledger_count(&conn), 1);
    }

    #[test]
    fn category_switches_no_longer_filter_since_feedback2() {
        // Q7/Q9：分类子开关作废——关着也照发（总开关才是唯一闸门）
        let conn = mem_conn();
        let a = colony(&conn, "活跃一号", "active");
        let h = colony(&conn, "冬眠一号", "hibernating");
        feed(&conn, a, "喂食", "2026-09-10 08:00:00");
        open_seg(&conn, h, TODAY);
        settings::set_settings(
            &conn,
            &AppSettings { notify_overdue_enabled: false, notify_hibernation_enabled: false, ..Default::default() },
        )
        .unwrap();
        let out = run_check(&conn, TODAY, NOW).unwrap();
        assert_eq!(out.toasts.len(), 3, "超期 1 + 临近/出眠日 2，分类开关不再过滤");
    }

    #[test]
    fn pushover_failures_retry_next_round_and_settle_on_success() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号", "active");
        feed(&conn, c, "喂食", "2026-09-10 08:00:00"); // 超期

        // 第一轮：新插入 → 1 条 toast + 1 个待发推送任务（发送在锁外，由调用方做）
        let out = run_check(&conn, TODAY, NOW).unwrap();
        assert_eq!(out.toasts.len(), 1);
        assert_eq!(out.push_jobs.len(), 1);
        assert_eq!(out.push_jobs[0].title, "喂食超期");
        // 模拟发送失败：不 settle

        // 同日第二轮：toast 不重发，推送任务仍在（补发）
        let out = run_check(&conn, TODAY, "2026-09-18 12:00:00").unwrap();
        assert!(out.toasts.is_empty());
        assert_eq!(out.push_jobs.len(), 1);
        let id = out.push_jobs[0].ledger_id;

        // 发送成功 → settle → 第三轮无任务
        settle_pushover(&conn, &[id]).unwrap();
        let out = run_check(&conn, TODAY, "2026-09-18 18:00:00").unwrap();
        assert!(out.push_jobs.is_empty());
    }

    #[test]
    fn stale_unsettled_push_jobs_are_abandoned_beyond_retry_window() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号", "active");
        feed(&conn, c, "喂食", "2026-09-01 08:00:00");
        let out = run_check(&conn, TODAY, NOW).unwrap();
        assert_eq!(out.push_jobs.len(), 1);
        // 伪造：该行三天前就登记且一直没发成功
        conn.execute(
            "UPDATE reminder_ledger SET sent_at = '2026-09-14 08:00:00' WHERE id = ?1",
            params![out.push_jobs[0].ledger_id],
        )
        .unwrap();
        let out = run_check(&conn, TODAY, NOW).unwrap();
        assert!(out.push_jobs.is_empty(), "超窗不再补发");
        let done: i64 = conn
            .query_row("SELECT pushover_done FROM reminder_ledger", [], |r| r.get(0))
            .unwrap();
        assert_eq!(done, 1, "超窗自动了结");
    }

    // ── 食物层超期（反馈第二轮 F3）──

    #[test]
    fn food_overdue_fires_independently_of_operation_layer() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号", "active");
        // 只喂种子（2 天前）：统一层不超期；面包虫 8 天前喂过 → 食物层超期
        feed_foods(&conn, c, "2026-09-16 20:00:00", &["种子"]);
        feed_foods(&conn, c, "2026-09-10 20:00:00", &["面包虫"]);

        let out = run_check(&conn, TODAY, NOW).unwrap();
        assert_eq!(out.toasts.len(), 1, "只有面包虫食物层一条");
        let r = &out.toasts[0];
        assert_eq!(r.kind, ReminderKind::FoodOverdue);
        assert_eq!(r.action_name.as_deref(), Some("喂食"));
        assert_eq!(r.food_name.as_deref(), Some("面包虫"));
        assert_eq!(r.days_since_last, Some(8));
        assert_eq!(r.base_date, fmt(day(TODAY) - Duration::days(7)));
        let (title, body) = r.notification_text();
        assert_eq!(title, "该喂面包虫了");
        assert_eq!(body, "「大头一号」已 8 天没喂面包虫（建议 7 天一次）");

        // 同日重查不重发；台账 food 维度去重
        assert!(run_check(&conn, TODAY, "2026-09-18 12:00:00").unwrap().toasts.is_empty());
        let kinds = ledger_rows(&conn);
        assert_eq!(kinds.len(), 1);
    }

    #[test]
    fn operation_and_food_layers_same_day_both_fire() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号", "active");
        feed_foods(&conn, c, "2026-09-10 20:00:00", &["种子", "面包虫"]); // 8 天 → 两层全超
        let out = run_check(&conn, TODAY, NOW).unwrap();
        assert_eq!(out.toasts.len(), 3, "统一层 1 + 种子 1 + 面包虫 1，各记各的");
    }

    #[test]
    fn hibernating_colony_food_layer_silent_too() {
        let conn = mem_conn();
        let h = colony(&conn, "冬眠一号", "hibernating");
        open_seg(&conn, h, "2027-03-01");
        feed_foods(&conn, h, "2026-06-01 08:00:00", &["面包虫"]);
        assert!(run_check(&conn, TODAY, NOW).unwrap().toasts.is_empty());
    }

    // ── 通知文案 ──

    #[test]
    fn notification_text_covers_three_kinds() {
        let r = Reminder {
            colony_id: 1,
            colony_name: "大头一号".into(),
            kind: ReminderKind::Overdue,
            action_id: Some(1),
            action_name: Some("喂食".into()),
            food_id: None,
            food_name: None,
            base_date: "2026-09-15".into(),
            days_since_last: Some(4),
            suggested_interval_days: Some(3),
        };
        let (title, body) = r.notification_text();
        assert_eq!(title, "喂食超期");
        assert_eq!(body, "「大头一号」已 4 天没喂食（建议 3 天一次）");

        let r = Reminder {
            kind: ReminderKind::ApproachingWake,
            base_date: "2026-10-01".into(),
            days_since_last: None,
            suggested_interval_days: None,
            action_id: None,
            action_name: None,
            ..r
        };
        let (title, body) = r.notification_text();
        assert_eq!(title, "临近出眠");
        assert!(body.contains("大头一号") && body.contains("2026-10-01"), "{body}");

        let r = Reminder { kind: ReminderKind::WakeDay, ..r };
        let (title, body) = r.notification_text();
        assert_eq!(title, "出眠日");
        assert!(body.contains("2026-10-01"), "{body}");
    }

    // ── 托盘概要 ──

    fn tile(name: &str, kind: &str, overdue: bool, days: Option<i64>) -> crate::care::ActionTile {
        crate::care::ActionTile {
            action_id: 1,
            name: name.into(),
            icon: None,
            kind: kind.into(),
            is_feeding: name == "喂食",
            suggested_interval_days: if kind == "reminding" { Some(3) } else { None },
            days_since_last: days,
            overdue,
            foods: vec![],
        }
    }

    fn col(name: &str, status: &str, actions: Vec<crate::care::ActionTile>) -> crate::colony::Colony {
        crate::colony::Colony {
            id: 1,
            name: name.into(),
            species: None,
            location_id: None,
            start_date: "2026-01-20".into(),
            status: status.into(),
            days_raised: 241,
            actions,
            recent: vec![],
            hibernation: None,
        }
    }

    #[test]
    fn tray_summary_builds_one_line_overview() {
        assert_eq!(tray_summary(&[]), "暂无窝");

        // spec 示例形态："2 窝活跃 · 大头一号喂食超期 1 天"
        let a = col(
            "大头一号",
            "active",
            vec![tile("喂食", "reminding", true, Some(4)), tile("垃圾清理", "reminding", false, Some(2))],
        );
        let b = col("针毛一号", "active", vec![tile("喂食", "reminding", false, Some(1))]);
        let h = col("冬眠一号", "hibernating", vec![]);
        let e = col("老窝", "ended", vec![tile("喂食", "reminding", true, Some(99))]);

        assert_eq!(
            tray_summary(&[a.clone(), b, h, e]),
            "2 窝活跃 · 1 窝冬眠中 · 大头一号喂食超期 4 天",
            "活跃数 + 冬眠数 + 仅活跃窝的超期摘要（已结束不算）"
        );

        // 无任何超期：只报窝数
        let quiet = col("安静一号", "active", vec![tile("喂食", "reminding", false, Some(1))]);
        assert_eq!(tray_summary(&[quiet]), "1 窝活跃");

        // 一窝两条超期：顿号列举
        let busy = col(
            "忙窝",
            "active",
            vec![tile("喂食", "reminding", true, Some(5)), tile("垃圾清理", "reminding", true, Some(9))],
        );
        assert_eq!(
            tray_summary(&[busy]),
            "1 窝活跃 · 忙窝喂食超期 5 天 · 忙窝垃圾清理超期 9 天"
        );
        let _ = a;
    }

    #[test]
    fn tray_summary_reports_food_overdue_alongside_operation_layer() {
        // F3：食物层超期补「某窝该喂某食物了」，与统一层句式并存
        let mut feed = tile("喂食", "reminding", true, Some(4));
        feed.foods = vec![
            crate::care::FoodTileStatus {
                food_id: 1,
                name: "面包虫".into(),
                suggested_interval_days: Some(7),
                days_since_last: Some(8),
                overdue: true,
            },
            crate::care::FoodTileStatus {
                food_id: 2,
                name: "种子".into(),
                suggested_interval_days: Some(3),
                days_since_last: Some(1),
                overdue: false,
            },
        ];
        let c = col("大头一号", "active", vec![feed]);
        assert_eq!(
            tray_summary(&[c]),
            "1 窝活跃 · 大头一号喂食超期 4 天 · 大头一号该喂面包虫了",
            "统一层句式不变 + 食物层各补一条；不超期的食物不报"
        );

        // 统一层新鲜、仅食物层超期：只报食物行
        let fresh = col(
            "针毛一号",
            "active",
            vec![tile("喂食", "reminding", false, Some(1))],
        );
        let text = tray_summary(&[fresh]);
        assert_eq!(text, "1 窝活跃", "无食物明细且统一层不超期 → 不加食物行");
    }

    #[test]
    fn tray_summary_truncates_to_windows_limit() {
        let long_name = "超".repeat(200);
        let c = col(
            &long_name,
            "active",
            vec![tile("喂食", "reminding", true, Some(9))],
        );
        let text = tray_summary(&[c]);
        assert!(text.chars().count() <= 128, "Windows tooltip 上限 128 字符，实际 {}", text.chars().count());
    }
}
