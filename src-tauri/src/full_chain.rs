//! 全链冒烟（数据安全二期票 05 收口，spec D11 + Testing Decisions）：
//! 「造数据 → 自动备份产生备份文件 → 修改数据 → 从备份恢复」端到端一遍，断言
//! ① 数据回到备份点；② backup-config.json 不随恢复变（D1）；③ 日志文件里含
//! 备份与恢复的动作流水行（D7）。
//!
//! 直调公开函数、不启 Tauri（票面铁律）。三处与生产形态的对应与偏差，说明如下：
//! - 写入/配置走业务函数（care::log_care / backup_config::update_and_save），
//!   引擎与恢复走 `auto_backup::run_triggered` / `restore::run_apply`；
//! - take_lock 只拿库锁：生产命令层在锁内还有「更新安装禁写复查」（票 04
//!   apply_inlock_gate_rejection_* 已用注入本地位的测试覆盖），此处刻意不触碰
//!   updater 全局禁写位——并行测试会临时置位它，冒烟测试不得依赖全局；
//! - 恢复成功的动作流水行由命令层（lib.rs restore_apply）写入，command 体
//!   无法脱离 Tauri 直调——本测试在 run_apply 成功后按 lib.rs 同款格式补写该行，
//!   使全链日志可断言（行格式以 lib.rs 为准，别处不得依赖本测试）。
//!
//! 日志落点：`applog::log_action`/`log_error` 走进程全局数据目录（OnceLock），
//! 整个测试套件只有本测试调用 `applog::init` 把它指进临时数据目录，备份引擎
//! 内部写的动作流水才会落进可断言的位置。

use std::io::Read;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use rusqlite::params;

use crate::care::CareLogInput;
use crate::restore::ApplyOutcome;

/// 轮询等待条件成立（防重入位被并行测试瞬时占住时不硬等）。
fn wait_for_file(dir: &std::path::Path, timeout: Duration) -> usize {
    let deadline = Instant::now() + timeout;
    loop {
        let produced = std::fs::read_dir(dir)
            .map(|rd| rd.filter_map(|e| e.ok()).count())
            .unwrap_or(0);
        if produced > 0 || Instant::now() >= deadline {
            return produced;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn count_logs(conn: &rusqlite::Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM care_log", [], |r| r.get(0))
        .expect("查记录数失败")
}

#[test]
fn full_chain_backup_then_restore_roundtrip() {
    let dir = tempfile::TempDir::new().expect("创建临时目录失败");
    let data_dir = dir.path().join("data");
    let backup_dir = dir.path().join("backup-target");
    std::fs::create_dir_all(&data_dir).expect("建数据目录失败");

    // 日志底座：applog 全局数据目录指进本测试临时目录（OnceLock 进程级一次；
    // 套件内仅本测试调用 init——若未来别的测试也调 init，两条流水断言会失效）
    crate::applog::init(&data_dir);

    // ── ① 造数据：1 窝 + 2 条记录（业务写入函数，非裸 SQL 拼记录）──────────
    let db_path = data_dir.join(crate::db::DB_FILE_NAME);
    let colony_id = {
        let conn = crate::db::open_and_migrate(&db_path).expect("建库失败");
        conn.execute(
            "INSERT INTO colony (name, start_date) VALUES ('大头一号', '2026-01-20')",
            [],
        )
        .unwrap();
        let colony_id = conn.last_insert_rowid();
        let feeding: i64 = conn
            .query_row("SELECT id FROM care_action WHERE name = '喂食'", [], |r| r.get(0))
            .expect("预置喂食操作缺失");
        for day in [10, 15] {
            crate::care::log_care(
                &conn,
                &CareLogInput {
                    colony_id,
                    action_id: feeding,
                    happened_at: format!("2026-09-{day} 08:00:00"),
                    note: None,
                    food_ids: vec![],
                },
                "2026-09-18 12:00:00",
            )
            .expect("造记录失败");
        }
        colony_id
    };

    // 运行态：DbState（Arc<Mutex<Connection>> + 库路径），同应用 setup 的形状
    let state = crate::DbState(
        std::sync::Arc::new(Mutex::new(
            crate::db::open_and_migrate(&db_path).expect("重开库失败"),
        )),
        db_path.clone(),
    );

    // ── ② 自动备份：配置（同 set_backup_config 编排）→ 写入记账 → 引擎直调 ──
    crate::backup_config::update_and_save(
        &data_dir,
        &crate::backup_config::BackupConfigInput {
            enabled: true,
            backup_dir: Some(backup_dir.to_string_lossy().to_string()),
            keep_count: 30,
        },
    )
    .expect("保存备份配置失败");

    let today = chrono::Local::now().date_naive();
    let today_iso = today.format("%Y-%m-%d").to_string();
    // 写入触发点的记账侧（同 lib.rs trigger_after_write 的先行动作）
    crate::auto_backup::record_data_write(&data_dir, today).expect("写入记账失败");

    // 引擎直调。防重入位是进程全局：并行的 auto_backup 测试可能瞬时占住，
    // run_triggered 被跳过时静默无产物——短窗等待后重试，总预算 10s
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut produced = 0;
    while produced == 0 {
        assert!(
            Instant::now() < deadline,
            "重试 10s 仍无备份产物（防重入位争用或引擎失败）"
        );
        crate::auto_backup::run_triggered(&state, &data_dir);
        produced = wait_for_file(&backup_dir, Duration::from_millis(200));
    }
    assert_eq!(produced, 1, "自动备份应产出恰好一份备份文件");

    // 账目追平：last_backup_date = 今日、结果成功
    let config_after_backup = crate::backup_config::load(&data_dir);
    assert_eq!(
        config_after_backup.last_backup_date.as_deref(),
        Some(today_iso.as_str()),
        "备份成功后 last_backup_date 追平今日"
    );
    assert!(
        config_after_backup.last_result.as_ref().expect("有备份结果").ok,
        "备份结果记账为成功"
    );

    // ── ③ 修改数据：备份点之后再加一条带可识别备注的记录 ──────────────────
    {
        let conn = state.0.lock().expect("库锁");
        let feeding: i64 = conn
            .query_row("SELECT id FROM care_action WHERE name = '喂食'", [], |r| r.get(0))
            .expect("预置喂食操作缺失");
        crate::care::log_care(
            &conn,
            &CareLogInput {
                colony_id,
                action_id: feeding,
                happened_at: "2026-09-19 09:00:00".into(),
                note: Some("备份点之后的新记录".into()),
                food_ids: vec![],
            },
            "2026-09-19 09:00:00",
        )
        .expect("加记录失败");
        assert_eq!(count_logs(&conn), 3, "修改后当前库 3 条");
    }
    crate::auto_backup::record_data_write(&data_dir, today).expect("写入记账失败");

    // 备份产物在当前库分叉后仍是备份点内容（2 条）。票 09：产物是数据包 zip，
    // 解出库条目验证内容；恢复通路本票仍吃裸库（数据包恢复随票 10），故恢复
    // 段以解出的库为源——数据完整走过一遍包。
    let source: PathBuf = {
        let mut paths: Vec<PathBuf> = std::fs::read_dir(&backup_dir)
            .expect("备份目录应存在")
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .collect();
        assert_eq!(paths.len(), 1, "实际：{paths:?}");
        let path = paths.pop().unwrap();
        assert!(
            path.extension().map(|e| e == "zip").unwrap_or(false),
            "备份产物应为数据包 zip，实际：{path:?}"
        );
        let f = std::fs::File::open(&path).expect("打开数据包失败");
        let mut z = zip::ZipArchive::new(f).expect("数据包可解");
        assert!(z.by_name(crate::backup_pkg::MANIFEST_ENTRY).is_ok(), "包内有根级清单");
        let mut db_bytes = Vec::new();
        z.by_name(crate::backup_pkg::DB_ENTRY)
            .expect("包内有库条目")
            .read_to_end(&mut db_bytes)
            .expect("读库条目失败");
        let extracted = dir.path().join("extracted-from-package.db");
        std::fs::write(&extracted, &db_bytes).expect("落解出的库失败");
        let backup_conn = crate::db::open_and_migrate(&extracted).expect("包内库可重开");
        assert_eq!(count_logs(&backup_conn), 2, "备份文件停在备份点");
        drop(backup_conn);
        extracted
    };

    // 恢复前基线：配置文件字节（D1：恢复不得动它）
    let config_path = data_dir.join(crate::backup_config::BACKUP_CONFIG_FILE);
    let config_before_restore = std::fs::read(&config_path).expect("读配置文件失败");

    // ── ④ 从备份恢复（restore 函数直调；锁内禁写复查属命令层，见模块头说明）──
    let outcome = crate::restore::run_apply(
        &source,
        &db_path,
        &data_dir,
        &crate::restore::stamp_now(),
        || state.0.lock().map_err(|e| format!("库锁不可用: {e}")),
        |p| crate::db::open_and_migrate(p).map_err(|e| e.to_string()),
    )
    .expect("恢复执行失败");
    assert_eq!(outcome, ApplyOutcome::Done);

    // 命令层动作流水（lib.rs restore_apply 成功分支同款格式；command 体无法
    // 脱离 Tauri 直调，此处补写使全链日志可断言）
    crate::applog::log_action(&format!(
        "恢复执行完成（来源 {}）：整库已替换，界面即将刷新",
        source.display()
    ));

    // ── 断言 1：数据回到备份点（经运行态连接查——它已被恢复协议重开到新库）──
    {
        let conn = state.0.lock().expect("库锁");
        let name: String = conn
            .query_row(
                "SELECT name FROM colony WHERE id = ?1",
                params![colony_id],
                |r| r.get(0),
            )
            .expect("查窝失败");
        assert_eq!(name, "大头一号", "整库替换后窝仍在");
        assert_eq!(count_logs(&conn), 2, "记录数回到备份点");
        let gone: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM care_log WHERE note = '备份点之后的新记录'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(gone, 0, "备份点之后新增的记录被整库替换掉");
    }

    // 反悔通道（D5 步骤 3）：恢复前的当前库留了 pre-restore 快照；staging 清干净
    let names_in_data: Vec<String> = std::fs::read_dir(&data_dir)
        .expect("数据目录应存在")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    assert_eq!(
        names_in_data
            .iter()
            .filter(|n| n.starts_with(crate::restore::SNAPSHOT_PREFIX))
            .count(),
        1,
        "恢复前快照恰一份，实际：{names_in_data:?}"
    );
    assert!(
        names_in_data
            .iter()
            .all(|n| !n.starts_with(crate::restore::STAGING_PREFIX)),
        "staging 已清理，实际：{names_in_data:?}"
    );

    // ── 断言 2：backup-config.json 不随恢复动（D1，字节级不变）─────────────
    assert_eq!(
        std::fs::read(&config_path).expect("恢复后读配置失败"),
        config_before_restore,
        "备份配置文件不随恢复回滚（D1）"
    );

    // ── 断言 3：日志文件含备份与恢复的动作流水行（D7）─────────────────────
    let log_text = std::fs::read_to_string(
        data_dir
            .join(crate::applog::LOG_DIR_NAME)
            .join(crate::applog::log_file_name(&today_iso)),
    )
    .expect("当日日志文件应存在");
    assert!(
        log_text.contains("[ACTION] 自动备份成功"),
        "缺备份动作流水，实际：\n{log_text}"
    );
    assert!(
        log_text.contains("[ACTION] 恢复执行完成（来源"),
        "缺恢复动作流水，实际：\n{log_text}"
    );
}
