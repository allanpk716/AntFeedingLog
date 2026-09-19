/**
 * 数据版本对账与订阅（webui-checkin 票 06，规格 Implementation Decisions C）。
 *
 * 任一端记录、另一端开着的页面自动刷新（User Story 12/15）：
 * - 桌面：后端写后钩子链广播 `data-version` 事件（负载 {epoch, version}）；
 * - 浏览器：`/api/sse` 推 hello / version 帧（ipc.subscribe 的 data-version
 *   事件，帧对象原样透传）。
 *
 * 两端同一条对账规则（reconcile 纯函数）：
 * - **epoch 不同 → epoch-changed**：epoch = 安装 id + 进程启动毫秒，变化即电脑
 *   重启过 → 无条件整体重拉（覆盖「手机长开 + 电脑重启」）；
 * - **同 epoch 本地版本落后 → stale**：重拉当前页；
 * - hello（SSE 首帧）紧跟页面加载、数据刚拉过 → 只建档不刷新；重连后的 hello
 *   带更高版本（断线期间漏帧）→ 补拉；
 * - 恢复完成：桌面端有 db-restored 事件（无条件刷新，语义不变）；网页端由服务
 *   端在恢复后把版本抬到「已广播最大值 + 1」再广播 version 帧 → 对账必判落后
 *   → 无条件刷新（版本号跨恢复单调，见 data_version.rs bump_floor）。
 */
import { subscribe, isTauri, type UnlistenFn } from "./ipc";
import type { SseVersionFrame } from "./ipc";

/** 一份版本戳：实例 epoch + 数据版本号（与后端 VersionFrame 同形）。 */
export interface VersionStamp {
  epoch: string;
  version: number;
}

/** 帧性质：hello = SSE 首帧（建档/补拉基线），write = 真实写事件。 */
export type FrameKind = "hello" | "write";

/** 对账裁决：stale = 重拉当前页；epoch-changed = 无条件整体重拉；none = 不动。 */
export type ReconcileVerdict = "stale" | "epoch-changed" | "none";

/**
 * 对账纯函数（可测）：本地缓存（null = 刚挂载还没见过任何帧） vs 来帧。
 * 规格口径「同 epoch 且版本落后才判旧；epoch 不同一律整体重拉」。
 */
export function reconcile(
  local: VersionStamp | null,
  incoming: VersionStamp,
  kind: FrameKind,
): ReconcileVerdict {
  if (local !== null && local.epoch !== incoming.epoch) {
    return "epoch-changed";
  }
  if (kind === "hello" && local === null) {
    // SSE 首帧紧跟页面加载：数据刚拉过，只建档
    return "none";
  }
  // 写事件 + 无缓存 = 首个写事件晚于页面挂载的初次拉取 → 必是新数据，刷新
  return incoming.version > (local?.version ?? -1) ? "stale" : "none";
}

/**
 * 订阅数据版本广播并在数据落后时回调 onStale（调用方接「当前页重拉」）。
 * 返回退订函数（可安全地在 subscribe resolve 前调用——resolve 后补退订）。
 * 每个调用方一份独立本地缓存；浏览器下多个调用方共享同一条 SSE 连接
 * （ipc.ts 单例 EventSource，按 handler 计数管理生命周期）。
 */
export function watchDataVersion(onStale: () => void): UnlistenFn {
  let cache: VersionStamp | null = null;
  let stopped = false;

  const handle = (kind: FrameKind, stamp: VersionStamp) => {
    if (stopped) return;
    const verdict = reconcile(cache, stamp, kind);
    cache = stamp; // 对账后更新本地缓存（乱序/重复帧自然被下一次对账吸收）
    if (verdict !== "none") {
      onStale();
    }
  };

  const subscription = isTauri()
    ? subscribe<VersionStamp>("data-version", (payload) => {
        handle("write", {
          epoch: String(payload?.epoch ?? ""),
          version: Number(payload?.version ?? 0),
        });
      })
    : subscribe<SseVersionFrame>("data-version", (frame) => {
        handle(frame?.type === "hello" ? "hello" : "write", {
          epoch: String(frame?.epoch ?? ""),
          version: Number(frame?.version ?? 0),
        });
      });

  return () => {
    stopped = true;
    void subscription.then((unlisten) => unlisten());
  };
}
