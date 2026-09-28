# 票 02 · 前端入口放开：新建/编辑/删除双端可见

## What to build

把窝资料维护的三个前端入口从桌面专属放开到网页端（浏览器渲染）：顶栏「＋ 新建窝」、窝卡片「⋯」菜单「✏️ 编辑窝信息」、编辑弹窗「删除」按钮；「置为已结束」按钮维持桌面专属。表单组件本身零改动。用户视角验收：手机浏览器上能新建窝、打开编辑表单改全部字段、删除空窝；桌面端外观与行为不变。

规格：`docs/superpowers/specs/20260928-web-colony-edit-spec.md`（Implementation Decisions · 前端节）。

## 验收标准

- [ ] `src/App.vue`：「＋ 新建窝」按钮去掉 `v-if="isTauri()"`，双端渲染（同步更新该行"桌面专属"注释）
- [ ] `src/components/ColonyCard.vue`：「⋯」菜单「✏️ 编辑窝信息」项去掉 `v-if="isTauri()"`，双端渲染（同步更新注释）
- [ ] `src/components/ColonyFormDialog.vue`：编辑模式「删除」按钮放开双端；「置为已结束」（archive-btn）维持 `v-if="isTauri()"`
- [ ] `src/App.test.ts`：既有断言"网页端不渲染新建窝按钮"的用例改为双端可见断言（或补网页端用例）
- [ ] `src/components/ColonyCard.test.ts`：同上，编辑菜单项双端可见
- [ ] `src/components/ColonyFormDialog.test.ts`：删除按钮双端可见、置为已结束按钮网页端不可见
- [ ] 表单组件逻辑零改动（不改校验/序列化/提交链路）
- [ ] `npx vitest run <三个测试文件>` 全绿；本票文件 `vue-tsc` 无新错

## Blocked by

无，可立即开始（渲染断言走 mock，不依赖服务端票）。

## 涉及路径

- src/App.vue
- src/App.test.ts
- src/components/ColonyCard.vue
- src/components/ColonyCard.test.ts
- src/components/ColonyFormDialog.vue
- src/components/ColonyFormDialog.test.ts

## 副作用声明

- `npx vitest run src/App.test.ts src/components/ColonyCard.test.ts src/components/ColonyFormDialog.test.ts` 为本票局部验证；`npx vue-tsc --noEmit` 为共享验证（只看本票文件有无新错，不修他票遗留）。

decision_refs: D2, D3, D4
review_blocks: 无
