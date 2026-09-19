import { beforeEach, describe, expect, it, vi } from "vitest";
import * as ipc from "./ipc";
import {
  WEBUI_TOKEN_KEY,
  createColony,
  getStats,
  isTauri,
  listColonies,
  subscribe,
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
      { id: 1, name: "大头一号", species: null, location_id: null, start_date: "2026-01-20", status: "active", days_raised: 1, actions: [], recent: [], hibernation: null },
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

  it("网络失败（fetch 抛错）原样上抛", async () => {
    vi.stubGlobal("fetch", vi.fn(async () => {
      throw new TypeError("Failed to fetch");
    }));

    await expect(listColonies()).rejects.toBeInstanceOf(TypeError);
  });

  it("subscribe（浏览器）：不碰 tauri listen，立即返回可调用的退订空实现（SSE 随票 05 接线）", async () => {
    const got = await subscribe("db-restored", () => {});
    expect(listenMock).not.toHaveBeenCalled();
    expect(typeof got).toBe("function");
    expect(() => got()).not.toThrow();
  });
});

describe("命令包装的导出面（测试 mock 工厂的路由依据）", () => {
  it("除 subscribe/isTauri 外的函数导出都挂 cmdName（按命令名路由给唯一 invokeMock）", () => {
    for (const [name, value] of Object.entries(ipc)) {
      if (name === "subscribe" || name === "isTauri") continue;
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
