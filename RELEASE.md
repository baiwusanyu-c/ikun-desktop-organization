# Release Process

How to publish a Desktop Organizer release (Windows-only product).

## Pipeline overview

```text
Local:  pnpm run release:beta:push          (scripts/release.mjs)
          bump version -> commit -> tag -> push
GitHub: tag push triggers Release workflow
          Job 1 test (fmt + clippy + cargo test)   <- quality gate
          Job 2 release-windows (needs: test)      <- package + publish
Result: version release with installers + signed updater setup.exe/.sig + latest.json;
          beta release mirrors latest.json
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

## Release notes / 报表

GitHub Release 正文由两部分拼成（见 `release-windows` job 的
*Build release notes body* 步骤）：

1. `CHANGELOG.md` 中与本次 tag 对应的 `## [vX.Y.Z]` 段落 —— **人工维护，发版前先写好**，
   这是发布报表的唯一来源；
2. 固定的 Beta notice + Downloads 表格（写在 workflow 里）。

因此发版顺序是：**先更新 `CHANGELOG.md` -> 再 `pnpm run release:beta:push`**。
CHANGELOG 缺该段落不会卡住发布，只会退化成第 2 部分；CI 日志里出现
`CHANGELOG.md 没有 vX.Y.Z 段落` 就说明报表漏写了。

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
3. Builds and signs Windows artifacts via `scripts/build.mjs --platform win`.
4. Generates `SHA256SUMS.txt`.
5. Publishes the GitHub Release with:
   `*-setup.exe` + `*-setup.exe.sig` (NSIS updater pair), `latest.json`,
   portable `*.zip`, checksums (plus `*.msi` for stable releases - MSI cannot
   encode pre-release tags).

Download page:
`https://github.com/baiwusanyu-c/ikun-desktop-organization/releases`

### Manual trigger (no tag push)

Actions -> **Release** -> *Run workflow* -> enter the version (e.g.
`0.1.0-beta.2`). The workflow creates the tag and the release. The version
must already be present in `Cargo.toml`.

## Online updater release (minisign)

The in-app updater uses a static manifest instead of the GitHub API:

```text
signed build (TAURI_SIGNING_PRIVATE_KEY)
  -> target/release/bundle/nsis/*-setup.exe + *-setup.exe.sig
  -> build.mjs copies setup.exe to ikun_organizer-v<version>-windows-x64-setup.exe
     and copies the signature beside it as ...-setup.exe.sig
  -> Generate latest.json (version / notes / pub_date / platform url)
  -> version release: installers + updater setup asset + signature + latest.json
  -> fixed `beta` release: latest.json only (rolling update channel)
App endpoint: https://github.com/baiwusanyu-c/ikun-desktop-organization/releases/download/beta/latest.json
```

`latest.json` points to the renamed setup.exe asset stored in `target/dist`.
The signature file is renamed alongside it; this is safe because minisign signs
the file bytes, not the path. The workflow validates that both files exist and
fails rather than publishing a broken manifest. The `platforms` key must remain
`windows-x86_64`, and `pub_date` is emitted as UTC RFC 3339 (a missing/invalid
date is treated by Tauri as 1970).

> **Artifact format:** this project deliberately keeps Tauri v2's native
> self-contained updater (`createUpdaterArtifacts: true`). Tauri signs
> `*-setup.exe` directly. We intentionally avoid `"v1Compatible"` because that
> would wrap setup.exe in a zip and add an unnecessary extraction step and
> failure mode.

### One-time signing key setup

1. Generate the minisign key pair:

   ```powershell
   pnpm dlx @tauri-apps/cli@2 signer generate -w ~/.tauri/ikun_updater.key
   ```

2. Put the generated **public key** into `tauri.conf.json` at
   `plugins.updater.pubkey`. Replacing the
   `REPLACE_WITH_MINISIGN_PUBLIC_KEY` placeholder is mandatory; the release
   workflow fails early while it is still present.

3. Add the private key and its password as GitHub Actions repository secrets:

   - `TAURI_SIGNING_PRIVATE_KEY` - the private key content
   - `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` - the password chosen during generation

### Private key backup and recovery

**Back up the private key offline** (for example on an encrypted USB drive or in
an offline password vault). A GitHub secret is a deployment copy, not a backup.

Existing installations trust the public key embedded in the version they
already installed. If the private key is lost, those users cannot receive any
future updater asset signed with a replacement key. Recovery requires signing
and publishing a new installer containing the new public key, then asking users
to download and reinstall it manually.

### minisign is not Authenticode

The updater signature protects the download and proves that it came from the
holder of the private key. It is **not** Authenticode code signing and does not
change Windows SmartScreen behavior; the warning described in
[Code signing (not configured)](#code-signing-not-configured) still applies.

## Local one-click packaging

```bash
pnpm run pack:win    # full release build into target/dist/
```

Full packaging signs the updater artifact because
`bundle.createUpdaterArtifacts` is enabled, so it requires
`TAURI_SIGNING_PRIVATE_KEY` (and the password secret when one is configured).
For an unsigned debug smoke build use `pnpm run pack:fast` (no installer).

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
| SmartScreen warning | Expected for unsigned builds (minisign updater signatures do not replace Authenticode) |
| `latest.json` returns 404 | The fixed `beta` release was not updated or has no manifest asset. Check the *Publish beta rolling channel* step, then inspect `gh release view beta` and re-upload `target/dist/latest.json`. |
| Updater fails signature verification | The `plugins.updater.pubkey` does not match the private key used for the build. Restore the matching key pair; if the old private key is lost, publish a new installer with the new public key for manual reinstall. |
| Installer reports `Failed to kill` / process in use | Another `ikun_organizer.exe` instance is still running, or the old process has not finished its cleanup. Close all instances, wait for `Cleanup complete`, and retry. Avoid force-killing because that skips the app's desktop-icon cleanup. |
