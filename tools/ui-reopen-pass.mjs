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
import { spawnSync } from 'node:child_process';
import { setTimeout as sleep } from 'node:timers/promises';
import { cdpEndpoint } from './cdp-endpoint.mjs';

const arg = (name, dflt) => {
  const hit = process.argv.find((a) => a.startsWith('--' + name + '='));
  return hit ? hit.split('=').slice(1).join('=') : dflt;
};
const fixtureHome = (arg('home', 'B') || 'B').toUpperCase();
const exe = arg('exe', 'src-tauri\\target\\release\\rclone-deck.exe');
const out = arg('out', 'docs/superpowers/evidence/ui/reopen-' + fixtureHome.toLowerCase());
fs.mkdirSync(out, { recursive: true });

// The acceptance driver reads the standard fixture home and must attach to its WebView.
delete process.env.RCLONE_DECK_HOME;
delete process.env.DECK_CDP_PORT;

const log = path.join(out, 'reopen-launch.log');
const fd = fs.openSync(log, 'w');
let restart;
try {
  restart = spawnSync('powershell.exe', [
    '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', 'tools/ui-fixture-panel.ps1',
    '-FixtureHome', fixtureHome, '-Exe', exe, '-Action', 'Restart',
  ], { stdio: ['ignore', fd, fd], windowsHide: true });
} finally {
  fs.closeSync(fd);
}
if (restart.error || restart.status !== 0) {
  console.error('REOPEN-FAILED', restart.error ?? fs.readFileSync(log, 'utf8'));
  process.exit(1);
}
console.log('PANEL-LAUNCHED', fs.readFileSync(log, 'utf8').trim());

// Reclaiming the fixture's daemon can delay WebView creation after the panel starts.
const fixture = path.join(os.tmpdir(), 'deck-ui-' + fixtureHome);
const deadline = Date.now() + 120_000;
let ready = false;
let lastError = '';
while (Date.now() < deadline) {
  try {
    const response = await fetch(cdpEndpoint(fixture), { signal: AbortSignal.timeout(2000) });
    const targets = await response.json();
    if (targets.some((t) => t.type === 'page' && t.url && t.url !== 'about:blank'
      && !t.url.startsWith('devtools://'))) {
      ready = true;
      break;
    }
    lastError = 'No application page on the fixture CDP endpoint';
  } catch (error) {
    lastError = String(error);
  }
  await sleep(1000);
}
if (!ready) {
  console.error('REOPEN-FAILED', lastError);
  process.exit(1);
}

// 五格各跑各的：`72` 要的是主跑最后那一格（`remember-what-was-open`）写下的"上次开在哪"，
//    所以它排最前 —— 后面那四格会换面，把那份偏好改写掉。
//    47/51 量的是"不热重载"的另一头（重开之后才换成新值），65/66 量的是临时卡散了、
//    存成常用的那条回来了。每一格自己的判据在驱动器里，这里只负责顺序与现场。
const RANGES = [['72', 'restart-keeps-face-and-tab'], ['47', 'size-only-bar-after-restart'],
  ['51', 'settings-reopened-shows-new-src'], ['65', 'draft-gone-after-restart'],
  ['66', 'draft-saved-card-back-after-restart']];
let failed = 0;
for (const [n, name] of RANGES) {
  const r = spawnSync(process.execPath, [
    'tools/ui-acceptance.mjs', `--home=${fixtureHome}`, '--reopen=1',
    `--from=${n}`, `--to=${n}`, `--out=${path.join(out, name)}`,
  ], { encoding: 'utf8' });
  const line = String(r.stdout || '').split('\n').filter(Boolean).pop() || '(没有汇总行)';
  console.log('STEP ' + n + ' ' + name + ' :: ' + line);
  if (r.error || r.status !== 0) {
    console.error(r.error ?? r.stderr);
    failed++;
  }
}
console.log(failed === 0 ? 'REOPEN-ALL-OK 五格全过' : 'REOPEN-RED ' + failed + ' 格红，读数在各格自己的目录里');
if (failed) process.exitCode = 1;
