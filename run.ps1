# Add Cargo to PATH for this session if it exists
$cargoBin = "$env:USERPROFILE\.cargo\bin"
if (Test-Path $cargoBin) {
    $env:Path = "$cargoBin;$env:Path"
}

$exe = "target\release\jamb.exe"

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Write-Host "Rust/cargo not found. Install from https://rustup.rs/ then run this script again."
    Read-Host "Press Enter to exit"
    exit 1
}

Write-Host "Building..."
cargo build --release
if ($LASTEXITCODE -ne 0) {
    Write-Host "Build failed."
    Read-Host "Press Enter to exit"
    exit 1
}

Write-Host "Running..."
& ".\$exe"
