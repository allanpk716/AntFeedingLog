# 票 02 · 待撤食派生状态 + 首页撤食块 + 打卡闭环

## What to build

打通"喂了易腐食物 → 卡片撤食块点亮 → 打卡撤食 → 块清零"的端到端切片：

- **派生计算（后端）**：某窝**待撤食** ⇔ 最近一次含易腐食物的喂食记录（经 log_food 关联 food.perishable=1）的 `occurred_at` 晚于该窝最近一次撤食记录的 `occurred_at`（无撤食记录则任何易腐喂食都算）。到期时刻 = 该次喂食时刻 + **min(该次所选易腐食物的 retrieval_hours)**（多种易腐同喂取最短；未设间隔的易腐食物按不存在处理——脏数据自愈兜底）。再喂易腐按最新一次重算；撤食一次清空；删/改喂食或撤食记录后状态自动重算（派生、不落库）。
- **tiles 接口**：`tiles_for_colony` 为撤食输出三态 `none / pending / overdue`（pending = 待撤食未到期，overdue = 已超到期时刻）；不占用 `days_since`/`is_overdue` 通道，`is_overdue` 对 `follow` 性质恒 false。
- **前端卡片**：操作块按 sort 渲染（撤食自然落喂食旁，票 01 已置 sort=2）；撤食块三档——无待撤食 → 置灰 `disabled` 不可点；待撤食未到期 → 正常色可点；逾期 → 红（复用现有超期红样式）。**无倒计时文字**。
- **打卡闭环**：点击撤食块打开通用打卡面板（QuickLogDialog，日期默认今天、可补录过去、可备注），提交走现有 `log_care`，撤食记录照常进记录流水。
- 编辑/删除喂食或撤食记录后，卡片状态随下次数据刷新自动重算（无特殊分支）。

## 验收标准

- [ ] 喂面包虫后卡片撤食块从置灰变为正常可点；打卡撤食后回到置灰
- [ ] 逾期后块变红；无倒计时/灰字
- [ ] 撤食块点击无待撤时无响应（disabled）
- [ ] 派生测试：min 取最短（种子+面包虫24h+湿食6h → 6h 到期）；最新喂食重算；撤一次清空；撤食晚于喂食则状态清空
- [ ] 打卡面板：取消零落库；补录过去日期成功；未来日期被拒（沿用既有校验）
- [ ] tiles 三态结构与前端渲染对应（组件/lib 测试）

## Blocked by

01

## 涉及路径

- src-tauri/src/care.rs
- src/lib/care.ts
- src/lib/care.test.ts
- src/lib/home.ts
- src/lib/home.test.ts
- src/components/ColonyCard.vue
- src/components/QuickLogDialog.vue
- src/components/QuickLogDialog.test.ts

## 副作用声明

- 独占验证命令：`cd src-tauri && cargo test`（全量 Rust 测试）
- 前端：`pnpm vitest run src/lib/care.test.ts src/lib/home.test.ts src/components/QuickLogDialog.test.ts`

decision_refs: D2, D5, D10, D12, D13; F2（"同喂取最短"为设计推定口径，按 spec 实施）
review_blocks: 无

注：本票会在 care.rs 落地待撤食派生助手（供票 05 引擎消费）；若发现必须改动上列之外文件（如 reminder.rs），回报 NEEDS_CONTEXT，不越界。
