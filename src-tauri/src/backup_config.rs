//! 备份配置：库外元数据读写（数据安全二期票 02，spec D1/D10 配置部分）。
//!
//! D1 钉死决策：备份管理的全部元数据——开关、备份目录、保留份数、
//! `last_backup_date`（最后成功备份日期）、`last_data_write_date`（最后业务写入
//! 日期）、上次备份结果与时间——存数据目录独立文件 [`BACKUP_CONFIG_FILE`]，
//! **不进 SQLite settings 表**（恢复整库不能回滚备份设置，也不触发「状态写库
//! 自备份」回路）。
//!
//! 纯函数核心与 IO 解耦（沿 settings.rs/applog.rs 惯例）：
//! - [`validate_keep_count`]：保留份数 1–365 之外拒绝（票面验收：非法值拒绝并提示）；
//! - [`normalize_dir`]：目录两端空白剪掉、空白串按未设处理；
//! - [`sanitize`]：读后收敛——手改配置文件的脏值（出界保留份数/空白目录）兜底；
//! - [`merge_input`]：用户三项输入合并进现有配置，账目字段只由 Rust 侧维护；
//! - [`status_of`]：get_backup_status 投影（D10 契约）。
//!
//! IO 薄层（TempDir 直测）：
//! - [`load`]：文件缺失/损坏按默认值，绝不 panic、不挡启动（spec D1）；
//! - [`save`]：pretty JSON 落盘，数据目录缺失先建；
//! - [`update_and_save`]：读 → 合并校验 → 写（set_backup_config 的编排）。
//!
//! Tauri command 薄封装在 lib.rs（目录选择对话框 rfd 也在那边）。

use std::path::Path;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// 备份配置文件名（数据目录下，与库文件平级）。
pub const BACKUP_CONFIG_FILE: &str = "backup-config.json";

/// 保留份数下界（票面验收：1–365）。
pub const KEEP_COUNT_MIN: i64 = 1;

/// 保留份数上界（票面验收：1–365）。
pub const KEEP_COUNT_MAX: i64 = 365;

/// 保留份数默认值（spec D1）。
pub const DEFAULT_KEEP_COUNT: u32 = 30;

/// 备份配置模型（字段名即 JSON 键，serde 默认 snake_case，前端契约对齐）。
/// 账目三字段（last_*）本票只定义与展示，票 03 接真数据。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BackupConfig {
    /// 自动备份开关（默认开）。
    pub enabled: bool,
    /// 备份目录；None = 未设（未设时自动备份不生效）。
    pub backup_dir: Option<String>,
    /// 保留份数 1–365（默认 30）。
    pub keep_count: u32,
    /// 最后成功备份日期（`YYYY-MM-DD`；票 03 接真数据）。
    pub last_backup_date: Option<String>,
    /// 最后业务写入日期（`YYYY-MM-DD`；票 03 接真数据）。
    pub last_data_write_date: Option<String>,
    /// 上次备份结果与时间；None = 尚未备份过（票 03 接真数据）。
    pub last_result: Option<LastBackupOutcome>,
}

impl Default for BackupConfig {
    fn default() -> Self {
        BackupConfig {
            enabled: true,
            backup_dir: None,
            keep_count: DEFAULT_KEEP_COUNT,
            last_backup_date: None,
            last_data_write_date: None,
            last_result: None,
        }
    }
}

/// 上次备份结果（配置内嵌 + get_backup_status 复用）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastBackupOutcome {
    /// true=成功 / false=失败。
    pub ok: bool,
    /// 结果发生时间（`YYYY-MM-DD HH:MM:SS`，定宽字符串）。
    pub at: String,
    /// 失败原因（成功为 None）。
    pub reason: Option<String>,
}

/// set_backup_config 入参：只含用户可改的三项（开关/目录/保留份数）。
/// 账目字段不在此列——前端不可覆写，票 03 的备份账目不被设置保存冲掉。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct BackupConfigInput {
    pub enabled: bool,
    pub backup_dir: Option<String>,
    pub keep_count: i64,
}

/// get_backup_status 返回体（D10：{上次结果, 上次时间, last_backup_date,
/// last_data_write_date}；上次时间在内嵌的 last_result.at 里）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BackupStatus {
    pub last_result: Option<LastBackupOutcome>,
    pub last_backup_date: Option<String>,
    pub last_data_write_date: Option<String>,
}

// ── 纯核心 ───────────────────────────────────────────────────────────────

/// 保留份数校验：1–365 之外拒绝并给出含范围的提示（票面验收 4）。
pub fn validate_keep_count(keep: i64) -> Result<u32, String> {
    if !(KEEP_COUNT_MIN..=KEEP_COUNT_MAX).contains(&keep) {
        return Err(format!(
            "保留份数须在 {KEEP_COUNT_MIN}–{KEEP_COUNT_MAX} 之间（当前：{keep}）"
        ));
    }
    Ok(keep as u32)
}

/// 目录规整：两端空白剪掉；空白串按未设（None）处理。
pub fn normalize_dir(dir: Option<String>) -> Option<String> {
    match dir {
        Some(d) => {
            let trimmed = d.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        None => None,
    }
}

/// 读后收敛：保留份数钳到 1–365、目录规整。手改配置文件的脏值兜底，
/// 不挡启动（spec D1「损坏按默认值」的补充——能解析但值脏的部分地收敛）。
pub fn sanitize(config: BackupConfig) -> BackupConfig {
    BackupConfig {
        keep_count: config.keep_count.clamp(KEEP_COUNT_MIN as u32, KEEP_COUNT_MAX as u32),
        backup_dir: normalize_dir(config.backup_dir),
        ..config
    }
}

/// 合并用户输入到现有配置：保留份数拒绝式校验（非法整体失败），
/// 账目字段原样保留（D1：设置保存不得冲掉备份账目）。
pub fn merge_input(
    current: &BackupConfig,
    input: &BackupConfigInput,
) -> Result<BackupConfig, String> {
    let keep_count = validate_keep_count(input.keep_count)?;
    Ok(BackupConfig {
        enabled: input.enabled,
        backup_dir: normalize_dir(input.backup_dir.clone()),
        keep_count,
        last_backup_date: current.last_backup_date.clone(),
        last_data_write_date: current.last_data_write_date.clone(),
        last_result: current.last_result.clone(),
    })
}

/// 状态投影：get_backup_status 的返回（D10 契约）。
pub fn status_of(config: &BackupConfig) -> BackupStatus {
    BackupStatus {
        last_result: config.last_result.clone(),
        last_backup_date: config.last_backup_date.clone(),
        last_data_write_date: config.last_data_write_date.clone(),
    }
}

// ── IO 薄层（TempDir 直测）───────────────────────────────────────────────

/// 配置文件单写者锁（评审转记，票 03 起）：后台备份记账线程与 UI 设置保存会
/// 并发读改写同一文件——所有「load → 改 → save」全程持锁，交错不丢更新。
/// 无锁的读（load）不持锁：最坏撞上写入中途读到半截 JSON，按默认值处理且不
/// 落盘（会落盘的路径都在锁内），不会造成持久性丢失。
static CONFIG_FILE_LOCK: Mutex<()> = Mutex::new(());

/// 拿配置写锁（锁毒化按原值续用：配置是提示性数据，不值得 panic）。
fn lock_config_file() -> std::sync::MutexGuard<'static, ()> {
    CONFIG_FILE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// 读配置：文件缺失/损坏/半损坏按默认值（serde(default) 补齐缺字段），
/// 读后收敛脏值。绝不 panic、不挡启动（spec D1）。
pub fn load(data_dir: &Path) -> BackupConfig {
    let path = data_dir.join(BACKUP_CONFIG_FILE);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return BackupConfig::default(),
        Err(e) => {
            eprintln!("[backup_config] 读配置失败（按默认值继续）: {e}");
            return BackupConfig::default();
        }
    };
    match serde_json::from_str::<BackupConfig>(&text) {
        Ok(config) => sanitize(config),
        Err(e) => {
            eprintln!("[backup_config] 配置文件损坏（按默认值重建）: {e}");
            BackupConfig::default()
        }
    }
}

/// 写配置（pretty JSON，人可直接看改）。数据目录缺失先建。
pub fn save(data_dir: &Path, config: &BackupConfig) -> Result<(), String> {
    std::fs::create_dir_all(data_dir).map_err(|e| format!("创建数据目录失败: {e}"))?;
    let text =
        serde_json::to_string_pretty(config).map_err(|e| format!("序列化备份配置失败: {e}"))?;
    std::fs::write(data_dir.join(BACKUP_CONFIG_FILE), text)
        .map_err(|e| format!("写入备份配置失败: {e}"))
}

/// 读 → 合并校验 → 写（set_backup_config 编排）。校验失败时不落盘，原配置原样。
/// 读改写全程持单写者锁（与备份记账并发安全）。
pub fn update_and_save(data_dir: &Path, input: &BackupConfigInput) -> Result<BackupConfig, String> {
    let _guard = lock_config_file();
    let current = load(data_dir);
    let next = merge_input(&current, input)?;
    save(data_dir, &next)?;
    Ok(next)
}

/// 账目更新（票 03 备份引擎记账入口）：锁内 load → 按 `f` 改账目字段 → save。
/// 与 [`update_and_save`] 共用同一把单写者锁，交错不丢更新。
pub fn update_accounting(
    data_dir: &Path,
    f: impl FnOnce(&mut BackupConfig),
) -> Result<BackupConfig, String> {
    let _guard = lock_config_file();
    let mut current = load(data_dir);
    f(&mut current);
    save(data_dir, &current)?;
    Ok(current)
}

// ── 测试：只测外部行为（spec「Testing Decisions」）────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn temp_data_dir() -> TempDir {
        TempDir::new().expect("创建临时目录失败")
    }

    fn input(enabled: bool, dir: Option<&str>, keep: i64) -> BackupConfigInput {
        BackupConfigInput {
            enabled,
            backup_dir: dir.map(str::to_string),
            keep_count: keep,
        }
    }

    fn outcome(ok: bool, at: &str, reason: Option<&str>) -> Option<LastBackupOutcome> {
        Some(LastBackupOutcome {
            ok,
            at: at.to_string(),
            reason: reason.map(str::to_string),
        })
    }

    // ── 默认值（D1：开关=开、目录=未设、保留=30）──

    #[test]
    fn defaults_are_enabled_no_dir_keep_30() {
        let c = BackupConfig::default();
        assert!(c.enabled, "自动备份默认开");
        assert_eq!(c.backup_dir, None, "目录默认未设");
        assert_eq!(c.keep_count, DEFAULT_KEEP_COUNT);
        assert_eq!(c.keep_count, 30);
        assert_eq!(c.last_backup_date, None, "账目字段默认空（票 03 接真数据）");
        assert_eq!(c.last_data_write_date, None);
        assert_eq!(c.last_result, None, "尚未备份");
    }

    // ── 保留份数校验（验收 4：1–365，非法拒绝）──

    #[test]
    fn keep_count_out_of_range_rejected() {
        assert!(validate_keep_count(0).is_err());
        assert!(validate_keep_count(-1).is_err());
        assert!(validate_keep_count(366).is_err());
        assert!(validate_keep_count(100_000).is_err());
        let err = validate_keep_count(366).unwrap_err();
        assert!(err.contains("1") && err.contains("365"), "提示语含合法范围，实际：{err}");
    }

    #[test]
    fn keep_count_boundaries_accepted() {
        assert_eq!(validate_keep_count(1).unwrap(), 1);
        assert_eq!(validate_keep_count(365).unwrap(), 365);
        assert_eq!(validate_keep_count(30).unwrap(), 30);
    }

    // ── 目录规整 ──

    #[test]
    fn empty_backup_dir_normalizes_to_none() {
        assert_eq!(normalize_dir(Some("  ".into())), None, "空白串按未设处理");
        assert_eq!(normalize_dir(None), None);
        assert_eq!(
            normalize_dir(Some("  D:\\ant-bk ".into())),
            Some("D:\\ant-bk".to_string()),
            "两端空白剪掉"
        );
    }

    // ── 读后收敛（手改配置文件的脏值兜底，不挡启动）──

    #[test]
    fn sanitize_clamps_keep_count_and_dir() {
        let c = sanitize(BackupConfig {
            keep_count: 0,
            backup_dir: Some("  ".into()),
            ..Default::default()
        });
        assert_eq!(c.keep_count, 1, "0 钳到下界");
        assert_eq!(c.backup_dir, None);

        let c = sanitize(BackupConfig {
            keep_count: 99_999,
            ..Default::default()
        });
        assert_eq!(c.keep_count, 365, "越界钳到上界");
    }

    // ── 文件读写（TempDir 直测）──

    #[test]
    fn save_then_load_roundtrips_all_fields() {
        let dir = temp_data_dir();
        let want = BackupConfig {
            enabled: false,
            backup_dir: Some("E:/ant-backups".into()),
            keep_count: 7,
            last_backup_date: Some("2026-09-18".into()),
            last_data_write_date: Some("2026-09-17".into()),
            last_result: outcome(false, "2026-09-18 08:00:00", Some("网盘掉线")),
        };
        save(dir.path(), &want).unwrap();
        assert!(dir.path().join(BACKUP_CONFIG_FILE).exists(), "文件落在数据目录");
        assert_eq!(load(dir.path()), want, "读写往返保真");
    }

    #[test]
    fn load_missing_file_returns_defaults() {
        let dir = temp_data_dir();
        assert_eq!(load(dir.path()), BackupConfig::default(), "缺失按默认值");
    }

    #[test]
    fn load_corrupted_file_returns_defaults_without_panicking() {
        // spec D1：配置文件损坏 → 按默认值重建，不 panic、不挡启动
        let dir = temp_data_dir();
        std::fs::write(dir.path().join(BACKUP_CONFIG_FILE), "不是 JSON{{{").unwrap();
        assert_eq!(load(dir.path()), BackupConfig::default());
    }

    #[test]
    fn load_partial_json_fills_missing_fields_with_defaults() {
        // serde(default)：老版本/手改缺字段的文件不炸，缺的补默认
        let dir = temp_data_dir();
        std::fs::write(dir.path().join(BACKUP_CONFIG_FILE), r#"{"enabled":false}"#).unwrap();
        let c = load(dir.path());
        assert!(!c.enabled, "有值的字段照读");
        assert_eq!(c.keep_count, DEFAULT_KEEP_COUNT, "缺的字段补默认");
        assert_eq!(c.backup_dir, None);
    }

    #[test]
    fn load_sanitizes_dirty_values() {
        // 手改出界保留份数：读取即收敛，不用等保存
        let dir = temp_data_dir();
        std::fs::write(
            dir.path().join(BACKUP_CONFIG_FILE),
            r#"{"enabled":true,"keep_count":99999}"#,
        )
        .unwrap();
        assert_eq!(load(dir.path()).keep_count, 365);
    }

    #[test]
    fn save_creates_missing_data_dir() {
        let dir = temp_data_dir();
        let data_dir = dir.path().join("nested").join("data");
        save(&data_dir, &BackupConfig::default()).unwrap();
        assert!(data_dir.join(BACKUP_CONFIG_FILE).exists());
    }

    // ── D1 核心：库与配置零联动（验收 1 单测：动库不联动配置）──

    #[test]
    fn db_operations_do_not_touch_backup_config() {
        let dir = temp_data_dir();
        update_and_save(dir.path(), &input(false, Some("D:/ant-backups"), 7)).unwrap();

        // 动库：建库 + 改设置（模拟恢复整库后库内设置被回滚的场景）
        let db_path = dir.path().join(crate::db::DB_FILE_NAME);
        let conn = crate::db::open_and_migrate(&db_path).expect("建库失败");
        crate::settings::set_settings(
            &conn,
            &crate::settings::AppSettings {
                wake_remind_days_ahead: 3,
                ..Default::default()
            },
        )
        .unwrap();
        drop(conn);

        let c = load(dir.path());
        assert!(!c.enabled, "配置不随库动");
        assert_eq!(c.backup_dir.as_deref(), Some("D:/ant-backups"));
        assert_eq!(c.keep_count, 7);
    }

    // ── 合并用户输入（set_backup_config 核心）──

    #[test]
    fn merge_input_updates_only_three_user_fields() {
        let current = BackupConfig {
            last_backup_date: Some("2026-09-18".into()),
            last_data_write_date: Some("2026-09-18".into()),
            last_result: outcome(true, "2026-09-18 08:00:00", None),
            ..Default::default()
        };
        let next = merge_input(&current, &input(true, Some("D:/bk"), 90)).unwrap();
        assert!(next.enabled);
        assert_eq!(next.backup_dir.as_deref(), Some("D:/bk"));
        assert_eq!(next.keep_count, 90);
        // 账目字段原样保留（设置保存不得冲掉备份账目，D1）
        assert_eq!(next.last_backup_date, current.last_backup_date);
        assert_eq!(next.last_data_write_date, current.last_data_write_date);
        assert_eq!(next.last_result, current.last_result);
    }

    #[test]
    fn merge_input_rejects_invalid_keep_count() {
        let current = BackupConfig::default();
        assert!(merge_input(&current, &input(true, None, 0)).is_err());
        assert!(merge_input(&current, &input(true, None, 366)).is_err());
    }

    #[test]
    fn update_and_save_rejected_input_leaves_file_untouched() {
        let dir = temp_data_dir();
        update_and_save(dir.path(), &input(false, Some("D:/keep"), 7)).unwrap();
        let err = update_and_save(dir.path(), &input(true, None, 999)).unwrap_err();
        assert!(err.contains("365"), "实际：{err}");
        let c = load(dir.path());
        assert!(!c.enabled && c.keep_count == 7, "拒绝后原配置原样");
        assert_eq!(c.backup_dir.as_deref(), Some("D:/keep"));
    }

    // ── 配置并发保护（评审转记：双线程交错 update 断言不丢更新）──

    #[test]
    fn concurrent_updates_do_not_lose_writes() {
        // 后台备份记账线程（写 last_data_write_date / last_result）与 UI 设置保存
        // 会并发读改写同一文件——load→改→save 全程持单写者锁，交错不丢更新。
        let dir = temp_data_dir();
        let data_dir = dir.path();
        std::thread::scope(|s| {
            // 线程甲：连写 28 次业务写入日（1 日 → 28 日递增）
            s.spawn(|| {
                for i in 0..28 {
                    update_accounting(data_dir, |c| {
                        c.last_data_write_date = Some(format!("2026-09-{:02}", i + 1))
                    })
                    .unwrap();
                }
            });
            // 线程乙：连写 28 次备份结果
            s.spawn(|| {
                for i in 0..28 {
                    update_accounting(data_dir, |c| {
                        c.last_result = outcome(true, &format!("2026-09-18 00:{:02}:00", i), None)
                    })
                    .unwrap();
                }
            });
        });
        let latest = load(data_dir);
        assert_eq!(
            latest.last_data_write_date.as_deref(),
            Some("2026-09-28"),
            "线程甲的最后一次更新必须幸存"
        );
        let r = latest.last_result.expect("线程乙的最后一次更新必须幸存");
        assert_eq!(r.at, "2026-09-18 00:27:00");
    }

    // ── 状态投影（get_backup_status，D10 契约）──

    #[test]
    fn status_of_projects_accounting_fields() {
        let c = BackupConfig {
            last_backup_date: Some("2026-09-18".into()),
            last_data_write_date: Some("2026-09-17".into()),
            last_result: outcome(false, "2026-09-18 08:00:00", Some("目录写不进去")),
            ..Default::default()
        };
        let s = status_of(&c);
        assert_eq!(s.last_backup_date.as_deref(), Some("2026-09-18"));
        assert_eq!(s.last_data_write_date.as_deref(), Some("2026-09-17"));
        let r = s.last_result.expect("有结果");
        assert!(!r.ok);
        assert_eq!(r.at, "2026-09-18 08:00:00");
        assert_eq!(r.reason.as_deref(), Some("目录写不进去"));

        // 尚未备份：全空
        let s = status_of(&BackupConfig::default());
        assert_eq!(s.last_result, None);
        assert_eq!(s.last_backup_date, None);
        assert_eq!(s.last_data_write_date, None);
    }

    // ── 前端契约：字段 snake_case（serde 默认，同 abnormal_exit 契约测试先例）──

    #[test]
    fn config_serializes_snake_case_for_frontend() {
        let c = BackupConfig {
            enabled: false,
            backup_dir: Some("C:/bk".into()),
            keep_count: 7,
            last_backup_date: Some("2026-09-18".into()),
            last_data_write_date: None,
            last_result: outcome(false, "2026-09-18 08:00:00", Some("网盘掉线")),
        };
        let json = serde_json::to_string(&c).unwrap();
        for key in [
            "enabled",
            "backup_dir",
            "keep_count",
            "last_backup_date",
            "last_data_write_date",
            "last_result",
        ] {
            assert!(json.contains(&format!("\"{key}\"")), "缺字段 {key}，实际：{json}");
        }
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["last_result"]["ok"], false);
        assert_eq!(v["last_result"]["at"], "2026-09-18 08:00:00");
        assert_eq!(v["last_result"]["reason"], "网盘掉线");

        let s = serde_json::to_string(&status_of(&c)).unwrap();
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        for key in ["last_result", "last_backup_date", "last_data_write_date"] {
            assert!(v.get(key).is_some(), "status 缺 {key}");
        }
    }
}
