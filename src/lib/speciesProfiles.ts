/**
 * 物种档案数据包 loader（ADR 0009，前端静态档案包）：构建期 glob 导入
 * src/data/species/*.json，一物种一文件、文件名 = key（拉丁名 slug）。
 * 加档案 = 加一份 JSON，代码零改动；分组等随之自动生效。
 *
 * 契约要点（spec/proposal）：
 * - `type` 是档案数据：分组一律 distinct 动态得出，禁写死枚举；
 * - `hibernation` 三值（冬眠/浅冬眠/不冬眠）供筛选，细节在 hibernationNote；
 * - 空字符串 = 查无实据（界面显示「暂无资料」），绝不填默认值；
 * - 有据值行内标注来源档（文献/多源/商家/经验/属级估计）；
 * - 内置 key = 拉丁名 slug（无前缀），与自建 `custom-N` 命名空间隔离。
 * 饲养资料永不进数据库；窝表只存 species_key + 显示名快照（见 spec §3.1）。
 */

/** 冬眠需求三值（供筛选；口径细节在 hibernationNote） */
export type HibernationLevel = "冬眠" | "浅冬眠" | "不冬眠";

/** 发展预期（10 字段；空字符串 = 查无实据） */
export interface SpeciesGrowth {
  eggLaying: string;
  eggToWorkerDays: string;
  firstWorkersDays: string;
  broodSurvival: string;
  firstYearSize: string;
  secondYearSize: string;
  matureSize: string;
  castes: string;
  queenLifespan: string;
  workerLifespan: string;
}

/** 风险敏感度（6 字段；惊吓分新后/成群两档，1~5，5 = 极度敏感） */
export interface SpeciesRisks {
  stressNew: number;
  stressColony: number;
  stressNote: string;
  escapeTendency: string;
  healthRisks: string;
  tempRedline: string;
}

/** 物种档案：顶层 21 字段 + growth（10）+ risks（6）+ careNotes */
export interface SpeciesProfile {
  key: string;
  cnName: string;
  latinName: string;
  aliases: string[];
  type: string;
  hibernation: HibernationLevel;
  hibernationNote: string;
  /** 1~3 星 */
  difficulty: number;
  beginnerRecommended: boolean;
  /** 新手推荐理由；仅 beginnerRecommended=true 时非空 */
  recommendReason: string;
  queenSize: string;
  workerSize: string;
  queenSystem: string;
  activityRhythm: string;
  tempRange: string;
  humidity: string;
  diet: string;
  price: string;
  distribution: string;
  appearance: string;
  purchaseWarning: string;
  growth: SpeciesGrowth;
  risks: SpeciesRisks;
  careNotes: string[];
}

const MODULES = import.meta.glob("../data/species/*.json", {
  eager: true,
  import: "default",
}) as Record<string, SpeciesProfile>;

/** "../data/species/camponotus-japonicus.json" → "camponotus-japonicus"（文件名即 key） */
function keyFromPath(path: string): string {
  const base = path.split("/").pop() ?? "";
  return base.replace(/\.json$/i, "");
}

/** key → 档案索引（构建期一次性建好；JSON 内的 key 字段与文件名一致，由测试守护） */
const BY_KEY: ReadonlyMap<string, SpeciesProfile> = (() => {
  const map = new Map<string, SpeciesProfile>();
  for (const [path, profile] of Object.entries(MODULES)) {
    map.set(keyFromPath(path), profile);
  }
  return map;
})();

/** 按 key 查询内置档案；查无返回 undefined（调用方回落自建表/显示名快照） */
export function getSpeciesProfile(key: string): SpeciesProfile | undefined {
  return BY_KEY.get(key);
}

/** 全部内置档案，按 key 字典序稳定排序（展示层可自行再排） */
export function allSpeciesProfiles(): SpeciesProfile[] {
  return [...BY_KEY.values()].sort((a, b) => (a.key < b.key ? -1 : a.key > b.key ? 1 : 0));
}

export interface SpeciesTypeGroup {
  type: string;
  profiles: SpeciesProfile[];
}

/**
 * 按类型分组：类型 distinct 从数据动态得出（加档案自动成组，禁写死枚举）。
 * 组按首次出现顺序，组内保持传入顺序。
 */
export function speciesTypeGroups(profiles: readonly SpeciesProfile[]): SpeciesTypeGroup[] {
  const groups: SpeciesTypeGroup[] = [];
  const byType = new Map<string, SpeciesTypeGroup>();
  for (const p of profiles) {
    let group = byType.get(p.type);
    if (!group) {
      group = { type: p.type, profiles: [] };
      byType.set(p.type, group);
      groups.push(group);
    }
    group.profiles.push(p);
  }
  return groups;
}

/** 一行式悬停摘要（温度/食物/冬眠三要素）；缺项如实显示「暂无资料」 */
export function speciesSummary(p: SpeciesProfile): string {
  return [p.tempRange, p.diet, p.hibernation]
    .map((part) => (part === "" ? "暂无资料" : part))
    .join(" · ");
}
