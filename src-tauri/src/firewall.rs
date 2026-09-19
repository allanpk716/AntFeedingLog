//! 防火墙联动（webui-checkin 票 03，规格 B：固定规则名、入站、限定所选网段+端口）。
//!
//! 规则构造是**纯核**（字符串生成，可测）：固定规则名 [`FIREWALL_RULE_NAME`]、
//! 入站 TCP、`remoteip=所选网段逗号拼接`、`localport=端口`。
//!
//! 真机执行 [`sync`] 不进单测——UAC 提权弹窗只能人工冒烟（进晨报清单），纯核只
//! 保证命令文本正确。执行语义：
//! - **幂等**：启用 = 先删同名规则（「无匹配规则」不算失败）再建，重复启用不重复
//!   建规则；停用 = 只删（enabled=false → 删规则，规格 User Story 16）；
//! - **UAC 提权一次**：删+建两条 netsh 打进同一个提权 PowerShell 里跑
//!   （`Start-Process -Verb RunAs`），不是两次弹窗；
//! - 内层脚本以 `-EncodedCommand`（UTF-16LE base64，[`base64_utf16le`] 纯核）
//!   传递，绕开两层 shell 引号转义；
//! - 失败返回 [`FirewallError`]：消息 + **现成手动命令**（管理员终端照贴可执行），
//!   lib.rs 落 applog::log_error 并把两者带给前端。

/// 固定防火墙规则名（删/建都按它定位，不追旧规则名）。
pub const FIREWALL_RULE_NAME: &str = "AntFeedingLog WebUI";

/// 同步失败（消息 + 现成手动命令文本）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FirewallError {
    pub message: String,
    pub manual_cmd: String,
}

// ── 纯核心：命令文本 ─────────────────────────────────────────────────────

/// 添加规则的完整 netsh 命令（入站 TCP，限定所选网段+端口）。
pub fn add_rule_cmd(segments: &[String], port: u16) -> String {
    format!(
        "netsh advfirewall firewall add rule name=\"{FIREWALL_RULE_NAME}\" dir=in action=allow protocol=TCP localport={port} remoteip={}",
        segments.join(",")
    )
}

/// 删除同名规则的完整 netsh 命令（先删后建实现幂等；关网页端时单独用它）。
pub fn delete_rule_cmd() -> String {
    format!("netsh advfirewall firewall delete rule name=\"{FIREWALL_RULE_NAME}\"")
}

/// 失败时给前端的现成手动命令（管理员终端照贴可执行；含删+建两条）。
pub fn manual_hint(segments: &[String], port: u16) -> String {
    format!(
        "在管理员 PowerShell 中手动执行：\n{}\n{}",
        delete_rule_cmd(),
        add_rule_cmd(segments, port)
    )
}

/// 内层提权脚本（启用）：先删（输出丢弃，规则不存在也无所谓）再建，退出码取建规则的。
pub fn enable_script(delete_cmd: &str, add_cmd: &str) -> String {
    format!("$null = {delete_cmd} 2>$null\n{add_cmd}\nexit $LASTEXITCODE")
}

/// 内层提权脚本（停用）：只删；netsh「无匹配规则」（中英两态）不算失败。
pub fn disable_script(delete_cmd: &str) -> String {
    format!(
        "$o = {delete_cmd} 2>&1\nif ($LASTEXITCODE -ne 0 -and (($o -join ' ') -notmatch 'match the specified|没有与指定的标准相匹配')) {{ exit $LASTEXITCODE }}\nexit 0"
    )
}

/// 外层脚本：一次 UAC 提权跑内层脚本（EncodedCommand），透传退出码。
/// UAC 被拒（用户取消）时 Start-Process 抛错 → 外层非零退出。
pub fn elevate_script(inner_b64: &str) -> String {
    format!(
        "$p = Start-Process -FilePath 'powershell' -Verb RunAs -Wait -PassThru -WindowStyle Hidden -ArgumentList '-NoProfile','-EncodedCommand','{inner_b64}'; if ($null -eq $p) {{ exit 1 }}; exit $p.ExitCode"
    )
}

/// 字符串 → UTF-16LE 字节的 base64（PowerShell -EncodedCommand 的要求编码）。
/// 手写小实现避免仅为它引入 base64 依赖。
pub fn base64_utf16le(s: &str) -> String {
    let mut bytes = Vec::with_capacity(s.len() * 2);
    for u in s.encode_utf16() {
        bytes.extend_from_slice(&u.to_le_bytes());
    }
    base64_of(&bytes)
}

fn base64_of(data: &[u8]) -> String {
    const TBL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TBL[(n >> 18) as usize & 63] as char);
        out.push(TBL[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { TBL[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { TBL[n as usize & 63] as char } else { '=' });
    }
    out
}

// ── IO 薄层（真机执行；UAC 路径手动冒烟，不进单测）──────────────────────

/// 同步防火墙规则：enabled=true 建规则（先删后建幂等），false 删规则。
/// 内部起 PowerShell + UAC 提权；失败返回消息与手动命令，调用方落日志并给前端。
pub fn sync(enabled: bool, segments: &[String], port: u16) -> Result<(), FirewallError> {
    let inner = if enabled {
        enable_script(&delete_rule_cmd(), &add_rule_cmd(segments, port))
    } else {
        disable_script(&delete_rule_cmd())
    };
    let script = elevate_script(&base64_utf16le(&inner));
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
        .map_err(|e| FirewallError {
            message: format!("无法启动 PowerShell（防火墙联动需要它）: {e}"),
            manual_cmd: if enabled { manual_hint(segments, port) } else { delete_rule_cmd() },
        })?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
    Err(FirewallError {
        message: format!(
            "防火墙规则同步失败（退出码 {:?}）{}",
            out.status.code(),
            if stderr.is_empty() { String::new() } else { format!("：{stderr}") }
        ),
        manual_cmd: if enabled { manual_hint(segments, port) } else { delete_rule_cmd() },
    })
}

// ── 测试：只测外部行为（spec「Testing Decisions」）────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn segs() -> Vec<String> {
        vec!["100.84.0.0/16".to_string(), "192.168.1.0/24".to_string()]
    }

    // ── 规则命令文本（验收：固定规则名、入站、网段+端口限定）──

    #[test]
    fn add_rule_cmd_contains_name_dir_port_and_all_segments() {
        let cmd = add_rule_cmd(&segs(), 17321);
        assert!(cmd.starts_with("netsh advfirewall firewall add rule"), "实际：{cmd}");
        assert!(cmd.contains("name=\"AntFeedingLog WebUI\""), "固定规则名，实际：{cmd}");
        assert!(cmd.contains("dir=in"), "入站");
        assert!(cmd.contains("action=allow"));
        assert!(cmd.contains("protocol=TCP"));
        assert!(cmd.contains("localport=17321"), "端口限定，实际：{cmd}");
        assert!(cmd.contains("remoteip=100.84.0.0/16,192.168.1.0/24"), "网段逗号拼接，实际：{cmd}");
    }

    #[test]
    fn delete_rule_cmd_uses_same_fixed_name() {
        assert_eq!(
            delete_rule_cmd(),
            "netsh advfirewall firewall delete rule name=\"AntFeedingLog WebUI\""
        );
    }

    #[test]
    fn manual_hint_contains_both_commands() {
        let hint = manual_hint(&segs(), 17321);
        assert!(hint.contains("管理员"), "说明给谁执行，实际：{hint}");
        assert!(hint.contains(&delete_rule_cmd()), "含删规则命令");
        assert!(hint.contains(&add_rule_cmd(&segs(), 17321)), "含建规则命令（现成 netsh 文本）");
    }

    // ── 提权脚本组装（幂等与一次 UAC 的载体）──

    #[test]
    fn enable_script_deletes_first_then_adds() {
        let s = enable_script(&delete_rule_cmd(), &add_rule_cmd(&segs(), 17321));
        let del = s.find(&delete_rule_cmd()).expect("含删命令");
        let add = s.find(&add_rule_cmd(&segs(), 17321)).expect("含建命令");
        assert!(del < add, "先删后建（重复启用不重复建规则的基础）");
        assert!(s.contains("exit $LASTEXITCODE"), "退出码取建规则的");
    }

    #[test]
    fn disable_script_tolerates_missing_rule() {
        let s = disable_script(&delete_rule_cmd());
        assert!(s.contains(&delete_rule_cmd()));
        // 「无匹配规则」（中英）不判失败
        assert!(s.contains("match the specified"));
        assert!(s.contains("没有与指定的标准相匹配"));
        assert!(s.ends_with("exit 0"), "无匹配 → 0");
    }

    #[test]
    fn elevate_script_runs_inner_once_with_runas() {
        let s = elevate_script("QUJD");
        assert!(s.contains("-Verb RunAs"), "UAC 提权，实际：{s}");
        assert!(s.contains("-Wait"), "等内层跑完取退出码");
        assert!(s.contains("-EncodedCommand"));
        assert!(s.contains("'QUJD'"), "内层以 base64 传入");
    }

    // ── EncodedCommand 编码（UTF-16LE base64）──

    #[test]
    fn base64_utf16le_known_vectors() {
        // 独立参照：python -c "import base64;print(base64.b64encode('hi'.encode('utf-16-le')).decode())"
        assert_eq!(base64_utf16le("hi"), "aABpAA==");
        assert_eq!(base64_utf16le(""), "");
        assert_eq!(base64_utf16le("A"), "QQA=");
    }

    #[test]
    fn base64_of_standard_vectors() {
        // RFC 4648 经典向量
        assert_eq!(base64_of(b""), "");
        assert_eq!(base64_of(b"f"), "Zg==");
        assert_eq!(base64_of(b"fo"), "Zm8=");
        assert_eq!(base64_of(b"foo"), "Zm9v");
        assert_eq!(base64_of(b"foob"), "Zm9vYg==");
        assert_eq!(base64_of(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_of(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn base64_utf16le_roundtrip_through_known_decoder_shape() {
        // 长 ASCII 文本：每字符 2 字节、全低字节——断言长度与尾部 padding 形态
        let text = "netsh advfirewall firewall delete rule name=\"AntFeedingLog WebUI\" 2>&1";
        let b64 = base64_utf16le(text);
        let raw_len = text.encode_utf16().count() * 2;
        assert_eq!(b64.len(), raw_len.div_ceil(3) * 4);
        assert!(b64.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/' || b == b'='), "标准字母表");
    }
}
