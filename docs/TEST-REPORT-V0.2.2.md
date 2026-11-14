# Pretty v0.2.2 platform feature report

**Run:** 0.2.2 maintenance release
**Date:** 2026-11-14 (Asia/Shanghai)
**Host:** Fedora 45, KDE Plasma 6.8.2 (Wayland), NVIDIA GTX 1050 /
proprietary 580.196.07
**Issue:** [Windows: selecting GLES crashes and persists an unopenable config #1](https://github.com/mappingAu/pretty/issues/1)

This release has one purpose: stop the Windows GLES startup crash from
happening, make Vulkan an explicit part of every platform build, and defer the
underlying Windows GLES investigation to the next version.

## 1. Windows GLES is disabled

The Windows release build is now compiled with:

```
cargo build --release -p pretty-app --no-default-features --features vulkan,dx
```

and therefore contains **Vulkan + Direct3D 12, not GLES**.

Three independent layers prevent a Windows user from selecting GLES again:

1. **Not compiled:** the Windows release omits the `gles` feature. The
   `pretty-renderer` dependency is now declared with
   `default-features = false` in `[workspace.dependencies]`; previously the
   renderer's default `gles` feature leaked back in even when the application
   passed `--no-default-features` (confirmed with `cargo tree --target
   x86_64-pc-windows-msvc`). The Windows feature tree is now exactly
   `pretty-renderer/vulkan` + `pretty-renderer/dx`.
2. **Typed runtime rejection:** `RendererKind::Gles.platform_supported()` is
   `false` on Windows, and `driver_for(Gles)` returns
   `BackendError::WrongOperatingSystem` instead of constructing a GL driver.
   `pretty.exe --renderer gles` (or `PRETTY_RENDERER=gles`) exits with a clear
   error before creating the window.
3. **UI and config recovery:**
   - the Settings backend list no longer offers `gles` on Windows;
   - `Config::validate` rewrites a stale `renderer = "gles"` to `auto` with a
     warning instead of making the injected config unopenable. The same
     fallback applies to any backend that is not compiled into the current
     binary or cannot exist on the host OS.

The `auto` order on Windows changed from `dx, vulkan, gles` to `dx, vulkan`.

The underlying wgpu/ANGLE Windows crash is **not fixed** in 0.2.2; issue #1
tracks the investigation (needs a Windows crash log/backtrace and GL driver
information). GLES remains available on Linux, where it is still hardware
tested.

## 2. Vulkan on every platform

Vulkan was already implemented in the renderer, but it is now an explicit part
of every release feature set instead of an implicit default:

| Platform | Release features | `auto` order | `compiled_backends` |
|---|---|---|---|
| Linux / FreeBSD | `vulkan,gles` (default) | Vulkan → GLES | `vulkan=true gles=true` |
| Windows x86_64 | `--no-default-features --features vulkan,dx` | DX12 → Vulkan | `vulkan=true dx=true gles=false` |
| macOS (arm64) | `--no-default-features --features vulkan,metal` | Metal → Vulkan (MoltenVK) | `vulkan=true metal=true gles=false` |
| Android arm64 | `vulkan,gles` + `android` (compile-only) | Vulkan → GLES | no APK |

The Windows and macOS CI jobs build/test with the same feature expression as
their release artifacts, and `--print-info` is uploaded with each artifact so
the compiled backend set is auditable.

### Feature-resolution regression fixed

The initial attempt to pass `--no-default-features` to `pretty-app` was not
enough: `pretty-app` still depended on `pretty-renderer` with its own default
features (`vulkan,gles`), so `compiled_backends` still reported `gles=true`.
Moving `default-features = false` to the workspace dependency fixed the leak. Verified with:

```
$ cargo build -p pretty-app --no-default-features --features vulkan
$ ./target/debug/pretty --print-info
compiled_backends: vulkan=true gles=false metal=false dx=false
```

and with the resolved Windows target feature tree:

```
pretty-renderer feature "dx"      <- pretty-app feature "dx"
pretty-renderer feature "vulkan"  <- pretty-app feature "vulkan"
# no pretty-renderer feature "default"; no "gles"
```

## 3. Tests

On the Linux host, with the release feature sets:

```
$ cargo test --workspace
45 app + 61 core unit + 9 audit + 2 PTY (+1 ignored) + 4 QA + 15 renderer = 136 passed

$ cargo test --workspace --no-default-features --features vulkan,dx
45 app + 61 core unit + 9 audit + 2 PTY (+1 ignored) + 4 QA + 14 renderer

$ cargo test --workspace --no-default-features --features vulkan,metal
45 app + 61 core unit + 9 audit + 2 PTY (+1 ignored) + 4 QA + 14 renderer
```

The 14-vs-15 renderer difference is the GLES driver construction test, which
is intentionally compiled only when `gles` exists. New tests:

- `backend::tests::auto_order_is_platform_aware` now expects
  `dx,vulkan` on Windows;
- `backend::tests::platform_support_is_os_specific` asserts GLES is unsupported
  on Windows;
- `backend::tests::gles_on_windows_is_a_typed_wrong_os_error` checks the typed
  error and the `--renderer vulkan` hint;
- `config::tests::unsupported_renderer_falls_back_to_auto` covers stale
  `dx`/`metal`/`gles` values in a config file.

## 4. CI / release artifacts

Native CI run: [43612515531](https://github.com/mappingAu/pretty/actions/runs/43612515531)
(Linux, Windows MSVC/DX12 and macOS arm64/Metal jobs all succeeded).
The uploaded Windows artifact reports:

```
compiled_backends: vulkan=true gles=false dx=true
auto_order: dx,vulkan
```

and the macOS artifact reports:

```
compiled_backends: vulkan=true gles=false metal=true dx=false
auto_order: metal,vulkan
```

The Linux release binary reports `vulkan=true gles=true`, `auto_order:
vulkan,gles`.

The Linux binary remains Vulkan-first with GLES available; the Windows binary
has no GLES code; the macOS app bundle has Metal + Vulkan and no GLES.

## Limitations

- The Windows GLES crash itself is deferred to the next version (issue #1).
  This release only prevents it from being reachable and recovers stale
  configs.
- Windows/macOS GUI behaviour is still not executed on physical hardware in
  this repository; the native CI jobs build, unit-test and package the
  binaries and run `--version`/`--print-info`.
- Android has no APK and remains a compile-only target.
