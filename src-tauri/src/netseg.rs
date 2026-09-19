//! 网段枚举与身份识别（webui-checkin 票 03，规格 Implementation Decisions B 闸一）。
//!
//! 纯核心与 IO 解耦（沿 settings.rs/applog.rs 惯例）：输入是 [`NicInfo`] 列表
//! （接口名/描述/IP/前缀），测试注入构造、不依赖真机。
//! - [`classify`]：噪音过滤（VMware/VMnet/vEthernet/WSL/Hyper-V/Loopback，外加
//!   环回 127/8 与链路本地 169.254/16 这类无组网意义地址）→ **按接口身份**识别
//!   NetBird（接口名 `wt0`，或名/描述含 netbird）→ `encrypted_mesh: true` 置顶
//!   标名「NetBird 虚拟网」；其余网段一律 `encrypted_mesh: false`——落在
//!   100.64–127（CGNAT）的物理网段不因地址段享受免警示（CGNAT≠加密隧道，规格 B 明文）；
//! - [`validate_cidr`] / [`to_cidr`] / [`pick_ip_for_segment`]：CIDR 解析校验、
//!   主机位归零、按受信网段挑本机 IP（get_access_url 用）。只认 IPv4，IPv6 一律拒绝。
//!
//! IO 薄层 [`list_nics`]：PowerShell `Get-NetIPAddress`（ConvertTo-Json 输出，
//! 字段名与系统界面语言无关——`ipconfig /all` 文本解析会随系统语言漂移，故弃；
//! 不引 ipconfig 等第三方 crate，避免新增依赖面）→ [`parse_powershell_nics`]，
//! 再经 [`classify`] 成段。Windows 专有路径，非 Windows 报错不 panic。

use serde::Serialize;

/// 已确认加密身份网段的置顶标名（规格 B：按接口身份识别，非地址段推断）。
pub const NETBIRD_LABEL: &str = "NetBird 虚拟网";

/// 噪音过滤关键字（接口名/描述不分大小写匹配）：虚拟化与环回网卡不进列表。
const NOISE_KEYWORDS: [&str; 6] = ["vmware", "vmnet", "vethernet", "wsl", "hyper-v", "loopback"];

/// 一张网卡的 IPv4 信息（纯核注入输入；每个 IPv4 地址一行，双址网卡出两行）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NicInfo {
    /// 接口名（如 wt0、以太网）
    pub name: String,
    /// 提供方/适配器描述（如 NetBird WireGuard Tunnel、Realtek PCIe …）
    pub description: String,
    /// IPv4 点分地址
    pub ip: String,
    /// 前缀长度 0–32
    pub prefix: u8,
}

/// 枚举结果一行（前端 DTO，serde 默认 snake_case）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NetworkSegment {
    /// 归一后的 CIDR（主机位已归零，如 192.168.1.0/24）
    pub cidr: String,
    /// true = 接口身份已确认是加密 mesh（NetBird）；物理网段恒 false
    pub encrypted_mesh: bool,
    /// 置顶标名（仅 NetBird 段有）
    pub label: Option<String>,
}

// ── IPv4 纯数学 ──────────────────────────────────────────────────────────

/// 严格解析点分 IPv4（四段、每段 0–255、纯数字）；非法返回 None。
pub fn parse_ipv4(s: &str) -> Option<u32> {
    let mut octets = [0u8; 4];
    let mut count = 0usize;
    for part in s.split('.') {
        if count >= 4 || part.is_empty() || part.len() > 3 || !part.bytes().all(|b| b.is_ascii_digit())
        {
            return None;
        }
        let v: u32 = part.parse().ok()?;
        if v > 255 {
            return None;
        }
        octets[count] = v as u8;
        count += 1;
    }
    if count != 4 {
        return None;
    }
    Some(u32::from_be_bytes(octets))
}

/// u32 转点分字符串。
pub fn ipv4_to_string(v: u32) -> String {
    let b = v.to_be_bytes();
    format!("{}.{}.{}.{}", b[0], b[1], b[2], b[3])
}

/// 前缀长度 → 掩码（prefix=0 → 0；32 → u32::MAX）。
pub fn prefix_mask(prefix: u8) -> u32 {
    if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - prefix as u32)
    }
}

/// IP + 前缀 → 主机位归零的 CIDR（如 192.168.1.77/24 → 192.168.1.0/24）。
pub fn to_cidr(ip: &str, prefix: u8) -> Result<String, String> {
    if prefix > 32 {
        return Err(format!("前缀长度须在 0–32（当前：{prefix}）"));
    }
    let v = parse_ipv4(ip).ok_or_else(|| format!("不是合法 IPv4 地址：{ip}"))?;
    Ok(format!("{}/{}", ipv4_to_string(v & prefix_mask(prefix)), prefix))
}

/// 解析 CIDR 为（网络地址数值，前缀）；非法返回 None。
pub fn parse_cidr(s: &str) -> Option<(u32, u8)> {
    let (ip, prefix) = s.split_once('/')?;
    if prefix.len() > 2 || !prefix.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let p: u8 = prefix.parse().ok()?;
    if p > 32 {
        return None;
    }
    Some((parse_ipv4(ip)? & prefix_mask(p), p))
}

/// CIDR 字符串校验 + 归一：IPv4 专用，主机位归零；非法给中文原因。
pub fn validate_cidr(s: &str) -> Result<String, String> {
    let t = s.trim();
    let (ip, prefix) = t
        .split_once('/')
        .ok_or_else(|| format!("网段须为 a.b.c.d/p 形式（当前：{t}）"))?;
    let p: u8 = prefix
        .trim()
        .parse()
        .map_err(|_| format!("前缀长度不是 0–32 的数字（当前：{t}）"))?;
    to_cidr(ip.trim(), p)
}

// ── 身份识别与分类（纯核心）─────────────────────────────────────────────

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|k| haystack.contains(k))
}

/// 噪音网卡：接口名或描述命中虚拟化/环回关键字。
fn is_noise(name: &str, description: &str) -> bool {
    let (n, d) = (name.to_lowercase(), description.to_lowercase());
    contains_any(&n, &NOISE_KEYWORDS) || contains_any(&d, &NOISE_KEYWORDS)
}

/// NetBird 身份：接口名恰为 wt0，或名/描述含 netbird（不分大小写）。
fn is_netbird(name: &str, description: &str) -> bool {
    let (n, d) = (name.to_lowercase(), description.to_lowercase());
    n == "wt0" || n.contains("netbird") || d.contains("netbird")
}

/// 分类（纯核心）：过滤噪音 → 逐网卡成段（同 CIDR 去重）→ NetBird 段置顶。
/// 物理网段（含 CGNAT 地址）恒 `encrypted_mesh: false`。
pub fn classify(nics: &[NicInfo]) -> Vec<NetworkSegment> {
    let mut out: Vec<NetworkSegment> = Vec::new();
    for nic in nics {
        if is_noise(&nic.name, &nic.description) {
            continue;
        }
        let Ok(cidr) = to_cidr(&nic.ip, nic.prefix) else {
            continue;
        };
        // 环回（127/8）与链路本地自动配置（169.254/16）无组网意义，一并滤掉
        if let Some((net, _)) = parse_cidr(&cidr) {
            if net & prefix_mask(8) == parse_ipv4("127.0.0.0").unwrap_or(0x7F00_0000) {
                continue;
            }
            if net & prefix_mask(16) == parse_ipv4("169.254.0.0").unwrap_or(0xA9FE_0000) {
                continue;
            }
        }
        if out.iter().any(|s| s.cidr == cidr) {
            continue;
        }
        let encrypted = is_netbird(&nic.name, &nic.description);
        out.push(NetworkSegment {
            cidr,
            encrypted_mesh: encrypted,
            label: encrypted.then(|| NETBIRD_LABEL.to_string()),
        });
    }
    // 稳定排序：NetBird 段置顶，物理网段保持出现顺序
    out.sort_by_key(|s| !s.encrypted_mesh);
    out
}

/// 在网卡列表里找落在指定网段上的本机 IP（get_access_url 拼 `http://<IP>:<端口>` 用）。
pub fn pick_ip_for_segment(nics: &[NicInfo], cidr: &str) -> Option<String> {
    let (net, prefix) = parse_cidr(cidr)?;
    let mask = prefix_mask(prefix);
    nics.iter()
        .find(|nic| parse_ipv4(&nic.ip).is_some_and(|ip| ip & mask == net))
        .map(|nic| nic.ip.clone())
}

// ── IO 薄层（PowerShell，真机路径不进单测；解析纯函数直测）──────────────

/// PowerShell 取本机全部 IPv4 地址 + 接口名/描述（JSON 输出与系统语言无关）。
pub const PS_LIST_SCRIPT: &str = "$p = Get-NetIPAddress -AddressFamily IPv4 -ErrorAction SilentlyContinue | ForEach-Object { $d = (Get-NetAdapter -InterfaceIndex $_.InterfaceIndex -ErrorAction SilentlyContinue).InterfaceDescription; [PSCustomObject]@{ name = $_.InterfaceAlias; desc = $d; ip = $_.IPAddress; prefix = $_.PrefixLength } }; ConvertTo-Json -InputObject $p -Compress";

/// 解析 PowerShell ConvertTo-Json 输出为网卡信息（单对象/数组两形态都吃；
/// 缺 ip/坏 ip 的行跳过，name/desc 缺失兜底空串、缺 prefix 兜底 32）。
/// 纯函数，直测。
pub fn parse_powershell_nics(json: &str) -> Vec<NicInfo> {
    let v: serde_json::Value = match serde_json::from_str(json) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let items: Vec<serde_json::Value> = match v {
        serde_json::Value::Array(items) => items,
        obj @ serde_json::Value::Object(_) => vec![obj],
        _ => return Vec::new(),
    };
    items
        .into_iter()
        .filter_map(|it| {
            let ip = it.get("ip")?.as_str()?.to_string();
            let name = it
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or("")
                .to_string();
            let description = it
                .get("desc")
                .and_then(|d| d.as_str())
                .unwrap_or("")
                .to_string();
            let prefix = (it.get("prefix").and_then(|p| p.as_u64()).unwrap_or(32) as u8).min(32);
            if parse_ipv4(&ip).is_none() {
                return None;
            }
            Some(NicInfo {
                name,
                description,
                ip,
                prefix,
            })
        })
        .collect()
}

/// 跑 PowerShell 拿本机网卡信息（get_access_url 的「找本机 IP」也用它）。
pub fn list_nics() -> Result<Vec<NicInfo>, String> {
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", PS_LIST_SCRIPT])
        .output()
        .map_err(|e| format!("枚举网卡失败（无法启动 PowerShell）: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!("枚举网卡失败: {}", err.trim()));
    }
    Ok(parse_powershell_nics(&String::from_utf8_lossy(&out.stdout)))
}

/// 枚举并分类本机网段（list_network_segments 命令本体）。
pub fn enumerate() -> Result<Vec<NetworkSegment>, String> {
    Ok(classify(&list_nics()?))
}

// ── 测试：只测外部行为（spec「Testing Decisions」）────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn nic(name: &str, desc: &str, ip: &str, prefix: u8) -> NicInfo {
        NicInfo {
            name: name.to_string(),
            description: desc.to_string(),
            ip: ip.to_string(),
            prefix,
        }
    }

    // ── IPv4 纯数学 ──

    #[test]
    fn parse_ipv4_accepts_only_wellformed() {
        assert_eq!(parse_ipv4("192.168.1.7"), Some(0xC0A8_0107));
        assert_eq!(parse_ipv4("0.0.0.0"), Some(0));
        assert_eq!(parse_ipv4("255.255.255.255"), Some(u32::MAX));
        assert_eq!(parse_ipv4("256.1.1.1"), None, "超 255 拒绝");
        assert_eq!(parse_ipv4("1.2.3"), None, "少一段拒绝");
        assert_eq!(parse_ipv4("1.2.3.4.5"), None, "多一段拒绝");
        assert_eq!(parse_ipv4("a.b.c.d"), None);
        assert_eq!(parse_ipv4("1.2.3.-4"), None);
        assert_eq!(parse_ipv4(""), None);
        assert_eq!(parse_ipv4("1..2.3"), None);
    }

    #[test]
    fn prefix_mask_boundaries() {
        assert_eq!(prefix_mask(0), 0);
        assert_eq!(prefix_mask(8), 0xFF00_0000);
        assert_eq!(prefix_mask(24), 0xFFFF_FF00);
        assert_eq!(prefix_mask(32), u32::MAX);
    }

    #[test]
    fn to_cidr_normalizes_host_bits() {
        assert_eq!(to_cidr("192.168.1.77", 24).unwrap(), "192.168.1.0/24");
        assert_eq!(to_cidr("10.221.4.9", 10).unwrap(), "10.192.0.0/10");
        assert_eq!(to_cidr("100.84.3.7", 32).unwrap(), "100.84.3.7/32");
        assert!(to_cidr("1.2.3.4", 33).is_err(), "前缀越界拒绝");
        assert!(to_cidr("bad", 24).is_err(), "坏 IP 拒绝");
    }

    #[test]
    fn validate_cidr_rules() {
        assert_eq!(validate_cidr(" 192.168.1.77/24 ").unwrap(), "192.168.1.0/24");
        assert_eq!(validate_cidr("10.0.0.0/8").unwrap(), "10.0.0.0/8");
        // IPv6 一律拒绝（规格 B：IPv4 CIDR 匹配）
        assert!(validate_cidr("fe80::/64").is_err());
        assert!(validate_cidr("192.168.1.0").is_err(), "无斜杠拒绝");
        assert!(validate_cidr("192.168.1.0/33").is_err());
        assert!(validate_cidr("192.168.1.0/-1").is_err());
        assert!(validate_cidr("192.168.1.0/abc").is_err());
        let err = validate_cidr(" nonsense ").unwrap_err();
        assert!(err.contains("a.b.c.d/p"), "拒绝原因含合法形式，实际：{err}");
    }

    // ── 分类：NetBird 置顶标名 ──

    #[test]
    fn netbird_identified_by_interface_name_wt0() {
        let segs = classify(&[nic("wt0", "", "100.84.12.3", 16)]);
        assert_eq!(segs.len(), 1);
        assert!(segs[0].encrypted_mesh, "wt0 = NetBird 身份");
        assert_eq!(segs[0].label.as_deref(), Some(NETBIRD_LABEL));
        assert_eq!(segs[0].cidr, "100.84.0.0/16");
    }

    #[test]
    fn netbird_identified_by_description_case_insensitive() {
        for (name, desc) in [
            ("NetBird", "NetBird WireGuard Tunnel"),
            ("以太网 3", "netbird network adapter"),
        ] {
            let segs = classify(&[nic(name, desc, "10.0.0.5", 24)]);
            assert_eq!(segs.len(), 1, "{name}/{desc}");
            assert!(segs[0].encrypted_mesh, "{name}/{desc} 应按接口身份识别为加密 mesh");
            assert_eq!(segs[0].label.as_deref(), Some(NETBIRD_LABEL));
        }
    }

    // ── 分类：虚拟化噪音不出现在列表 ──

    #[test]
    fn virtualization_noise_filtered() {
        let nics = vec![
            nic("VMware Network Adapter VMnet1", "VMware Virtual Ethernet Adapter", "192.168.11.1", 24),
            nic("vEthernet (WSL)", "Hyper-V Virtual Ethernet Adapter", "172.22.0.1", 20),
            nic("以太网 2", "VMware Virtual Ethernet Adapter for VMnet8", "192.168.22.1", 24),
            nic("Loopback Pseudo-Interface 1", "", "127.0.0.1", 8),
        ];
        assert!(classify(&nics).is_empty(), "VMware/WSL/Hyper-V/环回全滤，实际：{:?}", classify(&nics));
    }

    #[test]
    fn loopback_and_apipa_addresses_filtered() {
        let nics = vec![
            nic("以太网", "Realtek PCIe GbE", "127.0.0.9", 8),
            nic("本地连接", "Some Adapter", "169.254.33.7", 16),
        ];
        assert!(classify(&nics).is_empty(), "环回/APIPA 地址滤掉");
    }

    // ── 分类：物理网段恒不加密（CGNAT 不享受免警示，规格 B）──

    #[test]
    fn physical_cgnat_segment_stays_unencrypted() {
        let segs = classify(&[nic("以太网", "Realtek PCIe GbE Family Controller", "100.66.12.3", 10)]);
        assert_eq!(segs.len(), 1);
        assert!(!segs[0].encrypted_mesh, "100.64–127（CGNAT）但身份未确认 → 仍走硬警示");
        assert_eq!(segs[0].label, None);
        assert_eq!(segs[0].cidr, "100.64.0.0/10");
    }

    #[test]
    fn wt0_cgnat_is_encrypted_but_plain_cgnat_is_not() {
        let nics = vec![
            nic("以太网", "Realtek", "100.100.1.5", 24),
            nic("wt0", "NetBird WireGuard Tunnel", "100.84.9.9", 16),
        ];
        let segs = classify(&nics);
        assert_eq!(segs.len(), 2);
        assert!(segs[0].encrypted_mesh, "NetBird 段置顶");
        assert_eq!(segs[0].cidr, "100.84.0.0/16");
        assert!(!segs[1].encrypted_mesh, "同地址段的物理网卡不跟着享受免警示");
        assert_eq!(segs[1].cidr, "100.100.1.0/24");
    }

    #[test]
    fn physical_segments_keep_order_and_dedupe() {
        let nics = vec![
            nic("以太网", "Realtek", "192.168.1.10", 24),
            nic("Wi-Fi", "Intel Wi-Fi", "10.0.0.4", 24),
            nic("以太网 2", "Realtek 二号", "192.168.1.20", 24), // 同网段 → 去重
        ];
        let segs = classify(&nics);
        let cidrs: Vec<&str> = segs.iter().map(|s| s.cidr.as_str()).collect();
        assert_eq!(cidrs, vec!["192.168.1.0/24", "10.0.0.0/24"], "保持出现顺序且去重");
    }

    #[test]
    fn invalid_ip_rows_skipped() {
        let nics = vec![nic("坏行", "Bad", "999.1.1.1", 24), nic("wt0", "", "100.84.1.1", 24)];
        let segs = classify(&nics);
        assert_eq!(segs.len(), 1, "坏 IP 行跳过");
        assert!(segs[0].encrypted_mesh);
    }

    #[test]
    fn empty_input_yields_empty() {
        assert!(classify(&[]).is_empty());
    }

    // ── 网段 → 本机 IP ──

    #[test]
    fn pick_ip_finds_address_inside_segment() {
        let nics = vec![
            nic("VMware", "", "192.168.11.1", 24), // 噪音行照参与（过滤在上层，函数只管数学）
            nic("以太网", "", "192.168.1.10", 24),
            nic("wt0", "", "100.84.12.3", 16),
        ];
        assert_eq!(
            pick_ip_for_segment(&nics, "100.84.0.0/16").as_deref(),
            Some("100.84.12.3")
        );
        assert_eq!(
            pick_ip_for_segment(&nics, "192.168.1.55/24").as_deref(),
            Some("192.168.1.10"),
            "主机位不参与匹配，只比对网络位"
        );
        assert_eq!(pick_ip_for_segment(&nics, "172.30.0.0/16"), None, "无匹配返回 None");
        assert_eq!(pick_ip_for_segment(&nics, "坏 CIDR"), None, "非法网段返回 None");
    }

    // ── PowerShell JSON 解析（单对象/数组/脏行）──

    #[test]
    fn parse_powershell_nics_array_form() {
        let json = r#"[
            {"name":"wt0","desc":"NetBird WireGuard Tunnel","ip":"100.84.12.3","prefix":16},
            {"name":"以太网","desc":"Realtek PCIe GbE","ip":"192.168.1.10","prefix":24},
            {"name":"Loopback Pseudo-Interface 1","desc":null,"ip":"127.0.0.1","prefix":8}
        ]"#;
        let nics = parse_powershell_nics(json);
        assert_eq!(nics.len(), 3);
        assert_eq!(nics[0].name, "wt0");
        assert_eq!(nics[0].prefix, 16);
        assert_eq!(nics[1].description, "Realtek PCIe GbE");
        assert_eq!(nics[2].description, "", "desc=null 归空串（后续按名过滤）");
    }

    #[test]
    fn parse_powershell_nics_single_object_form() {
        // ConvertTo-Json 对单条结果输出对象而非数组
        let json = r#"{"name":"以太网","desc":"Realtek","ip":"10.1.2.3","prefix":24}"#;
        let nics = parse_powershell_nics(json);
        assert_eq!(nics.len(), 1);
        assert_eq!(nics[0].ip, "10.1.2.3");
    }

    #[test]
    fn parse_powershell_nics_tolerates_junk() {
        assert!(parse_powershell_nics("").is_empty(), "空输出（无 IPv4 网卡）");
        assert!(parse_powershell_nics("不是 JSON").is_empty());
        assert!(parse_powershell_nics("null").is_empty());
        // 缺字段/坏 IP 的行跳过；缺 prefix 补 32
        let json = r#"[
            {"name":"缺ip","desc":"","prefix":24},
            {"name":"坏ip","ip":"1.2.3.999","prefix":24},
            {"name":"缺prefix","ip":"10.0.0.1"},
            {"ip":"10.0.0.2"}
        ]"#;
        let nics = parse_powershell_nics(json);
        assert_eq!(nics.len(), 2, "缺 ip/坏 ip 的行跳过");
        assert_eq!(nics[0].prefix, 32, "缺 prefix 兜底 32（单主机段）");
        assert_eq!(nics[1].name, "", "缺 name 兜空串");
    }
}
