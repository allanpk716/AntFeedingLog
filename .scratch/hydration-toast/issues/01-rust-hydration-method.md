# 票 01 · Rust 数据层:schema v11 保湿方式列 + 原子命令 + 读取链路

## What to build
为"巢穴保湿按窝细调"打通 Rust 侧端到端:库迁移给 colonies 加 hydration_method 可空枚举列(NULL=未设/'manual'=手动加水/'tower'=水塔,迁移幂等且**不动 colony_action_interval 任何行**);窝的新建/编辑命令扩展为承载保湿方式与周期增删的**原子写入**——新建"窝+方式+初始周期"同命令同事务、任一步失败整体失败窝不落库;编辑保存为**整窗单事务**(基础字段+方式变更+周期增删一次提交,"清回未设+删周期行"原子);读取链路(list_colonies 等既有命令)带出 hydration_method,既有每窝周期字段与 interval_from_colony 语义原样不变。落库语义必须实现规格状态矩阵:库内方式原值=未设时,本次保存把方式"选了又改回未设"的净效果为零(不删已有周期行);库内原值非未设且本次清回未设 → 删该窝保湿周期行;未触碰方式与周期的编辑保存对两者零改动。

## 验收标准
- [ ] schema 升至 v11,hydration_method 列存在;对 v10 库迁移幂等,既有 colony_action_interval 行逐行保留(升级前后快照一致)
- [ ] 新建命令:带方式+初始周期同事务落库;构造周期写入失败(如非法天数)时窝整体不落库
- [ ] 编辑命令:整窗单事务——基础字段+方式变更+周期增删一次提交;保湿部分失败时名字等基础字段也不落库
- [ ] 净零语义:库内方式=未设+已有周期,保存"改为某方式又改回未设"后周期行仍在
- [ ] 清回未设:库内方式=manual/tower 且保存清回未设 → 周期行被删;库内=未设时任何编辑保存不删周期行
- [ ] list_colonies 等读取命令返回 hydration_method;interval_from_colony 既有行为有回归用例
- [ ] cargo test 全绿(新旧用例)

## Blocked by
无,可立即开始

## 涉及路径
src-tauri/src/(全部 Rust 源;含 db.rs、care.rs、lib.rs、reminder.rs、webui_args.rs、webui_server.rs 及 tests)

## 副作用声明
允许独占运行 `cargo test`(工作目录 src-tauri;含构建产物 target/);不允许安装依赖、不联网

## decision_refs
D1、D2、D9、D3;F2、F6(原子性)、F4(净零语义)、F1(状态矩阵)

## review_blocks
无
