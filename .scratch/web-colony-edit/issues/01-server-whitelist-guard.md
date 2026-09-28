# 票 01 · 服务端整链：白名单三命令 + 校验镜像 + 状态守卫

## What to build

网页端（HTTP 命令白名单）获得窝资料的写能力：`create_colony` / `update_colony` / `delete_colony` 三命令经 `dispatch_command` 可达，行为与桌面 IPC 同形；入参经 `webui_args` 校验镜像把关；`update_colony` 附加状态守卫。用户视角验收：受信网段浏览器里的编辑/新建/删除请求整链生效，冬眠状态不可被绕过冬眠流程直改。

规格：`docs/superpowers/specs/20260928-web-colony-edit-spec.md`（Implementation Decisions · 服务端节为权威细则）。

## 验收标准

- [ ] `webui_args.rs` 新增 `ColonyInputArgs`（`deny_unknown_fields`，键 snake_case：name/species/location_id/start_date/status/hydration_method/interval_changes）及其 `ValidatedArgs::validate`；`CreateColonyArgs`/`UpdateColonyArgs` 顶层形状（input 嵌套或 id+input，与既有命令风格一致）
- [ ] 镜像校验：status 枚举合法值收 create 场景 active|ended；hydration_method null|manual|tower；interval_days null 或 1..=365；action_id 正整数；name 非空；start_date ISO；未知字段拒绝（serde 层）
- [ ] `dispatch_command` 注册三命令（`write_cmd`，`with_tray=true`）：create→`colony::create_colony`；update→状态守卫后 `colony::update_colony`；delete→先 `photo::collect_colony_photo_paths` → `colony::delete_colony` → `photo::delete_photo_files`（失败仅 applog 孤儿记账，不回滚库；photos_root=deps.data_dir.join(PHOTOS_DIR_NAME)，同既有 `delete_checkin` 分支先例）
- [ ] 状态守卫（update 分支闭包内、同一把库锁先读库内 status 再判定，不过则返回人话错误）：库内=hibernating → 新值必须=hibernating（不变更豁免），active/ended 拒；库内∈{active,ended} → 新值=hibernating 拒，active↔ended 放行。守卫是镜像校验之外的附加判定，不替代镜像与纯核校验链
- [ ] 单测（webui_args）：合法载荷通过；status=hibernating 的 create 拒；interval_days 0/366 拒；未知字段拒
- [ ] 单测（webui_server dispatch）：三命令白名单路由可达；返回形状与桌面 invoke 同形（仿既有 `http_return_shape_matches_desktop_invoke_for_record_commands` 先例）
- [ ] 单测（delete 同序）：先库事务删行提交、再删照片文件（仿既有 `http_delete_checkin_removes_photo_files_same_order` 先例）
- [ ] 守卫专项四条：冬眠窝经 web update 改名字/周期可保存且库内 status 保持 hibernating；非冬眠窝直发 hibernating 拒；冬眠窝直发 active/ended 拒；active↔ended 直切放行
- [ ] 纯核（colony.rs / photo.rs）零改动；`cargo test`（webui 相关过滤）全绿

## Blocked by

无，可立即开始。

## 涉及路径

- src-tauri/src/webui_args.rs
- src-tauri/src/webui_server.rs

## 副作用声明

- `cargo test webui`（或按模块过滤的局部测试）为独占验证命令；不跑 `cargo build` 全量、不动端口。

decision_refs: D2, D3, D7
review_blocks: 无
