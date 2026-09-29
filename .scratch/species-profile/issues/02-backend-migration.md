# 票 02 · v15 迁移 + 自建物种后端 + webui 放行

## What to build

Rust 后端全链（本票无前端）：

1. **v14→v15 迁移**（`db.rs` 编号迁移函数链新增一步，单事务，沿既有先例）：
   - 新表 `custom_species`：id INTEGER PRIMARY KEY AUTOINCREMENT / key TEXT NOT NULL UNIQUE / name TEXT NOT NULL UNIQUE / type TEXT NOT NULL DEFAULT '自定义' / created_at
   - `colony` 加列 `species_key TEXT NULL`（轻迁移，不重建表）
   - 别名归组：迁移内置一张别名表（12 种各自的 正式中文名+全部俗名别名+拉丁名 → key；清单与 proposal §3.3 一致；匹配=trim 后精确匹配、大小写不敏感）；匹配成功 → species_key=档案 key，旧 `species` 列改写为 cnName（规范化）
   - 未匹配的非空旧文本 → 自动建自建物种（name=trim 后原文本、type=自定义；同名文本归并到同一行 INSERT OR IGNORE），species_key=该行 key，旧列快照=原文本
   - 空保持空（两列皆空）
   - 旧 `species` 列保留不删，语义=显示名快照
2. **自建物种命令**（沿 dict.rs/colony.rs 字典先例）：list（含被引用标记）/ create（名字 UNIQUE，重复即返回已存在）/ rename（只改 name 列+级联刷新引用窝的快照列，单条 UPDATE；**绝不改 key**）/ delete（按 key 查引用，被引用禁删）
3. **create/update_colony 扩展**：接受 species_key（可空；内置 key 或 custom-N；归一校验）
4. **webui**：自建清单 list 命令进 `WEBUI_COMMANDS` 白名单（只读）；`webui_args.rs` 的 create/update_colony 参数镜像同步增加 species_key 校验（缺失会拒/丢——这是评审点名的防漏行）；自建写命令不进白名单

key 契约（评审 F1 修复，必须逐字遵守）：自建 key=`custom-<自增id>`，AUTOINCREMENT 保证永不复用；species_key 一律存 key 绝不存名字；内置 key=拉丁 slug 与 custom- 前缀天然隔离。

## 验收标准

- [ ] v15 迁移测试（内存库，沿 db.rs per-version 先例）：别名归组（正式名/俗名/拉丁名/大小写/trim 各有用例）；未匹配转自建（同名归并、名字原样）；匹配成功快照规范化为 cnName；空保持空；幂等（重复跑不重复建行）
- [ ] 命令测试：CRUD；删除被引用禁删；改名后 key 不变、引用窝快照已刷新；重复 create 名字返回已存在不报错崩溃
- [ ] `SCHEMA_VERSION` 升 15，既有迁移链测试全绿
- [ ] webui：list 在白名单可通过；create/update_colony 带 species_key 经参数校验通过；写命令不在白名单（沿 registry 测试断言）
- [ ] cargo test 全绿（已知环境例：端口绑定类既有失败不算回归）

## Blocked by

无，可立即开始（与票 01 并行；别名表清单从 proposal §3.3 独立得出）

## 涉及路径

- src-tauri/src/db.rs
- src-tauri/src/species.rs（新）
- src-tauri/src/colony.rs
- src-tauri/src/lib.rs
- src-tauri/src/webui_server.rs
- src-tauri/src/webui_args.rs

## 副作用声明

cargo test（src-tauri 全量，运行期独占 target 目录）

## decision_refs

D4, D11, D13, D14, D19（key/快照契约=proposal §4.3/§5.7 rev1 定稿）

## review_blocks

无（F1/F2 已在 rev1 解除并复审确认）
