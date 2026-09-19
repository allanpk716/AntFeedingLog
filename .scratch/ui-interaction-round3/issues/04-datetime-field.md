# 票 04 · DateTimeField 日期+时间组合控件

**What to build**: DatePickerPop 管日期 + 时间快捷行（现在/−10分/＋10分 + 时分步进）；modelValue 与原 datetime-local 同形态 YYYY-MM-DDTHH:mm。（= 计划 rev1 Task 3 + 复审修正。）

**验收标准**
- [ ] 选日期与原时间合成；**空值选日期时间兜底 00:00**（复审修正）
- [ ] 快捷键/步进 emit 值正确（复审定稿语义）
- [ ] month 事件从内层转发
- [ ] **时间类断言全部"固定基值单步断言"**（复审修正：无 v-model 回写的 mount 下 props 恒为初值，多步链式断言无效）
- [ ] 单文件用例过后跑全量 `pnpm test` 再提交（复审修正）

**Blocked by**: 03
