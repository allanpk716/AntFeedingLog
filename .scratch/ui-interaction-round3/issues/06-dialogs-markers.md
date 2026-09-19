# 票 06 · 打卡/喂食弹窗：标记日历 + 重复黄条

**What to build**: monthview 纯函数（buildMarkers/duplicateInfo/dupWarningText）+ QuickLogDialog/FeedDialog 接入 DateTimeField：日历带该窝记录标记（橙点=当前操作、灰点=其它、悬停明细）、选中日已有同操作记录出黄条（不拦提交）。（= 计划 rev1 Task 5 + 复审修正。）

**验收标准**
- [ ] markers 名字表 = colony.actions 全量 + 当前操作兜底（复审修正：只装当前操作会让灰点/tip 整体失效）
- [ ] 灰点集成断言：fixture colony.actions 含第二个操作，当天有其它操作渲染灰点且 tip 带操作名
- [ ] 月份加载带请求序号守卫（快速翻月丢弃过期响应）；时间跨月自动重同步月份（复审修正）
- [ ] 黄条：今天已有 1 条（14:32）文案；无重复不出；提交不被拦
- [ ] App.test.ts baseMock 补 colony_month_records 分支；既有 mockResolvedValue 用例改按命令分发
- [ ] `pnpm test` 全绿后提交

**Blocked by**: 04, 05
