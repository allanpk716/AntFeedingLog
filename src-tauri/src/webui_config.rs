//! 网页端配置与访问凭证（webui-checkin 票 03）。
//!
//! 独立 JSON 文件 [`WEBUI_CONFIG_FILE`]（数据目录下，照 backup_config.rs 先例）：
//! **不进 SQLite settings 表**——恢复整库不回滚网页端设置、备份/恢复不触碰
//! （规格 B 闸二：凭证文件与库无关）。访问凭证只由程序生成（≥128bit 随机，
//! [`TOKEN_BYTES`]），没有「自定义凭证」的写入路径；重生成 = 直接覆盖旧值
//! （旧地址即刻作废）。
//!
//! 纯核心与 IO 解耦（沿 settings.rs/backup_config.rs 惯例）：
//! - [`validate_port`]：1024–65535 之外拒绝；
//! - [`validate_segments`]：逐项 CIDR（IPv4）校验并归一去重；
//! - [`merge_input`]：保存入参合并进现有配置（凭证字段原样保留，前端不可覆写），
//!   开着总开关却零网段拒绝；
//! - [`build_access_url`] / [`token_from_bytes`] / [`regenerate_with`]。
//!
//! IO 薄层（TempDir 直测）：
//! - [`load`]：文件缺失/损坏按默认值、读后收敛脏值（端口出界回默认、非法网段丢弃）；
//! - [`save`]：pretty JSON 原子写（temp+rename，数据目录缺失先建）；
//! - [`load_ready`]：load + 凭证为空即生成并落盘（get_webui_config 入口）；
//! - [`save_webui`] / [`regenerate_token`]：读→改→写，全程持单写者锁。
//!
//! Tauri command 薄封装在 lib.rs；防火墙联动也在那边（save 成功后执行）。

use std::path::Path;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::applog;
use crate::netseg::validate_cidr;

/// 网页端配置文件名（数据目录下，与库/backup-config.json 平级）。
pub const WEBUI_CONFIG_FILE: &str = "webui-config.json";

/// 端口默认值（规格 A）。
pub const DEFAULT_PORT: u16 = 17321;

/// 端口下界（1024–65535）。
pub const PORT_MIN: i64 = 1024;

/// 端口上界（1024–65535）。
pub const PORT_MAX: i64 = 65535;

/// 凭证随机字节数（128 bit；十六进制展开 32 字符）。
pub const TOKEN_BYTES: usize = 16;

/// 网页端配置（字段名即 JSON 键，serde 默认 snake_case，前端契约对齐）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WebUiConfig {
    /// 网页端开关（默认关，规格 A）。
    pub enabled: bool,
    /// 受信网段（归一 CIDR，闸一白名单；顺序即用户勾选顺序）。
    pub segments: Vec<String>,
    /// 服务端口 1024–65535（默认 17321）。
    pub port: u16,
    /// 访问凭证（十六进制 32 字符；空串 = 尚未生成）。
    pub token: String,
    /// 凭证生成时间（`YYYY-MM-DD HH:MM:SS`）；None = 尚未生成。
    pub token_generated_at: Option<String>,
}

impl Default for WebUiConfig {
    fn default() -> Self {
        WebUiConfig {
            enabled: false,
            segments: Vec::new(),
            port: DEFAULT_PORT,
            token: String::new(),
            token_generated_at: None,
        }
    }
}

/// save_webui_config 入参：只有用户可改的三项（开关/网段/端口）。
/// 凭证与生成时间不在此列——前端不可覆写，只可走 regenerate_token。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct WebUiSaveInput {
    pub enabled: bool,
    pub segments: Vec<String>,
    pub port: i64,
}

// ── 纯核心 ───────────────────────────────────────────────────────────────

/// 端口校验：1024–65535 之外拒绝并给出含范围的提示。
pub fn validate_port(port: i64) -> Result<u16, String> {
    if !(PORT_MIN..=PORT_MAX).contains(&port) {
        return Err(format!("端口须在 {PORT_MIN}–{PORT_MAX} 之间（当前：{port}）"));
    }
    Ok(port as u16)
}

/// 网段列表校验：逐项 CIDR（IPv4）校验并归一（主机位归零）、去重保序。
pub fn validate_segments(segments: &[String]) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = Vec::new();
    for s in segments {
        let cidr = validate_cidr(s)?;
        if !out.contains(&cidr) {
            out.push(cidr);
        }
    }
    Ok(out)
}

/// 合并保存入参：端口/网段拒绝式校验（非法整体失败），凭证字段原样保留；
/// 开着总开关却零网段 → 拒绝（闸一白名单不能为空）。
pub fn merge_input(current: &WebUiConfig, input: &WebUiSaveInput) -> Result<WebUiConfig, String> {
    let port = validate_port(input.port)?;
    let segments = validate_segments(&input.segments)?;
    if input.enabled && segments.is_empty() {
        return Err("启用网页端至少要选择一个受信网段".into());
    }
    Ok(WebUiConfig {
        enabled: input.enabled,
        segments,
        port,
        token: current.token.clone(),
        token_generated_at: current.token_generated_at.clone(),
    })
}

/// 完整访问地址：`http://<IP>:<端口>/#token=<凭证>`（fragment 不随请求发给服务器）。
pub fn build_access_url(ip: &str, port: u16, token: &str) -> String {
    format!("http://{ip}:{port}/#token={token}")
}

/// 随机字节 → 十六进制凭证（32 随机字节组 → 64… 不，16 字节 → 32 字符）。
pub fn token_from_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// 重生成凭证（纯核心）：新凭证 + 生成时间，其余字段原样。
/// 旧值被直接覆盖即「旧地址即刻作废」（规格 User Story 3）。
pub fn regenerate_with(current: &WebUiConfig, new_token: String, now: &str) -> WebUiConfig {
    WebUiConfig {
        token: new_token,
        token_generated_at: Some(now.to_string()),
        ..current.clone()
    }
}

// ── IO 薄层（TempDir 直测）───────────────────────────────────────────────

/// 配置单写者锁（照 backup_config 先例）：设置保存与凭证重生成/补生成都走
/// 「load → 改 → save」，全程持锁，交错不丢更新。
static CONFIG_FILE_LOCK: Mutex<()> = Mutex::new(());

/// 拿配置写锁（锁毒化按原值续用：配置是提示性数据，不值得 panic）。
fn lock_config_file() -> std::sync::MutexGuard<'static, ()> {
    CONFIG_FILE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// 读后收敛：端口出界回默认；逐项网段收敛（非法丢弃、去重保序）。
/// 手改配置文件的脏值兜底，不挡启动。
pub fn sanitize(config: WebUiConfig) -> WebUiConfig {
    let port_ok = (PORT_MIN..=PORT_MAX).contains(&(config.port as i64));
    let mut segments: Vec<String> = Vec::new();
    for s in &config.segments {
        if let Ok(cidr) = validate_cidr(s) {
            if !segments.contains(&cidr) {
                segments.push(cidr);
            }
        }
    }
    WebUiConfig {
        port: if port_ok { config.port } else { DEFAULT_PORT },
        segments,
        ..config
    }
}

/// 读配置：文件缺失/损坏/半损坏按默认值（serde(default) 补齐缺字段），读后收敛。
/// 绝不 panic、不挡启动。
pub fn load(data_dir: &Path) -> WebUiConfig {
    let path = data_dir.join(WEBUI_CONFIG_FILE);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return WebUiConfig::default(),
        Err(e) => {
            eprintln!("[webui_config] 读配置失败（按默认值继续）: {e}");
            return WebUiConfig::default();
        }
    };
    match serde_json::from_str::<WebUiConfig>(&text) {
        Ok(config) => sanitize(config),
        Err(e) => {
            eprintln!("[webui_config] 配置文件损坏（按默认值重建）: {e}");
            WebUiConfig::default()
        }
    }
}

/// 写配置（pretty JSON）。数据目录缺失先建；原子写（temp+rename，同 backup_config）。
pub fn save(data_dir: &Path, config: &WebUiConfig) -> Result<(), String> {
    std::fs::create_dir_all(data_dir).map_err(|e| format!("创建数据目录失败: {e}"))?;
    let text =
        serde_json::to_string_pretty(config).map_err(|e| format!("序列化网页端配置失败: {e}"))?;
    let path = data_dir.join(WEBUI_CONFIG_FILE);
    let tmp = data_dir.join(format!("{WEBUI_CONFIG_FILE}.tmp"));
    std::fs::write(&tmp, &text).map_err(|e| format!("写入网页端配置临时文件失败: {e}"))?;
    if let Err(e) = std::fs::rename(&tmp, &path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("替换网页端配置失败: {e}"));
    }
    Ok(())
}

/// OS 级随机生成新凭证（≥128bit；只程序生成，无自定义写入路径）。
pub fn generate_token() -> Result<String, String> {
    let mut bytes = [0u8; TOKEN_BYTES];
    getrandom::fill(&mut bytes).map_err(|e| format!("生成访问凭证失败（系统随机源不可用）: {e}"))?;
    Ok(token_from_bytes(&bytes))
}

/// 凭证为空则生成并落盘（锁内调用，不加锁）。
fn ensure_token(mut config: WebUiConfig, data_dir: &Path) -> Result<WebUiConfig, String> {
    if !config.token.is_empty() {
        return Ok(config);
    }
    config.token = generate_token()?;
    config.token_generated_at = Some(applog::now_local());
    save(data_dir, &config)?;
    Ok(config)
}

/// 读配置并确保凭证已生成（get_webui_config / get_access_url / regenerate_token
/// 共用入口）。首次调用即在数据目录落盘凭证。
pub fn load_ready(data_dir: &Path) -> Result<WebUiConfig, String> {
    let _guard = lock_config_file();
    ensure_token(load(data_dir), data_dir)
}

/// 读 → 合并校验 → 写（save_webui_config 编排）。校验失败不落盘、原配置原样；
/// 凭证为空顺路补生成。全程持单写者锁。
pub fn save_webui(data_dir: &Path, input: &WebUiSaveInput) -> Result<WebUiConfig, String> {
    let _guard = lock_config_file();
    let current = load(data_dir);
    let mut next = merge_input(&current, input)?;
    if next.token.is_empty() {
        next.token = generate_token()?;
        next.token_generated_at = Some(applog::now_local());
    }
    save(data_dir, &next)?;
    Ok(next)
}

/// 重生成凭证（regenerate_token 命令编排）：load_ready 后覆盖写新凭证。
/// 旧凭证即刻失效 = 直接覆盖（不保留旧值、不做灰度）。
pub fn regenerate_token(data_dir: &Path) -> Result<WebUiConfig, String> {
    let _guard = lock_config_file();
    let current = ensure_token(load(data_dir), data_dir)?;
    let next = regenerate_with(&current, generate_token()?, &applog::now_local());
    save(data_dir, &next)?;
    Ok(next)
}

// ── 测试：只测外部行为（spec「Testing Decisions」）────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn temp_data_dir() -> TempDir {
        TempDir::new().expect("创建临时目录失败")
    }

    fn input(enabled: bool, segments: &[&str], port: i64) -> WebUiSaveInput {
        WebUiSaveInput {
            enabled,
            segments: segments.iter().map(|s| s.to_string()).collect(),
            port,
        }
    }

    fn config_with_token() -> WebUiConfig {
        WebUiConfig {
            enabled: true,
            segments: vec!["100.84.0.0/16".into()],
            port: 17321,
            token: "abcd1234".into(),
            token_generated_at: Some("2026-09-19 08:00:00".into()),
        }
    }

    // ── 默认值（规格 A：开关=关、端口=17321、凭证未生成）──

    #[test]
    fn defaults_are_disabled_default_port_no_token() {
        let c = WebUiConfig::default();
        assert!(!c.enabled, "网页端默认关");
        assert!(c.segments.is_empty());
        assert_eq!(c.port, DEFAULT_PORT);
        assert_eq!(c.port, 17321);
        assert_eq!(c.token, "", "凭证默认未生成");
        assert_eq!(c.token_generated_at, None);
    }

    // ── 端口校验（验收：1024–65535）──

    #[test]
    fn port_out_of_range_rejected() {
        for bad in [0, 80, 443, 1023, 65536, 100_000, -1] {
            assert!(validate_port(bad).is_err(), "{bad} 应拒绝");
        }
        let err = validate_port(1023).unwrap_err();
        assert!(err.contains("1024") && err.contains("65535"), "提示含合法范围，实际：{err}");
    }

    #[test]
    fn port_boundaries_accepted() {
        assert_eq!(validate_port(1024).unwrap(), 1024);
        assert_eq!(validate_port(17321).unwrap(), 17321);
        assert_eq!(validate_port(65535).unwrap(), 65535);
    }

    // ── 网段列表校验 ──

    #[test]
    fn segments_validated_normalized_deduped() {
        let out = validate_segments(&["192.168.1.77/24".into(), " 10.0.0.0/8 ".into(), "192.168.1.9/24".into()])
            .unwrap();
        assert_eq!(out, vec!["192.168.1.0/24", "10.0.0.0/8"], "归一主机位并去重保序");
        assert!(validate_segments(&["垃圾".into()]).is_err(), "非法 CIDR 整体拒绝");
        assert!(validate_segments(&["fe80::/64".into()]).is_err(), "IPv6 拒绝");
        assert!(validate_segments(&[]).unwrap().is_empty());
    }

    // ── 合并保存入参（save_webui 核心）──

    #[test]
    fn merge_input_updates_three_user_fields_keeps_token() {
        let current = config_with_token();
        let next = merge_input(&current, &input(false, &["10.1.1.9/24"], 20480)).unwrap();
        assert!(!next.enabled);
        assert_eq!(next.segments, vec!["10.1.1.0/24"]);
        assert_eq!(next.port, 20480);
        assert_eq!(next.token, current.token, "凭证不被设置保存冲掉");
        assert_eq!(next.token_generated_at, current.token_generated_at);
    }

    #[test]
    fn merge_input_rejects_enabled_with_no_segments() {
        let current = WebUiConfig::default();
        let err = merge_input(&current, &input(true, &[], 17321)).unwrap_err();
        assert!(err.contains("网段"), "实际：{err}");
        // 关着的时候允许先存端口/网段草稿
        assert!(merge_input(&current, &input(false, &[], 17321)).is_ok());
    }

    #[test]
    fn merge_input_rejects_invalid_port_or_segment() {
        let current = WebUiConfig::default();
        assert!(merge_input(&current, &input(true, &["10.0.0.0/8"], 80)).is_err());
        assert!(merge_input(&current, &input(true, &["垃圾"], 17321)).is_err());
    }

    // ── URL 与凭证 ──

    #[test]
    fn access_url_contains_fragment_token() {
        assert_eq!(
            build_access_url("100.84.12.3", 17321, "a1b2c3"),
            "http://100.84.12.3:17321/#token=a1b2c3",
            "完整地址 = http://<IP>:<端口>/#token=<凭证>"
        );
    }

    #[test]
    fn token_from_bytes_is_lowercase_hex() {
        assert_eq!(token_from_bytes(&[]), "");
        assert_eq!(token_from_bytes(&[0x00, 0x0f, 0xa5]), "000fa5");
        assert_eq!(token_from_bytes(&[0xff; 8]), "f".repeat(16));
    }

    #[test]
    fn generated_token_is_128bit_hex_and_unique() {
        let t1 = generate_token().unwrap();
        let t2 = generate_token().unwrap();
        assert_eq!(t1.len(), 32, "16 字节 → 32 hex 字符（128 bit）");
        assert!(t1.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
        assert_ne!(t1, t2, "两次生成必须不同（随机源坏掉才会撞，概率可忽略）");
    }

    #[test]
    fn regenerate_with_replaces_token_and_keeps_rest() {
        let current = config_with_token();
        let next = regenerate_with(&current, "newtoken".into(), "2026-09-19 09:00:00");
        assert_eq!(next.token, "newtoken");
        assert_eq!(next.token_generated_at.as_deref(), Some("2026-09-19 09:00:00"));
        assert!(next.enabled, "其余字段原样");
        assert_eq!(next.segments, current.segments);
        assert_eq!(next.port, current.port);
    }

    // ── 文件读写（TempDir 直测）──

    #[test]
    fn save_then_load_roundtrips_all_fields() {
        let dir = temp_data_dir();
        let want = config_with_token();
        save(dir.path(), &want).unwrap();
        assert!(dir.path().join(WEBUI_CONFIG_FILE).exists(), "文件落在数据目录");
        assert_eq!(load(dir.path()), want, "读写往返保真");
    }

    #[test]
    fn load_missing_file_returns_defaults() {
        let dir = temp_data_dir();
        assert_eq!(load(dir.path()), WebUiConfig::default());
    }

    #[test]
    fn load_corrupted_file_returns_defaults_without_panicking() {
        let dir = temp_data_dir();
        std::fs::write(dir.path().join(WEBUI_CONFIG_FILE), "不是 JSON{{{").unwrap();
        assert_eq!(load(dir.path()), WebUiConfig::default());
    }

    #[test]
    fn load_partial_json_fills_missing_fields_with_defaults() {
        let dir = temp_data_dir();
        std::fs::write(dir.path().join(WEBUI_CONFIG_FILE), r#"{"enabled":true}"#).unwrap();
        let c = load(dir.path());
        assert!(c.enabled, "有值字段照读");
        assert_eq!(c.port, DEFAULT_PORT, "缺字段补默认");
        assert_eq!(c.token, "");
    }

    #[test]
    fn load_sanitizes_dirty_values() {
        // 手改脏值兜底：端口出界回默认、非法网段丢弃、合法网段归一
        let dir = temp_data_dir();
        std::fs::write(
            dir.path().join(WEBUI_CONFIG_FILE),
            r#"{"enabled":true,"port":80,"segments":["192.168.1.77/24","垃圾"],"token":"t"}"#,
        )
        .unwrap();
        let c = load(dir.path());
        assert_eq!(c.port, DEFAULT_PORT, "端口 80 回默认");
        assert_eq!(c.segments, vec!["192.168.1.0/24"], "非法网段丢弃、合法归一");
    }

    #[test]
    fn save_creates_missing_data_dir_and_no_tmp_leftover() {
        let dir = temp_data_dir();
        let data_dir = dir.path().join("nested").join("data");
        save(&data_dir, &WebUiConfig::default()).unwrap();
        assert!(data_dir.join(WEBUI_CONFIG_FILE).exists());
        save(&data_dir, &config_with_token()).unwrap();
        assert_eq!(load(&data_dir), config_with_token(), "覆盖后整体是新值");
        let leftovers: Vec<String> = std::fs::read_dir(&data_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "无临时文件残留，实际：{leftovers:?}");
    }

    // ── D1 同款：库与配置零联动（配置不随恢复整库回滚）──

    #[test]
    fn db_operations_do_not_touch_webui_config() {
        let dir = temp_data_dir();
        save_webui(dir.path(), &input(true, &["10.0.0.5/8"], 20480)).unwrap();

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
        assert!(c.enabled, "网页端配置不随库动");
        assert_eq!(c.segments, vec!["10.0.0.0/8"]);
        assert_eq!(c.port, 20480);
    }

    // ── load_ready：凭证补生成且落盘持久 ──

    #[test]
    fn load_ready_generates_token_once_and_persists() {
        let dir = temp_data_dir();
        let first = load_ready(dir.path()).unwrap();
        assert_eq!(first.token.len(), 32, "首次读取即补生成 128bit 凭证");
        assert!(first.token_generated_at.is_some());
        let second = load_ready(dir.path()).unwrap();
        assert_eq!(second.token, first.token, "第二次读同盘不换凭证（生成一次）");
        // 文件里就是它（手翻配置文件可见，凭证不进库）
        let on_disk = load(dir.path());
        assert_eq!(on_disk.token, first.token);
    }

    // ── save_webui：校验失败不落盘；凭证顺路补生成 ──

    #[test]
    fn save_webui_rejected_input_leaves_file_untouched() {
        let dir = temp_data_dir();
        save_webui(dir.path(), &input(false, &["10.0.0.5/8"], 20480)).unwrap();
        let err = save_webui(dir.path(), &input(true, &[], 17321)).unwrap_err();
        assert!(err.contains("网段"), "实际：{err}");
        let c = load(dir.path());
        assert!(!c.enabled && c.port == 20480, "拒绝后原配置原样");
        assert_eq!(c.segments, vec!["10.0.0.0/8"]);
    }

    #[test]
    fn save_webui_generates_token_when_absent() {
        let dir = temp_data_dir();
        let saved = save_webui(dir.path(), &input(true, &["10.0.0.5/8"], 17321)).unwrap();
        assert_eq!(saved.token.len(), 32, "启用保存时凭证已就绪");
        let reread = load(dir.path());
        assert_eq!(reread.token, saved.token, "落盘一致");
    }

    // ── regenerate_token：旧值覆盖即作废（User Story 3）──

    #[test]
    fn regenerate_token_overwrites_old_value_immediately() {
        let dir = temp_data_dir();
        let before = save_webui(dir.path(), &input(true, &["10.0.0.5/8"], 17321)).unwrap();
        let after = regenerate_token(dir.path()).unwrap();
        assert_ne!(after.token, before.token, "旧凭证即刻失效（覆盖写）");
        assert_eq!(after.segments, before.segments, "网段端口不受重生成影响");
        assert_eq!(after.port, before.port);
        assert_eq!(after.enabled, before.enabled);
        assert!(after.token_generated_at.is_some());
        assert_eq!(load(dir.path()).token, after.token, "落盘是新凭证");
    }

    // ── 并发保护：单写者锁（双线程交错不丢更新）──

    #[test]
    fn concurrent_updates_do_not_lose_writes() {
        let dir = temp_data_dir();
        let data_dir = dir.path();
        std::thread::scope(|s| {
            // 线程甲：反复改端口（1024..=1024+n）
            s.spawn(|| {
                for i in 0..20i64 {
                    save_webui(data_dir, &input(false, &["10.0.0.5/8"], 1024 + i)).unwrap();
                }
            });
            // 线程乙：反复保存开关开+固定网段
            s.spawn(|| {
                for _ in 0..20 {
                    save_webui(data_dir, &input(true, &["10.1.0.5/16"], 17321)).unwrap();
                }
            });
        });
        let final_config = load(data_dir);
        // 最后一次写完整幸存（而不是两个线程各自写了一半的字段拼盘）
        let is_thread_a_final = final_config.port >= 1024 + 19 && final_config.port < 2000;
        let is_thread_b_final = final_config.port == 17321 && final_config.enabled;
        assert!(
            is_thread_a_final || is_thread_b_final,
            "最终配置必须是某一次完整写入，实际：{final_config:?}"
        );
    }

    // ── 前端契约：字段 snake_case（serde 默认，同 backup_config 契约测试先例）──

    #[test]
    fn config_serializes_snake_case_for_frontend() {
        let c = config_with_token();
        let json = serde_json::to_string(&c).unwrap();
        for key in ["enabled", "segments", "port", "token", "token_generated_at"] {
            assert!(json.contains(&format!("\"{key}\"")), "缺字段 {key}，实际：{json}");
        }
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["token_generated_at"], "2026-09-19 08:00:00");
    }
}
