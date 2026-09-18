# 票 03 · CI：tag 触发自动发布 Release

## What to build

push `v[0-9]+.*` tag 时 GitHub Actions 自动构建并把产物发布到 GitHub Release（直接 publish 非草稿）：

- **版本校验第一步**（失败即中止 job）：tag == tauri.conf.json 的 version；且 package.json / tauri.conf.json / Cargo.toml 三处版本号互相一致。校验逻辑写成可单测的脚本（本地可跑），workflow 调用它。
- 用 `tauri-apps/tauri-action` 构建，显式配置 `includeUpdaterJson: true` 与 `updaterJsonPreferNsis: true`；注入 `GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}` 与签名变量 `TAURI_SIGNING_PRIVATE_KEY` / `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`（均来自 GitHub Secrets，仓库无这些 Secret 时允许 job 失败——这是用户白天配置项）。
- Release 操作只用 **upsert 语义**（重跑覆盖上传），**不删除已发布 release**；tauri-action 建完后 `gh release edit <tag> --generate-notes` 补自动 notes。
- job 级 `permissions: contents: write`；仅 Windows x64。

## 验收标准

- [ ] tag 触发 workflow 存在（`v[0-9]+.*` 过滤）且与 main job 权限分离（本 job 才有 contents: write）
- [ ] 版本一致性校验脚本有单测：三处一致+tag 匹配通过；任一不一致给出明确错误（cargo test 或 vitest，按脚本语言定）
- [ ] workflow 含 includeUpdaterJson / updaterJsonPreferNsis / GITHUB_TOKEN / 两个签名环境变量
- [ ] workflow 无删除 release 的步骤；notes 用 `gh release edit --generate-notes` 补
- [ ] YAML 语法本地校验通过（不 push）

## Blocked by

票 01（workflow 结构先立）、票 02（createUpdaterArtifacts 配置先就位，否则清单无从产出）。
