# 网页端窝资料编辑 · 规格

日期：2026-09-28 · 状态：已评审收敛（两轮异构评审，阻断已修复过复审）· 来源：用户逐问裁决的设计共识（.xcheck/20260928-094521 → 095738，D1–D7）

## Problem Statement（用户视角）

饲养者已经能在手机（受信网段浏览器）上打卡、看记录、传巢况照片，但窝本身的信息——名字、物种、地点、开始饲养日期、状态、每窝周期、保湿方式——只能回桌面端改；手滑建错的窝也无法在手机上删掉。"拍照换头像"经查证已有完整通路（头像永远等于最新巢况照片，传新照片即换），无需开发。

## Solution（用户视角）

网页端获得与桌面一致的窝资料维护能力：编辑任一窝的全部表单字段、新建窝、删除从未打过卡的窝（有记录的窝删除被拒绝、历史保留）。冬眠中的窝可以照常编辑其他资料而状态不被改变；任何入口都不能绕过冬眠流程直改冬眠状态。桌面端行为零变化。

## User Stories

1. 作为饲养者，我想在手机上编辑任一窝的名字、物种、地点、开始饲养日期、状态、每窝周期与保湿方式，以便不在电脑前也能维护窝资料。
2. 作为饲养者，我想在手机上新建窝（含每窝周期与保湿方式的随建随设），以便外头收了新蚁后即时登记。
3. 作为饲养者，我想在手机上删除一个从未打过卡的窝，以便即时清理建错的空窝；打过卡的窝删除被拒并得到人话错误，历史保留。
4. 作为饲养者，我想在手机上编辑冬眠中的窝的其他资料，且它的冬眠状态不被这次编辑改变，以便冬眠期照常维护资料。
5. 作为饲养者，我不希望任何入口能绕过「开始冬眠/确认出眠」流程直改冬眠状态，以便冬眠段记录与窝状态永远一致。

## Implementation Decisions

### 前端（三处入口放开，表单零改动）

- `src/App.vue`：顶栏「＋ 新建窝」按钮去掉 `v-if="isTauri()"`（双端渲染）。
- `src/components/ColonyCard.vue`：「⋯」菜单「✏️ 编辑窝信息」项去掉 `v-if="isTauri()"`。
- `src/components/ColonyFormDialog.vue`：编辑模式「删除」按钮放开到双端；「置为已结束」按钮维持桌面专属（"已结束"经状态下拉可达，功能无损）。
- 操作反馈遵守项目全局规范：保存/删除成功后窗关闭、列表刷新 = 天然反馈，不加轻提示；失败维持表单内联红字（既有行为）。

### 服务端（白名单 + 校验镜像 + 状态守卫）

- `webui_server.rs dispatch_command` 新增三个写命令，走既有 `write_cmd` 模式（`with_tray = true`；成功触发 `after_write`：托盘刷新 + 自动备份 + 数据版本广播）：
  - `create_colony` → `colony::create_colony`
  - `update_colony` → 状态守卫通过后调 `colony::update_colony`
  - `delete_colony` → 照桌面 `lib.rs` 同序编排：先 `photo::collect_colony_photo_paths` 取路径 → 库事务删行提交 → 删照片文件（失败仅孤儿记账，不回滚库）。
- `webui_args.rs` 新增 `ColonyInputArgs` 校验镜像（`deny_unknown_fields`，键 snake_case 与桌面 IPC 同形）：name（非空/长度上限对齐纯核）、species、location_id、start_date（ISO）、status、hydration_method（null|manual|tower）、interval_changes（action_id 存在性预检、interval_days null 或 1..=365）。
- **状态守卫（D7）**——状态变更仅允许 active↔ended；hibernating 只能原样透传：
  - `create_colony`：镜像枚举校验只放行 active|ended（新建不可能是冬眠中）。
  - `update_colony`：dispatch 闭包内（同一把库锁，读-判-写无 TOCTOU）读库内当前 status 判定：库内=hibernating 时新值必须=hibernating（不变更豁免——冬眠窝可正常编辑其他字段，表单复用使载荷必带原值）；新值=active/ended 拒绝（出眠走 confirm_wake）；库内∈{active,ended} 时新值=hibernating 拒绝（入眠走 start_hibernation）；active↔ended 照常。
  - 守卫只放 web 路径；守卫是镜像校验之外的附加判定，不替代镜像校验（载荷其他字段仍走镜像+纯核校验链）。
  - 桌面纯核零改动（桌面 IPC 靠前端禁选兜住同样语义，行为不变）。
- 纯核（`colony.rs` / `photo.rs`）零改动；`archive_colony` 不入白名单。
- 前端 `ipc.ts` 的 `createColony` / `updateColony` / `deleteColony` 桥已存在，无需改动。
- location 存在性由纯核 `normalize_colony_input` 的 `validate_location_id` 兜底（create/update 共用第一道校验）。

### 边界

- 地点下拉只能选已有地点（新建地点留桌面设置）。
- 并发写：后写覆盖前写，与全部既有写命令同语义（单库锁串行）；状态守卫同锁读判写无竞态窗口。
- CONTEXT.md「网页端」「桌面端」词条已按"数据性质划线"原则更新，属操作者工作区改动，不由夜链提交。

## Testing Decisions

只测外部行为，不测实现细节；沿用仓库既有同类测试先例（Rust 内嵌 `#[cfg(test)]` HTTP 层/纯函数测试；Vue 组件与同名 `*.test.ts` 同目录，Vitest）。

- Rust（`webui_args.rs` / `webui_server.rs` 内嵌测试）：
  - 镜像合法/非法载荷单测：status=hibernating 的 create 拒绝、interval_days 越界（0/366/负）拒绝、未知字段拒绝、合法载荷通过。
  - dispatch 白名单路由：三命令经 HTTP 可达且返回形状与桌面 invoke 同形（仿既有先例）。
  - `delete_colony` 先库后文件同序断言（仿 `http_delete_checkin_removes_photo_files_same_order` 先例）。
  - 状态守卫四条专项：冬眠窝经 web update_colony 改名字/周期等可保存且库内 status 保持 hibernating；非冬眠窝直发 status=hibernating 被拒；冬眠窝直发 status=active/ended 被拒；active↔ended 直切放行。
- 前端（Vitest）：
  - 三处入口双端渲染断言：网页端（非 Tauri 环境）「＋ 新建窝」「✏️ 编辑窝信息」「删除」可见，「置为已结束」不可见。
  - 既有断言"网页端不渲染这些入口"的测试同步更新（`src/App.test.ts`、`src/components/ColonyCard.test.ts`、`src/components/ColonyFormDialog.test.ts`）。
- 全量门槛：vitest 全绿、`vue-tsc` 0 错、cargo 测试通过（既有端口占用类环境败例如实归因，不算失败）。

## Out of Scope

- 头像通路任何改动（拍照/相册/裁剪已全有；头像纯派生模型不变）。
- 字典、设置、备份、恢复、导出、更新进网页端（系统管理一律桌面）。
- 新建地点入口进网页端。
- 冬眠流程任何改动。
- 桌面端行为变更。

## Further Notes

- 评审留档两条非阻断实现参考（未纳入验收，实现者自行斟酌）：载荷省略 status 字段时按镜像必填拒绝（serde 无 default 即天然拒绝，报错文案可复核）；守卫通过后载荷其余字段仍须走完镜像+纯核校验链（守卫不豁免其他校验），单测可保留一条"冬眠窝载荷其他字段非法仍被拒"。
- 词汇表（CONTEXT.md）：「网页端」词条现口径 = 饲养日常数据（打卡、历史记录、冬眠操作、巢况登记与照片、窝资料的编辑与新建、删除仅对未打过卡的窝放行）可读写；系统管理（字典、备份、恢复、导出、更新、设置）一律桌面。本规格术语与其一致。
- 数据版本广播：三命令成功后经既有 `after_write` 钩子触发，网页端 SSE 自动收推，无新增机制。
