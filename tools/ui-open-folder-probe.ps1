param([Parameter(Mandatory = $true)][string]$Path)
# 验收用：问一句"资源管理器里有没有这一层"，并把我们自己开的那一格收掉。
# 为什么走 Shell.Application 而不是枚举窗口标题：explorer 的窗口由 shell 进程持有，
# 标题会随视图设置变（显示扩展名与否），而 Folder.Self.Path 就是那一条路径本身。
$ErrorActionPreference = 'Stop'

# 两条路径**都要**过一遍正规化（2026-10-10 加的）。
#
# 原来只对传进来的那一侧 Resolve-Path，而那一侧来自驱动的 %TEMP% —— 本机它是
# **8.3 短名**（C:\Users\ADMINI~1\...）；资源管理器报回来的 Folder.Self.Path 却是**长名**
# （C:\Users\Administrator\...）。两边 TrimEnd 之后仍然不相等，于是
# HITS 恒为 0，而 OPEN 里**明明列着那个窗口**。
#
# 实测症状：`settings-open-conf-folder` 报「按钮没开出去」，可同一份读数里 OPEN
# 一次列出了三个窗口、全指着那一层 —— 产品开出去了，尺子认不出自己开的那一个。
# 而"没开出去"与"开了但没认出"读起来一模一样，红的当然是尺子。
#
# 正规化走 Get-Item 的 .FullName：短名进去出长名，长名进去原样出来，两边就此对齐。
function Get-CanonicalPath([string]$p) {
  if (-not $p) { return '' }
  try { return (Get-Item -LiteralPath $p -Force).FullName.TrimEnd('\') }
  catch { return $p.TrimEnd('\') }
}

$needle = Get-CanonicalPath (Resolve-Path -LiteralPath $Path).Path
$shell = New-Object -ComObject Shell.Application
$paths = @()
$closed = 0
foreach ($w in $shell.Windows()) {
  try {
    $p = $w.Document.Folder.Self.Path
  } catch {
    continue
  }
  if (-not $p) { continue }
  $norm = Get-CanonicalPath $p
  $paths += $norm
  if ($norm -eq $needle) {
    $closed++
    $w.Quit()
  }
}
Write-Output ("TARGET=" + $needle)
Write-Output ("OPEN=" + ($paths -join ';'))
Write-Output ("HITS=" + $closed)
