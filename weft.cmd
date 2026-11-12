@echo off
rem Start weft on Windows: double-click this file, or run `weft.cmd` in a
rem terminal. weft runs on its own; closing the terminal doesn't close it.
rem When cargo is installed, the release build is brought up to date first.
setlocal
cd /d "%~dp0"

where cargo >nul 2>&1
if not errorlevel 1 (
    cargo build --release
    if errorlevel 1 (
        echo.
        echo weft: the build failed, see above.
        pause
        exit /b 1
    )
)

set "WEFT_EXE=%~dp0target\release\weft.exe"
if not exist "%WEFT_EXE%" (
    echo weft: target\release\weft.exe not found. Install Rust from https://rustup.rs and run this again.
    pause
    exit /b 1
)

rem Started like a double-click in Explorer (ShellExecute): weft inherits
rem nothing from this console, so the terminal that ran this can close.
powershell -NoProfile -NoLogo -Command "Start-Process -FilePath $env:WEFT_EXE"
