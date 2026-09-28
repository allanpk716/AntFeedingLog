# 票 05 · 统计页：接通从未渲染的 loading 标志

## What to build
统计页（StatsPage.vue）首开加载占位：代码里已有 `loading` ref（refresh() 置位/复位）但模板从未渲染——把它接上：

- **首次读取期间（payload 为 null）**：图表区/数据区显示 LoadingHint 占位，不渲染空图表骨架。
- **已有数据时的刷新**（换窝/换时间范围/版本广播重拉）：数据已在内存，旧图表保持渲染，**不显示占位**（与"数据已在内存即时渲染"细则一致）。
- 加载失败维持现状（pageError 展示），失败也退出占位、不卡死。

## 验收标准
- [ ] 首次进入统计页、数据未返回前显示「加载中…」
- [ ] 数据返回后图表正常渲染；返回空数据也退出占位（用独立标志或 payload 非 null 判定，不得用"图表非空"判定）
- [ ] 已有数据时切换筛选/范围：旧图表保持显示，不闪「加载中…」
- [ ] 读取失败：显示错误，不卡在占位
- [ ] `pnpm vitest run src/components/StatsPage.test.ts` 全绿（含占位行为用例更新）
- [ ] `pnpm exec vue-tsc --noEmit` 0 错

## Blocked by
票 01（LoadingHint 组件）。

## 涉及路径
- src/components/StatsPage.vue
- src/components/StatsPage.test.ts

## 副作用声明
- 独占验证：`pnpm vitest run src/components/StatsPage.test.ts`
- 只读共用：`pnpm exec vue-tsc --noEmit`

## decision_refs: D4、D5、D8
## review_blocks: 无
