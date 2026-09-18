//! 应用内整库恢复（数据安全二期票 04）：四步协议 + 摘要预览 + pre-restore 快照。
//!
//! spec D5/D6/D10，决策背景 ADR-0002（整库替换语义，非合并）。纯函数核心与
//! IO/锁解耦（沿 auto_backup.rs/system.rs 惯例）：
//! - 步骤 1 源校验与 staging：[`same_path`] 规范化后比较、拒绝「源路径 ==
//!   当前库路径」；源文件复制为数据目录 [`staging_file_name`]（`restore-staging-
//!   <时间戳>.db`），此后一切校验在临时库上做，全程不碰当前库；
//! - 步骤 2 临时库校验 [`validate_staging`]：可打开（非 SQLite/0 字节 →
//!   「这不是本应用的备份」）→ `PRAGMA integrity_check` = ok（→「备份文件已
//!   损坏」）→ schema 版本：旧版在临时库上跑 [`crate::db::migrate`] 升级后复校，
//!   比当前程序新 → 「请先升级应用」；通过即产出 [`RestoreSummary`] 摘要
//!   （备份日期、窝数、记录数、库内备份目录设置值）——摘要预览是选错文件的
//!   最后防线（D6）；
//! - 步骤 3 pre-restore 快照：[`snapshot_current`] 拷当前库到数据目录
//!   `pre-restore-<时间戳>.db`，最多保留 [`SNAPSHOT_KEEP`] 份超出删最旧
//!   （[`parse_snapshot_file_name`] 只认自家命名，绝不碰其他文件）；
//! - 步骤 4 替换与重载：[`replace_and_reload`] 锁内先把旧连接降为内存占位
//!   连接（Windows 下不释放文件句柄 rename 覆盖会失败）→ 写 `<库>.new` +
//!   原子改名覆盖 → 重开新连接放回锁内；重开失败重试 3 次（短退避）→ 仍失败
//!   保持新库文件、返回「请重启应用」；
//! - 失败语义：步骤 1–3 任一失败 → 当前库字节级零改动、清理 staging、报具体
//!   原因；步骤 4 是唯一写动作且先写后改名，中途失败原库文件未破坏——
//!   「自动回滚」由协议结构保证。
//!
//! 命令薄封装与 rfd 选文件对话框在 lib.rs。

use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde::Serialize;

use crate::auto_backup;

/// 数据目录里恢复 staging 临时文件前缀（`restore-staging-<时间戳>.db`）。
pub const STAGING_PREFIX: &str = "restore-staging-";

/// pre-restore 快照文件前缀（数据目录下 `pre-restore-<时间戳>.db`）。
pub const SNAPSHOT_PREFIX: &str = "pre-restore-";

/// 快照保留份数（spec D5：最多保留 3 份，超出删最旧）。
pub const SNAPSHOT_KEEP: usize = 3;

/// 重开连接重试次数（spec D5 附录 #10：重试 3 次）。
pub const RECONNECT_ATTEMPTS: usize = 3;

/// 重开连接重试退避（短退避；库文件被杀毒/索引器短暂占用通常百毫秒级让位）。
pub const RECONNECT_BACKOFF: std::time::Duration = std::time::Duration::from_millis(120);

/// 恢复摘要（restore_preview 返回体，D10 契约；前端最后防线 D6）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RestoreSummary {
    /// 备份日期（`YYYY-MM-DD`）：优先文件名时间戳（自动备份命名），否则库内最新记录日期。
    pub backup_date: Option<String>,
    /// 备份内窝数。
    pub colony_count: i64,
    /// 备份内记录数。
    pub log_count: i64,
    /// 备份内的备份目录设置值（settings 表 `backup_dir` 键；D1 后备份设置存库外，
    /// 新备份无此键 → None，前端点明「备份设置保持当前值，不随恢复回滚」）。
    pub backup_dir_in_backup: Option<String>,
}

/// 恢复执行结果（restore_apply 返回体）。
/// Done = 替换且重连成功，前端广播刷新即可；DoneNeedsRestart = 新库文件已就位
/// 但重开连接重试仍失败，提示「恢复已完成，请重启应用」。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplyOutcome {
    Done,
    DoneNeedsRestart,
}

// ── 命名与解析（步骤 1/3 的地基）─────────────────────────────────────────

/// 真实时钟的时间戳（`YYYYMMDD-HHMMSS`，与自动备份同一格式；命令层用，测试注入固定串）。
pub fn stamp_now() -> String {
    auto_backup::stamp_now()
}

/// staging 临时文件名：`restore-staging-<时间戳>.db`。
pub fn staging_file_name(stamp: &str) -> String {
    format!("{STAGING_PREFIX}{stamp}.db")
}

/// pre-restore 快照文件名：`pre-restore-<时间戳>.db`。
pub fn snapshot_file_name(stamp: &str) -> String {
    format!("{SNAPSHOT_PREFIX}{stamp}.db")
}

/// 反向解析自家快照命名；不匹配/位数不对/非数字/日历不合法一律 None——
/// 快照保留淘汰只对解析成功的文件动手，数据目录其他文件绝不触碰。
pub fn parse_snapshot_file_name(name: &str) -> Option<chrono::NaiveDateTime> {
    let stem = name.strip_prefix(SNAPSHOT_PREFIX)?.strip_suffix(".db")?;
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

/// 同一文件的判定（拒绝「源路径 == 当前库路径」）：优先 `fs::canonicalize`
/// 规范化后比较（消解 `..`/`./`/符号盘符大小写）；任一路径不存在时退化为
/// 字符串比较（Windows 大小写不敏感）。
pub fn same_path(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(ca), Ok(cb)) => path_text_eq(&ca.to_string_lossy(), &cb.to_string_lossy()),
        _ => path_text_eq(&a.to_string_lossy(), &b.to_string_lossy()),
    }
}

fn path_text_eq(a: &str, b: &str) -> bool {
    #[cfg(windows)]
    {
        a.to_lowercase() == b.to_lowercase()
    }
    #[cfg(not(windows))]
    {
        a == b
    }
}

/// 替换用侧车文件路径：`<库文件>.new`（同目录，供原子改名覆盖）。
fn sidecar_new_path(db_path: &Path) -> PathBuf {
    let mut s = db_path.as_os_str().to_os_string();
    s.push(".new");
    PathBuf::from(s)
}

// ── 步骤 2：临时库校验链与摘要 ───────────────────────────────────────────

/// 临时库校验链（spec D5 步骤 2，校验集 = D6：可打开 + integrity_check +
/// schema 版本）。`file_date` = 源文件名时间戳（自动备份命名可解析时），供
/// 摘要的备份日期优先使用。任何失败都返回可直接展示的中文拒绝原因。
pub fn validate_staging(
    conn: &Connection,
    file_date: Option<chrono::NaiveDate>,
) -> Result<RestoreSummary, String> {
    check_integrity(conn)?;
    migrate_if_old(conn)?;
    build_summary(conn, file_date)
}

/// integrity_check：非 SQLite 文件第一条查询即报 not a database；损坏库返回
/// 非 ok 行（或查询本身报 malformed）。
fn check_integrity(conn: &Connection) -> Result<(), String> {
    match conn.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0)) {
        Ok(v) if v.eq_ignore_ascii_case("ok") => Ok(()),
        Ok(other) => Err(format!("备份文件已损坏（integrity_check：{other}）")),
        Err(e) => {
            let m = e.to_string();
            if m.contains("not a database") {
                Err("这不是本应用的备份".to_string())
            } else if m.contains("malformed") || m.contains("corrupt") || m.contains("encrypted") {
                Err("备份文件已损坏".to_string())
            } else {
                Err(format!("无法读取备份文件: {m}"))
            }
        }
    }
}

/// schema 版本：比当前程序新 → 拒绝（绝不降级）；旧版在临时库上跑迁移管线
/// 升级后复校 integrity（spec D5「升级后复校」）。
fn migrate_if_old(conn: &Connection) -> Result<(), String> {
    let version = crate::db::schema_version_of(conn)
        .map_err(|e| format!("无法读取备份 schema 版本: {e}"))?;
    if version > crate::db::SCHEMA_VERSION {
        return Err(format!(
            "这是未来版本的备份（schema v{version}，当前应用支持 v{}），请先升级应用",
            crate::db::SCHEMA_VERSION
        ));
    }
    if version < crate::db::SCHEMA_VERSION {
        crate::db::migrate(conn).map_err(|e| format!("备份 schema 升级失败: {e}"))?;
        check_integrity(conn)?;
    }
    Ok(())
}

/// 产出摘要：窝数 / 记录数 / 库内备份目录设置值；备份日期 = 文件名时间戳
/// （`file_date`，自动备份命名可解析时），否则库内最新记录的日期。
fn build_summary(
    conn: &Connection,
    file_date: Option<chrono::NaiveDate>,
) -> Result<RestoreSummary, String> {
    let backup_date = match file_date {
        Some(d) => Some(d.format("%Y-%m-%d").to_string()),
        None => conn
            .query_row("SELECT MAX(occurred_at) FROM care_log", [], |r| {
                r.get::<_, Option<String>>(0)
            })
            .map_err(|e| format!("读取备份内记录失败: {e}"))?
            .map(|s| {
                let bytes = s.as_bytes();
                if bytes.len() >= 10 && s.is_char_boundary(10) {
                    s[..10].to_string()
                } else {
                    s
                }
            }),
    };
    let colony_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM colony", [], |r| r.get(0))
        .map_err(|e| format!("读取备份内窝数失败: {e}"))?;
    let log_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM care_log", [], |r| r.get(0))
        .map_err(|e| format!("读取备份内记录数失败: {e}"))?;
    // D1 后备份设置存库外，正常备份无此键 → None；键存在（旧布局）则回显旧值
    let backup_dir_in_backup = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'backup_dir'",
            [],
            |r| r.get::<_, String>(0),
        )
        .ok();
    Ok(RestoreSummary {
        backup_date,
        colony_count,
        log_count,
        backup_dir_in_backup,
    })
}

// ── 步骤 1+2：staging 与校验编排 ─────────────────────────────────────────

/// 清理 staging 文件（NotFound 视为已清理；清理失败不掩盖原错误）。
fn cleanup_staging_file(staging: &Path) {
    if let Err(e) = std::fs::remove_file(staging) {
        if e.kind() != std::io::ErrorKind::NotFound {
            eprintln!("[restore] 清理 staging 失败（忽略）: {e}");
        }
    }
}

/// 步骤 1+2（preview 与 apply 共用）：拒绝自恢复 → 拒绝空文件 → 复制为
/// staging → 打开临时库跑校验链。失败路径 staging 一律清理，当前库从未被
/// 碰过。成功返回（临时库连接，staging 路径，摘要）——调用方负责用完后清理。
pub fn stage_and_validate(
    source: &Path,
    current_db: &Path,
    data_dir: &Path,
    stamp: &str,
) -> Result<(Connection, PathBuf, RestoreSummary), String> {
    if same_path(source, current_db) {
        return Err("不能恢复当前正在使用的库自身，请选择一份备份文件".to_string());
    }
    // 0 字节文件会被 SQLite 当全新空库放行，必须显式拒绝
    match std::fs::metadata(source) {
        Ok(m) if m.len() == 0 => return Err("这不是本应用的备份（文件为空）".to_string()),
        Ok(_) => {}
        Err(e) => return Err(format!("无法读取备份文件: {e}")),
    }
    let staging = data_dir.join(staging_file_name(stamp));
    let inner = || -> Result<(Connection, PathBuf, RestoreSummary), String> {
        crate::system::backup_db_file(source, &staging)?;
        let conn = Connection::open(&staging).map_err(|e| format!("无法打开备份文件: {e}"))?;
        // 文件名时间戳（自动备份命名可解析时）传给摘要，优先于库内最新记录
        let file_date = auto_backup::parse_backup_file_name(
            source.file_name().map(|n| n.to_string_lossy()).as_deref().unwrap_or(""),
        )
        .map(|ts| ts.date());
        let summary = validate_staging(&conn, file_date)?;
        Ok((conn, staging.clone(), summary))
    };
    match inner() {
        Ok(v) => Ok(v),
        Err(e) => {
            // inner 返回 Err 时连接已 drop（未随 Ok 返回），句柄已释放可删
            cleanup_staging_file(&staging);
            Err(e)
        }
    }
}

/// 用完 staging 后的收尾：关连接、删文件（preview 摘要产出后 / apply 替换后）。
pub fn discard_staging(conn: Connection, staging: &Path) {
    drop(conn);
    cleanup_staging_file(staging);
}

// ── 步骤 3：pre-restore 快照与保留 ──────────────────────────────────────

/// 快照保留淘汰判定：只匹配自家 `pre-restore-<8位日期>-<6位时间>.db` 且可解析
/// 的文件，按文件名内时间戳新→旧排序保留最新 `keep` 份，其余按旧→新返回。
pub fn stale_snapshot_names(names: &[&str], keep: usize) -> Vec<String> {
    let mut parsed: Vec<(chrono::NaiveDateTime, &str)> = names
        .iter()
        .filter_map(|n| parse_snapshot_file_name(n).map(|ts| (ts, *n)))
        .collect();
    parsed.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(a.1)));
    let mut stale: Vec<String> = parsed
        .into_iter()
        .skip(keep)
        .map(|(_, n)| n.to_string())
        .collect();
    stale.reverse();
    stale
}

/// 快照保留淘汰落地：扫数据目录，超出 `keep` 份删最旧（只删自家命名可解析的
/// **普通文件**；目录/无关文件绝不触碰）。返回删除数。
pub fn retain_snapshots(data_dir: &Path, keep: usize) -> Result<usize, String> {
    let entries = std::fs::read_dir(data_dir).map_err(|e| format!("读取数据目录失败: {e}"))?;
    let names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut removed = 0;
    for name in stale_snapshot_names(&refs, keep) {
        match std::fs::remove_file(data_dir.join(&name)) {
            Ok(()) => removed += 1,
            Err(e) => eprintln!("[restore] 删除超限快照 {name} 失败（忽略）: {e}"),
        }
    }
    Ok(removed)
}

/// 步骤 3：当前库拷为 `pre-restore-<时间戳>.db`（反悔通道），并保留最近
/// [`SNAPSHOT_KEEP`] 份。拷贝失败清掉半截产物。
pub fn snapshot_current(current_db: &Path, data_dir: &Path, stamp: &str) -> Result<PathBuf, String> {
    let snap = data_dir.join(snapshot_file_name(stamp));
    match crate::system::backup_db_file(current_db, &snap) {
        Ok(()) => {
            if let Err(e) = retain_snapshots(data_dir, SNAPSHOT_KEEP) {
                eprintln!("[restore] 快照保留淘汰失败（忽略）: {e}");
            }
            Ok(snap)
        }
        Err(e) => {
            let _ = std::fs::remove_file(&snap);
            Err(format!("恢复前快照失败: {e}"))
        }
    }
}

// ── 步骤 4：替换与重载 ──────────────────────────────────────────────────

/// 锁内替换与重载（spec D5 步骤 4）。`guard` 为调用方拿到的库锁守卫
/// （生产 = `DbState` 的 `MutexGuard<Connection>`；`open_conn` 注入连接工厂，
/// 测试可换失败注入版）：
/// 1. 旧连接先降为内存占位连接（Windows 下不释放文件句柄，rename 覆盖会
///    失败；占位库无任何表——若最终留在锁内，业务命令当场报错而非静默读写）；
/// 2. staging → `<库>.new` 拷贝 → 原子改名覆盖库文件（唯一写动作）；
/// 3. 重开新库放回锁内；失败重试 [`RECONNECT_ATTEMPTS`] 次（短退避），
///    仍失败保持新库文件、返回 [`ApplyOutcome::DoneNeedsRestart`]。
/// 中途失败（拷贝/改名）：原库文件从未被覆盖，重开原库放回锁内恢复可用态。
pub fn replace_and_reload<G>(
    mut guard: G,
    db_path: &Path,
    staging: &Path,
    open_conn: &impl Fn(&Path) -> Result<Connection, String>,
) -> Result<ApplyOutcome, String>
where
    G: std::ops::DerefMut<Target = Connection>,
{
    let new_path = sidecar_new_path(db_path);

    // ① 旧连接降为占位（锁内 drop；受锁保护，无并发使用）
    let placeholder =
        Connection::open_in_memory().map_err(|e| format!("创建占位连接失败: {e}"))?;
    // 旧值必须当场 drop = 立即关闭旧连接的文件句柄（Windows 上句柄不释放，
    // rename 覆盖会报拒绝访问）
    drop(std::mem::replace(&mut *guard, placeholder));

    // ② 写新文件 + 原子改名（中途失败原库文件未破坏）
    let swapped = (|| -> Result<(), String> {
        std::fs::copy(staging, &new_path).map_err(|e| format!("写入新库文件失败: {e}"))?;
        std::fs::rename(&new_path, db_path).map_err(|e| format!("替换库文件失败: {e}"))?;
        Ok(())
    })();
    if let Err(e) = swapped {
        let _ = std::fs::remove_file(&new_path);
        match open_conn(db_path) {
            Ok(conn) => {
                drop(std::mem::replace(&mut *guard, conn));
            }
            Err(e2) => {
                // 双失败：原库文件完好但重开也失败——占位连接留在锁内，业务
                // 命令会持续报错直到重启。错误必须带重启指引到前端（不能只留
                // 日志），并落一条错误流水（除命令层的失败日志外这里记库级
                // 细节，排障用）。
                let msg = format!("{e}；此外重开原库也失败（{e2}），请重启应用后重试恢复");
                crate::applog::log_error(&format!("[restore] 替换失败且重开原库失败: {msg}"));
                return Err(msg);
            }
        }
        return Err(e);
    }

    // ③ 重开新库放回锁内；失败重试（短退避）
    for attempt in 0..RECONNECT_ATTEMPTS {
        match open_conn(db_path) {
            Ok(conn) => {
                drop(std::mem::replace(&mut *guard, conn));
                return Ok(ApplyOutcome::Done);
            }
            Err(e) => {
                eprintln!("[restore] 重开新库失败（第 {} 次）: {e}", attempt + 1);
                if attempt + 1 < RECONNECT_ATTEMPTS {
                    std::thread::sleep(RECONNECT_BACKOFF);
                }
            }
        }
    }
    // 重试仍失败：新库文件已就位（staging 校验过 = 完整可用），保持新库并
    // 提示重启；内存占位留在锁内，业务命令报错而非静默空读（spec 附录 #10）
    Ok(ApplyOutcome::DoneNeedsRestart)
}

// ── 编排：preview 与 apply ──────────────────────────────────────────────

/// 恢复预览（restore_preview 主体）：步骤 1+2，产出摘要后 staging 即清。
pub fn run_preview(
    source: &Path,
    current_db: &Path,
    data_dir: &Path,
    stamp: &str,
) -> Result<RestoreSummary, String> {
    let (conn, staging, summary) = stage_and_validate(source, current_db, data_dir, stamp)?;
    discard_staging(conn, &staging);
    Ok(summary)
}

/// 恢复执行（restore_apply 主体）：重走完整校验（步骤 1+2，TOCTOU 安全——
/// preview 与 apply 之间源文件可能被换）→ 快照（步骤 3）→ 替换重载（步骤 4）。
/// 库锁窗口覆盖快照与替换两段（都是本地毫秒级拷贝，与自动备份阶段一同理）；
/// staging 无论成败一律清理。
pub fn run_apply<G, L, O>(
    source: &Path,
    current_db: &Path,
    data_dir: &Path,
    stamp: &str,
    take_lock: L,
    open_conn: O,
) -> Result<ApplyOutcome, String>
where
    L: FnOnce() -> Result<G, String>,
    G: std::ops::DerefMut<Target = Connection>,
    O: Fn(&Path) -> Result<Connection, String>,
{
    let (conn, staging, _summary) = stage_and_validate(source, current_db, data_dir, stamp)?;
    // 校验完先关临时库连接（句柄释放）；staging 留给步骤 4 替换用，替换后才清
    drop(conn);

    let outcome = (|| -> Result<ApplyOutcome, String> {
        let guard = take_lock()?;
        // 快照 + 替换共用同一锁窗口（本地毫秒级；锁内拷贝不受并发写干扰）
        snapshot_current(current_db, data_dir, stamp)?;
        replace_and_reload(guard, current_db, &staging, &open_conn)
    })();

    // staging 无论成败都清（成功时真内容已落库）
    cleanup_staging_file(&staging);
    outcome
}

// ── 测试：只测外部行为（spec「Testing Decisions」）────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use rusqlite::params;
    use tempfile::TempDir;

    // ── 通用脚手架 ───────────────────────────────────────────────────────

    /// 在指定路径建一个已迁移库并灌数据（colony 数量与记录数自定），
    /// 关闭连接返回。返回库里的记录数。
    fn make_db(path: &Path, colony_name: &str, log_count: usize) {
        let conn = crate::db::open_and_migrate(path).expect("建库失败");
        conn.execute(
            "INSERT INTO colony (name, start_date) VALUES (?1, '2026-01-20')",
            params![colony_name],
        )
        .unwrap();
        let action: i64 = conn
            .query_row("SELECT id FROM care_action WHERE name = '喂食'", [], |r| r.get(0))
            .unwrap();
        for i in 0..log_count {
            conn.execute(
                "INSERT INTO care_log (colony_id, action_id, occurred_at, note, created_at)
                 VALUES (1, ?1, ?2, '', ?2)",
                params![action, format!("2026-09-{:02} 08:00:00", i + 1)],
            )
            .unwrap();
        }
        drop(conn);
    }

    fn db_bytes(path: &Path) -> Vec<u8> {
        std::fs::read(path).unwrap()
    }

    fn list_names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        names.sort();
        names
    }

    fn no_staging_left(data_dir: &Path) -> bool {
        list_names(data_dir).iter().all(|n| !n.starts_with(STAGING_PREFIX))
    }

    /// 双库夹具：tempdir 里 data_dir + 当前库（现窝 + 1 记录）。返回
    /// (tempdir, data_dir, current_db_path)。
    fn fixture() -> (TempDir, PathBuf, PathBuf) {
        let dir = TempDir::new().unwrap();
        let data_dir = dir.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        let db_path = data_dir.join(crate::db::DB_FILE_NAME);
        make_db(&db_path, "现窝", 1);
        (dir, data_dir, db_path)
    }

    /// 模拟应用运行态：重开当前库连接的 Mutex（同 DbState.0 形状）。
    fn live_lock(db_path: &Path) -> Mutex<Connection> {
        Mutex::new(crate::db::open_and_migrate(db_path).unwrap())
    }

    const STAMP: &str = "20260918-120000";

    /// tempdir 根路径（fixture 返回的 _dir 借用期间取路径用）。
    fn dir_path(d: &TempDir) -> std::path::PathBuf {
        d.path().to_path_buf()
    }

    // ── 命名与解析 ───────────────────────────────────────────────────────

    #[test]
    fn file_names_use_own_prefixes_and_strict_parse() {
        assert_eq!(staging_file_name(STAMP), "restore-staging-20260918-120000.db");
        assert_eq!(snapshot_file_name(STAMP), "pre-restore-20260918-120000.db");

        let ok = parse_snapshot_file_name("pre-restore-20260918-120000.db").unwrap();
        assert_eq!(ok, chrono::NaiveDate::from_ymd_opt(2026, 9, 18).unwrap().and_hms_opt(12, 0, 0).unwrap());
        // 自动备份命名 / 手动命名 / staging 命名：绝不认（保留淘汰不得触碰）
        assert_eq!(parse_snapshot_file_name("ant-feeding-log-backup-20260918-120000.db"), None);
        assert_eq!(parse_snapshot_file_name("pre-restore-20260918.db"), None);
        assert_eq!(parse_snapshot_file_name("restore-staging-20260918-120000.db"), None);
        // 位数不对 / 非数字 / 日历不合法 / 别的后缀
        assert_eq!(parse_snapshot_file_name("pre-restore-2026091-120000.db"), None);
        assert_eq!(parse_snapshot_file_name("pre-restore-2026ab18-120000.db"), None);
        assert_eq!(parse_snapshot_file_name("pre-restore-20261332-120000.db"), None);
        assert_eq!(parse_snapshot_file_name("pre-restore-20260918-120000.db.bak"), None);
    }

    #[test]
    fn same_path_detects_self_restore_via_normalization() {
        let (dir, _data_dir, db_path) = fixture();
        // 同一文件：直接路径 vs 带中间点的路径 vs 大小写不同（Windows）
        assert!(same_path(&db_path, &db_path));
        let dotted = dir.path().join("data").join(".").join(crate::db::DB_FILE_NAME);
        assert!(same_path(&db_path, &dotted), "canonicalize 应消解中间点");
        // 不存在的路径：退化字符串比较
        assert!(same_path(Path::new("D:/x/y.db"), Path::new("d:/X/y.db")));
        assert!(!same_path(Path::new("D:/x/y.db"), Path::new("D:/x/z.db")));
        // 不同文件：false
        let other = dir.path().join("other.db");
        std::fs::write(&other, b"x").unwrap();
        assert!(!same_path(&db_path, &other));
    }

    // ── 步骤 1+2：preview 校验链与摘要 ──────────────────────────────────

    #[test]
    fn preview_good_backup_returns_summary_and_cleans_staging() {
        let (_dir, data_dir, db_path) = fixture();
        // 备份：2 窝 3 记录 + 旧布局的库内备份目录设置
        let backup = dir_path(&_dir).join("ant-feeding-log-backup-20260910-080000.db");
        make_db(&backup, "备份窝", 3);
        {
            let conn = crate::db::open_and_migrate(&backup).unwrap();
            conn.execute(
                "INSERT INTO settings (key, value) VALUES ('backup_dir', 'D:/old-bk')",
                [],
            )
            .unwrap();
        }

        let summary = run_preview(&backup, &db_path, &data_dir, STAMP).unwrap();

        assert_eq!(summary.backup_date.as_deref(), Some("2026-09-10"), "备份日期取文件名时间戳");
        assert_eq!(summary.colony_count, 1, "备份库自建的窝数");
        assert_eq!(summary.log_count, 3);
        assert_eq!(summary.backup_dir_in_backup.as_deref(), Some("D:/old-bk"));
        assert!(no_staging_left(&data_dir), "preview 不留 staging");
        // 当前库零改动
        let conn = crate::db::open_and_migrate(&db_path).unwrap();
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM colony", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1, "preview 绝不碰当前库");
    }

    #[test]
    fn preview_backup_without_filename_stamp_falls_back_to_latest_record() {
        let (_dir, data_dir, db_path) = fixture();
        let backup = dir_path(&_dir).join("my-manual-copy.db");
        make_db(&backup, "备份窝", 2);

        let summary = run_preview(&backup, &db_path, &data_dir, STAMP).unwrap();

        assert_eq!(summary.backup_date.as_deref(), Some("2026-09-02"), "无文件名时间戳 → 库内最新记录日期");
    }

    #[test]
    fn preview_rejects_garbage_empty_and_self() {
        let (_dir, data_dir, db_path) = fixture();
        let before = db_bytes(&db_path);

        // 非 SQLite 垃圾文件
        let garbage = dir_path(&_dir).join("garbage.db");
        std::fs::write(&garbage, b"this is not a sqlite database at all........").unwrap();
        let err = run_preview(&garbage, &db_path, &data_dir, STAMP).unwrap_err();
        assert!(err.contains("这不是本应用的备份"), "实际：{err}");

        // 0 字节
        let empty = dir_path(&_dir).join("empty.db");
        std::fs::write(&empty, b"").unwrap();
        let err = run_preview(&empty, &db_path, &data_dir, STAMP).unwrap_err();
        assert!(err.contains("这不是本应用的备份"), "实际：{err}");

        // 当前活动库自身
        let err = run_preview(&db_path, &db_path, &data_dir, STAMP).unwrap_err();
        assert!(err.contains("当前正在使用的库自身"), "实际：{err}");

        assert!(no_staging_left(&data_dir), "失败路径 staging 已清理");
        assert_eq!(db_bytes(&db_path), before, "当前库字节级零改动");
    }

    #[test]
    fn preview_rejects_corrupted_sqlite() {
        let (_dir, data_dir, db_path) = fixture();
        let before = db_bytes(&db_path);
        // 有效 SQLite 头 + 后续垃圾：可被识别为 SQLite 但页损坏
        let corrupt = dir_path(&_dir).join("corrupt.db");
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"SQLite format 3\0");
        bytes.extend_from_slice(&[0u8; 4096]);
        bytes.extend_from_slice(b"\xFF\xFE\xFD\xFC garbage tail");
        std::fs::write(&corrupt, &bytes).unwrap();

        let err = run_preview(&corrupt, &db_path, &data_dir, STAMP).unwrap_err();
        assert!(
            err.contains("备份文件已损坏") || err.contains("这不是本应用的备份"),
            "损坏文件必须被拒绝，实际：{err}"
        );
        assert!(no_staging_left(&data_dir));
        assert_eq!(db_bytes(&db_path), before, "当前库字节级零改动");
    }

    #[test]
    fn preview_rejects_future_version_backup() {
        let (_dir, data_dir, db_path) = fixture();
        let before = db_bytes(&db_path);
        let future = dir_path(&_dir).join("ant-feeding-log-backup-20260911-080000.db");
        make_db(&future, "未来窝", 1);
        {
            let conn = crate::db::open_and_migrate(&future).unwrap();
            conn.pragma_update(None, "user_version", crate::db::SCHEMA_VERSION + 1).unwrap();
        }

        let err = run_preview(&future, &db_path, &data_dir, STAMP).unwrap_err();
        assert!(err.contains("未来版本的备份"), "实际：{err}");
        assert!(err.contains("请先升级应用"), "实际：{err}");
        assert!(no_staging_left(&data_dir));
        assert_eq!(db_bytes(&db_path), before, "当前库字节级零改动");
        // 未来版本标记原样保留（绝不降级）；裸连接读版本——open_and_migrate 对
        // v7 库本就会拒绝（那正是被测行为），不能用来做断言的前置
        let conn = rusqlite::Connection::open(&future).unwrap();
        assert_eq!(
            crate::db::schema_version_of(&conn).unwrap(),
            crate::db::SCHEMA_VERSION + 1
        );
    }

    #[test]
    fn preview_upgrades_old_schema_backup_in_staging() {
        let (_dir, data_dir, db_path) = fixture();
        // 真实 v1 库：V1 schema + 预置 + 1 窝 1 记录，版本停在 1
        let old = dir_path(&_dir).join("ant-feeding-log-backup-20260909-080000.db");
        {
            let conn = rusqlite::Connection::open(&old).unwrap();
            conn.execute_batch(crate::db::V1_SCHEMA_SQL).unwrap();
            crate::db::seed_v1_presets(&conn).unwrap();
            conn.execute_batch(
                r#"
                INSERT INTO colony (name, start_date) VALUES ('v1老窝', '2025-01-01');
                INSERT INTO care_log (colony_id, action_id, occurred_at, note, created_at)
                    VALUES (1, 1, '2025-06-01 08:00:00', '', '2025-06-01 08:00:00');
                "#,
            )
            .unwrap();
            conn.pragma_update(None, "user_version", 1).unwrap();
        }

        let summary = run_preview(&old, &db_path, &data_dir, STAMP).unwrap();

        assert_eq!(summary.colony_count, 1, "旧库数据在升级后原样");
        assert_eq!(summary.log_count, 1);
        assert_eq!(summary.backup_date.as_deref(), Some("2026-09-09"));
        assert!(no_staging_left(&data_dir));
    }

    #[test]
    fn preview_of_empty_backup_exposes_zero_counts() {
        // D6 最后防线：选错文件（无关 SQLite 库 → user_version 0 会走迁移）时
        // 摘要以 0 窝 0 条暴露异常，而非静默放行
        let (_dir, data_dir, db_path) = fixture();
        let backup = dir_path(&_dir).join("ant-feeding-log-backup-20260912-080000.db");
        make_db(&backup, "空库占位", 0);
        // 删掉唯一一窝：0 窝 0 条（外键下先删记录）
        {
            let conn = crate::db::open_and_migrate(&backup).unwrap();
            conn.execute("DELETE FROM care_log", []).unwrap();
            conn.execute("DELETE FROM colony", []).unwrap();
        }

        let summary = run_preview(&backup, &db_path, &data_dir, STAMP).unwrap();
        assert_eq!(summary.colony_count, 0, "0 窝要在摘要里暴露");
        assert_eq!(summary.log_count, 0, "0 条要在摘要里暴露");
    }

    // ── 步骤 3：快照保留 ─────────────────────────────────────────────────

    #[test]
    fn retain_snapshots_keeps_newest_three_never_touches_foreign() {
        let (_dir, data_dir, _db_path) = fixture();
        for name in [
            snapshot_file_name("20250101-000000"), // 最旧 → 删
            snapshot_file_name("20250103-000000"), // 留
            snapshot_file_name("20250104-120000"), // 留
            snapshot_file_name("20250102-235959"), // 留（4 份 keep 3，只删 1 份）
            "pre-restore-20250105.db".to_string(), // 手动命名：绝不碰
            "my-notes.txt".to_string(),            // 无关文件：绝不碰
        ] {
            std::fs::write(data_dir.join(&name), "x").unwrap();
        }
        // 同名目录（异常占用）：绝不触碰
        std::fs::create_dir_all(data_dir.join(snapshot_file_name("20240101-000000"))).unwrap();

        retain_snapshots(&data_dir, SNAPSHOT_KEEP).unwrap();

        let names = list_names(&data_dir);
        assert!(!names.contains(&snapshot_file_name("20250101-000000")), "最旧被淘汰");
        assert!(names.contains(&snapshot_file_name("20250102-235959")), "未超限的留下");
        assert!(names.contains(&snapshot_file_name("20250103-000000")));
        assert!(names.contains(&snapshot_file_name("20250104-120000")));
        assert!(names.contains(&"pre-restore-20250105.db".to_string()), "手动命名幸存");
        assert!(names.contains(&"my-notes.txt".to_string()), "无关文件幸存");
        assert!(data_dir.join(snapshot_file_name("20240101-000000")).is_dir(), "目录绝不触碰");
    }

    #[test]
    fn retain_snapshots_fourth_restore_deletes_oldest() {
        // 票面验收：第 4 次恢复时最旧的快照被清理（保留 3 份）
        let (_dir, data_dir, _db_path) = fixture();
        for name in [
            snapshot_file_name("20250101-000000"), // 第 1 次的快照，最旧
            snapshot_file_name("20250102-000000"), // 第 2 次
            snapshot_file_name("20250103-000000"), // 第 3 次
        ] {
            std::fs::write(data_dir.join(&name), "x").unwrap();
        }
        // 第 4 次恢复的快照刚落下（保留淘汰随后执行）
        std::fs::write(data_dir.join(snapshot_file_name("20250104-000000")), "x").unwrap();

        retain_snapshots(&data_dir, SNAPSHOT_KEEP).unwrap();

        let names = list_names(&data_dir);
        assert_eq!(names.iter().filter(|n| n.starts_with(SNAPSHOT_PREFIX)).count(), 3);
        assert!(!names.contains(&snapshot_file_name("20250101-000000")), "第 4 次时最旧被清理");
    }

    // ── 步骤 4 + 全流程：apply ───────────────────────────────────────────

    #[test]
    fn apply_success_replaces_db_snapshots_and_keeps_config() {
        let (_dir, data_dir, db_path) = fixture();
        // 当前库：1 窝 + 1 记录 + 1 条提醒台账（恢复后随库回滚消失）
        {
            let conn = crate::db::open_and_migrate(&db_path).unwrap();
            conn.execute(
                "INSERT INTO reminder_ledger (colony_id, kind, action_id, base_date, sent_at)
                 VALUES (1, 'overdue', 1, '2026-09-15', '2026-09-15 08:00:00')",
                [],
            )
            .unwrap();
        }
        // D1：恢复不动备份配置文件
        let config_path = data_dir.join(crate::backup_config::BACKUP_CONFIG_FILE);
        std::fs::write(&config_path, r#"{"enabled":false,"keep_count":7}"#).unwrap();
        let config_before = db_bytes(&config_path);
        // 备份来源：1 窝 2 记录，无台账
        let source = dir_path(&_dir).join("ant-feeding-log-backup-20260910-080000.db");
        make_db(&source, "备份窝", 2);

        let lock = live_lock(&db_path);
        let outcome = run_apply(
            &source,
            &db_path,
            &data_dir,
            STAMP,
            || lock.lock().map_err(|e| e.to_string()),
            |p| crate::db::open_and_migrate(p).map_err(|e| e.to_string()),
        )
        .unwrap();

        assert_eq!(outcome, ApplyOutcome::Done);
        // 当前库 = 备份时刻
        let conn = crate::db::open_and_migrate(&db_path).unwrap();
        let name: String = conn.query_row("SELECT name FROM colony", [], |r| r.get(0)).unwrap();
        assert_eq!(name, "备份窝", "整库替换生效");
        let logs: i64 = conn.query_row("SELECT COUNT(*) FROM care_log", [], |r| r.get(0)).unwrap();
        assert_eq!(logs, 2);
        let ledger: i64 = conn
            .query_row("SELECT COUNT(*) FROM reminder_ledger", [], |r| r.get(0))
            .unwrap();
        // Further Notes 落账：台账回滚后的重发窗口是 Q14 已接受的取舍，留意实际体验
        assert_eq!(ledger, 0, "提醒台账随库回滚");
        drop(conn);
        // 锁内连接也已指向新库
        let name: String = lock
            .lock()
            .unwrap()
            .query_row("SELECT name FROM colony", [], |r| r.get(0))
            .unwrap();
        assert_eq!(name, "备份窝", "运行态连接已重开到新库");
        // 快照产出且可重开（含恢复前的现窝）
        let snaps: Vec<String> = list_names(&data_dir)
            .into_iter()
            .filter(|n| n.starts_with(SNAPSHOT_PREFIX))
            .collect();
        assert_eq!(snaps.len(), 1, "实际：{snaps:?}");
        let snap_conn = crate::db::open_and_migrate(&data_dir.join(&snaps[0])).unwrap();
        let name: String = snap_conn.query_row("SELECT name FROM colony", [], |r| r.get(0)).unwrap();
        assert_eq!(name, "现窝", "快照保存的是恢复前的当前库");
        drop(snap_conn);
        // staging 与侧车清理干净（侧车 = `<库文件>.new`，不是扩展名替换）
        assert!(no_staging_left(&data_dir));
        let sidecar = PathBuf::from(format!("{}.new", db_path.display()));
        assert!(!sidecar.exists(), ".new 侧车不残留");
        // D1：备份配置文件字节级不变
        assert_eq!(db_bytes(&config_path), config_before, "备份配置不随恢复动（D1）");
    }

    #[test]
    fn apply_step2_failure_leaves_current_db_byte_identical() {
        let (_dir, data_dir, db_path) = fixture();
        let before = db_bytes(&db_path);
        let garbage = dir_path(&_dir).join("ant-feeding-log-backup-20260910-080000.db");
        std::fs::write(&garbage, b"not a database").unwrap();
        let lock = live_lock(&db_path);

        let err = run_apply(
            &garbage,
            &db_path,
            &data_dir,
            STAMP,
            || lock.lock().map_err(|e| e.to_string()),
            |p| crate::db::open_and_migrate(p).map_err(|e| e.to_string()),
        )
        .unwrap_err();

        assert!(err.contains("这不是本应用的备份"), "实际：{err}");
        assert_eq!(db_bytes(&db_path), before, "当前库字节级零改动");
        assert!(no_staging_left(&data_dir), "staging 已清理");
        // 未到步骤 3：不产生快照
        assert!(
            !list_names(&data_dir).iter().any(|n| n.starts_with(SNAPSHOT_PREFIX)),
            "校验失败不应产生快照"
        );
    }

    #[test]
    fn apply_inlock_gate_rejection_aborts_before_any_write() {
        // 评审 R1 TOCTOU 锚点（票 04 版）：生产 take_lock 闭包在拿到锁之后立即
        // 复查更新禁写位（同 with_conn / daily_tick 段 3 纪律）——顶层检查与拿锁
        // 之间的窗口里用户可确认更新安装置位，置位后拿到的锁必须放弃执行。
        // 与 updater.rs 的时序先例同款：置位前放行 → 持锁内置位 → 锁内复查拒绝；
        // 禁写位用注入的本地位（不触碰 updater 全局，并行安全），时序等价。
        let (_dir, data_dir, db_path) = fixture();
        let before = db_bytes(&db_path);
        let source = dir_path(&_dir).join("ant-feeding-log-backup-20260910-080000.db");
        make_db(&source, "备份窝", 1);
        let lock = live_lock(&db_path);

        // 1. 在途恢复到达时禁写位尚未置位（对应生产：顶层检查已放行）
        let gate = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        assert!(!gate.load(std::sync::atomic::Ordering::SeqCst), "到达时禁写位未置位");
        // 2. 持锁段内置位（模拟 on_before_exit_snapshot 的锁内置位）
        gate.store(true, std::sync::atomic::Ordering::SeqCst);

        // 3. 恢复拿锁 → 锁内复查必须拒绝：整个协议放弃（未到校验/快照/替换）
        let gate2 = gate.clone();
        let err = run_apply(
            &source,
            &db_path,
            &data_dir,
            STAMP,
            || {
                let guard = lock.lock().map_err(|e| e.to_string())?;
                if gate2.load(std::sync::atomic::Ordering::SeqCst) {
                    return Err("更新安装禁写窗口，暂不能恢复".to_string());
                }
                Ok(guard)
            },
            |p| crate::db::open_and_migrate(p).map_err(|e| e.to_string()),
        )
        .unwrap_err();

        assert!(err.contains("禁写"), "实际：{err}");
        assert_eq!(db_bytes(&db_path), before, "当前库字节级零改动");
        assert!(no_staging_left(&data_dir), "staging 已清理");
        assert!(
            !list_names(&data_dir).iter().any(|n| n.starts_with(SNAPSHOT_PREFIX)),
            "锁内复查被拒不得产生快照"
        );
        // 4. 复位后（模拟安装窗口结束）同一条路放行
        gate.store(false, std::sync::atomic::Ordering::SeqCst);
        let outcome = run_apply(
            &source,
            &db_path,
            &data_dir,
            STAMP,
            || {
                let guard = lock.lock().map_err(|e| e.to_string())?;
                if gate.load(std::sync::atomic::Ordering::SeqCst) {
                    return Err("更新安装禁写窗口，暂不能恢复".to_string());
                }
                Ok(guard)
            },
            |p| crate::db::open_and_migrate(p).map_err(|e| e.to_string()),
        )
        .unwrap();
        assert_eq!(outcome, ApplyOutcome::Done, "禁写位复位后恢复照常执行");
    }

    #[test]
    fn apply_swap_double_failure_error_carries_restart_guidance() {
        // 替换失败（.new 被占）且重开原库也失败（注入恒败）：应用已进入占位
        // 连接态，业务命令会持续报错直到重启——错误信息必须带重启指引
        let (_dir, data_dir, db_path) = fixture();
        let before = db_bytes(&db_path);
        let source = dir_path(&_dir).join("ant-feeding-log-backup-20260910-080000.db");
        make_db(&source, "备份窝", 1);
        let new_target = PathBuf::from(format!("{}.new", db_path.display()));
        std::fs::create_dir_all(&new_target).unwrap();
        let lock = live_lock(&db_path);

        let err = run_apply(
            &source,
            &db_path,
            &data_dir,
            STAMP,
            || lock.lock().map_err(|e| e.to_string()),
            |_p| Err("重开失败（注入）".to_string()),
        )
        .unwrap_err();

        assert!(err.contains("请重启应用"), "双失败必须给重启指引，实际：{err}");
        assert_eq!(db_bytes(&db_path), before, "原库文件未被破坏");
        assert!(no_staging_left(&data_dir));
    }

    #[test]
    fn apply_step3_snapshot_failure_leaves_current_db_byte_identical() {
        let (_dir, data_dir, db_path) = fixture();
        let before = db_bytes(&db_path);
        let source = dir_path(&_dir).join("ant-feeding-log-backup-20260910-080000.db");
        make_db(&source, "备份窝", 1);
        // 注入快照失败：同名的「目录」占住快照落点（fs::copy 进目录必失败）
        let snap_target = data_dir.join(snapshot_file_name(STAMP));
        std::fs::create_dir_all(&snap_target).unwrap();
        let lock = live_lock(&db_path);

        let err = run_apply(
            &source,
            &db_path,
            &data_dir,
            STAMP,
            || lock.lock().map_err(|e| e.to_string()),
            |p| crate::db::open_and_migrate(p).map_err(|e| e.to_string()),
        )
        .unwrap_err();

        assert!(!err.is_empty(), "快照失败要报具体原因");
        assert_eq!(db_bytes(&db_path), before, "当前库字节级零改动");
        assert!(no_staging_left(&data_dir), "staging 已清理");
        assert!(snap_target.is_dir(), "占位目录（非自家产物）绝不误删");
    }

    #[test]
    fn apply_swap_write_failure_leaves_current_db_byte_identical() {
        let (_dir, data_dir, db_path) = fixture();
        let before = db_bytes(&db_path);
        let source = dir_path(&_dir).join("ant-feeding-log-backup-20260910-080000.db");
        make_db(&source, "备份窝", 1);
        // 注入步骤 4 拷贝失败：<库>.new 侧车路径被目录占住
        // Further Notes 落账：Windows「先写新文件+原子改名」的占用/权限边界由
        // 本测试与 apply_swap_double_failure_* 注入覆盖；杀毒/索引器的瞬态占用
        // 由重开连接的短退避重试兜底。
        let new_target = PathBuf::from(format!("{}.new", db_path.display()));
        std::fs::create_dir_all(&new_target).unwrap();
        let lock = live_lock(&db_path);

        let err = run_apply(
            &source,
            &db_path,
            &data_dir,
            STAMP,
            || lock.lock().map_err(|e| e.to_string()),
            |p| crate::db::open_and_migrate(p).map_err(|e| e.to_string()),
        )
        .unwrap_err();

        assert!(err.contains("写入新库文件失败"), "实际：{err}");
        assert_eq!(db_bytes(&db_path), before, "原库文件未被破坏（先写后改名）");
        assert!(no_staging_left(&data_dir), "staging 已清理");
        // 锁内连接恢复可用（原库重开回锁内）
        let n: i64 = lock
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM colony", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1, "原库连接已恢复");
    }

    #[test]
    fn apply_reconnect_retries_then_succeeds() {
        let (_dir, data_dir, db_path) = fixture();
        let source = dir_path(&_dir).join("ant-feeding-log-backup-20260910-080000.db");
        make_db(&source, "备份窝", 1);
        let lock = live_lock(&db_path);

        let attempts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let attempts2 = attempts.clone();
        let outcome = run_apply(
            &source,
            &db_path,
            &data_dir,
            STAMP,
            || lock.lock().map_err(|e| e.to_string()),
            move |p| {
                let n = attempts2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if n < 2 {
                    Err(format!("第 {n} 次重开失败（注入）"))
                } else {
                    crate::db::open_and_migrate(p).map_err(|e| e.to_string())
                }
            },
        )
        .unwrap();

        assert_eq!(outcome, ApplyOutcome::Done, "重试后成功");
        assert!(attempts.load(std::sync::atomic::Ordering::SeqCst) >= 3, "确实重试过");
        let name: String = lock
            .lock()
            .unwrap()
            .query_row("SELECT name FROM colony", [], |r| r.get(0))
            .unwrap();
        assert_eq!(name, "备份窝", "重试成功的连接指向新库");
    }

    #[test]
    fn apply_reconnect_always_failing_reports_needs_restart_with_new_file_in_place() {
        let (_dir, data_dir, db_path) = fixture();
        let source = dir_path(&_dir).join("ant-feeding-log-backup-20260910-080000.db");
        make_db(&source, "备份窝", 1);
        let lock = live_lock(&db_path);

        let outcome = run_apply(
            &source,
            &db_path,
            &data_dir,
            STAMP,
            || lock.lock().map_err(|e| e.to_string()),
            |_p| Err("重开失败（注入：杀毒占用）".to_string()),
        )
        .unwrap();

        assert_eq!(outcome, ApplyOutcome::DoneNeedsRestart, "重试 3 次仍失败 → 提示重启");
        // 新库文件已就位且完整（手动重开可见备份内容）
        let conn = crate::db::open_and_migrate(&db_path).unwrap();
        let name: String = conn.query_row("SELECT name FROM colony", [], |r| r.get(0)).unwrap();
        assert_eq!(name, "备份窝", "新库文件保持就位");
        drop(conn);
        // 锁内是占位连接：业务查询当场报错（不静默空读）
        let guard = lock.lock().unwrap();
        assert!(
            guard.query_row("SELECT COUNT(*) FROM colony", [], |r| r.get::<_, i64>(0)).is_err(),
            "占位连接无业务表——查询必须报错"
        );
        drop(guard);
        assert!(no_staging_left(&data_dir));
    }

    // ── 前端契约：字段 snake_case ────────────────────────────────────────
    #[test]
    fn summary_and_outcome_serialize_snake_case_for_frontend() {
        let s = RestoreSummary {
            backup_date: Some("2026-09-10".into()),
            colony_count: 2,
            log_count: 3,
            backup_dir_in_backup: Some("D:/old-bk".into()),
        };
        let json = serde_json::to_string(&s).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        for key in ["backup_date", "colony_count", "log_count", "backup_dir_in_backup"] {
            assert!(v.get(key).is_some(), "缺 {key}，实际：{json}");
        }
        assert_eq!(v["backup_dir_in_backup"], "D:/old-bk");

        assert_eq!(
            serde_json::to_string(&ApplyOutcome::Done).unwrap(),
            r#""done""#
        );
        assert_eq!(
            serde_json::to_string(&ApplyOutcome::DoneNeedsRestart).unwrap(),
            r#""done_needs_restart""#
        );
    }
}
