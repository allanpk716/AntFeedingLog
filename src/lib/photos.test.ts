import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  MAX_PHOTOS_PER_SUBMIT,
  MAX_PHOTO_BYTES,
  createCheckinPhotosHttp,
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

  // ── 创建模式（checkin-photo-entry 票 02）：「拍一张」不挂既有登记，直接建当天
  // 登记挂照片。multipart 无 checkinId 段：colonyId 必填文本段 + 可选 date 段
  // + photos 文件段 → POST /api/photos 返回 NestPhotoMeta[]。 ──

  describe("创建模式 createCheckinPhotosHttp（checkin-photo-entry 票 02）", () => {
    it("multipart 形状：colonyId 文本段 + date 段 + photos 文件段，无 checkinId 段，Bearer 头照带", async () => {
      localStorage.setItem(WEBUI_TOKEN_KEY, "tok-create");
      const created = [
        { id: 11, checkin_id: 55, rel_path: "1/x.jpg", original_name: null, note: "" },
      ];
      const fetchMock = vi.fn(async () => ({
        ok: true,
        status: 200,
        json: async () => created,
      }));
      vi.stubGlobal("fetch", fetchMock);
      const files = [new File(["a"], "a.jpg"), new File(["b"], "b.png")];

      const out = await createCheckinPhotosHttp(3, files, "2026-09-28");

      expect(out).toEqual(created);
      const [url, init] = fetchMock.mock.calls[0] as unknown as [string, RequestInit];
      expect(url).toBe("/api/photos");
      expect(init.method).toBe("POST");
      expect((init.headers as Record<string, string>).Authorization).toBe("Bearer tok-create");
      const form = init.body as FormData;
      expect(form.get("colonyId")).toBe("3");
      expect(form.get("date")).toBe("2026-09-28");
      expect(form.get("checkinId")).toBeNull(); // 创建模式与既有挂靠模式互斥的分界
      expect(form.getAll("photos")).toHaveLength(2);
    });

    it("date 缺省：multipart 不带 date 段（服务端缺省按今天）", async () => {
      const fetchMock = vi.fn(async () => ({ ok: true, status: 200, json: async () => [] }));
      vi.stubGlobal("fetch", fetchMock);

      await createCheckinPhotosHttp(3, [new File(["a"], "a.jpg")]);

      const [, init] = fetchMock.mock.calls[0] as unknown as [string, RequestInit];
      expect((init.body as FormData).get("date")).toBeNull();
    });

    it("字段段（票 03 缝合）：计数整数段、movedNest 仅 true 带、note trim 非空才带；null/缺省不带", async () => {
      const fetchMock = vi.fn(async () => ({ ok: true, status: 200, json: async () => [] }));
      vi.stubGlobal("fetch", fetchMock);

      await createCheckinPhotosHttp(3, [new File(["a"], "a.jpg")], "2026-09-28", {
        queenCount: 2,
        workerCount: null, // null = 未数，不带段
        movedNest: true,
        note: "  状态不错  ", // trim 后带
      });

      const form = (fetchMock.mock.calls[0] as unknown as [string, RequestInit])[1]
        .body as FormData;
      expect(form.get("queenCount")).toBe("2");
      expect(form.get("workerCount")).toBeNull();
      expect(form.get("movedNest")).toBe("true");
      expect(form.get("note")).toBe("状态不错");

      // 反向形态：movedNest false / note 空白 / 字段全缺省 → 三个段都不带
      await createCheckinPhotosHttp(3, [new File(["a"], "a.jpg")], "2026-09-28", {
        movedNest: false,
        note: "   ",
      });
      const form2 = (fetchMock.mock.calls[1] as unknown as [string, RequestInit])[1]
        .body as FormData;
      expect(form2.get("queenCount")).toBeNull();
      expect(form2.get("workerCount")).toBeNull();
      expect(form2.get("movedNest")).toBeNull();
      expect(form2.get("note")).toBeNull();
    });

    it("预检沿用：张数超 9 / 单张超 15MB 本地拒绝不发请求；空列表空数组返回", async () => {
      const fetchMock = vi.fn();
      vi.stubGlobal("fetch", fetchMock);
      const files = Array.from({ length: 10 }, (_, i) => new File(["x"], `${i}.jpg`));
      await expect(createCheckinPhotosHttp(3, files)).rejects.toBe("一次最多上传 9 张照片");
      const big = new File(["x"], "big.jpg");
      Object.defineProperty(big, "size", { value: MAX_PHOTO_BYTES + 1 });
      await expect(createCheckinPhotosHttp(3, [big])).rejects.toContain("big.jpg");
      await expect(createCheckinPhotosHttp(3, [])).resolves.toEqual([]);
      expect(fetchMock).not.toHaveBeenCalled();
    });

    it("非 2xx 以响应体 { error } 人话 reject（与既有模式同路）", async () => {
      vi.stubGlobal(
        "fetch",
        vi.fn(async () => ({
          ok: false,
          status: 400,
          json: async () => ({ error: "窝不存在" }),
        })),
      );
      await expect(createCheckinPhotosHttp(3, [new File(["x"], "a.jpg")])).rejects.toBe(
        "窝不存在",
      );
    });

    it("网络层中断换人话：新通道事务全成或全无，本次未保存可直接重试", async () => {
      vi.stubGlobal(
        "fetch",
        vi.fn(async () => {
          throw new TypeError("Failed to fetch");
        }),
      );
      await expect(createCheckinPhotosHttp(3, [new File(["x"], "a.jpg")])).rejects.toBe(
        "上传中断（网络断开或超时）。本次登记未保存，可直接重试",
      );
    });
  });
});

// ── 浏览器上传前压缩（拍照上传中断修复）：大图先缩到服务端同口径再传 ─────────
// 背景：手机网页端拍照上传稳定「上传中断」——手机 NetBird 直连建不起来、流量
// 绕道云服务器，MB 级原图过不去而亚 MB 级请求都能过。服务端本来就要重编码
// （长边 2048/质量 85），原图直传纯浪费。上传前浏览器端先压到同口径。

describe("photos 浏览器上传前压缩（拍照上传中断修复）", () => {
  /** 大图 File（600KB，size 用 defineProperty 虚标，不占内存）。 */
  function bigCamPhoto(name = "IMG_0001.jpg"): File {
    const f = new File(["x"], name);
    Object.defineProperty(f, "size", { value: 600 * 1024 });
    return f;
  }

  /** 画布桩：记录 toBlob 参数，回调小体积 JPEG blob（22B < 600KB 虚标原文件）。 */
  function stubCanvas(toBlobImpl?: (cb: (b: Blob | null) => void, type: string, q: number) => void) {
    const canvas = {
      width: 0,
      height: 0,
      getContext: () => ({ fillRect: vi.fn(), drawImage: vi.fn() }),
      toBlob:
        toBlobImpl ??
        ((cb: (b: Blob | null) => void, _type: string, _q: number) => {
          cb(new Blob(["compressed-jpeg-bytes"], { type: "image/jpeg" }));
        }),
    };
    const origCreate = document.createElement.bind(document);
    return vi
      .spyOn(document, "createElement")
      .mockImplementation((tag: string) =>
        tag === "canvas" ? (canvas as unknown as HTMLCanvasElement) : origCreate(tag),
      );
  }

  /** 图桩：4032×3024（iPhone 主摄量级），decode 可注入成败。 */
  function stubImage(decode: () => Promise<void> = () => Promise.resolve()) {
    class FakeImage {
      src = "";
      naturalWidth = 4032;
      naturalHeight = 3024;
      decode(): Promise<void> {
        return decode();
      }
    }
    vi.stubGlobal("Image", FakeImage);
    return FakeImage;
  }

  beforeEach(() => {
    localStorage.clear();
    Object.defineProperty(URL, "createObjectURL", {
      value: vi.fn(() => "blob:mock-url"),
      configurable: true,
      writable: true,
    });
    Object.defineProperty(URL, "revokeObjectURL", {
      value: vi.fn(),
      configurable: true,
      writable: true,
    });
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    localStorage.clear();
  });

  it("小文件（≤512KB）不压缩：原样引用返回，不建画布不解码", async () => {
    const { compressForUpload } = await import("./photos");
    const createSpy = vi.spyOn(document, "createElement");
    const small = new File(["x"], "a.jpg");

    const out = await compressForUpload(small);

    expect(out).toBe(small);
    expect(createSpy).not.toHaveBeenCalled();
  });

  it("大图压缩：缩到长边 2048、JPEG 0.85 重编码，名字保留、体积更小", async () => {
    const { compressForUpload } = await import("./photos");
    stubImage();
    const createSpy = stubCanvas();

    const out = await compressForUpload(bigCamPhoto());

    expect(createSpy).toHaveBeenCalled();
    expect(out).not.toHaveProperty("size", 600 * 1024);
    expect(out.name).toBe("IMG_0001.jpg");
    expect(out.type).toBe("image/jpeg");
    expect(out.size).toBeLessThan(600 * 1024);
    const canvas = createSpy.mock.results[0]!.value as { width: number; height: number };
    expect(canvas.width).toBe(2048);
    expect(canvas.height).toBe(1536);
    expect(URL.revokeObjectURL).toHaveBeenCalledWith("blob:mock-url");
  });

  it("解码不了（如部分浏览器对 HEIC）退回原文件：原样引用，服务端校验链兜底", async () => {
    const { compressForUpload } = await import("./photos");
    stubImage(() => Promise.reject(new Error("decode error")));
    stubCanvas();

    const src = bigCamPhoto();
    const out = await compressForUpload(src);

    expect(out).toBe(src);
  });

  it("重编码后不小于原文件（压不动）退回原文件", async () => {
    const { compressForUpload } = await import("./photos");
    stubImage();
    stubCanvas((cb) => {
      cb(new Blob([new Uint8Array(700 * 1024)], { type: "image/jpeg" }));
    });

    const src = bigCamPhoto();
    const out = await compressForUpload(src);

    expect(out).toBe(src);
  });

  it("uploadPhotosHttp 发出的是压缩后的照片（FormData photos 段）", async () => {
    stubImage();
    stubCanvas();
    const fetchMock = vi.fn(async () => ({
      ok: true,
      status: 200,
      json: async () => [],
    }));
    vi.stubGlobal("fetch", fetchMock);

    await uploadPhotosHttp(7, [bigCamPhoto()]);

    const [, init] = fetchMock.mock.calls[0] as unknown as [string, RequestInit];
    const form = init.body as FormData;
    const sent = form.getAll("photos")[0] as File;
    expect(sent.size).toBe(21); // 压缩桩产出的 blob 体积，不是 600KB 原图
    expect(sent.type).toBe("image/jpeg");
  });

  it("createCheckinPhotosHttp 同样先压缩（「拍一张」直达通道）", async () => {
    stubImage();
    stubCanvas();
    const fetchMock = vi.fn(async () => ({
      ok: true,
      status: 200,
      json: async () => [],
    }));
    vi.stubGlobal("fetch", fetchMock);

    await createCheckinPhotosHttp(3, [bigCamPhoto()], "2026-09-28");

    const [, init] = fetchMock.mock.calls[0] as unknown as [string, RequestInit];
    const form = init.body as FormData;
    const sent = form.getAll("photos")[0] as File;
    expect(sent.size).toBe(21);
    expect(sent.type).toBe("image/jpeg");
  });

  it("预检仍按原图口径：15MB 上限在压缩前拦（与服务端单张口径一致）", async () => {
    const fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock);
    const over = new File(["x"], "huge.jpg");
    Object.defineProperty(over, "size", { value: MAX_PHOTO_BYTES + 1 });

    await expect(uploadPhotosHttp(7, [over])).rejects.toContain("huge.jpg");
    expect(fetchMock).not.toHaveBeenCalled();
  });
});
