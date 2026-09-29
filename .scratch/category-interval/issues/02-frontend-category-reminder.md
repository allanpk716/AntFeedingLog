# 02 前端：大类周期（pill/悬停 + 设置页 + 类型收敛）

Status: resolved
Blocked by: 01

## Answer

- types/care/dict/ipc/SettingsDialog 全部改大类口径：pill「⚠ 该喂蛋白质、糖水了」、悬停大类在前逐食物参考、大类周期三输入随「保存」落库、食物行去间隔列
- dict.test 新增大类周期行模型用例；care.test 大类层 pill/悬停用例；App.test 食物 tab 大类周期保存链路 + 大类层顶红 pill 用例
- vitest 778/778 全绿、vue-tsc 0 错

## 范围

- `types.ts`：`FoodItem` 去 suggested_interval_days；`FoodTileInfo` 收窄 {food_id, name, days_since_last}；新增 `CategoryTileInfo {category, interval_days, days_since_last, overdue}`；`ColonyAction.categories`
- `care.ts`：`actionTile` 红药丸分支——操作层超期维持「⚠ 超期 N 天」，大类层顶红改「⚠ 该喂蛋白质、糖水了」（foodCategories.foodCategoryLabel）；`feedingTooltip` 逐食物去「超期」标
- `dict.ts`：`buildFoodRows`/`toFoodInputs` 去间隔；新增大类周期行模型（三行固定序）+ 载入/保存
- `ipc.ts`：`listFoodCategories` / `saveFoodCategoryInterval`；`saveFood` 入参去间隔
- `SettingsDialog.vue`：食物区顶部三个大类周期输入（可清空）+ hint 更新；食物行删间隔列
- 测试：care.test.ts（大类 pill / 悬停）、dict.test.ts、SettingsDialog.test.ts（大类周期保存/清空、食物行无间隔列）、App.test.ts / FeedDialog.test.ts / foodCategories.test.ts / QuickLogDialog.test.ts / LogListPage.test.ts fixtures 去 food 间隔字段

## 验收

vitest 全绿、vue-tsc 0 错。

## Comments
