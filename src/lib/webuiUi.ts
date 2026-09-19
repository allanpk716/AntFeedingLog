/**
 * 网页端设置页的纯逻辑（webui-checkin 票 03）：端口校验、凭证打码、固定文案、
 * 剪贴板降级复制。组件（WebUiPanel / WebUiWizard 及其子组件）一律 import 这里。
 */

/** 端口非法提示（1024–65535，与 Rust validate_port 同区间同口径）。 */
export const PORT_ERROR = "端口须在 1024–65535 之间";

/** 勾选物理网段（encrypted_mesh=false）时的硬警示（规格 B：不确认不给勾）。 */
export const PLAINTEXT_WARNING =
  "该网段内凭证与数据将明文传输，网内任何设备都可能看到。确认信任该物理网段吗？";

/** 重生成凭证的确认提示（旧地址立即作废）。 */
export const REGEN_WARNING = "重生成后旧访问地址立即作废，手机书签需要换成新地址。确认重生成？";

/** 完整地址展示区的固定风险提示。 */
export const RISK_HINT =
  "完整地址含进门凭证，勿转发给他人；怀疑泄露时在设置页重生成即可作废旧地址。";

/**
 * 端口输入校验：纯数字且落在 1024–65535 返回数值，否则 null（调用方展示 PORT_ERROR）。
 * number 输入框的 v-model 会给数值（Vue 自动转型），字符串/数值都收；两端空白容忍。
 */
export function validatePortText(value: string | number): number | null {
  const text = String(value).trim();
  if (!/^\d+$/.test(text)) return null;
  const n = Number(text);
  if (!Number.isInteger(n) || n < 1024 || n > 65535) return null;
  return n;
}

/** 凭证打码：保留头尾各 4 位，中间以 … 代替；短凭证（≤8 位）整串打码。 */
export function maskToken(token: string): string {
  if (!token) return "";
  if (token.length <= 8) return "•".repeat(token.length);
  return `${token.slice(0, 4)}…${token.slice(-4)}`;
}

/**
 * 复制文本（一键复制完整地址 / 手动 netsh 命令）。优先 Clipboard API，
 * 不可用（旧 WebView / 非安全上下文）时降级隐藏 textarea + execCommand。
 * 返回是否成功，调用方分别提示「已复制 / 复制失败」。
 */
export async function copyText(text: string): Promise<boolean> {
  try {
    if (typeof navigator !== "undefined" && navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(text);
      return true;
    }
  } catch {
    // 权限被拒等 → 走降级
  }
  try {
    const ta = document.createElement("textarea");
    ta.value = text;
    ta.style.position = "fixed";
    ta.style.opacity = "0";
    document.body.appendChild(ta);
    ta.select();
    const ok = document.execCommand("copy");
    document.body.removeChild(ta);
    return ok;
  } catch {
    return false;
  }
}
