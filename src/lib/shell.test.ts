import { afterEach, describe, expect, it } from "vitest";
import { installContextMenuShield } from "./shell";

/** 在 target 上派发 contextmenu，返回是否被 preventDefault */
function fire(el: Element): boolean {
  const e = new Event("contextmenu", { bubbles: true, cancelable: true });
  el.dispatchEvent(e);
  return e.defaultPrevented;
}

describe("installContextMenuShield", () => {
  afterEach(() => {
    // 卸载：shield 用幂等安装，测试间用新元素隔离即可
  });

  it("屏蔽普通元素与文本节点的右键菜单", () => {
    const off = installContextMenuShield();
    const div = document.createElement("div");
    document.body.appendChild(div);
    expect(fire(div)).toBe(true);
    div.remove();
    off();
  });

  it("输入框 / 文本域 / 可编辑区内放行（保留右键粘贴）", () => {
    const off = installContextMenuShield();
    const input = document.createElement("input");
    const ta = document.createElement("textarea");
    const editable = document.createElement("div");
    editable.setAttribute("contenteditable", "true");
    document.body.append(input, ta, editable);
    expect(fire(input)).toBe(false);
    expect(fire(ta)).toBe(false);
    expect(fire(editable)).toBe(false);
    input.remove(); ta.remove(); editable.remove();
    off();
  });

  it("重复安装幂等（同一卸载器、不叠监听）；off 后不再屏蔽，可重装", () => {
    const off1 = installContextMenuShield();
    const off2 = installContextMenuShield();
    expect(off2).toBe(off1); // 复审 #13/#14：同一卸载器，而非各挂各的
    const div = document.createElement("div");
    document.body.appendChild(div);
    const e = new Event("contextmenu", { bubbles: true, cancelable: true });
    div.dispatchEvent(e);
    expect(e.defaultPrevented).toBe(true);
    off1();
    off2(); // 同一函数，重复调用无害
    const e2 = new Event("contextmenu", { bubbles: true, cancelable: true });
    div.dispatchEvent(e2);
    expect(e2.defaultPrevented).toBe(false);
    const off3 = installContextMenuShield(); // 卸载后可重装
    const e3 = new Event("contextmenu", { bubbles: true, cancelable: true });
    div.dispatchEvent(e3);
    expect(e3.defaultPrevented).toBe(true);
    off3();
    div.remove();
  });

  it("卸载→重装→调用陈旧卸载器：不得清空当前安装的模块态（复审修正）", () => {
    const off1 = installContextMenuShield();
    off1(); // 卸载
    const off2 = installContextMenuShield(); // 重装
    off1(); // 陈旧卸载器被误调：不得清掉 off2 这次的安装态
    const again = installContextMenuShield();
    expect(again).toBe(off2); // 模块态仍是 off2 这次安装（未叠新监听、未被清空）

    const div = document.createElement("div");
    document.body.appendChild(div);
    expect(fire(div)).toBe(true); // 屏蔽仍在生效
    off2(); // 真卸载
    expect(fire(div)).toBe(false); // 放行
    div.remove();
  });
});
