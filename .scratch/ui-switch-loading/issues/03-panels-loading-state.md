# 票 03 · 网页端/更新面板：数据到齐前渲染占位、不渲染表单

## What to build
设置弹窗内两个自带数据读取的面板补加载态（消灭"先显示默认值再跳成真实值"）：

1. **WebUiPanel.vue**：配置（getWebUiConfig + listNetworkSegments）到齐前，面板主体渲染 LoadingHint 占位、**不渲染表单**（现在会先显示默认端口 17321、开关全关再跳成真实值）；读取完成后按现状渲染表单。读取失败时按现状显示错误，不卡在占位。子组件（网段选择器/凭证区/地址区）在占位期间不挂载或不受影响，地址区自身的刷新机制（urlKey watch）照旧。
2. **UpdatePanel.vue**：版本号与更新状态（getAppVersion + getUpdateState）到齐前，版本行与状态横幅区域显示 LoadingHint 占位、不渲染空版本号；读取完成后按现状渲染。挂载时的 update-download-progress 订阅逻辑不动（含已卸载竞态退订）。读取失败维持现状静默（console.error），但不得卡在占位——失败即退出占位、按无数据渲染。

两个面板的内部加载态与外层 v-show 急挂载（票 02）正交：本票只改面板内部，使其在任何挂载时机下首读期间都诚实显示占位。

## 验收标准
- [ ] WebUiPanel 挂载、配置未返回前：显示「加载中…」，表单（开关/网段/端口/凭证）不渲染
- [ ] 配置返回后表单渲染真实值；读取失败显示错误、不卡占位
- [ ] UpdatePanel 挂载、版本/状态未返回前：显示「加载中…」，不渲染空版本号
- [ ] 版本/状态返回（或失败）后按现状渲染，进度订阅与退订行为不变
- [ ] `pnpm vitest run src/components/WebUiPanel.test.ts src/components/UpdatePanel.test.ts` 全绿（含占位行为用例更新）
- [ ] `pnpm exec vue-tsc --noEmit` 0 错

## Blocked by
票 01（LoadingHint 组件）。与票 02 无路径交集，可并行。

## 涉及路径
- src/components/WebUiPanel.vue
- src/components/WebUiPanel.test.ts
- src/components/UpdatePanel.vue
- src/components/UpdatePanel.test.ts

## 副作用声明
- 独占验证：`pnpm vitest run src/components/WebUiPanel.test.ts src/components/UpdatePanel.test.ts`
- 只读共用：`pnpm exec vue-tsc --noEmit`

## decision_refs: D3、D4、D5
## review_blocks: 无
