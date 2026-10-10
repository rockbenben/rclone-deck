@echo off
rem Launch the panel against a FIXTURE home. Usage: tools\ui-launch.cmd A|B [exe-path]
rem Second arg picks the binary (e.g. src-tauri\target\release\rclone-deck.exe); default is debug.
rem
rem Why a .bat and not Start-Process: measured on this machine, Start-Process does NOT pass
rem RCLONE_DECK_HOME / RCLONE_CONFIG down to this exe, so the panel booted twice on the
rem operator's real config. The env must be set in the launching command's own environment.
rem First readout after launch (rule R68): rcd.pid appears inside the fixture dir, and the
rem top bar's log-dir line points at the fixture.
rem Comments stay ASCII: cmd reads a .bat in the OEM codepage, and UTF-8 Chinese here makes it
rem execute the rem lines as commands.
set UIHOME=%1
if "%UIHOME%"=="" set UIHOME=A
set UIEXE=%~2
if "%UIEXE%"=="" set UIEXE=%~dp0..\src-tauri\target\debug\rclone-deck.exe
rem Fixture home follows THIS machine's %TEMP% (it used to be the author's absolute path,
rem so the whole QA kit only ran on one box). RCLONE_DECK_HOME still wins if the caller set it.
if "%RCLONE_DECK_HOME%"=="" set RCLONE_DECK_HOME=%TEMP%\deck-ui-%UIHOME%
set RCLONE_CONFIG=%RCLONE_DECK_HOME%\conf\rclone.conf
rem rcd port: override it when the default is inside a Windows EXCLUDED port range.
rem Measured 2026-10-09: this box's dynamic reservation had moved to 5460-5659, which
rem swallowed the then-default 5599 - and "excluded" fails differently from "in use":
rem rclone rcd reports "bind: An attempt was made to access a socket in a way forbidden
rem by its access permissions", the panel never reaches a ready backend, and every driver
rem step then fails on "no task selected" - which reads like a broken UI, not a blocked
rem port. Same family as the CDP note below; the difference is that rcd's port cannot be
rem "let the OS pick" (0) because the number is written into the pid record and shown in
rem error copy, so this is an explicit opt-in instead.
rem
rem 2026-10-10: the hardcoded default is gone. That day the reservation had drifted to
rem 6383-6482, which swallows 6400 itself - so "6400" was no longer a default, it was a
rem broken value. free-rcd-port.ps1 now picks one on every launch.
rem Its rule is "try to bind it" - netsh's excluded-range table alone is NOT enough, and
rem getting that wrong cost a morning: it misses the dynamic port range (this box: 1024
rem start, 13977 ports, so 1024-15000) and it misses a port another process holds in the
rem Bound state (only Listen was being checked). Both look identical on screen - the same
rem WSAEACCES bind error - so a picker that guesses is a picker that lies.
rem An explicit RCLONE_DECK_PORT is still passed through untouched.
if "%RCLONE_DECK_PORT%"=="" for /f "usebackq delims=" %%p in (`powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0free-rcd-port.ps1"`) do set "RCLONE_DECK_PORT=%%p"
rem CDP port: ask the OS for a free one (0), do NOT pin 9222. Measured 2026-10-07: this box
rem reserves 9211-9310 (netsh interface ipv4 show excludedportrange protocol=tcp), so a pinned
rem 9222 is dropped in silence - the window opens, nothing listens, and the driver reports
rem "panel not up" while the real breakage is the borrowed port. Readers resolve the number
rem from %LOCALAPPDATA%\<identifier>\EBWebView\DevToolsActivePort (see tools/cdp-endpoint.mjs).
set WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=0
echo DECK_HOME=%RCLONE_DECK_HOME%
echo EXE=%UIEXE%
echo RCD_PORT=%RCLONE_DECK_PORT%
echo PORT_FILE=%LOCALAPPDATA%\top.aishort.rclone-deck\EBWebView\DevToolsActivePort
start "" "%UIEXE%"
