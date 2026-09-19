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
//!   （[`SseTicketStore`]，建连即消费；消费/过期纯核可测，SSE 端点票 06 挂上）。
//!
//! 白名单默认拒绝：`POST /api/cmd` 只派发注册表 [`WEBUI_COMMANDS`] 里登记的
//! 命令（本票仅 `health_check`），其余一律 404；四个网页端配置命令（凭证明文/
//! 安全配置面）**永久禁入**，由 `registry_excludes_forbidden_commands` 钉死。
//!
//! 请求限制（规格 A）：请求体 ≤1MB（照片上传端点票 08 自行放宽到 15MB，框架层
//! 预留按路径 `DefaultBodyLimit::max` 的口子）、普通并发 ≤8、SSE 连接单独计量
//! ≤4（计数器本票落地，票 06 挂到 SSE 路由）、读超时 30 秒（hyper 连接层
//! header 读超时 + 请求处理超时中间件）、header 条数/单条长度/总体积上限。
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

use crate::webui_config;

// ── 常量：请求限制（规格 A）─────────────────────────────────────────────────

/// 普通请求体上限（1MB）。照片上传端点（票 08）在自己的路由上用
/// `DefaultBodyLimit::max(15MB)` 放宽，不动全局值。
pub const MAX_BODY_BYTES: usize = 1024 * 1024;

/// 普通并发上限（8）。SSE 长连接单独计量（[`ConnLimiter`] 双计数器）。
pub const MAX_CONCURRENT_REQUESTS: usize = 8;

/// SSE 长连接上限（4，不挤占普通并发额度；票 06 的 SSE 路由消费）。
pub const MAX_CONCURRENT_SSE: usize = 4;

/// 读超时（30 秒）：连接层 header 读超时（慢速请求防挂）+ 请求处理超时。
pub const READ_TIMEOUT: Duration = Duration::from_secs(30);

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

// 票 06 接线前 consume/prune/len/is_empty 只有测试在用；逐方法豁免而非空注
// 全 impl——issue 已在 sse_ticket_handler 生产路径上
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
    #[allow(dead_code)] // 票 06 SSE 建连中间件消费
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

// ── 纯核心：并发计量（普通 ≤8 与 SSE ≤4 分开计数）──────────────────────────

/// 计量种类：普通请求与 SSE 长连接各用各的额度。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitKind {
    Normal,
    /// 票 06 的 SSE 路由消费（本票先落计量器）。
    #[allow(dead_code)]
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
/// 本票（04）只登记 `health_check`；票 05 起按网页端功能面（规格 H：首页/打卡/
/// 历史/冬眠/巢况）逐个加入，设置/管理/备份/更新**永不入表**。
///
/// ⚠ **永久禁入清单**（不得加入本表，测试 [`tests::registry_excludes_forbidden_commands`]
/// 钉死，票 03 评审 Important 的落点）：
/// - `get_webui_config`——响应含**访问凭证明文**；
/// - `save_webui_config`——可改受信网段/端口/开关（闸一安全配置）；
/// - `regenerate_token`——返回新凭证明文且可作废全部旧地址；
/// - `get_access_url`——响应就是含凭证的完整访问地址。
///
/// 与 lib.rs 各命令 doc 注释的禁入标记同源；网页端的功能面不含任何设置操作（规格 H）。
pub const WEBUI_COMMANDS: &[&str] = &["health_check"];

/// 注册表派发：登记且有实现 → Some(命令结果)；登记了但没实现 → None（404）。
/// 未登记的命令根本不会走到这（handler 先查 [`WEBUI_COMMANDS`]）。
/// 命令执行与桌面 IPC 同一把库锁（[`crate::run_with_conn`]），单连接串行。
pub fn dispatch_command(
    deps: &SharedDeps,
    cmd: &str,
) -> Option<Result<serde_json::Value, String>> {
    match cmd {
        // 健康检查与桌面 `health_check` 命令同源：同一把库锁里查 schema 版本
        "health_check" => Some(crate::run_with_conn(&deps.conn, |conn| {
            crate::db::schema_version_of(conn)
                .map(|schema_version| serde_json::json!({ "schema_version": schema_version }))
                .map_err(|e| e.to_string())
        })),
        // 不存在「注册表里有但这里没有」的分支——registry_entries_all_have_real_dispatch
        // 钉住两边同步；走到这等于调用方没先查注册表
        _ => None,
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
}

impl SharedDeps {
    pub fn new(conn: Arc<Mutex<Connection>>, data_dir: PathBuf) -> Self {
        SharedDeps {
            conn,
            data_dir,
            tickets: SseTicketStore::new(),
            limiter: ConnLimiter::new(MAX_CONCURRENT_REQUESTS, MAX_CONCURRENT_SSE),
            read_timeout: READ_TIMEOUT,
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
    if let Some(len) = req
        .headers()
        .get(axum::http::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<usize>().ok())
    {
        if len > MAX_BODY_BYTES {
            // 照片上传端点（票 08）不经过本层或自带放宽——上传路由自行挂
            // DefaultBodyLimit::max(15MB) 的按路径覆盖层
            let reason = format!("请求体超过 {}MB 上限", MAX_BODY_BYTES / 1024 / 1024);
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

/// 请求处理超时（读超时的 handler 侧：慢请求最多占 30 秒）。
/// 注意：SSE 长连接（票 06）的建连响应立即可返回，不受此层牵连。
async fn timeout_mw(
    req: axum::extract::Request,
    next: Next,
) -> axum::response::Response {
    match tokio::time::timeout(READ_TIMEOUT, next.run(req)).await {
        Ok(resp) => resp,
        Err(_) => json_error(
            axum::http::StatusCode::REQUEST_TIMEOUT,
            &format!("请求处理超时（{} 秒内未完成）", READ_TIMEOUT.as_secs()),
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

/// `GET /api/health`：健康检查（**闸二豁免**——连通性探测不带凭证也能用；
/// 响应只有 schema 版本，无敏感信息）。闸一照拦（连接准入在前）。
async fn health_handler(State(deps): State<Shared>) -> axum::response::Response {
    respond_dispatch(dispatch_command(&deps, "health_check"))
}

fn respond_dispatch(
    outcome: Option<Result<serde_json::Value, String>>,
) -> axum::response::Response {
    use axum::http::StatusCode;
    match outcome {
        Some(Ok(v)) => json_response(StatusCode::OK, &v),
        Some(Err(e)) => json_error(StatusCode::INTERNAL_SERVER_ERROR, &e),
        None => json_error(StatusCode::NOT_FOUND, "unknown command"),
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
    // args 键名与桌面 invoke 入参对象一致（camelCase 原样）；票 05 命令接入时
    // 在派发层做 Tauri 同款 camelCase → snake_case 映射
    let _args = parsed.get("args").cloned().unwrap_or(serde_json::Value::Null);
    if !WEBUI_COMMANDS.contains(&cmd) {
        // deny-by-default 的常态 404，不刷日志（网段外根本进不来，段内探测
        // 不构成安全事件；真正的准入拒绝在闸一/闸二层记流水）
        return json_error(StatusCode::NOT_FOUND, "unknown command");
    }
    respond_dispatch(dispatch_command(&deps, cmd))
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

// ── HTTP 层：路由组装 ───────────────────────────────────────────────────────

/// 路由（自外向内）：全局体上限 → [限额 → 超时 → 闸二 → 端点]。
/// `/api/health` 不挂闸二（唯一豁免）；其余一切路径 404（axum 无路由默认）。
fn build_router(deps: Shared) -> axum::Router {
    use axum::extract::DefaultBodyLimit;
    use axum::middleware as mw;
    use axum::routing::{get, post};

    let api = axum::Router::new()
        .route("/api/cmd", post(cmd_handler))
        .route("/api/sse-ticket", post(sse_ticket_handler))
        .layer(mw::from_fn_with_state(deps.clone(), auth_mw))
        .layer(mw::from_fn(timeout_mw))
        .layer(mw::from_fn_with_state(deps.clone(), limits_mw))
        .with_state(deps.clone());
    let health = axum::Router::new()
        .route("/api/health", get(health_handler))
        .with_state(deps);
    health.merge(api).layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
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
        let deps = SharedDeps {
            conn: Arc::new(Mutex::new(conn)),
            data_dir: dir.path().to_path_buf(),
            tickets: SseTicketStore::new(),
            limiter: ConnLimiter::new(2, 2),
            read_timeout: Duration::from_secs(1),
        };
        for cmd in WEBUI_COMMANDS {
            assert!(
                dispatch_command(&deps, cmd).is_some(),
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
        let deps = SharedDeps {
            conn: Arc::new(Mutex::new(conn)),
            data_dir: dir.path().to_path_buf(),
            tickets: SseTicketStore::new(),
            limiter: ConnLimiter::new(2, 2),
            read_timeout: Duration::from_secs(1),
        };
        let out = dispatch_command(&deps, "health_check").unwrap().unwrap();
        assert_eq!(out["schema_version"], schema, "HTTP 派发与桌面 health_check 同源");
    }

    // ── 集成：真监听（127.0.0.1 随机端口）+ ureq 真请求 ──

    /// 组装测试依赖：临时数据目录 + 配置文件（token/网段可注入）+ 真库连接 +
    /// 1 秒读超时（慢速请求防挂秒级可验证）。返回 (deps, dir)。
    fn test_deps(
        segments: &[&str],
        token: &str,
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
        let deps = Arc::new(SharedDeps {
            conn: Arc::new(Mutex::new(conn)),
            data_dir: dir.path().to_path_buf(),
            tickets: SseTicketStore::new(),
            limiter: ConnLimiter::new(MAX_CONCURRENT_REQUESTS, MAX_CONCURRENT_SSE),
            read_timeout: Duration::from_secs(1),
        });
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

        // 未登记命令 → 404 {"error":"unknown command"}（deny-by-default）
        let (status, body) = http(
            &a,
            "POST",
            &format!("{base}/api/cmd"),
            Some(&token),
            r#"{"cmd":"list_colonies","args":{}}"#,
        );
        assert_eq!(status, 404);
        assert_eq!(body["error"], "unknown command");

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
}
