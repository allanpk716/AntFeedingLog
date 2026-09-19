/**
 * 与 Rust 端 DTO 对齐的类型（字段 snake_case，serde 默认行为）。
 * 见 src-tauri/src/colony.rs。
 */

export type ColonyStatus = "active" | "hibernating" | "ended";

/** 维护操作性质：reminding=提醒（超期标红可通知）/ log_only=仅登记（永不催促） */
export type ActionKind = "reminding" | "log_only";

/** 喂食块里单个食物的「距上次」明细（Rust care::FoodTileStatus，反馈第二轮 F3） */
export interface FoodTileInfo {
  food_id: number;
  name: string;
  /** 该食物自己的建议间隔；null = 未设，只受喂食统一周期管 */
  suggested_interval_days: number | null;
  /** 今天 − 最近一次喂「该食物」日期（含出眠重置基线）；从未喂过且无出眠史为 null */
  days_since_last: number | null;
  /** 仅操作 kind=reminding 且已设周期且 > 周期 */
  overdue: boolean;
}

/** 窝卡片上单个操作块（Rust 已算好距上次/超期态） */
export interface ColonyAction {
  action_id: number;
  name: string;
  icon: string | null;
  kind: ActionKind;
  /** 是否喂食类操作（schema 标记位，决定记账时是否带食物多选；与名字无关） */
  is_feeding: boolean;
  suggested_interval_days: number | null;
  /** 今天 − 最近一次发生日期（自然日）；从未记录为 null */
  days_since_last: number | null;
  /** 仅提醒类且 > 建议间隔；喂食类任一设周期食物超期也算（F3，Q2） */
  overdue: boolean;
  /** 逐食物「距上次」明细（F3）；仅喂食类非空，其余操作恒空数组 */
  foods: FoodTileInfo[];
}

/** 最近记录摘要的一行（前端拼展示文案） */
export interface RecentLog {
  happened_at: string;
  action_name: string;
  food_names: string[];
}

/** 冬眠段（list_hibernations 行；actual_end_date 为 null = 开放段） */
export interface HibernationSegment {
  id: number;
  colony_id: number;
  start_date: string;
  expected_end_date: string;
  actual_end_date: string | null;
}

/** 开放段摘要（Colony.hibernation，冬眠卡片横幅数据；无开放段为 null） */
export interface HibernationPreview {
  id: number;
  start_date: string;
  expected_end_date: string;
}

/** 窝（Rust 已算好饲养天数、操作块、最近摘要） */
export interface Colony {
  id: number;
  name: string;
  species: string | null;
  location_id: number | null;
  start_date: string;
  status: ColonyStatus;
  days_raised: number;
  actions: ColonyAction[];
  recent: RecentLog[];
  hibernation: HibernationPreview | null;
  /** 巢况摘要（webui-checkin 票 02）：最新一组数 + 基线 + 距上次登记天数 */
  checkin: CheckinDigest;
}

/** 新建/编辑窝入参 */
export interface ColonyInput {
  name: string;
  species: string | null;
  location_id: number | null;
  start_date: string;
  status: ColonyStatus;
}

/** 地点（含停用的：首页分组仍按它排） */
export interface LocationItem {
  id: number;
  name: string;
  enabled: boolean;
  sort: number;
}

/** 新增(id=null)/修改(id=有值) 地点入参；停用/删除走单独命令 */
export interface LocationInput {
  id: number | null;
  name: string;
  sort: number;
}

/** 食物（含停用的：新建入口前端过滤 enabled；referenced=被历史记录或提醒台账引用，只能停用不能删） */
export interface FoodItem {
  id: number;
  name: string;
  enabled: boolean;
  sort: number;
  /** 食物建议间隔（天，F3）：距上次喂该食物超过它就单独提醒；null = 未设 */
  suggested_interval_days: number | null;
  /** 预置项禁删可停用（反馈第二轮 F2） */
  is_preset: boolean;
  referenced: boolean;
}

/** 维护操作字典项（含停用的；referenced=被记录/提醒台账引用，只能停用不能删） */
export interface CareActionItem {
  id: number;
  name: string;
  icon: string | null;
  kind: ActionKind;
  /** 是否喂食类操作（可编辑、不强制全局唯一，界面提示自理） */
  is_feeding: boolean;
  suggested_interval_days: number | null;
  enabled: boolean;
  sort: number;
  /** 预置项禁删可停用（反馈第二轮 F2） */
  is_preset: boolean;
  referenced: boolean;
}

/** 新增(id=null)/修改(id=有值) 操作入参；停用/删除走单独命令 */
export interface ActionInput {
  id: number | null;
  name: string;
  kind: ActionKind;
  is_feeding: boolean;
  suggested_interval_days: number | null;
  sort: number;
}

/** set_action_policy 入参：性质 + 建议间隔（null=保留现值）+ 可选 is_feeding（null=不改） */
export interface ActionPolicyInput {
  kind: ActionKind;
  suggested_interval_days: number | null;
  is_feeding: boolean | null;
}

/** 新增(id=null)/修改(id=有值) 食物入参；停用/删除走单独命令 */
export interface FoodInput {
  id: number | null;
  name: string;
  sort: number;
  /** 食物建议间隔（天，F3）；null = 不设 */
  suggested_interval_days: number | null;
}

/** 记账入参：喂食才带 food_ids（其余操作传空数组）；happened_at 可补录过去 */
export interface CareLogInput {
  colony_id: number;
  action_id: number;
  happened_at: string;
  note: string | null;
  food_ids: number[];
}

/** 记录列表筛选入参（list_logs；各筛选项可空、组合生效；start/end 为 ISO 日期） */
export interface LogFilter {
  colony_id: number | null;
  action_id: number | null;
  start: string | null;
  end: string | null;
  note_keyword: string | null;
  limit: number | null;
  offset: number | null;
}

/** 记录流水一行（食物按字典顺序；停用操作/食物照常返回显示名，规则 10） */
export interface LogRow {
  id: number;
  colony_id: number;
  colony_name: string;
  action_id: number;
  action_name: string;
  occurred_at: string;
  /** 录入时间（与发生时间分开存，补录合法） */
  created_at: string;
  note: string;
  food_ids: number[];
  food_names: string[];
}

/** list_logs 返回体：一页行 + 命中总数（total 不随分页变） */
export interface LogPage {
  total: number;
  rows: LogRow[];
}

/** 编辑记录入参：字段 null = 保持原值；food_ids 传空数组 = 清空食物关联 */
export interface LogUpdateInput {
  occurred_at: string | null;
  note: string | null;
  action_id: number | null;
  food_ids: number[] | null;
}

/** 统计页 payload（Rust stats::StatsPayload；spec API 契约 getStats） */
export interface StatsDailyCount {
  date: string;
  count: number;
}

/** 热力图悬停明细里的一天内单条记录（喂食带食物名，其他为空数组） */
export interface StatsDayEntry {
  action_name: string;
  food_names: string[];
}

/** 悬停明细按天分组（只含有记录的天） */
export interface StatsDayDetail {
  date: string;
  entries: StatsDayEntry[];
}

/** 食物出现次数（分母 = 各项合计，前端归一化 100%，规则 8） */
export interface StatsFoodShare {
  food_name: string;
  occurrences: number;
}

/** 每周操作数（周一为周首） */
export interface StatsWeeklyCount {
  week_start: string;
  count: number;
}

/** 单个操作间隔统计（间隔已扣冬眠重叠天数，规则 6） */
export interface StatsInterval {
  action_id: number;
  name: string;
  kind: ActionKind;
  /** 仅提醒类有值（前端画建议刻度竖线）；登记类恒 null（界面标「仅登记」） */
  suggested_interval_days: number | null;
  sample_count: number;
  avg_days: number | null;
  min_days: number | null;
  max_days: number | null;
}

/** get_stats 返回体 */
export interface StatsPayload {
  range_start: string;
  range_end: string;
  /** 规则 7：频率分母 = 范围自然日天数（含首尾，不扣冬眠） */
  range_days: number;
  daily: StatsDailyCount[];
  daily_detail: StatsDayDetail[];
  food_share: StatsFoodShare[];
  weekly: StatsWeeklyCount[];
  intervals: StatsInterval[];
}

export const COLONY_STATUS_LABELS: Record<ColonyStatus, string> = {
  active: "活跃",
  hibernating: "冬眠",
  ended: "已结束",
};

/** 设置模型（Rust settings::AppSettings；落 settings 键值表） */
export interface AppSettings {
  /** 通知总开关 */
  notify_master_enabled: boolean;
  /** 超期提醒开关 */
  notify_overdue_enabled: boolean;
  /** 冬眠提醒开关（临近出眠/出眠日） */
  notify_hibernation_enabled: boolean;
  /** 临近出眠提前天数（0–365；0=出眠日当天才提醒） */
  wake_remind_days_ahead: number;
  /** 开机自启（票 09：通知 tab 开关随保存落库，Rust 同步自启插件状态） */
  autostart_enabled: boolean;
  /** Pushover 用户键（票 11；空串 = 应用内未填，发送侧回落环境变量） */
  pushover_user: string;
  /** Pushover 应用令牌（票 11；空串 = 未填） */
  pushover_token: string;
}

/** pushover_status 返回体（票 11 三态）：生效来源 + 是否已配置（不含值） */
export type PushoverSource = "app" | "env" | "none";
export interface PushoverStatus {
  source: PushoverSource;
  configured: boolean;
}

/** send_test_notification 返回体：分渠道结果（pushover=null 表示未配置） */
export interface PushoverTestResult { ok: boolean; error: string | null; }
export interface TestNotifyOutcome {
  desktop_ok: boolean;
  desktop_error: string | null;
  pushover: PushoverTestResult | null;
}

/** get_last_abnormal_exit 返回体（数据安全二期票 01）；null = 上次正常退出 */
export interface AbnormalExitInfo {
  /** 异常会话的启动时间（运行标记写入时间）；标记损坏时 null（时间未知） */
  session_started_at: string | null;
  /** 原因行（上次会话的 panic 日志）；null = 无崩溃日志，疑强杀/断电 */
  reason: string | null;
}

/** 上次备份结果（backup-config.json 内嵌；null = 尚未备份过） */
export interface LastBackupOutcome {
  /** true=成功 / false=失败 */
  ok: boolean;
  /** 结果发生时间（YYYY-MM-DD HH:MM:SS） */
  at: string;
  /** 失败原因（成功为 null） */
  reason: string | null;
}

/** 备份配置（Rust backup_config::BackupConfig；落数据目录 backup-config.json，
 * 不进 SQLite——D1：恢复整库不回滚备份设置） */
export interface BackupConfigInfo {
  /** 自动备份开关（默认开） */
  enabled: boolean;
  /** 备份目录；null = 未设（未设时自动备份不生效） */
  backup_dir: string | null;
  /** 保留份数 1–365（默认 30） */
  keep_count: number;
  /** 最后成功备份日期（YYYY-MM-DD；票 03 接真数据） */
  last_backup_date: string | null;
  /** 最后业务写入日期（YYYY-MM-DD；票 03 接真数据） */
  last_data_write_date: string | null;
  /** 上次备份结果与时间；null = 尚未备份 */
  last_result: LastBackupOutcome | null;
}

/** set_backup_config 入参：只含用户可改的三项（账目字段 Rust 侧维护，前端不可覆写） */
export interface BackupConfigInput {
  enabled: boolean;
  backup_dir: string | null;
  keep_count: number;
}

/** restore_preview 返回体（数据安全二期票 04，Rust restore::RestoreSummary）：
 * 摘要预览是选错文件的最后防线（spec D6） */
export interface RestoreSummary {
  /** 备份日期（YYYY-MM-DD）：优先文件名时间戳，否则库内最新记录日期；null = 空库 */
  backup_date: string | null;
  /** 备份内窝数（0 窝 0 条 = 可能选错文件的信号，界面如实展示） */
  colony_count: number;
  /** 备份内记录数 */
  log_count: number;
  /** 备份内的备份目录设置值（settings 表旧布局才有；null = 备份内无此设置，
   * 备份设置存库外不随恢复回滚——D1） */
  backup_dir_in_backup: string | null;
  /** 备份内照片张数（webui-checkin 票 10）：数据包 = 包内清单张数；
   * 0 = 裸库或包内零照片，界面标注「不含照片」 */
  photo_count: number;
}

/** restore_apply 返回体：done = 界面当场刷新；done_needs_restart = 新库文件已
 * 就位但重开连接失败，提示「请重启应用」 */
export type RestoreApplyOutcome = "done" | "done_needs_restart";

// ── 巢况登记（webui-checkin 票 02，Rust nest_checkin.rs）──

/** 照片元数据一行（本票恒空数组：写入随票 07 照片管线接线） */
export interface NestPhotoMeta {
  id: number;
  checkin_id: number;
  /** photos/ 下相对路径 `<colonyId>/<uuid>.jpg` */
  rel_path: string;
  /** 客户端原始文件名，仅备注 */
  original_name: string | null;
  note: string;
}

/** 巢况登记一行（时间线按日期倒序返回） */
export interface NestCheckin {
  id: number;
  colony_id: number;
  /** 登记日期 YYYY-MM-DD（可补录过去） */
  date: string;
  /** null = 未数 */
  queen_count: number | null;
  worker_count: number | null;
  moved_nest: boolean;
  note: string;
  created_at: string;
  photos: NestPhotoMeta[];
}

/** 新增巢况入参（至少一项非空才可提交，后端兜底校验） */
export interface CheckinInput {
  colony_id: number;
  date: string;
  queen_count: number | null;
  worker_count: number | null;
  moved_nest: boolean;
  note: string | null;
}

/** 编辑巢况入参：全量覆盖（数可清回 null，日期必填） */
export interface CheckinUpdateInput {
  date: string;
  queen_count: number | null;
  worker_count: number | null;
  moved_nest: boolean;
  note: string | null;
}

/** 窝卡片/详情的巢况摘要（Rust 算好；从未登记三者皆 null） */
export interface CheckinDigest {
  latest: NestCheckin | null;
  /** 基线 = 最早一条登记的日期字段 */
  baseline_date: string | null;
  /** 距上次登记 = 今天 − 最新登记日期（自然日，当天 0） */
  days_since_last: number | null;
}

// ── 巢况照片（webui-checkin 票 07，Rust photo.rs；桌面专属命令）──

/** list_orphan_photos 返回体：photos/.orphan-* 隔离区现状 */
export interface OrphanPhotoStats {
  dir_count: number;
  file_count: number;
  total_bytes: number;
}

/** clean_orphan_photos 返回体：删除量与释放字节；errors 非空 = 个别目录删除失败 */
export interface OrphanCleanOutcome {
  removed_dirs: number;
  freed_bytes: number;
  errors: string[];
}

// ── 网页端设置（webui-checkin 票 03，Rust netseg.rs / webui_config.rs）──

/** 本机网段一行（NetBird 段已置顶标名；物理网段 encrypted_mesh=false 走硬警示） */
export interface NetworkSegment {
  /** 归一后的 CIDR（主机位归零，如 192.168.1.0/24） */
  cidr: string;
  /** true = 接口身份已确认是加密 mesh（NetBird）；物理网段（含 CGNAT 地址）恒 false */
  encrypted_mesh: boolean;
  /** 置顶标名（仅 NetBird 段有，如「NetBird 虚拟网」） */
  label: string | null;
}

/** 网页端配置（数据目录 webui-config.json，库外文件恢复不触碰；token 只程序生成） */
export interface WebUiConfigInfo {
  enabled: boolean;
  /** 受信网段（归一 CIDR，闸一白名单） */
  segments: string[];
  /** 服务端口 1024–65535（默认 17321） */
  port: number;
  /** 访问凭证（32 位十六进制 = 128bit；空串 = 尚未生成） */
  token: string;
  token_generated_at: string | null;
}

/** save_webui_config 入参：用户可改的三项（凭证不可覆写，只可重生成） */
export interface WebUiSaveInput {
  enabled: boolean;
  segments: string[];
  port: number;
}

/** save_webui_config 返回体（半成功语义）：配置已落盘 + 防火墙同步结果与服务
 * 起停结果分开回传，失败时各自附人话原因（防火墙附现成 netsh 手动命令） */
export interface WebUiSaveOutcome {
  config: WebUiConfigInfo;
  firewall_ok: boolean;
  firewall_error: string | null;
  firewall_manual_cmd: string | null;
  /** 网页端服务按新配置对齐成功（含「停用即关停」） */
  server_ok: boolean;
  /** 服务起停失败的人话原因（端口占用等）；null = 正常 */
  server_error: string | null;
}
