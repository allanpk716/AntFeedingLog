# 02 · 前端选择界面分组直选

Status: done

## 内容

- types.ts：`FoodCategory` 类型；FoodItem.category；FoodInput.category（可选）。
- 新 lib：src/lib/foodCategories.ts——大类顺序/中文映射/`groupFoodsByCategory`（未知大类兜底尾组，空组跳过）。
- FeedDialog.vue：食物区改三组一屏直选（组标签 + 组内 chips，样式沿用药丸多选）。
- LogListPage.vue 编辑弹窗：同款分组；停用项仅原值可保留的逻辑不动。

## 验收

- [ ] foodCategories 单测：顺序、映射、空组跳过、未知大类兜底。
- [ ] FeedDialog 测试：三组标签按序出现、chip 落在正确组。
- [ ] LogListPage 既有编辑用例照旧通过（chips 按文本查找不破）。
