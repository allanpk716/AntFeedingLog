# 票 01 · 共用加载占位组件 + 加载态规范沉淀

## What to build
打通"加载态"的基建端到端切片：新增全局共用的加载占位组件（纯文字「加载中…」），并把「加载态判定原则」沉淀进项目规范（CLAUDE.md 操作反馈规范旁并列一节）与领域词汇表（CONTEXT.md 新词条）。后续所有票的占位接入一律用这个组件，各处不得自造样式。

组件行为（用户视角）：渲染一行居中或左对齐的低调提示文字「加载中…」，无其它装饰；不接收业务 props；样式与既有 .hint/.wall-empty 一族（小字、muted 色）。

规范文案（逐字采用，勿改写语义）：

CLAUDE.md 增节（与「操作反馈规范」并列）：

> ## 加载态规范（全局强制，所有异步读取的界面适用）
>
> - **判定原则：界面切换或打开时，内容需异步读取的，读取完成前必须给「加载中…」占位——不许空白，也不许先显示默认值再跳成真实值；数据已在内存（缓存/父组件状态）时即时渲染，不显示加载态。**
> - 实现：全局唯一加载占位组件（`src/components/LoadingHint.vue`）；各处只调用，不得自造提示样式。
> - 加载态管「读之前」，轻提示管「做完之后」，两套永不通勤。

CONTEXT.md 在「轻提示（Toast）」词条附近增词条：

> **加载态（Loading State）**：
> 界面内容需等异步读取时的诚实占位（「加载中…」），读取完成即消失；数据已在内存时直接渲染、不进加载态。
> _Avoid_: 与「轻提示」混用——加载态是"还没读到"，轻提示是"做完了"，一个管读一个管写。

## 验收标准
- [ ] `src/components/LoadingHint.vue` 存在，渲染文本「加载中…」
- [ ] `src/components/LoadingHint.test.ts` 存在并通过：渲染出「加载中…」文本
- [ ] CLAUDE.md 含上述「加载态规范」一节（判定原则 + 全局唯一组件 + 与轻提示的边界三要素）
- [ ] CONTEXT.md 含「加载态（Loading State）」词条（定义 + Avoid 与轻提示区分）
- [ ] `pnpm vitest run src/components/LoadingHint.test.ts` 全绿
- [ ] `pnpm exec vue-tsc --noEmit` 0 错

## Blocked by
无，可立即开始。

## 涉及路径
- src/components/LoadingHint.vue（新增）
- src/components/LoadingHint.test.ts（新增）
- CLAUDE.md（追加一节）
- CONTEXT.md（追加一词条）

## 副作用声明
- 独占验证：`pnpm vitest run src/components/LoadingHint.test.ts`
- 只读共用：`pnpm exec vue-tsc --noEmit`（全局类型检查，可与其它票并行）

## decision_refs: D3、D5、D8
## review_blocks: 无
