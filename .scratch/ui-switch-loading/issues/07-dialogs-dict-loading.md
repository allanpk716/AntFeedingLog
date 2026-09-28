# 票 07 · 记账/打卡弹窗：字典选项区加载占位

## What to build
两个记录入口弹窗在字典未到时的加载提示：

1. **FeedDialog.vue**：打开时 listFoods/loadActions（及月份标记数据）未返回前，操作/食物选择区显示 LoadingHint 占位（或占位行），不渲染空选项组；字典返回后按现状渲染。提交按钮可维持现状禁用/启用逻辑，不因占位额外放宽。
2. **QuickLogDialog.vue**：同口径——allActions/loadMonth 未返回前，操作选择区显示占位；返回后按现状渲染。

两个弹窗的既有校验/提交流程不动；占位只影响"选项未到时的首屏呈现"。

## 验收标准
- [ ] FeedDialog 打开、字典未返回前：选择区显示「加载中…」，不渲染空选项
- [ ] 字典返回后选项正常渲染；喂食多选等联动行为不回归
- [ ] QuickLogDialog 同上两条
- [ ] 两弹窗校验与提交行为不回归（既有测试全绿）
- [ ] `pnpm vitest run src/components/FeedDialog.test.ts src/components/QuickLogDialog.test.ts` 全绿（含占位行为用例更新）
- [ ] `pnpm exec vue-tsc --noEmit` 0 错

## Blocked by
票 01（LoadingHint 组件）。

## 涉及路径
- src/components/FeedDialog.vue
- src/components/FeedDialog.test.ts
- src/components/QuickLogDialog.vue
- src/components/QuickLogDialog.test.ts

## 副作用声明
- 独占验证：`pnpm vitest run src/components/FeedDialog.test.ts src/components/QuickLogDialog.test.ts`
- 只读共用：`pnpm exec vue-tsc --noEmit`

## decision_refs: D3、D4、D5
## review_blocks: 无
