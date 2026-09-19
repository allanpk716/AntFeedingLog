# 票 01 · schema v8:colony_action_interval 表 + 每窝周期读写命令与校验

## What to build
库迁移链升到 v8,新增"窝 × 操作"周期表 `colony_action_interval`(结构见验收第 1 条);Rust 侧提供每窝周期的设置/清除命令(桌面 invoke),带 1..365 整数校验,并注册进命令表。本票只做"存得进、读得出、校验得住",不接提醒判定(票 02)、不接前端(票 03/04)。

## 验收标准
- [ ] v7 旧库迁移后出现空 `colony_action_interval` 表:`colony_id INTEGER NOT NULL REFERENCES colony(id)`、`action_id INTEGER NOT NULL REFERENCES care_action(id)`、`interval_days INTEGER NOT NULL` + CHECK(`interval_days BETWEEN 1 AND 365`)、PRIMARY KEY(`colony_id`,`action_id`);外键裸 REFERENCES(库内惯例,不用 ON DELETE CASCADE)
- [ ] 全新建库直接含该表;重复迁移幂等;data_version 升 v8
- [ ] 命令(命名可按仓库惯例,如 `set_colony_action_interval`):入参 colony_id、action_id、interval_days(Option)——Some(1..365) upsert 一行,None/清除删行;窝或操作不存在返回人话错误;0、负数、366、非整数一律拒绝并给人话报错
- [ ] 读侧提供按窝批量读周期的内部函数(供票 02 的 tiles/提醒接线;本票不改 tiles_for_colony 行为)
- [ ] 命令注册进桌面 invoke 表(src-tauri/src/lib.rs);不得加入 webui HTTP 白名单(若白名单在别处声明,保持不动即满足)
- [ ] cargo test 全绿(新增:迁移用例、命令 upsert/清除/校验/不存在用例);clippy 不引入新告警

## Blocked by
无,可立即开始。

## 涉及路径
- src-tauri/src/db.rs
- src-tauri/src/data_version.rs
- src-tauri/src/colony.rs
- src-tauri/src/lib.rs

## 副作用声明
- 独占验证命令:`cargo test`(在 src-tauri 下),会占构建目录;不跑前端命令。

decision_refs: D1, D2, D6
review_blocks: F4(校验部分)
