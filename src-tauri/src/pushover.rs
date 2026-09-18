//! Pushover 手机推送（反馈第二轮 F4，Q3/Q4/Q10）：凭证只读环境变量，不进库不进备份。
//! HTTP 经 `post` 参数注入（cargo test 不真发）；真实发送用 ureq（阻塞式，调度线程可承受）。

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

impl PushoverConfig {
    /// 两个变量都读到非空值才算已配置；缺任一 → None（未配置，不是错误）。
    pub fn from_env() -> Option<PushoverConfig> {
        let user = std::env::var(ENV_USER).ok()?.trim().to_string();
        let token = std::env::var(ENV_TOKEN).ok()?.trim().to_string();
        if user.is_empty() || token.is_empty() {
            return None;
        }
        Some(PushoverConfig { user, token })
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

/// 设置页状态探测：只报在/不在，不回报值。
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct PushoverStatus {
    pub user_found: bool,
    pub token_found: bool,
}

pub fn status_from_env() -> PushoverStatus {
    let found = |k: &str| std::env::var(k).map(|v| !v.trim().is_empty()).unwrap_or(false);
    PushoverStatus { user_found: found(ENV_USER), token_found: found(ENV_TOKEN) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

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
