#!/usr/bin/env node
// 票 03：版本一致性校验（发布 job 第一步，失败即中止）。
//
// 校验两条规则：
//   1. tag（去掉 v 前缀）== src-tauri/tauri.conf.json 的 version
//   2. package.json / src-tauri/tauri.conf.json / src-tauri/Cargo.toml 三处版本号互相一致
//
// 用法：
//   node scripts/check-versions.mjs <仓库路径> <tag>
//   或走环境变量：REPO_ROOT=<仓库路径> EXPECTED_TAG=<tag> node scripts/check-versions.mjs
//
// 校验通过：打印确认信息，退出码 0；任何不一致：打印明确中文错误，退出码 1。
// 校验逻辑拆成可单测的纯函数（tests: scripts/check-versions.test.mjs）。

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

const REL_PACKAGE_JSON = "package.json";
const REL_TAURI_CONF = join("src-tauri", "tauri.conf.json");
const REL_CARGO_TOML = join("src-tauri", "Cargo.toml");

/** tag 去掉开头一个 v 前缀（v0.2.0 → 0.2.0；0.2.0 原样） */
export function stripVPrefix(tag) {
  return tag.startsWith("v") ? tag.slice(1) : tag;
}

/**
 * 从 Cargo.toml 文本提取 [package] 段的 version。
 * 只认 [package] 段内的 `version = "x.y.z"`，不被 [dependencies] 等其他段干扰。
 * 找不到返回 null。
 */
export function extractCargoPackageVersion(tomlText) {
  let inPackage = false;
  for (const rawLine of tomlText.split(/\r?\n/)) {
    const line = rawLine.trim();
    if (line.startsWith("[")) {
      inPackage = line === "[package]";
      continue;
    }
    if (!inPackage || line === "" || line.startsWith("#")) continue;
    const m = line.match(/^version\s*=\s*"([^"]+)"/);
    if (m) return m[1];
  }
  return null;
}

/**
 * 读仓库三处版本号，返回 { package, tauri, cargo }。
 * 文件缺失 / JSON 非法 / 字段缺失时抛出带明确中文说明的错误。
 */
export function collectVersions(repoRoot) {
  let packageVersion;
  try {
    const pkg = JSON.parse(readFileSync(join(repoRoot, REL_PACKAGE_JSON), "utf8"));
    packageVersion = pkg?.version;
  } catch (err) {
    throw new Error(`无法读取 ${REL_PACKAGE_JSON}：${err.message}`);
  }
  if (typeof packageVersion !== "string" || packageVersion === "") {
    throw new Error(`${REL_PACKAGE_JSON} 缺少 version 字段`);
  }

  let tauriVersion;
  try {
    const conf = JSON.parse(readFileSync(join(repoRoot, REL_TAURI_CONF), "utf8"));
    tauriVersion = conf?.version;
  } catch (err) {
    throw new Error(`无法读取 ${REL_TAURI_CONF}：${err.message}`);
  }
  if (typeof tauriVersion !== "string" || tauriVersion === "") {
    throw new Error(`${REL_TAURI_CONF} 缺少 version 字段`);
  }

  let cargoVersion;
  try {
    cargoVersion = extractCargoPackageVersion(
      readFileSync(join(repoRoot, REL_CARGO_TOML), "utf8"),
    );
  } catch (err) {
    throw new Error(`无法读取 ${REL_CARGO_TOML}：${err.message}`);
  }
  if (cargoVersion === null) {
    throw new Error(`${REL_CARGO_TOML} 的 [package] 段缺少 version 字段`);
  }

  return { package: packageVersion, tauri: tauriVersion, cargo: cargoVersion };
}

/**
 * 纯校验：返回中文错误列表（空数组 = 通过）。
 * 规则 ①tag == tauri.conf.json version；②三处互相一致。两类错误都收集，一次报全。
 */
export function checkVersions(versions, tag) {
  const { package: pkg, tauri, cargo } = versions;
  const errors = [];

  const distinct =
    pkg === tauri && tauri === cargo
      ? []
      : [
          `三处版本号不一致：package.json=${pkg}、tauri.conf.json=${tauri}、src-tauri/Cargo.toml=${cargo}（发版前需手工改成一致）`,
        ];

  const tagVersion = stripVPrefix(tag);
  if (tauri !== tagVersion) {
    errors.push(
      `tag 与版本不一致：tag "${tag}"（去掉 v 前缀为 "${tagVersion}"）≠ src-tauri/tauri.conf.json 的 version "${tauri}"`,
    );
  }

  return [...distinct, ...errors];
}

/**
 * CLI 入口（依赖注入便于单测）。返回退出码：0 通过，1 失败。
 * 参数优先，其次环境变量 REPO_ROOT / EXPECTED_TAG。
 */
export function runCli({ argv = process.argv.slice(2), env = process.env, stdout = console.log, stderr = console.error } = {}) {
  const repoRoot = argv[0] ?? env.REPO_ROOT;
  const tag = argv[1] ?? env.EXPECTED_TAG;

  if (!repoRoot || !tag) {
    stderr("用法：node scripts/check-versions.mjs <仓库路径> <tag>（或设置环境变量 REPO_ROOT / EXPECTED_TAG）");
    return 1;
  }

  let versions;
  try {
    versions = collectVersions(repoRoot);
  } catch (err) {
    stderr(`版本校验失败：${err.message}`);
    return 1;
  }

  const errors = checkVersions(versions, tag);
  if (errors.length > 0) {
    for (const message of errors) stderr(`版本校验失败：${message}`);
    return 1;
  }

  stdout(`版本校验通过：tag ${tag} 与三处版本号一致（${versions.tauri}）`);
  return 0;
}

// 直接执行时才走 CLI（被 vitest import 时不触发）
const invokedAsMain =
  process.argv[1] !== undefined &&
  import.meta.url === pathToFileURL(process.argv[1]).href;
if (invokedAsMain) {
  process.exitCode = runCli({});
}
