#!/usr/bin/env node

/**
 * Desktop Organizer - Cross-platform build script
 *
 * Usage:
 *   node scripts/build.mjs                        # release build (default)
 *   node scripts/build.mjs --mode debug           # debug build
 *   node scripts/build.mjs --no-installer         # skip NSIS/MSI/DMG
 *   node scripts/build.mjs --no-frontend          # skip frontend build
 *   node scripts/build.mjs --target aarch64-apple-darwin
 *   node scripts/build.mjs --platform win         # Windows packaging (one-click)
 *   node scripts/build.mjs --platform mac         # macOS packaging (RESERVED, macOS host only)
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
const DIST_DIR = join(PROJECT_ROOT, "dist");
const IS_WINDOWS = platform() === "win32";
const IS_MACOS = platform() === "darwin";

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
  --no-installer          Skip installer generation (NSIS/MSI/DMG)
  --no-frontend           Skip frontend build (dev/debug only)
  --platform <win|mac>    Target platform (default: current host OS, one-click)
                          win: Windows host only; mac: RESERVED, macOS host only
  --bundles <list>        Tauri bundle targets, comma-separated (e.g. nsis / msi / nsis,msi).
                          Default: all configured targets. Useful to skip MSI where WiX download is blocked.
  --target <triple>       Cross-compilation target (e.g. aarch64-apple-darwin)
  -h, --help              Show this help
  `);
}

// ─── Platform Resolution ────────────────────────────────────
const PLATFORM_ALIASES = {
  win: "win", windows: "win", win32: "win",
  mac: "mac", macos: "mac", darwin: "mac",
  linux: "linux",
};

/**
 * Resolve --platform argument:
 * - defaults to the current host platform;
 * - win: Windows host only (no cross-compilation);
 * - mac: RESERVED, macOS host only, fails explicitly on other hosts;
 * - linux: Linux host only (dev convenience, not a release target).
 */
function resolvePlatform(opts) {
  const host = IS_WINDOWS ? "win" : IS_MACOS ? "mac" : "linux";
  if (!opts.platform) return host;

  const requested = PLATFORM_ALIASES[String(opts.platform).toLowerCase()];
  if (!requested) {
    logError(`Unknown platform: ${opts.platform} (valid: win | mac | linux)`);
    process.exit(1);
  }

  if (requested !== host) {
    if (requested === "mac") {
      logError("macOS packaging is RESERVED: cross-platform packaging is not supported yet, please run on a macOS host (not enabled)");
      process.exit(1);
    }
    logError(`Cannot package ${requested} on a ${host} host: cross-compilation is not supported, run on the matching platform`);
    process.exit(1);
  }

  if (requested === "mac") {
    log("Note: macOS packaging is RESERVED and has not been validated yet");
  }
  return requested;
}

// ─── Utility ────────────────────────────────────────────────────
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

  // npm
  try {
    const npmVersion = runCapture("npm --version");
    log(`  npm: ${npmVersion}`);
  } catch {
    logError("npm not found.");
    process.exit(1);
  }

  // Tauri CLI
  try {
    const tauriVersion = runCapture("npx --yes @tauri-apps/cli@2 --version");
    log(`  tauri-cli: ${tauriVersion}`);
  } catch {
    logError("@tauri-apps/cli not available. Install: npm install -g @tauri-apps/cli");
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
  run(`npm --prefix frontend install`);
}

// ─── Tauri Build ────────────────────────────────────────────────
function buildTauri(opts) {
  const parts = ["npx --yes @tauri-apps/cli@2 build"];

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
    log(`  Copy: ${basename(src)} -> dist/`);
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

function collectMacOSArtifacts(version, opts) {
  const targetDir = join(PROJECT_ROOT, "target", "release");
  const bundleDir = join(targetDir, "bundle");
  const osArch = opts.target || (arch() === "arm64" ? "aarch64" : "x64");
  const prefix = `ikun_organizer-v${version}-macos-${osArch}`;

  log("Collecting macOS artifacts...");
  ensureDir(DIST_DIR);

  // .app bundle
  const macosDir = join(bundleDir, "macos");
  const appDirs = findFiles(macosDir, /\.app$/);
  for (const appDir of appDirs) {
    const dest = join(DIST_DIR, `${prefix}.app`);
    copyDirectory(appDir, dest);
    log(`  Copy: ${basename(appDir)} -> dist/${prefix}.app`);
  }

  // DMG
  const dmgDir = join(bundleDir, "dmg");
  for (const f of findFiles(dmgDir, /\.dmg$/i)) {
    copyIf(f, join(DIST_DIR, `${prefix}.dmg`));
  }

  // Portable zip
  const mainBin = join(targetDir, "ikun_organizer");
  if (existsSync(mainBin)) {
    copyIf(mainBin, join(DIST_DIR, `${prefix}`));
    createPortableZip(
      join(DIST_DIR, `${prefix}`),
      join(DIST_DIR, `${prefix}.zip`),
      `${prefix}`
    );
  }
}

function copyDirectory(src, dest) {
  ensureDir(dest);
  for (const entry of readdirSync(src, { withFileTypes: true })) {
    const srcPath = join(src, entry.name);
    const destPath = join(dest, entry.name);
    if (entry.isDirectory()) {
      copyDirectory(srcPath, destPath);
    } else {
      copyFileSync(srcPath, destPath);
    }
  }
}

/**
 * Create a zip archive using Node.js built-in capabilities.
 * Uses the 'zlib' module for deflate compression and manual zip format construction.
 *
 * For simplicity and to avoid external dependencies, we use platform-native tools:
 * - Windows: PowerShell Compress-Archive
 * - macOS/Linux: zip command
 */
function createPortableZip(sourceFile, zipPath, entryName) {
  if (!existsSync(sourceFile)) {
    log(`  Skip zip: source not found (${basename(sourceFile)})`);
    return;
  }

  log(`  Creating zip: ${basename(zipPath)}`);
  const sourceDir = resolve(sourceFile, "..");

  if (IS_WINDOWS) {
    // PowerShell Compress-Archive
    const srcEsc = sourceFile.replace(/'/g, "''");
    const zipEsc = zipPath.replace(/'/g, "''");
    run(
      `powershell -NoProfile -Command "Compress-Archive -Force -LiteralPath '${srcEsc}' -DestinationPath '${zipEsc}'"`,
      { stdio: "pipe" }
    );
  } else {
    // Unix zip
    run(`cd "${sourceDir}" && zip -j "${zipPath}" "${basename(sourceFile)}"`, {
      stdio: "pipe",
    });
  }
}

// ─── Main ───────────────────────────────────────────────────────
async function main() {
  const startTime = Date.now();
  const opts = parseArgs();
  const targetPlatform = resolvePlatform(opts);

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

  if (targetPlatform === "win") {
    collectWindowsArtifacts(version);
  } else if (targetPlatform === "mac") {
    collectMacOSArtifacts(version, opts);
  } else {
    log(`Linux build - artifacts in target/release/`);
    ensureDir(DIST_DIR);
    const bin = join(PROJECT_ROOT, "target", "release", "ikun_organizer");
    if (existsSync(bin)) {
      const prefix = `ikun_organizer-v${version}-linux-x64`;
      copyIf(bin, join(DIST_DIR, prefix));
    }
  }

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
