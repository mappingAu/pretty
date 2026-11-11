# Windows portable zip

`package.ps1` stages a portable directory tree and compresses it to
`dist/pretty-<version>-windows-x86_64.zip`. It does not build Rust code.

```powershell
# From the repository root on Windows (PowerShell 7):
cargo build --release -p pretty-app --features dx
pwsh -File packaging/windows/package.ps1
```

`scripts/build-windows.sh` wraps both steps for Git Bash / CI:

```bash
scripts/build-windows.sh              # build + package
scripts/build-windows.sh --no-build   # package the existing binary
```

## Zip layout

```
pretty-<version>-windows-x86_64/
  pretty.exe
  README.md
  README-WINDOWS.txt
  LICENSE
  docs/*.md (including CROSS-PLATFORM.md and CROSS-COMPILE.md)
  icons/pretty-256.png
```

`dist/SHA256SUMS-windows.txt` contains the SHA-256 of the zip.

## Expected backend behaviour

Windows release builds pass `--features dx`, so the `auto` order is
**Direct3D 12 → Vulkan → OpenGL ES**. `pretty.exe --print-info` shows the
compiled features, the `auto` order and the enumerated adapters.

* The binaries are unsigned; SmartScreen may warn. This is documented in
  `README-WINDOWS.txt`.
* The build is performed by `.github/workflows/build.yml` on `windows-latest`
  (MSVC toolchain). The local release host has no MinGW linker, which is why
  Windows artifacts come from CI and local checks stay compile-only.
