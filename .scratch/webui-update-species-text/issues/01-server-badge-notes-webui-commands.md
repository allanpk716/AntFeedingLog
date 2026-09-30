# 票 01 · 服务端:红点存 notes + 网页端两个升级命令

## What to build

让网页端(浏览器)能查到升级红点详情、能触发主程序升级——全部在服务端(Rust)侧打通:

1. **红点落库扩展**:现有"远端可用版本号"落库处(species 无关,在 updater.rs 的 record_badge_from_result / set_available_version 一带),把检查拿到的 release 说明(notes,`Option<String>`)与版本号**一并落库**;检查发现无新版/已最新时清版本号的同一处**一并清 notes**(notes 生命周期跟随 available)。桌面端既有红点命令 get_update_badge 的返回形状(BadgeState{available:bool})与 update-badge-changed 事件负载**一律不变**。
2. **新只读命令**(网页端):无参数,返回 `{ available: bool, version: string|null, notes: string|null }`——available 沿用现有判活语义(version 数值上比当前新),version/notes 来自落库(无记录为 null)。登记进 WEBUI_COMMANDS 白名单注册表 + webui_args 校验链(无参命令按既有无参先例)。命名建议 `get_update_badge_detail`(以仓库既有命名为准,不强制)。
3. **升级触发命令**(网页端):无参数,**复用**现有升级确认流编排(复查 fresh_update → 下载 → 安装;升级前快照、禁写窗口、CONFIRM_IN_FLIGHT 防重入全部照旧,不复制粘贴新逻辑,走既有 confirm 流入口的网页端可达包装)。返回既有安装结果契约(install_started{version} / install_failed{version,message});Err(String)=检查失败类,含现有固定文案"远端已没有比当前更新的版本"与防重入串"已有安装流程正在进行"。命名建议 `confirm_and_install_update`。
4. **复查无新版时对齐红点**(spec D/出口3 的服务端半边):确认流复查返回"远端已没有比当前更新的版本"Err 的路径上,把落库红点按复查结果清掉(既有 record_badge 语义可复用则复用)。
5. 桌面端命令与行为零变更。

## 验收标准

- [ ] cargo test 全绿(既有 1 例端口占用环境例不计)
- [ ] 红点 notes 落库/清除有单测:发现新版→version+notes 都在;查无新版→两键皆清(照 updater.rs 既有注入式单测先例)
- [ ] 新只读命令:白名单路由放行 + 返回形状与桌面 invoke 同形(照 webui_server.rs 既有白名单测试先例,HTTP POST /api/cmd 实测)
- [ ] 升级触发命令:白名单放行 + 防重入语义沿用(CONFIRM_IN_FLIGHT 生效);测试用注入/Fake 手段,不打真网络、不真拉安装器
- [ ] 守护测试:其余更新类命令(如桌面专属 check_update_now/confirm_and_install 原名,若未登记)对 /api/cmd 仍 404
- [ ] get_update_badge 返回形状与 update-badge-changed 事件负载与改动前逐字节一致(既有测试不改动仍通过)

## Blocked by

无,可立即开始

## 涉及路径

- src-tauri/src/updater.rs(含文件内嵌 tests 模块)
- src-tauri/src/webui_server.rs
- src-tauri/src/webui_args.rs
- src-tauri/src/lib.rs

## 副作用声明

- 局部验证:`cargo test`(在 src-tauri 下);编译验证 `cargo check`
- 不跑 npm/vitest;不动 src/ 前端目录

## decision_refs

D1(边界反转:升级命令进白名单)、D4(只读查询)、D7(失败口径服务端不变)、F1(返回契约 {available,version,notes})、F6(notes 随红点清)

## review_blocks

无
