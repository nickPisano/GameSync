# Building & signing releases

The desktop app is the native **egui/eframe** crate `gamesync-gui` — no Node,
no WebView, no Tauri. A release build is a single, self-contained executable:

```sh
cargo build --release -p gamesync-gui
```

The binary lands at **`target/release/gamesync-gui`** (`.exe` on Windows). That
bare executable *is* the "portable" build — copy it anywhere and run it.

| OS | Portable binary | Notes |
| --- | --- | --- |
| **macOS** | `target/release/gamesync-gui` | Wrap in a `.app` bundle for a Dock icon (see below). |
| **Windows** | `target/release/gamesync-gui.exe` | No WebView2 or VC++ redistributable needed (CRT is statically linked in CI). |
| **Linux** | `target/release/gamesync-gui` | Dynamically links system GTK/X11 libs only. |

> Native OS installers (`.dmg`/`.msi`/`.deb`/`.AppImage`) were produced by Tauri
> and are no longer built. If you want them back, add a Rust bundler such as
> [`cargo-bundle`](https://github.com/burtonageo/cargo-bundle) or
> [`cargo-dist`](https://github.com/axodotdev/cargo-dist) — it doesn't change the
> app, only the packaging.

## Targets & architectures

Releases cover **macOS (universal), Windows x64 + arm64, and Linux x64 + arm64**.
Each architecture is built on a matching machine — the
[`Release` workflow](../.github/workflows/release.yml) does this automatically on
a tag push (`git tag v0.3.0 && git push origin v0.3.0`), using GitHub's native
ARM runners (`windows-11-arm`, `ubuntu-22.04-arm`) so nothing is cross-compiled.

To build a specific target **locally**:

```sh
# macOS universal (one binary that runs on Apple Silicon AND Intel):
rustup target add aarch64-apple-darwin x86_64-apple-darwin
cargo build --release -p gamesync-gui --target aarch64-apple-darwin
cargo build --release -p gamesync-gui --target x86_64-apple-darwin
lipo -create -output gamesync-gui-universal \
  target/aarch64-apple-darwin/release/gamesync-gui \
  target/x86_64-apple-darwin/release/gamesync-gui

# Windows: statically link the CRT so the .exe needs no VC++ redistributable.
$env:RUSTFLAGS = "-C target-feature=+crt-static"   # PowerShell
cargo build --release -p gamesync-gui

# Linux arm64: build on real arm64 hardware (or the ubuntu-22.04-arm CI runner).
```

Omit `--target` to build for the host architecture (the common case). The Linux
build needs the GUI dev libraries: `build-essential libssl-dev libgtk-3-dev
libxkbcommon-dev libayatana-appindicator3-dev` (plus the `libxcb-*` packages the
CI workflow installs).

> **Never commit** certificates, `.p12`/`.cer`/`.key` files, or passwords. Use
> environment variables locally and encrypted CI secrets. `.gitignore` already
> excludes the common ones.

## macOS — bundle, sign & notarize

For a double-clickable app, wrap the binary in a minimal `.app` bundle
(`Contents/MacOS/gamesync-gui`, `Contents/Info.plist`, an `AppIcon.icns`), then
sign and notarize with Apple's own tools (no Tauri involved):

```sh
# Ad-hoc sign (runs locally, still warns on other machines):
codesign --force --deep --sign - GameSync.app

# Trusted build — needs a "Developer ID Application" cert in your Keychain:
codesign --force --deep --options runtime \
  --sign "Developer ID Application: Your Name (TEAMID)" GameSync.app
xcrun notarytool submit GameSync.app --apple-id you@example.com \
  --team-id TEAMID --password "app-specific-pw" --wait
xcrun stapler staple GameSync.app
```

Verify afterwards with `spctl -a -vvv GameSync.app`.

## Windows — sign

With a code-signing certificate, sign the `.exe` directly:

```sh
signtool sign /fd sha256 /tr http://timestamp.digicert.com /td sha256 \
  /a target\release\gamesync-gui.exe
```

or use cloud signing (Azure Trusted Signing). Code signing is the durable fix for
SmartScreen/Gatekeeper warnings — independent of the framework.

## Linux

The bare binary and any `.AppImage` you choose to build are typically
distributed unsigned; if you publish an apt/rpm repo you can GPG-sign it. No
app-level signing is required to run them.

## CI (automated releases)

[`.github/workflows/release.yml`](../.github/workflows/release.yml) runs on a
`v*` tag push, builds all five targets in parallel (`cargo build --release -p
gamesync-gui`), and attaches the raw/portable binaries to a **draft** GitHub
release via `softprops/action-gh-release`. To ship:

1. Bump `version` in the workspace `Cargo.toml`, commit, then
   `git tag vX.Y.Z && git push origin vX.Y.Z`.
2. Wait for the matrix to finish, review the draft release, and **Publish**.

> The ARM runners (`windows-11-arm`, `ubuntu-22.04-arm`) are free for public
> repos; on a private repo they may require a paid plan — swap arm64 entries for
> cross-compilation or self-hosted runners if so.
