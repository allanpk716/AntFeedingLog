// @vitest-environment node
// 票 03：版本一致性校验脚本单测（tag == tauri.conf.json version；三处版本号互相一致）
import { describe, expect, it } from "vitest";
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  checkVersions,
  collectVersions,
  extractCargoPackageVersion,
  runCli,
} from "./check-versions.mjs";

// 在临时目录里造一个"迷你仓库"：package.json / src-tauri/tauri.conf.json / src-tauri/Cargo.toml
function makeRepo(overrides = {}) {
  const conf = {
    pkg: overrides.pkg ?? "0.2.0",
    tauri: overrides.tauri ?? "0.2.0",
    cargo: overrides.cargo ?? "0.2.0",
    cargoExtra: overrides.cargoExtra ?? "",
    omit: overrides.omit ?? [],
  };
  const dir = mkdtempSync(join(tmpdir(), "check-versions-"));
  if (!conf.omit.includes("package.json")) {
    writeFileSync(
      join(dir, "package.json"),
      JSON.stringify({ name: "ant-feeding-log", version: conf.pkg }),
      "utf8",
    );
  }
  mkdirSync(join(dir, "src-tauri"), { recursive: true });
  if (!conf.omit.includes("tauri.conf.json")) {
    writeFileSync(
      join(dir, "src-tauri", "tauri.conf.json"),
      JSON.stringify({ productName: "X", version: conf.tauri }),
      "utf8",
    );
  }
  if (!conf.omit.includes("Cargo.toml")) {
    writeFileSync(
      join(dir, "src-tauri", "Cargo.toml"),
      `[package]\nname = "ant-feeding-log"\nversion = "${conf.cargo}"\n\n[dependencies]\n${conf.cargoExtra}`,
      "utf8",
    );
  }
  return dir;
}

function cleanup(dir) {
  rmSync(dir, { recursive: true, force: true });
}

describe("extractCargoPackageVersion", () => {
  it("只取 [package] 段的 version，不被依赖段的 version 干扰", () => {
    const toml = [
      "[package]",
      'name = "ant-feeding-log"',
      'version = "0.2.0"',
      "",
      "[dependencies]",
      'tauri = { version = "2.9.0", features = [] }',
      'serde = "1.0.228"',
    ].join("\n");
    expect(extractCargoPackageVersion(toml)).toBe("0.2.0");
  });

  it("[package] 段没有 version 时返回 null", () => {
    expect(extractCargoPackageVersion('[package]\nname = "x"\n')).toBeNull();
  });
});

describe("collectVersions", () => {
  it("读出三处版本号", () => {
    const dir = makeRepo();
    try {
      expect(collectVersions(dir)).toEqual({
        package: "0.2.0",
        tauri: "0.2.0",
        cargo: "0.2.0",
      });
    } finally {
      cleanup(dir);
    }
  });
});

describe("checkVersions", () => {
  it("三处一致且 tag（带 v 前缀）匹配 → 通过", () => {
    const errors = checkVersions(
      { package: "0.2.0", tauri: "0.2.0", cargo: "0.2.0" },
      "v0.2.0",
    );
    expect(errors).toEqual([]);
  });

  it("tag 不带 v 前缀也接受", () => {
    const errors = checkVersions(
      { package: "0.2.0", tauri: "0.2.0", cargo: "0.2.0" },
      "0.2.0",
    );
    expect(errors).toEqual([]);
  });

  it("三处一致但 tag 不匹配 → 报 tag 与 tauri.conf.json version 不一致", () => {
    const errors = checkVersions(
      { package: "0.1.0", tauri: "0.1.0", cargo: "0.1.0" },
      "v0.2.0",
    );
    expect(errors).toHaveLength(1);
    expect(errors[0]).toContain("v0.2.0");
    expect(errors[0]).toContain("0.1.0");
    expect(errors[0]).toContain("tauri.conf.json");
  });

  it("一处版本错（Cargo.toml 落后）→ 报三处不一致并列出各自版本", () => {
    const errors = checkVersions(
      { package: "0.2.0", tauri: "0.2.0", cargo: "0.1.0" },
      "v0.2.0",
    );
    expect(errors).toHaveLength(1);
    expect(errors[0]).toContain("package.json=0.2.0");
    expect(errors[0]).toContain("tauri.conf.json=0.2.0");
    expect(errors[0]).toContain("Cargo.toml=0.1.0");
  });

  it("一处版本错（package.json 抢跑）+ tag 错 → 两类错误都报", () => {
    const errors = checkVersions(
      { package: "0.3.0", tauri: "0.2.0", cargo: "0.2.0" },
      "v0.2.1",
    );
    expect(errors).toHaveLength(2);
  });
});

describe("runCli", () => {
  it("全对 → 退出码 0，stdout 报通过", () => {
    const dir = makeRepo();
    try {
      const out = [];
      const code = runCli({
        argv: [dir, "v0.2.0"],
        env: {},
        stdout: (s) => out.push(s),
        stderr: (s) => out.push(s),
      });
      expect(code).toBe(0);
      expect(out.join("\n")).toContain("通过");
    } finally {
      cleanup(dir);
    }
  });

  it("Cargo.toml 落后 → 退出码非 0，stderr 有明确中文错误", () => {
    const dir = makeRepo({ cargo: "0.1.0" });
    try {
      const out = [];
      const code = runCli({
        argv: [dir, "v0.2.0"],
        env: {},
        stdout: (s) => out.push(s),
        stderr: (s) => out.push(s),
      });
      expect(code).not.toBe(0);
      expect(out.join("\n")).toContain("0.1.0");
    } finally {
      cleanup(dir);
    }
  });

  it("tag 与版本不一致 → 退出码非 0，错误提到 tag", () => {
    const dir = makeRepo();
    try {
      const out = [];
      const code = runCli({
        argv: [dir, "v0.9.9"],
        env: {},
        stdout: (s) => out.push(s),
        stderr: (s) => out.push(s),
      });
      expect(code).not.toBe(0);
      expect(out.join("\n")).toContain("v0.9.9");
    } finally {
      cleanup(dir);
    }
  });

  it("文件缺失 → 退出码非 0，错误指名缺失文件", () => {
    const dir = makeRepo({ omit: ["Cargo.toml"] });
    try {
      const out = [];
      const code = runCli({
        argv: [dir, "v0.2.0"],
        env: {},
        stdout: (s) => out.push(s),
        stderr: (s) => out.push(s),
      });
      expect(code).not.toBe(0);
      expect(out.join("\n")).toContain("Cargo.toml");
    } finally {
      cleanup(dir);
    }
  });

  it("参数可走环境变量（REPO_ROOT / EXPECTED_TAG）", () => {
    const dir = makeRepo();
    try {
      const out = [];
      const code = runCli({
        argv: [],
        env: { REPO_ROOT: dir, EXPECTED_TAG: "v0.2.0" },
        stdout: (s) => out.push(s),
        stderr: (s) => out.push(s),
      });
      expect(code).toBe(0);
      expect(out.join("\n")).toContain("通过");
    } finally {
      cleanup(dir);
    }
  });

  it("既无参数也无环境变量 → 退出码非 0 并提示用法", () => {
    const out = [];
    const code = runCli({
      argv: [],
      env: {},
      stdout: (s) => out.push(s),
      stderr: (s) => out.push(s),
    });
    expect(code).not.toBe(0);
    expect(out.join("\n")).toContain("用法");
  });
});
