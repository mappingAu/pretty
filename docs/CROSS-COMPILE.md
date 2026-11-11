# Cross-compilation status (generated)

Generated: 2026-11-10T16:23:00Z by `scripts/cross-check.sh`.

Per-platform prerequisites, the implemented/compiled/runtime-tested
breakdown and the release limitations live in
[CROSS-PLATFORM.md](CROSS-PLATFORM.md). This file is the machine-generated
result table for one run on this host.

**Scope:** every PASS below is a compile-only `cargo check`/`cargo test`
result. `cargo check` does not link or run a binary, and no physical
Windows, macOS or Android hardware was used. A SKIP (missing Rust std) is
never reported as a PASS.

Run log: `test-artifacts/cross-check/20261110T162209Z`

## Host feature checks

| Check | Features | Result | Exit | Log |
|---|---|---|---|---|
| `host-workspace` | - | PASS | 0 | [`test-artifacts/cross-check/20261110T162209Z/host-workspace`](../test-artifacts/cross-check/20261110T162209Z/host-workspace) |
| `host-vulkan-only` | vulkan | PASS | 0 | [`test-artifacts/cross-check/20261110T162209Z/host-vulkan-only`](../test-artifacts/cross-check/20261110T162209Z/host-vulkan-only) |
| `host-gles-only` | gles | PASS | 0 | [`test-artifacts/cross-check/20261110T162209Z/host-gles-only`](../test-artifacts/cross-check/20261110T162209Z/host-gles-only) |
| `host-tests-renderer` | default | PASS | 0 | [`test-artifacts/cross-check/20261110T162209Z/host-tests-renderer`](../test-artifacts/cross-check/20261110T162209Z/host-tests-renderer) |
| `host-tests-metal` | metal | PASS | 0 | [`test-artifacts/cross-check/20261110T162209Z/host-tests-metal`](../test-artifacts/cross-check/20261110T162209Z/host-tests-metal) |
| `host-tests-dx` | dx | PASS | 0 | [`test-artifacts/cross-check/20261110T162209Z/host-tests-dx`](../test-artifacts/cross-check/20261110T162209Z/host-tests-dx) |

## Cross-target checks

| Target | Features | Result | Exit | Log |
|---|---|---|---|---|
| `x86_64-pc-windows-msvc` | dx | COMPILE-ONLY PASS | 0 | [`test-artifacts/cross-check/20261110T162209Z/windows-msvc-dx`](../test-artifacts/cross-check/20261110T162209Z/windows-msvc-dx) |
| `x86_64-pc-windows-gnu` | dx | COMPILE-ONLY PASS | 0 | [`test-artifacts/cross-check/20261110T162209Z/windows-gnu-dx`](../test-artifacts/cross-check/20261110T162209Z/windows-gnu-dx) |
| `x86_64-apple-darwin` | metal | COMPILE-ONLY PASS | 0 | [`test-artifacts/cross-check/20261110T162209Z/macos-x86_64-metal`](../test-artifacts/cross-check/20261110T162209Z/macos-x86_64-metal) |
| `aarch64-apple-darwin` | metal | COMPILE-ONLY PASS | 0 | [`test-artifacts/cross-check/20261110T162209Z/macos-aarch64-metal`](../test-artifacts/cross-check/20261110T162209Z/macos-aarch64-metal) |
| `aarch64-linux-android` | android | COMPILE-ONLY PASS | 0 | [`test-artifacts/cross-check/20261110T162209Z/android-aarch64`](../test-artifacts/cross-check/20261110T162209Z/android-aarch64) |

## Environment

```text
generated_utc=2026-11-10T16:22:09Z
host=Linux 7.2.15-300.fc45.x86_64 x86_64
cargo=cargo 1.99.1 (5f94df478 2026-08-27)
host_target_dir=/home/mapping/文档/deepseek-harness/default-workspace/pretty/target
cross_target_dir=/home/mapping/文档/deepseek-harness/default-workspace/pretty/target/cross
rustup=rustup 1.29.2 (d8c92c676 2026-10-21)
toolchains:
  stable-x86_64-unknown-linux-gnu (active, default)
installed_targets:
  aarch64-apple-darwin
  aarch64-linux-android
  x86_64-apple-darwin
  x86_64-pc-windows-gnu
  x86_64-pc-windows-msvc
  x86_64-unknown-linux-gnu
linkers_and_sdks:
  x86_64-w64-mingw32-gcc=missing
  x86_64-w64-mingw32-ld=missing
  clang=missing
  lld=missing
  ld64.lld=missing
  aarch64-linux-android-clang=missing
  llvm-ar=missing
  ANDROID_NDK_HOME=(unset)
  ANDROID_NDK_ROOT=(unset)
  ANDROID_HOME=(unset)
  NDK_HOME=(unset)
git_head=e540a2a
git_dirty_files=36
```

## Check details

### `host-workspace` — PASS (exit 0)

```bash
/home/mapping/.cargo/bin/cargo check --workspace
```

Full output: [`test-artifacts/cross-check/20261110T162209Z/host-workspace/stdout.log`](../test-artifacts/cross-check/20261110T162209Z/host-workspace/stdout.log),
[`test-artifacts/cross-check/20261110T162209Z/host-workspace/stderr.log`](../test-artifacts/cross-check/20261110T162209Z/host-workspace/stderr.log),
[`test-artifacts/cross-check/20261110T162209Z/host-workspace/result.txt`](../test-artifacts/cross-check/20261110T162209Z/host-workspace/result.txt).

### `host-vulkan-only` — PASS (exit 0)

```bash
/home/mapping/.cargo/bin/cargo check -p pretty-app --no-default-features --features vulkan
```

Full output: [`test-artifacts/cross-check/20261110T162209Z/host-vulkan-only/stdout.log`](../test-artifacts/cross-check/20261110T162209Z/host-vulkan-only/stdout.log),
[`test-artifacts/cross-check/20261110T162209Z/host-vulkan-only/stderr.log`](../test-artifacts/cross-check/20261110T162209Z/host-vulkan-only/stderr.log),
[`test-artifacts/cross-check/20261110T162209Z/host-vulkan-only/result.txt`](../test-artifacts/cross-check/20261110T162209Z/host-vulkan-only/result.txt).

### `host-gles-only` — PASS (exit 0)

```bash
/home/mapping/.cargo/bin/cargo check -p pretty-app --no-default-features --features gles
```

Full output: [`test-artifacts/cross-check/20261110T162209Z/host-gles-only/stdout.log`](../test-artifacts/cross-check/20261110T162209Z/host-gles-only/stdout.log),
[`test-artifacts/cross-check/20261110T162209Z/host-gles-only/stderr.log`](../test-artifacts/cross-check/20261110T162209Z/host-gles-only/stderr.log),
[`test-artifacts/cross-check/20261110T162209Z/host-gles-only/result.txt`](../test-artifacts/cross-check/20261110T162209Z/host-gles-only/result.txt).

### `host-tests-renderer` — PASS (exit 0)

```bash
/home/mapping/.cargo/bin/cargo test -p pretty-renderer --lib
```

Full output: [`test-artifacts/cross-check/20261110T162209Z/host-tests-renderer/stdout.log`](../test-artifacts/cross-check/20261110T162209Z/host-tests-renderer/stdout.log),
[`test-artifacts/cross-check/20261110T162209Z/host-tests-renderer/stderr.log`](../test-artifacts/cross-check/20261110T162209Z/host-tests-renderer/stderr.log),
[`test-artifacts/cross-check/20261110T162209Z/host-tests-renderer/result.txt`](../test-artifacts/cross-check/20261110T162209Z/host-tests-renderer/result.txt).

### `host-tests-metal` — PASS (exit 0)

```bash
/home/mapping/.cargo/bin/cargo test -p pretty-renderer --lib --features metal
```

Full output: [`test-artifacts/cross-check/20261110T162209Z/host-tests-metal/stdout.log`](../test-artifacts/cross-check/20261110T162209Z/host-tests-metal/stdout.log),
[`test-artifacts/cross-check/20261110T162209Z/host-tests-metal/stderr.log`](../test-artifacts/cross-check/20261110T162209Z/host-tests-metal/stderr.log),
[`test-artifacts/cross-check/20261110T162209Z/host-tests-metal/result.txt`](../test-artifacts/cross-check/20261110T162209Z/host-tests-metal/result.txt).

### `host-tests-dx` — PASS (exit 0)

```bash
/home/mapping/.cargo/bin/cargo test -p pretty-renderer --lib --features dx
```

Full output: [`test-artifacts/cross-check/20261110T162209Z/host-tests-dx/stdout.log`](../test-artifacts/cross-check/20261110T162209Z/host-tests-dx/stdout.log),
[`test-artifacts/cross-check/20261110T162209Z/host-tests-dx/stderr.log`](../test-artifacts/cross-check/20261110T162209Z/host-tests-dx/stderr.log),
[`test-artifacts/cross-check/20261110T162209Z/host-tests-dx/result.txt`](../test-artifacts/cross-check/20261110T162209Z/host-tests-dx/result.txt).

### `windows-msvc-dx` — PASS (exit 0)

```bash
/home/mapping/.cargo/bin/cargo check --target x86_64-pc-windows-msvc -p pretty-app --features dx --locked
```

Full output: [`test-artifacts/cross-check/20261110T162209Z/windows-msvc-dx/stdout.log`](../test-artifacts/cross-check/20261110T162209Z/windows-msvc-dx/stdout.log),
[`test-artifacts/cross-check/20261110T162209Z/windows-msvc-dx/stderr.log`](../test-artifacts/cross-check/20261110T162209Z/windows-msvc-dx/stderr.log),
[`test-artifacts/cross-check/20261110T162209Z/windows-msvc-dx/result.txt`](../test-artifacts/cross-check/20261110T162209Z/windows-msvc-dx/result.txt).

### `windows-gnu-dx` — PASS (exit 0)

```bash
/home/mapping/.cargo/bin/cargo check --target x86_64-pc-windows-gnu -p pretty-app --features dx --locked
```

Full output: [`test-artifacts/cross-check/20261110T162209Z/windows-gnu-dx/stdout.log`](../test-artifacts/cross-check/20261110T162209Z/windows-gnu-dx/stdout.log),
[`test-artifacts/cross-check/20261110T162209Z/windows-gnu-dx/stderr.log`](../test-artifacts/cross-check/20261110T162209Z/windows-gnu-dx/stderr.log),
[`test-artifacts/cross-check/20261110T162209Z/windows-gnu-dx/result.txt`](../test-artifacts/cross-check/20261110T162209Z/windows-gnu-dx/result.txt).

### `macos-x86_64-metal` — PASS (exit 0)

```bash
/home/mapping/.cargo/bin/cargo check --target x86_64-apple-darwin -p pretty-app --features metal --locked
```

Full output: [`test-artifacts/cross-check/20261110T162209Z/macos-x86_64-metal/stdout.log`](../test-artifacts/cross-check/20261110T162209Z/macos-x86_64-metal/stdout.log),
[`test-artifacts/cross-check/20261110T162209Z/macos-x86_64-metal/stderr.log`](../test-artifacts/cross-check/20261110T162209Z/macos-x86_64-metal/stderr.log),
[`test-artifacts/cross-check/20261110T162209Z/macos-x86_64-metal/result.txt`](../test-artifacts/cross-check/20261110T162209Z/macos-x86_64-metal/result.txt).

### `macos-aarch64-metal` — PASS (exit 0)

```bash
/home/mapping/.cargo/bin/cargo check --target aarch64-apple-darwin -p pretty-app --features metal --locked
```

Full output: [`test-artifacts/cross-check/20261110T162209Z/macos-aarch64-metal/stdout.log`](../test-artifacts/cross-check/20261110T162209Z/macos-aarch64-metal/stdout.log),
[`test-artifacts/cross-check/20261110T162209Z/macos-aarch64-metal/stderr.log`](../test-artifacts/cross-check/20261110T162209Z/macos-aarch64-metal/stderr.log),
[`test-artifacts/cross-check/20261110T162209Z/macos-aarch64-metal/result.txt`](../test-artifacts/cross-check/20261110T162209Z/macos-aarch64-metal/result.txt).

### `android-aarch64` — PASS (exit 0)

```bash
/home/mapping/.cargo/bin/cargo check --target aarch64-linux-android -p pretty-app --features android --locked
```

Full output: [`test-artifacts/cross-check/20261110T162209Z/android-aarch64/stdout.log`](../test-artifacts/cross-check/20261110T162209Z/android-aarch64/stdout.log),
[`test-artifacts/cross-check/20261110T162209Z/android-aarch64/stderr.log`](../test-artifacts/cross-check/20261110T162209Z/android-aarch64/stderr.log),
[`test-artifacts/cross-check/20261110T162209Z/android-aarch64/result.txt`](../test-artifacts/cross-check/20261110T162209Z/android-aarch64/result.txt).

## Native CI builds

Native Windows/macOS release binaries are built and unit-tested by
`.github/workflows/build.yml` (DX12 feature on Windows, Metal feature on
macOS, default Vulkan+GLES on Linux). CI does not launch the GUI, so it
also does not prove runtime GPU support on those platforms.

## Notes

- A target can PASS `cargo check` even when its system linker, MSVC
  build tools, Xcode SDK or Android NDK is absent, because `cargo check`
  stops before linking. Do not read that as proof of a working build.
- Host feature tests execute the renderer's backend-gate unit tests with
  the `metal` and `dx` features compiled on Linux; the wrong-OS typed
  errors are exercised there, not on Apple/Windows hardware.
- Android is compile-checked only. The repository has an
  `android_main` NativeActivity entry point but no Gradle/NDK APK
  packaging; see CROSS-PLATFORM.md.
