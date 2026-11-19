@echo off
rem Usage: tools\ui-launch.cmd A^|B [exe-path]
setlocal
set "FIXTURE_HOME=%~1"
if not defined FIXTURE_HOME set "FIXTURE_HOME=A"
if "%~2"=="" (
  powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0ui-fixture-panel.ps1" -FixtureHome "%FIXTURE_HOME%"
) else (
  powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0ui-fixture-panel.ps1" -FixtureHome "%FIXTURE_HOME%" -Exe "%~2"
)
exit /b %ERRORLEVEL%
