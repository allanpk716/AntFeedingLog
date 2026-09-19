import { afterEach, describe, expect, it } from "vitest";

import { formatBytes, joinPhotoPath, photoSrc } from "./photos";

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

    it("浏览器环境返回空串（网页端照片读取随票 08 接线）", () => {
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
