# 共识文档：打 tag 自动构建发布 + 应用内自动更新（2026-09-18，rev1）

> 相对初稿（同目录 `20260918-release-update-consensus.md`）的修订：初稿经一轮外部评审收到 9 条已查实的修改意见，本版全部并入。修订点：①决策 11 快照机制写死为复用本仓既有"拿锁+拷贝"实现；②决策 3 版本校验从一处升为三处一致；③实现层补 workflow 权限与签名环境变量；④决策 2/实现层写明 latest.json 由官方 tauri-action 自动生成；⑤决策 9/10 补错误路径约定（404/网络错误按无更新静默处理）；⑥边界条件补 SmartScreen 预期与更新安装时序说明。

## 背景

- 项目：AntFeedingLog，Tauri 2 桌面应用（Vue 3 + naive-ui 前端，Rust 后端，pnpm + Vite + Vitest），面向 Windows 单机使用，托盘常驻 + 开机自启。
- 仓库：`github.com/allanpk716/AntFeedingLog`（公开仓库），已有 tag `v0.1.0`（无对应 release），尚无 `.github/` 目录（无任何 CI）。
- 版本号现状：`package.json` / `src-tauri/tauri.conf.json` / `src-tauri/Cargo.toml` 三处各持有版本号（当前均为 0.1.0）。
- 构建配置：`tauri.conf.json` 的 `bundle.targets = "all"`（Windows 下产出 NSIS 安装器与 MSI 两种安装包）。
- 数据：SQLite 单文件库（rusqlite bundled）。**本仓既定约定（上轮评审规则 11）：`journal_mode = DELETE`，无 -wal/-shm 伴生文件，拷贝单 .db 文件即完整快照**（db.rs 打开时强制该 PRAGMA 并断言生效）。库内 `PRAGMA user_version` 逐版本迁移，项目规格既定"只升不降"。
- 既有备份设施（票 09）：`system.rs::backup_db_file` 拷贝库文件到目标路径；`lib.rs::backup_to` 做法是"短暂拿锁挡住并发写 → 拷贝"；有"备份产物可被应用正式通道重开、数据完整"的测试覆盖。

## 目标

1. push 版本 tag 后，GitHub Actions 自动构建，产物自动挂到 GitHub Release。
2. 应用内自动更新：检测新版本 → 用户确认 → 下载并安装。

## 约束（用户明确要求）

- 更新检测与下载**不得调用 api.github.com**（未认证限流 60 次/小时/IP）；由应用自行下载并解析 release 下的内容。

## 已核实的事实

- 受限流影响的只有 `api.github.com`；`github.com` 网页与 release 附件下载不占该配额。
- GitHub release 页面 HTML **不含**附件链接：资源列表由浏览器二次请求未文档化接口 `/releases/expanded_assets/<tag>` 异步加载，直接解析页面 HTML 脆弱、随时可被改版破坏。
- `https://github.com/<owner>/<repo>/releases/latest` 在存在 release 时 302 跳转到 `/releases/tag/vX.Y.Z`（**跳过 prerelease**）；当前无 release 时跳 `/releases` 空列表页（2026-09-18 实测）。
- Tauri 2 官方更新插件（tauri-plugin-updater）的检查端点可指向任意普通文件 URL，无需任何 API 调用；`endpoints` 配置为数组、支持多地址；强制 minisign 签名校验，伪造包无法通过。
- GITHUB_TOKEN 在新仓库/组织默认只读；workflow 创建 Release 必须显式声明 `permissions: contents: write`。

## 决策清单

### 一、发布管线（打 tag → 自动编译 → GitHub Release）

1. **触发**：push `v*` tag → GitHub Actions 在 Windows x64 runner 构建（仅 Windows，不做 macOS/Linux）。
2. **产物**：release 挂 NSIS 安装器（正典升级目标）+ MSI（备用）+ `latest.json`（更新清单，内嵌各平台安装包 URL 与 minisign 签名字符串）+ `.sig` 签名文件。**清单生成机制**：CI 用官方 `tauri-apps/tauri-action`（release 模式）——它在 `createUpdaterArtifacts: true` 时自动产出更新工件（安装包的 .sig）并生成/上传 `latest.json`，Windows 条目自动指向 NSIS 安装器；不手写清单组装。
3. **版本纪律**：发版前手工同步三处版本号（package.json / tauri.conf.json / Cargo.toml）；**CI 校验两件事，任一不一致即失败：① tag == tauri.conf.json 的 version；② 三处版本号互 相一致**（防止只改了一处）。
4. **Release 形态**：构建完成直接 publish（非草稿）；notes 用 GitHub 原生 generate_release_notes 从 commits 生成，事后可手改。发布步骤用幂等方式（如 `gh release` 的 upsert 语义或先删再建），workflow 重跑不会留下半成品 release。
5. **main 分支**：每次 push 跑前端 Vitest + cargo test + 构建但不发布，提前暴露"CI 环境编不过"。
6. **存量 tag**：v0.1.0 保持孤儿状态不补 release；首个自动发布版本为 **v0.2.0**（整块新功能）。不使用 prerelease（与 `releases/latest` 跳过 prerelease 的行为天然一致，无需额外处理）。
7. **签名密钥**：生成设密码的 minisign 密钥对；公钥进 `tauri.conf.json`（入 git），私钥双副本——GitHub Secrets（`TAURI_SIGNING_PRIVATE_KEY` 与 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`，CI 签名时以环境变量注入 workflow）+ 密码管理器（灾备）。

### 二、应用内更新（自 v0.2.0 起生效）

8. **机制**：tauri-plugin-updater；检查端点 = `https://github.com/allanpk716/AntFeedingLog/releases/latest/download/latest.json`——普通文件下载、本地解析，零 api.github.com 调用。
9. **节奏**：托盘常驻进程每日一次静默检查 + 设置页"立即检查"按钮；定时器在 Rust 侧（窗口关闭也能查）。**错误路径约定：每日静默检查的任何失败（404——尚无 release 或清单资产缺失、网络错误、超时、清单解析失败）一律按"无更新"处理并记日志，绝不弹错误通知打扰**；仅设置页手动"立即检查"失败时给一次性失败提示。
10. **行为**：发现新版 → 提示；用户确认后才下载；**绝不静默自动安装**。
11. **升级前快照（机制写死）**：安装更新前，**复用票 09 既有备份设施**——短暂拿锁挡住并发写 → 拷贝 .db 单文件到 `backups/pre-update-v<版本>.db`（journal_mode=DELETE 下拷贝即完整快照，无 -wal 伴生文件，本仓既定约定；该机制已有"产物可重开、数据完整"测试覆盖）。**不引入 VACUUM INTO / backup API**——与既有实现保持一致，避免两套备份语义。快照失败不阻塞升级，但明确提示。
12. **网络**：只配 GitHub 官方直连；`endpoints` 数组留扩展位，将来可加镜像前缀（签名校验保证镜像无法投毒）。
13. **回滚**：不可回滚（规格既定"数据库只升不降"）；最坏情况 = 用快照手工恢复数据 + 手动安装旧包。

## 实现层既定细节（不再讨论）

- workflow 用 windows-latest runner；配置 Rust 工具链、pnpm、缓存；tag 过滤 `v[0-9]+.*`。
- **workflow 显式声明 `permissions: contents: write`**（创建/上传 Release 必需；默认只读 token 会发布失败）。
- 签名 secrets 以环境变量 `TAURI_SIGNING_PRIVATE_KEY` / `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` 注入 tauri-action。
- `tauri.conf.json` 增加 `createUpdaterArtifacts: true`。
- 更新 UI 挂进现有 SettingsDialog 新增一节；系统通知复用 tauri-plugin-notification。
- 遵循项目 TDD 惯例：版本比较、快照逻辑、检查节奏先写失败测试再实现。

## 边界条件

- "零 API 匿名下载"的前提是仓库保持**公开**；转私有则 release 附件下载需要 token，更新机制需重新设计。
- 已安装的 0.1.0 没有更新能力，首次升级需手动安装 0.2.0。
- **SmartScreen 预期**：本项目不做 Authenticode 代码签名（决策 7 的 minisign 只保证更新内容完整性，不消除 Windows 的"未知发布者"警告）；手动安装或更新安装时触发 SmartScreen 警告属预期行为，不是投毒信号。
- **更新安装时序**：Windows 上 tauri-plugin-updater 内建"下载 → 退出应用 → 启动 NSIS 安装器（静默 /S）→ 安装器负责替换运行中的 exe →（配置后）重启应用"流程；应用退出前的快照挂在更新确认流程内（决策 11），安装器执行失败由插件的 install 结果回传给 UI 提示，不出现"旧进程未退出、用户状态不明"的静默半失败。

## Out of Scope

- macOS / Linux 构建
- 便携版 zip 分发
- 静默自动更新（不做无感升级）
- 更新下载镜像（留扩展位，暂不启用）
- Authenticode 代码签名（接受 SmartScreen 警告）

---

## xcheck 评审附录 · 20260918-181223

> **以下为评审参考,以实际执行为准**(验证证据是评审时点的快照,代码可能已演进);
> 但"撞上关注项"的动作不是参考 —— 停下反馈用户,别默默绕过。

### 两轮评审总账

- **round 0**（产物 `.xcheck/20260918-174810/`）：codex/pi 双 SUGGEST_CHANGES，10 条意见 → 9 条当场查实（9/9 证实）→ 已全部并入本文 rev1。查证关键发现：快照的"WAL 丢内容"风险前提在本仓不成立——`journal_mode=DELETE` 是既有约定（db.rs 打开时强制并断言，上轮评审规则 11），票 09"拿锁+拷贝"即完整快照且有测试；故 rev1 采用复用既有机制而非引入 VACUUM INTO。
- **round 1**（产物 `.xcheck/20260918-181223/`，对象=本 rev1）：codex/pi 双 SUGGEST_CHANGES，9 条 → 9/9 证实。终态：**夜间收工**——下列清单带入开发，未再出 rev2。

### ① 查实的（round 1 清单，开发时逐条落实）

1. `[codex]`[高·遗漏] tauri-action 需显式 `includeUpdaterJson: true`；NSIS/MSI 并产时还需 `updaterJsonPreferNsis: true`（否则清单缺失或 Windows 条目不指向 NSIS）—— 证据：tauri-action 公开 inputs
2. `[pi]`+`[codex]`[中·风险/bug] "安装器失败由 install 结果回传 UI"断言**不成立**：Windows/NSIS 下插件 spawn 安装器后应用即退出，安装器异步执行，中途失败（权限/文件占用/杀毒拦截）无法回传 —— 开发动作：本文边界条件该句改为"已知残余风险"；实现加 next-run 状态标记或启动时检测版本兜底（"点了升级→应用退出→没升上去"要有引导）
3. `[pi]`[中·风险] 快照挂点写死插件 `onBeforeExit` 钩子（退出前、数据流已停），并注明锁持有窗口 —— 证据：插件公开 Rust API
4. `[codex]`[中·风险] 快照文件名防覆盖（源版本+时间戳），且快照完成后进入禁写状态直到安装流程结束 —— 证据：确定性命名 `pre-update-v<版本>.db` 会被同版本重试/重装覆盖
5. `[pi]`[低·含糊] release 职责写死一种次序：tauri-action 建 release → `gh release edit --generate-notes` 补自动 notes（tauri-action 无 auto-notes 选项）
6. `[codex]`[中·风险] 发布操作只用 upsert（编辑/覆盖上传），**禁止删除已发布 release**（先删再建使 URL 短暂失效、缓存清单与新安装器签名错配）；"release 先可见、资产后上"窗口期的 404 由每日检查自愈——保持用户既定"直接 publish"决策不变
7. `[codex]`[中·遗漏] tauri-action 显式注入 `GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}`；`contents: write` 只放 tag 发布 job，main 构建 job 保持只读
8. `[pi]`[低·遗漏] 前端调 updater API 需在 capabilities 加 `updater:default`（安装后重启应用需 process 权限）；若全部走 Rust 命令则在实现中写明

（round 0 的 9 条已全部并入本 rev1 正文，不再列。）

### ② 实验已做的

无（两轮均无可实验项——全部属可当场查证的事实/遗漏/含糊）。

### ③ 存疑的

无（round 0 的 1 条存疑——"更新安装时序是否需额外设计"——已被 round 1 第 2 条升级为查实问题并列入①）。

修订版：本文即 rev1（round 1 后按夜间规则带清单进开发，未再修订）。产物目录：`.xcheck/20260918-174810/`、`.xcheck/20260918-181223/`。
