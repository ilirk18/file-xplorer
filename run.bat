@echo off
setlocal
set "CARGO=%USERPROFILE%\.cargo\bin\cargo.exe"
set "EXE=target\release\jamb.exe"

if not exist "%CARGO%" (
    echo Rust not found at %CARGO%
    echo Install from https://rustup.rs/ then run this script again.
    pause
    exit /b 1
)

echo Building...
"%CARGO%" build --release
if errorlevel 1 (
    echo Build failed.
    pause
    exit /b 1
)

echo Running...
"%EXE%"
pause
