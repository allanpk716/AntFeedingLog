# 票 12 · F4 通知双通道 + Pushover

## What to build

Windows toast 保留 + Pushover 手机推送新增，并行双发互不依赖（Q3 修订/Q10=A）。凭证只读环境变量 PUSHOVER_USER/PUSHOVER_TOKEN，不进库不进备份（Q4）。断网失败的推送由下一轮调度（≤30 分钟）补发、超 2 天放弃（Q8=A）。通知开关收敛为单一总开关，删超期/冬眠两个子开关（Q7/Q9）。新增依赖 ureq；schema v5 给台账加 push_title/push_body/pushover_done 列。

## 验收标准

- [ ] Pushover 客户端表单组装/错误传播（注入传输层，不真发网络）
- [ ] run_check 拆 IO：新登记出 toasts + push_jobs；失败下一轮补发、settle 后不再发；超窗自动了结（Rust 测试）
- [ ] 总开关关=全静音；分类子开关不再过滤（Rust 测试）
- [ ] 设置页：Pushover 状态区（只报在/不在）+ 测试按钮分渠道回显"桌面 ✓ / 手机 ✓"
- [ ] v5 迁移：历史台账行直接视为已了结

## Blocked by

—

详细步骤：`docs/superpowers/plans/2026-09-18-feedback-round2.md` Task 3
