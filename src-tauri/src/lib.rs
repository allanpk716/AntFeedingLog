mod applog;
mod auto_backup;
mod backup_config;
mod backup_pkg;
mod care;
mod colony;
mod data_meta;
mod data_version;
mod db;
mod dict;
mod firewall;
mod hibernation;
mod nest_checkin;
mod netseg;
mod photo;
mod pushover;
mod reminder;
mod restore;
mod restore_pkg;
mod settings;
mod stats;
mod system;
mod updater;
mod webui_config;
mod webui_args;
mod webui_server;

/// 全链冒烟（数据安全二期票 05）：造数据 → 自动备份 → 改数据 → 恢复 的端到端
/// 断言。只在测试构建编译。
#[cfg(test)]
mod full_chain;

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};

use rusqlite::Connection;
use serde::Serialize;
use tauri::Manager;

/// 健康检查返回体：当前库的 schema 版本。
#[derive(Serialize)]
pub struct SchemaInfo {
    schema_version: i64,
}

/// 数据库连接托管在应用状态里：Rust 是数据层唯一属主，前端只经 command 读写。
/// 锁包一层 Arc（票 04）：网页端 HTTP 服务（webui_server）要拿同一把库锁派发
/// 命令——单连接不变，只是让 tokio 任务能持有克隆的锁柄。`.1` 是库文件路径
/// （安全备份直接拷这个文件，journal_mode=DELETE 拷贝即完整）。
pub struct DbState(Arc<Mutex<Connection>>, std::path::PathBuf);

impl DbState {
    /// 库锁的共享柄（HTTP 服务与桌面 IPC 同锁串行的接缝）。
    pub(crate) fn conn_handle(&self) -> Arc<Mutex<Connection>> {
        self.0.clone()
    }
}

/// 借出连接的统一入口（锁被毒化时转成前端可见的错误串）。`with_conn`（桌面
/// Tauri command）与网页端 HTTP 派发（webui_server::dispatch_command）共用本
/// 函数：同一把锁、同一禁写窗口、同一失败日志，两条通路一个口径。
/// 票 04 禁写窗口：快照完成后到进程退出前，一切请求在此拒绝。取舍：挂在统一
/// 入口把读也一并拦下——窗口只有安装器拉起前的一瞬（随后进程退出），读失败
/// 只是前端一次报错；而逐个写命令去挂太散、未来新命令可能漏挂。
/// 复查必须在锁内（评审 R1 TOCTOU）：置位发生在快照的持锁段，若在拿锁前检查，
/// 置位前已通过检查、正阻塞在 lock 上的在途写会在快照放锁后落库。
pub(crate) fn run_with_conn<T>(
    conn_mutex: &Mutex<Connection>,
    f: impl FnOnce(&rusqlite::Connection) -> Result<T, String>,
) -> Result<T, String> {
    let conn = match conn_mutex.lock() {
        Ok(conn) => conn,
        Err(e) => {
            // 票 01：命令失败落日志（库锁不可用）
            applog::log_error(&format!("命令执行失败（库锁不可用）: {e}"));
            return Err(e.to_string());
        }
    };
    if let Err(e) = updater::ensure_writable() {
        // 票 01：禁写窗口拒绝也留痕（安装窗口期的在途写，排查升级问题时用）
        applog::log_error(&format!("命令执行失败（更新安装禁写窗口）: {e}"));
        return Err(e);
    }
    let result = f(&conn);
    if let Err(e) = &result {
        // 票 01：命令失败落日志（D7 错误记录——统一入口一处挂，覆盖全部库命令）
        applog::log_error(&format!("命令执行失败: {e}"));
    }
    result
}

/// 桌面命令入口：State 里借出锁交给 [`run_with_conn`]。
fn with_conn<T>(
    state: tauri::State<'_, DbState>,
    f: impl FnOnce(&rusqlite::Connection) -> Result<T, String>,
) -> Result<T, String> {
    run_with_conn(&state.0, f)
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
    if result.is_ok() {
        trigger_after_write(&app); // 票 03：业务写入成功 → 自动备份触发点
    }
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

/// 按窝按月的记录摘要（交互第三轮票 05）：日历标记 + 重复黄条数据源。
/// `excludeLogId`：编辑场景传当前记录 id——正在编辑的这条不计入，防自计数误报。
#[tauri::command]
fn colony_month_records(
    state: tauri::State<'_, DbState>,
    colony_id: i64,
    year: i64,
    month: i64,
    exclude_log_id: Option<i64>,
) -> Result<Vec<care::MonthDayRecords>, String> {
    with_conn(state, |conn| {
        care::colony_month_records(conn, colony_id, year, month, exclude_log_id)
    })
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
    if result.is_ok() {
        trigger_after_write(&app);
    }
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
    if result.is_ok() {
        trigger_after_write(&app);
    }
    result
}

// ── 巢况登记（webui-checkin 票 02）──
// 巢况永不参与提醒/催促：不进维护操作清单、不调 refresh_tray_tooltip
// （托盘 tooltip 只算喂食/维护超期）；仅数据写入触发自动备份。

#[tauri::command]
fn save_checkin(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    input: nest_checkin::CheckinInput,
) -> Result<nest_checkin::NestCheckin, String> {
    let result = with_conn(state, |conn| {
        nest_checkin::save_checkin(conn, &input, &colony::today_iso(), &care::now_local())
    });
    if result.is_ok() {
        trigger_after_write(&app);
    }
    result
}

#[tauri::command]
fn list_checkins(
    state: tauri::State<'_, DbState>,
    colony_id: i64,
) -> Result<Vec<nest_checkin::NestCheckin>, String> {
    with_conn(state, |conn| nest_checkin::list_checkins(conn, colony_id))
}

#[tauri::command]
fn update_checkin(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    id: i64,
    input: nest_checkin::CheckinUpdateInput,
) -> Result<nest_checkin::NestCheckin, String> {
    let result = with_conn(state, |conn| {
        nest_checkin::update_checkin(conn, id, &input, &colony::today_iso())
    });
    if result.is_ok() {
        trigger_after_write(&app);
    }
    result
}

#[tauri::command]
fn delete_checkin(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    id: i64,
) -> Result<(), String> {
    let photos_root = current_data_dir()?.join(photo::PHOTOS_DIR_NAME);
    let result = with_conn(state, |conn| {
        // 删除顺序（票 07，规格 E）：先库事务删元数据提交，再删文件；
        // 文件删失败仅产生孤儿（下轮巡检隔离），不回滚库。
        let rel_paths = photo::collect_checkin_photo_paths(conn, id)?;
        nest_checkin::delete_checkin(conn, id)?;
        report_photo_orphans(&photo::delete_photo_files(&photos_root, &rel_paths));
        Ok(())
    });
    if result.is_ok() {
        trigger_after_write(&app);
    }
    result
}

#[tauri::command]
fn get_checkin_digest(
    state: tauri::State<'_, DbState>,
    colony_id: i64,
) -> Result<nest_checkin::CheckinDigest, String> {
    with_conn(state, |conn| {
        nest_checkin::digest_for_colony(conn, colony_id, &colony::today_iso())
    })
}

// ── 巢况照片（webui-checkin 票 07）：上传校验重编码 / 写入协议 / 孤儿治理 ──
// 纯核全在 photo.rs；这里只是 Tauri 薄包装。桌面 5 命令均**不入网页端白名单**
//（照片上传/读取的 HTTP 通路随票 08 单独登记，pick/get_photo_abs_dir 这类
// 桌面专属出口永不出网）。

/// 照片文件清理的孤儿记账：失败仅遗留孤儿（下轮巡检隔离），绝不回滚库（规格 E）。
fn report_photo_orphans(failures: &[String]) {
    for f in failures {
        applog::log_error(&format!("照片文件删除失败（遗留孤儿，待巡检隔离）: {f}"));
    }
}

/// rfd 系统文件选择框多选巢况照片（取消返回 None）。async command：对话框
/// 不能占主线程（评审 R1-2，与 pick_backup_dir 同理）。图片过滤器只是少让
/// 用户选错——白名单真闸在 Rust 校验链（按魔数探测，不信扩展名）。
#[tauri::command]
async fn pick_photo_files() -> Result<Option<Vec<String>>, String> {
    let picked = rfd::AsyncFileDialog::new()
        .set_title("选择巢况照片")
        .add_filter("图片（JPEG/PNG/WebP）", &["jpg", "jpeg", "png", "webp"])
        .pick_files()
        .await
        .map(|files| {
            files
                .iter()
                .map(|handle| handle.path().to_string_lossy().to_string())
                .collect::<Vec<_>>()
        });
    Ok(picked)
}

/// 上传巢况照片：读盘 → 校验链重编码（格式白名单/尺寸/解码炸弹头/缩放/JPEG
/// 重编码清 EXIF）→ 写入协议落盘插库。解码/编码是 CPU 重活且不碰库，放在
/// spawn_blocking 且库锁外做（锁内只留写入协议的几条小 IO + 插行）；成功 =
/// 元数据已提交（「照片写库即算当日新数据」的自动备份语义以它为准），走
/// trigger_after_write 与其他写命令同一咽喉。
#[tauri::command]
async fn attach_photos(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    checkin_id: i64,
    paths: Vec<String>,
) -> Result<Vec<nest_checkin::NestPhotoMeta>, String> {
    let photos_root = current_data_dir()?.join(photo::PHOTOS_DIR_NAME);
    let conn_handle = state.inner().conn_handle();
    let outcome = tauri::async_runtime::spawn_blocking(move || {
        let uploads = photo::read_photo_files(&paths)?;
        let uploads = photo::process_uploads(uploads)?;
        run_with_conn(&conn_handle, |conn| {
            photo::attach_photos(conn, &photos_root, checkin_id, &uploads)
        })
    })
    .await
    .map_err(|e| format!("照片上传任务异常退出: {e}"))?;
    if outcome.is_ok() {
        trigger_after_write(&app);
    }
    outcome
}

/// 照片根目录绝对路径：桌面显示本地图用——前端 convertFileSrc 把
/// `<数据目录>/photos/<relPath>` 变成 asset 协议 URL（scope 限定 photos/
/// 的配置在 tauri.conf.json app.security.assetProtocol）。
#[tauri::command]
fn get_photo_abs_dir() -> Result<String, String> {
    let dir = current_data_dir()?.join(photo::PHOTOS_DIR_NAME);
    Ok(dir.to_string_lossy().to_string())
}

/// 孤儿照片隔离区现状（设置页数据 tab「巢况照片孤儿」区）。
#[tauri::command]
fn list_orphan_photos() -> Result<photo::OrphanPhotoStats, String> {
    let photos_root = current_data_dir()?.join(photo::PHOTOS_DIR_NAME);
    Ok(photo::orphan_stats(&photos_root))
}

/// 一键清理孤儿照片（删除 photos/.orphan-* 隔离目录；前端两段确认后调用）。
/// 成败落流水（D7）。
#[tauri::command]
fn clean_orphan_photos() -> Result<photo::OrphanCleanOutcome, String> {
    let photos_root = current_data_dir()?.join(photo::PHOTOS_DIR_NAME);
    let outcome = photo::clean_orphans(&photos_root);
    if outcome.errors.is_empty() {
        applog::log_action(&format!(
            "孤儿照片清理完成：删除 {} 个隔离目录，释放 {} 字节",
            outcome.removed_dirs, outcome.freed_bytes
        ));
    } else {
        applog::log_error(&format!(
            "孤儿照片清理部分失败（已删 {} 个目录）: {}",
            outcome.removed_dirs,
            outcome.errors.join("；")
        ));
    }
    Ok(outcome)
}

// ── 字典管理与操作性质设置（票 04）──

#[tauri::command]
fn list_actions(state: tauri::State<'_, DbState>) -> Result<Vec<dict::CareAction>, String> {
    with_conn(state, dict::list_actions)
}

#[tauri::command]
fn save_action(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    input: dict::ActionInput,
) -> Result<dict::CareAction, String> {
    let result = with_conn(state, |conn| dict::save_action(conn, &input));
    if result.is_ok() {
        trigger_after_write(&app);
    }
    result
}

#[tauri::command]
fn set_action_enabled(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    id: i64,
    enabled: bool,
) -> Result<(), String> {
    let result = with_conn(state, |conn| dict::set_action_enabled(conn, id, enabled));
    if result.is_ok() {
        trigger_after_write(&app);
    }
    result
}

#[tauri::command]
fn erase_action(state: tauri::State<'_, DbState>, app: tauri::AppHandle, id: i64) -> Result<(), String> {
    let result = with_conn(state, |conn| dict::erase_action(conn, id));
    if result.is_ok() {
        trigger_after_write(&app);
    }
    result
}

#[tauri::command]
fn set_action_policy(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    id: i64,
    input: dict::ActionPolicyInput,
) -> Result<dict::CareAction, String> {
    let result = with_conn(state, |conn| dict::set_action_policy(conn, id, &input));
    if result.is_ok() {
        trigger_after_write(&app);
    }
    result
}

#[tauri::command]
fn save_food(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    input: dict::FoodInput,
) -> Result<care::Food, String> {
    let result = with_conn(state, |conn| dict::save_food(conn, &input));
    if result.is_ok() {
        trigger_after_write(&app);
    }
    result
}

#[tauri::command]
fn set_food_enabled(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    id: i64,
    enabled: bool,
) -> Result<(), String> {
    let result = with_conn(state, |conn| dict::set_food_enabled(conn, id, enabled));
    if result.is_ok() {
        trigger_after_write(&app);
    }
    result
}

#[tauri::command]
fn erase_food(state: tauri::State<'_, DbState>, app: tauri::AppHandle, id: i64) -> Result<(), String> {
    let result = with_conn(state, |conn| dict::erase_food(conn, id));
    if result.is_ok() {
        trigger_after_write(&app);
    }
    result
}

#[tauri::command]
fn set_location_enabled(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    id: i64,
    enabled: bool,
) -> Result<(), String> {
    let result = with_conn(state, |conn| colony::set_location_enabled(conn, id, enabled));
    if result.is_ok() {
        trigger_after_write(&app);
    }
    result
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
    if result.is_ok() {
        trigger_after_write(&app);
    }
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
    if result.is_ok() {
        trigger_after_write(&app);
    }
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
    if result.is_ok() {
        trigger_after_write(&app);
    }
    result
}

#[tauri::command]
fn delete_colony(
    state: tauri::State<DbState>,
    app: tauri::AppHandle,
    id: i64,
) -> Result<(), String> {
    let photos_root = current_data_dir()?.join(photo::PHOTOS_DIR_NAME);
    let result = with_conn(state, |conn| {
        // 删窝级联（票 07）：库事务删行提交后清照片文件；失败仅孤儿，不回滚库
        let rel_paths = photo::collect_colony_photo_paths(conn, id)?;
        colony::delete_colony(conn, id)?;
        report_photo_orphans(&photo::delete_photo_files(&photos_root, &rel_paths));
        Ok(())
    });
    reminder::refresh_tray_tooltip(&app);
    if result.is_ok() {
        trigger_after_write(&app);
    }
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
    app: tauri::AppHandle,
    input: colony::LocationInput,
) -> Result<colony::Location, String> {
    let result = with_conn(state, |conn| colony::save_location(conn, &input));
    if result.is_ok() {
        trigger_after_write(&app);
    }
    result
}

#[tauri::command]
fn erase_location(state: tauri::State<DbState>, app: tauri::AppHandle, id: i64) -> Result<(), String> {
    let result = with_conn(state, |conn| colony::erase_location(conn, id));
    if result.is_ok() {
        trigger_after_write(&app);
    }
    result
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
    if result.is_ok() {
        trigger_after_write(&app);
    }
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
    if result.is_ok() {
        trigger_after_write(&app);
    }
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
    if result.is_ok() {
        trigger_after_write(&app);
    }
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
    if result.is_ok() {
        trigger_after_write(&app);
    }
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

/// 设置保存的 command 层裁决：落库结果 + 自启同步结果 → 前端口径。
/// 落库失败恒报错；自启同步失败只记日志、不吞掉已保存的设置（设置是权威，
/// 启动时按设置重新对齐插件——与启动路径同一宽宽策略）。
/// 终局评审 D7：自启同步成败落日志（窗口化应用 stderr 丢失等于不可见）。
fn settle_settings_save(
    saved: Result<settings::AppSettings, String>,
    autostart_sync: Result<(), String>,
) -> Result<settings::AppSettings, String> {
    let effective = saved?;
    match autostart_sync {
        Ok(()) => applog::log_action(&format!(
            "开机自启已同步：{}",
            if effective.autostart_enabled { "开" } else { "关" }
        )),
        Err(e) => applog::log_error(&format!(
            "保存设置后同步开机自启失败（下次启动按设置重试）: {e}"
        )),
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
/// 双通道各测各的：桌面失败不影响手机，pushover=None 表示未配置。
/// ⚠ 严禁注册进网页端 HTTP 白名单：设置面命令桌面专属（规格 H；webui-checkin 票 11）。
#[tauri::command]
fn send_test_notification(app: tauri::AppHandle) -> reminder::TestNotifyOutcome {
    reminder::send_test_notification_dual(&app)
}

/// Pushover 配置状态探测（webui-checkin 票 11 三态）：只报生效来源
///（应用内/环境变量/未配置）与是否已配置，不回报值。
/// ⚠ 严禁注册进网页端 HTTP 白名单：设置面命令桌面专属（规格 H；同票 03 禁入先例）。
#[tauri::command]
fn pushover_status(state: tauri::State<DbState>) -> Result<pushover::PushoverStatus, String> {
    with_conn(state, pushover::status_from_db)
}

// ── 更新检查（票 02）──

/// 手动检查更新（设置页按钮，票 06 接 UI）：async command（评审 R1-2——同步
/// command 跑在主线程，阻塞网络会冻住整个 UI）。三态中"检查失败"折为 Err
/// 一次性展示；成功返回 `{"status":"up_to_date"}` 或 `{"status":"update_available",..}`。
#[tauri::command]
async fn check_update_now(app: tauri::AppHandle) -> Result<updater::CheckOutcome, String> {
    let outcome = updater::manual_check(app).await;
    // 终局评审 D7：非库命令失败也落日志（不经 with_conn，统一入口覆盖不到）
    if let Err(e) = &outcome {
        applog::log_error(&format!("手动检查更新失败: {e}"));
    }
    outcome
}

// ── 确认升级与安装（票 05）──

/// 确认安装防重入标志：确认流一跑几十秒（下载），双击/狂点会并发拉起两条流
///（两个安装器、两份下载），用一个原子位把第二条挡在门外。
static CONFIRM_IN_FLIGHT: AtomicBool = AtomicBool::new(false);

/// Drop 兜底复位防重入标志（检查数据目录失败等早退路径也复位）。
struct ConfirmInFlightGuard;
impl Drop for ConfirmInFlightGuard {
    fn drop(&mut self) {
        CONFIRM_IN_FLIGHT.store(false, Ordering::SeqCst);
    }
}

/// 确认升级并安装（设置页按钮，票 06 接 UI）。编排（再次检查 → 写 pending 标记
/// → 下载 → install）与失败路径在 updater::run_confirm_flow 纯函数层（FakeSteps
/// 测试锚定）；本 command 只做三件事：
/// - 防重入（CONFIRM_IN_FLIGHT）；
/// - 解析数据目录（标记落盘点）；
/// - 把编排丢进阻塞线程池：生产步骤内部用 block_on 桥接插件 async API，只允许
///   在非运行时线程上做（spawn_blocking 线程不是 tokio worker，阻塞安全），
///   下载是长网络任务也不占异步 worker。
/// 返回：成功 `{"status":"install_started",..}`（Windows 下进程随即退出）；
/// 下载/安装失败 `{"status":"install_failed",..}`（进程存活，禁写标志已被编排
/// 复位——票 04 评审 M-1 契约）；检查失败/写标记失败折为 Err 一次性展示。
#[tauri::command]
async fn confirm_and_install(app: tauri::AppHandle) -> Result<updater::InstallOutcome, String> {
    if CONFIRM_IN_FLIGHT.swap(true, Ordering::SeqCst) {
        return Err("已有安装流程正在进行，请稍候".into());
    }
    let _guard = ConfirmInFlightGuard;
    let data_dir = match app.path().app_data_dir() {
        Ok(dir) => dir,
        Err(e) => {
            let msg = format!("解析数据目录失败: {e}");
            applog::log_error(&format!("确认安装失败: {msg}"));
            return Err(msg);
        }
    };
    let outcome = tauri::async_runtime::spawn_blocking(move || {
        let steps = updater::PluginConfirmSteps::new(app);
        updater::run_confirm_flow(&data_dir, &steps)
    })
    .await
    .map_or_else(
        |e| {
            // 编排线程 panic（理论外路径，如插件步骤 panics）：panic 点可能已在
            // install 置位禁写之后——进程存活就必须复位（M-1 契约精神：失败后
            // 应用不得卡在只读态）；标记保留，交启动判定兜底。
            updater::set_write_blocked(false);
            applog::log_error(&format!("确认安装任务异常退出: {e}"));
            Err(format!("确认安装任务异常退出: {e}"))
        },
        Ok,
    )?;
    // 终局评审 D7：安装结果落流水（下载完成行在 PluginConfirmSteps::download；
    // 失败原因随 message 带全，目标版本齐全，供日志侧对账"想升到哪、成没成"）
    match &outcome {
        Ok(updater::InstallOutcome::InstallStarted { version }) => {
            applog::log_action(&format!("更新安装已启动（目标 v{version}），进程即将退出"));
        }
        Ok(updater::InstallOutcome::InstallFailed { version, message }) => {
            applog::log_error(&format!("更新安装失败（目标 v{version}）: {message}"));
        }
        Err(e) => {
            // 确认流在下载前中止（再次检查失败 / 写 pending 标记失败）
            applog::log_error(&format!("确认安装流程中止: {e}"));
        }
    }
    outcome
}

/// 查询更新状态（票 05）：`idle` 无残留 / `last_install_succeeded` 上次升级成功
///（可提示"已升级到 vX"）/ `last_install_incomplete` 上次升级未完成（含目标
/// 版本，UI 据此给重试/手动下载引导）。状态由启动判定与确认流失败路径暂存。
#[tauri::command]
fn get_update_state() -> Result<updater::UpdateState, String> {
    Ok(updater::current_update_state())
}

// ── 更新 UI 的轻量出口（release-update 票 06 设置页「更新」节）──

/// 当前应用版本（设置页展示）。与更新流同源（package_info，即 tauri.conf.json
/// 的 version），快照/启动判定/页面展示三方口径一致。
#[tauri::command]
fn get_app_version(app: tauri::AppHandle) -> String {
    app.package_info().version.to_string()
}

/// 打开发布页（升级未完成引导 / 安装失败的手动下载出口）。走 Rust 侧 opener
/// 插件（与 reveal_data_folder 同款），前端不需要 opener JS 权限；
/// capabilities 保持不加任何 updater/opener 权限。
#[tauri::command]
fn open_releases_page(app: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let result = app
        .opener()
        .open_url(updater::RELEASES_PAGE_URL, None::<&str>)
        .map_err(|e| format!("打开发布页失败: {e}"));
    // 终局评审 D7：非库命令失败也落日志（不经 with_conn，统一入口覆盖不到）
    if let Err(e) = &result {
        applog::log_error(&format!("打开发布页失败: {e}"));
    }
    result
}

// ── 网页端设置（webui-checkin 票 03）：网段枚举 / 配置与凭证 / 防火墙联动 ──
// 配置存数据目录 webui-config.json（库外，恢复不触碰，同 backup-config.json 取舍）。
// 票 04 起：保存成功后按新配置对齐内嵌 HTTP 服务（起/停/改端口即重启）；
// 凭证与网段由服务每请求现读，重生成/改网段即刻生效，无须动服务。

/// 枚举本机网段（虚拟化噪音已滤、NetBird 段置顶标名）。async：PowerShell 枚举
/// 不能占主线程（评审 R1-2 同款，与 pick_backup_dir 同理）。
#[tauri::command]
async fn list_network_segments() -> Result<Vec<netseg::NetworkSegment>, String> {
    tauri::async_runtime::spawn_blocking(netseg::enumerate)
        .await
        .map_err(|e| format!("网段枚举任务异常退出: {e}"))?
}

/// 读网页端配置；凭证为空顺路补生成并落盘（首次打开设置页即有凭证可用）。
///
/// ⚠ **严禁注册进网页端 HTTP 白名单**：本命令响应含**访问凭证明文**，只允许
/// 桌面 Tauri IPC 调用——webui_server::WEBUI_COMMANDS 的
/// `registry_excludes_forbidden_commands` 测试钉死本命令不得入表（票 03 评审
/// Important 的落点），一旦暴露，凭证即泄漏给页面侧脚本。
#[tauri::command]
fn get_webui_config() -> Result<webui_config::WebUiConfig, String> {
    let data_dir = current_data_dir()?;
    webui_config::load_ready(&data_dir)
}

/// save_webui_config 返回体（半成功语义，同 settle_settings_save 取舍）：配置
/// 落盘是事实，防火墙失败不吞掉它——结构化回传错误 + 现成手动命令，前端分开展示。
/// 服务起停结果同理（票 04）：端口被占用 → 配置已保存但 server_error 给人话
/// 提示（设置页回显），绝不把已保存的配置标成失败。
#[derive(Serialize)]
pub struct WebUiSaveOutcome {
    pub config: webui_config::WebUiConfig,
    pub firewall_ok: bool,
    pub firewall_error: Option<String>,
    pub firewall_manual_cmd: Option<String>,
    /// 网页端服务按新配置对齐成功（含「停用即关停」）。
    pub server_ok: bool,
    /// 服务起停失败的人话原因（端口占用等）；None = 正常。
    pub server_error: Option<String>,
}

/// 保存网页端配置（enabled/segments/port；CIDR 与端口校验在 webui_config 纯核，
/// 绕过前端的直调在此兜底拦下）。成功后两件联动，都走阻塞线程池/异步不冻 UI：
/// 防火墙规则（开 = 先删后建幂等，关 = 删规则，UAC 弹窗）+ 内嵌 HTTP 服务按新
/// 配置对齐（票 04：起/停/改端口重启；端口被占用不崩溃，人话错误回设置页）。
/// 防火墙与服务成败都落流水（D7），且都不吞掉已保存的配置（半成功语义）。
///
/// ⚠ **严禁注册进网页端 HTTP 白名单**：本命令可改受信网段/端口/开关（闸一
/// 安全配置），只能由桌面设置页发起；网页端的功能面不含任何设置操作（规格 H），
/// webui_server::WEBUI_COMMANDS 的禁入断言测试钉死本命令不得入表。
#[tauri::command]
async fn save_webui_config(
    app: tauri::AppHandle,
    input: webui_config::WebUiSaveInput,
) -> Result<WebUiSaveOutcome, String> {
    let data_dir = current_data_dir()?;
    let saved = webui_config::save_webui(&data_dir, &input)?;
    let segments_text = if saved.segments.is_empty() {
        "无".to_string()
    } else {
        saved.segments.join(",")
    };
    applog::log_action(&format!(
        "网页端配置已保存：开关{}，端口 {}，网段 {}",
        if saved.enabled { "开" } else { "关" },
        saved.port,
        segments_text,
    ));
    let fw_segments = saved.segments.clone();
    let fw_port = saved.port;
    let fw_enabled = saved.enabled;
    let fw = tauri::async_runtime::spawn_blocking(move || {
        firewall::sync(fw_enabled, &fw_segments, fw_port)
    })
    .await
    .map_err(|e| format!("防火墙同步任务异常退出: {e}"))?;
    let (firewall_ok, firewall_error, firewall_manual_cmd) = match fw {
        Ok(()) => {
            applog::log_action("防火墙规则已同步（AntFeedingLog WebUI）");
            (true, None, None)
        }
        Err(e) => {
            applog::log_error(&format!("防火墙规则同步失败: {}（手动命令已给前端）", e.message));
            (false, Some(e.message), Some(e.manual_cmd))
        }
    };
    // 服务对齐（票 04）：失败不回滚已落盘的配置，人话错误随回传体给设置页
    let server_outcome = sync_webui_server(&app, &saved).await;
    let (server_ok, server_error) = match server_outcome {
        Ok(()) => (true, None),
        Err(e) => (false, Some(e)),
    };
    Ok(WebUiSaveOutcome {
        config: saved,
        firewall_ok,
        firewall_error,
        firewall_manual_cmd,
        server_ok,
        server_error,
    })
}

/// 按配置对齐网页端服务（启动序列与 save_webui_config 共用入口）。
/// 从 Tauri 状态拿运行时槽位与共享依赖；状态未就绪（极端早退路径）报人话错误。
async fn sync_webui_server(app: &tauri::AppHandle, cfg: &webui_config::WebUiConfig) -> Result<(), String> {
    let runtime = app
        .try_state::<webui_server::WebUiRuntime>()
        .ok_or_else(|| "网页端服务运行时未就绪".to_string())?;
    let deps = app
        .try_state::<Arc<webui_server::SharedDeps>>()
        .ok_or_else(|| "网页端服务依赖未就绪".to_string())?;
    runtime.sync(deps.inner().clone(), cfg).await
}

/// 重生成访问凭证（旧地址即刻作废 = 覆盖写）。成功记一条动作流水（D7）。
///
/// ⚠ **严禁注册进网页端 HTTP 白名单**：本命令返回**新凭证明文**且可直接作废
/// 全部旧地址（安全管理操作），只允许桌面设置页调用——webui_server 的
/// `registry_excludes_forbidden_commands` 测试钉死本命令不得入表。
/// 凭证重生成后旧地址即刻失效的机制：HTTP 服务每请求现读配置文件（webui_server
/// auth_mw 取舍注释），无需通知服务。
#[tauri::command]
fn regenerate_token() -> Result<webui_config::WebUiConfig, String> {
    let data_dir = current_data_dir()?;
    let cfg = webui_config::regenerate_token(&data_dir)?;
    applog::log_action("网页端访问凭证已重生成（旧地址即刻作废）");
    Ok(cfg)
}

/// 完整访问地址 `http://<IP>:<端口>/#token=<凭证>`：按配置第一个受信网段上的
/// 本机 IP 拼（多段取第一个）。网段当前不在线报错（如拔掉 NetBird 后）。
/// async：网卡枚举不能占主线程。
///
/// ⚠ **严禁注册进网页端 HTTP 白名单**：本命令响应就是**含凭证的完整访问地址**，
/// 只允许桌面设置页/向导调用——webui_server 的 `registry_excludes_forbidden_commands`
/// 测试钉死本命令不得入表，经网页端 API 取到它等于把进门凭证递给页面侧。
#[tauri::command]
async fn get_access_url() -> Result<String, String> {
    let data_dir = current_data_dir()?;
    let cfg = webui_config::load_ready(&data_dir)?;
    if cfg.segments.is_empty() {
        return Err("尚未选择受信网段，先在设置里勾选".into());
    }
    let first = cfg.segments[0].clone();
    let nics = tauri::async_runtime::spawn_blocking(netseg::list_nics)
        .await
        .map_err(|e| format!("网卡枚举任务异常退出: {e}"))??;
    let ip = netseg::pick_ip_for_segment(&nics, &first).ok_or_else(|| {
        format!("所选网段 {first} 上未发现本机地址（网段当前不在线？确认 NetBird/该网段已连接）")
    })?;
    Ok(webui_config::build_access_url(&ip, cfg.port, &cfg.token))
}

/// 网页端首启向导做过没有（缺省 = 未做，前端据此弹一次向导）。
#[tauri::command]
fn get_webui_wizard_done(state: tauri::State<'_, DbState>) -> Result<bool, String> {
    with_conn(state, settings::get_webui_wizard_done)
}

/// 标记网页端首启向导已处理（完成或跳过都写；幂等）。
#[tauri::command]
fn mark_webui_wizard_done(state: tauri::State<'_, DbState>) -> Result<(), String> {
    with_conn(state, settings::mark_webui_wizard_done)
}

// ── 日志与异常退出（数据安全二期票 01，D10 命令契约）──

/// 打开日志文件夹（opener 打开数据目录下 logs/；目录不存在先创建）。
#[tauri::command]
fn open_logs_folder(app: tauri::AppHandle) -> Result<String, String> {
    use tauri_plugin_opener::OpenerExt;
    let result = (|| -> Result<String, String> {
        let data_dir = app
            .path()
            .app_data_dir()
            .map_err(|e| format!("解析数据目录失败: {e}"))?;
        let logs_dir = data_dir.join(applog::LOG_DIR_NAME);
        std::fs::create_dir_all(&logs_dir).map_err(|e| format!("创建日志目录失败: {e}"))?;
        app.opener()
            .open_path(logs_dir.to_string_lossy(), None::<&str>)
            .map_err(|e| format!("打开日志文件夹失败: {e}"))?;
        Ok(logs_dir.to_string_lossy().to_string())
    })();
    // 终局评审 D7：非库命令失败也落日志（不经 with_conn，统一入口覆盖不到）
    if let Err(e) = &result {
        applog::log_error(&format!("打开日志文件夹失败: {e}"));
    }
    result
}

/// 最近错误摘要（[ERROR]/[PANIC] 行，新→旧，限量 [`applog::MAX_RECENT_ERRORS`]）。
#[tauri::command]
fn get_recent_errors() -> Result<Vec<String>, String> {
    let data_dir = current_data_dir()?;
    Ok(applog::recent_errors_from(&data_dir, applog::MAX_RECENT_ERRORS))
}

/// 上次异常退出（启动判定暂存结果；None = 上次正常退出）。
#[tauri::command]
fn get_last_abnormal_exit() -> Result<Option<applog::AbnormalExit>, String> {
    Ok(applog::last_abnormal_exit())
}

/// 前端未捕获异常转发落盘（window.onerror / unhandledrejection → main.ts 调用）。
/// 入参截断与单行化在 format_line 内统一做，超长堆栈不撑破行结构。
#[tauri::command]
fn log_frontend_error(message: String) -> Result<(), String> {
    applog::log_error(&format!("前端未捕获异常: {message}"));
    Ok(())
}

/// 当前数据目录（日志与备份配置命令共用；全局未初始化时回退解析一次）。
fn current_data_dir() -> Result<std::path::PathBuf, String> {
    if let Some(dir) = applog::data_dir() {
        return Ok(dir.to_path_buf());
    }
    Err("数据目录未初始化（应用底座未就绪）".into())
}

// ── 备份配置（数据安全二期票 02，D1/D10 配置部分）──

/// 读备份配置（数据目录 backup-config.json；缺失/损坏按默认值，不报错）。
/// 配置在库外独立文件（D1）：恢复整库不影响它。
#[tauri::command]
fn get_backup_config() -> Result<backup_config::BackupConfig, String> {
    let data_dir = current_data_dir()?;
    Ok(backup_config::load(&data_dir))
}

/// 保存备份配置（只收用户可改的三项：开关/目录/保留份数；账目字段 Rust 侧
/// 维护，前端不可覆写）。保留份数 1–365 之外拒绝（前端已先行校验，这里兜底
/// 拦下绕过前端的直调）。成功记一条动作流水（D7）。
#[tauri::command]
fn set_backup_config(
    input: backup_config::BackupConfigInput,
) -> Result<backup_config::BackupConfig, String> {
    let data_dir = current_data_dir()?;
    let saved = backup_config::update_and_save(&data_dir, &input)?;
    applog::log_action(&format!(
        "备份配置已保存：开关{}，保留 {} 份，目录 {}",
        if saved.enabled { "开" } else { "关" },
        saved.keep_count,
        saved.backup_dir.as_deref().unwrap_or("未设"),
    ));
    Ok(saved)
}

/// rfd 系统文件夹选择框选备份目录（取消返回 None）。async command：对话框
/// 不能占主线程（与 backup_to 同理，评审 R1-2）。路径字符串回给前端保存。
#[tauri::command]
async fn pick_backup_dir() -> Result<Option<String>, String> {
    let picked = rfd::AsyncFileDialog::new()
        .set_title("选择自动备份目录")
        .pick_folder()
        .await
        .map(|handle| handle.path().to_string_lossy().to_string());
    Ok(picked)
}

/// 备份状态（D10：上次结果/时间 + last_backup_date/last_data_write_date；
/// 本票从配置文件投影，票 03 接真数据后同一出口）。
#[tauri::command]
fn get_backup_status() -> Result<backup_config::BackupStatus, String> {
    let data_dir = current_data_dir()?;
    Ok(backup_config::status_of(&backup_config::load(&data_dir)))
}

// ── 自动备份引擎（数据安全二期票 03，D2/D3/D4；引擎本体在 auto_backup.rs）──

/// 业务写入成功后的触发点（D2 ①）：票 06 起，第一件事是数据版本 +1 并双端
/// 广播（库内 data_meta 计数器 +1 → SSE hub 发布 → 桌面 `data-version` 事件）。
/// 这是统一咽喉：桌面 21 个写命令直接调本函数，HTTP 写命令经 webui_after_write
/// 钩子同路——绝不在各命令里散写 bump。随后照旧：同步记 `last_data_write_date`
/// （配置锁内快速落账，账目不丢），再后台线程判定 + 备份——网络盘等慢速目标
/// 目录不阻塞命令返回与 UI（spec D3 锁外拷贝）。备份失败静默（记账+日志），
/// 绝不把错误报给业务命令：写入照常成功返回（票面铁律），版本自增失败同样
/// 只落日志（少一次广播，客户端下次 hello/重连自动对齐）。
fn trigger_after_write(app: &tauri::AppHandle) {
    bump_data_version_and_broadcast(app);
    let data_dir = match current_data_dir() {
        Ok(dir) => dir,
        Err(e) => {
            applog::log_error(&format!("自动备份触发跳过（数据目录未就绪）: {e}"));
            return;
        }
    };
    if let Err(e) = auto_backup::record_data_write(&data_dir, chrono::Local::now().date_naive()) {
        // 记账失败不回滚业务写入；日志留痕排查配置文件 IO 问题
        applog::log_error(&format!("记录业务写入日期失败: {e}"));
    }
    spawn_auto_backup(app);
}

/// 数据版本自增 + 双端广播（webui-checkin 票 06）：库内计数 +1（与写命令同一
/// 把库锁、锁外串行），网页端经 SSE hub 收推、桌面端收 `data-version` 事件
/// （负载 {epoch, version} 与 SSE 帧同形）。Tauri 状态未就绪（极端早退路径）
/// 或自增失败只落日志，绝不影响业务写入结果。
fn bump_data_version_and_broadcast(app: &tauri::AppHandle) {
    let Some(db) = app.try_state::<DbState>() else {
        return;
    };
    let Some(deps) = app.try_state::<Arc<webui_server::SharedDeps>>() else {
        return;
    };
    match webui_server::bump_and_publish(&db.0, &deps.versions) {
        Ok(frame) => {
            use tauri::Emitter;
            let _ = app.emit("data-version", frame);
        }
        Err(e) => applog::log_error(&format!("数据版本自增失败（跳过广播）: {e}")),
    }
}

/// 孤儿照片巡检（票 07，规格 E/F）：库无引用文件移入 `photos/.orphan-<时间戳>/`
/// （移动非删除），`.tmp-` 残留直接删。启动时与恢复完成后各跑一轮；有动作才落
/// 流水（干净不刷屏）。
fn run_orphan_scan(app: &tauri::AppHandle) {
    let Some(state) = app.try_state::<DbState>() else {
        return;
    };
    let Ok(photos_root) = current_data_dir().map(|d| d.join(photo::PHOTOS_DIR_NAME)) else {
        return;
    };
    let outcome = run_with_conn(&state.0, |conn| {
        Ok::<photo::OrphanScanOutcome, String>(photo::scan_orphans(
            conn,
            &photos_root,
            &photo::stamp_now(),
        ))
    });
    match outcome {
        Ok(o) if !o.moved.is_empty() || !o.removed_tmp.is_empty() || !o.errors.is_empty() => {
            applog::log_action(&format!(
                "孤儿照片巡检：隔离 {} 个无引用文件，清理 {} 个 .tmp- 残留{}",
                o.moved.len(),
                o.removed_tmp.len(),
                if o.errors.is_empty() {
                    String::new()
                } else {
                    format!("，失败 {} 个：{}", o.errors.len(), o.errors.join("；"))
                }
            ));
        }
        Ok(_) => {}
        // run_with_conn 已落过失败日志；这里只补巡检语境
        Err(_) => {}
    }
}

/// 数据包恢复中断续跑（webui-checkin 票 10）：库替换后进程被杀 → 暂存目录
/// 留存，启动时把「当前库引用而 photos/ 缺失」且暂存区有的照片继续落位，然后
/// 清暂存目录。必须在孤儿巡检之前跑（巡检会把换库前的旧照片按孤儿隔离，顺序
/// 不能反）。有动作才落流水。
fn run_pkg_restore_resume(app: &tauri::AppHandle) {
    let Some(state) = app.try_state::<DbState>() else {
        return;
    };
    let Ok(data_dir) = current_data_dir() else {
        return;
    };
    let photos_root = data_dir.join(photo::PHOTOS_DIR_NAME);
    let outcome = run_with_conn(&state.0, |conn| {
        Ok::<restore_pkg::PkgResumeOutcome, String>(restore_pkg::resume_pending_pkg_restores(
            &data_dir,
            &photos_root,
            conn,
        ))
    });
    match outcome {
        Ok(o) if !o.resumed.is_empty() || !o.discarded.is_empty() || !o.errors.is_empty() => {
            if !o.errors.is_empty() {
                applog::log_error(&format!(
                    "数据包恢复续跑未完成（下次启动重试）: {}",
                    o.errors.join("；")
                ));
            }
            if !o.resumed.is_empty() || !o.discarded.is_empty() {
                applog::log_action(&format!(
                    "数据包恢复续跑：完成 {} 个暂存目录的照片落位，丢弃 {} 个半截暂存目录{}",
                    o.resumed.len(),
                    o.discarded.len(),
                    if o.landed.is_empty() {
                        String::new()
                    } else {
                        format!("，落位照片 {} 张", o.landed.len())
                    }
                ));
            }
        }
        Ok(_) => {}
        // run_with_conn 已落过失败日志；这里只补续跑语境
        Err(_) => {}
    }
}

/// 启动巡检放后台线程：照片多时扫盘不挡启动。先续跑数据包恢复（票 10）再
/// 巡检——顺序见 [`run_pkg_restore_resume`]。
fn spawn_orphan_scan(app: &tauri::AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        run_pkg_restore_resume(&app);
        run_orphan_scan(&app);
    });
}

/// 启动触发点（D2 ②）与写入触发点共用的后台执行入口：判定（含时钟回拨钳制）
/// → 需要才真备份，谓词不满足时线程空转一次即退。
fn spawn_auto_backup(app: &tauri::AppHandle) {
    let Ok(data_dir) = current_data_dir() else {
        return;
    };
    let app = app.clone();
    std::thread::spawn(move || {
        let Some(state) = app.try_state::<DbState>() else {
            eprintln!("[auto_backup] 库状态未就绪，本次触发跳过");
            return;
        };
        auto_backup::run_triggered(state.inner(), &data_dir);
    });
}

// ── 应用内恢复（数据安全二期票 04，D5/D6/D10；协议本体在 restore.rs）──

/// rfd 系统文件选择框选备份文件（取消返回 None）。async command：对话框不能占
/// 主线程（与 pick_backup_dir 同理，评审 R1-2）。`default_dir` = 备份目录已设置
/// 时作为对话框初始位置（目录真实存在才生效）。票 10：同时认数据包（.zip）与
/// 旧裸库（.db），恢复侧按扩展名分流。
#[tauri::command]
async fn pick_restore_file(default_dir: Option<String>) -> Result<Option<String>, String> {
    let mut dialog = rfd::AsyncFileDialog::new()
        .set_title("选择要恢复的备份文件")
        .add_filter("备份数据包 / 数据库", &["zip", "db"]);
    if let Some(dir) = default_dir {
        let p = std::path::PathBuf::from(&dir);
        if p.is_dir() {
            dialog = dialog.set_directory(&p);
        }
    }
    let picked = dialog
        .pick_file()
        .await
        .map(|handle| handle.path().to_string_lossy().to_string());
    Ok(picked)
}

/// 恢复预览（D10 契约）：校验链通过返回摘要（备份日期/窝数/记录数/备份内目录
/// 设置值），否则返回可展示的中文拒绝原因。只读校验，当前库零改动（spec D5
/// 步骤 1–2 在 staging 临时库上进行）。
#[tauri::command]
fn restore_preview(
    state: tauri::State<'_, DbState>,
    path: String,
) -> Result<restore::RestoreSummary, String> {
    let data_dir = current_data_dir()?;
    let result = restore::run_preview(Path::new(&path), &state.1, &data_dir, &restore::stamp_now());
    // 终局评审 D7：预览被拒（选错文件/损坏/未来版本）也留痕，排查"为什么不让恢复"
    if let Err(e) = &result {
        applog::log_error(&format!("恢复预览未通过（来源 {path}）: {e}"));
    }
    result
}

/// 恢复执行（D10 契约）：重走完整校验（TOCTOU 安全——preview 与 apply 之间
/// 源文件可能被换）→ pre-restore 快照 → 锁内原子替换 → 重开连接 → 广播前端
/// 刷新。返回 `done`（界面当场刷新）或 `done_needs_restart`（新库文件已就位
/// 但重开连接失败，前端提示「请重启应用」）。恢复执行写动作流水（D7，含来源）。
#[tauri::command]
fn restore_apply(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    path: String,
) -> Result<restore::ApplyOutcome, String> {
    let data_dir = current_data_dir()?;
    let db_path = state.1.clone();
    let outcome = restore::run_apply(
        Path::new(&path),
        &db_path,
        &data_dir,
        &restore::stamp_now(),
        || {
            // 更新安装禁写复查必须在锁内（评审 R1 TOCTOU 纪律，同 with_conn /
            // daily_tick 段 3）：整库校验要数百毫秒，顶层检查与拿锁之间的窗口里
            // 用户可确认更新安装（on_before_exit 持锁置禁写位），复查放在拿锁
            // 之后，置位后拿到的锁直接放弃执行——守卫随 ? 丢弃即放锁，当前库
            // 零改动。
            let guard = state.0.lock().map_err(|e| format!("库锁不可用: {e}"))?;
            updater::ensure_writable()?;
            Ok(guard)
        },
        |p| crate::db::open_and_migrate(p).map_err(|e| e.to_string()),
    );
    match &outcome {
        Ok(outcome) => {
            applog::log_action(&format!(
                "恢复执行完成（来源 {path}）：整库已替换{}",
                match outcome {
                    restore::ApplyOutcome::Done => "，界面即将刷新",
                    restore::ApplyOutcome::DoneNeedsRestart => "，重开连接失败待重启生效",
                }
            ));
            // 广播各页刷新（spec D5 步骤 4；done_needs_restart 也广播——库文件
            // 确实换了，前端此时应展示重启提示而不是旧数据）
            // Further Notes 落账：Q13 授权的「提示重启」降级未触发——db-restored
            // 事件刷新链路已工作（组件测试覆盖前端侧），降级预案仅在未来链路
            // 失灵时启用。db-restored 语义保持=无条件刷新（票 06 不改它）。
            use tauri::Emitter;
            let _ = app.emit("db-restored", ());
            // 票 06：恢复后版本可能回退（换入旧备份的计数器更小）——用
            // restore_bump_and_publish 把新库计数抬到「本实例已广播最大值+1」
            // 并广播 version 帧，网页端对账必判落后 → 无条件刷新。失败只落
            // 日志：桌面端已有 db-restored 兜底，网页端等下次写/重连对齐。
            if let Some(deps) = app.try_state::<Arc<webui_server::SharedDeps>>() {
                match webui_server::restore_bump_and_publish(&state.0, &deps.versions) {
                    Ok(frame) => {
                        let _ = app.emit("data-version", frame);
                    }
                    Err(e) => {
                        applog::log_error(&format!("恢复后数据版本对齐失败: {e}"));
                    }
                }
            }
            // 托盘 tooltip 是库内超期摘要的投影，恢复后立即重算
            reminder::refresh_tray_tooltip(&app);
            // 孤儿照片巡检（票 07）：换入的库/照片集合可能不一致（旧裸库恢复
            // 后照片场景按孤儿治理，规格 F），恢复完成立即扫一轮
            run_orphan_scan(&app);
        }
        Err(e) => {
            applog::log_error(&format!("恢复执行失败（来源 {path}）: {e}"));
        }
    }
    outcome
}

// ── 系统级数据出口（票 09）：打开数据文件夹 / 安全备份 / 导出 ──

/// 打开数据文件夹（opener 打开 app data 目录；目录不存在先创建）。
#[tauri::command]
fn reveal_data_folder(app: tauri::AppHandle) -> Result<String, String> {
    use tauri_plugin_opener::OpenerExt;
    let result = (|| -> Result<String, String> {
        let dir = app
            .path()
            .app_data_dir()
            .map_err(|e| format!("解析数据目录失败: {e}"))?;
        std::fs::create_dir_all(&dir).map_err(|e| format!("创建数据目录失败: {e}"))?;
        app.opener()
            .open_path(dir.to_string_lossy(), None::<&str>)
            .map_err(|e| format!("打开数据文件夹失败: {e}"))?;
        Ok(dir.to_string_lossy().to_string())
    })();
    // 终局评审 D7：非库命令失败也落日志（不经 with_conn，统一入口覆盖不到）
    if let Err(e) = &result {
        applog::log_error(&format!("打开数据文件夹失败: {e}"));
    }
    result
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

/// 安全备份（票 09 起产出数据包）：rfd 选目标 → 锁内拷库到临时（挡并发写，
/// 毫秒级本地拷贝，journal_mode=DELETE 拷贝即完整）→ 锁外打包（读 photos/ +
/// 流式写 zip；照片文件 immutable 文件名、清单以锁内库快照为准，一致性论证见
/// backup_pkg.rs 模块头）→ 清临时。缺照片文件降级（跳过+错误流水），包照常
/// 产出。用户取消返回 None。成败都落流水（终局评审 D7：备份成功/失败；本命令
/// 不经 with_conn）。
#[tauri::command]
async fn backup_to(state: tauri::State<'_, DbState>) -> Result<Option<String>, String> {
    let db_path = state.1.clone();
    let stamp = auto_backup::stamp_now();
    // 评审 R1：手动命名 manual- 中缀——parse_backup_file_name 不识别 → 落进
    // 自动备份目录也绝不进保留轮换池（手动产物由用户自管）。
    let default_name = auto_backup::manual_backup_file_name(&stamp);
    let Some(target) = pick_save_path(&default_name, "蚂蚁饲养记录数据包（zip）", &["zip"]).await
    else {
        return Ok(None);
    };
    let data_dir = current_data_dir()?;
    // 锁内拷库（对话框阶段不持锁，不卡其他命令）；staging 用手动链独立前缀
    //（manual-backup-staging-，评审 R1：与自动备份同秒不撞名）
    let temp_db = {
        let guard = match state.0.lock() {
            Ok(guard) => guard,
            Err(e) => {
                applog::log_error(&format!("安全备份失败（库锁不可用）: {e}"));
                return Err(e.to_string());
            }
        };
        let copied = auto_backup::copy_db_to_temp_manual(&db_path, &data_dir, &stamp);
        drop(guard); // 照片打包在锁外，不阻塞业务命令
        copied?
    };
    let photos_root = data_dir.join(photo::PHOTOS_DIR_NAME);
    let mut on_skip = |msg: &str| {
        applog::log_error(&format!("安全备份照片跳过（包内不含该张）: {msg}"))
    };
    let result = backup_pkg::build_package(
        &temp_db,
        &photos_root,
        &applog::now_local(),
        &target,
        &mut on_skip,
    );
    let _ = std::fs::remove_file(&temp_db);
    match result {
        Ok(outcome) => {
            applog::log_action(&format!(
                "安全备份成功: {}（照片 {} 张，包 {} 字节）",
                target.display(),
                outcome.photo_count,
                std::fs::metadata(&target).map(|m| m.len()).unwrap_or(0),
            ));
            Ok(Some(target.to_string_lossy().to_string()))
        }
        Err(e) => {
            applog::log_error(&format!("安全备份失败: {e}"));
            Err(e)
        }
    }
}

/// 导出归档（评审附录规则 11：定位为归档带走；恢复走应用内恢复通道 restore.rs）：csv=记录流水一行一条
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
    let app = tauri::Builder::default()
        // 单实例（数据安全二期票 01 D9）：必须注册在最前（插件文档要求）。
        // 第二实例启动时本回调在【主实例】进程里执行：聚焦已有主窗口，第二实例
        // 自身随即退出——运行标记、备份账目、库单写者都以单实例为前提。
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_notification::init())
        // 开机自启（票 09）：状态权威在 settings，启动/改设置时对齐插件
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_opener::init())
        // 应用内更新（票 02）：检查全在 Rust 侧（每日定时 + check_update_now），
        // 前端不直接调 updater JS API，capabilities 不加 updater:default
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            // 库文件放系统应用数据目录（Windows: %APPDATA%\<identifier>\），非项目目录。
            let data_dir = app.path().app_data_dir()?;
            // 日志底座先行（数据安全二期票 01）：panic 钩子 + 过期日志清理，再跑
            // 启动序列——读上次运行标记 → 判定异常退出并暂存（get_last_abnormal_exit
            // 查询）→ 清旧标记 → 写本次标记 → 记「应用启动」流水。
            applog::init(&data_dir);
            applog::startup_sequence(&data_dir);
            let db_path = data_dir.join(db::DB_FILE_NAME);
            let conn = db::open_and_migrate(&db_path).map_err(|e| e.to_string())?;
            let autostart_on = settings::get_settings(&conn)
                .map(|s| s.autostart_enabled)
                .unwrap_or(true);
            let db_state = DbState(Arc::new(Mutex::new(conn)), db_path);
            // 网页端服务（票 04）：共享依赖（与桌面 IPC 同一把库锁的柄）+ 运行时
            // 槽位先就位；配置 enabled 即拉起。启动失败只落日志不挡启动（不弹窗
            // 不崩溃；设置页保存时会对齐并回显错误，用户可当场看到原因）。
            // 票 05 接线两个钩子：
            // - after_write：写命令成功后的桌面同款收尾（按命令性质刷托盘 tooltip
            //   + trigger_after_write 自动备份记账/后台判定）；HTTP 写命令不再另
            //   起一条对齐路径，桌面/网页共用同一套触发点。
            // - frontend_assets：打包后的 frontendDist 产物嵌在二进制里，经
            //   asset resolver 取出托管（浏览器打开 / 即完整前端）；tauri dev 期
            //   嵌入资源为空，静态路由 404（开发期浏览器走 devUrl）。
            let webui_after_write: webui_server::AfterWriteHook = {
                let handle = app.handle().clone();
                Arc::new(move |with_tray| {
                    if with_tray {
                        reminder::refresh_tray_tooltip(&handle);
                    }
                    trigger_after_write(&handle);
                })
            };
            let webui_assets: webui_server::AssetLookup = {
                let handle = app.handle().clone();
                Arc::new(move |path| {
                    handle
                        .asset_resolver()
                        .get(path.to_string())
                        .map(|asset| asset.bytes().to_vec())
                })
            };
            let webui_deps = Arc::new(webui_server::SharedDeps::with_hooks(
                db_state.conn_handle(),
                data_dir.clone(),
                webui_after_write,
                webui_assets,
            ));
            app.manage(webui_deps);
            app.manage(webui_server::WebUiRuntime::new());
            app.manage(db_state);
            // 托盘常驻 + 提醒调度（启动即查一次，此后每 30 分钟；评审附录规则 1）。
            reminder::setup_tray(app)?;
            reminder::spawn_scheduler(app.handle().clone());
            // 升级残留兜底（票 05）：上次"想升没升成"的启动判定——读 pending 标记
            // 对比当前运行版本（与票 04 快照同源），三态结果暂存（get_update_state
            // 可查），标记判定后即清。失败只影响提示，绝不挡启动。
            let current_version = app.package_info().version.to_string();
            match updater::startup_judgment(&data_dir, &current_version) {
                updater::UpdateState::LastInstallIncomplete { version } => {
                    // 终局评审 D7：安装结果三态判定落流水（未完成是错误态）
                    applog::log_error(&format!(
                        "上次升级未完成（目标 v{version}）：可在设置页重试或手动下载安装包"
                    ));
                }
                updater::UpdateState::LastInstallSucceeded { version } => {
                    applog::log_action(&format!("升级成功，当前已是 v{version}"));
                }
                updater::UpdateState::Idle => {}
            }
            // 每日更新检查（票 02）：托盘常驻进程内跑，窗口关闭也查；
            // 内部按"上次检查日"决定真查还是跳过（重启不重查）。
            updater::spawn_daily_checker(app.handle().clone());
            // 自动备份启动触发点（数据安全二期票 03 D2 ②）：接在票 01 startup
            // 序列后，后台判定（含时钟回拨钳制）→ 有未备份的新数据才补跑
            // （跨日空启动不备；备份失败留给本触发点下次启动补）。
            spawn_auto_backup(app.handle());
            // 孤儿照片巡检（票 07）：启动后扫 photos/，库无引用文件移入隔离区。
            // 后台线程跑，扫盘不挡启动。
            spawn_orphan_scan(app.handle());
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
            // 开机自启默认开（settings 预置行=1 即「首次启动写入」）；成败都落
            // 日志（终局评审 D7：窗口化应用 stderr 丢失等于不可见），失败不拦启动。
            match apply_autostart(app.handle(), autostart_on) {
                Ok(()) => applog::log_action(&format!(
                    "开机自启已同步：{}",
                    if autostart_on { "开" } else { "关" }
                )),
                Err(e) => applog::log_error(&format!("启动时同步开机自启失败: {e}")),
            }
            // 网页端服务自启（票 04）：上次会话启用过即拉起（绑定失败时 start()
            // 已落日志——端口占用等细节在流水里，设置页重新保存可回显）。disabled
            // 静默（每次启动都记「未启用」是刷屏）。
            let webui_cfg = webui_config::load(&data_dir);
            if webui_cfg.enabled {
                let app_handle = app.handle().clone();
                let outcome =
                    tauri::async_runtime::block_on(sync_webui_server(&app_handle, &webui_cfg));
                if let Err(e) = outcome {
                    applog::log_error(&format!("启动时拉起网页端服务失败: {e}"));
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            health_check,
            log_care,
            list_foods,
            list_logs,
            colony_month_records,
            update_log,
            delete_log,
            save_checkin,
            list_checkins,
            update_checkin,
            delete_checkin,
            get_checkin_digest,
            pick_photo_files,
            attach_photos,
            get_photo_abs_dir,
            list_orphan_photos,
            clean_orphan_photos,
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
            check_update_now,
            confirm_and_install,
            get_update_state,
            get_app_version,
            open_releases_page,
            pushover_status,
            reveal_data_folder,
            open_logs_folder,
            get_recent_errors,
            get_last_abnormal_exit,
            log_frontend_error,
            get_backup_config,
            set_backup_config,
            pick_backup_dir,
            get_backup_status,
            backup_to,
            restore_preview,
            restore_apply,
            pick_restore_file,
            export_data,
            get_stats,
            earliest_log_date,
            list_network_segments,
            get_webui_config,
            save_webui_config,
            regenerate_token,
            get_access_url,
            get_webui_wizard_done,
            mark_webui_wizard_done,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    // 主事件循环（数据安全二期票 01 D8）：RunEvent::Exit 汇拢全部优雅退出路径——
    // 托盘「退出」（app.exit）与 OS 关机/注销的常规销毁序列都经这里收尾，
    // 清运行标记 + 记「应用正常退出」。更新安装路径不经事件循环（插件在
    // on_before_exit 里自清，见 updater.rs），双保险互不重叠。
    app.run(|_app, event| {
        if let tauri::RunEvent::Exit = event {
            applog::graceful_exit();
        }
    });
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
