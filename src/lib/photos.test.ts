import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  MAX_PHOTOS_PER_SUBMIT,
  MAX_PHOTO_BYTES,
  formatBytes,
  joinPhotoPath,
  loadPhotoBlobUrl,
  photoSrc,
  revokeObjectUrl,
  uploadPhotosHttp,
} from "./photos";
import { WEBUI_TOKEN_KEY } from "./ipc";

describe("photos（webui-checkin 票 07）", () => {
  describe("joinPhotoPath", () => {
    it("根目录尾部分隔符归一，统一正斜杠连接", () => {
      expect(
        joinPhotoPath("C:/Users/x/AppData/Roaming/com.antfeedinglog.app/photos", "1/abc.jpg"),
      ).toBe("C:/Users/x/AppData/Roaming/com.antfeedinglog.app/photos/1/abc.jpg");
      // Windows 反斜杠根目录 + 正斜杠相对路径：混合分隔符 Windows API 可解析
      expect(joinPhotoPath("C:\\data\\photos\\", "1/abc.jpg")).toBe("C:\\data\\photos/1/abc.jpg");
    });

    it("相对路径带前导斜杠不重复", () => {
      expect(joinPhotoPath("/home/x/photos/", "/1/abc.jpg")).toBe("/home/x/photos/1/abc.jpg");
    });
  });

  describe("formatBytes", () => {
    it("按 1024 进制分档，KB/MB 一位小数", () => {
      expect(formatBytes(0)).toBe("0 B");
      expect(formatBytes(512)).toBe("512 B");
      expect(formatBytes(1023)).toBe("1023 B");
      expect(formatBytes(2048)).toBe("2.0 KB");
      expect(formatBytes(15 * 1024 * 1024)).toBe("15.0 MB");
      expect(formatBytes(1536 * 1024 * 1024)).toBe("1.50 GB");
    });
  });

  describe("photoSrc", () => {
    afterEach(() => {
      delete (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
    });

    it("浏览器环境返回空串（网页端照片读取走 loadPhotoBlobUrl 的 blob 流）", () => {
      expect(photoSrc("1/a.jpg", "C:/data/photos")).toBe("");
    });

    it("桌面环境走 asset 协议（convertFileSrc），路径 = 根目录 + 相对路径", () => {
      (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = {
        convertFileSrc: (p: string) => `http://asset.localhost/${encodeURIComponent(p)}`,
      };
      const src = photoSrc("1/abc.jpg", "C:\\data\\photos");
      expect(src).toBe(
        `http://asset.localhost/${encodeURIComponent("C:\\data\\photos/1/abc.jpg")}`,
      );
    });

    it("根目录未就绪（空串）返回空串，调用方渲染占位", () => {
      (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = {
        convertFileSrc: (p: string) => p,
      };
      expect(photoSrc("1/a.jpg", "")).toBe("");
      expect(photoSrc("", "C:/data/photos")).toBe("");
    });
  });
});

// ── 网页端照片通路（webui-checkin 票 08）：blob 取图 + multipart 上传 ──────

describe("photos 网页端（webui-checkin 票 08）", () => {
  let createObjectURL: ReturnType<typeof vi.fn>;
  let revokeObjectURL: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    localStorage.clear();
    // happy-dom 不实现 objectURL：测试注入桩记录调用
    createObjectURL = vi.fn(() => "blob:mock-url");
    revokeObjectURL = vi.fn();
    Object.defineProperty(URL, "createObjectURL", {
      value: createObjectURL,
      configurable: true,
      writable: true,
    });
    Object.defineProperty(URL, "revokeObjectURL", {
      value: revokeObjectURL,
      configurable: true,
      writable: true,
    });
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    localStorage.clear();
  });

  it("上限常量与 Rust 纯核同口径（15MB/张、9 张/次）", () => {
    expect(MAX_PHOTO_BYTES).toBe(15 * 1024 * 1024);
    expect(MAX_PHOTOS_PER_SUBMIT).toBe(9);
  });

  describe("loadPhotoBlobUrl", () => {
    it("fetch /api/photo/<relPath> 带 Bearer 头，blob 转 objectURL（token 不进 URL）", async () => {
      localStorage.setItem(WEBUI_TOKEN_KEY, "tok-p");
      const blob = new Blob(["jpeg-bytes"], { type: "image/jpeg" });
      const fetchMock = vi.fn(async () => ({ ok: true, status: 200, blob: async () => blob }));
      vi.stubGlobal("fetch", fetchMock);

      const url = await loadPhotoBlobUrl("1/abc.jpg");

      expect(fetchMock).toHaveBeenCalledTimes(1);
      const [reqUrl, init] = fetchMock.mock.calls[0] as unknown as [string, RequestInit];
      expect(reqUrl).toBe("/api/photo/1/abc.jpg");
      expect(init.headers).toEqual({ Authorization: "Bearer tok-p" });
      expect(reqUrl).not.toContain("tok-p");
      expect(createObjectURL).toHaveBeenCalledWith(blob);
      expect(url).toBe("blob:mock-url");
    });

    it("无令牌时不带 Authorization 头", async () => {
      const fetchMock = vi.fn(async () => ({
        ok: true,
        status: 200,
        blob: async () => new Blob(["x"]),
      }));
      vi.stubGlobal("fetch", fetchMock);

      await loadPhotoBlobUrl("1/abc.jpg");

      const [, init] = fetchMock.mock.calls[0] as unknown as [string, RequestInit];
      expect(init.headers).toEqual({});
    });

    it("非 2xx 以响应体 { error } 人话 reject（缺图 404 同路）", async () => {
      vi.stubGlobal(
        "fetch",
        vi.fn(async () => ({
          ok: false,
          status: 404,
          json: async () => ({ error: "照片文件缺失（可能恢复过旧备份）" }),
        })),
      );
      await expect(loadPhotoBlobUrl("1/gone.jpg")).rejects.toBe(
        "照片文件缺失（可能恢复过旧备份）",
      );
    });

    it("非 2xx 且体不可解析：以 HTTP <status> reject", async () => {
      vi.stubGlobal(
        "fetch",
        vi.fn(async () => ({
          ok: false,
          status: 500,
          json: async () => {
            throw new Error("not json");
          },
        })),
      );
      await expect(loadPhotoBlobUrl("1/x.jpg")).rejects.toBe("HTTP 500");
    });
  });

  describe("revokeObjectUrl", () => {
    it("只释放 blob: 前缀，桌面 asset URL 原样放过", () => {
      revokeObjectUrl("blob:mock-url");
      expect(revokeObjectURL).toHaveBeenCalledWith("blob:mock-url");
      revokeObjectUrl("http://asset.localhost/C:%5Cdata%5Cphotos");
      revokeObjectUrl("");
      expect(revokeObjectURL).toHaveBeenCalledTimes(1);
    });
  });

  describe("uploadPhotosHttp", () => {
    it("张数超 9 本地拒绝不发请求（413 半路断连的体验前置）", async () => {
      const fetchMock = vi.fn();
      vi.stubGlobal("fetch", fetchMock);
      const files = Array.from({ length: 10 }, (_, i) => new File(["x"], `${i}.jpg`));

      await expect(uploadPhotosHttp(7, files)).rejects.toBe("一次最多上传 9 张照片");
      expect(fetchMock).not.toHaveBeenCalled();
    });

    it("单张超 15MB 本地拒绝，错误带文件名，不发请求", async () => {
      const fetchMock = vi.fn();
      vi.stubGlobal("fetch", fetchMock);
      const big = new File(["x"], "big.jpg");
      Object.defineProperty(big, "size", { value: MAX_PHOTO_BYTES + 1 });

      await expect(uploadPhotosHttp(7, [big])).rejects.toContain("big.jpg");
      expect(fetchMock).not.toHaveBeenCalled();
    });

    it("空文件列表直接空数组返回（不发请求）", async () => {
      const fetchMock = vi.fn();
      vi.stubGlobal("fetch", fetchMock);
      await expect(uploadPhotosHttp(7, [])).resolves.toEqual([]);
      expect(fetchMock).not.toHaveBeenCalled();
    });

    it("FormData 带 checkinId 文本段与 photos 文件段，Bearer 头照带", async () => {
      localStorage.setItem(WEBUI_TOKEN_KEY, "tok-up");
      const fetchMock = vi.fn(async () => ({ ok: true, status: 200, json: async () => [] }));
      vi.stubGlobal("fetch", fetchMock);
      const files = [new File(["a"], "a.jpg"), new File(["b"], "b.png")];

      const out = await uploadPhotosHttp(7, files);

      expect(out).toEqual([]);
      const [url, init] = fetchMock.mock.calls[0] as unknown as [string, RequestInit];
      expect(url).toBe("/api/photos");
      expect(init.method).toBe("POST");
      expect((init.headers as Record<string, string>).Authorization).toBe("Bearer tok-up");
      const form = init.body as FormData;
      expect(form.get("checkinId")).toBe("7");
      expect(form.getAll("photos")).toHaveLength(2);
    });

    it("非 2xx 以响应体 { error } 人话 reject（服务端 400 兜底同形）", async () => {
      vi.stubGlobal(
        "fetch",
        vi.fn(async () => ({
          ok: false,
          status: 400,
          json: async () => ({ error: "一次最多上传 9 张照片" }),
        })),
      );
      await expect(uploadPhotosHttp(7, [new File(["x"], "a.jpg")])).rejects.toBe(
        "一次最多上传 9 张照片",
      );
    });

    it("2xx 体不是数组：明确报错，不把垃圾当照片清单", async () => {
      vi.stubGlobal(
        "fetch",
        vi.fn(async () => ({ ok: true, status: 200, json: async () => ({ surprise: 1 }) })),
      );
      await expect(uploadPhotosHttp(7, [new File(["x"], "a.jpg")])).rejects.toBe(
        "HTTP 响应不是照片清单",
      );
    });

    it("网络层错误（断网/超时中断，fetch 直接 reject）换人话：提示先核对已保存的照片，避免重复上传（终局评审）", async () => {
      vi.stubGlobal(
        "fetch",
        vi.fn(async () => {
          throw new TypeError("Failed to fetch");
        }),
      );
      await expect(uploadPhotosHttp(7, [new File(["x"], "a.jpg")])).rejects.toBe(
        "上传中断（网络断开或超时）。请刷新查看已保存的照片，避免重复上传后再试",
      );
    });
  });
});
