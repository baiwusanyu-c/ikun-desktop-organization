#!/usr/bin/env node

/**
 * Desktop Organizer - Windows build script
 *
 * Usage:
 *   node scripts/build.mjs                        # release build (default)
 *   node scripts/build.mjs --mode debug           # debug build
 *   node scripts/build.mjs --no-installer         # skip NSIS/MSI
 *   node scripts/build.mjs --no-frontend          # skip frontend build
 *   node scripts/build.mjs --platform win         # Windows packaging (one-click)
 *   node scripts/build.mjs --bundles nsis         # NSIS installer only (skip MSI)
 */

import { execSync, spawnSync } from "node:child_process";
import { existsSync, readFileSync, mkdirSync, copyFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { join, resolve, basename, extname } from "node:path";
import { platform, arch } from "node:os";
import { createWriteStream } from "node:fs";
import { pipeline } from "node:stream/promises";
import { createReadStream } from "node:fs";
import { createHash } from "node:crypto";

// ─── Configuration ──────────────────────────────────────────────
const PROJECT_ROOT = resolve(import.meta.dirname, "..");
const FRONTEND_DIR = join(PROJECT_ROOT, "frontend");
const DIST_DIR = join(PROJECT_ROOT, "target", "dist");
const IS_WINDOWS = platform() === "win32";

// Tauri CLI：通过 pnpm dlx 按大版本固定临时获取，避免依赖全局安装
const TAURI_CLI = "pnpm dlx --package @tauri-apps/cli@2 tauri";

// ─── CLI Argument Parsing ───────────────────────────────────────
function parseArgs() {
  const args = process.argv.slice(2);
  const opts = { mode: "release", noInstaller: false, noFrontend: false, target: null, platform: null, bundles: null };
  for (let i = 0; i < args.length; i++) {
    switch (args[i]) {
      case "--mode":
        opts.mode = args[++i] || "release";
        break;
      case "--no-installer":
        opts.noInstaller = true;
        break;
      case "--no-frontend":
        opts.noFrontend = true;
        break;
      case "--target":
        opts.target = args[++i] || null;
        break;
      case "--platform":
      case "--os":
      case "-p":
        opts.platform = args[++i] || null;
        break;
      case "--bundles":
        opts.bundles = args[++i] || null;
        break;
      case "--help":
      case "-h":
        printHelp();
        process.exit(0);
    }
  }
  return opts;
}

function printHelp() {
  console.log(`
Desktop Organizer Build Script

Usage: node scripts/build.mjs [options]

Options:
  --mode <release|debug>  Build mode (default: release)
  --no-installer          Skip installer generation (NSIS/MSI)
  --no-frontend           Skip frontend build (dev/debug only)
  --platform <win>        Packaging target; Windows is the only supported platform
  --bundles <list>        Tauri bundle targets, comma-separated (e.g. nsis / msi / nsis,msi).
                          Default: all configured targets. Useful to skip MSI where WiX download is blocked.
  --target <triple>       Rust target triple (e.g. x86_64-pc-windows-msvc)
  -h, --help              Show this help
  `);
}

// ─── Platform Resolution ────────────────────────────────────

/**
 * Resolve --platform argument. Desktop Organizer is a Windows-only product:
 * packaging must run on a Windows host and only "win" is a valid target.
 */
function resolvePlatform(opts) {
  if (!IS_WINDOWS) {
    logError("Desktop Organizer is Windows-only; packaging must run on a Windows host");
    process.exit(1);
  }
  if (!opts.platform) return "win";

  const requested = PLATFORM_ALIASES[String(opts.platform).toLowerCase()];
  if (requested !== "win") {
    logError(`Unknown platform: ${opts.platform} (only "win" is supported)`);
    process.exit(1);
  }
  return requested;
}
const PLATFORM_ALIASES = {
  win: "win", windows: "win", win32: "win",
};

function log(msg) {
  const ts = new Date().toISOString().slice(11, 19);
  console.log(`[${ts}] ${msg}`);
}

function logError(msg) {
  const ts = new Date().toISOString().slice(11, 19);
  console.error(`[${ts}] ERROR: ${msg}`);
}

function run(cmd, opts = {}) {
  log(`> ${cmd}`);
  const result = spawnSync(cmd, [], {
    shell: true,
    stdio: "inherit",
    cwd: PROJECT_ROOT,
    ...opts,
  });
  if (result.status !== 0) {
    throw new Error(`Command failed (exit ${result.status}): ${cmd}`);
  }
  return result;
}

function runCapture(cmd, opts = {}) {
  const result = spawnSync(cmd, [], {
    shell: true,
    encoding: "utf-8",
    cwd: PROJECT_ROOT,
    ...opts,
  });
  if (result.status !== 0) {
    throw new Error(`Command failed (exit ${result.status}): ${cmd}`);
  }
  return result.stdout.trim();
}

// ─── Environment Check ──────────────────────────────────────────
function checkEnv() {
  log("Checking build environment...");

  // Rust
  try {
    const rustVersion = runCapture("rustc --version");
    log(`  rustc: ${rustVersion}`);
  } catch {
    logError("rustc not found. Install Rust: https://rustup.rs");
    process.exit(1);
  }

  // Node.js
  try {
    const nodeVersion = runCapture("node --version");
    log(`  node: ${nodeVersion}`);
  } catch {
    logError("node not found.");
    process.exit(1);
  }

  // pnpm
  try {
    const pnpmVersion = runCapture("pnpm --version");
    log(`  pnpm: ${pnpmVersion}`);
  } catch {
    logError("pnpm not found. Install: corepack enable || https://pnpm.io/installation");
    process.exit(1);
  }

  // Tauri CLI (auto-fetched via pnpm dlx, major version pinned)
  try {
    const tauriVersion = runCapture(`${TAURI_CLI} --version`);
    log(`  tauri-cli: ${tauriVersion}`);
  } catch {
    logError("@tauri-apps/cli not available via pnpm dlx (check network).");
    process.exit(1);
  }

  log("Environment check passed!");
}

// ─── Version ────────────────────────────────────────────────────
function getVersion() {
  const cargoPath = join(PROJECT_ROOT, "Cargo.toml");
  const content = readFileSync(cargoPath, "utf-8");
  const match = content.match(/^version\s*=\s*"([^"]+)"/m);
  if (!match) {
    logError("Cannot parse version from Cargo.toml");
    process.exit(1);
  }
  return match[1];
}

// ─── Frontend ───────────────────────────────────────────────────
function installFrontend() {
  log("Installing frontend dependencies...");
  run(`pnpm -C frontend install`);
}

// ─── Tauri Build ────────────────────────────────────────────────
function buildTauri(opts) {
  const parts = [`${TAURI_CLI} build`];

  if (opts.mode === "debug") {
    parts.push("--debug");
  }

  if (opts.noInstaller) {
    parts.push("--no-bundle");
  }

  if (opts.target) {
    parts.push(`--target ${opts.target}`);
  }

  if (opts.bundles) {
    parts.push(`--bundles ${opts.bundles}`);
  } else if (getVersion().includes("-")) {
    // MSI (WiX) requires fully numeric versions; pre-release identifiers such
    // as "beta.2" are rejected. NSIS has no such restriction, so pre-release
    // builds ship NSIS only; stable releases get NSIS + MSI automatically.
    parts.push("--bundles nsis");
    log("Pre-release version detected: building NSIS only (MSI requires a numeric version)");
  }

  // If skipping frontend, set empty beforeBuildCommand
  if (opts.noFrontend) {
    log("Skipping frontend build (--no-frontend)");
    // Tauri requires frontendDist to exist; we ensure it's there
    const distDir = join(FRONTEND_DIR, "dist");
    if (!existsSync(distDir)) {
      mkdirSync(distDir, { recursive: true });
      writeFileSync(join(distDir, "index.html"), "<!-- placeholder -->");
    }
  }

  const cmd = parts.join(" ");
  log(`Building with: ${cmd}`);
  run(cmd);
}

// ─── Artifact Collection ───────────────────────────────────────
function ensureDir(dir) {
  if (!existsSync(dir)) {
    mkdirSync(dir, { recursive: true });
  }
}

function copyIf(src, dest) {
  if (existsSync(src)) {
    log(`  Copy: ${basename(src)} -> target/dist/`);
    copyFileSync(src, dest);
    return true;
  }
  return false;
}

function findFiles(dir, pattern) {
  if (!existsSync(dir)) return [];
  return readdirSync(dir)
    .filter((f) => pattern.test(f))
    .map((f) => join(dir, f));
}

function collectWindowsArtifacts(version) {
  const targetDir = join(PROJECT_ROOT, "target", "release");
  const bundleDir = join(targetDir, "bundle");
  const prefix = `ikun_organizer-v${version}-windows-x64`;

  log("Collecting Windows artifacts...");
  ensureDir(DIST_DIR);

  // Main executable
  const exe = join(targetDir, "ikun_organizer.exe");
  copyIf(exe, join(DIST_DIR, `${prefix}.exe`));

  // NSIS installer
  const nsisDir = join(bundleDir, "nsis");
  for (const f of findFiles(nsisDir, /setup\.exe$/i)) {
    copyIf(f, join(DIST_DIR, `${prefix}-setup.exe`));
  }

  // MSI installer
  const msiDir = join(bundleDir, "msi");
  for (const f of findFiles(msiDir, /\.msi$/i)) {
    copyIf(f, join(DIST_DIR, `${prefix}.msi`));
  }

  // Portable zip
  createPortableZip(
    join(DIST_DIR, `${prefix}.exe`),
    join(DIST_DIR, `${prefix}.zip`),
    `${prefix}.exe`
  );
}

/**
 * Create a zip archive using Node.js built-in capabilities.
 * Uses the 'zlib' module for deflate compression and manual zip format construction.
 *
 * Packaging runs on Windows hosts only, so the archive is created with
 * PowerShell Compress-Archive.
 */
function createPortableZip(sourceFile, zipPath, entryName) {
  if (!existsSync(sourceFile)) {
    log(`  Skip zip: source not found (${basename(sourceFile)})`);
    return;
  }

  log(`  Creating zip: ${basename(zipPath)}`);
  // PowerShell Compress-Archive
  const srcEsc = sourceFile.replace(/'/g, "''");
  const zipEsc = zipPath.replace(/'/g, "''");
  run(
    `powershell -NoProfile -Command "Compress-Archive -Force -LiteralPath '${srcEsc}' -DestinationPath '${zipEsc}'"`,
    { stdio: "pipe" }
  );
}

// ─── Main ───────────────────────────────────────────────────────
async function main() {
  const startTime = Date.now();
  const opts = parseArgs();
  const targetPlatform = resolvePlatform(opts);
  opts.resolvedPlatform = targetPlatform;

  log("=".repeat(60));
  log("Desktop Organizer Build");
  log("=".repeat(60));
  log(`Platform: ${platform()} (${arch()})`);
  log(`Target platform: ${targetPlatform}`);
  log(`Mode: ${opts.mode}`);
  log(`Installer: ${opts.noInstaller ? "skip" : "yes"}`);
  log(`Frontend: ${opts.noFrontend ? "skip" : "yes"}`);
  if (opts.target) log(`Target: ${opts.target}`);
  log("");

  // Step 1: Environment check
  checkEnv();
  log("");

  // Step 2: Install frontend dependencies
  if (!opts.noFrontend) {
    installFrontend();
    log("");
  }

  // Step 3: Build
  buildTauri(opts);
  log("");

  // Step 4: Collect artifacts
  const version = getVersion();
  log(`Version: ${version}`);

  collectWindowsArtifacts(version);

  // Summary
  const elapsed = ((Date.now() - startTime) / 1000).toFixed(1);
  log("");
  log("=".repeat(60));
  log(`Build complete in ${elapsed}s`);
  log(`Artifacts: ${DIST_DIR}`);

  if (existsSync(DIST_DIR)) {
    for (const f of readdirSync(DIST_DIR).sort()) {
      const size = statSync(join(DIST_DIR, f)).size;
      const sizeStr =
        size > 1024 * 1024
          ? `${(size / 1024 / 1024).toFixed(1)} MB`
          : `${(size / 1024).toFixed(1)} KB`;
      log(`  ${f} (${sizeStr})`);
    }
  }
  log("=".repeat(60));
}

main().catch((err) => {
  logError(err.message);
  process.exit(1);
});
