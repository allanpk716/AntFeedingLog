//! 自动备份引擎（数据安全二期票 03）：触发 · 执行 · 保留淘汰。
//!
//! spec D2/D3/D4 的完整落地，纯函数核心与 IO/Tauri 解耦（沿 system.rs/applog.rs 惯例）：
//! - 触发（D2）：统一谓词 [`needs_backup`]——`last_data_write_date > last_backup_date`，
//!   叠加「开关开 ∧ 目录已设」即 [`evaluate_trigger`]；时钟回拨钳制 [`clamp_dates`]
//!   （`last_backup_date` 晚于今日清空、`last_data_write_date` 钳到今日），单测注入
//!   未来日期验证备份不停摆；
//! - 执行（D3）：两段式 [`copy_and_publish`]——库锁只覆盖「库文件 → 数据目录临时
//!   文件」的本地毫秒级段，锁外把临时文件拷到备份目录（网络盘/慢速盘不占库锁、
//!   不阻塞业务命令返回）；临时文件成功失败都清理，失败待下个触发点重试；
//! - 保留淘汰（D4）：[`stale_backup_names`] 只匹配自家命名
//!   `ant-feeding-log-backup-<8位日期>-<6位时间>.db` 且时间戳可解析的文件，按
//!   **文件名内时间戳**（非 mtime）排序超 N 删最旧；手动备份/无关 .db/文档绝不触碰；
//! - 触发点线程体 [`run_triggered`]：钳制持久化 → 谓词判定 → 两段式执行 →
//!   记账/动作流水/保留淘汰。备份失败静默（记账 + 日志），绝不影响业务命令。
//!
//! 触发点两处（lib.rs 接线）：①业务写入命令成功后（记录/窝/字典/冬眠的增删改；
//! 元数据写入不算——先 [`record_data_write`] 记账再后台判定）；②应用启动序列后。
//! 同一时刻至多一个备份在跑（[`begin_backup`] 防重入：快速连写产生的并发触发，
//! 后到者跳过——在跑的那个完成记账后谓词自然收敛，失败则下个触发点重试）。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::NaiveDateTime;

use crate::backup_config;

/// 自动备份文件名前缀（D4 保留淘汰只对「前缀 + 8位日期-6位时间 + 后缀」动手）。
pub const BACKUP_NAME_PREFIX: &str = "ant-feeding-log-backup-";

/// 自动备份文件名后缀。
pub const BACKUP_NAME_SUFFIX: &str = ".db";

/// 数据目录里两段式中转的临时文件名前缀（`auto-backup-staging-<时间戳>.db`）。
pub const STAGING_PREFIX: &str = "auto-backup-staging-";

// ── 命名与解析（D3/D4 的地基）────────────────────────────────────────────

/// 备份文件名时间戳：`YYYYMMDD-HHMMSS`（含时分秒——同日补跑不互相覆盖，D3）。
pub fn stamp_format(now: NaiveDateTime) -> String {
    now.format("%Y%m%d-%H%M%S").to_string()
}

/// 备份文件名：`ant-feeding-log-backup-YYYYMMDD-HHMMSS.db`。
pub fn backup_file_name(stamp: &str) -> String {
    format!("{BACKUP_NAME_PREFIX}{stamp}{BACKUP_NAME_SUFFIX}")
}

/// 真实时钟的时间戳（编排层用；纯逻辑测试走 [`stamp_format`] 注入）。
pub fn stamp_now() -> String {
    stamp_format(chrono::Local::now().naive_local())
}

/// 反向解析自家备份命名；不匹配/位数不对/非数字/时间戳不合法（如 13 月、25 点）
/// 一律 None——保留淘汰只对解析成功的文件动手，同目录其他文件绝不触碰（D4）。
pub fn parse_backup_file_name(name: &str) -> Option<NaiveDateTime> {
    let stem = name
        .strip_prefix(BACKUP_NAME_PREFIX)?
        .strip_suffix(BACKUP_NAME_SUFFIX)?;
    let (date_part, time_part) = stem.split_once('-')?;
    if date_part.len() != 8 || time_part.len() != 6 {
        return None;
    }
    if !date_part.bytes().all(|b| b.is_ascii_digit()) || !time_part.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let date = chrono::NaiveDate::parse_from_str(date_part, "%Y%m%d").ok()?;
    let time = chrono::NaiveTime::parse_from_str(time_part, "%H%M%S").ok()?;
    Some(date.and_time(time))
}

// ── 触发谓词与钳制（D2 纯核心）───────────────────────────────────────────

/// 时钟回拨钳制后的账目日期对（判定入口的输出，也用于持久化收敛）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClampedDates {
    pub last_backup_date: Option<String>,
    pub last_data_write_date: Option<String>,
}

fn date_to_string(d: chrono::NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

/// 时钟回拨钳制（spec D2）：
/// - `last_backup_date` 晚于今日 → 视为无效清 `None`（下次触发重新备份，不停摆）；
/// - `last_data_write_date` 晚于今日 → 钳到今日（数据不能凭空变"未来"）；
/// - 解析失败的日期按 `None` 处理（宁多备不漏备）。
pub fn clamp_dates(
    last_backup_date: Option<&str>,
    last_data_write_date: Option<&str>,
    today: chrono::NaiveDate,
) -> ClampedDates {
    let parse = |s: Option<&str>| s.and_then(|v| chrono::NaiveDate::parse_from_str(v, "%Y-%m-%d").ok());
    ClampedDates {
        last_backup_date: parse(last_backup_date)
            .filter(|d| *d <= today)
            .map(date_to_string),
        last_data_write_date: parse(last_data_write_date)
            .map(|d| d.min(today))
            .map(date_to_string),
    }
}

/// 统一谓词（D2）：需要备份 ⇔ `last_data_write_date > last_backup_date`
/// （存在比最后成功备份更新的业务数据）。`None` = 从未：
/// 有写入没备份过 → 备；没写入 → 不备（跨日空启动不备份的根据）；
/// 两者都有 → ISO 日期定宽字符串比较即时间比较。
pub fn needs_backup(last_data_write_date: Option<&str>, last_backup_date: Option<&str>) -> bool {
    match (last_data_write_date, last_backup_date) {
        (Some(w), Some(b)) => w > b,
        (Some(_), None) => true,
        (None, _) => false,
    }
}

/// 触发判定结果（D2：开关 ∧ 目录 ∧ 谓词）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TriggerDecision {
    /// 开关关：不产生任何备份动作。
    Disabled,
    /// 目录未设：不产生任何备份动作。
    NoDir,
    /// 谓词不成立：无比最后成功备份更新的业务数据。
    NotNeeded,
    /// 需要备份，携带备份目录。
    Go { backup_dir: String },
}

/// 触发判定（读取/判定入口，含时钟回拨钳制与目录规整）。
pub fn evaluate_trigger(config: &backup_config::BackupConfig, today: chrono::NaiveDate) -> TriggerDecision {
    if !config.enabled {
        return TriggerDecision::Disabled;
    }
    let Some(backup_dir) = backup_config::normalize_dir(config.backup_dir.clone()) else {
        return TriggerDecision::NoDir;
    };
    let clamped = clamp_dates(
        config.last_backup_date.as_deref(),
        config.last_data_write_date.as_deref(),
        today,
    );
    if !needs_backup(
        clamped.last_data_write_date.as_deref(),
        clamped.last_backup_date.as_deref(),
    ) {
        return TriggerDecision::NotNeeded;
    }
    TriggerDecision::Go { backup_dir }
}

/// 业务写入记账（D2 触发点①的先行动作）：`last_data_write_date = 今日`，
/// 配置锁内读改写（与 UI 设置保存/备份记账互不丢更新）。失败由调用方记日志，
/// 绝不回滚已成功的业务写入。
pub fn record_data_write(data_dir: &Path, today: chrono::NaiveDate) -> Result<(), String> {
    backup_config::update_accounting(data_dir, |c| {
        c.last_data_write_date = Some(date_to_string(today));
    })
    .map(|_| ())
}

// ── 保留淘汰（D4）────────────────────────────────────────────────────────

/// 保留淘汰判定（D4）：`names` 里可解析为自家命名的文件按**文件名内时间戳**
/// 新→旧排序，保留最新 `keep` 份，其余（要删的最旧者）按旧→新返回——先删最旧，
/// 中途出错也已删掉该删的；解析不了的（手动备份、无关 .db、文档）绝不返回。
pub fn stale_backup_names(names: &[&str], keep: usize) -> Vec<String> {
    let mut parsed: Vec<(NaiveDateTime, &str)> = names
        .iter()
        .filter_map(|n| parse_backup_file_name(n).map(|ts| (ts, *n)))
        .collect();
    // 新→旧；同秒（理论外，手动撞名）按名字倒序保持稳定
    parsed.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(a.1)));
    let mut stale: Vec<String> = parsed
        .into_iter()
        .skip(keep)
        .map(|(_, n)| n.to_string())
        .collect();
    stale.reverse();
    stale
}

/// 保留淘汰落地（D4）：扫备份目录，超出 `keep` 份删最旧（只删自家命名可解析
/// 的文件）。返回删除数。
pub fn enforce_retention(backup_dir: &Path, keep: usize) -> Result<usize, String> {
    let entries = std::fs::read_dir(backup_dir).map_err(|e| format!("读取备份目录失败: {e}"))?;
    let names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut removed = 0;
    for name in stale_backup_names(&refs, keep) {
        match std::fs::remove_file(backup_dir.join(&name)) {
            Ok(()) => removed += 1,
            Err(e) => eprintln!("[auto_backup] 删除超限备份 {name} 失败（忽略）: {e}"),
        }
    }
    Ok(removed)
}

// ── 执行协议（D3，IO 薄层）───────────────────────────────────────────────

/// 阶段一（调用方持库锁时调用）：库文件 → 数据目录临时文件（本地毫秒级；
/// journal_mode=DELETE 拷贝即完整，与手动备份 system::backup_db_file 同一依据）。
/// 拷贝失败清掉半截临时文件。
pub fn copy_db_to_temp(db_path: &Path, data_dir: &Path, stamp: &str) -> Result<PathBuf, String> {
    let temp = data_dir.join(format!("{STAGING_PREFIX}{stamp}.db"));
    match crate::system::backup_db_file(db_path, &temp) {
        Ok(()) => Ok(temp),
        Err(e) => {
            let _ = std::fs::remove_file(&temp);
            Err(e)
        }
    }
}

/// 阶段二（锁外调用）：临时文件 → 备份目录正式名。父目录缺失先建
/// （与手动备份同一行为）。
pub fn publish_temp(temp_path: &Path, target: &Path) -> Result<(), String> {
    crate::system::backup_db_file(temp_path, target).map(|_| ())
}

/// 清理临时文件（成功/失败路径都调；NotFound 视为已清理）。
fn cleanup_temp(temp_path: &Path) {
    if let Err(e) = std::fs::remove_file(temp_path) {
        if e.kind() != std::io::ErrorKind::NotFound {
            eprintln!("[auto_backup] 清理临时文件失败（忽略，下次备份覆盖）: {e}");
        }
    }
}

/// 两段式备份执行（D3）：`take_lock` 注入库锁获取（生产传 `DbState` 的
/// `Mutex<Connection>::lock`；测试传任意守卫）。锁窗口只覆盖阶段一的本地临时
/// 拷贝；阶段二（临时 → 备份目录，可能落在网络盘/慢速盘）在锁释放后执行，不
/// 阻塞业务命令。临时文件成功失败都清理。返回备份产物完整路径。
pub fn copy_and_publish<G, Guard>(
    db_path: &Path,
    data_dir: &Path,
    backup_dir: &Path,
    stamp: &str,
    take_lock: G,
) -> Result<PathBuf, String>
where
    G: FnOnce() -> Result<Guard, String>,
{
    let _guard = take_lock()?;
    let temp_path = copy_db_to_temp(db_path, data_dir, stamp)?;
    drop(_guard); // 锁窗口到此为止——备份目录拷贝绝不占库锁
    let target = backup_dir.join(backup_file_name(stamp));
    let result = publish_temp(&temp_path, &target);
    cleanup_temp(&temp_path);
    result.map(|_| target)
}

// ── 防重入 ───────────────────────────────────────────────────────────────

/// 备份执行中标志：同一时刻至多一个备份在跑。
static BACKUP_IN_FLIGHT: AtomicBool = AtomicBool::new(false);

/// 占住备份执行位：`Some(守卫)` = 拿到（drop 自动复位）；`None` = 已有备份在跑，
/// 本次触发跳过（在跑的那个完成记账后谓词自然收敛，无需排队）。
pub fn begin_backup() -> Option<InFlightGuard> {
    BACKUP_IN_FLIGHT
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .ok()
        .map(|_| InFlightGuard)
}

/// Drop 兜底复位执行位（执行体 panic/早退都不卡死后续触发）。
#[derive(Debug)]
pub struct InFlightGuard;
impl Drop for InFlightGuard {
    fn drop(&mut self) {
        BACKUP_IN_FLIGHT.store(false, Ordering::SeqCst);
    }
}

// ── 触发点线程体（lib.rs 后台线程调用）───────────────────────────────────

/// 触发 → 执行 → 记账 的完整一遍（D2/D3/D4 编排；写入触发与启动触发共用）：
/// 1. 读配置，时钟回拨钳制并在值有变时持久化（钳制在判定入口做）；
/// 2. 谓词判定（开关 ∧ 目录 ∧ 有新数据）——不满足则无声返回；
/// 3. 两段式执行（库锁只覆盖本地临时拷贝段）；
/// 4. 成功：记 last_backup_date/last_result + 动作流水 + 保留淘汰；
///    失败：静默记 last_result（原因+时间）+ 错误日志，last_backup_date 不动
///    → 谓词仍成立，当日后续写入与下次启动自动补跑（验收 3）。
/// 任何失败都不 panic、不影响调用方（业务命令照常成功返回）。
pub fn run_triggered(state: &crate::DbState, data_dir: &Path) {
    let today = chrono::Local::now().date_naive();
    let config = backup_config::load(data_dir);

    // 时钟回拨钳制：判定入口收敛，值有变才写盘
    let clamped = clamp_dates(
        config.last_backup_date.as_deref(),
        config.last_data_write_date.as_deref(),
        today,
    );
    if clamped.last_backup_date != config.last_backup_date
        || clamped.last_data_write_date != config.last_data_write_date
    {
        if let Err(e) = backup_config::update_accounting(data_dir, |c| {
            c.last_backup_date = clamped.last_backup_date.clone();
            c.last_data_write_date = clamped.last_data_write_date.clone();
        }) {
            eprintln!("[auto_backup] 钳制持久化失败（判定用已钳值继续）: {e}");
        }
    }

    // 谓词判定：开关 ∧ 目录 ∧ last_data_write_date > last_backup_date
    let TriggerDecision::Go { backup_dir } = evaluate_trigger(&config, today) else {
        return;
    };
    let keep = config.keep_count as usize;

    // 防重入：已有备份在跑则本次跳过
    let Some(_in_flight) = begin_backup() else {
        return;
    };

    // 两段式执行（库锁只覆盖本地临时拷贝段，spec D3）
    let stamp = stamp_now();
    let backup_dir_path = PathBuf::from(&backup_dir);
    let result = copy_and_publish(&state.1, data_dir, &backup_dir_path, &stamp, || {
        state
            .0
            .lock()
            .map_err(|e| format!("库锁不可用: {e}"))
    });

    match result {
        Ok(final_path) => {
            // 成功：追平账目 + 动作流水 + 保留淘汰
            let now = crate::applog::now_local();
            if let Err(e) = backup_config::update_accounting(data_dir, |c| {
                c.last_backup_date = Some(date_to_string(today));
                c.last_result = Some(backup_config::LastBackupOutcome {
                    ok: true,
                    at: now.clone(),
                    reason: None,
                });
            }) {
                eprintln!("[auto_backup] 备份成功记账失败: {e}");
            }
            crate::applog::log_action(&format!("自动备份成功: {}", final_path.display()));
            match enforce_retention(&backup_dir_path, keep) {
                Ok(0) => {}
                Ok(n) => crate::applog::log_action(&format!("自动备份保留淘汰：删除 {n} 份超限旧备份")),
                Err(e) => eprintln!("[auto_backup] 保留淘汰失败（忽略）: {e}"),
            }
        }
        Err(reason) => {
            // 失败静默：记账（原因+时间）+ 错误流水；不弹窗、不影响业务命令
            let now = crate::applog::now_local();
            if let Err(e) = backup_config::update_accounting(data_dir, |c| {
                c.last_result = Some(backup_config::LastBackupOutcome {
                    ok: false,
                    at: now.clone(),
                    reason: Some(reason.clone()),
                });
            }) {
                eprintln!("[auto_backup] 备份失败记账失败: {e}");
            }
            crate::applog::log_error(&format!("自动备份失败: {reason}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    use crate::backup_config::{self, BackupConfig};

    // ── 通用脚手架 ───────────────────────────────────────────────────────

    /// ISO 日期串（测试拼 expected 用）。
    fn iso(d: chrono::NaiveDate) -> String {
        d.format("%Y-%m-%d").to_string()
    }

    fn date(y: i32, m: u32, d: u32) -> chrono::NaiveDate {
        chrono::NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    /// 串行化所有触碰全局备份执行位的测试（run_triggered / begin_backup）：
    /// cargo test 默认并行跑线程，一个测试占住执行位会让另一个测试的
    /// run_triggered 被防重入跳过、断言落空。
    static FLOW_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn flow_guard() -> std::sync::MutexGuard<'static, ()> {
        FLOW_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// IO 脚手架：tempdir 里的数据目录（含已迁移、有 1 窝的库文件）+ 备份目录
    /// 路径（刻意不预建——两段式应自动创建，与手动备份同行为）。
    /// 返回 (tempdir, data_dir, backup_dir, db_path)。
    fn io_fixture() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf, std::path::PathBuf)
    {
        let dir = tempfile::TempDir::new().expect("创建临时目录失败");
        let data_dir = dir.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        let db_path = data_dir.join(crate::db::DB_FILE_NAME);
        let conn = crate::db::open_and_migrate(&db_path).expect("建库失败");
        conn.execute(
            "INSERT INTO colony (name, start_date) VALUES ('大头一号', '2026-01-20')",
            [],
        )
        .unwrap();
        drop(conn);
        let backup_dir = dir.path().join("backup-target");
        (dir, data_dir, backup_dir, db_path)
    }

    /// 用库路径拼一个 DbState（重开一份连接，模拟应用运行态）。
    fn db_state(db_path: &std::path::Path) -> crate::DbState {
        crate::DbState(
            std::sync::Mutex::new(crate::db::open_and_migrate(db_path).expect("重开库失败")),
            db_path.to_path_buf(),
        )
    }

    fn list_names(dir: &std::path::Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        names.sort();
        names
    }

    fn no_staging_left(data_dir: &std::path::Path) -> bool {
        list_names(data_dir)
            .iter()
            .all(|n| !n.starts_with(STAGING_PREFIX))
    }

    /// 轮询等待条件成立（线程调度不用固定睡眠硬等）。
    fn wait_until(pred: impl Fn() -> bool, timeout: Duration) -> bool {
        let start = std::time::Instant::now();
        while start.elapsed() < timeout {
            if pred() {
                return true;
            }
            thread::sleep(Duration::from_millis(10));
        }
        pred()
    }

    // ── 命名与解析（D3/D4 的地基）────────────────────────────────────────

    #[test]
    fn backup_file_name_has_second_resolution_stamp() {
        // 验收 5：命名含 YYYYMMDD-HHMMSS；同日两次（补跑场景）不互相覆盖
        let now = date(2026, 9, 18).and_hms_opt(9, 5, 3).unwrap();
        let stamp = stamp_format(now);
        assert_eq!(stamp, "20260918-090503");
        let name = backup_file_name(&stamp);
        assert_eq!(name, "ant-feeding-log-backup-20260918-090503.db");
        let one_second_later = backup_file_name(&stamp_format(now + chrono::Duration::seconds(1)));
        assert_ne!(name, one_second_later, "同秒只差 1 秒的两份备份不得同名");
    }

    #[test]
    fn parse_backup_file_name_accepts_only_own_complete_pattern() {
        let ok = parse_backup_file_name("ant-feeding-log-backup-20260918-090503.db").unwrap();
        assert_eq!(ok, date(2026, 9, 18).and_hms_opt(9, 5, 3).unwrap());
        // 手动备份默认命名（只有日期没时分秒）：绝不认——保留淘汰不得触碰
        assert_eq!(parse_backup_file_name("ant-feeding-log-backup-20260918.db"), None);
        // 位数不对
        assert_eq!(parse_backup_file_name("ant-feeding-log-backup-2026091-090503.db"), None);
        assert_eq!(parse_backup_file_name("ant-feeding-log-backup-20260918-09050.db"), None);
        // 非数字
        assert_eq!(parse_backup_file_name("ant-feeding-log-backup-2026ab918-090503.db"), None);
        // 日历上不合法的时间戳（解析兜底，不能只靠位数）
        assert_eq!(parse_backup_file_name("ant-feeding-log-backup-20261332-090503.db"), None);
        assert_eq!(parse_backup_file_name("ant-feeding-log-backup-20260918-250503.db"), None);
        // 别的前缀 / 别的后缀 / 无后缀
        assert_eq!(parse_backup_file_name("backup-20260918-090503.db"), None);
        assert_eq!(parse_backup_file_name("ant-feeding-log-backup-20260918-090503.db.bak"), None);
        assert_eq!(parse_backup_file_name("ant-feeding-log-backup-20260918-090503"), None);
    }

    // ── 触发谓词与钳制（D2 纯核心）───────────────────────────────────────

    #[test]
    fn needs_backup_truth_table() {
        // 有新数据、没备份过 → 备
        assert!(needs_backup(Some("2026-09-18"), None));
        // 写入晚于备份 → 备（当日首写）
        assert!(needs_backup(Some("2026-09-18"), Some("2026-09-17")));
        // 追平 → 不备（当日后续写入）
        assert!(!needs_backup(Some("2026-09-18"), Some("2026-09-18")));
        // 备份比写入新（异常态，保守不备）
        assert!(!needs_backup(Some("2026-09-17"), Some("2026-09-18")));
        // 没有任何业务写入 → 不备（跨日空启动不备份的根据）
        assert!(!needs_backup(None, Some("2026-09-17")));
        assert!(!needs_backup(None, None));
    }

    #[test]
    fn clamp_dates_rollback_rules() {
        let today = date(2026, 9, 18);
        // 正常日期原样
        let c = clamp_dates(Some("2026-09-17"), Some("2026-09-16"), today);
        assert_eq!(c.last_backup_date.as_deref(), Some("2026-09-17"));
        assert_eq!(c.last_data_write_date.as_deref(), Some("2026-09-16"));
        // 回拨：未来备份日清空；未来写入日钳到今日（验收 4）
        let c = clamp_dates(Some("9999-12-31"), Some("9999-01-01"), today);
        assert_eq!(c.last_backup_date, None, "未来备份日视为无效清空");
        assert_eq!(c.last_data_write_date.as_deref(), Some("2026-09-18"), "未来写入日钳到今日");
        // 解析失败按 None（宁多备不漏备）
        let c = clamp_dates(Some("不是日期"), Some(""), today);
        assert_eq!(c.last_backup_date, None);
        assert_eq!(c.last_data_write_date, None);
        // 恰好等于今日：都不动
        let c = clamp_dates(Some("2026-09-18"), Some("2026-09-18"), today);
        assert_eq!(c.last_backup_date.as_deref(), Some("2026-09-18"));
        assert_eq!(c.last_data_write_date.as_deref(), Some("2026-09-18"));
    }

    #[test]
    fn evaluate_trigger_matrix() {
        let today = date(2026, 9, 18);
        let base = |enabled: bool,
                    dir: Option<&str>,
                    backup: Option<&str>,
                    write: Option<&str>|
         -> BackupConfig {
            BackupConfig {
                enabled,
                backup_dir: dir.map(str::to_string),
                keep_count: 30,
                last_backup_date: backup.map(str::to_string),
                last_data_write_date: write.map(str::to_string),
                last_result: None,
            }
        };

        // 验收 8：开关关 / 目录未设 → 无动作
        assert_eq!(
            evaluate_trigger(&base(false, Some("D:/bk"), None, Some("2026-09-18")), today),
            TriggerDecision::Disabled
        );
        assert_eq!(
            evaluate_trigger(&base(true, None, None, Some("2026-09-18")), today),
            TriggerDecision::NoDir
        );
        assert_eq!(
            evaluate_trigger(&base(true, Some("  "), None, Some("2026-09-18")), today),
            TriggerDecision::NoDir,
            "空白目录等同未设"
        );
        // 验收 2：跨日空启动（写入与备份都停在昨天）→ 谓词不成立不备
        assert_eq!(
            evaluate_trigger(&base(true, Some("D:/bk"), Some("2026-09-17"), Some("2026-09-17")), today),
            TriggerDecision::NotNeeded
        );
        // 当日首写（昨日备、今日写）→ 备
        assert_eq!(
            evaluate_trigger(&base(true, Some("D:/bk"), Some("2026-09-17"), Some("2026-09-18")), today),
            TriggerDecision::Go {
                backup_dir: "D:/bk".into()
            }
        );
        // 当日后续写入（已追平）→ 不备（验收 1）
        assert_eq!(
            evaluate_trigger(&base(true, Some("D:/bk"), Some("2026-09-18"), Some("2026-09-18")), today),
            TriggerDecision::NotNeeded
        );
        // 失败补跑：上次成功备份停在前天、昨天又有写入 → 谓词仍成立（验收 3）
        assert_eq!(
            evaluate_trigger(&base(true, Some("D:/bk"), Some("2026-09-16"), Some("2026-09-17")), today),
            TriggerDecision::Go {
                backup_dir: "D:/bk".into()
            }
        );
        // 时钟回拨：账目全是未来 → 钳制后备份恢复运转（验收 4）
        assert_eq!(
            evaluate_trigger(&base(true, Some("D:/bk"), Some("9999-12-31"), Some("9999-12-31")), today),
            TriggerDecision::Go {
                backup_dir: "D:/bk".into()
            }
        );
    }

    // ── 保留淘汰判定（D4 纯核心）─────────────────────────────────────────

    #[test]
    fn stale_backup_names_keeps_newest_and_never_touches_foreign() {
        // 验收 6：按文件名内时间戳排序（刻意打乱入参顺序），超 N 删最旧
        let names = [
            "ant-feeding-log-backup-20250101-000000.db", // 最旧 → 删
            "ant-feeding-log-backup-20250104-000000.db", // 最新 → 留
            "ant-feeding-log-backup-20250102-000000.db", // 次旧 → 删
            "ant-feeding-log-backup-20250103-000000.db", // 留
            "ant-feeding-log-backup-20250105.db",        // 手动命名（无时分秒）：绝不返回
            "other.db",                                  // 无关 .db：绝不返回
            "notes.txt",                                 // 文档：绝不返回
        ];
        assert_eq!(
            stale_backup_names(&names, 2),
            vec![
                "ant-feeding-log-backup-20250101-000000.db".to_string(),
                "ant-feeding-log-backup-20250102-000000.db".to_string(),
            ]
        );
        // 数量未超：全留
        assert!(stale_backup_names(&names, 10).is_empty());
        // 全是解析不了的文件：即便数量超了也绝不返回
        assert!(stale_backup_names(&["a.db", "b.db", "c.db"], 1).is_empty());
    }

    // ── 执行协议（D3，IO）────────────────────────────────────────────────

    #[test]
    fn copy_and_publish_produces_reopenable_backup_and_cleans_temp() {
        let (_dir, data_dir, backup_dir, db_path) = io_fixture();
        let stamp = "20260918-120000";
        let target = copy_and_publish(&db_path, &data_dir, &backup_dir, stamp, || {
            Ok::<_, String>(())
        })
        .unwrap();
        assert_eq!(target, backup_dir.join(backup_file_name(stamp)));
        assert!(target.exists(), "备份目录缺失时自动创建并落产物");
        // 产物可被应用正式通道重开且数据完整
        let reopened = crate::db::open_and_migrate(&target).unwrap();
        let n: i64 = reopened
            .query_row("SELECT COUNT(*) FROM colony", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
        assert!(no_staging_left(&data_dir), "临时文件已清理");
    }

    #[test]
    fn copy_and_publish_missing_db_fails_without_temp_or_target() {
        let (dir, data_dir, backup_dir, _db_path) = io_fixture();
        let err =
            copy_and_publish(&dir.path().join("no-such.db"), &data_dir, &backup_dir, "20260918-120000", || {
                Ok::<_, String>(())
            })
            .unwrap_err();
        assert!(err.contains("拷贝"), "实际：{err}");
        assert!(no_staging_left(&data_dir), "不留半截临时文件");
        assert!(!backup_dir.exists(), "失败不产生目标产物");
    }

    #[test]
    fn copy_and_publish_unwritable_target_fails_and_cleans_temp() {
        let (dir, data_dir, _backup_dir, db_path) = io_fixture();
        // 目标"目录"被一个文件占住：建目录/拷贝必失败（模拟目录写不进去）
        let blocked = dir.path().join("blocked-target");
        std::fs::write(&blocked, "占位").unwrap();
        let err = copy_and_publish(&db_path, &data_dir, &blocked, "20260918-120000", || {
            Ok::<_, String>(())
        })
        .unwrap_err();
        assert!(!err.is_empty());
        assert!(no_staging_left(&data_dir), "失败也要清临时文件（待下个触发点重试）");
    }

    #[test]
    fn copy_and_publish_lock_error_stops_before_any_copy() {
        let (dir, data_dir, backup_dir, _db_path) = io_fixture();
        let err = copy_and_publish(&dir.path().join("x.db"), &data_dir, &backup_dir, "20260918-120000", || {
            Err::<(), _>("库锁不可用".into())
        })
        .unwrap_err();
        assert!(err.contains("库锁"), "实际：{err}");
        assert!(no_staging_left(&data_dir));
        assert!(!backup_dir.exists());
    }

    #[test]
    fn publish_waits_for_lock_release_slow_target_simulation() {
        // 验收 7：拷贝期间库锁只覆盖本地临时文件拷贝段——慢速目录（网络盘）
        // 模拟：守卫的 Drop 阻塞到主线程放行。若实现把「临时 → 备份目录」
        // 也放进了锁窗口，放行前目标文件就会出现，断言当场抓住。
        // Further Notes 落账：真实网络盘未实测（round 0 实验 #14 仅本地盘）；
        // 本测试以慢速模拟守住锁外语义，常态超时的实际体验（Q8 静默 +
        // 失败记账补跑）留开发期观察。
        let (_dir, data_dir, backup_dir, db_path) = io_fixture();
        let stamp = "20260918-120000";
        let temp = data_dir.join(format!("{STAGING_PREFIX}{stamp}.db"));
        let target = backup_dir.join(backup_file_name(stamp));

        let (release_tx, release_rx) = mpsc::channel::<()>();
        struct BlockUntilReleased(std::sync::mpsc::Receiver<()>);
        impl Drop for BlockUntilReleased {
            fn drop(&mut self) {
                // 模拟锁窗口被拉长：直到主线程放行，守卫才释放
                let _ = self.0.recv();
            }
        }

        let handle = thread::spawn({
            let db_path = db_path.clone();
            let data_dir = data_dir.clone();
            let backup_dir = backup_dir.clone();
            move || {
                copy_and_publish(&db_path, &data_dir, &backup_dir, stamp, || {
                    Ok::<_, String>(BlockUntilReleased(release_rx))
                })
                .expect("备份执行成功");
            }
        });

        // 等阶段一完成（临时文件出现）——此刻工作线程应卡在守卫 Drop
        assert!(
            wait_until(|| temp.exists(), Duration::from_secs(5)),
            "阶段一临时文件未出现"
        );
        // 给「错误实现」留出完成发布的充分窗口（本地小库拷贝是毫秒级）
        thread::sleep(Duration::from_millis(300));
        assert!(
            !target.exists(),
            "库锁守卫未释放就完成了备份目录拷贝 = 锁窗口泄漏（网络盘慢拷会阻塞业务命令）"
        );
        release_tx.send(()).unwrap();
        handle.join().unwrap();
        assert!(target.exists(), "放行后发布完成");
        assert!(!temp.exists(), "临时文件清理");
    }

    // ── 保留淘汰落地（D4，IO）────────────────────────────────────────────

    #[test]
    fn enforce_retention_deletes_only_oldest_own_named() {
        let (_dir, _data_dir, backup_dir, _db_path) = io_fixture();
        std::fs::create_dir_all(&backup_dir).unwrap();
        for name in [
            backup_file_name("20250101-000000"), // 最旧 → 删
            backup_file_name("20250102-120000"), // 次旧 → 删
            backup_file_name("20250103-000000"), // 留
            backup_file_name("20250104-235959"), // 留
            "ant-feeding-log-backup-20250105.db".to_string(), // 手动备份命名：绝不碰
            "my-manual-copy.db".to_string(),      // 无关 .db：绝不碰
            "说明文档.txt".to_string(),            // 文档：绝不碰
        ] {
            std::fs::write(backup_dir.join(&name), "x").unwrap();
        }
        let removed = enforce_retention(&backup_dir, 2).unwrap();
        assert_eq!(removed, 2, "只删超限的最旧自动备份");
        assert!(backup_dir.join(backup_file_name("20250103-000000")).exists());
        assert!(backup_dir.join(backup_file_name("20250104-235959")).exists());
        assert!(backup_dir.join("ant-feeding-log-backup-20250105.db").exists(), "手动备份幸存");
        assert!(backup_dir.join("my-manual-copy.db").exists(), "无关 .db 幸存");
        assert!(backup_dir.join("说明文档.txt").exists(), "文档幸存");
    }

    // ── 业务写入记账（D2 ① 先行动作）─────────────────────────────────────

    #[test]
    fn record_data_write_sets_today_and_preserves_rest() {
        let dir = tempfile::TempDir::new().unwrap();
        backup_config::save(
            dir.path(),
            &BackupConfig {
                enabled: false,
                backup_dir: Some("D:/bk".into()),
                keep_count: 7,
                last_backup_date: Some("2026-09-01".into()),
                last_data_write_date: None,
                last_result: Some(crate::backup_config::LastBackupOutcome {
                    ok: false,
                    at: "2026-09-01 08:00:00".into(),
                    reason: Some("网盘掉线".into()),
                }),
            },
        )
        .unwrap();
        record_data_write(dir.path(), date(2026, 9, 18)).unwrap();
        let after = backup_config::load(dir.path());
        assert_eq!(after.last_data_write_date.as_deref(), Some("2026-09-18"));
        assert_eq!(after.last_backup_date.as_deref(), Some("2026-09-01"), "其余字段原样");
        assert!(!after.enabled);
        assert_eq!(after.keep_count, 7);
        assert!(after.last_result.is_some());
    }

    // ── 防重入 ───────────────────────────────────────────────────────────

    #[test]
    fn begin_backup_blocks_reentrancy_until_guard_dropped() {
        let _flow = flow_guard();
        let guard = begin_backup().expect("首次应拿到执行位");
        assert!(begin_backup().is_none(), "执行中再触发应被挡");
        drop(guard);
        assert!(begin_backup().is_some(), "drop 后执行位复位");
    }

    // ── 触发点线程体（编排；真实时钟，日期相对注入）──────────────────────

    #[test]
    fn run_triggered_success_backs_up_and_settles_accounts() {
        let _flow = flow_guard();
        let (_dir, data_dir, backup_dir, db_path) = io_fixture();
        let today = chrono::Local::now().date_naive();
        let yesterday = today - chrono::Duration::days(1);
        backup_config::save(
            &data_dir,
            &BackupConfig {
                enabled: true,
                backup_dir: Some(backup_dir.to_string_lossy().to_string()),
                keep_count: 5,
                last_backup_date: None,
                last_data_write_date: Some(iso(yesterday)),
                last_result: None,
            },
        )
        .unwrap();
        let state = db_state(&db_path);

        run_triggered(&state, &data_dir);

        // 恰好一份产物，自家命名可解析、日期是今日（验收 5）
        let names = list_names(&backup_dir);
        assert_eq!(names.len(), 1, "实际：{names:?}");
        let ts = parse_backup_file_name(&names[0]).expect("自家命名可解析");
        assert_eq!(ts.date(), today);
        // 账目：last_backup_date 追平 + 结果成功（验收 1/9）
        let c = backup_config::load(&data_dir);
        assert_eq!(c.last_backup_date.as_deref(), Some(iso(today).as_str()));
        assert_eq!(
            c.last_data_write_date.as_deref(),
            Some(iso(yesterday).as_str()),
            "引擎不动写入账（记账是命令侧的事）"
        );
        let r = c.last_result.expect("有结果");
        assert!(r.ok);
        assert!(r.reason.is_none());
        // 产物可重开、临时文件已清
        let reopened = crate::db::open_and_migrate(&backup_dir.join(&names[0])).unwrap();
        let n: i64 = reopened
            .query_row("SELECT COUNT(*) FROM colony", [], |row| row.get(0))
            .unwrap();
        assert_eq!(n, 1);
        assert!(no_staging_left(&data_dir));
    }

    #[test]
    fn run_triggered_failure_records_quietly_then_retry_recovers() {
        // 验收 3：备份失败（目录写不进去）静默记账，补跑自动恢复，全程无弹窗路径
        let _flow = flow_guard();
        let (dir, data_dir, _backup_dir, db_path) = io_fixture();
        let today = chrono::Local::now().date_naive();
        let yesterday = today - chrono::Duration::days(1);
        let blocked = dir.path().join("blocked-target");
        std::fs::write(&blocked, "占位").unwrap();
        backup_config::save(
            &data_dir,
            &BackupConfig {
                enabled: true,
                backup_dir: Some(blocked.to_string_lossy().to_string()),
                keep_count: 30,
                last_backup_date: None,
                last_data_write_date: Some(iso(yesterday)),
                last_result: None,
            },
        )
        .unwrap();
        let state = db_state(&db_path);

        run_triggered(&state, &data_dir);

        // 失败记账：ok=false + 原因；last_backup_date 不动 → 谓词仍成立（补跑依据）
        let c = backup_config::load(&data_dir);
        let r = c.last_result.expect("失败也要记账");
        assert!(!r.ok);
        assert!(r.reason.is_some(), "失败原因要落账，实际：{r:?}");
        assert_eq!(c.last_backup_date, None);
        assert_eq!(c.last_data_write_date.as_deref(), Some(iso(yesterday).as_str()));
        assert!(no_staging_left(&data_dir), "失败也清临时文件");

        // 补跑：目录修好后，下个触发点（写入/启动）自动补上
        let fixed_dir = dir.path().join("fixed-target");
        backup_config::update_accounting(&data_dir, |c| {
            c.backup_dir = Some(fixed_dir.to_string_lossy().to_string());
        })
        .unwrap();
        run_triggered(&state, &data_dir);
        let c = backup_config::load(&data_dir);
        assert_eq!(c.last_backup_date.as_deref(), Some(iso(today).as_str()), "补跑成功追平");
        assert_eq!(list_names(&fixed_dir).len(), 1);
    }

    #[test]
    fn run_triggered_clock_rollback_recovers_backup_chain() {
        // 验收 4：时钟回拨注入——账目全是未来 → 钳制持久化 + 备份恢复运转（不停摆）
        let _flow = flow_guard();
        let (_dir, data_dir, backup_dir, db_path) = io_fixture();
        let today = chrono::Local::now().date_naive();
        backup_config::save(
            &data_dir,
            &BackupConfig {
                enabled: true,
                backup_dir: Some(backup_dir.to_string_lossy().to_string()),
                keep_count: 30,
                last_backup_date: Some("9999-12-31".into()),
                last_data_write_date: Some("9999-12-31".into()),
                last_result: None,
            },
        )
        .unwrap();
        let state = db_state(&db_path);

        run_triggered(&state, &data_dir);

        let c = backup_config::load(&data_dir);
        assert_eq!(
            c.last_backup_date.as_deref(),
            Some(iso(today).as_str()),
            "未来备份日清空后重新备份追平"
        );
        assert_eq!(c.last_data_write_date.as_deref(), Some(iso(today).as_str()), "未来写入日钳到今日");
        assert_eq!(list_names(&backup_dir).len(), 1, "备份恢复运转");
        assert!(c.last_result.as_ref().expect("成功记账").ok);
    }

    #[test]
    fn run_triggered_without_dir_still_persists_clamp_and_stays_silent() {
        // 目录未设：不产生任何备份动作（验收 8），但钳制在判定入口收敛并持久化
        let _flow = flow_guard();
        let (dir, data_dir, _backup_dir, db_path) = io_fixture();
        let today = chrono::Local::now().date_naive();
        backup_config::save(
            &data_dir,
            &BackupConfig {
                enabled: true,
                backup_dir: None,
                keep_count: 30,
                last_backup_date: Some("9999-12-31".into()),
                last_data_write_date: Some("9998-01-01".into()),
                last_result: None,
            },
        )
        .unwrap();
        let state = db_state(&db_path);

        run_triggered(&state, &data_dir);

        let c = backup_config::load(&data_dir);
        assert_eq!(c.last_backup_date, None, "钳制持久化：未来备份日清空");
        assert_eq!(c.last_data_write_date.as_deref(), Some(iso(today).as_str()), "钳制持久化：写入日钳今日");
        assert_eq!(c.last_result, None, "没跑备份就不记账");
        assert!(no_staging_left(&data_dir));
    }

    #[test]
    fn run_triggered_skips_when_already_backed_up_today() {
        // 跨日空启动：写入与备份都停在昨天 → 谓词不成立：不备、不写盘、不记账（验收 2）
        let _flow = flow_guard();
        let (_dir, data_dir, backup_dir, db_path) = io_fixture();
        let today = chrono::Local::now().date_naive();
        let yesterday = today - chrono::Duration::days(1);
        let saved = BackupConfig {
            enabled: true,
            backup_dir: Some(backup_dir.to_string_lossy().to_string()),
            keep_count: 30,
            last_backup_date: Some(iso(yesterday)),
            last_data_write_date: Some(iso(yesterday)),
            last_result: None,
        };
        backup_config::save(&data_dir, &saved).unwrap();
        let state = db_state(&db_path);

        run_triggered(&state, &data_dir);

        assert_eq!(backup_config::load(&data_dir), saved, "账目一个字节都不动");
        assert!(!backup_dir.exists(), "不产生任何备份产物");
        assert!(no_staging_left(&data_dir));
    }

    #[test]
    fn run_triggered_disabled_is_a_complete_no_op() {
        // 验收 8：开关关 → 谓词即便成立也不产生任何备份动作
        let _flow = flow_guard();
        let (_dir, data_dir, backup_dir, db_path) = io_fixture();
        let today = chrono::Local::now().date_naive();
        let saved = BackupConfig {
            enabled: false,
            backup_dir: Some(backup_dir.to_string_lossy().to_string()),
            keep_count: 30,
            last_backup_date: None,
            last_data_write_date: Some(iso(today - chrono::Duration::days(1))),
            last_result: None,
        };
        backup_config::save(&data_dir, &saved).unwrap();
        let state = db_state(&db_path);

        run_triggered(&state, &data_dir);

        assert_eq!(backup_config::load(&data_dir), saved, "配置原样");
        assert!(!backup_dir.exists(), "连备份目录都不建");
        assert!(no_staging_left(&data_dir));
    }

    #[test]
    fn run_triggered_retention_prunes_after_success() {
        // 验收 6 流程面：成功备份后执行保留淘汰，无关文件幸存
        let _flow = flow_guard();
        let (_dir, data_dir, backup_dir, db_path) = io_fixture();
        let today = chrono::Local::now().date_naive();
        std::fs::create_dir_all(&backup_dir).unwrap();
        std::fs::write(backup_dir.join(backup_file_name("20250101-000000")), "旧1").unwrap();
        std::fs::write(backup_dir.join(backup_file_name("20250102-000000")), "旧2").unwrap();
        std::fs::write(backup_dir.join("ant-feeding-log-backup-20250105.db"), "手动").unwrap();
        backup_config::save(
            &data_dir,
            &BackupConfig {
                enabled: true,
                backup_dir: Some(backup_dir.to_string_lossy().to_string()),
                keep_count: 2,
                last_backup_date: None,
                last_data_write_date: Some(iso(today - chrono::Duration::days(1))),
                last_result: None,
            },
        )
        .unwrap();
        let state = db_state(&db_path);

        run_triggered(&state, &data_dir);

        // 新备份 + 20250102 留下（keep=2），20250101 删，手动备份幸存
        let names = list_names(&backup_dir);
        assert_eq!(names.len(), 3, "实际：{names:?}");
        assert!(names.contains(&backup_file_name("20250102-000000")));
        assert!(names.contains(&"ant-feeding-log-backup-20250105.db".to_string()));
        assert!(!names.contains(&backup_file_name("20250101-000000").to_string()), "最旧被淘汰");
    }
}
