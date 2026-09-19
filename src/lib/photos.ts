/**
 * 照片展示纯函数（webui-checkin 票 07）。
 *
 * 桌面显示本地图：数据目录 photos/ 经 Tauri asset 协议读取——scope 限定
 * `$APPDATA/photos/**` 的配置在 src-tauri/tauri.conf.json
 * `app.security.assetProtocol`（JSON 不收注释，注在此处与 get_photo_abs_dir
 * 的 Rust 文档）。路径拼装 = get_photo_abs_dir 返回的绝对根目录 + 命令返回的
 * 相对路径（`<colonyId>/<uuid>.jpg`）。
 * 浏览器侧（网页端）照片读取随票 08 的 HTTP 端点接线，届时本文件补 web 分支；
 * 现状浏览器环境一律返回空串，由组件渲染「预览不可用」占位，不崩溃。
 */

import { convertFileSrc } from "@tauri-apps/api/core";

import { isTauri } from "./ipc";

/** 相对路径 + 照片根目录 → 可显示 URL；桌面外或根目录未就绪返回空串。 */
export function photoSrc(relPath: string, photoAbsDir: string): string {
  if (!photoAbsDir || !relPath) return "";
  if (!isTauri()) return "";
  return convertFileSrc(joinPhotoPath(photoAbsDir, relPath));
}

/** 拼绝对路径：根目录尾部分隔符归一 + 正斜杠连接（Windows 混合分隔符可解析）。 */
export function joinPhotoPath(dir: string, relPath: string): string {
  const d = dir.replace(/[\\/]+$/, "");
  const rel = relPath.replace(/^\/+/, "");
  return `${d}/${rel}`;
}

/** 字节数人话（1024 进制，界面口径）：B / KB / MB / GB。 */
export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  return `${(n / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}
