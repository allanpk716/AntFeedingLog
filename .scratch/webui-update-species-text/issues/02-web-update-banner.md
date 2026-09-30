# 票 02 · 网页端前端:升级横幅 + 确认面板 + 升级状态机

## What to build

浏览器模式(网页端)专属的"有新版→点击升级"前端(桌面端 Tauri 窗口**不渲染**这套 UI):

1. **ipc 命令包装**:在 src/lib/ipc.ts 按既有 CommandFn 模式包装票 01 的两个命令(只读详情查询、升级触发)。
2. **纯逻辑模块 src/lib/webUpdaterUi.ts**(+ 同名 .test.ts):网页端升级状态机与文案——
   - 状态:idle(无新版)/ banner(有新版待点)/ panel(确认面板)/ busy(确认中·命令在途)/ waiting-restart("正在下载升级包,应用将自动重启,本页会短暂断开——恢复后自动刷新")/ restarting("正在重启中…")/ failed(红条+原因+重试)/ restart-timeout("升级可能未完成,请到电脑端确认,或稍后刷新本页")
   - 出口1:install_failed 响应在 HTTP 生命周期内返回 → waiting-restart/restarting 切回 failed,重试可用
   - 出口2:确认升级起 **10 分钟**内 SSE 未重连成功 → restarting 切 restart-timeout,不再转圈,页面可手动刷新(重试=手动刷新,不出按钮)
   - 出口3:Err 含"远端已没有比当前更新的版本" → failed 红条(该文案)**不弹重试按钮**,收横幅、重读红点
   - 出口4:SSE 重连成功页面自动刷新后:读 sessionStorage 记录的目标版本,与当前 bundle 版本(package.json)比对——相等弹「已升级到 vX」成功 toast 并清记录;不相等(旧版本回来了)不弹、清记录
   - 防重入:Err 含"已有安装流程正在进行" → 平静提示"安装正在进行中,请稍候"(复用 updaterUi.ts 既有 isInstallInProgressError/installInProgressText 口径,**import 复用,不复制不改它**)
   - 其余检查/下载/安装失败:failed 红条带原因 + 重试按钮;文案口径对齐 updaterUi.ts installFailedText 风格
3. **组件 src/components/WebUpdateBanner.vue**(+ 同名 .test.ts):外壳层横幅+确认面板。挂载时(仅浏览器模式)调一次只读命令;available=true 才显示「有新版本 vY → 点击升级」;× 关闭后**本次会话**不再出现(组件内 ref 会话态,不落 localStorage);点开确认面板:当前 vX(取 package.json,不用命令)→ vY + 更新说明(notes=null 省略说明区)+「确认升级」;确认后进状态机;失败/成功 toast 走全局 toast 模块(showError/showSuccess),异步读取期 LoadingHint 合规(横幅首查数据未到前不渲染横幅,确认面板打开时不闪默认值)。
4. **App.vue 外壳挂载**:顶栏下挂 `<WebUpdateBanner v-if="!isTauri()" />`(浏览器模式才渲染;App.vue 已 import isTauri)。

**边界与冲突约定(必须遵守)**:
- **不许改 src/App.test.ts**——该文件归票 03 独占。若挂横幅导致 App.test.ts 断言失败,不修它,回报 DONE_WITH_CONCERNS 写明失败断言,由协调者处理。
- 不改 updaterUi.ts / versionSync.ts / toast 模块,只 import。

## 验收标准

- [ ] npx vitest run(票内涉及测试文件)全绿;vue-tsc 0 错误
- [ ] 状态机单测覆盖四条路径:install_failed 切回 failed / 10 分钟超时切 restart-timeout / 复查无新版收横幅不弹重试 / sessionStorage 比对弹成功提示(含旧版本回来不弹)
- [ ] 防重入提示复用既有纯函数口径,有测试
- [ ] 横幅会话级关闭:关后同会话重查不复活;组件测试验证
- [ ] 桌面模式(isTauri=true)不渲染横幅,组件测试验证
- [ ] toast/loading 全部走全局模块,无自造样式

## Blocked by

票 01(两个 ipc 命令的名字与返回形状以其实现为准)

## 涉及路径

- src/lib/ipc.ts
- src/lib/ipc.test.ts
- src/lib/webUpdaterUi.ts
- src/lib/webUpdaterUi.test.ts
- src/components/WebUpdateBanner.vue
- src/components/WebUpdateBanner.test.ts
- src/App.vue

## 副作用声明

- 局部验证:`npx vitest run src/lib/webUpdaterUi.test.ts src/components/WebUpdateBanner.test.ts src/lib/ipc.test.ts`;`npx vue-tsc --noEmit`
- 不跑 cargo;不动 src/App.test.ts 与 src/data/

## decision_refs

D2(有新版才现身/会话级关闭)、D3(简单版过程体验/无百分比)、D7(失败兜底)、D8(横幅可关)、F3(两条占位出口)、F4(sessionStorage 承载)、F7(超时=10 分钟)、F8(复查无新版不弹重试)、F10(notes=null 省略说明区)

## review_blocks

无
