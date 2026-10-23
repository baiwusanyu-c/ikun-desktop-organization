# Build Scripts

Windows build/release scripts for Desktop Organizer.

## Prerequisites

- Windows 10/11 (the product is Windows-only; packaging requires a Windows host)
- [Rust](https://rustup.rs/) (pinned by `rust-toolchain.toml`)
- [Node.js](https://nodejs.org/) (v22.13+，推荐 24 LTS) + [pnpm](https://pnpm.io/) 11（`corepack enable` 启用仓库锁定版本）
- [@tauri-apps/cli](https://tauri.app/) v2 (auto-fetched via `pnpm dlx`, major version pinned)

## Usage

One-click packaging (recommended):

```bash
# From repo root
pnpm run pack:win         # Windows release build (one-click)
pnpm run pack:debug       # debug build with installer
pnpm run pack:fast          # debug build without installer (fast iteration)
```

Or call the script directly:

```bash
node scripts/build.mjs                    # release build
node scripts/build.mjs --platform win     # explicit Windows build (only supported target)
node scripts/build.mjs --mode debug       # debug build
node scripts/build.mjs --no-installer     # skip NSIS/MSI
node scripts/build.mjs --no-frontend      # skip frontend build
node scripts/build.mjs --bundles nsis     # only NSIS (skip MSI / WiX download)
```

## CLI Options

| Option | Default | Description |
|--------|---------|-------------|
| `--platform <win>` | win | Packaging target. Windows is the only supported platform (aliases: `--os`, `-p`) |
| `--mode <release\|debug>` | `release` | Build mode |
| `--no-installer` | off | Skip NSIS/MSI installer generation |
| `--no-frontend` | off | Skip frontend build and pnpm install |
| `--bundles <list>` | all | Tauri bundle targets, comma-separated (`nsis`, `msi`). Pre-release versions auto-select NSIS only because MSI (WiX) rejects non-numeric versions |
| `--target <triple>` | auto | Rust target triple (e.g. `x86_64-pc-windows-msvc`) |
| `-h, --help` | - | Show help |

## Output

Build artifacts are collected into `target/dist/` (git-ignored):

```text
target/dist/
├── ikun_organizer-v{version}-windows-x64.exe          # Main executable
├── ikun_organizer-v{version}-windows-x64-setup.exe    # NSIS installer
├── ikun_organizer-v{version}-windows-x64.msi          # MSI installer (stable releases only)
└── ikun_organizer-v{version}-windows-x64.zip          # Portable archive
```

## CI/CD

| Workflow | Trigger | Purpose |
|----------|---------|---------|
| `.github/workflows/build.yml` (Windows CI) | push/PR to `master` | fmt/clippy/test + frontend check, build and upload Windows artifacts |
| `.github/workflows/release.yml` (Release) | `v*` tag push or manual dispatch | Quality gate, build Windows installers, SHA256, publish GitHub Release |

To cut a release, see [../RELEASE.md](../RELEASE.md).
