# 票 05 · 后端命令 colony_month_records（标记与黄条数据源）

**What to build**: 按窝按月返回逐日逐操作计数与最近时刻，支撑日历标记和重复提醒；**支持 excludeLogId 排除指定记录**（编辑场景防自计数——复审修正）。（= 计划 rev1 Task 4 + excludeLogId。）

**验收标准**
- [ ] 返回行 {day, action_id, count, last_time}，同日同操作聚合、last_time 取最近
- [ ] 月区间闭开字典序过滤：跨月、别窝不出现
- [ ] excludeLogId 传入时该记录不计入（编辑自身不报重复）
- [ ] 非法月份给友好错误
- [ ] `cd src-tauri && cargo test` 全绿后提交；TS 类型 MonthDayRecords 同步

**Blocked by**: 无，可立即开始
