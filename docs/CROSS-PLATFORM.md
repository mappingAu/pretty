# Cross-platform status — Pretty 0.2.0

This document is the per-platform companion to the generated
[`CROSS-COMPILE.md`](CROSS-COMPILE.md) result table. It separates three very
different claims:

| Claim | Meaning |
|---|---|
| **Implemented** | The code path exists in this repository and is covered by unit tests where testable without a GPU. |
| **Compile-checked** | `cargo check`/`cargo test` exited 0 for that target on the host documented below. This does **not** link a binary and does **not** prove the SDK/toolchain is complete. |
| **Runtime-tested** | The GUI was launched and a GPU surface was presented on that operating system. |

The 0.2.0 release host is Fedora Linux (x86-64) with an NVIDIA GTX 1050, KDE
Plasma Wayland + XWayland. Linux Vulkan/GLES runtime behaviour is covered by
previous hardware test runs in `test-artifacts/hardware-*`. **No physical
Windows or macOS machine, and no Android device or emulator, was used for this
release.** Nothing in this document claims otherwise.

## Backend matrix

`RendererKind` is the single source of truth. `crates/pretty-renderer/src/backend.rs`
maps it to exactly one `wgpu::Backend`; `context.rs` uses that same method for
adapter enumeration, adapter filtering and `--print-info`, so a driver can no
longer disagree with the instance that created it.

| RendererKind | wgpu backend | Cargo feature | Operating system | `auto` order |
|---|---|---|---|---|
| `vulkan` | `Backend::Vulkan` | `vulkan` (default) | Linux/BSD, Windows, Android (MoltenVK on macOS if compiled) | Linux/Android: 1st; Windows: 2nd; macOS: 2nd |
| `gles` | `Backend::Gl` | `gles` (default) | Linux/BSD, Windows, Android, macOS | Linux/Android: 2nd; Windows: 3rd |
| `metal` | `Backend::Metal` | `metal` | macOS only | macOS: 1st |
| `dx` | `Backend::Dx12` | `dx` | Windows only | Windows: 1st |

`auto` candidates are platform-aware (`RendererKind::candidates`); candidates
whose feature is not compiled in are skipped in the automatic path. Explicitly
selecting an unavailable backend is a hard error — there is no silent fallback
and no CPU rasteriser:

* `BackendError::FeatureNotCompiled { backend }` — the binary was built without
  e.g. `--features metal`; the message includes the exact rebuild flag.
* `BackendError::WrongOperatingSystem { backend, host, required }` — compiled
  in, but e.g. Metal on Linux (`only available on macOS; this binary targets
  linux`). `RendererError::Backend` wraps this typed error for the app.

Unit tests cover both gates: default builds assert the feature-missing errors;
`--features metal` and `--features dx` builds on Linux assert the wrong-OS
typed errors. The renderer test matrix runs all three on the host.

### Surface policy shared by every backend

`context.rs` uses one policy for Vulkan, GLES, Metal and Direct3D 12:

* **Format:** first available of `Bgra8UnormSrgb`, `Rgba8UnormSrgb`,
  `Bgra8Unorm`, `Rgba8Unorm`; otherwise wgpu's first advertised format.
* **Alpha:** when transparency is requested, `PreMultiplied` →
  `PostMultiplied` → `Inherit`, else `Opaque` with a warning (the app then
  reports transparency as unavailable).
* **Present mode:** `Fifo` when vsync is on; otherwise `Mailbox` →
  `Immediate` → `Fifo`.
* **Adapter choice:** discrete > integrated > virtual > other > CPU; CPU
  adapters are rejected unless `--allow-software` is passed explicitly.

These helpers (`select_format`, `select_present_mode`, `select_alpha_mode`)
are unit-tested on the host, independent of GPU hardware.

## Implementation status

| Platform | Backend | Implemented | Compile-checked | Runtime-tested | Notes |
|---|---|---|---|---|---|
| Linux | Vulkan | Yes | Yes (native) | Yes (GTX 1050, KDE Wayland; `test-artifacts/hardware-*`) | Default features; release gate. |
| Linux | GLES | Yes | Yes (native) | Yes (XWayland; `test-artifacts/hardware-*`) | Uses the local `vendor/wgpu-hal` EGL patch to force a real GLES context. |
| Windows | Direct3D 12 | Yes (driver + shared surface policy) | Yes (`cargo check`, MSVC & GNU targets) | **No** | Native release build is defined in CI, not reproduced on this host. |
| Windows | Vulkan / GLES | Yes (wgpu) | Yes (same check) | **No** | Available if the driver/feature is compiled; `auto` prefers DX12. |
| macOS | Metal | Yes (driver + shared surface policy) | Yes (`cargo check`, x86-64 + arm64) | **No** | Needs Xcode/CLT and a Metal-capable Mac; not run here. |
| macOS | Vulkan (MoltenVK) | Available through wgpu when compiled | Yes (same check) | **No** | Optional fallback; MoltenVK runtime not tested. |
| Android | Vulkan + GLES | NativeActivity initialization only | `cargo check --features android` (see generated table) | **No** | No APK/Gradle packaging, no NDK link performed. |

## Per-platform prerequisites

### Linux / BSD (reference platform)

* Rust stable (workspace `rust-version = "1.89"`; release host used 1.99.1).
* System build dependencies (Fedora names; Debian/Ubuntu equivalents in the CI
  workflow): `pkg-config`, `wayland-devel`, `libxkbcommon-devel`, `libX11-devel`,
  `libxcb-devel`, `mesa-libEGL-devel`, `mesa-libGLES-devel`, `vulkan-loader-devel`,
  plus a C toolchain.
* Runtime: a Vulkan ICD (`mesa-vulkan-drivers` / NVIDIA driver) and/or EGL +
  GLES 3.0 drivers. X11 or Wayland session; KDE blur additionally needs
  `ext_background_effect_manager_v1` (Plasma 6) or the X11 blur atom.
* Unix paths: XDG Base Directory variables (`XDG_CONFIG_HOME`,
  `XDG_DATA_HOME`, `XDG_CACHE_HOME`, `XDG_RUNTIME_DIR`, `XDG_DATA_DIRS`) with
  `$HOME/.config`, `$HOME/.local/share`, `$HOME/.cache` fallbacks.
* Build: `cargo build --release -p pretty-app` (default `vulkan,gles`).

### Windows — MSVC (recommended / CI)

* Windows 10 1903+ or Windows 11 on x86-64, with a Direct3D 12 capable GPU and
  a current vendor driver.
* Visual Studio 2022 Build Tools with **Desktop development with C++**,
  MSVC v143 and the Windows 10/11 SDK (the linker comes from there).
* Rust stable with the `x86_64-pc-windows-msvc` target; `rustup` is the
  supported way to get it.
* Build: `cargo build --release -p pretty-app --features dx` from a
  "x64 Native Tools Command Prompt", or `scripts/build-windows.sh` in Git Bash.
  The release feature set deliberately keeps `vulkan,gles` as well; `auto`
  prefers DX12.
* Package: `scripts/build-windows.sh --no-build` → `packaging/windows/package.ps1`
  (PowerShell 7) → `dist/pretty-<version>-windows-x86_64.zip`.
* Runtime data: `%APPDATA%\pretty\pretty\config\config.toml`;
  cache `%LOCALAPPDATA%\pretty\pretty\cache`; data
  `%APPDATA%\pretty\pretty\data`. Override with `--config`/`PRETTY_CONFIG`.
* Fonts: `%WINDIR%\Fonts` and `%LOCALAPPDATA%\Microsoft\Windows\Fonts`;
  terminfo is bundled in the zip (`share\terminfo`), not read from Unix paths.
* **Not runtime-tested on physical Windows hardware in this release.**

### Windows — GNU (secondary)

* `x86_64-pc-windows-gnu` Rust target (installable on Linux with
  `rustup target add x86_64-pc-windows-gnu`) plus a mingw-w64 toolchain for
  *linking*: `x86_64-w64-mingw32-gcc` and `x86_64-w64-mingw32-ld`.
* This host has the Rust std for the GNU target but **no mingw linker**, so only
  `cargo check` is meaningful here; it does not link. CI and the release
  artifacts use MSVC, not GNU.
* Set `CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER`/`CC_...` if a mingw
  toolchain is installed elsewhere.

### macOS — Metal

* macOS 11+ on a Metal-capable Mac (Intel or Apple Silicon), Xcode 15+ or the
  Command Line Tools (`xcode-select --install`) for the SDK, linker,
  `codesign`, `sips` and `iconutil`.
* Rust stable with the matching target: `aarch64-apple-darwin` (Apple Silicon)
  or `x86_64-apple-darwin` (Intel). Universal builds need both.
* Build: `cargo build --release -p pretty-app --features metal`. Apple's
  linker is only available on macOS; the Linux host cannot link this target.
* Package: `scripts/build-macos.sh --no-build` builds `Pretty.app`
  (`packaging/macos/package.sh`), ad-hoc signs it when `codesign` exists, and
  creates `.app.tar.gz` + standalone `.tar.gz`. It is **not** Developer ID
  signed or notarised.
* Runtime data: `~/Library/Application Support/org.pretty.pretty/config.toml`;
  cache `~/Library/Caches/org.pretty.pretty`.
* Fonts: `~/Library/Fonts`, `/Library/Fonts`, `/System/Library/Fonts`;
  terminfo: Homebrew/`/usr/share/terminfo` plus the bundled copy.
* **Not runtime-tested on physical Mac hardware in this release.**

### Android — NativeActivity (compile-check only)

* Android SDK and NDK r26+ (r27 recommended); `ANDROID_NDK_HOME`/`ANDROID_HOME`
  exported. NDK clang is required to link a real `.so`.
* Rust target `aarch64-linux-android` (arm64; add `armv7-linux-androideabi` /
  `x86_64-linux-android` for other ABIs).
* cargo-ndk or cargo-apk/cargo-xbuild to inject the NDK linker and package the
  APK. **Neither is set up in this repository**: there is no `AndroidManifest.xml`,
  Gradle project, JNI shim, or `[lib] crate-type = ["cdylib"]` target.
* What exists: `crates/pretty-app/src/main.rs` has a `#[cfg(target_os = "android")]`
  `android_main(app: AndroidApp)` entry point using winit's
  `android-native-activity` feature (enabled by `--features android`). It
  exports `PRETTY_CONFIG` from `AndroidApp::internal_data_path()` and builds the
  event loop with `EventLoopBuilderExtAndroid::with_android_app`. A plain
  `cargo check --target aarch64-linux-android --features android` validates it;
  an APK is explicitly out of scope for this release. The Android bin target
  itself compiles a stub `main` only so Cargo has a bin entry; the NativeActivity
  glue calls `android_main` when the crate is eventually built as a cdylib.
* **Not runtime-tested on any Android device or emulator.**

## Cross-platform path abstraction

`crates/pretty-app/src/platform/paths.rs` resolves user config/data/cache,
font and terminfo directories without `cfg!`-guessing at each call site. The
pure resolver can be tested for every platform on any host because it takes an
environment lookup closure instead of reading the process environment.

| Path | Linux/BSD | Windows | macOS | Android |
|---|---|---|---|---|
| config | `$XDG_CONFIG_HOME/pretty` or `~/.config/pretty` | `%APPDATA%\pretty\pretty\config` | `~/Library/Application Support/org.pretty.pretty` | per-app (`AndroidApp::internal_data_path`) |
| data | `$XDG_DATA_HOME/pretty` or `~/.local/share/pretty` | `%APPDATA%\pretty\pretty\data` | `~/Library/Application Support/org.pretty.pretty` | per-app |
| cache | `$XDG_CACHE_HOME/pretty` or `~/.cache/pretty` | `%LOCALAPPDATA%\pretty\pretty\cache` | `~/Library/Caches/org.pretty.pretty` | per-app |
| runtime | `$XDG_RUNTIME_DIR` (absolute only) | — | — | — |
| fonts | `$XDG_DATA_HOME/fonts`, `~/.fonts`, `$XDG_DATA_DIRS/*/fonts` | `%WINDIR%\Fonts`, `%LOCALAPPDATA%\Microsoft\Windows\Fonts` | `~/Library/Fonts`, `/Library/Fonts`, `/System/Library/Fonts` | `/system/fonts`, `/system/font` |
| terminfo | `$TERMINFO`, `$XDG_DATA_HOME/terminfo`, `~/.terminfo`, `/usr/share/terminfo`, … | `$TERMINFO`, data dir `terminfo` | `$TERMINFO`, `/usr/share/terminfo`, `/opt/homebrew/share/terminfo`, … | `$TERMINFO`, `/system/etc/terminfo` |

Rules: XDG variables must be absolute (relative values are ignored, as the
specification requires); empty variables fall back; missing `$HOME` yields no
per-user directories but keeps system font/terminfo roots; Windows joins use
`\` and honour `APPDATA`/`LOCALAPPDATA`/`USERPROFILE`/`WINDIR`. Ten unit tests
cover overrides, fallbacks, relative-value rejection, the Windows/macOS/Android
layouts and terminfo ordering. The tests use synthetic environments only — no
test reads the real user's home directory.

**Migration status (done):** the app owner migrated the call sites on the final
tree: `config::config_path` uses `Paths::host().config_file()` after the
explicit/`PRETTY_CONFIG` checks, `app::install_ui_font` uses
`Paths::host().cjk_font_candidates()`, and `settings.rs` uses
`Paths::host().kitty_config_file()` for the import hint. No hard-coded
Unix/macOS/Windows config or CJK-font path remains in those modules. On Linux
the mapping is identical to the previous `ProjectDirs` result;
`org.pretty.pretty` matches the existing macOS bundle id and the Windows layout
matches the portable-zip documentation.

## Real cross-check results (final tree)

Final re-run of `scripts/cross-check.sh` on the integrated 0.2.0 tree:
`test-artifacts/cross-check/20261110T162209Z/`, **pass=11 skip=0 fail=0**.
Every check below exited 0; the generated per-check logs contain the exact
commands, stdout/stderr and environment probes. The table above is refreshed by
the script; this paragraph records that the final integration (app UI/config,
core QA fixes and renderer changes) did not regress any cross target.

## Real cross-check results

The table below is generated from a real run on the release host; the source
of truth is [`CROSS-COMPILE.md`](CROSS-COMPILE.md) plus the logs under
`test-artifacts/cross-check/<timestamp>/`. A `cargo check` exit code of 0 is
reported as **COMPILE-ONLY PASS**; a missing Rust std is **SKIPPED**, never
PASS. The exact run directory and exit codes are in `CROSS-COMPILE.md` and
`test-artifacts/cross-check/<timestamp>/summary.txt`.

Final run: `test-artifacts/cross-check/20261110T160036Z` (2026-11-11T00:00:36Z UTC,
`cargo 1.99.1`, rustup 1.29.2). **All 11 checks exited 0.** This is the last
run after the working tree settled; it covers the final `android_main` import
gating and the final backend/context code. Earlier in-flight runs are not cited.

The Windows MSVC target std was downloaded during this work (`rustup target add
x86_64-pc-windows-msvc`). The host still has no MinGW, Apple linker, MSVC build
tools, Xcode or Android NDK (see `environment.log`), which is why these are
compile-only results.

### Host checks

| Check | Features | Result | Exit | Log |
|---|---|---|---|---|
| `host-workspace` | - | PASS | 0 | [`test-artifacts/cross-check/20261110T160036Z/host-workspace`](../test-artifacts/cross-check/20261110T160036Z/host-workspace) |
| `host-vulkan-only` | vulkan | PASS | 0 | [`test-artifacts/cross-check/20261110T160036Z/host-vulkan-only`](../test-artifacts/cross-check/20261110T160036Z/host-vulkan-only) |
| `host-gles-only` | gles | PASS | 0 | [`test-artifacts/cross-check/20261110T160036Z/host-gles-only`](../test-artifacts/cross-check/20261110T160036Z/host-gles-only) |
| `host-tests-renderer` | default | PASS | 0 | [`test-artifacts/cross-check/20261110T160036Z/host-tests-renderer`](../test-artifacts/cross-check/20261110T160036Z/host-tests-renderer) |
| `host-tests-metal` | metal | PASS | 0 | [`test-artifacts/cross-check/20261110T160036Z/host-tests-metal`](../test-artifacts/cross-check/20261110T160036Z/host-tests-metal) |
| `host-tests-dx` | dx | PASS | 0 | [`test-artifacts/cross-check/20261110T160036Z/host-tests-dx`](../test-artifacts/cross-check/20261110T160036Z/host-tests-dx) |

### Cross-target checks

| Target | Features | Result | Exit | Log |
|---|---|---|---|---|
| `x86_64-pc-windows-msvc` | dx | COMPILE-ONLY PASS | 0 | [`test-artifacts/cross-check/20261110T160036Z/windows-msvc-dx`](../test-artifacts/cross-check/20261110T160036Z/windows-msvc-dx) |
| `x86_64-pc-windows-gnu` | dx | COMPILE-ONLY PASS | 0 | [`test-artifacts/cross-check/20261110T160036Z/windows-gnu-dx`](../test-artifacts/cross-check/20261110T160036Z/windows-gnu-dx) |
| `x86_64-apple-darwin` | metal | COMPILE-ONLY PASS | 0 | [`test-artifacts/cross-check/20261110T160036Z/macos-x86_64-metal`](../test-artifacts/cross-check/20261110T160036Z/macos-x86_64-metal) |
| `aarch64-apple-darwin` | metal | COMPILE-ONLY PASS | 0 | [`test-artifacts/cross-check/20261110T160036Z/macos-aarch64-metal`](../test-artifacts/cross-check/20261110T160036Z/macos-aarch64-metal) |
| `aarch64-linux-android` | android | COMPILE-ONLY PASS | 0 | [`test-artifacts/cross-check/20261110T160036Z/android-aarch64`](../test-artifacts/cross-check/20261110T160036Z/android-aarch64) |

Each directory contains `command.txt`, `stdout.log`, `stderr.log`,
`result.txt` and `environment.txt`. The generated
[`CROSS-COMPILE.md`](CROSS-COMPILE.md) reproduces this matrix with the full
environment dump and error excerpts. Note that `host-tests-*` runs the
renderer's 15 unit tests; the metal/dx variants exercise the wrong-OS gates on
Linux (Metal/DX drivers are compiled in but must be refused).

## Reproducing

```bash
# All host + cross-target checks, logs and docs refresh:
scripts/cross-check.sh

# Just the target compile checks (requires rustup proxies on PATH or CARGO set):
~/.cargo/bin/rustup target add x86_64-pc-windows-msvc x86_64-pc-windows-gnu \
  x86_64-apple-darwin aarch64-apple-darwin aarch64-linux-android
~/.cargo/bin/cargo check --target x86_64-pc-windows-msvc -p pretty-app --features dx --locked
~/.cargo/bin/cargo check --target x86_64-pc-windows-gnu  -p pretty-app --features dx --locked
~/.cargo/bin/cargo check --target x86_64-apple-darwin    -p pretty-app --features metal --locked
~/.cargo/bin/cargo check --target aarch64-apple-darwin   -p pretty-app --features metal --locked
~/.cargo/bin/cargo check --target aarch64-linux-android  -p pretty-app --features android --locked
```

The Fedora host has a distro `cargo`/`rustc` in `/usr/bin` whose `rustc` cannot
see the rustup-installed target std libraries. The script therefore prefers
`$HOME/.cargo/bin/cargo` when it exists; if `rustup` itself is not on `PATH`,
pass `CARGO=$HOME/.cargo/bin/cargo` and `RUSTUP=$HOME/.cargo/bin/rustup`.

## Known limitations and ownership

* **No physical Windows/macOS hardware and no Android device** was available.
  Direct3D 12, Metal and Android surface presentation are therefore
  compile-checked, not proven at runtime.
* `cargo check` does not link. Missing MSVC build tools, the Apple linker,
  mingw-w64 or the NDK would not fail those checks; real release builds happen
  in `.github/workflows/build.yml` on native runners.
* No GUI was launched for this work (the release host's display was in use).
* **Path migration is complete as of the final tree** (see "Migration status"
  above). `config.rs`, `app.rs` and `settings.rs` now call
  `platform::paths::Paths::host()`; the only remaining hard-coded paths are
  inside `paths.rs` itself as documented platform candidates, plus assets
  bundled with the binary.
* macOS packaging documents the correct
  `~/Library/Application Support/org.pretty.pretty` settings path; the
  `directories` crate uses that bundle id, not `~/Library/Application Support/pretty`.
