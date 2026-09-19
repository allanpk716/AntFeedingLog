import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  PLAINTEXT_WARNING,
  PORT_ERROR,
  REGEN_WARNING,
  RISK_HINT,
  copyText,
  maskToken,
  validatePortText,
} from "./webuiUi";

describe("端口校验（1024–65535，与 Rust 同口径）", () => {
  it("合法端口返回数值", () => {
    expect(validatePortText("1024")).toBe(1024);
    expect(validatePortText("17321")).toBe(17321);
    expect(validatePortText("65535")).toBe(65535);
    expect(validatePortText(" 8080 ")).toBe(8080);
  });

  it("非法输入返回 null", () => {
    for (const bad of ["", "abc", "-1", "1023", "65536", "99999", "1.5", "80px"]) {
      expect(validatePortText(bad)).toBeNull();
    }
  });

  it("错误文案含合法区间", () => {
    expect(PORT_ERROR).toContain("1024");
    expect(PORT_ERROR).toContain("65535");
  });
});

describe("凭证打码", () => {
  it("保留头尾各 4 位", () => {
    expect(maskToken("0123456789abcdef0123456789abcdef")).toBe("0123…cdef");
  });

  it("短凭证整串打码，空串不打码", () => {
    expect(maskToken("abcd1234")).toBe("••••••••");
    expect(maskToken("abc")).toBe("•••");
    expect(maskToken("")).toBe("");
  });
});

describe("固定文案（规格 B / User Story 2、3）", () => {
  it("物理网段硬警示明示明文风险", () => {
    expect(PLAINTEXT_WARNING).toContain("明文传输");
    expect(PLAINTEXT_WARNING).toContain("凭证");
  });

  it("重生成确认明示旧地址作废", () => {
    expect(REGEN_WARNING).toContain("立即作废");
    expect(RISK_HINT).toContain("勿转发");
    expect(RISK_HINT).toContain("重生成");
  });
});

describe("copyText（复制含凭证的完整地址）", () => {
  beforeEach(() => {
    vi.unstubAllGlobals();
  });

  it("Clipboard API 可用时走它并返回 true", async () => {
    const writeText = vi.fn(async () => undefined);
    vi.stubGlobal("navigator", { clipboard: { writeText } });
    await expect(copyText("http://100.84.12.3:17321/#token=abc")).resolves.toBe(true);
    expect(writeText).toHaveBeenCalledWith("http://100.84.12.3:17321/#token=abc");
  });

  it("Clipboard API 抛错 → 降级 execCommand，成功也返回 true", async () => {
    const writeText = vi.fn(async () => {
      throw new Error("denied");
    });
    vi.stubGlobal("navigator", { clipboard: { writeText } });
    const execCommand = vi.fn(() => true);
    document.body.appendChild = document.body.appendChild.bind(document.body);
    const original = document.execCommand;
    Object.defineProperty(document, "execCommand", {
      value: execCommand,
      configurable: true,
      writable: true,
    });
    try {
      await expect(copyText("text")).resolves.toBe(true);
      expect(execCommand).toHaveBeenCalledWith("copy");
    } finally {
      Object.defineProperty(document, "execCommand", {
        value: original,
        configurable: true,
        writable: true,
      });
    }
  });

  it("两者都不可用 → 返回 false（调用方提示手动复制）", async () => {
    vi.stubGlobal("navigator", {});
    await expect(copyText("text")).resolves.toBe(false);
  });
});
