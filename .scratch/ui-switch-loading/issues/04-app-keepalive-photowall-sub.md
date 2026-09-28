# 票 04 · 主界面三页保活 + 照片墙数据版本订阅

## What to build
主界面导航页（统计/照片/记录）的"再进即时"与保活后的数据保鲜：

1. **App.vue 三页包 `<KeepAlive>`**：统计（StatsPage）、照片（PhotoWallPage）、记录（LogListPage）三个 `v-if` 页面组件包进 `<KeepAlive>`——会话内首次进入照常挂载读取（各页自己的加载态由票 05/06 与既有照片墙占位负责），**再进即时**（组件实例与本地状态保留，不重挂载、不重发 IPC）；首页（home 主区）不动；页签卸载释放资源的既有行为改为由 KeepAlive 缓存托管（ECharts 实例随实例保留，属预期）。注意 KeepAlive 多分支用法：`<KeepAlive>` 内只能有一个活动子节点，用包裹单一动态节点或分离包裹的方式实现，保持现有 template 结构与样式类不变。
2. **PhotoWallPage.vue 补 `watchDataVersion` 订阅**：对齐统计/记录页的既有做法（见 LogListPage.vue 的 unwatchVersion 模式）——任何一端写入导致数据版本变化时重拉照片墙数据；组件真正卸载时退订（保活切换不触发卸载，订阅持续有效，属预期——后台也自动刷新）。

## 验收标准
- [ ] 进入「统计」→ 切走 → 再进：组件不重挂载（不重发 listColonies/getStats 等 IPC），页面状态保留
- [ ] 「照片」「记录」两页同上（记录页筛选状态在往返间保留——预期行为）
- [ ] 首页行为不变（数据在根组件，切换不受影响）
- [ ] PhotoWallPage 收到数据版本广播时重拉（模拟版本变更 → photoWall() 重发）
- [ ] PhotoWallPage 真卸载（App 卸载）时退订，无泄漏
- [ ] `pnpm vitest run src/App.test.ts src/components/PhotoWallPage.test.ts` 全绿（含上述行为用例更新）
- [ ] `pnpm exec vue-tsc --noEmit` 0 错

## Blocked by
票 01（本票自身不接 LoadingHint，但保持波次一致，避免与其它票抢占 vue-tsc 时序）。实际代码依赖仅 KeepAlive 本身，若 01 未完成且无路径冲突可提前。

## 涉及路径
- src/App.vue
- src/App.test.ts
- src/components/PhotoWallPage.vue
- src/components/PhotoWallPage.test.ts

## 副作用声明
- 独占验证：`pnpm vitest run src/App.test.ts src/components/PhotoWallPage.test.ts`
- 只读共用：`pnpm exec vue-tsc --noEmit`

## decision_refs: D4、D7
## review_blocks: 无（F5 筛选保留=预期行为，验收已覆盖）
