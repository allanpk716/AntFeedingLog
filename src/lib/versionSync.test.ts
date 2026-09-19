import { beforeEach, describe, expect, it, vi } from "vitest";
import { reconcile, watchDataVersion, type VersionStamp } from "./versionSync";

// versionSync 只依赖 ipc 的 subscribe/isTauri：整模块替换，测对账与订阅编排。
const { subscribeMock, isTauriMock } = vi.hoisted(() => ({
  subscribeMock: vi.fn(),
  isTauriMock: vi.fn(),
}));
vi.mock("./ipc", () => ({
  subscribe: subscribeMock,
  isTauri: isTauriMock,
}));

const E1 = "install-1-1000";
const E2 = "install-1-9999"; // 同安装、重启后（epoch 变）

function stamp(version: number, epoch: string = E1): VersionStamp {
  return { epoch, version };
}

/** subscribe 桩：捕获 handler，手动派发帧；返回可控 unlisten。 */
function stubSubscribe(): { handlers: Array<(p: unknown) => void>; unlistens: Array<() => void> } {
  const handlers: Array<(p: unknown) => void> = [];
  const unlistens: Array<() => void> = [];
  subscribeMock.mockImplementation(async (_event: string, h: (p: unknown) => void) => {
    handlers.push(h);
    const unlisten = vi.fn();
    unlistens.push(unlisten);
    return unlisten;
  });
  return { handlers, unlistens };
}

beforeEach(() => {
  subscribeMock.mockReset();
  isTauriMock.mockReset();
});

describe("对账纯函数 reconcile（规格 C：同 epoch 比版本，epoch 不同无条件重拉）", () => {
  it("hello（SSE 首帧）+ 无缓存 = 建档不刷新（页面刚拉过数据）", () => {
    expect(reconcile(null, stamp(7), "hello")).toBe("none");
  });

  it("写事件 + 无缓存 = 刷新（桌面首事件晚于页面挂载，写入必是新数据）", () => {
    expect(reconcile(null, stamp(1), "write")).toBe("stale");
  });

  it("同 epoch 且本地版本落后 → stale（重拉当前页）", () => {
    expect(reconcile(stamp(3), stamp(4), "write")).toBe("stale");
    expect(reconcile(stamp(3), stamp(99), "write")).toBe("stale");
  });

  it("同 epoch 版本相同或更旧 → 不动（重复帧/乱序帧不触发重拉）", () => {
    expect(reconcile(stamp(4), stamp(4), "write")).toBe("none");
    expect(reconcile(stamp(4), stamp(3), "write")).toBe("none");
  });

  it("epoch 不同 → epoch-changed（电脑重启过，无条件整体重拉）", () => {
    expect(reconcile(stamp(9), stamp(0, E2), "write")).toBe("epoch-changed");
    expect(reconcile(stamp(9), stamp(0, E2), "hello")).toBe("epoch-changed");
    // 版本更大也照旧 epoch 优先
    expect(reconcile(stamp(9), stamp(100, E2), "write")).toBe("epoch-changed");
  });

  it("重连后的 hello 带更高版本（断线期间漏帧）→ 补拉", () => {
    expect(reconcile(stamp(3), stamp(5), "hello")).toBe("stale");
    expect(reconcile(stamp(5), stamp(5), "hello")).toBe("none");
  });
});

describe("watchDataVersion：桌面走 data-version 事件，浏览器走 SSE 帧", () => {
  it("桌面：subscribe('data-version')，payload 按写事件对账，落后即回调", async () => {
    isTauriMock.mockReturnValue(true);
    const { handlers } = stubSubscribe();
    const onStale = vi.fn();

    watchDataVersion(onStale);
    await Promise.resolve(); // subscribe 桩是 async
    expect(subscribeMock).toHaveBeenCalledWith("data-version", expect.any(Function));

    // 首个写事件：缓存为空 → 刷新
    handlers[0]?.({ epoch: E1, version: 1 });
    expect(onStale).toHaveBeenCalledTimes(1);
    // 同版本重复事件：不刷
    handlers[0]?.({ epoch: E1, version: 1 });
    expect(onStale).toHaveBeenCalledTimes(1);
    // 版本前进：刷
    handlers[0]?.({ epoch: E1, version: 2 });
    expect(onStale).toHaveBeenCalledTimes(2);
  });

  it("浏览器：hello 只建档不刷；version 落后才刷", async () => {
    isTauriMock.mockReturnValue(false);
    const { handlers } = stubSubscribe();
    const onStale = vi.fn();

    watchDataVersion(onStale);
    await Promise.resolve();
    expect(subscribeMock).toHaveBeenCalledWith("data-version", expect.any(Function));

    // hello 帧：建档，不刷（页面挂载时刚拉过）
    handlers[0]?.({ type: "hello", epoch: E1, version: 5 });
    expect(onStale).not.toHaveBeenCalled();
    // 同 epoch 落后 → 刷
    handlers[0]?.({ type: "version", epoch: E1, version: 6 });
    expect(onStale).toHaveBeenCalledTimes(1);
    // 同版本重复 → 不刷
    handlers[0]?.({ type: "version", epoch: E1, version: 6 });
    expect(onStale).toHaveBeenCalledTimes(1);
    // epoch 变化（电脑重启后重连的 hello）→ 无条件刷
    handlers[0]?.({ type: "hello", epoch: E2, version: 0 });
    expect(onStale).toHaveBeenCalledTimes(2);
  });

  it("退订：底层 unlisten 在 subscribe resolve 后被调；之后不再回调", async () => {
    isTauriMock.mockReturnValue(true);
    const { handlers, unlistens } = stubSubscribe();
    const onStale = vi.fn();

    const unwatch = watchDataVersion(onStale);
    await Promise.resolve(); // subscribe 桩 resolve
    unwatch();
    await Promise.resolve(); // 退订链（subscription.then → unlisten）执行
    expect(unlistens[0]).toHaveBeenCalledTimes(1);

    handlers[0]?.({ epoch: E1, version: 2 });
    expect(onStale).not.toHaveBeenCalled();
  });

  it("退订早于 subscribe resolve（挂载即卸载）：resolve 后仍会补退订", async () => {
    isTauriMock.mockReturnValue(true);
    // 用对象属性持有 resolve（TS 对闭包内赋值的窄化不跨作用域，直接用变量
    // 会被收窄成 never）
    const holder: { release?: (unlisten: () => void) => void } = {};
    const unlisten = vi.fn();
    subscribeMock.mockImplementation(
      () =>
        new Promise<() => void>((resolve) => {
          holder.release = resolve;
        }),
    );
    const onStale = vi.fn();

    const unwatch = watchDataVersion(onStale);
    unwatch(); // subscribe 还没 resolve
    holder.release?.(unlisten);
    await Promise.resolve();
    await Promise.resolve();
    expect(unlisten).toHaveBeenCalledTimes(1);
  });
});
