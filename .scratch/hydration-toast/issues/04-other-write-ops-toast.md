# 票 04 · 其余面板写操作按判定原则接轻提示

## What to build
按操作反馈规范的判定原则("操作完成后界面没有天然反馈的必须给轻提示;有天然反馈的不弹")审计并接线设置弹窗之外的写操作调用点:网页端设置面板(保存配置、重新生成凭证等)、地点管理面板(行级增删改即时落库)、更新面板(检查更新/下载安装)。对无天然反馈的写操作补成功/失败轻提示(失败带原因);已有明确进度/结果回显(天然反馈)的不弹并在回报中说明判定。字段级校验错误一律维持内联。只调用票 03 的全局 toast 模块,不自造样式。

## 验收标准
- [ ] 逐调用点列出判定表(操作/有无天然反馈/接不接/理由)写入回报
- [ ] 无天然反馈的写操作:成功/失败均有轻提示,失败带原因
- [ ] 有天然反馈的操作未被接线
- [ ] 校验错误维持内联,未混入 toast
- [ ] vitest(涉及文件)全绿 + vue-tsc 0 错

## Blocked by
票 03(toast 模块先就位)

## 涉及路径
src/components/WebUiPanel.vue、src/components/WebUiAccessUrl.vue(如需)、src/components/LocationManagerPanel.vue、src/components/UpdatePanel.vue,及各自同目录 *.test.ts(无测试的补建:LocationManagerPanel.test.ts 等)

## 副作用声明
允许 `pnpm vitest run <涉及测试文件>`;允许 `npx vue-tsc --noEmit`(只读全项目);不安装依赖、不联网

## decision_refs
D7(判定原则与形态)

## review_blocks
无
