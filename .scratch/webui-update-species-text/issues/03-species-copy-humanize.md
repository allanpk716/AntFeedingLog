# 票 03 · 物种档案文案人化:12 份 JSON + 契约与测试同步

## What to build

把图鉴页物种档案的文案从"论文注脚腔"改成自然话——**事实一个字不许动**:

1. **src/data/species/ 全部 12 份 JSON、全字段**(顶层 21 字段 + growth 10 + risks 6 + careNotes,凡非空字符串都过一遍):
   - **删纯来源括号**(删括号及其内容,句子衔接顺好):（多源）（文献）（文献+多源）（商家单源）（商家口径）（原调研）（属级估计）（属级）（经验）（属通病）（单源经验）
   - **括号里是内容的保留**:如「几块~30 元（新后小群，原调研）」→「几块~30 元（新后小群）」——只删"，原调研"这类来源尾巴;「单型无大工（多源：欧洲北非二十余种收获蚁中唯一无品级分化的种）」→「单型无大工——欧洲北非二十余种收获蚁中唯一无品级分化的种」(来源词删,考证内容留)。
   - **改元话语**:「低置信」→ 自然说法(如「各家说法不一」「不太靠得住」,按语境);「查无实据」/「查无可靠数据」→「没有可靠数据」;「量化查无实据;」→「没有可靠的量化数据;」这类顺句。
   - **事实一字不虚**:数字、温度区间、价格、周数、寿命、警告、口径冲突(如"适宜 22~26°C 或 24~28°C——两种商家口径并存"→"商家说法不一,22~26 和 24~28 都有")、考证结论(如"商家 6~8mm 大工说法与单型结论冲突,疑误不采")全部保留,只顺句子。空字符串字段一律不动(空=「暂无资料」契约)。
   - 改写参考风格(已获用户确认的口径):SpeciesGuidePage.vue 通用饲养知识与避坑名单的既有文案——短句、直给、术语少。
2. **speciesProfiles.ts 头注释**:删除「有据值行内标注来源档（文献/多源/商家/经验/属级估计）」一条;补一行"文案为自然话叙述,来源考证存档见 docs/调研养蚂蚁的文档/"(其余注释不动)。
3. **speciesProfiles.test.ts**:既有断言若引用被改文案则同步;**新增全量扫描断言**——allSpeciesProfiles() 全部档案的全部字符串字段(含 growth/risks/careNotes/aliases)不得含:上述 11 种来源括号标记、「低置信」、「查无实据」。
4. **SpeciesGuidePage.vue**:避坑名单香斑弓背蚁条「…暴毙率高、新手翻车多（多源一致），建议有经验后再碰」删「（多源一致）」→「…暴毙率高、新手翻车多,建议有经验后再碰」。**其余避坑 6 条、通用饲养知识七主题、页面结构一律不动。**
5. **App.test.ts / SpeciesSelect.test.ts / ColonyCard.test.ts**:若其中断言引用了被改的 JSON 文案,按新文案最小同步;不新增断言、不改测试结构。

**改写纪律**:逐文件手改,不许写脚本批量正则替换(会误伤内容括号);每改完一份自查:数字/温度/价格/周数与原文一致。

## 验收标准

- [ ] 12 份 JSON 全字段无 11 种来源括号、无「低置信」「查无实据」(扫描断言在测)
- [ ] 数字/温度/价格/周数/寿命与原值一致(自查 + 评审 diff 抽查)
- [ ] 空字符串字段保持空(「暂无资料」契约不动)
- [ ] npx vitest run(涉及测试文件)全绿;vue-tsc 0 错误
- [ ] SpeciesGuidePage.vue 只有香斑一处改动
- [ ] recommendReason/hibernationNote 等新手推荐区字段语义不变(推荐理由、路线归类仍成立)

## Blocked by

无,可立即开始

## 涉及路径

- src/data/species/(12 份 .json)
- src/lib/speciesProfiles.ts
- src/lib/speciesProfiles.test.ts
- src/components/SpeciesGuidePage.vue
- src/App.test.ts
- src/components/SpeciesSelect.test.ts
- src/components/ColonyCard.test.ts

## 副作用声明

- 局部验证:`npx vitest run src/lib/speciesProfiles.test.ts src/App.test.ts src/components/SpeciesSelect.test.ts src/components/ColonyCard.test.ts`
- 不跑 cargo;不动 docs/调研养蚂蚁的文档/(只读参考);不动 src/components/SpeciesGuidePage.vue 除香斑条外内容

## decision_refs

D5(全 12 份全字段)、D6(删留口径)、D9(避坑只删一处)、F5(机械护栏)、F9(扫描含两元话语)

## review_blocks

无
