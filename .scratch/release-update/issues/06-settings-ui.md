# 票 06 · 设置页：检查更新一节

## What to build

现有 SettingsDialog 新增"检查更新"一节（前端只经 Tauri command，不直接调 updater JS API）：

- 展示当前版本号。
- "立即检查更新"按钮 → 调 `check_update_now`：无更新给平静的"已是最新"；有新版展示版本号与说明；失败给一次性错误提示（不重试轰炸）。
- 有新版时展示"下载并安装"确认操作 → 调 `confirm_and_install`，展示下载进度；完成提示"将重启以完成安装"。
- 启动检测发现"上次升级未完成"时（`get_update_state`），在设置页顶部或应用通知给出引导文案。
- 沿用设置页现有 UI 模式（naive-ui、中文文案、既有分区样式）。

## 验收标准

- [ ] Vitest：设置页新增节的渲染与三态（无更新/有新版/失败）展示
- [ ] Vitest：确认安装的交互流（确认→进度→完成提示；取消不触发安装）
- [ ] Vitest：升级未完成引导文案在 update_state 为"失败残留"时出现
- [ ] 前端不 import updater JS API；capabilities 未新增 updater:default

## Blocked by

票 02（check_update_now）、票 05（confirm_and_install / get_update_state）。
