#!/usr/bin/env node

/**
 * Release helper: bump version, commit, tag (and optionally push).
 *
 * Usage:
 *   node scripts/release.mjs --bump beta              # 0.1.0-beta.1 -> 0.1.0-beta.2
 *   node scripts/release.mjs --bump patch             # 0.1.0-beta.2 -> 0.1.0
 *   node scripts/release.mjs --bump minor             # 0.1.0 -> 0.2.0
 *   node scripts/release.mjs --bump major             # 0.2.0 -> 1.0.0
 *   node scripts/release.mjs --set 0.3.0-beta.1       # explicit version
 *   Add --push to also push branch + tag to origin.
 *   Add --dry-run to preview without writing anything.
 *   Add --no-commit to only update version files (no commit, no tag).
 */

import { readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { spawnSync } from "node:child_process";

const PROJECT_ROOT = resolve(import.meta.dirname, "..");

function log(msg) {
  console.log(`[release] ${msg}`);
}
function fail(msg) {
  console.error(`[release] ERROR: ${msg}`);
  process.exit(1);
}
function run(cmd, args, opts = {}) {
  // No shell:true - args (e.g. commit messages with spaces/parentheses)
  // must reach the child process unmodified.
  const result = spawnSync(cmd, args, {
    cwd: PROJECT_ROOT,
    encoding: "utf-8",
    stdio: opts.capture ? "pipe" : "inherit",
  });
  if (result.status !== 0 && !opts.allowFail) {
    fail(`Command failed: ${cmd} ${args.join(" ")}`);
  }
  return (result.stdout || "").trim();
}

// ---- CLI parsing -----------------------------------------------------------
const argv = process.argv.slice(2);
const opts = { bump: null, set: null, push: false, dryRun: false, commit: true };
for (let i = 0; i < argv.length; i++) {
  switch (argv[i]) {
    case "--bump": opts.bump = argv[++i]; break;
    case "--set": opts.set = argv[++i]; break;
    case "--push": opts.push = true; break;
    case "--dry-run": opts.dryRun = true; break;
    case "--no-commit": opts.commit = false; break;
    case "-h":
    case "--help":
      console.log(readFileSync(import.meta.filename, "utf-8").split("*/")[0]);
      process.exit(0);
    default:
      fail(`Unknown option: ${argv[i]} (see --help)`);
  }
}
if (!opts.bump && !opts.set) fail("Specify --bump <beta|patch|minor|major> or --set <version>");
if (opts.bump && opts.set) fail("Use either --bump or --set, not both");

// ---- Version computation ---------------------------------------------------
function readCargoVersion() {
  const content = readFileSync(join(PROJECT_ROOT, "Cargo.toml"), "utf-8");
  const m = content.match(/^version\s*=\s*"([^"]+)"/m);
  if (!m) fail("Cannot read version from Cargo.toml");
  return m[1];
}

function parseVersion(v) {
  const m = v.match(/^(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?$/);
  if (!m) fail(`Invalid semver: ${v}`);
  return { major: +m[1], minor: +m[2], patch: +m[3], pre: m[4] || null };
}

function nextVersion(current, bump) {
  const v = parseVersion(current);
  switch (bump) {
    case "beta": {
      if (v.pre && /^beta\.(\d+)$/.test(v.pre)) {
        const n = +v.pre.match(/^beta\.(\d+)$/)[1];
        return `${v.major}.${v.minor}.${v.patch}-beta.${n + 1}`;
      }
      return `${v.major}.${v.minor}.${v.patch}-beta.1`;
    }
    case "patch":
      return v.pre
        ? `${v.major}.${v.minor}.${v.patch}` // pre-release -> stable of same triple
        : `${v.major}.${v.minor}.${v.patch + 1}`;
    case "minor":
      return `${v.major}.${v.minor + 1}.0`;
    case "major":
      return `${v.major + 1}.0.0`;
    default:
      fail(`Unknown bump level: ${bump} (valid: beta|patch|minor|major)`);
  }
}

const oldVersion = readCargoVersion();
const newVersion = opts.set ? opts.set : nextVersion(oldVersion, opts.bump);
parseVersion(newVersion); // validate
if (newVersion === oldVersion) fail(`Version is already ${oldVersion}, nothing to do`);

log(`Version: ${oldVersion} -> ${newVersion}`);

// ---- File updates ----------------------------------------------------------
// 两阶段：先把所有替换算完并逐个校验命中，全部成功后才统一写盘。
// 旧实现边算边写，Cargo.lock 一旦失配就 fail 退出，前面三个文件已经被改写，
// 留下「版本号改了但没提交」的半改状态（2026-11-27 发 v0.1.0-beta.9 时实发踩中）。
function eolOf(content) {
  return content.includes("\r\n") ? "\r\n" : "\n";
}

const pending = [];
function planReplace(relPath, replacer) {
  const fp = join(PROJECT_ROOT, relPath);
  const before = readFileSync(fp, "utf-8");
  const after = replacer(before);
  if (after === before) {
    fail(
      `Version string not found in ${relPath} (expected ${oldVersion} -> ${newVersion}). ` +
        `No files were written; the working tree is untouched.`
    );
  }
  pending.push({ fp, relPath, after });
}

planReplace("Cargo.toml", (c) =>
  c.replace(`version = "${oldVersion}"`, `version = "${newVersion}"`)
);
planReplace("tauri.conf.json", (c) =>
  c.replace(`"version": "${oldVersion}"`, `"version": "${newVersion}"`)
);
planReplace("package.json", (c) =>
  c.replace(`"version": "${oldVersion}"`, `"version": "${newVersion}"`)
);
// Cargo.lock 的换行符跟随文件实际风格：cargo 在 Windows 上可能写出 CRLF，
// 硬编码 \n 会静默失配，正是上面那个半改状态的触发点。
planReplace("Cargo.lock", (c) => {
  const eol = eolOf(c);
  return c.replace(
    `name = "ikun_organizer"${eol}version = "${oldVersion}"`,
    `name = "ikun_organizer"${eol}version = "${newVersion}"`
  );
});

if (opts.dryRun) {
  log("Dry run: no files written, no commit/tag.");
  process.exit(0);
}

for (const { fp, relPath, after } of pending) {
  writeFileSync(fp, after, "utf-8");
  log(`  update ${relPath}`);
}

// ---- Commit + tag ----------------------------------------------------------
const tag = `v${newVersion}`;
const files = ["Cargo.toml", "Cargo.lock", "tauri.conf.json", "package.json"];

if (!opts.commit) {
  log(`--no-commit: version files updated only. Commit/tag manually when ready.`);
  process.exit(0);
}

const dirty = run("git", ["status", "--porcelain"], { capture: true });
const unexpected = dirty
  .split("\n")
  .filter((l) => l.trim())
  .filter((l) => !files.some((f) => l.endsWith(f)));
if (unexpected.length > 0) {
  log(`WARNING: other uncommitted changes exist (left untouched):`);
  unexpected.slice(0, 8).forEach((l) => log(`  ${l}`));
}

run("git", ["add", ...files]);
run("git", ["commit", "-m", `chore(release): ${tag}`]);
run("git", ["tag", "-a", tag, "-m", `Release ${tag}`]);
log(`Committed and tagged ${tag}`);

if (opts.push) {
  const branch = run("git", ["branch", "--show-current"], { capture: true });
  run("git", ["push", "origin", branch]);
  run("git", ["push", "origin", tag]);
  log(`Pushed ${branch} and ${tag} to origin.`);
  const remote = run("git", ["remote", "get-url", "origin"], { capture: true, allowFail: true });
  const repoMatch = remote.match(/github\.com[:/]([^/]+\/[^/.]+)/);
  if (repoMatch) {
    log(`GitHub Release workflow: https://github.com/${repoMatch[1]}/actions`);
  }
} else {
  log(`Not pushed. When ready: git push origin $(git branch --show-current) && git push origin ${tag}`);
}
