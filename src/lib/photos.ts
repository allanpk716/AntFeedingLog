/**
 * 照片展示与网页端照片通路（webui-checkin 票 07/08）。
 *
 * 桌面显示本地图：数据目录 photos/ 经 Tauri asset 协议读取——scope 限定
 * `$APPDATA/photos/**` 的配置在 src-tauri/tauri.conf.json
 * `app.security.assetProtocol`（JSON 不收注释，注在此处与 get_photo_abs_dir
 * 的 Rust 文档）。路径拼装 = get_photo_abs_dir 返回的绝对根目录 + 命令返回的
 * 相对路径（`<colonyId>/<uuid>.jpg`）。
 *
 * 浏览器（网页端，票 08）：
 * - **取图**：`<img>` 标签带不了 Authorization 头，`loadPhotoBlobUrl` 用 fetch
 *   带 Bearer 请求 `GET /api/photo/<relPath>`，把响应转 blob 后
 *   `URL.createObjectURL`——token 不进 URL；调用方在组件卸载/换图时用
 *   [`revokeObjectUrl`] 释放，防内存泄漏。
 * - **上传**：`uploadPhotosHttp` 先做客户端预检（张数 ≤9、单张 ≤15MB——超限
 *   请求在 Windows 服务端常表现为连接被 RST，413 半路断连，先在本地挡体验），
 *   再以 multipart 表单 POST `/api/photos`（checkinId 文本段 + photos 文件段）。
 *   服务端一套校验链兜底（票 07 纯核），返回与桌面 attach_photos 同形的
 *   NestPhotoMeta[]。
 */

import { convertFileSrc } from "@tauri-apps/api/core";

import { isTauri, WEBUI_TOKEN_KEY } from "./ipc";
import type { NestPhotoMeta } from "../types";

/** 单张压缩前上限（与 Rust photo::MAX_INPUT_BYTES 同口径）：客户端预检用。 */
export const MAX_PHOTO_BYTES = 15 * 1024 * 1024;

/** 单次上传张数上限（与 Rust photo::MAX_PHOTOS_PER_SUBMIT 同口径）。 */
export const MAX_PHOTOS_PER_SUBMIT = 9;

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

/** 带凭证的请求头（本地有令牌才带 Authorization）。 */
function authHeaders(): Record<string, string> {
  const headers: Record<string, string> = {};
  const token = localStorage.getItem(WEBUI_TOKEN_KEY);
  if (token) {
    headers.Authorization = `Bearer ${token}`;
  }
  return headers;
}

/** 非 2xx 响应 → 人话错误串（{ error } 优先，解析不了回 HTTP <status>）。 */
async function httpErrorMessage(res: Response): Promise<string> {
  try {
    const body: unknown = await res.json();
    if (typeof body === "object" && body !== null && "error" in body) {
      const msg = (body as { error: unknown }).error;
      if (typeof msg === "string") return msg;
    }
  } catch {
    // 响应体不是 JSON：落到底部的 HTTP <status>
  }
  return `HTTP ${res.status}`;
}

/** 上传网络层错误（断网/Wi-Fi 切换/超时半路断连，fetch 直接 reject）的人话
 * （终局评审）：提示先刷新核对已落库的照片，避免盲目整批重传造成重复。 */
export const UPLOAD_INTERRUPTED_MSG =
  "上传中断（网络断开或超时）。请刷新查看已保存的照片，避免重复上传后再试";

/**
 * 浏览器取图（票 08）：fetch 带 Bearer 拿 blob → objectURL。失败 throw 人话
 * （缺图 404 的「照片文件缺失…」），调用方标占位符。
 */
export async function loadPhotoBlobUrl(relPath: string): Promise<string> {
  const res = await fetch(`/api/photo/${relPath}`, { headers: authHeaders() });
  if (!res.ok) {
    throw await httpErrorMessage(res);
  }
  const blob = await res.blob();
  return URL.createObjectURL(blob);
}

/** 释放 loadPhotoBlobUrl 产出的 objectURL；只认 blob: 前缀（桌面 asset URL 放过）。 */
export function revokeObjectUrl(url: string): void {
  if (url.startsWith("blob:")) {
    URL.revokeObjectURL(url);
  }
}

/**
 * 浏览器上传（票 08）：客户端预检（张数/单张体积）→ multipart POST
 * /api/photos。返回与桌面 attach_photos 同形的 NestPhotoMeta[]。
 */
export async function uploadPhotosHttp(
  checkinId: number,
  files: File[],
): Promise<NestPhotoMeta[]> {
  if (files.length === 0) return [];
  if (files.length > MAX_PHOTOS_PER_SUBMIT) {
    throw `一次最多上传 ${MAX_PHOTOS_PER_SUBMIT} 张照片`;
  }
  for (const f of files) {
    if (f.size > MAX_PHOTO_BYTES) {
      throw `${f.name}: 单张照片压缩前不能超过 ${MAX_PHOTO_BYTES / 1024 / 1024}MB`;
    }
  }
  const form = new FormData();
  form.append("checkinId", String(checkinId));
  for (const f of files) {
    form.append("photos", f, f.name);
  }
  let res: Response;
  try {
    res = await fetch("/api/photos", {
      method: "POST",
      headers: authHeaders(),
      body: form,
    });
  } catch {
    // 网络层错误（TypeError: Failed to fetch 等）：不透出浏览器原文，换人话
    throw UPLOAD_INTERRUPTED_MSG;
  }
  if (!res.ok) {
    throw await httpErrorMessage(res);
  }
  const body: unknown = await res.json();
  if (!Array.isArray(body)) {
    throw "HTTP 响应不是照片清单";
  }
  return body as NestPhotoMeta[];
}

/** 字节数人话（1024 进制，界面口径）：B / KB / MB / GB。 */
export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  return `${(n / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}
