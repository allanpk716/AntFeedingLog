# 共识文档：打 tag 自动构建发布 + 应用内自动更新（2026-09-18）

## 背景

- 项目：AntFeedingLog，Tauri 2 桌面应用（Vue 3 + naive-ui 前端，Rust 后端，pnpm + Vite + Vitest），面向 Windows 单机使用，托盘常驻 + 开机自启。
- 仓库：`github.com/allanpk716/AntFeedingLog`（公开仓库），已有 tag `v0.1.0`（无对应 release），尚无 `.github/` 目录（无任何 CI）。
- 版本号现状：`package.json` / `src-tauri/tauri.conf.json` / `src-tauri/Cargo.toml` 三处各持有版本号（当前均为 0.1.0）。
- 构建配置：`tauri.conf.json` 的 `bundle.targets = "all"`（Windows 下产出 NSIS 安装器与 MSI 两种安装包）。
- 数据：SQLite 单文件库（rusqlite bundled），库内 `PRAGMA user_version` 逐版本迁移，项目规格既定"只升不降"。

## 目标

1. push 版本 tag 后，GitHub Actions 自动构建，产物自动挂到 GitHub Release。
2. 应用内自动更新：检测新版本 → 用户确认 → 下载并安装。

## 约束（用户明确要求）

- 更新检测与下载**不得调用 api.github.com**（未认证限流 60 次/小时/IP）；由应用自行下载并解析 release 下的内容。

## 已核实的事实

- 受限流影响的只有 `api.github.com`；`github.com` 网页与 release 附件下载不占该配额。
- GitHub release 页面 HTML **不含**附件链接：资源列表由浏览器二次请求未文档化接口 `/releases/expanded_assets/<tag>` 异步加载，直接解析页面 HTML 脆弱、随时可被改版破坏。
- `https://github.com/<owner>/<repo>/releases/latest` 在存在 release 时 302 跳转到 `/releases/tag/vX.Y.Z`，Location 头即最新版本号（2026-09-18 实测：当前无 release，跳到 `/releases` 空列表页）。
- Tauri 2 官方更新插件（tauri-plugin-updater）的检查端点可指向任意普通文件 URL，无需任何 API 调用；`endpoints` 配置为数组、支持多地址；强制 minisign 签名校验，伪造包无法通过。

## 决策清单

### 一、发布管线（打 tag → 自动编译 → GitHub Release）

1. **触发**：push `v*` tag → GitHub Actions 在 Windows x64 runner 构建（仅 Windows，不做 macOS/Linux）。
2. **产物**：release 挂 NSIS 安装器（正典升级目标）+ MSI（备用）+ `latest.json`（更新清单）+ `.sig` 签名文件。
3. **版本纪律**：发版前手工同步三处版本号；CI 校验 tag 与 `tauri.conf.json` 的 version 不一致即失败。
4. **Release 形态**：构建完成直接 publish（非草稿）；notes 用 GitHub 原生 generate_release_notes 从 commits 生成，事后可手改。
5. **main 分支**：每次 push 跑前端 Vitest + cargo test + 构建但不发布，提前暴露"CI 环境编不过"。
6. **存量 tag**：v0.1.0 保持孤儿状态不补 release；首个自动发布版本为 **v0.2.0**（整块新功能）。
7. **签名密钥**：生成设密码的 minisign 密钥对；公钥进 `tauri.conf.json`（入 git），私钥双副本——GitHub Secrets（CI 签名用）+ 密码管理器（灾备）。

### 二、应用内更新（自 v0.2.0 起生效）

8. **机制**：tauri-plugin-updater；检查端点 = `https://github.com/allanpk716/AntFeedingLog/releases/latest/download/latest.json`——普通文件下载、本地解析，零 api.github.com 调用。
9. **节奏**：托盘常驻进程每日一次静默检查 + 设置页"立即检查"按钮；定时器在 Rust 侧（窗口关闭也能查）。
10. **行为**：发现新版 → 提示；用户确认后才下载；**绝不静默自动安装**。
11. **升级前快照**：安装更新前复制 SQLite 数据库到 `backups/pre-update-v<版本>.db`；快照失败不阻塞升级，但明确提示。
12. **网络**：只配 GitHub 官方直连；`endpoints` 数组留扩展位，将来可加镜像前缀（签名校验保证镜像无法投毒）。
13. **回滚**：不可回滚（规格既定"数据库只升不降"）；最坏情况 = 用快照手工恢复数据 + 手动安装旧包。

## 实现层既定细节（不再讨论）

- workflow 用 windows-latest runner；配置 Rust 工具链、pnpm、缓存；tag 过滤 `v[0-9]+.*`。
- `tauri.conf.json` 增加 `createUpdaterArtifacts: true`。
- 更新 UI 挂进现有 SettingsDialog 新增一节；系统通知复用 tauri-plugin-notification。
- 遵循项目 TDD 惯例：版本比较、快照逻辑、检查节奏先写失败测试再实现。

## 边界条件

- "零 API 匿名下载"的前提是仓库保持**公开**；转私有则 release 附件下载需要 token，更新机制需重新设计。
- 已安装的 0.1.0 没有更新能力，首次升级需手动安装 0.2.0。
- 升级走 NSIS 静默安装（`/S`），安装器负责替换运行中的 exe；应用退出前先做快照。

## Out of Scope

- macOS / Linux 构建
- 便携版 zip 分发
- 静默自动更新（不做无感升级）
- 更新下载镜像（留扩展位，暂不启用）
