//! 应用日志与异常退出判定（数据安全二期票 01）：日志底座 + 运行标记。
//!
//! 纯函数核心与 IO/Tauri 解耦，cargo test 直接覆盖（沿 system.rs/updater.rs 惯例）：
//! - [`log_file_name`] / [`parse_log_file_name`]：按天滚动文件名 `app-YYYY-MM-DD.log`
//!   与反向解析（不匹配自家命名的文件一律不碰）；
//! - [`stale_log_names`]：清理判定——文件名内日期早于「今天 − 保留天数」才删，
//!   时钟由调用方注入（票面：单测以注入时钟/文件名验证清理逻辑）；
//! - [`format_line`] / [`one_line`] / [`truncate_chars`]：单行日志格式
//!   `[时间] [级别] 消息`（换行折叠、超长截断，panic 消息不得撑破行结构）；
//! - [`judge_exit`]：异常退出判定——运行标记残留 = 上次异常退出；原因取上次会话
//!   期间（时间戳 ≥ 标记启动时间）的 `[PANIC]` 行，没有则「无崩溃日志，疑强杀/断电」；
//! - [`collect_recent_errors`]：跨文件收集 `[ERROR]`/`[PANIC]` 行（新→旧，限量）。
//!
//! IO 薄层（时钟真实、不进单测；但 [`cleanup_old_logs`] / [`append_line_to`] /
//! [`recent_errors_from`] / [`collect_panic_lines`] 吃注入参数，TempDir 直测）：
//! - [`init`]：设全局数据目录 + 装 panic 钩子 + 启动清理过期日志；
//! - [`startup_sequence`]：读运行标记 → 判定并暂存（get_last_abnormal_exit 查询）
//!   → 清旧标记 → 写本次标记 → 记「应用启动」流水；
//! - [`graceful_exit`]：清标记 + 记「应用正常退出」——挂在主事件循环 `RunEvent::Exit`
//!   （覆盖托盘退出与 OS 关机/注销的常规销毁序列）与更新重启钩子上（updater.rs）。
//!
//! 标记文件 [`RUN_MARKER_FILE`] 是数据目录下小 JSON（刻意不进数据库，与
//! update-pending.json 同款取舍：日志/标记的落盘凭据必须与库无关）。
//! 标记损坏或不可读一律按「残留异常」处理——宁可误报不漏报（spec D8）。
//!
//! 写日志绝不 panic（panic 钩子会调它）：锁毒化按原值续用、IO 失败只 eprintln。

use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// 日志目录名（数据目录下的 `logs/`）。
pub const LOG_DIR_NAME: &str = "logs";

/// 日志保留天数：文件名内日期早于「今天 − 14 天」的旧日志启动时清理（spec D7）。
pub const RETENTION_DAYS: i64 = 14;

/// 运行标记文件名（数据目录下小 JSON，不进数据库）。
pub const RUN_MARKER_FILE: &str = "run-marker.json";

/// 最近错误摘要的最大条数（get_recent_errors 返回上限）。
pub const MAX_RECENT_ERRORS: usize = 20;

/// 单行日志的消息截断长度（panic/前端异常消息可能巨长）。
pub const MAX_LINE_CHARS: usize = 500;

// ── 纯核心：文件名与清理 ─────────────────────────────────────────────────

/// 按天滚动的日志文件名：`app-YYYY-MM-DD.log`（date_iso = `YYYY-MM-DD`）。
pub fn log_file_name(date_iso: &str) -> String {
    format!("app-{date_iso}.log")
}

/// 从文件名解析日志日期；不匹配自家命名（其他文件/坏文件名）一律 None——
/// 清理逻辑只对解析成功的文件动手，用户放在同目录的无关文件绝不触碰。
pub fn parse_log_file_name(name: &str) -> Option<chrono::NaiveDate> {
    let stem = name.strip_prefix("app-")?.strip_suffix(".log")?;
    chrono::NaiveDate::parse_from_str(stem, "%Y-%m-%d").ok()
}

/// 清理判定：返回应删除的文件名列表。删除条件 = 文件名可解析出日期，且日期
/// 严格早于「today − keep_days」天（恰好满 keep_days 的保留，超过才删）。
/// 解析不出日期的文件名（无关文件/坏名）绝不返回。
pub fn stale_log_names(names: &[&str], today: chrono::NaiveDate, keep_days: i64) -> Vec<String> {
    let cutoff = today - chrono::Duration::days(keep_days);
    names
        .iter()
        .filter(|n| parse_log_file_name(n).is_some_and(|d| d < cutoff))
        .map(|n| n.to_string())
        .collect()
}

// ── 纯核心：行格式 ───────────────────────────────────────────────────────

/// 一行日志：`[时间] [级别] 消息`（含换行结尾）。消息先单行化再截断。
pub fn format_line(level: &str, now: &str, message: &str) -> String {
    format!(
        "[{now}] [{level}] {}\n",
        truncate_chars(&one_line(message), MAX_LINE_CHARS)
    )
}

/// 单行化：换行折叠为空格（panic/异常消息常带多行，行结构是收集逻辑的地基）。
pub fn one_line(s: &str) -> String {
    s.replace('\r', " ").replace('\n', " ")
}

/// 按字符数截断（中文按 1 字算），截断处补省略号。
pub fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let head: String = s.chars().take(max).collect();
        format!("{head}…")
    }
}

/// 从日志行提取时间戳（首对 `[...]` 内的内容）；无前缀的行返回 None。
pub fn line_timestamp(line: &str) -> Option<&str> {
    let inner = line.strip_prefix('[')?;
    let end = inner.find(']')?;
    Some(&inner[..end])
}

/// 从日志行提取消息部分（剥掉 `[时间] [级别] ` 前缀）；无前缀原样返回。
pub fn line_message(line: &str) -> &str {
    match line.find("] [") {
        Some(i) => {
            let rest = &line[i + 1..];
            match rest.find("] ") {
                Some(j) => &rest[j + 2..],
                None => rest,
            }
        }
        None => line,
    }
}

/// 是否错误行（最近错误摘要与 panic 原因搜索的口径）。
pub fn is_error_line(line: &str) -> bool {
    line.contains("[ERROR]") || line.contains("[PANIC]")
}

// ── 纯核心：异常退出判定 ─────────────────────────────────────────────────

/// 运行标记内容：本会话的启动时间（`YYYY-MM-DD HH:MM:SS`，定宽字符串比较即时间比较）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RunMarker {
    pub started_at: String,
}

/// 读运行标记的三态：无（正常）/ 有效 / 损坏（含不可读——都按残留异常处理，宁误报）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunMarkerRead {
    Absent,
    Valid(RunMarker),
    Corrupted,
}

/// 上次异常退出信息（get_last_abnormal_exit 的返回，前端契约）。
/// `reason` 为 None = 无崩溃日志，前端展示「疑强杀/断电」。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct AbnormalExit {
    /// 异常会话的启动时间（运行标记写入时间）；标记损坏时 None（时间未知）。
    pub session_started_at: Option<String>,
    /// 原因行（上次会话期间的 PANIC 日志）；None = 无崩溃日志。
    pub reason: Option<String>,
}

/// 异常退出判定（spec D8 纯逻辑）：
/// - 无标记 → 上次正常退出，None；
/// - 标记损坏 → 异常（时间未知、无原因）——标记残缺本身就是异常证据；
/// - 标记有效 → 异常；原因 = `panic_lines`（全部 `[PANIC]` 行）里时间戳 ≥ 标记
///   启动时间的最新一条；没有 → None（疑强杀/断电）。
pub fn judge_exit(marker: &RunMarkerRead, panic_lines: &[String]) -> Option<AbnormalExit> {
    match marker {
        RunMarkerRead::Absent => None,
        RunMarkerRead::Corrupted => Some(AbnormalExit {
            session_started_at: None,
            reason: None,
        }),
        RunMarkerRead::Valid(m) => {
            let reason = panic_lines
                .iter()
                .rev()
                .find(|l| line_timestamp(l).is_none_or(|ts| ts >= m.started_at.as_str()))
                .map(|l| line_message(l).to_string());
            Some(AbnormalExit {
                session_started_at: Some(m.started_at.clone()),
                reason,
            })
        }
    }
}

// ── 纯核心：最近错误收集 ─────────────────────────────────────────────────

/// 跨文件收集错误行：`files` 按「新文件在前」传入，文件内按行序倒取（新在上），
/// 返回最多 `max` 条 `[ERROR]`/`[PANIC]` 行（新→旧）。
pub fn collect_recent_errors(files: &[(String, Vec<String>)], max: usize) -> Vec<String> {
    let mut out = Vec::new();
    for (_, lines) in files {
        for line in lines.iter().rev() {
            if out.len() >= max {
                return out;
            }
            if is_error_line(line) {
                out.push(line.clone());
            }
        }
    }
    out
}

// ── IO 薄层（注入参数，TempDir 直测）──────────────────────────────────────

/// 落一行日志到 `data_dir/logs/` 下按 `now` 日期命名的文件（追加，无则建）。
/// 时钟由调用方注入（可测）；生产路径 [`append_line`] 传真实时间。
pub fn append_line_to(data_dir: &Path, level: &str, now: &str, message: &str) {
    let date = now.get(..10).unwrap_or("");
    let logs_dir = data_dir.join(LOG_DIR_NAME);
    if let Err(e) = std::fs::create_dir_all(&logs_dir) {
        eprintln!("[applog] 创建日志目录失败（忽略）: {e}");
        return;
    }
    let path = logs_dir.join(log_file_name(date));
    let line = format_line(level, now, message);
    let result = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut f| std::io::Write::write_all(&mut f, line.as_bytes()));
    if let Err(e) = result {
        eprintln!("[applog] 写日志失败（忽略）: {e}");
    }
}

/// 启动清理：删除 `logs/` 里文件名日期早于「today − 14 天」的旧日志，
/// 返回删除数。目录不存在则先建。无关文件绝不触碰（stale_log_names 保证）。
pub fn cleanup_old_logs(data_dir: &Path, today: chrono::NaiveDate) -> Result<usize, String> {
    let logs_dir = data_dir.join(LOG_DIR_NAME);
    std::fs::create_dir_all(&logs_dir).map_err(|e| format!("创建日志目录失败: {e}"))?;
    let entries = std::fs::read_dir(&logs_dir).map_err(|e| format!("读取日志目录失败: {e}"))?;
    let names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let stale = stale_log_names(&name_refs, today, RETENTION_DAYS);
    let mut removed = 0;
    for name in &stale {
        match std::fs::remove_file(logs_dir.join(name)) {
            Ok(()) => removed += 1,
            Err(e) => eprintln!("[applog] 删除过期日志 {name} 失败（忽略）: {e}"),
        }
    }
    Ok(removed)
}

/// 最近错误摘要（IO 版）：扫描 `logs/` 全部自家日志文件（新→旧）后交给
/// [`collect_recent_errors`]。目录缺失/不可读按空处理。
pub fn recent_errors_from(data_dir: &Path, max: usize) -> Vec<String> {
    let logs_dir = data_dir.join(LOG_DIR_NAME);
    let Ok(entries) = std::fs::read_dir(&logs_dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| parse_log_file_name(n).is_some())
        .collect();
    names.sort();
    names.reverse(); // 文件名定宽日期：字典序倒排 = 新→旧
    let mut files = Vec::new();
    for name in &names {
        if let Ok(text) = std::fs::read_to_string(logs_dir.join(name)) {
            files.push((name.clone(), text.lines().map(str::to_string).collect()));
        }
    }
    collect_recent_errors(&files, max)
}

/// 收集全部日志文件里的 `[PANIC]` 行（旧→新，供 [`judge_exit`] 按会话时间过滤）。
pub fn collect_panic_lines(data_dir: &Path) -> Vec<String> {
    let logs_dir = data_dir.join(LOG_DIR_NAME);
    let Ok(entries) = std::fs::read_dir(&logs_dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| parse_log_file_name(n).is_some())
        .collect();
    names.sort();
    let mut out = Vec::new();
    for name in &names {
        if let Ok(text) = std::fs::read_to_string(logs_dir.join(name)) {
            out.extend(
                text.lines()
                    .filter(|l| l.contains("[PANIC]"))
                    .map(str::to_string),
            );
        }
    }
    out
}

// ── IO 薄层：运行标记 ────────────────────────────────────────────────────

/// 写运行标记（启动序列在「判定完上一次」之后调用，覆盖旧文件）。
/// 数据目录缺失则先建（首启即崩的极端路径也成立）。
pub fn write_run_marker(data_dir: &Path, started_at: &str) -> Result<PathBuf, String> {
    std::fs::create_dir_all(data_dir).map_err(|e| format!("创建数据目录失败: {e}"))?;
    let marker = RunMarker {
        started_at: started_at.to_string(),
    };
    let path = data_dir.join(RUN_MARKER_FILE);
    let text =
        serde_json::to_string_pretty(&marker).map_err(|e| format!("序列化运行标记失败: {e}"))?;
    std::fs::write(&path, text).map_err(|e| format!("写入运行标记失败: {e}"))?;
    Ok(path)
}

/// 清除运行标记（优雅退出路径调用）；文件本来就不存在也按成功。
pub fn clear_run_marker(data_dir: &Path) -> Result<(), String> {
    match std::fs::remove_file(data_dir.join(RUN_MARKER_FILE)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("清除运行标记失败: {e}")),
    }
}

/// 读运行标记三态：缺失 = Absent；损坏（不可解析/启动时间为空）与不可读（权限等）
/// 一律 Corrupted——标记残缺本身是异常证据，宁误报不漏报。
pub fn read_run_marker(data_dir: &Path) -> RunMarkerRead {
    let path = data_dir.join(RUN_MARKER_FILE);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return RunMarkerRead::Absent,
        Err(e) => {
            eprintln!("[applog] 读运行标记失败（按残留异常处理）: {e}");
            return RunMarkerRead::Corrupted;
        }
    };
    match serde_json::from_str::<RunMarker>(&text) {
        Ok(m) if !m.started_at.trim().is_empty() => RunMarkerRead::Valid(m),
        other => {
            eprintln!("[applog] 运行标记损坏（按残留异常处理）: {other:?}");
            RunMarkerRead::Corrupted
        }
    }
}

// ── 薄封装：全局数据目录 + 真实时钟（Tauri 侧，系统行为不进单测）─────────────

/// 全局数据目录（init 时设置一次；log_error/log_action/graceful_exit 免传参）。
static LOG_DATA_DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// 当前数据目录（init 之后有值；日志相关命令经 lib.rs 消费）。
pub fn data_dir() -> Option<&'static PathBuf> {
    LOG_DATA_DIR.get()
}

/// 当前本地时间（`YYYY-MM-DD HH:MM:SS`）。定宽格式，字符串比较即时间比较。
pub fn now_local() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

fn today_naive() -> chrono::NaiveDate {
    chrono::Local::now().date_naive()
}

/// 级别常量：动作流水。
pub const LEVEL_ACTION: &str = "ACTION";
/// 级别常量：错误（命令失败/前端未捕获异常）。
pub const LEVEL_ERROR: &str = "ERROR";
/// 级别常量：Rust panic（崩溃原因行）。
pub const LEVEL_PANIC: &str = "PANIC";

/// 启动初始化：设全局数据目录 → 装 panic 钩子 → 清理过期日志。
/// 任何失败都只 eprintln，绝不挡启动。
pub fn init(data_dir: &Path) {
    let _ = LOG_DATA_DIR.set(data_dir.to_path_buf());
    install_panic_hook();
    if let Err(e) = cleanup_old_logs(data_dir, today_naive()) {
        eprintln!("[applog] 启动清理过期日志失败（忽略）: {e}");
    }
}

/// 记一条动作流水（本票接：启动/正常退出/更新重启；后续票接入备份/恢复/提醒）。
pub fn log_action(message: &str) {
    append_line(LEVEL_ACTION, message);
}

/// 记一条错误（Tauri 命令失败 / 前端未捕获异常转发）。
pub fn log_error(message: &str) {
    append_line(LEVEL_ERROR, message);
}

/// 写入当前会话的日志（真实时钟；未 init 时静默跳过——测试环境无副作用）。
fn append_line(level: &str, message: &str) {
    let Some(dir) = LOG_DATA_DIR.get() else {
        return;
    };
    append_line_to(dir, level, &now_local(), message);
}

/// 装 panic 钩子：先落 `[PANIC]` 日志再链到默认钩子（stderr 打印不变）。
/// 钩子在任意线程的 panic 时触发（含被 catch_unwind 接住的调度线程 panic）——
/// 有日志可查正是本票目的。钩子本体只做字符串拼接与文件追加，自身绝不 panic。
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let msg = format_panic_message(info.payload(), info.location());
        append_line(LEVEL_PANIC, &format!("Rust panic: {msg}"));
        previous(info);
    }));
}

/// panic 信息 → 单行摘要：payload 提取（&str / String / 其他）+ 源位置 + 多行折叠。
pub fn format_panic_message(
    payload: &(dyn std::any::Any + Send),
    location: Option<&std::panic::Location<'_>>,
) -> String {
    let core = if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "未知 panic".to_string()
    };
    let one = one_line(&core);
    match location {
        Some(loc) => format!("{one} @ {loc}"),
        None => one,
    }
}

/// 暂存的上次异常退出判定结果（启动序列写入，get_last_abnormal_exit 查询；
/// 只活在本进程，与 updater 的暂存状态同款）。
static LAST_ABNORMAL_EXIT: Mutex<Option<AbnormalExit>> = Mutex::new(None);

/// 启动序列：读运行标记 → 判定上次退出并暂存 → 清旧标记 → 写本次标记 → 记启动流水。
/// 顺序硬性：判定必须先于清标记；写标记必须先于记「应用启动」之外的任何动作流水。
/// 标记读写失败只 eprintln——宁可下次误报，绝不挡启动。
pub fn startup_sequence(data_dir: &Path) {
    let marker = read_run_marker(data_dir);
    let panic_lines = if matches!(marker, RunMarkerRead::Absent) {
        Vec::new()
    } else {
        collect_panic_lines(data_dir)
    };
    stage_abnormal_exit(judge_exit(&marker, &panic_lines));
    if let Err(e) = clear_run_marker(data_dir) {
        eprintln!("[applog] 清除旧运行标记失败（本次退出可能被误判异常）: {e}");
    }
    if let Err(e) = write_run_marker(data_dir, &now_local()) {
        eprintln!("[applog] 写运行标记失败（本次异常退出将无法识别）: {e}");
    }
    log_action("应用启动");
}

/// 暂存异常退出判定结果（锁毒化按原值续用：提示性数据，不值得 panic）。
fn stage_abnormal_exit(judgment: Option<AbnormalExit>) {
    let mut guard = LAST_ABNORMAL_EXIT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    *guard = judgment;
}

/// 上次异常退出（从未判定过或上次正常 = None）。
pub fn last_abnormal_exit() -> Option<AbnormalExit> {
    let guard = LAST_ABNORMAL_EXIT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    guard.clone()
}

/// 优雅退出收尾：清运行标记 + 记「应用正常退出」流水。挂在主事件循环
/// `RunEvent::Exit`（托盘退出 / OS 关机注销的常规销毁序列）与更新重启钩子上。
/// 清理失败只 eprintln——下次启动会如实误报（宁误报不漏报，spec D8）。
pub fn graceful_exit() {
    if let Some(dir) = LOG_DATA_DIR.get() {
        if let Err(e) = clear_run_marker(dir) {
            eprintln!("[applog] 退出清除运行标记失败（下次启动将误报异常退出）: {e}");
        }
        log_action("应用正常退出");
    }
}

// ── 测试：只测外部行为（spec「Testing Decisions」）────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// ISO 日期串 → NaiveDate（测试里拼 expected 用）。
    fn d(s: &str) -> chrono::NaiveDate {
        chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn temp_data_dir() -> TempDir {
        TempDir::new().expect("创建临时目录失败")
    }

    // ── 文件名命名与解析 ──

    #[test]
    fn log_file_name_roundtrip() {
        // 票面：按天滚动成多个文件——命名含日期即可天然滚动
        assert_eq!(log_file_name("2026-09-18"), "app-2026-09-18.log");
        assert_eq!(
            parse_log_file_name("app-2026-09-18.log"),
            Some(d("2026-09-18"))
        );
    }

    #[test]
    fn parse_log_file_name_rejects_foreign_names() {
        // 无关文件绝不误认（清理逻辑只对自家命名动手的前提）
        assert_eq!(parse_log_file_name("ant-feeding-log-backup-20260918-120000.db"), None);
        assert_eq!(parse_log_file_name("export.csv"), None);
        assert_eq!(parse_log_file_name("app-not-a-date.log"), None);
        assert_eq!(parse_log_file_name("app-2026-13-99.log"), None, "非法日期拒绝");
        assert_eq!(parse_log_file_name("app-2026-09-18.log.bak"), None);
        assert_eq!(parse_log_file_name("data.db"), None);
    }

    // ── 清理判定（票面：注入时钟/文件名验证清理逻辑）──

    #[test]
    fn stale_log_names_deletes_only_over_retention() {
        let today = d("2026-09-18");
        let names = [
            "app-2026-09-18.log", // 今天
            "app-2026-09-17.log", // 昨天
            "app-2026-09-04.log", // 恰好 14 天前：保留（超过才删）
            "app-2026-09-03.log", // 15 天前：删
            "app-2026-08-01.log", // 远古：删
        ];
        let stale = stale_log_names(&names, today, RETENTION_DAYS);
        assert_eq!(
            stale,
            vec!["app-2026-09-03.log".to_string(), "app-2026-08-01.log".to_string()],
            "恰好 14 天的保留，更旧的删除"
        );
    }

    #[test]
    fn stale_log_names_never_touches_unparseable_names() {
        // 同目录可能被用户放进任何东西：解析不了的绝不返回
        let names = [
            "ant-feeding-log-backup-20260801-120000.db",
            "pre-update-v0.2.0-20260801-120000.db",
            "run-marker.json",
            "app-broken.log",
            "",
        ];
        assert!(stale_log_names(&names, d("2026-09-18"), RETENTION_DAYS).is_empty());
    }

    #[test]
    fn stale_log_names_keeps_future_files() {
        // 时钟回拨场景：文件日期晚于今天 → 不删（宁可多留不误删）
        let names = ["app-2027-01-01.log"];
        assert!(stale_log_names(&names, d("2026-09-18"), RETENTION_DAYS).is_empty());
    }

    // ── 行格式 ──

    #[test]
    fn format_line_is_single_line_with_timestamp_and_level() {
        let line = format_line(LEVEL_ERROR, "2026-09-18 08:00:00", "出错了");
        assert_eq!(line, "[2026-09-18 08:00:00] [ERROR] 出错了\n");
        assert_eq!(line_timestamp(&line), Some("2026-09-18 08:00:00"));
        assert_eq!(line_message(line.trim_end()), "出错了");
        assert!(is_error_line(&line));
        assert!(!is_error_line("[2026-09-18 08:00:00] [ACTION] 应用启动"));
    }

    #[test]
    fn format_line_folds_newlines_and_truncates_long_messages() {
        // panic 消息常带多行：折叠成单行，行收集逻辑不被撑破
        let line = format_line(LEVEL_PANIC, "2026-09-18 08:00:00", "line1\nline2");
        assert!(line.starts_with("[2026-09-18 08:00:00] [PANIC] line1 line2"));
        assert_eq!(line.matches('\n').count(), 1, "除结尾换行外无换行");

        let long = "啊".repeat(MAX_LINE_CHARS + 100);
        let line = format_line(LEVEL_ERROR, "2026-09-18 08:00:00", &long);
        assert!(line.contains('…'), "超长截断补省略号");
        assert!(line.chars().count() < long.chars().count());
    }

    #[test]
    fn truncate_chars_counts_by_chars_not_bytes() {
        assert_eq!(truncate_chars("abc", 5), "abc");
        assert_eq!(truncate_chars(&"啊".repeat(3), 5), "啊啊啊");
        let cut = truncate_chars(&"啊".repeat(6), 5);
        assert_eq!(cut.chars().count(), 6, "5 字 + 省略号");
    }

    // ── 异常退出判定 ──

    fn panic_line(ts: &str, msg: &str) -> String {
        format!("[{ts}] [PANIC] {msg}")
    }

    #[test]
    fn judge_exit_absent_marker_means_normal_exit() {
        // 无标记 = 上次优雅退出（托盘退出/关机/更新重启后标记已清）
        assert_eq!(judge_exit(&RunMarkerRead::Absent, &[]), None);
        // 有历史 panic 但无标记（那是更早会话的事，已被正常退出收尾）→ 也不报
        let lines = vec![panic_line("2026-09-17 22:00:00", "老 panic")];
        assert_eq!(judge_exit(&RunMarkerRead::Absent, &lines), None);
    }

    #[test]
    fn judge_exit_reports_session_panic_as_reason() {
        // 验收 1：panic 后重启，显示「上次异常退出」且带原因行
        let marker = RunMarkerRead::Valid(RunMarker {
            started_at: "2026-09-18 08:00:00".into(),
        });
        let lines = vec![
            panic_line("2026-09-17 20:00:00", "上上次会话的旧 panic"),
            panic_line("2026-09-18 09:15:00", "Rust panic: 库已损坏 @ db.rs:1"),
            panic_line("2026-09-18 09:16:00", "Rust panic: 最后一条 @ updater.rs:2"),
        ];
        let out = judge_exit(&marker, &lines).expect("标记残留应判异常");
        assert_eq!(out.session_started_at.as_deref(), Some("2026-09-18 08:00:00"));
        assert_eq!(
            out.reason.as_deref(),
            Some("Rust panic: 最后一条 @ updater.rs:2"),
            "取会话期间最新一条 panic"
        );
    }

    #[test]
    fn judge_exit_ignores_panics_older_than_session() {
        // 会话启动前的 panic（更早会话遗留）不能当本次原因
        let marker = RunMarkerRead::Valid(RunMarker {
            started_at: "2026-09-18 08:00:00".into(),
        });
        let lines = vec![panic_line("2026-09-17 22:00:00", "老 panic")];
        let out = judge_exit(&marker, &lines).expect("标记残留仍判异常");
        assert_eq!(out.reason, None, "无本会话崩溃日志 → 疑强杀/断电");
    }

    #[test]
    fn judge_exit_without_panic_is_suspected_force_kill() {
        // 验收 2：强杀/断电（无崩溃日志）→ 异常成立、原因空，前端补「疑强杀/断电」
        let marker = RunMarkerRead::Valid(RunMarker {
            started_at: "2026-09-18 08:00:00".into(),
        });
        let out = judge_exit(&marker, &[]).expect("标记残留应判异常");
        assert_eq!(out.session_started_at.as_deref(), Some("2026-09-18 08:00:00"));
        assert_eq!(out.reason, None);
    }

    #[test]
    fn judge_exit_corrupted_marker_reports_abnormal_with_unknown_time() {
        // 标记损坏（如崩溃发生在写标记中途）也是异常证据：宁误报不漏报
        let out = judge_exit(&RunMarkerRead::Corrupted, &[]).expect("损坏标记判异常");
        assert_eq!(out.session_started_at, None, "时间未知");
        assert_eq!(out.reason, None);
    }

    #[test]
    fn abnormal_exit_serializes_snake_case_for_frontend() {
        // 前端契约：字段 snake_case（serde 默认），reason 为 null 表示疑强杀
        let json = serde_json::to_string(&AbnormalExit {
            session_started_at: Some("2026-09-18 08:00:00".into()),
            reason: None,
        })
        .unwrap();
        assert_eq!(
            json,
            r#"{"session_started_at":"2026-09-18 08:00:00","reason":null}"#
        );
    }

    // ── 最近错误收集 ──

    #[test]
    fn collect_recent_errors_newest_first_with_limit() {
        let files = vec![
            (
                "app-2026-09-18.log".to_string(),
                vec![
                    "[2026-09-18 10:00:00] [ACTION] 应用启动".to_string(),
                    "[2026-09-18 10:01:00] [ERROR] 错误甲".to_string(),
                    "[2026-09-18 10:02:00] [PANIC] 崩溃".to_string(),
                ],
            ),
            (
                "app-2026-09-17.log".to_string(),
                vec![
                    "[2026-09-17 09:00:00] [ERROR] 错误乙".to_string(),
                    "[2026-09-17 09:30:00] [ACTION] 应用正常退出".to_string(),
                ],
            ),
        ];
        let out = collect_recent_errors(&files, 20);
        assert_eq!(
            out,
            vec![
                "[2026-09-18 10:02:00] [PANIC] 崩溃".to_string(),
                "[2026-09-18 10:01:00] [ERROR] 错误甲".to_string(),
                "[2026-09-17 09:00:00] [ERROR] 错误乙".to_string(),
            ],
            "新→旧，ACTION 不算错误"
        );

        let out = collect_recent_errors(&files, 2);
        assert_eq!(out.len(), 2, "限量截断");
        assert!(out[0].contains("崩溃"), "限量也保持新在上");
    }

    // ── IO 薄层（TempDir）──

    #[test]
    fn run_marker_write_read_clear_roundtrip() {
        let dir = temp_data_dir();
        assert_eq!(read_run_marker(dir.path()), RunMarkerRead::Absent, "无标记");

        write_run_marker(dir.path(), "2026-09-18 08:00:00").unwrap();
        assert_eq!(
            read_run_marker(dir.path()),
            RunMarkerRead::Valid(RunMarker {
                started_at: "2026-09-18 08:00:00".into(),
            })
        );

        clear_run_marker(dir.path()).unwrap();
        assert_eq!(read_run_marker(dir.path()), RunMarkerRead::Absent);
        // 清除本就不存在的标记：no-op 成功（幂等，退出路径可重复调用）
        clear_run_marker(dir.path()).unwrap();
    }

    #[test]
    fn run_marker_write_creates_missing_data_dir() {
        // 首启即写标记：数据目录还不存在也能落（标记不依赖库/其他初始化）
        let dir = temp_data_dir();
        let data_dir = dir.path().join("data");
        write_run_marker(&data_dir, "2026-09-18 08:00:00").unwrap();
        assert!(data_dir.join(RUN_MARKER_FILE).exists());
    }

    #[test]
    fn corrupted_run_marker_treated_as_corrupted() {
        let dir = temp_data_dir();
        std::fs::write(dir.path().join(RUN_MARKER_FILE), "不是 JSON{{{").unwrap();
        assert_eq!(read_run_marker(dir.path()), RunMarkerRead::Corrupted);

        // JSON 合法但启动时间为空也算损坏
        std::fs::write(
            dir.path().join(RUN_MARKER_FILE),
            r#"{"started_at":""}"#,
        )
        .unwrap();
        assert_eq!(read_run_marker(dir.path()), RunMarkerRead::Corrupted);
    }

    #[test]
    fn cleanup_old_logs_deletes_only_stale_app_logs() {
        let dir = temp_data_dir();
        let logs = dir.path().join(LOG_DIR_NAME);
        std::fs::create_dir_all(&logs).unwrap();
        for name in [
            "app-2026-09-18.log",
            "app-2026-09-04.log", // 恰好 14 天：留
            "app-2026-09-03.log", // 15 天：删
            "ant-feeding-log-backup-20260801-120000.db", // 无关文件：绝不碰
            "app-broken.log",     // 坏名：不碰
        ] {
            std::fs::write(logs.join(name), "x").unwrap();
        }

        let removed = cleanup_old_logs(dir.path(), d("2026-09-18")).unwrap();
        assert_eq!(removed, 1, "只删超过 14 天的旧日志");
        assert!(logs.join("app-2026-09-18.log").exists());
        assert!(logs.join("app-2026-09-04.log").exists());
        assert!(!logs.join("app-2026-09-03.log").exists());
        assert!(logs.join("ant-feeding-log-backup-20260801-120000.db").exists());
        assert!(logs.join("app-broken.log").exists());

        // 目录不存在：先建后清，不报错（首启路径）
        let fresh = temp_data_dir();
        assert_eq!(cleanup_old_logs(fresh.path(), d("2026-09-18")).unwrap(), 0);
        assert!(fresh.path().join(LOG_DIR_NAME).exists());
    }

    #[test]
    fn append_and_recent_errors_roundtrip_across_files() {
        // 验收：错误落日志、按天分文件、摘要跨文件收集（新在上）
        let dir = temp_data_dir();
        append_line_to(dir.path(), LEVEL_ACTION, "2026-09-17 09:00:00", "应用启动");
        append_line_to(dir.path(), LEVEL_ERROR, "2026-09-17 09:05:00", "昨天的错误");
        append_line_to(dir.path(), LEVEL_ACTION, "2026-09-18 08:00:00", "应用启动");
        append_line_to(dir.path(), LEVEL_ERROR, "2026-09-18 09:00:00", "今天的错误");

        let logs = dir.path().join(LOG_DIR_NAME);
        assert!(
            logs.join("app-2026-09-17.log").exists() && logs.join("app-2026-09-18.log").exists(),
            "按天滚动成多个文件"
        );

        let out = recent_errors_from(dir.path(), MAX_RECENT_ERRORS);
        assert_eq!(out.len(), 2);
        assert!(out[0].contains("今天的错误"), "新在上");
        assert!(out[1].contains("昨天的错误"));

        // 目录缺失（还没写过日志）：空列表，不报错
        assert!(recent_errors_from(temp_data_dir().path(), 20).is_empty());
    }

    #[test]
    fn collect_panic_lines_scans_all_log_files_chronologically() {
        let dir = temp_data_dir();
        append_line_to(dir.path(), LEVEL_PANIC, "2026-09-17 20:00:00", "昨天的 panic");
        append_line_to(dir.path(), LEVEL_ERROR, "2026-09-18 08:30:00", "普通错误不算");
        append_line_to(dir.path(), LEVEL_PANIC, "2026-09-18 09:00:00", "今天的 panic");

        let lines = collect_panic_lines(dir.path());
        assert_eq!(lines.len(), 2, "只收 PANIC 行");
        assert!(lines[0].contains("昨天的 panic"), "旧→新（judge_exit 从后往前取最新）");
        assert!(lines[1].contains("今天的 panic"));
    }

    // ── panic 消息格式化 ──

    #[test]
    fn format_panic_message_extracts_payload_and_location() {
        // String payload（panic!("带参 {x}") 编译成 String）
        let owned: Box<dyn std::any::Any + Send> = Box::new("boom".to_string());
        assert_eq!(format_panic_message(owned.as_ref(), None), "boom");

        // &str payload（字面量 panic）
        let lit: Box<dyn std::any::Any + Send> = Box::new("字面量 panic");
        assert_eq!(format_panic_message(lit.as_ref(), None), "字面量 panic");

        // 未知 payload（panic_any(42) 这类）不 panic 钩子自身
        let weird: Box<dyn std::any::Any + Send> = Box::new(42u8);
        assert_eq!(format_panic_message(weird.as_ref(), None), "未知 panic");

        // 位置 + 多行折叠：单行输出
        let loc = std::panic::Location::caller();
        let multi: Box<dyn std::any::Any + Send> = Box::new("line1\nline2".to_string());
        let msg = format_panic_message(multi.as_ref(), Some(loc));
        assert!(msg.starts_with("line1 line2 @ "), "实际：{msg}");
        assert!(!msg.contains('\n'), "必须单行，实际：{msg}");
    }
}
