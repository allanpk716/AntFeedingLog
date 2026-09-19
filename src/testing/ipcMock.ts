import { vi } from "vitest";

/**
 * 组件测试统一 mock「调用层」的 vi.mock 工厂（webui-checkin 票 01）：
 * 替代旧式逐文件 `vi.mock("@tauri-apps/api/core")` + `vi.mock("@tauri-apps/api/event")`。
 *
 * - 每个命令包装按其 `cmdName` 透传给唯一的 `invokeMock`，调用形状
 *   `(命令名, 入参对象)` 与旧 mock invoke 完全一致——既有断言、
 *   `mockResolvedValueOnce` 队列、`mock.calls.some(([cmd]) => ...)` 全部零改写；
 * - `subscribe` 默认不透传（避免测试真的挂监听）：不传 overrides 时用
 *   立即退订的空实现桩，需要控制事件派发时经 overrides 注入自定桩
 *   （见 UpdatePanel.test.ts）；
 * - 其余导出（常量、isTauri）原样透传。
 *
 * 用法（测试文件内）：
 * ```ts
 * const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
 * vi.mock("../lib/ipc", async (importOriginal) => {
 *   const { ipcModuleMock } = await import("../testing/ipcMock");
 *   return ipcModuleMock(invokeMock)(importOriginal);
 * });
 * ```
 */
export function ipcModuleMock(
  invokeMock: (...args: unknown[]) => unknown,
  overrides: Record<string, unknown> = {},
) {
  return async (importOriginal: () => Promise<Record<string, unknown>>) => {
    const actual = await importOriginal();
    const mocked: Record<string, unknown> = {
      subscribe: vi.fn(async () => () => {}),
      ...overrides,
    };
    for (const [name, value] of Object.entries(actual)) {
      if (name in mocked) continue;
      const cmdName = (value as { cmdName?: unknown }).cmdName;
      if (typeof value === "function" && typeof cmdName === "string") {
        // 包装自身携带命令名（组件不再传）：还原旧 mock 的 (命令名, 入参) 调用形状
        mocked[name] = vi.fn((...args: unknown[]) => invokeMock(cmdName, ...args));
      } else {
        mocked[name] = value;
      }
    }
    return mocked;
  };
}
