@echo off
rem Start weft on Windows: double-click this file, or run `weft.bat` in a
rem terminal. weft runs on its own; closing the terminal doesn't close it.
rem When cargo is installed, the release build is brought up to date first.
setlocal
cd /d "%~dp0"
set "BUILT=%~dp0target\release\weft.exe"

rem A weft started from target\release itself (not from a copy, as below)
rem keeps weft.exe from being replaced, and cargo then fails with "Access is
rem denied". Windows does let a running .exe be renamed: move it aside.
del /q "%~dp0target\release\weft-old-*.exe" >nul 2>&1
if exist "%BUILT%" (
    2>nul (>>"%BUILT%" (call )) || move /y "%BUILT%" "%~dp0target\release\weft-old-%RANDOM%.exe" >nul
)

where cargo >nul 2>&1
if not errorlevel 1 (
    cargo build --release
    if errorlevel 1 (
        echo.
        echo weft: the build failed, see above.
        echo       "Access is denied" on weft.exe: antivirus may be scanning the
        echo       new file; run this again in a moment.
        pause
        exit /b 1
    )
)

if not exist "%BUILT%" (
    echo weft: target\release\weft.exe not found. Install Rust from https://rustup.rs and run this again.
    pause
    exit /b 1
)

rem weft runs from a copy in target\run, so the build output is never in use:
rem Windows won't replace a running .exe, and `cargo build` can now run while
rem weft is open (even inside it). Copies still running are skipped; the
rem others are removed. Data stays in target\release\data, as before.
set "RUN=%~dp0target\run"
if not exist "%RUN%" mkdir "%RUN%"
del /q "%RUN%\weft-*.exe" >nul 2>&1
set "WEFT_EXE="
for %%i in (1 2 3 4 5 6 7 8 9) do (
    if not defined WEFT_EXE (
        copy /y "%BUILT%" "%RUN%\weft-%%i.exe" >nul 2>&1 && set "WEFT_EXE=%RUN%\weft-%%i.exe"
    )
)
if not defined WEFT_EXE set "WEFT_EXE=%BUILT%"
if not defined WEFT_DATA_DIR set "WEFT_DATA_DIR=%~dp0target\release\data"
rem For "Restart weft" (and its "new weft" button): run this file again,
rem and watch the build output for a newer weft.
set "WEFT_LAUNCHER=%~f0"
set "WEFT_BUILT=%BUILT%"

rem Started like a double-click in Explorer (ShellExecute): weft inherits
rem nothing from this console, so the terminal that ran this can close.
powershell -NoProfile -NoLogo -Command "Start-Process -FilePath $env:WEFT_EXE"
