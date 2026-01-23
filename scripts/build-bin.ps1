$ErrorActionPreference = "Stop"

Write-Host "Building release binary..."
cargo build --release

$binDir = "bin"
if (-not (Test-Path $binDir)) {
  New-Item -ItemType Directory -Force -Path $binDir | Out-Null
}

$src = "target\\release\\rust-win32.exe"
$dst = "bin\\ace-tool-win.exe"

if (-not (Test-Path $src)) {
  throw "Missing build output: $src"
}

Copy-Item -Force $src $dst
Write-Host "Copied $src -> $dst"
