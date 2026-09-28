# 票 04 · PowerShell 闪窗修复：两处调用加 CREATE_NO_WINDOW

## What to build

设置页「网页」tab 打开/保存时不再弹出 PowerShell 窗体：`netseg.rs` 的网卡枚举与 `firewall.rs` 的防火墙规则同步两处 PowerShell 子进程调用加 Windows 隐藏窗口创建旗标。

规格：`docs/superpowers/specs/20260928-checkin-photo-entry-spec.md`（Implementation Decisions 末条 + User Story 9）。

## 验收标准

- [ ] `src-tauri/src/netseg.rs` `list_nics()`：Command 构造改为可变量 + `#[cfg(windows)]` 下 `creation_flags(0x08000000)`（CREATE_NO_WINDOW；`use std::os::windows::process::CommandExt` 同 cfg）；非 Windows 编译不受影响
- [ ] `src-tauri/src/firewall.rs` 同款处理（找到 PowerShell 调用点，≈L114）
- [ ] 两处仅改窗口旗标，参数/脚本/错误处理逐字不变；不改数据流（两次网卡查询不复用）
- [ ] `cargo test`（netseg/firewall 过滤）全绿；`cargo check` 无新警告
- [ ] 真机验证留人工：设置页开「网页」tab、保存配置，无窗体弹出（晨报注明）

## Blocked by

无，可立即开始。

## 涉及路径

- src-tauri/src/netseg.rs
- src-tauri/src/firewall.rs

## 副作用声明

- `cargo test netseg firewall`（模块过滤）+ `cargo check` 为本票局部验证；不动端口。

decision_refs: D7
review_blocks: 无
