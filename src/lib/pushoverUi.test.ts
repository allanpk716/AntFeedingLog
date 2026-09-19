import { describe, expect, it } from "vitest";
import type { PushoverStatus } from "../types";
import {
  PUSHOVER_PLACEHOLDER,
  PUSHOVER_PLAINTEXT_WARNING,
  PUSHOVER_SOURCE_LABELS,
  pushoverSourceLabel,
} from "./pushoverUi";

describe("Pushover 应用内配置（webui-checkin 票 11）：三态来源标注", () => {
  it("三态各有固定标签：应用内配置 / 系统环境变量 / 未配置", () => {
    expect(PUSHOVER_SOURCE_LABELS.app).toBe("应用内配置");
    expect(PUSHOVER_SOURCE_LABELS.env).toBe("系统环境变量");
    expect(PUSHOVER_SOURCE_LABELS.none).toBe("未配置");
  });

  it("pushoverSourceLabel 按状态回显标签", () => {
    const status = (source: PushoverStatus["source"], configured: boolean): PushoverStatus => ({
      source,
      configured,
    });
    expect(pushoverSourceLabel(status("app", true))).toBe("应用内配置");
    expect(pushoverSourceLabel(status("env", true))).toBe("系统环境变量");
    expect(pushoverSourceLabel(status("none", false))).toBe("未配置");
  });

  it("状态读取失败（null）→ 显示「读取失败」，不冒充任何一态", () => {
    expect(pushoverSourceLabel(null)).toBe("读取失败");
  });
});

describe("Pushover 应用内配置：固定文案", () => {
  it("明文入库并随备份扩散的风险提示（验收：设置页可见的一句人话）", () => {
    expect(PUSHOVER_PLAINTEXT_WARNING).toContain("明文");
    expect(PUSHOVER_PLAINTEXT_WARNING).toContain("备份");
  });

  it("占位符提示：留空则使用系统环境变量", () => {
    expect(PUSHOVER_PLACEHOLDER).toContain("留空");
    expect(PUSHOVER_PLACEHOLDER).toContain("环境变量");
  });
});
