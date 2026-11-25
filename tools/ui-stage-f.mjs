// Run against a disposable packaged panel using its CDP port, never the regular profile.
// --startup=<fixture> checks the real startup failure of a fixture with invalid jobs.toml.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { cdpEndpoint } from './cdp-endpoint.mjs';

const startup = process.argv.find((arg) => arg.startsWith('--startup='))?.slice(10);
const fixture = process.argv.find((arg) => arg.startsWith('--fixture='))?.slice(10) ?? startup;
const pollOnly = process.argv.includes('--poll-only');
const pairsOnly = process.argv.includes('--pairs-only');
const preferencesOnly = process.argv.includes('--preferences-only');
assert(fixture, 'Run with --fixture=<disposable home> or --startup=<invalid disposable home>');
let page;
for (let i = 0; i < 60 && !page; i++) {
  const targets = await (await fetch(cdpEndpoint(fixture))).json();
  page = targets.find((target) => target.type === 'page' && /^https?:\/\//.test(target.url));
  if (!page) await delay(100);
}
assert(page, 'Fixture panel has no WebView2 page');
const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((resolve, reject) => { ws.onopen = resolve; ws.onerror = reject; });
let seq = 0;
const pending = new Map();
const exceptions = [];
ws.onmessage = ({ data }) => {
  const response = JSON.parse(data);
  if (response.method === 'Runtime.exceptionThrown') {
    exceptions.push(response.params.exceptionDetails.exception?.description);
  }
  const entry = pending.get(response.id);
  if (!entry) return;
  pending.delete(response.id);
  if (response.error) entry.reject(new Error(JSON.stringify(response.error)));
  else entry.resolve(response.result);
};
function send(method, params = {}) {
  const id = ++seq;
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      pending.delete(id);
      reject(new Error('CDP did not answer ' + method));
    }, 10000);
    pending.set(id, {
      resolve: (result) => { clearTimeout(timer); resolve(result); },
      reject: (error) => { clearTimeout(timer); reject(error); },
    });
    ws.send(JSON.stringify({ id, method, params }));
  });
}
async function evaluate(expression) {
  const result = await send('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
  if (result.exceptionDetails) throw new Error(JSON.stringify(result.exceptionDetails));
  return result.result.value;
}
async function until(expression, description) {
  for (let i = 0; i < 100; i++) {
    const value = await evaluate(expression);
    if (value) return value;
    await delay(70);
  }
  throw new Error(description + '\n' + exceptions.join('\n') + '\n'
    + await evaluate(`location.href + "\\n" + document.body.innerText
      + (typeof stageF === 'object' ? "\\n" + JSON.stringify({
        calls: stageF.calls.slice(-18), state: stageF.state, next: stageF.next,
      }) : "")`));
}
const shows = (text) => 'document.body.innerText.includes(' + JSON.stringify(text) + ')';
async function click(label, selector = 'button, a, .ant-segmented-item-label, .ant-dropdown-menu-item') {
  const point = await until(String.raw`(() => {
    const elements = [...document.querySelectorAll(${JSON.stringify(selector)})];
    const hit = elements.find(e => e.getClientRects().length && !e.disabled
      && (e.innerText || '').replace(/\s/g, '') === ${JSON.stringify(label.replace(/\s/g, ''))});
    if (!hit) return false;
    hit.scrollIntoView({ block: 'nearest' });
    const { x, y, width, height } = hit.getBoundingClientRect();
    const cx = x + width / 2, cy = y + height / 2;
    return hit.contains(document.elementFromPoint(cx, cy)) && { x: cx, y: cy };
  })()`, 'Button or link unavailable: ' + label);
  await send('Input.dispatchMouseEvent', { type: 'mouseMoved', ...point });
  for (const type of ['mousePressed', 'mouseReleased']) {
    await send('Input.dispatchMouseEvent', { type, ...point, button: 'left', clickCount: 1 });
  }
}
async function fill(label, value) {
  await evaluate(`document.querySelector(${JSON.stringify('input[aria-label="' + label + '"]')}).focus()`);
  await send('Input.insertText', { text: value });
}

function fixtureIpc() {
  const realFetch = window.fetch;
  const local = { id: 'D:/stage-f', label: '本地夹具', isLocal: true, pairedWith: 'stage-f:' };
  const remote = { id: 'stage-f:', label: '远端夹具', isLocal: false, pairedWith: local.id };
  const draft = {
    id: 'D:/stage-f-draft', label: '新增草稿目录', isLocal: true, pairedWith: 'stage-f:draft',
  };
  const mock = window.stageF = {
    calls: [], notifications: [], state: { state: 'idle' }, next: 'done', withheld: [], jobs: 'copy',
    draft: false, holdRemote: false, holdJobs: false, heldJobs: [], pendingJobs: 0, peakJobs: 0,
  };
  const preferenceCase = new URL(location.href).searchParams.has('verify-preferences');
  const emptyNote = { short: '', detail: '' };
  const done = (dry) => ({
    state: 'done', jobid: 11, dry_run: dry, previewed: null,
    copied_files: ['changed.txt'], deleted_files: [],
    stats: {
      transfers: 1, totalTransfers: 1, checks: 0, totalChecks: 0, bytes: 1, totalBytes: 1, speed: 1,
      eta: null, errors: 0, deletes: 0, deletedDirs: 0, retryError: false, fatalError: false, elapsedTime: 1,
    },
  });
  function job(kind, isDraft = false) {
    return { id: isDraft ? 'temporary' : kind, name: isDraft ? '临时测试任务' : '验收任务 ' + kind,
      kind, src: isDraft ? draft.id : local.id, dst: isDraft ? draft.pairedWith : remote.id,
      openDir: null, lockedBy: null, srcIsLocal: true, dstIsLocal: false, isDraft, lastSummary: null,
      pairKey: isDraft ? 'draft-pair' : 'pair', pairSlot: 'a', aIsLocal: true };
  }
  function listing(fs, prefix) {
    const entries = fs === local.id && prefix === '' ? ['slow', 'fast'].map((name) => ({ name, size: 0, isDir: true }))
      : prefix === 'fast' ? [{ name: 'fast/latest.txt', size: 1, isDir: false }]
      : prefix === 'slow' ? [{ name: 'slow/outdated.txt', size: 1, isDir: false }] : [];
    return { path: fs + '/' + prefix, entries, filteredNote: emptyNote, capabilities: emptyNote,
      canClearPrefix: false, clearNote: emptyNote, totalBytes: 0, totalCount: entries.length,
      fileCount: entries.filter((entry) => !entry.isDir).length, dirCount: entries.filter((entry) => entry.isDir).length };
  }
  function reply(value, failure = false) {
    return new Response(JSON.stringify(value), { headers: { 'Content-Type': 'application/json',
      'Tauri-Response': failure ? 'error' : 'ok' } });
  }
  const terminal = (dry) => {
    if (mock.next === 'done') return done(dry);
    if (mock.next === 'unknown') return { state: 'unknown', reason: '验收回执未知' };
    const error = mock.next === 'partial'
      ? { short: '已删除 2 个文件，删除失败。', detail: '第三项 third.txt 失败。' }
      : { short: '验收任务失败原因', detail: '模拟复制失败' };
    return { state: 'failed', jobid: 11, error };
  };
  new MutationObserver(() => {
    for (const notice of document.querySelectorAll('.ant-notification-notice-success, .ant-notification-notice-info')) {
      if (!notice.hasAttribute('data-stage-f-seen')) {
        notice.setAttribute('data-stage-f-seen', '');
        mock.notifications.push(notice.textContent);
      }
    }
  }).observe(document, { subtree: true, childList: true });
  window.fetch = async function (resource, options) {
    const url = new URL(resource, location.href);
    if (url.hostname !== 'ipc.localhost' && url.protocol !== 'ipc:') return realFetch.call(window, resource, options);
    const cmd = url.pathname.replace(/^\//, '') || url.hostname;
    const payload = JSON.parse(options?.body || '{}');
    mock.calls.push(cmd);
    switch (cmd) {
      case 'rcd_status':
        return reply({ ok: true, alive: true, port: 1, pid: 1, rcloneVersion: 'fixture', executeId: '',
          startupError: null, locks: {}, logDir: '', configPath: '', activeCount: 0, sizeOnly: false });
      case 'get_jobs': {
        const jobs = [job(mock.jobs), ...(mock.draft ? [job('copy', true)] : [])];
        if (preferenceCase) jobs.push(
          { ...job('sync'), src: remote.id, dst: local.id, srcIsLocal: false, dstIsLocal: true, pairSlot: 'b' },
          { ...job('copy'), id: 'second', pairKey: 'second', src: 'D:/second', dst: 'stage-f:second' },
        );
        const response = reply(jobs);
        if (!mock.holdJobs) return response;
        mock.peakJobs = Math.max(mock.peakJobs, ++mock.pendingJobs);
        return new Promise((resolve) => mock.heldJobs.push(() => {
          mock.pendingJobs--;
          resolve(response);
        }));
      }
      case 'job_state': return reply(mock.state);
      case 'tail_log': return reply({ text: '', logPath: '' });
      case 'run_job':
      case 'run_mirror':
        mock.state = terminal(payload.dryRun ?? false);
        if (cmd === 'run_mirror' && mock.state.state === 'done') {
          mock.state.stats.deletes = 1;
          mock.state.deleted_files = ['extra.txt'];
        }
        return reply(mock.state);
      case 'run_inventory':
        return reply({ emptyBasis: false, orphans: [], pendingUpload: [], localCount: 1, remoteCount: 1,
          dstIsLocal: false, measuredAt: 'stage-f-time', listFile: '', diverged: [] });
      case 'confirm_mirror': return reply('stage-f-token');
      case 'browser_remotes':
        return reply([local, remote, ...(mock.draft ? [draft, {
          id: draft.pairedWith, label: '新增草稿远端', isLocal: false, pairedWith: draft.id,
        }] : [])]);
      case 'browser_list': {
        const { fs, prefix } = payload;
        const value = listing(fs, prefix);
        if (prefix === 'slow' || (fs === remote.id && mock.holdRemote)) {
          return new Promise((resolve) => mock.withheld.push((error) =>
            resolve(error ? reply({ short: '过期列表出错', detail: '' }, true) : reply(value))));
        }
        return reply(value);
      }
      case 'pair_draft':
        mock.draft = true;
        return reply('draft-pair');
      case 'drop_draft':
        mock.draft = false;
        return reply(null);
      case 'settings_current':
        return reply({ short: '测试设置读失败', detail: '验收夹具' }, true);
      default: return realFetch.call(window, resource, options);
    }
  };
}

const checks = [
  ['task buttons', async () => {
    await until(shows('后台就绪'), 'Panel did not load');
    await click('开始复制', '.pairs-cell button');
    await until("stageF.notifications.some(n => n.includes('复制完成'))", 'Done did not report success');
    await until(shows('已复制 1 个文件（1 B） · 已删除 0 个文件'), 'Copy counts are missing');
    await click('查看文件明细');
    await until(shows('changed.txt'), 'The copied path is missing from the result');
    await evaluate("stageF.next = 'failed'");
    await click('开始复制', '.pairs-cell button');
    await until(shows('验收任务失败原因'), 'Failed state is not visible');
    await evaluate("stageF.next = 'unknown'");
    await click('模拟复制', '.pairs-cell button');
    await until(shows('验收回执未知'), 'Unknown state is not visible');
    await delay(200);
    assert.equal(await evaluate('stageF.notifications.length'), 1, 'Failed or Unknown falsely reported success');
    await evaluate("stageF.next = 'done'");
    await click('模拟复制', '.pairs-cell button');
    await until("stageF.notifications.some(n => n.includes('模拟完成，文件未改动'))", 'Done trial did not report success');
    await until(shows('预计复制 1 个文件（1 B） · 预计删除 0 个文件'), 'Dry run counts are missing');
  }],
  ['partial mirror', async () => {
    await evaluate("stageF.jobs = 'sync'; stageF.next = 'partial'; stageF.state = {state:'idle'}");
    await click('确认单向同步', '.pairs-cell button');
    await until(shows('确认单向同步'), 'Mirror confirmation did not open');
    await delay(250);
    await click('核对后开始同步', '.ant-modal button');
    await until("stageF.calls.includes('run_mirror')", 'The mirror confirmation did not reach IPC');
    await until(shows('已删除 2 个文件，删除失败。'), 'Partial completion count is not visible');
    assert.equal(await evaluate("stageF.notifications.some(n => n.includes('同步完成'))"), false);
    await until(`[...document.querySelectorAll('.ant-modal button')].some(
      b => b.disabled && b.innerText.includes('重新生成删除清单'))`, 'The spent confirmation is still actionable');
    await click('为什么？', '.ant-modal button, .ant-modal a');
    await until(shows('第三项 third.txt 失败。'), 'The failed item is missing from the detail');
    await click('取消', '.ant-modal button');
  }],
  ['completed mirror', async () => {
    await evaluate("stageF.jobs = 'sync'");
    await click('确认单向同步', '.pairs-cell button');
    await until(shows('确认单向同步'), 'Mirror confirmation did not open');
    await delay(250);
    await click('核对后开始同步', '.ant-modal button');
    await until("stageF.notifications.some(n => n.includes('同步完成'))", 'Done mirror did not report success');
    await until(shows('已删除 1 个文件'), 'Mirror deletion count is missing');
    await click('查看文件明细');
    await until(shows('extra.txt'), 'The deleted path is missing from the result');
  }],
  ['late navigation and a closed comparison', async () => {
    await click('浏览文件');
    await until(shows('slow'), 'Local root has no folders');
    await click('进入', 'tr[data-row-key="slow"] button');
    await until('stageF.withheld.length > 0', 'Slow navigation was not requested');
    await click('本地夹具', '.ant-breadcrumb a');
    await until(shows('fast'), 'Root did not return');
    await click('进入', 'tr[data-row-key="fast"] button');
    await until(shows('latest.txt'), 'Fast navigation did not render');
    await evaluate('stageF.withheld.splice(0).forEach(reply => reply(true))');
    await delay(200);
    assert(await evaluate(shows('latest.txt')), 'Late local response overwrote the newer listing');
    assert(!await evaluate(shows('过期列表出错')), 'Late local failure was displayed');
    await evaluate('stageF.holdRemote = true');
    await click('和云端比一比');
    await until('stageF.withheld.length > 0', 'Comparison did not request the right pane');
    await click('不比了，回去看本地');
    await evaluate('stageF.withheld.splice(0).forEach(reply => reply(false))');
    await delay(250);
    assert(!await evaluate(shows('不比了，回去看本地')), 'A late remote response reopened comparison');
  }],
  ['settings retry', async () => {
    await click('设置');
    await until(shows('测试设置读失败'), 'Settings did not show the failure');
    await delay(450);
    assert.equal(await evaluate("stageF.calls.filter(c => c === 'settings_current').length"), 1);
    await click('再读一次');
    await until("stageF.calls.filter(c => c === 'settings_current').length >= 2", 'Manual retry did not read settings');
    await click('文件任务');
    await click('设置');
    await delay(250);
    assert.equal(await evaluate("stageF.calls.filter(c => c === 'settings_current').length"), 2);
  }],
  ['recent selections survive reload', async () => {
    const add = '复制（保留目标多余文件） —— 从本地复制到云端';
    const mirror = '单向同步（删除目标多余文件） —— 以云端为主（删本地多余的）';
    const firstPair = 'stage-f ↔ stage-f:';
    const secondPair = 'second ↔ stage-f:second';
    const selectors = '.pair-bar .ant-select';
    const options = '.ant-select-dropdown:not(.ant-select-dropdown-hidden) .ant-select-item-option';
    const select = async (from, to) => {
      await click(from, selectors);
      await until(`(() => {
        const item = [...document.querySelectorAll(${JSON.stringify(options)})].find(
          (option) => option.getClientRects().length && option.innerText.trim() === ${JSON.stringify(to)});
        if (!item) return false;
        item.click();
        return true;
      })()`, 'The dropdown did not offer ' + to);
    };
    const currentPreference = "JSON.parse(localStorage.getItem('deck.pair'))";
    await select(add, mirror);
    await until(`${currentPreference}?.operations.pair?.mode === 'mirror'`, 'The sync choice was not saved');
    await select(firstPair, secondPair);
    await until(`${currentPreference}?.pairKey === 'second'`, 'The second pair was not saved');

    await click('浏览文件');
    await click('进入', 'tr[data-row-key="fast"] button');
    await until(shows('latest.txt'), 'The initial local directory did not open');
    await click('和云端比一比');
    await until("stageF.calls.filter(c => c === 'browser_list').length === 3", 'Remote comparison did not open');
    await click('文件任务');
    await click('日志');

    const previous = await evaluate('performance.timeOrigin');
    await send('Page.reload');
    await until(`performance.timeOrigin !== ${previous} && ${shows(secondPair)}`, 'Last pair was not restored');
    assert.equal(await evaluate(`${currentPreference}?.pairKey`), 'second');
    assert.equal(await evaluate("stageF.calls.includes('browser_list')"), false, 'The hidden browser listed files on startup');
    assert.equal(await evaluate("stageF.calls.some(c => /^(preview_mirror|confirm_mirror|run_mirror)$/.test(c))"), false);
    await select(secondPair, firstPair);
    await until(shows(mirror), 'The first pair did not restore its sync direction');
    assert.equal(await evaluate("document.querySelector('.ant-layout-content .ant-segmented-item-selected').textContent.trim()"), '日志');
    await click('浏览文件');
    await until(`[...document.querySelectorAll('.ant-breadcrumb')].filter(e => e.textContent.includes('fast')).length === 2`,
      'Local and remote directories were not restored');
    assert.equal(await evaluate("stageF.calls.filter(c => c === 'browser_list').length"), 2);
  }],
  ['difference report reuse', async () => {
    const requests = "stageF.calls.filter(c => c === 'run_inventory').length";
    await click('看差异', '.pairs-cell button');
    await until(shows('上次比较于 stage-f-time'), 'The first report did not render');
    assert.equal(await evaluate(requests), 1);
    await click('看差异', '.pairs-cell button');
    await delay(250);
    assert.equal(await evaluate(requests), 1, 'Reopening the report requested another comparison');
    await click('重新比较');
    await until(requests + ' === 2', 'Explicit refresh did not request a new comparison');
  }],
  ['draft roots', async () => {
    await click('文件任务');
    await click('添加位置配对');
    await until(`document.querySelector('input[aria-label="这一侧"]')?.getClientRects().length`, 'Draft dialog did not open');
    await delay(250);
    await fill('这一侧', 'D:/stage-f-draft');
    await fill('那一侧', 'stage-f:draft');
    await click('确认配对', '.ant-modal button');
    await until(shows('临时测试任务'), 'Draft did not open');
    await click('浏览文件');
    await click('本地夹具', '.ant-select');
    await until(shows('新增草稿目录'), 'The new local root is missing from the selector');
    await click('新增草稿目录', '.ant-select-item-option');
    await click('文件任务');
    await click('更多', '.pairs-cell button');
    await until(shows('移除临时操作'), 'The draft menu did not open');
    await delay(350);
    await click('移除临时操作', '.ant-dropdown-menu-title-content');
    await until('!stageF.draft', 'The draft was not dropped');
    await click('浏览文件');
    await until(shows('本地夹具'), 'Removed root did not fall back to a valid local root');
    assert(!await evaluate(shows('新增草稿目录')), 'A dropped draft root is still shown');
  }],
];

const pollCheck = ['one refresh in flight', async () => {
  await until(shows('验收任务 copy'), 'Fixture did not publish the initial task');
  await evaluate('stageF.holdJobs = true');
  await until('stageF.pendingJobs > 0', 'No polling request was held');
  await click('开始复制', '.pairs-cell button');
  await until("stageF.calls.includes('run_job')", 'The explicit refresh action did not run');
  await delay(2400);
  const peak = await evaluate('stageF.peakJobs');
  await evaluate('stageF.holdJobs = false; stageF.heldJobs.splice(0).forEach(release => release())');
  assert.equal(peak, 1, 'More than one refresh reached IPC while a round was held');
  await until(shows('复制完成'), 'The explicit refresh did not complete after the held round');
  const calls = await evaluate("stageF.calls.filter(cmd => cmd === 'get_jobs').length");
  await until(`stageF.calls.filter(cmd => cmd === 'get_jobs').length > ${calls}`, 'Polling did not resume');
}];

try {
  if (startup) {
    await until(shows('后台没能启动'), 'Startup failure is not visible');
    const actual = await evaluate(`[...document.querySelectorAll('.deck-notice [title]')]
      .map(node => node.title).find(title => title.endsWith('jobs.toml'))`);
    assert.equal(path.normalize(actual ?? ''), path.join(startup, 'jobs.toml'));
    console.log('PASS startup failure displays the actual configuration path');
  } else {
    await until("!!document.querySelector('.ant-layout')", 'Fixture panel did not load before setup');
    if (pairsOnly) {
      await click('复制（保留目标多余文件） —— 从本地复制到云端', '.pair-bar .ant-select');
      await until("document.querySelectorAll('.ant-select-dropdown:not(.ant-select-dropdown-hidden) .ant-select-item-option').length === 4",
        'A saved local/remote pair did not offer all four operations');
      await send('Input.dispatchKeyEvent', { type: 'keyDown', key: 'Escape', code: 'Escape', windowsVirtualKeyCode: 27 });
      await send('Input.dispatchKeyEvent', { type: 'keyUp', key: 'Escape', code: 'Escape', windowsVirtualKeyCode: 27 });
      console.log('PASS one saved job exposes four operations');
    }
    if (process.argv.includes('--native-copy')) {
      await click('开始复制', '.pairs-cell button');
      await until(shows('复制完成'), 'Real rclone transfer did not report success');
      assert.equal(await readFile(path.join(fixture, 'remote', 'native-copy.txt'), 'utf8'), 'native-stage-f-copy');
      await until(shows('已复制 1 个文件'), 'Real rclone transfer count is missing');
      await click('查看文件明细');
      await until(shows('native-copy.txt'), 'Real rclone transfer path is missing');
      await click('开始复制', '.pairs-cell button');
      await until(shows('本次没有复制或删除文件'), 'An unchanged run did not report zero changes');
      console.log('PASS native transfer counts, file paths, and unchanged run');
    }
    await send('Runtime.enable');
    await send('Page.enable');
    await send('Page.addScriptToEvaluateOnNewDocument', {
      source: `(${fixtureIpc.toString()})();`,
    });
    let failed = 0;
    const selectedChecks = pairsOnly ? checks.filter(([name]) => name === 'difference report reuse') :
      preferencesOnly ? checks.filter(([name]) => name === 'recent selections survive reload') :
      pollOnly ? [pollCheck] : [...checks, pollCheck];
    for (const [index, [name, check]] of selectedChecks.entries()) {
      await evaluate('localStorage.clear()');
      const previousDocument = await evaluate('performance.timeOrigin');
      const address = new URL(page.url);
      address.searchParams.set('stage-f', index);
      if (name === 'recent selections survive reload') address.searchParams.set('verify-preferences', '1');
      await send('Page.navigate', { url: address.href });
      await until(`performance.timeOrigin !== ${previousDocument} && typeof stageF === 'object'
        && !!document.querySelector('.ant-layout')`, 'Fixture did not boot into ' + name);
      try {
        await check();
        console.log('PASS ' + name);
      } catch (error) {
        failed++;
        console.error('FAIL ' + name + '\n' + (error.stack ?? error));
      }
    }
    assert.equal(failed, 0);
  }
} finally {
  ws.close();
}
