# 票 01 · 字典与迁移基础（v7）+ 设置侧易腐配置

## What to build

打通"数据层就位 + 用户能在设置里配置"的端到端切片：老库升级到 schema v7 无损获得撤食预置与易腐配置能力；设置页能给食物开关易腐、配撤食间隔（必填守护），操作页签里撤食以固定性质呈现、可停用。

- **v7 迁移（单事务）**：
  - `food` 加 `perishable INTEGER NOT NULL DEFAULT 0 CHECK (perishable IN (0,1))`、`retrieval_hours INTEGER`（可空；合法值 1–168 整数，0/负/非整数由应用层拒收）；按名回填：面包虫、干虾仁 → `perishable=1, retrieval_hours=24`（改名过的预置匹配不到，已知接受边界）。
  - `care_action` 表重建：`kind` CHECK 加入第三值 `'follow'`（跟随喂食）；插入预置「撤食」`(name='撤食', icon=NULL, kind='follow', suggested_interval_days=NULL, enabled=1, is_preset=1, sort=2)`，现有 `sort>=2` 的操作顺延 +1；旧行与数据原样保留。
  - **撞名分支**：库中已存在名为「撤食」的自建操作时，不重复插入，改为升格该行（`UPDATE care_action SET kind='follow', is_preset=1 WHERE name='撤食'`，id 与历史引用保留，原停用态保持），**并同步置 sort=2、现有 sort>=2 的其它操作顺延 +1**（与预置插入分支同款排序处理）。
  - **存量基线**：迁移时写设置键 `retrieval_baseline_at` = 本次迁移执行时刻（完整时间戳）；全新安装**不写**该键。
  - `reminder_ledger` 加部分唯一索引 `uq_ledger_retrieval (colony_id, base_date) WHERE kind='retrieval_due'`（v6 已去 kind CHECK，无需重建表）。
  - 全新安装种子：操作种子含撤食（sort=2，原 2–4 顺延 3–5）；食物种子带易腐默认（面包虫/干虾仁 perishable=1 + 24h，种子 0）。
- **设置读取**：settings 模块支持读可选键 `retrieval_baseline_at`（缺省 None，不写死）。
- **后端守护**：`save_food` 拒收 `perishable=1` 且 `retrieval_hours` 缺失或越界（非 1–168 整数）的组合；`perishable=0` 时允许间隔为空（关易腐清空由前端触发、后端接受 NULL）。
- **前端**：Food DTO 与前端类型加 `perishable` / `retrievalHours`；设置-食物 tab 加"易腐"开关列 + "撤食间隔(小时)"输入——开易腐时间隔必填（空/0/负/非整数/越界拒收并提示），关易腐时自动清空并置灰；设置-操作 tab 撤食行不显示性质切换与间隔编辑，固定标注「跟随喂食」，停用按钮照常（停用=功能关闭）。

## 验收标准

- [ ] v6 旧库升级：数据原样、面包虫/干虾仁带易腐 24h、撤食预置出现且不可删（沿用 is_preset 守护）、台账旧行保留
- [ ] 已自建「撤食」的旧库升级：该行升格为 follow 预置（id 不变、历史引用不断、无重复行、sort=2 归位且其它操作顺延）
- [ ] 升级库写入了 `retrieval_baseline_at`（值=迁移时刻）；全新库无该键
- [ ] 设置校验矩阵：开易腐+空/0/负/非整数/169 → 保存被拒；开易腐+24 → 通过；关易腐 → 间隔清空且输入置灰
- [ ] 撤食行固定"跟随喂食"、无性质切换无间隔；可停用
- [ ] 迁移测试覆盖：保数据、按名回填命中/改名不命中、撞名升格、种子断言、follow 通过 CHECK

## Blocked by

无，可立即开始

## 涉及路径

- src-tauri/src/db.rs
- src-tauri/src/dict.rs
- src-tauri/src/settings.rs
- src/types.ts
- src/lib/dict.ts
- src/lib/dict.test.ts
- src/components/SettingsDialog.vue
- src/components/SettingsDialog.test.ts

## 副作用声明

- 独占验证命令：`cd src-tauri && cargo test`（全量 Rust 测试）
- 前端：`pnpm vitest run src/lib/dict.test.ts src/components/SettingsDialog.test.ts`
- 会触发 Rust 重编译（首次较慢，属正常）

decision_refs: D1, D3, D11, D16; F1, F5, F7
review_blocks: 无
