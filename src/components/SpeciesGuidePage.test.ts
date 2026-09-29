/**
 * 图鉴页测试（物种档案票 04）：推荐区两路线陈列与理由、避坑区（排除 7 种 + 香斑点名）、
 * 12 份档案卡展开渲染（空字段「暂无资料」、惊吓两档、难度★、careNotes 逐条）、
 * 通用饲养知识七主题、全文零隐私字样。纯前端内容页：不 mock IPC，直接用真实档案包。
 */
import { describe, expect, it } from "vitest";
import { mount, type DOMWrapper } from "@vue/test-utils";
import type { VueWrapper } from "@vue/test-utils";
import SpeciesGuidePage from "./SpeciesGuidePage.vue";

function mountGuide(): VueWrapper {
  return mount(SpeciesGuidePage);
}

/** 按中文名找到档案卡（.sp-card 整卡，含折叠头与详情区）。 */
function findCard(w: VueWrapper, cnName: string): DOMWrapper<Element> {
  const card = w.findAll(".sp-card").find((c) => c.find(".sp-name").text() === cnName);
  expect(card, `${cnName} 的档案卡应存在`).toBeDefined();
  return card!;
}

/** 展开指定物种的档案卡并返回详情区。 */
async function expandCard(w: VueWrapper, cnName: string): Promise<DOMWrapper<Element>> {
  await findCard(w, cnName).find(".sp-head").trigger("click");
  const detail = findCard(w, cnName).find(".sp-detail");
  expect(detail.exists()).toBe(true);
  return detail;
}

/** 详情区里按字段标签取值（dt=标签、dd=值）。 */
function rowValue(detail: DOMWrapper<Element>, label: string): string {
  const row = detail.findAll(".rows .row").find((r) => r.find("dt").text() === label);
  expect(row, `字段「${label}」行应存在`).toBeDefined();
  return row!.find("dd").text();
}

describe("图鉴页 · 新手推荐区", () => {
  it("推荐位恰好 4 种，按冬眠/不冬眠两路线陈列，卡片含中文名/拉丁名/难度★/推荐理由", () => {
    const w = mountGuide();
    const routes = w.findAll(".rec-route");
    expect(routes).toHaveLength(2);

    // 冬眠路线：冬眠+浅冬眠归此路线（针毛/红头浅冬眠、中亚冬眠）；不冬眠路线：拟光腹
    const dormantNames = routes[0].findAll(".rec-name").map((n) => n.text());
    expect(dormantNames).toEqual(["针毛收获蚁", "中亚弓背蚁", "红头收获蚁"]);
    const nonDormantNames = routes[1].findAll(".rec-name").map((n) => n.text());
    expect(nonDormantNames).toEqual(["拟光腹弓背蚁"]);

    // 恰好 4 张推荐卡；每张含拉丁名、难度★（1~3 个实心星）、非空理由
    expect(w.findAll(".rec-card")).toHaveLength(4);
    for (const card of w.findAll(".rec-card")) {
      expect(card.find(".rec-latin").text()).toMatch(/[A-Z][a-z]+ /);
      expect(card.find(".rec-stars").text()).toMatch(/^★{1,3}$/u);
      expect(card.find(".rec-reason").text().trim()).not.toBe("");
    }
  });

  it("红头推荐理由带「建群头两个月少看少动」提醒（D16）", () => {
    const w = mountGuide();
    const card = w
      .findAll(".rec-card")
      .find((c) => c.find(".rec-name").text() === "红头收获蚁");
    expect(card).toBeDefined();
    expect(card!.find(".rec-reason").text()).toContain("建群头两个月少看少动");
  });

  it("避坑区：排除 7 种各带一句原因 + 香斑弓背蚁点名「暴毙天王」", () => {
    const w = mountGuide();
    const avoid = w.find(".avoid-list");
    expect(avoid.exists()).toBe(true);

    const banned = [
      "窄颈弓背蚁",
      "墨西哥蜜罐蚁",
      "红头弓背蚁",
      "拟黑多刺蚁",
      "黄猄蚁",
      "红足修猛蚁",
      "巨人恐蚁",
    ];
    for (const name of banned) {
      const li = avoid.findAll("li").find((x) => x.text().includes(name));
      expect(li, `${name} 应出现在避坑区`).toBeDefined();
      expect(li!.text().length, `${name} 应带排除原因`).toBeGreaterThan(name.length + 4);
    }
    expect(avoid.text()).toContain("香斑弓背蚁");
    expect(avoid.text()).toContain("暴毙天王");
    expect(avoid.text()).toContain("建议有经验后再碰");
  });
});

describe("图鉴页 · 全部物种档案", () => {
  it("12 份档案全部渲染为可展开卡，按类型 distinct 分组（弓背蚁 8 + 收获蚁 4）", () => {
    const w = mountGuide();
    const groups = w.findAll(".guide-group");
    // 组与组内顺序来自档案数据（loader key 字典序：camponotus 在前）；标题带种数
    expect(groups.map((g) => g.find(".group-title").text())).toEqual(["弓背蚁 8 种", "收获蚁 4 种"]);
    const counts = groups.map((g) => g.findAll(".sp-card").length);
    expect(counts).toEqual([8, 4]);
    expect(w.findAll(".sp-head")).toHaveLength(12);

    // 默认全部折叠
    expect(w.find(".sp-detail").exists()).toBe(false);
  });

  it("12 份档案逐个展开都能渲染详情（折叠头含中文名/拉丁名/类型/冬眠/难度★）", async () => {
    const w = mountGuide();
    const heads = w.findAll(".sp-head");
    expect(heads).toHaveLength(12);
    for (const head of heads) {
      await head.trigger("click");
    }
    expect(w.findAll(".sp-detail")).toHaveLength(12);

    // 折叠头要素：香斑 ★★★、冬眠徽章
    const mutilarius = findCard(w, "香斑弓背蚁");
    expect(mutilarius.find(".sp-stars").text()).toBe("★★★");
    expect(mutilarius.find(".chip.hib").text()).toBe("不冬眠");
    expect(findCard(w, "工匠收获蚁").find(".sp-stars").text()).toBe("★");
  });

  it("展开后四区块齐备：饲养参数/发展预期/风险敏感度/购买提醒，惊吓分新后/成群两档", async () => {
    const w = mountGuide();
    const detail = await expandCard(w, "工匠收获蚁");

    expect(detail.find(".sp-params").text()).toContain("饲养参数");
    expect(detail.find(".sp-growth").text()).toContain("发展预期");
    expect(detail.find(".sp-risks").text()).toContain("风险敏感度");
    expect(detail.find(".buy-warning").text()).toContain("购买提醒");

    // 惊吓两档（工匠：新后 3、成群 2）+ 说明
    const tiers = detail.findAll(".stress-tier").map((t) => t.text());
    expect(tiers).toHaveLength(2);
    expect(tiers[0]).toContain("新后");
    expect(tiers[0]).toContain("3/5");
    expect(tiers[1]).toContain("成群");
    expect(tiers[1]).toContain("2/5");
    expect(detail.find(".stress-note").text()).toContain("怕打扰");

    // growth 组全部字段都有行（10 个标签）
    for (const label of [
      "产卵情况",
      "卵到工蚁",
      "首批工蚁",
      "幼体成活",
      "第一年规模",
      "第二年规模",
      "成熟规模",
      "品级分化",
      "蚁后寿命",
      "工蚁寿命",
    ]) {
      expect(rowValue(detail, label)).not.toBe("");
    }
    expect(rowValue(detail, "蚁后寿命")).toContain("10 年以上");
  });

  it("空字段显示「暂无资料」：拟哀弓背蚁（大量查无实据）展开后 13 处留空位全部如实显示", async () => {
    const w = mountGuide();
    const detail = await expandCard(w, "拟哀弓背蚁");

    // 逐字段核对：查无实据的显示「暂无资料」，有据的显示真值
    expect(rowValue(detail, "湿度")).toBe("暂无资料");
    expect(rowValue(detail, "食性")).toBe("暂无资料");
    expect(rowValue(detail, "后制")).toBe("暂无资料");
    expect(rowValue(detail, "活动节律")).toBe("暂无资料");
    expect(rowValue(detail, "产卵情况")).toBe("暂无资料");
    expect(rowValue(detail, "幼体成活")).toBe("暂无资料");
    expect(rowValue(detail, "工蚁寿命")).toBe("暂无资料");
    expect(rowValue(detail, "越狱倾向")).toBe("暂无资料");
    // 有据字段不受影响
    expect(rowValue(detail, "分布")).toContain("四川");
    expect(rowValue(detail, "蚁后寿命")).toBe("暂无资料");

    // 拟哀档案 14 个空字段中 14 处按行渲染（体型行因工蚁有据合并显示）→ 至少 13 处「暂无资料」
    const noneCount = (detail.text().match(/暂无资料/g) ?? []).length;
    expect(noneCount).toBeGreaterThanOrEqual(13);
  });

  it("购买提醒与 careNotes 逐条渲染", async () => {
    const w = mountGuide();
    const tk = await expandCard(w, "中亚弓背蚁");
    expect(tk.find(".buy-warning").text()).toContain("认准学名");

    const nl = await expandCard(w, "拟哀弓背蚁");
    expect(nl.findAll(".care-notes li").length).toBe(2);
  });
});

describe("图鉴页 · 通用饲养知识", () => {
  it("七个主题都在（糖水怎么兑/白水与糖水/交哺原理/产卵膏拆穿/蛋白质/报警信号/新后到手两周）", () => {
    const w = mountGuide();
    const kb = w.find(".kb-section");
    expect(kb.exists()).toBe(true);
    const topics = kb.findAll(".kb-topic h3").map((h) => h.text());
    expect(topics.some((t) => t.includes("糖水怎么兑"))).toBe(true);
    expect(topics.some((t) => t.includes("白水") && t.includes("糖水"))).toBe(true);
    expect(topics.some((t) => t.includes("交哺"))).toBe(true);
    expect(topics.some((t) => t.includes("产卵膏"))).toBe(true);
    expect(topics.some((t) => t.includes("蛋白质"))).toBe(true);
    expect(topics.some((t) => t.includes("报警信号"))).toBe(true);
    expect(topics.some((t) => t.includes("新后到手两周"))).toBe(true);
  });

  it("关键口径在位：蜂蜜 1:5~8/宁稀勿浓/禁代糖/凉白开、糖水不进巢、社会胃、产卵膏拆穿、冻干/FD、报警表、遮黑静置 3 天", () => {
    const w = mountGuide();
    const text = w.find(".kb-section").text();
    for (const kw of [
      "1:5~8",
      "宁稀勿浓",
      "代糖",
      "凉白开",
      "糖水永远不进巢",
      "社会胃",
      "助产卵",
      "冻干",
      "FD",
      "烘干",
      "蚁后带幼虫搬家",
      "遮黑静置 3 天",
      "忍住不观察",
      "开箱视频",
      "少折腾",
    ]) {
      expect(text).toContain(kw);
    }
  });
});

describe("图鉴页 · 隐私清洗", () => {
  it("页面全文零隐私字样（桂林/办公室/已养三窝/共用加热垫/长假带回家）", () => {
    const w = mountGuide();
    const text = w.text();
    for (const banned of ["桂林", "办公室", "已养三窝", "共用加热垫", "长假带回家"]) {
      expect(text).not.toContain(banned);
    }
  });
});
