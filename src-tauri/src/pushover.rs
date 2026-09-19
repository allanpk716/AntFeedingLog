//! Pushover 手机推送（反馈第二轮 F4，Q3/Q4/Q10；webui-checkin 票 11 应用内配置）：
//! 凭据取值优先级 应用内 settings 键 > 环境变量 > 未启用；应用内值明文入库
//! （风险已明示接受，规格 G）。HTTP 经 `post` 参数注入（cargo test 不真发）；
//! 真实发送用 ureq（阻塞式，调度线程可承受）。凭据值永不出现在日志/错误里。

use rusqlite::Connection;
use serde::Serialize;

pub const API_URL: &str = "https://api.pushover.net/1/messages.json";
pub const ENV_USER: &str = "PUSHOVER_USER";
pub const ENV_TOKEN: &str = "PUSHOVER_TOKEN";
pub const TIMEOUT_SECS: u64 = 10;

#[derive(Debug, Clone, PartialEq)]
pub struct PushoverConfig {
    pub user: String,
    pub token: String,
}

/// 生效来源三态（票 11）：应用内 settings 键 / 系统环境变量 / 未配置。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PushoverSource {
    App,
    Env,
    None,
}

/// 三态判定结果：生效配置 + 生效来源（config 为 None 时 source 恒为 [`PushoverSource::None`]）。
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedPushover {
    pub config: Option<PushoverConfig>,
    pub source: PushoverSource,
}

/// 三态优先级纯核（票 11）：应用内两键都非空 → 用应用内；否则环境变量两键都
/// 非空 → 回落环境；都无 → 未启用（None，不是错误）。应用内两键现值原样传入
///（settings 读侧缺键回空串），环境变量值由调用方注入（生产读 std::env，测试
/// 注入）——本函数不碰任何真实环境，可纯测。生效值统一去首尾空白。
pub fn resolve_pushover_credentials(
    app_user: &str,
    app_token: &str,
    env_user: Option<&str>,
    env_token: Option<&str>,
) -> ResolvedPushover {
    let complete = |user: Option<&str>, token: Option<&str>| {
        let (u, t) = (user.unwrap_or("").trim(), token.unwrap_or("").trim());
        if u.is_empty() || t.is_empty() {
            None
        } else {
            Some(PushoverConfig { user: u.to_string(), token: t.to_string() })
        }
    };
    if let Some(config) = complete(Some(app_user), Some(app_token)) {
        return ResolvedPushover { config: Some(config), source: PushoverSource::App };
    }
    match complete(env_user, env_token) {
        Some(config) => ResolvedPushover { config: Some(config), source: PushoverSource::Env },
        None => ResolvedPushover { config: None, source: PushoverSource::None },
    }
}

pub type Form = Vec<(String, String)>;

/// 组表单经注入的传输层发送；错只在传输层。
pub fn send_with(
    cfg: &PushoverConfig,
    title: &str,
    message: &str,
    post: &dyn Fn(&str, &Form) -> Result<(), String>,
) -> Result<(), String> {
    let form: Form = vec![
        ("token".into(), cfg.token.clone()),
        ("user".into(), cfg.user.clone()),
        ("title".into(), title.to_string()),
        ("message".into(), message.to_string()),
    ];
    post(API_URL, &form)
}

/// 真实发送（ureq 负责表单 percent-encoding，中文标题/正文安全）。
pub fn send(cfg: &PushoverConfig, title: &str, message: &str) -> Result<(), String> {
    send_with(cfg, title, message, &|url, form| {
        let data: Vec<(&str, &str)> =
            form.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let agent = ureq::AgentBuilder::new()
            .timeout(std::time::Duration::from_secs(TIMEOUT_SECS))
            .build();
        match agent.post(url).send_form(&data) {
            Ok(resp) if resp.status() < 300 => Ok(()),
            Ok(resp) => Err(format!("Pushover 返回 HTTP {}", resp.status())),
            Err(e) => Err(format!("Pushover 发送失败: {e}")),
        }
    })
}

/// 设置页状态探测（票 11 三态）：只报生效来源与在/不在，不回报值。
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct PushoverStatus {
    pub source: PushoverSource,
    pub configured: bool,
}

/// 变量不存在或全空白都算没填（与纯核口径一致）。
fn env_var(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.trim().is_empty())
}

/// 状态探测（生产入口）：库内应用内配置 + 真实环境变量 → 三态。
/// 借库失败向上传播（command 层统一落日志）。
pub fn status_from_db(conn: &Connection) -> Result<PushoverStatus, String> {
    let (app_user, app_token) = crate::settings::get_pushover_credentials(conn)?;
    let r = resolve_pushover_credentials(
        &app_user,
        &app_token,
        env_var(ENV_USER).as_deref(),
        env_var(ENV_TOKEN).as_deref(),
    );
    Ok(PushoverStatus { source: r.source, configured: r.config.is_some() })
}

/// 发送取值（生产入口，票 11）：应用内 > 环境变量 > None。
/// 借库失败向上传播，由调用方落错误流水（凭据值不进日志）。
pub fn config_from_db(conn: &Connection) -> Result<Option<PushoverConfig>, String> {
    let (app_user, app_token) = crate::settings::get_pushover_credentials(conn)?;
    Ok(resolve_pushover_credentials(
        &app_user,
        &app_token,
        env_var(ENV_USER).as_deref(),
        env_var(ENV_TOKEN).as_deref(),
    )
    .config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    // ── 三态优先级纯核（webui-checkin 票 11）：环境值全部注入，不碰真实进程环境 ──

    #[test]
    fn app_credentials_win_over_env_when_both_present() {
        let r = resolve_pushover_credentials("u-app", "t-app", Some("u-env"), Some("t-env"));
        let cfg = r.config.expect("应用内已填应生效");
        assert_eq!(cfg.user, "u-app");
        assert_eq!(cfg.token, "t-app");
        assert_eq!(r.source, PushoverSource::App);
    }

    #[test]
    fn empty_app_falls_back_to_env() {
        let r = resolve_pushover_credentials("", "", Some("u-env"), Some("t-env"));
        let cfg = r.config.expect("应用内未填应回落环境变量");
        assert_eq!(cfg.user, "u-env");
        assert_eq!(cfg.token, "t-env");
        assert_eq!(r.source, PushoverSource::Env);
    }

    #[test]
    fn both_absent_means_disabled_not_error() {
        let r = resolve_pushover_credentials("", "", None, None);
        assert_eq!(r.config, None, "都无 = 未启用（None，不是错误）");
        assert_eq!(r.source, PushoverSource::None);
    }

    #[test]
    fn partial_app_falls_back_to_complete_env() {
        // 应用内只填一半不算已配置：整套回落环境变量（与 from_env 时代同口径）
        let r = resolve_pushover_credentials("u-app", "", Some("u-env"), Some("t-env"));
        assert_eq!(r.source, PushoverSource::Env);
        assert_eq!(r.config.map(|c| c.token), Some("t-env".into()));
    }

    #[test]
    fn partial_env_is_not_configured() {
        let r = resolve_pushover_credentials("", "", Some("u-env"), None);
        assert_eq!(r.config, None);
        assert_eq!(r.source, PushoverSource::None);
    }

    #[test]
    fn whitespace_only_values_count_as_empty_and_values_are_trimmed() {
        // 全空白 = 未填；生效值去首尾空白（粘贴事故防御）
        let r = resolve_pushover_credentials("  ", "t-app", Some(" u-env "), Some(" t-env "));
        assert_eq!(r.source, PushoverSource::Env);
        let cfg = r.config.expect("环境变量齐全应生效");
        assert_eq!(cfg.user, "u-env");
        assert_eq!(cfg.token, "t-env");
    }

    #[test]
    fn app_credentials_are_trimmed_too() {
        let r = resolve_pushover_credentials(" u-app ", "t-app", None, None);
        let cfg = r.config.expect("应用内齐全应生效");
        assert_eq!(cfg.user, "u-app");
        assert_eq!(r.source, PushoverSource::App);
    }

    #[test]
    fn send_with_builds_form_with_credentials_and_text() {
        let cfg = PushoverConfig { user: "u-测试用户".into(), token: "t-令牌".into() };
        // &dyn Fn 不能改捕获变量：用 RefCell 记录传输层被调用的一次
        let seen: RefCell<Option<(String, Form)>> = RefCell::new(None);
        send_with(&cfg, "该喂面包虫了", "「大头一号」已 7 天没喂面包虫", &|url, form| {
            *seen.borrow_mut() = Some((url.to_string(), form.clone()));
            Ok(())
        })
        .unwrap();
        let (url, form) = seen
            .into_inner()
            .expect("传输层应被调用一次");
        assert_eq!(url, API_URL);
        let get = |k: &str| form.iter().find(|(key, _)| key == k).unwrap().1.clone();
        assert_eq!(get("token"), "t-令牌");
        assert_eq!(get("user"), "u-测试用户");
        assert_eq!(get("title"), "该喂面包虫了");
        assert_eq!(get("message"), "「大头一号」已 7 天没喂面包虫");
    }

    #[test]
    fn send_with_propagates_transport_error() {
        let cfg = PushoverConfig { user: "u".into(), token: "t".into() };
        let err = send_with(&cfg, "标题", "正文", &|_, _| Err("DNS 解析失败".into())).unwrap_err();
        assert!(err.contains("DNS"), "实际错误：{err}");
    }
}
