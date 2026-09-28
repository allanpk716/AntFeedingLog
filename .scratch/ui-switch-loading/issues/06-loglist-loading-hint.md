# 票 06 · 记录页：加载占位补齐（首载 + 空态下再筛选）

## What to build
记录流水页（LogListPage.vue）的加载占位：现状"没有符合条件的记录"空态与加载期不分——初始加载与"上一次结果为空后再改筛选"两种场景下，重读期间页面无提示。

- **显示条件（对齐"读取中且当前无可显内容"口径）**：`loading 为真 且 当前无行可显` 时在列表区显示 LoadingHint；覆盖首次加载与空态重查两条路径。
- **有旧数据在屏时**（改筛选后旧结果仍显示）：不显示占位（数据已在内存，旧内容保持渲染直到新结果到达——现状行为，保留）。
- 空态文案维持现状语义：仅"非加载中且无行"时显示「没有符合条件的记录」。
- 分页"加载更多"按钮的 loading 禁用行为不变。

## 验收标准
- [ ] 首次进入记录页、数据未返回前显示「加载中…」（而非空表格区/空态文案）
- [ ] 上一次查询结果为空、再改筛选触发重读：期间显示「加载中…」，新结果到达后正常
- [ ] 有旧数据在屏时改筛选：旧数据保持显示、不闪「加载中…」
- [ ] 非加载且无行：显示「没有符合条件的记录」（现状不回归）
- [ ] `pnpm vitest run src/components/LogListPage.test.ts` 全绿（含上述行为用例更新，至少覆盖一次筛选变更场景）
- [ ] `pnpm exec vue-tsc --noEmit` 0 错

## Blocked by
票 01（LoadingHint 组件）。

## 涉及路径
- src/components/LogListPage.vue
- src/components/LogListPage.test.ts

## 副作用声明
- 独占验证：`pnpm vitest run src/components/LogListPage.test.ts`
- 只读共用：`pnpm exec vue-tsc --noEmit`

## decision_refs: D4、D8
## review_blocks: 无（F6 口径已并入本票显示条件）
