# 票 06 · SSE 与数据版本广播(跨重启单调)

规格:Implementation Decisions C;User Stories 12/15

## What to build

- `data_meta` 持久数据版本计数:每次写事务内 +1(单连接串行下天然无竞态)。
- 写操作提交后广播:桌面 Tauri 事件 + 网页 SSE 同源同版本号;`db-restored` 并入同广播且语义=无条件刷新。
- SSE 首帧=实例 epoch(安装 id+启动时间)+当前版本号;**客户端关闭 EventSource 原生自动重连**(onerror 中 close),error→重取票据→重建连。
- 对账规则:**同 epoch 且本地版本落后→重拉当前页;epoch 不同→无条件重拉**(覆盖"手机长开+电脑重启"与恢复旧备份)。

## 验收标准

- [ ] 版本计数跨重启单调(持久化测试)
- [ ] 模拟新 epoch(重启)后客户端无条件重拉
- [ ] 票据消费后原生重连不发生(测试关掉自动重连)
- [ ] 恢复完成→两端无条件刷新
- [ ] 桌面事件与 SSE 同版本号;测试齐

## Blocked by

票 02、票 04
