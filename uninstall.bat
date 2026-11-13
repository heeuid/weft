@echo off
rem Remove weft installed by install.bat: the program, its shortcuts, its
rem entry in Settings > Apps and on PATH. Your data (settings, memos,
rem history, logs in data\) is kept unless you say to delete it.
rem
rem   uninstall.bat                 from %LOCALAPPDATA%\Programs\weft
rem   uninstall.bat D:\Apps\weft    from where you installed it
rem   ... purge                     also delete the data, without asking
rem   ... keep                      keep the data, without asking
setlocal EnableExtensions

set "DEST="
set "DATA_CHOICE="
set "ANSWER="
:args
if "%~1"=="" goto args_done
if /i "%~1"=="purge" (set "DATA_CHOICE=delete") else if /i "%~1"=="keep" (set "DATA_CHOICE=keep") else set "DEST=%~f1"
shift
goto args
:args_done
rem Run from the installed folder (Settings > Apps does): that folder.
if not defined DEST if exist "%~dp0weft.exe" if not exist "%~dp0Cargo.toml" set "DEST=%~dp0."
if not defined DEST set "DEST=%LOCALAPPDATA%\Programs\weft"
for %%d in ("%DEST%") do set "DEST=%%~fd"

rem This file can't delete the folder it runs from (nor can a cmd whose
rem current folder it is): run a copy from TEMP, in TEMP.
if /i "%~dp0"=="%DEST%\" if not defined WEFT_UNINSTALL_COPY (
    copy /y "%~f0" "%TEMP%\weft-uninstall.bat" >nul
    set "WEFT_UNINSTALL_COPY=1"
    cd /d "%TEMP%"
    "%TEMP%\weft-uninstall.bat" "%DEST%" %DATA_CHOICE%
    exit /b
)
cd /d "%TEMP%"

if not exist "%DEST%\weft.exe" (
    echo weft: not installed in %DEST%
    echo       ^(give the folder: uninstall.bat D:\path\to\weft^)
    pause
    exit /b 1
)

rem weft must be closed: Windows won't delete a running program.
set "WEFT_DEST=%DEST%"
:running
powershell -NoProfile -NoLogo -Command "$d = $env:WEFT_DEST.TrimEnd('\') + '\'; if (Get-Process -ErrorAction SilentlyContinue | Where-Object { $_.Path -and $_.Path.StartsWith($d, 'OrdinalIgnoreCase') }) { exit 1 }"
if errorlevel 1 (
    echo weft is running. Close every weft window, then press a key ^(Ctrl+C to stop^).
    pause >nul
    goto running
)

if not defined DATA_CHOICE (
    if exist "%DEST%\data" (
        echo Your weft data is in %DEST%\data ^(settings, memos, history, logs^).
        set /p "ANSWER=Delete it too? [y/N] "
    )
)
if not defined DATA_CHOICE (
    set "DATA_CHOICE=keep"
    if /i "%ANSWER%"=="y" set "DATA_CHOICE=delete"
    if /i "%ANSWER%"=="yes" set "DATA_CHOICE=delete"
)

echo Removing weft from %DEST%
powershell -NoProfile -NoLogo -ExecutionPolicy Bypass -Command ^
  "$dest = $env:WEFT_DEST;" ^
  "foreach ($f in [Environment]::GetFolderPath('Programs'), [Environment]::GetFolderPath('Desktop')) { $l = Join-Path $f 'weft.lnk'; if (Test-Path $l) { Remove-Item $l -Force; Write-Host ('  removed ' + $l) } }" ^
  "Remove-Item 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\weft' -Recurse -Force -ErrorAction SilentlyContinue;" ^
  "$p = [Environment]::GetEnvironmentVariable('Path', 'User');" ^
  "if ($p) { $parts = @($p -split ';' | Where-Object { $_ }); $kept = @($parts | Where-Object { $_.TrimEnd('\') -ne $dest.TrimEnd('\') }); if ($kept.Count -ne $parts.Count) { [Environment]::SetEnvironmentVariable('Path', ($kept -join ';'), 'User'); Write-Host '  taken off PATH' } }"

del /q "%DEST%\weft.exe" "%DEST%\weft-old-*.exe" "%DEST%\uninstall.bat" >nul 2>&1
if /i "%DATA_CHOICE%"=="delete" (
    rmdir /s /q "%DEST%" 2>nul
    echo   data deleted
) else (
    rem Only an empty folder goes; data\ stays where it was.
    rmdir "%DEST%" 2>nul
    if exist "%DEST%\data" echo   data kept in %DEST%\data ^(install.bat again picks it up^)
)
if exist "%DEST%\weft.exe" (
    echo weft: %DEST%\weft.exe could not be deleted.
    pause
    exit /b 1
)
echo weft is uninstalled.
if defined WEFT_UNINSTALL_COPY (
    rem Started from Settings ^> Apps: leave the window up long enough to read.
    timeout /t 4 >nul
    rem This copy in TEMP deletes itself last.
    (goto) 2>nul & del "%~f0"
)
endlocal
exit /b 0
