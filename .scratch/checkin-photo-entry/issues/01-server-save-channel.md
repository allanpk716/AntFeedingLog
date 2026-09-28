# 票 01 · 服务端保存通道：通用原子通道 + 更新径语义 + 双端出口

## What to build

「拍一张/完整登记带照片」的服务端整链：纯核通用原子保存通道（字段+照片同事务）、更新径"照片算内容"、网页端 `/api/photos` 创建分支、桌面端新命令。用户视角验收：不带登记号的照片上传=自动建当天登记挂照片；带字段的照片上传=字段照片一并落库；纯照片条目编辑空字段可保存。

规格：`docs/superpowers/specs/20260928-checkin-photo-entry-spec.md`（Implementation Decisions · 服务端节为权威契约——动手前先读）。

## 验收标准

- [ ] 纯核新函数 `save_checkin_with_photos(conn, photos_root, input: CheckinInput, uploads: Vec<PhotoUpload>)`：同事务建登记行（全部表单字段）+ `photo::attach_photos` 挂照片（沿用既有校验链与文件编排）；照片失败登记不落（事务性）
- [ ] 校验语义：uploads 非空 → 字段可全空（跳过 ensure_something_filled）；uploads 空 → 走既有 ensure_something_filled（字段至少一项）；colony 不存在拒；双空（无字段无照片）拒
- [ ] `update_checkin` 内容校验改为「数/换巢/备注全空**且该登记库内无照片**才拒」——已有照片空字段可保存；既有其余校验（日期/计数）不动
- [ ] `webui_server.rs /api/photos`：multipart `checkinId` 缺省时创建模式——补收文本段 `colonyId`（必填，缺则 400 人话）与可选 `date/queenCount/workerCount/movedNest/note`（缺省 date=今天、其余空）→ 走新通道；带 `checkinId` 既有行为逐字不变；成功触发既有写后钩子（巢况不刷托盘口径沿用）
- [ ] `lib.rs` 新 Tauri 命令 `save_checkin_with_photos`（入参 colony_id + 可选字段 + 照片文件路径，读字节后走纯核通道；成功触发 trigger_after_write）+ invoke_handler 注册
- [ ] 纯核单测：新通道四向（字段+照片/仅字段防呆/仅照片/双空拒）+ colony 不存在 + 事务性（照片失败登记不落）；update 两测（已有照片空字段可保存/无照片空字段仍拒）；同日两次「拍一张」头像取后一次（avatar 派生直测）
- [ ] webui_server 测试：/api/photos 创建分支（建行+落盘+返回 NestPhotoMeta[]；缺 colonyId 拒）与既有分支回归
- [ ] `cargo test`（photo/nest_checkin/webui 过滤）全绿（17321 端口环境败例如实归因）

## Blocked by

无，可立即开始。

## 涉及路径

- src-tauri/src/nest_checkin.rs
- src-tauri/src/photo.rs
- src-tauri/src/lib.rs
- src-tauri/src/webui_server.rs

## 副作用声明

- `cargo test`（按模块过滤）为独占验证命令；不跑全量 build、不动端口。

decision_refs: D4, D5, D6
review_blocks: 无
