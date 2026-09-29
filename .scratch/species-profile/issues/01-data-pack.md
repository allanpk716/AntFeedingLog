# 票 01 · 物种档案数据包与加载器

## What to build

建立前端物种档案基础设施（本票无 UI，图鉴/选择器票消费本票产出）：

1. **TS 类型**：`SpeciesProfile`（顶层 21 字段 + `growth` 组 10 字段 + `risks` 组 6 字段 + `careNotes: string[]`，字段清单见 spec 与 `.xcheck/20260929-164029/proposal.md` §3.2 的日本弓背蚁完整示例）。
2. **glob 加载器**：构建期导入 `src/data/species/*.json`（一物种一文件，文件名=key），提供：按 key 查询、全部列表、类型 distinct 分组（**禁写死枚举**——分组必须从数据动态得出）、供悬停摘要的一行式摘要助手（温度/食物/冬眠三要素）。
3. **12 份内置档案 JSON**：数据唯一来源=三份调研存档（路径见下），按 spec「内置数据与内容口径」填充——命名用正式中文名+俗名进 aliases、冬眠口径按修正值、香斑★★★、推荐位 4 种（针毛/红头/中亚/拟光腹）带 recommendReason、空字符串=查无实据（绝不编数字）、有据值行内标注来源档（文献/多源/商家/经验/属级估计）、**隐私清洗**（去个人位置/场景，物种分布中的广西保留，本地化结论改条件式——规则见 proposal §6）。

数据来源（只读）：`.xcheck/20260929-161746/research-messor.md`、`research-camponotus-dormant.md`、`research-camponotus-non-dormant.md`；清单/命名/冬眠/难度裁决见 `.xcheck/20260929-164029/proposal.md` §3.3（12 种 key 与中文名对照表在内）。用户原文档 `C:\WorkSpace\agent\AntFeedingLog\docs\调研养蚂蚁的文档\` 含个人隐私，仅作参考、内容必须清洗后才能进档案。

## 验收标准

- [ ] `src/data/species/` 下恰好 12 份 JSON，key 与 proposal §3.3 表一致（messor-aciculatus … camponotus-nicobarensis）
- [ ] 加载器测试：按 key 查询命中；全部=12；类型 distinct=收获蚁/弓背蚁两组；空字段原样保留（不填默认值）
- [ ] 每份档案字段完整覆盖 schema（允许空字符串，不允许缺字段）
- [ ] 档案内容零隐私信息（无 桂林/办公室/已养三窝/共用加热垫 字样）；分布产地中的广西允许保留
- [ ] 命名裁决落实：cnName=中亚弓背蚁/瑕疵弓背蚁（俗名在 aliases）；无"热带黑金"字样作丝腹别名
- [ ] 推荐位恰 4 种且带理由（红头含"建群头两个月少看少动"）
- [ ] vitest 全绿，既有测试不回归

## Blocked by

无，可立即开始

## 涉及路径

- src/data/species/（新目录）
- src/lib/speciesProfiles.ts（新）
- src/lib/speciesProfiles.test.ts（新）

## 副作用声明

默认（类型检查/单文件测试）

## decision_refs

D2, D3, D8, D9, D14, D15, D16, D17, D18

## review_blocks

无
