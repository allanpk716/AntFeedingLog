# 票 05 · 撤食提醒引擎 + 通知开关 + 托盘概要

## What to build

打通"到期催促送达用户"的端到端切片：新提醒种类 retrieval_due 进引擎，通知三分类、每日去重、冬眠例外、独立开关、托盘概要。

- **ReminderKind**：加 `RetrievalDue`（`as_str = "retrieval_due"`）；文案："「窝名」该撤食了：M月D日 HH:mm 喂的面包虫已超过 N 小时"。
- **时刻注入**：`compute_due_reminders` 增加完整时刻 now 参数（现有 `today` 保留给日粒度类）；撤食到期用时刻比较；台账基准日 = 当前自然日，`uq_ledger_retrieval (窝, 当日)` 去重 → **每天至多一条**，随今天推进实现每日催促。
- **参与窝**：活跃**或**冬眠（撤食提醒不吃冬眠静音）；已结束窝不提醒。
- **通知三分类**（消费票 02 在 care.rs 落地的待撤食派生助手）：
  1. 新鲜喂食（录入时未逾期：`occurred_at + interval > created_at`，分钟级容差）→ 到期后每日一条。
  2. 补录喂食（录入时已逾期）→ 录入当轮不追溯补发；**自录入次日起**每日催促（通知日须晚于 `created_at` 所在日）。
  3. 存量喂食（`created_at ≤ retrieval_baseline_at`，票 01 的设置键；键缺省视为无非存量）→ **永不推送**，仅状态红与托盘概要。
- **设置过滤**：`AppSettings` 加 `notify_retrieval_enabled`（默认 true，K_ 常量/读取/写入三处同款）；`run_check` 要求总开关与新键都开才发撤食通知。
- **托盘**：tooltip 概要纳入待撤食（如"2 窝活跃 · 大头一号待撤食"，沿用现有概要拼装处）。
- **前端**：notifySettings 前端模型 + 设置-通知 tab 加「撤食提醒」开关（默认开；总开关关则全静默的现有语义不变）。
- **不实施**：补录"永久静默豁免"（宽读）——开放问题待用户拍板，字面口径即本票口径。

## 验收标准

- [ ] 喂面包虫(24h)后 25 小时检查：桌面 + Pushover 各一条；再过 24 小时未撤 → 又一条；打卡撤食 → 不再发
- [ ] 冬眠窝待撤食照发；已结束窝不发
- [ ] 补录 3 天前面包虫：录入当天零通知；次日起每日一条直到撤食
- [ ] 存量喂食（created_at ≤ baseline）：零推送、状态红
- [ ] 总开关关 → 全静默；仅撤食开关关 → 其它照发、撤食不发
- [ ] 台账同窝同日不重复（每日一条）
- [ ] 托盘概要含待撤食
- [ ] 引擎测试组全绿（分类/去重/过滤/开关/文案）

## Blocked by

02

## 涉及路径

- src-tauri/src/reminder.rs
- src-tauri/src/settings.rs
- src-tauri/src/lib.rs
- src/lib/notifySettings.ts
- src/lib/notifySettings.test.ts
- src/components/SettingsDialog.vue
- src/components/SettingsDialog.test.ts

## 副作用声明

- 独占验证命令：`cd src-tauri && cargo test`（全量 Rust 测试）
- 前端：`pnpm vitest run src/lib/notifySettings.test.ts src/components/SettingsDialog.test.ts`

decision_refs: D4, D6, D7, D8, D9, D15; F3（字面口径，宽读不实施）, F4（存量类）, F9（编辑不刷新 created_at）
review_blocks: 无

注：待撤食派生助手由票 02 落地在 care.rs；本票只消费。若发现必须改 care.rs 签名或上列之外文件，回报 NEEDS_CONTEXT，不越界。
