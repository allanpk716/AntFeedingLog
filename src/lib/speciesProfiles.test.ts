/**
 * 物种档案数据包 loader 测试：12 份档案契约（数量与 key 清单、类型 distinct 动态分组、
 * schema 字段完整、空字段原样保留、隐私清洗、命名/冬眠/推荐裁决落实、悬停摘要）。
 */
import { describe, expect, it } from "vitest";
import {
  allSpeciesProfiles,
  getSpeciesProfile,
  speciesSummary,
  speciesTypeGroups,
  type SpeciesProfile,
} from "./speciesProfiles";

/** proposal §3.3 裁决的 12 种 key 清单 */
const EXPECTED_KEYS = [
  "messor-aciculatus",
  "messor-structor",
  "messor-barbarus",
  "messor-cephalotes",
  "camponotus-turkestanus",
  "camponotus-japonicus",
  "camponotus-vitiosus",
  "camponotus-pseudolendus",
  "camponotus-pseudoirritans",
  "camponotus-sericeiventris",
  "camponotus-mutilarius",
  "camponotus-nicobarensis",
];

/** 顶层 21 字段（growth/risks/careNotes 另计） */
const TOP_FIELDS = [
  "key",
  "cnName",
  "latinName",
  "aliases",
  "type",
  "hibernation",
  "hibernationNote",
  "difficulty",
  "beginnerRecommended",
  "recommendReason",
  "queenSize",
  "workerSize",
  "queenSystem",
  "activityRhythm",
  "tempRange",
  "humidity",
  "diet",
  "price",
  "distribution",
  "appearance",
  "purchaseWarning",
] as const;

const GROWTH_FIELDS = [
  "eggLaying",
  "eggToWorkerDays",
  "firstWorkersDays",
  "broodSurvival",
  "firstYearSize",
  "secondYearSize",
  "matureSize",
  "castes",
  "queenLifespan",
  "workerLifespan",
] as const;

const RISKS_FIELDS = [
  "stressNew",
  "stressColony",
  "stressNote",
  "escapeTendency",
  "healthRisks",
  "tempRedline",
] as const;

describe("speciesProfiles 内置档案包", () => {
  it("恰好 12 份，key 与裁决清单一致且文件名=key（按 key 能原样查回同一对象）", () => {
    const all = allSpeciesProfiles();
    expect(all).toHaveLength(12);
    expect(all.map((p) => p.key).sort()).toEqual([...EXPECTED_KEYS].sort());
    for (const p of all) {
      expect(getSpeciesProfile(p.key)).toBe(p);
    }
  });

  it("按 key 查询命中；查无返回 undefined；内置 key 禁 custom- 前缀", () => {
    const jp = getSpeciesProfile("camponotus-japonicus");
    expect(jp).toBeDefined();
    expect(jp!.cnName).toBe("日本弓背蚁");
    expect(jp!.latinName).toBe("Camponotus japonicus");
    expect(getSpeciesProfile("no-such-key")).toBeUndefined();
    for (const p of allSpeciesProfiles()) {
      expect(p.key.startsWith("custom-")).toBe(false);
    }
  });

  it("类型 distinct 动态分组：收获蚁/弓背蚁两组（4+8）；合成数据验证纯函数行为", () => {
    const groups = speciesTypeGroups(allSpeciesProfiles());
    expect(groups.map((g) => g.type).sort()).toEqual(["弓背蚁", "收获蚁"]);
    const byType = Object.fromEntries(groups.map((g) => [g.type, g.profiles.length]));
    expect(byType["收获蚁"]).toBe(4);
    expect(byType["弓背蚁"]).toBe(8);

    // 分组必须从数据得出：组按首次出现顺序、组内保持传入顺序（禁写死枚举的行为面）
    const fake = (type: string, key: string) => ({ type, key }) as SpeciesProfile;
    const synthetic = speciesTypeGroups([fake("X", "a"), fake("Y", "b"), fake("X", "c")]);
    expect(synthetic.map((g) => [g.type, g.profiles.map((p) => p.key)])).toEqual([
      ["X", ["a", "c"]],
      ["Y", ["b"]],
    ]);
  });

  it("每份档案字段完整覆盖 schema（允许空字符串，不允许缺字段）", () => {
    for (const p of allSpeciesProfiles()) {
      for (const field of TOP_FIELDS) {
        expect(p[field]).toBeDefined();
      }
      expect(Array.isArray(p.aliases)).toBe(true);
      expect(Array.isArray(p.careNotes)).toBe(true);
      for (const g of GROWTH_FIELDS) {
        expect(p.growth[g]).toBeDefined();
      }
      for (const r of RISKS_FIELDS) {
        expect(p.risks[r]).toBeDefined();
      }
      // 数值域契约
      expect(p.difficulty).toBeGreaterThanOrEqual(1);
      expect(p.difficulty).toBeLessThanOrEqual(3);
      expect(p.risks.stressNew).toBeGreaterThanOrEqual(1);
      expect(p.risks.stressNew).toBeLessThanOrEqual(5);
      expect(p.risks.stressColony).toBeGreaterThanOrEqual(1);
      expect(p.risks.stressColony).toBeLessThanOrEqual(5);
      expect(["冬眠", "浅冬眠", "不冬眠"]).toContain(p.hibernation);
    }
  });

  it("空字段原样保留（查无实据留空，不填默认值）", () => {
    expect(getSpeciesProfile("camponotus-turkestanus")!.diet).toBe("");
    expect(getSpeciesProfile("messor-cephalotes")!.growth.workerLifespan).toBe("");
    expect(getSpeciesProfile("camponotus-vitiosus")!.growth.eggLaying).toBe("");
    expect(getSpeciesProfile("camponotus-vitiosus")!.tempRange).toBe("");
    expect(getSpeciesProfile("camponotus-pseudolendus")!.queenSystem).toBe("");
    expect(getSpeciesProfile("messor-aciculatus")!.queenSize).toBe("");
  });

  it("档案内容零隐私信息；分布产地中的广西允许保留", () => {
    const dump = JSON.stringify(allSpeciesProfiles());
    for (const banned of ["桂林", "办公室", "已养三窝", "共用加热垫"]) {
      expect(dump).not.toContain(banned);
    }
    expect(dump).toContain("广西"); // 尼科巴产地（生物学事实，proposal §6 允许保留）
  });

  it("命名裁决：中亚/瑕疵 cnName+俗名进 aliases；热带黑金不作丝腹别名但购买提醒写明此坑", () => {
    const tk = getSpeciesProfile("camponotus-turkestanus")!;
    expect(tk.cnName).toBe("中亚弓背蚁");
    expect(tk.aliases).toContain("突厥弓背蚁");
    expect(tk.aliases).toContain("黑金土耳其");
    const vt = getSpeciesProfile("camponotus-vitiosus")!;
    expect(vt.cnName).toBe("瑕疵弓背蚁");
    expect(vt.aliases).toContain("统领弓背蚁");
    const sv = getSpeciesProfile("camponotus-sericeiventris")!;
    expect(sv.aliases).not.toContain("热带黑金");
    expect(sv.purchaseWarning).toContain("黑金");
    expect(sv.purchaseWarning).toContain("费氏弓背蚁");
  });

  it("推荐位恰 4 种且都带理由；红头理由含「建群头两个月少看少动」", () => {
    const recommended = allSpeciesProfiles().filter((p) => p.beginnerRecommended);
    expect(recommended.map((p) => p.key).sort()).toEqual(
      [
        "messor-aciculatus",
        "messor-barbarus",
        "camponotus-turkestanus",
        "camponotus-pseudoirritans",
      ].sort(),
    );
    for (const p of recommended) {
      expect(p.recommendReason.trim()).not.toBe("");
    }
    expect(getSpeciesProfile("messor-barbarus")!.recommendReason).toContain("建群头两个月少看少动");
  });

  it("悬停一行式摘要：温度/食物/冬眠三要素，缺项显示暂无资料", () => {
    const jp = getSpeciesProfile("camponotus-japonicus")!;
    const parts = speciesSummary(jp).split(" · ");
    expect(parts).toHaveLength(3);
    expect(parts[0]).toBe(jp.tempRange);
    expect(parts[1]).toBe(jp.diet);
    expect(parts[2]).toBe("冬眠");

    const tk = getSpeciesProfile("camponotus-turkestanus")!;
    expect(speciesSummary(tk)).toContain("暂无资料"); // diet 查无实据
    expect(speciesSummary(tk).endsWith("冬眠")).toBe(true);
  });
});
