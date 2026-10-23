# Release Process

How to publish a Desktop Organizer release (Windows-only product).

## Pipeline overview

```text
Local:  pnpm run release:beta:push          (scripts/release.mjs)
          bump version -> commit -> tag -> push
GitHub: tag push triggers Release workflow
          Job 1 test (fmt + clippy + cargo test)   <- quality gate
          Job 2 release-windows (needs: test)      <- package + publish
Result: GitHub Releases page with installers + SHA256SUMS.txt
```

## Versioning

- Version lives in **four places** that must stay in sync (release.mjs
  updates all of them):
  - `[package] version` in `Cargo.toml`
  - `"version"` in `tauri.conf.json`
  - `package.json` (root, build tooling)
  - `Cargo.lock` (package entry)
- Use [semantic versioning](https://semver.org/). Pre-releases carry a suffix,
  e.g. `0.1.0-beta.1`. The release workflow marks any version containing `-`
  as a GitHub **pre-release** automatically.

## Cutting a beta release

One command (bumps beta number, commits, tags, pushes):

```bash
pnpm run release:beta:push
# equivalent to: node scripts/release.mjs --bump beta --push
```

Options of `scripts/release.mjs`:

| Option | Effect |
|--------|--------|
| `--bump beta` | `0.1.0-beta.1 -> 0.1.0-beta.2` (or `x.y.z -> x.y.z-beta.1`) |
| `--bump patch/minor/major` | stable bump (`-beta.N` suffix dropped) |
| `--set <version>` | explicit version |
| `--dry-run` | preview only |
| `--no-commit` | only update version files |
| `--push` | push branch + tag to origin |

After the push, the **Release** workflow:
1. **Test gate**: `cargo fmt --check`, clippy, `cargo test`. Any failure
   aborts the release - nothing gets packaged or published.
2. Verifies the tag matches the `Cargo.toml` version.
3. Builds Windows artifacts via `scripts/build.mjs --platform win`.
4. Generates `SHA256SUMS.txt`.
5. Publishes the GitHub Release with:
   `*-setup.exe` (NSIS), portable `*.zip`, checksums
   (plus `*.msi` for stable releases - MSI cannot encode pre-release tags).

Download page:
`https://github.com/baiwusanyu-c/ikun-desktop-organization/releases`

### Manual trigger (no tag push)

Actions -> **Release** -> *Run workflow* -> enter the version (e.g.
`0.1.0-beta.2`). The workflow creates the tag and the release. The version
must already be present in `Cargo.toml`.

## Local one-click packaging

```bash
pnpm run pack:win    # full release build into target/dist/
```

The same script runs in CI; local and CI artifacts use the identical
pipeline (`scripts/build.mjs --platform win`). See
[scripts/README.md](scripts/README.md) for all options.

## Quality gate details

The test job enforces:

- `cargo fmt -- --check` - formatting must be clean.
- `cargo clippy --all-targets -- -D warnings` with an explicit **lint-debt
  allowlist** for the native platform layer (dead code / duplicated FFI
  declarations pending consolidation, style lints). New warnings outside
  the allowlist fail CI. Shrink the list when touching those modules instead
  of extending it.
- `cargo test` - full unit test suite.
- `pnpm -C frontend run check` - svelte-check + tsc.

## Code signing (not configured)

Beta builds are **unsigned**. Windows SmartScreen will warn on first run.
When a certificate is available, add signing env/steps to the release job
before the build step.

## Troubleshooting

| Symptom | Fix |
|---------|-----|
| Release workflow fails at version check | Tag and `Cargo.toml` version differ - bump version first |
| Release workflow fails at test gate | Fix fmt/clippy/test locally, push the fix, delete + re-push the tag |
| Build fails with `os error 32` (file in use) | A running `ikun_organizer.exe` or stale cargo process holds a lock - close the app and retry |
| Pre-release has no `.msi` | Expected: MSI (WiX) only accepts numeric versions. `build.mjs` auto-selects NSIS-only for pre-releases; stable releases get NSIS + MSI automatically |
| SmartScreen warning | Expected for unsigned builds |
