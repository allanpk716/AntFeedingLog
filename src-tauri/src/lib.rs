mod care;
mod colony;
mod db;
mod dict;
mod hibernation;
mod reminder;
mod settings;
mod stats;
mod system;
mod updater;

use std::sync::Mutex;

use rusqlite::Connection;
use serde::Serialize;
use tauri::Manager;

/// 健康检查返回体：当前库的 schema 版本。
#[derive(Serialize)]
pub struct SchemaInfo {
    schema_version: i64,
}

/// 数据库连接托管在应用状态里：Rust 是数据层唯一属主，前端只经 command 读写。
/// `.1` 是库文件路径（安全备份直接拷这个文件，journal_mode=DELETE 拷贝即完整）。
pub struct DbState(Mutex<Connection>, std::path::PathBuf);

/// 借出连接的统一入口（锁被毒化时转成前端可见的错误串）。
fn with_conn<T>(
    state: tauri::State<'_, DbState>,
    f: impl FnOnce(&rusqlite::Connection) -> Result<T, String>,
) -> Result<T, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    f(&conn)
}

/// IPC 通路健康检查：返回 schema 版本（迁移正常时应为 1）。
#[tauri::command]
fn health_check(state: tauri::State<DbState>) -> Result<SchemaInfo, String> {
    with_conn(state, |conn| {
        db::schema_version_of(conn)
            .map(|schema_version| SchemaInfo { schema_version })
            .map_err(|e| e.to_string())
    })
}

// ── 记账与字典查询（票 03）──

#[tauri::command]
fn log_care(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    input: care::CareLogInput,
) -> Result<i64, String> {
    let result = with_conn(state, |conn| care::log_care(conn, &input, &care::now_local()));
    // 停靠 C：数据变了 → 托盘 tooltip 即时重算（在锁释放后调用，避免自锁）
    reminder::refresh_tray_tooltip(&app);
    result
}

#[tauri::command]
fn list_foods(state: tauri::State<'_, DbState>) -> Result<Vec<care::Food>, String> {
    with_conn(state, care::list_foods)
}

// ── 记录列表 / 编辑 / 删除（票 08）──

#[tauri::command]
fn list_logs(
    state: tauri::State<'_, DbState>,
    filter: care::LogFilter,
) -> Result<care::LogPage, String> {
    with_conn(state, |conn| care::list_logs(conn, &filter))
}

#[tauri::command]
fn update_log(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    id: i64,
    input: care::LogUpdateInput,
) -> Result<(), String> {
    let result = with_conn(state, |conn| care::update_log(conn, id, &input, &care::now_local()));
    reminder::refresh_tray_tooltip(&app);
    result
}

#[tauri::command]
fn delete_log(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    id: i64,
) -> Result<(), String> {
    let result = with_conn(state, |conn| care::delete_log(conn, id));
    reminder::refresh_tray_tooltip(&app);
    result
}

// ── 字典管理与操作性质设置（票 04）──

#[tauri::command]
fn list_actions(state: tauri::State<'_, DbState>) -> Result<Vec<dict::CareAction>, String> {
    with_conn(state, dict::list_actions)
}

#[tauri::command]
fn save_action(
    state: tauri::State<'_, DbState>,
    input: dict::ActionInput,
) -> Result<dict::CareAction, String> {
    with_conn(state, |conn| dict::save_action(conn, &input))
}

#[tauri::command]
fn set_action_enabled(
    state: tauri::State<'_, DbState>,
    id: i64,
    enabled: bool,
) -> Result<(), String> {
    with_conn(state, |conn| dict::set_action_enabled(conn, id, enabled))
}

#[tauri::command]
fn erase_action(state: tauri::State<'_, DbState>, id: i64) -> Result<(), String> {
    with_conn(state, |conn| dict::erase_action(conn, id))
}

#[tauri::command]
fn set_action_policy(
    state: tauri::State<'_, DbState>,
    id: i64,
    input: dict::ActionPolicyInput,
) -> Result<dict::CareAction, String> {
    with_conn(state, |conn| dict::set_action_policy(conn, id, &input))
}

#[tauri::command]
fn save_food(
    state: tauri::State<'_, DbState>,
    input: dict::FoodInput,
) -> Result<care::Food, String> {
    with_conn(state, |conn| dict::save_food(conn, &input))
}

#[tauri::command]
fn set_food_enabled(
    state: tauri::State<'_, DbState>,
    id: i64,
    enabled: bool,
) -> Result<(), String> {
    with_conn(state, |conn| dict::set_food_enabled(conn, id, enabled))
}

#[tauri::command]
fn erase_food(state: tauri::State<'_, DbState>, id: i64) -> Result<(), String> {
    with_conn(state, |conn| dict::erase_food(conn, id))
}

#[tauri::command]
fn set_location_enabled(
    state: tauri::State<'_, DbState>,
    id: i64,
    enabled: bool,
) -> Result<(), String> {
    with_conn(state, |conn| colony::set_location_enabled(conn, id, enabled))
}

// ── 窝（票 02）──
// 窝/冬眠的增改会动状态与超期摘要 → 命令收尾统一刷托盘 tooltip（终局评审定点修 4，
// 与 log_care 同一锁外模式）。

#[tauri::command]
fn list_colonies(state: tauri::State<DbState>) -> Result<Vec<colony::Colony>, String> {
    with_conn(state, |conn| colony::list_colonies(conn, &colony::today_iso()))
}

#[tauri::command]
fn create_colony(
    state: tauri::State<DbState>,
    app: tauri::AppHandle,
    input: colony::ColonyInput,
) -> Result<colony::Colony, String> {
    let result = with_conn(state, |conn| {
        colony::create_colony(conn, &input, &colony::today_iso())
    });
    reminder::refresh_tray_tooltip(&app);
    result
}

#[tauri::command]
fn update_colony(
    state: tauri::State<DbState>,
    app: tauri::AppHandle,
    id: i64,
    input: colony::ColonyInput,
) -> Result<colony::Colony, String> {
    let result = with_conn(state, |conn| {
        colony::update_colony(conn, id, &input, &colony::today_iso())
    });
    reminder::refresh_tray_tooltip(&app);
    result
}

#[tauri::command]
fn archive_colony(
    state: tauri::State<DbState>,
    app: tauri::AppHandle,
    id: i64,
) -> Result<colony::Colony, String> {
    let result = with_conn(state, |conn| colony::archive_colony(conn, id, &colony::today_iso()));
    reminder::refresh_tray_tooltip(&app);
    result
}

#[tauri::command]
fn delete_colony(
    state: tauri::State<DbState>,
    app: tauri::AppHandle,
    id: i64,
) -> Result<(), String> {
    let result = with_conn(state, |conn| colony::delete_colony(conn, id));
    reminder::refresh_tray_tooltip(&app);
    result
}

// ── 地点（票 02）──

#[tauri::command]
fn list_locations(state: tauri::State<DbState>) -> Result<Vec<colony::Location>, String> {
    with_conn(state, colony::list_locations)
}

#[tauri::command]
fn save_location(
    state: tauri::State<DbState>,
    input: colony::LocationInput,
) -> Result<colony::Location, String> {
    with_conn(state, |conn| colony::save_location(conn, &input))
}

#[tauri::command]
fn erase_location(state: tauri::State<DbState>, id: i64) -> Result<(), String> {
    with_conn(state, |conn| colony::erase_location(conn, id))
}

// ── 冬眠（票 05）──

#[tauri::command]
fn start_hibernation(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    colony_id: i64,
    start_date: String,
    expected_end_date: String,
) -> Result<hibernation::Hibernation, String> {
    let result = with_conn(state, |conn| {
        hibernation::start_hibernation(
            conn,
            colony_id,
            &start_date,
            &expected_end_date,
            &colony::today_iso(),
        )
    });
    reminder::refresh_tray_tooltip(&app);
    result
}

#[tauri::command]
fn confirm_wake(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    colony_id: i64,
    actual_end_date: String,
) -> Result<hibernation::Hibernation, String> {
    let result = with_conn(state, |conn| {
        hibernation::confirm_wake(conn, colony_id, &actual_end_date, &colony::today_iso())
    });
    reminder::refresh_tray_tooltip(&app);
    result
}

#[tauri::command]
fn add_past_hibernation(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    colony_id: i64,
    start_date: String,
    end_date: String,
) -> Result<hibernation::Hibernation, String> {
    let result = with_conn(state, |conn| {
        hibernation::add_past_hibernation(conn, colony_id, &start_date, &end_date)
    });
    reminder::refresh_tray_tooltip(&app);
    result
}

#[tauri::command]
fn list_hibernations(
    state: tauri::State<'_, DbState>,
    colony_id: i64,
) -> Result<Vec<hibernation::Hibernation>, String> {
    with_conn(state, |conn| hibernation::list_hibernations(conn, colony_id))
}

/// 修改预计出眠日（票 09 停靠 D）：改 base 即可，未发的提醒按新日期自然重算（规则 2）。
#[tauri::command]
fn update_expected_end(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    colony_id: i64,
    new_expected_end_date: String,
) -> Result<hibernation::Hibernation, String> {
    let result = with_conn(state, |conn| {
        hibernation::update_expected_end(conn, colony_id, &new_expected_end_date)
    });
    reminder::refresh_tray_tooltip(&app);
    result
}

// ── 统计页（票 07）──

#[tauri::command]
fn get_stats(
    state: tauri::State<'_, DbState>,
    colony_id: Option<i64>,
    start_date: String,
    end_date: String,
) -> Result<stats::StatsPayload, String> {
    with_conn(state, |conn| stats::get_stats(conn, colony_id, &start_date, &end_date))
}

/// 全部记录里最早的 occurred_at 日期（前端「全部」范围下界用；无记录为 null）。
#[tauri::command]
fn earliest_log_date(state: tauri::State<'_, DbState>) -> Result<Option<String>, String> {
    with_conn(state, stats::earliest_log_date)
}

// ── 设置与通知（票 06）──

#[tauri::command]
fn get_settings(state: tauri::State<'_, DbState>) -> Result<settings::AppSettings, String> {
    with_conn(state, settings::get_settings)
}

#[tauri::command]
fn set_settings(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    input: settings::AppSettings,
) -> Result<settings::AppSettings, String> {
    let saved = with_conn(state, |conn| settings::set_settings(conn, &input));
    // 半成功语义（终局评审定点修 2）：自启同步失败不再整条 Err——设置已落库，
    // 下次启动还会按设置重新同步收敛；tooltip 刷新放最后，成功失败都执行（无 ? 短路）。
    let outcome = match saved {
        Ok(effective) => {
            let sync = apply_autostart(&app, effective.autostart_enabled);
            settle_settings_save(Ok(effective), sync)
        }
        Err(e) => settle_settings_save(Err(e), Ok(())),
    };
    reminder::refresh_tray_tooltip(&app);
    outcome
}

/// 设置保存的 command 层裁决（纯函数，测试用）：落库结果 + 自启同步结果 → 前端口径。
/// 落库失败恒报错；自启同步失败只记日志、不吞掉已保存的设置（设置是权威，
/// 启动时按设置重新对齐插件——与启动路径同一宽宽策略）。
fn settle_settings_save(
    saved: Result<settings::AppSettings, String>,
    autostart_sync: Result<(), String>,
) -> Result<settings::AppSettings, String> {
    let effective = saved?;
    if let Err(e) = autostart_sync {
        eprintln!("[autostart] 保存设置后同步开机自启失败（下次启动按设置重试）: {e}");
    }
    Ok(effective)
}

/// 把开机自启插件状态对齐设置值（票 09：读改插件状态 + settings 同步）。
fn apply_autostart(app: &tauri::AppHandle, enabled: bool) -> Result<(), String> {
    use tauri_plugin_autostart::ManagerExt;
    let result = if enabled {
        app.autolaunch().enable()
    } else {
        app.autolaunch().disable()
    };
    result.map_err(|e| format!("同步开机自启状态失败（目标：{enabled}）: {e}"))
}

/// 发送测试通知（设置弹窗按钮；不经开关与台账，排障用）。
#[tauri::command]
fn send_test_notification(app: tauri::AppHandle) -> Result<(), String> {
    reminder::send_test_notification(&app)
}

// ── 系统级数据出口（票 09）：打开数据文件夹 / 安全备份 / 导出 ──

/// 打开数据文件夹（opener 打开 app data 目录；目录不存在先创建）。
#[tauri::command]
fn reveal_data_folder(app: tauri::AppHandle) -> Result<String, String> {
    use tauri_plugin_opener::OpenerExt;
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("解析数据目录失败: {e}"))?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建数据目录失败: {e}"))?;
    app.opener()
        .open_path(dir.to_string_lossy(), None::<&str>)
        .map_err(|e| format!("打开数据文件夹失败: {e}"))?;
    Ok(dir.to_string_lossy().to_string())
}

/// rfd 保存对话框选目标路径（取消返回 None）。同名文件覆盖确认由对话框承担。
async fn pick_save_path(
    default_name: &str,
    filter_name: &str,
    extensions: &[&str],
) -> Option<std::path::PathBuf> {
    rfd::AsyncFileDialog::new()
        .set_file_name(default_name)
        .add_filter(filter_name, extensions)
        .save_file()
        .await
        .map(|handle| handle.path().to_path_buf())
}

/// 今日日期戳（备份/导出默认文件名用）。
fn date_stamp() -> String {
    chrono::Local::now().format("%Y%m%d").to_string()
}

/// 安全备份：rfd 选目标 → 短暂拿锁挡住并发写 → 拷贝库文件
/// （journal_mode=DELETE，拷贝即完整，评审附录规则 11）。用户取消返回 None。
#[tauri::command]
async fn backup_to(state: tauri::State<'_, DbState>) -> Result<Option<String>, String> {
    let db_path = state.1.clone();
    let default_name = format!("ant-feeding-log-backup-{}.db", date_stamp());
    let Some(target) = pick_save_path(&default_name, "SQLite 数据库", &["db"]).await else {
        return Ok(None);
    };
    // 拷贝期间短暂持锁：保证没有并发写（对话框阶段不持锁，不卡其他命令）
    let _guard = state.0.lock().map_err(|e| e.to_string())?;
    system::backup_db_file(&db_path, &target)?;
    Ok(Some(target.to_string_lossy().to_string()))
}

/// 导出归档（评审附录规则 11：归档带走，不是恢复通道）：csv=记录流水一行一条
/// （BOM，Excel 直开不乱码）；json=全库数据结构化。用户取消返回 None。
#[tauri::command]
async fn export_data(
    state: tauri::State<'_, DbState>,
    format: String,
) -> Result<Option<String>, String> {
    let stamp = date_stamp();
    let (default_name, filter_name, ext): (String, &str, &str) = match format.as_str() {
        "csv" => (
            format!("ant-feeding-log-export-{stamp}.csv"),
            "CSV（逗号分隔）",
            "csv",
        ),
        "json" => (format!("ant-feeding-log-export-{stamp}.json"), "JSON 文件", "json"),
        other => return Err(format!("未知导出格式：{other}（支持 csv / json）")),
    };
    let Some(target) = pick_save_path(&default_name, filter_name, &[ext]).await else {
        return Ok(None);
    };
    with_conn(state, |conn| match format.as_str() {
        "csv" => system::export_csv_to(conn, &target),
        _ => system::export_json_to(conn, &target),
    })?;
    Ok(Some(target.to_string_lossy().to_string()))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        // 开机自启（票 09）：状态权威在 settings，启动/改设置时对齐插件
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // 库文件放系统应用数据目录（Windows: %APPDATA%\<identifier>\），非项目目录。
            let data_dir = app.path().app_data_dir()?;
            let db_path = data_dir.join(db::DB_FILE_NAME);
            let conn = db::open_and_migrate(&db_path).map_err(|e| e.to_string())?;
            let autostart_on = settings::get_settings(&conn)
                .map(|s| s.autostart_enabled)
                .unwrap_or(true);
            app.manage(DbState(Mutex::new(conn), db_path));
            // 托盘常驻 + 提醒调度（启动即查一次，此后每 30 分钟；评审附录规则 1）。
            reminder::setup_tray(app)?;
            reminder::spawn_scheduler(app.handle().clone());
            // 关窗 = 最小化到托盘（票 09 验收 1）：拦截关闭请求只隐藏，托盘「退出」才真退。
            if let Some(window) = app.get_webview_window("main") {
                let win = window.clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = win.hide();
                    }
                });
            }
            // 开机自启默认开（settings 预置行=1 即「首次启动写入」）；失败只打日志不拦启动。
            if let Err(e) = apply_autostart(app.handle(), autostart_on) {
                eprintln!("[autostart] 启动时同步开机自启失败: {e}");
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            health_check,
            log_care,
            list_foods,
            list_logs,
            update_log,
            delete_log,
            list_actions,
            save_action,
            set_action_enabled,
            erase_action,
            set_action_policy,
            save_food,
            set_food_enabled,
            erase_food,
            set_location_enabled,
            list_colonies,
            create_colony,
            update_colony,
            archive_colony,
            delete_colony,
            list_locations,
            save_location,
            erase_location,
            start_hibernation,
            confirm_wake,
            add_past_hibernation,
            list_hibernations,
            update_expected_end,
            get_settings,
            set_settings,
            send_test_notification,
            reveal_data_folder,
            backup_to,
            export_data,
            get_stats,
            earliest_log_date,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

// ── 测试：command 层裁决语义（纯函数，spec「Testing Decisions」）──────────

#[cfg(test)]
mod tests {
    use super::settle_settings_save;
    use crate::settings::AppSettings;

    #[test]
    fn autostart_sync_failure_does_not_sink_saved_settings() {
        // 终局评审定点修 2：设置落库成功后自启同步失败 → 不整条 Err，
        // 已保存的设置照常返回（下次启动按设置重新同步收敛）
        let saved = Ok(AppSettings {
            autostart_enabled: false,
            ..Default::default()
        });
        let outcome = settle_settings_save(saved, Err("注册表被组策略锁住".into()));
        let effective = outcome.expect("自启同步失败不应吞掉已落库的设置");
        assert!(!effective.autostart_enabled, "返回收敛后的生效值");

        // 落库失败恒报错（自启同步成功也救不了落库失败）
        let outcome = settle_settings_save(Err("数据库已锁定".into()), Ok(()));
        let err = outcome.unwrap_err();
        assert!(err.contains("锁定"), "实际错误：{err}");
    }
}
