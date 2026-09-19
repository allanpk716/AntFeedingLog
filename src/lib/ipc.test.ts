import { beforeEach, afterEach, describe, expect, it, vi } from "vitest";
import * as ipc from "./ipc";
import {
  WEBUI_TOKEN_KEY,
  createColony,
  getStats,
  isTauri,
  listColonies,
  subscribe,
  type SseVersionFrame,
} from "./ipc";
import type { Colony } from "../types";

// 只有调用层模块允许碰 @tauri-apps/api/*：本文件 mock 这两个底层模块，
// 验证调用层自身的双模式路由（桌面透传 / 浏览器 fetch）。
const { invokeMock, listenMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  listenMock: vi.fn(),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));
vi.mock("@tauri-apps/api/event", () => ({ listen: listenMock }));

function asTauri(): void {
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
}

function asBrowser(): void {
  delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
}

/** fetch 桩：记录调用并按给定 status/body 应答 */
function stubFetch(status: number, body: string): ReturnType<typeof vi.fn> {
  const fetchMock = vi.fn(async () => ({
    ok: status >= 200 && status < 300,
    status,
    text: async () => body,
  }));
  vi.stubGlobal("fetch", fetchMock);
  return fetchMock;
}

beforeEach(() => {
  vi.unstubAllGlobals();
  asBrowser();
  localStorage.clear();
  invokeMock.mockReset();
  listenMock.mockReset();
});

describe("环境判定", () => {
  it("有 __TAURI_INTERNALS__ 即桌面，无则浏览器", () => {
    asTauri();
    expect(isTauri()).toBe(true);
    asBrowser();
    expect(isTauri()).toBe(false);
  });
});

describe("桌面路由（透传 Tauri invoke，命令名/参数/返回逐字等价）", () => {
  it("无参命令：invoke(命令名)，返回值原样透传", async () => {
    asTauri();
    const cols: Colony[] = [
      { id: 1, name: "大头一号", species: null, location_id: null, start_date: "2026-01-20", status: "active", days_raised: 1, actions: [], recent: [], hibernation: null, checkin: { latest: null, baseline_date: null, days_since_last: null } },
    ];
    invokeMock.mockResolvedValue(cols);

    await expect(listColonies()).resolves.toBe(cols);

    expect(invokeMock).toHaveBeenCalledTimes(1);
    expect(invokeMock).toHaveBeenCalledWith("list_colonies");
  });

  it("带参命令：invoke(命令名, 入参对象原样透传)", async () => {
    asTauri();
    invokeMock.mockResolvedValue(undefined);
    const input = {
      name: "新窝",
      species: null,
      location_id: null,
      start_date: "2026-09-19",
      status: "active" as const,
    };

    await createColony({ input });

    expect(invokeMock).toHaveBeenCalledTimes(1);
    expect(invokeMock).toHaveBeenCalledWith("create_colony", { input });
  });

  it("camelCase 入参键保持原样（冬眠/统计命令现状即此形状）", async () => {
    asTauri();
    invokeMock.mockResolvedValue(null);

    await getStats({ colonyId: 3, startDate: "2026-03-23", endDate: "2026-09-19" });

    expect(invokeMock).toHaveBeenCalledWith("get_stats", {
      colonyId: 3,
      startDate: "2026-03-23",
      endDate: "2026-09-19",
    });
  });

  it("命令失败：invoke 的 reject 原样上抛（组件层 String(e) 处理不变）", async () => {
    asTauri();
    invokeMock.mockRejectedValue("库已锁");

    await expect(listColonies()).rejects.toBe("库已锁");
  });

  it("subscribe：透传 tauri listen，事件对象拆包为 payload 再交处理器，退订函数透传", async () => {
    asTauri();
    const handlers: Array<(e: { payload: unknown }) => void> = [];
    const unlisten = () => {};
    listenMock.mockImplementation(async (_event: string, h: (e: { payload: unknown }) => void) => {
      handlers.push(h);
      return unlisten;
    });
    const seen: unknown[] = [];

    const got = await subscribe<{ downloaded: number }>("update-download-progress", (p) => seen.push(p));
    expect(listenMock).toHaveBeenCalledWith("update-download-progress", expect.any(Function));
    expect(got).toBe(unlisten);

    handlers[0]?.({ payload: { downloaded: 7 } });
    expect(seen).toEqual([{ downloaded: 7 }]);
  });
});

describe("浏览器路由（POST /api/cmd，Bearer 凭证）", () => {
  it("请求形状：POST /api/cmd + JSON 体 { cmd, args }（无参命令 args 为空对象）", async () => {
    const fetchMock = stubFetch(200, JSON.stringify([{ id: 1 }]));

    await listColonies();

    expect(fetchMock).toHaveBeenCalledTimes(1);
    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(url).toBe("/api/cmd");
    expect(init.method).toBe("POST");
    expect(init.headers).toMatchObject({ "Content-Type": "application/json" });
    expect(JSON.parse(init.body as string)).toEqual({ cmd: "list_colonies", args: {} });
  });

  it("带参命令：args 原样进 JSON 体；入参对象不被改写", async () => {
    const fetchMock = stubFetch(200, "null");
    const input = { name: "大头一号", species: null, location_id: 1, start_date: "2026-01-20", status: "active" as const };

    await createColony({ input });

    const [, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(JSON.parse(init.body as string)).toEqual({ cmd: "create_colony", args: { input } });
  });

  it("凭证：localStorage 有令牌时带 Authorization: Bearer", async () => {
    const fetchMock = stubFetch(200, "null");
    localStorage.setItem(WEBUI_TOKEN_KEY, "tok-abc123");

    await listColonies();

    const [, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(init.headers).toMatchObject({ Authorization: "Bearer tok-abc123" });
  });

  it("凭证：无令牌时不带 Authorization 头", async () => {
    const fetchMock = stubFetch(200, "null");

    await listColonies();

    const [, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    const headers = init.headers as Record<string, string>;
    expect(headers.Authorization).toBeUndefined();
  });

  it("返回体即命令返回值；空返回体归一为 null", async () => {
    stubFetch(200, JSON.stringify({ total: 0, rows: [] }));
    await expect(listColonies()).resolves.toEqual({ total: 0, rows: [] });

    stubFetch(200, "");
    await expect(listColonies()).resolves.toBeNull();
  });

  it("非 2xx：以响应体 { error } 的字符串 reject（与桌面 reject 字符串同形）", async () => {
    stubFetch(401, JSON.stringify({ error: "凭证无效或已重生成" }));

    await expect(listColonies()).rejects.toBe("凭证无效或已重生成");
  });

  it("非 2xx 且响应体不可解析：以 HTTP <status> reject，绝不静默成功", async () => {
    stubFetch(500, "<html>gateway error</html>");

    await expect(listColonies()).rejects.toBe("HTTP 500");
  });

  it("2xx 且响应体非空但 JSON 不可解析：以 HTTP <status> 不可解析响应 reject（票 05 评审 Minor：不静默归 null）", async () => {
    stubFetch(200, "<html>proxy injected</html>");

    await expect(listColonies()).rejects.toBe("HTTP 200 不可解析响应");
  });

  it("网络失败（fetch 抛错）原样上抛", async () => {
    vi.stubGlobal("fetch", vi.fn(async () => {
      throw new TypeError("Failed to fetch");
    }));

    await expect(listColonies()).rejects.toBeInstanceOf(TypeError);
  });

  it("subscribe（浏览器）：无源事件返回可调用的退订空实现（update-download-progress 等桌面专属）", async () => {
    const got = await subscribe("update-download-progress", () => {});
    expect(listenMock).not.toHaveBeenCalled();
    expect(typeof got).toBe("function");
    expect(() => got()).not.toThrow();
  });
});

// ── 票 06：浏览器 SSE（data-version → /api/sse，一次性票据 + 退避重连）──────

/** EventSource 桩：捕获实例，测试手动 emit 帧 / 触发 error。 */
class FakeEventSource {
  static instances: FakeEventSource[] = [];
  url: string;
  onopen: (() => void) | null = null;
  onmessage: ((ev: { data: string }) => void) | null = null;
  onerror: ((ev: unknown) => void) | null = null;
  closed = false;
  constructor(url: string | URL) {
    this.url = String(url);
    FakeEventSource.instances.push(this);
  }
  close(): void {
    this.closed = true;
  }
  emit(frame: unknown): void {
    this.onmessage?.({ data: JSON.stringify(frame) });
  }
  fail(): void {
    this.onerror?.(new Error("connection lost"));
  }
}

/** 票据端点桩：POST /api/sse-ticket 恒回同一张票。 */
function stubSseBackend(ticket: string): ReturnType<typeof vi.fn> {
  const fetchMock = vi.fn(async () => ({
    ok: true,
    status: 200,
    json: async () => ({ ticket, expires_in: 60 }),
  }));
  vi.stubGlobal("fetch", fetchMock);
  vi.stubGlobal("EventSource", FakeEventSource);
  return fetchMock;
}

describe("浏览器 SSE（data-version 订阅，票 06）", () => {
  beforeEach(() => {
    FakeEventSource.instances = [];
    ipc.resetBrowserSseForTests(); // SSE 单例是模块级状态：跨测试复位
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("首订：先 POST /api/sse-ticket 换票（带 Bearer），再连 /api/sse?ticket=；帧原样透传", async () => {
    vi.useFakeTimers();
    localStorage.setItem(WEBUI_TOKEN_KEY, "tok-abc");
    const fetchMock = stubSseBackend("tick-1");
    const seen: SseVersionFrame[] = [];

    await subscribe<SseVersionFrame>("data-version", (p) => seen.push(p));
    await vi.advanceTimersByTimeAsync(0); // 冲洗微任务：取票 + 建连

    expect(FakeEventSource.instances.length).toBe(1);
    expect(fetchMock).toHaveBeenCalledTimes(1);
    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(url).toBe("/api/sse-ticket");
    expect(init.method).toBe("POST");
    expect((init.headers as Record<string, string>).Authorization).toBe("Bearer tok-abc");

    const es = FakeEventSource.instances[0];
    expect(es.url).toBe("/api/sse?ticket=tick-1");
    expect(es.closed).toBe(false);

    es.emit({ type: "hello", epoch: "e1", version: 3 });
    es.emit({ type: "version", epoch: "e1", version: 4 });
    expect(seen).toEqual([
      { type: "hello", epoch: "e1", version: 3 },
      { type: "version", epoch: "e1", version: 4 },
    ]);
  });

  it("错误即 close 关掉原生自动重连（票据一次性，原生重连只会 401），按 1s/3s/10s 退避重取票据重建连", async () => {
    vi.useFakeTimers();
    const fetchMock = stubSseBackend("tick-x");
    await subscribe("data-version", () => {});
    await vi.advanceTimersByTimeAsync(0);
    expect(FakeEventSource.instances.length).toBe(1);

    // 第 1 次 error → close + 1 秒后重连
    let es = FakeEventSource.instances[0];
    es.fail();
    // onerror 必须 close，关掉 EventSource 原生自动重连（否则拿旧票撞 401）
    expect(es.closed).toBe(true);
    await vi.advanceTimersByTimeAsync(999);
    expect(FakeEventSource.instances.length).toBe(1); // 1 秒未到不重连
    await vi.advanceTimersByTimeAsync(1);
    expect(FakeEventSource.instances.length).toBe(2); // 退避 1 秒后重取票据重建连
    expect(fetchMock).toHaveBeenCalledTimes(2); // 重连先重取票据
    expect(FakeEventSource.instances[1].url).toBe("/api/sse?ticket=tick-x");

    // 第 2 次 → 3 秒
    es = FakeEventSource.instances[1];
    es.fail();
    await vi.advanceTimersByTimeAsync(2999);
    expect(FakeEventSource.instances.length).toBe(2);
    await vi.advanceTimersByTimeAsync(1);
    expect(FakeEventSource.instances.length).toBe(3);

    // 第 3 次 → 10 秒
    es = FakeEventSource.instances[2];
    es.fail();
    await vi.advanceTimersByTimeAsync(9999);
    expect(FakeEventSource.instances.length).toBe(3);
    await vi.advanceTimersByTimeAsync(1);
    expect(FakeEventSource.instances.length).toBe(4);

    // 封顶：之后每次仍 10 秒（不无限增长）
    es = FakeEventSource.instances[3];
    es.fail();
    await vi.advanceTimersByTimeAsync(10_000);
    expect(FakeEventSource.instances.length).toBe(5);
  });

  it("连上过一次（onopen）后退避归零：断线重连从 1 秒起步", async () => {
    vi.useFakeTimers();
    stubSseBackend("tick-x");
    await subscribe("data-version", () => {});
    await vi.advanceTimersByTimeAsync(0);
    const es1 = FakeEventSource.instances[0];
    es1.onopen?.(); // 连上过：重试计数清零
    es1.fail();
    await vi.advanceTimersByTimeAsync(1000);
    expect(FakeEventSource.instances.length).toBe(2);
  });

  it("取票失败（服务不可达）不建连，同样按退避重试", async () => {
    vi.useFakeTimers();
    const fetchMock = vi.fn(async () => {
      throw new TypeError("Failed to fetch");
    });
    vi.stubGlobal("fetch", fetchMock);
    vi.stubGlobal("EventSource", FakeEventSource);

    await subscribe("data-version", () => {});
    await vi.advanceTimersByTimeAsync(0);
    expect(FakeEventSource.instances.length).toBe(0); // 没票不建连
    await vi.advanceTimersByTimeAsync(1000);
    await vi.advanceTimersByTimeAsync(3000);
    expect(fetchMock).toHaveBeenCalledTimes(3); // 1 秒 + 3 秒两次重试
  });

  it("畸形帧（非 JSON / 缺字段）忽略不炸，不影响后续帧", async () => {
    vi.useFakeTimers();
    stubSseBackend("tick-1");
    const seen: SseVersionFrame[] = [];
    await subscribe<SseVersionFrame>("data-version", (p) => seen.push(p));
    await vi.advanceTimersByTimeAsync(0);
    const es = FakeEventSource.instances[0];
    es.onmessage?.({ data: "不是 JSON{{{" });
    es.onmessage?.({ data: JSON.stringify({ type: "bogus" }) });
    es.emit({ type: "version", epoch: "e1", version: 9 });
    expect(seen).toEqual([{ type: "version", epoch: "e1", version: 9 }]);
  });

  it("退订：最后一个处理器退订后断开 SSE，之后 error 不再触发重连", async () => {
    vi.useFakeTimers();
    stubSseBackend("tick-x");
    const unlisten = await subscribe("data-version", () => {});
    await vi.advanceTimersByTimeAsync(0);
    unlisten();
    const es = FakeEventSource.instances[0];
    expect(es.closed).toBe(true); // 退订即断开连接
    es.fail(); // 退订后的 error 是残余事件，不得触发重连
    await vi.advanceTimersByTimeAsync(60_000);
    expect(FakeEventSource.instances.length).toBe(1);
  });

  it("多个订阅者共享同一条 SSE 连接；全部退订才断开", async () => {
    vi.useFakeTimers();
    stubSseBackend("tick-x");
    const un1 = await subscribe("data-version", () => {});
    const un2 = await subscribe("data-version", () => {});
    await vi.advanceTimersByTimeAsync(0);
    expect(FakeEventSource.instances.length).toBe(1); // 共享单连接
    un1();
    expect(FakeEventSource.instances[0].closed).toBe(false); // 还有订阅者在
    un2();
    expect(FakeEventSource.instances[0].closed).toBe(true); // 全部退订才断开
  });

  it("db-restored 浏览器侧无独立源（恢复经版本帧刷新，见 versionSync）", async () => {
    vi.useFakeTimers();
    const fetchMock = stubSseBackend("tick-x");
    const unlisten = await subscribe("db-restored", () => {});
    await vi.advanceTimersByTimeAsync(0);
    expect(fetchMock).not.toHaveBeenCalled();
    expect(FakeEventSource.instances.length).toBe(0);
    expect(() => unlisten()).not.toThrow();
  });
});

describe("命令包装的导出面（测试 mock 工厂的路由依据）", () => {
  it("除 subscribe/isTauri 外的函数导出都挂 cmdName（按命令名路由给唯一 invokeMock）", () => {
    for (const [name, value] of Object.entries(ipc)) {
      // subscribe/isTauri/resetBrowserSseForTests 是环境/测试设施，不是命令包装
      if (name === "subscribe" || name === "isTauri" || name === "resetBrowserSseForTests") {
        continue;
      }
      if (typeof value !== "function") continue; // WEBUI_TOKEN_KEY 等常量
      expect((value as { cmdName?: unknown }).cmdName, `export ${name}`).toEqual(expect.any(String));
    }
  });

  it("cmdName 与命令包装一一对应（抽查既有直调命令名不漂移）", () => {
    expect(listColonies.cmdName).toBe("list_colonies");
    expect(createColony.cmdName).toBe("create_colony");
    expect(getStats.cmdName).toBe("get_stats");
  });
});
