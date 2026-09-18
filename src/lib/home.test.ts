import { describe, expect, it } from "vitest";
import type { Colony, LocationItem } from "../types";
import { groupColonies, splitColonies } from "./home";

function colony(partial: Partial<Colony> & { id: number }): Colony {
  return {
    name: `窝${partial.id}`,
    species: "大头收获蚁",
    location_id: null,
    start_date: "2026-01-20",
    status: "active",
    days_raised: 100,
    ...partial,
  };
}

const locations: LocationItem[] = [
  { id: 1, name: "家", enabled: true, sort: 1 },
  { id: 2, name: "公司", enabled: true, sort: 2 },
];

describe("首页分组", () => {
  it("按地点清单顺序分组，组内保留给定顺序", () => {
    const a = colony({ id: 1, location_id: 1 });
    const b = colony({ id: 2, location_id: 2 });
    const c = colony({ id: 3, location_id: 2 });
    const groups = groupColonies([a, b, c], locations);

    expect(groups.map((g) => g.title)).toEqual(["家", "公司"]);
    expect(groups[0].colonies.map((x) => x.id)).toEqual([1]);
    expect(groups[1].colonies.map((x) => x.id)).toEqual([2, 3]);
  });

  it("地点顺序按 sort 而不是 id", () => {
    const reordered: LocationItem[] = [
      { id: 2, name: "公司", enabled: true, sort: 1 },
      { id: 1, name: "家", enabled: true, sort: 2 },
    ];
    const groups = groupColonies(
      [colony({ id: 1, location_id: 1 }), colony({ id: 2, location_id: 2 })],
      reordered,
    );
    expect(groups.map((g) => g.title)).toEqual(["公司", "家"]);
  });

  it("没有地点的窝归入「未分组」且排最后", () => {
    const grouped = colony({ id: 1, location_id: 2 });
    const free = colony({ id: 2, location_id: null });
    const groups = groupColonies([free, grouped], locations);

    expect(groups.map((g) => g.title)).toEqual(["公司", "未分组"]);
    expect(groups[1].colonies.map((x) => x.id)).toEqual([2]);
  });

  it("地点清单里找不到的窝（地点已删/脏数据）也归「未分组」", () => {
    const orphan = colony({ id: 1, location_id: 99 });
    const groups = groupColonies([orphan], locations);
    expect(groups.map((g) => g.title)).toEqual(["未分组"]);
  });

  it("没有窝的地点不产生空分组", () => {
    const groups = groupColonies([colony({ id: 1, location_id: 1 })], locations);
    expect(groups.map((g) => g.title)).toEqual(["家"]);
  });

  it("停用地点仍参与分组（窝还挂在那），只是不进新建下拉", () => {
    const disabled: LocationItem[] = [{ id: 1, name: "旧家", enabled: false, sort: 1 }];
    const groups = groupColonies([colony({ id: 1, location_id: 1 })], disabled);
    expect(groups.map((g) => g.title)).toEqual(["旧家"]);
  });
});

describe("活跃/已结束拆分", () => {
  it("已结束的窝单独一组，其余保持顺序", () => {
    const active1 = colony({ id: 1 });
    const ended = colony({ id: 2, status: "ended" });
    const active2 = colony({ id: 3, status: "hibernating" });
    const { active, ended: endedOut } = splitColonies([active1, ended, active2]);
    expect(active.map((c) => c.id)).toEqual([1, 3]);
    expect(endedOut.map((c) => c.id)).toEqual([2]);
  });
});
