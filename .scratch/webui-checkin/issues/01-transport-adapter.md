# 票 01 · 统一调用层预重构(双模式地基)

规格:`docs/superpowers/specs/20260919-webui-checkin-spec.md`(Implementation Decisions A)

## What to build

前端引入统一调用层(单一模块):桌面环境走 Tauri IPC、浏览器走 HTTP fetch(HTTP 实现本票先留接口,随票 05 接入端点后自然生效);把现有约 55 处组件内直接 `invoke()` 与 2 处 `listen()` 全部改为经调用层;既有逐组件 `vi.mock("@tauri-apps/api/core")` 测试迁移为集中 mock 调用层。**应用行为零变化**——本票是纯预重构,不改任何用户可见行为。

## 验收标准

- [ ] 组件层不再直接 import `invoke`/`listen`(仅调用层模块可以)
- [ ] 桌面模式下命令名/参数/返回与现状完全一致
- [ ] `pnpm test` 全绿,mock 收敛为单一模块
- [ ] `cargo test` 不受影响、全绿
- [ ] `vue-tsc --noEmit` 通过

## Blocked by

无,可立即开始
