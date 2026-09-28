# 票 02 · 前端拍照直达：卡片菜单「拍一张」+ 上传创建模式

## What to build

窝卡片「⋯」菜单置顶新增「📷 拍一张」：点了不进弹窗直接调相机（网页端 capture input）/文件选择（桌面），选好照片自动调新保存通道（建当天登记挂照片），成功轻提示「✓ 已登记到「窝名」· 头像已更新」并抛 saved 刷新。

规格：`docs/superpowers/specs/20260928-checkin-photo-entry-spec.md`（Implementation Decisions · 前端节 ColonyCard 部分 + User Stories 1）。

## 验收标准

- [ ] `src/components/ColonyCard.vue`：「⋯」菜单**第一项**新增「📷 拍一张」（双端渲染，桌面/网页行为分流同 NestCheckinDialog onAddPhotos 先例）；卡片层隐藏 capture input（网页）+ 桌面走 pickPhotoFiles
- [ ] 拍照编排：选好 File → 调新通道（见 ipc/photos 扩展）→ 成功 `showSuccess` 轻提示「✓ 已登记到「{窝名}」· 头像已更新」+ `emit("saved")`；失败红轻提示带原因；busy 期间菜单按钮禁用
- [ ] `src/lib/ipc.ts`：新增 `saveCheckinWithPhotos` 包装（命令名 `save_checkin_with_photos`，camelCase 入参：colonyId + 可选 date/queenCount/workerCount/movedNest/note + 照片——桌面传路径数组；网页端不走此命令、走 photos.ts 的 multipart 创建模式）；新命令名进 cmdName 契约测试
- [ ] `src/lib/photos.ts`：`uploadPhotosHttp` 扩展创建模式（或新增函数）：不传 checkinId 时 multipart 带 `colonyId` 必填文本段 + 可选 `date` 段（今天）+ photos 文件段 → `POST /api/photos`，返回 NestPhotoMeta[]；既有 checkinId 模式逐字不变；预检（≤9 张/15MB）沿用
- [ ] 前端测试：菜单项双端可见与置顶断言；拍照编排（mock ipc/photos：断言调用参数与 saved/toast）；photos.test.ts 创建模式 multipart 形状与预检；ipc.test.ts 新命令名
- [ ] `npx vitest run`（票内文件）全绿；`vue-tsc` 本票文件无新错

## Blocked by

无，可立即开始（渲染/编排断言走 mock，命令契约按规格钉死：网页端 multipart 段名 checkinId 缺省+colonyId/date；桌面命令 save_checkin_with_photos camelCase）。

## 涉及路径

- src/components/ColonyCard.vue
- src/components/ColonyCard.test.ts
- src/lib/photos.ts
- src/lib/photos.test.ts
- src/lib/ipc.ts
- src/lib/ipc.test.ts

## 副作用声明

- `npx vitest run src/components/ColonyCard.test.ts src/lib/photos.test.ts src/lib/ipc.test.ts` 为本票局部验证；`npx vue-tsc --noEmit` 共享（只看本票文件新错）。

decision_refs: D2, D4
review_blocks: 无
