# 01 卡片横幅 + 形状偏好移除

Status: resolved

## Answer

- ColonyCard 横幅落地：内嵌正方形容器（宽=横幅宽、aspect-ratio 1/1、垂直居中）沿用 avatarImgStyle 百分比裁剪数学原样；名字/物种/状态/天数叠 scrim；右上角「巢况/📷」玻璃按钮（stop 冒泡）；点横幅开时间线；🐜 占位放大居中；冬眠卡照片灰化；桌面 118px / ≤480px 150px；底部「巢况」按钮区整体移除（横幅已有入口）
- 形状偏好全链路删除：ipc.ts（类型/命令/镜像/claim/reset）、ColonyCard（shape prop/shapeClass/启动读取）、SettingsDialog（外观 tab 整个移除：类型/按钮/页签体/script/恢复路径重拉）、PhotoCropEditor（圆形预览遮罩）、lib.rs 命令与注册、settings.rs 键与函数与测试、webui_server.rs 白名单/dispatch/测试；settings 键清理由 v14 迁移 DELETE 覆盖
- 验证：cargo test 652/653（唯一败=端口环境例）、vitest 770/770、vue-tsc 0 错

## Comments
