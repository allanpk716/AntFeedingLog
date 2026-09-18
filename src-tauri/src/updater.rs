//! 更新核心（票 02）：updater 插件接入 + 每日静默检查 + 手动检查命令。
//!
//! 纯逻辑与 Tauri 解耦，cargo test 直接覆盖（沿 db.rs/settings.rs 惯例）：
//! - [`should_check_today`]：每日节奏——同一天只查一次、跨天重查，重启也不重查
//!   （时间源注入：today 由调用方传入）；
//! - [`to_outcome`]：检查结果三态映射（无更新 / 有新版 / 检查失败）；
//! - [`fold_daily`]：每日路径错误折叠——任何失败（404/网络/超时/解析）一律按
//!   "无更新"记日志静默，绝不外抛失败态；
//! - [`manual_result`]：手动路径出口——检查失败折为 Err（前端一次性展示）；
//! - 每日检查拆成两段纯函数（评审 R1-1：网络段绝不持 DB 锁）：
//!   [`should_run_daily_check`]（持锁段：读记账定该不该查）→ 调用方放锁执行
//!   `checker.check()`（网络段）→ [`finish_daily_check`]（持锁段：记账+折算，
//!   签名只吃已取回的结果值，结构上不存在网络调用）；
//!   检查执行器经 [`UpdateChecker`] trait 注入：生产实现走 tauri-plugin-updater，
//!   测试用假实现，不碰真网络。
//!
//! "上次检查日"复用 settings 表存储（settings.rs 同款 key-value，键
//! [`K_LAST_CHECK_DAY`]），跨启动持久化；手动检查不写这个键（互不干扰）。
//!
//! 配置面（tauri.conf.json，严格 JSON 不带注释，决策记这里）：
//! - `bundle.createUpdaterArtifacts: true`：发布产物带 minisign 签名；main CI
//!   无私钥，构建步用 `--config` 覆盖关闭（见 .github/workflows/ci.yml）；
//! - `plugins.updater.endpoints`：GitHub latest.json 直连，数组留镜像扩展位；
//! - `plugins.updater.pubkey` 当前是占位串（base64 的 "untrusted comment:"），
//!   **票 07** 由用户生成 minisign 密钥对后填真实公钥。公钥只在插件下载/验签
//!   阶段解析（插件源码 v2.11.0：verify_signature），check 阶段不受影响；占位
//!   期间尚无 release 时检查 404 → 每日按无更新静默、手动返回 Err，应用启动
//!   不受影响——票 07 会在首发前填真实公钥。
//!
//! 生产检查器、每日定时线程与通知发送是薄封装（lib.rs 调
//! `spawn_daily_checker`，系统行为不进单测）。

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

/// settings 表键名：每日更新检查最近一次执行的日期（ISO 日期串）。
pub const K_LAST_CHECK_DAY: &str = "update_last_check_day";

/// 远端有新版时的最小信息：版本号 + release 说明。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UpdateInfo {
    pub version: String,
    pub notes: Option<String>,
}

/// 检查执行器抽象（seam）：Ok(None)=远端无新版；Ok(Some)=有新版；
/// Err=检查失败（网络/404/超时/解析等）。
pub trait UpdateChecker {
    fn check(&self) -> Result<Option<UpdateInfo>, String>;
}

/// 检查结果三态（票面）。`CheckFailed` 仅手动路径对外可见（每日路径已折叠）。
/// serde 形态是前端契约（票 06 设置页消费）：`{"status":"up_to_date"}` 等。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CheckOutcome {
    UpToDate,
    UpdateAvailable {
        version: String,
        notes: Option<String>,
    },
    CheckFailed {
        message: String,
    },
}

/// 一轮每日检查的对外结果：`Skipped`=今天已查过；`NoUpdate`=查了但无新版
/// （含失败静默）；`UpdateAvailable`=查到新版，调用方发系统通知。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DailyOutcome {
    Skipped,
    NoUpdate,
    UpdateAvailable {
        version: String,
        notes: Option<String>,
    },
}

// ── 核心：每日节奏 ────────────────────────────────────────────────────────

/// 该不该在今天做每日检查：今天还没记过 → 查（含从没查过、记的是昨天或脏值）；
/// 今天已查过 → 跳过（重启也不重查）。两侧都是 today_iso 同源格式，直接比串。
pub fn should_check_today(last_check_day: Option<&str>, today: &str) -> bool {
    last_check_day != Some(today)
}

/// 三态映射：检查器原始结果 → 对外三态。
pub fn to_outcome(result: Result<Option<UpdateInfo>, String>) -> CheckOutcome {
    match result {
        Ok(None) => CheckOutcome::UpToDate,
        Ok(Some(info)) => CheckOutcome::UpdateAvailable {
            version: info.version,
            notes: info.notes,
        },
        Err(message) => CheckOutcome::CheckFailed { message },
    }
}

/// 每日路径错误折叠：任何失败一律按"无更新"处理并记日志，绝不外抛 `CheckFailed`
/// （约定：每日静默检查绝不因失败发通知打扰）。
pub fn fold_daily(outcome: CheckOutcome) -> CheckOutcome {
    match outcome {
        CheckOutcome::CheckFailed { message } => {
            eprintln!("[updater] 每日静默检查失败，按无更新处理: {message}");
            CheckOutcome::UpToDate
        }
        other => other,
    }
}

/// 手动路径出口：检查失败折为 Err（前端一次性展示），成功两态原样通过。
pub fn manual_result(outcome: CheckOutcome) -> Result<CheckOutcome, String> {
    match outcome {
        CheckOutcome::CheckFailed { message } => Err(message),
        other => Ok(other),
    }
}

fn db_err(e: rusqlite::Error) -> String {
    format!("数据库操作失败: {e}")
}

/// 上次每日检查日（ISO 日期串）；从没查过 = None。
pub fn last_check_day(conn: &Connection) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT value FROM settings WHERE key = ?1",
        params![K_LAST_CHECK_DAY],
        |row| row.get(0),
    )
    .optional()
    .map_err(db_err)
}

fn set_last_check_day(conn: &Connection, day: &str) -> Result<(), String> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![K_LAST_CHECK_DAY, day],
    )
    .map_err(db_err)?;
    Ok(())
}

/// 每日检查 · 段 1（持锁段）：读"上次检查日"判断今天该不该查。调用方拿锁调它、
/// 拿到结果立即放锁，网络检查在锁外做（评审 R1-1）。
pub fn should_run_daily_check(conn: &Connection, today: &str) -> Result<bool, String> {
    Ok(should_check_today(last_check_day(conn)?.as_deref(), today))
}

/// 每日检查 · 段 3（持锁段）：对"已取回的检查结果"折算 + 记账（记今天）。
/// 签名只吃结果值、没有检查器参数——结构上保证本段不可能发起网络调用。
/// 失败结果也记今天（当天不再重试，跨天自愈）；记账失败按 Err 返回，
/// 调用方记日志、下个周期重试当天这轮。
pub fn finish_daily_check(
    conn: &Connection,
    result: Result<Option<UpdateInfo>, String>,
    today: &str,
) -> Result<DailyOutcome, String> {
    let outcome = fold_daily(to_outcome(result));
    set_last_check_day(conn, today)?;
    Ok(match outcome {
        CheckOutcome::UpdateAvailable { version, notes } => {
            DailyOutcome::UpdateAvailable { version, notes }
        }
        // fold_daily 之后失败态已折为 UpToDate，这里兜底同口径
        _ => DailyOutcome::NoUpdate,
    })
}

// ── 薄封装：生产检查器 / 手动入口 / 每日定时（Tauri 侧，系统行为不进单测）──

use std::time::Duration;
use tauri::Manager;

/// 单次检查的总超时（评审 R1-3）：插件 builder 默认无超时（reqwest 无总超时），
/// 网络黑洞会把锁外网络段和手动路径的 UI 等待拖到无上限。
pub const CHECK_TIMEOUT: Duration = Duration::from_secs(30);

/// 带总超时的 updater 构造（endpoints / pubkey 来自 tauri.conf.json）。
fn build_updater(app: &tauri::AppHandle) -> Result<tauri_plugin_updater::Updater, String> {
    use tauri_plugin_updater::UpdaterExt;
    app.updater_builder()
        .timeout(CHECK_TIMEOUT)
        .build()
        .map_err(|e| format!("初始化更新器失败: {e}"))
}

/// 插件 Update → 最小信息（每日/手动两条路径共用）。
fn update_to_info(u: tauri_plugin_updater::Update) -> UpdateInfo {
    UpdateInfo {
        version: u.version.clone(),
        notes: u.body.clone(),
    }
}

/// 生产检查器：走 tauri-plugin-updater。只在每日定时的独立 std 线程里用
/// （线程内无异步运行时，block_on 安全）。
struct PluginChecker {
    app: tauri::AppHandle,
}

impl UpdateChecker for PluginChecker {
    fn check(&self) -> Result<Option<UpdateInfo>, String> {
        let updater = build_updater(&self.app)?;
        let update = tauri::async_runtime::block_on(updater.check())
            .map_err(|e| format!("检查更新失败: {e}"))?;
        // 版本比较由插件内部完成：远端不比当前新时返回 None
        Ok(update.map(update_to_info))
    }
}

/// 手动检查入口（设置页 `check_update_now`）。async：同步 command 跑在主线程/
/// 事件循环上，阻塞网络会把整个 UI 冻住（评审 R1-2），改 async 由 Tauri 丢进
/// 异步运行时、内部直接 `.await`（此处不得再 block_on）。
/// 失败折为 Err 一次性展示。刻意不写每日记账（last_check_day）：两条路径互不
/// 干扰，当天每日检查照常执行。
pub async fn manual_check(app: tauri::AppHandle) -> Result<CheckOutcome, String> {
    let updater = build_updater(&app)?;
    let update = updater
        .check()
        .await
        .map_err(|e| format!("检查更新失败: {e}"))?;
    manual_result(to_outcome(Ok(update.map(update_to_info))))
}

/// 发现新版的系统通知（每日路径唯一的对外打扰；复用 tauri-plugin-notification，
/// 与提醒同款；发送失败静默——通知是副产物）。
fn notify_update(handle: &tauri::AppHandle, version: &str, notes: Option<&str>) {
    use tauri_plugin_notification::NotificationExt;
    let mut body = format!("发现新版本 v{version}，可到设置页查看并安装。");
    if let Some(n) = notes.map(str::trim).filter(|n| !n.is_empty()) {
        let mut brief: String = n.chars().take(80).collect();
        if brief.chars().count() < n.chars().count() {
            brief.push('…');
        }
        body = format!("发现新版本 v{version}：{brief}");
    }
    let _ = handle
        .notification()
        .builder()
        .title("蚂蚁饲养记录有更新")
        .body(body)
        .show();
}

/// 一轮每日 tick（评审 R1-1 锁拆分三段式，与测试侧 `run_daily_flow` 同序）：
/// 段 1 持锁读记账、立即放锁 → 段 2 锁外做网络检查（30s 总超时兜底）→
/// 段 3 持锁记账折算 → 查到新版发系统通知（窗口关着也跑，托盘常驻线程）。
/// DB 锁被全应用的页面 command 与提醒调度线程共用，绝不持锁等网络。
/// 任何失败都止于日志，绝不弹错误通知（spec 错误路径约定；
/// 评审 R1-4：DB 侧 Err 也记日志，不静默丢弃）。
fn daily_tick(handle: &tauri::AppHandle) {
    let Some(state) = handle.try_state::<crate::DbState>() else {
        return;
    };
    let today = crate::colony::today_iso();

    // 段 1（持锁）：今天该不该查——纯 DB 读，拿到结果立即放锁
    let should = {
        let Ok(conn) = state.0.lock() else { return };
        should_run_daily_check(&conn, &today)
    };
    match should {
        Ok(false) => return,
        Err(e) => {
            eprintln!("[updater] 每日更新检查读记账失败（本轮跳过）: {e}");
            return;
        }
        Ok(true) => {}
    }

    // 段 2（放锁）：网络检查——锁已归还，堵也只堵本线程
    let checker = PluginChecker {
        app: handle.clone(),
    };
    let result = checker.check();

    // 段 3（持锁）：记账 + 折算
    let outcome = {
        let Ok(conn) = state.0.lock() else { return };
        finish_daily_check(&conn, result, &today)
    };
    match outcome {
        Ok(DailyOutcome::UpdateAvailable { version, notes }) => {
            notify_update(handle, &version, notes.as_deref());
        }
        Ok(_) => {}
        Err(e) => eprintln!("[updater] 每日更新检查记账失败（下个 30 分钟周期重试当天这轮）: {e}"),
    }
}

/// 每日检查定时器：启动即跑一轮（内部按"上次检查日"决定真查还是跳过，重启
/// 不重查），此后每 30 分钟醒一次，跨天后的第一轮负责真查。独立 std 线程 +
/// catch_unwind，与提醒调度同款（票 09 停靠 A）。
pub fn spawn_daily_checker(handle: tauri::AppHandle) {
    std::thread::spawn(move || loop {
        let h = handle.clone();
        let round = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            daily_tick(&h);
        }));
        if let Err(panic) = round {
            eprintln!("[updater] 本轮每日更新检查 panic（已跳过，下个 30 分钟周期重试）: {panic:?}");
        }
        std::thread::sleep(std::time::Duration::from_secs(30 * 60));
    });
}

// ── 测试：只测外部行为（spec「Testing Decisions」）────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    /// 建内存库并迁移到最新 schema（票 01 地基）。
    fn mem_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("内存库打开失败");
        crate::db::migrate(&conn).expect("迁移失败");
        conn
    }

    /// 假检查器：返回预置结果并计数，绝不碰真网络。
    struct FakeChecker {
        result: Result<Option<UpdateInfo>, String>,
        calls: Cell<usize>,
    }

    impl FakeChecker {
        fn new(result: Result<Option<UpdateInfo>, String>) -> Self {
            FakeChecker {
                result,
                calls: Cell::new(0),
            }
        }
        fn up_to_date() -> Self {
            Self::new(Ok(None))
        }
        fn new_version() -> Self {
            Self::new(Ok(Some(UpdateInfo {
                version: "0.3.0".into(),
                notes: Some("修复若干问题".into()),
            })))
        }
        fn failing() -> Self {
            Self::new(Err("HTTP 404：latest.json 不存在".into()))
        }
        fn call_count(&self) -> usize {
            self.calls.get()
        }
    }

    impl UpdateChecker for FakeChecker {
        fn check(&self) -> Result<Option<UpdateInfo>, String> {
            self.calls.set(self.calls.get() + 1);
            self.result.clone()
        }
    }

    const D1: &str = "2026-09-18";
    const D2: &str = "2026-09-19";

    /// daily_tick 的同序纯逻辑镜像（评审 R1-1 锁拆分后）：段 1 持锁查记账 →
    /// 段 2 放锁做网络（fake）→ 段 3 持锁记账折算。与薄封装里的三段一一对应。
    fn run_daily_flow<C: UpdateChecker>(
        conn: &Connection,
        checker: &C,
        today: &str,
    ) -> Result<DailyOutcome, String> {
        if !should_run_daily_check(conn, today)? {
            return Ok(DailyOutcome::Skipped);
        }
        finish_daily_check(conn, checker.check(), today)
    }

    // ── 三态映射 ──

    #[test]
    fn to_outcome_maps_three_states() {
        // 无更新
        assert_eq!(to_outcome(Ok(None)), CheckOutcome::UpToDate);

        // 有新版：版本号 + 说明
        let info = UpdateInfo {
            version: "0.3.0".into(),
            notes: Some("新增xx".into()),
        };
        assert_eq!(
            to_outcome(Ok(Some(info))),
            CheckOutcome::UpdateAvailable {
                version: "0.3.0".into(),
                notes: Some("新增xx".into())
            }
        );

        // 说明可缺（release notes 可能为空）
        let bare = UpdateInfo {
            version: "0.3.0".into(),
            notes: None,
        };
        assert_eq!(
            to_outcome(Ok(Some(bare))),
            CheckOutcome::UpdateAvailable {
                version: "0.3.0".into(),
                notes: None
            }
        );

        // 检查失败
        assert_eq!(
            to_outcome(Err("请求超时".into())),
            CheckOutcome::CheckFailed {
                message: "请求超时".into()
            }
        );
    }

    #[test]
    fn check_outcome_serializes_tagged_for_frontend() {
        // 票 06 前端契约：tag = status，snake_case
        let json = serde_json::to_string(&CheckOutcome::UpToDate).unwrap();
        assert_eq!(json, r#"{"status":"up_to_date"}"#);

        let json = serde_json::to_string(&CheckOutcome::UpdateAvailable {
            version: "0.3.0".into(),
            notes: Some("n".into()),
        })
        .unwrap();
        assert_eq!(
            json,
            r#"{"status":"update_available","version":"0.3.0","notes":"n"}"#
        );
    }

    // ── 错误路径：每日静默 vs 手动 Err ──

    #[test]
    fn daily_path_folds_any_failure_into_no_update() {
        // 404 / 网络 / 超时 / 解析——任何 Err 都按"无更新"，绝不外抛失败态
        for msg in [
            "HTTP 404：latest.json 不存在",
            "network unreachable",
            "请求超时",
            "清单 JSON 解析失败",
        ] {
            let folded = fold_daily(to_outcome(Err(msg.into())));
            assert_eq!(folded, CheckOutcome::UpToDate, "失败「{msg}」应折为无更新");
        }

        // 成功两态原样通过（有新版仍要提示用户，只折叠失败）
        assert_eq!(fold_daily(to_outcome(Ok(None))), CheckOutcome::UpToDate);
        assert_eq!(
            fold_daily(to_outcome(Ok(Some(UpdateInfo {
                version: "0.3.0".into(),
                notes: None,
            })))),
            CheckOutcome::UpdateAvailable {
                version: "0.3.0".into(),
                notes: None
            }
        );
    }

    #[test]
    fn manual_path_surfaces_failure_as_err_and_keeps_good_states() {
        // 失败 → Err（前端一次性展示）
        let err = manual_result(to_outcome(Err("HTTP 404".into()))).unwrap_err();
        assert_eq!(err, "HTTP 404");

        // 成功两态原样
        assert_eq!(
            manual_result(to_outcome(Ok(None))).unwrap(),
            CheckOutcome::UpToDate
        );
        assert_eq!(
            manual_result(to_outcome(Ok(Some(UpdateInfo {
                version: "0.3.0".into(),
                notes: Some("n".into()),
            }))))
            .unwrap(),
            CheckOutcome::UpdateAvailable {
                version: "0.3.0".into(),
                notes: Some("n".into())
            }
        );
    }

    // ── 每日节奏：同一天一次、跨天重查、跨启动持久化 ──

    #[test]
    fn should_check_today_rules() {
        // 从没查过 → 查
        assert!(should_check_today(None, D1));
        // 记的是别的日子（昨天）或脏值 → 查
        assert!(should_check_today(Some("2026-09-17"), D1));
        assert!(should_check_today(Some("不是日期"), D1));
        // 今天已查 → 不查（重启也不重查的判定基础）
        assert!(!should_check_today(Some(D1), D1));
    }

    // ── 三段式拆分（评审 R1-1：网络段不持 DB 锁）──

    #[test]
    fn should_run_daily_check_reads_bookkeeping() {
        let conn = mem_conn();
        // 从没查过 → 查；记了今天 → 不查；记了昨天 → 查
        assert!(should_run_daily_check(&conn, D1).unwrap());
        set_last_check_day(&conn, D1).unwrap();
        assert!(!should_run_daily_check(&conn, D1).unwrap());
        set_last_check_day(&conn, "2026-09-17").unwrap();
        assert!(should_run_daily_check(&conn, D1).unwrap());
    }

    #[test]
    fn finish_daily_check_records_without_network_access() {
        // 结构锚点（R1-1）：finish_daily_check 只吃"已取回的结果值"、没有检查器
        // 参数——记账段结构上不可能发起网络调用；网络段在两段之间的锁外完成。

        // 查到新版：记账 + 原样折算
        let conn = mem_conn();
        let out = finish_daily_check(
            &conn,
            Ok(Some(UpdateInfo {
                version: "0.3.0".into(),
                notes: Some("n".into()),
            })),
            D1,
        )
        .unwrap();
        assert_eq!(
            out,
            DailyOutcome::UpdateAvailable {
                version: "0.3.0".into(),
                notes: Some("n".into())
            }
        );
        assert_eq!(last_check_day(&conn).unwrap().as_deref(), Some(D1));

        // 失败结果：同样记账（当天不重试），折为无更新（每日静默）
        let conn = mem_conn();
        let out = finish_daily_check(&conn, Err("请求超时".into()), D1).unwrap();
        assert_eq!(out, DailyOutcome::NoUpdate);
        assert_eq!(last_check_day(&conn).unwrap().as_deref(), Some(D1));
    }

    #[test]
    fn daily_checks_once_per_day_and_again_next_day() {
        let conn = mem_conn();
        assert_eq!(last_check_day(&conn).unwrap(), None, "全新库没查过");

        let checker = FakeChecker::new_version();

        // 第一次（无记录）：查到新版，并记下今天
        let out = run_daily_flow(&conn, &checker, D1).unwrap();
        assert_eq!(
            out,
            DailyOutcome::UpdateAvailable {
                version: "0.3.0".into(),
                notes: Some("修复若干问题".into())
            }
        );
        assert_eq!(checker.call_count(), 1);
        assert_eq!(last_check_day(&conn).unwrap().as_deref(), Some(D1), "查完记今天");

        // 同日再来（模拟重启）：跳过，不再打远端
        let out = run_daily_flow(&conn, &checker, D1).unwrap();
        assert_eq!(out, DailyOutcome::Skipped);
        assert_eq!(checker.call_count(), 1, "同一天只查一次");

        // 跨天：重查，记录推进到新的一天
        let out = run_daily_flow(&conn, &checker, D2).unwrap();
        assert_eq!(
            out,
            DailyOutcome::UpdateAvailable {
                version: "0.3.0".into(),
                notes: Some("修复若干问题".into())
            }
        );
        assert_eq!(checker.call_count(), 2);
        assert_eq!(last_check_day(&conn).unwrap().as_deref(), Some(D2));
    }

    #[test]
    fn daily_failure_still_counts_as_checked_for_the_day() {
        // 失败也按"今天查过了"记账：当天不再重试，对外是无更新、绝不发通知；
        // 跨天后自然重查（404 窗口期由每日检查自愈）。
        let conn = mem_conn();
        let checker = FakeChecker::failing();

        let out = run_daily_flow(&conn, &checker, D1).unwrap();
        assert_eq!(out, DailyOutcome::NoUpdate, "失败按无更新");
        assert_eq!(last_check_day(&conn).unwrap().as_deref(), Some(D1));

        // 同日再跑：跳过（失败的当天也不查第二次）
        let out = run_daily_flow(&conn, &checker, D1).unwrap();
        assert_eq!(out, DailyOutcome::Skipped);
        assert_eq!(checker.call_count(), 1);
    }

    #[test]
    fn daily_run_without_update_records_day_and_stays_quiet() {
        // 无新版：安静记一天，跨天后照常重查
        let conn = mem_conn();
        let checker = FakeChecker::up_to_date();

        let out = run_daily_flow(&conn, &checker, D1).unwrap();
        assert_eq!(out, DailyOutcome::NoUpdate);
        assert_eq!(last_check_day(&conn).unwrap().as_deref(), Some(D1));

        let out = run_daily_flow(&conn, &checker, D2).unwrap();
        assert_eq!(out, DailyOutcome::NoUpdate);
        assert_eq!(checker.call_count(), 2);
    }

    #[test]
    fn manual_check_leaves_daily_bookkeeping_alone() {
        // 手动检查不写 update_last_check_day：当天每日检查照常执行（互不干扰）
        let conn = mem_conn();
        let checker = FakeChecker::up_to_date();

        let manual = manual_result(to_outcome(checker.check())).unwrap();
        assert_eq!(manual, CheckOutcome::UpToDate);
        assert_eq!(
            last_check_day(&conn).unwrap(),
            None,
            "手动检查不写每日记账"
        );

        // 每日照常可查
        let out = run_daily_flow(&conn, &checker, D1).unwrap();
        assert_eq!(out, DailyOutcome::NoUpdate);
    }
}
