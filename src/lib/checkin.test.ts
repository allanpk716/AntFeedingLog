import { describe, expect, it } from "vitest";
import { checkinCardLine, checkinEntryLine, countsText, daysSinceText } from "./checkin";
import type { CheckinDigest, NestCheckin } from "../types";

function checkin(overrides: Partial<NestCheckin> = {}): NestCheckin {
  return {
    id: 1,
    colony_id: 1,
    date: "2026-09-15",
    queen_count: 2,
    worker_count: 3000,
    moved_nest: false,
    note: "",
    created_at: "2026-09-15 21:00:00",
    photos: [],
    ...overrides,
  };
}

describe("巢况展示纯函数（webui-checkin 票 02）", () => {
  it("countsText：都空返回空串，单项只出该项，两项用 · 连接", () => {
    expect(countsText(null, null)).toBe("");
    expect(countsText(2, null)).toBe("蚁后 2");
    expect(countsText(null, 3000)).toBe("工蚁 3000");
    expect(countsText(2, 3000)).toBe("蚁后 2 · 工蚁 3000");
  });

  it("daysSinceText：当天 0 = 今天登记，其余 = 距上次登记 N 天", () => {
    expect(daysSinceText(0)).toBe("今天登记");
    expect(daysSinceText(3)).toBe("距上次登记 3 天");
  });

  it("checkinCardLine：从未登记返回空串（调用方隐藏该行）", () => {
    const digest: CheckinDigest = { latest: null, baseline_date: null, days_since_last: null };
    expect(checkinCardLine(digest)).toBe("");
  });

  it("checkinCardLine：最新一组数 + 距上次登记天数", () => {
    const digest: CheckinDigest = {
      latest: checkin(),
      baseline_date: "2026-09-01",
      days_since_last: 3,
    };
    expect(checkinCardLine(digest)).toBe("巢况：蚁后 2 · 工蚁 3000 · 距上次登记 3 天");
  });

  it("checkinCardLine：只数了蚁后 / 今天刚登记 / 换巢 标记", () => {
    const digest = (c: NestCheckin, days: number | null): CheckinDigest => ({
      latest: c,
      baseline_date: c.date,
      days_since_last: days,
    });
    expect(checkinCardLine(digest(checkin({ worker_count: null }), 1))).toBe(
      "巢况：蚁后 2 · 距上次登记 1 天",
    );
    expect(checkinCardLine(digest(checkin({ queen_count: null }), 0))).toBe(
      "巢况：工蚁 3000 · 今天登记",
    );
    expect(checkinCardLine(digest(checkin({ queen_count: null, moved_nest: true }), 2))).toBe(
      "巢况：工蚁 3000 · 换巢 · 距上次登记 2 天",
    );
  });

  it("checkinEntryLine：时间线单条主文案，含换巢与备注", () => {
    expect(checkinEntryLine(checkin())).toBe("蚁后 2 · 工蚁 3000");
    expect(checkinEntryLine(checkin({ moved_nest: true }))).toBe("蚁后 2 · 工蚁 3000 · 换巢");
    expect(checkinEntryLine(checkin({ queen_count: null, note: "状态不错" }))).toBe(
      "工蚁 3000 · 备注：状态不错",
    );
    expect(
      checkinEntryLine(checkin({ queen_count: null, worker_count: null, moved_nest: true })),
    ).toBe("换巢");
    expect(
      checkinEntryLine(checkin({ queen_count: null, worker_count: null, note: "只写了备注" })),
    ).toBe("备注：只写了备注");
  });
});
