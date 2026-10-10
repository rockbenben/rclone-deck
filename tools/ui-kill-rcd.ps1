# 这一格跑在 HOME=A 上（`list-failure-visible` 是那一档独有的），但 pid 文件那一行曾被写死成 A ——
# 于是任何一次对 B 跑整套时，它要么找不到文件、要么杀掉**另一套家的** daemon，
# 报错读起来像"停不掉后台"，而停不掉的是路径不是进程。家从参数取，与 `ui-launch.cmd` 同一份。
#
# 参数名不叫 `$Home`：那是 PowerShell 的**只读自动变量**（用户主目录），拿它当参数名会在
# 绑定时报 `VariableNotWritable`（2026-10-09 实测，那句报错的读法与"停不掉后台"毫无关系）。
param([string]$FixtureHome = 'A')
# 夹具家跟着**这台机器的** %TEMP% 走（原来抄的是作者那台的一条绝对路径，别人跑只会得到 NO-PID-FILE）。
$pidFile = Join-Path $env:TEMP ("deck-ui-{0}\rcd.pid" -f $FixtureHome)
if (-not (Test-Path $pidFile)) { 'NO-PID-FILE'; exit 1 }
$j = Get-Content $pidFile -Raw | ConvertFrom-Json
$p = $j.pid
$me = Get-Process -Id $p -ErrorAction SilentlyContinue
if ($null -eq $me) { "ALREADY-GONE pid=$p"; exit 0 }
if ($me.Name -ne 'rclone') { "WRONG-PROCESS pid=$p name=$($me.Name)"; exit 1 }
Stop-Process -Id $p -Force
"KILLED pid=$p port=$($j.port)"
