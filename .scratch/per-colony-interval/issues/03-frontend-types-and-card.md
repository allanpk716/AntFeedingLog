# 票 03 · 前端类型与卡片展示(距上次/周期/超期三形态)

## What to build
前端消费票 02 的 tiles 新数据:类型层同步、纯逻辑层文案三形态、卡片渲染。只做展示,不做设置入口(票 04)。

## 验收标准
- [ ] tile 文案三形态:①设了周期未超 → "距上次 N 天 / 周期 M 天"(M=该窝有效周期);②超期 → "⚠ 超期 X 天",X = days − 有效周期(与判定同源);③未设周期 → 与现状完全一致(登记类只显示距上次、永不红)
- [ ] 喂食食物行照旧独立标各自超期,不因每窝周期改变食物行口径
- [ ] 网页端首页同源生效(同一 IPC 数据,无端别代码)
- [ ] vitest 全绿(新增/更新 care 纯逻辑用例覆盖三形态与食物行);类型检查(vue-tsc)无新错误

## Blocked by
票 02。

## 涉及路径
- src/types.ts
- src/lib/care.ts
- src/lib/care.test.ts
- src/lib/home.ts
- src/components/ColonyCard.vue

## 副作用声明
- 独占验证命令:`pnpm vitest run src/lib/care.test.ts` 与类型检查;全量 vitest 留给票 04 前跑一次。

decision_refs: D7
review_blocks: F2(展示口径)
