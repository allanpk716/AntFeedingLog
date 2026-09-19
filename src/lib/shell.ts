/**
 * 应用外壳行为（交互改进第三轮 #2）：屏蔽 WebView 默认右键菜单——
 * 桌面应用里浏览器菜单（刷新/检查）露馅；输入类控件放行（右键粘贴有用）。
 * 幂等安装，返回卸载函数（目前仅测试用）。
 */

function isEditable(target: EventTarget | null): boolean {
  const el = target instanceof Element ? target : null;
  if (el === null) return false;
  if (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement) return true;
  return el instanceof HTMLElement && el.isContentEditable;
}

let offShield: (() => void) | null = null;

export function installContextMenuShield(): () => void {
  if (offShield !== null) return offShield; // 复审 #13/#14：真幂等——重复安装返回同一卸载器，不叠监听
  const handler = (e: MouseEvent) => {
    if (isEditable(e.target)) return;
    e.preventDefault();
  };
  const uninstall = () => {
    // 复审修正：卸载器执行前校验自身仍是当前安装——陈旧卸载器不得清空模块态
    if (offShield !== uninstall) return;
    document.removeEventListener("contextmenu", handler);
    offShield = null;
  };
  document.addEventListener("contextmenu", handler);
  offShield = uninstall;
  return offShield;
}
