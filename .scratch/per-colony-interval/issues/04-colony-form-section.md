# 票 04 · 窝编辑对话框「周期提醒」小节(桌面端)

## What to build
窝编辑对话框(ColonyFormDialog)新增「周期提醒」小节:该窝每个**启用**的操作一行——操作名 + 可选周期输入(整数天)+ 清除;撤食不出现。设/清走票 01 命令,保存后首页数据刷新。入口仅桌面端。

## 验收标准
- [x] 小节列出该窝全部启用操作(含自定义操作),撤食(follow)不出现;停用操作不出现
- [x] 输入校验 1..365 整数,非法给人话报错不落库;留空/清除 = 未设
- [x] 保存调用票 01 命令(Some upsert / None 删行),成功后窝列表与卡片反映新周期
- [x] 仅桌面端可达:窝编辑本属桌面管理路径,确认没有网页端泄漏入口(网页端功能面不含窝编辑)
- [x] vitest 全绿(529/529,新增 16 用例:小节渲染、撤食缺席、校验分支、设/清调用;沿仓库 mock IPC 先例)

## Blocked by
票 01, 票 03。

## 涉及路径
- src/components/ColonyFormDialog.vue
- src/components/ColonyFormDialog.test.ts
- src/lib/colonyForm.ts
- src/lib/colonyForm.test.ts
- src/lib/ipc.ts
- src/lib/ipc.test.ts

## 副作用声明
- 独占验证命令:`pnpm vitest run`(全量前端);不跑 cargo。

decision_refs: D2, D6, D7, D8
review_blocks: 无
