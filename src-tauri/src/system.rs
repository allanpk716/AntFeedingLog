//! 系统级数据出口（票 09）：安全备份 / CSV·JSON 导出。
//!
//! 纯函数核心只吃 `&Connection` 或路径，与 Tauri/rfd 对话框解耦，cargo test 直接
//! 覆盖；对话框与 command 薄封装在 lib.rs（spec API 契约「系统」节）。
//!
//! 行为对齐 spec 评审附录规则 11：
//! - 备份 = Rust 拷贝库文件到用户选定目录（journal_mode=DELETE，拷贝即完整）；
//! - 导出 CSV/JSON 定位为归档带走；恢复走应用内恢复通道（restore.rs，
//!   数据安全二期票 04 整库替换），不再依赖手工拷回（ADR-0002 遗留口径同步）；
//! - CSV 带 UTF-8 BOM（Excel 直开不乱码），行数 = 记录数，食物名分号连接。

use std::path::Path;

use rusqlite::{params, Connection};
use serde_json::json;

/// UTF-8 BOM：Excel 识别无 BOM 的 CSV 时按本地编码猜，中文必乱码。
const UTF8_BOM: &str = "\u{FEFF}";

/// CSV 表头（票面口径：id,窝,操作,发生时间,录入时间,备注,食物）。
const CSV_HEADER: [&str; 7] = ["id", "窝", "操作", "发生时间", "录入时间", "备注", "食物"];

/// 巢况区表头（webui-checkin 票 02）：记录区之后空一行接巢况区，同文件同 BOM。
const CHECKIN_CSV_HEADER: [&str; 9] = [
    "巢况id", "窝", "日期", "蚁后数", "工蚁数", "换巢", "备注", "录入时间", "照片数",
];

// ── CSV ──────────────────────────────────────────────────────────────────

/// CSV 单字段转义：
/// 1. 公式注入防护——以 `=`/`+`/`-`/`@` 开头的值会被 Excel 当公式执行，加 `'`
///    前缀强制按文本处理（ODSF 惯例，终局评审定点修 1）；
/// 2. 含逗号/引号/换行才加引号，引号翻倍（RFC 4180）。
pub fn csv_escape(field: &str) -> String {
    let guarded = match field.chars().next() {
        Some('=') | Some('+') | Some('-') | Some('@') => format!("'{field}"),
        _ => field.to_string(),
    };
    if guarded.contains(',') || guarded.contains('"') || guarded.contains('\n') || guarded.contains('\r')
    {
        format!("\"{}\"", guarded.replace('"', "\"\""))
    } else {
        guarded
    }
}

/// 记录流水 → CSV 文本：BOM + 记录区表头 + 每记录一行（食物名分号连接，顺序按
/// 字典序）+ 空行 + 巢况区表头 + 每登记一行（webui-checkin 票 02）。所有字段过
/// csv_escape（公式注入防护沿用）。
pub fn build_csv(rows: &[crate::care::LogRow], checkins: &[CheckinExportRow]) -> String {
    let mut out = String::from(UTF8_BOM);
    out.push_str(&CSV_HEADER.join(","));
    out.push('\n');
    for r in rows {
        let line: Vec<String> = vec![
            csv_escape(&r.id.to_string()),
            csv_escape(&r.colony_name),
            csv_escape(&r.action_name),
            csv_escape(&r.occurred_at),
            csv_escape(&r.created_at),
            csv_escape(&r.note),
            csv_escape(&r.food_names.join(";")),
        ];
        out.push_str(&line.join(","));
        out.push('\n');
    }
    out.push('\n');
    out.push_str(&CHECKIN_CSV_HEADER.join(","));
    out.push('\n');
    for c in checkins {
        let count_or_empty = |n: Option<i64>| n.map(|v| v.to_string()).unwrap_or_default();
        let line: Vec<String> = vec![
            csv_escape(&c.id.to_string()),
            csv_escape(&c.colony_name),
            csv_escape(&c.date),
            csv_escape(&count_or_empty(c.queen_count)),
            csv_escape(&count_or_empty(c.worker_count)),
            csv_escape(if c.moved_nest { "是" } else { "否" }),
            csv_escape(&c.note),
            csv_escape(&c.created_at),
            csv_escape(&c.photo_count.to_string()),
        ];
        out.push_str(&line.join(","));
        out.push('\n');
    }
    out
}

// ── 备份 ─────────────────────────────────────────────────────────────────

/// 拷贝库文件到目标路径。journal_mode=DELETE 下无 -wal 伴生文件，
/// 配合调用方短暂拿锁挡住并发写，拷贝即完整（评审附录规则 11）。
pub fn backup_db_file(db_path: &Path, target: &Path) -> Result<(), String> {
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("创建备份目录失败: {e}"))?;
    }
    std::fs::copy(db_path, target)
        .map(|_| ())
        .map_err(|e| format!("备份拷贝失败: {e}"))
}

// ── 全量数据 ─────────────────────────────────────────────────────────────

/// 全量记录流水：复用 list_logs 的查询口径，分页循环拉全（绕开单页上限）。
pub fn all_log_rows(conn: &Connection) -> Result<Vec<crate::care::LogRow>, String> {
    const PAGE: i64 = 500;
    let mut out = Vec::new();
    let mut offset = 0i64;
    loop {
        let filter = crate::care::LogFilter {
            limit: Some(PAGE),
            offset: Some(offset),
            ..Default::default()
        };
        let page = crate::care::list_logs(conn, &filter)?;
        let got = page.rows.len() as i64;
        out.extend(page.rows);
        if got < PAGE {
            return Ok(out);
        }
        offset += got;
    }
}

/// 巢况导出一行（附窝名展示值与照片计数，归档自足可读；webui-checkin 票 02）。
pub struct CheckinExportRow {
    pub id: i64,
    pub colony_name: String,
    pub date: String,
    pub queen_count: Option<i64>,
    pub worker_count: Option<i64>,
    pub moved_nest: bool,
    pub note: String,
    pub created_at: String,
    pub photo_count: i64,
}

/// 全量巢况登记（日期倒序、同日按 id 倒序），JOIN colony 取窝名、子查询取照片数。
pub fn all_checkin_rows(conn: &Connection) -> Result<Vec<CheckinExportRow>, String> {
    let db_err = |e: rusqlite::Error| format!("数据库操作失败: {e}");
    let mut stmt = conn
        .prepare(
            "SELECT nc.id, c.name, nc.date, nc.queen_count, nc.worker_count, nc.moved_nest,
                    nc.note, nc.created_at,
                    (SELECT COUNT(*) FROM nest_photo np WHERE np.checkin_id = nc.id)
             FROM nest_checkin nc JOIN colony c ON c.id = nc.colony_id
             ORDER BY nc.date DESC, nc.id DESC",
        )
        .map_err(db_err)?;
    let rows = stmt
        .query_map([], |row| {
            Ok(CheckinExportRow {
                id: row.get(0)?,
                colony_name: row.get(1)?,
                date: row.get(2)?,
                queen_count: row.get(3)?,
                worker_count: row.get(4)?,
                moved_nest: row.get::<_, i64>(5)? != 0,
                note: row.get(6)?,
                created_at: row.get(7)?,
                photo_count: row.get(8)?,
            })
        })
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?;
    Ok(rows)
}

/// 全库数据结构化（JSON 导出主体）：
/// `{colonies, actions, foods, locations, logs, checkins, hibernations, settings}`。
/// 行取库内原始数据（logs/checkins 附带窝名等展示值，归档自足可读）。
pub fn json_dump(conn: &Connection) -> Result<serde_json::Value, String> {
    let db_err = |e: rusqlite::Error| format!("数据库操作失败: {e}");

    let colonies: Vec<serde_json::Value> = {
        let mut stmt = conn
            .prepare(
                "SELECT id, name, species, location_id, start_date, status, sort
                 FROM colony ORDER BY id",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map([], |row| {
                Ok(json!({
                    "id": row.get::<_, i64>(0)?,
                    "name": row.get::<_, String>(1)?,
                    "species": row.get::<_, Option<String>>(2)?,
                    "location_id": row.get::<_, Option<i64>>(3)?,
                    "start_date": row.get::<_, String>(4)?,
                    "status": row.get::<_, String>(5)?,
                    "sort": row.get::<_, i64>(6)?,
                }))
            })
            .map_err(db_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db_err)?;
        rows
    };

    let mut hibernations = Vec::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT id, colony_id, start_date, expected_end_date, actual_end_date
                 FROM hibernation ORDER BY id",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map([], |row| {
                Ok(crate::hibernation::Hibernation {
                    id: row.get(0)?,
                    colony_id: row.get(1)?,
                    start_date: row.get(2)?,
                    expected_end_date: row.get(3)?,
                    actual_end_date: row.get(4)?,
                })
            })
            .map_err(db_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db_err)?;
        for h in rows {
            hibernations.push(
                serde_json::to_value(&h).map_err(|e| format!("序列化冬眠段失败: {e}"))?,
            );
        }
    }

    let actions: Vec<crate::dict::CareAction> = crate::dict::list_actions(conn)?;
    let foods: Vec<crate::care::Food> = crate::care::list_foods(conn)?;
    let locations: Vec<crate::colony::Location> = crate::colony::list_locations(conn)?;
    let logs: Vec<crate::care::LogRow> = all_log_rows(conn)?;
    let settings: crate::settings::AppSettings = crate::settings::get_settings(conn)?;

    // 巢况登记（webui-checkin 票 02）：带窝名展示值与照片元数据（本票照片恒空数组）
    let mut checkins = Vec::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT nc.id, nc.colony_id, c.name, nc.date, nc.queen_count, nc.worker_count,
                        nc.moved_nest, nc.note, nc.created_at
                 FROM nest_checkin nc JOIN colony c ON c.id = nc.colony_id
                 ORDER BY nc.date DESC, nc.id DESC",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                    row.get::<_, i64>(6)? != 0,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                ))
            })
            .map_err(db_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db_err)?;
        for (id, colony_id, colony_name, date, queen_count, worker_count, moved_nest, note, created_at) in rows
        {
            let mut ps = conn
                .prepare(
                    "SELECT rel_path, original_name, note FROM nest_photo
                     WHERE checkin_id = ?1 ORDER BY id",
                )
                .map_err(db_err)?;
            let photos = ps
                .query_map(params![id], |row| {
                    Ok(json!({
                        "rel_path": row.get::<_, String>(0)?,
                        "original_name": row.get::<_, Option<String>>(1)?,
                        "note": row.get::<_, String>(2)?,
                    }))
                })
                .map_err(db_err)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(db_err)?;
            checkins.push(json!({
                "id": id,
                "colony_id": colony_id,
                "colony_name": colony_name,
                "date": date,
                "queen_count": queen_count,
                "worker_count": worker_count,
                "moved_nest": moved_nest,
                "note": note,
                "created_at": created_at,
                "photos": photos,
            }));
        }
    }

    fn to_value<T: serde::Serialize>(v: &T) -> Result<serde_json::Value, String> {
        serde_json::to_value(v).map_err(|e| format!("序列化导出数据失败: {e}"))
    }

    Ok(json!({
        "colonies": colonies,
        "actions": to_value(&actions)?,
        "foods": to_value(&foods)?,
        "locations": to_value(&locations)?,
        "logs": to_value(&logs)?,
        "checkins": checkins,
        "hibernations": hibernations,
        "settings": to_value(&settings)?,
    }))
}

// ── 落盘 ─────────────────────────────────────────────────────────────────

/// 导出 CSV 到目标路径，返回写入路径。父目录不存在时自动创建。
pub fn export_csv_to(conn: &Connection, path: &Path) -> Result<String, String> {
    let csv = build_csv(&all_log_rows(conn)?, &all_checkin_rows(conn)?);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建导出目录失败: {e}"))?;
    }
    std::fs::write(path, csv).map_err(|e| format!("写入 CSV 失败: {e}"))?;
    Ok(path.to_string_lossy().to_string())
}

/// 导出 JSON 到目标路径（pretty 排版便于人读），返回写入路径。父目录不存在时自动创建。
pub fn export_json_to(conn: &Connection, path: &Path) -> Result<String, String> {
    let dump = json_dump(conn)?;
    let text = serde_json::to_string_pretty(&dump).map_err(|e| format!("序列化 JSON 失败: {e}"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建导出目录失败: {e}"))?;
    }
    std::fs::write(path, text).map_err(|e| format!("写入 JSON 失败: {e}"))?;
    Ok(path.to_string_lossy().to_string())
}

// ── 测试：只测外部行为（spec「Testing Decisions」）────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;
    use tempfile::TempDir;

    /// 建文件库（tempdir）并迁移到最新 schema。
    fn file_conn() -> (Connection, std::path::PathBuf, TempDir) {
        let dir = TempDir::new().expect("创建临时目录失败");
        let db_path = dir.path().join("data").join(crate::db::DB_FILE_NAME);
        let conn = crate::db::open_and_migrate(&db_path).expect("建库失败");
        (conn, db_path, dir)
    }

    /// 建内存库并迁移到最新 schema（不落盘的快测用）。
    fn mem_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("内存库打开失败");
        crate::db::migrate(&conn).expect("迁移失败");
        conn
    }

    /// 测试用极简 RFC 4180 行解析（引号感知），验证列数与转义还原。
    fn split_csv_line(line: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut cur = String::new();
        let mut in_quotes = false;
        let mut chars = line.chars().peekable();
        while let Some(ch) = chars.next() {
            if in_quotes {
                if ch == '"' {
                    if chars.peek() == Some(&'"') {
                        chars.next();
                        cur.push('"');
                    } else {
                        in_quotes = false;
                    }
                } else {
                    cur.push(ch);
                }
            } else {
                match ch {
                    '"' => in_quotes = true,
                    ',' => out.push(std::mem::take(&mut cur)),
                    other => cur.push(other),
                }
            }
        }
        out.push(cur);
        out
    }

    fn action_id(conn: &Connection, name: &str) -> i64 {
        conn.query_row("SELECT id FROM care_action WHERE name = ?1", params![name], |r| {
            r.get(0)
        })
        .expect("查操作失败")
    }

    fn food_id(conn: &Connection, name: &str) -> i64 {
        conn.query_row("SELECT id FROM food WHERE name = ?1", params![name], |r| r.get(0))
            .expect("查食物失败")
    }

    fn log(
        conn: &Connection,
        colony_id: i64,
        action: &str,
        happened_at: &str,
        note: Option<&str>,
        foods: &[&str],
    ) -> i64 {
        crate::care::log_care(
            conn,
            &crate::care::CareLogInput {
                colony_id,
                action_id: action_id(conn, action),
                happened_at: happened_at.into(),
                note: note.map(str::to_string),
                food_ids: foods.iter().map(|f| food_id(conn, f)).collect(),
            },
            "2026-09-18 12:00:00",
        )
        .expect("记账失败")
    }

    /// 搭一个有真实数据的库：1 窝（冬眠中）+ 3 条记录（含备注逗号与两食物）+
    /// 自建操作/食物/地点 + 改过的设置。返回库里的记录数（3）。
    fn seed(conn: &Connection) -> i64 {
        conn.execute(
            "INSERT INTO colony (name, species, start_date, status)
             VALUES ('大头一号', '大头收获蚁', '2026-01-20', 'hibernating')",
            [],
        )
        .unwrap();
        let c = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO hibernation (colony_id, start_date, expected_end_date)
             VALUES (?1, '2026-08-01', '2027-03-01')",
            params![c],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO care_action (name, icon, kind, is_feeding, suggested_interval_days, enabled, sort)
             VALUES ('糖水', NULL, 'log_only', 0, NULL, 1, 5)",
            [],
        )
        .unwrap();
        conn.execute("INSERT INTO food (name, enabled, sort) VALUES ('蚕蛹', 1, 4)", []).unwrap();
        conn.execute("INSERT INTO location (name, enabled, sort) VALUES ('阳台', 1, 3)", []).unwrap();

        log(conn, c, "喂食", "2026-09-10 08:00", Some("补录,含逗号"), &[]);
        log(conn, c, "喂食", "2026-09-15 20:30:00", None, &["种子", "蚕蛹"]);
        log(conn, c, "垃圾清理", "2026-09-16 09:00:00", Some("清理残渣"), &[]);

        crate::settings::set_settings(
            conn,
            &crate::settings::AppSettings {
                wake_remind_days_ahead: 3,
                notify_overdue_enabled: false,
                ..Default::default()
            },
        )
        .unwrap();
        c
    }

    // ── CSV 转义 ──

    #[test]
    fn csv_escape_quotes_only_when_needed() {
        assert_eq!(csv_escape("plain"), "plain");
        assert_eq!(csv_escape(""), "");
        assert_eq!(csv_escape("a,b"), "\"a,b\"");
        assert_eq!(csv_escape("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_escape("line\nbreak"), "\"line\nbreak\"");
    }

    #[test]
    fn csv_escape_guards_formula_injection_prefixes() {
        // 终局评审定点修 1：= + - @ 开头会被 Excel 当公式执行，加 ' 前缀按文本处理
        assert_eq!(csv_escape("=cmd|' /C calc'!A0"), "'=cmd|' /C calc'!A0");
        assert_eq!(csv_escape("+1+1"), "'+1+1");
        assert_eq!(csv_escape("-2"), "'-2");
        assert_eq!(csv_escape("@SUM(1)"), "'@SUM(1)");
        // 日期/数字等无害开头不前缀
        assert_eq!(csv_escape("2026-09-18 08:00:00"), "2026-09-18 08:00:00");
        // 危险开头 + 需引号字符：先前缀再引号
        assert_eq!(csv_escape("=a,b"), "\"'=a,b\"");
    }

    #[test]
    fn export_csv_guards_formula_injection_in_field_values() {
        let (conn, _db_path, dir) = file_conn();
        conn.execute(
            "INSERT INTO colony (name, start_date) VALUES ('=公式窝', '2026-01-20')",
            [],
        )
        .unwrap();
        let c = conn.last_insert_rowid();
        log(
            &conn,
            c,
            "垃圾清理",
            "2026-09-10 08:00:00",
            Some("=HYPERLINK(\"http://evil.example\")"),
            &[],
        );

        let path = dir.path().join("inject.csv");
        export_csv_to(&conn, &path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("'=公式窝"), "窝名公式前缀，实际：{text}");
        assert!(
            text.contains("'=HYPERLINK"),
            "备注公式前缀（未加引号时原样带 '），实际：{text}"
        );
        assert!(
            !text.contains("\n=HYPERLINK") && !text.contains(",=HYPERLINK"),
            "任何字段值都不得以裸 = 开头"
        );
    }

    // ── 导出 CSV（票面验收 4：行数=记录数，内容写临时文件断言）──

    #[test]
    fn export_csv_has_one_row_per_log_with_bom_and_semicolon_foods() {
        let (conn, _db_path, dir) = file_conn();
        let _colony_id = seed(&conn);

        let path = dir.path().join("out").join("export.csv");
        let written = export_csv_to(&conn, &path).unwrap();
        assert_eq!(written, path.to_string_lossy(), "返回写入路径");

        let bytes = std::fs::read(&path).unwrap();
        assert!(
            bytes.starts_with(&[0xEF, 0xBB, 0xBF]),
            "CSV 带 UTF-8 BOM（Excel 直开不乱码）"
        );
        let text = String::from_utf8(bytes).unwrap();

        let mut lines = text.trim_end_matches('\n').split('\n');
        let header = lines.next().unwrap().trim_start_matches(UTF8_BOM);
        assert_eq!(header, "id,窝,操作,发生时间,录入时间,备注,食物");
        let body: Vec<&str> = lines.collect();
        assert_eq!(body.len(), 5, "3 条记录 + 空行分隔 + 巢况区表头（本测试无巢况数据）");
        assert_eq!(body[3], "", "记录区与巢况区空一行分隔");
        assert!(body[4].starts_with("巢况id,窝,日期"), "巢况区表头");
        let log_rows = &body[..3];

        // 喂食那行：食物分号连接（顺序按字典序）
        let feeding = body
            .iter()
            .find(|l| l.contains("2026-09-15 20:30:00"))
            .expect("含喂食行");
        assert!(feeding.contains("种子;蚕蛹"), "食物分号连接，实际：{feeding}");
        assert!(feeding.contains("大头一号"), "窝列 = 窝名展示值");

        // 备注含逗号 → 整段加引号（转义生效，列不错位）
        let noted = body
            .iter()
            .find(|l| l.contains("2026-09-10"))
            .expect("含补录行");
        assert!(noted.contains("\"补录,含逗号\""), "实际：{noted}");

        // 每行都可按 RFC 4180 还原成 7 列；含逗号备注还原后原样
        for line in log_rows {
            let fields = split_csv_line(line);
            assert_eq!(fields.len(), 7, "应为 7 列，实际：{fields:?}");
        }
        let noted_fields = split_csv_line(log_rows.iter().find(|l| l.contains("2026-09-10")).unwrap());
        assert_eq!(noted_fields[5], "补录,含逗号", "转义还原后备注原样");
    }

    #[test]
    fn export_csv_of_empty_db_has_both_section_headers_only() {
        let (conn, _db_path, dir) = file_conn();
        let path = dir.path().join("empty.csv");
        export_csv_to(&conn, &path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            text.trim_start_matches(UTF8_BOM).trim_end(),
            "id,窝,操作,发生时间,录入时间,备注,食物\n\n巢况id,窝,日期,蚁后数,工蚁数,换巢,备注,录入时间,照片数"
        );
    }

    // ── 导出 CSV/JSON · 巢况区（webui-checkin 票 02）──

    /// 在 seed 之外加两条巢况：一条常规（蚁后+工蚁）、一条备注以 = 开头的换巢登记。
    fn seed_checkins(conn: &Connection, colony_id: i64) {
        crate::nest_checkin::save_checkin(
            conn,
            &crate::nest_checkin::CheckinInput {
                colony_id,
                date: "2026-09-15".into(),
                queen_count: Some(2),
                worker_count: Some(3000),
                moved_nest: false,
                note: Some("数量稳定".into()),
            },
            "2026-09-18",
            "2026-09-18 21:00:00",
        )
        .unwrap();
        crate::nest_checkin::save_checkin(
            conn,
            &crate::nest_checkin::CheckinInput {
                colony_id,
                date: "2026-09-17".into(),
                queen_count: None,
                worker_count: None,
                moved_nest: true,
                note: Some("=cmd|' /C calc'!A0".into()),
            },
            "2026-09-18",
            "2026-09-18 21:05:00",
        )
        .unwrap();
    }

    #[test]
    fn export_csv_includes_checkin_section_with_injection_guard() {
        let (conn, _db_path, dir) = file_conn();
        let c = seed(&conn);
        seed_checkins(&conn, c);

        let path = dir.path().join("with-checkins.csv");
        export_csv_to(&conn, &path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();

        // 巢况区表头存在，且在记录区之后
        let header_pos = text.find("巢况id,窝,日期,蚁后数,工蚁数,换巢,备注,录入时间,照片数")
            .expect("巢况区表头");
        let log_header_pos = text.find("id,窝,操作,发生时间").expect("记录区表头");
        assert!(header_pos > log_header_pos, "巢况区在记录区之后");

        let body_start = text.trim_start_matches(UTF8_BOM);
        let section = &body_start[body_start.find("巢况id,").unwrap()..];
        let lines: Vec<&str> = section.trim_end_matches('\n').split('\n').collect();
        assert_eq!(lines.len(), 3, "表头 + 两行巢况");

        // 每行 9 列（RFC 4180 还原）
        for line in &lines[1..] {
            assert_eq!(split_csv_line(line).len(), 9, "巢况行应为 9 列：{line}");
        }

        // 换巢是/否；空数留空；公式注入防护沿用（= 开头加 ' 前缀）
        let guarded = split_csv_line(lines[1]);
        assert_eq!(guarded[1], "大头一号", "窝名展示值");
        assert_eq!(guarded[2], "2026-09-17", "日期倒序在前");
        assert_eq!(guarded[3], "", "未数为空");
        assert_eq!(guarded[4], "");
        assert_eq!(guarded[5], "是");
        assert_eq!(guarded[6], "'=cmd|' /C calc'!A0", "公式前缀防护沿用");
        assert_eq!(guarded[8], "0", "照片数本票恒 0");

        let normal = split_csv_line(lines[2]);
        assert_eq!(normal[2], "2026-09-15");
        assert_eq!(normal[3], "2");
        assert_eq!(normal[4], "3000");
        assert_eq!(normal[5], "否");
        assert_eq!(normal[6], "数量稳定");
    }

    #[test]
    fn export_json_includes_checkins_with_colony_name_and_photo_meta() {
        let (conn, _db_path, dir) = file_conn();
        let c = seed(&conn);
        seed_checkins(&conn, c);
        // 手工插一张照片元数据（模拟库内既有引用；文件清理随票 07）
        conn.execute(
            "INSERT INTO nest_photo (checkin_id, rel_path, original_name, note)
             VALUES (1, '1/uuid-a.jpg', 'IMG_0001.jpg', '全景')",
            [],
        )
        .unwrap();

        let path = dir.path().join("with-checkins.json");
        export_json_to(&conn, &path).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).expect("JSON 可解析");

        let checkins = v["checkins"].as_array().expect("checkins 数组");
        assert_eq!(checkins.len(), 2);
        let first = &checkins[0];
        for key in [
            "id", "colony_id", "colony_name", "date", "queen_count", "worker_count",
            "moved_nest", "note", "created_at", "photos",
        ] {
            assert!(first.get(key).is_some(), "checkin 缺 {key}");
        }
        assert_eq!(first["colony_name"], "大头一号", "归档自足可读");
        assert_eq!(first["date"], "2026-09-17", "日期倒序");
        assert_eq!(first["moved_nest"], true);
        assert!(first["queen_count"].is_null(), "未数为 JSON null");
        assert_eq!(checkins[1]["queen_count"], 2);
        assert_eq!(checkins[1]["worker_count"], 3000);
        // 照片元数据随行（本票无写入路径，这里手工插行验证读取链）
        let older_photos = checkins[1]["photos"].as_array().unwrap();
        assert_eq!(older_photos.len(), 1);
        assert_eq!(older_photos[0]["rel_path"], "1/uuid-a.jpg");
        assert_eq!(older_photos[0]["original_name"], "IMG_0001.jpg");
        assert_eq!(checkins[0]["photos"].as_array().unwrap().len(), 0);
    }

    // ── 导出 JSON（票面验收 4：可解析且结构完整）──

    #[test]
    fn export_json_is_parseable_and_structurally_complete() {
        let (conn, _db_path, dir) = file_conn();
        seed(&conn);

        let path = dir.path().join("export.json");
        export_json_to(&conn, &path).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&text).expect("JSON 可解析");

        for key in [
            "colonies",
            "actions",
            "foods",
            "locations",
            "logs",
            "checkins",
            "hibernations",
        ] {
            assert!(v.get(key).and_then(|x| x.as_array()).is_some(), "缺 {key} 数组");
        }
        assert!(v.get("settings").and_then(|x| x.as_object()).is_some(), "缺 settings 对象");
        assert_eq!(v["colonies"].as_array().unwrap().len(), 1);
        assert_eq!(v["actions"].as_array().unwrap().len(), 6, "预置 5（v7 起含撤食）+ 自建 1");
        assert_eq!(v["foods"].as_array().unwrap().len(), 4, "预置 3 + 自建 1");
        assert_eq!(v["locations"].as_array().unwrap().len(), 3, "预置 2 + 自建 1");
        assert_eq!(v["logs"].as_array().unwrap().len(), 3, "全部记录");
        assert_eq!(v["hibernations"].as_array().unwrap().len(), 1);
        assert_eq!(v["settings"]["wake_remind_days_ahead"], 3);
        assert_eq!(v["settings"]["notify_overdue_enabled"], false);

        // 单条 log 结构完整（含窝名/操作名/食物名展示值，归档自足可读）
        let log = &v["logs"][0];
        for key in ["id", "colony_id", "colony_name", "action_name", "occurred_at", "created_at", "note", "food_names"] {
            assert!(log.get(key).is_some(), "log 缺 {key}");
        }
        let with_food = v["logs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|l| l["food_names"].as_array().is_some_and(|f| !f.is_empty()))
            .expect("有一条带食物的记录");
        assert_eq!(with_food["food_names"][0], "种子");
        assert_eq!(with_food["colony_name"], "大头一号");

        // colony / hibernation 行结构完整
        let colony = &v["colonies"][0];
        for key in ["id", "name", "species", "start_date", "status"] {
            assert!(colony.get(key).is_some(), "colony 缺 {key}");
        }
        let hiber = &v["hibernations"][0];
        for key in ["id", "colony_id", "start_date", "expected_end_date", "actual_end_date"] {
            assert!(hiber.get(key).is_some(), "hibernation 缺 {key}");
        }
    }

    // ── 安全备份（票面验收 3：产物可被应用重新打开读取）──

    #[test]
    fn backup_copy_is_reopenable_and_data_complete() {
        let (conn, db_path, dir) = file_conn();
        seed(&conn);

        let target = dir.path().join("backup").join("copy.db");
        backup_db_file(&db_path, &target).unwrap();

        // 备份产物用应用的正式打开通道重开：迁移幂等通过、数据齐、版本一致
        let reopened = crate::db::open_and_migrate(&target).expect("备份产物可被应用重新打开");
        let logs: i64 = reopened
            .query_row("SELECT COUNT(*) FROM care_log", [], |r| r.get(0))
            .unwrap();
        assert_eq!(logs, 3, "记录完整");
        let foods: i64 = reopened
            .query_row("SELECT COUNT(*) FROM log_food", [], |r| r.get(0))
            .unwrap();
        assert_eq!(foods, 2, "记录-食物关联完整");
        assert_eq!(
            crate::db::schema_version_of(&reopened).unwrap(),
            crate::db::SCHEMA_VERSION
        );
    }

    #[test]
    fn backup_to_missing_source_reports_error() {
        let (_conn, _db_path, dir) = file_conn();
        let err = backup_db_file(&dir.path().join("no-such.db"), &dir.path().join("x.db"));
        assert!(err.is_err(), "源不存在应报错而不是静默产出空文件");
    }

    // ── 全量拉取（绕开单页上限）──

    #[test]
    fn all_log_rows_pulls_everything_beyond_single_page_limit() {
        let conn = mem_conn();
        conn.execute(
            "INSERT INTO colony (name, start_date) VALUES ('大头一号', '2026-01-20')",
            [],
        )
        .unwrap();
        let c = conn.last_insert_rowid();
        // list_logs 单页上限 500：灌 505 条，验证分页循环拉全（同刻多条合法）
        for i in 0..505 {
            log(&conn, c, "垃圾清理", "2026-01-01 00:00:00", None, &[]);
            let _ = i;
        }
        let rows = all_log_rows(&conn).unwrap();
        assert_eq!(rows.len(), 505, "分页循环拉全，不受单页 500 限制");
    }
}
