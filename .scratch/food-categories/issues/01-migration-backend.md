# 01 · DB 迁移 v13 + Rust 后端

Status: done

## 内容

- db.rs：SCHEMA_VERSION 12→13；`migrate_v12_to_v13` 单事务——food 加 `category`（TEXT NOT NULL DEFAULT 'seed'，CHECK 三值，幂等防重加列）；按名归组（干虾仁/面包虫→protein）；预置改名（撞名跳过）；INSERT OR IGNORE 四新预置 + 按名升格（樱桃蟑螂/蜂蜜/冰糖水/白糖水，24h 易腐）；间隔改值（种子 7、蛋白质 3、糖水 3）；sort 分组重排（自建行 +100 挪到预置后，相对序保留）。
- care.rs：Food DTO 加 `category`；FOOD_SQL/row_to_food 带列。
- dict.rs：`FOOD_CATEGORIES` 常量；FoodInput 加 `category: Option<String>`（serde 缺省兼容）；save_food 校验（新增必填、非法拒收、更新 COALESCE 保留）。

## 验收

- [ ] 新迁移测试：v12 库（含自建行+历史引用）升级后七预置的 name/category/interval/perishable/sort 全对、自建行归 seed 且 sort+100、log_food 引用显示新名。
- [ ] 撞名升格测试：预先自建「蜂蜜」→ 升格为预置属性、无重复行。
- [ ] save_food 四条校验测试。
- [ ] 全量 cargo test 通过（既有夹具随新预置名/新间隔更新）。
