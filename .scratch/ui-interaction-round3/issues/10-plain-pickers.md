# 票 10 · 新建窝/冬眠弹窗换素版日历

**What to build**: ColonyFormDialog（1 处）与 HibernationDialog（5 处）的原生 date 输入换 DatePickerPop；冬眠 +120 天联动行为不变。（= 计划 rev1 Task 9。）

**验收标准**
- [ ] 6 处原生 date 全部替换为 DatePickerPop（v-model 形态一致）
- [ ] 开始冬眠：默认今天、预计=+120 天；手动改过预计后不再自动覆盖（既有两条用例语义不变）
- [ ] App.test.ts 相关 setValue 改 findComponent emit/props
- [ ] `pnpm test` 全绿后提交

**Blocked by**: 03
