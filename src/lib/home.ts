/**
 * 首页分组逻辑：按地点清单顺序分组，没有地点（或地点已不存在）的窝归「未分组」排最后，
 * 已结束的窝不进分组（归底部折叠区，见 splitColonies）。视觉基线 mocks/mock-a-light.html。
 */

import type { Colony, LocationItem } from "../types";

export interface HomeGroup {
  key: string;
  title: string;
  colonies: Colony[];
}

/** 活跃/冬眠的窝 + 已结束的窝。 */
export function splitColonies(colonies: Colony[]): { active: Colony[]; ended: Colony[] } {
  return {
    active: colonies.filter((c) => c.status !== "ended"),
    ended: colonies.filter((c) => c.status === "ended"),
  };
}

/** 有窝的地点才成组；停用地点照常参与（窝还挂在那），只是不进新建/编辑下拉。 */
export function groupColonies(colonies: Colony[], locations: LocationItem[]): HomeGroup[] {
  const active = colonies.filter((c) => c.status !== "ended");
  const sortedLocations = [...locations].sort((a, b) => a.sort - b.sort || a.id - b.id);

  const groups: HomeGroup[] = [];
  for (const loc of sortedLocations) {
    const members = active.filter((c) => c.location_id === loc.id);
    if (members.length > 0) {
      groups.push({ key: `loc-${loc.id}`, title: loc.name, colonies: members });
    }
  }

  const knownIds = new Set(sortedLocations.map((l) => l.id));
  const ungrouped = active.filter((c) => c.location_id === null || !knownIds.has(c.location_id));
  if (ungrouped.length > 0) {
    groups.push({ key: "ungrouped", title: "未分组", colonies: ungrouped });
  }
  return groups;
}
