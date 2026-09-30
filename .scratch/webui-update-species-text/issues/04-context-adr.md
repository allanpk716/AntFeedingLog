# 票 04 · 文档:CONTEXT.md 词条 + 回填 ADR 0009 + 新 ADR 0010

## What to build

三份文档改动(体例照 docs/adr/ 既有文件:标题一句定调 + 正文叙述 + Considered Options + Consequences;ADR-0003 是最近的对比例):

1. **CONTEXT.md** 两处词条更新:
   - 「网页端」词条:把「更新」移出"系统管理一律没有"清单——系统管理列举改为(字典、备份、恢复、导出、设置),并加一句:网页端可"发现新版→点击升级"(只读红点详情 + 触发升级流;升级的其余管理仍留桌面端)。
   - 「桌面端」词条:同步把「更新」从"网页端没有的"列举中拿掉,改为表述"字典、备份、恢复、导出、设置仍在桌面端;更新两端可触发(桌面完整管理,网页端点击升级)"。
   - 其余词条一字不动;词汇表风格(定义 + _Avoid_)保持。
2. **docs/adr/0009-frontend-species-archive.md**(回填,文件从未写过但代码 src/lib/speciesProfiles.ts 头注释已引用该编号):决策=物种档案用前端静态档案包(构建期 glob 导入 src/data/species/*.json,一物种一文件、文件名=key),而不用数据库表。正文写实现状:饲养资料永不进数据库;窝表只存 species_key + 显示名快照;类型分组 distinct 动态得出;空字符串=查无实据显示「暂无资料」;内置 key 与自建 custom-N 命名空间隔离;v15 迁移别名归组与 species_key 校验在 Rust 侧镜像清单。日期按仓库存档现状(species-profile 批次,2026-09 下旬)。Consequences:加档案=加一份 JSON 零代码改动;文案修订(如本次人化)直接改 JSON。
3. **docs/adr/0010-webui-update-entry.md**(新增):决策=网页端开放"点击升级"入口,部分反转 ADR-0003 中"更新留桌面端"一点。Context:ADR-0003 当年以"更新等命令在浏览器里无意义或结果落在服务器侧"裁掉网页端更新;2026-09-30 用户反转——升级结果落在服务器侧正是要的行为(手机点、电脑升,升级对象就是同进程的内嵌服务本体)。决策内容:两个命令进网页端白名单(只读红点详情 + 触发既有确认流);令牌+受信网段两闸不变、不另加确认;升级前快照/禁写/防重入复用;桌面端更新管理(设置页 tab、手动检查、进度)仍桌面专属。Consequences:CONTEXT.md 网页端/桌面端词条同步;其余系统管理(字典/备份/恢复/导出/设置)边界不动。

## 验收标准

- [ ] CONTEXT.md 仅两词条改动,diff 干净
- [ ] ADR 0009 编号与 speciesProfiles.ts 头注释引用一致,内容与仓库存档现状相符
- [ ] ADR 0010 明确"部分反转 ADR-0003 的更新一条",其余裁剪不动
- [ ] 三份文件中文行文自然(说人话,不堆术语),体例与既有 ADR 一致

## Blocked by

无,可立即开始

## 涉及路径

- CONTEXT.md
- docs/adr/0009-frontend-species-archive.md(新建)
- docs/adr/0010-webui-update-entry.md(新建)

## 副作用声明

- 纯文档,不跑任何验证命令

## decision_refs

D1(边界反转)、D10(ADR 编号与回填)

## review_blocks

无
