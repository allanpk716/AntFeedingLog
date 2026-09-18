# 实施 Spec：打 tag 自动构建发布 + 应用内自动更新（2026-09-18）

> 输入：`20260918-release-update-consensus.rev1.md`（含文末 xcheck 评审附录，两轮 9+9 条查实清单）。
> 本 spec 是附录①清单的开发落地契约；术语遵守 CONTEXT.md 与 docs/adr/0001。

## Problem Statement

发布 AntFeedingLog 新版本需要手工在本地打包、手工上传 GitHub Release；已安装的程序也没有任何升级途径，用户必须自己发现新版本并手动重装。需要一个"打 tag → 自动构建发布 → 程序自己发现并引导升级"的闭环，且更新检测不得调用 api.github.com（限流约束）。

## Solution（用户视角）

- 我给仓库打一个 `v*` tag 并 push，几分钟后 GitHub 上出现一个自动发布的 Release，里面带着安装包和更新清单；main 分支每次 push 也会自动跑测试和构建，提前暴露编译问题。
- 我的程序每天自己悄悄查一次有没有新版本；有新版时它提示我，我确认后它先备份数据、再下载安装包、重启完成升级。我不确认它绝不装；升级失败（比如应用退出了却没装上）它下次启动时告诉我结果。

## User Stories

1. 作为唯一用户，我希望 push `v*` tag 后 GitHub Actions 自动构建并把安装包挂到自动发布的 Release，以便不用手动打包上传。
2. 作为唯一用户，我希望 main 分支每次 push 自动跑测试与构建，以便打 tag 前就知道 CI 环境编不编得过。
3. 作为唯一用户，我希望托盘常驻的程序每天自动检查一次新版本（窗口关着也查），以便及时知道有更新。
4. 作为唯一用户，我希望设置页有"立即检查更新"按钮并显示结果，以便主动升级。
5. 作为唯一用户，我希望确认升级后程序自动先备份数据库再安装，以便升级出问题时能救回数据。
6. 作为唯一用户，我希望升级中途失败后重新打开程序时有明确提示与引导，以便知道要重试。
7. 作为唯一用户，我希望任何检查失败（无网、404）都不打扰我，以便托盘程序保持安静。

## Implementation Decisions

### 发布管线（CI）

- **两个 job 分离**：main CI job（push main 触发：前端 Vitest + cargo test + tauri build，**只读权限**）与 tag 发布 job（push `v[0-9]+.*` tag 触发：构建 + 发布，`permissions: contents: write` 且显式注入 `GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}`）。
- **tag 发布 job 用官方 `tauri-apps/tauri-action`**，显式配置：`includeUpdaterJson: true`（上传 latest.json 清单）与 `updaterJsonPreferNsis: true`（NSIS/MSI 并产时 Windows 清单条目指向 NSIS 安装器）；签名经环境变量 `TAURI_SIGNING_PRIVATE_KEY` / `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`（来自 GitHub Secrets）。
- **Release 操作只用 upsert 语义**（编辑/覆盖上传），**禁止删除已发布 release**；notes 由 tauri-action 建完后 `gh release edit <tag> --generate-notes` 补齐（tauri-action 自身无 auto-notes 选项）。"release 先可见、资产后上"窗口期的 404 由每日检查自愈，不改"直接 publish"决策。
- **版本校验**（发布 job 第一步，失败即中止）：tag == tauri.conf.json 的 version，且 package.json / tauri.conf.json / Cargo.toml 三处版本号互相一致。
- 仅 Windows x64；v0.1.0 保持孤儿 tag 不补 release；首发 v0.2.0；不使用 prerelease。

### 应用内更新（Rust 侧属主）

- **插件与配置**：tauri-plugin-updater；`tauri.conf.json` 加 `createUpdaterArtifacts: true`、updater 插件公钥与 `endpoints: ["https://github.com/allanpk716/AntFeedingLog/releases/latest/download/latest.json"]`（数组留镜像扩展位）。**更新流程全部在 Rust 侧**经 Tauri command 暴露给前端（前端不直接调 updater JS API，capabilities 无需 `updater:default`）。
- **检查**：托盘常驻进程内每日一次定时静默检查 + `check_update_now` command（设置页用）。**错误路径**：每日检查的任何失败（404/网络/超时/解析）一律按"无更新"记日志静默；仅手动检查失败时经 command 返回一次性错误提示。
- **升级确认流**（用户确认后）：`mark_pending_install(新版本号)`（持久化标记）→ 下载 → 安装。安装走插件 `install()`，NSIS 静默（/S）。
- **升级前快照**：挂插件 `on_before_exit` 钩子（退出前、数据流已停）——复用票 09 既有"短暂拿锁 → 拷贝单 .db 文件"机制（journal_mode=DELETE 下拷贝即完整，已有测试）；**文件名防覆盖**：`backups/pre-update-v<当前版本>-<yyyymmdd-hhmmss>.db`；快照完成后**进入禁写状态直到安装流程结束**；快照失败不阻塞升级，但状态要可查。
- **安装失败兜底（已知残余风险）**：Windows/NSIS 下应用 spawn 安装器后即退出，安装器中途失败无法回传 UI——启动时对比"上次标记的待安装版本"与当前运行版本：一致 → 清标记并可提示"升级完成"；不一致（仍旧版）→ 提示"上次升级未完成"+ 重试/手动下载引导。

### 版本与密钥

- 版本号三处手工同步，CI 兜底校验；发版流程 = 改三处版本 → commit → tag → push。
- minisign 密钥对由用户生成（设密码）、私钥与密码入 GitHub Secrets、公钥入 tauri.conf.json；私钥另存密码管理器。**私钥绝不入 git**。

## Testing Decisions

- 沿仓库既有惯例：Rust 逻辑 cargo test（tempfile 造库）、前端 Vitest。
- 单测对象（只测外部行为）：每日检查的节奏与错误→"无更新"映射（时间与网络层可注入/mocked）；快照命名防覆盖与禁写窗口（同版本重试不覆盖旧快照）；pending-install 标记的写入/清除与启动判定；版本一致性校验脚本。
- 不测：tauri-action/GitHub 侧行为（公开文档行为）、真实网络下载、真实安装器执行——这些属手动验收（首次真实发版由用户白天执行）。

## Out of Scope

macOS/Linux 构建；便携版 zip；静默自动更新（绝不无感安装）；下载镜像前缀（仅留数组位）；Authenticode 代码签名（接受 SmartScreen"未知发布者"警告，属预期）。

## Further Notes

- 前提：仓库保持公开（私有化会破坏零 API 匿名下载）。
- 已安装的 0.1.0 无更新能力，首次升级需手动装 0.2.0。
- 快照不引入 VACUUM INTO/backup API：journal_mode=DELETE 是本仓既定约定（db.rs 强制断言），拿锁+拷贝即完整快照。
- seam 裁定（夜链自动过）：沿既有最高 seam——Rust 为数据/系统能力唯一属主，前端只经 Tauri command；更新全流程在 Rust 侧。
