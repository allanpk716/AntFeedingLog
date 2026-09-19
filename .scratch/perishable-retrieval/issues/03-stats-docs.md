# 票 03 · 统计/流水/导出纳入撤食 + 文档刷新

## What to build

打通"撤食记录出现在所有可视化与导出"的端到端切片：排查统计与流水前后端是否有写死的操作清单（只认四种操作/两种性质），有则纳入撤食；README 功能清单补撤食句。

- **统计**：日历热力图（悬停当天明细）、每周操作次数、喂食构成等各统计口径中，撤食作为普通维护操作参与计数与展示；`follow` 性质不参与任何超期/建议间隔口径（它没有间隔）。测试自种撤食记录（直接 INSERT care_action + care_log，不依赖卡片 UI）验证聚合。
- **流水**：记录页按操作筛选的下拉含撤食；行内编辑/删除对撤食记录照常。
- **导出**：CSV/JSON 导出行含撤食记录（沿用现有导出管道，确认无操作白名单）。
- **README**：功能清单补一句撤食提醒（喂易腐食物超时未撤每日提醒、卡片撤食块、设置易腐配置），测试计数如有变化按实际刷新。

## 验收标准

- [ ] 日历热力图含撤食记录，悬停明细可見
- [ ] 每周操作次数统计含撤食
- [ ] 记录页按"撤食"筛选出结果；撤食行可编辑/删除
- [ ] CSV/JSON 导出含撤食行
- [ ] README 已补撤食功能句
- [ ] 相应 lib/组件测试通过

## Blocked by

无，可立即开始

## 涉及路径

- src-tauri/src/stats.rs
- src/lib/stats.ts
- src/lib/stats.test.ts
- src/lib/loglist.ts
- src/lib/loglist.test.ts
- src/components/StatsPage.vue
- src/components/StatsPage.test.ts
- src/components/LogListPage.vue
- src/components/LogListPage.test.ts
- README.md

## 副作用声明

- Rust 验证（**模块过滤，避免与并行票抢全量 cargo 锁**）：`cd src-tauri && cargo test stats`
- 前端：`pnpm vitest run src/lib/stats.test.ts src/lib/loglist.test.ts src/components/StatsPage.test.ts src/components/LogListPage.test.ts`

decision_refs: D14
review_blocks: 无
