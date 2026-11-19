# Start or reopen a panel belonging to an acceptance fixture.
param(
    [ValidateSet('A', 'B')][string]$FixtureHome = 'A',
    [string]$Exe,
    [ValidateSet('Start', 'Restart', 'Stop')][string]$Action = 'Start'
)
$ErrorActionPreference = 'Stop'
if (!$Exe) { $Exe = Join-Path $PSScriptRoot '..\src-tauri\target\debug\rclone-deck.exe' }

$fixture = if ($env:RCLONE_DECK_HOME) {
    [IO.Path]::GetFullPath($env:RCLONE_DECK_HOME)
} else {
    Join-Path $env:TEMP "deck-ui-$FixtureHome"
}
$config = Join-Path $fixture 'conf\rclone.conf'
$profile = Join-Path $fixture 'webview'
$portFile = Join-Path $profile 'EBWebView\DevToolsActivePort'
$record = Join-Path $fixture 'ui-panel.json'

$lock = [IO.File]::Open((Join-Path $fixture 'ui-panel.lock'), 'OpenOrCreate', 'ReadWrite', 'None')
try {
    if ($Action -ne 'Stop') {
        $image = (Get-Item -LiteralPath $Exe).FullName
        foreach ($file in @((Join-Path $fixture 'jobs.toml'), $config)) {
            if (!(Test-Path -LiteralPath $file -PathType Leaf)) { throw "Missing fixture file $file" }
        }
    }
    $previous = if (Test-Path -LiteralPath $record) {
        Get-Content -LiteralPath $record -Raw | ConvertFrom-Json
    }
    if (!$previous -and $Action -eq 'Restart') {
        throw "Missing launch record $record. Start the fixture with tools\ui-launch.cmd and run the first pass."
    }
    if ($previous) {
        $panelId = 0
        if (![int]::TryParse([string]$previous.pid, [ref]$panelId) -or $panelId -le 0) {
            throw "Invalid launch record $record"
        }
        $panel = Get-Process -Id $panelId -ErrorAction SilentlyContinue
        if ($panel) {
            # Keep the process handle open while checking identity and requesting close.
            $null = $panel.Handle
            if (!$panel.HasExited) {
                if ($panel.Path -ne $previous.exe -or
                    [string]$panel.StartTime.ToUniversalTime().Ticks -ne $previous.started) {
                    throw "PID $panelId does not match the fixture launch record $record"
                }
                if ($Action -eq 'Start') { throw "Fixture panel $panelId is already running" }
                if (!$panel.CloseMainWindow() -and !$panel.HasExited) {
                    throw "The fixture panel has no window available to close"
                }
                if (!$panel.WaitForExit(20000)) { throw "Fixture panel $panelId did not close within 20 seconds" }
            }
        }
    }
    if ($Action -eq 'Stop') { return }

    $env:RCLONE_DECK_HOME = $fixture
    $env:RCLONE_CONFIG = $config
    $env:WEBVIEW2_USER_DATA_FOLDER = $profile
    $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=0'
    if (!$env:RCLONE_DECK_PORT) {
        # If a fixture daemon survived shutdown, let the app verify and reclaim it on its old port.
        $daemonFile = Join-Path $fixture 'rcd.pid'
        $daemon = if (Test-Path -LiteralPath $daemonFile) {
            Get-Content -LiteralPath $daemonFile -Raw | ConvertFrom-Json
        }
        if ($previous -and $daemon -and $daemon.panel_pid -eq $previous.pid) {
            $env:RCLONE_DECK_PORT = [string]$daemon.port
        } else {
            $env:RCLONE_DECK_PORT = & (Join-Path $PSScriptRoot 'free-rcd-port.ps1')
        }
    }
    if (Test-Path -LiteralPath $portFile) { Remove-Item -LiteralPath $portFile }

    # Redirect the child away from the caller's pipes so the launcher can return
    # while the fixture window stays open.
    $child = Start-Process -FilePath $image -WorkingDirectory (Get-Location).Path -PassThru `
        -RedirectStandardOutput (Join-Path $fixture 'ui-panel.stdout.log') `
        -RedirectStandardError (Join-Path $fixture 'ui-panel.stderr.log')
    $startedImage = $child.MainModule.FileName
    $identity = @{
        pid = $child.Id
        exe = if ($startedImage) { $startedImage } else { $image }
        started = [string]$child.StartTime.ToUniversalTime().Ticks
    } | ConvertTo-Json -Compress
    [IO.File]::WriteAllText($record, $identity)
    "PANEL_PID=$($child.Id)"
    "DECK_HOME=$fixture"
    "RCD_PORT=$env:RCLONE_DECK_PORT"
    "PORT_FILE=$portFile"
}
finally {
    $lock.Dispose()
}

