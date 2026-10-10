# 挑一个 rcd 用的端口：**真去绑一下**看能不能绑。
#
# 为什么必须真绑（2026-10-10 实测两次翻车）：
#   第一版只读 `netsh interface ipv4 show excludedportrange` —— 那只是**两套机制里的第一套**。
#   它漏掉的两种：
#     ① **动态（临时）端口范围**：`netsh int ipv4 show dynamicport tcp` 在这台机上是
#        **1024 起、13977 个**（不是默认的 49152 起、16384 个），也就是 1024-15000 整段
#        随时可能被系统抓去当临时端口。排除区表里根本没有它。
#     ② **别的进程占着**：6483 被 pid 5512 占着，状态是 `Bound`/`Established` 而不是
#        `Listen` —— 只查 `State -eq 'Listen'` 会把它当成空的。
#   于是第一版挑了 6483：07:19 还能绑，08:46 同一个号就被占了，而两个症状长得一模一样
#   （`bind: ... access permissions`，WSAEACCES），读起来像界面坏了。
#
# 所以判据换成**唯一的事实来源：这个号现在绑不绑得上**。netsh 那两条只用来**收窄候选窗口**
# 与在最后解释"为什么一个都没挑出来"，不参与判定。
#
# 竞态仍在：挑完到 rcd 真去绑之间还可能被抢（本机 07:19→08:46 就是这么发生的）。
# 抢了的话面板顶栏会按那套完整话术说「端口被占」，不该由这个脚手架替人重试。
#
# 显式设了 RCLONE_DECK_PORT 就原样交出去，不猜、不改 —— 那是要被尊重的。
#
# 用法：powershell -NoProfile -File tools\free-rcd-port.ps1
# 输出：一个端口号（纯文本一行，给 for /f 吃）

$ErrorActionPreference = 'Stop'

if ($env:RCLONE_DECK_PORT) { Write-Output $env:RCLONE_DECK_PORT; exit 0 }

# ---- 收窄窗口用（不参与判定）----
function Get-Excluded([string]$cmd) {
    $out = @()
    foreach ($l in (Invoke-Expression $cmd 2>$null)) {
        if ($l -match '^\s*(\d+)\s+(\d+)\s*\*?\s*$') { $out += , @([int]$Matches[1], [int]$Matches[2]) }
    }
    return $out
}
$excluded = @(Get-Excluded 'netsh interface ipv4 show excludedportrange protocol=tcp')

# 动态端口范围：起 + 个数。避开它，否则挑中的号随时会被系统抓去当临时端口。
$dynStart = $null; $dynCount = 0
foreach ($l in (netsh int ipv4 show dynamicport tcp 2>$null)) {
    if ($l -match 'Start Port\s*:\s*(\d+)') { $dynStart = [int]$Matches[1] }
    if ($l -match 'Number of Ports\s*:\s*(\d+)') { $dynCount = [int]$Matches[1] }
}
$dynEnd = if ($null -ne $dynStart) { $dynStart + $dynCount - 1 } else { $null }

# ---- 唯一的判据：绑得上吗 ----
function Test-Bindable([int]$port) {
    $l = $null
    try {
        $l = New-Object Net.Sockets.TcpListener ([Net.IPAddress]::Parse('127.0.0.1'), $port)
        $l.Start()
        return $true
    } catch { return $false }
    finally { if ($l) { try { $l.Stop() } catch {} } }
}

# 候选窗口 15000-49000：躲开这台机的动态段（1024-15000），也躲开 49152 起那段默认临时区。
# 两头各留余量，因为 rcd 的端口要写进 pid 记录并在错误话术里显示，不能"让系统随便挑"。
foreach ($p in 15000..49000) {
    if ($null -ne $dynStart -and $p -ge $dynStart -and $p -le $dynEnd) { continue }
    $skip = $false
    foreach ($r in $excluded) { if ($p -ge $r[0] -and $p -le $r[1]) { $skip = $true; break } }
    if ($skip) { continue }
    if (Test-Bindable $p) { Write-Output $p; exit 0 }
}

# 一个都没挑出来：交回面板的默认值，让它按那套完整话术去说
# （静默挑一个"大概是空的"在这里比失败更坏 —— 操作员看到的会是别的原因）。
Write-Output 6400
exit 0
