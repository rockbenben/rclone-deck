@echo off
rem Silent-install the freshly built NSIS package. Usage: tools\ui-install-silent.cmd [setup-exe]
rem Default target is the bundle npm run tauri build just produced.
rem
rem Why a .bat exists: from Git Bash a bare `/S` gets rewritten into a path, and the installer
rem then pops its GUI and writes nothing (measured twice on this machine). `start /wait` from the
rem shell is worse here - it returned "access denied" and installed nothing.
rem After this runs: check the process table (the NSIS stub may or may not auto-start the app),
rem and compare the installed exe against target\release\*.exe byte for byte - the only expected
rem difference is tauri's 3-byte bundle-type marker (UNK -> NSS).
if "%OS%"=="Windows_NT" setlocal
set SETUP=%~1
if "%SETUP%"=="" set SETUP=%~dp0..\src-tauri\target\release\bundle\nsis\rclone-deck_0.1.0_x64-setup.exe
if not exist "%SETUP%" (
  echo SETUP_MISSING=%SETUP%
  exit /b 2
)
echo SETUP=%SETUP%
"%SETUP%" /S
echo SETUP_EXIT=%ERRORLEVEL%
