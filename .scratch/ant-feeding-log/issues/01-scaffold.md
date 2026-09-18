# 票 01 · 项目骨架与数据库地基

## What to build

可构建可测试的应用骨架：Tauri 2 壳 + Vue 3 + TypeScript + Naive UI + Vitest 前端；Rust 侧用 rusqlite 持有 SQLite（journal_mode=DELETE），启动时执行空库→v1 的迁移（建齐 spec「数据 schema」节的全部表 + 预置数据：四种操作、三种食物、两个地点），数据库文件放系统应用数据目录；首页渲染空状态（"暂无窝"）。前端与 Rust 之间先打通一个最小的健康检查命令（如返回 schema 版本），验证 IPC 通路。

## 验收标准

- [ ] `cargo test` 通过，且含迁移测试：空库建全部表、预置四操作/三食物/两地点、user_version=1
- [ ] `cargo check` 与前端 `pnpm build` 成功（不要求跑 GUI）
- [ ] Vitest 跑通至少一个冒烟测试
- [ ] IPC 健康检查命令返回 schema 版本 1
- [ ] 数据库文件创建在系统应用数据目录（非项目目录）
- [ ] journal_mode 为 DELETE（有测试或启动日志佐证）

## Blocked by

无，可立即开始
