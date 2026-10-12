// 验收链的第二遍：那五格 `needsReopen` 要的是**重开面板之后**才成立的事实。
// 为什么单独一遍：驱动器对 `needsReopen` 的处置是"这一遍没带 --reopen=1 就记 SKIP"，
// 而混在正常那一段里跑会绿在错的理由上（旧窗口读到旧值，恰好和"没重开"的预期一样）。
// 为什么不在这里重铺夹具：这一遍**吃的就是主跑留下的现场**（那一枚参数、那一条改过的位置、
// 那张存成常用的卡），重铺等于把要证明的东西擦掉。所以规矩是：先 `--home=X` 跑完主链，
// 紧接着跑这一条，中间不要铺夹具。
//
// 用法：node tools/ui-reopen-pass.mjs --home=B
// （npm 入口：`npm run accept:ui:reopen -- --home=B`；换 exe 用 `--exe=...`）
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawn, spawnSync } from 'node:child_process';

const arg = (name, dflt) => {
  const hit = process.argv.find((a) => a.startsWith('--' + name + '='));
  return hit ? hit.split('=').slice(1).join('=') : dflt;
};
const HOME = (arg('home', 'B') || 'B').toUpperCase();
const EXE = arg('exe', 'src-tauri\\target\\release\\rclone-deck.exe');
const OUT = arg('out', 'docs/superpowers/evidence/ui/reopen-' + HOME.toLowerCase());
fs.mkdirSync(OUT, { recursive: true });
const say = (s) => console.log(s);
const run = (cmd) => spawnSync(cmd, { shell: true, encoding: 'utf8' });
// 等待用 `Atomics.wait`：这个脚本的 shell 是 cmd.exe，那里没有 `sleep` 这条命令，
// 一条抛错会把整遍带走。
const waitMs = (ms) => Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms);
// 面板活没活，按**进程名**数，不拿 pid 文件里那个 `panel_pid` 去问：那份记录可能是上一个
// 面板留下的，拿旧 pid 问"活着吗"永远得到"没活"，于是脚本把一次成功的启动读成失败
// （第一版就是这么红的：等不到面板，五格全报"没有汇总行"，读数说的都是尺子）。
const panelCount = () => Number(String(run('powershell -NoProfile -Command'
  + ' "(Get-Process rclone-deck -ErrorAction SilentlyContinue).Count"').stdout || '0').trim());

// 1) 收掉上一遍留下的面板与它拉起的 rcd。
//    rcd 只按 pid 文件里那个 pid 杀（`ui-kill-rcd.ps1`），不按进程名广杀 —— 操作员自己
//    可能跑着一个 rclone，广杀会把他的后台一起收掉。
//    杀完等面板真的没了再起下一个：`taskkill /F` 是急杀，WebView2 那个 user-data 目录上的
//    锁要一会儿才松开，立刻起第二个会起不来。
run('taskkill /IM rclone-deck.exe /F');
say('KILL-RCD ' + String(run('powershell -NoProfile -ExecutionPolicy Bypass -File tools/ui-kill-rcd.ps1'
  + ' -FixtureHome ' + HOME).stdout || '').trim());
for (let i = 0; i < 20 && panelCount() > 0; i++) waitMs(1000);
say('GONE panel=' + panelCount());

// 2) 起面板。环境变量必须写在 .bat 自己的环境里（`Start-Process` 传不下去，实测过：
//    那样起的面板加载的是操作员那一份配置），所以走 `tools/ui-launch.cmd`。
//    形状是量出来的，不是抄的：`spawn('cmd.exe', ['/d','/s','/c', 串], {stdio:'ignore'})` 与
//    `spawn('cmd.exe', ['/c', 串], {stdio:'ignore'})` 两种都**连日志文件都没写出来**
//    （参数数组那条路上重定向被当成参数交下去），只有"整条命令交给 shell 一个字符串 +
//    `detached` + `stdio:'ignore'`"这一种把面板起来了。
//    不用 `spawnSync`：GUI 进程继承重定向那个文件句柄，`spawnSync` 要等它退出才返回 ——
//    实测一条"起面板"的命令挂了三分钟不回来。
const log = path.join('design-preview', 'reopen-launch.log');
const launch = () => {
  const kid = spawn('tools\\ui-launch.cmd ' + HOME + ' ' + EXE + ' > "' + log + '" 2>&1',
    { shell: true, detached: true, stdio: 'ignore' });
  kid.unref();
  return kid.pid;
};

// 3) 等面板起来；45 秒没等到就再按一次启动器（最多两趟）。急杀之后"那一次没起来"是这台
//    机器上真实存在的一档，重试一行就盖掉，而它留下的读数会让人以为界面坏了。
let ready = false;
for (let attempt = 1; attempt <= 2 && !ready; attempt++) {
  say('LAUNCH attempt=' + attempt + ' launcher-pid=' + launch());
  const t0 = Date.now();
  while (Date.now() - t0 < 45000) {
    waitMs(2000);
    if (panelCount() > 0) { ready = true; break; }
  }
}
if (!ready) {
  // 后面五格全都建立在"这是一个刚重开的面板"上。面板不在就停 —— 别让五格红去冒充五条发现。
  say('NO-PANEL :: 两趟都没等到面板，看 ' + log + '，五格没有跑');
  process.exit(1);
}
// 面板活着还不等于后台就绪：驱动器自己会再等 40 秒 CDP + 30 秒「后台就绪」。
// 这里只把现场报出来（那份 pid 记录此刻应当是这一趟新写的）。
let rec = null;
try {
  rec = JSON.parse(fs.readFileSync(path.join(os.tmpdir(), 'deck-ui-' + HOME, 'rcd.pid'), 'utf8'));
} catch (_) { /* 还没写出来：驱动器那一头会等，报一个 null 就够 */ }
say('PANEL-UP count=' + panelCount() + ' rcd=' + JSON.stringify(rec || {}));

// 4) 五格各跑各的：`72` 要的是主跑最后那一格（`remember-what-was-open`）写下的"上次开在哪"，
//    所以它排最前 —— 后面那四格会换面，把那份偏好改写掉。
//    47/51 量的是"不热重载"的另一头（重开之后才换成新值），65/66 量的是临时卡散了、
//    存成常用的那条回来了。每一格自己的判据在驱动器里，这里只负责顺序与现场。
const RANGES = [['72', 'restart-keeps-face-and-tab'], ['47', 'size-only-bar-after-restart'],
  ['51', 'settings-reopened-shows-new-src'], ['65', 'draft-gone-after-restart'],
  ['66', 'draft-saved-card-back-after-restart']];
let failed = 0;
for (const [n, name] of RANGES) {
  const dir = path.join(OUT, name).replace(/\\/g, '/');
  const r = run('node tools/ui-acceptance.mjs --home=' + HOME + ' --reopen=1 --from=' + n
    + ' --to=' + n + ' --out=' + dir);
  const line = String(r.stdout || '').split('\n').filter(Boolean).pop() || '(没有汇总行)';
  say('STEP ' + n + ' ' + name + ' :: ' + line);
  if (r.status !== 0) failed++;
}
say(failed === 0 ? 'REOPEN-ALL-OK 五格全过' : 'REOPEN-RED ' + failed + ' 格红，读数在各格自己的目录里');
if (failed) process.exitCode = 1;
