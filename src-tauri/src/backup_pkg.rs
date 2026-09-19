//! 数据包备份（webui-checkin 票 09，规格 Implementation Decisions F）。
//!
//! 备份产物从「裸库 .db」升级为单文件 zip 数据包：根级 [`MANIFEST_ENTRY`] 清单
//! （格式版本 / 创建应用版本 / 创建时间 / 库文件名+SHA256 / 照片清单
//! [相对路径+SHA256+字节] / 总张数与总字节）+ 库 + 照片（按库内相对路径）。
//! 旧 `.db` 命名保留给兼容读取（恢复侧票 10），不再产出。
//!
//! **清单驱动**：只收快照库 `nest_photo` 表引用的照片——库无引用的散文件、
//! `.tmp-` 半截临时、`.orphan-*` 隔离区天然不进包（[`is_packable_rel_path`]
//! 的严格 grammar `[A-Za-z0-9_-]+/[A-Za-z0-9_-]+\.jpg` 一道闸全拦：`.` 不在
//! 字符集内，点开头的 tmp/孤儿路径与 `..`/反斜杠/绝对路径全部拒绝，与恢复侧
//! 严格 grammar 同源）。
//!
//! **降级语义**：每张引用照片先存在性后内容读取，任一步失败 → 跳过该张 +
//! [`on_skip`] 回调告警（生产侧落 applog 错误流水），包照常产出，且 **manifest
//! 只列实际收入的**——包永远自洽（哈希/张数/字节对得上）。整包失败只有一种：
//! 库快照读不了 / zip 写入失败（库是包的根，无降级可言），失败路径清掉半截产物。
//!
//! **锁纪律与一致性论证**（调用方编排：先锁内拷库，再锁外打包）：
//! - 库在锁内拷到临时文件（毫秒级本地拷贝，挡并发写，journal_mode=DELETE 拷贝
//!   即完整）；
//! - 照片读取在锁外：照片文件经写入协议落位（tmp+fsync → 同目录原子 rename →
//!   库事务提交）后**不再改写**——文件名 immutable，「修改」= 写新文件 + 换库内
//!   引用，「删除」= 先库事务后删文件；manifest 以锁内库快照为准，锁外读到的
//!   照片内容字节稳定；
//! - 锁外打包不阻塞业务命令（沿 auto_backup.rs 两段式先例）。

use std::io::Write;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

/// 数据包格式版本（恢复侧：大于本应用支持的版本拒收，规格 F）。
pub const FORMAT_VERSION: u32 = 1;

/// 清单在 zip 根级的条目名。
pub const MANIFEST_ENTRY: &str = "manifest.json";

/// 库文件在包内的条目名（= 应用库文件名；恢复时按它落位）。
pub const DB_ENTRY: &str = crate::db::DB_FILE_NAME;

/// manifest 照片条目。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackageManifestPhoto {
    /// photos/ 下相对路径（正斜杠，`<colonyId>/<uuid>.jpg`）。
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
}

/// 数据包根级清单（manifest.json 的结构；恢复侧按 format_version 校验后消费）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackageManifest {
    pub format_version: u32,
    pub app_version: String,
    /// 本地时间 `YYYY-MM-DD HH:MM:SS`（applog::now_local 同格式）。
    pub created_at: String,
    pub db_file: String,
    pub db_sha256: String,
    /// **只列实际收入的照片**（缺文件降级后自洽）。
    pub photos: Vec<PackageManifestPhoto>,
    pub photo_count: usize,
    /// 照片字节总量（照片条目 bytes 之和；库体积由 db 条目自身承载）。
    pub total_bytes: u64,
}

/// 打包结果（调用方记流水用；跳过明细已逐条走 on_skip）。
#[derive(Debug, Clone, PartialEq)]
pub struct PackageOutcome {
    pub photo_count: usize,
    pub skipped_photos: Vec<String>,
}

/// 创建应用版本号（编译期 Cargo 版本，与 tauri.conf.json version 同步维护；
/// 纯核不注入——版本在单次构建内恒定，不是可变依赖）。
pub fn app_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// 打包：库快照 + 照片 → 单文件 zip 数据包（写入 `target`）。
///
/// 参数见模块头锁纪律说明。`on_skip` 每跳过一张照片回调一条人话原因（生产传
/// applog::log_error 包装；测试注入收集器——applog 全局目录 OnceLock 进程级
/// 一次，套件内不可抢占，与 full_chain.rs 同款取舍）。任何整包失败都清掉半截
/// target（目录占位等删除失败忽略，绝不误删非自家产物）。
pub fn build_package(
    db_snapshot: &Path,
    photos_root: &Path,
    created_at: &str,
    target: &Path,
    on_skip: &mut dyn FnMut(&str),
) -> Result<PackageOutcome, String> {
    let rel_paths = referenced_photo_paths(db_snapshot)?;
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建数据包目录失败: {e}"))?;
    }
    match write_package(db_snapshot, photos_root, created_at, target, &rel_paths, on_skip) {
        Ok(outcome) => Ok(outcome),
        Err(e) => {
            let _ = std::fs::remove_file(target);
            Err(e)
        }
    }
}

fn write_package(
    db_snapshot: &Path,
    photos_root: &Path,
    created_at: &str,
    target: &Path,
    rel_paths: &[String],
    on_skip: &mut dyn FnMut(&str),
) -> Result<PackageOutcome, String> {
    let file = std::fs::File::create(target).map_err(|e| format!("创建数据包文件失败: {e}"))?;
    let mut zip = ZipWriter::new(file);
    // 照片已是重编码 JPEG（压缩率有限），库是 SQLite 页（零页多、deflate 收益明显）
    let options =
        SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    let zip_err = |e: zip::result::ZipError| format!("写数据包失败: {e}");
    let mut skipped = Vec::new();

    // ① 库条目：流式拷贝 + 即时 SHA256（不整载进内存）。库快照是包的根，
    //    读取失败 = 整包失败（无降级），错误向上、半截产物由 build_package 清。
    let db_sha256 = {
        let mut db_file =
            std::fs::File::open(db_snapshot).map_err(|e| format!("读取库快照失败: {e}"))?;
        zip.start_file(DB_ENTRY, options).map_err(zip_err)?;
        let mut tee = Sha256Writer { inner: &mut zip, hasher: Sha256::new() };
        std::io::copy(&mut db_file, &mut tee).map_err(|e| format!("写入库条目失败: {e}"))?;
        hex(&tee.hasher.finalize())
    };

    // ② 照片条目（清单驱动）：先整读再写条目——单张照片是重编码 JPEG（上传侧
    //    ≤15MB 闸，落盘后更小），整读的内存尖峰可控；换来「读取失败发生在条目
    //    开始之前」，跳过该张不污染 zip 结构。
    let mut photos: Vec<PackageManifestPhoto> = Vec::new();
    for rel in rel_paths {
        if !is_packable_rel_path(rel) {
            let msg = format!("{rel}: 路径形态不收（.tmp-/.orphan-/越界），已跳过");
            on_skip(&msg);
            skipped.push(msg);
            continue;
        }
        match std::fs::read(&abs_photo_path(photos_root, rel)) {
            Ok(bytes) => {
                zip.start_file(rel, options).map_err(zip_err)?;
                zip.write_all(&bytes).map_err(|e| format!("写入照片条目失败: {e}"))?;
                photos.push(PackageManifestPhoto {
                    path: rel.clone(),
                    sha256: hex(&Sha256::digest(&bytes)),
                    bytes: bytes.len() as u64,
                });
            }
            Err(e) => {
                // 读取时缺失/不可读：跳过该张 + 告警，不整包失败（规格 F）
                let msg = format!("{rel}: 照片读取失败，已跳过（{e}）");
                on_skip(&msg);
                skipped.push(msg);
            }
        }
    }

    // ③ 清单最后写：此刻哈希/张数/字节都已定，manifest 只列实际收入（自洽）
    let manifest = PackageManifest {
        format_version: FORMAT_VERSION,
        app_version: app_version().to_string(),
        created_at: created_at.to_string(),
        db_file: DB_ENTRY.to_string(),
        db_sha256,
        photo_count: photos.len(),
        total_bytes: photos.iter().map(|p| p.bytes).sum(),
        photos,
    };
    zip.start_file(MANIFEST_ENTRY, options).map_err(zip_err)?;
    let json = serde_json::to_vec(&manifest).map_err(|e| format!("序列化清单失败: {e}"))?;
    zip.write_all(&json).map_err(|e| format!("写入清单条目失败: {e}"))?;

    zip.finish().map_err(zip_err)?;
    Ok(PackageOutcome {
        photo_count: manifest.photo_count,
        skipped_photos: skipped,
    })
}

/// 快照库 `nest_photo` 引用的照片相对路径（去重、按字典序，打包顺序稳定）。
/// 升级前的旧库没有 nest_photo 表（照片能力之前的老备份快照）→ 按零照片处理。
fn referenced_photo_paths(db_snapshot: &Path) -> Result<Vec<String>, String> {
    // 只读打开：快照文件缺失/非 SQLite 在这里就地报错，绝不新建空库
    let conn = Connection::open_with_flags(db_snapshot, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| format!("打开库快照失败: {e}"))?;
    let query = || -> Result<Vec<String>, rusqlite::Error> {
        let mut stmt = conn.prepare("SELECT DISTINCT rel_path FROM nest_photo ORDER BY rel_path")?;
        let rows = stmt
            .query_map([], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    };
    match query() {
        Ok(v) => Ok(v),
        Err(e) => {
            if e.to_string().contains("no such table") {
                Ok(Vec::new())
            } else {
                Err(format!("读取库快照照片清单失败: {e}"))
            }
        }
    }
}

/// 数据包内照片路径 grammar（打包侧与恢复侧同一份表驱动判定，规格 F：
/// 仅 `<段>/<文件>.jpg` 两段，段内字符限 `[A-Za-z0-9_-]`）。`.tmp-`/`.orphan-*`
/// 以 `.` 开头、`..`/反斜杠/冒号/绝对路径全都不在字符集内——「永不进包」与
/// 「解包只认清单内合法路径」在这一个闸上同时成立。
pub fn is_packable_rel_path(rel: &str) -> bool {
    let mut parts = rel.split('/');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(dir), Some(file), None) => {
            is_path_segment(dir) && is_path_segment(file.strip_suffix(".jpg").unwrap_or(""))
        }
        _ => false,
    }
}

fn is_path_segment(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn abs_photo_path(photos_root: &Path, rel: &str) -> PathBuf {
    let mut p = photos_root.to_path_buf();
    for comp in rel.split('/') {
        p.push(comp);
    }
    p
}

/// 边写 zip 条目边算 SHA256 的三通 writer（库条目用，流式不整载）。
struct Sha256Writer<'a, W: Write> {
    inner: &'a mut W,
    hasher: Sha256,
}

impl<W: Write> Write for Sha256Writer<'_, W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.hasher.update(&buf[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

fn hex(digest: &[u8]) -> String {
    let mut s = String::with_capacity(digest.len() * 2);
    for b in digest {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

// ── 测试：只测外部行为（spec「Testing Decisions」）────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::Read;
    use std::path::PathBuf;

    use rusqlite::params;
    use tempfile::TempDir;

    /// 一个字节都不同的假 JPEG 内容（打包不做解码，只按字节算哈希）。
    fn photo_bytes(tag: u8) -> Vec<u8> {
        vec![0xFF, 0xD8, 0xFF, tag, b'A', b'B', tag, 0x00]
    }

    /// 打包夹具：数据目录（含已迁移库：1 窝 1 条带照片的巢况）+ photos 根。
    /// 返回 (tempdir, db_path, photos_root)。
    fn fixture() -> (TempDir, PathBuf, PathBuf) {
        let dir = TempDir::new().expect("创建临时目录失败");
        let data_dir = dir.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        let db_path = data_dir.join(crate::db::DB_FILE_NAME);
        let conn = crate::db::open_and_migrate(&db_path).expect("建库失败");
        conn.execute(
            "INSERT INTO colony (name, start_date) VALUES ('大头一号', '2026-01-20')",
            [],
        )
        .unwrap();
        let _checkin_id = crate::nest_checkin::save_checkin(
            &conn,
            &crate::nest_checkin::CheckinInput {
                colony_id: 1,
                date: "2026-09-18".into(),
                queen_count: Some(1),
                worker_count: None,
                moved_nest: false,
                note: Some("带照片".into()),
            },
            "2026-09-18",
            "2026-09-18 21:00:00",
        )
        .unwrap()
        .id;
        drop(conn);
        let photos_root = data_dir.join(crate::photo::PHOTOS_DIR_NAME);
        (dir, db_path, photos_root)
    }

    /// 写照片文件 + 插引用行（模拟写入协议落位后的最终态）。
    fn add_photo(db_path: &Path, photos_root: &Path, checkin_id: i64, rel: &str, bytes: &[u8]) {
        let p = abs_photo_path(photos_root, rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, bytes).unwrap();
        let conn = rusqlite::Connection::open(db_path).unwrap();
        conn.execute(
            "INSERT INTO nest_photo (checkin_id, rel_path, original_name, note)
             VALUES (?1, ?2, '', '')",
            params![checkin_id, rel],
        )
        .unwrap();
    }

    /// 直接插引用行（不写文件：缺文件降级 / 防御层用例用）。
    fn add_photo_row_only(db_path: &Path, checkin_id: i64, rel: &str) {
        let conn = rusqlite::Connection::open(db_path).unwrap();
        conn.execute(
            "INSERT INTO nest_photo (checkin_id, rel_path, original_name, note)
             VALUES (?1, ?2, '', '')",
            params![checkin_id, rel],
        )
        .unwrap();
    }

    fn read_zip_entry_names(pkg: &Path) -> Vec<String> {
        let f = std::fs::File::open(pkg).unwrap();
        let mut z = zip::ZipArchive::new(f).unwrap();
        (0..z.len())
            .map(|i| z.by_index(i).unwrap().name().to_string())
            .collect()
    }

    fn read_zip_entry(pkg: &Path, name: &str) -> Vec<u8> {
        let f = std::fs::File::open(pkg).unwrap();
        let mut z = zip::ZipArchive::new(f).unwrap();
        let mut e = z.by_name(name).unwrap();
        let mut buf = Vec::new();
        e.read_to_end(&mut buf).unwrap();
        buf
    }

    /// 静默 on_skip（不关心告警的用例用）。
    fn no_skip() -> impl FnMut(&str) {
        |_| {}
    }

    const CREATED_AT: &str = "2026-09-19 08:30:00";

    // ── 端到端：打包 → 解包验证 manifest ─────────────────────────────────

    #[test]
    fn package_carries_db_and_referenced_photos_with_verifiable_manifest() {
        let (_dir, db_path, photos_root) = fixture();
        let a = photo_bytes(0x11);
        let b = photo_bytes(0x22);
        add_photo(&db_path, &photos_root, 1, "1/uuid-a.jpg", &a);
        add_photo(&db_path, &photos_root, 1, "1/uuid-b.jpg", &b);

        let target = _dir.path().join("pkg").join("ant-feeding-log-backup-20260919-083000.zip");
        let outcome = build_package(&db_path, &photos_root, CREATED_AT, &target, &mut no_skip())
            .expect("打包成功");
        assert_eq!(outcome.photo_count, 2);
        assert!(outcome.skipped_photos.is_empty());

        // 条目齐：库 + 两张照片 + 根级清单（库无引用的照片一概没有）
        let mut names = read_zip_entry_names(&target);
        names.sort();
        assert_eq!(
            names,
            vec![
                "1/uuid-a.jpg".to_string(),
                "1/uuid-b.jpg".to_string(),
                DB_ENTRY.to_string(),
                MANIFEST_ENTRY.to_string(),
            ],
            "实际条目：{names:?}"
        );

        // manifest 字段全量对账
        let manifest: serde_json::Value =
            serde_json::from_slice(&read_zip_entry(&target, MANIFEST_ENTRY)).unwrap();
        assert_eq!(manifest["format_version"], FORMAT_VERSION);
        assert_eq!(manifest["app_version"], app_version());
        assert_eq!(manifest["created_at"], CREATED_AT);
        assert_eq!(manifest["db_file"], crate::db::DB_FILE_NAME);

        // 库哈希 = 包内库条目的实际哈希（清单自洽）
        let db_entry_bytes = read_zip_entry(&target, DB_ENTRY);
        assert_eq!(manifest["db_sha256"], hex(&Sha256::digest(&db_entry_bytes)));
        // 包内库就是快照库：可被应用正式通道重开、数据完整
        let extracted_db = _dir.path().join("extracted.db");
        std::fs::write(&extracted_db, &db_entry_bytes).unwrap();
        let conn = crate::db::open_and_migrate(&extracted_db).unwrap();
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM colony", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1, "包内库可重开且数据完整");

        // 照片清单：路径/哈希/字节逐张对上源文件
        let photos = manifest["photos"].as_array().unwrap();
        assert_eq!(photos.len(), 2);
        assert_eq!(photos[0]["path"], "1/uuid-a.jpg");
        assert_eq!(photos[0]["sha256"], hex(&Sha256::digest(&a)));
        assert_eq!(photos[0]["bytes"], a.len() as u64);
        assert_eq!(photos[1]["path"], "1/uuid-b.jpg");
        assert_eq!(photos[1]["sha256"], hex(&Sha256::digest(&b)));
        assert_eq!(photos[1]["bytes"], b.len() as u64);
        // 包内照片字节与源文件一致
        assert_eq!(read_zip_entry(&target, "1/uuid-a.jpg"), a);
        assert_eq!(read_zip_entry(&target, "1/uuid-b.jpg"), b);
        // 总张数与总字节
        assert_eq!(manifest["photo_count"], 2);
        assert_eq!(manifest["total_bytes"], (a.len() + b.len()) as u64);
    }

    // ── 缺文件降级：跳过 + 告警，包仍自洽 ────────────────────────────────

    #[test]
    fn missing_referenced_photo_is_skipped_with_warning_and_manifest_stays_consistent() {
        let (_dir, db_path, photos_root) = fixture();
        let a = photo_bytes(0x33);
        add_photo(&db_path, &photos_root, 1, "1/uuid-a.jpg", &a);
        add_photo(&db_path, &photos_root, 1, "1/uuid-b.jpg", &photo_bytes(0x44));
        // 引用还在、文件没了（磁盘被清/恢复裸库遗留）
        std::fs::remove_file(photos_root.join("1").join("uuid-b.jpg")).unwrap();

        let mut warnings: Vec<String> = Vec::new();
        {
            let mut on_skip = |msg: &str| warnings.push(msg.to_string());
            let target = _dir.path().join("degraded.zip");
            let outcome =
                build_package(&db_path, &photos_root, CREATED_AT, &target, &mut on_skip)
                    .expect("缺一张照片不整包失败");
            assert_eq!(outcome.photo_count, 1, "实际收入的只剩一张");
            assert_eq!(outcome.skipped_photos.len(), 1);
        }
        // 告警流水逐条落在被跳过的那张上
        assert_eq!(warnings.len(), 1, "实际：{warnings:?}");
        assert!(warnings[0].contains("1/uuid-b.jpg"), "告警带路径：{warnings:?}");

        // 包自洽：manifest 只列实际收入的
        let manifest: serde_json::Value =
            serde_json::from_slice(&read_zip_entry(&_dir.path().join("degraded.zip"), MANIFEST_ENTRY))
                .unwrap();
        let photos = manifest["photos"].as_array().unwrap();
        assert_eq!(photos.len(), 1);
        assert_eq!(photos[0]["path"], "1/uuid-a.jpg");
        assert_eq!(manifest["photo_count"], 1);
        assert_eq!(manifest["total_bytes"], a.len() as u64);
    }

    // ── 永不进包：.tmp- / .orphan- / 库无引用 ────────────────────────────

    #[test]
    fn tmp_orphan_and_unreferenced_files_never_enter_package() {
        let (_dir, db_path, photos_root) = fixture();
        add_photo(&db_path, &photos_root, 1, "1/uuid-a.jpg", &photo_bytes(0x55));
        // 库无引用的散文件
        std::fs::create_dir_all(photos_root.join("1")).unwrap();
        std::fs::write(photos_root.join("1").join("loose.jpg"), "无引用".as_bytes()).unwrap();
        // 半截临时
        std::fs::write(photos_root.join("1").join(".tmp-abc"), "半截".as_bytes()).unwrap();
        // 隔离区（孤儿目录带文件）
        let orphan = photos_root.join(".orphan-20260919-010101");
        std::fs::create_dir_all(orphan.join("1")).unwrap();
        std::fs::write(orphan.join("1").join("old.jpg"), "孤儿".as_bytes()).unwrap();

        let target = _dir.path().join("clean.zip");
        let outcome = build_package(&db_path, &photos_root, CREATED_AT, &target, &mut no_skip())
            .expect("打包成功");
        assert_eq!(outcome.photo_count, 1);
        let names = read_zip_entry_names(&target);
        assert!(names.contains(&"1/uuid-a.jpg".to_string()));
        assert!(
            !names.iter().any(|n| n.contains("loose") || n.contains(".tmp-") || n.contains(".orphan-")),
            "tmp/孤儿/无引用绝不进包，实际：{names:?}"
        );
    }

    // ── 防御层：引用行本身形态不收（老库被外部改过等）一律跳过 ────────────

    #[test]
    fn referenced_paths_failing_grammar_are_skipped_defensively() {
        let (_dir, db_path, photos_root) = fixture();
        let bad_paths = [
            "1/.tmp-x.jpg",      // tmp 前缀（点不在字符集）
            ".orphan-1/x.jpg",   // 孤儿目录开头
            "1/../evil.jpg",     // 越界
            "1\\backslash.jpg",  // 反斜杠
            "1/sub/deep.jpg",    // 三段
            "1/notes.txt",       // 非 .jpg
            "/abs/x.jpg",        // 绝对路径
            "solo.jpg",          // 单段无目录
        ];
        for rel in bad_paths {
            add_photo_row_only(&db_path, 1, rel);
        }

        let mut warnings: Vec<String> = Vec::new();
        {
            let mut on_skip = |msg: &str| warnings.push(msg.to_string());
            let target = _dir.path().join("defensive.zip");
            let outcome =
                build_package(&db_path, &photos_root, CREATED_AT, &target, &mut on_skip)
                    .expect("全是坏路径也不整包失败（零照片包）");
            assert_eq!(outcome.photo_count, 0);
            assert_eq!(outcome.skipped_photos.len(), bad_paths.len());
        }
        assert_eq!(warnings.len(), bad_paths.len());
        let names = read_zip_entry_names(&_dir.path().join("defensive.zip"));
        assert_eq!(
            names.iter().filter(|n| **n != MANIFEST_ENTRY && **n != DB_ENTRY).count(),
            0,
            "坏路径一条都不进包，实际：{names:?}"
        );
    }

    // ── 整包失败与边界 ───────────────────────────────────────────────────

    #[test]
    fn missing_db_snapshot_fails_whole_package_without_partial_target() {
        let (dir, _db_path, photos_root) = fixture();
        let target = dir.path().join("nope.zip");
        let err =
            build_package(&dir.path().join("no-such.db"), &photos_root, CREATED_AT, &target, &mut no_skip())
                .unwrap_err();
        assert!(err.contains("打开库快照失败"), "实际：{err}");
        assert!(!target.exists(), "半截产物不残留");
    }

    #[test]
    fn db_without_photo_table_packs_as_photo_free_package() {
        // 升级前的老库（无 nest_photo 表）：零照片包照常产出
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("old.db");
        {
            let conn = rusqlite::Connection::open(&db_path).unwrap();
            conn.execute_batch(crate::db::V1_SCHEMA_SQL).unwrap();
            crate::db::seed_v1_presets(&conn).unwrap();
            conn.execute("INSERT INTO colony (name, start_date) VALUES ('老窝', '2025-01-01')", [])
                .unwrap();
            conn.pragma_update(None, "user_version", 1).unwrap();
        }
        let empty_photos = dir.path().join("photos");
        let target = dir.path().join("old-pkg.zip");
        let outcome =
            build_package(&db_path, &empty_photos, CREATED_AT, &target, &mut no_skip()).unwrap();
        assert_eq!(outcome.photo_count, 0);
        let manifest: serde_json::Value =
            serde_json::from_slice(&read_zip_entry(&target, MANIFEST_ENTRY)).unwrap();
        assert_eq!(manifest["photo_count"], 0);
        assert_eq!(manifest["photos"].as_array().unwrap().len(), 0);
        assert_eq!(manifest["total_bytes"], 0);
    }

    #[test]
    fn zero_byte_and_grammar_edge_paths() {
        // grammar 判定表：合法形态只有 <段>/<段>.jpg 且段内 [A-Za-z0-9_-]
        assert!(is_packable_rel_path("1/uuid-a.jpg"));
        assert!(is_packable_rel_path("12/0f8d3e2a-1234-4c5d-9e8f-aabbccddeeff.jpg"));
        assert!(!is_packable_rel_path("1/.tmp-x.jpg"));
        assert!(!is_packable_rel_path(".orphan-1/x.jpg"));
        assert!(!is_packable_rel_path("1/../x.jpg"));
        assert!(!is_packable_rel_path("1/x.PNG"));
        assert!(!is_packable_rel_path("1/x.jpg/"));
        assert!(!is_packable_rel_path(""));
        assert!(!is_packable_rel_path("/x.jpg"));
        assert!(!is_packable_rel_path("C:/x.jpg"));
    }

    #[test]
    fn zero_length_referenced_photo_is_included_with_empty_hash() {
        // 0 字节但可读：按空内容收入（哈希=空串哈希），不跳过——跳过只给读不了的
        let (dir, db_path, photos_root) = fixture();
        add_photo(&db_path, &photos_root, 1, "1/empty.jpg", &[]);
        let target = dir.path().join("empty-photo.zip");
        let outcome =
            build_package(&db_path, &photos_root, CREATED_AT, &target, &mut no_skip()).unwrap();
        assert_eq!(outcome.photo_count, 1);
        let manifest: serde_json::Value =
            serde_json::from_slice(&read_zip_entry(&target, MANIFEST_ENTRY)).unwrap();
        assert_eq!(manifest["photos"][0]["bytes"], 0);
        assert_eq!(manifest["photos"][0]["sha256"], hex(&Sha256::digest(Vec::<u8>::new())));
    }
}
