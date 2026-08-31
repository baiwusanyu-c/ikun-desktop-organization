# Build Scripts

Cross-platform build scripts for Desktop Organizer.

## Prerequisites

- [Rust](https://rustup.rs/) (stable toolchain)
- [Node.js](https://nodejs.org/) (v18+)
- [@tauri-apps/cli](https://tauri.app/) v2 (auto-fetched via npx, major version pinned)

Note: the default build uses the `tauri-ui` feature only. The legacy
`gpui-ui` feature additionally requires the Windows SDK (`fxc.exe`).

## Usage

One-click packaging (recommended):

```bash
# From repo root
npm run pack:win          # Windows release build (one-click)
npm run pack:mac          # macOS (RESERVED - not implemented yet)
npm run pack:debug        # debug build on current host
npm run pack:fast         # debug build without installer (fast iteration)
```

Or call the script directly:

```bash
node scripts/build.mjs                            # release build for current host
node scripts/build.mjs --platform win             # explicit Windows build
node scripts/build.mjs --platform mac             # macOS build (RESERVED, macOS host only)
node scripts/build.mjs --mode debug               # debug build
node scripts/build.mjs --no-installer             # skip NSIS/MSI/DMG
node scripts/build.mjs --no-frontend              # skip frontend build
node scripts/build.mjs --target aarch64-apple-darwin
```

## CLI Options

| Option | Default | Description |
|--------|---------|-------------|
| `--platform <win\|mac\|linux>` | current host | Target platform. `win`: Windows host only. `mac`: **reserved**, macOS host only. Cross-compilation is not supported. Aliases: `--os`, `-p` |
| `--mode <release\|debug>` | `release` | Build mode |
| `--no-installer` | off | Skip NSIS/MSI (Win) or DMG (macOS) |
| `--no-frontend` | off | Skip frontend build and npm install |
| `--target <triple>` | auto | Cross-compilation target (macOS arch only) |
| `-h, --help` | - | Show help |

## Output

Build artifacts are collected into `dist/` (git-ignored):

### Windows

```
dist/
├── ikun_organizer-v{version}-windows-x64.exe          # Main executable
├── ikun_organizer-v{version}-windows-x64-setup.exe    # NSIS installer
├── ikun_organizer-v{version}-windows-x64.msi          # MSI installer
└── ikun_organizer-v{version}-windows-x64.zip          # Portable archive
```

### macOS (reserved)

```
dist/
├── ikun_organizer-v{version}-macos-{arch}.app         # App bundle
├── ikun_organizer-v{version}-macos-{arch}.dmg         # DMG installer
└── ikun_organizer-v{version}-macos-{arch}.zip         # Portable archive
```

### Linux

```
dist/
└── ikun_organizer-v{version}-linux-x64                # Main executable
```

## CI/CD

| Workflow | Trigger | Purpose |
|----------|---------|---------|
| `.github/workflows/build.yml` | push/PR to `master` | Build Windows (+ macOS reserved), fmt/clippy/test, upload artifacts |
| `.github/workflows/release.yml` | `v*` tag push or manual dispatch | Build Windows installers, SHA256, publish **GitHub Release** |

To cut a release, see [../docs/RELEASE.md](../RELEASE.md).
