import { createApp } from "vue";
import App from "./App.vue";
import { logFrontendError } from "./lib/ipc";
import { captureTokenFromHash } from "./lib/webuiEntry";
import { installContextMenuShield } from "./lib/shell";

// 网页端进门凭证入口（webui-checkin 票 05，规格 B）：挂载前先把 #token= 落进
// localStorage——组件挂载后立刻会发首批请求，凭证必须先就位；顺路清净地址栏。
// 桌面（Tauri）没有 fragment，本调用原样空转。
captureTokenFromHash();

// 前端未捕获异常转发 Rust 落日志（数据安全二期票 01 D7）：只记不打断，
// 应用照常运行；转发失败（如日志底座未就绪）静默吞掉，绝不二次抛错。
function reportFrontendError(reason: unknown): void {
  const text =
    reason instanceof Error
      ? `${reason.name}: ${reason.message}${reason.stack ? ` | ${reason.stack.split("\n")[1]?.trim() ?? ""}` : ""}`
      : String(reason);
  logFrontendError({ message: text }).catch(() => {});
}

window.addEventListener("error", (e) => {
  reportFrontendError(e.error ?? e.message ?? "未知脚本错误");
});
window.addEventListener("unhandledrejection", (e) => {
  reportFrontendError(e.reason ?? "未处理的 Promise 拒绝");
});

// 交互第三轮 #2：屏蔽 web 原生右键菜单（输入框内保留）
installContextMenuShield();

createApp(App).mount("#app");
