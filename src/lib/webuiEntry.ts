/**
 * 网页端进门凭证入口（webui-checkin 票 05，规格 B）。
 *
 * 完整访问地址形如 `http://<IP>:<端口>/#token=<凭证>`——凭证放在 URL fragment
 * 里，浏览器不会把它随请求发给服务器。页面加载时经本模块把凭证落进
 * localStorage（ipc.ts 的 httpInvoke 从那里读），并 history.replaceState 清净
 * 地址栏，避免凭证留在可见地址里被随手转发。
 *
 * 定位是「降低暴露」（规格 B）：不承诺清除浏览器历史/书签/同步，设置页另有明示。
 */
import { WEBUI_TOKEN_KEY } from "./ipc";

/**
 * 纯核：从 `#...` fragment 解析进门凭证，并拼出剥掉 token 后的清净地址。
 * 返回 null = 没有 token（不清洗地址、不落存储）；token 空值视为未携带。
 * 其余参数原样保留在 hash 里（只剥 token 一项）。
 */
export function parseTokenHash(
  hash: string,
  pathname: string,
  search: string,
): { token: string; cleanedUrl: string } | null {
  const params = new URLSearchParams(hash.replace(/^#/, ""));
  const token = params.get("token");
  if (!token) {
    return null;
  }
  params.delete("token");
  const rest = params.toString();
  return { token, cleanedUrl: `${pathname}${search}${rest ? `#${rest}` : ""}` };
}

/**
 * 页面入口：location.hash 里有 token 就存 localStorage 并 replaceState 清净
 * 地址，返回 token（无 token 返回 null）。任何一步失败都不打断应用启动
 * （凭证没落地只会让后续请求 401，页面照常渲染）。
 */
export function captureTokenFromHash(): string | null {
  try {
    const parsed = parseTokenHash(
      window.location.hash,
      window.location.pathname,
      window.location.search,
    );
    if (!parsed) {
      return null;
    }
    localStorage.setItem(WEBUI_TOKEN_KEY, parsed.token);
    window.history.replaceState(null, "", parsed.cleanedUrl);
    return parsed.token;
  } catch {
    return null;
  }
}
