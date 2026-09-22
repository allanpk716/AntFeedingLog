# 票 03 · 头像形状偏好(设置键 + 设置 UI + 全局生效)

## What to build
头像显示形状的全局偏好:
1. 设置存储新增键 avatar_shape(circle|square,默认 circle),存库内既有设置键值表;读给双端,写只在桌面设置页。
2. 桌面设置弹窗新增"头像形状"选择(圆形/方形,单选,即存);保存成功按「轻提示」规范给结果提示(设置保存属"改了不关窗"写操作)。
3. 卡片头像渲染接线形状偏好(消费票 02 的渲染形状参数,从设置读;改设置后已渲染头像立即切换,无需刷新页面)。
4. 网页端读同一设置(网页无设置页,只读生效)。

## 验收标准
- [ ] 默认圆形;设置切换为方形后全局所有窝头像立即变方形,无残留圆形
- [ ] 设置持久化:重启应用/刷新网页后形状保持
- [ ] 网页端跟随同一设置(只读)
- [ ] 设置保存有轻提示(成功绿色;失败红色带原因)
- [ ] 圆形=方形裁剪框挖角,已有裁剪区域不重算(裁剪与形状解耦)
- [ ] SettingsDialog/ColonyCard 相关测试先红后绿;vue-tsc 0 错

## Blocked by
票 02(ColonyCard 头像渲染形状参数)。

## 涉及路径
- src-tauri/src/settings.rs
- src-tauri/src/lib.rs(如需注册命令)
- src-tauri/src/webui_server.rs(网页只读接入,如需)
- src/lib/ipc.ts
- src/components/SettingsDialog.vue
- src/components/SettingsDialog.test.ts
- src/components/ColonyCard.vue
- src/components/ColonyCard.test.ts

## 副作用声明
- 默认只跑类型检查与单文件组件测试(npx vitest run src/components/SettingsDialog.test.ts src/components/ColonyCard.test.ts);如改 Rust 需 cargo test;不联网、不装依赖

decision_refs: D2, D3(形状与裁剪解耦) · review_blocks: 无
