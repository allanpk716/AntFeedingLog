# 票 01 · CI：main 分支测试+构建流水线

## What to build

push 到 main 时 GitHub Actions 自动跑：前端 Vitest + Rust cargo test + tauri build（仅构建、不发布）。目的：平时就暴露"CI 环境编不过"，而不是打 tag 当天才炸。runner 为 windows-latest，配 Rust 工具链与 pnpm、缓存依赖。**该 job 只读权限**（不声明 contents: write）。

## 验收标准

- [ ] `.github/workflows/` 下存在 main 触发的 workflow（push main + 可手动 workflow_dispatch）
- [ ] 步骤覆盖：checkout → pnpm 安装（带缓存）→ Rust 工具链（带缓存）→ `pnpm test` → `cargo test`（src-tauri 内）→ `pnpm tauri build`
- [ ] 未声明任何写权限；无发布/上传步骤
- [ ] YAML 语法可被 actionlint 或等价校验通过（本地校验，不 push）

## Blocked by

无，可立即开始。
