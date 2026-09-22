# 票 01 · 后端基座:schema v12 裁剪列 + 头像投影 + 裁剪更新与照片墙命令

## What to build
打通"窝头像与照片墙"的全部后端能力(端到端窄基座,本票无 UI):
1. **schema v11→v12 迁移**:nest_photo 表新增裁剪三列(归一化 x/y/边长,REAL 可空);存量行保持 NULL(=默认居中),不回填;迁移幂等,旧库升级不丢数据。
2. **照片元数据贯通**:照片元数据(NestPhotoMeta,桌面/网页同构)带裁剪字段(NULL=居中);既有照片链路(时间线/上传返回/卡片摘要)不受影响。
3. **头像投影**:首页窝列表载荷每窝新增"头像照片"引用——按排序契约(登记日期倒序、同日期登记序号倒序、照片按序号正序)扫描,取**第一条含照片的登记**的第一张照片(纯文字登记跳过);无照片的窝为空。纯投影不落库。
4. **裁剪更新命令**:update_photo_crop(照片 id,裁剪或 NULL=重置居中);服务端校验(数值 ∈[0,1]、x+边长≤1、y+边长≤1,非法拒绝);桌面 IPC + 网页端命令白名单各接一条。
5. **照片墙只读命令**:photo_wall() 返回全部窝的照片——每窝{窝 id、窝名、按登记日期倒序的分组[日期、该日期登记按序、登记内照片按上传序]},照片带元数据(含裁剪);桌面 IPC + 网页端只读白名单各接一条。
6. 前端类型与 IPC 声明同步(types.ts / ipc.ts 加声明,不实现 UI)。

## 验收标准
- [ ] v11 库升级到 v12:迁移后既有照片行裁剪列为 NULL,数据无损,重复启动幂等
- [ ] update_photo_crop:合法裁剪落库并可读回;NULL 重置为居中语义;越界(x+边长>1 等)与非法值被拒且有测试
- [ ] 头像投影:最新登记无照片时跳过取更早含照片登记;同日期多条登记取创建序号最大者;同登记多照片取序号最小者;删登记后投影自动回退;无照片窝返回空
- [ ] photo_wall:分组/排序符合排序契约(跨窝、同日期多登记、登记内多照片),载荷含窝名与日期
- [ ] 桌面 IPC 与网页端命令白名单均注册;网页端无写路径泄漏(photo_wall 只读、update_photo_crop 为唯一新写)
- [ ] cargo test 全绿;vue-tsc 0 错(types/ipc 声明同步后)

## Blocked by
无,可立即开始。

## 涉及路径
- src-tauri/src/db.rs
- src-tauri/src/nest_checkin.rs
- src-tauri/src/photo.rs
- src-tauri/src/colony.rs
- src-tauri/src/lib.rs
- src-tauri/src/webui_server.rs
- src-tauri/src/webui_args.rs
- src/types.ts
- src/lib/ipc.ts

## 副作用声明
- 独占验证命令:cargo test(全量);npx vue-tsc --noEmit
- 新增数据库迁移(用户库升级路径);不联网、不装依赖

decision_refs: D1, D3, D6, D7, D8 · review_blocks: 无(F1 已解除;F3/F6 在本票落实)
