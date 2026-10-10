// CDP harness for the rclone-deck window: drives and screenshots the REAL WebView2
// that `npm run tauri dev` opens, via the WebView2 DevTools endpoint (`cdp-endpoint.mjs` —
// the port is picked by the OS, it is NOT a fixed 9222; see why there).
// No new dependencies: Node 24 ships a global WebSocket client.
//
//   node tools/cdp-harness.mjs shot <absolute.png> [scale]
//   node tools/cdp-harness.mjs text                       // visible text of the window
//   node tools/cdp-harness.mjs click "<visible text>"     // trusted mouse event
//   node tools/cdp-harness.mjs tick "<row text>"          // click a table row's checkbox
//   node tools/cdp-harness.mjs eval "<js expression>"     // Runtime.evaluate, JSON printed
import { cdpEndpoint } from './cdp-endpoint.mjs';

const [cmd, ...args] = process.argv.slice(2);

async function connect() {
  const res = await fetch(cdpEndpoint());
  const targets = await res.json();
  const page = targets.find((t) => t.type === 'page' && !t.url.startsWith('devtools://'));
  if (!page) throw new Error('没有可驱动的页面：' + JSON.stringify(targets.map((t) => t.url)));
  const ws = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((ok, bad) => { ws.onopen = ok; ws.onerror = bad; });
  let seq = 0;
  const pending = new Map();
  ws.onmessage = (ev) => {
    const msg = JSON.parse(ev.data);
    const p = pending.get(msg.id);
    if (p) { pending.delete(msg.id); msg.error ? p.reject(new Error(JSON.stringify(msg.error))) : p.resolve(msg.result); }
  };
  const send = (method, params = {}) => new Promise((resolve, reject) => {
    const id = ++seq;
    pending.set(id, { resolve, reject });
    ws.send(JSON.stringify({ id, method, params }));
  });
  return { send, ws, url: page.url };
}

async function clickAt(send, x, y) {
  await send('Input.dispatchMouseEvent', { type: 'mouseMoved', x, y });
  await send('Input.dispatchMouseEvent', { type: 'mousePressed', x, y, button: 'left', clickCount: 1 });
  await send('Input.dispatchMouseEvent', { type: 'mouseReleased', x, y, button: 'left', clickCount: 1 });
}

async function findPoint(send, js) {
  const r = await send('Runtime.evaluate', { expression: js, returnByValue: true, awaitPromise: true });
  if (r.exceptionDetails) throw new Error(r.exceptionDetails.text + ' ' + JSON.stringify(r.result));
  return r.result.value;
}

const { send, ws } = await connect();
await send('Runtime.enable');
await send('Page.enable');

if (cmd === 'text') {
  const t = await findPoint(send, 'document.body.innerText');
  console.log(t);
} else if (cmd === 'console') {
  // 把 N 毫秒内的 console/Log 事件全部打出来（排页面为什么没渲染时用）。
  const lines = [];
  ws.addEventListener('message', (ev) => {
    const m = JSON.parse(ev.data);
    if (m.method === 'Runtime.consoleAPICalled') {
      lines.push(m.params.type + ': ' + (m.params.args || []).map((a) => a.value ?? a.description ?? a.type).join(' '));
    } else if (m.method === 'Log.entryAdded') {
      lines.push(m.params.entry.level + ': ' + m.params.entry.text + ' @' + (m.params.entry.url || ''));
    } else if (m.method === 'Runtime.exceptionThrown') {
      lines.push('exception: ' + JSON.stringify(m.params.exceptionDetails).slice(0, 500));
    }
  });
  await send('Runtime.enable');
  await send('Log.enable');
  if (args[0] === 'reload') await send('Page.reload', { ignoreCache: false });
  await new Promise((r) => setTimeout(r, Number(args[1] || args[0] || 4000)));
  console.log(lines.join('\n') || '(没有 console 输出)');
} else if (cmd === 'nav') {
  await send('Page.navigate', { url: args[0] });
  console.log('navigated to ' + args[0]);
} else if (cmd === 'eval') {
  console.log(JSON.stringify(await findPoint(send, args[0]), null, 2));
} else if (cmd === 'click') {
  const want = args[0];
  const pt = await findPoint(send, `(() => {
    const want = ${JSON.stringify(want)};
    const pool = [...document.querySelectorAll('button, a, .ant-tag, .ant-segmented-item, .ant-radio-button-wrapper, td, .ant-checkbox-wrapper, .ant-list-item, .ant-select, .ant-select-item-option')];
    const hits = pool.filter((e) => e.getClientRects().length && (e.textContent || '').trim().includes(want));
    // 取最深的那个匹配：LI 的 textContent 也含按钮文字，按文档顺序它会先命中，
    // 点它的中心会落在描述文字上而不是按钮上。
    const inner = hits.filter((e) => !hits.some((o) => o !== e && e.contains(o)));
    const hit = inner[${Number(args[1] || 0)}];
    if (!hit) return { err: '找不到带这段文字的可见元素：' + want };
    hit.scrollIntoView({ block: 'center' });
    const b = hit.getBoundingClientRect();
    return { x: b.x + b.width / 2, y: b.y + b.height / 2, tag: hit.tagName, text: hit.textContent.trim().slice(0, 60) };
  })()`);
  if (pt.err) throw new Error(pt.err);
  await clickAt(send, pt.x, pt.y);
  console.log('clicked ' + JSON.stringify(pt));
} else if (cmd === 'hover') {
  // 只发一次可信的 mouseMoved（不按下、不抬起）：给 disabled 控件的 Tooltip 用。
  const want = args[0];
  const pt = await findPoint(send, `(() => {
    const want = ${JSON.stringify(want)};
    const pool = [...document.querySelectorAll('button, a, .ant-tag, td, .ant-list-item, span')];
    const hits = pool.filter((e) => e.getClientRects().length && (e.textContent || '').trim() === want.trim());
    const inner = hits.filter((e) => !hits.some((o) => o !== e && e.contains(o)));
    const hit = inner[${Number(args[1] || 0)}] || inner[0];
    if (!hit) return { err: '找不到这段文字的可见元素：' + want };
    const b = hit.getBoundingClientRect();
    return { x: b.x + b.width / 2, y: b.y + b.height / 2 };
  })()`);
  if (pt.err) throw new Error(pt.err);
  await send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: pt.x, y: pt.y });
  console.log('hovered ' + JSON.stringify(pt));
} else if (cmd === 'hoverxy') {
  // 可信 mouseMoved 到给定坐标（不按下/不抬起）：Tooltip 只在 hover 时出现。
  await send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: Number(args[0]), y: Number(args[1]) });
  console.log('hovered ' + args[0] + ',' + args[1]);
} else if (cmd === 'tick') {
  const want = args[0];
  const pt = await findPoint(send, `(() => {
    const want = ${JSON.stringify(want)};
    const row = [...document.querySelectorAll('tr')].find((r) => (r.textContent || '').includes(want));
    if (!row) return { err: '找不到含「' + want + '」的那一行' };
    const box = row.querySelector('.ant-checkbox, input[type=checkbox]');
    if (!box) return { err: '那一行里没有勾选框' };
    box.scrollIntoView({ block: 'center' });
    const b = box.getBoundingClientRect();
    return { x: b.x + b.width / 2, y: b.y + b.height / 2 };
  })()`);
  if (pt.err) throw new Error(pt.err);
  await clickAt(send, pt.x, pt.y);
  console.log('ticked ' + JSON.stringify(pt));
} else if (cmd === 'shot') {
  const out = args[0];
  const scale = Number(args[1] || 2);
  const size = await findPoint(send, '({ w: window.innerWidth, h: window.innerHeight })');
  const s = await send('Page.captureScreenshot', {
    format: 'png',
    captureBeyondViewport: true,
    clip: { x: 0, y: 0, width: size.w, height: size.h, scale },
  });
  const fs = await import('node:fs');
  fs.writeFileSync(out, Buffer.from(s.data, 'base64'));
  console.log(`saved ${out} (${size.w}x${size.h} @${scale}x)`);
} else {
  throw new Error('用法：shot|text|click|tick|eval');
}

ws.close();
