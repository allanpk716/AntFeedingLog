//! 数据版本计数与实例 epoch（webui-checkin 票 06，规格 Implementation Decisions C）。
//!
//! 三件事：
//! - **持久数据版本计数器**：`data_meta` 表 `data_version` 键，u64；每次业务写
//!   命令成功后 +1（统一咽喉在写后钩子链，见 lib.rs `trigger_after_write` /
//!   webui_server::bump_and_publish——桌面与 HTTP 写命令一条路，绝不在 21+17
//!   个命令里散写）。单连接互斥下与写命令串行递增，等价「写事务内 +1」。
//!   跨重启单调：计数器随库文件持久化；跨恢复单调由 [`bump_floor`]
//!   （恢复完成把新库计数抬到不低于本实例已广播值）保证。
//! - **实例 epoch**：安装 id（数据目录 `install-id` 文件，首启生成、持久，
//!   整库恢复不触碰它——epoch 不随恢复变）+ 本次进程启动毫秒，拼成字符串。
//!   SSE 首帧携带；客户端见 epoch 变化即「电脑重启过」→ 无条件重拉。
//! - 读取缺键回 0，绝不毒死调用方。

use std::path::Path;
use std::sync::OnceLock;

use rusqlite::Connection;

use crate::data_meta;

/// data_meta 里版本计数器的键。
pub const DATA_VERSION_KEY: &str = "data_version";

/// 安装 id 文件名（数据目录下；与 webui-config.json / backup-config.json 同层，
/// 库外文件——整库恢复不回滚安装身份）。
pub const INSTALL_ID_FILE: &str = "install-id";

/// 安装 id 长度（128 bit 随机 → 32 hex 字符，与访问凭证/票据同规格）。
const INSTALL_ID_BYTES: usize = 16;

// ── 版本计数器 ──────────────────────────────────────────────────────────────

/// 读当前数据版本；缺键或脏值回 0（计数器是提示性数据，不值得报错）。
pub fn get(conn: &Connection) -> u64 {
    data_meta::get(conn, DATA_VERSION_KEY)
        .ok()
        .flatten()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(0)
}

/// 版本 +1，返回新值。
pub fn bump(conn: &Connection) -> Result<u64, String> {
    let next = get(conn)
        .checked_add(1)
        .ok_or_else(|| "数据版本计数溢出（u64 上限）".to_string())?;
    data_meta::set(conn, DATA_VERSION_KEY, &next.to_string())?;
    Ok(next)
}

/// 版本抬到「max(当前值, floor) + 1」，返回新值。恢复完成专用：换入的旧备份
/// 计数器可能比本实例已广播过的版本还小，用已广播最大值兜底，保证已连客户端
/// 对账必判「落后」→ 无条件刷新（版本号跨恢复单调，规格 C）。
pub fn bump_floor(conn: &Connection, floor: u64) -> Result<u64, String> {
    let next = get(conn)
        .max(floor)
        .checked_add(1)
        .ok_or_else(|| "数据版本计数溢出（u64 上限）".to_string())?;
    data_meta::set(conn, DATA_VERSION_KEY, &next.to_string())?;
    Ok(next)
}

// ── 实例 epoch ──────────────────────────────────────────────────────────────

/// 本进程「启动时刻」毫秒（unix 时钟；首次调用即记，调用点在应用 setup 最早段，
/// 与真实进程启动的误差对「实例身份」用途无影响——它只需要本进程内稳定）。
pub fn process_start_millis() -> u64 {
    static START: OnceLock<u64> = OnceLock::new();
    *START.get_or_init(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    })
}

/// epoch = `<安装id>-<进程启动毫秒>`。安装 id 相同（同一台安装）而毫秒不同
/// （重启过）→ epoch 变 → 客户端无条件重拉。
pub fn epoch_of(install_id: &str, start_millis: u64) -> String {
    format!("{install_id}-{start_millis}")
}

/// 读安装 id；没有就生成（≥128bit 随机）并落盘。文件读失败/目录不可写时退回
/// 「本次进程内存随机值」并打日志——SSE 与刷新语义照常工作，只是该次安装身份
/// 不跨重启（退化可接受，绝不挡启动）。
pub fn load_or_create_install_id(data_dir: &Path) -> String {
    let file = data_dir.join(INSTALL_ID_FILE);
    if let Ok(existing) = std::fs::read_to_string(&file) {
        let trimmed = existing.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    let mut bytes = [0u8; INSTALL_ID_BYTES];
    if let Err(e) = getrandom::fill(&mut bytes) {
        crate::applog::log_error(&format!("生成安装 id 失败（系统随机源不可用）: {e}"));
        return format!("ephemeral-{}", std::process::id());
    }
    let id: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    let write = (|| -> std::io::Result<()> {
        std::fs::create_dir_all(data_dir)?;
        std::fs::write(&file, &id)?;
        Ok(())
    })();
    if let Err(e) = write {
        crate::applog::log_error(&format!(
            "安装 id 落盘失败（{}）: {e}——本次运行用临时身份，重启后会变",
            file.display()
        ));
    }
    id
}

// ── 测试：只测外部行为（spec「Testing Decisions」）────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn mem_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("内存库打开失败");
        crate::db::migrate(&conn).expect("迁移失败");
        conn
    }

    // ── 版本计数 ──

    #[test]
    fn version_starts_at_zero_when_key_missing() {
        let conn = mem_conn();
        assert_eq!(get(&conn), 0, "缺键回 0，不报错");
    }

    #[test]
    fn dirty_version_value_reads_as_zero() {
        let conn = mem_conn();
        data_meta::set(&conn, DATA_VERSION_KEY, "不是数字").unwrap();
        assert_eq!(get(&conn), 0, "脏值不毒死读侧");
    }

    #[test]
    fn bump_increments_one_by_one() {
        let conn = mem_conn();
        assert_eq!(bump(&conn).unwrap(), 1);
        assert_eq!(bump(&conn).unwrap(), 2);
        assert_eq!(bump(&conn).unwrap(), 3);
        assert_eq!(get(&conn), 3);
    }

    #[test]
    fn version_counter_is_monotonic_across_restart() {
        // 验收「版本计数跨重启单调（持久化测试）」：计数器随库文件持久化，
        // 关连接再开（模拟重启），新连接读到旧值并继续递增。
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join(crate::db::DB_FILE_NAME);
        {
            let conn = crate::db::open_and_migrate(&db_path).unwrap();
            assert_eq!(bump(&conn).unwrap(), 1);
            assert_eq!(bump(&conn).unwrap(), 2);
        }
        let conn = crate::db::open_and_migrate(&db_path).unwrap();
        assert_eq!(get(&conn), 2, "重启后计数不丢");
        assert_eq!(bump(&conn).unwrap(), 3, "重启后继续单调递增");
    }

    #[test]
    fn bump_floor_lifts_restored_counter_above_broadcast_max() {
        // 恢复场景：旧备份换进来计数器只有 3，但本实例已广播到 10——
        // 抬到 11（> 任何已广播值），已连客户端对账必判「落后」。
        let conn = mem_conn();
        assert_eq!(bump_floor(&conn, 10).unwrap(), 11);
        assert_eq!(get(&conn), 11);

        // 新库计数本来就高（恢复到更新的备份）：只 +1，不被 floor 压低
        let conn = mem_conn();
        assert_eq!(bump(&conn).unwrap(), 1);
        assert_eq!(bump(&conn).unwrap(), 2);
        assert_eq!(bump(&conn).unwrap(), 3);
        assert_eq!(bump_floor(&conn, 2).unwrap(), 4, "floor 低于现值时只 +1");
    }

    #[test]
    fn bump_floor_on_fresh_restored_db_starts_from_floor() {
        // 全新库（计数 0）换入 + 已广播到 7 → 8
        let conn = mem_conn();
        assert_eq!(bump_floor(&conn, 7).unwrap(), 8);
    }

    // ── 安装 id 与 epoch ──

    #[test]
    fn install_id_created_once_and_stable() {
        let dir = TempDir::new().unwrap();
        let first = load_or_create_install_id(dir.path());
        assert_eq!(first.len(), INSTALL_ID_BYTES * 2, "128bit → 32 hex 字符");
        assert_eq!(
            load_or_create_install_id(dir.path()),
            first,
            "同一数据目录重复读 = 同一身份"
        );
        // 文件确实落了盘
        let on_disk = std::fs::read_to_string(dir.path().join(INSTALL_ID_FILE)).unwrap();
        assert_eq!(on_disk.trim(), first);
    }

    #[test]
    fn install_id_differs_across_data_dirs() {
        let a = load_or_create_install_id(&TempDir::new().unwrap().path());
        let b = load_or_create_install_id(&TempDir::new().unwrap().path());
        assert_ne!(a, b, "不同安装不同身份");
    }

    #[test]
    fn install_id_creates_missing_data_dir() {
        let dir = TempDir::new().unwrap();
        let nested = dir.path().join("not-yet-created");
        let id = load_or_create_install_id(&nested);
        assert_eq!(id.len(), INSTALL_ID_BYTES * 2);
        assert!(nested.join(INSTALL_ID_FILE).is_file(), "目录不存在则先建");
    }

    #[test]
    fn epoch_changes_on_restart_same_install() {
        // 同一安装 id、不同启动时刻 → epoch 必不同（客户端据此判「电脑重启过」）
        let id = "0123456789abcdef0123456789abcdef";
        let e1 = epoch_of(id, 1_000);
        let e2 = epoch_of(id, 2_000);
        assert_ne!(e1, e2);
        assert_eq!(epoch_of(id, 1_000), "0123456789abcdef0123456789abcdef-1000");
    }

    #[test]
    fn process_start_millis_is_stable_within_process() {
        assert_eq!(process_start_millis(), process_start_millis(), "进程内稳定");
        assert!(process_start_millis() > 0);
    }
}
