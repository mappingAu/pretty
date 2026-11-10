#!/usr/bin/env pwsh
# Build the portable Windows zip for Pretty.
#
# Run from the repository root (after building the release binary):
#   pwsh -File packaging/windows/package.ps1
#
# Options:
#   -Version <x.y.z>   Override the version taken from Cargo.toml
#   -Binary <path>     Release binary (default: target/release/pretty.exe)
#   -OutDir <dir>      Output directory (default: dist)
#   -Arch <name>       Architecture label (default: x86_64)
#
# Layout inside the zip:
#   pretty-<version>-windows-<arch>/
#     pretty.exe
#     README.md
#     README-WINDOWS.txt
#     LICENSE
#     docs/*.md
#     icons/pretty-256.png
#
# The script only stages files: it never builds Rust code itself.
# Use scripts/build-windows.sh for "build + package" in one command.
param(
    [string]$Version = "",
    [string]$Binary = "",
    [string]$OutDir = "",
    [string]$Arch = "x86_64"
)

$ErrorActionPreference = "Stop"

$Root = (Resolve-Path (Join-Path $PSScriptRoot "../..")).Path
Push-Location $Root
try {
    if (-not $Version) {
        $match = Get-Content "Cargo.toml" |
            Select-String -Pattern '^version = "(.+)"' |
            Select-Object -First 1
        if (-not $match) { throw "cannot read the workspace version from Cargo.toml" }
        $Version = $match.Matches[0].Groups[1].Value
    }
    if (-not $Binary) {
        $Binary = "target/release/pretty.exe"
    }
    if (-not (Test-Path -Path $Binary -PathType Leaf)) {
        throw "missing release binary '$Binary'; run: cargo build --release -p pretty-app --features dx"
    }
    if (-not $OutDir) { $OutDir = "dist" }

    $StageName = "pretty-$Version-windows-$Arch"
    $Stage = Join-Path (Join-Path "build" "windows") $StageName
    if (Test-Path $Stage) { Remove-Item -Recurse -Force $Stage }
    New-Item -ItemType Directory -Force -Path $Stage | Out-Null
    New-Item -ItemType Directory -Force -Path (Join-Path $Stage "docs") | Out-Null
    New-Item -ItemType Directory -Force -Path (Join-Path $Stage "icons") | Out-Null
    New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

    Copy-Item -Path $Binary -Destination (Join-Path $Stage "pretty.exe")
    Copy-Item -Path "README.md", "LICENSE" -Destination $Stage
    Copy-Item -Path "packaging/windows/README-WINDOWS.txt" -Destination $Stage
    foreach ($doc in @(
            "README.md", "RENDERER.md", "ARCHITECTURE.md",
            "CROSS-COMPILE.md", "PACKAGING.md", "KNOWN-ISSUES.md")) {
        $src = Join-Path "docs" $doc
        if (Test-Path $src) {
            Copy-Item -Path $src -Destination (Join-Path $Stage "docs")
        }
    }
    if (Test-Path "icons/pretty-256.png") {
        Copy-Item -Path "icons/pretty-256.png" -Destination (Join-Path $Stage "icons")
    }

    $Zip = Join-Path $OutDir "$StageName.zip"
    if (Test-Path $Zip) { Remove-Item -Force $Zip }
    Compress-Archive -Path (Join-Path $Stage "*") -DestinationPath $Zip -CompressionLevel Optimal

    $Hash = (Get-FileHash -Algorithm SHA256 -Path $Zip).Hash.ToLowerInvariant()
    $SumFile = Join-Path $OutDir "SHA256SUMS-windows.txt"
    "$Hash  $StageName.zip" | Set-Content -Encoding ascii -Path $SumFile

    Write-Host "staged:   $Stage"
    Write-Host "artifact: $Zip"
    Write-Host "checksum: $Hash"
}
finally {
    Pop-Location
}
