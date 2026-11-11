# macOS application bundle

`package.sh` stages `Pretty.app`, generates a `.icns` icon with macOS tooling
and produces two archives:

- `dist/pretty-<version>-macos-<arch>.app.tar.gz` — the `.app` bundle;
- `dist/pretty-<version>-macos-<arch>.tar.gz` — standalone CLI tree.

```bash
# On macOS, from the repository root:
cargo build --release -p pretty-app --features metal
packaging/macos/package.sh
```

`scripts/build-macos.sh` wraps the build and packaging steps:

```bash
scripts/build-macos.sh              # build + package
scripts/build-macos.sh --no-build   # package the existing binary
```

## Bundle layout

```
Pretty.app/
  Contents/
    Info.plist          # generated from packaging/macos/Info.plist
    PkgInfo
    MacOS/pretty
    Resources/pretty.icns   # when sips + iconutil are available
    Resources/LICENSE
```

The bundle identifier is `io.github.mappingAu.Pretty`, matching the Flatpak app
id. The bundle is ad-hoc signed (`codesign --sign -`) when `codesign` exists; it
is **not** Developer ID signed and not notarised.

## Expected backend behaviour

The macOS release build passes `--features metal`, so `auto` prefers **Metal**
and falls back to Vulkan (MoltenVK, when present). `pretty --print-info` lists
the compiled backends and the platform `auto` order.

## Build provenance

macOS artifacts are produced by `.github/workflows/build.yml` on
`macos-latest` (Apple Silicon, arm64). The development host used for this
release is Linux with no Apple linker, so the local check for this target is
`cargo check` compile-only; see `docs/CROSS-PLATFORM.md` for prerequisites and
`docs/CROSS-COMPILE.md` for the generated result table. The macOS GUI was not
executed on physical Mac hardware as part of this release.
