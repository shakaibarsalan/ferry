# Ferry, built on Windows. build.sh is the macOS script and is untouched.
#
# Needs Rust (rustup) and the MSVC build tools. The Tauri CLI comes from npx, so
# there is no package.json here and no "npm run tauri build" to go with it.
#
#   .\build-windows.ps1           the Rust tests, then Ferry.exe, the NSIS setup and the MSI
#   .\build-windows.ps1 -NoTest   skip the tests
param([switch]$NoTest)

$ErrorActionPreference = "Stop"
Set-Location (Split-Path -Parent $MyInvocation.MyCommand.Path)

if (-not $NoTest) {
    Write-Host "== cargo test ==" -ForegroundColor Cyan
    cargo test --manifest-path src-tauri/Cargo.toml
    if ($LASTEXITCODE -ne 0) { throw "tests failed" }
}

Write-Host "== tauri build ==" -ForegroundColor Cyan
npx --yes @tauri-apps/cli@2 build --bundles nsis,msi
if ($LASTEXITCODE -ne 0) { throw "build failed" }

$rel = "src-tauri/target/release"
Get-ChildItem "$rel/ferry.exe", "$rel/bundle/nsis/*-setup.exe", "$rel/bundle/msi/*.msi" -ErrorAction SilentlyContinue |
    Select-Object @{n = "Artifact"; e = { $_.FullName } }, @{n = "MB"; e = { [math]::Round($_.Length / 1MB, 2) } }
