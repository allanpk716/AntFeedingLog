# 发版操作手册（首发 v0.2.0）

> 面向：仓库主人自己。目标：打一个 `v0.2.0` tag 推上去，GitHub Actions 自动构建并发布带安装包的 Release，程序内自动更新从此可用。
>
> 首次发版比以后多三件事：生成签名密钥、填公钥、配 GitHub Secrets。这三件只做一次，以后发版就是"改版本号 → tag → push"。
>
> 全程约 30 分钟（大部分时间在等 GitHub Actions 编译，约 10-20 分钟）。

---

## 第 1 步：生成 minisign 密钥对

更新包要靠 minisign 密钥对签名：私钥放 GitHub Secrets 用来签名，公钥写进程序配置用来验签。没有这对密钥，自动更新的产物签不了名。

在**仓库根目录**（`package.json` 所在目录）跑：

```bash
pnpm tauri signer generate -w ~/.tauri/antfeedinglog.key
```

说明：

- 建议在 **Git Bash** 里跑，`~` 会自动展开成你的用户主目录（`C:\Users\allan716`）。在 PowerShell / CMD 下 `~` 不展开，请写完整路径：`-w C:\Users\allan716\.tauri\antfeedinglog.key`。
- 命令会让你**设一个密码**（passphrase），签名私钥时要用。**这个密码别忘**，忘了私钥就废了，只能重新生成一对（并更新公钥和 Secrets）。
- 产出两个文件：
  - `~/.tauri/antfeedinglog.key` — **私钥**，签更新包用，绝不给任何人、绝不进 git；
  - `~/.tauri/antfeedinglog.key.pub` — **公钥**，下一步填进程序配置，公开无妨。

## 第 2 步：把公钥填进 tauri.conf.json

打开 `src-tauri/tauri.conf.json`，找到 `plugins` → `updater` → `pubkey`（当前是一个占位串 `dW50cnVzdGVkIGNvbW1lbnQ6`，票 02 预留的位置）：

```json
"plugins": {
  "updater": {
    "pubkey": "<把这里整串替换成 .pub 文件的内容>",
    "endpoints": [
      "https://github.com/allanpk716/AntFeedingLog/releases/latest/download/latest.json"
    ]
  }
}
```

用文本编辑器打开 `~/.tauri/antfeedinglog.key.pub`——里面是**一行** base64（以 `dW50cnVzdGVkIGNvbW1lbnQ6` 开头），把这一整行复制，替换掉占位串。**原样粘贴即可，不要再做任何加工**（tauri-cli 生成时已把"注释 + 密钥"整体编码过一次，程序验签时自己解回来；自己再拆行或转义反而会坏）：

```json
"pubkey": "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IFhYWFgKUlczLi4uLg=="
```

想自查复制对了没有：在 Git Bash 跑 `echo "<pubkey那一行>" | base64 -d`，第一行应是 `untrusted comment: minisign public key: <一段十六进制>`（下面还有一行 base64，正常）。改完这个文件会随第 5 步一起提交。

## 第 3 步：GitHub 仓库配置两个 Secrets

CI 在云端签名，需要拿到私钥和密码。到 GitHub 仓库页面 → **Settings** → 左侧 **Secrets and variables** → **Actions** → **New repository secret**，加两条：

| Name | Secret 值 |
| --- | --- |
| `TAURI_SIGNING_PRIVATE_KEY` | 私钥**文件内容**：用文本编辑器打开 `~/.tauri/antfeedinglog.key`，里面是**一行** base64，整行原样粘进去（别带引号、别带多余换行） |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | 第 1 步生成密钥时设的那个密码 |

两条都加完，CI 才能签出 `.sig` 文件。

## 第 4 步：私钥灾备 + 安全红线

- **灾备**：把 `~/.tauri/antfeedinglog.key`（私钥）和它的密码另存一份到你的密码管理器（或离线 U 盘等不吃系统重装的介质）。这台电脑坏了，密钥就没了，以后所有已装用户都验不了新包的签名。
- **红线：私钥绝不入 git。** 仓库 `.gitignore` 已加 `.tauri/` 和 `*.key`（放行 `.key.pub`）兜底；生成路径 `~/.tauri/` 本身也在仓库外。如果你另存到仓库目录里，确认 `git status` 里没出现它再提交。

## 第 5 步：打 tag 并 push，等 CI 发版

前面第 2 步的公钥改动、本仓库已改好的三处版本号（package.json / src-tauri/tauri.conf.json / src-tauri/Cargo.toml，均为 0.2.0）先提交进 main：

```bash
git push origin main          # 版本改动的 commit 上去
git tag v0.2.0                # 在该 commit 上打 tag
git push origin v0.2.0        # 单独推 tag（也可 git push --tags）
```

tag 一推上去，**Actions** 页会出现 "Release" 流水线：先校验 tag 与三处版本号一致（不一致直接失败，改完重新打 tag），再在 Windows 上构建并发布。等 10-20 分钟。

**验收**：仓库 **Releases** 页出现 `v0.2.0`，资产四件套齐全：

1. `AntFeedingLog_0.2.0_x64-setup.exe`（NSIS 安装包，应用内更新走它）
2. `AntFeedingLog_0.2.0_x64_en-US.msi`（MSI 安装包）
3. `latest.json`（更新清单，已装用户的应用就是读它发现新版本）
4. `*.sig`（exe 和 msi 各一个签名文件）

（文件名以实际产物为准，认准 setup.exe / .msi / latest.json / .sig 四类。）

## 第 6 步：本机手动安装 0.2.0

从 Release 页下载 `*-setup.exe` 双击安装。**Windows SmartScreen 会弹"Windows 已保护你的电脑 / 未知发布者"**——这是预期行为：本应用没做 Authenticode 代码签名（成本原因，见 `docs/superpowers/specs/20260918-release-update-spec.md` 的 Out of Scope），**不是投毒**，点"更多信息"→"仍要运行"即可。

装好后打开应用 → 设置 → **"更新"** 页签，能看到"当前版本：v0.2.0"，点"立即检查更新"应显示"已是最新版本"。

---

## 附 1：Release 失败了怎么办（重跑纪律）

Actions 的 Release 流水线对 Release **只做覆盖更新（upsert）**：同名资产覆盖上传、notes 重新生成，**绝不删除已发布的 Release**。所以失败就直接在 Actions 页点 **Re-run failed jobs**，安全；不要去手动删 Release 重来。

## 附 2：本机旧的 0.1.0 怎么办

v0.1.0 是孤儿 tag（没有 Release），且旧版里根本没有更新功能。所以**本机已装的 0.1.0 感知不到 0.2.0**，需要按第 6 步手动装一次 0.2.0。从这以后，新版本就能在应用内自动检查、确认、升级了。

## 附 3：以后发版（速查）

```bash
# 1. 改三处版本号：package.json / src-tauri/tauri.conf.json / src-tauri/Cargo.toml
node scripts/check-versions.mjs . v0.3.0   # 本地先自检（三处一致 + tag 匹配）
git push origin main && git tag v0.3.0 && git push origin v0.3.0
# 2. 等 Actions，验收四件套，Rust 依赖有变时确认 lock 文件已刷新
```
