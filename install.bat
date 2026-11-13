@echo off
rem Install weft for this Windows user (no administrator rights needed).
rem
rem   install.bat                 into %LOCALAPPDATA%\Programs\weft
rem   install.bat D:\Apps\weft    somewhere else
rem   ... desktop                 also a desktop shortcut
rem   ... path                    also put weft on your PATH (new terminals)
rem
rem weft is one .exe (shell scripts and the remote agent are built in); its
rem data (settings, memos, history, logs) goes in data\ next to it. Run this
rem again to update: weft may be running, it picks up the new build when it
rem is next started. uninstall.bat (or Settings > Apps) removes it.
setlocal EnableExtensions
cd /d "%~dp0"

set "DEST=%LOCALAPPDATA%\Programs\weft"
set "WANT_DESKTOP="
set "WANT_PATH="
:args
if "%~1"=="" goto args_done
if /i "%~1"=="desktop" (set "WANT_DESKTOP=1") else if /i "%~1"=="path" (set "WANT_PATH=1") else set "DEST=%~f1"
shift
goto args
:args_done

rem The program: built here when cargo is installed, else a weft.exe that
rem came next to this script (a release download) or an earlier build.
set "SRC="
where cargo >nul 2>&1
if not errorlevel 1 if exist "%~dp0Cargo.toml" (
    rem A running weft.exe in target\release can't be replaced, but it can be renamed.
    del /q "%~dp0target\release\weft-old-*.exe" >nul 2>&1
    if exist "%~dp0target\release\weft.exe" (
        2>nul (>>"%~dp0target\release\weft.exe" (call )) || move /y "%~dp0target\release\weft.exe" "%~dp0target\release\weft-old-%RANDOM%.exe" >nul
    )
    cargo build --release
    if errorlevel 1 (
        echo.
        echo weft: the build failed, see above. Nothing was installed.
        pause
        exit /b 1
    )
)
if exist "%~dp0target\release\weft.exe" set "SRC=%~dp0target\release\weft.exe"
if not defined SRC if exist "%~dp0weft.exe" set "SRC=%~dp0weft.exe"
if not defined SRC (
    echo weft: no weft.exe to install. Install Rust from https://rustup.rs and run this again.
    pause
    exit /b 1
)

set "VERSION=0"
if exist "%~dp0Cargo.toml" (
    for /f "tokens=2 delims== " %%v in ('findstr /b /c:"version" "%~dp0Cargo.toml"') do if "%VERSION%"=="0" set "VERSION=%%~v"
)

echo Installing weft %VERSION% into %DEST%
if not exist "%DEST%" mkdir "%DEST%" || (echo weft: can't create %DEST% & pause & exit /b 1)

rem A weft started from the installed copy keeps weft.exe from being
rem replaced: move it aside (it runs on), and clear those left from before.
del /q "%DEST%\weft-old-*.exe" >nul 2>&1
if exist "%DEST%\weft.exe" (
    2>nul (>>"%DEST%\weft.exe" (call )) || (
        move /y "%DEST%\weft.exe" "%DEST%\weft-old-%RANDOM%.exe" >nul
        echo   weft is running: the new version starts next time you open weft.
    )
)
copy /y "%SRC%" "%DEST%\weft.exe" >nul || (echo weft: copying weft.exe failed & pause & exit /b 1)
if exist "%~dp0uninstall.bat" if /i not "%~dp0"=="%DEST%\" copy /y "%~dp0uninstall.bat" "%DEST%\uninstall.bat" >nul

rem Data from weft.bat (target\release\data), the first time only.
if not exist "%DEST%\data\settings.json" if exist "%~dp0target\release\data\settings.json" (
    echo   copying your data from target\release\data
    xcopy "%~dp0target\release\data" "%DEST%\data\" /e /i /q /y >nul
)

rem Shortcuts, the entry in Settings > Apps, and PATH. Paths reach
rem PowerShell through the environment, never through this file's code page.
set "WEFT_DEST=%DEST%"
set "WEFT_VERSION=%VERSION%"
set "WEFT_DESKTOP=%WANT_DESKTOP%"
set "WEFT_PATH=%WANT_PATH%"
powershell -NoProfile -NoLogo -ExecutionPolicy Bypass -Command ^
  "$ErrorActionPreference = 'Stop';" ^
  "$dest = $env:WEFT_DEST; $exe = Join-Path $dest 'weft.exe';" ^
  "$shell = New-Object -ComObject WScript.Shell;" ^
  "function Link($at) { $l = $shell.CreateShortcut($at); $l.TargetPath = $exe; $l.WorkingDirectory = $dest; $l.IconLocation = $exe; $l.Description = 'weft terminal'; $l.Save() }" ^
  "Link (Join-Path ([Environment]::GetFolderPath('Programs')) 'weft.lnk'); Write-Host '  Start menu shortcut';" ^
  "if ($env:WEFT_DESKTOP) { Link (Join-Path ([Environment]::GetFolderPath('Desktop')) 'weft.lnk'); Write-Host '  desktop shortcut' }" ^
  "if ($env:WEFT_PATH) { $p = [Environment]::GetEnvironmentVariable('Path', 'User'); $parts = @($p -split ';' | Where-Object { $_ }); if ($parts -notcontains $dest) { [Environment]::SetEnvironmentVariable('Path', (($parts + $dest) -join ';'), 'User') }; Write-Host '  on your PATH (in terminals opened from now on)' }" ^
  "$key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\weft';" ^
  "New-Item -Path $key -Force | Out-Null;" ^
  "$kb = [int]((Get-ChildItem $dest -File | Measure-Object Length -Sum).Sum / 1KB);" ^
  "$v = @{ DisplayName = 'weft'; DisplayVersion = $env:WEFT_VERSION; DisplayIcon = $exe; InstallLocation = $dest; UninstallString = ('cmd.exe /c \"' + (Join-Path $dest 'uninstall.bat') + '\"'); Publisher = 'weft' };" ^
  "foreach ($n in $v.Keys) { Set-ItemProperty -Path $key -Name $n -Value $v[$n] };" ^
  "foreach ($n in 'NoModify','NoRepair') { Set-ItemProperty -Path $key -Name $n -Value 1 -Type DWord };" ^
  "Set-ItemProperty -Path $key -Name EstimatedSize -Value $kb -Type DWord;" ^
  "Write-Host '  listed in Settings > Apps'"
if errorlevel 1 (
    echo weft: weft.exe is installed, but shortcuts or the Apps entry failed ^(see above^).
    pause
    exit /b 1
)

echo.
echo weft is installed: Start menu ^> weft, or "%DEST%\weft.exe".
echo Remove it with uninstall.bat, or in Settings ^> Apps.
endlocal
exit /b 0
