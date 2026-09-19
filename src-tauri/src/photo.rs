//! 巢况照片管线（webui-checkin 票 07，规格 Implementation Decisions E）。
//!
//! 三块职责，全部只吃 `&Connection` 与路径参数，与 Tauri 解耦（lib.rs 命令是薄包装）：
//!
//! 1. **校验 + 重编码纯核**（桌面/网页同一套）：
//!    单张压缩前 ≤15MB、单次提交 ≤9 张、格式白名单 JPEG/PNG/WebP（按魔数探测，
//!    不信扩展名；GIF 不在白名单本就拒）、拒绝动画（多帧 WebP——GIF 已被白名单拒）；
//!    **解码前先读头部宽高**（宽/高 >12000 或 宽×高 >4000 万像素 → 拒，防解码炸弹），
//!    解码再配 `image::Limits` 双保险；完整解码 → 缩放（长边 2048，只缩不放）→
//!    重编码 JPEG 质量 85。重编码天然不携带 APP1/EXIF（含 GPS）。
//! 2. **写入协议**：落盘名一律服务端 UUID；ensure `photos/<colonyId>/` → 写
//!    `photos/<colonyId>/.tmp-<uuid>` 并 fsync → 同目录原子 rename 为 `<uuid>.jpg`
//!    → 库事务插 nest_photo 并提交；失败路径：rename 后插库失败 → 删文件再报错
//!    （不留半截）；库存相对路径，客户端文件名只进 original_name 备注。
//! 3. **删除顺序 + 孤儿治理**：删除先库事务提交、再删文件（文件删失败仅孤儿、
//!    不回滚库）；启动/恢复后把库无引用文件移入 `photos/.orphan-<时间戳>/`
//!    （移动非删除），`.tmp-` 残留直接删；设置页读 [`orphan_stats`] /
//!    [`clean_orphans`] 查看并清理。

use std::collections::HashSet;
use std::io::Cursor;
use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use crate::nest_checkin::NestPhotoMeta;

// ── 常量（规格 E 数字）────────────────────────────────────────────────────

/// 数据目录下的照片根目录名。
pub const PHOTOS_DIR_NAME: &str = "photos";
/// 孤儿隔离目录前缀（完整名 `.orphan-<YYYYMMDD-HHMMSS>`）。
pub const ORPHAN_DIR_PREFIX: &str = ".orphan-";
/// 半截临时文件前缀（完整名 `.tmp-<uuid>`）。
pub const TMP_PREFIX: &str = ".tmp-";
/// 单张压缩前上限：15MB。
pub const MAX_INPUT_BYTES: usize = 15 * 1024 * 1024;
/// 单次提交上限：9 张。
pub const MAX_PHOTOS_PER_SUBMIT: usize = 9;
/// 单边尺寸上限（解码前头部校验）。
pub const MAX_EDGE: u32 = 12_000;
/// 总像素上限（解码前头部校验）：4000 万。
pub const MAX_PIXELS: u64 = 40_000_000;
/// 输出长边：2048（只缩不放）。
pub const LONG_EDGE: u32 = 2048;
/// 输出 JPEG 质量。
const JPEG_QUALITY: u8 = 85;

// ── 校验 + 重编码纯核 ────────────────────────────────────────────────────

/// 一张待写入的照片：原始文件名（只进备注列）+ 处理后的 JPEG 字节。
#[derive(Debug, Clone, PartialEq)]
pub struct PhotoUpload {
    pub original_name: Option<String>,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum DetectedFormat {
    Jpeg,
    Png,
    WebP,
}

/// 单次提交数量闸：≤9 张。
pub fn ensure_batch_size(count: usize) -> Result<(), String> {
    if count > MAX_PHOTOS_PER_SUBMIT {
        return Err(format!("一次最多上传 {MAX_PHOTOS_PER_SUBMIT} 张照片"));
    }
    Ok(())
}

/// 单张压缩前体积闸：≤15MB。
pub fn ensure_input_size(len: usize) -> Result<(), String> {
    if len > MAX_INPUT_BYTES {
        return Err(format!(
            "单张照片压缩前不能超过 {}MB",
            MAX_INPUT_BYTES / 1024 / 1024
        ));
    }
    Ok(())
}

/// 按魔数探测格式（不信扩展名）；白名单外一律拒（GIF/BMP/HEIC 等都在此拦下）。
fn sniff_format(bytes: &[u8]) -> Result<DetectedFormat, String> {
    if bytes.len() >= 3 && bytes[0] == 0xFF && bytes[1] == 0xD8 && bytes[2] == 0xFF {
        return Ok(DetectedFormat::Jpeg);
    }
    const PNG_SIG: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    if bytes.len() >= 8 && bytes[..8] == PNG_SIG {
        return Ok(DetectedFormat::Png);
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Ok(DetectedFormat::WebP);
    }
    Err("仅支持 JPEG / PNG / WebP 图片".into())
}

/// WebP 动画探测：走 RIFF chunk 链——VP8X 的 Animation bit 置位或出现 ANIM
/// 块即多帧。纯字节探测，不依赖 image crate 暴露动画标记。
fn webp_is_animated(bytes: &[u8]) -> bool {
    let mut off = 12usize; // 跳过 "RIFF" + size + "WEBP"
    while off + 8 <= bytes.len() {
        let fourcc = &bytes[off..off + 4];
        let size = u32::from_le_bytes(bytes[off + 4..off + 8].try_into().unwrap()) as usize;
        let payload = off + 8;
        if fourcc == b"ANIM" {
            return true;
        }
        if fourcc == b"VP8X" && payload < bytes.len() && bytes[payload] & 0x02 != 0 {
            return true;
        }
        // chunk payload 奇数长度补 1 字节 padding（RIFF 规约）
        off = payload + size + (size & 1);
    }
    false
}

fn image_format(fmt: DetectedFormat) -> image::ImageFormat {
    match fmt {
        DetectedFormat::Jpeg => image::ImageFormat::Jpeg,
        DetectedFormat::Png => image::ImageFormat::Png,
        DetectedFormat::WebP => image::ImageFormat::WebP,
    }
}

/// 解码前的头部尺寸校验：只读头，不解码像素。宽/高 >12000 或总像素 >4000 万拒
/// （防解码炸弹——小文件巨尺寸头在这里拦下，根本走不到解码）。
fn ensure_header_dimensions(bytes: &[u8], fmt: DetectedFormat) -> Result<(u32, u32), String> {
    let (w, h) = image::ImageReader::with_format(Cursor::new(bytes), image_format(fmt))
        .into_dimensions()
        .map_err(|e| format!("图片头解析失败: {e}"))?;
    if w == 0 || h == 0 {
        return Err(format!("图片尺寸无效（{w}×{h}）"));
    }
    if w > MAX_EDGE || h > MAX_EDGE {
        return Err(format!(
            "图片尺寸超限（{w}×{h}），单边最大 {MAX_EDGE}"
        ));
    }
    if w as u64 * h as u64 > MAX_PIXELS {
        return Err(format!(
            "图片总像素超限（{w}×{h}），最大 {MAX_PIXELS} 像素"
        ));
    }
    Ok((w, h))
}

/// 解码（Limits 双保险：头部校验已拦大头，这里防绕过头部校验的漏网构造）。
fn decode_with_limits(bytes: &[u8], fmt: DetectedFormat) -> Result<image::DynamicImage, String> {
    // Limits 是 non_exhaustive 结构，跨 crate 只能 default 后逐字段改
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_EDGE);
    limits.max_image_height = Some(MAX_EDGE);
    limits.max_alloc = Some(256 * 1024 * 1024);
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), image_format(fmt));
    reader.limits(limits);
    reader
        .decode()
        .map_err(|e| format!("图片解码失败（文件可能已损坏）: {e}"))
}

/// 重编码 JPEG 质量 85：不携带任何 APP1/EXIF 段（GPS 清除的实现原理）。
fn reencode_jpeg(img: &image::DynamicImage) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let encoder =
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY);
    img.write_with_encoder(encoder)
        .map_err(|e| format!("照片重编码失败: {e}"))?;
    Ok(out)
}

/// 处理一张上传照片：完整校验链 → 重编码 JPEG。输入原始字节，输出处理后的
/// JPEG 字节（长边 2048、质量 85、无 EXIF/GPS）。
pub fn process_photo_bytes(bytes: &[u8]) -> Result<Vec<u8>, String> {
    ensure_input_size(bytes.len())?;
    let fmt = sniff_format(bytes)?;
    if fmt == DetectedFormat::WebP && webp_is_animated(bytes) {
        return Err("不支持动态图（动画 WebP）".into());
    }
    ensure_header_dimensions(bytes, fmt)?;
    let img = decode_with_limits(bytes, fmt)?;
    let long_edge = img.width().max(img.height());
    let scaled;
    let img = if long_edge > LONG_EDGE {
        scaled = img.resize(LONG_EDGE, LONG_EDGE, image::imageops::FilterType::Lanczos3);
        &scaled
    } else {
        &img
    };
    reencode_jpeg(img)
}

/// 批量处理：逐张走校验链，任何一张失败整批拒绝（此时还没碰磁盘，不留半截）。
pub fn process_uploads(uploads: Vec<PhotoUpload>) -> Result<Vec<PhotoUpload>, String> {
    ensure_batch_size(uploads.len())?;
    let mut out = Vec::with_capacity(uploads.len());
    for up in uploads {
        let name = up.original_name.clone().unwrap_or_default();
        let bytes = process_photo_bytes(&up.bytes).map_err(|e| format!("{name}: {e}"))?;
        out.push(PhotoUpload {
            original_name: up.original_name,
            bytes,
        });
    }
    Ok(out)
}

/// 单文件路径预检上限：超过按异常路径拒（正常文件选择器产不出这种长度）。
pub const MAX_PATH_LEN: usize = 4096;

/// 读盘前的单文件预检（评审 R1）：metadata 先行——读不到拒、不是常规文件
/// （目录等）拒、路径超长拒、**压缩前 >15MB 拒且不发起读**（堵"任意大文件
/// 路径先整读进 RAM"的内存尖峰面）。体积真闸仍以读后 [`ensure_input_size`]
/// 兜底（metadata 与实际读之间文件可能被换大），魔数白名单在解码链上不动。
fn precheck_photo_path(p: &str) -> Result<(), String> {
    if p.chars().count() > MAX_PATH_LEN {
        return Err(format!("照片路径过长（超过 {MAX_PATH_LEN} 字符）"));
    }
    let meta = std::fs::metadata(p).map_err(|e| format!("读取照片失败（{p}）: {e}"))?;
    if !meta.is_file() {
        return Err(format!("{p}: 不是常规文件（目录等不收）"));
    }
    if meta.len() > MAX_INPUT_BYTES as u64 {
        return Err(format!(
            "{p}: 单张照片压缩前不能超过 {}MB（实际约 {}MB），已跳过读入",
            MAX_INPUT_BYTES / 1024 / 1024,
            meta.len() / 1024 / 1024
        ));
    }
    Ok(())
}

/// 从磁盘读所选照片文件：数量 ≤9；单文件先走 [`precheck_photo_path`] 预检
/// （体积闸在读内存之前），读后 [`ensure_input_size`] 兜底；解码链校验随
/// [`process_uploads`]。重活前置，库锁外做。
pub fn read_photo_files(paths: &[String]) -> Result<Vec<PhotoUpload>, String> {
    ensure_batch_size(paths.len())?;
    let mut out = Vec::with_capacity(paths.len());
    for p in paths {
        precheck_photo_path(p)?;
        let bytes = std::fs::read(p).map_err(|e| format!("读取照片失败（{p}）: {e}"))?;
        ensure_input_size(bytes.len()).map_err(|e| format!("{p}: {e}"))?;
        let original_name = Path::new(p)
            .file_name()
            .map(|s| s.to_string_lossy().to_string());
        out.push(PhotoUpload {
            original_name,
            bytes,
        });
    }
    Ok(out)
}

/// 服务端 UUID v4（getrandom OS 随机源）：落盘文件名一律服务端生成，
/// 客户端文件名不参与路径（规格 E）。
pub fn random_uuid() -> String {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).expect("OS 随机源不可用");
    b[6] = (b[6] & 0x0f) | 0x40; // version 4
    b[8] = (b[8] & 0x3f) | 0x80; // variant 10x
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

// ── 写入协议 ─────────────────────────────────────────────────────────────

/// 把已处理的照片按写入协议落到 `photos/<colonyId>/` 并插元数据：
/// 任一张失败即整体报错停批（已完成的张都是完整状态：文件+库行成对存在）。
pub fn attach_photos(
    conn: &Connection,
    photos_root: &Path,
    checkin_id: i64,
    uploads: &[PhotoUpload],
) -> Result<Vec<NestPhotoMeta>, String> {
    ensure_batch_size(uploads.len())?;
    let colony_id: i64 = conn
        .query_row(
            "SELECT colony_id FROM nest_checkin WHERE id = ?1",
            params![checkin_id],
            |row| row.get(0),
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => "登记不存在".to_string(),
            other => db_err(other),
        })?;
    let dir = photos_root.join(colony_id.to_string());
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建照片目录失败: {e}"))?;
    let dir_name = dir
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let mut saved = Vec::with_capacity(uploads.len());
    for up in uploads {
        saved.push(persist_one(conn, &dir, &dir_name, checkin_id, up)?);
    }
    Ok(saved)
}

/// 单张落库：tmp + fsync → 同目录原子 rename → 库事务插行。rename 后插库失败
/// → 删文件再报错（验收：各断点失败不留半截）。
fn persist_one(
    conn: &Connection,
    dir: &Path,
    dir_name: &str,
    checkin_id: i64,
    up: &PhotoUpload,
) -> Result<NestPhotoMeta, String> {
    let uuid = random_uuid();
    let tmp_path = dir.join(format!("{TMP_PREFIX}{uuid}"));
    let final_path = dir.join(format!("{uuid}.jpg"));
    if let Err(e) = write_fsync_rename(&tmp_path, &final_path, &up.bytes) {
        // 半截 tmp 一并清掉（write 各步失败都可能残留）
        let _ = std::fs::remove_file(&tmp_path);
        return Err(format!("照片写入失败: {e}"));
    }
    // 库存相对路径（统一正斜杠，跨平台口径一致）
    let rel_path = format!("{dir_name}/{uuid}.jpg");
    let insert = || -> Result<i64, String> {
        let tx = conn
            .unchecked_transaction()
            .map_err(db_err)?;
        tx.execute(
            "INSERT INTO nest_photo (checkin_id, rel_path, original_name, note)
             VALUES (?1, ?2, ?3, '')",
            params![checkin_id, rel_path, up.original_name],
        )
        .map_err(db_err)?;
        let id = tx.last_insert_rowid();
        tx.commit().map_err(db_err)?;
        Ok(id)
    };
    match insert() {
        Ok(id) => Ok(NestPhotoMeta {
            id,
            checkin_id,
            rel_path,
            original_name: up.original_name.clone(),
            note: String::new(),
        }),
        Err(e) => {
            // rename 后插库失败 → 删文件再报错，不留半截
            if let Err(del) = std::fs::remove_file(&final_path) {
                return Err(format!("{e}；失败照片的文件清理也失败（遗留孤儿）: {del}"));
            }
            Err(e)
        }
    }
}

/// 写临时文件 → fsync → 同目录 rename（同目录保证原子性，规格 E）。
fn write_fsync_rename(tmp: &Path, final_path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut f = std::fs::File::create(tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    drop(f);
    std::fs::rename(tmp, final_path)?;
    Ok(())
}

fn db_err(e: rusqlite::Error) -> String {
    format!("数据库操作失败: {e}")
}

// ── 删除顺序：先库事务提交，再删文件 ─────────────────────────────────────

/// 收集一条登记的全部照片相对路径（删库前先取，删文件用）。
pub fn collect_checkin_photo_paths(conn: &Connection, checkin_id: i64) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare("SELECT rel_path FROM nest_photo WHERE checkin_id = ?1 ORDER BY id")
        .map_err(db_err)?;
    let rows = stmt
        .query_map(params![checkin_id], |row| row.get(0))
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?;
    Ok(rows)
}

/// 收集一窝（删窝级联）全部照片相对路径。
pub fn collect_colony_photo_paths(conn: &Connection, colony_id: i64) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT np.rel_path FROM nest_photo np
             JOIN nest_checkin nc ON np.checkin_id = nc.id
             WHERE nc.colony_id = ?1 ORDER BY np.id",
        )
        .map_err(db_err)?;
    let rows = stmt
        .query_map(params![colony_id], |row| row.get(0))
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?;
    Ok(rows)
}

/// 库内是否引用该相对路径（webui-checkin 票 08 照片读取端点的存在性闸：
/// 路径 grammar 之外再查库，防枚举库未引用的文件名）。
pub fn rel_path_referenced(conn: &Connection, rel_path: &str) -> Result<bool, String> {
    let found: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM nest_photo WHERE rel_path = ?1 LIMIT 1",
            params![rel_path],
            |row| row.get(0),
        )
        .optional()
        .map_err(db_err)?;
    Ok(found.is_some())
}

/// 库行删除已提交后删文件。逐张尽力删，失败的记为孤儿（调用方落 applog），
/// 绝不回滚库。返回失败清单（人话、逐条）。
pub fn delete_photo_files(photos_root: &Path, rel_paths: &[String]) -> Vec<String> {
    let mut failures = Vec::new();
    for rel in rel_paths {
        if !is_safe_rel_path(rel) {
            failures.push(format!("{rel}: 相对路径形态非法，跳过删除"));
            continue;
        }
        let mut p = photos_root.to_path_buf();
        for comp in rel.split('/') {
            p.push(comp);
        }
        if let Err(e) = std::fs::remove_file(&p) {
            failures.push(format!("{rel}: {e}"));
        }
    }
    failures
}

/// 防路径逃逸的相对路径形态：`<非点开头的目录名>/<.jpg 结尾的文件名>`，
/// 无反斜杠/冒号/绝对路径/`..`。库内 rel_path 只由本模块写入，这里是防御层。
fn is_safe_rel_path(rel: &str) -> bool {
    if rel.contains('\\') || rel.contains(':') || rel.starts_with('/') {
        return false;
    }
    let mut comps = rel.split('/');
    let dir = comps.next().unwrap_or("");
    if dir.is_empty() || dir.starts_with('.') {
        return false;
    }
    match (comps.next(), comps.next()) {
        (Some(file), None) => !file.is_empty() && !file.starts_with('.') && file.ends_with(".jpg"),
        _ => false,
    }
}

// ── 孤儿治理 ─────────────────────────────────────────────────────────────

/// 巡检结果（动作口径全在 outcome 里，调用方落 applog）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OrphanScanOutcome {
    pub orphan_dir_name: String,
    /// 移入隔离区的文件（photos/ 下相对路径）。
    pub moved: Vec<String>,
    /// 直接删除的 `.tmp-` 残留。
    pub removed_tmp: Vec<String>,
    /// 非致命失败（文件原位不动，下轮巡检重试）。
    pub errors: Vec<String>,
}

/// 巡检时间戳（孤儿目录名用）：`YYYYMMDD-HHMMSS`。
pub fn stamp_now() -> String {
    chrono::Local::now().format("%Y%m%d-%H%M%S").to_string()
}

/// 孤儿巡检（启动/恢复后调用）：把库无引用的常规文件移入
/// `photos/.orphan-<stamp>/`（移动非删除），`.tmp-` 残留直接删（半截垃圾）。
/// 已隔离的 `.orphan-*` 目录跳过。全程非致命：单个文件失败记 errors、原位不动。
pub fn scan_orphans(conn: &Connection, photos_root: &Path, stamp: &str) -> OrphanScanOutcome {
    let mut outcome = OrphanScanOutcome {
        orphan_dir_name: format!("{ORPHAN_DIR_PREFIX}{stamp}"),
        moved: Vec::new(),
        removed_tmp: Vec::new(),
        errors: Vec::new(),
    };
    let referenced: HashSet<String> = match load_referenced_paths(conn) {
        Ok(set) => set,
        Err(e) => {
            outcome.errors.push(e);
            return outcome;
        }
    };
    let entries = match std::fs::read_dir(photos_root) {
        Ok(entries) => entries,
        Err(e) => {
            // photos/ 不存在 = 还没有照片，直接无事而返
            if e.kind() != std::io::ErrorKind::NotFound {
                outcome.errors.push(format!("读取照片目录失败: {e}"));
            }
            return outcome;
        }
    };
    let orphan_root = photos_root.join(&outcome.orphan_dir_name);
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with(ORPHAN_DIR_PREFIX) {
            continue; // 已隔离区不动
        }
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if is_dir {
            scan_colony_dir(&referenced, &entry.path(), &name, &orphan_root, &mut outcome);
        } else {
            // 散落在 photos/ 根的文件（正常不会有）：tmp 删、其余全按无引用隔离
            scan_root_file(&referenced, &entry.path(), &name, &orphan_root, &mut outcome);
        }
    }
    outcome
}

fn load_referenced_paths(conn: &Connection) -> Result<HashSet<String>, String> {
    let mut stmt = conn
        .prepare("SELECT rel_path FROM nest_photo")
        .map_err(db_err)?;
    let rows = stmt
        .query_map([], |row| row.get(0))
        .map_err(db_err)?
        .collect::<Result<HashSet<_>, _>>()
        .map_err(db_err)?;
    Ok(rows)
}

fn scan_colony_dir(
    referenced: &HashSet<String>,
    dir_path: &Path,
    dir_name: &str,
    orphan_root: &Path,
    outcome: &mut OrphanScanOutcome,
) {
    let entries = match std::fs::read_dir(dir_path) {
        Ok(entries) => entries,
        Err(e) => {
            outcome.errors.push(format!("读取照片子目录 {dir_name} 失败: {e}"));
            return;
        }
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let path = entry.path();
        let is_file = entry.file_type().map(|t| t.is_file()).unwrap_or(false);
        if !is_file {
            continue;
        }
        if name.starts_with(TMP_PREFIX) {
            match std::fs::remove_file(&path) {
                Ok(()) => outcome.removed_tmp.push(format!("{dir_name}/{name}")),
                Err(e) => outcome.errors.push(format!("{dir_name}/{name}: {e}")),
            }
            continue;
        }
        let rel = format!("{dir_name}/{name}");
        if referenced.contains(&rel) {
            continue;
        }
        move_to_orphan(&path, &rel, orphan_root, outcome);
    }
}

fn scan_root_file(
    referenced: &HashSet<String>,
    path: &Path,
    name: &str,
    orphan_root: &Path,
    outcome: &mut OrphanScanOutcome,
) {
    if name.starts_with(TMP_PREFIX) {
        match std::fs::remove_file(path) {
            Ok(()) => outcome.removed_tmp.push(name.to_string()),
            Err(e) => outcome.errors.push(format!("{name}: {e}")),
        }
        return;
    }
    if referenced.contains(name) {
        return; // 理论外（库存的两段式路径），保守不碰
    }
    move_to_orphan(path, name, orphan_root, outcome);
}

fn move_to_orphan(path: &Path, rel: &str, orphan_root: &Path, outcome: &mut OrphanScanOutcome) {
    if let Some(parent) = Path::new(rel).parent() {
        let target_dir = orphan_root.join(parent);
        if let Err(e) = std::fs::create_dir_all(&target_dir) {
            outcome.errors.push(format!("{rel}: 建隔离目录失败: {e}"));
            return;
        }
    }
    match std::fs::rename(path, orphan_root.join(rel)) {
        Ok(()) => outcome.moved.push(rel.to_string()),
        Err(e) => outcome.errors.push(format!("{rel}: {e}")),
    }
}

/// 设置页孤儿统计（`photos/.orphan-*` 隔离区现状）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OrphanPhotoStats {
    pub dir_count: usize,
    pub file_count: u64,
    pub total_bytes: u64,
}

pub fn orphan_stats(photos_root: &Path) -> OrphanPhotoStats {
    let mut stats = OrphanPhotoStats {
        dir_count: 0,
        file_count: 0,
        total_bytes: 0,
    };
    let Ok(entries) = std::fs::read_dir(photos_root) else {
        return stats;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with(ORPHAN_DIR_PREFIX) || !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        stats.dir_count += 1;
        walk_files(&entry.path(), &mut |size| {
            stats.file_count += 1;
            stats.total_bytes += size;
        });
    }
    stats
}

fn walk_files(dir: &Path, f: &mut impl FnMut(u64)) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let is_file = entry.file_type().map(|t| t.is_file()).unwrap_or(false);
        if is_file {
            if let Ok(meta) = entry.metadata() {
                f(meta.len());
            }
        } else {
            walk_files(&entry.path(), f);
        }
    }
}

/// 一键清理：删除全部 `.orphan-*` 目录。返回删除量与释放字节；个别目录删除
/// 失败记 errors（下次再清），不影响其余。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OrphanCleanOutcome {
    pub removed_dirs: usize,
    pub freed_bytes: u64,
    pub errors: Vec<String>,
}

pub fn clean_orphans(photos_root: &Path) -> OrphanCleanOutcome {
    let mut outcome = OrphanCleanOutcome {
        removed_dirs: 0,
        freed_bytes: 0,
        errors: Vec::new(),
    };
    let Ok(entries) = std::fs::read_dir(photos_root) else {
        return outcome;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with(ORPHAN_DIR_PREFIX) || !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let mut bytes = 0u64;
        walk_files(&entry.path(), &mut |size| bytes += size);
        match std::fs::remove_dir_all(entry.path()) {
            Ok(()) => {
                outcome.removed_dirs += 1;
                outcome.freed_bytes += bytes;
            }
            Err(e) => outcome.errors.push(format!("{name}: {e}")),
        }
    }
    outcome
}

// ── 测试：只测外部行为（spec「Testing Decisions」）────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, ImageFormat, Rgb, RgbImage};
    use tempfile::TempDir;

    /// 建内存库并迁移到最新 schema，外键照生产连接开启。
    fn mem_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("内存库打开失败");
        crate::db::migrate(&conn).expect("迁移失败");
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        conn
    }

    fn colony(conn: &Connection, name: &str) -> i64 {
        conn.execute(
            "INSERT INTO colony (name, start_date, status) VALUES (?1, '2026-01-20', 'active')",
            params![name],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    /// 一条只有备注的合法登记（照片挂靠点）。
    fn checkin(conn: &Connection, colony_id: i64) -> i64 {
        crate::nest_checkin::save_checkin(
            conn,
            &crate::nest_checkin::CheckinInput {
                colony_id,
                date: "2026-09-18".into(),
                queen_count: None,
                worker_count: None,
                moved_nest: false,
                note: Some("带照片".into()),
            },
            "2026-09-18",
            "2026-09-18 21:00:00",
        )
        .unwrap()
        .id
    }

    fn gradient(w: u32, h: u32) -> DynamicImage {
        let mut img = RgbImage::new(w, h);
        for y in 0..h {
            for x in 0..w {
                img.put_pixel(
                    x,
                    y,
                    Rgb([(x % 256) as u8, (y % 256) as u8, ((x + y) % 256) as u8]),
                );
            }
        }
        DynamicImage::ImageRgb8(img)
    }

    fn encode_bytes(img: &DynamicImage, fmt: ImageFormat) -> Vec<u8> {
        let mut buf = Cursor::new(Vec::new());
        img.write_to(&mut buf, fmt).unwrap();
        buf.into_inner()
    }

    fn jpeg_bytes(w: u32, h: u32) -> Vec<u8> {
        encode_bytes(&gradient(w, h), ImageFormat::Jpeg)
    }

    fn png_bytes(w: u32, h: u32) -> Vec<u8> {
        encode_bytes(&gradient(w, h), ImageFormat::Png)
    }

    fn webp_bytes(w: u32, h: u32) -> Vec<u8> {
        encode_bytes(&gradient(w, h), ImageFormat::WebP)
    }

    fn decoded_size(bytes: &[u8]) -> (u32, u32) {
        let img = image::load_from_memory(bytes).unwrap();
        (img.width(), img.height())
    }

    /// 解码炸弹夹具（JPEG）：拿真实编码的小 JPEG，按标记流找到 SOF0，把宽高
    /// 改写成超大值——文件其余部分（DHT/SOS/扫描数据）原样，头部校验必须在
    /// 解码前拦下这种「小文件巨尺寸头」。
    fn jpeg_with_patched_dims(base: &[u8], w: u16, h: u16) -> Vec<u8> {
        let mut v = base.to_vec();
        let mut i = 2usize; // 跳过 SOI
        loop {
            assert_eq!(v[i], 0xFF, "标记流错位 @ {i}");
            let marker = v[i + 1];
            assert!(marker != 0xDA && marker != 0xD9, "走到 {marker:#x} 还没见 SOF0");
            let len = u16::from_be_bytes([v[i + 2], v[i + 3]]) as usize;
            if marker == 0xC0 || marker == 0xC2 {
                // 段内布局：FF C0 len(2) 精度(1) 高(2) 宽(2) → 偏移 5..9
                v[i + 5..i + 7].copy_from_slice(&h.to_be_bytes());
                v[i + 7..i + 9].copy_from_slice(&w.to_be_bytes());
                return v;
            }
            i += 2 + len;
        }
    }

    fn crc32(data: &[u8]) -> u32 {
        let mut crc: u32 = 0xFFFF_FFFF;
        for &b in data {
            crc ^= b as u32;
            for _ in 0..8 {
                let mask = (crc & 1).wrapping_neg();
                crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
            }
        }
        !crc
    }

    /// 解码炸弹夹具（PNG）：拿真实编码的小 PNG，改写 IHDR 宽高并重算 CRC。
    fn png_with_patched_dims(base: &[u8], w: u32, h: u32) -> Vec<u8> {
        let mut v = base.to_vec();
        // IHDR 载荷起点 = 8 签名 + 4 长度 + 4 类型 = 16
        v[16..20].copy_from_slice(&w.to_be_bytes());
        v[20..24].copy_from_slice(&h.to_be_bytes());
        let crc = crc32(&v[12..29]); // CRC 覆盖 类型+载荷（13 字节）
        v[29..33].copy_from_slice(&crc.to_be_bytes());
        v
    }

    /// 动画 WebP 夹具：VP8X 的 Animation bit 置位 + ANIM 块（手工拼）。
    fn animated_webp_bytes() -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(b"WEBP");
        payload.extend_from_slice(b"VP8X");
        payload.extend_from_slice(&10u32.to_le_bytes());
        payload.extend_from_slice(&[0x02, 0, 0, 0]); // flags: animation bit
        payload.extend_from_slice(&[1, 0, 0, 1, 0, 0]); // canvas 2×2
        payload.extend_from_slice(b"ANIM");
        payload.extend_from_slice(&6u32.to_le_bytes());
        payload.extend_from_slice(&[0, 0, 0xFF, 0xFF, 0, 0]);
        let mut v = Vec::new();
        v.extend_from_slice(b"RIFF");
        v.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        v.extend_from_slice(&payload);
        v
    }

    /// 把 APP1（Exif 头 + GPS 痕迹 + 独有标记）插到 SOI 之后。
    fn jpeg_with_exif_gps(jpeg: &[u8]) -> Vec<u8> {
        const MARKER: &[u8] = b"ANT-GPS-PROOF-20260919";
        let mut payload = Vec::new();
        payload.extend_from_slice(b"Exif\0\0");
        payload.extend_from_slice(b"MM\0\x2a\0\0\0\x08"); // TIFF 头（大端，IFD0 偏移 8）
        payload.extend_from_slice(b"GPS\0");
        payload.extend_from_slice(MARKER);
        let mut app1 = Vec::new();
        app1.extend_from_slice(&[0xFF, 0xE1]);
        app1.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
        app1.extend_from_slice(&payload);
        let mut out = Vec::new();
        out.extend_from_slice(&jpeg[..2]);
        out.extend_from_slice(&app1);
        out.extend_from_slice(&jpeg[2..]);
        out
    }

    /// SOS 之前的段标记清单（0xE1 = APP1/EXIF）。
    fn jpeg_app_markers(jpeg: &[u8]) -> Vec<u8> {
        let mut i = 2usize;
        let mut found = Vec::new();
        while i + 4 <= jpeg.len() && jpeg[i] == 0xFF {
            let marker = jpeg[i + 1];
            if marker == 0xD9 || (0xD0..=0xD7).contains(&marker) || marker == 0xDA {
                break;
            }
            let len = u16::from_be_bytes([jpeg[i + 2], jpeg[i + 3]]) as usize;
            found.push(marker);
            i += 2 + len;
        }
        found
    }

    fn upload(name: &str, bytes: Vec<u8>) -> PhotoUpload {
        PhotoUpload {
            original_name: Some(name.into()),
            bytes,
        }
    }

    fn photo_dir(tmp: &TempDir) -> std::path::PathBuf {
        tmp.path().join(PHOTOS_DIR_NAME)
    }

    // ── 校验链 ──

    #[test]
    fn process_accepts_jpeg_downscales_long_edge_to_2048() {
        let out = process_photo_bytes(&jpeg_bytes(3000, 2000)).unwrap();
        assert_eq!(decoded_size(&out), (2048, 1365), "长边缩到 2048，比例保持");
    }

    #[test]
    fn process_keeps_small_image_unscaled() {
        // 只缩不放：长边不足 2048 的原图尺寸原样
        let out = process_photo_bytes(&jpeg_bytes(800, 600)).unwrap();
        assert_eq!(decoded_size(&out), (800, 600));
        // PNG/WebP 白名单同样直通重编码
        let out_png = process_photo_bytes(&png_bytes(300, 200)).unwrap();
        assert_eq!(decoded_size(&out_png), (300, 200));
        let out_webp = process_photo_bytes(&webp_bytes(64, 48)).unwrap();
        assert_eq!(decoded_size(&out_webp), (64, 48));
    }

    #[test]
    fn process_rejects_input_over_15mb() {
        let big = vec![0u8; MAX_INPUT_BYTES + 1];
        let err = process_photo_bytes(&big).unwrap_err();
        assert!(err.contains("15"), "实际错误：{err}");
    }

    #[test]
    fn read_photo_files_rejects_oversize_file_before_reading() {
        // 评审 R1：体积闸前置到 metadata 预检——超限文件**不发起读**，
        // 错误信息带「跳过读入」标记（只有 metadata 路径产出，即证明未整读进 RAM）
        let tmp = TempDir::new().unwrap();
        let big_path = tmp.path().join("big.jpg");
        std::fs::write(&big_path, vec![0u8; MAX_INPUT_BYTES + 1]).unwrap();

        let err = read_photo_files(&[big_path.to_string_lossy().to_string()]).unwrap_err();
        assert!(err.contains("跳过读入"), "错误应来自 metadata 预检：{err}");
        assert!(err.contains("15"), "实际错误：{err}");
    }

    #[test]
    fn read_photo_files_rejects_directory_missing_and_overlong_paths() {
        let tmp = TempDir::new().unwrap();
        // 目录：metadata 拿得到但不是常规文件 → 拒
        let err = read_photo_files(&[tmp.path().to_string_lossy().to_string()]).unwrap_err();
        assert!(err.contains("不是常规文件"), "实际错误：{err}");
        // 不存在的路径：metadata 失败 → 拒
        let missing = tmp.path().join("nope.jpg");
        let err = read_photo_files(&[missing.to_string_lossy().to_string()]).unwrap_err();
        assert!(err.contains("读取照片失败"), "实际错误：{err}");
        // 超长路径：预检直接拒（先于 metadata）
        let long = format!("C:\\{}", "a".repeat(MAX_PATH_LEN + 1));
        let err = read_photo_files(&[long]).unwrap_err();
        assert!(err.contains("路径过长"), "实际错误：{err}");
    }

    #[test]
    fn read_photo_files_reads_small_file_with_original_name() {
        let tmp = TempDir::new().unwrap();
        let p = tmp.path().join("IMG_007.jpg");
        std::fs::write(&p, b"\xFF\xD8\xFF-tiny").unwrap();

        let ups = read_photo_files(&[p.to_string_lossy().to_string()]).unwrap();
        assert_eq!(ups.len(), 1);
        assert_eq!(ups[0].bytes, b"\xFF\xD8\xFF-tiny");
        assert_eq!(ups[0].original_name.as_deref(), Some("IMG_007.jpg"));
    }

    #[test]
    fn process_rejects_non_whitelisted_formats() {
        // GIF 不在白名单本就拒
        let mut gif = b"GIF89a".to_vec();
        gif.extend_from_slice(&[0u8; 32]);
        assert!(process_photo_bytes(&gif).unwrap_err().contains("JPEG / PNG / WebP"));
        // 纯文本（改了扩展名的假图）同样拒
        assert!(process_photo_bytes(b"not an image at all").is_err());
    }

    #[test]
    fn process_rejects_animated_webp_but_accepts_static() {
        let err = process_photo_bytes(&animated_webp_bytes()).unwrap_err();
        assert!(err.contains("动态"), "实际错误：{err}");
        // 静态 WebP（image crate 编码）放行
        assert!(process_photo_bytes(&webp_bytes(64, 48)).is_ok());
    }

    #[test]
    fn process_rejects_bomb_headers_before_decoding() {
        // 小文件巨尺寸头 JPEG：单边超 12000
        let bomb = jpeg_with_patched_dims(&jpeg_bytes(64, 48), 30000, 30000);
        assert!(bomb.len() < 4096, "夹具必须是小文件，实际 {} 字节", bomb.len());
        let err = process_photo_bytes(&bomb).unwrap_err();
        assert!(err.contains("超限"), "实际错误：{err}");

        // PNG 单边超 12000
        let err = process_photo_bytes(&png_with_patched_dims(&png_bytes(64, 48), 20000, 10))
            .unwrap_err();
        assert!(err.contains("超限"), "实际错误：{err}");

        // 单边都在限内但总像素超 4000 万（10000×5000 = 5000 万）
        let err = process_photo_bytes(&png_with_patched_dims(&png_bytes(64, 48), 10000, 5000))
            .unwrap_err();
        assert!(err.contains("像素"), "实际错误：{err}");

        // JPEG 同理（12000×3400 = 4080 万）
        let err = process_photo_bytes(&jpeg_with_patched_dims(&jpeg_bytes(64, 48), 12000, 3400))
            .unwrap_err();
        assert!(err.contains("像素"), "实际错误：{err}");

        // 边界内放行（头部关）：5000×8000 = 4000 万整，不因像素数被拒
        // （随后死在真实数据只有 64×48 的解码上，报「解码失败」而非「像素」）
        let err = process_photo_bytes(&png_with_patched_dims(&png_bytes(64, 48), 5000, 8000))
            .unwrap_err();
        assert!(!err.contains("像素"), "边界内不得因像素数被拒：{err}");
    }

    #[test]
    fn process_strips_exif_gps_from_jpeg() {
        let with_exif = jpeg_with_exif_gps(&jpeg_bytes(64, 48));
        // 控制断言：夹具确实带 EXIF（APP1 段 + GPS 痕迹）
        assert!(jpeg_app_markers(&with_exif).contains(&0xE1), "夹具应含 APP1");
        assert!(with_exif.windows(4).any(|w| w == b"GPS\0"));
        assert!(with_exif.windows(6).any(|w| w == b"Exif\0\0"));

        let out = process_photo_bytes(&with_exif).unwrap();
        // 重编码后：输出可解码、无 APP1 段、无 EXIF 签名、无 GPS 痕迹、无独有标记
        assert_eq!(decoded_size(&out), (64, 48), "输出仍是合法 JPEG");
        assert!(!jpeg_app_markers(&out).contains(&0xE1), "输出不得有 APP1 段");
        assert!(!out.windows(6).any(|w| w == b"Exif\0\0"));
        assert!(!out.windows(4).any(|w| w == b"GPS\0"));
        assert!(!out.windows(8).any(|w| w == b"ANT-GPS-"));
    }

    #[test]
    fn process_uploads_rejects_whole_batch_on_any_bad_photo() {
        let ups = vec![
            upload("a.jpg", jpeg_bytes(64, 48)),
            upload("evil.gif", b"GIF89a-evil".to_vec()),
        ];
        let err = process_uploads(ups).unwrap_err();
        assert!(err.contains("evil.gif"), "错误带文件名：{err}");
        assert!(err.contains("JPEG / PNG / WebP"));
    }

    #[test]
    fn random_uuid_is_canonical_v4() {
        for _ in 0..32 {
            let u = random_uuid();
            assert_eq!(u.len(), 36);
            let chars: Vec<char> = u.chars().collect();
            assert_eq!(chars[8], '-');
            assert_eq!(chars[13], '-');
            assert_eq!(chars[18], '-');
            assert_eq!(chars[23], '-');
            assert_eq!(chars[14], '4', "version 4");
            assert!(chars.iter().all(|c| *c == '-' || c.is_ascii_hexdigit()));
        }
        // 两次生成为不同值（随机源活着）
        assert_ne!(random_uuid(), random_uuid());
    }

    // ── 写入协议 ──

    #[test]
    fn attach_writes_uuid_file_and_metadata_row() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        let id = checkin(&conn, c);
        let tmp = TempDir::new().unwrap();
        let ups = vec![upload("手机相册 IMG_001.jpg", jpeg_bytes(64, 48))];

        let saved = attach_photos(&conn, &photo_dir(&tmp), id, &ups).unwrap();
        assert_eq!(saved.len(), 1);
        let meta = &saved[0];
        assert_eq!(meta.checkin_id, id);
        assert_eq!(
            meta.original_name.as_deref(),
            Some("手机相册 IMG_001.jpg"),
            "客户端文件名只进 original_name 备注"
        );
        // rel_path = <colonyId>/<uuid>.jpg：目录段是窝 id，文件名是服务端 UUID
        let (dir_seg, file_seg) = meta.rel_path.split_once('/').unwrap();
        assert_eq!(dir_seg, c.to_string());
        assert!(file_seg.ends_with(".jpg"));
        assert_eq!(file_seg.len(), 36 + 4, "文件名 = 36 位 UUID + .jpg");
        assert!(!meta.rel_path.contains("IMG_001"), "落盘名与客户端文件名无关");

        // 文件真实存在且字节就是处理后的内容
        let on_disk = std::fs::read(photo_dir(&tmp).join(&meta.rel_path)).unwrap();
        assert_eq!(on_disk, ups[0].bytes);
        // 目录里没有 .tmp- 残留
        let leftovers: Vec<String> = std::fs::read_dir(photo_dir(&tmp).join(c.to_string()))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(leftovers.len(), 1, "目录里只有落定的一张");

        // 库行可查回
        let paths = collect_checkin_photo_paths(&conn, id).unwrap();
        assert_eq!(paths, vec![meta.rel_path.clone()]);
    }

    #[test]
    fn attach_rejects_batch_over_nine_without_touching_disk() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        let id = checkin(&conn, c);
        let tmp = TempDir::new().unwrap();
        let ups: Vec<PhotoUpload> = (0..10)
            .map(|i| upload(&format!("{i}.jpg"), jpeg_bytes(64, 48)))
            .collect();
        let err = attach_photos(&conn, &photo_dir(&tmp), id, &ups).unwrap_err();
        assert!(err.contains("9"), "实际错误：{err}");
        assert!(!photo_dir(&tmp).exists(), "超批拒绝不碰磁盘");
    }

    #[test]
    fn attach_unknown_checkin_rejected_without_creating_dirs() {
        let conn = mem_conn();
        let tmp = TempDir::new().unwrap();
        let ups = vec![upload("a.jpg", jpeg_bytes(64, 48))];
        let err = attach_photos(&conn, &photo_dir(&tmp), 999, &ups).unwrap_err();
        assert!(err.contains("登记不存在"), "实际错误：{err}");
        assert!(!photo_dir(&tmp).exists(), "未知登记不建目录");
    }

    #[test]
    fn attach_insert_failure_after_rename_removes_file() {
        // 验收：rename 后插库失败 → 文件被清，不留半截
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        let id = checkin(&conn, c);
        let tmp = TempDir::new().unwrap();
        conn.execute("DROP TABLE nest_photo", []).unwrap(); // 注入插库失败

        let ups = vec![upload("a.jpg", jpeg_bytes(64, 48))];
        let err = attach_photos(&conn, &photo_dir(&tmp), id, &ups).unwrap_err();
        assert!(err.contains("数据库操作失败"), "实际错误：{err}");

        let colony_dir = photo_dir(&tmp).join(c.to_string());
        assert!(colony_dir.exists());
        let leftovers: Vec<String> = std::fs::read_dir(&colony_dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert!(leftovers.is_empty(), "renamed 文件与 tmp 都应被清掉，实际残留 {leftovers:?}");
    }

    #[test]
    fn attach_multiple_photos_all_land() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        let id = checkin(&conn, c);
        let tmp = TempDir::new().unwrap();
        let ups = vec![
            upload("1.jpg", jpeg_bytes(64, 48)),
            upload("2.png", png_bytes(80, 60)),
            upload("3.webp", webp_bytes(40, 30)),
        ];
        let saved = attach_photos(&conn, &photo_dir(&tmp), id, &ups).unwrap();
        assert_eq!(saved.len(), 3);
        for meta in &saved {
            assert!(photo_dir(&tmp).join(&meta.rel_path).exists());
        }
        let paths = collect_checkin_photo_paths(&conn, id).unwrap();
        assert_eq!(paths.len(), 3);
    }

    // ── 删除顺序 ──

    #[test]
    fn rel_path_referenced_reflects_rows() {
        // 票 08 读取端点的存在性闸：库有该行 → true；无行（含没插入过）→ false
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        let id = checkin(&conn, c);
        assert!(!rel_path_referenced(&conn, "1/a.jpg").unwrap());
        conn.execute(
            "INSERT INTO nest_photo (checkin_id, rel_path, original_name, note) VALUES (?1, '1/a.jpg', NULL, '')",
            params![id],
        )
        .unwrap();
        assert!(rel_path_referenced(&conn, "1/a.jpg").unwrap());
        assert!(!rel_path_referenced(&conn, "1/b.jpg").unwrap(), "别的文件名不牵连");
    }

    #[test]
    fn delete_checkin_then_files_removes_rows_first_and_its_files_only() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        let a = checkin(&conn, c);
        let b = crate::nest_checkin::save_checkin(
            &conn,
            &crate::nest_checkin::CheckinInput {
                colony_id: c,
                date: "2026-09-17".into(),
                queen_count: None,
                worker_count: None,
                moved_nest: false,
                note: Some("另一条".into()),
            },
            "2026-09-18",
            "2026-09-18 21:00:00",
        )
        .unwrap()
        .id;
        let tmp = TempDir::new().unwrap();
        let pa = &attach_photos(&conn, &photo_dir(&tmp), a, &[upload("a.jpg", jpeg_bytes(64, 48))]).unwrap()[0];
        let pb = &attach_photos(&conn, &photo_dir(&tmp), b, &[upload("b.jpg", jpeg_bytes(64, 48))]).unwrap()[0];

        // 顺序：先取路径 → 库事务删行提交 → 再删文件（lib.rs 命令同此序）
        let rel_paths = collect_checkin_photo_paths(&conn, a).unwrap();
        crate::nest_checkin::delete_checkin(&conn, a).unwrap();
        let failures = delete_photo_files(&photo_dir(&tmp), &rel_paths);
        assert!(failures.is_empty(), "文件删除失败清单应空：{failures:?}");

        assert!(!photo_dir(&tmp).join(&pa.rel_path).exists(), "该登记的照片文件被删");
        assert!(photo_dir(&tmp).join(&pb.rel_path).exists(), "别条登记的照片不牵连");
        assert!(collect_checkin_photo_paths(&conn, a).unwrap().is_empty(), "库行已删");
    }

    #[test]
    fn delete_photo_files_failure_reports_orphan_and_keeps_rest() {
        // 注入：把目标"文件"换成同名目录 → remove_file 必失败
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        let id = checkin(&conn, c);
        let tmp = TempDir::new().unwrap();
        let saved = attach_photos(&conn, &photo_dir(&tmp), id, &[upload("a.jpg", jpeg_bytes(64, 48))]).unwrap();
        let p1 = &saved[0].rel_path;
        let p2 = {
            let more = attach_photos(&conn, &photo_dir(&tmp), id, &[upload("b.jpg", jpeg_bytes(64, 48))]).unwrap();
            more[0].rel_path.clone()
        };
        // p1 换成目录（注入删失败），p2 正常
        std::fs::remove_file(photo_dir(&tmp).join(p1)).unwrap();
        std::fs::create_dir(photo_dir(&tmp).join(p1)).unwrap();

        let failures = delete_photo_files(&photo_dir(&tmp), &[p1.clone(), p2.clone()]);
        assert_eq!(failures.len(), 1, "只 p1 失败：{failures:?}");
        assert!(failures[0].contains(p1));
        assert!(!photo_dir(&tmp).join(&p2).exists(), "p2 照常删掉");
        assert!(photo_dir(&tmp).join(p1).is_dir(), "p1 留在原地（孤儿，不回滚库）");
    }

    #[test]
    fn delete_photo_files_rejects_unsafe_rel_path() {
        let tmp = TempDir::new().unwrap();
        let failures = delete_photo_files(
            &photo_dir(&tmp),
            &["../escape.jpg".into(), "C:/abs/abs.jpg".into(), "1\\win.jpg".into(), "1/ok.jpg".into()],
        );
        assert_eq!(failures.len(), 4, "三条非法 + 一条不存在：{failures:?}");
        assert!(failures[0].contains("非法"));
        assert!(failures[1].contains("非法"));
        assert!(failures[2].contains("非法"));
        assert!(!failures[3].contains("非法"), "合法形态的报错应是文件不存在的 IO 错");
    }

    #[test]
    fn collect_colony_photo_paths_covers_all_checkins() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        let a = checkin(&conn, c);
        let b = crate::nest_checkin::save_checkin(
            &conn,
            &crate::nest_checkin::CheckinInput {
                colony_id: c,
                date: "2026-09-17".into(),
                queen_count: None,
                worker_count: None,
                moved_nest: false,
                note: Some("乙".into()),
            },
            "2026-09-18",
            "2026-09-18 21:00:00",
        )
        .unwrap()
        .id;
        let tmp = TempDir::new().unwrap();
        attach_photos(&conn, &photo_dir(&tmp), a, &[upload("a.jpg", jpeg_bytes(64, 48))]).unwrap();
        attach_photos(&conn, &photo_dir(&tmp), b, &[upload("b.jpg", jpeg_bytes(64, 48))]).unwrap();

        let mut paths = collect_colony_photo_paths(&conn, c).unwrap();
        paths.sort();
        assert_eq!(paths.len(), 2, "删窝级联要拿到全窝照片");
    }

    // ── 孤儿巡检 ──

    /// 造照片目录现场：引用文件 + 孤儿文件 + tmp 残留 + 散落文件 + 既有隔离区。
    fn seed_photo_tree(root: &Path) -> String {
        // 1/a.jpg（库引用）、1/b.jpg（孤儿）、1/.tmp-x（残留）、2/c.jpg（孤儿）、
        // 根下 loose.jpg（散落）、既有 .orphan-123/（不碰）
        let d1 = root.join("1");
        let d2 = root.join("2");
        std::fs::create_dir_all(&d1).unwrap();
        std::fs::create_dir_all(&d2).unwrap();
        std::fs::create_dir_all(root.join(".orphan-123")).unwrap();
        std::fs::write(d1.join("a.jpg"), b"referenced").unwrap();
        std::fs::write(d1.join("b.jpg"), b"orphan-b").unwrap();
        std::fs::write(d1.join(".tmp-x"), b"half-written").unwrap();
        std::fs::write(d2.join("c.jpg"), b"orphan-c").unwrap();
        std::fs::write(root.join("loose.jpg"), b"loose").unwrap();
        std::fs::write(root.join(".orphan-123").join("old.jpg"), b"old").unwrap();
        "1/a.jpg".into()
    }

    #[test]
    fn scan_orphans_moves_unreferenced_deletes_tmp_keeps_referenced() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        let id = checkin(&conn, c);
        let tmp = TempDir::new().unwrap();
        let root = photo_dir(&tmp);
        let referenced = seed_photo_tree(&root);
        // 让 1/a.jpg 成为库引用
        conn.execute(
            "INSERT INTO nest_photo (checkin_id, rel_path, original_name, note) VALUES (?1, ?2, NULL, '')",
            params![id, referenced],
        )
        .unwrap();

        let outcome = scan_orphans(&conn, &root, "20260919-080000");
        assert!(outcome.errors.is_empty(), "巡检不应有错误：{:?}", outcome.errors);
        assert_eq!(outcome.orphan_dir_name, ".orphan-20260919-080000");
        let mut moved = outcome.moved.clone();
        moved.sort();
        assert_eq!(moved, vec!["1/b.jpg", "2/c.jpg", "loose.jpg"], "无引用文件全部移入隔离区");
        assert_eq!(outcome.removed_tmp, vec!["1/.tmp-x"], "tmp 残留直接删");

        // 磁盘现状：引用的原位、孤儿进了隔离区、tmp 没了
        assert!(root.join("1/a.jpg").exists(), "引用文件原位不动");
        assert!(root.join(".orphan-20260919-080000").join("1/b.jpg").exists());
        assert!(root.join(".orphan-20260919-080000").join("2/c.jpg").exists());
        assert!(root.join(".orphan-20260919-080000").join("loose.jpg").exists());
        assert!(!root.join("1/b.jpg").exists());
        assert!(!root.join("1/.tmp-x").exists());
        // 既有隔离区原样
        assert!(root.join(".orphan-123").join("old.jpg").exists());

        // 再扫一轮（新时间戳）：无新增动作，引用文件安然
        let second = scan_orphans(&conn, &root, "20260919-090000");
        assert!(second.moved.is_empty(), "第二轮不应再收到文件：{:?}", second.moved);
        assert!(second.removed_tmp.is_empty());
        assert!(root.join("1/a.jpg").exists());
    }

    #[test]
    fn orphan_stats_then_clean_reports_and_frees() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        let id = checkin(&conn, c);
        let tmp = TempDir::new().unwrap();
        let root = photo_dir(&tmp);
        let referenced = seed_photo_tree(&root);
        conn.execute(
            "INSERT INTO nest_photo (checkin_id, rel_path, original_name, note) VALUES (?1, ?2, NULL, '')",
            params![id, referenced],
        )
        .unwrap();
        scan_orphans(&conn, &root, "20260919-080000");

        // 统计：1 个新隔离目录、3 个文件（b/c/loose）、既有 .orphan-123 也计入
        let stats = orphan_stats(&root);
        assert_eq!(stats.dir_count, 2, "新隔离目录 + 既有 .orphan-123");
        assert_eq!(stats.file_count, 4, "b/c/loose + old");
        assert_eq!(stats.total_bytes, (b"orphan-b".len() + b"orphan-c".len() + b"loose".len() + b"old".len()) as u64);

        // 清理：目录全删、字节对账、统计归零
        let outcome = clean_orphans(&root);
        assert_eq!(outcome.removed_dirs, 2);
        assert_eq!(outcome.freed_bytes, stats.total_bytes);
        assert!(outcome.errors.is_empty());
        let after = orphan_stats(&root);
        assert_eq!(after.dir_count, 0);
        assert_eq!(after.file_count, 0);
        assert!(root.join("1/a.jpg").exists(), "清理只动隔离区，引用文件安然");
    }

    #[test]
    fn orphan_stats_and_clean_on_missing_dir_are_empty_no_error() {
        let tmp = TempDir::new().unwrap();
        let stats = orphan_stats(&photo_dir(&tmp));
        assert_eq!(stats, OrphanPhotoStats { dir_count: 0, file_count: 0, total_bytes: 0 });
        let outcome = clean_orphans(&photo_dir(&tmp));
        assert_eq!(outcome.removed_dirs, 0);
        assert!(outcome.errors.is_empty());
    }

    #[test]
    fn scan_orphans_on_missing_dir_is_noop() {
        let conn = mem_conn();
        let tmp = TempDir::new().unwrap();
        let outcome = scan_orphans(&conn, &photo_dir(&tmp), "20260919-080000");
        assert!(outcome.moved.is_empty());
        assert!(outcome.errors.is_empty());
    }
}
