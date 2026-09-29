# 票 05 · 设置·物种 tab（自建物种管理）

## What to build

设置对话框新增「物种」tab（桌面端；依赖票 02 命令与票 03 的 IPC 包装/类型）：

1. **SpeciesManagerPanel**（新组件，沿 LocationManagerPanel 先例）：自建物种清单（名字/类型/被引用标记）+ 行级操作：
   - 改名（行内编辑；保存后引用窝的徽章显示随之刷新——级联刷新在后端，前端只需重新拉取）
   - 删除（有窝引用禁删——按钮置灰+原因提示；未被引用可删）
2. 内置 12 种不在此管理（不可删改、无停用开关）；tab 排在现有 tabs 中（建议「地点」后）
3. 写操作走全局 toast（成功绿 2.5s/失败红 5s 带原因）；清单异步读取显示全局 LoadingHint 占位

## 验收标准

- [ ] 清单渲染自建物种（含类型与被引用标记）；无自建时空态提示
- [ ] 改名保存生效；重名报错走失败 toast（带原因）
- [ ] 被引用行删除禁用并提示；未引用行可删
- [ ] 写操作 toast 合规（全局唯一 toast，不自造样式）
- [ ] 设置对话框既有 tabs 不回归；vitest 全绿

## Blocked by

票 02、票 03

## 涉及路径

- src/components/SpeciesManagerPanel.vue（新）+ 同名 test（新）
- src/components/SettingsDialog.vue（+既有 test 更新）

## 副作用声明

默认（类型检查/单文件测试）

## decision_refs

D4, D13, D19

## review_blocks

无
