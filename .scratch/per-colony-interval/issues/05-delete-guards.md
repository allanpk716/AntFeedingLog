# 票 05 · 删除路径守卫与清理(自定义操作引用计数 + 删窝清行)

## What to build
数据生命周期收口:删除自定义操作的引用守卫补上每窝周期表计数;删除窝的事务顺带清理该窝全部周期行。

## 验收标准
- [ ] delete_action(dict.rs):在既有 care_log、reminder_ledger 引用计数之外,补 colony_action_interval 计数——该操作存在周期行时拒删,报错文案人话、风格同既有(提示可先清各窝周期或改停用)
- [ ] delete_colony(colony.rs):删除事务内顺带 `DELETE FROM colony_action_interval WHERE colony_id = ?`,删后无孤儿行;预置操作不可删的既有约束不变
- [ ] cargo test 全绿(新增:有周期行拒删、清空后可删、删窝后行清零)

## Blocked by
票 01。

## 涉及路径
- src-tauri/src/dict.rs
- src-tauri/src/colony.rs

## 副作用声明
- 独占验证命令:`cargo test`(在 src-tauri 下);不跑前端命令。

decision_refs: D8
review_blocks: F4(删除部分)
