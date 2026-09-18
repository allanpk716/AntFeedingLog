mod colony;
mod db;

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
pub struct DbState(Mutex<Connection>);

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

// ── 窝（票 02）──

#[tauri::command]
fn list_colonies(state: tauri::State<DbState>) -> Result<Vec<colony::Colony>, String> {
    with_conn(state, |conn| colony::list_colonies(conn, &colony::today_iso()))
}

#[tauri::command]
fn create_colony(
    state: tauri::State<DbState>,
    input: colony::ColonyInput,
) -> Result<colony::Colony, String> {
    with_conn(state, |conn| {
        colony::create_colony(conn, &input, &colony::today_iso())
    })
}

#[tauri::command]
fn update_colony(
    state: tauri::State<DbState>,
    id: i64,
    input: colony::ColonyInput,
) -> Result<colony::Colony, String> {
    with_conn(state, |conn| {
        colony::update_colony(conn, id, &input, &colony::today_iso())
    })
}

#[tauri::command]
fn archive_colony(state: tauri::State<DbState>, id: i64) -> Result<colony::Colony, String> {
    with_conn(state, |conn| colony::archive_colony(conn, id, &colony::today_iso()))
}

#[tauri::command]
fn delete_colony(state: tauri::State<DbState>, id: i64) -> Result<(), String> {
    with_conn(state, |conn| colony::delete_colony(conn, id))
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
fn deactivate_location(state: tauri::State<DbState>, id: i64) -> Result<(), String> {
    with_conn(state, |conn| colony::deactivate_location(conn, id))
}

#[tauri::command]
fn erase_location(state: tauri::State<DbState>, id: i64) -> Result<(), String> {
    with_conn(state, |conn| colony::erase_location(conn, id))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            // 库文件放系统应用数据目录（Windows: %APPDATA%\<identifier>\），非项目目录。
            let data_dir = app.path().app_data_dir()?;
            let conn = db::open_and_migrate(&data_dir.join(db::DB_FILE_NAME))
                .map_err(|e| e.to_string())?;
            app.manage(DbState(Mutex::new(conn)));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            health_check,
            list_colonies,
            create_colony,
            update_colony,
            archive_colony,
            delete_colony,
            list_locations,
            save_location,
            deactivate_location,
            erase_location,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
