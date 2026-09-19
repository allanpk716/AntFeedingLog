import { beforeEach, describe, expect, it, vi } from "vitest";
import { captureTokenFromHash, parseTokenHash } from "./webuiEntry";
import { WEBUI_TOKEN_KEY } from "./ipc";

// 网页端进门凭证入口（webui-checkin 票 05，规格 B）：完整地址形如
// `/#token=<凭证>`，页面加载时把 fragment 里的凭证存 localStorage 并
// replaceState 清净地址。fragment 不随请求发给服务器，凭证只经此路径落地。

describe("parseTokenHash（纯核：hash 解析与清净地址拼装）", () => {
  it("无 hash / 纯 # / 无 token 参数 → null", () => {
    expect(parseTokenHash("", "/", "")).toBeNull();
    expect(parseTokenHash("#", "/", "")).toBeNull();
    expect(parseTokenHash("#foo=1", "/", "")).toBeNull();
  });

  it("#token=xxx → 取出 token，清净地址 = pathname + search", () => {
    const got = parseTokenHash("#token=abc123", "/", "");
    expect(got).toEqual({ token: "abc123", cleanedUrl: "/" });
  });

  it("带查询串与子路径时清净地址保留它们", () => {
    const got = parseTokenHash("#token=abc123", "/app/index.html", "?v=2");
    expect(got?.cleanedUrl).toBe("/app/index.html?v=2");
  });

  it("token 之外的其余参数保留在 hash 里（只剥 token）", () => {
    const got = parseTokenHash("#tab=history&token=abc123", "/", "");
    expect(got?.token).toBe("abc123");
    expect(got?.cleanedUrl).toBe("/#tab=history");
  });

  it("token 空值视为未携带（不清洗地址、不落存储）", () => {
    expect(parseTokenHash("#token=", "/", "")).toBeNull();
  });
});

describe("captureTokenFromHash（页面入口：localStorage + replaceState）", () => {
  beforeEach(() => {
    localStorage.clear();
    window.location.hash = "";
  });

  it("有 token：写入 localStorage 并 replaceState 清净地址，返回 token", () => {
    const replace = vi.spyOn(window.history, "replaceState");
    window.location.hash = "#token=tok-xyz789";

    const token = captureTokenFromHash();

    expect(token).toBe("tok-xyz789");
    expect(localStorage.getItem(WEBUI_TOKEN_KEY)).toBe("tok-xyz789");
    expect(replace).toHaveBeenCalledWith(null, "", "/");
  });

  it("无 token：不写 localStorage 也不碰历史", () => {
    const replace = vi.spyOn(window.history, "replaceState");
    window.location.hash = "#tab=home";

    expect(captureTokenFromHash()).toBeNull();
    expect(localStorage.getItem(WEBUI_TOKEN_KEY)).toBeNull();
    expect(replace).not.toHaveBeenCalled();
  });

  it("先落地凭证再清净地址（清净后 localStorage 里仍有）", () => {
    window.location.hash = "#token=tok-keep";
    captureTokenFromHash();
    expect(localStorage.getItem(WEBUI_TOKEN_KEY)).toBe("tok-keep");
    expect(window.location.hash).not.toContain("tok-keep");
  });
});
