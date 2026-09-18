//! 更新核心（票 02）：updater 插件接入 + 每日静默检查 + 手动检查命令。
//!
//! 纯逻辑与 Tauri 解耦，cargo test 直接覆盖（沿 db.rs/settings.rs 惯例）：
//! - [`should_check_today`]：每日节奏——同一天只查一次、跨天重查，重启也不重查
//!   （时间源注入：today 由调用方传入）；
//! - [`to_outcome`]：检查结果三态映射（无更新 / 有新版 / 检查失败）；
//! - [`fold_daily`]：每日路径错误折叠——任何失败（404/网络/超时/解析）一律按
//!   "无更新"记日志静默，绝不外抛失败态；
//! - [`manual_result`]：手动路径出口——检查失败折为 Err（前端一次性展示）；
//! - 每日检查拆成两段纯函数（评审 R1-1：网络段绝不持 DB 锁）：
//!   [`should_run_daily_check`]（持锁段：读记账定该不该查）→ 调用方放锁执行
//!   `checker.check()`（网络段）→ [`finish_daily_check`]（持锁段：记账+折算，
//!   签名只吃已取回的结果值，结构上不存在网络调用）；
//!   检查执行器经 [`UpdateChecker`] trait 注入：生产实现走 tauri-plugin-updater，
//!   测试用假实现，不碰真网络。
//!
//! "上次检查日"复用 settings 表存储（settings.rs 同款 key-value，键
//! [`K_LAST_CHECK_DAY`]），跨启动持久化；手动检查不写这个键（互不干扰）。
//!
//! 配置面（tauri.conf.json，严格 JSON 不带注释，决策记这里）：
//! - `bundle.createUpdaterArtifacts: true`：发布产物带 minisign 签名；main CI
//!   无私钥，构建步用 `--config` 覆盖关闭（见 .github/workflows/ci.yml）；
//! - `plugins.updater.endpoints`：GitHub latest.json 直连，数组留镜像扩展位；
//! - `plugins.updater.pubkey` 当前是占位串（base64 的 "untrusted comment:"），
//!   **票 07** 由用户生成 minisign 密钥对后填真实公钥。公钥只在插件下载/验签
//!   阶段解析（插件源码 v2.11.0：verify_signature），check 阶段不受影响；占位
//!   期间尚无 release 时检查 404 → 每日按无更新静默、手动返回 Err，应用启动
//!   不受影响——票 07 会在首发前填真实公钥。
//!
//! 生产检查器、每日定时线程与通知发送是薄封装（lib.rs 调
//! `spawn_daily_checker`，系统行为不进单测）。
//!
//! 升级前快照（票 04）：[`pre_update_snapshot`] 复用 system.rs 既有拷贝（评审附录
//! 规则 11：journal_mode=DELETE 短暂拿锁+拷贝单 .db 即完整），文件名
//! `backups/pre-update-v<版本>-<yyyymmdd-hhmmss>.db` 同版本并存不覆盖；禁写闸门
//! [`gate_write`] + 进程级 AtomicBool 标志，快照落成后到进程退出前拒绝一切写请求
//! （拦截点：lib.rs `with_conn` 统一入口 / [`daily_tick`] /
//! reminder::check_and_notify，取舍记在各处注释）。挂点在 [`build_updater`] 的
//! `on_before_exit`（插件 2.11.0 在 Windows 安装器启动前、进程退出前回调，闭包
//! 签名 `Fn()` 无参）；快照失败只记日志，绝不阻塞安装流程。
//!
//! 确认流与失败兜底（票 05）：用户确认升级后 [`run_confirm_flow`] 编排
//! 再次检查 → 写 pending 标记 → 下载 → install，三步经 [`ConfirmSteps`] 注入
//!（生产 [`PluginConfirmSteps`] 薄封装，测试 FakeSteps）；标记是数据目录小 JSON
//!（[`PENDING_MARKER_FILE`]，不进数据库），**先标记后下载**顺序硬性。Windows 下
//! 安装成功即进程退出，标记保留为"想升到 vX"的跨启动凭据；下次启动
//! [`startup_judgment`] 对比标记目标与当前运行版本（语义化比较 [`version_cmp`]），
//! 三态判定后清标记并暂存可查状态（[`current_update_state`]，get_update_state
//! command 消费）。安装失败（install 返回 Err、进程存活）由失败臂复位禁写标志
//!（评审 M-1 契约）+ 暂存"升级未完成"，UI 据此给重试/手动下载引导。

use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

/// settings 表键名：每日更新检查最近一次执行的日期（ISO 日期串）。
pub const K_LAST_CHECK_DAY: &str = "update_last_check_day";

/// 发布页地址（票 06 手动下载出口：升级未完成引导 / 安装失败的兜底）。
/// 与 tauri.conf.json endpoints 同仓库；`releases/latest` 恒指最新发布，
/// 不随版本号变，无需在发版时改这里。
pub const RELEASES_PAGE_URL: &str = "https://github.com/allanpk716/AntFeedingLog/releases/latest";

/// 远端有新版时的最小信息：版本号 + release 说明。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UpdateInfo {
    pub version: String,
    pub notes: Option<String>,
}

/// 检查执行器抽象（seam）：Ok(None)=远端无新版；Ok(Some)=有新版；
/// Err=检查失败（网络/404/超时/解析等）。
pub trait UpdateChecker {
    fn check(&self) -> Result<Option<UpdateInfo>, String>;
}

/// 检查结果三态（票面）。`CheckFailed` 仅手动路径对外可见（每日路径已折叠）。
/// serde 形态是前端契约（票 06 设置页消费）：`{"status":"up_to_date"}` 等。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CheckOutcome {
    UpToDate,
    UpdateAvailable {
        version: String,
        notes: Option<String>,
    },
    CheckFailed {
        message: String,
    },
}

/// 一轮每日检查的对外结果：`Skipped`=今天已查过；`NoUpdate`=查了但无新版
/// （含失败静默）；`UpdateAvailable`=查到新版，调用方发系统通知。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DailyOutcome {
    Skipped,
    NoUpdate,
    UpdateAvailable {
        version: String,
        notes: Option<String>,
    },
}

// ── 核心：每日节奏 ────────────────────────────────────────────────────────

/// 该不该在今天做每日检查：今天还没记过 → 查（含从没查过、记的是昨天或脏值）；
/// 今天已查过 → 跳过（重启也不重查）。两侧都是 today_iso 同源格式，直接比串。
pub fn should_check_today(last_check_day: Option<&str>, today: &str) -> bool {
    last_check_day != Some(today)
}

/// 三态映射：检查器原始结果 → 对外三态。
pub fn to_outcome(result: Result<Option<UpdateInfo>, String>) -> CheckOutcome {
    match result {
        Ok(None) => CheckOutcome::UpToDate,
        Ok(Some(info)) => CheckOutcome::UpdateAvailable {
            version: info.version,
            notes: info.notes,
        },
        Err(message) => CheckOutcome::CheckFailed { message },
    }
}

/// 每日路径错误折叠：任何失败一律按"无更新"处理并记日志，绝不外抛 `CheckFailed`
/// （约定：每日静默检查绝不因失败发通知打扰）。
pub fn fold_daily(outcome: CheckOutcome) -> CheckOutcome {
    match outcome {
        CheckOutcome::CheckFailed { message } => {
            eprintln!("[updater] 每日静默检查失败，按无更新处理: {message}");
            CheckOutcome::UpToDate
        }
        other => other,
    }
}

/// 手动路径出口：检查失败折为 Err（前端一次性展示），成功两态原样通过。
pub fn manual_result(outcome: CheckOutcome) -> Result<CheckOutcome, String> {
    match outcome {
        CheckOutcome::CheckFailed { message } => Err(message),
        other => Ok(other),
    }
}

// ── 核心：升级前快照与禁写窗口（票 04）───────────────────────────────────

/// 快照文件名：`pre-update-v<当前版本>-<yyyymmdd-hhmmss>.db`（票面命名）。
/// 时间戳由调用方注入（可测），版本与 stamp 都按字符串拼接、不做格式裁剪。
pub fn snapshot_file_name(version: &str, stamp: &str) -> String {
    format!("pre-update-v{version}-{stamp}.db")
}

/// 升级前快照：拷贝库文件到 `<数据目录>/backups/`，返回快照完整路径。
/// 复用 [`crate::system::backup_db_file`]（journal_mode=DELETE 下短暂拿锁挡住
/// 并发写后拷贝单 .db 即完整，评审附录规则 11）——不另写拷贝逻辑；拿锁由调用方
/// 负责（与 lib.rs backup_to 同一模式）。文件名带秒级时间戳 → 同版本重试/重装
/// 两次快照并存不覆盖（同一秒内的第二次才会覆盖；挂点每进程至多一次、重装间隔
/// 远大于 1s，不做尾缀去重——取舍记录）。失败返回 Err（源缺失/磁盘满等），
/// 不 panic 不中断升级流程——调用方记日志继续安装。
pub fn pre_update_snapshot(
    db_path: &Path,
    data_dir: &Path,
    version: &str,
    stamp: &str,
) -> Result<std::path::PathBuf, String> {
    let target = data_dir
        .join("backups")
        .join(snapshot_file_name(version, stamp));
    crate::system::backup_db_file(db_path, &target)?;
    Ok(target)
}

/// 禁写闸门（纯逻辑）：`blocked` = true（更新安装窗口）时写请求一律拒绝；
/// false（日常运行）原样放行。
pub fn gate_write(blocked: bool) -> Result<(), String> {
    if blocked {
        Err("应用正在安装更新，数据写入已暂停（升级前数据快照已保存）；若停留此状态请重启应用".into())
    } else {
        Ok(())
    }
}

/// 进程级禁写标志（票 04）：快照完成后置位，直到安装流程结束。
static WRITE_BLOCKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 置位/复位进程级禁写标志。置位后 [`ensure_writable`] 拒绝一切写请求。
/// 复位契约：安装成功 = 进程随即退出（标志随进程消亡）；安装失败（install 返回
/// Err、进程存活）由票 05 的确认流复位，避免应用卡在只读态。
pub fn set_write_blocked(blocked: bool) {
    WRITE_BLOCKED.store(blocked, std::sync::atomic::Ordering::SeqCst);
}

/// 当前是否处于禁写窗口（快照已完成、等待进程退出）。
pub fn is_write_blocked() -> bool {
    WRITE_BLOCKED.load(std::sync::atomic::Ordering::SeqCst)
}

/// 写请求统一入口的闸门：读进程级标志并放行/拒绝。
pub fn ensure_writable() -> Result<(), String> {
    gate_write(is_write_blocked())
}

// ── 核心：pending 标记与启动判定（票 05）───────────────────────────────────
//
// 确认升级后的完整执行链与失败兜底。标记是应用数据目录下的一个小 JSON 文件
//（[`PENDING_MARKER_FILE`]，刻意不进数据库——安装窗口期数据库可能正被迁移/
// 快照，落盘凭据必须与库无关）。生命周期：
// 1. 确认流**先写标记再下载**（顺序硬性：下载失败时标记仍在，启动检测才能
//    发现"想升没升成"）；
// 2. 安装（成功 = 进程随即退出），标记保留——它是"想升到 vX"的跨启动凭据；
// 3. 下次启动 [`startup_judgment`] 对比标记目标版本与当前运行版本：目标 ≤ 当前
//    → 升级成功；目标 > 当前 → 上次升级未完成（已知残余风险：Windows 下安装
//    器 spawn 后应用即退出，中途失败无法回传 UI）。判定后标记即清，结果暂存
//    进程内供 [`current_update_state`]（get_update_state command）查询。
// 标记文件损坏/缺失一律按无标记处理，绝不影响启动。

/// pending 标记文件名（应用数据目录下的小 JSON，不进数据库）。
pub const PENDING_MARKER_FILE: &str = "update-pending.json";

/// pending 标记：想升到的目标版本 + 写入时间戳（yyyymmdd-HHMMSS）。
/// serde 形态落盘即持久化格式；跨启动只读这一份。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PendingMarker {
    pub target_version: String,
    pub stamped_at: String,
}

/// 语义化版本比较（票面硬性：0.2.0 < 0.10.0，禁用字符串比较）：按 '.' 分段逐段
/// 取数字比较，缺段补 0（0.2 与 0.2.0 等价）；段内非数字按字典序兜底（本应用
/// 不发 prerelease，正常只走数字分支）；容忍前导 `v`。
pub fn version_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    fn segs(s: &str) -> Vec<&str> {
        s.trim().trim_start_matches('v').split('.').collect()
    }
    let (a, b) = (segs(a), segs(b));
    for i in 0..a.len().max(b.len()) {
        let av = a.get(i).copied().unwrap_or("0");
        let bv = b.get(i).copied().unwrap_or("0");
        let ord = match (av.parse::<u64>(), bv.parse::<u64>()) {
            (Ok(x), Ok(y)) => x.cmp(&y),
            _ => av.cmp(bv),
        };
        if ord != std::cmp::Ordering::Equal {
            return ord;
        }
    }
    std::cmp::Ordering::Equal
}

/// 写 pending 标记（确认流在下载前调用——顺序硬性）。返回标记文件完整路径。
/// 数据目录缺失则先建（首启即确认升级的极端路径也成立）。
pub fn write_pending_marker(
    data_dir: &Path,
    target_version: &str,
    stamp: &str,
) -> Result<std::path::PathBuf, String> {
    std::fs::create_dir_all(data_dir).map_err(|e| format!("创建数据目录失败: {e}"))?;
    let marker = PendingMarker {
        target_version: target_version.to_string(),
        stamped_at: stamp.to_string(),
    };
    let path = data_dir.join(PENDING_MARKER_FILE);
    let text =
        serde_json::to_string_pretty(&marker).map_err(|e| format!("序列化更新标记失败: {e}"))?;
    std::fs::write(&path, text).map_err(|e| format!("写入更新标记失败: {e}"))?;
    Ok(path)
}

/// 读 pending 标记：缺失一律 None；损坏（不可解析/字段缺失/目标版本为空）按
/// 无标记处理并顺手删掉坏文件（避免每次启动重复报错）；读取失败（权限等）
/// 也按无标记——标记只影响升级提示，绝不挡启动。
pub fn load_pending_marker(data_dir: &Path) -> Option<PendingMarker> {
    let path = data_dir.join(PENDING_MARKER_FILE);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            eprintln!("[updater] 读取 pending 标记失败（按无标记处理）: {e}");
            return None;
        }
    };
    match serde_json::from_str::<PendingMarker>(&text) {
        Ok(marker) if !marker.target_version.trim().is_empty() => Some(marker),
        other => {
            eprintln!("[updater] pending 标记损坏（按无标记处理并清除）: {other:?}");
            let _ = std::fs::remove_file(&path);
            None
        }
    }
}

/// 清除 pending 标记（启动判定后调用）；文件本来就不存在也按成功。
pub fn clear_pending_marker(data_dir: &Path) -> Result<(), String> {
    match std::fs::remove_file(data_dir.join(PENDING_MARKER_FILE)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("清除更新标记失败: {e}")),
    }
}

/// 更新状态（get_update_state 的返回，票 06 前端契约）：无残留 / 上次升级成功
///（可提示"已升级到 vX"）/ 上次升级未完成（含目标版本，供重试/手动下载引导）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum UpdateState {
    Idle,
    LastInstallSucceeded { version: String },
    LastInstallIncomplete { version: String },
}

/// 进程内暂存的最近一次更新状态（启动判定 / 确认流失败路径写入；只活在本进程，
/// 无需跨启动——跨启动凭据是标记文件本身）。
static LAST_UPDATE_STATE: std::sync::Mutex<Option<UpdateState>> = std::sync::Mutex::new(None);

/// 暂存更新状态（锁毒化按原值续用：状态只是提示性数据，不值得 panic）。
fn stage_update_state(state: UpdateState) {
    let mut guard = LAST_UPDATE_STATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    *guard = Some(state);
}

/// 当前更新状态（从未暂存过 = [`UpdateState::Idle`]）。
pub fn current_update_state() -> UpdateState {
    let guard = LAST_UPDATE_STATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    guard.clone().unwrap_or(UpdateState::Idle)
}

/// 启动判定（票面三态纯逻辑）：无标记（含损坏/缺失按无标记口径）→ 正常路径；
/// 标记目标 ≤ 当前版本 → 升级成功；目标 > 当前版本 → 上次升级未完成。
/// 比较走 [`version_cmp`] 语义化比较（0.2.0 < 0.10.0）。
pub fn judge_startup(pending: Option<&PendingMarker>, current_version: &str) -> UpdateState {
    match pending {
        None => UpdateState::Idle,
        Some(marker) => {
            if version_cmp(&marker.target_version, current_version) == std::cmp::Ordering::Greater {
                UpdateState::LastInstallIncomplete {
                    version: marker.target_version.clone(),
                }
            } else {
                UpdateState::LastInstallSucceeded {
                    version: marker.target_version.clone(),
                }
            }
        }
    }
}

/// 启动时应用判定：读标记 → 判定 → 清标记 → 暂存结果（get_update_state 可查）。
/// 返回判定结果供调用方记日志。判定完即清：标记只服务于"跨启动这一次"判定，
/// 成败结果已转为可查状态；清除失败只记日志（下次启动会再判定一次，幂等）。
pub fn startup_judgment(data_dir: &Path, current_version: &str) -> UpdateState {
    let pending = load_pending_marker(data_dir);
    if pending.is_none() {
        // 无标记（或损坏已被 load 清掉；missing 时 clear 是 no-op）
        let _ = clear_pending_marker(data_dir);
        stage_update_state(UpdateState::Idle);
        return UpdateState::Idle;
    }
    let state = judge_startup(pending.as_ref(), current_version);
    if let Err(e) = clear_pending_marker(data_dir) {
        eprintln!("[updater] 清除 pending 标记失败（下次启动会再判定一次）: {e}");
    }
    stage_update_state(state.clone());
    state
}

// ── 核心：确认流编排（票 05，可注入纯函数层）──────────────────────────────
//
// 流程 = 再次检查拿最新 Update → 写 pending 标记 → 下载 → install。
// 三步经 [`ConfirmSteps`] 注入：生产实现是插件薄封装（[`PluginConfirmSteps`]），
// 测试用 FakeSteps（照 FakeChecker 模式）——编排顺序与失败路径在纯函数层锚定，
// 插件 install 本体不进单测。

/// 确认流的三个外部步骤（seam）。全部同步：生产实现内部用
/// tauri::async_runtime::block_on 桥接插件 async API（只允许在非运行时线程上，
/// 生产经 spawn_blocking 进入，见 lib.rs confirm_and_install）。
pub trait ConfirmSteps {
    /// 再次检查拿最新 Update（确认动作可能距用户看到提示已有时隔，必须复查）。
    /// Err = 检查失败；查无新版也按 Err（无可装之物，流程中止）。
    fn fresh_update(&self) -> Result<UpdateInfo, String>;
    /// 下载新版本安装包（生产实现附带进度事件）。
    fn download(&self) -> Result<(), String>;
    /// 拉起安装器（Windows：快照钩子+置位禁写在插件 install_inner 内触发；
    /// 成功即进程退出不再返回；返回 Err = 进程存活，调用方必须兑现复位契约）。
    fn install(&self) -> Result<(), String>;
}

/// 确认流的对外结果（票 06 前端契约）。下载/安装失败是"状态"不是 Err：
/// 残留物（标记/暂存状态）已就位，前端按 `install_failed` 给重试/手动下载引导；
/// 检查失败/写标记失败则折为 Err 一次性展示（与 manual_check 同口径）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum InstallOutcome {
    InstallStarted { version: String },
    InstallFailed { version: String, message: String },
}

/// 确认流编排（票 05 本体）：
/// 1. 再次检查（失败 → Err，不写标记不下载）；
/// 2. **先写 pending 标记再下载**（顺序硬性：下载失败时标记仍在，启动检测才能
///    发现"想升没升成"；写标记失败同样在下载前中止）；
/// 3. 下载 → install（Windows 成功 = 进程退出不再返回；标记保留作启动判定凭据）；
/// 4. 任一步失败（进程存活）：复位禁写标志（评审 M-1 契约——快照置位后 install
///    返回 Err 时应用不得卡在只读态）、保留标记、暂存"升级未完成"，返回
///    [`InstallOutcome::InstallFailed`]。
pub fn run_confirm_flow<S: ConfirmSteps>(
    data_dir: &Path,
    steps: &S,
) -> Result<InstallOutcome, String> {
    // 1. 再次检查拿最新
    let info = steps.fresh_update()?;

    // 2. 先写标记（下载前落盘凭据；时间戳与快照同款秒级格式）
    let stamp = now_stamp();
    write_pending_marker(data_dir, &info.version, &stamp)?;

    // 3. 下载 → 4. 安装
    match (|| -> Result<(), String> {
        steps.download()?;
        steps.install()
    })() {
        Ok(()) => {
            // Windows 下 install 成功即进程退出；标记保留，交下次启动判定
            Ok(InstallOutcome::InstallStarted {
                version: info.version,
            })
        }
        Err(message) => {
            // 评审 M-1 契约：失败且进程存活 → 复位禁写标志（此刻本进程再无安装
            // 窗口，禁写只会卡死应用）；标记保留（启动判定兜底）；状态暂存可查。
            set_write_blocked(false);
            stage_update_state(UpdateState::LastInstallIncomplete {
                version: info.version.clone(),
            });
            // 数据安全二期票 01 D8：安装失败进程存活——重启前运行标记可能已被
            // on_before_exit 清掉，重写一份（宁误报不漏报：此后若强杀，下次启动
            // 仍能判异常退出）。失败只 eprintln，不吞安装失败的主结果。
            if let Err(e) = crate::applog::write_run_marker(data_dir, &crate::applog::now_local()) {
                eprintln!("[updater] 安装失败后重写运行标记失败: {e}");
            }
            Ok(InstallOutcome::InstallFailed {
                version: info.version,
                message,
            })
        }
    }
}

fn db_err(e: rusqlite::Error) -> String {
    format!("数据库操作失败: {e}")
}

/// 上次每日检查日（ISO 日期串）；从没查过 = None。
pub fn last_check_day(conn: &Connection) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT value FROM settings WHERE key = ?1",
        params![K_LAST_CHECK_DAY],
        |row| row.get(0),
    )
    .optional()
    .map_err(db_err)
}

fn set_last_check_day(conn: &Connection, day: &str) -> Result<(), String> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![K_LAST_CHECK_DAY, day],
    )
    .map_err(db_err)?;
    Ok(())
}

/// 每日检查 · 段 1（持锁段）：读"上次检查日"判断今天该不该查。调用方拿锁调它、
/// 拿到结果立即放锁，网络检查在锁外做（评审 R1-1）。
pub fn should_run_daily_check(conn: &Connection, today: &str) -> Result<bool, String> {
    Ok(should_check_today(last_check_day(conn)?.as_deref(), today))
}

/// 每日检查 · 段 3（持锁段）：对"已取回的检查结果"折算 + 记账（记今天）。
/// 签名只吃结果值、没有检查器参数——结构上保证本段不可能发起网络调用。
/// 失败结果也记今天（当天不再重试，跨天自愈）；记账失败按 Err 返回，
/// 调用方记日志、下个周期重试当天这轮。
pub fn finish_daily_check(
    conn: &Connection,
    result: Result<Option<UpdateInfo>, String>,
    today: &str,
) -> Result<DailyOutcome, String> {
    let outcome = fold_daily(to_outcome(result));
    set_last_check_day(conn, today)?;
    Ok(match outcome {
        CheckOutcome::UpdateAvailable { version, notes } => {
            DailyOutcome::UpdateAvailable { version, notes }
        }
        // fold_daily 之后失败态已折为 UpToDate，这里兜底同口径
        _ => DailyOutcome::NoUpdate,
    })
}

// ── 薄封装：生产检查器 / 手动入口 / 每日定时（Tauri 侧，系统行为不进单测）──

use std::time::Duration;
use tauri::Manager;

/// 单次检查的总超时（评审 R1-3）：插件 builder 默认无超时（reqwest 无总超时），
/// 网络黑洞会把锁外网络段和手动路径的 UI 等待拖到无上限。
pub const CHECK_TIMEOUT: Duration = Duration::from_secs(30);

/// 带总超时的 updater 构造（endpoints / pubkey 来自 tauri.conf.json）。
/// 票 04：升级前快照挂在本构造的 `on_before_exit` 上——插件 2.11.0 在 Windows
/// 安装器启动前、`std::process::exit` 前回调（插件源码 install_inner），检查
/// 路径不触发它；票 05 的安装流复用本构造即自动带上快照挂点。
fn build_updater(app: &tauri::AppHandle) -> Result<tauri_plugin_updater::Updater, String> {
    use tauri_plugin_updater::UpdaterExt;
    let app_for_exit = app.clone();
    app.updater_builder()
        .timeout(CHECK_TIMEOUT)
        // 闭包签名 Fn() 无参（updater 2.11.0 `OnBeforeExit`），AppHandle 靠捕获进来；
        // 本调用会顶掉 UpdaterExt::updater_builder 预挂的 cleanup_before_exit，故在
        // 闭包末尾补调一次（tauri 2.11.5 起 pub），保持插件退出清理行为不变。
        .on_before_exit(move || {
            before_exit_snapshot(&app_for_exit);
            app_for_exit.cleanup_before_exit();
        })
        .build()
        .map_err(|e| format!("初始化更新器失败: {e}"))
}

/// 快照时间戳（本地时间，yyyymmdd-hhmmss）。薄封装不进单测——防覆盖语义由
/// snapshot_file_name / pre_update_snapshot 的注入式测试覆盖。
fn now_stamp() -> String {
    chrono::Local::now().format("%Y%m%d-%H%M%S").to_string()
}

/// on_before_exit 挂点本体（票 04，薄封装不进单测；核心 pre_update_snapshot /
/// set_write_blocked 全测）：**先清运行标记**（数据安全二期票 01 D8：更新重启是
/// 优雅退出路径之一，本钩子在进程 std::process::exit 前触发、不经主事件循环，
/// 必须在这里自清）→ 拿锁挡并发写 → 拷贝快照 → 锁内先置禁写标志 → 放锁。
/// 标志在持锁期间置位：放锁后所有写路径都在各自锁内复查标志（with_conn /
/// daily_tick 段 3 / check_and_notify；评审 R1 TOCTOU——置位前已过检查、阻塞在
/// 锁上的在途写由锁内复查拦下），新发起与在途写都不再落库，快照与实际库之间
/// 不存在漂移窗口。
/// 任何失败只记日志、绝不阻塞安装（票面：失败不阻塞升级，状态即本函数日志）；
/// 快照失败也不置禁写——没有快照可保护，且安装失败残留时应用需保持可用。
/// 安装失败（进程存活）由 run_confirm_flow 失败臂重写运行标记（防漏报）。
fn before_exit_snapshot(app: &tauri::AppHandle) {
    // 更新安装重启前清运行标记（票 01）：标记被清后即使安装器拉起失败、进程
    // 存活，后续退出仍走常规优雅路径；只有"清了标记又强杀"的窗口会漏报，由
    // run_confirm_flow 失败臂重写标记兜底。
    crate::applog::graceful_exit();
    let Some(state) = app.try_state::<crate::DbState>() else {
        return;
    };
    let data_dir = match app.path().app_data_dir() {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("[updater] 升级前快照失败（不阻塞升级）：解析数据目录失败: {e}");
            return;
        }
    };
    // 版本号与插件同源：updater 2.11.0 的 UpdaterBuilder 也取 package_info().version
    // 作 current_version，保证快照名里的版本 = 更新流认为的当前版本（三处版本号
    // 一致性由票 03 校验脚本兜底）。
    let version = app.package_info().version.to_string();
    let guard = match state.0.lock() {
        Ok(guard) => guard,
        Err(e) => {
            eprintln!("[updater] 升级前快照失败（不阻塞升级）：库锁不可用: {e}");
            return;
        }
    };
    match pre_update_snapshot(&state.1, &data_dir, &version, &now_stamp()) {
        Ok(path) => {
            set_write_blocked(true);
            eprintln!("[updater] 升级前快照完成，进入禁写窗口: {}", path.display());
        }
        Err(e) => eprintln!("[updater] 升级前快照失败（不阻塞升级）: {e}"),
    }
    drop(guard);
}

/// 插件 Update → 最小信息（每日/手动两条路径共用）。
fn update_to_info(u: tauri_plugin_updater::Update) -> UpdateInfo {
    UpdateInfo {
        version: u.version.clone(),
        notes: u.body.clone(),
    }
}

/// 生产检查器：走 tauri-plugin-updater。只在每日定时的独立 std 线程里用
/// （线程内无异步运行时，block_on 安全）。
struct PluginChecker {
    app: tauri::AppHandle,
}

impl UpdateChecker for PluginChecker {
    fn check(&self) -> Result<Option<UpdateInfo>, String> {
        let updater = build_updater(&self.app)?;
        let update = tauri::async_runtime::block_on(updater.check())
            .map_err(|e| format!("检查更新失败: {e}"))?;
        // 版本比较由插件内部完成：远端不比当前新时返回 None
        Ok(update.map(update_to_info))
    }
}

/// 确认流的生产步骤（票 05 薄封装，不进单测）：编排顺序与失败路径由
/// [`run_confirm_flow`] + FakeSteps 在纯函数层锚定，本结构只桥接插件 API。
/// 插件 2.11.0 的 `Update::download` 返回安装包字节，`install(bytes)` 拉起
/// 安装器（Windows 成功即进程退出）；三步都经 block_on 桥接（只在
/// spawn_blocking 线程上跑，见 lib.rs confirm_and_install）。
pub struct PluginConfirmSteps {
    app: tauri::AppHandle,
    /// fresh_update 检查到的 Update，download/install 两步续用同一份。
    update: std::sync::Mutex<Option<tauri_plugin_updater::Update>>,
    /// download 落好的安装包字节，install 步取走。
    bytes: std::sync::Mutex<Option<Vec<u8>>>,
}

impl PluginConfirmSteps {
    pub fn new(app: tauri::AppHandle) -> Self {
        Self {
            app,
            update: std::sync::Mutex::new(None),
            bytes: std::sync::Mutex::new(None),
        }
    }
}

impl ConfirmSteps for PluginConfirmSteps {
    fn fresh_update(&self) -> Result<UpdateInfo, String> {
        let updater = build_updater(&self.app)?;
        let update = tauri::async_runtime::block_on(updater.check())
            .map_err(|e| format!("检查更新失败: {e}"))?
            .ok_or_else(|| "远端已没有比当前更新的版本".to_string())?;
        let info = update_to_info(update.clone());
        *self
            .update
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(update);
        Ok(info)
    }

    fn download(&self) -> Result<(), String> {
        let bytes = {
            let guard = self
                .update
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let update = guard
                .as_ref()
                .ok_or_else(|| "内部状态错误：尚未检查到更新".to_string())?;
            let app = self.app.clone();
            let mut downloaded: u64 = 0;
            tauri::async_runtime::block_on(update.download(
                move |chunk, total| {
                    // 进度事件（票 06 前端契约：update-download-progress，
                    // 负载 {downloaded, total}）；emit 失败静默（副产物不拦流程）
                    use tauri::Emitter;
                    downloaded += chunk as u64;
                    let _ = app.emit(
                        "update-download-progress",
                        DownloadProgress { downloaded, total },
                    );
                },
                || {},
            ))
            .map_err(|e| format!("下载更新失败: {e}"))?
        };
        *self
            .bytes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(bytes);
        Ok(())
    }

    fn install(&self) -> Result<(), String> {
        let guard = self
            .update
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let update = guard
            .as_ref()
            .ok_or_else(|| "内部状态错误：尚未检查到更新".to_string())?;
        let bytes = self
            .bytes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            .ok_or_else(|| "内部状态错误：尚未下载更新包".to_string())?;
        update
            .install(bytes)
            .map_err(|e| format!("启动安装器失败: {e}"))
    }
}

/// 下载进度事件负载（事件名 `update-download-progress`；票 06 前端契约）。
/// `downloaded` 为累计字节数，`total` 为远端未给 Content-Length 时 None。
#[derive(Clone, serde::Serialize)]
struct DownloadProgress {
    downloaded: u64,
    total: Option<u64>,
}

/// 手动检查入口（设置页 `check_update_now`）。async：同步 command 跑在主线程/
/// 事件循环上，阻塞网络会把整个 UI 冻住（评审 R1-2），改 async 由 Tauri 丢进
/// 异步运行时、内部直接 `.await`（此处不得再 block_on）。
/// 失败折为 Err 一次性展示。刻意不写每日记账（last_check_day）：两条路径互不
/// 干扰，当天每日检查照常执行。
pub async fn manual_check(app: tauri::AppHandle) -> Result<CheckOutcome, String> {
    let updater = build_updater(&app)?;
    let update = updater
        .check()
        .await
        .map_err(|e| format!("检查更新失败: {e}"))?;
    manual_result(to_outcome(Ok(update.map(update_to_info))))
}

/// 发现新版的系统通知（每日路径唯一的对外打扰；复用 tauri-plugin-notification，
/// 与提醒同款；发送失败静默——通知是副产物）。
fn notify_update(handle: &tauri::AppHandle, version: &str, notes: Option<&str>) {
    use tauri_plugin_notification::NotificationExt;
    let mut body = format!("发现新版本 v{version}，可到设置页查看并安装。");
    if let Some(n) = notes.map(str::trim).filter(|n| !n.is_empty()) {
        let mut brief: String = n.chars().take(80).collect();
        if brief.chars().count() < n.chars().count() {
            brief.push('…');
        }
        body = format!("发现新版本 v{version}：{brief}");
    }
    let _ = handle
        .notification()
        .builder()
        .title("蚂蚁饲养记录有更新")
        .body(body)
        .show();
}

/// 一轮每日 tick（评审 R1-1 锁拆分三段式，与测试侧 `run_daily_flow` 同序）：
/// 段 1 持锁读记账、立即放锁 → 段 2 锁外做网络检查（30s 总超时兜底）→
/// 段 3 持锁记账折算 → 查到新版发系统通知（窗口关着也跑，托盘常驻线程）。
/// DB 锁被全应用的页面 command 与提醒调度线程共用，绝不持锁等网络。
/// 任何失败都止于日志，绝不弹错误通知（spec 错误路径约定；
/// 评审 R1-4：DB 侧 Err 也记日志，不静默丢弃）。
fn daily_tick(handle: &tauri::AppHandle) {
    if is_write_blocked() {
        // 票 04 禁写窗口：本轮检查与记账都会写库 → 整轮跳过（窗口只到进程退出，
        // 一瞬即逝；错过的一轮重开后按"上次检查日"自然补查）
        return;
    }
    let Some(state) = handle.try_state::<crate::DbState>() else {
        return;
    };
    let today = crate::colony::today_iso();

    // 段 1（持锁）：今天该不该查——纯 DB 读，拿到结果立即放锁
    let should = {
        let Ok(conn) = state.0.lock() else { return };
        should_run_daily_check(&conn, &today)
    };
    match should {
        Ok(false) => return,
        Err(e) => {
            eprintln!("[updater] 每日更新检查读记账失败（本轮跳过）: {e}");
            return;
        }
        Ok(true) => {}
    }

    // 段 2（放锁）：网络检查——锁已归还，堵也只堵本线程
    let checker = PluginChecker {
        app: handle.clone(),
    };
    let result = checker.check();

    // 段 3（持锁）：记账 + 折算。锁内复查禁写标志（评审 R1 TOCTOU：段 2 网络检查
    // 期间快照可能已置位，置位后才拿到的锁必须放弃写入，不能落"快照之后"的记账）
    let outcome = {
        let Ok(conn) = state.0.lock() else { return };
        if is_write_blocked() {
            return;
        }
        finish_daily_check(&conn, result, &today)
    };
    match outcome {
        Ok(DailyOutcome::UpdateAvailable { version, notes }) => {
            notify_update(handle, &version, notes.as_deref());
        }
        Ok(_) => {}
        Err(e) => eprintln!("[updater] 每日更新检查记账失败（下个 30 分钟周期重试当天这轮）: {e}"),
    }
}

/// 每日检查定时器：启动即跑一轮（内部按"上次检查日"决定真查还是跳过，重启
/// 不重查），此后每 30 分钟醒一次，跨天后的第一轮负责真查。独立 std 线程 +
/// catch_unwind，与提醒调度同款（票 09 停靠 A）。
pub fn spawn_daily_checker(handle: tauri::AppHandle) {
    std::thread::spawn(move || loop {
        let h = handle.clone();
        let round = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            daily_tick(&h);
        }));
        if let Err(panic) = round {
            eprintln!("[updater] 本轮每日更新检查 panic（已跳过，下个 30 分钟周期重试）: {panic:?}");
        }
        std::thread::sleep(std::time::Duration::from_secs(30 * 60));
    });
}

// ── 测试：只测外部行为（spec「Testing Decisions」）────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    /// 建内存库并迁移到最新 schema（票 01 地基）。
    fn mem_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("内存库打开失败");
        crate::db::migrate(&conn).expect("迁移失败");
        conn
    }

    /// 假检查器：返回预置结果并计数，绝不碰真网络。
    struct FakeChecker {
        result: Result<Option<UpdateInfo>, String>,
        calls: Cell<usize>,
    }

    impl FakeChecker {
        fn new(result: Result<Option<UpdateInfo>, String>) -> Self {
            FakeChecker {
                result,
                calls: Cell::new(0),
            }
        }
        fn up_to_date() -> Self {
            Self::new(Ok(None))
        }
        fn new_version() -> Self {
            Self::new(Ok(Some(UpdateInfo {
                version: "0.3.0".into(),
                notes: Some("修复若干问题".into()),
            })))
        }
        fn failing() -> Self {
            Self::new(Err("HTTP 404：latest.json 不存在".into()))
        }
        fn call_count(&self) -> usize {
            self.calls.get()
        }
    }

    impl UpdateChecker for FakeChecker {
        fn check(&self) -> Result<Option<UpdateInfo>, String> {
            self.calls.set(self.calls.get() + 1);
            self.result.clone()
        }
    }

    const D1: &str = "2026-09-18";
    const D2: &str = "2026-09-19";

    /// daily_tick 的同序纯逻辑镜像（评审 R1-1 锁拆分后）：段 1 持锁查记账 →
    /// 段 2 放锁做网络（fake）→ 段 3 持锁记账折算。与薄封装里的三段一一对应。
    fn run_daily_flow<C: UpdateChecker>(
        conn: &Connection,
        checker: &C,
        today: &str,
    ) -> Result<DailyOutcome, String> {
        if !should_run_daily_check(conn, today)? {
            return Ok(DailyOutcome::Skipped);
        }
        finish_daily_check(conn, checker.check(), today)
    }

    // ── 三态映射 ──

    #[test]
    fn to_outcome_maps_three_states() {
        // 无更新
        assert_eq!(to_outcome(Ok(None)), CheckOutcome::UpToDate);

        // 有新版：版本号 + 说明
        let info = UpdateInfo {
            version: "0.3.0".into(),
            notes: Some("新增xx".into()),
        };
        assert_eq!(
            to_outcome(Ok(Some(info))),
            CheckOutcome::UpdateAvailable {
                version: "0.3.0".into(),
                notes: Some("新增xx".into())
            }
        );

        // 说明可缺（release notes 可能为空）
        let bare = UpdateInfo {
            version: "0.3.0".into(),
            notes: None,
        };
        assert_eq!(
            to_outcome(Ok(Some(bare))),
            CheckOutcome::UpdateAvailable {
                version: "0.3.0".into(),
                notes: None
            }
        );

        // 检查失败
        assert_eq!(
            to_outcome(Err("请求超时".into())),
            CheckOutcome::CheckFailed {
                message: "请求超时".into()
            }
        );
    }

    #[test]
    fn check_outcome_serializes_tagged_for_frontend() {
        // 票 06 前端契约：tag = status，snake_case
        let json = serde_json::to_string(&CheckOutcome::UpToDate).unwrap();
        assert_eq!(json, r#"{"status":"up_to_date"}"#);

        let json = serde_json::to_string(&CheckOutcome::UpdateAvailable {
            version: "0.3.0".into(),
            notes: Some("n".into()),
        })
        .unwrap();
        assert_eq!(
            json,
            r#"{"status":"update_available","version":"0.3.0","notes":"n"}"#
        );
    }

    // ── 错误路径：每日静默 vs 手动 Err ──

    #[test]
    fn daily_path_folds_any_failure_into_no_update() {
        // 404 / 网络 / 超时 / 解析——任何 Err 都按"无更新"，绝不外抛失败态
        for msg in [
            "HTTP 404：latest.json 不存在",
            "network unreachable",
            "请求超时",
            "清单 JSON 解析失败",
        ] {
            let folded = fold_daily(to_outcome(Err(msg.into())));
            assert_eq!(folded, CheckOutcome::UpToDate, "失败「{msg}」应折为无更新");
        }

        // 成功两态原样通过（有新版仍要提示用户，只折叠失败）
        assert_eq!(fold_daily(to_outcome(Ok(None))), CheckOutcome::UpToDate);
        assert_eq!(
            fold_daily(to_outcome(Ok(Some(UpdateInfo {
                version: "0.3.0".into(),
                notes: None,
            })))),
            CheckOutcome::UpdateAvailable {
                version: "0.3.0".into(),
                notes: None
            }
        );
    }

    #[test]
    fn manual_path_surfaces_failure_as_err_and_keeps_good_states() {
        // 失败 → Err（前端一次性展示）
        let err = manual_result(to_outcome(Err("HTTP 404".into()))).unwrap_err();
        assert_eq!(err, "HTTP 404");

        // 成功两态原样
        assert_eq!(
            manual_result(to_outcome(Ok(None))).unwrap(),
            CheckOutcome::UpToDate
        );
        assert_eq!(
            manual_result(to_outcome(Ok(Some(UpdateInfo {
                version: "0.3.0".into(),
                notes: Some("n".into()),
            }))))
            .unwrap(),
            CheckOutcome::UpdateAvailable {
                version: "0.3.0".into(),
                notes: Some("n".into())
            }
        );
    }

    // ── 每日节奏：同一天一次、跨天重查、跨启动持久化 ──

    #[test]
    fn should_check_today_rules() {
        // 从没查过 → 查
        assert!(should_check_today(None, D1));
        // 记的是别的日子（昨天）或脏值 → 查
        assert!(should_check_today(Some("2026-09-17"), D1));
        assert!(should_check_today(Some("不是日期"), D1));
        // 今天已查 → 不查（重启也不重查的判定基础）
        assert!(!should_check_today(Some(D1), D1));
    }

    // ── 三段式拆分（评审 R1-1：网络段不持 DB 锁）──

    #[test]
    fn should_run_daily_check_reads_bookkeeping() {
        let conn = mem_conn();
        // 从没查过 → 查；记了今天 → 不查；记了昨天 → 查
        assert!(should_run_daily_check(&conn, D1).unwrap());
        set_last_check_day(&conn, D1).unwrap();
        assert!(!should_run_daily_check(&conn, D1).unwrap());
        set_last_check_day(&conn, "2026-09-17").unwrap();
        assert!(should_run_daily_check(&conn, D1).unwrap());
    }

    #[test]
    fn finish_daily_check_records_without_network_access() {
        // 结构锚点（R1-1）：finish_daily_check 只吃"已取回的结果值"、没有检查器
        // 参数——记账段结构上不可能发起网络调用；网络段在两段之间的锁外完成。

        // 查到新版：记账 + 原样折算
        let conn = mem_conn();
        let out = finish_daily_check(
            &conn,
            Ok(Some(UpdateInfo {
                version: "0.3.0".into(),
                notes: Some("n".into()),
            })),
            D1,
        )
        .unwrap();
        assert_eq!(
            out,
            DailyOutcome::UpdateAvailable {
                version: "0.3.0".into(),
                notes: Some("n".into())
            }
        );
        assert_eq!(last_check_day(&conn).unwrap().as_deref(), Some(D1));

        // 失败结果：同样记账（当天不重试），折为无更新（每日静默）
        let conn = mem_conn();
        let out = finish_daily_check(&conn, Err("请求超时".into()), D1).unwrap();
        assert_eq!(out, DailyOutcome::NoUpdate);
        assert_eq!(last_check_day(&conn).unwrap().as_deref(), Some(D1));
    }

    #[test]
    fn daily_checks_once_per_day_and_again_next_day() {
        let conn = mem_conn();
        assert_eq!(last_check_day(&conn).unwrap(), None, "全新库没查过");

        let checker = FakeChecker::new_version();

        // 第一次（无记录）：查到新版，并记下今天
        let out = run_daily_flow(&conn, &checker, D1).unwrap();
        assert_eq!(
            out,
            DailyOutcome::UpdateAvailable {
                version: "0.3.0".into(),
                notes: Some("修复若干问题".into())
            }
        );
        assert_eq!(checker.call_count(), 1);
        assert_eq!(last_check_day(&conn).unwrap().as_deref(), Some(D1), "查完记今天");

        // 同日再来（模拟重启）：跳过，不再打远端
        let out = run_daily_flow(&conn, &checker, D1).unwrap();
        assert_eq!(out, DailyOutcome::Skipped);
        assert_eq!(checker.call_count(), 1, "同一天只查一次");

        // 跨天：重查，记录推进到新的一天
        let out = run_daily_flow(&conn, &checker, D2).unwrap();
        assert_eq!(
            out,
            DailyOutcome::UpdateAvailable {
                version: "0.3.0".into(),
                notes: Some("修复若干问题".into())
            }
        );
        assert_eq!(checker.call_count(), 2);
        assert_eq!(last_check_day(&conn).unwrap().as_deref(), Some(D2));
    }

    #[test]
    fn daily_failure_still_counts_as_checked_for_the_day() {
        // 失败也按"今天查过了"记账：当天不再重试，对外是无更新、绝不发通知；
        // 跨天后自然重查（404 窗口期由每日检查自愈）。
        let conn = mem_conn();
        let checker = FakeChecker::failing();

        let out = run_daily_flow(&conn, &checker, D1).unwrap();
        assert_eq!(out, DailyOutcome::NoUpdate, "失败按无更新");
        assert_eq!(last_check_day(&conn).unwrap().as_deref(), Some(D1));

        // 同日再跑：跳过（失败的当天也不查第二次）
        let out = run_daily_flow(&conn, &checker, D1).unwrap();
        assert_eq!(out, DailyOutcome::Skipped);
        assert_eq!(checker.call_count(), 1);
    }

    #[test]
    fn daily_run_without_update_records_day_and_stays_quiet() {
        // 无新版：安静记一天，跨天后照常重查
        let conn = mem_conn();
        let checker = FakeChecker::up_to_date();

        let out = run_daily_flow(&conn, &checker, D1).unwrap();
        assert_eq!(out, DailyOutcome::NoUpdate);
        assert_eq!(last_check_day(&conn).unwrap().as_deref(), Some(D1));

        let out = run_daily_flow(&conn, &checker, D2).unwrap();
        assert_eq!(out, DailyOutcome::NoUpdate);
        assert_eq!(checker.call_count(), 2);
    }

    #[test]
    fn manual_check_leaves_daily_bookkeeping_alone() {
        // 手动检查不写 update_last_check_day：当天每日检查照常执行（互不干扰）
        let conn = mem_conn();
        let checker = FakeChecker::up_to_date();

        let manual = manual_result(to_outcome(checker.check())).unwrap();
        assert_eq!(manual, CheckOutcome::UpToDate);
        assert_eq!(
            last_check_day(&conn).unwrap(),
            None,
            "手动检查不写每日记账"
        );

        // 每日照常可查
        let out = run_daily_flow(&conn, &checker, D1).unwrap();
        assert_eq!(out, DailyOutcome::NoUpdate);
    }

    // ── 升级前快照与禁写窗口（票 04）──

    use tempfile::TempDir;

    /// 建文件库（tempdir）并迁移到最新 schema（沿 system.rs 测试先例）。
    fn file_conn() -> (Connection, std::path::PathBuf, TempDir) {
        let dir = TempDir::new().expect("创建临时目录失败");
        let db_path = dir.path().join("data").join(crate::db::DB_FILE_NAME);
        let conn = crate::db::open_and_migrate(&db_path).expect("建库失败");
        (conn, db_path, dir)
    }

    #[test]
    fn snapshot_file_name_embeds_version_and_stamp() {
        // 票面命名：pre-update-v<当前版本>-<yyyymmdd-hhmmss>.db
        assert_eq!(
            snapshot_file_name("0.2.0", "20260918-181223"),
            "pre-update-v0.2.0-20260918-181223.db"
        );
    }

    #[test]
    fn same_version_snapshots_coexist_without_overwrite() {
        // 同版本重试/重装：不同时间戳两次快照并存，旧快照不被覆盖
        let (_conn, db_path, dir) = file_conn();
        let data_dir = dir.path();

        let first = pre_update_snapshot(&db_path, data_dir, "0.2.0", "20260918-120000").unwrap();
        let second = pre_update_snapshot(&db_path, data_dir, "0.2.0", "20260918-120001").unwrap();

        assert_ne!(first, second, "两次快照路径不同");
        assert!(first.exists() && second.exists(), "两份快照并存");
        assert_eq!(
            first.parent(),
            Some(data_dir.join("backups").as_path()),
            "落在数据目录 backups/ 下"
        );
        assert!(
            first
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("pre-update-v0.2.0-"),
            "文件名含源版本，实际：{}",
            first.display()
        );

        // 两份都是完整库（可被应用正式打开通道重开，迁移幂等通过）
        for path in [&first, &second] {
            let reopened = crate::db::open_and_migrate(path).expect("快照产物可重开");
            assert_eq!(
                crate::db::schema_version_of(&reopened).unwrap(),
                crate::db::SCHEMA_VERSION
            );
        }
    }

    #[test]
    fn snapshot_product_reopenable_with_data_complete() {
        // 带数据的快照 → 应用正式打开通道重开 → 数据齐（沿 system.rs 备份测试先例）
        let (conn, db_path, dir) = file_conn();
        conn.execute(
            "INSERT INTO colony (name, start_date) VALUES ('大头一号', '2026-01-20')",
            [],
        )
        .unwrap();

        let snap = pre_update_snapshot(&db_path, dir.path(), "0.1.0", "20260918-181223").unwrap();
        let reopened = crate::db::open_and_migrate(&snap).expect("快照产物可被应用重开");
        let colonies: i64 = reopened
            .query_row("SELECT COUNT(*) FROM colony", [], |r| r.get(0))
            .unwrap();
        assert_eq!(colonies, 1, "快照数据完整");
    }

    #[test]
    fn snapshot_failure_returns_error_without_panicking() {
        // 快照失败（源缺失等）→ Err 交给调用方记日志后继续安装，不 panic 不中断
        let dir = TempDir::new().expect("创建临时目录失败");
        let outcome = pre_update_snapshot(
            &dir.path().join("no-such.db"),
            dir.path(),
            "0.2.0",
            "20260918-181223",
        );
        assert!(outcome.is_err(), "源不存在应报错而不是静默产出空快照");
    }

    #[test]
    fn gate_write_rejects_only_during_window() {
        // 禁写闸门：窗口外（快照前/日常运行）正常写放行；窗口内写请求一律拒绝
        assert!(gate_write(false).is_ok(), "未禁写不受影响");

        let err = gate_write(true).unwrap_err();
        assert!(err.contains("安装更新"), "拒绝文案要点明原因，实际：{err}");
    }

    #[test]
    fn write_block_flag_roundtrip() {
        // 进程级标志：置位后写请求被拒、复位后恢复（Drop 兜底复位，不污染其他测试）。
        // 与票 05 的标志/状态测试共用 GLOBAL_LOCK 串行（并行测试互不踩全局）。
        let _g = lock_globals();
        struct ResetFlag;
        impl Drop for ResetFlag {
            fn drop(&mut self) {
                set_write_blocked(false);
            }
        }
        let _reset = ResetFlag;

        assert!(!is_write_blocked(), "初始未禁写");
        assert!(ensure_writable().is_ok());

        set_write_blocked(true);
        assert!(is_write_blocked());
        assert!(ensure_writable().is_err(), "置位后写请求被拒");

        set_write_blocked(false);
        assert!(ensure_writable().is_ok(), "复位后恢复放行");
    }

    #[test]
    fn in_flight_write_passing_prelock_check_is_rejected_by_inlock_recheck() {
        // 评审 R1 TOCTOU 锚点：置位前已通过闸门检查、随后阻塞拿锁的在途写，
        // 拿到锁后由"锁内复查"拒绝。生产形态：with_conn / daily_tick 段 3 /
        // check_and_notify 均在拿到锁之后才调 ensure_writable / is_write_blocked
        //（真锁无法进单测，这里按同一时序在纯逻辑层模拟闸门顺序）。
        // 与票 05 的标志/状态测试共用 GLOBAL_LOCK 串行（并行测试互不踩全局）。
        let _g = lock_globals();
        struct ResetFlag;
        impl Drop for ResetFlag {
            fn drop(&mut self) {
                set_write_blocked(false);
            }
        }
        let _reset = ResetFlag;

        // 1. 在途写到达，通过拿锁前的检查（此刻标志未置位）
        assert!(ensure_writable().is_ok(), "置位前在途写检查放行");

        // 2. 快照持锁段内置位（模拟 before_exit_snapshot 的锁内置位）
        set_write_blocked(true);

        // 3. 在途写拿到锁 → 锁内复查必须拒绝（写落在快照之后 = 禁止）
        assert!(ensure_writable().is_err(), "锁内复查拒绝在途写");
    }

    // ── 票 05：pending 标记 / 启动判定 / 确认流编排 ─────────────────────────

    use std::cell::RefCell;
    use std::sync::Mutex as StdMutex;
    use std::sync::MutexGuard;

    /// 串行化触及进程级全局（禁写标志 / 暂存更新状态）的测试：并行测试互不踩踏。
    static GLOBAL_LOCK: StdMutex<()> = StdMutex::new(());

    fn lock_globals() -> MutexGuard<'static, ()> {
        GLOBAL_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// 测试期间的禁写标志复位兜底（全局标志，绝不把置位泄漏给其他测试）。
    struct ResetWriteFlag;
    impl Drop for ResetWriteFlag {
        fn drop(&mut self) {
            set_write_blocked(false);
        }
    }

    fn marker_path(dir: &Path) -> std::path::PathBuf {
        dir.join(PENDING_MARKER_FILE)
    }

    /// 假确认流步骤：预置各步结果并按序记录调用，绝不碰真网络/真安装器
    ///（照 FakeChecker 模式——编排顺序与失败路径在纯函数层锚定）。
    struct FakeSteps {
        update: Result<UpdateInfo, String>,
        download: Result<(), String>,
        install: Result<(), String>,
        calls: RefCell<Vec<&'static str>>,
    }

    impl FakeSteps {
        fn ok_flow(version: &str) -> Self {
            FakeSteps {
                update: Ok(UpdateInfo {
                    version: version.into(),
                    notes: None,
                }),
                download: Ok(()),
                install: Ok(()),
                calls: RefCell::new(Vec::new()),
            }
        }
        fn call_log(&self) -> Vec<&'static str> {
            self.calls.borrow().clone()
        }
    }

    impl ConfirmSteps for FakeSteps {
        fn fresh_update(&self) -> Result<UpdateInfo, String> {
            self.calls.borrow_mut().push("check");
            self.update.clone()
        }
        fn download(&self) -> Result<(), String> {
            self.calls.borrow_mut().push("download");
            self.download.clone()
        }
        fn install(&self) -> Result<(), String> {
            self.calls.borrow_mut().push("install");
            self.install.clone()
        }
    }

    #[test]
    fn version_cmp_is_semantic_not_lexicographic() {
        use std::cmp::Ordering::{Equal, Greater, Less};
        // 票面硬锚：0.2.0 < 0.10.0（字符串比较会得出反结论）
        assert_eq!(version_cmp("0.2.0", "0.10.0"), Less);
        assert_eq!(version_cmp("0.9.9", "0.10.0"), Less);
        assert_eq!(version_cmp("0.2.0", "0.2.0"), Equal);
        assert_eq!(version_cmp("0.2.0", "0.2.1"), Less);
        assert_eq!(version_cmp("0.3.0", "0.2.9"), Greater);
        assert_eq!(version_cmp("1.0.0", "0.99.99"), Greater);
        // 段数不等补 0：0.2 等价 0.2.0
        assert_eq!(version_cmp("0.2", "0.2.0"), Equal);
        assert_eq!(version_cmp("0.2", "0.2.1"), Less);
        // 前导 v 容忍（远端清单异常带 v 也不误判）
        assert_eq!(version_cmp("v0.10.0", "0.2.0"), Greater);
    }

    #[test]
    fn pending_marker_write_read_roundtrip() {
        let dir = TempDir::new().expect("创建临时目录失败");
        let path = write_pending_marker(dir.path(), "0.3.0", "20260918-181223").unwrap();
        assert_eq!(path.parent(), Some(dir.path()), "标记落在数据目录");
        assert_eq!(
            path.file_name().unwrap().to_string_lossy(),
            PENDING_MARKER_FILE,
            "文件名固定，不进数据库"
        );

        let marker = load_pending_marker(dir.path()).expect("刚写的标记应能读回");
        assert_eq!(marker.target_version, "0.3.0");
        assert_eq!(marker.stamped_at, "20260918-181223");

        // 落盘 JSON 含目标版本（跨启动唯一的"想升没升成"凭据）
        let text = std::fs::read_to_string(marker_path(dir.path())).unwrap();
        assert!(text.contains("0.3.0"), "JSON 里应含目标版本，实际：{text}");
    }

    #[test]
    fn pending_marker_clear_and_missing_tolerated() {
        let dir = TempDir::new().expect("创建临时目录失败");

        // 从来没有标记：清除是 no-op 成功
        clear_pending_marker(dir.path()).unwrap();
        assert!(load_pending_marker(dir.path()).is_none());

        write_pending_marker(dir.path(), "0.3.0", "s").unwrap();
        clear_pending_marker(dir.path()).unwrap();
        assert!(load_pending_marker(dir.path()).is_none(), "清除后应读不到");
    }

    #[test]
    fn corrupted_marker_treated_as_no_marker_and_cleaned() {
        let dir = TempDir::new().expect("创建临时目录失败");

        // 纯垃圾字节
        std::fs::write(marker_path(dir.path()), "不是 JSON{{{").unwrap();
        assert!(
            load_pending_marker(dir.path()).is_none(),
            "损坏按无标记处理"
        );
        assert!(
            !marker_path(dir.path()).exists(),
            "坏文件顺手清掉，不反复报错"
        );

        // JSON 合法但字段缺失
        std::fs::write(marker_path(dir.path()), r#"{"version":"0.3.0"}"#).unwrap();
        assert!(load_pending_marker(dir.path()).is_none());

        // 目标版本为空串也算损坏
        std::fs::write(
            marker_path(dir.path()),
            r#"{"target_version":"","stamped_at":"t"}"#,
        )
        .unwrap();
        assert!(load_pending_marker(dir.path()).is_none());
    }

    #[test]
    fn judge_startup_three_states() {
        let m = |v: &str| PendingMarker {
            target_version: v.into(),
            stamped_at: "s".into(),
        };

        // 无标记 → 正常路径
        assert_eq!(judge_startup(None, "0.2.0"), UpdateState::Idle);

        // 标记 == 当前 → 升级成功
        assert_eq!(
            judge_startup(Some(&m("0.2.0")), "0.2.0"),
            UpdateState::LastInstallSucceeded {
                version: "0.2.0".into()
            }
        );
        // 标记 < 当前（成功后又升过）也算成功
        assert_eq!(
            judge_startup(Some(&m("0.2.0")), "0.3.0"),
            UpdateState::LastInstallSucceeded {
                version: "0.2.0".into()
            }
        );

        // 标记 > 当前 → 上次升级未完成
        assert_eq!(
            judge_startup(Some(&m("0.3.0")), "0.2.0"),
            UpdateState::LastInstallIncomplete {
                version: "0.3.0".into()
            }
        );
        // 语义化判定：0.10.0 > 0.2.0（字符串比较会误判成"成功"）
        assert_eq!(
            judge_startup(Some(&m("0.10.0")), "0.2.0"),
            UpdateState::LastInstallIncomplete {
                version: "0.10.0".into()
            }
        );
    }

    #[test]
    fn startup_judgment_consumes_marker_and_stages_result() {
        let _g = lock_globals();
        let dir = TempDir::new().expect("创建临时目录失败");

        // 无标记：Idle，不炸、可查
        assert_eq!(startup_judgment(dir.path(), "0.2.0"), UpdateState::Idle);
        assert_eq!(current_update_state(), UpdateState::Idle);

        // 未完成：判定 → 清标记 → 暂存可查（get_update_state 口径）
        write_pending_marker(dir.path(), "0.3.0", "s").unwrap();
        assert_eq!(
            startup_judgment(dir.path(), "0.2.0"),
            UpdateState::LastInstallIncomplete {
                version: "0.3.0".into()
            }
        );
        assert!(!marker_path(dir.path()).exists(), "判定后标记即清");
        assert_eq!(
            current_update_state(),
            UpdateState::LastInstallIncomplete {
                version: "0.3.0".into()
            }
        );

        // 成功：同上，状态换成"已升级到 vX"
        write_pending_marker(dir.path(), "0.2.0", "s").unwrap();
        assert_eq!(
            startup_judgment(dir.path(), "0.2.0"),
            UpdateState::LastInstallSucceeded {
                version: "0.2.0".into()
            }
        );
        assert!(!marker_path(dir.path()).exists());
        assert_eq!(
            current_update_state(),
            UpdateState::LastInstallSucceeded {
                version: "0.2.0".into()
            }
        );

        // 损坏标记：按无标记处理（Idle），坏文件顺手清掉，启动不受影响
        std::fs::write(marker_path(dir.path()), "{{{损坏").unwrap();
        assert_eq!(startup_judgment(dir.path(), "0.2.0"), UpdateState::Idle);
        assert!(!marker_path(dir.path()).exists());
    }

    #[test]
    fn confirm_flow_success_checks_marker_then_downloads_then_installs() {
        let dir = TempDir::new().expect("创建临时目录失败");
        let steps = FakeSteps::ok_flow("0.3.0");

        let out = run_confirm_flow(dir.path(), &steps).unwrap();

        assert_eq!(
            out,
            InstallOutcome::InstallStarted {
                version: "0.3.0".into()
            }
        );
        assert_eq!(
            steps.call_log(),
            vec!["check", "download", "install"],
            "流程顺序：再次检查 → 下载 → 安装"
        );
        // 成功路径标记保留：它是"想升到 0.3.0"的凭据（Windows 下安装器拉起后进程
        // 即退出），由下次启动的判定消费（成功/未完成由此分辨）
        assert!(
            marker_path(dir.path()).exists(),
            "安装成功后标记保留，交启动判定"
        );
    }

    #[test]
    fn confirm_flow_download_failure_keeps_marker_and_resets_flag() {
        // 票面硬性顺序锚点 + 评审 M-1 契约：
        // 先写标记后下载 —— 下载失败时标记必须仍在（启动检测才能发现"想升没升成"）；
        // 失败且进程存活 —— 禁写标志必须复位。
        let _g = lock_globals();
        let _reset = ResetWriteFlag;
        let dir = TempDir::new().expect("创建临时目录失败");

        let mut steps = FakeSteps::ok_flow("0.3.0");
        steps.download = Err("网络断了".into());
        steps.install = Err("下载失败后不得走到安装".into());

        // 预置置位（真实时序里置位发生在 install 内部钩子；这里验证失败臂复位语义）
        set_write_blocked(true);

        let out = run_confirm_flow(dir.path(), &steps).unwrap();

        assert_eq!(
            out,
            InstallOutcome::InstallFailed {
                version: "0.3.0".into(),
                message: "网络断了".into()
            }
        );
        assert_eq!(
            steps.call_log(),
            vec!["check", "download"],
            "下载失败后不得触发安装"
        );
        // 锚点：标记先于下载写入，下载失败后仍在
        let marker = load_pending_marker(dir.path()).expect("下载失败标记必须仍在");
        assert_eq!(marker.target_version, "0.3.0");
        // M-1：失败路径复位禁写标志，应用不得卡在只读态
        assert!(!is_write_blocked(), "失败路径必须复位禁写标志");
        // 失败状态暂存可查（供 UI 重试/手动下载引导）
        assert_eq!(
            current_update_state(),
            UpdateState::LastInstallIncomplete {
                version: "0.3.0".into()
            }
        );
    }

    #[test]
    fn confirm_flow_install_failure_keeps_marker_and_resets_flag() {
        // 评审 M-1 契约主路径：Windows 下插件 install_inner（快照+置位）先跑，
        // 安装器启动失败 → install() 返回 Err 且进程存活 → 必须复位禁写标志，
        // 否则应用永久禁写。
        let _g = lock_globals();
        let _reset = ResetWriteFlag;
        let dir = TempDir::new().expect("创建临时目录失败");

        let mut steps = FakeSteps::ok_flow("0.3.0");
        steps.install = Err("启动安装器失败: ShellExecute".into());

        set_write_blocked(true); // 模拟 on_before_exit 快照完成后的置位

        let out = run_confirm_flow(dir.path(), &steps).unwrap();

        assert_eq!(
            out,
            InstallOutcome::InstallFailed {
                version: "0.3.0".into(),
                message: "启动安装器失败: ShellExecute".into()
            }
        );
        assert_eq!(steps.call_log(), vec!["check", "download", "install"]);
        assert!(
            !is_write_blocked(),
            "M-1：install 返回 Err 且进程存活必须复位禁写标志"
        );
        assert!(
            load_pending_marker(dir.path()).is_some(),
            "安装失败标记保留（残余风险的启动判定兜底）"
        );
        assert_eq!(
            current_update_state(),
            UpdateState::LastInstallIncomplete {
                version: "0.3.0".into()
            }
        );
    }

    #[test]
    fn confirm_flow_check_failure_writes_no_marker() {
        let dir = TempDir::new().expect("创建临时目录失败");
        let mut steps = FakeSteps::ok_flow("0.3.0");
        steps.update = Err("HTTP 404：latest.json 不存在".into());

        let out = run_confirm_flow(dir.path(), &steps);

        assert_eq!(out.unwrap_err(), "HTTP 404：latest.json 不存在");
        assert_eq!(
            steps.call_log(),
            vec!["check"],
            "检查失败：不写标记、不下载、不安装"
        );
        assert!(!marker_path(dir.path()).exists(), "检查失败不落标记");
    }

    #[test]
    fn confirm_flow_marker_write_failure_aborts_before_download() {
        // data_dir 是一个普通文件 → 写标记必失败 → 必须在下载前中止
        //（没有"想升"凭据就绝不下载安装包）
        let dir = TempDir::new().expect("创建临时目录失败");
        let blocker = dir.path().join("not-a-dir");
        std::fs::write(&blocker, b"x").unwrap();

        let steps = FakeSteps::ok_flow("0.3.0");
        let out = run_confirm_flow(&blocker, &steps);

        assert!(out.is_err(), "写标记失败应报错中止");
        assert_eq!(steps.call_log(), vec!["check"], "写标记失败：绝不下载");
        // 禁写标志断言刻意不写在此处：此刻还没走到 install，标志本就不可能置位
        //（ ambient 状态归 write_block_flag_roundtrip / M-1 两个专项测试管）
    }

    #[test]
    fn confirm_flow_failure_rewrites_run_marker_for_abnormal_exit_detection() {
        // 数据安全二期票 01 D8：下载/安装失败且进程存活 → 重写运行标记
        //（重启前可能已被 on_before_exit 清掉），此后强杀仍能被判异常退出
        //（宁误报不漏报）。
        let dir = TempDir::new().expect("创建临时目录失败");
        let mut steps = FakeSteps::ok_flow("0.3.0");
        steps.install = Err("启动安装器失败".into());
        run_confirm_flow(dir.path(), &steps).unwrap();
        assert!(
            dir.path().join(crate::applog::RUN_MARKER_FILE).exists(),
            "安装失败后运行标记必须被重写"
        );

        // 检查失败路径：流程在写任何东西前中止，不落运行标记
        let dir2 = TempDir::new().expect("创建临时目录失败");
        let mut steps2 = FakeSteps::ok_flow("0.3.0");
        steps2.update = Err("HTTP 404：latest.json 不存在".into());
        assert!(run_confirm_flow(dir2.path(), &steps2).is_err());
        assert!(
            !dir2.path().join(crate::applog::RUN_MARKER_FILE).exists(),
            "检查失败不写运行标记"
        );
    }

    #[test]
    fn install_and_update_states_serialize_for_frontend() {
        // 票 06 前端契约：tag = status，snake_case
        assert_eq!(
            serde_json::to_string(&InstallOutcome::InstallStarted {
                version: "0.3.0".into()
            })
            .unwrap(),
            r#"{"status":"install_started","version":"0.3.0"}"#
        );
        assert_eq!(
            serde_json::to_string(&InstallOutcome::InstallFailed {
                version: "0.3.0".into(),
                message: "boom".into(),
            })
            .unwrap(),
            r#"{"status":"install_failed","version":"0.3.0","message":"boom"}"#
        );
        assert_eq!(
            serde_json::to_string(&UpdateState::Idle).unwrap(),
            r#"{"status":"idle"}"#
        );
        assert_eq!(
            serde_json::to_string(&UpdateState::LastInstallSucceeded {
                version: "0.2.0".into()
            })
            .unwrap(),
            r#"{"status":"last_install_succeeded","version":"0.2.0"}"#
        );
        assert_eq!(
            serde_json::to_string(&UpdateState::LastInstallIncomplete {
                version: "0.3.0".into()
            })
            .unwrap(),
            r#"{"status":"last_install_incomplete","version":"0.3.0"}"#
        );
    }
}
