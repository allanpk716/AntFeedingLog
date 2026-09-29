# 票 03 · 物种选择器 + 窝表单集成 + 卡片徽章

## What to build

前端消费层（依赖票 01 的档案加载器、票 02 的后端命令）：

1. **SpeciesSelect 组件**（新）：替换 ColonyFormDialog 里的物种自由文本输入框——按类型分组（档案 distinct + 自建物种类型；分组动态）、冬眠需求筛选（三值）、搜索（中文名/拉丁名/别名）、悬停一句话摘要（温度/食物/冬眠，来自票 01 的摘要助手）、**＋自建物种内联新建**（名字必填、类型默认"自定义"可改填新类型名；名字与已有自建重复→提示已存在并直接选用）；物种可不填（保持可空语义，可清除）
2. **表单与 IPC**：`colonyForm.ts` 增 species_key（空→null）；`types.ts` Colony/ColonyInput 增 species_key；`ipc.ts` 增自建物种命令包装（list/create/rename/delete）与 create/update colony 透传
3. **窝卡片徽章**（ColonyCard.vue）：渲染改为**按 species_key 实时解析优先**（内置档案∪自建清单）显示当前中文名；key 不可解析→回落 species 快照列；两皆空不显示；悬停 title="拉丁名 · 类型 · 冬眠需求"
4. 写操作走全局 toast 规范（自建新建等无天然反馈的操作）；列表数据已有内存缓存时即时渲染不进加载态，异步拉自建清单才显示加载占位（全局 LoadingHint 规范）

## 验收标准

- [ ] 选择器：类型分组正确（含自建"自定义"组）；冬眠筛选；搜索命中中文名/拉丁名/别名；悬停摘要出现
- [ ] 内联自建：建后即选；重名提示并选用；可清除物种（两列皆空语义）
- [ ] 物种可不填：不选也能保存
- [ ] 保存链路：species_key 正确写入（表单→ipc→命令）
- [ ] 徽章：解析优先/快照兜底/两皆空不显示/悬停三要素，四种情形各有用例
- [ ] vitest 全绿，既有 ColonyFormDialog/ColonyCard 测试不回归（按新行为更新断言属正常）

## Blocked by

票 01、票 02

## 涉及路径

- src/components/SpeciesSelect.vue（新）+ 同名 test（新）
- src/components/ColonyFormDialog.vue（+既有 test 更新）
- src/components/ColonyCard.vue（+既有 test 更新）
- src/lib/colonyForm.ts（+test）
- src/lib/ipc.ts
- src/types.ts

## 副作用声明

默认（类型检查/单文件测试）

## decision_refs

D3, D6, D12, D13, D19（快照渲染=proposal §4.4/§5.7）

## review_blocks

无
