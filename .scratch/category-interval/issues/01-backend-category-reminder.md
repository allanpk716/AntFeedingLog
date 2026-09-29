# 01 后端：大类周期（schema v14 + 判定 + 提醒 + 字典 API）

Status: resolved

## Answer

- schema v13→v14 全量落地：`food_category_interval` 表（初值=大类内食物间隔最大值，宽松优先）、台账整表重建（food_id→category，存量 food_overdue 归并留最早）、food 间隔列 DROP、avatar_shape 键清理
- care.rs：`FoodTileStatus` 收窄为参考明细（无 overdue/间隔），新增 `CategoryTileStatus` + `ActionTile.categories`；大类聚合 days=各食物 Some 最小值；tile.overdue=操作层||任一大类
- reminder.rs：`CategoryOverdue`("category_overdue")、文案「该喂蛋白质了/已 N 天没喂蛋白质（建议 3 天一次）」、tray「该喂蛋白质了」
- dict.rs：FoodInput/Food 去间隔；`list_food_categories`/`save_food_category_interval` + 中文标签映射；erase_food 台账检查移除
- cargo test 658/659（唯一败=端口环境例 runtime_sync，应用占用 17321 所致，历史已知）
- 真实库副本冒烟：v13→v14 ✓ 大类周期 protein3/seed7/sugar3 ✓ 4 条 food_overdue 正确换 category 维度（无同日归并场景）✓ 旧列/键删净 ✓ 读侧 tiles 跑通 ✓
- 统计页确认无需改动（间隔图只含操作层）——ADR 已修正

## 范围

- `db.rs`：SCHEMA_VERSION 13→14；`migrate_v13_to_v14`——
  - 建 `food_category_interval(category PK CHECK三值, interval_days INTEGER CHECK(≥1或NULL))`；初值 `INSERT ... SELECT category, MAX(suggested_interval_days) FROM food GROUP BY category HAVING MAX(...) IS NOT NULL`
  - `reminder_ledger` 整表重建：去 `food_id` 列、加 `category TEXT CHECK`；存量 `food_overdue` 行改 kind=`category_overdue`、category 由 food_id 关联回填、food_id 置 NULL；同 (colony_id, action_id, base_date) 归并留 MIN(id)；其余 kind 原样；重建三唯一索引（overdue / category / wake）
  - `ALTER TABLE food DROP COLUMN suggested_interval_days`
  - `DELETE FROM settings WHERE key='avatar_shape'`（头像横幅票一并清形状偏好）
- `care.rs`：`FoodTileStatus` 收窄为 {food_id, name, days_since_last}（悬停参考，不驱动提醒）；新增 `CategoryTileStatus {category, interval_days, days_since_last, overdue}`；`ActionTile.categories` 新字段；`tiles_for_colony_at` 食物层查询改取 category、按大类聚合（days = 大类内各食物 Some 值取最小；全 None → None）、大类超期判定（kind=reminding 且设了周期且 is_overdue）；`tile.overdue` = 操作层 || 任一大类超期；`FOOD_SQL` 去 interval 列、`Food`/`row_to_food` 同步
- `reminder.rs`：`ReminderKind::CategoryOverdue`("category_overdue")；`Reminder` 去 food_id/food_name 加 `category`；`notification_text` 大类文案（label 中文映射在 dict.rs）；`compute_due_reminders` 食物层循环换大类层；`run_check` 台账 INSERT 换 category 列；`tray_summary`「该喂X了」按大类
- `dict.rs`：`FoodInput` 去 suggested_interval_days；`save_food` 去间隔校验与落库；新增 `FoodCategoryInterval {category, interval_days}` + `list_food_categories`（恒三行固定序）+ `save_food_category_interval`（key 守卫 + ≥1 校验 + upsert，None=清空）；`FOOD_CATEGORY_LABELS` 中文映射
- `lib.rs`：注册 `list_food_categories` / `save_food_category_interval`（仅桌面，不进 webui 白名单）
- 测试：db.rs 迁移测试（v13→v14 三件事各自断言）；care.rs 食物层测试改大类语义（聚合、从未喂不催、全停用不催、悬停参考仍在）；reminder.rs 文案/判定/台账/托盘测试改大类

## 验收

cargo test 全绿（端口环境例除外）。

## Comments
