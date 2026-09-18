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

/// IPC 通路健康检查：返回 schema 版本（迁移正常时应为 1）。
#[tauri::command]
fn health_check(state: tauri::State<DbState>) -> Result<SchemaInfo, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    let schema_version = db::schema_version_of(&conn).map_err(|e| e.to_string())?;
    Ok(SchemaInfo { schema_version })
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
        .invoke_handler(tauri::generate_handler![health_check])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
