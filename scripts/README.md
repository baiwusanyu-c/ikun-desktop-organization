# Build Scripts

Cross-platform build scripts for Desktop Organizer.

## Prerequisites

- [Rust](https://rustup.rs/) (stable toolchain)
- [Node.js](https://nodejs.org/) (v18+)
- [@tauri-apps/cli](https://tauri.app/) (auto-installed via npx)

## Usage

```bash
# Release build (default) - generates exe + installers
node scripts/build.mjs

# Debug build
node scripts/build.mjs --mode debug

# Skip installer generation (faster, dev only)
node scripts/build.mjs --no-installer

# Skip frontend build (for rapid Rust iteration)
node scripts/build.mjs --no-frontend

# Cross-compilation (e.g., Apple Silicon on Intel Mac)
node scripts/build.mjs --target aarch64-apple-darwin
```

## CLI Options

| Option | Default | Description |
|--------|---------|-------------|
| `--mode <release\|debug>` | `release` | Build mode |
| `--no-installer` | off | Skip NSIS/MSI (Win) or DMG (macOS) |
| `--no-frontend` | off | Skip frontend build and npm install |
| `--target <triple>` | auto | Cross-compilation target |
| `-h, --help` | - | Show help |

## Output

Build artifacts are collected into `dist/`:

### Windows

```
dist/
├── ikun_organizer-v{version}-windows-x64.exe          # Main executable
├── ikun_organizer-v{version}-windows-x64-setup.exe    # NSIS installer
├── ikun_organizer-v{version}-windows-x64.msi          # MSI installer
└── ikun_organizer-v{version}-windows-x64.zip          # Portable archive
```

### macOS

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

## CI Integration

The `.github/workflows/build.yml` workflow uses this script for both platforms.
