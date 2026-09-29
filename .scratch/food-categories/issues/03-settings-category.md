# 03 · 设置·食物页签的大类编辑

Status: done

## 内容

- dict.ts：FoodRow 加 category；buildFoodRows/toFoodInputs 带大类；validateFoodRows 加大类必选运行时守护。
- SettingsDialog.vue 食物页签：每行加大类下拉（行内可改）；新增行前先选大类（下拉不预填、必选，未选报内联错误）；新增行的占位提示语改掉「如：糖水」（糖水已是大类名，防混淆）。

## 验收

- [ ] dict.test.ts：行带 category、入参透传、校验文案。
- [ ] SettingsDialog 测试：不选大类点添加 → 内联错误；选大类添加 → 新行落正确大类并随保存透传。
