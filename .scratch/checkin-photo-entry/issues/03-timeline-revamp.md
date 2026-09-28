# 票 03 · 前端时间线改版：双按钮/表单嵌照片/左轴视觉/空态占位

## What to build

`NestCheckinDialog` 信息架构与视觉改版：顶部「📷 拍一张」主按钮 +「📝 完整登记」次按钮（原常驻表单改按需展开，展开后表单内嵌照片选择区、保存走新通道一并上传）；时间线列表改左轴视觉（轴+节点+今日橙色高亮+轴旁换巢/基线标记）；纯照片条目无正文行；空时间线换成大占位拍照按钮。

视觉基线：`mocks/mock-checkin-photo-entry.html`（场景 2/3）与 `mocks/mock-timeline-styles.html`（形态 2）——工作区未跟踪文件，只读参考不上传。

规格：`docs/superpowers/specs/20260928-checkin-photo-entry-spec.md`（Implementation Decisions · 前端节 NestCheckinDialog 部分 + User Stories 2-8）。

## 验收标准

- [ ] 顶部双按钮：「📷 拍一张」主按钮（accent 主色，拍照编排=调新通道建当天登记，成功后 saved 刷新时间线与外层；busy 反馈）；「📝 完整登记」次按钮切换原表单展开/收起（默认收起；编辑既有条目时自动展开）
- [ ] 表单内嵌照片选择区：File 暂存、缩略预览、可多选（≤9 张/15MB 预检沿用）、可移除；保存走 `photos.ts` 创建模式（带全部表单字段）一并上传；空提交前端拦截（字段全空且未选照片→内联提示不弹 toast）
- [ ] 时间线左轴视觉：左竖轴贯穿 + 每条登记圆点节点；日期挂轴（今日节点橙色高亮 accent）；换巢/基线标记贴轴日期旁；纯照片条目（无文字）不显示正文行，只显示照片区+操作按钮；既有条目按钮（拍照/传照片/从相册选/编辑/删除）保留
- [ ] 空时间线：大占位「📷 拍一张巢况照片」按钮（同拍照编排）+ 次级链接「或做一次完整登记」（展开表单）
- [ ] 双端适配：手机断点（≤480px）按钮与轴布局可用（对齐既有 vp 断点先例）
- [ ] 测试（`src/components/NestCheckinDialog.test.ts`）：双按钮渲染与编排（mock photos 创建模式断言参数）；表单展开/收起与照片选择区；空提交拦截；左轴结构类断言（轴容器/节点/今日高亮类）；纯照片条目无正文；空态占位按钮
- [ ] `npx vitest run src/components/NestCheckinDialog.test.ts` 全绿；`vue-tsc` 本票文件无新错

## Blocked by

无，可立即开始（编排断言走 mock，photos.ts 创建模式契约按票 02/规格钉死；若票 02 的 photos.ts 扩展尚未合入，本票测试可先以既有 uploadPhotosHttp 形状 + 规格契约 mock）。

## 涉及路径

- src/components/NestCheckinDialog.vue
- src/components/NestCheckinDialog.test.ts

## 副作用声明

- `npx vitest run src/components/NestCheckinDialog.test.ts` 为本票局部验证；`npx vue-tsc --noEmit` 共享（只看本票文件新错）。

decision_refs: D2, D3, D5, D6
review_blocks: 无
