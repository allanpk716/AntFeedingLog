# 票 07 · 后端 list_logs 地点筛选 + 行带地点名

**What to build**: 记录查询支持按地点过滤；每行返回所属地点名。（= 计划 rev1 Task 6。）

**验收标准**
- [ ] LogFilter 加 location_id（与 colony_id 组合生效）；COUNT 查询补 JOIN colony
- [ ] LogRow 加 location_name（LEFT JOIN location；未分组为 null）
- [ ] TS 类型与 lib/loglist（表单字段、buildLogFilter、级联纯函数 colonyOptionsFor）同步
- [ ] LogListPage.test.ts 期望对象补 location_id（页面改造在票 08，先保绿）
- [ ] `cargo test` + `pnpm test` 全绿后提交

**Blocked by**: 无，可立即开始
