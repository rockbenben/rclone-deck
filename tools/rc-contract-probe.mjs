// rc-contract-probe — re-verify every rclone rc fact the rclone-deck spec relies on.
//
// Run after any rclone upgrade, before touching panel code:
//   npm run probe:rc          (uses a fresh temp dir)
//   node tools/rc-contract-probe.mjs <empty-temp-dir>
//
// Local-to-local only: no cloud remote is ever contacted. Every assertion prints
// PASS/FAIL with the measured value, and the exit code is the number of failures.
import { spawn, execFileSync } from 'node:child_process';
import { mkdirSync, writeFileSync, readFileSync, existsSync, readdirSync, rmSync, mkdtempSync, utimesSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const RCLONE = process.env.RCLONE_BIN || 'rclone';
const givenDir = process.argv[2];
const rawDir = givenDir || mkdtempSync(join(tmpdir(), 'rc-contract-probe-'));
const BASE = execFileSync('cygpath', ['-w', '-l', rawDir]).toString().trim();

// A run that dies halfway (a thrown assertion, a lost rclone) used to leave its whole fixture
// behind — 88 of these accumulated in three days of re-running the probe. Clean up on EVERY exit
// path, but only when the directory is ours: an operator who passed a path explicitly owns it and
// may be about to inspect it.
if (!givenDir) {
  process.on('exit', () => {
    // A swallowed cleanup failure is exactly how the 88-directory leak happened, so it gets a line.
    try {
      rmSync(rawDir, { recursive: true, force: true });
    } catch (e) {
      console.log(`FAIL  probe fixture was NOT cleaned up — delete it by hand: ${rawDir} (${e.message})`);
    }
  });
}

let failures = 0;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function check(label, actual, expected) {
  const ok = JSON.stringify(actual) === JSON.stringify(expected);
  if (!ok) failures++;
  console.log(`${ok ? 'PASS' : 'FAIL'}  ${label}\n        measured=${JSON.stringify(actual)} expected=${JSON.stringify(expected)}`);
}
function checkBool(label, actual, expected) { check(label, actual, expected); }

function fixture(name) {
  const root = BASE + '\\' + name;
  rmSync(root, { recursive: true, force: true });
  mkdirSync(root + '\\src\\imgprompt\\2026', { recursive: true });
  mkdirSync(root + '\\dst', { recursive: true });
  return root;
}

// ---------------------------------------------------------------- rcd lifecycle
let port = 5700;
async function startRcd(root, extraArgs = [], creds = { user: 'u', pass: 'p' }) {
  const p = ++port;
  const auth = 'Basic ' + Buffer.from(`${creds.user}:${creds.pass}`).toString('base64');
  const child = spawn(RCLONE, ['rcd', '--rc-addr', `127.0.0.1:${p}`, ...extraArgs], {
    stdio: ['ignore', 'ignore', 'pipe'],
    env: { ...process.env, RCLONE_RC_USER: creds.user, RCLONE_RC_PASS: creds.pass },
  });
  let stderr = '';
  child.stderr.on('data', (d) => { stderr += d; });
  for (let i = 0; i < 80; i++) {
    await sleep(200);
  // 探活必须是**需要鉴权**的方法。rc/noop 免鉴权：拿它探活的话"凭据没生效"这一档会被读成"就绪"，
  // 整轮探针拿着 403 当测量值（F10/I8 事故就是这条）。
    try { if ((await raw(p, 'config/listremotes', {}, auth)).status === 200) break; } catch {}
  }
  return { child, port: p, auth, stderr: () => stderr };
}

async function raw(p, method, params, auth) {
  const body = new URLSearchParams();
  for (const [k, v] of Object.entries(params)) body.append(k, String(v));
  const res = await fetch(`http://127.0.0.1:${p}/${method}`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/x-www-form-urlencoded', Authorization: auth },
    body,
  });
  const text = await res.text();
  let json; try { json = JSON.parse(text); } catch { json = text; }
  return { status: res.status, json };
}
const call = (ctx, method, params = {}) => raw(ctx.port, method, params, ctx.auth);

async function runJob(ctx, method, params) {
  const sub = await call(ctx, method, params);
  if (sub.status !== 200 || !sub.json.jobid) return { sub, st: null, stats: null };
  let st = {};
  for (let i = 0; i < 100; i++) {
    await sleep(150);
    st = (await call(ctx, 'job/status', { jobid: sub.json.jobid })).json;
    if (st.finished) break;
  }
  const stats = (await call(ctx, 'core/stats', { group: `job/${sub.json.jobid}` })).json;
  return { sub, st, stats };
}

const version = execFileSync(RCLONE, ['version']).toString().split('\n')[0].trim();
console.log(`# rc contract probe against ${version}`);
console.log(`# fixture base: ${BASE}\n`);

// ---------------------------------------------------------- 1. auth & credentials
{
  const root = fixture('auth');
  writeFileSync(root + '\\src\\a.txt', 'a');
  const PASS = 'S3cr3t-probe-pass';
  const ctx = await startRcd(root, ['--log-file', root + '\\env.log', '--log-level', 'DEBUG'], { user: 'probeuser', pass: PASS });
  check('unauthenticated call is rejected', (await fetch(`http://127.0.0.1:${ctx.port}/rc/noop`, { method: 'POST' })).status, 401);
  check('wrong password is rejected', (await fetch(`http://127.0.0.1:${ctx.port}/rc/noop`, {
    method: 'POST', headers: { Authorization: 'Basic ' + Buffer.from(`probeuser:${PASS}x`).toString('base64') },
  })).status, 401);
  check('correct env credentials are accepted', (await call(ctx, 'rc/noop')).status, 200);
  check('readiness probe method requires auth (unauthenticated call is refused)',
    (await fetch(`http://127.0.0.1:${ctx.port}/config/listremotes`, { method: 'POST' })).status, 401);

  const cmdlines = () => execFileSync('powershell', ['-NoProfile', '-Command',
    "Get-CimInstance Win32_Process -Filter \"Name='rclone.exe'\" | Select-Object -ExpandProperty CommandLine"],
    { encoding: 'utf8' });
  checkBool('env-passed password absent from Win32_Process.CommandLine', cmdlines().includes(PASS), false);
  ctx.child.kill(); await sleep(400);
  const envLog = readFileSync(root + '\\env.log', 'utf8');
  checkBool('DEBUG log records startup parameters', envLog.includes('starting with parameters'), true);
  checkBool('env-passed password absent from DEBUG log', envLog.includes(PASS), false);

  // Break it once: the same rcd started with the password in argv MUST leak, otherwise
  // the two assertions above are vacuous.
  const leaky = spawn(RCLONE, ['rcd', '--rc-addr', `127.0.0.1:${++port}`, '--rc-user', 'probeuser',
    '--rc-pass', PASS, '--log-file', root + '\\argv.log', '--log-level', 'DEBUG'],
    { stdio: ['ignore', 'ignore', 'pipe'] });
  for (let i = 0; i < 80; i++) { await sleep(200); if (existsSync(root + '\\argv.log') && readFileSync(root + '\\argv.log', 'utf8').includes('starting with parameters')) break; }
  checkBool('control: argv-passed password DOES appear in Win32_Process.CommandLine', cmdlines().includes(PASS), true);
  checkBool('control: argv-passed password DOES appear in the DEBUG log', readFileSync(root + '\\argv.log', 'utf8').includes(PASS), true);
  leaky.kill(); await sleep(400);
}

// ---------------------------------------------------- 2. job submission & stats
{
  const root = fixture('jobs');
  writeFileSync(root + '\\src\\a.txt', 'hello');
  writeFileSync(root + '\\dst\\orphan.txt', 'orphan');
  const ctx = await startRcd(root);

  const async = await call(ctx, 'sync/copy', { srcFs: root + '\\src', dstFs: root + '\\dst', _async: 'true' });
  check('_async submit returns jobid', typeof async.json.jobid, 'number');
  checkBool('_async submit returns executeId', typeof async.json.executeId === 'string', true);

  // The job is asynchronous: poll until finished before asserting on its effects.
  let st = {};
  for (let i = 0; i < 100; i++) {
    await sleep(150);
    st = (await call(ctx, 'job/status', { jobid: async.json.jobid })).json;
    if (st.finished) break;
  }
  checkBool('job/status reports finished+success', st.finished === true && st.success === true, true);
  check('job/status rejects group= instead of jobid=',
    (await call(ctx, 'job/status', { group: `job/${async.json.jobid}` })).status, 400);

  const stats = (await call(ctx, 'core/stats', { group: `job/${async.json.jobid}` })).json;
  check('core/stats field set', Object.keys(stats).sort(), ['bytes', 'checks', 'deletedDirs', 'deletes',
    'elapsedTime', 'errors', 'eta', 'fatalError', 'listed', 'renames', 'retryError', 'serverSideCopies',
    'serverSideCopyBytes', 'serverSideMoveBytes', 'serverSideMoves', 'speed', 'totalBytes', 'totalChecks',
    'totalTransfers', 'transferTime', 'transfers']);
  checkBool('core/stats carries no executeId', 'executeId' in stats, false);
  check('copy leaves the dst orphan alone (deletes)', stats.deletes, 0);
  check('copy did not delete orphan.txt', readdirSync(root + '\\dst').sort(), ['a.txt', 'orphan.txt']);

  const jl = (await call(ctx, 'job/list')).json;
  checkBool('job/list executeId matches the submitting call', jl.executeId === async.json.executeId, true);
  checkBool('job/list jobids outnumber our own submissions (rcd internal jobs)',
    jl.jobids.length > 1, true);
  checkBool('rc/noop returns no executeId', 'executeId' in (await call(ctx, 'rc/noop')).json, false);

  const sync = await runJob(ctx, 'sync/sync', { srcFs: root + '\\src', dstFs: root + '\\dst', _async: 'true' });
  check('sync deletes the dst orphan (the invariant-break case)', sync.stats.deletes, 1);
  check('orphan.txt is gone after sync', readdirSync(root + '\\dst').sort(), ['a.txt']);
  ctx.child.kill(); await sleep(300);
}

// ------------------------------------------------- 3. flag routing (startup vs _config)
{
  const root = fixture('flags');
  writeFileSync(root + '\\src\\keep.txt', 'keep');
  writeFileSync(root + '\\src\\Thumbs.db', 'junk');
  const ctx = await startRcd(root, ['--exclude', 'Thumbs.db', '--transfers', '4', '--checkers', '8', '--fast-list']);
  await runJob(ctx, 'sync/copy', { srcFs: root + '\\src', dstFs: root + '\\dst', _async: 'true' });
  check('rcd startup --exclude applies to rc jobs', readdirSync(root + '\\dst').sort(), ['keep.txt']);

  rmSync(root + '\\dst', { recursive: true, force: true }); mkdirSync(root + '\\dst');
  await runJob(ctx, 'sync/copy', { srcFs: root + '\\src', dstFs: root + '\\dst', _async: 'true', _config: '{"dryRun":true}' });
  check('per-call _config dryRun prevents writes', readdirSync(root + '\\dst').sort(), []);

  rmSync(root + '\\dst', { recursive: true, force: true }); mkdirSync(root + '\\dst');
  await runJob(ctx, 'sync/copy', { srcFs: root + '\\src', dstFs: root + '\\dst', _async: 'true', _config: '{"exclude":["keep.txt"]}' });
  check('_config exclude is IGNORED (why filters must go to startup args)',
    readdirSync(root + '\\dst').sort(), ['keep.txt']);

  const bogus = await runJob(ctx, 'sync/copy', { srcFs: root + '\\src', dstFs: root + '\\dst', _async: 'true', _config: '{"notARealFlag":1}' });
  check('unknown _config key fails OPEN (needs a client-side allowlist)', bogus.st.success, true);
  check('malformed _config returns 400', (await call(ctx, 'sync/copy', {
    srcFs: root + '\\src', dstFs: root + '\\dst', _async: 'true', _config: 'not-json' })).status, 400);
  ctx.child.kill(); await sleep(300);
}

// --------------------------------------------------------- 4. operations/list semantics
{
  const root = fixture('list');
  writeFileSync(root + '\\src\\root.txt', 'r');
  writeFileSync(root + '\\src\\imgprompt\\2026\\p1.webp', 'p');
  const ctx = await startRcd(root);
  const L = async (params) => (await call(ctx, 'operations/list', params)).json;

  check('remote="" lists the fs root one level', (await L({ fs: root + '\\src', remote: '' })).list.map((e) => e.Path).sort(),
    ['imgprompt', 'root.txt']);
  check('Path already includes the remote prefix', (await L({ fs: root + '\\src', remote: 'imgprompt/2026' })).list.map((e) => e.Path),
    ['imgprompt/2026/p1.webp']);
  check('remote="." is NOT the current directory', (await call(ctx, 'operations/list', { fs: root + '\\src', remote: '.' })).status, 404);
  check('trailing slash silently yields an empty list', (await L({ fs: root + '\\src', remote: 'imgprompt/2026/' })).list, []);
  check('missing directory yields 404', (await call(ctx, 'operations/list', { fs: root + '\\src', remote: 'nope' })).status, 404);
  check('opts.recursive is the WRONG key — silently ignored',
    (await L({ fs: root + '\\src', remote: '', opts: '{"recursive":true}' })).list.map((e) => e.Path).sort(),
    ['imgprompt', 'root.txt']);
  // 这条原来叫 "no recursion via rc"，那是从"这个拼法没用"过度推广成"rc 不能递归"，
  // 而整个 BFS 走目录的设计就建在那句过度推广上（复核过：那是过度推广）。
  // 两处拼错叠在一起：容器键是 `opt`（单数）不是 `opts`，递归字段是 `recurse` 不是 `recursive`。
  // 本探针用 **表单编码** 发请求，所以下面这条只证它自己量到的那件事：这个拼法被静默忽略。
  check('`opts`+`recursive` are both wrong spellings — silently ignored (single-level list)',
    (await L({ fs: root + '\\src', remote: '', opts: '{"recursive":true}' })).list.map((e) => e.Path).sort(),
    ['imgprompt', 'root.txt']);
  // 已量到（一次性 curl 夹具，JSON 请求体）：`{"opt":{"recurse":true}}` 一次调用返回整棵树
  // （134/134 条目），同夹具下不带 opt 与 `opt.recursive` 都只返回 14 条。
  // **没量到**：`opt` 能不能穿过本客户端用的表单编码。两次尝试都因夹具问题失败（含对照组），
  // 所以这里不下断言——把"能不能一次列完"留作未决问题，而不是拿一个假绿把它钉成已解决。
  check('entries carry IsDir so a BFS walk is possible',
    (await L({ fs: root + '\\src', remote: '' })).list.find((e) => e.Path === 'imgprompt').IsDir, true);
  ctx.child.kill(); await sleep(300);
}

// ---------------- 4b. does rcd's startup --exclude filter operations/list too? (spec §4.1)
//
// The whole mirror safety story rests on this. The panel builds its doomed list by diffing
// two listings; rclone deletes according to the startup filters. If the listing path ignored
// those filters, an excluded object (a `Thumbs.db` sitting in the bucket) would appear in the
// list the operator confirms while rclone never deletes it — the preview count and the actual
// deletes count would then disagree on **every** real mirror, and the only deletion path in the
// product would turn into a refusal loop. Measured both halves on purpose: listing AND acting.
{
  const root = fixture('listfilter');
  writeFileSync(root + '\\src\\keep.txt', 'keep');
  writeFileSync(root + '\\dst\\keep.txt', 'keep');
  writeFileSync(root + '\\dst\\Thumbs.db', 'junk at the root');
  mkdirSync(root + '\\dst\\sub', { recursive: true });
  writeFileSync(root + '\\dst\\sub\\Thumbs.db', 'junk one level down');
  writeFileSync(root + '\\dst\\sub\\extra.txt', 'a real orphan');

  const ctx = await startRcd(root, ['--exclude', 'Thumbs.db', '--fast-list']);

  // A top-level `recurse=true` over form encoding is NOT a recursion switch: measured on the fixture,
  // the same fs with and without it returns the same one-level list. The panel therefore walks
  // with single-level listings (BFS), and so must this probe — a check that leans on a flag
  // nobody uses would pass for the wrong reason.
  const flat = (await call(ctx, 'operations/list', { fs: root + '\\dst', remote: '', recurse: 'true' })).json.list;
  const noflag = (await call(ctx, 'operations/list', { fs: root + '\\dst', remote: '' })).json.list;
  check('form-encoded recurse=true is a no-op (same list as without it)',
    flat.map((e) => e.Path).sort(), noflag.map((e) => e.Path).sort());
  check('so a single-level listing really is single-level (the nested file is absent)',
    flat.map((e) => e.Path).sort(), ['keep.txt', 'sub']);

  // The cell spec §3.2 left unmeasured: `opt` was proven to work only in a **JSON** body, and two
  // earlier form-encoding attempts failed for fixture reasons (control included), so the panel
  // walks directories one level at a time. Form-encoding `opt` as a JSON string is what our own
  // client would send — if it works, a whole-tree listing costs one request instead of one per
  // directory, which is the R2-billed quantity MAX_LIST_REQUESTS guards.
  const first = (await call(ctx, 'operations/list', { fs: root + '\\dst', remote: '', opt: '{"recurse":true}' })).json.list ?? [];
  const again = (await call(ctx, 'operations/list', { fs: root + '\\dst', remote: '', opt: '{"recurse":true}' })).json.list ?? [];
  const viaOpt = first.filter((e) => !e.IsDir).map((e) => e.Path).sort();
  check('form-encoded opt={"recurse":true} survives: one call returns the whole tree',
    viaOpt, ['keep.txt', 'sub/extra.txt']);
  check('control: the same daemon with no opt returns one level (so the above is the flag, not the fixture)',
    (noflag ?? []).filter((e) => !e.IsDir).map((e) => e.Path).sort(), ['keep.txt']);
  checkBool('opt is idempotent across two calls (not a cached first-answer fluke)',
    JSON.stringify(again.map((e) => e.Path).sort()) === JSON.stringify(first.map((e) => e.Path).sort()), true);

  // Half one: what does the panel SEE — its own BFS over single-level listings.
  // `Path` already carries the queried remote as a prefix (fact measured in section 4), so the
  // name is used as returned and doubles as the next `remote` to descend into.
  async function walk(ctxRef, fs, remote = '') {
    const out = [];
    const queue = [remote];
    while (queue.length) {
      const cur = queue.shift();
      const list = (await call(ctxRef, 'operations/list', { fs, remote: cur })).json.list ?? [];
      for (const e of list) {
        if (e.IsDir) queue.push(e.Path); else out.push(e.Path);
      }
    }
    return out.sort();
  }
  const seen = await walk(ctx, root + '\\dst');
  checkBool('startup --exclude THUMBS.DB is applied to operations/list at every depth',
    seen.some((p) => p.includes('Thumbs.db')), false);
  check('what the walk does return, unchanged in count', seen, ['keep.txt', 'sub/extra.txt']);

  // Half two: what does rclone DO with the same fs? doomed-by-panel = dst \ src.
  const srcNames = new Set(await walk(ctx, root + '\\src'));
  const panelDoomed = seen.filter((p) => !srcNames.has(p));
  const dry = await runJob(ctx, 'sync/sync', {
    srcFs: root + '\\src', dstFs: root + '\\dst', _async: 'true', _config: '{"dryRun":true}',
  });
  check('panel doomed count === rclone dry-run deletes count under an active --exclude',
    dry.stats.deletes, panelDoomed.length);
  check('both agree on which name it is', panelDoomed, ['sub/extra.txt']);

  // Control: with the same fixture but a filter that matches nothing, the excluded-looking
  // file must show up in both readings — otherwise the two checks above pass because of an
  // empty fixture rather than because the two matchers agree.
  const ctx2 = await startRcd(root, ['--exclude', 'definitely-no-such-name-9f3c2a', '--fast-list']);
  const seen2 = await walk(ctx2, root + '\\dst');
  checkBool('control: a non-matching filter lets Thumbs.db into the listing (root and nested)',
    seen2.filter((p) => p.includes('Thumbs.db')).length, 2);
  const dry2 = await runJob(ctx2, 'sync/sync', {
    srcFs: root + '\\src', dstFs: root + '\\dst', _async: 'true', _config: '{"dryRun":true}',
  });
  check('control: and rclone then really would delete it (3 = both Thumbs.db + extra.txt)',
    dry2.stats.deletes, 3);

  ctx.child.kill(); ctx2.child.kill(); await sleep(400);
}

// ---------------- 4c. what a MISSING side answers — the boundary R64's verdict sits on (spec §4.1)
//
// `side_set` maps a 404 on the listing ROOT to "this side is empty" (裁定 R64), because a prefix
// nobody has uploaded to yet is a normal, harmless emptiness. That mapping is only safe if the
// two failures that LOOK like emptiness answer differently: a remote name that isn't in the config
// at all, and a directory that vanished (unmounted drive). Measured once:
//   unknown remote name      -> 500 "didn't find section in config file"   (loud, not swallowed)
//   alias to a missing dir   -> 404 "error in ListJSON: directory not found" (reads as empty)
// The second one is the accepted cost of R64: an unmounted drive and a never-uploaded prefix are
// indistinguishable from the listing. What must NOT regress is the first — a typo in jobs.toml
// becoming a quiet "nothing over there".
{
  const root = fixture('missing');
  mkdirSync(root + '\\alive', { recursive: true });
  writeFileSync(root + '\\alive\\a.webp', 'a');
  writeFileSync(root + '\\rclone-alias.conf', `[ghost]\ntype = alias\nremote = ${root}\\never-created\n[real]\ntype = alias\nremote = ${root}\\alive\n`);
  const ctx = await startRcd(root, ['--config', root + '\\rclone-alias.conf']);
  const L = (fs) => call(ctx, 'operations/list', { fs, remote: '' });

  const typo = await L('nodecknosuchremote:bucket');
  check('a remote name absent from the config answers 500, NOT 404', typo.status, 500);
  checkBool('and it names the offending remote so the operator can find the typo',
    String(typo.json.error ?? typo.json).includes('nodecknosuchremote'), true);
  const ghost = await L('ghost:');
  check('an alias whose directory does not exist answers 404 (this is the case R64 reads as empty)',
    ghost.status, 404);
  // Control: the same alias kind, pointed at a directory that exists but is empty, answers
  // 200 + [] — so the 404 above is the missing directory, not "aliases always fail".
  mkdirSync(root + '\\emptydir', { recursive: true });
  writeFileSync(root + '\\rclone-alias2.conf', `[blank]\ntype = alias\nremote = ${root}\\emptydir\n`);
  const ctx2 = await startRcd(root, ['--config', root + '\\rclone-alias2.conf']);
  const blank = await call(ctx2, 'operations/list', { fs: 'blank:', remote: '' });
  check('control: an existing but empty side answers 200 with an empty list', [blank.status, blank.json.list], [200, []]);
  const alive = await call(ctx2, 'operations/list', { fs: root + '\\alive', remote: '' });
  check('control: and a populated side lists its file (so 200/[] above is really emptiness)',
    alive.json.list.map((e) => e.Path), ['a.webp']);

  // A whole job against the unknown remote must fail, not silently "transfer nothing".
  const sub = await call(ctx, 'sync/sync', {
    srcFs: 'nodecknosuchremote:bucket', dstFs: root + '\\dst', _async: 'true', _config: '{"dryRun":true}',
  });
  let st = {};
  if (sub.json.jobid) {
    for (let i = 0; i < 60; i++) {
      await sleep(150);
      st = (await call(ctx, 'job/status', { jobid: sub.json.jobid })).json;
      if (st.finished) break;
    }
  }
  check('a sync whose source remote is unknown reports success=false', st.success, false);
  ctx.child.kill(); ctx2.child.kill(); await sleep(400);
}

// ---------------- 4d. which `opt` keys does operations/list actually honor?
//
// 起因：操作员在真桶上点「看差异」拿到 90 秒超时，而 rclone 自己的 rcd.log 给出的是
// **逐对象**的 `HeadObject`（那 90 秒推进 768 个对象 ≈ 一次 API 往返一个），面板拿到结果
// 之后第一件事就是把目录行全丢掉（`rc/listing.rs` 两处 `!e.is_dir`）。所以发请求时少要一样
// 东西，可能就把整次列举从 N 次往返变成一次分页。
//
// **这一节先记下的是我自己踩的那个坑**：第一版按 `rclone lsf` 的旗标名发了
// `{"recurse":true,"dirs":false,"files":true}` —— 两个键都不存在，rclone **静默按默认答**
// （逐字节与对照组相同）。这就是 §3 与裁定 R51 记过的 fail-open：未识别的 rc 参数不报错、
// 不回声，只给你一份"看起来成功了"的默认结果。所以这里的判据全部落在**答案的形状**上。
//
// 真名（官方 rc 文档）：`filesOnly` / `dirsOnly` / `noModTime` / `noMimeType` / `metadata`。
// 面板只读 `Path/Name/Size/IsDir`（`rc/types.rs` 的 `ListEntry`），所以"少给 ModTime/MimeType"
// 对我们是免费的；目录行本来就丢。至于这些开关在真 S3 上**省不省**那次逐对象确认，
// 本地后端量不出来（它没有 HEAD 这一说）——那一半在证据文件里，量的是真桶的只读列举。
{
  const root = fixture('listdirs');
  for (const p of ['a.webp', 'x.txt', 'sub/b.webp', 'sub/deep/c.webp']) {
    const full = root + '\\dst\\' + p.replace(/\//g, '\\');
    mkdirSync(full.substring(0, full.lastIndexOf('\\')), { recursive: true });
    writeFileSync(full, p);
  }
  mkdirSync(root + '\\dst\\empty', { recursive: true });   // 一个只有目录、没有文件的层
  mkdirSync(root + '\\dst\\sub\\deep', { recursive: true });
  const ctx = await startRcd(root, ['--fast-list']);

  const rows = (r) => r.json.list ?? [];
  const paths = (r) => rows(r).map((e) => e.Path).sort();
  const dirRows = (r) => rows(r).filter((e) => e.IsDir).map((e) => e.Path).sort();
  const files = (r) => rows(r).filter((e) => !e.IsDir).map((e) => e.Path).sort();
  const list = (opt, remote = '') => call(ctx, 'operations/list', { fs: root + '\\dst', remote, opt });

  const plain = await list('{"recurse":true}');
  const fake = await list('{"recurse":true,"dirs":false,"files":true}');
  const filesOnly = await list('{"recurse":true,"filesOnly":true}');
  const lean = await list('{"recurse":true,"filesOnly":true,"noModTime":true,"noMimeType":true}');

  check('control: the default recursive listing DOES carry directory rows',
    dirRows(plain), ['empty', 'sub', 'sub/deep']);
  checkBool('the WRONG key names are silently ignored (fail-open, so this probe is the only thing that catches it)',
    JSON.stringify(paths(fake)) === JSON.stringify(paths(plain)), true);
  check('filesOnly really does drop the directory rows', dirRows(filesOnly), []);
  check('filesOnly keeps the FILE set byte-identical (this set is the deletion coordinate system)',
    paths(filesOnly), ['a.webp', 'sub/b.webp', 'sub/deep/c.webp', 'x.txt']);
  check('the lean shape (filesOnly + noModTime + noMimeType) answers the same file set',
    files(lean), files(filesOnly));
  // 实测形状：`noMimeType` 是真的**不返回那个键**，而 `noModTime` 回的是**空串**（rclone 把零值
  // 时间格式化出来的样子）。两种"没给"长得不一样，所以这条断言按实测写，不按文档措辞写。
  checkBool('and it stops returning MimeType, with ModTime emptied (the panel reads neither: ListEntry has 4 fields)',
    rows(lean).every((e) => e.MimeType === undefined && (e.ModTime === '' || e.ModTime === undefined)), true);
  checkBool('control: the default shape DOES return both (so the above is the opt, not the backend)',
    rows(plain).every((e) => typeof e.MimeType === 'string' && e.MimeType !== '' && e.ModTime !== ''), true);

  checkBool('Size survives the lean shape (browser rows and the "撞名" refusal read it)',
    rows(lean).every((e) => typeof e.Size === 'number'), true);

  // 面板的另一档：`remote=<前缀>`（闸门对账走的就是这个形状）。同一套对照在子层再走一遍，
  // 因为"Path 已带前缀"这条事实在子层才最容易破。
  const subPlain = await list('{"recurse":true}', 'sub');
  const subLean = await list('{"recurse":true,"filesOnly":true,"noModTime":true,"noMimeType":true}', 'sub');
  check('control at a prefix: the nested dir row is there and its Path carries the prefix',
    dirRows(subPlain), ['sub/deep']);
  check('at a prefix the lean file set is the same, prefix included',
    paths(subLean), ['sub/b.webp', 'sub/deep/c.webp']);

  // ---- 面板的另一档：单层（浏览器进一层用的就是这一串）
  //
  // 这一档与上面那串的关系是"各守一半"：目录行必须留着（浏览器靠它画文件夹、决定能不能点进去），
  // 而 metadata 那两样必须关掉（`ListEntry` 一个都不读）。所以它的判据不是"答案与递归那串相同"，
  // 而是**与同层的默认那一发相同**：文件集合、目录集合、Size 三样都要逐字相等。
  // 省不省那次逐对象确认，本地后端没有 HEAD 这一说，量不出来——那一半是桶上的读数。
  const listNoOpt = (remote = '') => call(ctx, 'operations/list', { fs: root + '\\dst', remote });
  const levelPlain = await listNoOpt();
  const levelLean = await list('{"noModTime":true,"noMimeType":true}');

  check('control: a plain one-level listing carries the dir rows',
    dirRows(levelPlain), ['empty', 'sub']);
  check('the lean one-level shape keeps the FILE set of the plain one',
    files(levelLean), files(levelPlain));
  check('and keeps the DIR set too (filesOnly must never enter this shape)',
    dirRows(levelLean), dirRows(levelPlain));
  checkBool('it still stops returning MimeType and empties ModTime, with Size intact on every row',
    rows(levelLean).every((e) => e.MimeType === undefined
      && (e.ModTime === '' || e.ModTime === undefined) && typeof e.Size === 'number'), true);

  ctx.child.kill(); await sleep(400);
}

// ------------------------------ 5. the three rc facts the mirror path leans on (spec §4.1)
{
  const root = fixture('mirror');
  // src keeps a/b/c; dst holds a/b/c plus two orphans. The panel's doomed set is dst \ src = {x,y}.
  for (const n of ['a', 'b', 'c']) writeFileSync(root + '\\src\\' + n + '.webp', n);
  for (const n of ['a', 'b', 'c', 'x', 'y']) writeFileSync(root + '\\dst\\' + n + '.webp', n);
  const ctx = await startRcd(root);

  // Fact 1: a dry-run sync/sync answers job/status with output === {} — core/stats carries only
  // the deletes COUNT, no per-name list. That is why the doomed list must be computed by the
  // panel itself from its own listings (裁定 R60: no core/command escape hatch).
  const dry = await runJob(ctx, 'sync/sync', {
    srcFs: root + '\\src', dstFs: root + '\\dst', _async: 'true', _config: '{"dryRun":true}',
  });
  check('dry-run job/status.output is exactly {} (no per-name list anywhere)', dry.st.output, {});
  check('dry-run changed nothing on disk (the count is a promise, not an act)',
    readdirSync(root + '\\dst').sort(), ['a.webp', 'b.webp', 'c.webp', 'x.webp', 'y.webp']);
  const statsJson = JSON.stringify(dry.stats);
  checkBool('core/stats names no doomed file (x/y appear nowhere in it — count only)',
    statsJson.includes('x.webp') || statsJson.includes('y.webp'), false);

  // Fact 2: rclone's own dry-run deletes count equals the panel's set-difference count.
  // The two paths are independent (rclone compares per its own rules; the panel diffs its
  // listings). Equality is the cross-check the UI shows; disagreement here means the fixture
  // or the assumption broke, and this probe must say so loudly.
  const panelDoomed = readdirSync(root + '\\dst').filter((f) => !existsSync(root + '\\src\\' + f));
  check('dry-run deletes === panel doomed count (the cross-check in the window)',
    dry.stats.deletes, panelDoomed.length);
  check('panel doomed set is exactly {x,y}', panelDoomed.sort(), ['x.webp', 'y.webp']);

  // Fact 3: operations/list with a TRAILING SLASH on remote still answers 200 + empty list.
  // A silent empty list is worse than a 404: the endpoint layer strips the tail so a listing
  // bug never reads as "the other side is empty" (which, for a mirror, means "delete nothing" —
  // or, mirrored wrong, "everything is doomed").
  mkdirSync(root + '\\dst\\sub', { recursive: true });
  writeFileSync(root + '\\dst\\sub\\z.webp', 'z');
  const slashed = await call(ctx, 'operations/list', { fs: root + '\\dst', remote: 'sub/' });
  check('trailing-slash remote still returns HTTP 200 (no error to catch)', slashed.status, 200);
  check('trailing-slash remote lists NOTHING (the empty list is the trap)', slashed.json.list, []);
  const plain = await call(ctx, 'operations/list', { fs: root + '\\dst', remote: 'sub' });
  check('same remote without the slash lists the file (so the emptiness is the slash)',
    plain.json.list.map((e) => e.Path).sort(), ['sub/z.webp']);
  ctx.child.kill(); await sleep(300);
}

// ---------------- 6. 浏览器动作面的六档形状（spec §2 的 F10–F15，实测）
// 为什么必须在实现之前存在：撞名那一档如果按"单次样本"钉，我会把随机的 3–7 次写成"一律拒绝"
// ——上一版就是这么错的。所以它按**分布**钉（注入 modtime，两边各 10 次）。
{
  const root = fixture('browser');                       // fixture() 先 rmSync 再建 src/dst
  const confPath = `${root}\\rclone.conf`;
  // 每个用例一对独立 alias：一次动作一份夹具，绝不共享（v6 那一轮的教训）。
  const cases = ['ov', 'sz', 'pg', 'st', 'rm', 'as'];
  writeFileSync(confPath, cases.map((c) => [
    `[${c}s]`, 'type = alias', `remote = ${root}\\${c}\\s`, '',
    `[${c}t]`, 'type = alias', `remote = ${root}\\${c}\\t`, '',
  ].join('\n')).join('\n'));
  for (const c of cases) {
    mkdirSync(`${root}\\${c}\\s`, { recursive: true });
    mkdirSync(`${root}\\${c}\\t`, { recursive: true });
  }
  const ctx = await startRcd(root, ['--config', confPath]);
  const filt = await startRcd(root, ['--config', confPath, '--exclude', '*.log']);

  const seed = (c, srcBody, dstBody, when) => {
    writeFileSync(`${root}\\${c}\\s\\in.bin`, srcBody);
    writeFileSync(`${root}\\${c}\\t\\out.bin`, dstBody);
    if (when) {
      const t = new Date(when);
      utimesSync(`${root}\\${c}\\s\\in.bin`, t, t);
      utimesSync(`${root}\\${c}\\t\\out.bin`, t, t);
    }
  };
  const target = (c) => readFileSync(`${root}\\${c}\\t\\out.bin`, 'utf8');

  // F10 ①：目标已存在 + 同大小 + 同 modtime ⇒ rclone 回 200 而**不写**
  let landed = 0;
  for (let i = 0; i < 10; i++) {
    seed('ov', 'NEW', 'OLD', '2020-01-01T00:00:00Z');
    const r = await call(ctx, 'operations/copyfile', { srcFs: 'ovs:', srcRemote: 'in.bin', dstFs: 'ovt:', dstRemote: 'out.bin' });
    if (r.status !== 200) failures++;
    if (target('ov') === 'NEW') landed++;
  }
  check('F10 same size + same modtime: a copyfile onto an existing name never lands (10 trials)', landed, 0);
  // 对照组：只是大小不同就必须每次覆盖（证明上一条不是"copyfile 整体坏了"）
  let replaced = 0;
  for (let i = 0; i < 10; i++) {
    seed('ov', 'NEWER-AND-LONGER', 'OLD', '2020-01-01T00:00:00Z');
    const r = await call(ctx, 'operations/copyfile', { srcFs: 'ovs:', srcRemote: 'in.bin', dstFs: 'ovt:', dstRemote: 'out.bin' });
    if (r.status !== 200) failures++;
    if (target('ov') === 'NEWER-AND-LONGER') replaced++;
  }
  check('F10 control: a different size does overwrite (10 trials)', replaced, 10);
  const collide = await call(ctx, 'operations/copyfile', { srcFs: 'ovs:', srcRemote: 'in.bin', dstFs: 'ovt:', dstRemote: 'out.bin' });
  check('F10 a collision answers HTTP 200 — there is no error to inspect, hence our own pre-flight check', collide.status, 200);
  mkdirSync(`${root}\\ov\\t\\box`, { recursive: true });
  const dirName = await call(ctx, 'operations/copyfile', { srcFs: 'ovs:', srcRemote: 'in.bin', dstFs: 'ovt:', dstRemote: 'box' });
  check('F10 dstRemote naming an existing DIRECTORY is refused by rc with 500',
    [dirName.status, dirName.json.error], [500, 'is a directory not a file']);

  // F13：stat 是存在性问话——存在给对象；不存在是 **200 + item:null，不是 404**
  seed('st', 'S', 'D', null);
  const stExists = await call(ctx, 'operations/stat', { fs: 'sts:', remote: 'in.bin' });
  check('F13 stat on an existing object returns it under `item`',
    [stExists.status, stExists.json.item.Name, stExists.json.item.IsDir], [200, 'in.bin', false]);
  const stMissing = await call(ctx, 'operations/stat', { fs: 'sts:', remote: 'definitely-not-there.bin' });
  check('F13 stat on a missing object is HTTP 200 with item:null (NOT 404)',
    [stMissing.status, stMissing.json.item], [200, null]);

  // F11：同一份内容、两个 daemon，只差一条 --exclude
  writeFileSync(`${root}\\sz\\t\\visible.txt`, 'v');
  writeFileSync(`${root}\\sz\\t\\hidden.log`, 'h');
  const lFilt = await call(filt, 'operations/list', { fs: 'szt:', remote: '', opt: '{"recurse":true}' });
  const lPlain = await call(ctx, 'operations/list', { fs: 'szt:', remote: '', opt: '{"recurse":true}' });
  check('F11 listing is filtered (one rule hides one object)', lFilt.json.list.map((e) => e.Path), ['visible.txt']);
  check('F11 control: the same listing on an unfiltered daemon shows both',
    lPlain.json.list.map((e) => e.Path).sort(), ['hidden.log', 'visible.txt']);
  const sFilt = await call(filt, 'operations/size', { fs: 'szt:', remote: '' });
  const sPlain = await call(ctx, 'operations/size', { fs: 'szt:', remote: '' });
  check('F11 operations/size is filtered too (the count an operator would trust)', sFilt.json.count, 1);
  check('F11 control: the same operations/size on an unfiltered daemon counts both', sPlain.json.count, 2);
  // 这两条是 I8 的命门：**读**被过滤，而**按名字的写**不被过滤
  const delHidden = await call(filt, 'operations/deletefile', { fs: 'szt:', remote: 'hidden.log' });
  check('F13 deletefile reaches the object the filter hides, so names are the safe unit',
    [delHidden.status, existsSync(`${root}\\sz\\t\\hidden.log`)], [200, false]);

  // F11 + F12：purge 删的比显示的多，而且把目录本身一起删
  writeFileSync(`${root}\\pg\\t\\visible.txt`, 'v');
  writeFileSync(`${root}\\pg\\t\\hidden.log`, 'h');
  const purged = await call(filt, 'operations/purge', { fs: 'pgt:', remote: '' });
  check('F11 purge over a filtered prefix answers 200', purged.status, 200);
  check('F11 purge removed the object the listing never showed', existsSync(`${root}\\pg\\t\\hidden.log`), false);
  check('F12 on an alias-to-local backend purge removes the directory itself', existsSync(`${root}\\pg\\t`), false);

  // F12：purge 的三档失败形状。**都是 500**——F9 那条"404=这一侧是空的"只属于列举
  const purgeFile = await call(ctx, 'operations/purge', { fs: 'sts:', remote: 'in.bin' });
  // 断**包含**而不是整句相等：实测原文是 `failed to remove directories: is a file not a directory`，
  // 前面那截是 rclone 的包装，会随版本与后端变（R82：本条初稿按核心短语写死了全等，实跑打红）。
  check('F12 purge on a FILE name is 500 and says "is a file not a directory" inside rclone\'s wrapper',
    [purgeFile.status, String(purgeFile.json.error).includes('is a file not a directory')], [500, true]);
  const purgeGone = await call(ctx, 'operations/purge', { fs: 'sts:', remote: 'no-such-prefix' });
  check('F12 purge on a missing prefix is 500 (must never be read as "already empty")', purgeGone.status, 500);
  const purgeBadRemote = await call(ctx, 'operations/purge', { fs: 'no-such-remote:', remote: '' });
  check('F12 purge on an unknown remote names it loudly',
    [purgeBadRemote.status, String(purgeBadRemote.json.error).includes("didn't find section")], [500, true]);

  // F14：mkdir 幂等 / rmdir 只删空的、非空那句原文带绝对路径
  const mk1 = await call(ctx, 'operations/mkdir', { fs: 'rmt:', remote: 'box' });
  const mk2 = await call(ctx, 'operations/mkdir', { fs: 'rmt:', remote: 'box' });
  check('F14 mkdir is idempotent (an existing directory answers 200 again)', [mk1.status, mk2.status], [200, 200]);
  const rmdirEmpty = await call(ctx, 'operations/rmdir', { fs: 'rmt:', remote: 'box' });
  await call(ctx, 'operations/mkdir', { fs: 'rmt:', remote: 'box' });
  writeFileSync(`${root}\\rm\\t\\box\\inside.txt`, 'x');
  const rmdirFull = await call(ctx, 'operations/rmdir', { fs: 'rmt:', remote: 'box' });
  check('F14 rmdir succeeds on an EMPTY directory and is 500 on a non-empty one',
    [rmdirEmpty.status, rmdirFull.status], [200, 500]);
  check('F14 rmdir refusal carries the real path (show it to the operator, never invent one)',
    // R83：这句 label 断的是"带绝对路径"，所以断言必须钉**路径性**而不是钉"名字被回显了"。
    // `.includes('box')` 在一个只回显相对名或只说"not empty"的实现上照样 PASS —— 标签与断言不符。
    // 实测形状（rclone v1.75.1 / Windows，整串原文）：
    //   `remove \\?\C:\Users\<user>\AppData\Local\Temp\rc-contract-probe-<rand>\browser\rm\t\box: The directory is not empty.`
    // `\\?\` 扩展长度前缀落在夹具根**之前**，分隔符与 conf 里写的 `remote = <root>\rm\t` 一致，
    // 所以 `root` 整段是连续子串：断 `includes(root)` 钉住的正是"绝对路径"，无需放宽回 echo-ness。
    [String(rmdirFull.json.error).includes(root), String(rmdirFull.json.error).includes('box')], [true, true]);

  // F15：purge 可以异步提交并轮询；core/stats 给 deletes/deletedDirs，里面**还有 listed**
  writeFileSync(`${root}\\as\\t\\a.txt`, 'a');
  writeFileSync(`${root}\\as\\t\\b.txt`, 'b');
  const ack = await call(ctx, 'operations/purge', { fs: 'ast:', remote: '', _async: 'true' });
  check('F15 async purge submit returns a jobid', typeof ack.json.jobid, 'number');
  let asJob = {};
  for (let i = 0; i < 40; i++) {
    await sleep(150);
    asJob = (await call(ctx, 'job/status', { jobid: ack.json.jobid })).json;
    if (asJob.finished) break;
  }
  check('F15 the async purge job finishes successfully', [asJob.finished, asJob.success], [true, true]);
  const asStats = await call(ctx, 'core/stats', { group: `job/${ack.json.jobid}` });
  check('F15 purge stats carry deletes/deletedDirs (and `listed`, which must never reach the UI)',
    [asStats.json.deletes, asStats.json.deletedDirs, typeof asStats.json.listed], [2, 1, 'number']);

  ctx.child.kill(); filt.child.kill();
  await sleep(400);
}

// ---------------- 7. F16：把**本地绝对路径**当 rc 的 `fs` 发出去，三档写法各答什么（裁定 R95）
//
// 浏览器左列要让任务里的本地目录可点，`browser_list` 就会是**这个应用里第一次**把本地路径放进
// `fs` 参数。`runner/checks.rs` 讲的就是"列举绝不能拿本地路径"（那会把"远端集合"读成
// "再扫一遍本地盘"，是本项目要拦的假绿），所以这一格在写下 §7 之前是**没有读数**的形状，而计划被这一族咬过
// 两次（把"没跑成"读成"行为"、把"形状像"读成"实测过"）。这一节先把四种回答量下来，再决定实现：
// 哪个写法能列、fsinfo/size 给什么、junction 会不会被悄悄列进来、rcd 的 `--exclude` 管不管本地 fs。
// 全程 local-to-local：`--config` 指到临时 conf，操作员的真远端一个字节都不会被碰。
{
  const root = fixture('f16local');
  const TREE = `${root}\\tree`;
  mkdirSync(TREE, { recursive: true });
  writeFileSync(`${TREE}\\a.txt`, 'aaaa');            // 4 B
  mkdirSync(`${TREE}\\sub\\deep`, { recursive: true });
  writeFileSync(`${TREE}\\sub\\b.txt`, 'bb');         // 2 B
  writeFileSync(`${TREE}\\sub\\deep\\c.txt`, 'c');    // 1 B
  writeFileSync(`${TREE}\\note.log`, 'LOGGED');       // 6 B，给 --exclude *.log 用
  mkdirSync(`${root}\\other`, { recursive: true });
  writeFileSync(`${root}\\other\\via-junction.txt`, 'VIA');
  // junction 不需要管理员令牌；建不出来就是夹具坏了，下面第一条控件会响亮地红。
  try { execFileSync('cmd', ['/c', 'mklink', '/J', `${TREE}\\link`, `${root}\\other`], { stdio: ['ignore', 'pipe', 'pipe'] }); } catch {}
  check('fixture control: the junction is a directory rclone could list if it wanted to',
    [readdirSync(TREE).includes('link'), readdirSync(`${TREE}\\link`).includes('via-junction.txt')], [true, true]);

  const confPath = `${root}\\rclone.conf`;
  writeFileSync(confPath, [
    `[alive]\ntype = alias\nremote = ${TREE}`,
    `[ghost]\ntype = alias\nremote = ${root}\\never-created`,
  ].join('\n') + '\n');

  const ctx = await startRcd(root, ['--config', confPath]);
  const filt = await startRcd(root, ['--config', confPath, '--exclude', '*.log']);
  const BACK = TREE;                    // ①反斜杠原样
  const FWD = TREE.replace(/\\/g, '/'); // ②正斜杠
  const LEAD = `/${FWD}`;               // ③正斜杠再加一个开头斜杠
  const paths = (r) => (r.json.list ?? []).map((e) => e.Path).sort();
  const sum = (r) => (r.json.list ?? []).filter((e) => !e.IsDir)
      .reduce((n, e) => n + e.Size, 0);

  // ① ②：两种写法都列得出整棵树（`opt={"recurse":true}` 是面板唯一在发的形状）。
  // `Path` 是 fs 根内的相对名、**已经带上前缀**，分隔符是 `/`；`Name` 只是最后一段。
  const whole = ['a.txt', 'note.log', 'sub', 'sub/b.txt', 'sub/deep', 'sub/deep/c.txt'];
  check('F16 backslash fs lists the whole tree in ONE call (Path uses forward slashes)',
    (await call(ctx, 'operations/list', { fs: BACK, remote: '', opt: '{"recurse":true}' })).json.list.map((e) => e.Path).sort(), whole);
  check('F16 forward-slash fs lists exactly the same set (so the conversion is a spelling, not a different fs)',
    (await call(ctx, 'operations/list', { fs: FWD, remote: '', opt: '{"recurse":true}' })).json.list.map((e) => e.Path).sort(), whole);
  check('F16 a nested prefix on a local fs answers with prefixed Paths and basename Names',
    (await call(ctx, 'operations/list', { fs: BACK, remote: 'sub', opt: '{"recurse":true}' })).json.list
      .map((e) => `${e.Path}|${e.Name}|${e.IsDir}`).sort(),
    ['sub/b.txt|b.txt|false', 'sub/deep/c.txt|c.txt|false', 'sub/deep|deep|true']);
  // 控件：`Name` 只有最后一段，所以同一个前缀下两个同名对象在"名字"上分不开——
  // 界面拿当身份的那一个必须是 `Path`（F16 的这条直接决定 browser_list 交出去哪个字段）。
  // 单独一棵树：混进 `tree` 会让下面那些按字节数与前缀算的读数全部变味。
  const dup = `${root}\\dup`;
  mkdirSync(`${dup}\\x`, { recursive: true });
  writeFileSync(`${dup}\\x\\pick.webp`, 'deep');
  writeFileSync(`${dup}\\pick.webp`, 'shallow');
  check('F16 control: Name is only the last segment, so two different objects share it (Path is the identity)',
    (await call(ctx, 'operations/list', { fs: dup, remote: '', opt: '{"recurse":true}' })).json.list
      .map((e) => `${e.Path}=${e.Name}`).sort(),
    ['pick.webp=pick.webp', 'x/pick.webp=pick.webp', 'x=x']);

  // ③：开头多一个 `/` 的写法**不能用**，而且坏得很难看：list 与 size 回 404（会被读成"这一侧没有东西"），
  // fsinfo 却回 200 —— 它把那段路径按**当前盘**的相对路径解析了（本机量到 `//?/D:/C:/Users/...`），
  // 也就是它一本正经地介绍了一个没人请求过的目录。
  const leadList = await call(ctx, 'operations/list', { fs: LEAD, remote: '', opt: '{"recurse":true}' });
  const leadSize = await call(ctx, 'operations/size', { fs: LEAD, remote: '' });
  const leadInfo = await call(ctx, 'operations/fsinfo', { fs: LEAD });
  const okInfo = await call(ctx, 'operations/fsinfo', { fs: FWD });
  check('F16 leading-slash fs: list and size answer 404 (the shape R64 reads as "this side is empty")',
    [leadList.status, leadSize.status], [404, 404]);
  checkBool('F16 ... while fsinfo answers 200 for that same non-existent fs (so an error there is NOT proof)',
    leadInfo.status === 200, true);
  checkBool('F16 and fsinfo describes a DIFFERENT root than the working spelling (a drive got inserted)',
    [leadInfo.json.Root !== okInfo.json.Root, leadInfo.json.Root.includes(FWD)], [true, true]);

  // fsinfo 在能用的两种写法上答什么：Name=local，Root 是 `//?/` + 正斜杠路径，
  // 能力位是 F6 那一族（Copy=false / Purge=false）而 FilterAware=true（I8 那句提示的来源）。
  check('F16 fsinfo on a local fs names the local backend and its canonical //?/ root',
    [okInfo.json.Name, okInfo.json.Root], ['local', `//?/${FWD}`]);
  check('F16 the four feature bits the browser reads, measured on a local fs (F6 family)',
    ['Copy', 'Purge', 'FilterAware', 'IsLocal'].map((k) => okInfo.json.Features[k]), [false, false, true, true]);
  // 一个不存在的目录：fsinfo 照样 200（它不做存在性检查），list 才是 404。
  const ghostInfo = await call(ctx, 'operations/fsinfo', { fs: `${root}\\never-created` });
  const ghostList = await call(ctx, 'operations/list', { fs: `${root}\\never-created`, remote: '' });
  check('F16 fsinfo does NOT check existence: a missing directory answers 200 while its listing is 404',
    [ghostInfo.status, ghostList.status], [200, 404]);
  // 唯一量到的"fsinfo 自己失败"那一档：fs 是一个**文件**。而那一档 list 同样失败（500），
  // 所以真 daemon 上造不出"list 成功而 fsinfo 失败"的形状——Task 8 那句"取不到就留空"只能
  // 在单元测试里钉，不能拿 daemon 证人钉（读数见这两行）。
  const fileInfo = await call(ctx, 'operations/fsinfo', { fs: `${TREE}\\a.txt` });
  const fileList = await call(ctx, 'operations/list', { fs: `${TREE}\\a.txt`, remote: '' });
  check('F16 fs on a FILE: fsinfo and list both answer 500 (no local shape fails fsinfo alone)',
    [fileInfo.status, fileList.status, String(fileInfo.json.error).includes('is a file not a directory')], [500, 500, true]);

  // junction：**不会**进列表，一层都不进（rclone 默认不跟随重解析点，stderr 给的是
  // `Can't follow symlink without -L/--copy-links`）。面板自己的本地扫描（`normalize_expected`）
  // 同样把重解析点排除在外，所以"浏览器看到的本地树"与"对账用的本地树"是同一个口径。
  const localWhole = (await call(ctx, 'operations/list', { fs: BACK, remote: '', opt: '{"recurse":true}' })).json.list;
  check('F16 the junction is not listed at all (no entry, no descent) — same exclusion as our own local walk',
    [localWhole.map((e) => e.Path).some((x) => x.startsWith('link')), localWhole.some((e) => e.Name === 'link')], [false, false]);

  // rcd 的 `--exclude` 管不管本地 fs（I8 那句"按 N 条排除规则显示"要在左列也为真）。
  check('F16 the daemon --exclude filters a local-path fs too (the filtered note is true here as well)',
    paths(await call(filt, 'operations/list', { fs: BACK, remote: '', opt: '{"recurse":true}' })),
    ['a.txt', 'sub', 'sub/b.txt', 'sub/deep', 'sub/deep/c.txt']);
  check('F16 control: the same local fs on an unfiltered daemon still lists note.log',
    paths(await call(ctx, 'operations/list', { fs: BACK, remote: '', opt: '{"recurse":true}' })).includes('note.log'), true);
  // `usage_of` 那句"过滤后的求和 == 过滤后的 size"必须有实测，不能靠两侧各自读一遍再心算。
  // 这一格同时也是 I8 的下游：那句提示说"按 N 条规则显示"，显示的量和报出来的量得是同一批对象。
  const filtList = (await call(filt, 'operations/list', { fs: BACK, remote: '', opt: '{"recurse":true}' })).json.list;
  const filtSize = await call(filt, 'operations/size', { fs: BACK, remote: '' });
  check('F16 on the filtered daemon the listing sum equals operations/size (7 B / 3 files, note.log gone from both)',
    [filtList.filter((e) => !e.IsDir).reduce((n, e) => n + e.Size, 0),
     filtList.filter((e) => !e.IsDir).length, filtSize.json.bytes, filtSize.json.count], [7, 3, 7, 3]);

  // operations/size 在本地这一族上**不看前缀**（三种 fs 形状都一样，包括具名 alias）：
  // 问 `sub`、问一个不存在的前缀，回来的都是**整棵树**的数，而且都是 200。
  // 于是"这个前缀用了多少"不能拿它回答——浏览器改成按**已经列到的那一份**求和（下面两条）。
  const sizeAt = (fs, remote) => call(ctx, 'operations/size', { fs, remote });
  for (const [name, fs] of [['backslash', BACK], ['forward', FWD], ['alias', 'alive:']]) {
    const r0 = await sizeAt(fs, '');
    check(`F16 operations/size at the root matches the listing sum for ${name} fs (13 B / 4 files, dirs not counted)`,
      [r0.json.bytes, r0.json.count, sum(await call(ctx, 'operations/list', { fs, remote: '', opt: '{"recurse":true}' }))],
      [13, 4, 13]);
    const sub = await sizeAt(fs, 'sub');
    check(`F16 operations/size IGNORES the prefix on a local-backed fs (${name}): 3 B/2 files is never answered`,
      [sub.status, sub.json.bytes, sub.json.count], [200, 13, 4]);
    const gone = await sizeAt(fs, 'no-such-dir-9f3c');
    check(`F16 ... and a missing prefix answers 200 with the same whole-tree numbers for ${name} (no 404 to catch)`,
      [gone.status, gone.json.bytes], [200, 13]);
  }
  check('F16 the listing DOES honour the prefix (so the per-prefix number has to come from the listing)',
    [paths(await call(ctx, 'operations/list', { fs: BACK, remote: 'sub', opt: '{"recurse":true}' }))
      .filter((x) => !x.endsWith('deep')), sum(await call(ctx, 'operations/list', { fs: BACK, remote: 'sub', opt: '{"recurse":true}' }))],
    [['sub/b.txt', 'sub/deep/c.txt'], 3]);

  // 结尾斜杠与 `remote` 上的结尾斜杠：前者无害，后者仍是那个静默空列表（§4/§5 量过的同一格，
  // 这里补本地 fs 那一档，因为转换只保证去掉尾部，不能替 rclone 说话）。
  check('F16 a trailing slash on the local fs changes nothing (the conversion may keep or trim it)',
    paths(await call(ctx, 'operations/list', { fs: `${FWD}/`, remote: '', opt: '{"recurse":true}' })), whole);
  check('F16 trailing slash on remote still lists NOTHING on a local fs (the client strips it before sending)',
    (await call(ctx, 'operations/list', { fs: BACK, remote: 'sub/' })).json.list, []);

  ctx.child.kill(); filt.child.kill();
  await sleep(400);
}

console.log(`\n# ${failures} failing check(s)`);
process.exit(failures === 0 ? 0 : 1);
