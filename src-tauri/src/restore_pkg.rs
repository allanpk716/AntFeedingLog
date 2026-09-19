//! 数据包恢复（webui-checkin 票 10，规格 Implementation Decisions F 恢复协议）。
//!
//! 与 [`crate::restore`]（四步协议本体）分层：.db 裸库走既有链路不动；.zip 数据包
//! 走本模块——**staging 校验段**与**换库后落位段**都挂进同一套快照/替换原语上。
//!
//! **恢复协议（数据包）**：
//! 1. 读 manifest → 格式版本检查（未来版本拒，同库 schema 纪律，人话错误）；
//! 2. 逐文件 SHA256 校验（库 + 每张照片，坏即拒）；
//! 3. 磁盘空间预检按 zip 实际条目元数据（总解压字节 ×1.2，目标盘 = 数据目录所在盘）；
//! 4. 解包到数据目录暂存目录 [`pkg_staging_dir_name`]（**严格路径 grammar 表驱动拒绝**：
//!    zip 内任何清单外条目 / 反斜杠 / 冒号 / 绝对路径 / `..` / 符号链接属性条目 /
//!    重复条目 → 整包拒，人话列第一条违规；manifest 路径与打包侧共用
//!    [`crate::backup_pkg::is_packable_rel_path`] 同一份闸）；
//! 5. 库条目走既有 staging 校验链（integrity + 迁移，[`crate::restore::validate_staging`]）；
//! 6. **包照片集合覆盖库内引用校验**：解包出的库查 `nest_photo` 全部 rel_path，
//!    manifest 缺任何一个 → 拒绝恢复（人话「包内缺 N 张照片」）；
//! 7. pre-restore 快照（既有，数据包形态）→ 锁内换库（既有
//!    [`crate::restore::replace_and_reload`]）→ 暂存照片逐文件 rename 落位（建父目录；
//!    库已引用的照片必然在暂存区——覆盖校验保证）→ 清退 photos/ 中非清单文件
//!    （`.orphan-*` 目录整个保留不动；清退失败仅记流水不回滚——多余文件 = 下轮巡检
//!    孤儿）→ 广播由命令层无条件刷新（既有 db-restored + 版本 bump_floor）。
//!
//! **失败不变量（命根）：任何一步失败，当前库与 photos/ 均未被破坏。**
//! - 换库之前的一切失败：暂存目录一律清理，当前库/照片零改动（与裸库路径同款）；
//! - 换库之后落位失败：新库已引用的照片必然已在暂存区就绪——暂存目录**保留**，
//!   返回错误并提示「重启应用后将自动继续」；
//! - **中断可续**：换库前会先在暂存目录写 [`SWAP_MARKER_FILE`] 标记（写入点在换库
//!   之前，任何时点的进程被杀都不会漏判）；启动时 [`resume_pending_pkg_restores`]
//!   （复用孤儿巡检挂点，先续跑后巡检）把「当前库引用而 photos/ 缺失」且暂存区
//!   有的照片继续落位，然后清掉暂存目录；无标记的暂存目录（换库前就崩了/解包
//!   半截）直接丢弃。暂存成功后清理，失败（换库后）保留供续跑/排障。
//!
//! 恢复摘要：[`crate::restore::RestoreSummary`] 增 `photo_count`（= manifest 张数）；
//! 裸库路径 photo_count = 0（前端标注「不含照片」）。

use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use sha2::{Digest, Sha256};

use crate::backup_pkg::{self, PackageManifest, DB_ENTRY, FORMAT_VERSION, MANIFEST_ENTRY};
use crate::restore::{self, ApplyOutcome, RestoreSummary};

/// 数据目录里数据包恢复暂存目录前缀（`restore-staging-pkg-<时间戳>/`）。
pub const PKG_STAGING_PREFIX: &str = "restore-staging-pkg-";

/// 暂存目录内的换库标记文件名。写入点在换库之前：启动续跑看到它才执行落位，
/// 看不到（解包半截 / 崩在换库前）就整目录丢弃——当前库那时必然还是旧库。
pub const SWAP_MARKER_FILE: &str = "swap-pending";

/// 暂存目录内的 manifest 副本文件名（续跑不依赖它，仅为排障留底）。
pub const MANIFEST_COPY_FILE: &str = "manifest.json";

/// 空间预检余量分子（×1.2 = 12/10，规格 F：峰值留 1.2×）。
pub const SPACE_MARGIN_NUM: u128 = 12;
/// 空间预检余量分母。
pub const SPACE_MARGIN_DEN: u128 = 10;

// ── 暂存目录命名 ─────────────────────────────────────────────────────────

/// 数据包暂存目录名：`restore-staging-pkg-<时间戳>`（目录，非裸库路径的单文件）。
pub fn pkg_staging_dir_name(stamp: &str) -> String {
    format!("{PKG_STAGING_PREFIX}{stamp}")
}

/// 暂存区内照片子树根（镜像 photos/ 相对布局，落位即整树 rename 搬家）。
fn staging_photos_root(staging_dir: &Path) -> PathBuf {
    staging_dir.join(crate::photo::PHOTOS_DIR_NAME)
}

/// rel_path（`<段>/<文件>` 正斜杠）拼到根下（仅用于已过 grammar 的路径）。
fn join_rel(root: &Path, rel: &str) -> PathBuf {
    let mut p = root.to_path_buf();
    for comp in rel.split('/') {
        p.push(comp);
    }
    p
}

/// 清理暂存目录（NotFound 视为已清理；清理失败不掩盖原错误）。
pub fn cleanup_staging_dir(staging_dir: &Path) {
    if let Err(e) = std::fs::remove_dir_all(staging_dir) {
        if e.kind() != std::io::ErrorKind::NotFound {
            eprintln!("[restore_pkg] 清理暂存目录失败（忽略）: {e}");
        }
    }
}

// ── manifest 读取与校验 ──────────────────────────────────────────────────

/// manifest 校验（表驱动逐闸）：格式版本（未来拒/无效拒）、库文件名、照片路径
/// grammar（与打包侧同源）、清单内路径不重复、张数与总字节自洽。人话错误。
pub fn validate_manifest(m: &PackageManifest) -> Result<(), String> {
    if m.format_version > FORMAT_VERSION {
        return Err(format!(
            "这是未来版本的数据包（格式 v{}，当前应用支持 v{FORMAT_VERSION}），请先升级应用",
            m.format_version
        ));
    }
    if m.format_version < FORMAT_VERSION {
        return Err(format!(
            "数据包格式版本无效（v{}，当前应用支持 v{FORMAT_VERSION}）",
            m.format_version
        ));
    }
    if m.db_file != DB_ENTRY {
        return Err(format!(
            "数据包清单损坏（库文件名应为 {DB_ENTRY}，实际 {}）",
            m.db_file
        ));
    }
    let mut seen = HashSet::new();
    for p in &m.photos {
        if !backup_pkg::is_packable_rel_path(&p.path) {
            return Err(format!(
                "数据包清单含非法照片路径（{}），已拒绝恢复",
                p.path
            ));
        }
        if !seen.insert(p.path.as_str()) {
            return Err(format!("数据包清单损坏（照片路径重复：{}）", p.path));
        }
    }
    if m.photo_count != m.photos.len() {
        return Err(format!(
            "数据包清单损坏（照片张数应为 {}，清单实列 {}）",
            m.photos.len(),
            m.photo_count
        ));
    }
    let total: u64 = m.photos.iter().map(|p| p.bytes).sum();
    if m.total_bytes != total {
        return Err(format!(
            "数据包清单损坏（照片总字节应为 {total}，清单写 {}）",
            m.total_bytes
        ));
    }
    Ok(())
}

/// 从 zip 里读 manifest 内容（根级 [`MANIFEST_ENTRY`] 解析）。重名条目不在
/// by_index 视野里（zip crate 读侧开包即按名折叠，后者胜）——查重走
/// [`raw_cd_names`] 的原始中央目录扫描。
fn read_manifest<R: std::io::Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
) -> Result<PackageManifest, String> {
    let mut manifest_bytes: Option<Vec<u8>> = None;
    for i in 0..archive.len() {
        let is_manifest = {
            let entry = archive
                .by_index(i)
                .map_err(|e| format!("这不是本应用的数据包（无法读取 zip 条目: {e}）"))?;
            entry.name() == MANIFEST_ENTRY
        };
        if is_manifest {
            let mut buf = Vec::new();
            let mut entry = archive
                .by_index(i)
                .map_err(|e| format!("读取数据包清单失败: {e}"))?;
            entry
                .read_to_end(&mut buf)
                .map_err(|e| format!("读取数据包清单失败: {e}"))?;
            manifest_bytes = Some(buf);
        }
    }
    let bytes = manifest_bytes
        .ok_or_else(|| "这不是本应用的数据包（缺少 manifest.json 清单）".to_string())?;
    let manifest: PackageManifest = serde_json::from_slice(&bytes)
        .map_err(|e| format!("数据包清单损坏（manifest.json 无法解析）: {e}"))?;
    Ok(manifest)
}

/// 原始中央目录扫描（票 10「重复条目 → 整包拒」）：zip crate 4.6 读侧开包即把
/// 同名条目折叠进按名索引（后者胜，len 只数不重名），by_index 看不到重复——
/// 这里自走中央目录记录逐条收名，查重即拒（这也正是重名条目的歧义攻击面：
/// 不同读实现取首/取末不一致，我们直接整包拒）。
fn raw_cd_names(file: &std::fs::File, cd_start: u64) -> Result<Vec<String>, String> {
    use std::io::{Seek, SeekFrom};
    let mut f = file
        .try_clone()
        .map_err(|e| format!("读取数据包失败: {e}"))?;
    f.seek(SeekFrom::Start(cd_start))
        .map_err(|e| format!("读取数据包失败: {e}"))?;
    let mut names: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    loop {
        let mut header = [0u8; 46];
        match read_exactly(&mut f, &mut header) {
            Ok(false) => break, // 干净走到结尾（EOCD 已越过）
            Ok(true) => {}
            Err(e) => return Err(format!("读取数据包失败: {e}")),
        }
        let sig = u32::from_le_bytes(header[0..4].try_into().unwrap());
        if sig != 0x0201_4b50 {
            break; // 中央目录走完（EOCD / ZIP64 定位记录）
        }
        let name_len = u16::from_le_bytes(header[28..30].try_into().unwrap()) as usize;
        let extra_len = u16::from_le_bytes(header[30..32].try_into().unwrap()) as usize;
        let comment_len = u16::from_le_bytes(header[32..34].try_into().unwrap()) as usize;
        let mut name_buf = vec![0u8; name_len];
        f.read_exact(&mut name_buf)
            .map_err(|e| format!("读取数据包失败: {e}"))?;
        let name = String::from_utf8_lossy(&name_buf).to_string();
        if !seen.insert(name.clone()) {
            return Err(format!("数据包含重复条目（{name}），已拒绝恢复"));
        }
        names.push(name);
        f.seek(SeekFrom::Current((extra_len + comment_len) as i64))
            .map_err(|e| format!("读取数据包失败: {e}"))?;
    }
    Ok(names)
}

/// 读满 buf；流干净结束（含读不满）返回 false，IO 错误上抛。
fn read_exactly(f: &mut std::fs::File, buf: &mut [u8]) -> std::io::Result<bool> {
    let mut filled = 0;
    while filled < buf.len() {
        match f.read(&mut buf[filled..])? {
            0 => return Ok(false),
            n => filled += n,
        }
    }
    Ok(true)
}

// ── 磁盘空间预检 ─────────────────────────────────────────────────────────

/// 空间预检（纯函数）：总解压字节 ×1.2 ≤ 可用字节，超出报人话（MB 口径）。
pub fn check_disk_space(total_uncompressed: u64, free_bytes: u64) -> Result<(), String> {
    let need = (total_uncompressed as u128) * SPACE_MARGIN_NUM / SPACE_MARGIN_DEN;
    if (free_bytes as u128) < need {
        return Err(format!(
            "磁盘空间不足：解包约需 {}MB，目标盘可用 {}MB，请清理磁盘后重试",
            need / 1024 / 1024,
            (free_bytes as u128) / 1024 / 1024
        ));
    }
    Ok(())
}

/// 生产侧可用空间：数据目录所在盘（Windows GetDiskFreeSpaceExW；非 Windows
/// 无现成 API，返回 u64::MAX 降级放行——本应用实际只跑 Windows）。
#[cfg(windows)]
pub fn free_bytes_on(dir: &Path) -> Result<u64, String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let wide: Vec<u16> = dir.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut free: u64 = 0;
    let ok = unsafe {
        GetDiskFreeSpaceExW(wide.as_ptr(), &mut free, std::ptr::null_mut(), std::ptr::null_mut())
    };
    if ok == 0 {
        return Err(format!("读取磁盘可用空间失败（{}）", dir.display()));
    }
    Ok(free)
}

/// 非 Windows：预检降级放行（见 [`free_bytes_on`] Windows 注）。
#[cfg(not(windows))]
pub fn free_bytes_on(_dir: &Path) -> Result<u64, String> {
    Ok(u64::MAX)
}

/// 生产侧空间探针（把数据目录绑定进探针闭包；测试注入假配额）。
pub fn real_free_space(dir: &Path) -> impl Fn() -> Result<u64, String> + '_ {
    move || free_bytes_on(dir)
}

// ── staging：解包 + 校验 + 覆盖检查 ──────────────────────────────────────

/// staging 校验完成的一包数据（preview 只用摘要；apply 继续换库/落位）。
pub struct PkgStaged {
    /// 已过 integrity + 迁移校验的暂存库连接（用完由调用方关闭）。
    pub conn: Connection,
    /// 暂存目录（成功恢复后清理；换库后失败保留供续跑）。
    pub staging_dir: PathBuf,
    pub summary: RestoreSummary,
    /// 新库引用的全部照片相对路径（= 落位清单；覆盖校验保证 ⊆ manifest）。
    pub referenced: Vec<String>,
}

/// staging 校验段（preview 与 apply 共用；全程不碰当前库与 photos/）：
/// 空/坏文件闸 → 读 manifest + 条目查重 → manifest 校验 → 空间预检 → 严格
/// grammar 白名单解包（哈希边写边校）→ 暂存库走既有校验链 → 覆盖检查。
/// 任何失败清掉暂存目录，报人话原因。
pub fn stage_and_validate_pkg(
    source: &Path,
    data_dir: &Path,
    stamp: &str,
    free_space: &impl Fn() -> Result<u64, String>,
) -> Result<PkgStaged, String> {
    // 0 字节文件连 zip 头都没有，显式拒绝（与裸库路径同款闸）
    match std::fs::metadata(source) {
        Ok(m) if m.len() == 0 => {
            return Err("这不是本应用的数据包（文件为空）".to_string());
        }
        Ok(_) => {}
        Err(e) => return Err(format!("无法读取数据包文件: {e}")),
    }
    let file = std::fs::File::open(source).map_err(|e| format!("无法读取数据包文件: {e}"))?;
    // 中央目录原始扫描用独立句柄（try_clone 与归档共享游标，但两边的每次读取
    // 都先 seek 定位，交错使用无碍）
    let cd_handle = file.try_clone().map_err(|e| format!("读取数据包失败: {e}"))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("这不是本应用的数据包（无法读取 zip: {e}）"))?;

    // ① 原始中央目录扫描（重名即拒）+ manifest 内容
    let names = raw_cd_names(&cd_handle, archive.central_directory_start())?;
    let manifest = read_manifest(&mut archive)?;
    validate_manifest(&manifest)?;

    // ② 严格 grammar 白名单（表驱动逐闸，人话列第一条违规）
    let photo_map: HashSet<String> = manifest.photos.iter().map(|p| p.path.clone()).collect();
    audit_entry_names(&names, &photo_map)?;

    // ③ 空间预检：zip 实际条目元数据（未压缩大小）×1.2，目标盘 = 数据目录所在盘
    let mut total_uncompressed: u64 = 0;
    for i in 0..archive.len() {
        let entry = archive
            .by_index(i)
            .map_err(|e| format!("读取数据包条目元数据失败: {e}"))?;
        total_uncompressed = total_uncompressed.saturating_add(entry.size());
    }
    let free = free_space().map_err(|e| format!("磁盘空间预检失败: {e}"))?;
    check_disk_space(total_uncompressed, free)?;

    // ④ 解包（哈希边写边校）。同 stamp 残留目录先清（崩溃遗留，绝不混用）。
    let staging_dir = data_dir.join(pkg_staging_dir_name(stamp));
    if staging_dir.exists() {
        cleanup_staging_dir(&staging_dir);
    }
    std::fs::create_dir_all(&staging_dir).map_err(|e| format!("创建暂存目录失败: {e}"))?;
    let mut inner = || -> Result<PkgStaged, String> {
        extract_verified(&mut archive, &staging_dir, &manifest, &photo_map)?;
        // manifest 副本留底（续跑不依赖；排障用）。失败仅提示，不整包拒。
        let manifest_copy = serde_json::to_vec_pretty(&manifest)
            .map_err(|e| format!("序列化清单副本失败: {e}"));
        match manifest_copy.and_then(|bytes| {
            std::fs::write(staging_dir.join(MANIFEST_COPY_FILE), bytes)
                .map_err(|e| format!("写入清单副本失败: {e}"))
        }) {
            Ok(()) => {}
            Err(e) => eprintln!("[restore_pkg] {e}（忽略）"),
        }
        // ⑤ 暂存库走既有校验链（integrity + 迁移 + 摘要）
        let staged_db = staging_dir.join(DB_ENTRY);
        let conn = Connection::open(&staged_db).map_err(|e| format!("无法打开数据包内库文件: {e}"))?;
        // 备份日期：manifest.created_at（`YYYY-MM-DD HH:MM:SS`）的日期部分
        let file_date = manifest
            .created_at
            .get(..10)
            .and_then(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok());
        let mut summary = restore::validate_staging(&conn, file_date)?;
        summary.photo_count = manifest.photo_count;
        // ⑥ 覆盖校验：库引用 ⊆ manifest 照片集合（缺失即拒）
        let referenced = referenced_photo_paths(&conn)?;
        let missing: Vec<String> = referenced
            .iter()
            .filter(|rel| !photo_map.contains(rel.as_str()))
            .cloned()
            .collect();
        if !missing.is_empty() {
            return Err(format!(
                "包内缺 {} 张照片（库引用了 {} 等），数据包不完整，拒绝恢复",
                missing.len(),
                missing[0]
            ));
        }
        Ok(PkgStaged {
            conn,
            staging_dir: staging_dir.clone(),
            summary,
            referenced,
        })
    };
    match inner() {
        Ok(v) => Ok(v),
        Err(e) => {
            // inner 返回 Err 时连接已随作用域 drop，句柄已释放可整目录清
            cleanup_staging_dir(&staging_dir);
            Err(e)
        }
    }
}

/// 条目名审计（表驱动，逐闸人话）：目录条目 / 符号链接属性 / 反斜杠 / 冒号 /
/// 绝对路径 / `..` 越界 / 清单外——命中第一条即整包拒。
fn audit_entry_names(names: &[String], photo_map: &HashSet<String>) -> Result<(), String> {
    for name in names {
        if name.ends_with('/') {
            return Err(format!("数据包含清单外条目（目录条目 {name}），已拒绝恢复"));
        }
        if name.contains('\\') {
            return Err(format!(
                "数据包含非法路径条目（{name} 含反斜杠），已拒绝恢复"
            ));
        }
        if name.contains(':') {
            return Err(format!(
                "数据包含非法路径条目（{name} 含冒号），已拒绝恢复"
            ));
        }
        if name.starts_with('/') {
            return Err(format!(
                "数据包含非法路径条目（{name} 是绝对路径），已拒绝恢复"
            ));
        }
        if name.split('/').any(|seg| seg == "..") {
            return Err(format!(
                "数据包含越界路径条目（{name}），已拒绝恢复"
            ));
        }
        if name != MANIFEST_ENTRY && name != DB_ENTRY && !photo_map.contains(name) {
            return Err(format!("数据包含清单外条目（{name}），已拒绝恢复"));
        }
    }
    Ok(())
}

/// 边写文件边算 SHA256 的 writer（不整载进内存，防敌意 zip 声明巨尺寸的解压炸弹）。
struct HashingWriter {
    file: std::fs::File,
    hasher: Sha256,
}

impl Write for HashingWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.file.write(buf)?;
        self.hasher.update(&buf[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}

fn hex(digest: &[u8]) -> String {
    let mut s = String::with_capacity(digest.len() * 2);
    for b in digest {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// 逐条目解包（哈希边写边校，任何不符整包拒）：
/// - manifest.json 已在 staging 段读过（另行留副本），不落文件；
/// - 库条目 ↔ manifest.db_sha256；照片条目 ↔ 各自 sha256；
/// - 完了核对 db 与全部照片条目都真实存在（少一条即拒）。
fn extract_verified<R: std::io::Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    staging_dir: &Path,
    manifest: &PackageManifest,
    photo_map: &HashSet<String>,
) -> Result<(), String> {
    let mut hash_of: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("读取数据包条目失败: {e}"))?;
        // 符号链接属性条目：哪怕名字在清单内也整包拒（防逃逸）
        if entry.is_symlink() {
            return Err(format!(
                "数据包含符号链接条目（{}），已拒绝恢复",
                entry.name()
            ));
        }
        let name = entry.name().to_string();
        if name == MANIFEST_ENTRY {
            continue;
        }
        let expected = if name == DB_ENTRY {
            manifest.db_sha256.clone()
        } else {
            debug_assert!(photo_map.contains(&name), "audit 已放行的名字必在白名单");
            manifest
                .photos
                .iter()
                .find(|p| p.path == name)
                .map(|p| p.sha256.clone())
                .ok_or_else(|| format!("数据包含清单外条目（{name}），已拒绝恢复"))?
        };
        // 库条目落暂存根（换库从这里取）；照片条目镜像 photos/ 相对布局
        // （落位/续跑都从 photos/<rel> 同路径 rename，目录结构一目了然）
        let dest = if name == DB_ENTRY {
            staging_dir.join(&name)
        } else {
            join_rel(&staging_photos_root(staging_dir), &name)
        };
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("创建暂存子目录失败: {e}"))?;
        }
        let out = std::fs::File::create(&dest).map_err(|e| format!("写入暂存文件失败: {e}"))?;
        let mut tee = HashingWriter {
            file: out,
            hasher: Sha256::new(),
        };
        std::io::copy(&mut entry, &mut tee).map_err(|e| format!("解包条目 {name} 失败: {e}"))?;
        let actual = hex(&tee.hasher.finalize());
        if actual != expected.to_lowercase() {
            return Err(format!(
                "数据包文件校验失败（{name} 与清单哈希不符），已拒绝恢复"
            ));
        }
        hash_of.insert(name, actual);
    }
    // 条目齐套：库 + 清单里每一张照片都必须真实解到
    if !hash_of.contains_key(DB_ENTRY) {
        return Err(format!("数据包缺少条目（{DB_ENTRY}），已拒绝恢复"));
    }
    for p in &manifest.photos {
        if !hash_of.contains_key(&p.path) {
            return Err(format!("数据包缺少条目（{}），已拒绝恢复", p.path));
        }
    }
    Ok(())
}

/// 暂存库 `nest_photo` 引用的全部相对路径（去重、字典序；无 nest_photo 表的老库
/// → 空）。与打包侧同语义，但吃 `&Connection`（暂存库连接已开）。
fn referenced_photo_paths(conn: &Connection) -> Result<Vec<String>, String> {
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
                Err(format!("读取库内照片引用失败: {e}"))
            }
        }
    }
}

// ── 落位与清退 ───────────────────────────────────────────────────────────

/// 暂存照片逐文件 rename 落位（建父目录；同目标已存在则覆盖——恢复 = photos/
/// 被包内容整体替换，ADR-0004）。源缺失而目标已在（上一轮落过）→ 跳过（可续）。
/// 任一张失败即停：已落的保持原位，未落的留在暂存区——调用方保留暂存目录，
/// 重启后 [`resume_pending_pkg_restores`] 继续。
pub fn land_photos(
    staging_dir: &Path,
    photos_root: &Path,
    rel_paths: &[String],
) -> Result<(), String> {
    let src_root = staging_photos_root(staging_dir);
    for rel in rel_paths {
        if !backup_pkg::is_packable_rel_path(rel) {
            return Err(format!("{rel}: 相对路径形态非法，拒绝落位"));
        }
        let src = join_rel(&src_root, rel);
        let dst = join_rel(photos_root, rel);
        if !src.exists() {
            if dst.exists() {
                continue; // 上一轮已落位（可续语义）
            }
            return Err(format!("{rel}: 暂存区没有这张照片（数据包不完整？）"));
        }
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("创建照片目录失败: {e}"))?;
        }
        std::fs::rename(&src, &dst).map_err(|e| format!("{rel}: {e}"))?;
    }
    Ok(())
}

/// 清退 photos/ 中非清单文件（清单 = 换入库引用集合）。`.orphan-*` 目录整个
/// 保留不动。逐文件尽力删，失败仅返回清单（调用方落流水，残留 = 下轮巡检
/// 孤儿），绝不回滚。返回失败明细。
pub fn evict_non_manifest_files(photos_root: &Path, keep: &HashSet<String>) -> Vec<String> {
    let mut failures = Vec::new();
    let entries = match std::fs::read_dir(photos_root) {
        Ok(entries) => entries,
        Err(e) => {
            if e.kind() != std::io::ErrorKind::NotFound {
                failures.push(format!("读取照片目录失败: {e}"));
            }
            return failures;
        }
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with(crate::photo::ORPHAN_DIR_PREFIX) {
            continue; // 隔离区不动
        }
        let path = entry.path();
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            evict_dir(&path, &name, keep, &mut failures);
            // 尽力删掉清空后的窝目录（非空则失败忽略——目录无害）
            let _ = std::fs::remove_dir(&path);
        } else if !keep.contains(&name) {
            if let Err(e) = std::fs::remove_file(&path) {
                failures.push(format!("{name}: {e}"));
            }
        }
    }
    failures
}

fn evict_dir(dir: &Path, prefix: &str, keep: &HashSet<String>, failures: &mut Vec<String>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => {
            failures.push(format!("{prefix}: {e}"));
            return;
        }
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let path = entry.path();
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            evict_dir(&path, &format!("{prefix}/{name}"), keep, failures);
            let _ = std::fs::remove_dir(&path);
        } else {
            let rel = format!("{prefix}/{name}");
            if !keep.contains(&rel) {
                if let Err(e) = std::fs::remove_file(&path) {
                    failures.push(format!("{rel}: {e}"));
                }
            }
        }
    }
}

// ── 中断可续：启动续跑 ───────────────────────────────────────────────────

/// 续跑结果（动作全在字段里，调用方落流水）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PkgResumeOutcome {
    /// 续跑完成的暂存目录名（换库标记在 → 落位 → 清目录）。
    pub resumed: Vec<String>,
    /// 丢弃的半截暂存目录名（无换库标记：解包半截或崩在换库前）。
    pub discarded: Vec<String>,
    /// 本轮实际落位的照片相对路径。
    pub landed: Vec<String>,
    /// 非致命失败（暂存目录原样保留，下次启动重试）。
    pub errors: Vec<String>,
}

/// 启动续跑（复用孤儿巡检挂点，先续跑后巡检）：扫数据目录的
/// `restore-staging-pkg-*`——
/// - 无换库标记（[`SWAP_MARKER_FILE`]）→ 解包半截或崩在换库前，当前库必是旧库，
///   整目录丢弃；
/// - 有标记 → 当前库可能是新库：把「库引用而 photos/ 缺失」且暂存区有的照片
///   继续落位，然后清掉暂存目录（落位都完成后，暂存区没有值得留的东西；库引用
///   之外的多余旧文件由随后的孤儿巡检收）。
///
/// 全程非致命：单个失败记 errors、原样保留，下次启动重试。
pub fn resume_pending_pkg_restores(
    data_dir: &Path,
    photos_root: &Path,
    conn: &Connection,
) -> PkgResumeOutcome {
    let mut outcome = PkgResumeOutcome::default();
    let entries = match std::fs::read_dir(data_dir) {
        Ok(entries) => entries,
        Err(e) => {
            outcome.errors.push(format!("读取数据目录失败: {e}"));
            return outcome;
        }
    };
    let referenced = match referenced_photo_paths(conn) {
        Ok(v) => v,
        Err(e) => {
            outcome.errors.push(e);
            return outcome;
        }
    };
    for entry in entries.flatten() {
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with(PKG_STAGING_PREFIX) {
            continue;
        }
        let staging_dir = entry.path();
        if !staging_dir.join(SWAP_MARKER_FILE).exists() {
            match std::fs::remove_dir_all(&staging_dir) {
                Ok(()) => outcome.discarded.push(name),
                Err(e) => outcome.errors.push(format!("{name}: 丢弃半截暂存失败: {e}")),
            }
            continue;
        }
        match resume_landing(&staging_dir, photos_root, &referenced, &mut outcome.landed) {
            Ok(()) => match std::fs::remove_dir_all(&staging_dir) {
                Ok(()) => outcome.resumed.push(name),
                Err(e) => outcome.errors.push(format!("{name}: 照片已落位但清目录失败: {e}")),
            },
            Err(e) => outcome.errors.push(format!("{name}: {e}")),
        }
    }
    outcome
}

/// 单个暂存目录的续跑落位：只搬「库引用 ∩ photos/ 缺失 ∩ 暂存区存在」的照片。
fn resume_landing(
    staging_dir: &Path,
    photos_root: &Path,
    referenced: &[String],
    landed: &mut Vec<String>,
) -> Result<(), String> {
    let src_root = staging_photos_root(staging_dir);
    for rel in referenced {
        if !backup_pkg::is_packable_rel_path(rel) {
            continue; // 防御：库被外部改出坏路径，续跑不碰
        }
        let dst = join_rel(photos_root, rel);
        if dst.exists() {
            continue; // 已在位
        }
        let src = join_rel(&src_root, rel);
        if !src.exists() {
            continue; // 暂存区没有：标记前的半截/异包残留，交给缺图占位
        }
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{rel}: 创建照片目录失败: {e}"))?;
        }
        std::fs::rename(&src, &dst).map_err(|e| format!("{rel}: {e}"))?;
        landed.push(rel.clone());
    }
    Ok(())
}

// ── 编排：preview 与 apply ───────────────────────────────────────────────

/// 数据包恢复预览：staging 校验段产出摘要后即清暂存目录。
pub fn run_preview_pkg(
    source: &Path,
    data_dir: &Path,
    stamp: &str,
    free_space: &impl Fn() -> Result<u64, String>,
) -> Result<RestoreSummary, String> {
    let staged = stage_and_validate_pkg(source, data_dir, stamp, free_space)?;
    let summary = staged.summary;
    drop(staged.conn);
    cleanup_staging_dir(&staged.staging_dir);
    Ok(summary)
}

/// 数据包恢复执行：staging 校验段 → 写换库标记 → 快照（既有两段式）→ 锁内换库
/// （staged db 在暂存目录里）→ 落位 → 清退 → 清暂存目录。
///
/// 失败语义（不变量：换库前失败当前库与 photos/ 零改动）：
/// - 标记/快照/换库失败（[`crate::restore::replace_and_reload`] 自身保证旧库回位）
///   → 清暂存目录、原样报错；
/// - 落位失败 → **暂存目录保留**，报错带「重启应用后将自动继续」指引（可续）；
/// - 清退失败仅落流水，恢复照常完成（残留 = 孤儿，下轮巡检收）。
pub fn run_apply_pkg<G, L, O>(
    source: &Path,
    current_db: &Path,
    data_dir: &Path,
    stamp: &str,
    free_space: &impl Fn() -> Result<u64, String>,
    take_lock: L,
    open_conn: O,
) -> Result<ApplyOutcome, String>
where
    L: Fn() -> Result<G, String>,
    G: std::ops::DerefMut<Target = Connection>,
    O: Fn(&Path) -> Result<Connection, String>,
{
    let photos_root = data_dir.join(crate::photo::PHOTOS_DIR_NAME);
    let staged = stage_and_validate_pkg(source, data_dir, stamp, free_space)?;
    drop(staged.conn); // 校验完先关暂存库连接（句柄释放）；目录留给换库用

    let staging_dir = staged.staging_dir;
    // 换库前阶段：任何失败清暂存目录（当前库/照片零改动）
    let pre_swap = (|| -> Result<ApplyOutcome, String> {
        // 换库标记先落（写入点在换库之前：任何时点被杀，启动续跑都不会漏判；
        // 标记在而换库没发生时，续跑只搬「当前库引用且缺失」的照片 = 无害子集）
        std::fs::write(staging_dir.join(SWAP_MARKER_FILE), b"").map_err(|e| {
            format!("写换库标记失败: {e}")
        })?;
        // 快照阶段一：锁内拷库（毫秒级；锁内禁写复查随闭包跑）
        let snap_temp = {
            let _guard = take_lock()?;
            restore::snapshot_stage_db(current_db, data_dir, stamp)?
        };
        // 快照阶段二（锁外）：既有数据包形态 + 保留淘汰
        restore::snapshot_publish(&snap_temp, data_dir, stamp)?;
        // 锁内换库：staged db 就在暂存目录里
        let staged_db = staging_dir.join(DB_ENTRY);
        let guard = take_lock()?;
        restore::replace_and_reload(guard, current_db, &staged_db, &open_conn)
    })();
    let outcome = match pre_swap {
        Err(e) => {
            cleanup_staging_dir(&staging_dir);
            return Err(e);
        }
        Ok(outcome) => outcome,
    };

    // 换库已发生：落位（失败保留暂存目录供续跑）
    if let Err(e) = land_photos(&staging_dir, &photos_root, &staged.referenced) {
        return Err(format!(
            "照片落位失败（{e}）；当前库已替换为备份内容，重启应用后将自动继续完成落位"
        ));
    }
    // 清退放最后：失败仅记流水（残留 = 下轮巡检孤儿），不回滚
    let keep: HashSet<String> = staged.referenced.iter().cloned().collect();
    for f in evict_non_manifest_files(&photos_root, &keep) {
        crate::applog::log_error(&format!(
            "恢复清退多余照片失败（下轮孤儿巡检会处理）: {f}"
        ));
    }
    cleanup_staging_dir(&staging_dir);
    Ok(outcome)
}

// ── 测试：只测外部行为（spec「Testing Decisions」）────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::sync::Mutex;

    use rusqlite::params;
    use tempfile::TempDir;
    use zip::write::SimpleFileOptions;

    use crate::restore::STAGING_PREFIX;

    // ── 通用脚手架 ───────────────────────────────────────────────────────

    fn photo_bytes(tag: u8) -> Vec<u8> {
        vec![0xFF, 0xD8, 0xFF, tag, b'A', b'B', tag, 0x00]
    }

    fn hex_of(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        let mut s = String::new();
        for b in Sha256::digest(bytes) {
            s.push_str(&format!("{b:02x}"));
        }
        s
    }

    /// 建库（1 窝 + 1 条带照片引用的巢况）并在 photos_root 写真实照片文件。
    fn make_db_with_photo(db_path: &Path, photos_root: &Path, rel: &str, bytes: &[u8]) {
        let conn = crate::db::open_and_migrate(db_path).expect("建库失败");
        conn.execute(
            "INSERT INTO colony (name, start_date) VALUES ('大头一号', '2026-01-20')",
            [],
        )
        .unwrap();
        let checkin_id = crate::nest_checkin::save_checkin(
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
        let p = join_rel(photos_root, rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, bytes).unwrap();
        conn.execute(
            "INSERT INTO nest_photo (checkin_id, rel_path, original_name, note)
             VALUES (?1, ?2, '', '')",
            params![checkin_id, rel],
        )
        .unwrap();
        drop(conn);
    }

    /// 真实数据包：backup_pkg::build_package 产出（manifest 自洽）。
    fn build_real_package(
        db_snapshot: &Path,
        photos_root: &Path,
        target: &Path,
    ) -> backup_pkg::PackageOutcome {
        let mut no_skip = |_msg: &str| {};
        backup_pkg::build_package(db_snapshot, photos_root, "2026-09-19 08:30:00", target, &mut no_skip)
            .expect("打包失败")
    }

    /// 数据目录夹具：tempdir + data_dir + 当前库（现窝 1 条，无照片）。返回
    /// (tempdir, data_dir, current_db_path)。
    fn fixture() -> (TempDir, PathBuf, PathBuf) {
        let dir = TempDir::new().unwrap();
        let data_dir = dir.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        let db_path = data_dir.join(crate::db::DB_FILE_NAME);
        let conn = crate::db::open_and_migrate(&db_path).unwrap();
        conn.execute(
            "INSERT INTO colony (name, start_date) VALUES ('现窝', '2026-01-20')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO care_log (colony_id, action_id, occurred_at, note, created_at)
             VALUES (1, 1, '2026-09-01 08:00:00', '', '2026-09-01 08:00:00')",
            [],
        )
        .unwrap();
        drop(conn);
        (dir, data_dir, db_path)
    }

    /// 备份库夹具（备份窝 2 条 + 可选照片引用），独立于当前库。
    fn make_backup_db(path: &Path) {
        let conn = crate::db::open_and_migrate(path).expect("建库失败");
        conn.execute(
            "INSERT INTO colony (name, start_date) VALUES ('备份窝', '2026-02-01')",
            [],
        )
        .unwrap();
        for i in 0..2 {
            conn.execute(
                "INSERT INTO care_log (colony_id, action_id, occurred_at, note, created_at)
                 VALUES (1, 1, ?1, '', ?1)",
                params![format!("2026-09-{:02} 08:00:00", i + 1)],
            )
            .unwrap();
        }
        drop(conn);
    }

    fn db_bytes(path: &Path) -> Vec<u8> {
        std::fs::read(path).unwrap()
    }

    fn list_names(dir: &Path) -> Vec<String> {
        let mut names: Vec<std::fs::DirEntry> = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .collect();
        names.sort_by_key(|e| e.file_name());
        names
            .into_iter()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect()
    }

    /// 暂存（裸库 staging 文件 + 数据包 staging 目录 + 快照 staging）全无残留。
    fn no_staging_left(data_dir: &Path) -> bool {
        list_names(data_dir).iter().all(|n| {
            !n.starts_with(STAGING_PREFIX)
                && !n.starts_with(restore::SNAPSHOT_STAGING_PREFIX)
                && !n.starts_with(PKG_STAGING_PREFIX)
        })
    }

    fn space_ok() -> impl Fn() -> Result<u64, String> {
        || Ok(u64::MAX)
    }

    fn space_of(free: u64) -> impl Fn() -> Result<u64, String> {
        move || Ok(free)
    }

    /// 手工拼 zip（恶意包/改包测试用）：条目名 + 字节。
    fn write_zip(entries: &[(&str, Vec<u8>)], target: &Path) {
        let file = std::fs::File::create(target).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, bytes) in entries {
            zip.start_file(name, options).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
    }

    fn read_zip_entry(pkg: &Path, name: &str) -> Vec<u8> {
        let f = std::fs::File::open(pkg).unwrap();
        let mut z = zip::ZipArchive::new(f).unwrap();
        let mut e = z.by_name(name).unwrap();
        let mut buf = Vec::new();
        e.read_to_end(&mut buf).unwrap();
        buf
    }

    fn read_zip_names(pkg: &Path) -> Vec<String> {
        let f = std::fs::File::open(pkg).unwrap();
        let mut z = zip::ZipArchive::new(f).unwrap();
        (0..z.len()).map(|i| z.by_index(i).unwrap().name().to_string()).collect()
    }

    const STAMP: &str = "20260919-120000";

    fn live_lock(db_path: &Path) -> Mutex<Connection> {
        Mutex::new(crate::db::open_and_migrate(db_path).unwrap())
    }

    /// 造一个带 2 张照片的数据包（备份库引用 1/uuid-a.jpg 与 1/uuid-b.jpg）。
    /// 返回 (tempdir 保持存活用元组, 包路径, 照片内容)。
    fn package_fixture() -> (TempDir, PathBuf, (Vec<u8>, Vec<u8>)) {
        let dir = TempDir::new().unwrap();
        let pkg_db = dir.path().join("pkg-src").join(crate::db::DB_FILE_NAME);
        let photos_root = dir.path().join("pkg-src").join(crate::photo::PHOTOS_DIR_NAME);
        std::fs::create_dir_all(dir.path().join("pkg-src")).unwrap();
        make_backup_db(&pkg_db);
        // 给备份库挂两带照片的巢况引用
        {
            let conn = crate::db::open_and_migrate(&pkg_db).unwrap();
            let checkin_id = crate::nest_checkin::save_checkin(
                &conn,
                &crate::nest_checkin::CheckinInput {
                    colony_id: 1,
                    date: "2026-09-17".into(),
                    queen_count: None,
                    worker_count: None,
                    moved_nest: false,
                    note: Some("照片两条".into()),
                },
                "2026-09-17",
                "2026-09-17 21:00:00",
            )
            .unwrap()
            .id;
            for rel in ["1/uuid-a.jpg", "1/uuid-b.jpg"] {
                let p = join_rel(&photos_root, rel);
                std::fs::create_dir_all(p.parent().unwrap()).unwrap();
                std::fs::write(&p, photo_bytes(rel.as_bytes()[0])).unwrap();
                conn.execute(
                    "INSERT INTO nest_photo (checkin_id, rel_path, original_name, note)
                     VALUES (?1, ?2, '', '')",
                    params![checkin_id, rel],
                )
                .unwrap();
            }
            drop(conn);
        }
        let pkg = dir.path().join("ant-feeding-log-backup-20260919-083000.zip");
        let outcome = build_real_package(&pkg_db, &photos_root, &pkg);
        assert_eq!(outcome.photo_count, 2);
        let a = read_zip_entry(&pkg, "1/uuid-a.jpg");
        let b = read_zip_entry(&pkg, "1/uuid-b.jpg");
        (dir, pkg, (a, b))
    }

    // ── 命名 ─────────────────────────────────────────────────────────────

    #[test]
    fn pkg_staging_dir_name_uses_own_prefix() {
        assert_eq!(
            pkg_staging_dir_name(STAMP),
            "restore-staging-pkg-20260919-120000"
        );
        // 与裸库 staging（restore-staging-<stamp>.db）、快照命名互不重叠
        assert_ne!(
            pkg_staging_dir_name(STAMP),
            restore::staging_file_name(STAMP)
        );
    }

    // ── manifest 校验 ────────────────────────────────────────────────────

    fn good_manifest() -> PackageManifest {
        PackageManifest {
            format_version: FORMAT_VERSION,
            app_version: "0.3.0".into(),
            created_at: "2026-09-19 08:30:00".into(),
            db_file: DB_ENTRY.to_string(),
            db_sha256: hex_of(b"db"),
            photos: vec![backup_pkg::PackageManifestPhoto {
                path: "1/uuid-a.jpg".into(),
                sha256: hex_of(b"a"),
                bytes: 1,
            }],
            photo_count: 1,
            total_bytes: 1,
        }
    }

    #[test]
    fn validate_manifest_rejects_future_and_invalid_versions_with_plain_words() {
        let mut m = good_manifest();
        m.format_version = FORMAT_VERSION + 1;
        let err = validate_manifest(&m).unwrap_err();
        assert!(err.contains("未来版本"), "实际：{err}");
        assert!(err.contains("请先升级应用"), "实际：{err}");

        let mut m = good_manifest();
        m.format_version = 0;
        let err = validate_manifest(&m).unwrap_err();
        assert!(err.contains("版本无效"), "实际：{err}");

        let mut m = good_manifest();
        m.format_version = FORMAT_VERSION;
        assert!(validate_manifest(&m).is_ok());
    }

    #[test]
    fn validate_manifest_rejects_bad_db_file_dup_paths_and_inconsistent_counts() {
        let mut m = good_manifest();
        m.db_file = "other.db".into();
        assert!(validate_manifest(&m).unwrap_err().contains("库文件名"));

        let mut m = good_manifest();
        m.photos.push(backup_pkg::PackageManifestPhoto {
            path: "1/uuid-a.jpg".into(),
            sha256: "x".into(),
            bytes: 0,
        });
        assert!(validate_manifest(&m).unwrap_err().contains("重复"));

        let mut m = good_manifest();
        m.photo_count = 2;
        assert!(validate_manifest(&m).unwrap_err().contains("张数"));

        let mut m = good_manifest();
        m.total_bytes = 99;
        assert!(validate_manifest(&m).unwrap_err().contains("总字节"));

        // 非法路径（grammar 与打包侧同源）：越界/反斜杠/点开头目录全拒
        for bad in ["1/../x.jpg", "1\\x.jpg", ".orphan-1/x.jpg", "1/.tmp-x.jpg", "/x.jpg", "C:/x.jpg", "solo.jpg", "1/x.PNG"] {
            let mut m = good_manifest();
            m.photos[0].path = bad.into();
            assert!(validate_manifest(&m).unwrap_err().contains("非法照片路径"), "{bad}");
        }
    }

    // ── 空间预检（纯函数 + 注入假配额）────────────────────────────────────

    #[test]
    fn check_disk_space_applies_1_2x_margin_and_reports_mb() {
        // 100MB 解压 → 需 120MB：可用 120MB 恰好过，119MB 拒
        let total = 100 * 1024 * 1024;
        assert!(check_disk_space(total, 120 * 1024 * 1024).is_ok());
        let err = check_disk_space(total, 119 * 1024 * 1024).unwrap_err();
        assert!(err.contains("磁盘空间不足"), "实际：{err}");
        assert!(err.contains("120"), "错误带需求量（MB 口径）：{err}");
        // 零需求（空包）恒过
        assert!(check_disk_space(0, 0).is_ok());
    }

    #[test]
    fn stage_rejects_package_when_injected_free_space_is_too_low() {
        let (_dir, data_dir, db_path) = fixture();
        let before = db_bytes(&db_path);
        let (_pdir, pkg, _) = package_fixture();

        let err = run_preview_pkg(&pkg, &data_dir, STAMP, &space_of(1))
            .unwrap_err();
        assert!(err.contains("磁盘空间不足"), "实际：{err}");
        assert!(no_staging_left(&data_dir), "空间不足不落暂存");
        assert_eq!(db_bytes(&db_path), before, "当前库零改动");
    }

    // ── preview：摘要 + 暂存清理 + 当前库零改动 ───────────────────────────

    #[test]
    fn preview_package_returns_summary_with_photo_count_and_cleans_staging() {
        let (_dir, data_dir, db_path) = fixture();
        let before = db_bytes(&db_path);
        let (_pdir, pkg, _) = package_fixture();

        let summary = run_preview_pkg(&pkg, &data_dir, STAMP, &space_ok()).unwrap();

        assert_eq!(summary.colony_count, 1, "备份库 1 窝");
        assert_eq!(summary.log_count, 2, "备份库 2 条");
        assert_eq!(summary.photo_count, 2, "photo_count = manifest 张数");
        assert_eq!(
            summary.backup_date.as_deref(),
            Some("2026-09-19"),
            "备份日期取 manifest.created_at 的日期部分"
        );
        assert!(no_staging_left(&data_dir), "preview 不留暂存目录");
        assert_eq!(db_bytes(&db_path), before, "当前库字节级零改动");
    }

    #[test]
    fn preview_rejects_empty_and_non_zip_files_with_plain_words() {
        let (_dir, data_dir, db_path) = fixture();
        let before = db_bytes(&db_path);

        let empty = _dir.path().join("empty.zip");
        std::fs::write(&empty, b"").unwrap();
        let err = run_preview_pkg(&empty, &data_dir, STAMP, &space_ok()).unwrap_err();
        assert!(err.contains("文件为空"), "实际：{err}");

        let garbage = _dir.path().join("garbage.zip");
        std::fs::write(&garbage, b"this is not a zip archive............").unwrap();
        let err = run_preview_pkg(&garbage, &data_dir, STAMP, &space_ok()).unwrap_err();
        assert!(
            err.contains("这不是本应用的数据包") || err.contains("无法读取"),
            "实际：{err}"
        );

        assert!(no_staging_left(&data_dir));
        assert_eq!(db_bytes(&db_path), before);
    }

    #[test]
    fn preview_rejects_package_without_manifest_or_with_broken_manifest() {
        let (_dir, data_dir, _db_path) = fixture();
        // 无 manifest 的 zip
        let no_manifest = _dir.path().join("no-manifest.zip");
        write_zip(&[("1/x.jpg", b"x".to_vec())], &no_manifest);
        let err = run_preview_pkg(&no_manifest, &data_dir, STAMP, &space_ok()).unwrap_err();
        assert!(err.contains("缺少 manifest.json"), "实际：{err}");

        // manifest 非 JSON
        let bad_json = _dir.path().join("bad-json.zip");
        write_zip(
            &[(MANIFEST_ENTRY, b"not json".to_vec())],
            &bad_json,
        );
        let err = run_preview_pkg(&bad_json, &data_dir, STAMP, &space_ok()).unwrap_err();
        assert!(err.contains("无法解析"), "实际：{err}");

        assert!(no_staging_left(&data_dir));
    }

    // ── 哈希校验 ─────────────────────────────────────────────────────────

    #[test]
    fn preview_rejects_package_when_db_bytes_do_not_match_manifest_hash() {
        let (_dir, data_dir, _db_path) = fixture();
        let (_pdir, pkg, _) = package_fixture();
        // 重拼：原 manifest + 被篡改的库字节（条目名/清单照旧）
        let manifest_bytes = read_zip_entry(&pkg, MANIFEST_ENTRY);
        let evil_db = {
            let mut b = read_zip_entry(&pkg, DB_ENTRY);
            b[64] ^= 0xFF; // SQLite 头 100 字节内翻一位，必与清单哈希不符
            b
        };
        let tampered = _dir.path().join("tampered-db.zip");
        write_zip(
            &[
                (DB_ENTRY, evil_db),
                ("1/uuid-a.jpg", read_zip_entry(&pkg, "1/uuid-a.jpg")),
                ("1/uuid-b.jpg", read_zip_entry(&pkg, "1/uuid-b.jpg")),
                (MANIFEST_ENTRY, manifest_bytes),
            ],
            &tampered,
        );
        let err = run_preview_pkg(&tampered, &data_dir, STAMP, &space_ok()).unwrap_err();
        assert!(err.contains("校验失败") && err.contains(DB_ENTRY), "实际：{err}");
        assert!(no_staging_left(&data_dir));
    }

    #[test]
    fn preview_rejects_package_when_photo_bytes_do_not_match_manifest_hash() {
        let (_dir, data_dir, _db_path) = fixture();
        let (_pdir, pkg, _) = package_fixture();
        let manifest_bytes = read_zip_entry(&pkg, MANIFEST_ENTRY);
        let tampered = _dir.path().join("tampered-photo.zip");
        write_zip(
            &[
                (DB_ENTRY, read_zip_entry(&pkg, DB_ENTRY)),
                ("1/uuid-a.jpg", b"evil-bytes".to_vec()),
                ("1/uuid-b.jpg", read_zip_entry(&pkg, "1/uuid-b.jpg")),
                (MANIFEST_ENTRY, manifest_bytes),
            ],
            &tampered,
        );
        let err = run_preview_pkg(&tampered, &data_dir, STAMP, &space_ok()).unwrap_err();
        assert!(err.contains("校验失败") && err.contains("uuid-a"), "实际：{err}");
        assert!(no_staging_left(&data_dir));
    }

    #[test]
    fn preview_rejects_package_missing_a_manifest_listed_entry() {
        let (_dir, data_dir, _db_path) = fixture();
        let (_pdir, pkg, _) = package_fixture();
        // 清单里列了 uuid-b，包里没放这条目
        let manifest_bytes = read_zip_entry(&pkg, MANIFEST_ENTRY);
        let short = _dir.path().join("short.zip");
        write_zip(
            &[
                (DB_ENTRY, read_zip_entry(&pkg, DB_ENTRY)),
                ("1/uuid-a.jpg", read_zip_entry(&pkg, "1/uuid-a.jpg")),
                (MANIFEST_ENTRY, manifest_bytes),
            ],
            &short,
        );
        let err = run_preview_pkg(&short, &data_dir, STAMP, &space_ok()).unwrap_err();
        assert!(err.contains("缺少条目") && err.contains("uuid-b"), "实际：{err}");
        assert!(no_staging_left(&data_dir));
    }

    // ── 恶意 zip：表驱动全拒（验收第 2 条）────────────────────────────────

    /// 用真包的 manifest + 库打底，注入一个恶意/异常条目后必须整包拒。
    fn evil_entry_rejected(name: &str, bytes: Vec<u8>, want: &[&str]) {
        let dir = TempDir::new().unwrap();
        let data_dir = dir.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        let (_pdir, pkg, _) = package_fixture();
        let mut all: Vec<(&str, Vec<u8>)> = vec![
            (DB_ENTRY, read_zip_entry(&pkg, DB_ENTRY)),
            ("1/uuid-a.jpg", read_zip_entry(&pkg, "1/uuid-a.jpg")),
            ("1/uuid-b.jpg", read_zip_entry(&pkg, "1/uuid-b.jpg")),
            (MANIFEST_ENTRY, read_zip_entry(&pkg, MANIFEST_ENTRY)),
        ];
        all.push((name, bytes));
        let mixed = dir.path().join("mixed.zip");
        write_zip(&all, &mixed);
        let err = run_preview_pkg(&mixed, &data_dir, STAMP, &space_ok()).unwrap_err();
        for w in want {
            assert!(err.contains(w), "注入 {name}：期望含 {w}，实际：{err}");
        }
        assert!(no_staging_left(&data_dir), "注入 {name}：暂存必须清干净");
    }

    #[test]
    fn malicious_zips_are_all_rejected() {
        // 路径穿越（zip 条目与清单外两种身份都试）
        evil_entry_rejected("1/../evil.jpg", b"x".to_vec(), &["越界"]);
        evil_entry_rejected("../escape.jpg", b"x".to_vec(), &["越界"]);
        // ADS 冒号（NTFS 数据流）
        evil_entry_rejected("1/ads.jpg:hidden", b"x".to_vec(), &["冒号"]);
        // 驱器相对 / 绝对路径
        evil_entry_rejected("C:evil.jpg", b"x".to_vec(), &["冒号"]);
        evil_entry_rejected("/abs/evil.jpg", b"x".to_vec(), &["绝对路径"]);
        // 反斜杠
        evil_entry_rejected("1\\evil.jpg", b"x".to_vec(), &["反斜杠"]);
        // 清单外文件
        evil_entry_rejected("1/extra.jpg", b"x".to_vec(), &["清单外"]);
        evil_entry_rejected("manifest.json.bak", b"x".to_vec(), &["清单外"]);
        // 目录条目
        evil_entry_rejected("1/", b"".to_vec(), &["目录条目"]);
    }

    #[test]
    fn symlink_flagged_entry_is_rejected_even_when_named_in_manifest() {
        // 符号链接条目：名字必须真在清单里（否则先死在清单外闸上），才能证明
        // 符号链接闸独立生效。ZipWriter::add_symlink 产出带 S_IFLNK 模式位的条目。
        let dir = TempDir::new().unwrap();
        let data_dir = dir.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        let (_pdir, pkg, _) = package_fixture();
        let f = std::fs::File::create(dir.path().join("symlink.zip")).unwrap();
        let mut z = zip::ZipWriter::new(f);
        let options = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        z.start_file(DB_ENTRY, options).unwrap();
        z.write_all(&read_zip_entry(&pkg, DB_ENTRY)).unwrap();
        z.add_symlink("1/uuid-a.jpg", "/etc/passwd", options).unwrap();
        z.start_file(MANIFEST_ENTRY, options).unwrap();
        z.write_all(&read_zip_entry(&pkg, MANIFEST_ENTRY)).unwrap();
        z.finish().unwrap();

        let err = run_preview_pkg(&dir.path().join("symlink.zip"), &data_dir, STAMP, &space_ok())
            .unwrap_err();
        assert!(
            err.contains("符号链接") && err.contains("uuid-a"),
            "实际：{err}"
        );
        assert!(no_staging_left(&data_dir));
    }

    #[test]
    fn duplicate_zip_entries_are_rejected() {
        let (_dir, data_dir, _db_path) = fixture();
        let (_pdir, pkg, _) = package_fixture();
        // 同一条目写两次（zip 规范允许；zip crate 的 writer 自己拒绝重名，故手拼
        // 原始字节：读侧 by_index 会看到两条同名 → 必须整包拒）
        let dup = _dir.path().join("dup.zip");
        std::fs::write(
            &dup,
            craft_zip_raw(&[
                (DB_ENTRY, &read_zip_entry(&pkg, DB_ENTRY)),
                (MANIFEST_ENTRY, &read_zip_entry(&pkg, MANIFEST_ENTRY)),
                ("1/uuid-a.jpg", &read_zip_entry(&pkg, "1/uuid-a.jpg")),
                ("1/uuid-a.jpg", &read_zip_entry(&pkg, "1/uuid-a.jpg")),
            ]),
        )
        .unwrap();

        let err = run_preview_pkg(&dup, &data_dir, STAMP, &space_ok()).unwrap_err();
        assert!(err.contains("重复条目"), "实际：{err}");
        assert!(no_staging_left(&data_dir));
    }

    /// 手拼 STORED 条目的原始 zip 字节（测试专用；支持重名条目——zip crate 的
    /// writer 拒绝重名，而读侧的重名闸正是要测的对象）。
    fn craft_zip_raw(entries: &[(&str, &[u8])]) -> Vec<u8> {
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
        fn push_u16(v: &mut Vec<u8>, x: u16) {
            v.extend_from_slice(&x.to_le_bytes());
        }
        fn push_u32(v: &mut Vec<u8>, x: u32) {
            v.extend_from_slice(&x.to_le_bytes());
        }
        let mut out = Vec::new();
        struct Central {
            name: String,
            crc: u32,
            size: u32,
            offset: u32,
        }
        let mut centrals: Vec<Central> = Vec::new();
        for (name, data) in entries {
            let offset = out.len() as u32;
            let crc = crc32(data);
            push_u32(&mut out, 0x0403_4b50); // local file header
            push_u16(&mut out, 20); // version needed
            push_u16(&mut out, 0); // flags
            push_u16(&mut out, 0); // method = stored
            push_u16(&mut out, 0); // time
            push_u16(&mut out, 0x21); // date（非零即可）
            push_u32(&mut out, crc);
            push_u32(&mut out, data.len() as u32);
            push_u32(&mut out, data.len() as u32);
            push_u16(&mut out, name.len() as u16);
            push_u16(&mut out, 0); // extra len
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(data);
            centrals.push(Central {
                name: name.to_string(),
                crc,
                size: data.len() as u32,
                offset,
            });
        }
        let cd_offset = out.len() as u32;
        for c in &centrals {
            push_u32(&mut out, 0x0201_4b50); // central directory header
            push_u16(&mut out, 20); // version made by
            push_u16(&mut out, 20); // version needed
            push_u16(&mut out, 0); // flags
            push_u16(&mut out, 0); // method
            push_u16(&mut out, 0); // time
            push_u16(&mut out, 0x21); // date
            push_u32(&mut out, c.crc);
            push_u32(&mut out, c.size);
            push_u32(&mut out, c.size);
            push_u16(&mut out, c.name.len() as u16);
            push_u16(&mut out, 0); // extra len
            push_u16(&mut out, 0); // comment len
            push_u16(&mut out, 0); // disk start
            push_u16(&mut out, 0); // internal attrs
            push_u32(&mut out, 0); // external attrs
            push_u32(&mut out, c.offset);
            out.extend_from_slice(c.name.as_bytes());
        }
        let cd_size = out.len() as u32 - cd_offset;
        push_u32(&mut out, 0x0605_4b50); // end of central directory
        push_u16(&mut out, 0);
        push_u16(&mut out, 0);
        push_u16(&mut out, centrals.len() as u16);
        push_u16(&mut out, centrals.len() as u16);
        push_u32(&mut out, cd_size);
        push_u32(&mut out, cd_offset);
        push_u16(&mut out, 0); // comment len
        out
    }

    // ── 覆盖校验：包照片集合 ⊉ 库引用 → 拒 ────────────────────────────────

    #[test]
    fn preview_rejects_package_missing_photos_referenced_by_db() {
        let dir = TempDir::new().unwrap();
        let data_dir = dir.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        // 库引用 1/uuid-a.jpg，但包里不带它（manifest 也不列）→ 解包/校验都过，
        // 覆盖检查必须拦下
        let pkg_db = dir.path().join("src.db");
        let photos_root = dir.path().join("photos");
        std::fs::create_dir_all(&photos_root).unwrap();
        make_db_with_photo(&pkg_db, &photos_root, "1/uuid-a.jpg", &photo_bytes(0x61));
        let pkg = dir.path().join("full.zip");
        build_real_package(&pkg_db, &photos_root, &pkg);
        // 抠掉照片条目 + manifest 里对应清单项 → 重建自洽但缺照片的包
        let manifest: serde_json::Value =
            serde_json::from_slice(&read_zip_entry(&pkg, MANIFEST_ENTRY)).unwrap();
        let mut m = manifest.clone();
        m["photos"] = serde_json::Value::Array(vec![]);
        m["photo_count"] = serde_json::json!(0);
        m["total_bytes"] = serde_json::json!(0);
        let short = dir.path().join("short-coverage.zip");
        write_zip(
            &[
                (DB_ENTRY, read_zip_entry(&pkg, DB_ENTRY)),
                (MANIFEST_ENTRY, serde_json::to_vec(&m).unwrap()),
            ],
            &short,
        );

        let err = run_preview_pkg(&short, &data_dir, STAMP, &space_ok()).unwrap_err();
        assert!(err.contains("包内缺 1 张照片"), "实际：{err}");
        assert!(err.contains("1/uuid-a.jpg"), "错误点出缺失的路径：{err}");
        assert!(no_staging_left(&data_dir));
    }

    // ── apply：端到端 ─────────────────────────────────────────────────────

    #[test]
    fn apply_package_replaces_db_lands_photos_evicts_and_cleans_staging() {
        let (_dir, data_dir, db_path) = fixture();
        let photos_root = data_dir.join(crate::photo::PHOTOS_DIR_NAME);
        // 当前 photos/：旧数据照片（新库不引用 → 清退）+ 既有隔离区（保留）
        std::fs::create_dir_all(photos_root.join("7")).unwrap();
        std::fs::write(photos_root.join("7").join("old.jpg"), b"old-data").unwrap();
        std::fs::write(photos_root.join("7").join(".tmp-half"), b"half").unwrap();
        let orphan_dir = photos_root.join(".orphan-20260901-010101");
        std::fs::create_dir_all(&orphan_dir).unwrap();
        std::fs::write(orphan_dir.join("quarantined.jpg"), b"q").unwrap();

        let (_pdir, pkg, (a, b)) = package_fixture();

        let lock = live_lock(&db_path);
        let outcome = crate::restore::run_apply(
            &pkg,
            &db_path,
            &data_dir,
            STAMP,
            || lock.lock().map_err(|e| e.to_string()),
            |p| crate::db::open_and_migrate(p).map_err(|e| e.to_string()),
        )
        .unwrap();

        assert_eq!(outcome, ApplyOutcome::Done);
        // 换库生效
        let conn = crate::db::open_and_migrate(&db_path).unwrap();
        let name: String = conn.query_row("SELECT name FROM colony", [], |r| r.get(0)).unwrap();
        assert_eq!(name, "备份窝", "整库替换生效");
        drop(conn);
        // 照片落位：字节与包内一致
        assert_eq!(std::fs::read(photos_root.join("1/uuid-a.jpg")).unwrap(), a);
        assert_eq!(std::fs::read(photos_root.join("1/uuid-b.jpg")).unwrap(), b);
        // 清退：新库不引用的旧照片与 .tmp- 残留都没了；隔离区原样
        assert!(!photos_root.join("7/old.jpg").exists(), "旧数据照片被清退");
        assert!(!photos_root.join("7/.tmp-half").exists(), ".tmp- 残留被清退");
        assert!(orphan_dir.join("quarantined.jpg").exists(), "隔离区整个保留");
        // 暂存目录清干净；快照产出（数据包形态）
        assert!(no_staging_left(&data_dir));
        let snaps: Vec<String> = list_names(&data_dir)
            .into_iter()
            .filter(|n| n.starts_with(restore::SNAPSHOT_PREFIX))
            .collect();
        assert_eq!(snaps.len(), 1, "实际：{snaps:?}");
        assert!(snaps[0].ends_with(".zip"), "快照是数据包形态");
        // 锁内连接指向新库
        let name: String = lock
            .lock()
            .unwrap()
            .query_row("SELECT name FROM colony", [], |r| r.get(0))
            .unwrap();
        assert_eq!(name, "备份窝");
    }

    #[test]
    fn apply_package_failure_before_swap_keeps_current_state_and_cleans_staging() {
        let (_dir, data_dir, db_path) = fixture();
        let before = db_bytes(&db_path);
        // 注入快照失败：同 stamp 快照落点被「目录」占住
        let snap_target = data_dir.join(restore::snapshot_file_name(STAMP));
        std::fs::create_dir_all(&snap_target).unwrap();
        let (_pdir, pkg, _) = package_fixture();
        let lock = live_lock(&db_path);

        let err = crate::restore::run_apply(
            &pkg,
            &db_path,
            &data_dir,
            STAMP,
            || lock.lock().map_err(|e| e.to_string()),
            |p| crate::db::open_and_migrate(p).map_err(|e| e.to_string()),
        )
        .unwrap_err();

        assert!(!err.is_empty());
        assert_eq!(db_bytes(&db_path), before, "换库前失败：当前库零改动");
        assert!(no_staging_left(&data_dir), "换库前失败：暂存目录清干净");
        assert!(snap_target.is_dir(), "占位目录（非自家产物）不误删");
        // 当前 photos/ 零改动（根本没建）
        assert!(
            !data_dir.join(crate::photo::PHOTOS_DIR_NAME).join("1").exists(),
            "换库前失败：photos/ 零改动"
        );
    }

    #[test]
    fn apply_landing_failure_keeps_staging_and_resume_completes_landing() {
        let (_dir, data_dir, db_path) = fixture();
        let photos_root = data_dir.join(crate::photo::PHOTOS_DIR_NAME);
        let (_pdir, pkg, (a, _b)) = package_fixture();
        // 注入落位失败：photos/ 位置被「文件」占住 → 建照片目录必失败
        std::fs::create_dir_all(&data_dir).unwrap();
        std::fs::write(&photos_root, b"not-a-dir").unwrap();
        let lock = live_lock(&db_path);

        let err = crate::restore::run_apply(
            &pkg,
            &db_path,
            &data_dir,
            STAMP,
            || lock.lock().map_err(|e| e.to_string()),
            |p| crate::db::open_and_migrate(p).map_err(|e| e.to_string()),
        )
        .unwrap_err();

        assert!(err.contains("落位失败"), "实际：{err}");
        assert!(err.contains("重启应用后将自动继续"), "错误带续跑指引：{err}");
        // 换库已发生（新库内容在位）
        let conn = crate::db::open_and_migrate(&db_path).unwrap();
        let name: String = conn.query_row("SELECT name FROM colony", [], |r| r.get(0)).unwrap();
        assert_eq!(name, "备份窝", "库已换");
        drop(conn);
        // 暂存目录保留（含换库标记与照片），可续
        let staging = data_dir.join(pkg_staging_dir_name(STAMP));
        assert!(staging.is_dir(), "落位失败：暂存目录保留供续跑");
        assert!(staging.join(SWAP_MARKER_FILE).exists(), "换库标记在");
        assert_eq!(
            std::fs::read(staging.join("photos/1/uuid-a.jpg")).unwrap(),
            a,
            "照片还在暂存区待落位"
        );

        // 解除注入 → 启动续跑：照片落位、暂存清掉
        std::fs::remove_file(&photos_root).unwrap();
        let live = crate::db::open_and_migrate(&db_path).unwrap();
        let resume = resume_pending_pkg_restores(&data_dir, &photos_root, &live);
        assert!(resume.errors.is_empty(), "实际：{:?}", resume.errors);
        assert_eq!(resume.resumed.len(), 1, "续跑一个暂存目录：{:?}", resume.resumed);
        assert!(resume.landed.contains(&"1/uuid-a.jpg".to_string()));
        assert!(resume.landed.contains(&"1/uuid-b.jpg".to_string()));
        drop(live);
        assert_eq!(std::fs::read(photos_root.join("1/uuid-a.jpg")).unwrap(), a);
        assert!(!staging.exists(), "续跑完成：暂存目录清掉");
    }

    #[test]
    fn resume_discards_staging_without_swap_marker() {
        let (_dir, data_dir, db_path) = fixture();
        let photos_root = data_dir.join(crate::photo::PHOTOS_DIR_NAME);
        // 半截暂存（解包中途被杀：无换库标记）
        let staging = data_dir.join(pkg_staging_dir_name(STAMP));
        std::fs::create_dir_all(staging.join("photos/1")).unwrap();
        std::fs::write(staging.join("photos/1/uuid-a.jpg"), b"half").unwrap();
        std::fs::write(staging.join(DB_ENTRY), b"half-db").unwrap();

        let live = crate::db::open_and_migrate(&db_path).unwrap();
        let resume = resume_pending_pkg_restores(&data_dir, &photos_root, &live);
        assert_eq!(resume.discarded, vec![pkg_staging_dir_name(STAMP)]);
        assert!(resume.resumed.is_empty());
        assert!(resume.landed.is_empty());
        assert!(resume.errors.is_empty(), "实际：{:?}", resume.errors);
        assert!(!staging.exists(), "半截暂存被丢弃");
        // 当前库（旧库）引用的照片没被乱落
        assert!(!photos_root.join("1/uuid-a.jpg").exists());
    }

    #[test]
    fn resume_lands_only_current_db_referenced_missing_photos() {
        let (_dir, data_dir, db_path) = fixture();
        let photos_root = data_dir.join(crate::photo::PHOTOS_DIR_NAME);
        // 当前库引用 1/keep.jpg（文件在位）——续跑不得碰它
        make_db_with_photo(&db_path, &photos_root, "1/keep.jpg", b"keep");
        // 暂存目录：带换库标记，photos/1/keep.jpg（同路径旧字节）+ 1/other.jpg
        let staging = data_dir.join(pkg_staging_dir_name(STAMP));
        std::fs::create_dir_all(staging.join("photos/1")).unwrap();
        std::fs::write(staging.join(SWAP_MARKER_FILE), b"").unwrap();
        std::fs::write(staging.join("photos/1/keep.jpg"), b"stale-bytes").unwrap();
        std::fs::write(staging.join("photos/1/other.jpg"), b"other").unwrap();

        let live = crate::db::open_and_migrate(&db_path).unwrap();
        let resume = resume_pending_pkg_restores(&data_dir, &photos_root, &live);
        assert!(resume.errors.is_empty(), "实际：{:?}", resume.errors);
        assert_eq!(resume.resumed.len(), 1);
        assert!(resume.landed.is_empty(), "在位照片不重落：{:?}", resume.landed);
        assert_eq!(
            std::fs::read(photos_root.join("1/keep.jpg")).unwrap(),
            b"keep",
            "库引用且已在位的文件绝不被暂存内容覆盖"
        );
        assert!(!staging.exists(), "续跑后暂存清掉");
    }

    #[test]
    fn resume_keeps_staging_when_landing_fails_and_reports_error() {
        let (_dir, data_dir, db_path) = fixture();
        let photos_root = data_dir.join(crate::photo::PHOTOS_DIR_NAME);
        // 当前库引用 1/want.jpg（photos/ 缺失）；photos/ 根被「文件」占住 → 落位必败
        {
            let conn = crate::db::open_and_migrate(&db_path).unwrap();
            let checkin_id = crate::nest_checkin::save_checkin(
                &conn,
                &crate::nest_checkin::CheckinInput {
                    colony_id: 1,
                    date: "2026-09-17".into(),
                    queen_count: None,
                    worker_count: None,
                    moved_nest: false,
                    note: Some("续跑目标".into()),
                },
                "2026-09-17",
                "2026-09-17 21:00:00",
            )
            .unwrap()
            .id;
            conn.execute(
                "INSERT INTO nest_photo (checkin_id, rel_path, original_name, note)
                 VALUES (?1, '1/want.jpg', '', '')",
                params![checkin_id],
            )
            .unwrap();
        }
        std::fs::write(&photos_root, b"not-a-dir").unwrap();
        // 暂存目录带换库标记且有货
        let staging = data_dir.join(pkg_staging_dir_name(STAMP));
        std::fs::create_dir_all(staging.join("photos/1")).unwrap();
        std::fs::write(staging.join(SWAP_MARKER_FILE), b"").unwrap();
        std::fs::write(staging.join("photos/1/want.jpg"), b"staged").unwrap();

        let live = crate::db::open_and_migrate(&db_path).unwrap();
        let resume = resume_pending_pkg_restores(&data_dir, &photos_root, &live);
        assert_eq!(resume.errors.len(), 1, "实际：{:?}", resume.errors);
        assert!(resume.errors[0].contains(&pkg_staging_dir_name(STAMP)));
        assert!(resume.resumed.is_empty());
        assert!(staging.is_dir(), "落位失败：暂存保留，下次启动重试");

        // 解除注入后重跑：成功续完
        std::fs::remove_file(&photos_root).unwrap();
        let resume2 = resume_pending_pkg_restores(&data_dir, &photos_root, &live);
        assert!(resume2.errors.is_empty(), "实际：{:?}", resume2.errors);
        assert_eq!(resume2.resumed.len(), 1);
        assert_eq!(std::fs::read(photos_root.join("1/want.jpg")).unwrap(), b"staged");
    }

    #[test]
    fn resume_ignores_non_staging_entries_and_missing_dirs() {
        let (_dir, data_dir, db_path) = fixture();
        // 无暂存目录 + 无关文件/目录：无事而返
        std::fs::write(data_dir.join("random.txt"), b"x").unwrap();
        std::fs::create_dir_all(data_dir.join("some-other-dir")).unwrap();
        let live = crate::db::open_and_migrate(&db_path).unwrap();
        let resume = resume_pending_pkg_restores(&data_dir, &data_dir.join("photos"), &live);
        assert_eq!(resume, PkgResumeOutcome::default());
    }

    // ── 落位/清退单元 ─────────────────────────────────────────────────────

    #[test]
    fn land_photos_renames_all_and_overwrites_stale_same_name_files() {
        let dir = TempDir::new().unwrap();
        let staging = dir.path().join("staging");
        let photos = dir.path().join("photos");
        std::fs::create_dir_all(staging.join("photos/1")).unwrap();
        std::fs::write(staging.join("photos/1/a.jpg"), b"new-a").unwrap();
        std::fs::create_dir_all(photos.join("1")).unwrap();
        std::fs::write(photos.join("1/a.jpg"), b"stale-a").unwrap();

        land_photos(&staging, &photos, &["1/a.jpg".into()]).unwrap();

        assert_eq!(std::fs::read(photos.join("1/a.jpg")).unwrap(), b"new-a", "恢复 = 整体替换（覆盖旧同名文件）");
        // 源缺失而目标已在 → 可续语义跳过
        land_photos(&staging, &photos, &["1/a.jpg".into()]).unwrap();
        // 源缺失且目标不在 → 报错
        let err = land_photos(&staging, &photos, &["1/missing.jpg".into()]).unwrap_err();
        assert!(err.contains("暂存区没有"), "实际：{err}");
        // 非法路径防御
        let err = land_photos(&staging, &photos, &["../evil.jpg".into()]).unwrap_err();
        assert!(err.contains("非法"), "实际：{err}");
    }

    #[test]
    fn evict_keeps_manifest_files_and_orphan_dirs_removes_the_rest() {
        let dir = TempDir::new().unwrap();
        let photos = dir.path().join("photos");
        std::fs::create_dir_all(photos.join("1")).unwrap();
        std::fs::create_dir_all(photos.join("2")).unwrap();
        std::fs::create_dir_all(photos.join(".orphan-1")).unwrap();
        std::fs::write(photos.join("1/keep.jpg"), b"k").unwrap();
        std::fs::write(photos.join("1/stale.jpg"), b"s").unwrap();
        std::fs::write(photos.join("2/gone.jpg"), b"g").unwrap();
        std::fs::write(photos.join("1/.tmp-x"), b"t").unwrap();
        std::fs::write(photos.join("loose.jpg"), b"l").unwrap();
        std::fs::write(photos.join(".orphan-1").join("q.jpg"), b"q").unwrap();

        let keep: HashSet<String> = ["1/keep.jpg".to_string()].into_iter().collect();
        let failures = evict_non_manifest_files(&photos, &keep);
        assert!(failures.is_empty(), "实际：{failures:?}");

        assert!(photos.join("1/keep.jpg").exists(), "清单内保留");
        assert!(!photos.join("1/stale.jpg").exists());
        assert!(!photos.join("2/gone.jpg").exists());
        assert!(!photos.join("1/.tmp-x").exists(), ".tmp- 不在清单 → 清退");
        assert!(!photos.join("loose.jpg").exists(), "根下散落文件清退");
        assert!(photos.join(".orphan-1").join("q.jpg").exists(), "隔离区整个保留");
        // 清空后的窝目录被顺手移除（2/ 已空）
        assert!(!photos.join("2").exists(), "清空的窝目录不留空壳");
        assert!(photos.join("1").exists(), "还有清单文件的目录保留");
    }

    #[test]
    fn evict_failure_is_reported_without_blocking_the_rest() {
        let dir = TempDir::new().unwrap();
        let photos = dir.path().join("photos");
        std::fs::create_dir_all(photos.join("1")).unwrap();
        std::fs::write(photos.join("1/keep.jpg"), b"k").unwrap();
        let stuck = photos.join("1/stuck.jpg");
        std::fs::write(&stuck, b"s").unwrap();
        // 注入删失败：Windows 以「只共享读」方式打开（std::fs::File::open 自带
        // 共享删除挡不住 DeleteFileW，必须 OpenOptionsExt::share_mode 显式收窄），
        // DeleteFileW 因共享冲突失败。现代 Windows 对只读文件也不再拒绝删除，
        // 故不走 readonly 属性。
        #[cfg(windows)]
        let _guard = {
            use std::os::windows::fs::OpenOptionsExt;
            const FILE_SHARE_READ: u32 = 0x0000_0001;
            std::fs::OpenOptions::new()
                .read(true)
                .share_mode(FILE_SHARE_READ)
                .open(&stuck)
                .unwrap()
        };
        std::fs::write(photos.join("1/normal.jpg"), b"n").unwrap();

        let keep: HashSet<String> = ["1/keep.jpg".to_string()].into_iter().collect();
        let failures = evict_non_manifest_files(&photos, &keep);
        #[cfg(windows)]
        {
            assert_eq!(failures.len(), 1, "只 stuck 失败：{failures:?}");
            assert!(failures[0].contains("1/stuck.jpg"));
            assert!(stuck.exists(), "失败文件原位（下轮巡检孤儿）");
        }
        #[cfg(not(windows))]
        assert!(failures.is_empty(), "{failures:?}");
        assert!(!photos.join("1/normal.jpg").exists(), "其余照常清退");
        #[cfg(windows)]
        drop(_guard);
    }

    // ── 序列化契约 ───────────────────────────────────────────────────────

    #[test]
    fn pkg_summary_photo_count_serializes_snake_case() {
        let s = RestoreSummary {
            backup_date: Some("2026-09-19".into()),
            colony_count: 1,
            log_count: 2,
            backup_dir_in_backup: None,
            photo_count: 7,
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains(r#""photo_count":7"#), "实际：{json}");
    }
}
