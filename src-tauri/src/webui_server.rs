//! 网页端内嵌 HTTP 服务与双闸准入（webui-checkin 票 04）。
//!
//! 进程内 HTTP 服务跑在 Tauri 自带的 tokio 运行时上（axum + hyper，选型理由见
//! Cargo.toml），与桌面 IPC 共享同一把库锁（[`crate::DbState`] 的单连接，绝不
//! 另开连接——规格铁律「不动单连接」）。
//!
//! 双闸（规格 Implementation Decisions B）：
//! - **闸一·网段白名单**：连接级准入——accept 到 socket 先看对端地址（只认
//!   socket 对端，忽略一切转发头），IPv4 CIDR 匹配受信网段；IPv6 一律拒。
//!   不在段内直接在 HTTP 解析前回一段极简 403 再断开（TCP 层拒绝，验收口径）。
//!   纯核 [`admit_peer`] 注入对端 IP + 网段表，独立可测。
//! - **闸二·Bearer 凭证**：除健康检查（`GET /api/health`）外所有路径要求
//!   `Authorization: Bearer <token>`。**凭证每请求从 webui-config.json 现读**
//!   （取舍注释见 [`auth_mw`]）：重生成即刻生效拒旧值，零失效耦合；小文件读
//!   在 ≤8 并发下开销可忽略，读失败按「凭证未就绪」拒绝（fail-closed）。
//! - **SSE 短时票据**：`POST /api/sse-ticket` 主凭证换 60 秒一次性票据
//!   （[`SseTicketStore`]，建连即消费；消费/过期纯核可测）。`GET /api/sse`
//!   （票 06）：建连消费票据、首帧 `{type:"hello", epoch, version}`、此后每次
//!   版本 bump 广播 `{type:"version", ...}`——版本枢纽 [`VersionHub`]，写侧
//!   [`bump_and_publish`] / [`restore_bump_and_publish`]（lib.rs 写后钩子与
//!   恢复完成的统一咽喉），连接单独计量 ≤4（[`ConnLimiter::try_acquire_sse`]）。
//! - **数据版本查询**：`GET /api/data-version`（票 06，规格 A 白名单）回
//!   `{epoch, version}` 库内真值。
//!
//! 白名单默认拒绝：`POST /api/cmd` 只派发注册表 [`WEBUI_COMMANDS`] 里登记的
//! 命令，其余一律 404；四个网页端配置命令（凭证明文/安全配置面）**永久禁入**，
//! 由 `registry_excludes_forbidden_commands` 钉死。
//!
//! 命令派发（票 05）：白名单≠校验——每个登记命令在 [`crate::webui_args`] 有
//! 类型化参数 schema（顶层键 camelCase 与桌面 Tauri invoke 映射同款），反序列
//! 化 + 取值校验不过即 `400 {"error":人话}`，不进库；过校验的命令经
//! [`crate::run_with_conn`] 与桌面同一把库锁执行（与桌面纯核同源，不复制业务）。
//! 写命令成功后按桌面语义触发 [`AfterWriteHook`]（自动备份记账 + 按命令性质刷
//! 托盘 tooltip；巢况写命令不刷托盘——巢况永不参与提醒/催促）。
//!
//! 前端静态资源（票 05）：打包后的 frontendDist 产物经 [`AssetLookup`] 托管，
//! 浏览器打开 `http://<IP>:<端口>/` 即得完整前端。静态路由只豁免闸二（页面要
//! 先加载才能执行 `#token=` 入库的入口 JS，带不了 Authorization 头；产物是
//! 公开构建物、无敏感内容），闸一照拦；并保留限额与超时。
//!
//! 请求限制（规格 A）：请求体 ≤1MB（照片上传端点票 08 自行放宽到 15MB/张，
//! 框架层预留按路径 `DefaultBodyLimit::max` 的口子）、普通并发 ≤8、SSE 连接单独计量
//! ≤4（计数器本票落地，票 06 挂到 SSE 路由）、读超时 30 秒（hyper 连接层
//! header 读超时 + 请求处理超时中间件；照片上传的处理超时按路径放宽到 300
//! 秒——慢 Wi-Fi 大批量，终局评审）、header 条数/单条长度/总体积上限。
//!
//! 照片端点（票 08，规格 E「桌面/网页同一套」）：**独立 HTTP 端点，不进 /api/cmd
//! 白名单**（二进制体不进 JSON 派发层，也不新增命令名）：
//! - `POST /api/photos`——multipart 表单（`checkinId` 文本段 + `photos` 文件段
//!   ≤9 张），单张流式体积闸 ≤15MB；字节过 [`crate::photo::process_uploads`]
//!   同一套校验链（魔数白名单/头部尺寸/解码炸弹/重编码 JPEG 清 EXIF），落盘名
//!   一律服务端 UUID（[`crate::photo::attach_photos`] 写入协议），成功触发写后
//!   钩子（版本 bump + 自动备份；巢况不刷托盘）。体上限按路径放宽到
//!   9×15MB+框架开销（[`MAX_PHOTOS_BODY_BYTES`]），并发计入普通 8 槽。
//! - `GET /api/photo/{colonyId}/{file}`——Bearer 照挂；严格路径 grammar + 查库
//!   确认引用（防枚举未引用文件）；库引用在而文件缺 → 404 占位；响应头三件套
//!   `image/jpeg` + `nosniff` + `inline`（只以图片身份渲染）。
//!
//! 日志（D7）：起/停/端口占用/准入与凭证拒绝（来源 IP + 原因，绝不带 token 值）
//! 落 applog 流水。

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rusqlite::Connection;

use axum::extract::{ConnectInfo, State};
use axum::middleware::Next;
use axum::response::IntoResponse;

use crate::webui_args;
use crate::webui_config;

// ── 常量：请求限制（规格 A）─────────────────────────────────────────────────

/// 普通请求体上限（1MB）。照片上传端点（票 08）在自己的路由上用
/// `DefaultBodyLimit::max(15MB)` 放宽，不动全局值。
pub const MAX_BODY_BYTES: usize = 1024 * 1024;

/// 照片上传端点路径（票 08；limits_mw 的体上限按路径放宽以此判定）。
pub const PHOTO_UPLOAD_PATH: &str = "/api/photos";

/// 照片上传端点请求体上限：单张 ≤15MB × 单次 ≤9 张 + multipart 框架开销
/// （票 04 预留的按路径 `DefaultBodyLimit` 口子；全局 1MB 对其余路径不动）。
pub const MAX_PHOTOS_BODY_BYTES: usize =
    crate::photo::MAX_INPUT_BYTES * crate::photo::MAX_PHOTOS_PER_SUBMIT + 64 * 1024;

/// 普通并发上限（8）。SSE 长连接单独计量（[`ConnLimiter`] 双计数器）。
pub const MAX_CONCURRENT_REQUESTS: usize = 8;

/// SSE 长连接上限（4，不挤占普通并发额度；票 06 的 SSE 路由消费）。
pub const MAX_CONCURRENT_SSE: usize = 4;

/// 读超时（30 秒）：连接层 header 读超时（慢速请求防挂）+ 请求处理超时。
pub const READ_TIMEOUT: Duration = Duration::from_secs(30);

/// 照片上传端点的请求处理超时（300 秒；终局评审）：慢 Wi-Fi 一批传 9 张大图
/// （服务端还要逐张解码重编码），30 秒普通读超时太紧——与体上限同款「按路径
/// 放宽」的口子（[`PHOTO_UPLOAD_PATH`] 专用，其余路径维持 [`READ_TIMEOUT`]）。
pub const PHOTO_UPLOAD_TIMEOUT: Duration = Duration::from_secs(300);

/// 请求处理超时按（方法, 路径）取值：仅 `POST /api/photos` 放宽（照片上传，
/// 慢 Wi-Fi 大批量），其余一律 [`READ_TIMEOUT`]。纯核独立可测。
fn request_timeout_for(method: &axum::http::Method, path: &str) -> Duration {
    if method == axum::http::Method::POST && path == PHOTO_UPLOAD_PATH {
        PHOTO_UPLOAD_TIMEOUT
    } else {
        READ_TIMEOUT
    }
}

/// header 条数上限。
pub const MAX_HEADER_COUNT: usize = 64;

/// header 单条（值）长度上限。
pub const MAX_HEADER_VALUE_BYTES: usize = 4 * 1024;

/// header 名+值总体积上限。
pub const MAX_HEADER_TOTAL_BYTES: usize = 32 * 1024;

// ── 常量：SSE 短时票据 ──────────────────────────────────────────────────────

/// 票据有效期（60 秒，仅约束建连）。
pub const TICKET_TTL_SECS: i64 = 60;

/// 票据随机字节数（128 bit → 32 hex 字符，与访问凭证同规格）。
pub const TICKET_BYTES: usize = 16;

// ── 纯核心：闸一（网段白名单判定）───────────────────────────────────────────

/// 闸一纯核：对端地址是否在受信网段内。IPv6 一律拒（规格 B）；网段逐条 CIDR
/// 匹配；非法网段项解析失败视为不匹配（fail-closed，脏配置不会变成放行）。
pub fn admit_peer(peer: IpAddr, segments: &[String]) -> bool {
    let v4 = match peer {
        IpAddr::V4(v4) => v4,
        // IPv6 默认拒绝（规格 B：IPv4 CIDR 匹配，Out of Scope 明列 IPv6）
        IpAddr::V6(_) => return false,
    };
    let ip = u32::from(v4);
    segments.iter().any(|s| {
        crate::netseg::parse_cidr(s).is_some_and(|(net, prefix)| ip & crate::netseg::prefix_mask(prefix) == net)
    })
}

// ── 纯核心：闸二（Bearer 凭证校验）──────────────────────────────────────────

/// 闸二纯核：校验 `Authorization` 头。`Ok(())` = 通过；`Err(人话原因)` = 拒绝。
/// 方案名不分大小写（RFC 7235），token 部分定长常数时间比较（长度不等直接拒，
/// 等长逐字节异或折叠，比较时间不随内容漂移）。
pub fn check_bearer(auth_header: Option<&str>, expected: &str) -> Result<(), &'static str> {
    // 服务端凭证未就绪（配置缺失/损坏按默认值 → 空 token）：fail-closed，
    // 带什么都拒——空凭证绝不能变成「空令牌放行」。
    if expected.is_empty() {
        return Err("服务端访问凭证尚未就绪");
    }
    let Some(raw) = auth_header else {
        return Err("缺少访问凭证");
    };
    let Some((scheme, token)) = raw.split_once(' ') else {
        return Err("凭证格式不是 Bearer");
    };
    if !scheme.eq_ignore_ascii_case("Bearer") {
        return Err("凭证格式不是 Bearer");
    }
    if token.is_empty() {
        return Err("缺少访问凭证");
    }
    // 常数时间比较：长度不等直接拒（凭证长度 32 hex 定长，长度本就不泄露内容）；
    // 等长时逐字节异或折叠成单值再比较——比较耗时只随长度走，不随「第几位不同」走。
    if token.len() != expected.len() {
        return Err("访问凭证无效或已重生成");
    }
    let diff = token
        .bytes()
        .zip(expected.bytes())
        .fold(0u8, |acc, (a, b)| acc | (a ^ b));
    if diff != 0 {
        return Err("访问凭证无效或已重生成");
    }
    Ok(())
}

// ── 纯核心：SSE 短时票据（一次性 + 60 秒过期）───────────────────────────────

/// SSE 票据仓库（内存表；服务实例私有，重启即清空——票据本来只活 60 秒）。
/// 消费语义：**建连即消费**（取出即作废，二次消费拒）；过期即拒。
/// consume/prune 生产消费方在票 06 的 SSE 建连中间件（本票只落签发与纯核）。
#[derive(Default)]
pub struct SseTicketStore {
    inner: Mutex<HashMap<String, i64>>,
}

// 票 06 起 consume 在 SSE 建连（sse_handler）投产；len/is_empty/prune 仍只有
// 测试与签发顺路清理在用，逐方法豁免
impl SseTicketStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 表锁（锁毒化按原值续用：票据是短命提示性数据，不值得 panic）。
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, i64>> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// 签发一张票据：随机 ≥128bit，`now`（unix 秒）注入时钟。签发顺路清理过期项。
    pub fn issue(&self, now: i64) -> Result<String, String> {
        // 随机源与访问凭证同款（getrandom，OS 级）；票据 32 hex 字符
        let mut bytes = [0u8; TICKET_BYTES];
        getrandom::fill(&mut bytes).map_err(|e| format!("生成 SSE 票据失败（系统随机源不可用）: {e}"))?;
        let ticket: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        let mut table = self.lock();
        table.retain(|_, expires_at| *expires_at > now);
        table.insert(ticket.clone(), now + TICKET_TTL_SECS);
        Ok(ticket)
    }

    /// 消费票据（建连即消费）：存在且未过期 → 移除并返回 true；否则 false。
    pub fn consume(&self, ticket: &str, now: i64) -> bool {
        if ticket.is_empty() {
            return false;
        }
        let mut table = self.lock();
        match table.get(ticket) {
            // 有效窗 = [签发, 签发+TICKET_TTL_SECS) 秒；满 60 秒整即过期
            Some(&expires_at) if now < expires_at => {
                table.remove(ticket);
                true
            }
            // 过期项顺手清掉（表只增不减的兜底）
            Some(_) => {
                table.remove(ticket);
                false
            }
            None => false,
        }
    }

    /// 清理已过期票据，返回清理数（签发时顺路调用，防内存表无界增长）。
    #[allow(dead_code)] // 票 06 SSE 建连中间件消费
    pub fn prune(&self, now: i64) -> usize {
        let mut table = self.lock();
        let before = table.len();
        table.retain(|_, expires_at| *expires_at > now);
        before - table.len()
    }

    /// 当前在表票据数（测试与观测用）。
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

// ── 数据版本广播枢纽（票 06：写后广播的进程内单写点）────────────────────────

/// 一帧版本广播的负载：实例 epoch + 当前数据版本。桌面 `data-version` 事件与
/// SSE 帧（hello / version）共用同一形状（前端对账纯函数吃同一结构）。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct VersionFrame {
    pub epoch: String,
    pub version: u64,
}

/// 版本广播枢纽（tokio watch：每个 SSE 连接持一个订阅者，写侧单写即通知，
/// 不需要队列——对账只看最新值，中间值丢了由「版本落后即重拉」兜住）。
/// 不变式：hub 值 ≡ 库内计数——初值 = 建依赖时的库内计数，此后唯一写入口是
/// [`bump_and_publish`] / [`restore_bump_and_publish`]（bump 与发布同步）。
pub struct VersionHub {
    tx: tokio::sync::watch::Sender<VersionFrame>,
    /// 必须持有：tokio watch 在「接收者数归零」时关闭通道，此后 `send` 不更新
    /// 值只回 Err——没有 SSE 客户端在订时发生的版本 bump 会被静默丢掉，后连的
    /// 客户端会拿到陈旧的 hello 基线。留着这个原始接收者，`send` 恒可用。
    _keepalive: tokio::sync::watch::Receiver<VersionFrame>,
}

impl VersionHub {
    pub fn new(epoch: String, initial_version: u64) -> Self {
        let (tx, rx) = tokio::sync::watch::channel(VersionFrame {
            epoch,
            version: initial_version,
        });
        VersionHub {
            tx,
            _keepalive: rx,
        }
    }

    /// 当前帧（SSE hello 与 `GET /api/data-version` 用）。
    pub fn current(&self) -> VersionFrame {
        self.tx.borrow().clone()
    }

    /// 发布新版本（写侧；通知所有在订 SSE 连接）。
    pub fn publish(&self, version: u64) -> VersionFrame {
        let mut frame = self.tx.borrow().clone();
        frame.version = version;
        let _ = self.tx.send(frame.clone());
        frame
    }

    /// 订阅（SSE 连接各持一个；watch 语义：订阅即见当前值，此后每次发布一帧）。
    pub fn subscribe(&self) -> tokio::sync::watch::Receiver<VersionFrame> {
        self.tx.subscribe()
    }
}

/// 写命令成功后的版本自增 + 广播（票 06 的统一咽喉）。生产链路：桌面写命令与
/// HTTP 写命令（经 after_write 钩子）都汇到 lib.rs `trigger_after_write`，它调
/// 本函数完成「库内 +1 → hub 发布 → 桌面 data-version 事件（emit 在 lib.rs 侧）」。
/// 与写命令同一把库锁、锁外串行执行，等价「写事务内 +1」（规格 C）。
pub fn bump_and_publish(
    conn: &Mutex<Connection>,
    hub: &VersionHub,
) -> Result<VersionFrame, String> {
    let guard = conn.lock().map_err(|e| format!("库锁不可用: {e}"))?;
    let version = crate::data_version::bump(&guard)?;
    Ok(hub.publish(version))
}

/// 恢复完成后的版本对齐（票 06）：恢复可能换入计数器更小的旧备份，把新库计数
/// 抬到「max(新库计数, 本实例已广播最大值) + 1」，保证已连客户端对账必判
/// 「落后」→ 无条件刷新（版本号跨恢复单调，规格 C；桌面端另有 db-restored
/// 无条件刷新事件，两者并存）。
pub fn restore_bump_and_publish(
    conn: &Mutex<Connection>,
    hub: &VersionHub,
) -> Result<VersionFrame, String> {
    let floor = hub.current().version;
    let guard = conn.lock().map_err(|e| format!("库锁不可用: {e}"))?;
    let version = crate::data_version::bump_floor(&guard, floor)?;
    Ok(hub.publish(version))
}

// ── 纯核心：并发计量（普通 ≤8 与 SSE ≤4 分开计数）──────────────────────────

/// 计量种类：普通请求与 SSE 长连接各用各的额度。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitKind {
    Normal,
    /// SSE 额度的具名种类（生产建连走 [`ConnLimiter::try_acquire_sse`]，
    /// 本变体供 RAII 槽位测试与 in_flight 观测按种类取数）。
    #[allow(dead_code)] // 仅测试构造
    Sse,
}

/// 双计数并发限流器（纯原子量，无 tokio 依赖，注入上限直测）。
/// 槽位是 RAII：[`ConnLimiter::acquire`] 返回 [`Slot`]，Drop 时自动归还。
pub struct ConnLimiter {
    normal: AtomicUsize,
    sse: AtomicUsize,
    max_normal: usize,
    max_sse: usize,
}

/// 持有的并发槽位（Drop = 归还）。
pub struct Slot<'a> {
    limiter: &'a ConnLimiter,
    kind: LimitKind,
}

impl ConnLimiter {
    pub fn new(max_normal: usize, max_sse: usize) -> Self {
        ConnLimiter {
            normal: AtomicUsize::new(0),
            sse: AtomicUsize::new(0),
            max_normal,
            max_sse,
        }
    }

    /// 占一个槽位；额度满返回 None（调用方回 429）。
    pub fn acquire(&self, kind: LimitKind) -> Option<Slot<'_>> {
        let counter = match kind {
            LimitKind::Normal => &self.normal,
            LimitKind::Sse => &self.sse,
        };
        let max = match kind {
            LimitKind::Normal => self.max_normal,
            LimitKind::Sse => self.max_sse,
        };
        let ok = counter
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < max).then_some(n + 1)
            })
            .is_ok();
        ok.then(|| Slot { limiter: self, kind })
    }

    /// 归还槽位（Slot::drop 会调；测试里显式归还用）。
    fn release(&self, kind: LimitKind) {
        let counter = match kind {
            LimitKind::Normal => &self.normal,
            LimitKind::Sse => &self.sse,
        };
        let _ = counter.fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
            n.checked_sub(1).or(Some(0))
        });
    }

    /// 当前占用数（测试与观测用）。
    #[allow(dead_code)]
    pub fn in_flight(&self, kind: LimitKind) -> usize {
        match kind {
            LimitKind::Normal => self.normal.load(Ordering::Acquire),
            LimitKind::Sse => self.sse.load(Ordering::Acquire),
        }
    }

    /// 占一个 SSE 槽位（满则 false，调用方回 429）。SSE 槽随连接存活（可能
    /// 数小时），而 RAII [`Slot`] 的借用形态进不了 'static 的 SSE 响应流——
    /// 长连接用这对显式接口，归还走 [`Self::release_sse`]（流断开时由
    /// [`VersionStream`] Drop 调用；建连中途早退由 handler 显式归还）。
    pub fn try_acquire_sse(&self) -> bool {
        self.sse
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < self.max_sse).then_some(n + 1)
            })
            .is_ok()
    }

    /// 归还一个 SSE 槽位（配对 [`Self::try_acquire_sse`]；饱和回 0，绝不回绕）。
    pub fn release_sse(&self) {
        let _ = self.sse.fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
            n.checked_sub(1).or(Some(0))
        });
    }
}

impl Drop for Slot<'_> {
    fn drop(&mut self) {
        self.limiter.release(self.kind);
    }
}

// ── 纯核心：header 上限 ─────────────────────────────────────────────────────

/// header 上限检查：条数、单条（值）长度、名+值总体积。超限给原因（人话）。
pub fn header_limit_error(headers: &axum::http::HeaderMap) -> Option<&'static str> {
    if headers.len() > MAX_HEADER_COUNT {
        return Some("请求头条数超过上限");
    }
    let mut total = 0usize;
    for (name, value) in headers {
        if value.len() > MAX_HEADER_VALUE_BYTES {
            return Some("请求头单条超过长度上限");
        }
        total += name.as_str().len() + value.len();
        if total > MAX_HEADER_TOTAL_BYTES {
            return Some("请求头总体积超过上限");
        }
    }
    None
}

// ── 命令白名单注册表（deny-by-default）─────────────────────────────────────

/// 网页端命令白名单注册表：**不登记即 404**。
///
/// 票 05 登记记录类命令（规格 H 功能面：首页/打卡/历史/冬眠/巢况）；设置/
/// 管理/备份/更新**永不入表**。
///
/// ⚠ **永久禁入清单**（不得加入本表，测试 [`tests::registry_excludes_forbidden_commands`]
/// 钉死，票 03 评审 Important 的落点）：
/// - `get_webui_config`——响应含**访问凭证明文**；
/// - `save_webui_config`——可改受信网段/端口/开关（闸一安全配置）；
/// - `regenerate_token`——返回新凭证明文且可作废全部旧地址；
/// - `get_access_url`——响应就是含凭证的完整访问地址。
///
/// 与 lib.rs 各命令 doc 注释的禁入标记同源；网页端的功能面不含任何设置操作（规格 H）。
pub const WEBUI_COMMANDS: &[&str] = &[
    "health_check",
    // 首页：窝卡片墙（含超期投影）+ 地点下拉
    "list_colonies",
    "list_locations",
    // 打卡：字典读 + 提交（colony_month_records 为交互第三轮日历标记数据源，
    // 打卡面板与记录页在用，网页端同样可达）
    "list_actions",
    "list_foods",
    "log_care",
    "colony_month_records",
    // 历史记录：查 / 改 / 删
    "list_logs",
    "update_log",
    "delete_log",
    // 冬眠操作
    "start_hibernation",
    "confirm_wake",
    "add_past_hibernation",
    "update_expected_end",
    // 巢况登记：查 / 改 / 删 / 摘要（照片上传随票 08）
    "save_checkin",
    "list_checkins",
    "update_checkin",
    "delete_checkin",
    "get_checkin_digest",
];

/// 写命令成功后的副作用钩子（票 05）：参数 = 是否同时刷新托盘 tooltip。
/// 生产在 lib.rs setup 接线（`refresh_tray_tooltip` + `trigger_after_write`，
/// 与桌面写命令收尾两件套同一语义）；测试注入计数器断言。
pub type AfterWriteHook = std::sync::Arc<dyn Fn(bool) + std::marker::Send + std::marker::Sync>;

/// 前端静态资源查找（票 05）：dist 产物相对路径 → 字节；None = 无此资源。
/// 生产 = Tauri 嵌入资源（frontendDist 打进二进制，asset resolver 取出）；
/// 测试 = 临时目录注入；`tauri dev` 期嵌入资源为空 → 静态路由 404（开发期
/// 浏览器走 devUrl，本路径仅为打包后场景服务）。
pub type AssetLookup = std::sync::Arc<dyn Fn(&str) -> Option<Vec<u8>> + std::marker::Send + std::marker::Sync>;

/// 命令派发结果（票 05 起三态分明）：
/// - [`CmdOutcome::Ok`] → 200（返回体 = 命令返回值，与桌面 invoke 同形）；
/// - [`CmdOutcome::Rejected`] → 400（schema/输入校验层拒绝，**未进库**——
///   「越权/畸形参数一律 400」验收口径）；
/// - [`CmdOutcome::Failed`] → 500（执行失败，桌面 invoke 的同款人话错误串）。
#[derive(Debug, Clone, PartialEq)]
pub enum CmdOutcome {
    Ok(serde_json::Value),
    Rejected(String),
    Failed(String),
}

/// 注册表派发：登记且有实现 → Some(结果)；未登记 → None（404）。
/// 命令执行与桌面 IPC 同一把库锁（[`crate::run_with_conn`]），单连接串行。
/// 入参 `args` 是请求体里的 args 对象（键 camelCase，schema 层映射）。
pub fn dispatch_command(
    deps: &SharedDeps,
    cmd: &str,
    args: &serde_json::Value,
) -> Option<CmdOutcome> {
    use crate::webui_args::ValidatedArgs;

    /// 读命令：解析 schema（400）→ 同一把库锁执行（500）。
    fn read_cmd<A, T, F>(deps: &SharedDeps, args: &serde_json::Value, f: F) -> CmdOutcome
    where
        A: ValidatedArgs,
        T: serde::Serialize,
        F: FnOnce(&Connection, A) -> Result<T, String>,
    {
        let parsed = match parse_validated::<A>(args) {
            Ok(a) => a,
            Err(msg) => return CmdOutcome::Rejected(msg),
        };
        to_outcome(crate::run_with_conn(&deps.conn, |conn| f(conn, parsed)))
    }

    /// 写命令：同读命令，成功后触发桌面语义的写后副作用（托盘 + 自动备份）。
    fn write_cmd<A, T, F>(
        deps: &SharedDeps,
        args: &serde_json::Value,
        with_tray: bool,
        f: F,
    ) -> CmdOutcome
    where
        A: ValidatedArgs,
        T: serde::Serialize,
        F: FnOnce(&Connection, A) -> Result<T, String>,
    {
        let parsed = match parse_validated::<A>(args) {
            Ok(a) => a,
            Err(msg) => return CmdOutcome::Rejected(msg),
        };
        let outcome = crate::run_with_conn(&deps.conn, |conn| f(conn, parsed));
        match outcome {
            Ok(v) => {
                // 桌面语义（lib.rs 写命令收尾）：成功才触发；钩子在库锁释放后
                // 调用（trigger_after_write 的后台备份要重新拿锁，持锁调会自锁）
                (deps.after_write)(with_tray);
                CmdOutcome::Ok(serde_json::to_value(v).unwrap_or(serde_json::Value::Null))
            }
            Err(e) => CmdOutcome::Failed(e),
        }
    }

    match cmd {
        // 健康检查与桌面 `health_check` 命令同源：同一把库锁里查 schema 版本
        "health_check" => Some(to_outcome(crate::run_with_conn(&deps.conn, |conn| {
            crate::db::schema_version_of(conn)
                .map(|schema_version| serde_json::json!({ "schema_version": schema_version }))
                .map_err(|e| e.to_string())
        }))),
        // ── 首页 ──
        "list_colonies" => Some(read_cmd(deps, args, |conn, _: webui_args::EmptyArgs| {
            crate::colony::list_colonies(conn, &crate::colony::today_iso())
        })),
        "list_locations" => Some(read_cmd(deps, args, |conn, _: webui_args::EmptyArgs| {
            crate::colony::list_locations(conn)
        })),
        // ── 打卡 ──
        "list_actions" => Some(read_cmd(deps, args, |conn, _: webui_args::EmptyArgs| {
            crate::dict::list_actions(conn)
        })),
        "list_foods" => Some(read_cmd(deps, args, |conn, _: webui_args::EmptyArgs| {
            crate::care::list_foods(conn)
        })),
        "log_care" => Some(write_cmd(
            deps,
            args,
            true,
            |conn, a: webui_args::LogCareArgs| crate::care::log_care(conn, &a.input.into_core(), &crate::care::now_local()),
        )),
        "colony_month_records" => Some(read_cmd(
            deps,
            args,
            |conn, a: webui_args::ColonyMonthRecordsArgs| {
                crate::care::colony_month_records(
                    conn,
                    a.colony_id,
                    a.year,
                    a.month,
                    a.exclude_log_id,
                )
            },
        )),
        // ── 历史记录 ──
        "list_logs" => Some(read_cmd(deps, args, |conn, a: webui_args::ListLogsArgs| {
            crate::care::list_logs(conn, &a.filter.into_core())
        })),
        "update_log" => Some(write_cmd(
            deps,
            args,
            true,
            |conn, a: webui_args::UpdateLogArgs| {
                crate::care::update_log(conn, a.id, &a.input.into_core(), &crate::care::now_local())
            },
        )),
        "delete_log" => Some(write_cmd(deps, args, true, |conn, a: webui_args::IdArgs| {
            crate::care::delete_log(conn, a.id)
        })),
        // ── 冬眠 ──
        "start_hibernation" => Some(write_cmd(
            deps,
            args,
            true,
            |conn, a: webui_args::StartHibernationArgs| {
                crate::hibernation::start_hibernation(
                    conn,
                    a.colony_id,
                    &a.start_date,
                    &a.expected_end_date,
                    &crate::colony::today_iso(),
                )
            },
        )),
        "confirm_wake" => Some(write_cmd(
            deps,
            args,
            true,
            |conn, a: webui_args::ConfirmWakeArgs| {
                crate::hibernation::confirm_wake(
                    conn,
                    a.colony_id,
                    &a.actual_end_date,
                    &crate::colony::today_iso(),
                )
            },
        )),
        "add_past_hibernation" => Some(write_cmd(
            deps,
            args,
            true,
            |conn, a: webui_args::AddPastHibernationArgs| {
                crate::hibernation::add_past_hibernation(conn, a.colony_id, &a.start_date, &a.end_date)
            },
        )),
        "update_expected_end" => Some(write_cmd(
            deps,
            args,
            true,
            |conn, a: webui_args::UpdateExpectedEndArgs| {
                crate::hibernation::update_expected_end(conn, a.colony_id, &a.new_expected_end_date)
            },
        )),
        // ── 巢况登记（永不参与提醒/催促：写命令不刷托盘，只触发自动备份）──
        "save_checkin" => Some(write_cmd(
            deps,
            args,
            false,
            |conn, a: webui_args::SaveCheckinArgs| {
                crate::nest_checkin::save_checkin(
                    conn,
                    &a.input.into_core(),
                    &crate::colony::today_iso(),
                    &crate::care::now_local(),
                )
            },
        )),
        "list_checkins" => Some(read_cmd(deps, args, |conn, a: webui_args::ColonyIdArgs| {
            crate::nest_checkin::list_checkins(conn, a.colony_id)
        })),
        "update_checkin" => Some(write_cmd(
            deps,
            args,
            false,
            |conn, a: webui_args::UpdateCheckinArgs| {
                crate::nest_checkin::update_checkin(
                    conn,
                    a.id,
                    &a.input.into_core(),
                    &crate::colony::today_iso(),
                )
            },
        )),
        "delete_checkin" => Some(write_cmd(deps, args, false, |conn, a: webui_args::IdArgs| {
            // 票 07：与桌面 delete_checkin 同序——先库事务删元数据提交，再删照片
            // 文件；文件删失败仅孤儿（下轮巡检隔离），不回滚库。巢况写命令不刷托盘。
            let rel_paths = crate::photo::collect_checkin_photo_paths(conn, a.id)?;
            crate::nest_checkin::delete_checkin(conn, a.id)?;
            let photos_root = deps.data_dir.join(crate::photo::PHOTOS_DIR_NAME);
            for f in crate::photo::delete_photo_files(&photos_root, &rel_paths) {
                crate::applog::log_error(&format!("照片文件删除失败（遗留孤儿，待巡检隔离）: {f}"));
            }
            Ok(())
        })),
        "get_checkin_digest" => Some(read_cmd(deps, args, |conn, a: webui_args::ColonyIdArgs| {
            crate::nest_checkin::digest_for_colony(conn, a.colony_id, &crate::colony::today_iso())
        })),
        // 不存在「注册表里有但这里没有」的分支——registry_entries_all_have_real_dispatch
        // 钉住两边同步；走到这等于调用方没先查注册表
        _ => None,
    }
}

/// schema 解析 + 取值校验（两段都过才进库；任一失败 = 400 人话）。
fn parse_validated<A: webui_args::ValidatedArgs>(
    args: &serde_json::Value,
) -> Result<A, String> {
    let a: A = webui_args::parse(args)?;
    a.validate()?;
    Ok(a)
}

/// 纯核 Result → 派发结果（序列化失败理论外，兜底 null）。
fn to_outcome<T: serde::Serialize>(result: Result<T, String>) -> CmdOutcome {
    match result {
        Ok(v) => CmdOutcome::Ok(serde_json::to_value(v).unwrap_or(serde_json::Value::Null)),
        Err(e) => CmdOutcome::Failed(e),
    }
}

// ── 共享依赖与服务状态 ──────────────────────────────────────────────────────

/// HTTP 服务共享依赖（axum State；一个服务实例一份）。
pub struct SharedDeps {
    /// 与桌面 IPC 同一把库锁（单连接；HTTP 派发与 Tauri command 串行共存）。
    pub conn: Arc<Mutex<Connection>>,
    /// 数据目录（webui-config.json 所在；闸二每请求读凭证/网段）。
    pub data_dir: PathBuf,
    /// SSE 短时票据仓库（票 06 的 SSE 建连消费）。
    pub tickets: SseTicketStore,
    /// 并发计量（普通 ≤8 / SSE ≤4）。
    pub limiter: ConnLimiter,
    /// 连接层 header 读超时（默认 30 秒；测试注入短值）。
    pub read_timeout: Duration,
    /// 写命令成功后的副作用钩子（票 05，桌面语义；默认空操作，生产 lib.rs 接线）。
    pub after_write: AfterWriteHook,
    /// 前端静态资源查找（票 05；默认无资源=静态路由 404，生产 lib.rs 接线）。
    pub frontend_assets: AssetLookup,
    /// 数据版本广播枢纽（票 06：初值 = 库内计数；SSE 连接订阅它收推送，
    /// lib.rs 的写后/恢复链路经 bump_and_publish 系列同步发布）。
    pub versions: VersionHub,
}

impl SharedDeps {
    /// 空钩子构造（测试用；生产走 [`Self::with_hooks`] 接真实副作用与静态资源）。
    #[allow(dead_code)] // 测试构建在用；生产构造见 lib.rs setup 的 with_hooks
    pub fn new(conn: Arc<Mutex<Connection>>, data_dir: PathBuf) -> Self {
        SharedDeps::with_hooks(
            conn,
            data_dir,
            Arc::new(|_with_tray| {}),
            Arc::new(|_path| None),
        )
    }

    /// 带钩子构造（生产接线用：写后副作用 + 前端静态资源）。
    /// 顺路完成票 06 的两件初始化：epoch（安装 id + 进程启动毫秒；安装 id 文件
    /// 缺失则此刻生成落盘）与 hub 初值（锁内读库内计数；锁毒化按 0 起步——
    /// 版本是提示性数据，不值得让服务起不来）。
    pub fn with_hooks(
        conn: Arc<Mutex<Connection>>,
        data_dir: PathBuf,
        after_write: AfterWriteHook,
        frontend_assets: AssetLookup,
    ) -> Self {
        let epoch = {
            let install_id = crate::data_version::load_or_create_install_id(&data_dir);
            crate::data_version::epoch_of(&install_id, crate::data_version::process_start_millis())
        };
        let initial_version = conn
            .lock()
            .map(|conn| crate::data_version::get(&conn))
            .unwrap_or(0);
        SharedDeps {
            conn,
            data_dir,
            tickets: SseTicketStore::new(),
            limiter: ConnLimiter::new(MAX_CONCURRENT_REQUESTS, MAX_CONCURRENT_SSE),
            read_timeout: READ_TIMEOUT,
            after_write,
            frontend_assets,
            versions: VersionHub::new(epoch, initial_version),
        }
    }
}

type Shared = Arc<SharedDeps>;

// ── HTTP 层：响应助手 ───────────────────────────────────────────────────────

fn json_response(status: axum::http::StatusCode, value: &serde_json::Value) -> axum::response::Response {
    let body = serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string());
    (
        status,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        body,
    )
        .into_response()
}

/// 统一错误体：`{"error": <人话>}`（前端调用层非 2xx 以 body.error 字符串 reject，
/// 与桌面 invoke 的字符串错误同形——src/lib/ipc.ts 契约）。
fn json_error(status: axum::http::StatusCode, message: &str) -> axum::response::Response {
    json_response(status, &serde_json::json!({ "error": message }))
}

/// 从请求扩展里取 socket 对端 IP（accept 循环逐连接注入；缺失按 unknown 记日志）。
fn peer_ip(req: &axum::extract::Request) -> String {
    req.extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0.ip().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

// ── HTTP 层：中间件（自外向内 = 限额 → 超时 → 闸二）────────────────────────

/// 最外层：header 上限 → 请求体大小预检 → 普通并发槽（429/413/431 人话错误）。
async fn limits_mw(
    State(deps): State<Shared>,
    req: axum::extract::Request,
    next: Next,
) -> axum::response::Response {
    let peer = peer_ip(&req);
    if let Some(reason) = header_limit_error(req.headers()) {
        crate::applog::log_error(&format!("网页端拒绝来源 {peer}（{reason}）"));
        return json_error(axum::http::StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE, reason);
    }
    // 体上限按路径放宽（票 08）：照片上传端点放宽到 9×15MB+multipart 框架开销，
    // 其余路径维持全局 1MB。这是 Content-Length 有值时的前置预检；分块编码等
    // 残余路径由路由层 DefaultBodyLimit 与端点自身的流式闸兜底。
    let max_body = if req.uri().path() == PHOTO_UPLOAD_PATH {
        MAX_PHOTOS_BODY_BYTES
    } else {
        MAX_BODY_BYTES
    };
    if let Some(len) = req
        .headers()
        .get(axum::http::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<usize>().ok())
    {
        if len > max_body {
            let reason = format!("请求体超过 {}MB 上限", max_body / 1024 / 1024);
            crate::applog::log_error(&format!("网页端拒绝来源 {peer}（{reason}）"));
            return json_error(axum::http::StatusCode::PAYLOAD_TOO_LARGE, &reason);
        }
    }
    let Some(slot) = deps.limiter.acquire(LimitKind::Normal) else {
        let reason = format!("并发请求已达上限（{MAX_CONCURRENT_REQUESTS}），请稍后再试");
        crate::applog::log_error(&format!("网页端拒绝来源 {peer}（{reason}）"));
        return json_error(axum::http::StatusCode::TOO_MANY_REQUESTS, &reason);
    };
    let resp = next.run(req).await;
    drop(slot); // 响应头已产出即归还槽位（本票响应都是整包 JSON）
    resp
}

/// 请求处理超时（读超时的 handler 侧：慢请求最多占 [`READ_TIMEOUT`] 30 秒；
/// `POST /api/photos` 按路径放宽到 [`PHOTO_UPLOAD_TIMEOUT`] 300 秒——慢 Wi-Fi
/// 大批量照片，终局评审）。
/// 注意：SSE 长连接（票 06）的建连响应立即可返回，不受此层牵连。
async fn timeout_mw(
    req: axum::extract::Request,
    next: Next,
) -> axum::response::Response {
    let limit = request_timeout_for(req.method(), req.uri().path());
    match tokio::time::timeout(limit, next.run(req)).await {
        Ok(resp) => resp,
        Err(_) => json_error(
            axum::http::StatusCode::REQUEST_TIMEOUT,
            &format!("请求处理超时（{} 秒内未完成）", limit.as_secs()),
        ),
    }
}

/// 闸二：除健康检查外的所有路径要求 `Authorization: Bearer <token>`。
///
/// 取舍：**每请求从 webui-config.json 现读凭证**（不缓存）——重生成即刻生效
/// 拒旧值是规格 User Story 3 的硬要求，缓存 + 写入失效钩子要多维护一条同步
/// 路径，漏一处就是「旧凭证继续可用」的事故；而配置文件 <1KB，≤8 并发下每
/// 请求读盘的开销可忽略，顺带让网段/开关改动也即刻生效。读失败按默认值
/// （空凭证）处理 = fail-closed。
async fn auth_mw(
    State(deps): State<Shared>,
    req: axum::extract::Request,
    next: Next,
) -> axum::response::Response {
    let cfg = webui_config::load(&deps.data_dir);
    let auth = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());
    if let Err(reason) = check_bearer(auth, &cfg.token) {
        let peer = peer_ip(&req);
        // 只记来源与原因，绝不记 token 值（票面纪律）
        crate::applog::log_error(&format!("网页端拒绝来源 {peer}（{reason}）"));
        let mut resp = json_error(axum::http::StatusCode::UNAUTHORIZED, reason);
        resp.headers_mut().insert(
            axum::http::header::WWW_AUTHENTICATE,
            axum::http::HeaderValue::from_static("Bearer"),
        );
        return resp;
    }
    next.run(req).await
}

// ── HTTP 层：本票登记的端点 ─────────────────────────────────────────────────

/// `GET /api/health`：健康检查（**仅豁免闸二**——连通性探测不带凭证也能用；
/// 响应只有 schema 版本，无敏感信息）。闸一照拦（连接准入在前），限额与超时
/// 照挂（普通并发上限 8 无端点豁免——免凭证端点更要防洪泛，见 build_router）。
async fn health_handler(State(deps): State<Shared>) -> axum::response::Response {
    dispatch_on_blocking(deps, "health_check".to_string(), serde_json::json!({})).await
}

fn respond_dispatch(outcome: Option<CmdOutcome>) -> axum::response::Response {
    use axum::http::StatusCode;
    match outcome {
        Some(CmdOutcome::Ok(v)) => json_response(StatusCode::OK, &v),
        // 输入校验层拒绝：畸形参数一律 400（验收口径），错误体 = 人话
        Some(CmdOutcome::Rejected(msg)) => json_error(StatusCode::BAD_REQUEST, &msg),
        // 执行失败：桌面 invoke 同款人话错误串（业务拒绝/库错误）
        Some(CmdOutcome::Failed(e)) => json_error(StatusCode::INTERNAL_SERVER_ERROR, &e),
        None => json_error(StatusCode::NOT_FOUND, "未知命令或桌面端专属功能"),
    }
}

/// 同步派发挪到 blocking 池执行：`dispatch_command` 与桌面 IPC 共用同一把库锁，
/// 锁等待必须占 blocking 线程而非 tokio worker——否则 ≤8 核机器上 8 个并发请求
/// 各占死一个 worker，整个网页端（accept/限流 429/SSE 保活）齐停到锁释放
/// （4 核 CI runner 确定性复现，回归钉 health_is_subject_to_concurrency_limit；
/// 先例：照片解码同走 spawn_blocking）。
async fn dispatch_on_blocking(
    deps: Shared,
    cmd: String,
    args: serde_json::Value,
) -> axum::response::Response {
    let joined =
        tauri::async_runtime::spawn_blocking(move || dispatch_command(&deps, &cmd, &args)).await;
    match joined {
        Ok(outcome) => respond_dispatch(outcome),
        Err(e) => json_error(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            &format!("命令执行线程异常: {e}"),
        ),
    }
}

/// `POST /api/cmd`：统一命令端点。体 `{"cmd":<命令名>,"args":{...}}`（与桌面
/// invoke 的入参对象同形），响应体 = 命令返回值 JSON；非 2xx = `{"error":..}`。
/// deny-by-default：不在 [`WEBUI_COMMANDS`] 的命令一律 404。
async fn cmd_handler(
    State(deps): State<Shared>,
    body: axum::body::Bytes,
) -> axum::response::Response {
    use axum::http::StatusCode;
    if body.len() > MAX_BODY_BYTES {
        // content-length 有值时已在 limits_mw 拦下；这里兜住分块编码等残余路径
        return json_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            &format!("请求体超过 {}MB 上限", MAX_BODY_BYTES / 1024 / 1024),
        );
    }
    let parsed: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => return json_error(StatusCode::BAD_REQUEST, "请求体不是合法 JSON"),
    };
    let Some(cmd) = parsed.get("cmd").and_then(|c| c.as_str()) else {
        return json_error(
            StatusCode::BAD_REQUEST,
            "请求体缺少 cmd 字段（应为 {\"cmd\":命令名,\"args\":{}}）",
        );
    };
    // args 键名与桌面 invoke 入参对象一致（顶层 camelCase）；缺省/null 按 {}
    // 处理，交给 schema 层判定缺字段
    let args = parsed
        .get("args")
        .cloned()
        .filter(|v| !v.is_null())
        .unwrap_or_else(|| serde_json::json!({}));
    if !WEBUI_COMMANDS.contains(&cmd) {
        // deny-by-default 的常态 404，不刷日志（网段外根本进不来，段内探测
        // 不构成安全事件；真正的准入拒绝在闸一/闸二层记流水）。错误串点明
        // 「也可能是桌面端专属命令」——网页端撞上的多半是这种情况。
        return json_error(StatusCode::NOT_FOUND, "未知命令或桌面端专属功能");
    }
    dispatch_on_blocking(deps, cmd.to_string(), args).await
}

/// `POST /api/sse-ticket`：主凭证换 60 秒一次性票据（闸二已在此路径生效）。
/// 票据只在内存表；SSE 建连时消费（票 06），客户端 error 后重取再建连（规格 B）。
async fn sse_ticket_handler(State(deps): State<Shared>) -> axum::response::Response {
    let now = chrono::Utc::now().timestamp();
    match deps.tickets.issue(now) {
        Ok(ticket) => json_response(
            axum::http::StatusCode::OK,
            &serde_json::json!({ "ticket": ticket, "expires_in": TICKET_TTL_SECS }),
        ),
        Err(e) => json_error(axum::http::StatusCode::INTERNAL_SERVER_ERROR, &e),
    }
}

// ── HTTP 层：SSE 端点与数据版本查询（票 06）────────────────────────────────

/// `GET /api/sse` 的查询参数解析（手动拆查询串，不引 axum query feature）：
/// EventSource 建连带不了 Authorization 头，闸二凭证在 `/api/sse-ticket` 已
/// 验证过，这里只取一次性票据。票据是 32 hex 字符，无须百分号解码。
fn parse_sse_ticket(uri: &axum::http::Uri) -> String {
    uri.query()
        .and_then(|q| {
            q.split('&')
                .find_map(|kv| kv.strip_prefix("ticket="))
                .map(str::to_string)
        })
        .unwrap_or_default()
}

/// `GET /api/data-version` 的应答帧：库内计数真值 + 实例 epoch。
fn version_payload(deps: &SharedDeps, version: u64) -> serde_json::Value {
    serde_json::json!({ "epoch": deps.versions.current().epoch, "version": version })
}

/// `GET /api/data-version`（规格 A 白名单「数据版本」）：库内计数真值 + epoch。
/// 走闸二（主凭证）；SSE 建不上的客户端也可轮询它对账（前端当前只用 SSE）。
async fn data_version_handler(State(deps): State<Shared>) -> axum::response::Response {
    // 同 dispatch_on_blocking：库锁读取挪 blocking 池，不占 tokio worker
    let joined = tauri::async_runtime::spawn_blocking(move || {
        crate::run_with_conn(&deps.conn, |conn| Ok(crate::data_version::get(conn)))
            .map(|version| version_payload(&deps, version))
    })
    .await;
    match joined {
        Ok(Ok(payload)) => json_response(axum::http::StatusCode::OK, &payload),
        Ok(Err(e)) => json_error(axum::http::StatusCode::INTERNAL_SERVER_ERROR, &e),
        Err(e) => json_error(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            &format!("版本读取线程异常: {e}"),
        ),
    }
}

/// SSE 帧序列化：`{type:"hello"|"version", epoch, version}`（data: 行单行 JSON）。
fn sse_event(kind: &str, frame: &VersionFrame) -> axum::response::sse::Event {
    let body = serde_json::json!({ "type": kind, "epoch": frame.epoch, "version": frame.version });
    axum::response::sse::Event::default().data(body.to_string())
}

/// SSE 帧流：首帧 `hello`（当前值，对账基线），此后每次版本发布一帧 `version`。
/// 内核是 tokio_stream 的 WatchStream（订阅即得当前值 + 每次发布一帧）；持有
/// `Shared` 克隆：Drop 时归还 SSE 槽位（连接活多久槽占多久——RAII [`Slot`] 的
/// 借用进不了 'static 的响应体，见 [`ConnLimiter::try_acquire_sse`]）。
struct VersionStream {
    deps: Shared,
    inner: tokio_stream::wrappers::WatchStream<VersionFrame>,
    hello_sent: bool,
}

impl Drop for VersionStream {
    fn drop(&mut self) {
        self.deps.limiter.release_sse();
    }
}

impl futures_core::Stream for VersionStream {
    type Item = Result<axum::response::sse::Event, std::convert::Infallible>;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        // 全字段 Unpin，直接 get_mut
        let this = self.get_mut();
        match std::pin::Pin::new(&mut this.inner).poll_next(cx) {
            std::task::Poll::Ready(Some(frame)) => {
                let kind = if this.hello_sent {
                    "version"
                } else {
                    this.hello_sent = true;
                    "hello"
                };
                std::task::Poll::Ready(Some(Ok(sse_event(kind, &frame))))
            }
            // 发送端没了（理论外：deps 与流同生共死）→ 结束流，连接收尾
            std::task::Poll::Ready(None) => std::task::Poll::Ready(None),
            std::task::Poll::Pending => std::task::Poll::Pending,
        }
    }
}

/// SSE 建连的轻量闸：header 上限检查。不占普通并发槽（SSE 在 handler 内单独
/// 计量 ≤4）；**不走闸二**——EventSource 建连带不了 Authorization 头，凭证换
/// 一次性票据（规格 B），票据在 [`sse_handler`] 内建连即消费。
async fn sse_gate_mw(req: axum::extract::Request, next: Next) -> axum::response::Response {
    if let Some(reason) = header_limit_error(req.headers()) {
        let peer = peer_ip(&req);
        crate::applog::log_error(&format!("网页端拒绝来源 {peer}（{reason}）"));
        return json_error(
            axum::http::StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE,
            reason,
        );
    }
    next.run(req).await
}

/// `GET /api/sse?ticket=<一次性票据>`（票 06）：建连即消费票据（无效 → 401），
/// 首帧立即推 `{type:"hello", epoch, version}`，此后每次版本 bump 广播
/// `{type:"version", ...}`。连接计入 SSE 单独额度（≤4，规格 A）。
///
/// 读超时语义：本端点挂在 [`timeout_mw`] 下，但那只覆盖建连阶段（响应头产出
/// 即返回）；流式响应体在中间件链返回后才由 hyper 消费，**30 秒读超时不掐
/// SSE 长连接**（规格 B：读超时只对首字节/建连阶段应用）。断开由客户端断连、
/// 服务优雅关停（3 秒宽限）或 SSE 槽位规则兜底。
async fn sse_handler(
    State(deps): State<Shared>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    uri: axum::http::Uri,
) -> axum::response::Response {
    let ticket = parse_sse_ticket(&uri);
    // 槽位先行：满员 429 时不消费票据（客户端可原票重试一次）
    if !deps.limiter.try_acquire_sse() {
        let reason = format!("SSE 连接数已达上限（{MAX_CONCURRENT_SSE}），请稍后再试");
        crate::applog::log_error(&format!("网页端拒绝来源 {}（{reason}）", peer.ip()));
        return json_error(axum::http::StatusCode::TOO_MANY_REQUESTS, &reason);
    }
    // 建连即消费（一次性）：二次消费（EventSource 原生自动重连没有新票据）必拒，
    // 前端 onerror 里 close 后重取票据重建连（ipc.ts）
    if !deps.tickets.consume(&ticket, chrono::Utc::now().timestamp()) {
        deps.limiter.release_sse(); // 早退：槽位当场归还（长连接槽在流 Drop 归还）
        crate::applog::log_error(&format!(
            "网页端拒绝来源 {}（SSE 票据无效或已过期）",
            peer.ip()
        ));
        return json_error(
            axum::http::StatusCode::UNAUTHORIZED,
            "SSE 票据无效或已过期，请刷新页面重试",
        );
    }
    let stream = VersionStream {
        inner: tokio_stream::wrappers::WatchStream::new(deps.versions.subscribe()),
        deps,
        hello_sent: false,
    };
    axum::response::sse::Sse::new(stream)
        // 15 秒注释帧保活：NAT/路由器不掐空闲连接（对账只认 data 帧，注释帧前端不解析）
        .keep_alive(
            axum::response::sse::KeepAlive::new()
                .interval(Duration::from_secs(15))
                .text("keep-alive"),
        )
        .into_response()
}

// ── HTTP 层：照片端点（票 08，规格 E）────────────────────────────────────────

/// `GET /api/photo/{colonyId}/{file}` 路径 grammar（严格）：窝段纯数字（≤19 位，
/// i64 rowid 形态）；文件段 = 36 位 `[A-Za-z0-9-]` + `.jpg`（服务端 UUID 落盘
/// 口径，36 位由 photo::random_uuid 的产物钉住）。axum Path 已做百分号解码，
/// 本 grammar 是最后一道闸：`..`/反斜杠/冒号/绝对路径/多余段全过不了段数与
/// 字符集检查，`..%2F` 之类编码花样解码后同样被字符集拒；只从 photos/ 根内
/// 按段拼接，无任何穿越面。
fn photo_url_parts_valid(colony_id: &str, file_name: &str) -> bool {
    if colony_id.is_empty() || colony_id.len() > 19 || !colony_id.bytes().all(|b| b.is_ascii_digit())
    {
        return false;
    }
    let Some(base) = file_name.strip_suffix(".jpg") else {
        return false;
    };
    base.len() == 36 && base.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// 照片响应头三件套（规格 E）：类型钉死 JPEG + `nosniff` 禁 MIME 嗅探 +
/// `inline` 只以图片身份渲染（禁下载/禁当文档内嵌）。
fn photo_response_headers() -> [(&'static str, &'static str); 3] {
    [
        ("Content-Type", "image/jpeg"),
        ("X-Content-Type-Options", "nosniff"),
        ("Content-Disposition", "inline"),
    ]
}

/// multipart 里收到的一段原始文件（未过校验链）。
struct RawPhoto {
    original_name: Option<String>,
    bytes: Vec<u8>,
}

/// 单个 multipart 字段的流式读取：逐 chunk 累积，超 `cap` 即停（任意大文件
/// 不会整段进内存）。错误信息由调用方按字段类型给（人话）。
async fn read_field_capped(
    field: &mut axum::extract::multipart::Field<'_>,
    cap: usize,
    over_cap_message: &str,
) -> Result<Vec<u8>, String> {
    let mut buf = Vec::new();
    while let Some(chunk) = field
        .chunk()
        .await
        .map_err(|e| format!("上传数据读取失败: {e}"))?
    {
        if buf.len() + chunk.len() > cap {
            return Err(over_cap_message.to_string());
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(buf)
}

/// 把当前字段剩余字节耗尽（chunk 级丢弃，不累计内存）——出错后继续解析流的
/// 排干手段，让客户端稳定拿到 400 人话而不是半路被掐断的连接。
async fn drain_field(field: &mut axum::extract::multipart::Field<'_>) {
    while field.chunk().await.ok().flatten().is_some() {}
}

/// multipart 解析：`checkinId` 文本段 + `photos` 文件段（≤9 张，逐张流式体积
/// 闸 ≤15MB）。任何坏形状（缺 checkinId/超张数/超单张/未知字段）返回人话
/// Err，由调用方统一回 400——先把剩余体排干再回，413 半路断连的形态只留给
/// limits_mw 的 Content-Length 预检。
async fn read_multipart_photos(
    mut multipart: axum::extract::Multipart,
) -> Result<(i64, Vec<RawPhoto>), String> {
    let mut checkin_id: Option<i64> = None;
    let mut photos: Vec<RawPhoto> = Vec::new();
    let mut error: Option<String> = None;
    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|e| format!("上传数据不是合法的 multipart 表单: {e}"))?
    {
        if error.is_some() {
            drain_field(&mut field).await; // 已有错在身：耗尽字节即走，不再解析
            continue;
        }
        match field.name() {
            Some("checkinId") => {
                let read = read_field_capped(&mut field, 64, "checkinId 字段超长").await;
                match read {
                    Ok(bytes) => {
                        let text = String::from_utf8_lossy(&bytes).trim().to_string();
                        match text.parse::<i64>() {
                            Ok(v) if v >= 1 => checkin_id = Some(v),
                            _ => error = Some(format!("checkinId 必须是正整数（收到 {text}）")),
                        }
                    }
                    Err(e) => error = Some(e),
                }
            }
            Some("photos") => {
                if photos.len() >= crate::photo::MAX_PHOTOS_PER_SUBMIT {
                    error = Some(format!(
                        "一次最多上传 {} 张照片",
                        crate::photo::MAX_PHOTOS_PER_SUBMIT
                    ));
                    drain_field(&mut field).await;
                    continue;
                }
                let original_name = field.file_name().map(str::to_string);
                let message = format!(
                    "单张照片压缩前不能超过 {}MB",
                    crate::photo::MAX_INPUT_BYTES / 1024 / 1024
                );
                match read_field_capped(&mut field, crate::photo::MAX_INPUT_BYTES, &message).await
                {
                    Ok(bytes) => photos.push(RawPhoto {
                        original_name,
                        bytes,
                    }),
                    Err(e) => error = Some(e),
                }
            }
            _ => {
                error = Some("表单包含不认识的字段（只收 checkinId 与 photos）".to_string());
                drain_field(&mut field).await;
            }
        }
    }
    if let Some(e) = error {
        return Err(e);
    }
    match checkin_id {
        Some(id) => Ok((id, photos)),
        None => Err("缺少 checkinId 表单字段".to_string()),
    }
}

/// 上传失败的两态（票 08 验收口径）：**内容坏**（票 07 校验链拒——格式/尺寸/
/// 解码失败，客户端给的垃圾 → 400 人话）与**服务端失败**（库/盘/任务异常，
/// 与 /api/cmd 的业务错误同口径 → 500 人话）。
enum UploadRejection {
    BadRequest(String),
    Failed(String),
}

/// `POST /api/photos`（票 08，规格 E「桌面/网页同一套校验链」）：multipart
/// `checkinId` + ≤9 张照片，字节过 [`crate::photo::process_uploads`] 纯核
///（魔数白名单/头部尺寸/解码炸弹/重编码 JPEG 清 EXIF），落盘名一律服务端
/// UUID（[`crate::photo::attach_photos`] 写入协议：tmp+fsync+原子 rename+插库，
/// 失败不留半截），成功触发写后钩子（版本 bump + 自动备份；巢况不刷托盘）。
///
/// 取舍：**整段在内存处理，不经临时文件**——单张 ≤15MB、单次 ≤9 张，批量内存
/// 峰值 ≤135MB 与桌面 read_photo_files（整读文件进内存）同包络；超限在
/// multipart 流式读取时前置拦截，不会先囤满再拒。解码/重编码是 CPU 重活，
/// 放 spawn_blocking 且库锁外（锁内只留写入协议的小 IO 与插行），与桌面
/// attach_photos 命令同款。并发计入普通 8 槽（limits_mw 照挂）。
async fn photo_upload_handler(
    State(deps): State<Shared>,
    multipart: axum::extract::Multipart,
) -> axum::response::Response {
    use axum::http::StatusCode;
    let (checkin_id, raws) = match read_multipart_photos(multipart).await {
        Ok(v) => v,
        Err(msg) => return json_error(StatusCode::BAD_REQUEST, &msg),
    };
    if raws.is_empty() {
        return json_error(StatusCode::BAD_REQUEST, "没有收到照片（photos 字段为空）");
    }
    let photos_root = deps.data_dir.join(crate::photo::PHOTOS_DIR_NAME);
    let conn = deps.conn.clone();
    let uploads: Vec<crate::photo::PhotoUpload> = raws
        .into_iter()
        .map(|r| crate::photo::PhotoUpload {
            original_name: r.original_name,
            bytes: r.bytes,
        })
        .collect();
    let joined = tauri::async_runtime::spawn_blocking(
        move || -> Result<Vec<crate::nest_checkin::NestPhotoMeta>, UploadRejection> {
            // 快速失败：登记不存在就不烧解码重编码的 CPU（业务错误 → 500）
            let exists =
                crate::run_with_conn(&conn, |c| crate::nest_checkin::checkin_exists(c, checkin_id))
                    .map_err(UploadRejection::Failed)?;
            if !exists {
                return Err(UploadRejection::Failed("登记不存在".to_string()));
            }
            // 校验链拒 = 客户端内容坏 → 400
            let processed = crate::photo::process_uploads(uploads)
                .map_err(UploadRejection::BadRequest)?;
            crate::run_with_conn(&conn, |c| {
                crate::photo::attach_photos(c, &photos_root, checkin_id, &processed)
            })
            .map_err(UploadRejection::Failed)
        },
    )
    .await
    .map_err(|e| UploadRejection::Failed(format!("照片上传任务异常退出: {e}")));
    match joined {
        Ok(Ok(saved)) => {
            // 巢况写后语义：不刷托盘，自动备份记账/版本广播走同一钩子
            (deps.after_write)(false);
            json_response(
                StatusCode::OK,
                &serde_json::to_value(&saved).unwrap_or(serde_json::Value::Null),
            )
        }
        Ok(Err(UploadRejection::BadRequest(msg))) => json_error(StatusCode::BAD_REQUEST, &msg),
        Ok(Err(UploadRejection::Failed(msg))) => json_error(StatusCode::INTERNAL_SERVER_ERROR, &msg),
        // join 层只产 Failed；此臂为穷尽性要求
        Err(UploadRejection::BadRequest(msg)) => json_error(StatusCode::BAD_REQUEST, &msg),
        Err(UploadRejection::Failed(msg)) => json_error(StatusCode::INTERNAL_SERVER_ERROR, &msg),
    }
}

/// `GET /api/photo/{colonyId}/{file}`（票 08）：Bearer 照挂（api 组闸二）；严格
/// 路径 grammar + 查库确认引用（防枚举库未引用的文件名）；库引用在而文件缺
/// → 404 占位（规格 E「缺图展示」的服务端半边，前端渲染占位符不崩溃）。
/// 响应头三件套见 [`photo_response_headers`]。
async fn photo_read_handler(
    State(deps): State<Shared>,
    axum::extract::Path((colony_id, file_name)): axum::extract::Path<(String, String)>,
) -> axum::response::Response {
    use axum::http::StatusCode;
    if !photo_url_parts_valid(&colony_id, &file_name) {
        return json_error(StatusCode::NOT_FOUND, "资源不存在");
    }
    let rel_path = format!("{colony_id}/{file_name}");
    let referenced = crate::run_with_conn(&deps.conn, |conn| {
        crate::photo::rel_path_referenced(conn, &rel_path)
    });
    let referenced = match referenced {
        Ok(v) => v,
        Err(e) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, &e),
    };
    if !referenced {
        return json_error(StatusCode::NOT_FOUND, "资源不存在");
    }
    let path = deps
        .data_dir
        .join(crate::photo::PHOTOS_DIR_NAME)
        .join(&colony_id)
        .join(&file_name);
    match std::fs::read(&path) {
        Ok(bytes) => (StatusCode::OK, photo_response_headers(), bytes).into_response(),
        Err(_) => json_error(StatusCode::NOT_FOUND, "照片文件缺失（可能恢复过旧备份）"),
    }
}

// ── HTTP 层：前端静态资源（票 05，规格「浏览器打开即完整前端」）────────────

/// 静态资源响应的附加头：nosniff 防 MIME 嗅探（规格 E 精神，照片路由票 08 同款）。
fn static_headers(mime: &'static str) -> [(&'static str, &'static str); 2] {
    [("Content-Type", mime), ("X-Content-Type-Options", "nosniff")]
}

/// dist 产物扩展名 → MIME（vite 产出的有限集合，未知一律 octet-stream）。
fn mime_for(path: &str) -> &'static str {
    let ext = path.rsplit('.').next().unwrap_or("");
    match ext {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "txt" => "text/plain; charset=utf-8",
        "webmanifest" => "application/manifest+json",
        _ => "application/octet-stream",
    }
}

/// 静态资源取用：路径安全检查 → 资源查找 → 带类型回包。
/// 路径只放行 dist 产物形态（相对路径，vite hash 文件名全部落在
/// `[A-Za-z0-9._-/]`）；盘符/反斜杠/上跳/百分号编码一律 404——闸一网段内也
/// 不给路径花样试探留口子。
fn serve_asset(deps: &SharedDeps, raw_path: &str) -> axum::response::Response {
    use axum::http::StatusCode;
    let path = raw_path.trim_start_matches('/');
    let suspicious = path.is_empty()
        || path.contains('\\')
        || path.contains("..")
        || path.contains(':')
        || path.contains('%');
    if suspicious {
        return json_error(StatusCode::NOT_FOUND, "资源不存在");
    }
    match (deps.frontend_assets)(path) {
        Some(bytes) => (
            StatusCode::OK,
            static_headers(mime_for(path)),
            bytes,
        )
            .into_response(),
        // 开发期（tauri dev 前端走 devUrl）嵌入资源为空：404 人话，不阻塞
        None => json_error(StatusCode::NOT_FOUND, "资源不存在"),
    }
}

/// `GET /`：前端入口页（dist 产物 index.html）。
async fn index_handler(State(deps): State<Shared>) -> axum::response::Response {
    serve_asset(&deps, "index.html")
}

/// `GET /{*path}`：前端其余静态资源（vite hash 文件名 / 图标 / 字体）。
async fn static_handler(
    State(deps): State<Shared>,
    axum::extract::Path(path): axum::extract::Path<String>,
) -> axum::response::Response {
    serve_asset(&deps, &path)
}

// ── HTTP 层：路由组装 ───────────────────────────────────────────────────────

/// 路由（自外向内）：全局体上限 → [限额 → 超时 → 闸二 → 端点]。
/// `/api/health` 挂同一套限额/超时、**仅豁免闸二**（唯一免凭证端点：若连并发
/// 上限都无，段内无凭证者可连接洪泛，把并发任务全堵在库锁上占满 tokio worker
/// ——恢复校验/备份持锁窗口期尤甚。规格 A「普通并发上限 8」没有端点豁免授权）。
/// 前端静态资源（票 05）同 health：**豁免闸二**（页面要先加载才能跑 `#token=`
/// 入库的入口 JS，带不了 Authorization 头；产物是公开构建物无敏感内容），
/// 限额/超时照挂。
/// `/api/sse`（票 06）独立成组：豁免闸二（EventSource 建连带不了 Bearer 头，
/// 凭证换一次性票据在 handler 内消费）；**不占普通并发槽**（SSE 长连接单独
/// 计量 ≤4，handler 内 try_acquire_sse）；header 上限与建连超时照挂（超时只
/// 覆盖建连——流式响应体在中间件链返回后才被消费，见 sse_handler 注释）。
/// `/api/data-version`（票 06）走闸二与全部限额（普通短请求）。照片端点
///（票 08）同在 api 组：`POST /api/photos` 闸二/限额照挂、处理超时按路径放宽
/// 到 300 秒（终局评审），体上限经路由层 DefaultBodyLimit 放宽到 9×15MB+框架
/// 开销（其余路径全局 1MB 不动）；`GET /api/photo/...` 是普通短请求。其余一切
/// 路径 404（axum 无路由默认）。
fn build_router(deps: Shared) -> axum::Router {
    use axum::extract::DefaultBodyLimit;
    use axum::middleware as mw;
    use axum::routing::{get, post};

    let api = axum::Router::new()
        .route("/api/cmd", post(cmd_handler))
        .route("/api/sse-ticket", post(sse_ticket_handler))
        .route("/api/data-version", get(data_version_handler))
        // 照片端点（票 08）：闸二/限额/超时照组层挂；上传路由用路由层
        // DefaultBodyLimit 按路径放宽（路由层比全局层更贴 handler，覆盖生效）
        .route(
            "/api/photos",
            post(photo_upload_handler).layer(DefaultBodyLimit::max(MAX_PHOTOS_BODY_BYTES)),
        )
        .route(
            "/api/photo/{colony_id}/{file_name}",
            get(photo_read_handler),
        )
        .layer(mw::from_fn_with_state(deps.clone(), auth_mw))
        .layer(mw::from_fn(timeout_mw))
        .layer(mw::from_fn_with_state(deps.clone(), limits_mw))
        .with_state(deps.clone());
    let health = axum::Router::new()
        .route("/api/health", get(health_handler))
        .layer(mw::from_fn(timeout_mw))
        .layer(mw::from_fn_with_state(deps.clone(), limits_mw))
        .with_state(deps.clone());
    let sse = axum::Router::new()
        .route("/api/sse", get(sse_handler))
        .layer(mw::from_fn(sse_gate_mw))
        .layer(mw::from_fn(timeout_mw))
        .with_state(deps.clone());
    let assets = axum::Router::new()
        .route("/", get(index_handler))
        .route("/{*path}", get(static_handler))
        .layer(mw::from_fn(timeout_mw))
        .layer(mw::from_fn_with_state(deps.clone(), limits_mw))
        .with_state(deps);
    health
        .merge(api)
        .merge(sse)
        .merge(assets)
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
}

// ── 服务生命周期：accept 循环 / 起停 ────────────────────────────────────────

/// 优雅关停时在途连接的收尾宽限。
const GRACE_PERIOD: Duration = Duration::from_secs(3);

/// 运行中的服务实例句柄。
pub struct ServerHandle {
    port: u16,
    shutdown: tokio::sync::watch::Sender<bool>,
    join: tokio::task::JoinHandle<()>,
}

impl std::fmt::Debug for ServerHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerHandle").field("port", &self.port).finish()
    }
}

impl ServerHandle {
    /// 实际监听端口（端口传 0 时 = OS 分配值，测试用）。
    pub fn port(&self) -> u16 {
        self.port
    }

    /// 优雅关停：广播停机 → accept 循环退出 → 在途连接各给 3 秒收尾。
    pub async fn stop(self) {
        let _ = self.shutdown.send(true);
        let _ = self.join.await;
    }
}

/// 启动服务：全网卡监听（闸一在连接准入收口）；端口被占用 → 不启动 + 人话
/// 错误（设置页回显 + applog）。`port` 传 0 让 OS 分配（仅测试用）。
pub async fn start(deps: Shared, port: u16) -> Result<ServerHandle, String> {
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port))
        .await
        .map_err(|e| {
            let msg = format!("端口 {port} 被占用或不可用（{e}），网页端服务未启动；可改用其他端口后重新保存");
            crate::applog::log_error(&format!("网页端服务启动失败: {msg}"));
            msg
        })?;
    let bound = listener
        .local_addr()
        .map_err(|e| format!("读取监听端口失败: {e}"))?;
    let app = build_router(deps.clone());
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let join = tokio::spawn(accept_loop(listener, app, deps, shutdown_rx));
    crate::applog::log_action(&format!("网页端服务已启动，端口 {}", bound.port()));
    Ok(ServerHandle {
        port: bound.port(),
        shutdown: shutdown_tx,
        join,
    })
}

/// accept 循环：连接级闸一准入（TCP 层，先于任何 HTTP 解析）→ 每连接一个
/// 任务；关停广播后停收新连接、在途连接各给 3 秒宽限。
async fn accept_loop(
    listener: tokio::net::TcpListener,
    app: axum::Router,
    deps: Shared,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
) {
    let mut conns = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, _)) => admit_and_serve(stream, &app, &deps, &shutdown_rx, &mut conns),
                    Err(e) => eprintln!("[webui_server] accept 失败（继续监听）: {e}"),
                }
            }
            _ = shutdown_rx.changed() => break,
        }
    }
    while conns.join_next().await.is_some() {}
}

/// 单连接：闸一准入 → 挂 hyper http1 服务（读超时、关停宽限）。
fn admit_and_serve(
    stream: tokio::net::TcpStream,
    app: &axum::Router,
    deps: &Shared,
    shutdown_rx: &tokio::sync::watch::Receiver<bool>,
    conns: &mut tokio::task::JoinSet<()>,
) {
    let peer = stream
        .peer_addr()
        .unwrap_or_else(|_| SocketAddr::new(IpAddr::from([0, 0, 0, 0]), 0));
    // 闸一·连接准入：只认 socket 对端地址（忽略一切转发头——转发头是 HTTP 层
    // 概念，这里根本还没读任何字节）
    let cfg = webui_config::load(&deps.data_dir);
    if !admit_peer(peer.ip(), &cfg.segments) {
        crate::applog::log_error(&format!("网页端拒绝来源 {}（不在受信网段）", peer.ip()));
        conns.spawn(reject_connection(stream));
        return;
    }
    if *shutdown_rx.borrow() {
        return; // 关停已开始：不再接新连接
    }
    // 对端地址进请求扩展（等价 into_make_service_with_connect_info 的手动版）
    let svc = hyper_util::service::TowerToHyperService::new(
        app.clone().layer(axum::Extension(ConnectInfo(peer))),
    );
    let conn = hyper::server::conn::http1::Builder::new()
        // 读超时要计时器（hyper 1 无内置时钟）：TokioTimer 即 tokio 时间轮
        .timer(hyper_util::rt::TokioTimer::new())
        .header_read_timeout(Some(deps.read_timeout))
        .serve_connection(hyper_util::rt::TokioIo::new(stream), svc);
    // owned Receiver 才能进 'static 任务（&Receiver 出不了本函数）
    let rx = shutdown_rx.clone();
    conns.spawn(async move {
        let mut rx = rx;
        tokio::pin!(conn);
        tokio::select! {
            result = &mut conn => {
                if let Err(e) = result {
                    eprintln!("[webui_server] 连接异常结束: {e}");
                }
            }
            _ = rx.changed() => {
                // 优雅关停：在途请求最多再给 3 秒
                let _ = tokio::time::timeout(GRACE_PERIOD, &mut conn).await;
            }
        }
    });
}

/// 闸一拒绝：HTTP 解析前回一段极简 403（人话）再断开——TCP 层拒绝且客户端
/// 浏览器里看得懂原因。
async fn reject_connection(mut stream: tokio::net::TcpStream) {
    use tokio::io::AsyncWriteExt;
    let body = serde_json::json!({ "error": "来源地址不在受信网段，拒绝访问" }).to_string();
    let head = format!(
        "HTTP/1.1 403 Forbidden\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes()).await;
    let _ = stream.write_all(body.as_bytes()).await;
    let _ = stream.shutdown().await;
}

// ── 运行时槽位（Tauri 状态）：按配置对齐起停 ────────────────────────────────

/// 服务运行时（管理进 Tauri 状态）：启动序列与 save_webui_config 共用的
/// 「按配置对齐」入口，保证任意时刻至多一个监听实例。
pub struct WebUiRuntime {
    /// tokio Mutex：跨 await 持锁（sync 内部要停旧起新），且不毒化。
    slot: tokio::sync::Mutex<Option<ServerHandle>>,
}

impl WebUiRuntime {
    pub fn new() -> Self {
        WebUiRuntime {
            slot: tokio::sync::Mutex::new(None),
        }
    }

    /// 当前监听端口（在跑才有；票 05/06 设置页运行状态与 SSE 调试会消费）。
    #[allow(dead_code)]
    pub async fn running_port(&self) -> Option<u16> {
        self.slot.lock().await.as_ref().map(ServerHandle::port)
    }

    /// 按配置对齐服务状态（幂等）：
    /// - `enabled=false` → 停（没在跑就什么都不做）；
    /// - `enabled=true` 且已在跑且端口没变 → 不动（凭证/网段每请求读配置，
    ///   改了即刻生效，无须重启）；
    /// - 其余 → 停旧、按新端口起；绑定失败 → 旧已停、Err 人话（设置页回显）。
    pub async fn sync(
        &self,
        deps: Shared,
        cfg: &webui_config::WebUiConfig,
    ) -> Result<(), String> {
        let mut slot = self.slot.lock().await;
        if !cfg.enabled {
            if let Some(handle) = slot.take() {
                handle.stop().await;
                crate::applog::log_action("网页端服务已停用");
            }
            return Ok(());
        }
        if slot.as_ref().is_some_and(|h| h.port() == cfg.port) {
            return Ok(());
        }
        if let Some(handle) = slot.take() {
            handle.stop().await;
        }
        let handle = start(deps, cfg.port).await?;
        *slot = Some(handle);
        Ok(())
    }
}

impl Default for WebUiRuntime {
    fn default() -> Self {
        Self::new()
    }
}

// ── 测试：只测外部行为（spec「Testing Decisions」）─────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn v4(s: &str) -> IpAddr {
        IpAddr::V4(s.parse::<Ipv4Addr>().unwrap())
    }

    fn segs(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    // ── 闸一：网段白名单判定 ──

    #[test]
    fn peer_inside_trusted_segment_admitted() {
        let s = segs(&["127.0.0.0/8", "10.0.0.0/8"]);
        assert!(admit_peer(v4("127.0.0.1"), &s), "环回段在列");
        assert!(admit_peer(v4("127.99.1.2"), &s), "/8 只看网络位");
        assert!(admit_peer(v4("10.221.4.9"), &s));
    }

    #[test]
    fn peer_outside_all_segments_rejected() {
        let s = segs(&["127.0.0.0/8"]);
        assert!(!admit_peer(v4("192.168.1.5"), &s));
        assert!(!admit_peer(v4("10.0.0.1"), &s), "只认登记的段");
        assert!(!admit_peer(v4("126.255.255.255"), &s), "段边界外一位");
        assert!(!admit_peer(v4("128.0.0.0"), &s));
    }

    #[test]
    fn ipv6_peer_always_rejected() {
        let s = segs(&["127.0.0.0/8", "::/0"]);
        assert!(!admit_peer(IpAddr::V6("::1".parse().unwrap()), &s), "环回 v6 也拒");
        assert!(
            !admit_peer(IpAddr::V6("2001:db8::1".parse().unwrap()), &s),
            "全球 v6 一律拒（规格 B：IPv4 CIDR 匹配，IPv6 默认拒绝）"
        );
    }

    #[test]
    fn empty_or_dirty_segment_list_rejects_everything() {
        assert!(!admit_peer(v4("127.0.0.1"), &[]), "空白名单拒一切（fail-closed）");
        // 非法网段项解析失败 = 不匹配，绝不放行
        assert!(!admit_peer(v4("127.0.0.1"), &segs(&["垃圾"])));
        assert!(!admit_peer(v4("127.0.0.1"), &segs(&["127.0.0.0"])), "缺前缀不解析");
    }

    #[test]
    fn multi_segment_any_match_admits() {
        let s = segs(&["100.84.0.0/16", "192.168.1.0/24"]);
        assert!(admit_peer(v4("100.84.12.3"), &s));
        assert!(admit_peer(v4("192.168.1.77"), &s));
        assert!(!admit_peer(v4("192.168.2.1"), &s), "/24 边界外拒");
    }

    // ── 闸二：Bearer 凭证校验 ──

    #[test]
    fn bearer_accepts_exact_match() {
        assert_eq!(check_bearer(Some("Bearer abc123"), "abc123"), Ok(()));
        // 方案名不分大小写（RFC 7235）
        assert_eq!(check_bearer(Some("bearer abc123"), "abc123"), Ok(()));
        assert_eq!(check_bearer(Some("BEARER abc123"), "abc123"), Ok(()));
    }

    #[test]
    fn bearer_rejects_missing_or_malformed() {
        let expected = "abc123";
        assert!(check_bearer(None, expected).is_err(), "缺头拒");
        assert!(check_bearer(Some(""), expected).is_err(), "空头拒");
        assert!(check_bearer(Some("abc123"), expected).is_err(), "无方案名拒");
        assert!(check_bearer(Some("Basic abc123"), expected).is_err(), "错方案名拒");
        assert!(check_bearer(Some("Bearer abc123 extra"), expected).is_err(), "多余段拒");
    }

    #[test]
    fn bearer_rejects_wrong_or_stale_token() {
        let expected = "a".repeat(32);
        assert!(check_bearer(Some(&format!("Bearer {}", "b".repeat(32))), &expected).is_err());
        assert!(
            check_bearer(Some(&format!("Bearer {}", "a".repeat(31))), &expected).is_err(),
            "长度不等拒（旧凭证即刻 401 的形状之一）"
        );
        // 大小写敏感：token 是十六进制，AA ≠ aa
        assert!(check_bearer(Some(&format!("Bearer {}", "A".repeat(32))), &expected).is_err());
    }

    #[test]
    fn bearer_rejects_when_server_token_not_ready() {
        // 服务端凭证为空（配置缺失/未生成）：fail-closed，带什么都拒
        assert!(check_bearer(Some("Bearer "), "").is_err(), "空 token 头也拒");
        assert!(check_bearer(None, "").is_err());
    }

    #[test]
    fn bearer_compare_is_constant_time_bytewise_xor() {
        // 正确值通过（这行主要钉行为语义；常数时间是实现性质，见实现注释）
        let expected = "0123456789abcdef0123456789abcdef";
        assert_eq!(check_bearer(Some(&format!("Bearer {expected}")), expected), Ok(()));
    }

    // ── SSE 短时票据：一次性 + 过期 ──

    #[test]
    fn ticket_issue_consume_once() {
        let store = SseTicketStore::new();
        let t = store.issue(1000).unwrap();
        assert_eq!(t.len(), TICKET_BYTES * 2, "128bit → 32 hex 字符");
        assert_eq!(store.len(), 1);

        assert!(store.consume(&t, 1000), "签发即有效");
        assert!(!store.consume(&t, 1001), "一次性：第二次消费拒");
        assert!(store.is_empty(), "消费即出表");
    }

    #[test]
    fn ticket_expiry_boundary() {
        let store = SseTicketStore::new();
        let t = store.issue(1000).unwrap();
        assert!(store.consume(&t, 1000 + TICKET_TTL_SECS - 1), "第 59 秒仍有效");
        let t2 = store.issue(2000).unwrap();
        assert!(
            !store.consume(&t2, 2000 + TICKET_TTL_SECS),
            "满 60 秒整即过期（有效窗 = [签发, 签发+60) 秒）"
        );
    }

    #[test]
    fn ticket_unknown_or_empty_rejected() {
        let store = SseTicketStore::new();
        assert!(!store.consume("不存在的票据", 1000));
        assert!(!store.consume("", 1000));
    }

    #[test]
    fn ticket_prune_removes_only_expired() {
        let store = SseTicketStore::new();
        let fresh = store.issue(1000).unwrap();
        let _old = store.issue(10).unwrap(); // 10+60=70 秒已过 → 过期
        assert_eq!(store.len(), 2);
        assert_eq!(store.prune(150), 1, "只清过期的");
        assert_eq!(store.len(), 1);
        assert!(store.consume(&fresh, 150), "新鲜票据不受清理牵连");
    }

    #[test]
    fn ticket_issue_prunes_and_is_unique() {
        let store = SseTicketStore::new();
        let _old = store.issue(100).unwrap();
        let t2 = store.issue(5000).unwrap(); // 100+60 << 5000：旧票过期被顺路清
        assert_eq!(store.len(), 1, "签发顺路清理过期票据");
        let t3 = store.issue(5000).unwrap();
        assert_ne!(t2, t3, "两次签发必不同（随机源坏掉才会撞）");
    }

    // ── 并发计量：普通与 SSE 分开 ──

    #[test]
    fn limiter_normal_slots_fill_then_release() {
        let lim = ConnLimiter::new(2, 4);
        let s1 = lim.acquire(LimitKind::Normal);
        let s2 = lim.acquire(LimitKind::Normal);
        assert!(s1.is_some() && s2.is_some());
        assert!(lim.acquire(LimitKind::Normal).is_none(), "普通额度满 → 拒");
        drop(s2);
        assert!(lim.acquire(LimitKind::Normal).is_some(), "归还后可再占");
    }

    #[test]
    fn limiter_sse_counted_separately() {
        let lim = ConnLimiter::new(2, 2);
        let n1 = lim.acquire(LimitKind::Normal).unwrap();
        let n2 = lim.acquire(LimitKind::Normal).unwrap();
        assert!(lim.acquire(LimitKind::Normal).is_none(), "普通满");
        let s1 = lim.acquire(LimitKind::Sse).unwrap();
        let s2 = lim.acquire(LimitKind::Sse).unwrap();
        assert!(lim.acquire(LimitKind::Sse).is_none(), "SSE 满但不挤占普通额度");
        assert!(lim.acquire(LimitKind::Normal).is_none());
        drop((n1, n2, s1, s2));
        assert_eq!(lim.in_flight(LimitKind::Normal), 0, "Drop 归还");
        assert_eq!(lim.in_flight(LimitKind::Sse), 0);
    }

    #[test]
    fn limiter_default_production_caps() {
        let lim = ConnLimiter::new(MAX_CONCURRENT_REQUESTS, MAX_CONCURRENT_SSE);
        let mut slots = Vec::new();
        for _ in 0..MAX_CONCURRENT_REQUESTS {
            slots.push(lim.acquire(LimitKind::Normal).expect("8 个普通槽应可全占"));
        }
        assert!(lim.acquire(LimitKind::Normal).is_none(), "第 9 个拒");
        let mut sse = Vec::new();
        for _ in 0..MAX_CONCURRENT_SSE {
            sse.push(lim.acquire(LimitKind::Sse).expect("4 个 SSE 槽独立可全占"));
        }
        assert!(lim.acquire(LimitKind::Sse).is_none());
        drop((slots, sse));
    }

    // ── 超时按路径放宽（终局评审）──

    #[test]
    fn photo_upload_timeout_relaxed_by_path_others_keep_read_timeout() {
        // 仅 POST /api/photos 放宽（慢 Wi-Fi 一批传 9 张大图，服务端还要逐张
        // 解码重编码，30 秒普通读超时太紧）；其余路径一律 READ_TIMEOUT 不动
        assert_eq!(
            request_timeout_for(&axum::http::Method::POST, PHOTO_UPLOAD_PATH),
            PHOTO_UPLOAD_TIMEOUT
        );
        assert_eq!(
            request_timeout_for(&axum::http::Method::POST, "/api/cmd"),
            READ_TIMEOUT
        );
        assert_eq!(
            request_timeout_for(&axum::http::Method::GET, PHOTO_UPLOAD_PATH),
            READ_TIMEOUT
        );
        assert_eq!(
            request_timeout_for(&axum::http::Method::GET, "/api/data-version"),
            READ_TIMEOUT
        );
        // 前后缀相近路径不误放宽
        assert_eq!(
            request_timeout_for(&axum::http::Method::POST, "/api/photosX"),
            READ_TIMEOUT
        );
        assert_eq!(
            request_timeout_for(&axum::http::Method::POST, "/api/photos/1/a.jpg"),
            READ_TIMEOUT
        );
    }

    // ── header 上限 ──

    fn header_map(pairs: &[(&str, &str)]) -> axum::http::HeaderMap {
        let mut map = axum::http::HeaderMap::new();
        for (k, v) in pairs {
            map.insert(
                axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                axum::http::HeaderValue::from_str(v).unwrap(),
            );
        }
        map
    }

    #[test]
    fn headers_within_limits_pass() {
        let map = header_map(&[("host", "localhost:17321"), ("authorization", "Bearer x")]);
        assert_eq!(header_limit_error(&map), None);
    }

    #[test]
    fn headers_count_over_limit_rejected() {
        let pairs: Vec<(String, String)> = (0..=MAX_HEADER_COUNT)
            .map(|i| (format!("x-h{i}"), "v".to_string()))
            .collect();
        let refs: Vec<(&str, &str)> = pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let map = header_map(&refs);
        let err = header_limit_error(&map).expect("超条数应拒");
        assert!(err.contains("条数"), "人话原因，实际：{err}");
    }

    #[test]
    fn headers_single_value_over_limit_rejected() {
        let big = "x".repeat(MAX_HEADER_VALUE_BYTES + 1);
        let map = header_map(&[("x-big", &big)]);
        let err = header_limit_error(&map).expect("单条超长应拒");
        assert!(err.contains("长"), "实际：{err}");
    }

    #[test]
    fn headers_total_over_limit_rejected() {
        // 每条不超单条上限，但条数 × 条长超过总体积上限
        let chunk = "x".repeat(MAX_HEADER_VALUE_BYTES);
        let need = MAX_HEADER_TOTAL_BYTES / MAX_HEADER_VALUE_BYTES + 2;
        let pairs: Vec<(String, String)> = (0..need)
            .map(|i| (format!("x-c{i}"), chunk.clone()))
            .collect();
        let refs: Vec<(&str, &str)> = pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let map = header_map(&refs);
        assert!(header_limit_error(&map).is_some(), "总体积超限应拒");
    }

    // ── 白名单注册表：deny-by-default + 永久禁入清单 ──

    /// 票 03 评审 Important 落点：四个网页端配置命令含凭证明文或可改安全配置，
    /// **永久禁入**网页端注册表。这条测试是红线，动注册表时先看它。
    #[test]
    fn registry_excludes_forbidden_commands() {
        const FORBIDDEN: [&str; 4] = [
            "get_webui_config",
            "save_webui_config",
            "regenerate_token",
            "get_access_url",
        ];
        for cmd in FORBIDDEN {
            assert!(
                !WEBUI_COMMANDS.contains(&cmd),
                "网页端注册表严禁包含 {cmd}（凭证明文/安全配置面）"
            );
        }
    }

    #[test]
    fn registry_has_no_duplicates_and_no_empty_names() {
        for (i, cmd) in WEBUI_COMMANDS.iter().enumerate() {
            assert!(!cmd.is_empty());
            assert!(!WEBUI_COMMANDS[..i].contains(cmd), "注册表重复项：{cmd}");
        }
    }

    #[test]
    fn registry_entries_all_have_real_dispatch() {
        // 注册表里每个名字都必须有真实派发分支（防「登记了但没实现」的 404 摆烂）
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join(crate::db::DB_FILE_NAME);
        let conn = crate::db::open_and_migrate(&db_path).unwrap();
        let deps = SharedDeps::new(Arc::new(Mutex::new(conn)), dir.path().to_path_buf());
        for cmd in WEBUI_COMMANDS {
            assert!(
                dispatch_command(&deps, cmd, &serde_json::json!({})).is_some(),
                "注册表里的 {cmd} 没有派发分支"
            );
        }
    }

    #[test]
    fn dispatch_health_check_hits_shared_single_connection() {
        // 派发走 with_conn 同一把库锁：写进库的数据经 HTTP 派发能读到（单连接不绕行）
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join(crate::db::DB_FILE_NAME);
        let conn = crate::db::open_and_migrate(&db_path).unwrap();
        let schema = crate::db::SCHEMA_VERSION;
        drop(conn);
        // 重开成共享形态（模拟运行态 DbState 的 Arc 锁）
        let conn = crate::db::open_and_migrate(&db_path).unwrap();
        let deps = SharedDeps::new(Arc::new(Mutex::new(conn)), dir.path().to_path_buf());
        let out = match dispatch_command(&deps, "health_check", &serde_json::json!({})).unwrap() {
            CmdOutcome::Ok(v) => v,
            other => panic!("health_check 应成功，实际 {other:?}"),
        };
        assert_eq!(out["schema_version"], schema, "HTTP 派发与桌面 health_check 同源");
    }

    #[test]
    fn http_delete_checkin_removes_photo_files_same_order() {
        // 票 07：HTTP 侧删登记与桌面同序——元数据行删干净，照片文件连带删掉
        //（不是留成孤儿等巡检；文件删失败才留孤儿）
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join(crate::db::DB_FILE_NAME);
        let conn = crate::db::open_and_migrate(&db_path).unwrap();
        conn.execute(
            "INSERT INTO colony (name, start_date, status) VALUES ('大头一号', '2026-01-20', 'active')",
            [],
        )
        .unwrap();
        let checkin_id = crate::nest_checkin::save_checkin(
            &conn,
            &crate::nest_checkin::CheckinInput {
                colony_id: 1,
                date: "2026-09-18".into(),
                queen_count: None,
                worker_count: None,
                moved_nest: false,
                note: Some("带照片".into()),
            },
            "2026-09-18",
            "2026-09-18 21:00:00",
        )
        .unwrap()
        .id;
        let photos_root = dir.path().join(crate::photo::PHOTOS_DIR_NAME);
        let uploads = vec![crate::photo::PhotoUpload {
            original_name: Some("a.jpg".into()),
            bytes: vec![1, 2, 3],
        }];
        let saved = crate::photo::attach_photos(&conn, &photos_root, checkin_id, &uploads).unwrap();
        let rel = saved[0].rel_path.clone();
        assert!(photos_root.join(&rel).exists(), "前置：照片文件在盘上");

        let deps = SharedDeps::new(Arc::new(Mutex::new(conn)), dir.path().to_path_buf());
        let out = dispatch_command(
            &deps,
            "delete_checkin",
            &serde_json::json!({"id": checkin_id}),
        )
        .unwrap();
        assert!(matches!(out, CmdOutcome::Ok(_)), "实际：{out:?}");
        assert!(!photos_root.join(&rel).exists(), "HTTP 删除登记连带删照片文件");
        let count: i64 = deps
            .conn
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM nest_photo", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0, "元数据行一并删除");
    }

    // ── 集成：真监听（127.0.0.1 随机端口）+ ureq 真请求 ──

    /// 组装测试依赖：临时数据目录 + 配置文件（token/网段可注入）+ 真库连接 +
    /// 1 秒读超时（慢速请求防挂秒级可验证）。返回 (deps, dir)。
    /// `after_write`/`frontend_assets` 可注入（票 05：副作用断言 / 静态托管）。
    fn test_deps(
        segments: &[&str],
        token: &str,
    ) -> (Arc<SharedDeps>, tempfile::TempDir) {
        let (deps, dir) = test_deps_with(segments, token, Arc::new(|_| {}), Arc::new(|_| None));
        (deps, dir)
    }

    /// 同 [`test_deps`]，另注入写后副作用钩子与前端静态资源查找。
    fn test_deps_with(
        segments: &[&str],
        token: &str,
        after_write: AfterWriteHook,
        frontend_assets: AssetLookup,
    ) -> (Arc<SharedDeps>, tempfile::TempDir) {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join(crate::db::DB_FILE_NAME);
        let conn = crate::db::open_and_migrate(&db_path).unwrap();
        webui_config::save(
            dir.path(),
            &webui_config::WebUiConfig {
                enabled: true,
                segments: segments.iter().map(|s| s.to_string()).collect(),
                port: 17321, // 占位；测试一律 start(port=0) 让 OS 分配
                token: token.to_string(),
                token_generated_at: None,
            },
        )
        .unwrap();
        let mut shared = SharedDeps::with_hooks(
            Arc::new(Mutex::new(conn)),
            dir.path().to_path_buf(),
            after_write,
            frontend_assets,
        );
        // 测试注入短读超时（慢速请求秒级断开可验证），与票 04 行为一致
        shared.read_timeout = Duration::from_secs(1);
        let deps = Arc::new(shared);
        (deps, dir)
    }

    fn block<F: std::future::Future>(f: F) -> F::Output {
        tauri::async_runtime::block_on(f)
    }

    fn agent() -> ureq::Agent {
        ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(5))
            .build()
    }

    /// 发请求，返回 (状态码, JSON 体)——ureq 对非 2xx 抛 Err(Status, resp)，摊平。
    fn http(
        agent: &ureq::Agent,
        method: &str,
        url: &str,
        auth: Option<&str>,
        body: &str,
    ) -> (u16, serde_json::Value) {
        let call = match method {
            "GET" => agent.get(url),
            _ => agent.post(url),
        };
        let call = match auth {
            Some(token) => call.set("Authorization", &format!("Bearer {token}")),
            None => call,
        };
        let result = if method == "GET" {
            call.call()
        } else {
            call.set("Content-Type", "application/json").send_string(body)
        };
        let to_json = |resp: ureq::Response| -> serde_json::Value {
            let text = resp.into_string().unwrap_or_default();
            serde_json::from_str(&text).unwrap_or(serde_json::Value::Null)
        };
        match result {
            Ok(resp) => (resp.status(), to_json(resp)),
            Err(ureq::Error::Status(code, resp)) => (code, to_json(resp)),
            Err(e) => panic!("请求 {method} {url} 失败: {e}"),
        }
    }

    #[test]
    fn server_serves_health_cmd_gates_and_tickets_end_to_end() {
        let token = "a".repeat(32);
        let (deps, _dir) = test_deps(&["127.0.0.0/8"], &token);
        let handle = block(start(deps.clone(), 0)).expect("启动失败");
        assert_ne!(handle.port(), 0, "port=0 时句柄回 OS 分配端口");
        let base = format!("http://127.0.0.1:{}", handle.port());
        let a = agent();

        // 健康检查：闸二豁免（不带凭证），闸一放行，真库读 schema 版本
        let (status, body) = http(&a, "GET", &format!("{base}/api/health"), None, "");
        assert_eq!(status, 200);
        assert_eq!(body["schema_version"], crate::db::SCHEMA_VERSION);

        // 正确凭证：health_check 经注册表真派发（与桌面命令同库锁）
        let (status, body) = http(
            &a,
            "POST",
            &format!("{base}/api/cmd"),
            Some(&token),
            r#"{"cmd":"health_check","args":{}}"#,
        );
        assert_eq!(status, 200, "实际：{body}");
        assert_eq!(body["schema_version"], crate::db::SCHEMA_VERSION);

        // 缺凭证 → 401 {"error":..}
        let (status, body) = http(
            &a,
            "POST",
            &format!("{base}/api/cmd"),
            None,
            r#"{"cmd":"health_check"}"#,
        );
        assert_eq!(status, 401);
        assert!(body["error"].as_str().unwrap_or("").contains("凭证"), "实际：{body}");

        // 错凭证 → 401
        let (status, _) = http(
            &a,
            "POST",
            &format!("{base}/api/cmd"),
            Some(&"b".repeat(32)),
            r#"{"cmd":"health_check"}"#,
        );
        assert_eq!(status, 401);

        // 未登记命令 → 404 人话（deny-by-default；get_settings 是桌面专属设置
        // 命令，规格 H 永不入表——错误串点明「可能是桌面端专属」）
        let (status, body) = http(
            &a,
            "POST",
            &format!("{base}/api/cmd"),
            Some(&token),
            r#"{"cmd":"get_settings","args":{}}"#,
        );
        assert_eq!(status, 404);
        assert_eq!(body["error"], "未知命令或桌面端专属功能");

        // 未登记路径 → 404（axum 无路由默认，不泄漏存在性）
        let (status, _) = http(&a, "GET", &format!("{base}/api/nothing"), Some(&token), "");
        assert_eq!(status, 404);

        // 坏 JSON → 400 人话
        let (status, body) = http(
            &a,
            "POST",
            &format!("{base}/api/cmd"),
            Some(&token),
            "不是 JSON{{{",
        );
        assert_eq!(status, 400);
        assert!(body["error"].as_str().unwrap_or("").contains("JSON"), "实际：{body}");

        // sse-ticket：主凭证换一次性票据，票据真进仓库、可消费一次
        let (status, body) = http(&a, "POST", &format!("{base}/api/sse-ticket"), Some(&token), "");
        assert_eq!(status, 200, "实际：{body}");
        let ticket = body["ticket"].as_str().expect("缺 ticket 字段").to_string();
        assert_eq!(ticket.len(), TICKET_BYTES * 2, "128bit → 32 hex 字符");
        assert_eq!(body["expires_in"], TICKET_TTL_SECS);
        let now = chrono::Utc::now().timestamp();
        assert!(deps.tickets.consume(&ticket, now), "签发的票据应可消费");
        assert!(!deps.tickets.consume(&ticket, now), "一次性：二次消费拒");

        block(handle.stop());
    }

    #[test]
    fn gate1_rejects_peer_outside_segments_at_tcp_layer() {
        let token = "a".repeat(32);
        let (deps, _dir) = test_deps(&["10.0.0.0/8"], &token);
        let handle = block(start(deps, 0)).unwrap();
        let base = format!("http://127.0.0.1:{}", handle.port());
        // 本机来源 127.0.0.1 不在受信网段 → 连接级 403（浏览器也能看到人话原因）
        let (status, body) = http(&agent(), "GET", &format!("{base}/api/health"), None, "");
        assert_eq!(status, 403, "实际：{body}");
        assert!(
            body["error"].as_str().unwrap_or("").contains("受信网段"),
            "实际：{body}"
        );
        block(handle.stop());
    }

    #[test]
    fn regenerated_token_rejects_old_value_immediately() {
        let old = "a".repeat(32);
        let (deps, dir) = test_deps(&["127.0.0.0/8"], &old);
        let handle = block(start(deps, 0)).unwrap();
        let base = format!("http://127.0.0.1:{}", handle.port());
        let a = agent();
        // 用 /api/cmd 探测凭证（闸二内层路径）
        let probe = |tok: &str| {
            http(
                &a,
                "POST",
                &format!("{base}/api/cmd"),
                Some(tok),
                r#"{"cmd":"health_check"}"#,
            )
            .0
        };
        assert_eq!(probe(&old), 200, "旧凭证先可用");

        // 模拟设置页重生成：覆盖写配置文件（服务端零通知——每请求读文件生效）
        webui_config::save(
            dir.path(),
            &webui_config::WebUiConfig {
                enabled: true,
                segments: vec!["127.0.0.0/8".into()],
                port: handle.port(),
                token: "f".repeat(32),
                token_generated_at: None,
            },
        )
        .unwrap();

        assert_eq!(probe(&old), 401, "旧凭证即刻 401（User Story 3）");
        assert_eq!(probe(&"f".repeat(32)), 200, "新凭证即刻可用");
        block(handle.stop());
    }

    #[test]
    fn health_is_subject_to_concurrency_limit() {
        // 评审 R1 回归钉：health 仅豁免闸二，限额层照挂——>8 个免凭证并发请求，
        // 超出者必须被限流（429），不得全部涌进 handler 阻塞在库锁上。
        // 确定性手法：测试占住库锁 → 持槽的 8 个请求全部阻塞在派发层的锁上，
        // 于是「立即返回」的只可能是被限流的后来者。
        let token = "a".repeat(32);
        let (deps, _dir) = test_deps(&["127.0.0.0/8"], &token);
        let handle = block(start(deps.clone(), 0)).unwrap();
        let base = format!("http://127.0.0.1:{}", handle.port());

        let guard = deps.conn.lock().expect("占住库锁失败");
        let statuses = Arc::new(Mutex::new(Vec::<u16>::new()));
        let mut joins = Vec::new();
        for _ in 0..12 {
            let base = base.clone();
            let statuses = statuses.clone();
            joins.push(std::thread::spawn(move || {
                let (status, _) = http(&agent(), "GET", &format!("{base}/api/health"), None, "");
                statuses.lock().unwrap().push(status);
            }));
        }
        // 等 4 个「立即返回」——持有槽的 8 个在库锁释放前不可能完成
        let expected_rejected = 12 - MAX_CONCURRENT_REQUESTS; // = 4
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while statuses.lock().unwrap().len() < expected_rejected {
            assert!(
                std::time::Instant::now() < deadline,
                "限流未生效：仅 {} 个请求返回（应至少 {expected_rejected} 个 429 立即返回）",
                statuses.lock().unwrap().len()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        drop(guard); // 放锁：8 个持槽请求立刻完成
        for join in joins {
            join.join().unwrap();
        }
        let all = statuses.lock().unwrap().clone();
        assert_eq!(all.len(), 12);
        assert_eq!(
            all.iter().filter(|&&s| s == 429).count(),
            12 - MAX_CONCURRENT_REQUESTS,
            "超出并发上限的请求被限流"
        );
        assert_eq!(
            all.iter().filter(|&&s| s == 200).count(),
            MAX_CONCURRENT_REQUESTS,
            "上限内的请求放行且健康检查正常"
        );
        block(handle.stop());
    }

    #[test]
    fn port_conflict_fails_without_panic_and_reports() {
        let token = "a".repeat(32);
        let (deps, _dir) = test_deps(&["127.0.0.0/8"], &token);
        let first = block(start(deps.clone(), 0)).unwrap();
        // 同端口再起 → 人话错误、不崩溃、原服务不受影响
        let err = block(start(deps, first.port())).unwrap_err();
        assert!(err.contains("端口") && err.contains("占用"), "实际：{err}");
        let base = format!("http://127.0.0.1:{}", first.port());
        let (status, _) = http(&agent(), "GET", &format!("{base}/api/health"), None, "");
        assert_eq!(status, 200, "冲突后原服务照常工作");
        block(first.stop());
    }

    #[test]
    fn oversized_body_rejected_with_413_or_disconnect() {
        let token = "a".repeat(32);
        let (deps, _dir) = test_deps(&["127.0.0.0/8"], &token);
        let handle = block(start(deps, 0)).unwrap();
        let base = format!("http://127.0.0.1:{}", handle.port());
        let big = "x".repeat(MAX_BODY_BYTES + 1);
        let body = format!(r#"{{"cmd":"health_check","args":{{"pad":"{big}"}}}}"#);
        // 超限响应在客户端还在发体时就已产出：服务端提前回 413 或直接断开
        // （验收口径「请求限制触发即断开」），两种都不算成功
        let result = agent()
            .post(&format!("{base}/api/cmd"))
            .set("Authorization", &format!("Bearer {token}"))
            .set("Content-Type", "application/json")
            .send_string(&body);
        match result {
            Ok(resp) => panic!("超限请求绝不能成功返回，实际状态 {}", resp.status()),
            Err(ureq::Error::Status(code, resp)) => {
                assert_eq!(code, 413, "若服务端来得及回，应是 413");
                let text = resp.into_string().unwrap_or_default();
                assert!(text.contains("请求体"), "人话错误，实际：{text}");
            }
            Err(ureq::Error::Transport(_)) => {} // 断开 = 合格（验收：触发即断开）
        }
        block(handle.stop());
    }

    #[test]
    fn slow_header_writer_gets_disconnected() {
        let token = "a".repeat(32);
        // deps 的读超时 1 秒（test_deps 注入）：慢速请求秒级断开
        let (deps, _dir) = test_deps(&["127.0.0.0/8"], &token);
        let handle = block(start(deps, 0)).unwrap();
        let mut stream =
            std::net::TcpStream::connect(("127.0.0.1", handle.port())).expect("连接失败");
        stream
            .set_read_timeout(Some(Duration::from_millis(300)))
            .unwrap();
        use std::io::{Read, Write};
        // 只写一半请求头就停手（不发结尾空行）——slowloris 形状
        stream
            .write_all(b"GET /api/health HTTP/1.1\r\nHost: t\r\n")
            .unwrap();
        let started = std::time::Instant::now();
        let mut buf = [0u8; 128];
        loop {
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "读超时应使连接在秒级断开，实际一直挂着"
            );
            match stream.read(&mut buf) {
                Ok(0) => break,          // 服务端关连接
                Err(_) => {
                    // 读超时窗口内无数据 = 连接还开着，继续等到服务端关
                    if started.elapsed() >= Duration::from_secs(5) {
                        panic!("连接未被服务端关闭");
                    }
                }
                Ok(_) => {} // 可能先收到 408 响应，随后连接关闭
            }
        }
        assert!(
            started.elapsed() >= Duration::from_millis(500),
            "连接确实因读超时断开（用时 {:?}）",
            started.elapsed()
        );
        block(handle.stop());
    }

    #[test]
    fn runtime_sync_aligns_start_stop_and_port_change() {
        let token = "a".repeat(32);
        let (deps, _dir) = test_deps(&["127.0.0.0/8"], &token);
        let runtime = WebUiRuntime::new();

        // enabled=false：空转，幂等
        let mut cfg = webui_config::load(&deps.data_dir);
        cfg.enabled = false;
        block(runtime.sync(deps.clone(), &cfg)).unwrap();
        assert_eq!(block(runtime.running_port()), None);

        // enabled=true：起服务，回真实端口
        let cfg = webui_config::load(&deps.data_dir);
        block(runtime.sync(deps.clone(), &cfg)).unwrap();
        let port = block(runtime.running_port()).expect("应已在跑");

        // 同端口再 sync：不动（幂等，不换实例）
        let cfg = webui_config::load(&deps.data_dir);
        block(runtime.sync(deps.clone(), &cfg)).unwrap();
        assert_eq!(block(runtime.running_port()), Some(port));

        // enabled=false：停
        let mut off = cfg.clone();
        off.enabled = false;
        block(runtime.sync(deps.clone(), &off)).unwrap();
        assert_eq!(block(runtime.running_port()), None, "停用即关停");

        // 再开：端口被旧实例释放后可复用
        block(runtime.sync(deps.clone(), &cfg)).unwrap();
        assert!(block(runtime.running_port()).is_some(), "重新启用");
        // 收尾：停掉（避免测试进程遗留监听）
        let mut off = cfg;
        off.enabled = false;
        block(runtime.sync(deps, &off)).unwrap();
        assert_eq!(block(runtime.running_port()), None);
    }

    // ── 票 05：记录类命令端到端（ureq 真请求 + 校验层 + 写后副作用）─────────

    /// 种子一个活跃窝（今天开始），返回其 id。预设字典已由 v1 迁移播种
    /// （操作 1 = 喂食（喂食类）、食物 1 = 种子），打卡测试直接引用。
    fn seed_colony(conn: &Arc<Mutex<Connection>>, name: &str) -> i64 {
        let conn = conn.lock().unwrap();
        let today = crate::colony::today_iso();
        let colony = crate::colony::create_colony(
            &conn,
            &crate::colony::ColonyInput {
                name: name.to_string(),
                species: None,
                location_id: None,
                start_date: today.clone(),
                status: "active".to_string(),
            },
            &today,
        )
        .expect("种子窝失败");
        colony.id
    }

    #[test]
    fn record_commands_serve_full_flow_end_to_end() {
        // User Stories 4/5/6/7 的 HTTP 版全流程：读首页 → 打卡 → 历史（改/删）
        // → 冬眠（开/改/出眠）→ 巢况（登/改/删/摘要）。SSE 未接线（票 06），
        // 每步读回即「切页刷新兜底」语义。
        let token = "a".repeat(32);
        let (deps, _dir) = test_deps(&["127.0.0.0/8"], &token);
        let colony_id = seed_colony(&deps.conn, "网页端窝");
        let today = crate::colony::today_iso();
        let handle = block(start(deps, 0)).expect("启动失败");
        let base = format!("http://127.0.0.1:{}", handle.port());
        let a = agent();
        let post = |body: String| -> (u16, serde_json::Value) {
            http(&a, "POST", &format!("{base}/api/cmd"), Some(&token), &body)
        };

        // 首页：list_colonies 返回这窝（桌面同形数组投影）
        let (status, body) = post(json_body("list_colonies", &serde_json::json!({})));
        assert_eq!(status, 200, "实际：{body}");
        assert_eq!(body.as_array().expect("数组").len(), 1);
        assert_eq!(body[0]["name"], "网页端窝");
        assert_eq!(body[0]["status"], "active");

        // 下拉字典：list_locations / list_actions / list_foods 非空
        for cmd in ["list_locations", "list_actions", "list_foods"] {
            let (status, body) = post(json_body(cmd, &serde_json::json!({})));
            assert_eq!(status, 200, "{cmd} 实际：{body}");
            assert!(!body.as_array().expect("数组").is_empty(), "{cmd} 应有预置项");
        }

        // 打卡：喂食 + 种子 → 返回新记录 id
        let (status, body) = post(
            serde_json::json!({
                "cmd": "log_care",
                "args": {"input": {"colony_id": colony_id, "action_id": 1,
                                    "happened_at": today, "note": "网页端打卡", "food_ids": [1]}}
            })
            .to_string(),
        );
        assert_eq!(status, 200, "实际：{body}");
        let log_id = body.as_i64().expect("新记录 id");

        // 历史查：list_logs 看得到（食物名字典序投影）
        let (status, body) = post(
            serde_json::json!({
                "cmd": "list_logs",
                "args": {"filter": {"colony_id": colony_id, "limit": 50, "offset": 0}}
            })
            .to_string(),
        );
        assert_eq!(status, 200, "实际：{body}");
        assert_eq!(body["total"], 1);
        assert_eq!(body["rows"][0]["id"], log_id);
        assert_eq!(body["rows"][0]["note"], "网页端打卡");
        assert_eq!(body["rows"][0]["food_names"], serde_json::json!(["种子"]));

        // 历史改：update_log 改备注 → 读回确认
        let (status, body) = post(
            serde_json::json!({
                "cmd": "update_log",
                "args": {"id": log_id, "input": {"note": "改过的备注"}}
            })
            .to_string(),
        );
        assert_eq!(status, 200, "实际：{body}");
        let (status, body) = post(
            serde_json::json!({
                "cmd": "list_logs",
                "args": {"filter": {"colony_id": colony_id}}
            })
            .to_string(),
        );
        assert_eq!(status, 200);
        assert_eq!(body["rows"][0]["note"], "改过的备注");

        // 历史删：delete_log → total 归零
        let (status, _) = post(
            serde_json::json!({"cmd": "delete_log", "args": {"id": log_id}}).to_string(),
        );
        assert_eq!(status, 200);
        let (status, body) = post(
            serde_json::json!({"cmd": "list_logs", "args": {"filter": {}}}).to_string(),
        );
        assert_eq!(status, 200);
        assert_eq!(body["total"], 0, "删除后归零，实际：{body}");

        // 冬眠开段：start_hibernation → 窝状态投影 hibernating + 横幅数据
        let plus30 = chrono::NaiveDate::parse_from_str(&today, "%Y-%m-%d")
            .unwrap()
            .checked_add_days(chrono::Days::new(30))
            .unwrap()
            .to_string();
        let (status, body) = post(
            serde_json::json!({
                "cmd": "start_hibernation",
                "args": {"colonyId": colony_id, "startDate": today, "expectedEndDate": plus30}
            })
            .to_string(),
        );
        assert_eq!(status, 200, "实际：{body}");
        assert_eq!(body["colony_id"], colony_id);
        assert_eq!(body["actual_end_date"], serde_json::Value::Null);
        let (status, body) = post(json_body("list_colonies", &serde_json::json!({})));
        assert_eq!(status, 200);
        assert_eq!(body[0]["status"], "hibernating", "实际：{body}");
        assert_eq!(body[0]["hibernation"]["start_date"], today);

        // 改预计出眠日：update_expected_end（票 09 停靠 D 同款语义）
        let plus60 = chrono::NaiveDate::parse_from_str(&today, "%Y-%m-%d")
            .unwrap()
            .checked_add_days(chrono::Days::new(60))
            .unwrap()
            .to_string();
        let (status, body) = post(
            serde_json::json!({
                "cmd": "update_expected_end",
                "args": {"colonyId": colony_id, "newExpectedEndDate": plus60}
            })
            .to_string(),
        );
        assert_eq!(status, 200, "实际：{body}");
        assert_eq!(body["expected_end_date"], plus60);

        // 出眠：confirm_wake → 窝回到 active
        let (status, body) = post(
            serde_json::json!({
                "cmd": "confirm_wake",
                "args": {"colonyId": colony_id, "actualEndDate": today}
            })
            .to_string(),
        );
        assert_eq!(status, 200, "实际：{body}");
        assert_eq!(body["actual_end_date"], today);
        let (status, body) = post(json_body("list_colonies", &serde_json::json!({})));
        assert_eq!(status, 200);
        assert_eq!(body[0]["status"], "active", "实际：{body}");

        // 补录冬眠期：add_past_hibernation（不冲突的历史段）
        let (status, body) = post(
            serde_json::json!({
                "cmd": "add_past_hibernation",
                "args": {"colonyId": colony_id,
                          "startDate": "2025-12-01", "endDate": "2026-02-20"}
            })
            .to_string(),
        );
        assert_eq!(status, 200, "实际：{body}");

        // 巢况登记：save_checkin（至少一项非空业务规则由纯核管）
        let (status, body) = post(
            serde_json::json!({
                "cmd": "save_checkin",
                "args": {"input": {"colony_id": colony_id, "date": today,
                                    "queen_count": 1, "worker_count": 42,
                                    "moved_nest": false, "note": "初登记"}}
            })
            .to_string(),
        );
        assert_eq!(status, 200, "实际：{body}");
        let checkin_id = body["id"].as_i64().expect("登记 id");
        assert_eq!(body["photos"], serde_json::json!([]), "本票 photos 恒空（票 08 接线）");

        // 巢况摘要：get_checkin_digest（卡片投影）
        let (status, body) = post(
            serde_json::json!({"cmd": "get_checkin_digest", "args": {"colonyId": colony_id}})
                .to_string(),
        );
        assert_eq!(status, 200, "实际：{body}");
        assert_eq!(body["latest"]["id"], checkin_id);
        assert_eq!(body["days_since_last"], 0, "当天登记 = 0 天");

        // 巢况改：update_checkin 全量覆盖（数可清回 null）
        let (status, body) = post(
            serde_json::json!({
                "cmd": "update_checkin",
                "args": {"id": checkin_id, "input": {"date": today, "queen_count": null,
                                                      "worker_count": 50, "moved_nest": true,
                                                      "note": null}}
            })
            .to_string(),
        );
        assert_eq!(status, 200, "实际：{body}");
        assert_eq!(body["worker_count"], 50);
        assert_eq!(body["queen_count"], serde_json::Value::Null);
        assert_eq!(body["moved_nest"], true);

        // 巢况查：list_checkins 时间线（照片元数据恒空数组）
        let (status, body) = post(
            serde_json::json!({"cmd": "list_checkins", "args": {"colonyId": colony_id}})
                .to_string(),
        );
        assert_eq!(status, 200, "实际：{body}");
        assert_eq!(body.as_array().expect("数组").len(), 1);
        assert_eq!(body[0]["photos"], serde_json::json!([]));

        // 巢况删：delete_checkin → 摘要回到从未登记
        let (status, _) = post(
            serde_json::json!({"cmd": "delete_checkin", "args": {"id": checkin_id}}).to_string(),
        );
        assert_eq!(status, 200);
        let (status, body) = post(
            serde_json::json!({"cmd": "get_checkin_digest", "args": {"colonyId": colony_id}})
                .to_string(),
        );
        assert_eq!(status, 200);
        assert_eq!(body["latest"], serde_json::Value::Null, "实际：{body}");

        block(handle.stop());
    }

    /// 构造 `{"cmd":..,"args":..}` 请求体（帮助类型推断的小工具）。
    fn json_body(cmd: &str, args: &serde_json::Value) -> String {
        serde_json::json!({ "cmd": cmd, "args": args }).to_string()
    }

    // ── 票 06：数据版本广播与 SSE 端到端（原始 TCP 读帧，ureq 不能流式读）──

    /// 主凭证换一张一次性票据。
    fn issue_ticket(agent: &ureq::Agent, base: &str, token: &str) -> String {
        let (status, body) = http(agent, "POST", &format!("{base}/api/sse-ticket"), Some(token), "");
        assert_eq!(status, 200, "取票失败：{body}");
        body["ticket"].as_str().expect("缺 ticket 字段").to_string()
    }

    /// 原始 TCP 发 SSE 建连请求（EventSource 同款 GET /api/sse?ticket=）。
    fn sse_request(port: u16, ticket: &str) -> std::net::TcpStream {
        use std::io::Write;
        let mut stream =
            std::net::TcpStream::connect(("127.0.0.1", port)).expect("SSE 建连失败");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        write!(
            stream,
            "GET /api/sse?ticket={ticket} HTTP/1.1\r\nHost: t\r\nAccept: text/event-stream\r\n\r\n"
        )
        .expect("写 SSE 请求失败");
        stream
    }

    /// 读到 HTTP 响应头结束（\r\n\r\n），返回头文本。
    fn read_response_head(stream: &mut std::net::TcpStream) -> String {
        use std::io::Read;
        let mut buf: Vec<u8> = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            match stream.read(&mut byte) {
                Ok(0) => panic!("服务端在头部读完前关闭连接（已读 {} 字节）", buf.len()),
                Ok(_) => buf.push(byte[0]),
                Err(e) => panic!("读响应头失败: {e}"),
            }
            if buf.ends_with(b"\r\n\r\n") {
                return String::from_utf8(buf).unwrap();
            }
        }
    }

    /// 读下一个带 data: 行的 SSE 帧，回解析后的 JSON。hyper 用 chunked 编码，
    /// 帧会被 chunk 大小行/终止行包住，keep-alive 是注释帧——都按行过滤，
    /// 只认 `data:` 行。
    fn read_sse_data(stream: &mut std::net::TcpStream) -> serde_json::Value {
        use std::io::Read;
        let mut buf: Vec<u8> = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            match stream.read(&mut byte) {
                Ok(0) => panic!("SSE 连接被服务端关闭（已读 {} 字节）", buf.len()),
                Ok(_) => buf.push(byte[0]),
                Err(e) => panic!("读 SSE 帧失败: {e}"),
            }
            if buf.ends_with(b"\n\n") {
                let text = String::from_utf8(buf.clone()).unwrap();
                for line in text.lines() {
                    if let Some(data) = line.strip_prefix("data:") {
                        return serde_json::from_str(data.trim())
                            .unwrap_or_else(|e| panic!("data 行不是 JSON（{e}）: {data}"));
                    }
                }
                buf.clear(); // keep-alive 注释帧：跳过，继续等 data 帧
            }
        }
    }

    #[test]
    fn sse_sends_hello_with_epoch_and_version_on_connect() {
        // 验收「SSE 首帧=实例 epoch+当前版本号」：取票 → 原始 TCP 建连 →
        // 首帧 type=hello，epoch = 安装 id-启动毫秒，version = 库内计数真值。
        let token = "a".repeat(32);
        let (deps, dir) = test_deps(&["127.0.0.0/8"], &token);
        // 先写两笔（走统一咽喉），hello 应带当前计数而非 0
        bump_and_publish(&deps.conn, &deps.versions).unwrap();
        bump_and_publish(&deps.conn, &deps.versions).unwrap();
        let handle = block(start(deps.clone(), 0)).expect("启动失败");
        let base = format!("http://127.0.0.1:{}", handle.port());
        let a = agent();

        let ticket = issue_ticket(&a, &base, &token);
        let mut stream = sse_request(handle.port(), &ticket);
        let head = read_response_head(&mut stream);
        assert!(head.starts_with("HTTP/1.1 200"), "实际：{head}");
        assert!(
            head.to_ascii_lowercase()
                .contains("content-type: text/event-stream"),
            "SSE 响应类型：{head}"
        );
        let frame = read_sse_data(&mut stream);
        assert_eq!(frame["type"], "hello");
        assert_eq!(frame["version"], 2, "hello 带库内计数真值：{frame}");
        let epoch = frame["epoch"].as_str().expect("缺 epoch");
        // epoch = <安装id 32 hex>-<启动毫秒>
        let install_id = std::fs::read_to_string(dir.path().join("install-id")).unwrap();
        assert!(
            epoch.starts_with(install_id.trim()) && epoch.contains('-'),
            "epoch 应为 安装id-启动毫秒：{epoch}"
        );
        assert_eq!(epoch, deps.versions.current().epoch, "与 hub 同源");
        drop(stream);
        block(handle.stop());
    }

    #[test]
    fn sse_pushes_version_frame_after_write_command() {
        // 验收「HTTP 写触发广播帧」：写命令成功 → 写后钩子链（生产 = lib.rs
        // trigger_after_write → bump_and_publish；测试注入同语义钩子）→ 在订
        // SSE 连接收到 {type:"version"} 帧。读命令不广播。
        let token = "a".repeat(32);
        let deps_cell: Arc<std::sync::OnceLock<Arc<SharedDeps>>> =
            Arc::new(std::sync::OnceLock::new());
        let cell = deps_cell.clone();
        let after_write: AfterWriteHook = Arc::new(move |_with_tray| {
            if let Some(deps) = cell.get() {
                let _ = bump_and_publish(&deps.conn, &deps.versions);
            }
        });
        let (deps, _dir) = test_deps_with(&["127.0.0.0/8"], &token, after_write, Arc::new(|_| None));
        let _ = deps_cell.set(deps.clone());
        let colony_id = seed_colony(&deps.conn, "广播窝");
        let today = crate::colony::today_iso();
        let handle = block(start(deps, 0)).expect("启动失败");
        let base = format!("http://127.0.0.1:{}", handle.port());
        let a = agent();

        // 建连：hello version=0
        let mut stream = sse_request(handle.port(), &issue_ticket(&a, &base, &token));
        assert_eq!(read_sse_data(&mut stream)["type"], "hello");

        // 读命令：不广播
        let (status, _) = http(
            &a,
            "POST",
            &format!("{base}/api/cmd"),
            Some(&token),
            &json_body("list_colonies", &serde_json::json!({})),
        );
        assert_eq!(status, 200);
        // 写命令：广播 version=1
        let (status, body) = http(
            &a,
            "POST",
            &format!("{base}/api/cmd"),
            Some(&token),
            &serde_json::json!({
                "cmd": "log_care",
                "args": {"input": {"colony_id": colony_id, "action_id": 1,
                                    "happened_at": today, "food_ids": []}}
            })
            .to_string(),
        );
        assert_eq!(status, 200, "实际：{body}");

        let frame = read_sse_data(&mut stream);
        assert_eq!(frame["type"], "version", "实际：{frame}");
        assert_eq!(frame["version"], 1, "实际：{frame}");
        assert!(frame["epoch"].as_str().is_some());

        // 再写一笔：版本继续单调推帧
        let (status, _) = http(
            &a,
            "POST",
            &format!("{base}/api/cmd"),
            Some(&token),
            &serde_json::json!({
                "cmd": "save_checkin",
                "args": {"input": {"colony_id": colony_id, "date": today, "note": "巢况"}}
            })
            .to_string(),
        );
        assert_eq!(status, 200);
        let frame = read_sse_data(&mut stream);
        assert_eq!(frame["type"], "version");
        assert_eq!(frame["version"], 2, "版本跨写命令单调：{frame}");
        drop(stream);
        block(handle.stop());
    }

    #[test]
    fn sse_rejects_invalid_ticket_with_401() {
        let token = "a".repeat(32);
        let (deps, _dir) = test_deps(&["127.0.0.0/8"], &token);
        let handle = block(start(deps, 0)).unwrap();
        let base = format!("http://127.0.0.1:{}", handle.port());
        let a = agent();
        // 未知票据 / 空票据 → 401 人话
        let (status, body) = http(&a, "GET", &format!("{base}/api/sse?ticket=deadbeef"), None, "");
        assert_eq!(status, 401, "未知票据应 401");
        assert!(
            body["error"].as_str().unwrap_or("").contains("票据"),
            "实际：{body}"
        );
        let (status, _) = http(&a, "GET", &format!("{base}/api/sse"), None, "");
        assert_eq!(status, 401, "缺 ticket 参数（解析为空票）同样 401");
        block(handle.stop());
    }

    #[test]
    fn sse_ticket_is_single_use_on_connect() {
        // 验收「票据消费后原生重连不发生」的服务端半边：建连即消费，同票再连
        // 必 401——EventSource 原生自动重连不带新票据，必然死在这里，前端必须
        // close 后重取票据（ipc.ts 的实现与测试在前端侧钉）。
        let token = "a".repeat(32);
        let (deps, _dir) = test_deps(&["127.0.0.0/8"], &token);
        let handle = block(start(deps, 0)).unwrap();
        let base = format!("http://127.0.0.1:{}", handle.port());
        let a = agent();
        let ticket = issue_ticket(&a, &base, &token);
        let mut stream = sse_request(handle.port(), &ticket);
        assert!(read_response_head(&mut stream).starts_with("HTTP/1.1 200"), "首连成功");
        let (status, _) = http(&a, "GET", &format!("{base}/api/sse?ticket={ticket}"), None, "");
        assert_eq!(status, 401, "同票二次建连（原生重连形状）必 401");
        drop(stream);
        block(handle.stop());
    }

    #[test]
    fn sse_fifth_connection_rejected_and_slot_released_on_disconnect() {
        // 验收「第 5 条 SSE 连接被拒」：SSE 单独计量 ≤4，不挤占普通并发额度；
        // 断开一条后槽位随流归还，可再连。
        let token = "a".repeat(32);
        let (deps, _dir) = test_deps(&["127.0.0.0/8"], &token);
        let handle = block(start(deps, 0)).unwrap();
        let base = format!("http://127.0.0.1:{}", handle.port());
        let a = agent();
        let mut streams = Vec::new();
        for _ in 0..MAX_CONCURRENT_SSE {
            let mut s = sse_request(handle.port(), &issue_ticket(&a, &base, &token));
            assert!(
                read_response_head(&mut s).starts_with("HTTP/1.1 200"),
                "前 4 条应放行"
            );
            streams.push(s);
        }
        // 第 5 条：取票成功但 429
        let (status, body) = http(
            &a,
            "GET",
            &format!("{base}/api/sse?ticket={}", issue_ticket(&a, &base, &token)),
            None,
            "",
        );
        assert_eq!(status, 429, "实际：{body}");
        assert!(body["error"].as_str().unwrap_or("").contains("SSE"), "实际：{body}");
        // 普通请求额度未被 SSE 挤占
        let (status, _) = http(&a, "GET", &format!("{base}/api/health"), None, "");
        assert_eq!(status, 200, "SSE 满员不挤占普通并发");

        // 断开一条 → 槽位随流归还 → 可再连（轮询等 hyper 感知断连）
        drop(streams.pop());
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut reopened = None;
        while std::time::Instant::now() < deadline {
            let mut s = sse_request(handle.port(), &issue_ticket(&a, &base, &token));
            let head = read_response_head(&mut s);
            if head.starts_with("HTTP/1.1 200") {
                reopened = Some(s);
                break;
            }
            assert!(head.starts_with("HTTP/1.1 429"), "非 429 的意外响应：{head}");
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(reopened.is_some(), "断开后槽位应归还并允许新连接");
        block(handle.stop());
    }

    #[test]
    fn bump_and_publish_updates_db_and_hub_in_lockstep() {
        // 「桌面路径写触发 bump」单测级（票面纪律：桌面窗口级不测）：lib.rs
        // trigger_after_write → bump_data_version_and_broadcast 的可测核心就是
        // 本函数（锁内 bump + hub 发布）；AppHandle 事件 emit 不进单测。
        let (deps, _dir) = test_deps(&["127.0.0.0/8"], &"a".repeat(32));
        let f1 = bump_and_publish(&deps.conn, &deps.versions).unwrap();
        let f2 = bump_and_publish(&deps.conn, &deps.versions).unwrap();
        assert_eq!(f1.version, 1);
        assert_eq!(f2.version, 2, "连续写版本单调");
        assert_eq!(f1.epoch, f2.epoch, "同进程 epoch 不变");
        assert_eq!(deps.versions.current().version, 2, "hub 与库同步");
        let truth =
            crate::run_with_conn(&deps.conn, |conn| Ok(crate::data_version::get(conn))).unwrap();
        assert_eq!(truth, 2, "库内计数真值");
    }

    #[test]
    fn restore_bump_and_publish_keeps_version_monotonic_over_rollback() {
        // 验收「恢复完成→两端无条件刷新」的机制半边：恢复换入计数器更小的旧
        // 备份时，restore_bump_and_publish 抬到「已广播最大值+1」，已连客户端
        // 对账必判落后 → 刷新。（桌面端 db-restored 无条件刷新在 lib.rs 并存。）
        let (deps, _dir) = test_deps(&["127.0.0.0/8"], &"a".repeat(32));
        for _ in 0..5 {
            bump_and_publish(&deps.conn, &deps.versions).unwrap();
        }
        assert_eq!(deps.versions.current().version, 5);
        // 模拟恢复：新库换进来，计数器只有 1
        {
            let conn = deps.conn.lock().unwrap();
            crate::data_meta::set(&conn, crate::data_version::DATA_VERSION_KEY, "1").unwrap();
        }
        let frame = restore_bump_and_publish(&deps.conn, &deps.versions).unwrap();
        assert_eq!(frame.version, 6, "抬到 已广播最大值+1，不随备份回退");
        assert_eq!(deps.versions.current().version, 6);
        let truth =
            crate::run_with_conn(&deps.conn, |conn| Ok(crate::data_version::get(conn))).unwrap();
        assert_eq!(truth, 6, "抬升后的计数落进新库");
    }

    #[test]
    fn data_version_endpoint_reports_truth_gated_by_bearer() {
        let token = "a".repeat(32);
        let (deps, _dir) = test_deps(&["127.0.0.0/8"], &token);
        bump_and_publish(&deps.conn, &deps.versions).unwrap();
        let handle = block(start(deps, 0)).unwrap();
        let base = format!("http://127.0.0.1:{}", handle.port());
        let a = agent();
        // 走闸二：无凭证 401
        let (status, _) = http(&a, "GET", &format!("{base}/api/data-version"), None, "");
        assert_eq!(status, 401);
        // 带凭证：{epoch, version} 与 hub 同源
        let (status, body) = http(&a, "GET", &format!("{base}/api/data-version"), Some(&token), "");
        assert_eq!(status, 200);
        assert_eq!(body["version"], 1);
        assert!(body["epoch"].as_str().is_some());
        block(handle.stop());
    }

    #[test]
    fn shared_deps_epoch_is_install_id_plus_start_time() {
        // epoch 的组成钉死：安装 id（数据目录 install-id 文件）+ 进程启动毫秒
        let (deps, dir) = test_deps(&["127.0.0.0/8"], &"a".repeat(32));
        let install_id =
            std::fs::read_to_string(dir.path().join(crate::data_version::INSTALL_ID_FILE)).unwrap();
        let expected = crate::data_version::epoch_of(
            install_id.trim(),
            crate::data_version::process_start_millis(),
        );
        assert_eq!(deps.versions.current().epoch, expected);
        // 重开同一数据目录的依赖（模拟服务重启）：安装 id 稳定，启动毫秒不变
        //（同进程内 OnceLock），epoch 稳定；跨进程才会变（纯核测试已钉）
        let conn2 = {
            let conn = deps.conn.lock().unwrap();
            crate::data_version::get(&conn)
        };
        assert_eq!(conn2, 0);
    }

    #[test]
    fn validation_layer_rejects_garbage_with_400_without_touching_db() {
        // 白名单≠校验：每个 case 都是真请求打在校验层；断言 400 + 人话 + 库未污染
        let token = "a".repeat(32);
        let (deps, _dir) = test_deps(&["127.0.0.0/8"], &token);
        let colony_id = seed_colony(&deps.conn, "校验窝");
        let today = crate::colony::today_iso();
        let handle = block(start(deps, 0)).expect("启动失败");
        let base = format!("http://127.0.0.1:{}", handle.port());
        let a = agent();

        let cases: Vec<(&str, serde_json::Value)> = vec![
            // 缺字段
            ("缺 input", serde_json::json!({"cmd": "log_care", "args": {}})),
            ("缺嵌套字段", serde_json::json!({
                "cmd": "log_care",
                "args": {"input": {"colony_id": colony_id}}
            })),
            // 错型
            ("id 错型", serde_json::json!({
                "cmd": "log_care",
                "args": {"input": {"colony_id": "abc", "action_id": 1, "happened_at": today}}
            })),
            ("布尔错型", serde_json::json!({
                "cmd": "save_checkin",
                "args": {"input": {"colony_id": colony_id, "date": today, "moved_nest": "是"}}
            })),
            // 未知字段（顶层与嵌套）
            ("顶层未知字段", serde_json::json!({"cmd": "list_colonies", "args": {"foo": 1}})),
            ("嵌套未知字段", serde_json::json!({
                "cmd": "save_checkin",
                "args": {"input": {"colony_id": colony_id, "date": today, "bogus": true}}
            })),
            // ID 非正整数
            ("负数 id", serde_json::json!({"cmd": "delete_log", "args": {"id": -1}})),
            ("零 id", serde_json::json!({"cmd": "delete_checkin", "args": {"id": 0}})),
            ("负 colonyId", serde_json::json!({
                "cmd": "list_checkins", "args": {"colonyId": -3}
            })),
            // 日期垃圾与超范围
            ("日期垃圾", serde_json::json!({
                "cmd": "save_checkin",
                "args": {"input": {"colony_id": colony_id, "date": "不是日期", "queen_count": 1}}
            })),
            ("日期超年段", serde_json::json!({
                "cmd": "start_hibernation",
                "args": {"colonyId": colony_id, "startDate": "1800-01-01", "expectedEndDate": "1800-03-01"}
            })),
            // 字符串超长
            ("备注超长", serde_json::json!({
                "cmd": "log_care",
                "args": {"input": {"colony_id": colony_id, "action_id": 1,
                                    "happened_at": today, "note": "长".repeat(2001)}}
            })),
            // 分页上限
            ("分页超上限", serde_json::json!({
                "cmd": "list_logs", "args": {"filter": {"limit": 501}}
            })),
            ("分页负偏移", serde_json::json!({
                "cmd": "list_logs", "args": {"filter": {"offset": -5}}
            })),
            // 数值范围
            ("负数蚁后", serde_json::json!({
                "cmd": "save_checkin",
                "args": {"input": {"colony_id": colony_id, "date": today, "queen_count": -1}}
            })),
            // 冬眠缺字段
            ("冬眠缺字段", serde_json::json!({
                "cmd": "start_hibernation",
                "args": {"colonyId": colony_id, "startDate": today}
            })),
        ];

        for (label, body) in cases {
            let (status, resp) = http(
                &a,
                "POST",
                &format!("{base}/api/cmd"),
                Some(&token),
                &body.to_string(),
            );
            assert_eq!(status, 400, "{label} 应 400，实际：{resp}");
            let msg = resp["error"].as_str().unwrap_or_default();
            assert!(!msg.is_empty(), "{label} 错误体应带人话，实际：{resp}");
        }

        // 库未污染：合法读命令确认没有任何写入落地
        let (status, body) = http(
            &a,
            "POST",
            &format!("{base}/api/cmd"),
            Some(&token),
            r#"{"cmd":"list_logs","args":{"filter":{}}}"#,
        );
        assert_eq!(status, 200);
        assert_eq!(body["total"], 0, "畸形请求一律不落库");

        // 业务拒绝（形状合法但引用不存在）走 500 + 桌面同款错误串，不误标 400
        let (status, resp) = http(
            &a,
            "POST",
            &format!("{base}/api/cmd"),
            Some(&token),
            r#"{"cmd":"delete_log","args":{"id":99999}}"#,
        );
        assert_eq!(status, 500);
        assert_eq!(resp["error"], "记录不存在");

        block(handle.stop());
    }

    #[test]
    fn write_commands_fire_after_write_hook_with_desktop_semantics() {
        // 写命令成功后按桌面语义触发写后副作用（lib.rs trigger_after_write 同款：
        // 记账 last_data_write_date）；读命令与失败写（业务拒绝/校验拒）不触发。
        let token = "a".repeat(32);
        let fired = Arc::new(Mutex::new(Vec::<bool>::new()));
        let data_dir_holder = Arc::new(Mutex::new(None::<std::path::PathBuf>));
        let after_write: AfterWriteHook = {
            let fired = fired.clone();
            let holder = data_dir_holder.clone();
            Arc::new(move |with_tray| {
                fired.lock().unwrap().push(with_tray);
                // 模拟生产钩子的记账动作（record_data_write 写 backup-config.json）
                if let Some(dir) = holder.lock().unwrap().as_ref() {
                    crate::auto_backup::record_data_write(
                        dir,
                        chrono::Local::now().date_naive(),
                    )
                    .unwrap();
                }
            })
        };
        let (deps, dir) = test_deps_with(
            &["127.0.0.0/8"],
            &token,
            after_write,
            Arc::new(|_| None),
        );
        // 把真实数据目录喂给钩子（SharedDeps 持有同一目录）
        *data_dir_holder.lock().unwrap() = Some(deps.data_dir.clone());
        let colony_id = seed_colony(&deps.conn, "副作用窝");
        let today = crate::colony::today_iso();
        let handle = block(start(deps, 0)).expect("启动失败");
        let base = format!("http://127.0.0.1:{}", handle.port());
        let a = agent();
        let post = |body: String| -> (u16, serde_json::Value) {
            http(&a, "POST", &format!("{base}/api/cmd"), Some(&token), &body)
        };
        let fired_count = || fired.lock().unwrap().len();

        // 读命令不触发
        let (status, _) = post(json_body("list_colonies", &serde_json::json!({})));
        assert_eq!(status, 200);
        assert_eq!(fired_count(), 0, "读命令不触发写后副作用");

        // 打卡（带托盘语义）成功 → 触发一次 with_tray=true + 记账已落
        let (status, _) = post(
            serde_json::json!({
                "cmd": "log_care",
                "args": {"input": {"colony_id": colony_id, "action_id": 1,
                                    "happened_at": today, "food_ids": []}}
            })
            .to_string(),
        );
        assert_eq!(status, 200);
        assert_eq!(fired_count(), 1, "写命令成功应触发一次");
        assert_eq!(*fired.lock().unwrap().last().unwrap(), true, "打卡带托盘刷新语义");
        assert_eq!(
            crate::backup_config::load(&dir.path().to_path_buf()).last_data_write_date,
            Some(today.clone()),
            "自动备份记账已按桌面语义落账"
        );

        // 巢况写（永不参与提醒/催促）→ 触发但 with_tray=false
        let (status, _) = post(
            serde_json::json!({
                "cmd": "save_checkin",
                "args": {"input": {"colony_id": colony_id, "date": today, "note": "巢况"}}
            })
            .to_string(),
        );
        assert_eq!(status, 200);
        assert_eq!(fired_count(), 2);
        assert_eq!(*fired.lock().unwrap().last().unwrap(), false, "巢况不刷托盘");

        // 业务拒绝写（窝不存在）→ 500 且不触发
        let (status, _) = post(
            serde_json::json!({
                "cmd": "log_care",
                "args": {"input": {"colony_id": 999_999, "action_id": 1,
                                    "happened_at": today, "food_ids": []}}
            })
            .to_string(),
        );
        assert_eq!(status, 500);
        assert_eq!(fired_count(), 2, "失败写不触发");

        // 校验拒（缺字段）→ 400 且不触发
        let (status, _) = post(r#"{"cmd":"log_care","args":{}}"#.to_string());
        assert_eq!(status, 400);
        assert_eq!(fired_count(), 2, "校验拒不触发");

        block(handle.stop());
    }

    #[test]
    fn static_serving_delivers_frontend_and_bypasses_bearer_only() {
        // 打包后场景：dist 产物由服务托管，浏览器打开 / 即得完整前端；
        // 静态资源豁免闸二（页面加载带不了 Authorization 头）但保留闸一与限额。
        let token = "a".repeat(32);
        let dist = tempfile::TempDir::new().unwrap();
        std::fs::write(dist.path().join("index.html"), "<!doctype html><html>喂蚁</html>").unwrap();
        std::fs::create_dir_all(dist.path().join("assets")).unwrap();
        std::fs::write(dist.path().join("assets").join("app-1a2b3c.js"), "console.log(1)").unwrap();
        let dist_root = dist.path().to_path_buf();
        let assets: AssetLookup = Arc::new(move |p| std::fs::read(dist_root.join(p)).ok());

        let (deps, _dir) = test_deps_with(
            &["127.0.0.0/8"],
            &token,
            Arc::new(|_| {}),
            assets,
        );
        let handle = block(start(deps, 0)).expect("启动失败");
        let base = format!("http://127.0.0.1:{}", handle.port());
        let a = agent();

        // 入口页：不带凭证（闸二豁免）→ 200 text/html
        let resp = a
            .get(&format!("{base}/"))
            .call()
            .expect("入口页应可达");
        assert_eq!(resp.status(), 200);
        assert_eq!(
            resp.header("content-type"),
            Some("text/html; charset=utf-8")
        );
        assert!(resp.into_string().unwrap().contains("喂蚁"));

        // hash 资源：200 + js 类型
        let resp = a.get(&format!("{base}/assets/app-1a2b3c.js")).call().unwrap();
        assert_eq!(resp.status(), 200);
        assert_eq!(
            resp.header("content-type"),
            Some("text/javascript; charset=utf-8")
        );

        // 未知资源 404
        let (status, _) = http(&a, "GET", &format!("{base}/missing.js"), None, "");
        assert_eq!(status, 404);

        // 路径花样（.. 上跳）：404。ureq/url 客户端会把点段规整掉，用原始 socket 打
        use std::io::{Read, Write};
        let mut stream = std::net::TcpStream::connect(("127.0.0.1", handle.port())).unwrap();
        stream
            .write_all(b"GET /../secret.txt HTTP/1.1\r\nHost: t\r\nConnection: close\r\n\r\n")
            .unwrap();
        let mut raw = String::new();
        stream.read_to_string(&mut raw).unwrap();
        assert!(raw.starts_with("HTTP/1.1 404"), "上跳路径应 404，实际：{raw}");

        // API 面不受影响：health 照常；/api/xxx 未登记路径仍 404（不落静态兜底泄漏）
        let (status, _) = http(&a, "GET", &format!("{base}/api/health"), None, "");
        assert_eq!(status, 200);
        let (status, _) = http(&a, "GET", &format!("{base}/api/nothing"), Some(&token), "");
        assert_eq!(status, 404);

        block(handle.stop());
    }

    #[test]
    fn http_return_shape_matches_desktop_invoke_for_record_commands() {
        // 返回值序列化与桌面 IPC 同形（前端 types.ts 单一契约）：同一库、同一参，
        // 桌面形态 = 纯核调用 serde_json::to_value；HTTP 形态 = 派发 Ok 值。逐字段
        // 相等（含 snake_case 键名），至少覆盖首页/历史/巢况三面 + 一个写命令返回。
        let (deps, _dir) = test_deps(&["127.0.0.0/8"], &"a".repeat(32));
        let colony_id = seed_colony(&deps.conn, "同形窝");
        let today = crate::colony::today_iso();
        {
            let conn = deps.conn.lock().unwrap();
            // 桌面同源种子：一条记录 + 一条巢况
            crate::care::log_care(
                &conn,
                &crate::care::CareLogInput {
                    colony_id,
                    action_id: 1,
                    happened_at: today.clone(),
                    note: Some("同形".into()),
                    food_ids: vec![1],
                },
                &crate::care::now_local(),
            )
            .unwrap();
            crate::nest_checkin::save_checkin(
                &conn,
                &crate::nest_checkin::CheckinInput {
                    colony_id,
                    date: today.clone(),
                    queen_count: Some(1),
                    worker_count: Some(7),
                    moved_nest: false,
                    note: Some("同形巢况".into()),
                },
                &today,
                &crate::care::now_local(),
            )
            .unwrap();
        }

        /// 桌面形态 = 纯核调用结果的 serde 序列化（Tauri invoke 返回值同款）。
        fn desktop_of<T: serde::Serialize>(v: Result<T, String>) -> serde_json::Value {
            serde_json::to_value(v.unwrap()).unwrap()
        }

        // 读命令一：list_colonies（首页投影，含 actions/recent/checkin 嵌套）
        let desktop = {
            let conn = deps.conn.lock().unwrap();
            desktop_of(crate::colony::list_colonies(&conn, &today))
        };
        assert_eq!(
            dispatch_command(&deps, "list_colonies", &serde_json::json!({})),
            Some(CmdOutcome::Ok(desktop)),
        );

        // 读命令二：list_logs（历史行，含 food_ids/food_names/created_at）
        let filter = crate::care::LogFilter {
            colony_id: Some(colony_id),
            ..Default::default()
        };
        let desktop = {
            let conn = deps.conn.lock().unwrap();
            desktop_of(crate::care::list_logs(&conn, &filter))
        };
        assert_eq!(
            dispatch_command(
                &deps,
                "list_logs",
                &serde_json::json!({"filter": {"colony_id": colony_id}})
            ),
            Some(CmdOutcome::Ok(desktop)),
        );

        // 读命令三：list_checkins + get_checkin_digest（巢况时间线与卡片摘要）
        let desktop = {
            let conn = deps.conn.lock().unwrap();
            desktop_of(crate::nest_checkin::list_checkins(&conn, colony_id))
        };
        assert_eq!(
            dispatch_command(
                &deps,
                "list_checkins",
                &serde_json::json!({"colonyId": colony_id})
            ),
            Some(CmdOutcome::Ok(desktop)),
        );
        let desktop = {
            let conn = deps.conn.lock().unwrap();
            desktop_of(crate::nest_checkin::digest_for_colony(&conn, colony_id, &today))
        };
        assert_eq!(
            dispatch_command(
                &deps,
                "get_checkin_digest",
                &serde_json::json!({"colonyId": colony_id})
            ),
            Some(CmdOutcome::Ok(desktop)),
        );

        // 写命令返回同形：HTTP save_checkin 的返回体 == 桌面 get_checkin 的序列化
        let outcome = dispatch_command(
            &deps,
            "save_checkin",
            &serde_json::json!({
                "input": {"colony_id": colony_id, "date": today, "worker_count": 9,
                           "moved_nest": false, "note": null}
            }),
        )
        .unwrap();
        let http_row = match outcome {
            CmdOutcome::Ok(v) => v,
            other => panic!("save_checkin 应成功，实际 {other:?}"),
        };
        let desktop = {
            let conn = deps.conn.lock().unwrap();
            desktop_of(crate::nest_checkin::get_checkin(&conn, http_row["id"].as_i64().unwrap()))
        };
        assert_eq!(http_row, desktop);
    }

    // ── 票 08：照片上传 / 照片读取端点 ─────────────────────────────────────

    /// multipart 请求体构造（ureq 无 multipart 支持，手拼——字段顺序与浏览器
    /// FormData 同形：checkinId 文本段在前，photos 文件段若干）。
    const MULTIPART_BOUNDARY: &str = "----antfeedinglog-test-boundary";

    fn multipart_body(checkin_id: Option<i64>, photos: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut b = Vec::new();
        if let Some(id) = checkin_id {
            b.extend_from_slice(
                format!(
                    "--{MULTIPART_BOUNDARY}\r\n\
                     Content-Disposition: form-data; name=\"checkinId\"\r\n\r\n{id}\r\n"
                )
                .as_bytes(),
            );
        }
        for (name, bytes) in photos {
            b.extend_from_slice(
                format!(
                    "--{MULTIPART_BOUNDARY}\r\n\
                     Content-Disposition: form-data; name=\"photos\"; filename=\"{name}\"\r\n\
                     Content-Type: application/octet-stream\r\n\r\n"
                )
                .as_bytes(),
            );
            b.extend_from_slice(bytes);
            b.extend_from_slice(b"\r\n");
        }
        b.extend_from_slice(format!("--{MULTIPART_BOUNDARY}--\r\n").as_bytes());
        b
    }

    /// 测试夹具：一张 64×48 的真 JPEG（走 image crate 编码，重编码链可完整过）。
    fn tiny_jpeg() -> Vec<u8> {
        use std::io::Cursor;
        let img = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            64,
            48,
            image::Rgb([120, 90, 60]),
        ));
        let mut buf = Cursor::new(Vec::new());
        img.write_to(&mut buf, image::ImageFormat::Jpeg).unwrap();
        buf.into_inner()
    }

    /// POST /api/photos（multipart），返回 (状态码, JSON 体)。
    fn post_photos(
        a: &ureq::Agent,
        base: &str,
        token: Option<&str>,
        body: Vec<u8>,
    ) -> (u16, serde_json::Value) {
        let mut call = a
            .post(&format!("{base}/api/photos"))
            .set(
                "Content-Type",
                &format!("multipart/form-data; boundary={MULTIPART_BOUNDARY}"),
            );
        if let Some(t) = token {
            call = call.set("Authorization", &format!("Bearer {t}"));
        }
        let to_json = |resp: ureq::Response| -> serde_json::Value {
            serde_json::from_str(&resp.into_string().unwrap_or_default())
                .unwrap_or(serde_json::Value::Null)
        };
        match call.send_bytes(&body) {
            Ok(resp) => (resp.status(), to_json(resp)),
            Err(ureq::Error::Status(code, resp)) => (code, to_json(resp)),
            Err(e) => panic!("POST /api/photos 失败: {e}"),
        }
    }

    /// 种子一窝一条登记，返回 checkin_id（内部自取库锁，调用方不得持锁）。
    fn seed_checkin(deps: &Arc<SharedDeps>, name: &str) -> i64 {
        let colony_id = seed_colony(&deps.conn, name);
        let conn = deps.conn.lock().unwrap();
        crate::nest_checkin::save_checkin(
            &conn,
            &crate::nest_checkin::CheckinInput {
                colony_id,
                date: "2026-09-18".into(),
                queen_count: None,
                worker_count: None,
                moved_nest: false,
                note: Some("带照片".into()),
            },
            "2026-09-18",
            "2026-09-18 21:00:00",
        )
        .unwrap()
        .id
    }

    #[test]
    fn photo_upload_saves_file_and_metadata_pair_and_fires_after_write() {
        // 验收「浏览器上传→库+文件成对」：multipart 过校验链重编码，落盘名
        // 服务端 UUID，original_name 只进备注；写后钩子按巢况语义（不刷托盘）
        // 触发——生产钩子里是 trigger_after_write（版本 bump + 自动备份）。
        let token = "a".repeat(32);
        let fired = Arc::new(Mutex::new(Vec::<bool>::new()));
        let fired2 = fired.clone();
        let after_write: AfterWriteHook = Arc::new(move |with_tray| {
            fired2.lock().unwrap().push(with_tray);
        });
        let (deps, _dir) = test_deps_with(&["127.0.0.0/8"], &token, after_write, Arc::new(|_| None));
        let checkin_id = seed_checkin(&deps, "照片窝");
        let handle = block(start(deps.clone(), 0)).unwrap();
        let base = format!("http://127.0.0.1:{}", handle.port());

        let (status, body) = post_photos(
            &agent(),
            &base,
            Some(&token),
            multipart_body(Some(checkin_id), &[("IMG_001.jpg", tiny_jpeg())]),
        );
        assert_eq!(status, 200, "实际：{body}");
        let arr = body.as_array().expect("返回照片数组");
        assert_eq!(arr.len(), 1);
        let rel = arr[0]["rel_path"].as_str().expect("rel_path");
        assert_eq!(arr[0]["original_name"], "IMG_001.jpg", "客户端文件名只进备注");
        let (dir_seg, file_seg) = rel.split_once('/').unwrap();
        assert_eq!(dir_seg, "1", "目录段 = 窝 id（服务端归属反查）");
        assert_eq!(file_seg.len(), 36 + 4, "落盘名 = 服务端 UUID + .jpg，与客户端名无关");

        // 文件+库行成对存在（写入协议收尾状态），盘上是重编码后的 JPEG
        let photos_root = deps.data_dir.join(crate::photo::PHOTOS_DIR_NAME);
        let on_disk = std::fs::read(photos_root.join(rel)).expect("照片文件应已落盘");
        assert_eq!(&on_disk[..3], b"\xFF\xD8\xFF", "落盘的是 JPEG");
        let count: i64 = deps
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM nest_photo WHERE rel_path = ?1",
                [rel],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "元数据行已插");

        assert_eq!(fired.lock().unwrap().as_slice(), &[false], "巢况写不刷托盘");
        block(handle.stop());
    }

    #[test]
    fn photo_upload_rejects_oversize_batch_bad_format_and_bad_checkin_id() {
        // 超张数（10>9）/ 坏格式（GIF）/ 缺 checkinId / 零张 / 单张超 15MB：
        // 一律 400 人话且不碰库不落盘；未知 checkinId 是业务错误走 500。
        let token = "a".repeat(32);
        let (deps, _dir) = test_deps(&["127.0.0.0/8"], &token);
        let checkin_id = seed_checkin(&deps, "拒绝窝");
        let handle = block(start(deps.clone(), 0)).unwrap();
        let base = format!("http://127.0.0.1:{}", handle.port());
        let a = agent();

        // 超张数
        let photos: Vec<(String, Vec<u8>)> = (0..10)
            .map(|i| (format!("{i}.jpg"), tiny_jpeg()))
            .collect();
        let refs: Vec<(&str, Vec<u8>)> =
            photos.iter().map(|(n, b)| (n.as_str(), b.clone())).collect();
        let (status, body) = post_photos(&a, &base, Some(&token), multipart_body(Some(checkin_id), &refs));
        assert_eq!(status, 400, "实际：{body}");
        assert!(body["error"].as_str().unwrap_or("").contains('9'), "实际：{body}");
        assert!(
            !deps.data_dir.join(crate::photo::PHOTOS_DIR_NAME).exists(),
            "超批拒绝不落盘"
        );

        // 坏格式（GIF 魔数，改扩展名没用——白名单按魔数）
        let mut gif = b"GIF89a".to_vec();
        gif.extend_from_slice(&[0u8; 32]);
        let (status, body) =
            post_photos(&a, &base, Some(&token), multipart_body(Some(checkin_id), &[("evil.gif", gif)]));
        assert_eq!(status, 400, "实际：{body}");
        assert!(
            body["error"].as_str().unwrap_or("").contains("JPEG / PNG / WebP"),
            "实际：{body}"
        );

        // 缺 checkinId
        let (status, body) =
            post_photos(&a, &base, Some(&token), multipart_body(None, &[("a.jpg", tiny_jpeg())]));
        assert_eq!(status, 400);
        assert!(
            body["error"].as_str().unwrap_or("").contains("checkinId"),
            "实际：{body}"
        );

        // 零张照片
        let (status, body) =
            post_photos(&a, &base, Some(&token), multipart_body(Some(checkin_id), &[]));
        assert_eq!(status, 400);
        assert!(
            body["error"].as_str().unwrap_or("").contains("照片"),
            "实际：{body}"
        );

        // 单张压缩前超 15MB：流式闸 400（不整段囤进内存再拒）
        let big = vec![0u8; crate::photo::MAX_INPUT_BYTES + 1];
        let (status, body) =
            post_photos(&a, &base, Some(&token), multipart_body(Some(checkin_id), &[("big.jpg", big)]));
        assert_eq!(status, 400, "实际：{}", body["error"].as_str().unwrap_or_default());
        assert!(body["error"].as_str().unwrap_or("").contains("15"), "实际：{body}");

        // 未登记的 checkinId：业务错误 500（与 /api/cmd 的业务拒绝同口径）
        let (status, body) =
            post_photos(&a, &base, Some(&token), multipart_body(Some(999), &[("a.jpg", tiny_jpeg())]));
        assert_eq!(status, 500);
        assert!(
            body["error"].as_str().unwrap_or("").contains("登记不存在"),
            "实际：{body}"
        );

        // 一轮拒绝之后登记照常可用（无半截状态）
        let (status, _) = post_photos(
            &a,
            &base,
            Some(&token),
            multipart_body(Some(checkin_id), &[("ok.jpg", tiny_jpeg())]),
        );
        assert_eq!(status, 200, "拒绝轮不污染后续上传");
        block(handle.stop());
    }

    #[test]
    fn photo_upload_requires_bearer_token() {
        let token = "a".repeat(32);
        let (deps, _dir) = test_deps(&["127.0.0.0/8"], &token);
        let handle = block(start(deps, 0)).unwrap();
        let base = format!("http://127.0.0.1:{}", handle.port());
        let (status, body) = post_photos(&agent(), &base, None, multipart_body(Some(1), &[]));
        assert_eq!(status, 401, "无凭证上传必须 401");
        assert!(
            body["error"].as_str().unwrap_or("").contains("凭证"),
            "实际：{body}"
        );
        block(handle.stop());
    }

    #[test]
    fn photo_read_serves_referenced_file_with_triple_headers() {
        // 验收「响应头三件套 + 带凭证取图 200」：走写入协议种一张真照片，
        // GET /api/photo/<rel_path> 回 JPEG 字节 + 三件套。
        let token = "a".repeat(32);
        let (deps, _dir) = test_deps(&["127.0.0.0/8"], &token);
        let checkin_id = seed_checkin(&deps, "读图窝");
        let rel_path = {
            let conn = deps.conn.lock().unwrap();
            let photos_root = deps.data_dir.join(crate::photo::PHOTOS_DIR_NAME);
            let saved = crate::photo::attach_photos(
                &conn,
                &photos_root,
                checkin_id,
                &[crate::photo::PhotoUpload {
                    original_name: Some("a.jpg".into()),
                    bytes: tiny_jpeg(),
                }],
            )
            .unwrap();
            saved[0].rel_path.clone()
        };
        let handle = block(start(deps.clone(), 0)).unwrap();
        let base = format!("http://127.0.0.1:{}", handle.port());

        let resp = agent()
            .get(&format!("{base}/api/photo/{rel_path}"))
            .set("Authorization", &format!("Bearer {token}"))
            .call()
            .expect("库引用照片应可达");
        assert_eq!(resp.status(), 200);
        assert_eq!(resp.header("content-type"), Some("image/jpeg"), "类型钉死 JPEG");
        assert_eq!(resp.header("x-content-type-options"), Some("nosniff"));
        assert_eq!(resp.header("content-disposition"), Some("inline"));
        use std::io::Read;
        let mut buf = Vec::new();
        resp.into_reader().read_to_end(&mut buf).unwrap();
        let on_disk = std::fs::read(
            deps.data_dir
                .join(crate::photo::PHOTOS_DIR_NAME)
                .join(&rel_path),
        )
        .unwrap();
        assert_eq!(buf, on_disk, "回包字节与盘上文件一致");
        block(handle.stop());
    }

    #[test]
    fn photo_read_requires_bearer_rejects_bad_grammar_and_unreferenced() {
        let token = "a".repeat(32);
        let (deps, dir) = test_deps(&["127.0.0.0/8"], &token);
        let handle = block(start(deps, 0)).unwrap();
        let base = format!("http://127.0.0.1:{}", handle.port());
        let a = agent();
        let uuid_jpg = "6ba7b810-9dad-11d1-80b4-00c04fd430c8.jpg";

        // 无凭证 → 401
        let (status, _) = http(&a, "GET", &format!("{base}/api/photo/1/{uuid_jpg}"), None, "");
        assert_eq!(status, 401, "取图同样要过闸二");

        // 路径 grammar 拒绝表（404 人话；不泄漏形状原因给探测者）
        let cases = [
            format!("api/photo/abc/{uuid_jpg}"),           // 窝段非数字
            format!("api/photo/1x/{uuid_jpg}"),            // 窝段混字母
            format!("api/photo/-1/{uuid_jpg}"),            // 窝段负号
            format!("api/photo/1/{uuid_jpg}.jpg"),         // 双尾缀（base 形态破）
            "api/photo/1/short.jpg".to_string(),           // 非 UUID 长度
            "api/photo/1/x!@$.jpg".to_string(),            // 字符集外
            format!("api/photo/1/{}.png", &uuid_jpg[..36]), // 非 .jpg
        ];
        for path in cases {
            let (status, body) = http(&a, "GET", &format!("{base}/{path}"), Some(&token), "");
            assert_eq!(status, 404, "{path} 应 404，实际：{}", body["error"].as_str().unwrap_or_default());
        }

        // `..` 点段与反斜杠：URL 客户端会规整/转义，用原始 socket 打
        use std::io::{Read as _, Write as _};
        let raw_get = |target: &str| {
            let mut s = std::net::TcpStream::connect(("127.0.0.1", handle.port())).unwrap();
            write!(
                s,
                "GET {target} HTTP/1.1\r\nHost: t\r\nAuthorization: Bearer {token}\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
            let mut raw = String::new();
            s.read_to_string(&mut raw).unwrap();
            raw.lines().next().unwrap_or_default().to_string()
        };
        assert_eq!(raw_get("/api/photo/1/../x.jpg"), "HTTP/1.1 404 Not Found", "点段应 404");
        assert_eq!(raw_get("/api/photo/1/a\\b.jpg"), "HTTP/1.1 404 Not Found", "反斜杠应 404");

        // 未引用文件（磁盘有、库无行）→ 404 防枚举
        let photos_root = dir.path().join(crate::photo::PHOTOS_DIR_NAME);
        std::fs::create_dir_all(photos_root.join("1")).unwrap();
        std::fs::write(photos_root.join("1").join(uuid_jpg), b"orphan").unwrap();
        let (status, body) = http(
            &a,
            "GET",
            &format!("{base}/api/photo/1/{uuid_jpg}"),
            Some(&token),
            "",
        );
        assert_eq!(status, 404, "库无引用的文件不得经端点读出");
        assert!(body["error"].as_str().unwrap_or("").contains("不存在"));
        block(handle.stop());
    }

    #[test]
    fn photo_read_referenced_but_file_missing_returns_404_placeholder() {
        // 库引用在、文件缺（恢复裸库遗留）→ 404 人话占位；前端渲染「文件缺失」
        // 占位符，不崩溃（规格 E「缺图展示」的服务端半边）。
        let token = "a".repeat(32);
        let (deps, _dir) = test_deps(&["127.0.0.0/8"], &token);
        let checkin_id = seed_checkin(&deps, "缺图窝");
        {
            let conn = deps.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO nest_photo (checkin_id, rel_path, original_name, note)
                 VALUES (?1, '1/6ba7b810-9dad-11d1-80b4-00c04fd430c8.jpg', NULL, '')",
                [checkin_id],
            )
            .unwrap();
        }
        let handle = block(start(deps, 0)).unwrap();
        let base = format!("http://127.0.0.1:{}", handle.port());
        let (status, body) = http(
            &agent(),
            "GET",
            &format!("{base}/api/photo/1/6ba7b810-9dad-11d1-80b4-00c04fd430c8.jpg"),
            Some(&token),
            "",
        );
        assert_eq!(status, 404);
        assert!(
            body["error"].as_str().unwrap_or("").contains("缺失"),
            "实际：{body}"
        );
        block(handle.stop());
    }

    #[test]
    fn photo_url_grammar_accepts_only_digits_and_uuid_jpg_form() {
        // 表驱动 grammar 纯核：只放行 <纯数字>/<36位[A-Za-z0-9-]>.jpg
        let ok = |c: &str, f: &str| photo_url_parts_valid(c, f);
        let uuid = "6ba7b810-9dad-11d1-80b4-00c04fd430c8";
        let file = format!("{uuid}.jpg");
        assert!(ok("1", &file));
        assert!(ok("123456789", &file), "多位窝 id 合法");
        assert!(ok("1", &format!("{}.jpg", uuid.to_uppercase())), "大写十六进制在字符集内");
        // 窝段
        assert!(!ok("", &file));
        assert!(!ok("01a", &file), "混字母拒");
        assert!(!ok("-1", &file), "负号拒");
        assert!(!ok("1.5", &file));
        assert!(!ok(&"9".repeat(20), &file), "20 位超 i64 形态拒");
        // 文件段
        assert!(!ok("1", ".."));
        assert!(!ok("1", "../x.jpg"), "点段过不了字符集（'.' 不在集内）");
        assert!(!ok("1", "a\\b.jpg"), "反斜杠拒");
        assert!(!ok("1", "a:b.jpg"), "冒号拒");
        assert!(!ok("1", "x.jpg"), "长度不足");
        assert!(!ok("1", &format!("{uuid}.png")), "非 .jpg 后缀拒");
        assert!(!ok("1", &format!("{uuid}.jpg.jpg")), "双尾缀拒");
    }
}
