# 票 02 · 更新核心：updater 插件接入 + 每日检查 + 手动检查命令

## What to build

应用接入 tauri-plugin-updater 并具备"检查新版本"能力（Rust 侧全属主）：

- `tauri.conf.json`：`createUpdaterArtifacts: true`；updater 插件 `endpoints = ["https://github.com/allanpk716/AntFeedingLog/releases/latest/download/latest.json"]`（数组，留镜像扩展位）；公钥字段留占位（票 07 由用户填真实公钥）。
- Rust 侧注册 updater 插件；实现**每日一次静默检查**定时器（托盘常驻、窗口关闭也查；跨启动持久化"上次检查日"，避免重启就重查）。
- 实现 `check_update_now` Tauri command 供设置页调用。
- **错误路径**：每日静默检查的任何失败（404/网络/超时/解析失败）一律按"无更新"处理并记日志，绝不发通知；手动检查失败经 command 返回错误信息（由 UI 一次性展示）。
- 检查结果对外只区分三态：无更新 / 有新版（含版本号与说明）/ 检查失败（仅手动路径可见）。

## 验收标准

- [ ] cargo test：每日检查节奏（同一天只查一次；跨天重查）——时间源可注入
- [ ] cargo test：错误→"无更新"映射（每日路径静默；手动路径返回 Err）
- [ ] cargo test：检查结果三态的映射逻辑（网络层可注入/模拟清单）
- [ ] tauri.conf.json 配置就位（endpoints 数组、createUpdaterArtifacts、pubkey 占位）
- [ ] 不在前端直接调用 updater JS API（capabilities 不加 updater:default）

## Blocked by

无，可立即开始。
