@echo off
rem Builds Discord Quest Assistant in release mode and moves the exe to this folder.
setlocal
cd /d "%~dp0"

rem A fresh install may not have Cargo on PATH until a new terminal is opened.
where cargo >nul 2>nul || set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
where cargo >nul 2>nul || (
    echo Cargo was not found. Install Rust from https://rustup.rs and try again.
    goto :fail
)

echo Building Discord Quest Assistant (release)...
cargo build --release || (
    echo The build failed.
    goto :fail
)

rem A running copy locks the exe, so it can't be replaced.
tasklist /fi "imagename eq discord-quest-assistant.exe" 2>nul | find /i "discord-quest-assistant.exe" >nul && (
    echo Discord Quest Assistant is running. Close it and run this again.
    goto :fail
)

move /y "target\release\discord-quest-assistant.exe" "discord-quest-assistant.exe" >nul || (
    echo Couldn't move the exe into this folder.
    goto :fail
)

echo Done: %~dp0discord-quest-assistant.exe
endlocal
exit /b 0

:fail
endlocal
pause
exit /b 1
