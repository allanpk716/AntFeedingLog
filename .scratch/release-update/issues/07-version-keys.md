# 票 07 · 首发 0.2.0：版本号三处统一 + 密钥与发版操作手册

## What to build

为首次真实发版 v0.2.0 做好一切本地准备（真实 push tag 与密钥生成属用户白天操作，夜里不做）：

- 三处版本号统一改为 **0.2.0**：package.json、src-tauri/tauri.conf.json、src-tauri/Cargo.toml（含 Cargo.lock 刷新）。
- 写 `docs/release.md` 操作手册（面向明早的用户）：
  1. 生成 minisign 密钥对（`pnpm tauri signer generate`，设密码）；
  2. 把公钥填进 tauri.conf.json 的 updater pubkey 占位（票 02 预留）；
  3. GitHub 仓库 Secrets 配置 `TAURI_SIGNING_PRIVATE_KEY` 与 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`；
  4. 私钥另存密码管理器（灾备，警告：私钥绝不入 git）；
  5. 打 tag `v0.2.0` 并 push → 验收 Release 产物（setup.exe / msi / latest.json / .sig）；
  6. 手动安装 0.2.0（预期 SmartScreen 警告，属正常）。
- README 开发指南补一行"发版看 docs/release.md"。

## 验收标准

- [ ] 三处版本号一致为 0.2.0；`cargo metadata`/`pnpm` 构建配置可解析（lock 刷新无漂移）
- [ ] docs/release.md 覆盖上述 6 步，含 SmartScreen 预期说明与私钥安全警告
- [ ] 手册中的命令与仓库实际脚本/结构对应（无凭空路径）
- [ ] README 指向发版手册

## Blocked by

票 02（pubkey 占位与 conf 形状先就位）。
