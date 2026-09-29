//! 自建物种（species-profile 票 02）：内置档案别名表 + 自建物种行级命令。
//!
//! 全部函数只吃 `&Connection`，与 Tauri 解耦，可被 cargo test 直接覆盖；
//! Tauri command 只是薄包装（见 lib.rs）；行级命令沿 dict.rs/colony.rs 先例。
//!
//! key 契约（proposal §4.3 rev1 定稿，评审 F1 修复）：
//! - 内置 key = 拉丁名 slug（无前缀），与 `custom-` 前缀天然隔离；
//! - 自建 key = `custom-<自增 id>`：AUTOINCREMENT 保证 id 永不复用 →
//!   key 永不复用、永不改变；改名只改 name 列，绝不改 key；
//! - colony.species_key 一律存 key，绝不存名字。

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

// ── 内置档案注册表（迁移别名表与 species_key 校验的同源数据）──────────────

/// 内置 12 种（收获蚁 4 + 弓背蚁 8）的匹配注册表。key/中文名/俗名清单与
/// proposal §3.3 一致；拉丁名进 aliases（匹配 trim 后精确、大小写不敏感）。
/// 前端档案包（`src/data/species/`）是档案数据的权威，本表只承担两件事：
/// v15 迁移别名归组 + species_key 校验/快照推导——两个消费方都在 Rust 侧。
pub(crate) struct BuiltinSpecies {
    /// 档案 key = 拉丁名 slug（与前端档案包同键）。
    pub key: &'static str,
    /// 正式中文名（别名匹配命中后的快照规范化目标 cnName）。
    pub cn_name: &'static str,
    /// 其余匹配名：俗名/商家名/拉丁名（含正式中文名与拉丁名的全部变体）。
    pub aliases: &'static [&'static str],
}

/// 自建物种 key 前缀（解析规则：以它开头 → 查自建表，否则 → 查内置档案）。
pub(crate) const CUSTOM_KEY_PREFIX: &str = "custom-";

/// 自建物种默认类型（迁移自动转自建与选择器内联新建都用它）。
pub(crate) const DEFAULT_TYPE: &str = "自定义";

/// 两步落键（插行 → 按 id 回填 key）时 key 列的临时占位值。单连接串行 +
/// 调用方事务包裹，同一时刻至多一行 pending，事务回滚绝不残留。
const PENDING_KEY: &str = "__pending__";

pub(crate) const BUILTIN_SPECIES: [BuiltinSpecies; 12] = [
    // ── 收获蚁属（Messor）4 种 ──
    BuiltinSpecies {
        key: "messor-aciculatus",
        cn_name: "针毛收获蚁",
        aliases: &["针毛", "针毛收获", "Messor aciculatus"],
    },
    BuiltinSpecies {
        key: "messor-structor",
        cn_name: "工匠收获蚁",
        aliases: &["工匠", "小收获蚁", "Messor structor"],
    },
    BuiltinSpecies {
        key: "messor-barbarus",
        cn_name: "红头收获蚁",
        aliases: &[
            "巴巴拉",
            "红头收获",
            "红头大头",
            "巴巴拉收获蚁",
            "野蛮收获蚁",
            "Messor barbarus",
        ],
    },
    BuiltinSpecies {
        key: "messor-cephalotes",
        cn_name: "肯尼亚收获蚁",
        aliases: &[
            "大头收获蚁",
            "肯尼亚",
            "大头",
            "巨首收获蚁",
            "Messor cephalotes",
        ],
    },
    // ── 弓背蚁属（Camponotus）8 种 ──
    BuiltinSpecies {
        key: "camponotus-turkestanus",
        cn_name: "中亚弓背蚁",
        aliases: &[
            "突厥弓背蚁",
            "黑金土耳其",
            "黑金土耳其弓背蚁",
            "Camponotus turkestanus",
        ],
    },
    BuiltinSpecies {
        key: "camponotus-japonicus",
        cn_name: "日本弓背蚁",
        aliases: &["Camponotus japonicus"],
    },
    BuiltinSpecies {
        key: "camponotus-vitiosus",
        cn_name: "瑕疵弓背蚁",
        aliases: &["统领", "统领弓背蚁", "东京弓背蚁", "Camponotus vitiosus"],
    },
    BuiltinSpecies {
        key: "camponotus-pseudolendus",
        cn_name: "拟哀弓背蚁",
        aliases: &["Camponotus pseudolendus"],
    },
    BuiltinSpecies {
        key: "camponotus-pseudoirritans",
        cn_name: "拟光腹弓背蚁",
        aliases: &["拟光腹", "Camponotus pseudoirritans"],
    },
    BuiltinSpecies {
        key: "camponotus-sericeiventris",
        cn_name: "丝腹弓背蚁",
        aliases: &["丝腹", "Camponotus sericeiventris"],
    },
    BuiltinSpecies {
        key: "camponotus-mutilarius",
        cn_name: "香斑弓背蚁",
        aliases: &["香斑", "截胸弓背蚁", "Camponotus mutilarius"],
    },
    BuiltinSpecies {
        key: "camponotus-nicobarensis",
        cn_name: "尼科巴弓背蚁",
        aliases: &["尼科巴", "Camponotus nicobarensis"],
    },
];

/// 是否自建物种 key（`custom-` 前缀）。
pub(crate) fn is_custom_key(key: &str) -> bool {
    key.starts_with(CUSTOM_KEY_PREFIX)
}

/// 别名匹配：trim 后精确匹配（正式中文名 ∪ 俗名别名 ∪ 拉丁名），大小写不敏感。
/// 命中返回 `(key, cnName)`。内置 key 本身（slug 形态）不在匹配集——旧自由
/// 文本里存的是人写的名字，不是 slug。
pub(crate) fn match_builtin(text: &str) -> Option<(&'static str, &'static str)> {
    let needle = text.trim().to_lowercase();
    BUILTIN_SPECIES.iter().find_map(|s| {
        let hit = needle == s.cn_name.to_lowercase()
            || s.aliases.iter().any(|a| needle == a.to_lowercase());
        hit.then_some((s.key, s.cn_name))
    })
}

/// 按档案 key 查内置种正式中文名（create/update_colony 的快照推导用）。
/// 查不到 = 未来 JSON 新增档案（零代码承诺：Rust 注册表不拦新 key）。
pub(crate) fn builtin_cn_name(key: &str) -> Option<&'static str> {
    BUILTIN_SPECIES
        .iter()
        .find(|s| s.key == key)
        .map(|s| s.cn_name)
}

// ── DTO ──────────────────────────────────────────────────────────────────

/// 自建物种行（列名 `type` 是 Rust 关键字，DTO 字段名 species_type、
/// 序列化键对齐档案包的 `type`）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CustomSpecies {
    pub id: i64,
    /// `custom-<自增 id>`：永不复用、永不改变（改的是 name，不是它）。
    pub key: String,
    pub name: String,
    #[serde(rename = "type")]
    pub species_type: String,
    pub created_at: String,
    /// 是否被窝的 species_key 引用（被引用禁删，只能先在窝表单改选）。
    pub referenced: bool,
}

// ── 内部共用 ──────────────────────────────────────────────────────────────

fn db_err(e: rusqlite::Error) -> String {
    format!("数据库操作失败: {e}")
}

const CUSTOM_SQL: &str = concat!(
    "SELECT s.id, s.key, s.name, s.type, s.created_at, ",
    "EXISTS(SELECT 1 FROM colony c WHERE c.species_key = s.key) ",
    "FROM custom_species s ",
);

fn row_to_custom(row: &rusqlite::Row<'_>) -> rusqlite::Result<CustomSpecies> {
    Ok(CustomSpecies {
        id: row.get(0)?,
        key: row.get(1)?,
        name: row.get(2)?,
        species_type: row.get(3)?,
        created_at: row.get(4)?,
        referenced: row.get::<_, i64>(5)? != 0,
    })
}

/// 名字 trim 后非空。
fn validate_name(name: &str) -> Result<String, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("自建物种名字不能为空".into());
    }
    Ok(trimmed.to_string())
}

/// 类型归一：None / 空白 → 默认「自定义」。
fn normalize_type(species_type: Option<&str>) -> String {
    match species_type.map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) => t.to_string(),
        None => DEFAULT_TYPE.to_string(),
    }
}

/// 建一行自建物种（key = `custom-<自增 id>`，插行后按 id 回填 key，调用方
/// 事务包裹保证两步原子）；名字已存在（name UNIQUE）时归并返回既有行、
/// 绝不报错、绝不重复建行（迁移同名归并与选择器「同名即选用」同一语义）。
/// 先按名查再 INSERT OR IGNORE 兜底：AUTOINCREMENT 下被 IGNORE 的插入也会
/// 烧掉一个自增 id（key 出空洞），预检让正常路径的 key 严格按首现顺序递增。
pub(crate) fn insert_custom_or_merge(
    conn: &Connection,
    name: &str,
    species_type: &str,
    created_at: &str,
) -> Result<(i64, String), rusqlite::Error> {
    if let Some(existing) = conn
        .query_row(
            "SELECT id, key FROM custom_species WHERE name = ?1",
            params![name],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
    {
        return Ok(existing);
    }
    let inserted = conn.execute(
        "INSERT OR IGNORE INTO custom_species (key, name, type, created_at)
         VALUES (?1, ?2, ?3, ?4)",
        params![PENDING_KEY, name, species_type, created_at],
    )? > 0;
    if inserted {
        let id = conn.last_insert_rowid();
        let key = format!("{CUSTOM_KEY_PREFIX}{id}");
        conn.execute(
            "UPDATE custom_species SET key = ?1 WHERE id = ?2",
            params![key, id],
        )?;
        Ok((id, key))
    } else {
        conn.query_row(
            "SELECT id, key FROM custom_species WHERE name = ?1",
            params![name],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
    }
}

/// 按 key 取整行（命令返回用）；不存在给人话报错。
fn get_by_key(conn: &Connection, key: &str) -> Result<CustomSpecies, String> {
    conn.query_row(
        &format!("{CUSTOM_SQL}WHERE s.key = ?1"),
        params![key],
        row_to_custom,
    )
    .map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => "自建物种不存在".to_string(),
        other => db_err(other),
    })
}

// ── 命令（桌面端；webui 只放行 list）──────────────────────────────────────

/// 全部自建物种（按创建序），含被引用标记（被引用禁删）。
pub fn list_custom_species(conn: &Connection) -> Result<Vec<CustomSpecies>, String> {
    let mut stmt = conn
        .prepare(&format!("{CUSTOM_SQL}ORDER BY s.id"))
        .map_err(db_err)?;
    let rows = stmt
        .query_map([], row_to_custom)
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?;
    Ok(rows)
}

/// 新建自建物种：名字 UNIQUE，重复名**返回已存在行不报错**（选择器内联
/// 新建的「同名即选用」语义）；key = custom-<自增 id> 绝不复用。
pub fn create_custom_species(
    conn: &Connection,
    name: &str,
    species_type: Option<&str>,
) -> Result<CustomSpecies, String> {
    let name = validate_name(name)?;
    let species_type = normalize_type(species_type);
    let tx = conn.unchecked_transaction().map_err(db_err)?;
    let (_id, key) = insert_custom_or_merge(&tx, &name, &species_type, &crate::care::now_local())
        .map_err(db_err)?;
    tx.commit().map_err(db_err)?;
    get_by_key(conn, &key)
}

/// 改名：只改 name 列 + 级联刷新所有引用窝的快照列（colony.species，单条
/// UPDATE 覆盖全部引用窝），**绝不改 key**；两步同一事务，任一失败整体回滚。
pub fn rename_custom_species(
    conn: &Connection,
    key: &str,
    new_name: &str,
) -> Result<CustomSpecies, String> {
    let name = validate_name(new_name)?;
    let existing = conn
        .query_row(
            "SELECT id FROM custom_species WHERE key = ?1",
            params![key],
            |r| r.get::<_, i64>(0),
        )
        .optional()
        .map_err(db_err)?
        .ok_or_else(|| "自建物种不存在".to_string())?;
    let dupes: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM custom_species WHERE name = ?1 AND key != ?2",
            params![name, key],
            |r| r.get(0),
        )
        .map_err(db_err)?;
    if dupes > 0 {
        return Err(format!("自建物种名字「{name}」已存在"));
    }
    let tx = conn.unchecked_transaction().map_err(db_err)?;
    tx.execute(
        "UPDATE custom_species SET name = ?1 WHERE id = ?2",
        params![name, existing],
    )
    .map_err(db_err)?;
    // 级联刷新引用窝的显示名快照（快照生命周期：自建改名必须与显示一致）
    tx.execute(
        "UPDATE colony SET species = ?1 WHERE species_key = ?2",
        params![name, key],
    )
    .map_err(db_err)?;
    tx.commit().map_err(db_err)?;
    get_by_key(conn, key)
}

/// 删除：按 key 查引用，被窝引用禁删（与食物字典/地点的引用守护同构）。
pub fn delete_custom_species(conn: &Connection, key: &str) -> Result<(), String> {
    let used_by: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM colony WHERE species_key = ?1",
            params![key],
            |r| r.get(0),
        )
        .map_err(db_err)?;
    if used_by > 0 {
        return Err(format!(
            "该物种仍被 {used_by} 个窝使用，不能删除；请先在窝表单里改选其它物种"
        ));
    }
    let changed = conn
        .execute("DELETE FROM custom_species WHERE key = ?1", params![key])
        .map_err(db_err)?;
    if changed == 0 {
        return Err("自建物种不存在".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 建内存库并迁移到最新 schema（colony.rs 同款地基）。
    fn mem_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("内存库打开失败");
        crate::db::migrate(&conn).expect("迁移失败");
        conn
    }

    // ── 内置注册表完整性（key 契约的数据面）──

    #[test]
    fn builtin_table_has_12_unique_keys_isolated_from_custom_prefix() {
        let mut keys: Vec<&str> = BUILTIN_SPECIES.iter().map(|s| s.key).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(BUILTIN_SPECIES.len(), 12);
        assert_eq!(keys.len(), 12, "档案 key 不可重复");
        for s in BUILTIN_SPECIES.iter() {
            assert!(
                !s.key.starts_with(CUSTOM_KEY_PREFIX),
                "内置 key 不得以 custom- 开头（命名空间隔离）：{}",
                s.key
            );
            assert!(!s.cn_name.trim().is_empty());
        }
    }

    #[test]
    fn builtin_match_names_have_no_cross_species_collision() {
        // 匹配名（正式中文名 ∪ 别名）归一化后全局唯一——一个文本只归一种
        let mut seen: Vec<String> = Vec::new();
        for s in BUILTIN_SPECIES.iter() {
            for name in std::iter::once(s.cn_name).chain(s.aliases.iter().copied()) {
                let norm = name.trim().to_lowercase();
                assert!(!norm.is_empty());
                assert!(
                    !seen.contains(&norm),
                    "匹配名跨种冲突（一个文本会归到多种）：{name}"
                );
                seen.push(norm);
            }
        }
    }

    #[test]
    fn match_builtin_hits_formal_alias_latin_case_and_trim() {
        // 正式中文名
        assert_eq!(
            match_builtin("针毛收获蚁"),
            Some(("messor-aciculatus", "针毛收获蚁"))
        );
        // 俗名 / 商家名
        assert_eq!(
            match_builtin("大头收获蚁"),
            Some(("messor-cephalotes", "肯尼亚收获蚁"))
        );
        assert_eq!(
            match_builtin("巴巴拉"),
            Some(("messor-barbarus", "红头收获蚁"))
        );
        assert_eq!(
            match_builtin("小收获蚁"),
            Some(("messor-structor", "工匠收获蚁"))
        );
        assert_eq!(
            match_builtin("突厥弓背蚁"),
            Some(("camponotus-turkestanus", "中亚弓背蚁"))
        );
        assert_eq!(
            match_builtin("统领"),
            Some(("camponotus-vitiosus", "瑕疵弓背蚁"))
        );
        // 拉丁名 + 大小写变体 + 首尾空白
        assert_eq!(
            match_builtin("  Camponotus nicobarensis "),
            Some(("camponotus-nicobarensis", "尼科巴弓背蚁"))
        );
        assert_eq!(
            match_builtin("messor barbarus"),
            Some(("messor-barbarus", "红头收获蚁"))
        );
        // 未匹配 / 空白 / slug 形态（key 不是匹配文本）
        assert_eq!(match_builtin("阿根廷蚁"), None);
        assert_eq!(match_builtin("   "), None);
        assert_eq!(match_builtin("camponotus-japonicus"), None);
    }

    #[test]
    fn builtin_cn_name_lookup_by_key() {
        assert_eq!(builtin_cn_name("messor-barbarus"), Some("红头收获蚁"));
        assert_eq!(builtin_cn_name("camponotus-nicobarensis"), Some("尼科巴弓背蚁"));
        assert_eq!(builtin_cn_name("custom-1"), None);
        assert_eq!(builtin_cn_name("future-species"), None);
    }

    // ── 行级命令 ──

    #[test]
    fn create_assigns_custom_key_and_returns_full_row() {
        let conn = mem_conn();
        let sp = create_custom_species(&conn, " 蜜罐蚁 ", Some(" 蜜罐蚁科 ")).unwrap();
        assert_eq!(sp.name, "蜜罐蚁", "名字 trim 后落库");
        assert_eq!(sp.species_type, "蜜罐蚁科");
        assert_eq!(sp.key, format!("custom-{}", sp.id), "key = custom-<自增 id>");
        assert!(!sp.referenced);
        // 库内一致
        let row: (String, String, String) = conn
            .query_row(
                "SELECT key, name, type FROM custom_species WHERE id = ?1",
                params![sp.id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(row, ("custom-1".into(), "蜜罐蚁".into(), "蜜罐蚁科".into()));
    }

    #[test]
    fn create_defaults_type_and_rejects_blank_name() {
        let conn = mem_conn();
        let sp = create_custom_species(&conn, "阿根廷蚁", None).unwrap();
        assert_eq!(sp.species_type, "自定义", "类型缺省 = 自定义");
        assert_eq!(sp.created_at.len(), 19, "created_at 库内统一时间戳格式");
        assert!(create_custom_species(&conn, "  ", None)
            .unwrap_err()
            .contains("不能为空"));
    }

    #[test]
    fn duplicate_create_returns_existing_row_without_error_or_new_row() {
        let conn = mem_conn();
        let a = create_custom_species(&conn, "蜜罐蚁", None).unwrap();
        let b = create_custom_species(&conn, "蜜罐蚁", Some("别的类型")).unwrap();
        assert_eq!(a.id, b.id, "同名返回既有行，不报错");
        assert_eq!(b.key, "custom-1");
        assert_eq!(b.species_type, "自定义", "既有行不被改写");
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM custom_species", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1, "绝不重复建行");
        // 再建第二种 → key 顺延
        let c = create_custom_species(&conn, "阿根廷蚁", None).unwrap();
        assert_eq!(c.key, "custom-2");
    }

    #[test]
    fn custom_key_never_reused_after_delete() {
        // AUTOINCREMENT：id 用过即弃，key 永不复用（key 契约的库层保证）
        let conn = mem_conn();
        let a = create_custom_species(&conn, "甲蚁", None).unwrap();
        let b = create_custom_species(&conn, "乙蚁", None).unwrap();
        delete_custom_species(&conn, &a.key).unwrap();
        delete_custom_species(&conn, &b.key).unwrap();
        let c = create_custom_species(&conn, "丙蚁", None).unwrap();
        assert_eq!(c.key, "custom-3", "删过的 key 不回收");
    }

    #[test]
    fn rename_only_changes_name_and_cascades_snapshots() {
        let conn = mem_conn();
        let sp = create_custom_species(&conn, "蜜罐蚁", None).unwrap();
        conn.execute(
            "INSERT INTO colony (name, start_date, status, species, species_key)
             VALUES ('甲', '2026-01-01', 'active', '蜜罐蚁', ?1)",
            params![sp.key],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO colony (name, start_date, status, species)
             VALUES ('乙', '2026-01-01', 'active', '别的')",
            [],
        )
        .unwrap();

        let renamed = rename_custom_species(&conn, &sp.key, " 蜜罐蚁二号 ").unwrap();
        assert_eq!(renamed.name, "蜜罐蚁二号");
        assert_eq!(renamed.key, sp.key, "改名绝不改 key");
        // 引用窝快照级联刷新；未引用窝不动
        let a: (Option<String>, Option<String>) = conn
            .query_row(
                "SELECT species, species_key FROM colony WHERE name = '甲'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            a,
            (
                Some("蜜罐蚁二号".into()),
                Some(sp.key.clone()),
            ),
            "快照随改名刷新、key 原样"
        );
        let b: Option<String> = conn
            .query_row("SELECT species FROM colony WHERE name = '乙'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(b, Some("别的".into()));
        // 改回自己的名字不算撞名
        assert!(rename_custom_species(&conn, &sp.key, "蜜罐蚁二号").is_ok());
    }

    #[test]
    fn rename_rejects_missing_key_blank_and_duplicate_name() {
        let conn = mem_conn();
        let a = create_custom_species(&conn, "蜜罐蚁", None).unwrap();
        let _b = create_custom_species(&conn, "阿根廷蚁", None).unwrap();
        assert!(rename_custom_species(&conn, "custom-999", "新名")
            .unwrap_err()
            .contains("不存在"));
        assert!(rename_custom_species(&conn, &a.key, "  ")
            .unwrap_err()
            .contains("不能为空"));
        let err = rename_custom_species(&conn, &a.key, "阿根廷蚁").unwrap_err();
        assert!(err.contains("已存在"), "实际：{err}");
    }

    #[test]
    fn delete_guarded_by_colony_references() {
        let conn = mem_conn();
        let sp = create_custom_species(&conn, "蜜罐蚁", None).unwrap();
        conn.execute(
            "INSERT INTO colony (name, start_date, status, species, species_key)
             VALUES ('甲', '2026-01-01', 'active', '蜜罐蚁', ?1)",
            params![sp.key],
        )
        .unwrap();
        let err = delete_custom_species(&conn, &sp.key).unwrap_err();
        assert!(err.contains("1 个窝"), "实际：{err}");
        // 解除引用后可删；再删报不存在
        conn.execute(
            "UPDATE colony SET species_key = NULL WHERE species_key = ?1",
            params![sp.key],
        )
        .unwrap();
        delete_custom_species(&conn, &sp.key).unwrap();
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM custom_species", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
        assert!(delete_custom_species(&conn, &sp.key)
            .unwrap_err()
            .contains("不存在"));
    }

    #[test]
    fn list_orders_by_id_and_marks_referenced() {
        let conn = mem_conn();
        let a = create_custom_species(&conn, "蜜罐蚁", None).unwrap();
        let b = create_custom_species(&conn, "阿根廷蚁", None).unwrap();
        conn.execute(
            "INSERT INTO colony (name, start_date, status, species, species_key)
             VALUES ('甲', '2026-01-01', 'active', '阿根廷蚁', ?1)",
            params![b.key],
        )
        .unwrap();
        let list = list_custom_species(&conn).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, a.id, "按创建序");
        assert!(!list[0].referenced);
        assert!(list[1].referenced);
    }
}
