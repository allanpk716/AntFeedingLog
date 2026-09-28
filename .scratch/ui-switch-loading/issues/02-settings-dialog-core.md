# 票 02 · 设置弹窗本体：默认页签占位 + 三面板急挂载保活 + 打开链并行化 + 孤儿区错态

## What to build
设置弹窗（SettingsDialog.vue）的首开与页签切换体验端到端改造：

1. **默认页签加载占位**：初始字典数据未到时，「操作」「食物」页签显示 LoadingHint 占位、不渲染空列表；首次读取完成后退出占位。**退出判定必须用独立的"首次读取完成"布尔标志，不得用列表长度**（字典合法为空时不能永久卡在"加载中"）。保存后重拉等"数据已在内存"的场景不显示占位。
2. **三面板急挂载保活**：地点/网页端/更新三个页签分支从 `v-if` 链改为独立 `v-show` 块——弹窗一打开，三个面板就在后台挂载并与主加载同批读取；页签切换不再销毁面板（未保存的表单修改、地点面板行内改名在页签往返间保留）；弹窗关闭时整体销毁（现状语义不变，由外层 v-if 保证）。注意保持模板结构与其余 v-if 分支的正确共存（混合 v-if/v-show 时三面板块要脱离 v-else-if 链）。
3. **打开加载链并行化**：onMounted 中 `load()` 与 refreshPushoverStatus / refreshAvatarShape / loadLogSection / loadBackupSection / loadOrphanSection 六段同批并行（如 Promise.allSettled）；各段失败语义保持现状——主链失败走轻提示（showError("设置加载失败")），辅助段各自静默。
4. **孤儿照片区错态修正**：orphanStats 为 null 且尚未完成首次读取时显示 LoadingHint；读取完成（含结果为"无孤儿"）按现状显示统计文本；**读取失败才**显示「读取失败」（现状是首读期间就误显"读取失败"）。

## 验收标准
- [ ] 弹窗打开、字典未返回前，「操作」页签显示「加载中…」而非空白
- [ ] 字典返回（含返回空数组）后占位消失、列表/空态正常
- [ ] 打开弹窗即挂载地点/网页端/更新三面板（挂载时刻不晚于主链完成）
- [ ] 「网页端」页签改表单 → 切「操作」→ 切回：修改保留
- [ ] 页签往返不重发面板的初始读取（面板未销毁）
- [ ] 六段加载并行：任一辅助段失败不影响其它段与主链（失败语义与现状一致）
- [ ] 孤儿区首读期间显示「加载中…」，失败才显示「读取失败」
- [ ] `pnpm vitest run src/components/SettingsDialog.test.ts` 全绿（含上述行为的用例更新）
- [ ] `pnpm exec vue-tsc --noEmit` 0 错

## Blocked by
票 01（LoadingHint 组件）。

## 涉及路径
- src/components/SettingsDialog.vue
- src/components/SettingsDialog.test.ts

## 副作用声明
- 独占验证：`pnpm vitest run src/components/SettingsDialog.test.ts`
- 只读共用：`pnpm exec vue-tsc --noEmit`

## decision_refs: D2、D3、D4、D5、D6、D8
## review_blocks: 无（F7 的空字典口径已并入本票验收第 2 条）
