<script setup lang="ts">
/**
 * 图鉴页（物种档案票 04）：双端纯前端内容页——数据全部来自前端档案包 loader
 * （speciesProfiles，构建期 glob 导入），无任何页面级 IPC，内存数据即时渲染
 * 不进加载态（CLAUDE.md 加载态规范）。结构三段（D6 单页装下）：
 * ① 新手推荐区：beginnerRecommended 档案按冬眠/不冬眠两路线陈列（D5，浅冬眠
 *    归冬眠路线），下方避坑提醒——排除 7 种各一句原因 + 香斑弓背蚁点名（D2/D16）；
 * ② 全部物种档案：类型 distinct 动态分组（D3 禁写死枚举），每物种一张可展开卡，
 *    空字符串字段如实显示「暂无资料」（D17 绝不编数字），难度★，惊吓敏感度
 *    新后/成群两档（D18）；
 * ③ 通用饲养知识：与物种无关的共通照看知识写死在本组件、不进档案包（D10），
 *    内容取自调研存档与 proposal §3.5，已按 §6 规则过隐私清洗（无个人位置/
 *    场景，本地化结论改条件式）。
 */
import { ref } from "vue";
import {
  allSpeciesProfiles,
  speciesTypeGroups,
  type SpeciesProfile,
} from "../lib/speciesProfiles";

const profiles = allSpeciesProfiles();
const groups = speciesTypeGroups(profiles);

/** 推荐位按路线陈列（D5）：冬眠/浅冬眠归「冬眠路线」，不冬眠归「不冬眠路线」；
 *  路线内难度低的排前面（同为入门种，先看最省心的），同难度按 loader 稳定序。 */
function byDifficultyThenKey(a: SpeciesProfile, b: SpeciesProfile): number {
  if (a.difficulty !== b.difficulty) return a.difficulty - b.difficulty;
  return a.key < b.key ? -1 : a.key > b.key ? 1 : 0;
}
const dormantRoute = profiles
  .filter((p) => p.beginnerRecommended && p.hibernation !== "不冬眠")
  .sort(byDifficultyThenKey);
const nonDormantRoute = profiles
  .filter((p) => p.beginnerRecommended && p.hibernation === "不冬眠")
  .sort(byDifficultyThenKey);

/** 避坑提醒：排除的 7 种不做档案不可选（D2），只在此以文字提及；香斑点名单列
 *  （D16「暴毙天王」）。文案提炼自调研存档，已过隐私清洗。 */
const AVOID_LIST: Array<{ name: string; reason: string }> = [
  {
    name: "窄颈弓背蚁",
    reason: "体型天花板（工蚁可到 19mm）但属低温蚁怕热，夏天没有空调的房间是死线，新后还神经质",
  },
  {
    name: "墨西哥蜜罐蚁",
    reason: "观赏天花板，但新后成活率不足 25%、发展慢、价格贵——建议两三年经验后再上",
  },
  {
    name: "红头弓背蚁",
    reason: "不冬眠但发展慢，帅气的兵蚁要两三年才分化出来（注意与推荐的「红头收获蚁」一字之差，别买混）",
  },
  { name: "拟黑多刺蚁", reason: "体型小、爱越狱" },
  { name: "黄猄蚁", reason: "凶猛爱越狱，还需要树栖巢" },
  { name: "红足修猛蚁", reason: "皮实胆大但偏肉食，糖水对它只是零食" },
  { name: "巨人恐蚁", reason: "几千元一窝加发展极慢，发烧友专属" },
];

/** 展开的档案卡 key 集合（多开互不影响；页面保活时会话内保留展开状态）。 */
const expandedKeys = ref(new Set<string>());
function toggle(key: string): void {
  const next = new Set(expandedKeys.value);
  if (next.has(key)) next.delete(key);
  else next.add(key);
  expandedKeys.value = next;
}

/** 难度★：1~3 星实心星（D16/D18，档案数值域 1~3）。 */
function stars(n: number): string {
  return "★".repeat(n);
}

/** 空字符串 = 查无实据 → 「暂无资料」（D17），绝不编数字。 */
function orNone(v: string): string {
  return v === "" ? "暂无资料" : v;
}

/** 体型行：蚁后/工蚁两段拼一行，两边都查无实据才整行「暂无资料」。 */
function bodySize(p: SpeciesProfile): string {
  const parts: string[] = [];
  if (p.queenSize !== "") parts.push(`蚁后 ${p.queenSize}`);
  if (p.workerSize !== "") parts.push(`工蚁 ${p.workerSize}`);
  return parts.length > 0 ? parts.join("；") : "暂无资料";
}

/** 冬眠需求行：三值口径 + 细则注记。 */
function hibernationText(p: SpeciesProfile): string {
  return p.hibernationNote !== "" ? `${p.hibernation}——${p.hibernationNote}` : p.hibernation;
}

interface Row {
  label: string;
  value: string;
}

/** 饲养参数（温度/湿度/食性/体型/后制/活动节律/冬眠/分布/外观/价格）。 */
function careRows(p: SpeciesProfile): Row[] {
  return [
    { label: "温度", value: orNone(p.tempRange) },
    { label: "湿度", value: orNone(p.humidity) },
    { label: "食性", value: orNone(p.diet) },
    { label: "体型", value: bodySize(p) },
    { label: "后制", value: orNone(p.queenSystem) },
    { label: "活动节律", value: orNone(p.activityRhythm) },
    { label: "冬眠需求", value: hibernationText(p) },
    { label: "分布", value: orNone(p.distribution) },
    { label: "外观", value: orNone(p.appearance) },
    { label: "价格", value: orNone(p.price) },
  ];
}

/** 发展预期（growth 组全部 10 字段）。 */
function growthRows(p: SpeciesProfile): Row[] {
  return [
    { label: "产卵情况", value: orNone(p.growth.eggLaying) },
    { label: "卵到工蚁", value: orNone(p.growth.eggToWorkerDays) },
    { label: "首批工蚁", value: orNone(p.growth.firstWorkersDays) },
    { label: "幼体成活", value: orNone(p.growth.broodSurvival) },
    { label: "第一年规模", value: orNone(p.growth.firstYearSize) },
    { label: "第二年规模", value: orNone(p.growth.secondYearSize) },
    { label: "成熟规模", value: orNone(p.growth.matureSize) },
    { label: "品级分化", value: orNone(p.growth.castes) },
    { label: "蚁后寿命", value: orNone(p.growth.queenLifespan) },
    { label: "工蚁寿命", value: orNone(p.growth.workerLifespan) },
  ];
}

/** 风险敏感度数值档之外的文字行（越狱/健康/温度红线）。 */
function riskRows(p: SpeciesProfile): Row[] {
  return [
    { label: "越狱倾向", value: orNone(p.risks.escapeTendency) },
    { label: "健康风险", value: orNone(p.risks.healthRisks) },
    { label: "温度红线", value: orNone(p.risks.tempRedline) },
  ];
}
</script>

<template>
  <main class="species-guide">
    <!-- ① 新手推荐区：4 推荐位按两路线陈列 + 避坑提醒 -->
    <section class="rec-section">
      <h2>新手推荐</h2>
      <p class="rec-lead">
        第一次买蚁，先按「冬天要不要管它」选路线；四个推荐位都是皮实、资料齐全的入门种。
      </p>
      <div class="rec-routes">
        <div class="rec-route">
          <h3>
            冬眠路线
            <span class="route-note">冬季停产休整；室温能自然降到 5~10°C 的，扔冷角落即可零设备越冬</span>
          </h3>
          <div class="rec-cards">
            <article v-for="p in dormantRoute" :key="p.key" class="rec-card">
              <header class="rec-head">
                <span class="rec-name">{{ p.cnName }}</span>
                <span class="rec-stars" :title="`饲养难度 ${p.difficulty} 星（1~3）`">{{ stars(p.difficulty) }}</span>
              </header>
              <p class="rec-latin">{{ p.latinName }} · {{ p.hibernation }}</p>
              <p class="rec-reason">{{ p.recommendReason }}</p>
            </article>
          </div>
        </div>
        <div class="rec-route">
          <h3>
            不冬眠路线
            <span class="route-note">全年活跃，冬天也有得看；需要加温设备兜底</span>
          </h3>
          <div class="rec-cards">
            <article v-for="p in nonDormantRoute" :key="p.key" class="rec-card">
              <header class="rec-head">
                <span class="rec-name">{{ p.cnName }}</span>
                <span class="rec-stars" :title="`饲养难度 ${p.difficulty} 星（1~3）`">{{ stars(p.difficulty) }}</span>
              </header>
              <p class="rec-latin">{{ p.latinName }} · {{ p.hibernation }}</p>
              <p class="rec-reason">{{ p.recommendReason }}</p>
            </article>
          </div>
        </div>
      </div>
      <div class="avoid">
        <h3>避坑提醒——这些不建议新手碰</h3>
        <ul class="avoid-list">
          <li v-for="a in AVOID_LIST" :key="a.name">
            <b>{{ a.name }}</b>——{{ a.reason }}
          </li>
          <li class="avoid-callout">
            <b>香斑弓背蚁</b>——中文社区公认的「暴毙天王」，暴毙率高、新手翻车多（多源一致），建议有经验后再碰。
          </li>
        </ul>
      </div>
    </section>

    <!-- ② 全部物种档案：类型 distinct 分组，逐卡展开 -->
    <section class="all-section">
      <h2>全部物种档案</h2>
      <div v-for="g in groups" :key="g.type" class="guide-group">
        <h3 class="group-title">
          {{ g.type }}
          <span class="cnt">{{ g.profiles.length }} 种</span>
        </h3>
        <article
          v-for="p in g.profiles"
          :key="p.key"
          class="sp-card"
          :class="{ open: expandedKeys.has(p.key) }"
        >
          <button
            class="sp-head"
            type="button"
            :aria-expanded="expandedKeys.has(p.key)"
            @click="toggle(p.key)"
          >
            <span class="sp-name">{{ p.cnName }}</span>
            <span class="sp-latin">{{ p.latinName }}</span>
            <span class="sp-chips">
              <span class="chip">{{ p.type }}</span>
              <span class="chip hib">{{ p.hibernation }}</span>
              <span class="sp-stars" :title="`饲养难度 ${p.difficulty} 星（1~3）`">{{ stars(p.difficulty) }}</span>
            </span>
            <span class="sp-caret">{{ expandedKeys.has(p.key) ? "▲ 收起" : "▼ 展开" }}</span>
          </button>

          <div v-if="expandedKeys.has(p.key)" class="sp-detail">
            <div class="sp-params">
              <h4>饲养参数</h4>
              <dl class="rows">
                <div v-for="r in careRows(p)" :key="r.label" class="row">
                  <dt>{{ r.label }}</dt>
                  <dd :class="{ none: r.value === '暂无资料' }">{{ r.value }}</dd>
                </div>
              </dl>
            </div>

            <div class="sp-growth">
              <h4>发展预期</h4>
              <dl class="rows">
                <div v-for="r in growthRows(p)" :key="r.label" class="row">
                  <dt>{{ r.label }}</dt>
                  <dd :class="{ none: r.value === '暂无资料' }">{{ r.value }}</dd>
                </div>
              </dl>
            </div>

            <div class="sp-risks">
              <h4>风险敏感度</h4>
              <div class="stress-tiers">
                <span class="stress-tier">惊吓敏感 · 新后 <b>{{ p.risks.stressNew }}/5</b></span>
                <span class="stress-tier">惊吓敏感 · 成群 <b>{{ p.risks.stressColony }}/5</b></span>
                <span class="stress-scale">（1~5 档，5 = 极度敏感）</span>
              </div>
              <p class="stress-note" :class="{ none: orNone(p.risks.stressNote) === '暂无资料' }">
                {{ orNone(p.risks.stressNote) }}
              </p>
              <dl class="rows">
                <div v-for="r in riskRows(p)" :key="r.label" class="row">
                  <dt>{{ r.label }}</dt>
                  <dd :class="{ none: r.value === '暂无资料' }">{{ r.value }}</dd>
                </div>
              </dl>
            </div>

            <div class="buy-warning">
              <h4>购买提醒</h4>
              <p :class="{ none: orNone(p.purchaseWarning) === '暂无资料' }">{{ orNone(p.purchaseWarning) }}</p>
            </div>

            <div v-if="p.careNotes.length > 0" class="care-notes">
              <h4>补充说明</h4>
              <ul>
                <li v-for="(note, i) in p.careNotes" :key="i">{{ note }}</li>
              </ul>
            </div>
          </div>
        </article>
      </div>
    </section>

    <!-- ③ 通用饲养知识：与物种无关的共通内容，写死在本组件（D10 不进档案包）。
         内容取自调研存档与 proposal §3.5，已过隐私清洗（无个人位置/场景）。 -->
    <section class="kb-section">
      <h2>通用饲养知识</h2>
      <p class="kb-lead">与具体物种无关的共通照看知识，日常照着做即可。</p>

      <article class="kb-topic">
        <h3>糖水怎么兑</h3>
        <ul>
          <li>蜂蜜兑水 <b>1:5~8</b> 最佳：自带抑菌，喂水器里放 2~3 周不坏，最省心</li>
          <li>白砂糖 1:5 最便宜，但一周多易发酵、要换得勤；冰糖成分同砂糖、化得慢，没必要特意用</li>
          <li>水用<b>凉白开</b>；<b>宁稀勿浓</b>——浓糖水黏稠，容易溺蚁</li>
          <li><b>禁用代糖</b>（木糖醇、赤藓糖醇）：蚂蚁代谢不了</li>
          <li>一次兑 30~50ml，密封冷藏可存一个月</li>
        </ul>
      </article>

      <article class="kb-topic">
        <h3>白水与糖水的区别</h3>
        <ul>
          <li>白水是命：巢体水塔 + 活动区饮水器常备，一周看两次水位</li>
          <li>糖水是饭：放活动区喂水器（棉花堵头防溺），位置选巢门口附近、背阴角落；蜂蜜版两周补一次</li>
          <li>糖水不能替代白水：蚂蚁调湿度、喂幼虫用的是纯水</li>
          <li><b>糖水永远不进巢</b>：打翻只污染活动区，不污染巢体</li>
          <li>水发浑、有酒味 = 发酵，立刻换</li>
        </ul>
      </article>

      <article class="kb-topic">
        <h3>交哺原理——幼蚁怎么吃到糖水</h3>
        <p>
          工蚁喝下糖水存进「<b>社会胃</b>」，回巢嘴对嘴反刍喂给蚁后、幼虫和新工蚁——这叫「交哺」。
          幼虫一辈子不出巢，全靠工蚁送饭。活动区那点距离对蚂蚁约等于人下楼拿快递，放哪都送得到；
          小群落唯一要注意的是别接太大的活动区。
        </p>
      </article>

      <article class="kb-topic">
        <h3>「产卵膏」是什么——不用买</h3>
        <ul>
          <li>本质是膏状复合饲料（糖 + 蛋白粉 + 维生素），「助产卵」只是商家卖点</li>
          <li>营养上蜂蜜水 + 冻干虫完全覆盖</li>
          <li>膏是湿食，放一两天就发霉、必须当天清残渣——和「半月喂一次」的懒养路线正面冲突</li>
          <li>想给新后加餐可以偶尔买一小盒当零食，第二天没吃完就夹走</li>
        </ul>
      </article>

      <article class="kb-topic">
        <h3>蛋白质怎么喂</h3>
        <ul>
          <li>首选<b>冻干面包虫</b>：捏成 1cm 小段，干货不霉，两周扔 2~3 段，无需当天清渣</li>
          <li>认准「<b>冻干 / FD</b>」字样，别买成「烘干」——营养差还可能带油盐</li>
        </ul>
      </article>

      <article class="kb-topic">
        <h3>报警信号速查</h3>
        <table class="kb-table">
          <thead>
            <tr><th>现象</th><th>含义</th><th>处理</th></tr>
          </thead>
          <tbody>
            <tr><td>蚁后带幼虫搬家到试管/角落</td><td>巢体过干或过湿</td><td>检查加水槽</td></tr>
            <tr><td>巢壁永远挂大水珠</td><td>过湿</td><td>停加水，加强通风</td></tr>
            <tr><td>巢材发白干裂</td><td>过干</td><td>加水槽注水</td></tr>
            <tr><td>不产卵、幼虫不长</td><td>缺蛋白质或温度低</td><td>加冻干虫、查加温设备</td></tr>
            <tr><td>糖水发浑有酒味</td><td>发酵</td><td>立刻换</td></tr>
            <tr><td>工蚁腹部干瘪、活动迟缓</td><td>缺水</td><td>检查水塔和饮水器</td></tr>
          </tbody>
        </table>
      </article>

      <article class="kb-topic">
        <h3>新后到手两周</h3>
        <ol>
          <li><b>拍开箱视频</b>：维权凭证，顺带记录初始状态</li>
          <li>整巢<b>遮黑静置 3 天</b>：不看不碰</li>
          <li><b>第一周忍住不观察</b>：蚁后受惊会吃卵</li>
          <li>一周后进入正常喂食节奏</li>
          <li>铁律：<b>少折腾 = 养得活</b></li>
        </ol>
      </article>
    </section>
  </main>
</template>

<style scoped>
/* 版面：与首页 .container 同宽同级（本组件自带容器样式，不依赖父级 scoped 类） */
.species-guide {
  max-width: 1080px;
  margin: 0 auto;
  padding: 0 20px 40px;
}

.species-guide h2 {
  font-size: 17px;
  margin: 26px 0 8px;
}

.all-section h2,
.kb-section h2 {
  border-top: 1px solid var(--border);
  padding-top: 18px;
}

/* ── ① 新手推荐区 ── */

.rec-lead,
.kb-lead {
  color: var(--muted);
  font-size: 13px;
  margin: 0 0 10px;
}

.rec-routes {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(320px, 1fr));
  gap: 12px;
}

.rec-route h3 {
  font-size: 14px;
  margin: 4px 0 8px;
}

.route-note {
  margin-left: 6px;
  font-size: 12px;
  font-weight: 400;
  color: var(--muted);
}

.rec-cards {
  display: grid;
  gap: 10px;
}

.rec-card {
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: 12px;
  box-shadow: var(--shadow);
  padding: 10px 14px;
}

.rec-head {
  display: flex;
  align-items: baseline;
  gap: 8px;
}

.rec-name {
  font-size: 15px;
  font-weight: 700;
}

.rec-stars {
  color: var(--accent);
  letter-spacing: 2px;
  font-size: 13px;
}

.rec-latin {
  margin: 2px 0 4px;
  font-size: 12px;
  font-style: italic;
  color: var(--muted);
}

.rec-reason {
  margin: 0;
  font-size: 13px;
}

.avoid {
  margin-top: 16px;
  background: var(--bad-soft);
  border: 1px solid var(--border);
  border-radius: 12px;
  padding: 10px 14px;
}

.avoid h3 {
  font-size: 14px;
  margin: 0 0 6px;
  color: var(--bad);
}

.avoid-list {
  margin: 0;
  padding-left: 18px;
  font-size: 13px;
  display: grid;
  gap: 4px;
}

.avoid-callout {
  font-weight: 600;
}

/* ── ② 全部物种档案 ── */

.guide-group {
  margin-top: 14px;
}

.group-title {
  font-size: 14px;
  display: flex;
  align-items: center;
  gap: 8px;
}

.group-title .cnt {
  font-size: 12px;
  font-weight: 400;
  color: var(--muted);
  background: var(--tile);
  border: 1px solid var(--border);
  padding: 1px 9px;
  border-radius: 999px;
}

.sp-card {
  margin-top: 8px;
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: 12px;
  box-shadow: var(--shadow);
  overflow: hidden;
}

.sp-card.open {
  border-color: var(--border-strong);
}

.sp-head {
  width: 100%;
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 9px 14px;
  border: none;
  background: transparent;
  font: inherit;
  color: inherit;
  text-align: left;
  cursor: pointer;
}

.sp-head:hover {
  background: var(--tile);
}

.sp-name {
  font-size: 14px;
  font-weight: 700;
  white-space: nowrap;
}

.sp-latin {
  font-size: 12px;
  font-style: italic;
  color: var(--muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.sp-chips {
  margin-left: auto;
  display: flex;
  align-items: center;
  gap: 6px;
  flex: none;
}

.chip {
  font-size: 12px;
  color: var(--muted);
  background: var(--tile);
  border: 1px solid var(--border);
  padding: 1px 8px;
  border-radius: 999px;
  white-space: nowrap;
}

.chip.hib {
  color: var(--hib);
  background: var(--hib-soft);
}

.sp-stars {
  color: var(--accent);
  letter-spacing: 2px;
  font-size: 13px;
}

.sp-caret {
  font-size: 12px;
  color: var(--muted);
  flex: none;
}

.sp-detail {
  border-top: 1px dashed var(--border);
  padding: 10px 14px 12px;
  display: grid;
  gap: 12px;
}

.sp-detail h4 {
  margin: 0 0 6px;
  font-size: 13px;
  color: var(--accent-deep);
}

.rows {
  margin: 0;
  display: grid;
  gap: 3px;
}

.row {
  display: grid;
  grid-template-columns: 84px 1fr;
  gap: 10px;
  font-size: 13px;
}

.row dt {
  color: var(--muted);
}

.row dd {
  margin: 0;
}

/* 「暂无资料」弱化显示：查无实据如实呈现但不抢视线（D17） */
.none {
  color: var(--muted);
  opacity: 0.75;
}

.stress-tiers {
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: 10px;
  font-size: 13px;
}

.stress-tier {
  background: var(--tile);
  border: 1px solid var(--border);
  border-radius: 8px;
  padding: 2px 10px;
}

.stress-tier b {
  color: var(--accent-deep);
}

.stress-scale {
  font-size: 12px;
  color: var(--muted);
}

.stress-note {
  margin: 6px 0 0;
  font-size: 13px;
}

.buy-warning {
  background: var(--accent-soft);
  border: 1px solid var(--border);
  border-radius: 10px;
  padding: 8px 12px;
}

.buy-warning h4 {
  color: var(--accent-deep);
}

.buy-warning p {
  margin: 0;
  font-size: 13px;
}

.care-notes ul {
  margin: 0;
  padding-left: 18px;
  font-size: 13px;
  display: grid;
  gap: 3px;
}

/* ── ③ 通用饲养知识 ── */

.kb-topic {
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: 12px;
  box-shadow: var(--shadow);
  padding: 10px 14px;
  margin-top: 10px;
}

.kb-topic h3 {
  font-size: 14px;
  margin: 0 0 6px;
}

.kb-topic ul,
.kb-topic ol {
  margin: 0;
  padding-left: 18px;
  font-size: 13px;
  display: grid;
  gap: 3px;
}

.kb-topic p {
  margin: 0;
  font-size: 13px;
}

.kb-table {
  width: 100%;
  border-collapse: collapse;
  font-size: 13px;
}

.kb-table th,
.kb-table td {
  border: 1px solid var(--border);
  padding: 4px 8px;
  text-align: left;
}

.kb-table th {
  background: var(--tile);
  color: var(--muted);
  font-weight: 600;
}

/* ── 手机竖屏（webui-checkin 票 08 先例）：≤480px 单列 + 收紧边距 ── */
@media (max-width: 480px) {
  .species-guide {
    padding: 0 12px 60px;
  }

  .rec-routes {
    grid-template-columns: 1fr;
  }

  .sp-head {
    flex-wrap: wrap;
  }

  .sp-latin {
    flex-basis: 100%;
    order: 3;
    white-space: normal;
  }

  .row {
    grid-template-columns: 72px 1fr;
  }
}
</style>
