// 界面整修的验收驱动器：在**真窗口**里逐动作走一遍，每屏一张 PNG + 一行 DOM 读数。
// 用法：node tools/ui-acceptance.mjs --home=A --from=0 --to=20 --out=docs/superpowers/evidence/ui/<名字>
// 读数写进 <out>/readings.txt：含中文的行会被终端回显吞掉，所以判定必须读文件，不读 stdout。
import fs from 'node:fs';
import path from 'node:path';
import { tmpdir } from 'node:os';
import { execSync } from 'node:child_process';
import { cdpEndpoint } from './cdp-endpoint.mjs';

const arg = (name, dflt) => {
  const hit = process.argv.find((a) => a.startsWith('--' + name + '='));
  return hit ? hit.split('=').slice(1).join('=') : dflt;
};
const HOME = (arg('home', 'A') || 'A').toUpperCase();
// 每一片的屏落在自己的目录里：before / after 成对留下来，才有同一屏改前改后可对。
const OUT = arg('out', 'docs/superpowers/evidence/ui/latest');
fs.mkdirSync(OUT, { recursive: true });
const LOG = OUT + '/readings.txt';
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
// 夹具家跟着**这台机器的** %TEMP% 走（原来抄的是作者那台的一条绝对路径，
// 别人克隆下来跑驱动器会直接死在"找不到那台家"上）。
const PROC = { home: HOME, remote: HOME === 'A' ? 'fxa' : 'fxb',
  fx: path.join(tmpdir(), 'deck-ui-' + HOME) };
// 配对卡那两对位置的远端那一侧：名字跟着夹具家走（A 家 fxa、B 家 fxb）。写死 'fxa:c'
// 的那一遍在 B 家报「找不到那张卡」，读起来像界面坏了，其实是尺子把另一家的名字抄进了判据。
const PAIR_C = PROC.remote + ':c';
const PAIR_B = PROC.remote + ':b';
// 「换一对位置」那张临时卡用的**一对没配过的位置**：本地那一侧是夹具里一直在的 `local/`
// （它是 fx-gate 的源，所以它出现在候选里），云端那一侧 `:d` 是这一遍才有的层。
// 名字跟着家走（A 家 fxa:d、B 家 fxb:d）：写死 fxa 的那把尺子在 B 家报"找不到那张卡"，
// 读起来像界面坏了。
const DRAFT_LOCAL = PROC.fx + '/local';
const DRAFT_PAIR = PROC.remote + ':d';
const DRAFT_DIR = PROC.fx + '/bucket/d';
// 「存成常用 → 保存 → 重开」那一整圈用的是**另一对**位置（`:e`），不是上面那对 `:d`：
// `:d` 那张留给"重开之后它不在了"那一格（R96）。同一对既当"散了"又当"回来了"，
// 那两格就会互相把对方的前提拆掉，红的读起来像界面坏了。
const SAVE_PAIR = PROC.remote + ':e';
const SAVE_ID = 'accept-keep-' + HOME.toLowerCase();
const SAVE_NAME = '验收存下来的一条';

/**
 * 配对那三枚下拉在屏上的位置 —— **只留这一处**。
 *
 * 2026-10-08 界面简化那一轮把「位置 / 模式 / 方向」从左列提成**横跨整窗的选择条**
 * （`.pair-bar`，它是 `.ant-layout-sider` 的**兄弟**节点，不再是后代的）。真窗口量过的：
 *   旧定位 `.ant-layout-sider .ant-select` 匹配 **0** 枚（原先 3 枚）——
 *   不是"读错盒子"，是 `openCardSelect` / `selectValue` / `selectWhy` 与找「模式」
 *   那几处会直接报"找不到那一格"；
 *   新定位 `.pair-bar .ant-select` 匹配 **3** 枚，而三个 `aria-label`
 *   （位置 / 模式 / 方向）在**可见**元素里各恰好 1 个 —— 所以这个替换是精确的，不会多认。
 * 抄第二份的下一处就会静默落空（量不到不等于界面坏了），所以下面四处都引这一个。
 *
 * 2026-10-09：**这一行现在是 2 枚**（位置 / 要做什么）。模式与方向并进后一枚之后，
 * 按 3 枚编号写死的那几处读数全错位：`selectValue(2)` 报「选择条上只有 2 枚下拉」，
 * `topSelectBox(2)` 同理，而 `aria-label` 也从「模式」变成了「要做什么」。
 * 所以认那一枚一律走 `positionSelect`（按 `aria-label` 认，不按下标）——
 * 下标指到「要做什么」上时驱动会去"换位置"却其实换了要做的事，量的却是另一格，
 * 而且读数看着完全正常；`aria-label` 认不出会报出来，那才是尺子坏了的样子。
 */
const SEL_PAIR_BAR = '.pair-bar .ant-select';
/** 选择条上现在有哪几枚，**以及它们的顺序**。**从屏上数，不写死** ——
 *  写死 `['位置', '模式', '方向']` 的那一份在 2026-10-09 合并之后就变成了三枚里有两枚不存在，
 *  而报出来的是"点开了一个空弹层"，读着像界面坏了。
 *  按屏上的 `aria-label` 当场读：多一枚少一枚都能读到，而"少的那一枚"本身就是一条读数。
 *  （早先在这里试过让一段表达式**返回 DOM 节点**，那是过不去的 —— `ev()` 走 JSON，
 *  节点会变成 `{}`。认框一律在页面里做，只把坐标/文字带回来。） */
async function pairBarLabels() {
  return String(await ev(`(() => {
    const aria = ${ariaOf};
    return [...document.querySelectorAll('${SEL_PAIR_BAR}')]
      .filter((e) => e.getBoundingClientRect().width)
      .map((e) => aria(e) || '?').join('|');
  })()`)).split('|');
}

/** 夹具那一份 `jobs.toml` 里的**任务身份**：名字 ↔ 那两条完整路径 + kind。
 *  卡上那行名字撤了（它在两行之内复述了「模式」与「方向」那两枚框），于是
 *  "当前这一格与右列讲的是同一条"不能再拿一个同时出现在两处的字符串认 —— 那等于把判据
 *  押在最该省掉的那一行上。改成两处各读各的，再拿**后台那份数据**对上：
 *  格上那两条路径（顺序也算）必须是右列那一行那条任务在文件里的 `src`/`dst`。
 *  每次现读、不冻结：设置面那几步会改这份文件，读一次就定型的名单在后半遍里会说瞎话。 */
function fixtureJobs() {
  const txt = fs.readFileSync(PROC.fx + '/jobs.toml', 'utf8');
  const flat = (s) => s.replace(/\\/g, '/');
  const one = (re, s) => { const m = s.match(re); return m ? m[1] : ''; };
  return txt.split('[[job]]').slice(1).map((b) => ({
    name: one(/^name = "([^"]+)"/m, b),
    kind: one(/^kind = "([^"]+)"/m, b),
    src: flat(one(/^src = ['"]([^'"]+)['"]/m, b)),
    dst: flat(one(/^dst = ['"]([^'"]+)['"]/m, b)),
  })).filter((j) => j.name !== '');
}

/** 右列开头那一行（四段共用的那一层）写着哪条任务。 */
async function railHead() {
  return String(await ev('(() => { const c=document.querySelector(".ant-layout-content");'
    + 'const e=c?c.querySelector(".ant-typography"):null; return e?e.textContent.trim():""; })()'));
}

/** 当前那一格与右列**对得上吗**：`{ same, job, paths, rail }`。
 *  `job` 是按右列那一行在文件里认出来的那一条；`same` 是"格上那两条路径正是它的那两条，
 *  而且先后也一样"。名字对不上文件（临时格那几条只活在内存里）时 `job` 是 undefined，
 *  那种场合用 `draftCardShape` 读它的 `pos`（位置框里那句「（临时）」），别拿这条判失败。 */
async function cellJob(rowKey) {
  if (rowKey !== undefined) await showPair(rowKey);
  const s = JSON.parse(String(await ev('(() => { const r=document.querySelector(".pairs-cell");'
    + 'if (!r) return JSON.stringify({ err: "当前那一格不在" });'
    + 'const p=r.querySelector(".fs-code");'
    + 'return JSON.stringify({ paths:p?p.textContent.trim():"" }); })()')));
  if (s.err) throw new Error(s.err);
  s.rail = await railHead();
  const flat = (x) => String(x).replace(/\\/g, '/').replace(/\s+/g, '');
  const job = fixtureJobs().find((j) => s.rail.includes(j.name));
  return { ...s, job, same: !!job && flat(job.src + '→' + job.dst) === flat(s.paths) };
}

const lines = [];
const log = (x) => lines.push(typeof x === 'string' ? x : JSON.stringify(x));

/** 这一遍量的必须是**当前源码构建出来的那一份**。
 *  有一遍：源码 08:00 改过而产物还是 07:40 的，于是"某一格点不开"的红一路追到
 *  界面、时序、下拉层数，量的其实是一份旧界面。产物时刻比源码新，是这一遍有意义的前提。 */
function newestSource() {
  let best = { f: '', t: 0 };
  const walk = (d) => {
    for (const e of fs.readdirSync(d, { withFileTypes: true })) {
      const p = path.join(d, e.name);
      if (e.isDirectory()) walk(p);
      else if (/\.(ts|tsx|css)$/.test(e.name)) {
        const t = fs.statSync(p).mtimeMs;
        if (t > best.t) best = { f: p, t };
      }
    }
  };
  walk('src');
  return best;
}
{
  const exes = ['src-tauri/target/release/rclone-deck.exe', 'src-tauri/target/debug/rclone-deck.exe']
    .filter((p) => fs.existsSync(p));
  if (exes.length) {
    const src = newestSource();
    const fresh = exes.map((p) => fs.statSync(p).mtimeMs).some((t) => t >= src.t);
    if (!fresh) {
      throw new Error('产物比源码旧，这一遍量的是旧界面：' + src.f + ' 改于 '
        + new Date(src.t).toISOString() + '，而 ' + exes.join(' / ') + ' 更早。先跑 `npx tauri build --no-bundle`。');
    }
  }
}

async function connect() {
  // 面板从启动到 WebView2 开放调试端口要十几秒（实测 10s 会偶发连不上，而那时
  // 驱动直接死在 fetch 上、什么读数都不留）。等它，而不是把一次启动当成失败。
  // 每一轮都重新问一次端口：那个号是系统挑的（见 `cdp-endpoint.mjs` 里为什么不能写死），
  // 面板重启还会换一次，缓存的那一份就是下一次"连不上"的假象。
  let targets = null;
  let last = '';
  for (let i = 0; i < 40; i++) {
    try { targets = await (await fetch(cdpEndpoint())).json(); break; }
    catch (e) { last = String(e && e.message || e); await sleep(1000); }
  }
  if (!targets) throw new Error('等 40 秒还是没有 CDP 端口：面板起来了吗？最后一次：' + last);
  const page = targets.find((t) => t.type === 'page' && !t.url.startsWith('devtools://'));
  if (!page) throw new Error('没有可驱动的页面');
  const ws = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((ok, bad) => { ws.onopen = ok; ws.onerror = bad; });
  let seq = 0;
  const pending = new Map();
  ws.onmessage = (ev) => {
    const m = JSON.parse(ev.data);
    const p = pending.get(m.id);
    if (p) { pending.delete(m.id); m.error ? p.reject(new Error(JSON.stringify(m.error))) : p.resolve(m.result); }
  };
  const send = (method, params = {}) => new Promise((res, rej) => {
    const id = ++seq; pending.set(id, { resolve: res, reject: rej });
    ws.send(JSON.stringify({ id, method, params }));
  });
  return { send, ws };
}
const { send, ws } = await connect();
await send('Runtime.enable');
await send('Page.enable');

// CDP 通了不等于界面起来了：WebView 先开窗、React 后挂载。上一版第 0 格就在空白页上
// 找「详情」，报"候选=0"，而后面每一格都正常 —— 那种红读起来像缺陷，其实是抢跑。
async function waitReady(timeoutMs = 30000) {
  const t0 = Date.now();
  for (;;) {
    const seen = await ev('(() => document.body && document.body.innerText.includes("配好的位置") ? 1 : 0)()');
    if (seen === 1) return;
    if (Date.now() - t0 > timeoutMs) throw new Error('等 30 秒界面还没挂上（没有"配好的位置"那个分段控件）');
    await sleep(500);
  }
}
await waitReady();

async function ev(expression) {
  const r = await send('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
  if (r.exceptionDetails) throw new Error('表达式出错 ' + JSON.stringify(r.exceptionDetails).slice(0, 260));
  return r.result.value;
}
async function at(x, y, kind) {
  await send('Input.dispatchMouseEvent', { type: kind, x, y, button: 'left', clickCount: 1 });
}
async function clickPoint(pt) {
  await send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: pt.x, y: pt.y });
  await at(pt.x, pt.y, 'mousePressed');
  await at(pt.x, pt.y, 'mouseReleased');
}

/** 找元素：`side` 0=左半（本地栏）1=右半（远端栏）'any'=不限；`inRow` 限定在某一行里。
 *  行这一族两种都要认：浏览器那两面是 `Table`（`.ant-table-row`），对子面与设置面的任务清单是
 *  `Listy`（antd v6 里它的行类名是 `.ant-listy-item`，不是 v5 的 `.ant-list-item`——上一版按后者
 *  写，`inRow` 在列表上永远报"找不到"，而报出来的那句读数看着像界面坏了，其实是尺子只有一半长）。
 *  行也要按宽度筛：三面都一直挂着、只用 display 切换，隐藏面里的行有文本没尺寸，
 *  不筛就会点到一个 0x0 的中心点上。 */
function finder(text, side, nth, inRow) {
  return `(async () => {
    const want = ${JSON.stringify(text)}.replace(/\\s+/g, '');
    const norm = (e) => (e.textContent || '').replace(/\\s+/g, '');
    const half = window.innerWidth / 2;
    const inRow = ${JSON.stringify(inRow || '')};
    let pool;
    if (inRow) {
      const rows = [...document.querySelectorAll('.ant-table-row, .ant-listy-item, .pairs-cell')].filter((r) => {
        const b = r.getBoundingClientRect();
        if (!b.width) return false;
        const sideOk = ${side === 'any' ? 'true' : (side === 0 ? 'b.x < half' : 'b.x >= half - 40')};
        return sideOk && norm(r).includes(inRow.replace(/\\s+/g, ''));
      });
      pool = rows.flatMap((r) => [...r.querySelectorAll('button')]);
    } else {
      pool = [...document.querySelectorAll('button, a, .ant-segmented-item, .ant-select, .ant-select-item-option-content, .ant-select-item, [role=option], .ant-modal .ant-btn, .ant-table-row, .ant-dropdown-menu-item, [role=menuitem]')]
        .filter((e) => {
          const b = e.getBoundingClientRect();
          if (!b.width) return false;
          return ${side === 'any' ? 'true' : (side === 0 ? 'b.x < half' : 'b.x >= half - 40')};
        });
    }
    const hits = pool.filter((e) => norm(e).includes(want));
    const deep = hits.filter((e) => !hits.some((o) => o !== e && e.contains(o)));
    const hit = deep[${nth || 0}];
    if (!hit) return { err: '找不到：' + want + (inRow ? '（在行 ' + inRow + ' 里）' : '') + ' 候选=' + hits.length };
    // 等这一颗**真的站在读到的那个坐标上**再交出去。以前是 scrollIntoView 之后立刻读矩形，
    // 而那正是层还在滑入、列表还在重排的时候 —— 读到的是假位置，点下去落在隔壁那颗上。
    // 一整遍里为此红过三次（灭着的按钮点空、设置面那发删除没落盘、列举途中抢读）。
    // 命中测试说的是"这个坐标上真是它（或它的孩子）"，不是"它曾经在那里"。
    // 要等多久按**飘着的那一层**算：toast 一站就是 3 秒，而它正压在顶栏下面那条分段控件上
    //（实测：两格的「浏览文件」都被上一格留下的 toast 盖住）。老代码不管这些，
    // 一点就落在 toast 上 —— 那一格"过了"，可它点的那一颗根本没换面。等到飘的那层自己走。
    let blocker = '';
    let last = null, still = 0;
    for (let i = 0; i < 110; i++) {
      hit.scrollIntoView({ block: 'center' });
      const b = hit.getBoundingClientRect();
      const x = b.x + b.width / 2, y = b.y + b.height / 2;
      const at = document.elementFromPoint(x, y);
      // ⚠ 命中之后**还要位置不动**才算数（2026-10-10 实测加上的）。
      // 上面那个命中测试说的只是"这一瞬间那个坐标上真是它"——它**不保证那一瞬间之后它还在那儿**。
      // antd 的弹层入场是缩放动画：整层连页脚一起从中心放大到位，而 waitLayer 一看见
      // .ant-modal 有宽度就返回。于是「我知道，保存」那时候还在往最终位置挪，
      // finder 记下坐标、clickPoint 按下去，按钮已经走了 —— **点击落空，而屏上一点异样都没有**。
      //
      // 实测症状：settings-save-remote-line 约一半的遍数里 30 秒超时，读回来那颗按钮**一直是「平着」**。
      // 「平着」就是判据：commit 一进去就 setBusy(true)，antd 立刻加 ant-btn-loading 并置灰 ——
      // 按钮要是平了 30 秒，说明 onClick 从来没被调用过，命令压根没发出去，
      // 而「层还没关」那一句读起来像 apply 慢（实测 apply 真跑起来只要 600 毫秒）。
      // 换句话说：**红的是坐标过期，不是产品慢**。
      //
      // 因此这里要求**连续两次读到的矩形一样**才交出去：一样 = 它不再动了。
      // 预算沿用上面那 110 轮 × 100ms（11 秒），而每轮之间要真的隔开，
      // 所以稳定判定用的是"和上一轮比"，不是一次读两遍 —— 同一轮里连读两次当然一样，那是同一帧。
    //
    // ⚠ 本函数整体是一个**反引号模板字面量**（整段要发到页面里跑），
    //   所以**这一整段里不许出现反引号**，注释里也不行。
    //   违反的后果：node --check 报的是**上面那一行**的 "Unexpected identifier"，
    //   而写错的是下面几十行开外的某个词 —— 同一个坑今晚已经踩过三次。
      const key = Math.round(b.x) + 'x' + Math.round(b.y);
      if (b.width && at && (hit.contains(at) || at.contains(hit))) {
        still = (last === key) ? still + 1 : 0;
        last = key;
        if (still >= 1) {
          return { x, y, tag: hit.tagName, disabled: !!hit.disabled, waited: i, still,
                   text: hit.textContent.trim().slice(0, 50) };
        }
      } else {
        still = 0; last = null;
      }
      blocker = at ? (at.className ? String(at.className).split(' ')[0] : at.tagName.toLowerCase()) : '空';
      await new Promise((r) => setTimeout(r, 100));
    }
    return { err: '那颗等 11 秒也没站定（坐标上总是「' + blocker + '」）：' + want
      + (inRow ? '（在行 ' + inRow + ' 里）' : '') };
  })()`;
}
async function locate(text, side = 'any', nth = 0, inRow) {
  // 2026-10-10：`候选=0` 那一条原来**立刻返回**（finder 第 220 行），于是
  // 「进一个目录 → 固定 sleep(1200) → 找刚渲染出来的那一行」这种写法会在列举
  // 还没回来时就判失败。实测症状：右栏 `fxa/del1` 只列出 keep.webp，
  // 而 drop.webp 在盘上明明存在、按钮（改 名 / 删这一个）也都渲染得好好的。
  // 一遍里连带 `delete-one-layer` / `delete-one-done` / `delete-visible-done` /
  // `rename-collision-stays-open` / `rename-retry-ok` 五条一起红。
  //
  // 底下那个 110 轮 × 100ms 的循环只管「**等它停下来**」，管不了「**等它出现**」——
  // 所以这里补一个同预算的出现等待。两个等待用同一份时间，别一个给 11 秒另一个不给。
  const APPEAR_TRIES = 40, APPEAR_WAIT = 250;
  let pt = await ev(finder(text, side, nth, inRow));
  for (let i = 0; i < APPEAR_TRIES && pt && pt.err; i++) {
    await sleep(APPEAR_WAIT);
    pt = await ev(finder(text, side, nth, inRow));
  }
  if (pt.err && inRow) {
    // 那一面只剩"当前那一格"：要的那一颗可能属于**没站上来**的那一对。
    // 先把那一对站到位置下拉上再找一次；站不上来就还报最后一次那句（别把原因换成尺子的）。
    try { await showPair(inRow); pt = await ev(finder(text, side, nth, inRow)); } catch (_) { /* 留着最后一次的错 */ }
  }
  if (pt.err) throw new Error(pt.err);
  return pt;
}
async function click(text, side = 'any', nth = 0, inRow) {
  const pt = await locate(text, side, nth, inRow);
  await clickPoint(pt);
  return pt;
}
async function clickIfEnabled(text, side = 'any', nth = 0, inRow) {
  const pt = await locate(text, side, nth, inRow).catch(() => null);
  if (!pt || pt.disabled) return false;
  await clickPoint(pt);
  return true;
}
async function hover(text, side = 'any', nth = 0, inRow) {
  const pt = await locate(text, side, nth, inRow);
  await send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: pt.x, y: pt.y });
  await sleep(700);
  return pt;
}
/** 等卡上那颗按钮从"灭"变成"亮"。灭着按下去是什么都不发生（层不会开），而**为什么**灭写在卡上那句
 *  「正在写这棵目录树：X」里 —— 所以这里把那句话一起带回去当读数，而不是让人只看到一句"正文为空"。
 *  实测到的形状：上一格（`settings-save-while-running`）故意留了一次 6000 个文件的传输在跑，
 *  整遍跑到这一格时那颗还是灭的；单独跑这两格则一直是亮的。界面按设计拒绝"同一棵树上有事在跑再起一个"，
 *
 * 2026-10-10 加 `inRow`：**按那颗按钮所在的格去找，不要全屏找**。
 * 原因是「让两边对齐」这个词在屏上有两处：卡上那颗按钮，和选择条里「要做什么」那一枚
 * 下拉框的**显示值**（换到那一档之后框里就写着它）。而选择条在 DOM 里排在卡前面，
 * `locate` 取文档顺序第一个命中 —— 于是点下去落在下拉框上，层当然不开。
 * 实测读数（`mirror-layer-both-halves` 那一步自己打的）：
 * `MIRROR-CLICK 命中=<div.input.ant-select-input|div.ant-select-content|div.ant-select>`，
 * 坐标 (371,162) 正落在选择条那一行上。而"点空了"与"点中了但层没开"在屏上一模一样。
 * `finder` 带 `inRow` 时只收 `button`，于是那一枚下拉框根本不在候选里。
 *  红的是驱动抢跑，不是界面。 */
async function waitEnabled(text, timeoutMs = 300000, inRow) {
  const t0 = Date.now();
  for (;;) {
    const hit = await locate(text, 'any', 0, inRow);
    const hint = String(await ev('(() => { const x = [...document.querySelectorAll(".pairs-cell, .ant-listy-item")]'
      + '.find((i) => (i.innerText || "").includes(' + JSON.stringify(text) + '));'
      + 'return x ? x.innerText.replace(/\\s+/g, " ").slice(0, 160) : "没有那张卡"; })()'));
    if (!hit.disabled) return { ms: Date.now() - t0, hit, hint };
    if (Date.now() - t0 > timeoutMs) throw new Error('「' + text + '」那颗等了 '
      + Math.round((Date.now() - t0) / 1000) + ' 秒还是灭的，卡上写着：' + hint);
    await sleep(3000);
  }
}
/** 等**没有任务在跑**。判据就是界面自己那道闸：卡上任何一颗亮着的按钮都表示没在跑（灭 = `busy || locked || running`），
 *  再加一句「正在写这棵目录树：X」必须不在屏上。
 *  为什么要有这一条：`busy-while-transfer` 起的那一次传输（夹具是两万个文件）跑不跑得完，取决于当时机器多忙，
 *  不取决于界面。有一遍它没跑完，设置面从第一次保存起连红几步，每条写的都是
 *  「现在有事正在跑：图床批量上传（夹具）」—— 那**是**闸门按设计工作，红的是驱动没等。 */
async function waitIdle(timeoutMs = 600000) {
  const t0 = Date.now();
  await click('配好的位置', 'any'); await sleep(1200);
  for (;;) {
    const now = JSON.parse(String(await ev('(() => { const btns = [...document.querySelectorAll(".pairs-cell button, .ant-listy-item button")]'
      + '.filter((b) => b.getBoundingClientRect().width);'
      + 'const m = document.body.innerText.replace(/\\s+/g, " ").match(/正在写这棵目录树：\\S{1,30}/g);'
      + 'return JSON.stringify({ off: btns.filter((b) => b.disabled).length, total: btns.length,'
      + ' hint: m ? m.join(",") : "" }); })()')));
    if (now.off === 0 && now.hint === '') return { ms: Date.now() - t0, now };
    if (Date.now() - t0 > timeoutMs) throw new Error('等了 ' + Math.round((Date.now() - t0) / 1000)
      + ' 秒还没空：灭着的按钮=' + now.off + '/' + now.total + ' 卡上写着=' + now.hint);
    await sleep(5000);
  }
}
/** 某一栏的位置选择器：pane 0=左 1=右。等它出现，最多 6 秒（视图切换后 React 还要挂一轮）。 */
async function pickPane(pane, optionText) {
  let box = { err: '还没挂上' };
  for (let i = 0; i < 20 && box.err; i++) {
    await sleep(300);
    box = await ev(`(() => {
      // 文档顺序里**可见的**第 0 个是左栏、第 1 个是右栏。必须先按宽度筛：
      // 对子那一面现在也有一颗下拉框（列表上面那颗「只看」），而两面都一直挂着、只用 display 切换
      // —— 不筛就会取到隐藏面里那颗，宽度 0，于是一句"还没宽度"把整条浏览器通路全报成失败
      // （真窗口跑出来的）。
      // 这段是模板字符串：注释里**不许出现反引号**，出现就等于把字符串在那一行终止，
      // 后面所有内容都变成驱动自己的代码 —— 同一件事我今天连踩两次（ReferenceError 来自这里）。
      const vis = [...document.querySelectorAll('.ant-select')].filter((e) => e.getBoundingClientRect().width);
      const s = vis[${pane}];
      if (!s) return { err: '可见的位置选择器不足 ' + (${pane} + 1) + ' 个（总共 ' + vis.length + ' 个）' };
      const b = s.getBoundingClientRect();
      if (!b.width) return { err: '那一栏的选择器还没宽度' };
      return { x: b.x + b.width / 2, y: b.y + b.height / 2 };
    })()`);
  }
  if (box.err) throw new Error(box.err);
  await clickPoint(box);
  await sleep(600);
  const pt = await ev(`(() => {
    const want = ${JSON.stringify(optionText)}.replace(/\\s+/g, '');
    const norm = (e) => (e.textContent || '').replace(/\\s+/g, '');
    const o = [...document.querySelectorAll('.ant-select-item-option-content, .ant-select-item, [role=option]')].filter((e) => norm(e) === want);
    if (!o.length) return { err: '下拉里没有这个位置：' + want + ' 全部=' + [...document.querySelectorAll('.ant-select-item, [role=option]')].map((e) => e.textContent.trim()).join(',') };
    const b = o[0].getBoundingClientRect();
    return { x: b.x + b.width / 2, y: b.y + b.height / 2 };
  })()`);
  if (pt.err) throw new Error(pt.err);
  await clickPoint(pt);
  await sleep(1400);
}
/** 浏览器那一面「两栏并排」这一**形状**的入口（四步共用）。
 *  2026-10-11 起这一面默认是**单边满宽**：打开它是为了翻本地那堆图，所以屏上只有
 *  一枚位置框、一句「个文件 ·」。按下「和云端比一比」才并排出第二栏。
 *  于是凡是要 `pickPane(1, ...)` 或 `waitListed()` 的步骤都必须先走这里 ——
 *  不走进来就红成「可见的位置选择器不足 2 个（总共 1 个）」，读着像界面坏了，
 *  其实那是尺子还按"这一面永远两栏"的老形状走路（整段 20 步一起红就是这么级联的）。
 *  幂等：已经在比对就什么都不做（顶栏那一句换成「左右是同一个位置的两面」，认它）。
 *  进比对之后两栏都不是 `compact`，那一族"动文件"的动作与「上一级」都在明面上，
 *  所以这里只负责把形状摆对，各步自己走自己的下一步。 */
async function twoPanes() {
  await click('浏览文件', 'any'); await sleep(900);
  const comparing = Number(await ev('document.body.innerText.includes("左右是同一个位置的两面") ? 1 : 0'));
  if (comparing === 0) {
    await click('和云端比一比', 'any'); await sleep(1200);
  }
  await pickPane(0, 'local'); await pickPane(1, PROC.remote); await waitListed();
}
/** 设置面「任务」那一块：按**分组标题（那一对位置）**认出某一行，再点它那颗按钮。
 *  2026-10-11 起行上第一句是「要做什么」那句（与卡上同一份 `intentOptions`），
 *  任务名与那两条完整路径收进了「为什么？」—— 而 `NoteLine` 收起时那一段**根本没挂进 DOM**
 *  （`{open && ...}`，不是 display:none），所以连 `textContent` 里都没有。
 *  于是按名字定位（`inRow` 交「封面对齐」）整族失效：报出来是「候选=0」，
 *  而界面上那颗「改这条」活得好好的 —— 认的是一个已经不在这屏上的字符串。
 *  分组标题就是卡上位置下拉里那一句（`settings-jobs-grouped-by-pair` 钉的正是同一份说法），
 *  拿它当身份 = 拿这一面自己声明的身份当身份。
 *  对账那几条不在配对分组里（`intentPhraseFor` 交回原名），所以那几条仍可按名字认。 */
async function clickJobRow(pairLabel, buttonLabel) {
  // `async` 是必须的：下面要 `await` 一帧等滚动停下来再读坐标（少了 async，
  // 页面里报的是 `SyntaxError: await is only valid in async functions`，
  // 而驱动那边只回一句「表达式出错」—— 报的是尺子，不是界面）。
  const pt = await ev(`(async () => {
    const norm = (s) => String(s || '').replace(/\\s+/g, '');
    const card = [...document.querySelectorAll('.ant-card')].filter((c) => c.getBoundingClientRect().width)
      .find((c) => norm((c.querySelector('.ant-card-head-title') || {}).textContent) === '任务');
    if (!card) return { err: '看不见「任务」那块卡片' };
    // 分组标题里不许混进行自己那两颗「为什么？」：它们是 .fs-caption，但住在行里面，
    // 混进来的话"下一组从哪儿开始"就会算错一行。
    const heads = [...card.querySelectorAll('.fs-caption')]
      .filter((e) => e.getBoundingClientRect().width && !e.closest('.ant-listy-item'));
    const want = norm(${JSON.stringify(pairLabel)});
    const h = heads.find((e) => norm(e.textContent).includes(want));
    if (!h) return { err: '「任务」那一块里没有那个分组标题：' + want
      + ' 有的=' + heads.map((x) => norm(x.textContent)).join(',') };
    const after = (a, b) => !!(a.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING);
    // 「这一行属于 h」= h 在它之前，而**没有任何别的标题夹在中间**。
    // 夹在中间那半句必须写成 "after(h, x) 且 after(x, r)"：只写 after(x, r) 的话，
    // 凡是排在前一个分组标题之后的行都会被算成"不属于 h"，于是第二个及以后的分组永远读到零行
    // （第一版就是这么红的：报「那个分组标题下面一条行都没有：fxb:b」，而那一行明明挂在屏上）。
    // 这一整段是发到页面里跑的模板字面量，**注释里也不许出现反引号**（文件头那条规矩，今天又踩一次）。
    const rows = [...card.querySelectorAll('.ant-listy-item')].filter((r) => r.getBoundingClientRect().width
      && after(h, r) && !heads.some((x) => x !== h && after(h, x) && after(x, r)));
    if (!rows.length) return { err: '那个分组标题下面一条行都没有：' + want };
    const btnWant = norm(${JSON.stringify(buttonLabel)});
    const btn = rows.flatMap((r) => [...r.querySelectorAll('button')]).find((b) => norm(b.textContent).includes(btnWant));
    if (!btn) return { err: '那一组里没有那颗按钮：' + btnWant + ' 行数=' + rows.length };
    // 先把它滚到眼前再读坐标：那一块住在设置面的下半部，整页滚着，
    // 直接读矩形会拿到视口外的 y，点下去落在窗口外面 —— 按钮"按了"而表单没开，
    // 下一步就报「没有那个字段」（真窗口跑出来的就是这个顺序）。
    btn.scrollIntoView({ block: "center" });
    await new Promise((r) => setTimeout(r, 200));
    const b = btn.getBoundingClientRect();
    const x = b.x + b.width / 2, y = b.y + b.height / 2;
    const at = document.elementFromPoint(x, y);
    if (!b.width || !at || !(btn.contains(at) || at.contains(btn))) {
      return { err: '那颗按钮滚到眼前之后坐标上不是它（落在「'
        + (at ? String(at.className).split(" ")[0] || at.tagName.toLowerCase() : "空") + '」）：' + btnWant };
    }
    return { x, y, tag: btn.tagName,
      disabled: !!btn.disabled, rows: rows.length, text: btn.textContent.trim() };
  })()`);
  if (pt.err) throw new Error(String(pt.err));
  await clickPoint(pt);
  return pt;
}
/** 卡上那一格选择框。`which` 0=模式 1=方向；`want` 给了就在打开之后按文字点那一格。
 *  为什么不复用 `click()`：`finder` 的行内那一支只在行里找 `button`，而选择框不是按钮。
 *  返回**列出来的选项文字**（用 | 拼）：判"框里只列真有的组合"要看的正是这一串。
 *  点之前走的是 `finder` 同一套命中测试等待（第一遍就是这么红的：读到矩形就点，
 *  落点被上一格留下的那一层盖住，报出来是"框里一个选项都没有"，看着像界面坏了，其实是尺子少等了）。
 *  等不到、或点开却数不到选项，都会把**挡着的那一层**说进报错里，不留"空字符串"那种读数。 */
/** 选择条上**写着这个 `aria-label` 的那一枚**（'位置' / '要做什么'），等到它站定、点得着。
 *  筛"看得见的"是因为三面一直挂着（display 切的），隐藏面里的框量得到却点不着。
 *
 *  2026-10-09：它原来收的是**下标**（0=位置、1=模式、2=方向）。下标这个参数本身就是
 *  「模式+方向合并」带坏的那一处 —— 合并之后只有两枚，而调用方里还留着 `topSelectBox(2)`，
 *  它报的是「看得见的一共 2 枚」：读着像尺子过期，其实只是**下标已经数不到那一枚**。
 *  改成按 `aria-label` 认之后，"那一枚不见了"与"那一枚改名了"是两种不同的红，
 *  前者报"找不到位置"，后者报"没有叫要做什么的那一枚" —— 都能指到地方。
 *
 *  `aria-label` 要**两处都认**：antd 把这个 prop 转发给内部那枚 `<input>`，挂在
 *  `.ant-select` 根节点上读出来是空的。只认根节点的那一版报的是「看得见的一共 2 枚：?/?」
 *  —— 两枚都在，就是都读不到名字，而红的那一句在说"名字读不到"，不在"框不在"。
 *  同一个坑本文件里犯过两回（`restart-keeps-face-and-tab` 那一段原来就写了这条提醒），
 *  所以把它提成一个 `ariaOf`，认框的那几处一律引这一个，不再各抄一份。 */
const ariaOf = `(e) => e.getAttribute('aria-label')
  || (e.querySelector('input') || {}).getAttribute?.('aria-label')
  || (e.querySelector('.ant-select-selector') || {}).getAttribute?.('aria-label')
  || ''`;
async function topSelectBox(label) {
  const box = await ev(`(async () => {
    const all = [...document.querySelectorAll('${SEL_PAIR_BAR}')]
      .filter((e) => e.getBoundingClientRect().width);
    const aria = ${ariaOf};
    const s = all.find((e) => aria(e) === ${JSON.stringify(label)});
    if (!s) return { err: '选择条上没有「' + ${JSON.stringify(label)} + '」那一枚（看得见的一共 '
      + all.length + ' 枚：' + all.map((e) => aria(e) || '?').join('/') + '）' };
    let blocker = '';
    for (let i = 0; i < 110; i++) {
      s.scrollIntoView({ block: 'center' });
      const b = s.getBoundingClientRect();
      const x = b.x + b.width / 2, y = b.y + b.height / 2;
      const at = document.elementFromPoint(x, y);
      if (b.width && at && (s.contains(at) || at.contains(s))) return { x, y, waited: i };
      blocker = at ? (at.className ? String(at.className).split(' ')[0] : at.tagName.toLowerCase()) : '空';
      await new Promise((r) => setTimeout(r, 100));
    }
    return { err: '「' + ${JSON.stringify(label)} + '」那一枚等 11 秒也没站定（坐标上总是「' + blocker + '」）' };
  })()`);
  if (box.err) throw new Error(String(box.err));
  return box;
}

/** 把某一**对位置**站到「位置」那枚下拉上。那一面只剩当前那一格，
 *  "挑哪一家"从点一行卡变成了选一项下拉 —— 读任何一格之前都要先走这一步。
 *  项里的名字是**短名**（`local2 ↔ fxa:c`），所以按 needle 子串匹配就够。 */
async function showPair(needle) {
  return openSelectAt(() => topSelectBox('位置'), needle, '位置', false, false);
}

/** 某一格下拉里**有几项**（数"有几对位置"用的就是它：列表撤了，卡数变成了选项数）。 */
/** 有几对位置 = 位置那枚下拉里有几项（列表撤了之后，"卡数"没有别的读者）。
 *  `openSelectAt` 交回的是 `'|'` 拼起来的一串，所以数的是拆开之后的项。 */
/** 选择条上写着这个 `aria-label` 的那一枚下拉**显示着的那一句**（'位置' / '要做什么'）。
 *  方向那句话以前写在卡上那一格里；模式与方向并进「要做什么」之后，
 *  "这一格要做的是什么"整句只活在这一枚框里，量"换过去了没有"就得读这里 ——
 *  而它读到的正是一句**自己就完整**的话（`只补，不删 —— 从本地补到云端`），
 *  不再是两枚框各说一半、要人拼起来才读得懂的东西。 */
async function selectValue(label) {
  // 这一段是**要在页面里跑的表达式**，所以模板里的 `+` 全是页面侧的拼接。
  // （早先写成 Node 侧的单引号串，于是 `all.map(...)` 落在这一层求值 —— `all` 在这里不存在，
  // 模块加载就炸。而那炸法读起来像"这段读数偶尔抽风"，不是像拼错了。）
  // v6 里显示值住在 `.ant-select-content`（`.ant-select-selection-item` 是 v5 的类名）。
  const expr = `(() => {
    const aria = ${ariaOf};
    const all = [...document.querySelectorAll(${JSON.stringify(SEL_PAIR_BAR)})]
      .filter((e) => e.getBoundingClientRect().width);
    const s = all.find((e) => aria(e) === ${JSON.stringify(label)});
    if (!s) return 'ERR 选择条上没有「${label}」那一枚，看得见的 ' + all.length + ' 枚：'
      + all.map((e) => (aria(e) || '?').slice(0, 10)).join('/');
    const t = s.querySelector('.ant-select-content') || s.querySelector('.ant-select-selection-item');
    return t ? t.textContent.trim() : 'ERR 那一枚没有显示值（框里是空的）';
  })()`;
  const t = String(await ev(expr));
  if (t.startsWith('ERR ')) throw new Error(t);
  return t;
}
async function pairCount() {
  return (await showPair(null)).split('|').filter((o) => o.trim() !== '').length;
}

/** 点开某一对的那一枚「要做什么」框，读出那几项；给了 `want` 就点中那一项。
 *
 *  2026-10-09 模式与方向并成一枚之后，**`which` 这个参数整个没了**：原来 `0=模式 / 1=方向`
 *  两枚框，现在选择条上只有「位置」与「要做什么」两枚（`SEL_PAIR_BAR` 量到 2 枚）。
 *  留着一个恒为 0 的形参会让调用方继续写着 `openCardSelect(x, 1, …)`，而那一发会去点
 *  `topSelectBox(2)` —— 选择条上根本没有第三枚，报「看得见的一共 2 枚」。
 *  与其留一位永远传 0 的空位，不如让签名直接说实话：调用方少写一个数，忘了就编译不过。
 *  `want` 传的是**档位的那一半**（`只补，不删` / `让两边对齐`），匹配走 `includes`，
 *  所以不必把 `模式 —— 方向` 那一整句抄下来 —— 那一半会随方向变，抄死就钉在某一对上。 */
async function openCardSelect(rowText, want) {
  await showPair(rowText);
  return openSelectAt(() => topSelectBox('要做什么'), want, rowText + ' 要做什么', false, false);
}

/** 点右列那一段（`.ant-segmented-item`）。**不要用 `click(段名)`**。
 *
 *  2026-10-09 词表迁移撞出来的：那一段原名「检查」是个几乎不撞名的词，改成「对账」之后
 *  左列闸门卡上那颗「开始对账」按钮**也**含这两个字，而 `finder` 取的是文档顺序第一个命中
 *  （左列在前）—— 于是「点右列那一段」会点成「起一次对账」，而那一格照样绿着：
 *  它量的翻页器数与对账报告没关系，读数看着正常，量的却是另一件事。
 *  这里按**组件**认（`.ant-segmented-item`，只有顶栏三面与右列四段是它）再按**全等**的文字挑，
 *  撞名就报出来而不是赌一个：右列那几段与顶栏那三面互不撞名，全等匹配就是精确的。 */
async function railTabPoint(name) {
  // 模板字面量：这段是**在页面里跑**的，里面的 `+` 是页面侧的拼接。写成 Node 侧的单引号串时，
  // 下一行那句 `all.map(...)` 会在这一层求值（`all` 在这里不存在），炸法是运行时的
  // ReferenceError —— 语法全对，所以只有真跑起来才看得见。
  const p = await ev(`(() => {
    const want = ${JSON.stringify(name)}.replace(/\\s+/g, '');
    const all = [...document.querySelectorAll('.ant-segmented-item')]
      .filter((e) => e.getBoundingClientRect().width);
    const hit = all.filter((e) => (e.textContent || '').replace(/\\s+/g, '') === want);
    if (!hit.length) return { err: '那一排分段控件里没有「' + want + '」这一项，列出的='
      + all.map((e) => e.textContent.trim()).join(' / ') };
    if (hit.length > 1) return { err: '那一排分段控件里有 ' + hit.length + ' 项都叫「' + want
      + '」：全等匹配本该是精确的，出现两个说明还有第三处同名，分不清点的是哪一处' };
    const b = hit[0].getBoundingClientRect();
    return { x: b.x + b.width / 2, y: b.y + b.height / 2 };
  })()`);
  if (p && p.err) throw new Error(String(p.err));
  return p;
}
async function clickRailTab(name) {
  await sleep(400);
  const pt = await railTabPoint(name);
  await clickPoint(pt);
  return pt;
}

/** 当前屏上真正占着宽度（不是被隐藏的）的翻页器数。 */
function pagerCount() {
  return ev('(() => [...document.querySelectorAll(".ant-pagination")]'
    + '.filter((e) => e.getBoundingClientRect().width).length)()');
}

/** 点开着一格选择框之后共用的那三步：数选项、（给了 want 就）点那一格、按 Esc 收掉。
 *  抽出来不是为了少写几行 —— 是为了**只有一份**那条 Esc 的教训（见 `closeDropdown`）。
 *  「层里那两行的选择框」与「卡上那两格」走的是同一条通路，抄第二份就会漏改第二份。 */
async function openSelectAt(box, want, tag, inModal, exact = true) {
  let listed = '';
  for (let attempt = 0; attempt < 3; attempt++) {
    // 每一发都重新定位：上一发点开过别的框之后 React 会重渲染那一行，旧坐标指的是别人。
    const point = typeof box === 'function' ? await box() : box;
    if (point.err) throw new Error(String(point.err));
    await clickPoint(point);
    await sleep(600);
    listed = String(await ev('(() => { const dd=[...document.querySelectorAll(".ant-select-dropdown")]'
      + '.filter((e) => !String(e.className).includes("hidden"));'
      + 'return dd.map((d) => [...d.querySelectorAll(".ant-select-item-option-content")]'
      + '.map((e) => e.textContent.trim()).join("|")).join("##"); })()'));
    if (listed !== '') break;
    // 空列表 + 一堆 hidden 的层 = 上一次点开的还没收干净，这一次点击被当成"关掉它"。
    await closeDropdown(tag, inModal);
    await sleep(400);
  }
  if (listed === '') {
    const why = String(await ev('(() => { const dd=[...document.querySelectorAll(".ant-select-dropdown")];'
      + 'const sels=[...document.querySelectorAll(' + JSON.stringify(SEL_PAIR_BAR) + ')].filter(e=>e.getBoundingClientRect().width);'
      + 'const box=sels.map((s)=>{const b=s.getBoundingClientRect();const at=document.elementFromPoint(b.x+b.width/2,b.y+b.height/2);'
      + 'return "灰=" + (String(s.className).includes("disabled")?"Y":"N") + " 点上=" + (at?String(at.className).split(" ")[0]:"空");});'
      + 'return "活动元素=" + (document.activeElement?String(document.activeElement.className).split(" ")[0]:"?")'
      + ' + " 开着的框=" + document.querySelectorAll(".ant-select-open").length'
      + ' + " 选择条上的框=[" + box.join(" | ") + "]"'
      + ' + " 层数=" + dd.length + " 类名=" + dd.map((d)=>String(d.className)).join(" ~ ").slice(0,120); })()'));
    throw new Error('那一格点开之后一个选项都没有（' + why + '）：' + tag);
  }
  if (want) {
    const pt = await ev('(() => { const norm = (s) => String(s).replace(/\\s+/g, "");'
      + 'const needle = norm(' + JSON.stringify(want) + '); const exact = ' + (exact ? 'true' : 'false') + ';'
      + 'const drop = ".ant-select-dropdown:not(.ant-select-dropdown-hidden)";'
      + 'const o = [...document.querySelectorAll(drop + " .ant-select-item")]'
      + '.filter((e) => exact ? norm(e.textContent) === needle : norm(e.textContent).includes(needle));'
      + 'if (!o.length) return { err: "框里没有这一格选项：" + ' + JSON.stringify(want)
      + ' + " 列出的=" + ' + JSON.stringify(listed) + ' };'
      + 'const b = o[0].getBoundingClientRect();'
      + 'if (!b.width) return { err: "那一格选项看不见（被上一层挡住了？）" };'
      + 'return { x: b.x + b.width / 2, y: b.y + b.height / 2 }; })()');
    if (pt.err) throw new Error(String(pt.err));
    await clickPoint(pt);
    await sleep(900);
  }
  await closeDropdown(tag, inModal);
  return listed;
}

/** 收掉那一格：**按 Escape**，不是把鼠标挪开。antd 的下拉关掉的触发是 outside mousedown 或 Esc，
 *  而只发 mouseMoved 的"挪开"两样都不是 —— 于是下一格选择框正压在这一格剩下的选项底下，
 *  命中测试永远读到 `ant-select-item-option-content`（第一遍的两格就是这么红的）。 */
/** 收掉那一格的下拉。
 *  平时走 **Escape**：antd 关下拉的触发是 outside mousedown 或 Esc，而只发 mouseMoved 的
 *  "挪开"两样都不是 —— 下一格选择框正压在这一格剩下的选项底下，命中测试永远读到
 *  `ant-select-item-option-content`（第一遍的两格就是这么红的）。
 *  但**层里那一格不许按第二下**：第一下关掉下拉，第二下就落到那个模态框身上把整层关了
 *  （`draft-card-created` 那一格量到的就是这个：下拉确实两次才收掉，而第二次把层带走了，
 *  后面往框里写字自然什么都留不下）。所以在模态框里改成点一下那一层的标题 ——
 *  那是一次真的 outside mousedown，而点标题不会动层里任何一格。 */
async function closeDropdown(tag, inModal) {
  const left = async () => Number(await ev('(() => [...document.querySelectorAll(".ant-select-dropdown")]'
    + '.filter((d) => !String(d.className).includes("hidden") && d.getBoundingClientRect().width).length)()'));
  if (inModal) {
    const hit = await ev('(() => { const vis=[...document.querySelectorAll(".ant-modal")]'
      + '.filter((m)=>m.getBoundingClientRect().width); const m=vis[vis.length-1];'
      + 'if (!m) return { err: "那一层已经不在屏上" };'
      + 'const h=m.querySelector(".ant-modal-header") || m;'
      + 'const b=h.getBoundingClientRect(); return { x: b.x + b.width/2, y: b.y + 12 }; })()');
    if (hit.err) throw new Error(tag + '：' + String(hit.err));
    await clickPoint(hit);
    await sleep(350);
    const n = await left();
    if (n === 0) return;
    // 点标题没关掉它才按**一次** Esc；再不行就把"还开着几层"报出来，不许悄悄按第二下。
    await pressEscape();
    await sleep(300);
    if (await left() === 0) { log('SELECT-CLOSE 层里那一格是靠 Esc 收的：' + tag); return; }
    throw new Error('层里那一格的下拉关不掉（还开着 ' + (await left()) + ' 层）：' + tag);
  }
  for (let i = 0; i < 20; i++) {
    await pressEscape();
    await sleep(150);
    const n = await left();
    if (n === 0) { if (i) log('SELECT-CLOSE 第 ' + (i + 1) + ' 次 Esc 才收掉那一层'); return; }
    if (i === 19) throw new Error('那一格的下拉关不掉（还开着 ' + n + ' 层）：' + tag);
  }
}

/** 「换一对位置」那一层里的第 `nth` 个位置选择框（层里两行各一个，文档顺序：这一侧、那一侧）。
 *  读的是**看得见的那一层模态框**里的框（antd 关掉的模态框 DOM 还挂着，宽度 0 ——
 *  与 `setInput` 同一条口径，撞上同一族事故两次了）。 */
async function openLayerSelect(nth, want) {
  const box = await ev('(() => { const vis=[...document.querySelectorAll(".ant-modal")]'
    + '.filter((m)=>m.getBoundingClientRect().width); const m=vis[vis.length-1];'
    + 'if (!m) return { err: "那一层没开" };'
    + 'const s=[...m.querySelectorAll(".ant-select")][' + nth + '];'
    + 'if (!s) return { err: "层里没有第 ' + (nth + 1) + ' 个位置选择框（有的数="'
    + ' + m.querySelectorAll(".ant-select").length + "）" };'
    + 'const b=s.getBoundingClientRect(); return { x: b.x + b.width/2, y: b.y + b.height/2 }; })()');
  if (box.err) throw new Error(String(box.err));
  return openSelectAt(box, want, '那一层第 ' + (nth + 1) + ' 行的位置框', true);
}

/** 层里那一行的路径框：按 `aria-label`（这一侧 / 那一侧）认，不走"最后一个框"那种猜。
 *  写值仍然走原生 setter + input 事件 —— 受控框的值归 React 拿着，直接改 `value`
 *  会让 DOM 与 state 分家（与 `setFieldInput` 同一条理由，那条注释里记着这个坑）。 */
async function setDraftField(label, value) {
  const r = await ev('(() => { const vis=[...document.querySelectorAll(".ant-modal")]'
    + '.filter((m)=>m.getBoundingClientRect().width); const m=vis[vis.length-1];'
    + 'if (!m) return { err: "那一层没开" };'
    + 'const i=[...m.querySelectorAll("input.ant-input")]'
    + '.find((x)=>(x.getAttribute("aria-label")||"") === ' + JSON.stringify(label) + ');'
    + 'if (!i) return { err: "层里没有那一行的框：" + ' + JSON.stringify(label) + ' };'
    + 'const set = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, "value").set;'
    + 'set.call(i, ' + JSON.stringify(value) + ');'
    + 'i.dispatchEvent(new Event("input", { bubbles: true }));'
    + 'return { read: i.value }; })()');
  if (r.err) throw new Error(String(r.err));
  if (r.read !== value) {
    // 现场一起报：只说"值没留下"的话，下一遍还是猜（层没了、框是哪一个、还有几层看得见，是三件不同的事）。
    const scene = String(await ev('(() => { const vis=[...document.querySelectorAll(".ant-modal")]'
      + '.filter((m)=>m.getBoundingClientRect().width);'
      + 'return "看得见的层=" + vis.length + " 标题=" + vis.map((m)=>((m.querySelector(".ant-modal-title")||{}).textContent||"?")).join("/")'
      + ' + " 框=" + vis.map((m)=>[...m.querySelectorAll("input.ant-input")].map((i)=>(i.getAttribute("aria-label")||"?")+"="+i.value).join("|"))'
      + '.join(" // "); })()'));
    throw new Error('值没留在框里（读回 ' + JSON.stringify(r.read) + '）：' + label + ' —— ' + scene);
  }
  await sleep(250);
}

/** 当前那一格的三样读数：按钮那几颗的字、那句"试跑不动数据"在不在、以及「模式」那两字的实际渲染色。
 *  换模式要量的就是这几样**一起**换：少一颗、注跟着换过去、颜色不动，都是界面在说两件不一样的事。
 *  颜色取的是 `getComputedStyle` 那一份（不是 CSS 变量名）：变量名对了而组件把它吃掉，也是这一格要抓的。
 *  那一面不再是一列卡：先站到那一对位置上，再读**当前那一格**（`.pairs-cell`）；
 *  「模式」那两字现在住在顶部那一行，所以它从整个屏上找（那一面只剩这一枚，找不错）。 */
async function cardShape(rowKey) {
  try { await showPair(rowKey); } catch (e) { return { err: String(e) }; }
  return JSON.parse(String(await ev('(() => { const r=document.querySelector(".pairs-cell");'
    + 'if (!r) return JSON.stringify({ err: "当前那一格不在" });'
    + 'const txt=r.innerText.replace(/\\s+/g," ");'
    // 「模式」那两个字现在住在**选择条**上（横跨整窗）。它与那枚框同在 `.pair-field` 里，
    // 而那一格里除了它还有框的显示值 —— 所以不能按 `textContent === "模式"` 找整格，
    // 要找**那个 class 自己的标签**（量过：`.pair-field` 的第一个 `.fs-caption`）。
    // 判据锚在选择条上而不是"整屏第一个 .ant-typography"：旧写法量到的是别处的字。
    + 'const bar=document.querySelector(' + JSON.stringify(SEL_PAIR_BAR.replace(' .ant-select','')) + ');'
    + 'const sels=bar?[...bar.querySelectorAll(".ant-select")]:[];'
    + 'const f=sels.length>1?sels[1].parentElement:null;'
    + 'const t=f?f.querySelector(".fs-caption"):null;'
    + 'return JSON.stringify({ btns:[...r.querySelectorAll("button")].map((b)=>b.textContent.trim()).join("|"),'
    // 2026-10-08：那句注不再**常驻**在卡上，挂在试跑那颗的悬停上（改的是"什么时候说"，
    // 没改"说什么"）。那颗 2026-10-09 改名叫「看看会动什么」，悬停的位置没动。
    // 所以 `note` 这一档量的是"屏上没有它"= 0；"悬停时它出现"由
    // `pair-card-one-per-position` 用真悬停量（量得到 Tooltip 弹层里那一句）。
    + ' note: txt.includes("只告诉你会动哪些") ? 1 : 0,'
    // 这一格**不再有名字那一行**（撤的：它在两行之内复述「模式」与「方向」两枚框）。
    // 它交出来的是那两条完整路径，读这一枚 `.fs-code` 而不是整格文字：整格里还有那颗
    // 「正在写这棵目录树：…」的红标，它带着别的任务的名字（同一族教训见这里的历史版本）。
    + ' paths: (() => { const p=r.querySelector(".fs-code"); return p ? p.textContent.trim() : ""; })(),'
    + ' warm: t ? getComputedStyle(t).color : "" }); })()')));
}
/** 当前那一格（临时卡那一头）的读数：整格文字、按钮那几颗、那句"试跑不动数据"在不在、
 *  **有几对位置**，以及位置那一枚框里显示的那一句（临时那一半的标志住在那里）。
 *  比 `cardShape` 多读文字本身：那一格上没有任何配置痕迹，那两处标志（那句重开就没了、
 *  那颗存成常用）都在文字里，判定吃的就是这些原话。
 *  以前还有第三处：名字那一行写的是后台给的那句「临时（没存进配置）」—— 那一行撤了
 *  （配置格的名字在两行之内复述模式与方向，撤的是那一行），所以临时的判据改读**位置框里那半句**，
 *  它与「存成常用」那颗一起才是这一格"没写进配置"的两份证据。
 * 「要做什么」那一枚列了几格不在这里读 —— 那要点开那一枚，走的是 `openCardSelect`。（原来这一句说的是"模式框与方向框"，那是 2026-10-09 合并之前的形状。） */
async function draftCardShape(rowKey) {
  let cards = -1;
  try { cards = await pairCount(); } catch (_) { /* 数不到不判失败，读格那一半自会报为什么 */ }
  const s = await cardShape(rowKey);
  if (s.err) return { ...s, cards };
  const txt = String(await ev('(() => { const r=document.querySelector(".pairs-cell");'
    + 'return r ? r.innerText.replace(/\\s+/g," ") : ""; })()'));
  return { ...s, txt, cards, pos: await selectValue('位置') };
}
async function setInput(value) {
  const r = await ev(`(() => {
    // 只认**看得见**的那些模态框：antd 关掉的模态框 DOM 还留着（宽度 0），
    // 而拿 querySelector 直接找 .ant-modal input 会先撞上那份陈的——写进去、
    // 焦点也给进去，界面上却什么都没有（那一步就是这么红的）。
    // 这段注释里不许出现反引号：整段在模板字符串里，一个反引号就把那一行终止了。
    const vis = [...document.querySelectorAll('.ant-modal')].filter((m) => m.getBoundingClientRect().width);
    const i = vis.length ? vis[vis.length - 1].querySelector('input.ant-input, input[type=text]') : null;
    if (!i) return { err: '没有那个输入框（看得见的模态框 ' + vis.length + ' 个）' };
    const set = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set;
    set.call(i, ${JSON.stringify(value)});
    i.dispatchEvent(new Event('input', { bubbles: true }));
    return { ok: true, now: i.value };
  })()`);
  if (r.err) throw new Error(r.err);
  return r;
}
/** 在焦点所在的那只框里按一次回车。走这条路是为了量"框里空着按回车"那一档：
 *  那颗「就这么定」在空值时是禁的、点不到，而回车点得动——以前那一支是静默返回，
 *  界面上一个字都不说。React 的 onPressEnter 听的是带 text 的 keyDown。 */
async function focusModalInput() {
  // 回车要落在那只框上才叫 `onPressEnter`：合成键事件发给的是**当前焦点**，
  // 而 `setInput` 只写值不给焦点。上一版这一步红在"一个字都不说"，红的其实是这里——
  // 键发到了没人聚焦的地方，不是界面没做那句。
  // 与 `setInput` 同一条口径：只取**看得见**的那只模态框里的框（陈的那份 DOM 还挂着）。
  const r = await ev('(() => { const vis = [...document.querySelectorAll(".ant-modal")].filter((m) => m.getBoundingClientRect().width); const i = vis.length ? vis[vis.length - 1].querySelector("input.ant-input, input[type=text]") : null; if (!i) return { err: "没有那个输入框（看得见的模态框 " + vis.length + " 个）" }; i.focus(); return { ok: true, now: document.activeElement === i }; })()');
  if (r.err) throw new Error(r.err);
  if (!r.now) throw new Error('那只框没拿到焦点，回车不会走到它身上');
  await sleep(200);
}
async function pressEnter() {
  await send('Input.dispatchKeyEvent', { type: 'keyDown', key: 'Enter', code: 'Enter',
    windowsVirtualKeyCode: 13, nativeVirtualKeyCode: 13, text: '\r', unmodifiedText: '\r' });
  await send('Input.dispatchKeyEvent', { type: 'keyUp', key: 'Enter', code: 'Enter',
    windowsVirtualKeyCode: 13, nativeVirtualKeyCode: 13 });
  await sleep(700);
}
async function pressEscape() {
  await send('Input.dispatchKeyEvent', { type: 'keyDown', key: 'Escape', code: 'Escape',
    windowsVirtualKeyCode: 27, nativeVirtualKeyCode: 27 });
  await send('Input.dispatchKeyEvent', { type: 'keyUp', key: 'Escape', code: 'Escape',
    windowsVirtualKeyCode: 27, nativeVirtualKeyCode: 27 });
}
async function mouseAway() {
  // 合成的悬停不会自己离开：Tooltip 的 container 在 v6 是 pointer-events:auto，
  // 停在灰按钮上时会盖住旁边那颗。每一段之前先把鼠标挪开。
  await send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: 8, y: 150 });
  await sleep(250);
}
/** 把还开着的确认层合上（按「取消」，没有那颗就按右上角那个叉）。
 *  为什么要有这一条：整遍 0..46 一次跑完时，浏览器那一段故意留着的层（「改名撞名：确认层不关」
 *  那一步就是这么设计的）会盖住后面设置面那一段的每一格读数 —— 量出来是五格红，
 *  而界面其实没坏（同一批步骤在干净窗口里单跑 10/10 全过）。留着的层必须显式合上并记下来，
 *  不能让"整遍跑"变成一份看起来像缺陷的读数。 */
async function closeStrayLayers(tag) {
  for (let i = 0; i < 4; i++) {
    const got = await ev('(() => {'
      + 'const w = [...document.querySelectorAll(".ant-modal-wrap")]'
      + '.find((x) => x.getClientRects().length && getComputedStyle(x).visibility !== "hidden");'
      + 'if (!w) return "";'
      + 'const b = [...w.querySelectorAll(".ant-modal-footer .ant-btn, .ant-modal-close")]'
      + '.map((e) => (e.textContent || "").replace(/\\s+/g, "") || "×").join(",");'
      + 'const c = [...w.querySelectorAll(".ant-modal-footer .ant-btn")]'
      + '.find((e) => (e.textContent || "").replace(/\\s+/g, "").includes("取消"))'
      + '|| w.querySelector(".ant-modal-close");'
      + 'if (!c) return { err: "层开着但找不到合上的入口：" + b };'
      + 'c.scrollIntoView({ block: "center" });'
      + 'const q = c.getBoundingClientRect();'
      + 'return { x: q.x + q.width / 2, y: q.y + q.height / 2, buttons: b };'
      + '})()');
    if (!got) return;
    if (got.err) throw new Error(`${tag} :: ${got.err}`);
    log(`STRAY-LAYER ${tag} :: 上一层留下的层还开着（按钮=${got.buttons}），先合上再跑这一格`);
    await clickPoint(got);
    await sleep(700);
  }
  throw new Error(tag + ' :: 连按四次还有层开着，这一遍的状态不可信');
}
/** 层要按**看得见的那一份**取。三面面板一直挂着，别的段留下的层只是被 display 掉了，
 *  它的行数与正文照样能读出来 —— 整遍 0..46 一次跑完时就这样错出四格红（同一批步骤在干净窗口里
 *  单跑 10/10 全绿，因为那时前面那一段的陈层根本还没被创建过）。
 *  这个筛子本仓早有先例（`DELETE 屏上任务行数` 那格就按宽度筛过 `.ant-listy-item`），
 *  漏的就是直接 `querySelector(".ant-modal")` 的那四格读数。 */
async function visibleModalText(limit = 600) {
  return String(await ev('(() => { const m=[...document.querySelectorAll(".ant-modal")]'
    + '.filter((x)=>x.getBoundingClientRect().width)[0];'
    + 'return m ? m.innerText.replace(/\\s+/g," ").slice(0,' + limit + ') : ""; })()'));
}
/** -1 = 这一屏没有开着的层（"0 行"和"没层"是两件事，别混）。 */
async function visibleModalRows() {
  return Number(await ev('(() => { const m=[...document.querySelectorAll(".ant-modal")]'
    + '.filter((x)=>x.getBoundingClientRect().width)[0];'
    + 'return m ? m.querySelectorAll(".ant-table-row").length : -1; })()'));
}
/** 等两栏的位置回显都出现（递归列举在 3000 个对象那一栏要跑一会儿）。 */
async function waitListed(timeoutMs = 20000) {
  const t0 = Date.now();
  for (;;) {
    const n = await ev('(() => { const t = document.body.innerText; return (t.match(/个文件 · /g) || []).length; })()');
    if (n >= 2) return n;
    if (Date.now() - t0 > timeoutMs) throw new Error('两栏的位置回显没都出来（等到的个数=' + n + '）');
    await sleep(500);
  }
}
/** 拍一屏。`clipY` 是**页面坐标**里那一屏的起点：`captureBeyondViewport` 下裁剪框读的是页面，
 *  不是滚动位置 —— 只 `scrollIntoView` 不给 clipY，拍回来的永远是页首（目检"任务那一块"
 *  时踩到的就是这个，看着像界面没滚，其实是尺子把滚动忽略了）。 */
async function shot(name, clipY = 0) {
  const size = await ev('({ w: window.innerWidth, h: window.innerHeight })');
  const s = await send('Page.captureScreenshot', { format: 'png', captureBeyondViewport: true,
    clip: { x: 0, y: clipY, width: size.w, height: size.h, scale: 2 } });
  fs.writeFileSync(path.join(OUT, name), Buffer.from(s.data, 'base64'));
  log(`SHOT ${name} @${size.w}x${size.h}`);
}
async function facts(tag) {
  const f = await ev(`(() => {
    const t = document.body.innerText;
    return {
      bar: (t.match(/日志目录.*/) || [''])[0],
      // 顶栏那颗主题切换口也是 .ant-segmented-item。这两处读数的意思是"视图与右列的分段"，
      // 不该被它污染 —— 全页扫的选择器一旦把 chrome 也算进来，将来的断言就会因错的理由红。
      seg: [...document.querySelectorAll('.ant-segmented-item')].filter((e) => !e.closest('.bar')).map((e) => e.textContent.trim()),
      warn: [...document.querySelectorAll('.ant-typography-warning')].map((e) => e.textContent.trim().slice(0, 60)),
      titles: [...document.querySelectorAll('.ant-alert-title, .ant-modal-title')]
        .filter((e) => e.getClientRects().length).map((e) => e.textContent.trim().slice(0, 70)),
      descs: [...document.querySelectorAll('.ant-alert-description')]
        .filter((e) => e.getClientRects().length).map((e) => e.textContent.trim().slice(0, 150)),
      // 那句拒绝走 notification（它才有"标题=人话、正文=原文"两段）。
      // 两族节点都要数：只数 message 那一族的话，一次真拒绝会被读成"界面没说话"。
      // （这一格别用反引号写选择器：这段是模板字符串，注释里的反引号会把它提前终止。）
      toast: document.querySelectorAll('.ant-message-notice').length
        + document.querySelectorAll('.ant-notification-notice').length,
      toastText: [...document.querySelectorAll('.ant-message-notice, .ant-notification-notice')]
        .map((e) => e.textContent.trim().slice(0, 220)),
      tip: [...document.querySelectorAll('.ant-tooltip, .ant-tooltip-container, [class*=tooltip]')]
        .filter((e) => e.getClientRects().length).map((e) => e.textContent.trim().slice(0, 150)).slice(-2),
      rows: [...document.querySelectorAll('.ant-table-row')].map((r) => r.textContent.trim().slice(0, 40)),
      btns: [...document.querySelectorAll('button')].filter((b) => b.getClientRects().length)
        .map((b) => (b.disabled ? '[灰]' : '') + b.textContent.trim()).filter((x) => x).slice(0, 40),
    };
  })()`);
  log('FACTS ' + tag + ' ' + JSON.stringify(f));
  return f;
}

// ---- 设置面那几步的小工具 ----------------------------------------------------
// 判定一律在 node 侧读盘后算，读数写进 readings.txt：含中文的行会被终端回显吞掉（本机实测），
// 而"文件有没有被动过"这件事只能拿字节说话，屏上那句"没改成"不算证据。
const fxFile = (which) => fs.readFileSync(
  `${PROC.fx}/${which === 'conf' ? 'conf/rclone.conf' : 'jobs.toml'}`, 'latin1');
// latin1 是唯一能一字节对一字符来回搬的编码：基线要比的是字节，不是语义。
const BASELINE = JSON.parse(fs.readFileSync(`${PROC.fx}/settings-baseline.json`, 'utf8'));
/** 只比"改了哪几行"，不比长度：行尾若被整文件换掉会一次动很多行，而那正是要拦的那一族。 */
function countDiffLines(a, b) {
  const x = a.split('\n'), y = b.split('\n');
  if (x.length !== y.length) return Math.abs(x.length - y.length) + 1000;
  return x.filter((l, i) => l !== y[i]).length;
}
/** 删行那种改动用"逐行对齐"比不出来（长度就不同），所以按**多重集**求差：
 *  拿走的是哪些行、加进来的是哪些行。判"只拿走了那一块的行"要的就是这两份名单。 */
function lineDiff(a, b) {
  const bag = new Map();
  for (const l of a.split('\n')) bag.set(l, (bag.get(l) || 0) + 1);
  const removed = [], added = [];
  for (const l of b.split('\n')) {
    const n = bag.get(l) || 0;
    if (n === 0) added.push(l); else bag.set(l, n - 1);
  }
  for (const [l, n] of bag) for (let i = 0; i < n; i++) removed.push(l);
  return { removed, added };
}
const crlf = (s) => (s.match(/\r\n/g) || []).length;
function assertBytes(which, want, msg) {
  const now = fxFile(which);
  if (now !== want) {
    throw new Error(msg + `（现在 ${now.length} 字节，基线 ${want.length} 字节，差 ${countDiffLines(want, now)} 行）`);
  }
  log('BYTES ' + which + ' 未变');
}
const readConf = () => fxFile('conf');
const readJobs = () => fxFile('jobs');
/** 同一份文件的**文本**读法：判"那句中文注释还在不在"要按 utf-8 读。
 *  拿 latin1 那份去 includes 一句中文，每一句都会报"没找到"—— 那是尺子坏了，不是界面把注释吃了；
 *  字节比对（上面那一份）与文本比对（这一份）分开，两边才都说得准。 */
const fxText = (which) => fs.readFileSync(
  `${PROC.fx}/${which === 'conf' ? 'conf/rclone.conf' : 'jobs.toml'}`, 'utf8');
/** 往"那一行的中文说法"当锚点的字段里塞值。
 *  为什么不用现成的 `locate`：那只池子扫的是 button/a/segmented/select 那一族，
 *  `input.ant-input` 与那行标签（一个 span）都不在里面 —— 拿它找「服务地址」只会报"找不到"。
 *  为什么走原生 setter + input 事件：受控 input 的值由 React 拿着，直接改 `value` 会让 DOM 与
 *  state 分家（按钮的禁用态读的是 state，屏上却写着新值）。最后那一句读回是给这一步自己作证的：
 *  值没进 DOM 就直接红，别让下一步"按钮灰着点不动"去替你解释。 */
async function setFieldInput(marker, value) {
  const r = await ev(`(() => {
    const want = ${JSON.stringify(marker)}.replace(/\\s+/g, '');
    const cands = [...document.querySelectorAll('.ant-space-vertical')]
      .filter((s) => s.getBoundingClientRect().width)
      .filter((s) => (s.textContent || '').replace(/\\s+/g, '').startsWith(want))
      .filter((s) => s.querySelector('input.ant-input'));
    if (!cands.length) return { err: '没有那个字段：' + want };
    cands.sort((a, b) => (a.textContent || '').length - (b.textContent || '').length);
    const i = cands[0].querySelector('input.ant-input');
    const set = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set;
    set.call(i, ${JSON.stringify(value)});
    i.dispatchEvent(new Event('input', { bubbles: true }));
    return { read: i.value, fields: cands.length };
  })()`);
  if (r.err) throw new Error(r.err);
  if (r.read !== value) throw new Error(`值没留在框里（读回 ${JSON.stringify(r.read)}）：${marker}`);
  await sleep(250);
}
/** 「每次都要带的参数」那一格是"能一项项加"的那种控件（tags），不是普通框：
 *  值要先写进它里面那只搜索框、拿到焦点，然后**一次真的回车**才算收下这一项。
 *  与 `setFieldInput` 同一族判据：只取**看得见**的那一格，读回来核对，别拿"打了字"当"进了 state"。 */
async function addTagToken(marker, value) {
  const r = await ev(`(() => {
    const want = ${JSON.stringify(marker)}.replace(/\\s+/g, '');
    const cands = [...document.querySelectorAll('.ant-space-vertical')]
      .filter((s) => s.getBoundingClientRect().width)
      .filter((s) => (s.textContent || '').replace(/\\s+/g, '').startsWith(want))
      .filter((s) => s.querySelector('.ant-select'));
    if (!cands.length) return { err: '没有那一格：' + want };
    cands.sort((a, b) => (a.textContent || '').length - (b.textContent || '').length);
    const i = cands[0].querySelector('.ant-select input');
    if (!i) return { err: '那一格里没有可打字的框' };
    const set = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set;
    i.focus();
    set.call(i, ${JSON.stringify(value)});
    i.dispatchEvent(new Event('input', { bubbles: true }));
    return { typed: i.value, focused: document.activeElement === i };
  })()`);
  if (r.err) throw new Error(r.err);
  if (!r.focused) throw new Error('那只框没拿到焦点，回车不会走到它身上：' + marker);
  if (r.typed !== value) throw new Error(`字没留在框里（读回 ${JSON.stringify(r.typed)}）：${marker}`);
  await pressEnter();
  const got = await ev(`(() => {
    const want = ${JSON.stringify(marker)}.replace(/\\s+/g, '');
    const s = [...document.querySelectorAll('.ant-space-vertical')]
      .filter((x) => x.getBoundingClientRect().width)
      .filter((x) => (x.textContent || '').replace(/\\s+/g, '').startsWith(want)).pop();
    if (!s) return { err: '那一格不见了' };
    return { items: [...s.querySelectorAll('.ant-select-selection-item')].map((e) => e.textContent.trim()) };
  })()`);
  if (got.err) throw new Error(got.err);
  if (!got.items.includes(value)) throw new Error(`那一项没被收下（屏上已有 ${JSON.stringify(got.items)}）：${marker}`);
  return got.items;
}

/** 顶栏（看得见那一条）的文字。为什么只读 `.bar` 而不是整张屏：三面一直挂着，
 *  而设置面那一格的话里本来就有"只比大小"那四个字——整屏找会得到一句假在场。 */
async function barText() {
  return String(await ev('(() => { const b = [...document.querySelectorAll(".bar")].filter((x) => x.getBoundingClientRect().width);'
    + ' return b.length ? b[0].innerText.replace(/\\s+/g, " ") : ""; })()'));
}

/** 等那一层真的开着（看得见、而且正文非空）。固定睡不够时，层会在下一步**之后**才开出来，
 *  于是这一格红在『没开』、下一格红在『还开着』—— B 家那条设置链连红过就是
 *  这个形状（机器正忙，plan 那一次要读两份文件，1.7 秒不够）。
 *  只等**开层**那一步：期望『根本没有层』的那两格（空路径、有事在跑）自己数，这里不代等。 */
async function waitLayer(timeoutMs = 12000) {
  const t0 = Date.now();
  for (;;) {
    const n = await ev('(() => [...document.querySelectorAll(".ant-modal")].filter((m) => m.getBoundingClientRect().width'
      + ' && (m.innerText || "").trim().length > 0).length)()');
    if (n >= 1) return Date.now() - t0;
    if (Date.now() - t0 > timeoutMs) throw new Error('等 ' + Math.round(timeoutMs / 1000) + ' 秒那一层还是没开');
    await sleep(250);
  }
}

/** 那一颗保存现在的样子（灰不灰）。禁用态读的是 React state，所以这一条同时证明"塞进去的值真的进了 state"。 */
/** 等设置面那一格里**某个字段真的出现**。那一面是第一次被点亮才去读那两份文件（还要 spawn 两次
 *  只读的 rclone 探针），固定睡 1.3 秒在整遍里够（前面几步把它焐热了），单独跑那一步就不够 ——
 *  报出来是"没有那个字段"，红的是驱动抢跑。 */
async function waitField(marker, timeoutMs = 25000) {
  const t0 = Date.now();
  for (;;) {
    const ok = await ev('(() => { const w=' + JSON.stringify(marker) + '.replace(/\\s+/g,"");'
      + 'return [...document.querySelectorAll(".ant-space-vertical")].some((s)=>s.getBoundingClientRect().width'
      + ' && (s.textContent||"").replace(/\\s+/g,"").startsWith(w) && s.querySelector("input.ant-input")); })()');
    if (ok === true) return Date.now() - t0;
    if (Date.now() - t0 > timeoutMs) throw new Error('等 ' + Math.round(timeoutMs / 1000) + ' 秒那一格还没出现：' + marker);
    await sleep(400);
  }
}
async function buttonDisabled(text, nth = 0) {
  const pt = await locate(text, 'any', nth);
  return pt.disabled === true;
}
/** 屏上现在亮着的那几条 Alert 的原文（卡片脚下那一档 + 层里那一档都算）。
 *  拒绝的句子归后台，界面只渲染 —— 所以验收要吃**原句**，不是"屏上红了一下"。
 *  只看 toast 不够：面板没有"能开层却没开"的那一档时，那句落在卡片脚下的 Alert 上（R54）。 */
async function refusalLine() {
  const js = '(() => [...document.querySelectorAll(".ant-alert")]' +
    '.filter((e) => e.getBoundingClientRect().width)' +
    '.map((e) => e.innerText.replace(/\\s+/g, " ")).join(" || "))()';
  return String(await ev(js));
}
/** 等某句话出现在**可见 Alert** 里，最多 `ms` 毫秒，返回等到了没有。
 *
 * 为什么不给固定 sleep：这一族每一步都要走一次后台往返（plan / apply / run），
 * 固定 1700ms 在面板刚起、机器还在忙的时候就量早了 —— 量到的是"上一条成功提示还在"，
 * 读起来像"拒绝没走状态口径那一档"，而其实那句拒绝 2 秒后才到（2026-10-09 实测）。
 * 于是**等那句话出现**，而不是等一个猜出来的秒数；等不到时报出现在屏上的原句。
 */
async function waitAlert(needle, ms = 12000) {
  const t0 = Date.now();
  for (;;) {
    const now = await refusalLine();
    if (now.includes(needle)) return { ok: true, line: now, ms: Date.now() - t0 };
    if (Date.now() - t0 > ms) return { ok: false, line: now, ms: Date.now() - t0 };
    await sleep(400);
  }
}
function walkFiles(dir) {
  const out = [];
  if (!fs.existsSync(dir)) return out;
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) out.push(...walkFiles(p)); else out.push(p);
  }
  return out;
}

const STEPS = [
  { id: 'bar-detail', what: '顶栏「详情」悬停：版本/端口/进程收在这里，日志目录仍在明面（R68 的前提）', run: async () => {
      await hover('详情'); await sleep(400);
    } },
  { id: 'rail-empty-state', what: '右列说的是当前那一格那条任务（"没选中"那一档在这一面已经不可达）', run: async () => {
      // 前提换了：那一面**永远有一格被选中**（列表撤了，只剩当前那一格），"没选中时四段共用那一句空态"
      // 在这一扇窗里点不出来 —— 不是判据错了，是它量到的那个状态没了。
      // "四段共用一个空态、不许一段一个图标"仍由 `frontend_pairs.rs` 的
      // `the_four_faces_share_one_empty_state` 钉（它读源码，不依赖屏上有没有选中）。
      // 这一格改量新前提本身：右列首行说的必须是当前那一格那条任务。
      const said = await ev('document.body.innerText.includes("先选一个任务。") ? 1 : 0');
      const lines = await ev('(() => { const c = document.querySelector(".ant-layout-content"); return c ? c.innerText.replace(/\\s+/g, " ").slice(0, 90) : ""; })()');
      const cell = await ev('(() => { const r = document.querySelector(".pairs-cell"); return r ? r.innerText.replace(/\\s+/g, " ").slice(0, 90) : ""; })()');
      log(`RAIL_EMPTY 那句空态还在=${said} 右列首行="${lines}" 那一格="${cell}"`);
      if (said !== 0) throw new Error('这一面已经永远有一格被选中，右列却还写着没选中：' + lines.slice(0, 60));
      // 格上写那两条完整路径、右列写那条任务的名字 —— 两处不是同一串字，所以拿后台那份数据对上：
      // 右列那条名字在 `jobs.toml` 里的 `src`/`dst`（连先后一起）必须就是格上那两条。
      const j = await cellJob();
      log(`RAIL_EMPTY 对上=${j.same ? 1 : 0} 格="${j.paths}" 右列="${j.rail}" 文件里=${j.job ? j.job.kind : '没有这一条'}`);
      if (!j.job) throw new Error('右列那一行在夹具那份文件里认不出任务：格="' + j.paths + '" 右列="' + j.rail + '"');
      if (!j.same) throw new Error('右列讲的不是当前那一格那条任务（格上="' + j.paths
        + '"，文件里那条="' + j.job.src + ' → ' + j.job.dst + '"，右列="' + j.rail + '"）');
      // 反向也要有尺子：那一行名字**不该在格上**（它回来了就是那处改动被回退了）。
      // 只钉源码的那条证人拦得住提交，拦不住"装出来的那一份其实是旧的"。
      const cellText = String(await ev('(() => { const r=document.querySelector(".pairs-cell");'
        + 'return r ? r.innerText.replace(/\\s+/g," ") : ""; })()'));
      if (j.rail !== '' && cellText.includes(j.rail)) {
        throw new Error('格上又写着右列那条任务的名字（那一行撤了）：格="' + cellText.slice(0, 160) + '"');
      }
      log('RAIL_EMPTY 格上不含名字=' + (cellText.includes(j.rail) ? 0 : 1) + ' 那一格="' + cellText.slice(0, 120) + '"');
      await shot('rail-empty-state.png');
    } },
  { id: 'browser-two-panes', what: '双栏 + 后台那句过滤说明（I8）', run: async () => {
      await twoPanes();
    } },
  { id: 'enter-a-right', what: '右栏进到 a（进入按钮）', run: async () => {
      await click('进入', 1, 0); await sleep(1200);
    } },
  { id: 'download-one', what: '下载 orphan.webp 到左栏当前位置', run: async () => {
      await click('下载', 'any', 0, 'orphan.webp'); await sleep(2200);
    } },
  { id: 'upload-collision', what: '上传撞名被拒（I2c，看不见 object not found）', run: async () => {
      await click('a.webp', 0); await sleep(300);
      await click('上传到这里', 1); await sleep(1600);
    } },
  { id: 'delete-one-layer', what: '删这一个的确认层（正文第一行是完整位置）', run: async () => {
      await click('上一级', 1); await sleep(1200);
      await click('del1', 1); await sleep(900);
      await click('进入', 1, 0, 'del1'); await sleep(1200);
      await click('删这一个', 'any', 0, 'drop.webp'); await sleep(1600);
    } },
  { id: 'delete-one-done', keepsLayer: true, what: '确认之后：清单 1 · 实际 1 · 一致', run: async () => {
      await click('我知道，删这', 'any'); await sleep(2400);
    } },
  { id: 'delete-visible-layer', what: '删掉这一层看到的 N 个（N 来自后台）', run: async () => {
      await click('上一级', 1); await sleep(1200);
      await click('进入', 1, 0, 'delN'); await sleep(1400);
      await click('删掉这一层看到的', 1); await sleep(1600);
    } },
  { id: 'delete-visible-done', keepsLayer: true, what: '那一次的报告句（两个数并列）', run: async () => {
      await click('我知道，删这', 'any'); await sleep(2600);
    } },
  { id: 'purge-greyed-note', what: '清空这一层置灰 + Tooltip 里那句后台的话（R74/I8）', run: async () => {
      await click('上一级', 1); await sleep(1200);
      await click('进入', 1, 0, 'grey'); await sleep(1400);
      await hover('清空这一层', 1);
    } },
  { id: 'mkdir', what: '新建目录', run: async () => {
      await click('上一级', 1); await sleep(1200);
      await click('进入', 1, 0, 'mk'); await sleep(1400);
      await click('新建目录', 1); await sleep(700);
      await setInput('shot-made');
      await click('就这么定', 'any'); await sleep(2200);
    } },
  { id: 'mkdir-empty-name-says', what: '框里空着按回车：要说得出为什么（带 / 那一档会说，这一档以前只是静默）', run: async () => {
      await click('新建目录', 1); await sleep(700);
      await setInput('');
      await focusModalInput();
      await pressEnter(); await sleep(500);
      const said = await ev('document.body.innerText.includes("名字还是空的。") ? 1 : 0');
      const echo = await ev('document.body.innerText.includes("名字就是它在这一侧里的位置") ? 1 : 0');
      log(`EMPTY_NAME 说了=${said} 空值回显=${echo}`);
      // 先拍，而且**用别的名字**：每一步跑完驱动还会按步骤 id 自动拍一张，
      // 同名就把这一张盖掉——而这一张要留的是"那句在屏上"的样子，cleanup 之后就没了。
      await shot('mkdir-empty-name-message.png');
      // 再把这一步自己的事做完（用一个真名字建出来、把层关掉），然后才翻红：
      // 层开着往下走的话，后面每一步都找不到自己的按钮，那一条红就说不清是谁的。
      await setInput('shot-made2');
      await click('就这么定', 'any'); await sleep(2200);
      if (said !== 1) throw new Error('空名字按回车仍然一个字都不说');
      if (echo !== 0) throw new Error('空值时那句改名回显还挂着（光尾巴）');
    } },
  { id: 'mkdir-slash-says-why', what: '名字带 /：明面那一格只留七个字，条件收进「为什么？」（批 6 T7 的形状）', run: async () => {
      await click('新建目录', 1); await sleep(700);
      // 先确认那一层真的开着：不确认的话，后面每一个读数都是"没有模态框"那种假 0，
      // 报出来像界面坏了（这一条第一版就是这么红的）。
      const opened = await ev('(() => [...document.querySelectorAll(".ant-modal")].filter((m) => m.getBoundingClientRect().width).length)()');
      if (!opened) throw new Error('点了「新建目录」没有开出看得见的模态框');
      await setInput('a/b');
      await focusModalInput();
      await pressEnter(); await sleep(500);
      // 全部读数都**只在看得见的那只模态框里取**：整页 `body.innerText` 会把两栏那些常驻的
      // 「为什么？」一起算进来，而 antd 关掉的模态框 DOM 还留着——两头都会骗人。
      const inModal = (needle) => `(() => {
        const vis = [...document.querySelectorAll('.ant-modal')].filter((m) => m.getBoundingClientRect().width);
        const m = vis.length ? vis[vis.length - 1] : null;
        return m && m.innerText.includes(${JSON.stringify(needle)}) ? 1 : 0;
      })()`;
      const short = await ev(inModal('目录名里不要带 /'));
      const why = await ev(inModal('为什么？'));
      // 展开之前那半句不许在明面上——这正是"短槽 ≤16 字 + 细节进展开"要量的东西。
      const before = await ev(inModal('要建在别的那一层'));
      const pt = await ev(`(() => {
        const vis = [...document.querySelectorAll('.ant-modal')].filter((m) => m.getBoundingClientRect().width);
        const m = vis.length ? vis[vis.length - 1] : null;
        if (!m) return { err: '没有看得见的模态框' };
        const a = [...m.querySelectorAll('a')].find((e) => (e.textContent || '').includes('为什么'));
        if (!a) return { err: '模态框里没有那句「为什么？」可点' };
        const b = a.getBoundingClientRect();
        return { x: b.x + b.width / 2, y: b.y + b.height / 2, tag: a.tagName, text: a.textContent.trim() };
      })()`);
      if (pt.err) throw new Error(pt.err);
      await clickPoint(pt); await sleep(400);
      const after = await ev(inModal('要建在别的那一层'));
      log(`SLASH 明面=${short} 有为什么=${why} 展开前=${before} 展开后=${after}`);
      await shot('mkdir-slash-why.png');
      await click('取消', 'any'); await sleep(500);
      if (short !== 1) throw new Error('带 / 那一档不说话了');
      if (why !== 1) throw new Error('那句没有「为什么？」那一格：两段式没生效');
      if (before !== 0) throw new Error('条件那半句还挂在明面上（没进展开）');
      if (after !== 1) throw new Error('点了「为什么？」展不开那半句');
    } },
  { id: 'rmdir-empty-layer', what: '删空目录也先出确认层（清单上就那一个目录名）', run: async () => {
      await click('删空目录', 'any', 0, 'shot-made'); await sleep(1800);
    } },
  { id: 'rmdir-empty-ok', keepsLayer: true, what: '确认之后：空目录没了，报告句带那两个数', run: async () => {
      await click('我知道，删这', 'any'); await sleep(2400);
    } },
  { id: 'rmdir-nonempty-refused', what: '删非空目录：预览就拒（不等 rclone 回 500），句子在层里', run: async () => {
      await click('删空目录', 'any', 0, 'full'); await sleep(1800);
    } },
  { id: 'rmdir-nonempty-closed', keepsLayer: true, what: '关掉那一层：拒绝说过一次就够，别挡住后面几步', run: async () => {
      await click('取消', 'any'); await sleep(1200);
    } },
  { id: 'rename-collision-stays-open', what: '改名撞名：确认层不关，句子在眼前', run: async () => {
      await click('上一级', 1); await sleep(1200);
      await click('进入', 1, 0, 'rn'); await sleep(1400);
      await click('改名', 'any', 0, 'old-name.webp'); await sleep(700);
      await setInput('rn/taken.webp');
      await click('就这么定', 'any'); await sleep(1800);
    } },
  { id: 'rename-retry-ok', keepsLayer: true, what: '换个名字再来一次 → 改名跑成', run: async () => {
      await click('换个名字再来一次', 'any'); await sleep(700);
      await setInput('rn/renamed-ok.webp');
      await click('就这么定', 'any'); await sleep(1800);
      await click('我知道，搬走这', 'any'); await sleep(2400);
    } },
  { id: 'move-here-layer', what: '移到这里（跨栏：左栏选中的文件搬进右栏这一层）', run: async () => {
      await click('上一级', 1); await sleep(1200);
      await click('进入', 1, 0, 'mv'); await sleep(1400);
      await click('only-local.webp', 0); await sleep(400);
      await click('移到这里', 1); await sleep(1700);
    } },
  { id: 'move-here-done', keepsLayer: true, what: '搬完之后两侧各一行（源不再存在）', run: async () => {
      await click('我知道，搬走这', 'any'); await sleep(2600);
    } },
  { id: 'pair-summary', what: '任务行上的摘要 + 它是几点算的（跑一次盘点：点卡面那颗「看差异」）', run: async () => {
      await click('配好的位置', 'any'); await sleep(1200);
      // 点名要**那一对小位置**（对齐那一对只有几个文件）：两万个文件的复制对，它的差异面
      // 本来就该挂翻页器 —— 不点名就会量错对象（第一遍：`single-page-no-pager` 报「差异 1」，
      // 红的是尺子换了靶子）。
      // 2026-10-08：**「看差异」从收纳口提到了卡面**（它是这一格唯一只读的动作，
      // 而格上那句"点「看差异」算一次"原先指着一颗看不见的按钮）。所以不再先点「更多」。
      await click('看差异', 0, 0, PAIR_B); await sleep(2600);
    } },
  { id: 'rail-names-the-card', what: '右列开头那一行说的是哪张卡（四段共用的那一层，不用回头数底色）', run: async () => {
      const head = await ev('(() => { const c = document.querySelector(".ant-layout-content"); const el = c ? c.querySelector(".ant-typography") : null; return el ? el.textContent.trim() : ""; })()');
      log('RAIL_HEAD 首行="' + head + '"');
      if (head === '' || head.includes('先选一个任务')) throw new Error('右列没把选中的任务名摆出来："' + head + '"');
      await shot('rail-names-the-card.png');
    } },
  { id: 'pair-rail-tabs', what: '右列四段可翻：任何任务都翻得到日志（③片改掉的旧毛病）', run: async () => {
      await clickRailTab('日志');
    } },
  { id: 'rail-log-path', what: '日志面说得出完整内容在**哪一个文件**（每次动作新开一个带时间戳的文件）', run: async () => {
      const txt = await ev('(() => { const c = document.querySelector(".ant-layout-content"); return c ? c.innerText.replace(/\\s+/g, " ").slice(0, 200) : ""; })()');
      log('LOGFACE ' + txt);
      if (!/\.log/.test(txt)) throw new Error('日志面没说出是哪一个文件');
      await shot('rail-log-path.png');
    } },
  { id: 'pair-gate-flips-rail', what: '卡上点「开始对账」之后右列翻到装它结果的那一段（报告以前掉在看不见的第二段里）', run: async () => {
      await click('配好的位置', 'any'); await sleep(900);
      await click('开始对账', 0, 0); await sleep(3000);
      const sel = await ev('[...document.querySelectorAll(".ant-segmented-item-selected")].filter((e) => !e.closest(".bar")).map((e) => e.textContent.trim()).join("|")');
      const timed = await ev('document.body.innerText.includes("查的") ? 1 : 0');
      const gateRows = await ev('document.querySelectorAll(".ant-table-tbody tr").length');
      const gatePagers = await pagerCount();
      log(`GATE_FLIP 选中段="${sel}" 报告带时刻=${timed} 表格行=${gateRows} 翻页器=${gatePagers}`);
      // 段名 2026-10-09 从「检查」改成「对账」；下面那句「开始对账」是闸门卡上那颗按钮。
      // 两者现在只差一个「开始」，所以这一格读的是**选中段**（右列那一枚），不是"屏上有没有这个词"。
      if (!sel.includes('对账')) throw new Error('跑完对账没把右列翻到那一段：' + sel);
      // 对账面没有表时，翻页器断言是空断言（`single-page-no-pager` 曾经就是这种绿）。
      if (gateRows === 0) throw new Error('对账报告没有表格行，翻页器断言没有对象');
      if (gatePagers !== 0) throw new Error(`只有一两行的对账表还挂着翻页器：${gatePagers}`);
      await shot('pair-gate-flips-rail.png');
    } },
  { id: 'diff-face-one-line', what: '差异面：一句人话 + 「为什么？」展开（旧形状是一个标题 + 四行块 + 三行长文 + 列标题复述方向）', run: async () => {
      await clickRailTab('差异');
      // 2026-10-09：明面那一句原来说的是「要删请走**镜像**任务。」，词表迁移后改成了
      // 「要删请走**对齐**任务。」，而这一格还在认旧的那句 —— 于是 `one` 恒为 0，
      // 红的那一句是"解释没收成一句 + 展开"，读着像**说明又长回去了**，
      // 而实际只是它在找一句早就不在屏上的话。
      // 教训与本文件里那一族判据同源：**尺子里抄的每一句界面话都会过期**，
      // 而过期的那一种偏偏长得像"界面退步了"。所以这句跟着源码走时，
      // 上面那句 `InventoryPanel.tsx:66` 就是它的出处。
      const one = await ev('document.body.innerText.includes("这里只给你看，要删请走对齐任务。") ? 1 : 0');
      const why = await ev('[...document.querySelectorAll(".ant-typography a")].filter((e) => e.textContent.trim() === "为什么？").length');
      const oldTitle = await ev('document.body.innerText.includes("本地和云端差在哪") ? 1 : 0');
      const restated = await ev('document.body.innerText.includes("还没上传的（") ? 1 : 0');
      log(`DIFF_FACE 明面那句=${one} 展开入口=${why} 旧标题=${oldTitle} 复述式列标题=${restated}`);
      if (one !== 1 || why < 1) throw new Error('差异面的解释没收成一句 + 展开');
      if (oldTitle !== 0 || restated !== 0) throw new Error('说明又长回半屏（标题或复述式列标题回来了）');
      await shot('diff-face-one-line.png');
    } },
  // 那份"拉一次存成文件、之后自己 diff"的名单（`runner::lists`）：屏上说的路径、盘上的文件、
  // 和内容的形状，三头一起对一次。只量屏会漏"话说到了、文件没落地"；只量盘会漏"落了地、
  // 但落的不是那一串"（与设置面那几条同一族判据）。
  { id: 'inventory-list-file-on-disk', what: '差异面露出的那份名单：路径在夹具里、盘上真有、内容只有两节名字', run: async () => {
      // 自己走一遍盘点，不靠上一步恰好选中哪张卡（上一版就是这么红的：那一步把选中换成了
      // 「封面对账」，右列站在一档从没算过的差异面上，读出来是"没露出"，红的是尺子）。
      await click('配好的位置', 'any'); await sleep(1000);
      // 同上：这份名单落在**小那一对**上。判据（屏上说的路径=盘上真有=内容只有两节名字）
      // 与哪一对无关，但两万文件那一对会把十几万字节写进夹具，还会把下面那格翻页器的靶子搅浑。
      await click('看差异', 0, 0, PAIR_B); await sleep(2600);
      await clickRailTab('差异');
      const p = await ev('(() => { const c = document.querySelector(".ant-layout-content");'
        + ' const t = (c ? c.innerText : "").replace(/\\s+/g," ");'
        + ' const m = t.match(/名单存成了 (\\S+\\.txt)/); return m ? m[1] : ""; })()');
      log('LIST-FILE 屏上=' + p);
      if (p === '') throw new Error('差异面没露出这份名单落在哪（刚跑过盘点，该有这一行）');
      if (!p.startsWith(PROC.fx.replace(/\//g, '\\')) && !p.startsWith(PROC.fx)) {
        throw new Error('屏上那个路径不在夹具里（这是操作员那一份！）：' + p);
      }
      if (!fs.existsSync(p)) throw new Error('屏上说了，盘上却没有那一份：' + p);
      const text = fs.readFileSync(p, 'utf8');
      const lines = text.split(/\r?\n/);
      if (lines[0] !== '# 本地有、云端没有' || !lines.includes('# 云端有、本地没有')) {
        throw new Error('节头不对，拿去 diff 会脏：' + JSON.stringify(lines.slice(0, 3)));
      }
      if (/\d{8}-\d{6}|算的/.test(text)) {
        throw new Error('内容里漂进了时刻：两周的两份文件比起来会整片红');
      }
      await shot('inventory-list-file-on-disk.png');
    } },
  { id: 'single-page-no-pager', what: '只有一两行的表不再挂着「‹ 1 ›」（差异面两张 + 浏览器两栏）', run: async () => {
      await click('配好的位置', 'any'); await sleep(900);
      // 走 `clickRailTab` 而不是 `click(段名)`：左列卡上就有「看差异」这颗，
      // 段名一撞名，`click` 按文档顺序取到的就是**卡上那颗**——而这一格量的翻页器数
      // 在"开着一张清单"与"停在右列那一段"下可能都是 0，绿着而量的不是那一件事。
      await clickRailTab('差异');
      const diff = await pagerCount();
      await click('浏览文件', 'any'); await sleep(900);
      const browser = await pagerCount();
      log(`PAGER 差异面=${diff} 浏览器两栏=${browser}`);
      await shot('single-page-no-pager.png');
      // 收尾翻回**配好的位置那一面**：下一步（并发那一条）要在左列点「开始」，
      // 这一条量完还停在浏览器那一档的话，它就找不到自己的按钮（这么红过一次）。
      await click('配好的位置', 'any'); await sleep(800);
      if (diff !== 0 || browser !== 0)
        throw new Error(`只有一页的表还挂着翻页器：差异 ${diff}、浏览器 ${browser}`);
    } },
  { id: 'busy-while-transfer', what: '并发：一次传输正写着这棵树', run: async () => {
      // 站到那对上、拨回「只补，不删」再点：卡列表撤了之后屏上只有当前那一格，
      // 上一发把模式停在对齐那一档时这里本就没有「开始」可点。
      await openCardSelect(PAIR_C, '只补，不删');
      await click('开始', 0, 0); await sleep(600);
    } },
  { id: 'busy-refusal-in-layer', what: '浏览器发起删除被同一棵树挡住那句话', run: async () => {
      await click('浏览文件', 'any'); await sleep(700);
      await click('上一级', 1); await sleep(700);
      await click('进入', 1, 0, 'c'); await sleep(700);
      await click('删这一个', 'any', 0, 'u0.webp'); await sleep(900);
      await click('我知道，删这', 'any'); await sleep(1500);
    } },
  { id: 'refusal-toast-visible', what: '一次撞名拒绝真的到了眼前：toast 节点数与那句原文（Step 5 的口径）', run: async () => {
      await twoPanes();
      // 选同一个位置不会触发 onChange（antd 的 Select 就值不变这件事不说话），于是上一站的前缀
      // 会留在栏上。回到根这一件事现在有正规通路了：点面包屑上那个根。
      await click(PROC.remote, 'any'); await sleep(1200); await waitListed();
      await click('进入', 1, 0, 'a'); await sleep(1500);
      await click('a.webp', 0); await sleep(400);
      await click('上传到这里', 1); await sleep(1500);
    } },
  { id: 'purge-runs-without-rules', only: 'B', what: '去掉排除规则之后：清空这一层能执行（R74 的另一档）', run: async () => {
      // 远端那一栏在 `twoPanes()` 里按 HOME 取（`PROC.remote`）：写死 fxb 的话，A 那一段在
      // 下拉里找不到它 —— 同一族问题修过一次（`refusal-toast-visible`），当时漏了这条。
      await twoPanes();
      // 上一栏可能还站在某一层里（上一步刚进过 a）：Select 选**同一个位置**不触发 onChange，
      // 所以那一层的读数会留着 —— 回到根要靠面包屑那个根节点（同族见 refusal-toast-visible）。
      await click(PROC.remote, 'any'); await sleep(1400); await waitListed();
      await click('进入', 1, 0, 'grey'); await sleep(1600);
      await click('清空这一层', 1); await sleep(2500);
    } },
  { id: 'note-expand', only: 'A', what: '那句常驻解释默认只露摘要，点「为什么？」才出细节（⑤片的分层口径）', run: async () => {
      await twoPanes();
      const before = await ev('(() => document.body.innerText.includes("被排除的对象不在列表里") ? 1 : 0)()');
      await click('为什么？', 0); await sleep(600);
      const after = await ev('(() => document.body.innerText.includes("被排除的对象不在列表里") ? 1 : 0)()');
      log('NOTE before=' + before + ' after=' + after);
      if (before !== 0) throw new Error('细节没被收进去：摘要那一档就把整句露出来了');
      if (after !== 1) throw new Error('点了「为什么？」没出细节');
    } },
  { id: 'purge-done', keepsLayer: true, only: 'B', what: '清空跑完：清单 N · 实际 N · 一致，那一层空了', run: async () => {
      await click('我知道，删这', 'any'); await sleep(3000);
    } },
  // ---- 子项目 5：设置面那几步（每一滴写盘都落在夹具家；操作员那两份文件只在开头与结尾各读一次时刻）----
  // 每条都判两件事：屏上那句话 + 盘上那几个字节。只看屏会漏"话说到了、字节没落地"，
  // 只看盘会漏"落地了、但落的不是那句话"。拒绝那几条吃**原句**：句子归后台，界面只渲染。
  { id: 'settings-surface', what: '第三面在：三块卡片齐、远端就是那一份文件里的那几段、密钥值与它的键名都不在屏上', run: async () => {
      // 设置面那一段的**入口**先等界面自己那道闸放开：上面 `busy-while-transfer` 故意留了一趟
      // 两万个文件的传输在跑，而 `settings_current` 要 spawn 两次只读的 rclone 探针——机器还忙时
      // 它 1.8 秒回不来，于是这一段每一格都红在"这一栏列不出来"上，看着像界面坏了（
      // 这一遍量到的就是这个：17 格红里有 11 格是这一处传导的）。等了多久是读数，打出来。
      const idle = await waitIdle();
      log('WAIT-IDLE 进设置面之前等了 ' + idle.ms + ' 毫秒（' + JSON.stringify(idle.now) + '）');
      await click('设置', 'any');
      // 那一面**第一次被点亮时**才读那两份文件，还要 spawn 两次只读的 rclone 探针（问配置在哪、问版本）。
      // 刚跑完两万个小文件的那几十秒里，1.8 秒回不来 —— 等的是"读出来了"那一格，不是固定时长。
      // 等了多久是读数：这一段以前红过 11 格，红的全是这一处（界面没错，错在没等）。
      const t0 = Date.now();
      for (;;) {
        const now = String(await ev('(() => document.body.innerText.replace(/\\s+/g," "))()'));
        if (now.includes('这台机器上')) break;
        if (Date.now() - t0 > 40000) throw new Error('等 40 秒那一面还没读出来（现在屏上：' + now.slice(0, 160) + '）');
        await sleep(500);
      }
      log('WAIT-SETTINGS 那一面读出来等了 ' + (Date.now() - t0) + ' 毫秒');
      const t = await ev('document.body.innerText.replace(/\\s+/g," ")');
      for (const w of ['这台机器上', '你的远端', '任务', PROC.remote, '已配置，不回显']) {
        if (!t.includes(w)) throw new Error('少了一格：' + w);
      }
      if (t.includes('SENTINEL')) throw new Error('界面上出现了密钥值');
      for (const banned of ['access_key_id', 'secret_access_key', 'chunk_size']) {
        if (t.includes(banned)) throw new Error('把密钥那一行的键名端上屏了：' + banned);
      }
      // 这一条钉的是本仓在真窗口里抓到的那个缺陷：名单来自别处（浏览器那一栏的**位置** `fxa:a`）时，
      // 远端卡片明明列出来了、脚边却同时挂着「这一栏列不出来」——两句同时说就是有两个来源。
      if (t.includes('这一栏列不出来')) throw new Error('远端列出来了却同时说这一栏列不出来：' + t.slice(0, 300));
      log('SETTINGS 三块卡片=齐 远端名=' + PROC.remote + '（就是那一份文件里的段名） 哨兵与键名上屏=零 「这一栏列不出来」=没出现');
    } },
  // 操作员原话「配置难懂」落在这里：每一格除了名字，还要当场说清"它决定什么"与"空着会怎样"。
  // 源码证人钉的是字，这一步钉的是**那些字真的排到了每一格脚下**（少一格是 React 的条件，扫不出）。
  { id: 'settings-every-cell-explains-itself', what: '每格脚下那两句看得见：决定什么 + 空着的后果；两份文件各说各的名字', run: async () => {
      const t = await ev('document.body.innerText.replace(/\\s+/g," ")');
      const help = t.match(/决定[^。]{4,40}。空着[^。]{4,40}。/g) || [];
      // 这台机器上那四格 + 那一块远端的四格 = 八格（夹具一套一个远端）。
      log('CELL-HELP 成句=' + help.length + ' 样例=' + help.slice(0, 2).join(' | '));
      if (help.length < 8) throw new Error('有格子没把两句说全（读到 ' + help.length + ' 句）：' + help.join(' | '));
      for (const one of ['决定面板拿哪一份 rclone 起后台。空着保存会被拦下。',
                         '决定这个远端连哪台服务器。空着就用 rclone 自己的默认。']) {
        if (!t.includes(one)) throw new Error('那一句没在屏上：' + one);
      }
      // 两份文件的称呼要分得开（以前一句「配置文件在」指的是面板自己那份，找不到 rclone 那一面的人就卡在这）
      // 且屏上那一串必须是**夹具**的路径 —— 这一条同时证明这一遍跑的不是操作员那一份。
      if (!t.includes('任务清单文件')) throw new Error('面板自己那一份没有自己的称呼：' + t.slice(0, 200));
      const confLine = t.match(/rclone 自己的配置文件 (\S+)/);
      if (!confLine) throw new Error('rclone 那一行没上屏：' + t.slice(0, 200));
      if (!confLine[1].includes('deck-ui-' + PROC.home)) {
        throw new Error('屏上那个路径不在夹具里（这是操作员那一份！）：' + confLine[1]);
      }
      log('CONF-PATH 屏上=' + confLine[1]);
    } },
  { id: 'settings-jobs-grouped-by-pair', what: '「任务」那一块按同一对位置分组：标题就是卡上位置下拉里那一句，行一条都没被折掉', run: async () => {
      await click('设置', 'any'); await sleep(1300);
      await waitField('rclone 程序在哪');
      const g = await ev('(() => { const card = [...document.querySelectorAll(".ant-card")]'
        + '.filter((c) => c.getBoundingClientRect().width)'
        + '.find((c) => ((c.querySelector(".ant-card-head-title") || {}).textContent || "").trim() === "任务");'
        + ' if (!card) return { err: "没有「任务」那块卡片" };'
        + ' return { heads: [...card.querySelectorAll(".fs-caption")].map((e) => e.textContent.trim()).filter((t) => t !== ""),'
        + '   rows: card.querySelectorAll(".ant-listy-item").length }; })()');
      if (g.err) throw new Error(String(g.err));
      const arrow = String.fromCharCode(8596);
      const pairHeads = g.heads.filter((h) => h.includes(arrow));
      log('GROUP 标题=' + JSON.stringify(g.heads) + ' 配对标题=' + pairHeads.length + ' 行=' + g.rows);
      // 夹具那四条非对账的任务住在两对位置上 → 配对标题必须只有两个，而五条行一条不少：
      // 分组只是加标题，把行折掉就等于这一面少让人看见一条任务。
      if (pairHeads.length !== 2) throw new Error('配对标题不是两个：' + JSON.stringify(g.heads));
      if (g.rows !== 5) throw new Error('分组把行折掉了（该 5 条，读到 ' + g.rows + '）');
      // 目检要看得见那一段：分组排得对不对，DOM 读数说不上版式（标题压住行、短名撑破那一格都是看得出来的）。
      const at = await ev('(() => { const c = [...document.querySelectorAll(".ant-card")]'
        + '.filter((x) => x.getBoundingClientRect().width)'
        + '.find((x) => ((x.querySelector(".ant-card-head-title") || {}).textContent || "").trim() === "任务");'
        + ' if (!c) return -1;'
        + ' c.scrollIntoView({ block: "start" });'
        + ' return Math.round(c.getBoundingClientRect().top + window.scrollY); })()');
      await sleep(400);
      if (Number(at) < 0) throw new Error('看不见「任务」那块卡片，这一张拍不到东西');
      await shot('settings-grouped-block.png', Number(at) - 8);
      // 同一份说法：这里的标题必须就是卡上位置下拉里的那几句（一份键、一个称呼）。
      await click('配好的位置', 'any'); await sleep(1400);
      const opts = (await showPair(null)).split('|').map((o) => o.trim()).filter((o) => o !== '');
      const missing = pairHeads.filter((h) => !opts.some((o) => o.replace(/\s+/g, ' ').includes(h)));
      log('GROUP-LABELS 下拉=' + JSON.stringify(opts) + ' 对不上=' + missing.length);
      if (missing.length) throw new Error('分组标题与卡上位置下拉不是同一句：' + JSON.stringify(missing));
    } },
  // 那颗按钮要的是一条真实通路：开的是**它所在的那一层**，由后台问 rclone 得到。
  // 判据不落在"点了没报错"，而是真去问资源管理器："这一层的窗口开出来了吗"，并把我们自己
  // 开的那一格收掉（收掉=回到这遍开始前的样子）。
  { id: 'settings-open-conf-folder', what: '「打开这个目录」真的开出 rclone 那一层（夹具 conf），开完收掉', run: async () => {
      // 自己先站上这一面：这一格要能单独重跑（整遍跑完只剩"没有后台"那一种屏时，
      // 靠上一步恰好站在哪儿就不叫验收）。
      await click('设置', 'any'); await sleep(1600);
      const folder = PROC.fx + '/conf';
      // 那一串要**先把空白收成一个空格**再取：标签与路径是两个元素，`innerText` 在它们中间
      // 留的是换行 —— 上一版直接拿原文配 `/名字 (\S+)/`，配不上就报"不在夹具里"，
      // 红的是尺子（自己踩过：同一屏上一步用收白后的文本就量到了）。
      const conf = await ev('document.body.innerText.replace(/\\s+/g," ").match(/rclone 自己的配置文件 (\\S+)/)?.[1] || ""');
      if (!String(conf).startsWith(PROC.fx.replace(/\//g, '\\')) && !String(conf).startsWith(PROC.fx)) {
        throw new Error('屏上那一串不在夹具里，不敢拿它开：' + conf);
      }
      await click('打开这个目录', 'any');
      const ps = 'tools/ui-open-folder-probe.ps1';
      // 资源管理器那一带多久出现**不由我们定**：机器忙的时候（同一遍里 `waitIdle` 刚等了 21 秒）
      // 固定睡 2.5 秒会把它读成"按钮没开出去"，而那一格红完，后面整条设置链都被留下的层挡住。
      // 所以问到有为止，最多 10 秒；问不到才报，报的才是真没开。
      let out = '';
      for (let i = 0; i < 10; i++) {
        out = execSync('powershell -NoProfile -ExecutionPolicy Bypass -File "' + ps
          + '" -Path "' + folder + '"', { encoding: 'utf8' }).trim();
        if (/HITS=[1-9]/.test(out)) break;
        await sleep(1000);
      }
      log('OPEN-FOLDER ' + out.replace(/\r?\n/g, ' :: '));
      if (!/HITS=[1-9]/.test(out)) throw new Error('问了 10 秒资源管理器里还是没有那一层（按钮没开出去）：' + out);
      const notice = await ev('document.body.innerText.includes("打开目录失败") ? 1 : 0');
      if (Number(notice)) throw new Error('开出去了却同时报了一次失败：' + out);
    } },
  { id: 'settings-cancel-keeps-bytes', what: '改服务地址 → 差异里只有那一行 → 取消 → conf 一个字节没动', run: async () => {
      // 设置面这几步要**真的保存**，而"有任务在跑时保存"那道闸会先拒：先等前面那一次传输跑完，
      // 不然这几步红的是没等，不是界面（见 `waitIdle` 那段）。
      const idle = await waitIdle();
      log('WAIT-IDLE 设置面之前等到没有任务在跑=' + idle.ms + ' 毫秒');
      await click('设置', 'any'); await sleep(1300);
      const before = readConf();
      await setFieldInput('服务地址', 'https://changed-by-cdp.invalid');
      // 那颗按钮的禁用态读的是 React 的 state，所以"它亮了"就是"值真的进了 state"的读数 ——
      // 少了这一条，下一步只能报"差异表 0 行"，而那是从"值没进去"绕出来的第二手话。
      if (await buttonDisabled('保存这几项', 1)) throw new Error('改了值那颗还是灰的：值没进 React 的 state');
      // 那颗是**远端卡片**上的保存（DOM 序里第二颗：第一颗在"这台机器上"那一块）。
      await click('保存这几项', 'any', 1); await waitLayer();
      const rows = await visibleModalRows();
      const body = await visibleModalText(200);
      log('LAYER 行数=' + rows + ' 正文=' + body);
      if (rows !== 1) throw new Error('差异表不止一行：' + rows);
      // 层里那份路径同时证明"点的是远端那一颗"：那一颗写的是 rclone 自己的文件。
      if (!String(body).includes('rclone.conf')) throw new Error('正文第一行不是那份文件的完整路径：' + body);
      await click('取消', 'any'); await sleep(700);
      assertBytes('conf', before, '取消之后 conf 被动过');
    } },
  { id: 'settings-save-remote-line', what: '保存服务地址：conf 上只那一行变了，段内注释、密钥行、白名单之外的键原样', run: async () => {
      const before = readConf();
      await setFieldInput('服务地址', 'https://saved-by-cdp.invalid');
      await click('保存这几项', 'any', 1); await waitLayer();
      // 点**之前**先读一次那颗的亮灭。`finder` 只保证"那个坐标上真是它"，
      // 而一颗 `disabled` 的按钮照样占着那个坐标、照样收得到点击 —— 事件被浏览器丢掉，
      // 读数却与"点了没反应"完全一样。这一读把"按下去被丢掉"与"按下去执行了但慢"分开。
      const goBefore = String(await ev('(() => {'
        + 'const m = [...document.querySelectorAll(".ant-modal")].find((x) => x.getBoundingClientRect().width);'
        + 'if (!m) return "层不在";'
        + 'const b = [...m.querySelectorAll(".ant-modal-footer button")]'
        + '  .map((e) => (e.textContent || "").replace(/\\s+/g, "").trim() + (e.disabled ? "[灭]" : "[亮]"));'
        + 'return "[" + b.join(" | ") + "]"; })()'));
      log('SAVE-LINE 点之前那两颗=' + goBefore);
      await click('我知道，保存', 'any');
      // 按下之后**等层真的关掉**再读盘：上一版这里是一句固定 `sleep(2400)`，于是"没落盘"与
      // "落盘了但层还开着"都读成同一句「改动不止一行：0」—— 那句红得说不清是谁的错（整遍里就撞过一次）。
      //
      // ⚠ 这一段曾经**三次归错因**，所以把三次都留在原处：
      //   ① 2026-10-09：12 秒整报超时，失败屏上层已经关了 ⇒ 归因「这个猜出来的秒数太短」，放宽到 30 秒。
      //      那次的结论在当时不算错（只是没有别的解释可挑）；实测 apply 真跑起来只要 **615 毫秒**。
      //   ② 2026-10-10：放宽到 30 秒之后**仍然**报超时，红了约一半遍数，快的遍数只要 620 毫秒。
      //   ③ 2026-10-10 定案（真因）：**点击压根没落到那颗上**。判据是那颗按钮的 class ——
      //      commit 一进去就 setBusy(true)，antd 立刻加 ant-btn-loading 并置灰；
      //      而失败那 30 秒里它一直是「平着」，也就是 onClick 从没被调用过、命令压根没发出去。
      //      机制在 finder：它只验「那一瞬间那个坐标上真是它」，不保证那一刻之后它还在那儿；
      //      而 antd 弹层入场是缩放动画，waitLayer 一看见有宽度就返回，页脚按钮还在往到位处挪。
      //      修法在 finder：要求**连续两轮矩形相同**才交出坐标。
      // 教训落在读数上：**「层还在」不是一个够用的读数**。它背后是慢 / 被拒 / 点空三种完全不同的原因，
      //   屏上其他东西（做完了在不在、有没有 Alert）都分不开 —— 只有那颗按钮的 class 分得开。
      //   所以下面除了等，还把每一轮都记下来：30 秒里哪怕只变化一次，也带得回来。

      const waitFrom = Date.now();
      let openFor = 0;
      // 等层关掉的这 30 秒里，每 300 毫秒记一行**时间线**（层数 / 做完了在不在 / 有没有错误 Alert / 还脏几处）。
      // 只有"层还在"这一个读数时，"慢 / 被拒 / 那一颗是灭的"三种原因读起来一模一样；
      // 有了时间线，"层是什么时候没的、没的那一刻屏上是什么"就成了一句话。
      const timeline = [];
      for (; openFor < 100; openFor++) {
        // 整段用**模板字面量**，不是一串单引号拼接：
        // 页面侧代码里到处是双引号，逐段转义早晚有一处漏；漏掉的那一处报的是
        // `SyntaxError: Invalid or unexpected token` 加一个指向**拼接中间**的插入符，
        // 读起来像"这一行表达式有问题"，实际是**上一行**的引号没配平。
        // （唯一纪律：模板里面不许再出现反引号。）
        const snap = String(await ev(`(() => {
          const mods = [...document.querySelectorAll('.ant-modal')].filter((m) => m.getBoundingClientRect().width);
          const t = document.body.innerText;
          const alerts = [...document.querySelectorAll('.ant-alert')].filter((a) => a.getBoundingClientRect().width);
          const kinds = alerts.map((a) => (a.className || '').includes('error') ? '错误' : '提示').join('+') || '无';
          // 那颗「我知道，保存」的 class 才是**这一格真正的判据**：
          // commit 一跑就 setBusy(true)，antd 立刻给它加 ant-btn-loading 并置灰。
          // 「平着」= 点击压根没生效；「转着 / 灰着」= 命令发出去了、正挂在后台。
          // 层在不在、做完了在不在、有没有 Alert —— 这三样都分不开上面那两种，只有 class 分得开。
          const goBtn = mods[0]
            ? [...mods[0].querySelectorAll('.ant-modal-footer button')]
                .filter((b) => (b.textContent || '').includes('我知道'))[0]
            : null;
          const go = !mods[0] ? '层没了'
            : !goBtn ? '没找到那颗'
            : (goBtn.className || '').includes('loading') ? '转着'
            : goBtn.disabled ? '灰着' : '平着';
          const dirty = (t.match(/有 (\d+) 处改了还没保存/) || [])[1] || '?';
          return mods.length + '|' + (t.includes('做完了') ? '做完了在' : '做完了无')
            + '|' + kinds + '|' + dirty + '处脏' + '|' + go;
        })()`));
        timeline.push(openFor + ':' + snap);
        if (snap.startsWith('0|')) break;
        await sleep(300);
      }
      // 只留**变化过**的那几行 —— 30 秒里 100 行一模一样的读数会把真正那一次变化埋掉。
      const shape = (x) => x.slice(x.indexOf(':') + 1);
      const changes = timeline.filter((x, i) => i === 0 || shape(x) !== shape(timeline[i - 1]));
      log('SAVE-LINE 时间线（轮次/层数|做完了|Alert|脏几处|那颗）=' + JSON.stringify(changes));
      log('SAVE-LINE 层关了，用了 ' + (Date.now() - waitFrom) + ' 毫秒（上限 30000）');
      if (openFor === 100) {
        // 这一句原来只报"层还没关" + 层正文的前 240 字，而那 240 字里**永远**是
        // 「会改这几行 … 这一步不可撤销 …」，也就是**每次都一样的那段抬头**。
        // 真正有别的东西——被拒的那句话（`setError` 写在卡片脚下，不在层里）、
        // 确认那颗此刻是不是灭的（`disabled={plan === null || busy || spent}`）、
        // 「做完了 · 改设置」出没出来——一条都没带。
        //
        // 于是三种完全不同的红读起来一模一样：
        //   ① apply 真的慢；② apply 被拒（后台那句在层**外面**，所以正文里看不到）；
        //   ③ 那一颗本来就是灭的、按下去什么也没发生（层会一直开着）。
        // 2026-10-10 实测到的是同一段代码 615 毫秒与 31016 毫秒各出现过一次，
        // 而失败屏上**既没有**「做完了 · 改设置」**也没有**任何错误提示 ——
        // 两者都没有 = 既没成功也没被拒，也就是 ③：那一颗没被按下去。
        // 把这三样一起读出来，下一次红就不用再猜是哪一种。
        // ⚠ 下面这段是**要在页面里跑的表达式**，所以里面每一处 `+` 都是页面侧的拼接。
        // 本文件里为此栽过三次，每一次长得很像"这段读数偶尔抽风"而不是"拼错了"：
        //   · `all.map(...)` 落到 Node 侧求值（`all` 在这一层不存在）→ 模块加载就炸；
        //   · `return "..." + x + " || 层数=" + n` 把展示用的 `||` 写在了**引号外面**，
        //     于是 `||` 成了运算符、`层数 = ...` 成了赋值 → 页面报 SyntaxError，
        //     而那一行正好落在报错的第 498 列附近，读起来像"CDP 通道出了怪事"。
        // 分隔符要显示成 `||` 就得自己带上引号，不能靠它站在语句里。
        const why = String(await ev('(() => {'
          + 'const norm = (s) => (s || "").replace(/\\s+/g, " ").trim();'
          + 'const mods = [...document.querySelectorAll(".ant-modal")].filter((m) => m.getBoundingClientRect().width);'
          + 'const m = mods[0];'
          + 'const go = m ? [...m.querySelectorAll(".ant-modal-footer button")]'
          + '  .map((b) => norm(b.textContent) + (b.disabled ? "[灭]" : "[亮]")).join(" ") : "层没了";'
          + 'const alerts = [...document.querySelectorAll(".ant-alert")]'
          + '  .filter((a) => a.getBoundingClientRect().width).map((a) => norm(a.innerText).slice(0, 80));'
          + 'return "确认那颗=" + go + " ／ 屏上的 Alert=" + JSON.stringify(alerts)'
          + ' + " ／ 层数=" + mods.length; })()'));
        throw new Error('按下「我知道，保存」30 秒层还没关。那一句量的是"层还在"，'
          + '而层还在有三种完全不同的原因（慢 / 被拒 / 那一颗是灭的按下去什么也没发生）——'
          + '以前这一条只报层正文的前 240 字，而那 240 字每次都是同一段抬头，读不出是哪一种。'
          + '现在带上的读数：' + why);
      }
      const now = readConf();
      const nowText = fxText('conf');
      const diff = countDiffLines(before, now);
      log('SAVE-LINE 差行=' + diff + ' CRLF ' + crlf(before) + '→' + crlf(now)
        + ' 注释在=' + (nowText.includes('# 这行注释是操作员自己写的理由') ? 1 : 0)
        + ' 密钥行在=' + (now.includes('SENTINEL-SK-' + PROC.remote) ? 1 : 0)
        + ' 白名单外的键在=' + (now.includes('chunk_size = 4194304') ? 1 : 0));
      if (diff !== 1) throw new Error('改动不止一行：' + diff);
      if (crlf(now) !== crlf(before)) throw new Error('行尾被整文件换掉了');
      if (!now.includes('endpoint = https://saved-by-cdp.invalid')) throw new Error('那一行没落地');
      if (!nowText.includes('# 这行注释是操作员自己写的理由')) throw new Error('段内注释被吃掉了');
      if (!now.includes('SENTINEL-SK-' + PROC.remote)) throw new Error('密钥那一行被动了');
      if (!now.includes('chunk_size = 4194304')) throw new Error('白名单之外的键被顺手删了');
      const said = await ev('document.body.innerText.includes("重开面板后生效") ? 1 : 0');
      if (said !== 1) throw new Error('保存成功那一条没走口径表第一档（那句是后台交的）');
    } },
  { id: 'settings-refusal-empty-bin', what: 'rclone 程序那一格清空：plan 那一步就拒，句子落在卡片脚下，两份文件没动', run: async () => {
      const jobs = readJobs(), conf = readConf();
      await setFieldInput('rclone 程序在哪', '');
      await click('保存这几项', 'any');
      const w = await waitAlert('没改成');
      const alerts = w.line;
      log('REFUSE-BIN 等 ' + w.ms + '毫秒 屏上=' + alerts.slice(0, 260));
      if (!w.ok) throw new Error('等 12 秒那一句「没改成」还没出来（屏上只有上一格的成功提示）：' + alerts.slice(0, 200));
      if (!alerts.includes('那一格是空的')) throw new Error('空路径那一档拿的是"那个位置没有可执行文件"那句（说的是操作员没指过的路径）：' + alerts.slice(0, 200));
      // 被拒之后**把那一格填回原值**，而不是点「取消」：默认那几块根本没有「取消」那颗
      // （撤掉草稿的唯一通路是重开面板），`clickIfEnabled` 找不到就静默什么也不做 ——
      // 于是这份"空路径"草稿一直挂着，后面每一格按「保存这几项」都吃同一句 plan 拒绝、
      // **根本没有层可开**（历史那 5 条级联红的头就在这里，而逐条单独跑全过：单独跑时
      // 前面那一格没把现场留下）。检查类的一步也要收拾自己留下的现场。
      const bin = (fxText('jobs').match(/^rclone_bin = '([^']+)'/m) || [])[1];
      if (!bin) throw new Error('夹具那份 jobs.toml 里读不到 rclone_bin，没法把那一格填回去');
      await setFieldInput('rclone 程序在哪', bin);
      await sleep(600);
      const dirty = Number(await ev('document.body.innerText.includes("还没保存：默认那几项") ? 1 : 0'));
      log('REFUSE-BIN 填回原值之后那句还挂着=' + dirty);
      if (dirty !== 0) throw new Error('填回原值之后仍说默认那几项没保存（这一格把脏现场留给了后面）');
      assertBytes('jobs', jobs, '被拒的一次动了 jobs.toml');
      assertBytes('conf', conf, '被拒的一次动了 rclone.conf');
    } },
  { id: 'settings-new-job-collision', what: '新加一条撞了已有的编号：装载侧那句原样转达，而且没有层可开', run: async () => {
      const before = readJobs();
      await click('新加一条', 'any'); await sleep(800);
      await setFieldInput('这条的编号', 'fx-copy');
      await setFieldInput('这条叫什么名字', '验收新加的一条');
      await setFieldInput('从哪里来', PROC.fx + '/local');
      await setFieldInput('到哪里去', PROC.remote + ':a');
      await click('保存这条', 'any'); await sleep(1800);
      const alerts = await refusalLine();
      const modals = await ev('(() => [...document.querySelectorAll(".ant-modal")].filter((m) => m.getBoundingClientRect().width).length)()');
      log('COLLIDE 屏上=' + alerts.slice(0, 260) + ' 看得见的层=' + modals);
      if (!alerts.includes('已经有一条')) throw new Error('撞车那句没转达上来：' + alerts.slice(0, 200));
      // 被拦在 plan 那一步 ⇒ **没有层可开**（层是"看过差异之后"那一格）。这一条判的就是那个形状。
      if (modals !== 0) throw new Error('plan 被拦却还是开了层：那等于让人确认一份没有差异的写盘');
      await click('不要了', 'any'); await sleep(600);
      assertBytes('jobs', before, '被拒的一次动了 jobs.toml');
    } },
  { id: 'settings-remove-job-layer', what: '删这条：层里正文是那一对完整位置（不是任务名），删掉对账那一条要说它会拿走闸门', run: async () => {
      const before = readJobs();
      await click('删这条', 'any', 0, '封面对账'); await waitLayer();
      const body = await visibleModalText();
      log('REMOVE 正文=' + body.slice(0, 240));
      if (!body.includes('→')) throw new Error('正文里找不到那一串位置：' + body.slice(0, 160));
      if (!body.includes(PROC.remote + ':a')) throw new Error('正文不是那一对完整位置（名字可以撞，配对才是身份）：' + body.slice(0, 160));
      if (!body.includes('闸门')) throw new Error('删对账那一条时没说它会拿走一道闸门：' + body.slice(0, 200));
      await click('取消', 'any'); await sleep(600);
      assertBytes('jobs', before, '只是开了层，就改了文件');
    } },
  { id: 'settings-save-job-pairing', what: '改这条：镜像那一行的「从哪里来」换新目录 → jobs.toml 上只有那一行变，两条理由注释都在', run: async () => {
      const before = readJobs();
      // 自己站上设置那一面并等那两份文件读回来（与相邻那几格同一开头）：这一格以前"能单独跑"
      // 只是因为上一格恰好把它留在设置那一面 —— 单独跑在别的面时 `clickJobRow` 报的是
      // 「看不见「任务」那块卡片」，红的是这一步没带自己的前提。
      await click('设置', 'any'); await sleep(1300);
      await waitField('rclone 程序在哪');
      // 认这一行靠**分组标题**（那一对位置），不靠任务名：2026-10-11 起行上第一句是
      // 「要做什么」，名字与路径住在「为什么？」的展开里（收起时不挂 DOM），
      // `inRow='封面对齐'` 那种写法报的是「候选=0」，而那颗「改这条」在屏上好好的。
      await clickJobRow(PROC.remote + ':b', '改这条'); await sleep(800);
      await setFieldInput('从哪里来', PROC.fx + '/mirror-src-changed');
      await click('保存这条', 'any'); await waitLayer();
      const rows = await visibleModalRows();
      log('PAIR 层内行数=' + rows);
      if (rows !== 1) throw new Error('改一个字段却列出不止一行：' + rows);
      await click('我知道，保存', 'any'); await sleep(2400);
      const now = readJobs();
      const nowText = fxText('jobs');
      const diff = countDiffLines(before, now);
      log('PAIR-DIFF 差行=' + diff + ' CRLF ' + crlf(before) + '→' + crlf(now)
        + ' 头注释在=' + (nowText.includes('# 这一段是操作员自己写的理由') ? 1 : 0)
        + ' 块内注释在=' + (nowText.includes('# 这一条是批量上传用的') ? 1 : 0));
      if (diff !== 1) throw new Error('改动不止一行：' + diff);
      if (!now.includes('mirror-src-changed')) throw new Error('那一行没落地');
      // 改动落在**哪一条**上也要钉一次。只判"文件里出现了新值 + 只差一行 + 卡上还是旧的"，
      // 点错隔壁那一行也会全绿：同一块里几条任务长得一模一样，而这三条判据都不看身份。
      // 判据用这一面自己声明的那份身份 —— 新值必须住在 `fxb:b` 那一对的条目里
      // （`clickJobRow` 按分组标题认行，这里就是它那条判据的另一头）。
      const touched = now.split('[[job]]').find((b) => b.includes('mirror-src-changed')) || '';
      if (!touched.includes('dst = "' + PROC.remote + ':b"')) {
        throw new Error('新值落在了别的那一条上（该 ' + PROC.remote
          + ':b 那一对）：' + touched.replace(/\s+/g, ' ').slice(0, 160));
      }
      if (before.includes('mirror-src-changed')) throw new Error('基线里就已经是新值了（夹具没铺对，这一遍没在证明任何事）');
      if (!nowText.includes('# 这一段是操作员自己写的理由')) throw new Error('文件头那两行理由注释被吃掉了');
      if (!nowText.includes('# 这一条是批量上传用的')) throw new Error('块内那行理由注释被吃掉了');
      if (crlf(now) !== crlf(before)) throw new Error('jobs.toml 的行尾被动过');
      // 同一遍里回头再看一眼**没重开**的那一屏：卡上必须还是旧位置。
      // 这一条与最后那条 needsReopen 是一对：只量"重开之后是新值"的话，一个本来就热重载的
      // 实现也能过 —— 两头一起量，「不热重载」这句话才真有内容。
      await click('配好的位置', 'any'); await sleep(1400);
      // 读的是**配对那一面的当前那一格**。三面一直挂着，按任务名在整张屏上找会先撞上设置面那一行
      // （那一面读的就是文件，改完当然是新值）—— 上一版它绿着，量的却是配对那一面的卡；
      // 卡撤了之后同一发抓到了设置面那一行，于是红成"热进来了"，红的是尺子抓错了脸。
      await showPair(PAIR_B);
      const stale = String(await ev('(() => { const r = document.querySelector(".pairs-cell"); return r ? r.innerText.replace(/\\s+/g," ") : ""; })()'));
      log('NO-RELOAD 卡面（还没重开）=' + stale.slice(0, 200));
      // 盘上的路径分隔符两种都可能出现（夹具写的是反斜杠那一份，改之后交的是正斜杠那一份），
      // 所以先把分隔符归一再判 —— 拿一种写法去 includes 另一种，会得到一个假的"没热重载"。
      const flat = stale.replace(/\\/g, '/');
      if (flat.includes('mirror-src-changed')) throw new Error('这一面把新值热进来了（spec §2 决定 3 说的是不热重载）：' + flat.slice(0, 160));
      if (!/mirror-src(?!-changed)/.test(flat)) {
        throw new Error('卡上连旧位置都读不到，这一遍没在证明任何事：' + flat.slice(0, 160));
      }
    } },
  { id: 'settings-flags-size-only-saved', what: '「每次都要带的参数」那一格填 --size-only：闸门放行、层里只有一行，落盘之后顶栏那句**还没**出来（不热重载）', run: async () => {
      const before = readJobs();
      if (before.includes('--size-only')) throw new Error('基线里就已经带着那一枚（夹具没铺对，这一遍没在证明任何事）');
      await click('设置', 'any'); await sleep(1300);   // 上一步收尾站在对子那一面
      // 那一面是**第一次被点亮才去读那两份文件**（还要 spawn 两次只读的 rclone 探针），
      // 单独跑这一格时那 1.3 秒不够 —— 等同一张卡片里那格有 `ant-input` 的字段出现再动手。
      // （`waitField` 认的是 `input.ant-input`，而参数那一格是 tags 控件，等它自己会永远等不到。）
      await waitField('rclone 程序在哪');
      await addTagToken('每次都要带的参数', '--size-only');
      await click('保存这几项', 'any'); await waitLayer();
      // 白名单还拦着的话，plan 那一步就报错、**根本没有层可开**（与「空路径那一档」同一形状）。
      // 所以"层开着而且列了那一行"这一条量的就是"这一枚现在进得了闸门"。
      const body = await visibleModalText();
      if (!body.includes('--size-only')) throw new Error('层里没有那一枚（被拒了？还是没写进这一份意图？）：' + body.slice(0, 200));
      const rows = await visibleModalRows();
      log('FLAGS-PLAN 层内行数=' + rows);
      if (rows !== 1) throw new Error('填一枚参数却列出不止一行：' + rows);
      await click('我知道，保存', 'any'); await sleep(2400);
      const now = readJobs();
      if (!now.includes('--size-only')) throw new Error('那一枚没落到文件上：' + now.slice(0, 200));
      // 判"只多了那一行"用的是**按多重集求差**的那把（`lineDiff`）：`countDiffLines` 是逐行对齐，
      // 加一行会让它把长度差算成 1000+（它那个 +1000 就是"长度都不一样，别装了"的哨兵），
      // 于是红的是尺子的形状，而界面那一次保存是对的（同一遍里它就是差一行）。
      const { added, removed } = lineDiff(before, now);
      const text = fxText('jobs');
      log('FLAGS-DIFF 加行=' + JSON.stringify(added) + ' 拿行=' + removed.length
        + ' 头注释在=' + (text.includes('# 这一段是操作员自己写的理由') ? 1 : 0)
        + ' 块内注释在=' + (text.includes('# 这一条是批量上传用的') ? 1 : 0));
      if (removed.length !== 0) throw new Error('加一枚参数却拿走了行：' + JSON.stringify(removed));
      if (added.length !== 1 || !added[0].includes('--size-only')) {
        throw new Error('加的不止那一枚参数那一行：' + JSON.stringify(added));
      }
      if (!text.includes('# 这一段是操作员自己写的理由') || !text.includes('# 这一条是批量上传用的')) {
        throw new Error('加一行参数把理由注释吃掉了');
      }
      // 同一遍里回头看一眼**没重开**的顶栏：比法是那一次启动读进去的，没重开就不许说。
      // 与下面那条 needsReopen 是一对（两头一起量，"不热重载"这句话才真有内容）。
      const bar = await barText();
      log('FLAGS-BAR 还没重开，顶栏=' + bar.slice(0, 160));
      if (bar.includes('只比大小')) throw new Error('没重开就把新比法说成正在跑的那一种：' + bar.slice(0, 160));
    } },
  { id: 'size-only-bar-after-restart', needsReopen: true,
    what: '重开之后顶栏一直写着只比大小那一句，而且把代价一起说（那句读的是后台那枚布尔）', run: async () => {
      const bar = await barText();
      log('BAR-SIZE-ONLY 顶栏=' + bar.slice(0, 220));
      if (!bar.includes('只比大小')) throw new Error('重开了，顶栏还是没说出这一趟的比法：' + bar.slice(0, 160));
      if (!bar.includes('不会重传')) throw new Error('只报了比法、没报代价：' + bar.slice(0, 160));
    } },
  { id: 'settings-delete-job-lands', what: '删这条真的按下去：那一块的行被拿走，两块理由注释与另外两条任务原样', run: async () => {
      // 这一格的判断全走 utf8 那一份读法：拿 latin1 去比"哪几行没了"，中文那几行会成乱码，
      // 而"没提到别的任务"这种判据就悄悄不成立了（字节比对才用 latin1，见上面那几个 BYTES）。
      const before = fxText('jobs');
      await click('设置', 'any'); await sleep(1300);   // 上一步收尾站在对子那一面
      await click('删这条', 'any', 0, '封面对账'); await waitLayer();
      // 删的那一颗是 danger 那一档，文案与"保存"不是一颗：这里按下去的就是它。
      await click('我知道，删掉这条', 'any'); await sleep(2400);
      const now = fxText('jobs');
      const { removed, added } = lineDiff(before, now);
      log('DELETE 拿走=' + removed.length + ' 行 加进=' + added.length + ' 行 拿走的=[' + removed.join(' | ').slice(0, 240) + ']');
      if (added.length !== 0) throw new Error('删一条任务却往里加了行：' + JSON.stringify(added));
      if (removed.length === 0) throw new Error('一个字节都没拿走——那一发根本没落盘');
      // 拿走的必须只属于被点名的那一条：任何一行提到别的任务、或是注释，都是把别人的东西搬走了。
      for (const l of removed) {
        if (l.trim() === '') continue;                       // 空行是块与块之间的那一条
        if (l.trim().startsWith('#')) throw new Error('拿走了一行注释（那是操作员写的理由）：' + l);
        if (/fx-copy|图床批量上传|fx-sync|封面对齐|封面修剪/.test(l)) throw new Error('删对账那一条时动了别的任务：' + l);
      }
      if (!removed.some((l) => l.includes('fx-gate'))) throw new Error('拿走的行里没有那一条的编号：' + removed.join(' | '));
      if (now.includes('id = "fx-gate"') || now.includes('封面对账')) throw new Error('那一条还在文件里');
      if (!now.includes('# 这一段是操作员自己写的理由')) throw new Error('文件头那两行理由注释被吃掉了');
      if (!now.includes('# 这一条是批量上传用的')) throw new Error('块内那行理由注释被吃掉了');
      for (const keep of ['fx-copy', 'fx-copy-down', 'fx-sync', 'fx-sync-c']) {
        if (!now.includes('id = "' + keep + '"')) throw new Error('删对账那一条时动了别的任务：' + keep);
      }
      if (crlf(now) !== crlf(before)) throw new Error('行尾被动过');
      // 屏上：结果条走第一档，而任务那一块只剩两条（这一面在落盘后自己重读过一遍）。
      const t = await ev('document.body.innerText.replace(/\\s+/g," ")');
      if (!t.includes('做完了 · 改设置')) throw new Error('删完没走口径表第一档：' + t.slice(0, 160));
      if (t.includes('封面对账')) throw new Error('屏上那一块还挂着被删的那一条：' + t.slice(0, 200));
      // 只数量得着的那些行：三面都一直挂着，隐藏面里的 `.ant-listy-item` 也在 DOM 里。
      const rows = await ev('(() => [...document.querySelectorAll(".ant-listy-item")]'
        + '.filter((e) => e.getBoundingClientRect().width).length)()');
      log('DELETE 屏上任务行数=' + rows);
      if (rows !== 4) throw new Error('任务那一块该剩四条（上传、下载、两镜像）：' + rows);
    } },
  { id: 'settings-save-while-running', only: 'A', what: '有任务在跑时保存：拒，并说出是哪一条在跑', run: async () => {
      // 这一格的前提是"点保存的那一刻还在跑"。整遍跑的时候前面已经复制过一轮 `local2`，
      // 目标侧大多是命中同名的跳过 —— 传输几秒就完，那句「正在跑」永远等不到（全绿单跑、整遍跑就红）。
      // 所以每次都在这里新塞一批文件进去，让"还在跑"这件事由夹具保证，而不是由时序碰运气。
      const seed = 20000;
      const dir = PROC.fx + '/local2';
      fs.mkdirSync(dir, { recursive: true });
      for (let i = 0; i < seed; i++) fs.writeFileSync(dir + '/seed-' + i + '.webp', 'y'.repeat(48));
      log('RUNNING 新塞进 local2 的文件=' + seed + '（保证点保存那一刻还在传输）');
      await click('配好的位置', 'any'); await sleep(1000);
      await openCardSelect(PAIR_C, '只补，不删');
      const t0 = Date.now();
      await click('开始', 'any');
      // 等到卡上真写着「正在跑」再往下走：以前是固定睡 1.5 秒就去切面、填两格、点保存，
      // 而这一台机器重启之后把 6000 个小文件几秒钟就传完了 —— 那句"有事在跑"根本没机会说出口，
      // 红的其实是驱动没等（与"整遍之前要等界面那道闸放开"同一条）。
      for (;;) {
        // 卡上那一句 2026-10-09 起叫「正在写这棵目录树：X」（`types.ts::runningHint`）：
        // 旧尺子等的「正在跑」已经不在卡面上了（设置面的拒绝仍用「现在有事正在跑」，见下面那一句）。
        const running = String(await ev('document.body.innerText.includes("正在写这棵目录树") ? 1 : 0'));
        if (running === '1') break;
        if (Date.now() - t0 > 20000) throw new Error('点完「开始」等 20 秒，卡上也没写过"正在写这棵目录树"');
        await sleep(300);
      }
      log('RUNNING 从按下到看见「正在跑」=' + (Date.now() - t0) + ' 毫秒');
      await click('设置', 'any');
      const warm = await waitField('rclone 程序在哪');
      log('WAIT-SETTINGS 那一格出现=' + warm + ' 毫秒');
      // 上一格清空的那个值先补回去：这一档要判的是"有事在跑"那句，
      // 别让上一格里那个空路径替它说话（两道闸都会拒，混在一起就读不出是哪一道）。
      const bin = (/rclone_bin = '([^']+)'/.exec(readJobs()) || [])[1] || '';
      if (bin === '') throw new Error('从 jobs.toml 里没读到 rclone_bin，这一格的补值没得补');
      await setFieldInput('rclone 程序在哪', bin);
      const before = readJobs();
      await setFieldInput('日志放哪', PROC.fx + '/logs2');
      await click('保存这几项', 'any'); await sleep(1800);
      const alerts = await refusalLine();
      log('RUNNING 屏上=' + alerts.slice(0, 300));
      if (!alerts.includes('正在跑')) throw new Error('没走"有事在跑"那一档（可能那一次传输已经跑完了）：' + alerts.slice(0, 200));
      if (!alerts.includes('图床批量上传')) throw new Error('那句拒绝没说出是哪一条：' + alerts.slice(0, 200));
      assertBytes('jobs', before, '在跑的时候也被写了');
    } },
  { id: 'settings-secret-scan', what: '整遍跑完后的哨兵扫描：日志目录与证据目录里搜不到密钥值', run: async () => {
      // 扫的是**哨兵值**（SENTINEL-AK-<远端> / SENTINEL-SK-<远端>），不是 SENTINEL 这个前缀：
      // 这一格自己的读数就写着那个前缀，扫前缀等于让它自己抓自己。
      const needles = ['SENTINEL-AK-' + PROC.remote, 'SENTINEL-SK-' + PROC.remote];
      const roots = [PROC.fx + '/logs', OUT];
      const hits = [];
      let scanned = 0;
      for (const r of roots) {
        for (const f of walkFiles(r)) {
          scanned++;
          const text = fs.readFileSync(f, 'latin1');
          if (needles.some((n) => text.includes(n))) hits.push(f);
        }
      }
      log('SECRET-SCAN 根=' + roots.join(' + ') + ' 文件=' + scanned + ' 命中=' + hits.length);
      if (hits.length) throw new Error('密钥哨兵出现在盘上：' + hits.join(', '));
    } },
  { id: 'settings-reopened-shows-new-src', needsReopen: true,
    what: '重开面板后卡上才是新位置（不热重载这件事唯一的真证据）', run: async () => {
      await click('配好的位置', 'any'); await sleep(1400);
      await showPair(PAIR_B);   // 同上：重开那一遍量的必须是**配对那一面**的当前那一格
      const card = String(await ev('(() => { const r = document.querySelector(".pairs-cell"); return r ? r.innerText.replace(/\\s+/g," ") : ""; })()'));
      log('REOPEN 卡面=' + card.slice(0, 240));
      // 这一条**只在带 --reopen=1 时跑**：`runner.jobs()` 是启动那一次读进来的，
      // 没重开的窗口里它必然还是旧值 —— 那种失败才是这句话的证据，而不是通过。
      if (!card.includes('mirror-src-changed')) throw new Error('重开后卡上还是旧位置：' + card.slice(0, 160));
    } },
  // 操作员那句「完全搞不清哪个是合适的」有两半。一半是"两颗并排会按错"（注只在并排那一格说一次），
  // 另一半是「以本地为主、以镜像为主应该是选择框，而不是 N 条不同的条目」——
  // 左列改成**一张卡一对位置**，模式与方向是卡上两格选择。
  // 下面三格量的是这件事在真窗口里真的成立：卡数按对收、换格子屏上跟着换、对齐那一格只剩一颗按钮。
  // （源码证人钉的是形状，这三格钉的是"那个形状排出来之后人看得见什么"。）
  { id: 'pair-card-one-per-position', what: '一张卡=一对位置：夹具那几条任务在左列是两张配对卡，注只并排那一格说一句', run: async () => {
      // 上一格故意留了一趟两万个文件的传输；它跑完之前卡上那颗是灰的、悬停也只挂锁的那句，
      // 这一格量的"卡面注"就会被读成"没有"。先等界面自己那道闸放开（与 `settings-surface` 同一条）。
      const idle = await waitIdle();
      log('WAIT-IDLE 配对卡之前等到没有任务在跑=' + idle.ms + ' 毫秒');
      await click('配好的位置', 'any'); await sleep(1400);
      // 一列卡换成了一枚位置下拉：数"有几对"就是数那枚下拉里有几项（项面写的就是短名）。
      await openCardSelect(PAIR_C, '只补，不删');   // 那句注只跟着「只补，不删」那一档，先站到那一档上
      const opts = (await showPair(null)).split('|').map((o) => o.trim()).filter((o) => o !== '');
      const cellText = String(await ev('(() => { const r=document.querySelector(".pairs-cell");'
        + 'return r ? r.innerText.replace(/\\s+/g," ") : ""; })()'));
      // 2026-10-08：那句注**不再常驻在卡上**，挂在试跑那颗自己的**悬停**上（那颗 2026-10-09 改名叫「看看会动什么」，悬停的位置没动）。
      // 改的是"什么时候说"，没改"说什么"：它说的是那两颗的区别（并排时最容易按错），
      // 不是这一格的状态；常驻那一行让"现在是什么状态"被两行解释压着。
      // 这一格因此量两件事：屏上**没有**它（平时不占地方），悬停时它**出现**。
      //
      // 认的是 `ADD_MODE_NOTE`（`src/components/JobListView.tsx:28`）里**最稳的那半截**。
      // 原先这里找的是「**会**告诉你会动哪些，不真传」，而屏上写的是「只告诉你会动哪些，
      // 不真传；要把东西放上去，按「开始」。」—— 差一个"会"字。红出来的那一句是
      // 「悬停那颗没有那句注」，读着像**悬停没生效**，实际是悬停好好地弹了、而尺子在找
      // 一句早就不在的话。同一族的教训在 `diff-face-one-line` 那一步又犯了一次。
      const NOTE_FRAG = '只告诉你会动哪些，不真传';
      const permanent = (cellText.match(/只告诉你会动哪些，不真传/g) || []).length;
      await hover('看看会动什么', 0, 0, PAIR_C);
      const tip = String(await ev('(() => { const ts=[...document.querySelectorAll(".ant-tooltip")]'
        + '.filter((e)=>e.getClientRects().length && !String(e.className).includes("hidden"));'
        + 'const t=ts.map((e)=>e.innerText).join("|"); return t; })()'));
      await mouseAway();
      const r = { titles: opts,
        arrows: opts.filter((o) => o.includes(String.fromCharCode(8596))).length,
        permanent, tip: tip.includes(NOTE_FRAG) ? 1 : 0 };
      log('PAIR-CARDS 位置项=' + r.arrows + ' 常驻注=' + r.permanent + ' 悬停注=' + r.tip
        + ' 项面=' + JSON.stringify(r.titles));
      // 判的是"带那枚双向箭头的卡"= 2：上传与下载并入一张，对齐自己一张。
      // 行总数不钉死 —— 这一格在**没重开**与**重开**两遍里都要跑，而设置面那几步删掉了对账那一条：
      // 没重开时它还在左列（`runner.jobs()` 是启动那一次读进来的，spec §2 决定 3 说的就是不热重载）。
      if (r.arrows !== 2) throw new Error('配对卡不是两张（一对位置一张）：' + JSON.stringify(r.titles));
      if (r.permanent !== 0) throw new Error('那句"不动数据"又常驻回卡面上了（' + r.permanent + ' 次 / 该 0 次）');
      if (r.tip !== 1) throw new Error('悬停「看看会动什么」没有那句"不动数据"：' + tip.slice(0, 120));
      await shot('pair-card-one-per-position.png');
    } },
  { id: 'pair-card-direction-switch', what: '一格选项自己就是一句完整的话：换方向之后格与右列说的都是另一条任务', run: async () => {
      await click('配好的位置', 'any'); await sleep(1000);
      // 卡按**那对位置**认（远端那一侧带 `:c` 的只有这一对在用）：按任务名认会在换过方向之后找不到自己
      // —— 卡上那一行名字跟着换，这正是这一格要量的事，不能反过来当锚点用。
      //
      // 2026-10-09 重写的理由：这一格原来量的是"方向那一枚框里有两项"，而模式与方向并进
      // 「要做什么」之后**那枚框已经不存在了**。方向不再是一枚要单独拨的框，而是**每一个选项
      // 自带的后半句**（`只补，不删 —— 从本地补到云端`）。于是"换方向"在新形状里是
      // "换到**另一个**那一档的选项"，而"换对方向"的判据从"框里有两项"变成了
      // **"那一句自己就说得完"** —— 档名 + 方向 + 会不会删，不用拼两枚框的话。
      // 这一格现在真正钉的是三条：① 一个选项说得全；② 不删的那两档话里不许出现"删"；
      // ③ 换过去之后格上那两条路径换了先后、右列跟着换成另一条任务。
      const opts = await openCardSelect(PAIR_C, null);
      log('SELECT 「要做什么」那一枚=' + opts);
      const list = opts.split('|').map((s) => s.trim()).filter((s) => s !== '');
      const adds = list.filter((s) => s.includes('只补，不删'));
      const mirrors = list.filter((s) => s.includes('让两边对齐'));
      // 站在不删那一档才量换方向（有几档由下一格 `pair-card-mode-switch` 量）。
      if (adds.length === 0) throw new Error('这一对没有不删的那一档，换方向的量没有落点：' + opts);
      // 不删的那一档，**方向那半句**不许说"删"：这是操作员分不清"哪个合适"的另一半来源。
      // 2026-10-09 踩过一次：这条原来判的是**整项**里有没有"删"，而新档名「只补，不删」
      // 自己就带一个"删"字 —— 于是两档全被判成"说了删"，红的那一句读着像界面坏了。
      // 判据因此收窄到 `——` **之后**那半截（方向才是会说"删"的那一半），
      // 而"档名不许带删"这件事由另一条判据管：整项必须以 `只补，不删` 开头（见下）。
      const dirHalf = (s) => (s.includes('——') ? s.slice(s.indexOf('——') + 1) : s);
      const saysDelete = adds.filter((s) => dirHalf(s).includes('删'));
      if (saysDelete.length) throw new Error('不删的那几档，方向那半句里出现了"删"：' + JSON.stringify(saysDelete));
      // 档位那半句**只认这两个词**，不多不少 —— 「只放上去」时代这一条是靠"整项不含删"顺带
      // 兜住的，现在档名自带"不删"，兜不住了，得把它显式写下来。
      for (const s of adds) {
        const head = s.split('——')[0].trim();
        if (head !== '只补，不删') throw new Error('不删的那一档档名不是「只补，不删」：' + s);
      }
      // 这一对是**本地↔远端**，所以方向那半句必须说"本地/云端"。说成路径就是落进了"两侧同型"
      // 那一支 —— 真窗口第一遍抓到的正是这个：另一面的类型是拿 a 面取反推的。
      const toCloud = adds.find((s) => s.includes('从本地补到云端')) || '';
      const toLocal = adds.find((s) => s.includes('从云端补到本地')) || '';
      if (!toCloud) throw new Error('本地↔远端那一档没说"从本地补到云端"（读出来是：' + JSON.stringify(adds) + '）');
      if (!toLocal) throw new Error('没有"从云端补到本地"那一档：' + JSON.stringify(adds));
      // 这一格原来还钉"方向框里恰好两项"。合并之后那一条的**形状**变了，但判据没变：
      // 那一对配了上传与下载两个方向，所以不删的档位应当恰好两项 —— 多了少了都说明
      // `intentOptions` 把某一格算错了（多出来的是没建任务的方向，少了的是方向读丢了）。
      if (adds.length !== 2) {
        throw new Error('不删的那几档不是两项（这一对只配了上传与下载两个方向）：' + JSON.stringify(adds));
      }
      // 每个选项自己就说完了"哪一档 + 往哪边"，不许只有半句 —— 原来两枚框各说一半，
      // 拼错了拼出来的还是一句通顺的话，那是最难发现的一类错。
      for (const s of list) {
        if (!s.includes('——')) {
          throw new Error('有一个选项自己不是一句完整的话（少了"档位 —— 方向"这半截）：' + s);
        }
      }
      // 两个方向各走一遍：只量"换过去之后是新值"的话，一次本来就停在 b 的跑法也能过（幂等）。
      const both = [['从本地补到云端', toCloud, '图床批量上传'], ['从云端补到本地', toLocal, '图床批量下载']];
      for (const [name, opt, jobName] of both) {
        // 先拨到**另一头**再拨回来：值没变的那一发 Select 不会发 onChange（这是界面对的行为），
        // 于是"停在同一格上"的那一遍换不出右列跟着走，量到的只是上一格留下的选中。
        const away = opt === toCloud ? toLocal : toCloud;
        await openCardSelect(PAIR_C, away);
        await openCardSelect(PAIR_C, opt);
        await sleep(600);
        // 换方向在这一格上留下的**看得见的证据**是那两条路径换了先后（卡上没名字那一行了）：
        // 所以判据走 `cellJob` —— 格上那两条的先后，正是右列那条任务在文件里的 `src`/`dst`。
        const j = await cellJob();
        const rail = j.rail;
        log('SWITCH ' + name + ' 格="' + j.paths + '" 对上=' + (j.same ? 1 : 0)
          + ' 文件里=' + (j.job ? j.job.kind + ' ' + j.job.src + '→' + j.job.dst : '没有这一条'));
        log('SWITCH ' + name + ' 右列首行="' + rail + '"');
        if (!j.job || !j.same) {
          throw new Error('换了' + name + '而格上那两条路径与右列那条任务对不上：格="' + j.paths
            + '" 右列="' + j.rail + '" 文件里=' + (j.job ? j.job.src + '→' + j.job.dst : '没有这一条'));
        }
        // 框里显示的应当是**换过去的完整那一句**（含方向那半截），而不只是档位那半截。
        const shown = await selectValue('要做什么');
        if (shown !== opt) throw new Error('「要做什么」那一枚没显示换过去的这一格（框里="' + shown + '"，该="' + opt + '"）：' + j.paths);
        // 右列必须跟着翻：它讲的要是这张卡站着的那条任务（「刚点的那颗在右列演的是哪条」同族）。
        if (!rail.includes(jobName)) throw new Error('换了' + name + '而右列还站在上一条上（首行="' + rail + '"）');
      }
      await shot('pair-card-direction-switch.png');
    } },
  { id: 'pair-card-mode-switch', what: '同一张卡换档：框里只剩那一档的选项、两颗按钮换成一颗、注跟着撤、颜色跟着走', run: async () => {
      await click('配好的位置', 'any'); await sleep(1000);
      // 2026-10-09：原来这一格先读「模式」那一枚、再读「方向」那一枚，两枚各量一半。
      // 合并之后只有一枚，所以 `modes`/`dirs` 变成了 `opts`/`list` —— 每一项就是
      // **一个完整的意图**（档位 + 方向），"换模式"也就是换到**另一档的那一项**。
      const opts = await openCardSelect(PAIR_C, null);
      const list = opts.split('|').map((s) => s.trim()).filter((s) => s !== '');
      const addOpt = list.find((s) => s.includes('只补，不删')) || '';
      const mirrorOpt = list.find((s) => s.includes('让两边对齐')) || '';
      const before = await cardShape(PAIR_C);
      log('MODE 「要做什么」那一枚=' + opts + ' 不删那格=' + JSON.stringify(before));
      if (before.err) throw new Error(String(before.err));
      // 这一对三种格子都有任务（上传、下载、以本地为主对齐），所以两档都必须在框里。
      if (!addOpt || !mirrorOpt) throw new Error('两档都有任务，「要做什么」那一枚却缺了一档：' + opts);
      // 「只补，不删」那一格是**唯一**并排着两颗会动数据的按钮的地方（看看会动什么 / 开始），
      // 所以那句"试跑不动数据"只属于它。2026-10-08 起它挂在「看看会动什么」的悬停上，
      // 屏上不常驻 —— 底下那对「不常驻 / 悬停出现」由 `pair-card-one-per-position` 用真悬停量。
      // 这一格量的是"它没有被撤掉"：那两颗必须并排在，注才有意义。
      if (before.note !== 0) {
        throw new Error('那句"试跑不动数据"又常驻回卡面上了（' + before.note + '）：' + JSON.stringify(before));
      }
      if (!(before.btns.includes('看看会动什么') && before.btns.includes('开始'))) {
        throw new Error('不删的那一格没有并排着那两颗（那句注就没有可解释的对象）：' + before.btns);
      }
      await openCardSelect(PAIR_C, mirrorOpt);
      const mid = await cardShape(PAIR_C);
      // 格上那一行名字撤了（它在两行之内复述「要做什么」那一枚框的话），于是
      // "换到会删的那一格之后说的就是那一条会删的任务"不能再用一串同时出现在两处的字认。
      // 改成三件对上：**格上那两条路径（连先后）↔ 右列那一行的名字 ↔ 那份文件里那一条的 kind**。
      // 名字也不写死在尺子里：从夹具那份文件里按"这一对里那条 sync"认（写死过一回，
      // 夹具改名就永远读回空串，红的是尺子）。
      const mirrorJob = fixtureJobs().find((j) => j.kind === 'sync' && j.dst.endsWith(PAIR_C));
      const joined = await cellJob();
      log('MODE 三件对上=' + JSON.stringify({ same: joined.same, paths: joined.paths,
        rail: joined.rail, kind: joined.job ? joined.job.kind : '认不出', 那一条: mirrorJob && mirrorJob.name }));
      if (!mirrorJob) throw new Error('夹具那份文件里这一对没有一条 sync（这一格没得量）：' + PAIR_C);
      if (!joined.job) throw new Error('右列那一行在夹具那份文件里认不出是哪条任务（首行="' + joined.rail + '"）');
      if (!joined.same) {
        throw new Error('换到会删的那一格之后，格上那两条路径与右列那条任务对不上：格="' + joined.paths
          + '" 文件里那条="' + joined.job.src + ' → ' + joined.job.dst + '"（右列="' + joined.rail + '"）');
      }
      if (joined.job.name !== mirrorJob.name) {
        throw new Error('换了档而格与右列站的不是这一对的 sync 那一条（右列="' + joined.rail
          + '"，该="' + mirrorJob.name + '"）');
      }
      // 原来这里还读"另一张会删的卡"的颜色当基准。那一半随卡列表一起退了：一屏只剩一枚
      // 「要做什么」标签，两处读到的是同一个节点，"两张卡读到同一枚色"这句**没有对象**了
      // （不是判据错了，是它量到的形状没了）。接住"颜色跟着这一格走、不跟着 kind 走"的
      // 是下面那对 不删/会删 的色差本身。
      log('MODE 换到会删=' + JSON.stringify(mid));
      if (mid.btns.includes('看看会动什么') || !mid.btns.includes('让两边对齐')) {
        throw new Error('换到会删的那一格，按钮没跟着换：' + mid.btns);
      }
      if (mid.note !== 0) throw new Error('那句"试跑不动数据"跟着换到了会删的那一格（这一格没有并排，就没有可错的地方）');
      // 换档之后框里**只应剩下那一档的选项**：这一对的对齐只配了一个方向。
      // 合并之后这件事比原来更好量 —— 方向是每个选项自带的后半句，不用再开第二枚框去数。
      const after = (await openCardSelect(PAIR_C, null)).split('|').map((s) => s.trim()).filter((s) => s !== '');
      const nowMirrors = after.filter((s) => s.includes('让两边对齐'));
      if (nowMirrors.length !== 1 || !nowMirrors[0].includes('为主（删')) {
        throw new Error('换到会删那一档之后，框里没有且只有那一格（也没说出它删谁）：' + JSON.stringify(nowMirrors));
      }
      // 颜色信号：站在会删那一格是暖色，不删那一档不是 —— 同一枚标签跟着这一格换。
      if (mid.warm === before.warm) {
        throw new Error('「要做什么」那几个字的颜色没跟着这一格走（不删=' + before.warm + ' 会删=' + mid.warm + '）');
      }
      // 右列跟着这一格走那一半，由上面那三件对上接管（那里读的是同一行，且认的是名字与 kind）。
      await openCardSelect(PAIR_C, addOpt);
      const back = await cardShape(PAIR_C);
      const backOpt = await selectValue('要做什么');
      const backJob = (await cellJob()).job;
      log('MODE 换回不删=' + JSON.stringify(back) + ' 框里显示=' + backOpt
        + ' 右列那条=' + (backJob ? backJob.kind + ' ' + backJob.name : '认不出'));
      if (back.note !== 0 || back.warm !== before.warm || backOpt !== addOpt
        || !(back.btns.includes('看看会动什么') && back.btns.includes('开始'))) {
        throw new Error('换回不删的那一格没回到原样（注=' + back.note + ' 框里=' + backOpt
          + ' 按钮=' + back.btns + ' 颜色=' + back.warm + '/' + before.warm + '）');
      }
      // 换回来之后**站的是另一条**：这一对不删的那一格是 copy（上面那条 sync 的读数不该还挂着）。
      if (!backJob || backJob.kind !== 'copy') {
        throw new Error('换回不删的那一格而右列站的不是这一条 copy（' + JSON.stringify(backJob) + '）');
      }
      await shot('pair-card-mode-switch.png');
    } },
  { id: 'mirror-cell-single-button', what: '会删的那一格：只一颗「让两边对齐」、没有「看看会动什么」、没有那句注', run: async () => {
      await click('配好的位置', 'any'); await sleep(1000);
      // 2026-10-09：这一对只配了对齐那一条，所以「要做什么」那一枚**只有一项**。
      // 原来"模式框只有一格 + 方向框那一格说了删谁"是分两枚框量的；合并之后
      // 一项就是全部 —— 而且那一项自己**说得完**（档名 + 方向 + 删谁），不用拼。
      const opts = await openCardSelect(PAIR_B, null);
      const list = opts.split('|').map((s) => s.trim()).filter((s) => s !== '');
      const m = JSON.parse(String(await ev('(() => { const r=[...document.querySelectorAll(".pairs-cell, .ant-listy-item")]'
        // 这一格按**那一对位置**认，不按任务名认：卡上那行名字撤了，
        // 而格上那两条完整路径一直在（挑错一家靠的就是全文认）。
        + '.find((x)=>x.innerText.includes(' + JSON.stringify(PAIR_B) + ')); if (!r) return JSON.stringify({err:"找不到那一格（这一对没站上来？）"});'
        + 'const txt=r.innerText.replace(/\\s+/g," ");'
        + 'return JSON.stringify({ btns:[...r.querySelectorAll("button")].map((b)=>b.textContent.trim()).join("|"),'
        + ' note: txt.includes("只告诉你会动哪些") ? 1 : 0 }); })()')));
      if (m.err) throw new Error(String(m.err));
      log('MIRROR-CELL 「要做什么」那一枚=' + opts + ' 按钮=' + m.btns + ' 注=' + m.note);
      if (list.length !== 1 || !list[0].includes('让两边对齐')) {
        throw new Error('这一对的「要做什么」不是"只有一项且是会删的那一档"（它没铺不删的那些）：' + opts);
      }
      if (!list[0].includes('为主（删')) throw new Error('会删的那一项没说出它删谁：' + opts);
      if (m.btns.includes('看看会动什么')) throw new Error('会删的那一格上还有「看看会动什么」：那趟只给计数的重复列举没撤干净：' + m.btns);
      if (!m.btns.includes('让两边对齐')) throw new Error('会删的那一格没有那颗入口：' + m.btns);
      // 2026-10-08：**「看差异」从收纳口提到了卡面** —— 它是这一格唯一**只读**的动作，
      // 也是动手之前该先按的那一颗。收在「更多」里的那一次，格上那句
      // 「差异还没算过：点「看差异」算一次」指着一颗看不见的按钮，而同时看得见的那颗
      // 是会删东西的 —— 把只读的藏起来、把危险的那条摆出来，顺序是反的。
      // 所以会删的那一格现在是 **3 颗**：让两边对齐（唯一那颗会动数据的）+ 看差异（只读） + 更多。
      // 这一格真正要钉的仍然是"**没有**第二颗会动数据的按钮"：出现了「开始」才是回退。
      // 注意这里判的是**那颗传输入口**「开始」，不是档位名「让两边对齐」——
      // 两者现在都叫得出，而会删那一格上根本没有「开始」那颗。
      if (m.btns.split('|').some((b) => b.trim() === '开始')) {
        throw new Error('会删的那一格上出现了「开始」——那是不预览直接删的那条路：' + m.btns);
      }
      if (!m.btns.includes('看差异')) throw new Error('会删的那一格没有那颗只读的「看差异」：' + m.btns);
      // 「更多」只在**有东西可收**时才在屏上：会删这一格若没有 `openDir` 就没有收纳口，
      // 于是「看差异」上卡之后它是 **2 颗**而不是 3 颗。判据钉的是"**会动数据的只有一颗**"
      // （上面那条「开始」不许出现）加上"只读的那颗在"，不是颗数本身 ——
      // 颗数是布局的副产物，而这一格真正的风险是"删的入口不止一个"。
      if (m.btns.split('|').length < 2) {
        throw new Error('会删的那一格只剩一颗按钮（收起了那一格唯一那颗会动数据的？）：' + m.btns);
      }
      // 那句注 2026-10-08 之后挂在**「看看会动什么」那颗自己的悬停**上（会删这一格没有那颗，
      // 屏上更不该有它）。这一格量的是"看不见"：内文里一个字都不许有。
      if (m.note !== 0) throw new Error('那句"试跑不动数据"又跟着会删的那一格说了一遍：这一格只有一颗会动数据的按钮，没有可错的地方');
      await shot('mirror-cell-single-button.png');
    } },
  // 那一层以前只说"拿走"，没说这一趟同时会把缺的那批补上去 —— 读起来像"这一档只能删东西"，
  // 于是没人按红色的那颗。两句都要在层里看得见（前提那句按方向分叉，这一档是"本地是全集"）。
  // ==== 「换一对位置」那九步（R93-R99 的真窗口证据）====
  // 前八步在正常那一段里跑完；最后那一步 `needsReopen`，与设置面那一条重开格同一跑法
  //（单独一遍 --reopen=1）。为什么留一张卡不折掉：折掉的卡"重开之后不在"证明不了 R96 ——
  // 那已经是「删掉这张临时卡」做过的事，重开那一步就白跑了。
  { id: 'draft-layer-opens', what: '「换一对位置」那一层：两行各有位置框与路径框，候选来自后台', run: async () => {
      await click('配好的位置', 'any'); await sleep(1000);
      await click('换一对位置', 'any'); await sleep(800);
      const shape = JSON.parse(String(await ev('(() => { const vis=[...document.querySelectorAll(".ant-modal")]'
        + '.filter((m)=>m.getBoundingClientRect().width); const m=vis[vis.length-1];'
        + 'if (!m) return JSON.stringify({err:"那一层没开"});'
        + 'const ins=[...m.querySelectorAll("input.ant-input")];'
        + 'return JSON.stringify({ inputs: ins.length, labels: ins.map((i)=>i.getAttribute("aria-label")||""),'
        + ' selects: m.querySelectorAll(".ant-select").length }); })()')));
      const body = String(await visibleModalText(900));
      log('DRAFT-LAYER 形状=' + JSON.stringify(shape) + ' 正文=' + body.replace(/\s+/g, ' ').slice(0, 220));
      if (shape.err) throw new Error(String(shape.err));
      if (shape.inputs !== 2 || shape.selects !== 2) throw new Error('层里不是"两行各一格"：' + JSON.stringify(shape));
      if (!(shape.labels || []).includes('这一侧') || !(shape.labels || []).includes('那一侧')) {
        throw new Error('两行没说自己是谁（读屏里就剩两个无名框）：' + JSON.stringify(shape.labels));
      }
      if (!body.includes('就这么定')) throw new Error('那颗确认不在层里：' + body.slice(0, 120));
      // 候选必须是**问后台要的**那一份：出现这套家的远端与任务里在用的那个本地目录，
      // 才说明这一层没有自己的名单（写死一份的话，操作员加一朵云就少一个候选）。
      const opts = await openLayerSelect(0, null);
      log('DRAFT-OPTS 第 1 行的候选=' + opts.slice(0, 300));
      if (!opts.includes(PROC.remote)) throw new Error('位置候选里没有这套家的远端（那一份名单不是后台交的）：' + opts.slice(0, 200));
      // 只判"名单不空、而且带着这套家的远端"。本地那几行是**从任务里出来的**，而设置面那几步
      // 会删掉对账那一条、把对齐的源换掉 —— 拿某个具体目录名当判据，量的就成了"夹具没被前面
      // 那几步改过"，不是"候选来自后台"（整遍跑到这一格时就是这么红的）。
      if (opts.split('|').length < 4) throw new Error('位置候选少得不像后台交的那一份：' + opts.slice(0, 200));
      await click('取消', 'any'); await sleep(600);
    } },
  { id: 'draft-refusal-same-position', what: '两处写成同一个位置：那句拒绝在层里看得见，卡没多出来', run: async () => {
      // 数"有几对"要点开位置那枚下拉，所以**开层之前**数一次、**关掉之后**再数一次：
      // 层还开着的时候顶部那枚点不动（挡住它的是 `ant-modal-wrap`，等 11 秒也站不定）。
      const before = await pairCount();
      await click('换一对位置', 'any'); await sleep(800);
      await setDraftField('这一侧', DRAFT_LOCAL);
      await setDraftField('那一侧', DRAFT_LOCAL);
      await click('就这么定', 'any'); await sleep(900);
      const alerts = await refusalLine();
      if (!alerts.includes('同一个位置')) throw new Error('那句拒绝没在层里说（只飘 toast 等于没有拒绝）：' + alerts.slice(0, 240));
      const still = String(await visibleModalText(400));
      if (still === '') throw new Error('拒绝说过之后那一层自己也没了：' + still.slice(0, 120));
      await click('取消', 'any'); await sleep(600);
      const after = await pairCount();
      log('DRAFT-REFUSE-SAME 层里的句子=' + alerts.slice(0, 240) + ' 卡数=' + before + '→' + String(after));
      if (after !== before) throw new Error('被拒的那一次还是多出一张卡：' + before + '→' + after);
    } },
  { id: 'draft-card-created', what: '挑两处、就这么定：卡立起来，那一格带着临时标志（位置框里那句 + 那句"重开就没了"）', run: async () => {
      await click('换一对位置', 'any'); await sleep(800);
      // 这一侧从候选里**挑**（挑中之后的框里必须是后台那个 id 原样，不许被拼过），那一侧直接改写。
      // 挑的是远端根那一行：它的 id 以冒号结尾（`fxa:`），正是"顺手补一个斜杠 / 把冒号吃掉"最容易
      // 写坏的那种。本地目录名不适合当判据——设置面那几步会删掉对账那一条、把对齐的源换掉。
      await openLayerSelect(0, PROC.remote);
      const prefill = String(await ev('(() => { const vis=[...document.querySelectorAll(".ant-modal")]'
        + '.filter((m)=>m.getBoundingClientRect().width); const m=vis[vis.length-1];'
        + 'const i=[...m.querySelectorAll("input.ant-input")]'
        + '.find((x)=>(x.getAttribute("aria-label")||"")==="这一侧");'
        + 'return i ? i.value : ""; })()'));
      log('DRAFT-PREFILL 挑中 ' + PROC.remote + ' 之后框里=' + prefill);
      if (prefill !== PROC.remote + ':') {
        throw new Error('挑中一个起点之后框里不是后台交的那个位置原样（补过或吃掉了冒号）：'
          + prefill + ' 应当是 ' + PROC.remote + ':');
      }
      // 之后在框里下潜一层：这一格交出去的是 `fxa:d`，候选里没有这一行；
      // 这一侧改回那个本地目录（挑远端只是量"原样进框"那一件事）。
      await setDraftField('这一侧', DRAFT_LOCAL);
      await setDraftField('那一侧', DRAFT_PAIR);
      await click('就这么定', 'any'); await sleep(1400);
      const s = await draftCardShape(DRAFT_PAIR);
      log('DRAFT-CARD ' + JSON.stringify(s).slice(0, 420));
      if (s.err) throw new Error(String(s.err));
      if (!s.pos.includes('（临时）')) throw new Error('位置那一枚框里没说这一对上有临时的格子：' + s.pos);
      if (!s.txt.includes('重开面板就没了')) throw new Error('那句"重开面板就没了"不在卡上（R96 不许只记在我们心里）：' + s.txt.slice(0, 200));
      // 这一对没配过任何条目 ⇒ 四格全建 ⇒ 「要做什么」那一枚里有四项
      // （两档 × 两个方向）。合并之后这一格比原来**更好量**：原来要点开两枚框各数两项，
      // 而两项 × 两项的组合里有一项是配不出来的（两侧都本地时没有会删那一档）——
      // 现在每一项自己就说完了它是哪一档，所以"四格里每一格都列得出来"直接数项数就够。
      // 那些项要点开那一枚才读得到，所以走 `openCardSelect`（下拉那份逻辑只许有一份）。
      const cells = await openCardSelect(DRAFT_PAIR, null);
      const cellList = cells.split('|').map((s) => s.trim()).filter((s) => s !== '');
      log('DRAFT-CELLS 「要做什么」那一枚=' + cells);
      if (cellList.length !== 4) throw new Error('新卡那一枚没有列出四格（两档 × 两个方向）：' + cells);
      const draftAdds = cellList.filter((s) => s.includes('只补，不删'));
      const draftMirrors = cellList.filter((s) => s.includes('让两边对齐'));
      if (draftAdds.length !== 2 || draftMirrors.length !== 2) {
        throw new Error('新卡那四格不是"两档各两个方向"：' + JSON.stringify(cellList));
      }
      // 每一项自己就是一句完整的话（档位 —— 方向），不许只列半句。
      for (const s of cellList) {
        if (!s.includes('——')) throw new Error('新卡那枚里有一项自己不是完整的一句：' + s);
      }
      if (!s.btns.includes('存成常用')) throw new Error('临时卡上没有那颗「存成常用」：' + s.btns);
      // `btns` 里那颗收纳口写作「更 多」：antd 在两个汉字之间插一个空格（同一族判据在 `finder` 里也是先压空格）。
      if (!s.btns.replace(/\s+/g, '').includes('更多')) throw new Error('那颗「删掉这张临时卡」没进收纳口（卡面上多一颗主操作）：' + s.btns);
    } },
  { id: 'draft-transfer-runs', what: '临时格真跑一次传输：文件落在盘上，右列跟着讲这张卡', run: async () => {
      await waitIdle();
      const hit = await locate('开始', 'any', 0, DRAFT_PAIR);
      if (hit.err) throw new Error(String(hit.err));
      await clickPoint(hit);
      const idle = await waitIdle();
      log('DRAFT-RUN 等到屏上没在跑=' + idle.ms + ' 毫秒');
      const files = fs.existsSync(DRAFT_DIR) ? fs.readdirSync(DRAFT_DIR) : [];
      log('DRAFT-RUN 盘上 ' + DRAFT_DIR + ' = ' + JSON.stringify(files));
      if (files.length < 3) throw new Error('临时格跑完，云端那一侧没多出东西（那一步跑的到底是什么）：' + JSON.stringify(files));
      const rail = String(await ev('(() => { const c=document.querySelector(".ant-layout-content");'
        + 'const e=c?c.querySelector(".ant-typography"):null; return e?e.textContent.trim():""; })()'));
      log('DRAFT-RUN 右列首行=' + rail);
      if (!rail.includes('临时（没存进配置）')) throw new Error('传输跑完了而右列还站在别的卡上（首行="' + rail + '"）');
    } },
  { id: 'draft-mirror-needs-confirmation', what: '临时卡换到会删的那一档：只一颗「让两边对齐」，层里两半都说全，取消之后一个东西没动', run: async () => {
      const modes = await openCardSelect(DRAFT_PAIR, null);
      const addOpt = modes.split('|').find((x) => x.includes('只补，不删')) || '';
      const mirrorOpt = modes.split('|').find((x) => x.includes('让两边对齐')) || '';
      if (!mirrorOpt) throw new Error('临时卡那一枚里没有会删的那一档（操作员批准过的一格没建出来）：' + modes);
      await openCardSelect(DRAFT_PAIR, mirrorOpt);
      const s = await draftCardShape(DRAFT_PAIR);
      log('DRAFT-MIRROR 按钮=' + s.btns + ' 注=' + s.note);
      if (s.btns.includes('看看会动什么') || !s.btns.includes('让两边对齐')) throw new Error('换到会删的那一档按钮没跟着换：' + s.btns);
      if (s.note !== 0) throw new Error('那句"试跑不动数据"跟着换到了会删的那一档');
      await click('让两边对齐', 'any', 0, DRAFT_PAIR); await sleep(1000);
      const body = String(await visibleModalText(900));
      log('DRAFT-MIRROR-LAYER 正文=' + body.replace(/\s+/g, ' ').slice(0, 300));
      if (body === '') throw new Error('那一层没开（卡上写着：' + s.txt.slice(0, 80) + '）');
      for (const half of ['修剪成', '也会把本地有、云端没有的那批补上去', '两边都有但内容不一样的']) {
        if (!body.includes(half)) throw new Error('层里少了那一半：' + half);
      }
      await shot('draft-mirror-layer.png');
      await click('取消', 'any'); await sleep(700);
      const after = fs.existsSync(DRAFT_DIR) ? fs.readdirSync(DRAFT_DIR) : [];
      log('DRAFT-MIRROR 取消之后云端那一侧还是 ' + after.length + ' 个');
      if (after.length < 3) throw new Error('只是开层又取消，就把云端那一侧删空了：' + JSON.stringify(after));
      await openCardSelect(DRAFT_PAIR, addOpt);
    } },
  { id: 'draft-save-opens-the-form', what: '「存成常用」= 切到设置面、已有的那张表单带着这两个位置打开、那句实话在', run: async () => {
      await click('存成常用', 'any', 0, DRAFT_PAIR);
      // 那一面第一次被点亮时才读那两份文件（还要 spawn 两次只读的 rclone 探针），
      // 所以固定等 1.4 秒会读到"还没读出来"那一档：屏上只有顶栏，表单连影子都没有。
      // 等的是**那句实话出现**，并把等了多久当读数打出来（与"等界面那道闸放开"同一族）。
      const t0 = Date.now();
      for (;;) {
        const now = String(await ev('(() => document.body.innerText.replace(/\\s+/g," "))()'));
        if (now.includes('它才作为一张配对卡出现')) break;
        if (Date.now() - t0 > 25000) throw new Error('等 25 秒那句实话还没出来（那一面读出来了吗？）：' + now.slice(0, 160));
        await sleep(500);
      }
      log('DRAFT-SAVE 等到那句实话=' + (Date.now() - t0) + ' 毫秒');
      const where = String(await ev('(() => { const seg=[...document.querySelectorAll(".ant-segmented-item")]'
        + '.find((e)=>/设置/.test(e.textContent) && String(e.className).includes("selected"));'
        + 'return (seg ? "在设置那一面 " : "没切过去 ") + document.body.innerText.replace(/\\s+/g," ").slice(0,80); })()'));
      const body = String(await ev('(() => document.body.innerText.replace(/\\s+/g," "))()'));
      log('DRAFT-SAVE ' + where);
      if (!where.startsWith('在设置那一面')) throw new Error('那一颗没切到设置面：' + where);
      if (!body.includes('它才作为一张配对卡出现')) throw new Error('那句实话没说（面板不热重载这条要当场讲清）：' + where);
      // 两个位置带着过来：框里的值就是刚才那一格的两头（不重排、不截短、不补斜杠）。
      const vals = String(await ev('(() => [...document.querySelectorAll("input.ant-input")]'
        + '.filter((i)=>i.getBoundingClientRect().width).map((i)=>i.value).join(" | "))()'));
      log('DRAFT-SAVE 表单里的值=' + vals.slice(0, 400));
      if (!vals.includes(DRAFT_LOCAL) || !vals.includes(DRAFT_PAIR)) {
        throw new Error('那两个位置没带进表单（或带来的不是原样）：' + vals.slice(0, 300));
      }
      await shot('draft-save-opens-the-form.png');
      // 那张表单的退出那颗叫「不要了」（`JobFields` 自己的词表），不是模态框那两颗里的「取消」。
      await click('不要了', 'any'); await sleep(700);
    } },
  { id: 'draft-fold-away', what: '「删掉这张临时卡」在那张卡的收纳口里：收掉之后卡没了，写过的东西留在盘上', run: async () => {
      await click('配好的位置', 'any'); await sleep(1400);
      const before = await draftCardShape(DRAFT_PAIR);
      if (before.err) throw new Error('那张卡不在（前一步没立住？）：' + String(before.err));
      const menu = String(await ev('(() => { const norm=(s)=>String(s).replace(/\\s+/g,"");'
        + 'const r=[...document.querySelectorAll(".pairs-cell, .ant-listy-item")]'
        + '.find((x)=>norm(x.innerText).includes(norm(' + JSON.stringify(DRAFT_PAIR) + ')));'
        + 'if (!r) return "找不到那张卡";'
        + 'const b=[...r.querySelectorAll("button")].find((x)=>(x.textContent||"").replace(/\\s+/g,"").includes("更多"));'
        + 'const flat=(x)=>(x.textContent||"").replace(/\\s+/g,"");' + 'if (!b) return "没有收纳口那颗：" + [...r.querySelectorAll("button")].map(flat).join("|"); b.click(); return "点了收纳口"; })()'));
      log('DRAFT-FOLD ' + menu);
      if (menu !== '点了收纳口') throw new Error('那颗「删掉这张临时卡」够不着：' + menu);
      await sleep(600);
      await click('删掉这张临时卡', 'any'); await sleep(1400);
      const gone = await draftCardShape(DRAFT_PAIR);
      const files = fs.existsSync(DRAFT_DIR) ? fs.readdirSync(DRAFT_DIR) : [];
      log('DRAFT-FOLD 删掉这张临时卡之后=' + JSON.stringify(gone).slice(0, 200) + ' 盘上还剩 ' + files.length + ' 个');
      if (!gone.err) throw new Error('收了而那张卡还在：' + String(gone.txt).slice(0, 160));
      if (files.length < 3) throw new Error('收一张卡把已经写出去的东西一起回滚了：' + JSON.stringify(files));
      // 配置里那几条一张不少（收卡只动这一趟临时立的那几格）。
      const still = await cardShape(PAIR_C);
      if (still.err) throw new Error('收一张临时卡把配置的卡也带走了：' + String(still.err));
    } },
  { id: 'draft-saved-into-config', what: '「存成常用」走到头：补编号与名字 → 那三条 → jobs.toml 上只多出那一块，屏上还是那张临时卡', run: async () => {
      await click('配好的位置', 'any'); await sleep(1300);
      const idle = await waitIdle();
      log('WAIT-IDLE 存成常用之前等到没有任务在跑=' + idle.ms + ' 毫秒');
      // 立一张 `local ↔ fxa:e` 的临时卡（`:d` 那张已经被上一格收掉了）。
      await click('换一对位置', 'any'); await sleep(900);
      await setDraftField('这一侧', DRAFT_LOCAL);
      await setDraftField('那一侧', SAVE_PAIR);
      await click('就这么定', 'any'); await sleep(1400);
      const card = await draftCardShape(SAVE_PAIR);
      if (card.err) throw new Error('那张卡没立起来，这一格没得走：' + String(card.err));
      if (!card.pos.includes('（临时）')) throw new Error('位置那一枚框里没说这一对有临时的格子：' + card.pos);
      if (!card.txt.includes('重开面板就没了')) throw new Error('那句"重开面板就没了"不在卡上：' + card.txt.slice(0, 160));
      // 存下去：切到设置面，把那两个空着的一格补上，走「保存这条 → 看差异 → 我知道，保存」。
      await click('存成常用', 'any', 0, SAVE_PAIR);
      const t0 = Date.now();
      for (;;) {
        const now = String(await ev('(() => document.body.innerText.replace(/\\s+/g," "))()'));
        if (now.includes('它才作为一张配对卡出现')) break;
        if (Date.now() - t0 > 25000) throw new Error('等 25 秒那句实话还没出来：' + now.slice(0, 160));
        await sleep(500);
      }
      log('DRAFT-SAVE2 等到那一面=' + (Date.now() - t0) + ' 毫秒');
      // 表单交回来的那两个位置就是卡上那一格的两头，原样：这一步之后落盘的应当是**读到的**这一对，
      // 不是脚本自己拼的第二份（脚本拼的话，界面把路径改了也量不出来）。
      const carried = JSON.parse(String(await ev(`(() => {
        const at = (want) => {
          const w = want.replace(/\\s+/g, '');
          const c = [...document.querySelectorAll('.ant-space-vertical')]
            .filter((s) => s.getBoundingClientRect().width)
            .filter((s) => (s.textContent || '').replace(/\\s+/g, '').startsWith(w))
            .filter((s) => s.querySelector('input.ant-input'));
          c.sort((a, b) => (a.textContent || '').length - (b.textContent || '').length);
          const i = c[0] && c[0].querySelector('input.ant-input');
          return i ? i.value : '';
        };
        return JSON.stringify({ from: at('从哪里来'), to: at('到哪里去') });
      })()`)));
      log('DRAFT-SAVE2 表单带着的位置=' + JSON.stringify(carried));
      if (!carried.from || !carried.to) throw new Error('那两个位置在表单里读不到：' + JSON.stringify(carried));
      if (!(carried.from.includes(DRAFT_LOCAL.replace(/^.+\//, '')) || carried.to.includes(DRAFT_LOCAL.replace(/^.+\//, '')))) {
        throw new Error('表单里那一对不含本地那一侧：' + JSON.stringify(carried));
      }
      if (!(carried.from.includes(SAVE_PAIR) || carried.to.includes(SAVE_PAIR))) {
        throw new Error('表单里那一对不含 ' + SAVE_PAIR + '：' + JSON.stringify(carried));
      }
      await setFieldInput('这条的编号', SAVE_ID);
      await setFieldInput('这条叫什么名字', SAVE_NAME);
      const before = readJobs();
      const fxTextBefore = fxText('jobs');
      if (before.includes(SAVE_ID)) throw new Error('基线里就已经有那条了（夹具没铺对，这一遍没在证明任何事）');
      await click('保存这条', 'any'); await waitLayer();
      const rows = await visibleModalRows();
      const body = await visibleModalText(300);
      // 层里那份差异**必须说得出这一条是新加的还是改哪一行**：它此刻是"要不要落盘"的唯一依据。
      log('DRAFT-SAVE2 层内行数=' + rows + ' 正文=' + body);
      if (rows < 1) throw new Error('开了层却没列出要写的那几行（那这份确认没在看任何东西）：' + body);
      // 确认层要复述**那一对完整位置**（对齐那一条钉的是同一件事）：落盘前人该看到两头，
      // 不是任务名，也不是"有一处改动"。
      if (!body.includes(SAVE_PAIR) || !body.includes('→')) {
        throw new Error('层里没把那一一对完整位置复述出来：' + body.slice(0, 220));
      }
      await click('我知道，保存', 'any'); await sleep(2400);
      const now = readJobs();
      const { removed } = lineDiff(before, now);
      // 那一块的行要按 **utf-8** 那份比：`readJobs` 交的是 latin1（一字节一字符，比字节用的），
      // 拿它去比 `name = "验收…"` 这种带中文的行，每一行都会比不中——那是尺子坏了，不是没写进去。
      const nowText = fxText('jobs');
      const { added } = lineDiff(fxTextBefore, nowText);
      log('DRAFT-SAVE2 拿走=' + removed.length + ' 加上=' + added.length
        + ' CRLF ' + crlf(before) + '→' + crlf(now)
        + ' 头注释在=' + (nowText.includes('# 这一段是操作员自己写的理由') ? 1 : 0)
        + ' 块内注释在=' + (nowText.includes('# 这一条是批量上传用的') ? 1 : 0));
      // 存一张临时卡**不该动到别人**：拿走 0 行 = 配置里原有的每一行都在原位。
      if (removed.length !== 0) throw new Error('存这一条把已有的行拿走了：' + JSON.stringify(removed.slice(0, 6)));
      // 加上的那几行必须**就是**这一条的那几行（哪一格存下去，落的就只有那一格）：
      // 多出一行 = 顺手写了没选的东西（open_dir、manifest 那一档就在这条判据的射程里）；
      // 少一行 = 那一格没存全，重开之后它回不来。
      const kindWord = card.txt.includes('让两边对齐') ? 'sync' : 'copy';
      const block = ['[[job]]', `id = "${SAVE_ID}"`, `name = "${SAVE_NAME}"`,
        `kind = "${kindWord}"`, `src = "${carried.from}"`, `dst = "${carried.to}"`];
      const got = added.map((l) => l.trim());
      // 2026-10-10：比的是**解掉外层引号之后的值**，不是原样那几行字符。
      // 原先这里直接 `got.includes(l)`，于是有一条永远是红的，而红的那一句是
      // 「存这一条没把那一块写全，缺：["src = \"...\""]」—— 读着像**面板少写了一行**，
      // 实际那一行就在盘上，只是引号样式不一样：路径里含反斜杠，后台写的是 **TOML 字面量串**
      // （src = 'C:\Users\...'），而夹具里原有的那几行也全是单引号 —— **面板是对的**，
      // 写死双引号的是尺子。
      //
      // 这一族与 `settings-open-conf-folder` 那条同源：**比同一个值的两种写法**，
      // 而那一种写法不改变那个值。早先那一条是 8.3 短名 vs 长名，这一条是单引号 vs 双引号。
      // 判据要比的是"值是什么"，不是"值被写成什么样" —— 后者是排版，读数不该管。
      const unquote = (l) => l.replace(/^(\s*\w+\s*=\s*)'(.*)'\s*$/, '$1"$2"');
      const gotPlain = got.map(unquote);
      const missing = block.filter((l) => !gotPlain.includes(l));
      const extra = gotPlain.filter((l) => l !== '' && !block.includes(l));
      log('DRAFT-SAVE2 那一块的行=' + JSON.stringify(got));
      if (missing.length) throw new Error('存这一条没把那一块写全，缺：' + JSON.stringify(missing));
      if (extra.length) throw new Error('存这一条多写了不属于它的行：' + JSON.stringify(extra));
      if (!nowText.includes('# 这一段是操作员自己写的理由')) throw new Error('文件头那两行理由注释被吃掉了');
      if (!nowText.includes('# 这一条是批量上传用的')) throw new Error('块内那行理由注释被吃掉了');
      if (crlf(now) < crlf(before)) throw new Error('jobs.toml 的行尾被整文件换掉了');
      // 那句实话的另一半：存完了，**这一屏**跑的还是一张临时卡（面板不热重载）。
      await click('配好的位置', 'any'); await sleep(1400);
      const still = await draftCardShape(SAVE_PAIR);
      log('DRAFT-SAVE2 没重开那一张=' + JSON.stringify(still).slice(0, 240));
      if (still.err) throw new Error('存完这一趟就把那张卡换掉了（面板不热重载那条是反的）：' + String(still.err));
      if (!still.pos.includes('（临时）')) throw new Error('卡上的临时标志没了：要么热进来了，要么那张卡已经不是这一张：' + still.pos);
      await shot('draft-saved-into-config.png');
    } },
  { id: 'draft-card-standby-for-restart', what: '再立同一对（这张留给重开那一遍量"它不在了"）', run: async () => {
      // 自己先站到配好的位置那一面。这一格原来开头就直接点「换一对位置」，
      // 而那颗在**选择条**上 —— 上一格（`draft-saved-into-config`）做完停在设置那一面，
      // 于是它报「找不到：换一对位置 候选=0」。读着像"那颗按钮不见了"，实际是
      // **站在别的面上** —— 与 `settings-open-conf-folder` 那条开头那句
      // 「靠上一步恰好站在哪儿就不叫验收」是同一条纪律。
      await click('配好的位置', 'any'); await sleep(1200);
      await click('换一对位置', 'any'); await sleep(900);
      await setDraftField('这一侧', DRAFT_LOCAL);
      await setDraftField('那一侧', DRAFT_PAIR);
      await click('就这么定', 'any'); await sleep(1400);
      const s = await draftCardShape(DRAFT_PAIR);
      log('DRAFT-STANDBY ' + JSON.stringify(s).slice(0, 240));
      if (s.err) throw new Error('重开前那张卡没立住（那一步就没得量）：' + String(s.err));
      if (!s.pos.includes('（临时）')) throw new Error('这张 standby 卡没带临时标志：' + s.pos);
      await shot('draft-card-standby.png');
    } },
  { id: 'draft-gone-after-restart', needsReopen: true,
    what: '重开面板之后那张卡不在了，而它写出去的东西还在（R96 唯一的真证据）', run: async () => {
      const s = await draftCardShape(DRAFT_PAIR);
      const files = fs.existsSync(DRAFT_DIR) ? fs.readdirSync(DRAFT_DIR) : [];
      const screen = String(await ev('(() => document.body.innerText.replace(/\\s+/g," "))()'));
      log('DRAFT-RESTART 那张卡=' + JSON.stringify(s).slice(0, 160) + ' 盘上=' + JSON.stringify(files));
      if (files.length < 3) throw new Error('夹具没铺好：重开之前那一趟传输该在盘上留下东西（现在读到一个都没有）');
      if (!s.err) throw new Error('重开之后那张临时卡还在（它本该只活在这一趟面板里）：' + String(s.txt).slice(0, 160));
      if (screen.includes('临时（没存进配置）')) throw new Error('屏上还留着那句临时标志：卡没了而字还在，或根本没散');
      const still = await cardShape(PAIR_C);
      if (still.err) throw new Error('重开之后连配置的卡也没了（那就不叫"临时卡散掉"，叫读不到配置）：' + String(still.err));
      await shot('draft-gone-after-restart.png');
    } },
  { id: 'draft-saved-card-back-after-restart', needsReopen: true,
    what: '重开之后「存成常用」那一条作为一张配置的配对卡回来：名字是填的那个，那三处临时标志都不在', run: async () => {
      // 与 `draft-gone-after-restart` 成对：那一条量的是**没存**的那张散了，这一条量的是**存了**的那条回来了。
      // 少任何一边，"存成常用"都可能是颗没落地的按钮（文件写了但界面上永远看不见），
      // 或者那张卡本来就常驻（散不掉）。
      await click('配好的位置', 'any'); await sleep(1600);
      const s = await draftCardShape(SAVE_PAIR);
      log('DRAFT-RETURN 那一张=' + JSON.stringify(s).slice(0, 300));
      if (s.err) throw new Error('存进配置的那一条重开之后没成为一张卡：' + String(s.err));
      // 名字不住在格上了（那一行撤的），它住的地方是**右列开头那一行**。
      // 而"这一条真的回到配置里了"要的不止那串字：拿那份文件认 ——
      // 右列那一条必须正是填进去的名字，而格上那两条路径必须正是它的那两条（连先后）。
      const back = await cellJob();
      log('DRAFT-RETURN 格↔右列↔文件=' + JSON.stringify({ same: back.same, paths: back.paths,
        rail: back.rail, kind: back.job ? back.job.kind : '认不出' }));
      if (!back.job || back.job.name !== SAVE_NAME) {
        throw new Error('重开之后右列那一行不是填进去的那个名字（首行="' + back.rail + '"，该="' + SAVE_NAME + '"）');
      }
      if (!back.same) {
        throw new Error('那条名字动的位置与格上那两条对不上：格="' + back.paths
          + '" 文件里那条="' + back.job.src + ' → ' + back.job.dst + '"');
      }
      // 临时的两处标志都不该在：位置那一枚框里那句「（临时）」，与格上那句"重开就没了"。
      if (s.pos.includes('（临时）')) throw new Error('位置那一枚框还写着临时：' + s.pos);
      if (s.txt.includes('重开面板就没了')) throw new Error('那句"重开就没了"还在（后台还当它是临时格）：' + s.txt.slice(0, 160));
      if (String(s.btns).includes('存成常用')) throw new Error('那张卡还在提供「存成常用」——它已经是配置里的一条了：' + s.btns);
      // 存进去的是**一格**（那一对其余三格没写进文件），所以卡上摆出来的是那一档的颗：
      // 「看看会动什么」那颗在 = 这一条走的是不删的那一档，且它现在是一条真任务，不是内存里的样子。
      if (!String(s.btns).includes('看看会动什么')) throw new Error('卡上没有那颗试跑（回来的不像一条能跑的任务）：' + s.btns);
      await shot('draft-saved-card-back-after-restart.png');
    } },  { id: 'mirror-layer-both-halves', what: '镜像那一层把两半说全：哪一侧是全集 + 这一趟也会把缺的那批补上去', run: async () => {
      await click('配好的位置', 'any'); await sleep(1200);
      // 那一面只剩当前那一格：先站到"配了对齐那一档"的对上（夹具的 `fxa:c` 有 sync 那条），
      // 再把「要做什么」换到会删的那一档。以前这一发是在列表里找写着「让两边对齐」的那一行点它 ——
      // 列表撤了之后没有可点的行，"站对位置 + 换档"才是这一格的入口。
      await openCardSelect(PAIR_B, '让两边对齐');
      await sleep(500);
      const free = await waitEnabled('让两边对齐', 300000, PAIR_B);
      log('MIRROR-WAIT 等到那颗亮起来=' + free.ms + ' 毫秒（灭的时候卡上写着：' + free.hint + '）');
      const hit = free.hit;
      // 点空了和点中了但没开层，是两件事：上一版只看到"正文为空"，猜不出是哪一件。
      // 这里把"坐标落在谁身上"打出来（elementFromPoint 说的是命中测试的第一层）。
      log('MIRROR-CLICK 命中=<' + hit.tag + '.' + String(await ev('(() => { const e = document.elementFromPoint('
        + hit.x + ',' + hit.y + '); const c = []; let n = e;'
        + 'for (let i = 0; n && i < 3; i++, n = n.parentElement) c.push(n.tagName.toLowerCase() + (n.className ? "." + String(n.className).split(" ")[0] : ""));'
        + 'return c.join("|") + "||" + (e ? (e.textContent || "").replace(/\\s+/g, " ").trim().slice(0, 40) : "none"); })()')) + '>'
        + ' 禁用=' + (hit.disabled ? 1 : 0) + ' 坐标=' + Math.round(hit.x) + ',' + Math.round(hit.y)
        + ' 视口=' + await ev('window.innerWidth + "x" + window.innerHeight'));
      await clickPoint(hit);
      // 那两趟等待（列清单 / 复核）必须在钮上说话：以前列举途中它写着「还没清单，先重新预览」，
      // 等于人家正在算的时候叫他再去按一颗。这里采那颗的措辞序列 —— 抓到就算读数，
      // 列举太快没抓到"正在…"那一档也不判失败（存在性由源码证人钉），但**第一格就不许是那句**。
      const seen = [];
      let firstOff = null;
      for (let i = 0; i < 20; i++) {
        const s = JSON.parse(String(await ev('(() => { const m=[...document.querySelectorAll(".ant-modal")]'
          + '.filter((x)=>x.getBoundingClientRect().width)[0];'
          + 'if (!m) return JSON.stringify({open:0,label:""});'
          + 'const b=[...m.querySelectorAll(".ant-modal-footer button")];'
          + 'const go=b[b.length-1];'
          + 'return JSON.stringify({open:1,label:go?go.textContent.replace(/\\s+/g,"").trim():"", '
          + 'off:go?!!go.disabled:null}); })()')));
        if (s.open && s.label && seen[seen.length - 1] !== s.label) seen.push(s.label);
        if (s.open) firstOff = s.off;
        await sleep(150);
      }
      log('MIRROR-LOADING 那颗依次=' + (seen.join(' → ') || '一格都没抓到（层没开？）'));
      if (seen.some((x) => x.includes('还没清单'))) {
        throw new Error('列举途中那颗还在叫人家去按那颗：' + seen.join(' → '));
      }
      const body = String(await visibleModalText(900));
      const nodes = await ev('document.querySelectorAll(".ant-modal").length');
      log('MIRROR-LAYER 层节点=' + nodes + '（开着且看得见的=正文非空） 正文=' + body);
      // 「正文为空」以前会一路走到"少了那一半"那条断言，报出来像界面缺文案，其实是**层压根没开**。
      if (body === '') throw new Error('那一层没开（按下去没开层；卡上写着：' + free.hint + '）');
      // 这一格结束时会把层关掉，所以每步一张的默认那张拍不到层里那两半 —— 趁它还开着补一张。
      await shot('mirror-layer-open.png');
      for (const half of ['本地是全集：', '也会把本地有、云端没有的那批补上去', '两边都有但内容不一样的']) {
        if (!body.includes(half)) throw new Error('层里少了那一半：' + half);
      }
      // 那两条**完整位置**必须由这一层自己说出来（左列换成选择框之后，卡上不再有"这一行的 src → dst"）。
      // 判的是那一句里两个位置的**顺序**：被修剪的是 dst，src 是全集 —— 换过来就是层在指错方向。
      const flat = body.replace(/\\/g, '/');
      const trim = /把 (\S+?) 修剪成 (\S+?) 那样/.exec(flat.replace(/\/+/g, '/'));
      log('LAYER-WHERE 修剪=' + (trim ? trim[1] : '?') + ' 全集=' + (trim ? trim[2] : '?'));
      if (!trim) throw new Error('层里没把那两个完整位置说出来（哪一侧被修剪读不出来）：' + flat.slice(0, 240));
      // 这一格是「以本地为主」：被修剪的是**远端**那一侧，全集是本地那一侧。
      //（第一遍我把这两半写反了 —— 层里那一句和它脚下的前提句本来就是一致的，红的是尺子。）
      if (!trim[1].includes(PROC.remote + ':b') || !trim[2].includes('mirror-src')) {
        throw new Error('那一句指的位置不是这一格的那对（被修剪的那一侧应当是远端）：'
          + trim[1] + ' / ' + trim[2]);
      }
      // 开层**不再先列一遍**（操作员那句「不该让用户等结果」）。所以这一格第一件要量的事
      // 是"没名单那一颗当下就按得动"，而不是等一次列举才亮。
      if (firstOff !== false) throw new Error('没名单那一档那颗还是灭的（读到 disabled=' + firstOff + '）：' + seen.join(' → '));
      if (!seen.some((x) => x.includes('照对齐跑'))) throw new Error('没名单那一档没说出它是"这一趟先核对一遍"：' + seen.join(' → '));
      log('MIRROR-NO-LIST 开层即能批=1 那颗依次=' + seen.join(' → '));
      if (body.includes('我知道')) throw new Error('那颗又要人复述一遍授权（只该问一次）：' + body.slice(0, 200));
      // 「重新预览」还在：想看逐条名字的人有一条现成的路，那两个数也是在那一条之后才说得出。
      await click('重新预览', 'any');
      let listed = '';
      for (let i = 0; i < 40 && !/删这 \d+ 个/.test(listed); i++) { await sleep(500); listed = String(await visibleModalText(400)); }
      if (!/删这 \d+ 个/.test(listed)) throw new Error('预览之后那颗还是没带删除数：' + listed.slice(0, 200));
      const also = listed.match(/补上那 (\d+) 个/);
      if (also && also[1] === '0') throw new Error('那句在说"补上那 0 个"：没有可补的时候这句不许在');
      log('MIRROR-BUTTON 预览后删除数在=1 补上的那一半=' + (also ? also[1] : '没有（这一趟无可补）'));
      await click('取消', 'any'); await sleep(800);
    } },
  // 这一格是上面那句话的**反面证据**：不预览真的能把这一趟跑完，而且那几十秒屏上有人说话。
  // 它排在 `list-failure-visible`（最后一条，它要杀 rcd）之前 —— 这一跑会真的删掉远端那一侧
  // 多出来的那些，之后的步骤不再读这一对的树。
  { id: 'mirror-run-without-a-preview', what: '不预览直接批：按下就能批、右列说出"正在核对清单"、多出来的真被删掉', run: async () => {
      // 这一格要**真的动手**，所以开头先等界面自己那道闸放开：前面那几步故意留了一次传输在跑，
      // 闸没开的时候按下去只会拿到一句「现在有事正在跑」，而那是闸门按设计工作 —— 红的是没等。
      const idle = await waitIdle();
      log('WAIT-IDLE 按下之前等到没有任务在跑=' + idle.ms + ' 毫秒');
      await click('配好的位置', 'any'); await sleep(1200);
      await openCardSelect(PAIR_B, '让两边对齐');
      const free = await waitEnabled('让两边对齐', 300000, PAIR_B);
      if (free.hit.disabled) throw new Error('那颗还灭着，跑不了：' + free.hint);
      const before = walkFiles(PROC.fx + '/bucket/b').map((p) => path.basename(p)).sort();
      await click('让两边对齐', 'any', 0, PAIR_B); await sleep(600);
      const go = JSON.parse(String(await ev('(() => { const m=[...document.querySelectorAll(".ant-modal")]'
        + '.filter((x)=>x.getBoundingClientRect().width)[0]; if (!m) return JSON.stringify({err:"层没开"});'
        + 'const b=[...m.querySelectorAll(".ant-modal-footer button")]; const g=b[b.length-1];'
        + 'return JSON.stringify({label:g.textContent.replace(/\\s+/g,"").trim(), off:!!g.disabled}); })()')));
      if (go.err) throw new Error(String(go.err));
      // 只判"按得动"：上一格为了量那两个数预览过这一对，缓存的清单还在（那是界面对的行为），
      // 所以这一格那颗可能带着名单、也可能是"照对齐跑"那一档。"没名单那一档开层就能批"
      // 由上面那一格钉（MIRROR-NO-LIST），这一格钉的是**按下之后真的跑得完、真的删东西**。
      if (go.off) throw new Error('按下之后那颗还是灭的（这一格要的就是不用等一次列举）：' + JSON.stringify(go));
      log('MIRROR-RUN 那颗=' + go.label);
      await shot('mirror-no-preview-layer.png');
      // 按下那一颗不能只按文字找：整遍跑下来屏上有结果条与飘出来的通知，它们可能正压在那颗上 ——
      // 上一版就是这么"点了但什么都没发生"，右列 18 秒一个字都没变。走**命中测试站定**那一条路：
      // 坐标上必须是这一颗本身才发事件，压着的东西报出来当读数。
      const goPoint = await ev('(() => { const m=[...document.querySelectorAll(".ant-modal")]'
        + '.filter((x)=>x.getBoundingClientRect().width)[0]; if (!m) return { err:"层已经不在屏上" };'
        + 'const b=[...m.querySelectorAll(".ant-modal-footer button")]; const g=b[b.length-1];'
        + 'const r=g.getBoundingClientRect();'
        + 'const at=document.elementFromPoint(r.x+r.width/2, r.y+r.height/2);'
        + 'return { x:r.x+r.width/2, y:r.y+r.height/2, label:g.textContent.replace(/\\s+/g,"").trim(),'
        + ' on:g.contains(at)||g===at, blocker: at?String(at.className).split(" ")[0]||at.tagName.toLowerCase():"空" }; })()');
      if (goPoint.err) throw new Error(String(goPoint.err));
      log('MIRROR-PRESS 那颗=' + goPoint.label + ' 坐标上就是它=' + (goPoint.on ? 1 : 0) + ' 压着的是=' + goPoint.blocker);
      if (!goPoint.on) throw new Error('那颗被压住了，按下去不会有人接到：' + goPoint.blocker);
      const t0 = Date.now();
      await clickPoint(goPoint);
      // 右列那一句「正在核对清单」是瞬时的（夹具上列举很快），所以同一发脚本里点、下一发里立刻轮询；
      // 抓不到不算失败（存在性由源码证人钉），但抓到了要能对上时刻。
      // 终态**不靠右列停在哪一段**：上一格可能把右列留在「对账」，那一句「任务完成」根本不在屏上
      // （整遍跑就是这样红的：层已经关了、东西也删了，尺子还在等一段看不见的话）。
      // 所以这里等的是"层自己关了"，右列那两段话只作读数；真正的判据在下面盘上那两个集合。
      const seenRail = [];
      const railText = () => ev('(() => { const cs=[...document.querySelectorAll(".ant-layout-content")]'
        + '.filter((e)=>e.getBoundingClientRect().width); const c=cs[cs.length-1];'
        + 'return c?c.innerText.replace(/\\s+/g," "):""; })()');
      const modalOpen = () => ev('(() => [...document.querySelectorAll(".ant-modal")]'
        + '.filter((x)=>x.getBoundingClientRect().width).length)()');
      let refused = '';
      let open = 1;
      for (let i = 0; i < 80; i++) {
        const rail = String(await railText());
        for (const t of ['正在核对清单', '传输进行中', '排队中']) {
          if (rail.includes(t) && seenRail[seenRail.length - 1] !== t) seenRail.push(t);
        }
        open = Number(await modalOpen());
        if (!open) break;
        // 层还开着 = 这一步还没走完，或者被拒了（拒绝落在层里那句「没改成」上）。
        const body = String(await ev('document.body.innerText.replace(/\\s+/g," ")'));
        if (body.includes('没改成')) {
          // 拒绝的那一句住在**层里**（`没改成` + 「为什么？」），别去右列找它。
          refused = String(await ev('(() => { const m=[...document.querySelectorAll(".ant-modal")]'
            + '.filter((x)=>x.getBoundingClientRect().width)[0];'
            + 'return m?[...m.querySelectorAll(".ant-alert")].map(a=>a.innerText.replace(/\\s+/g," ")).join(" || ").slice(0,300):""; })()'));
          break;
        }
        await sleep(250);
      }
      log('MIRROR-RUN 从按下到层关/被拒=' + (Date.now() - t0) + ' 毫秒 右列依次=' + JSON.stringify(seenRail));
      if (refused) throw new Error('这一趟被拒了（屏上：' + refused + '）——夹具形状不该撞那两道闸');
      if (open) throw new Error('按下去 20 秒层还没关，也没见拒绝：' + JSON.stringify(seenRail));
      if (!seenRail.includes('正在核对清单')) log('MIRROR-RUN 没抓到"正在核对清单"那一档（夹具列举太快，不判失败）');
      const after = walkFiles(PROC.fx + '/bucket/b').map((p) => path.basename(p)).sort();
      // 全集**从确认层正文里读**，不写死夹具里的某个目录（2026-10-10 改）。
      // 那一层第一句就写着「这一趟把 <被修剪侧> 修剪成 <全集> 那样」，两边都摆在屏上；
      // 而这一格要的正是「远端 == **这一格当前**那个源」。
      //
      // 原先写死 mirror-src，成立的**唯一**前提是「第 45 步改的那个 src 还没生效」——
      // 而面板故意不热重载（spec §2），连续跑时确实要等重启。可编排里为了跑那几个
      // needsReopen 的步骤重开了面板，改动就生效了，于是拿旧路径当全集，红出来的是
      // 「跑完之后远端那一侧没变成源端那样」—— 那句读着像**跑了却没对齐**（真缺陷的形状），
      // 实际是尺子比了一个不是这一格的那个源。两次的差别全在这一行：
      //   连续跑  → 这一格站着 mirror-src        → 补上那 1 个 → 绿
      //   重开过  → 这一格站着 mirror-src-changed → 无可补     → 红（而那一趟删得完全正确）
      // 所以这里改成读屏上说的那个源：**这一格量的是这一格，不该假设它站在谁身上**。
      // 全集取**层开着的时候**屏上说的那个（上一行 MIRROR-PRESS 已经带回来了）。
      // 写死夹具里的某个目录不成立：第 45 步 settings-save-job-pairing 会把这条任务的 src
      // 改成 mirror-src-changed，而面板**故意不热重载**（spec §2），所以它要等重启才生效 ——
      // 那一格的前置条件原先就建立在这上面。于是同一个动作有两种结局，而红的那一种读着像
      // 「跑了却没对齐」（真缺陷的形状），实际是尺子比了一个不是这一格的那个源：
      //   连续跑 → 这一格站着 mirror-src        → 补上那 1 个 → 绿
      //   重开过 → 这一格站着 mirror-src-changed → 无可补     → 红（而那一趟删得完全正确）
      // 这一格量的应当是「远端 == **这一格当前**那个源」，与它站在谁身上无关。
      // 全集取**卡上那一格**自己脸上的 `.fs-code` —— 那一处摆的就是这一格的 `src→dst`
      // （`cellJob` 用的也是它）。不写死夹具里的目录，也不去正则确认层里的一句人话：
      // 第 45 步 `settings-save-job-pairing` 会把这条任务的 src 改成 mirror-src-changed，
      // 而面板**故意不热重载**（spec §2），所以它要等重启才生效 —— 连续跑与重开过两种编排
      // 走的是两个不同的源，而这一格量的应当是「远端 == **这一格当前**那个源」，与它站在谁身上无关。
      //
      // （走过一段弯路：先改成从确认层正文里正则那句「这一趟把 X 修剪成 Y 那样」，
      //   连着几版都不通 —— 而拿盘上那份读数离线验同一个正则又是通的。
      //   与其继续在"层开着那一瞬间、层内 innerText 的确切形状"上赌，不如换这个本来就稳的锚点：
      //   卡上那一格一直在屏上、不随层开关消失、而且有一份既有的读法。）
      // 用**模板字面量**而不是单引号串：这段页面侧代码要跨行，而单引号串不能带换行
      // （node --check 报的是 "Invalid or unexpected token"，指向这一行，读起来像表达式写坏了）。
      const shown = String(await ev(`(function () {
        const r = document.querySelector(".pairs-cell");
        const p = r ? r.querySelector(".fs-code") : null;
        if (!p) return "";
        const arrow = (p.textContent || "").indexOf(String.fromCharCode(8594));
        return arrow > 0 ? (p.textContent || "").slice(0, arrow).trim() : "";
      })()`));
      const srcDir = shown !== "" ? shown : PROC.fx + '/mirror-src';
      log('MIRROR-RUN 卡上这一格的源是 ' + srcDir + '（从 .fs-code 读的，不是写死的那个目录）');
      const src = walkFiles(srcDir).map((p) => path.basename(p)).sort();
      log('MIRROR-RUN 删之前=' + JSON.stringify(before) + ' 删之后=' + JSON.stringify(after) + ' 全集=' + JSON.stringify(src));
      if (JSON.stringify(after) !== JSON.stringify(src)) {
        throw new Error('跑完之后远端那一侧没变成源端那样：' + JSON.stringify(after) + ' vs ' + JSON.stringify(src));
      }
      // 判的是「有名字消失了」而不是个数：这一趟同时会补上缺的那些，个数可以不变。
      const removed = before.filter((f) => !after.includes(f));
      if (removed.length === 0) throw new Error('这一趟什么都没删（夹具的镜像对没铺多出来的东西？）：' + JSON.stringify(before));
      log('MIRROR-RUN 消失的那些=' + JSON.stringify(removed));
    } },
  // ==== 操作员那两报：「有些下拉框无法完整展示」+「我常用设置怎么没保存」 ====
  //
  // 量出来的根因不是弹层被挡：antd 默认 `popupMatchSelectWidth: true`，弹层宽 = 框宽，
  // 2026-10-06 当时最长的一句「只放上去（不删）」需要 108px 而 132 的框里只剩 100 —— 永远读不全。
  // 2026-10-09 模式与方向并成一枚之后，**最长的一句变得更长了**
  // （`让两边对齐（会删） —— 以本地为主（删云端多余的）`），框数从三枚降到两枚：
  // 这一格要量的仍然是"展开那一列按内容宽"，而判据必须跟着新形状走 —— 按下标循环
  // `[0,1,2]` 会去点一枚**不存在**的第三枚，报出来的是"选择条上只有 2 枚"，与
  // "弹层宽度不够"长得不像同一件事。改成**按 aria-label 遍历当前真的有的那几枚**。
  // 而"没保存"是三件不同的事：填进框里的字活着、重开才静默没了（屏上没说）；
  // 面与段根本不记；模式/方向是**故意**不记。下面三格各钉一件。
  { id: 'select-list-shows-all-the-words', what: '每一枚下拉展开的那一列按内容宽：最长那句读得全（弹层不再等于框宽）', run: async () => {
      await click('配好的位置', 'any'); await sleep(1200);
      const rows = [];
      for (const label of await pairBarLabels()) {
        const box = await topSelectBox(label);
        await clickPoint(box);
        await sleep(650);
        const r = JSON.parse(String(await ev('(() => {'
          + 'const dd=[...document.querySelectorAll(".ant-select-dropdown")].filter((d)=>d.getBoundingClientRect().width>0).pop();'
          + 'if (!dd) return JSON.stringify({ err: "弹层没开" });'
          + 'const q = dd.getBoundingClientRect();'
          + 'const items=[...dd.querySelectorAll(".ant-select-item-option")].map((it)=>{'
          + 'const b=it.querySelector(".ant-select-item-option-content")||it;'
          + 'return { t:(b.textContent||"").trim().slice(0,40), need:Math.ceil(b.scrollWidth), got:Math.ceil(b.clientWidth) };'
          + '});'
          + 'return JSON.stringify({ w: Math.round(q.width), items }); })()')));
        if (r.err) throw new Error(String(r.err));
        const clipped = r.items.filter((o) => o.need > o.got + 1);
        rows.push({ label, w: r.w, items: r.items.map((o) => o.t + '/' + o.need + '≤' + o.got) });
        // 「要做什么」那一枚最长（整句意图都在它里面），开着的时候拍一张**展开那一列**：
        // 这一格断言的是"读得全"，而循环收尾那张是关掉之后的 ——
        // 拿它当证据等于没证（目检的人看到的是两枚关着的框）。
        if (label === '要做什么') await shot('select-list-open.png');
        await closeDropdown('选择条「' + label + '」那一枚', false);
        if (clipped.length) throw new Error('「' + label + '」展开的列表把选项切掉了（弹层宽='
          + r.w + '）：' + JSON.stringify(clipped));
      }
      // 合并之后这一枚框是**唯一**装得下整句意图的地方，所以它一个都不能被切。
      // 少了这一句，下面那次循环全过也说明不了什么（位置那枚的选项本来就短）。
      if (!rows.some((r) => r.label === '要做什么')) {
        throw new Error('这一遍没量到「要做什么」那一枚：整句意图住在它里面，它是唯一要紧的那一枚');
      }
      log('SELECT-WIDTH 每一枚展开的列表宽与每格的"需要≤给到"=' + JSON.stringify(rows));
      await shot('select-list-shows-all-the-words.png');
    } },
  { id: 'settings-says-what-is-unsaved', what: '改了还没保存那一句：往默认那一块打一个词就说得出是哪一处，撤掉之后自己走', run: async () => {
      await click('设置', 'any'); await sleep(2800);
      const said = async () => String(await ev('(() => { const a=[...document.querySelectorAll(".ant-alert")]'
        + '.map((x)=>x.innerText.replace(/\\s+/g," ")).find((t)=>t.includes("还没保存")); return a || ""; })()'));
      const before = await said();
      log('UNSAVED 开局=' + JSON.stringify(before.slice(0, 140)));
      // 开局必须没有这一句：挂着的一句"没保存"跟没有这一句一样骗人（这一面刚读盘回来，全是干净的）。
      if (before !== '') throw new Error('什么都没改，这一句就先挂着（它该按"有没有脏"摆）：' + before);
      // 往**默认那一块第一枚打得进字的框**里塞一个哨兵词（不保存）。现在站在它上面的是
      // 「每次都要带的参数」—— 具体哪一格不重要，重要的是那一句说的是"默认那几项"。
      const typed = await ev('(() => {'
        + 'const inp=[...document.querySelectorAll(".ant-select input")].find((i)=>i.getBoundingClientRect().width);'
        + 'if (!inp) return { err: "没有可打的框" };'
        + 'inp.focus();'
        + 'const set=Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype,"value").set;'
        + 'set.call(inp,"r30-sentinel"); inp.dispatchEvent(new Event("input",{bubbles:true}));'
        + 'return { ok:true, now: inp.value }; })()');
      if (typed.err) throw new Error(String(typed.err));
      await send('Input.dispatchKeyEvent', { type: 'keyDown', code: 'Enter', key: 'Enter', windowsVirtualKeyCode: 13 });
      await send('Input.dispatchKeyEvent', { type: 'keyUp', code: 'Enter', key: 'Enter', windowsVirtualKeyCode: 13 });
      await sleep(800);
      const after = await said();
      log('UNSAVED 打进哨兵之后=' + JSON.stringify(after.slice(0, 180)));
      if (!after.includes('默认那几项')) throw new Error('改了默认那几项而这一句没说出是哪一处：' + after);
      if (!after.includes('1 处')) throw new Error('那句报的处数不对（只改了一格）：' + after);
      // 挂着的那一句要单独留一张：循环收尾那张拍的是**撤掉之后**（那一句已经跟着走了），
      // 两张各证一半 —— 只有一张"没了"的图，证不了那句曾经说得出是哪一处。
      await shot('settings-unsaved-said.png');
      // 把那个词撤掉：那一句该跟着走。留着它就变成"永远在提醒"，跟没有一样。
      const removed = await ev('(() => {'
        + 'const it=[...document.querySelectorAll(".ant-select-selection-item")]'
        + '.find((e)=>(e.textContent||"").includes("r30-sentinel"));'
        + 'if (!it) return { err: "找不到那个哨兵词" };'
        + 'const x=it.querySelector(".ant-select-selection-item-remove");'
        + 'if (!x) return { err: "那一格没有可删的那颗" };'
        + 'x.click(); return { ok: true }; })()');
      if (removed.err) throw new Error(String(removed.err));
      await sleep(700);
      const gone = await said();
      log('UNSAVED 撤掉之后=' + JSON.stringify(gone.slice(0, 140)));
      if (gone !== '') throw new Error('草稿撤回了而那一句还挂着：' + gone);
    } },
  // **必须排在所有要后台的步骤之后**：它把面板自己拉起的那个 rcd 杀掉来造一次真的列不出来，
  // 而面板不会中途重启后台 —— 它之后的每一步都会打在"没有后台"上（真窗口
  // 跑出来过：它当时排在中间，后面三条全报成失败，看着像界面坏了）。
  // 它后面只留两格，而且两格都不碰后台：写下"上次开在哪一面/哪一段"（纯点一下分段控件），
  // 以及重开之后量那一份回来（`needsReopen`，主跑那一遍里是 SKIP）。
  { id: 'list-failure-visible', what: '列不出来那一句看得见：toast 节点数 + 栏上常驻的 Alert（把面板自己拉起的 rcd 停掉）', run: async () => {
      await twoPanes();
      // 这一面没有命令行，交不出一个不存在的远端名（那正是它的设计）。所以失败由**面板自己拉起的
      // rcd 停掉**来造：只杀 rcd.pid 里那个 pid，走的是同一条 browser_list 失败通路。
      // 家从参数传给那个脚本：它曾把 pid 文件路径写死成 A，于是对 B 跑整套时它杀的是
      // **另一套家的** daemon（或干脆找不到文件），报出来是"停不掉后台"—— 而病因是路径不是进程。
      // 参数名是 `-FixtureHome` 不是 `-Home`：后者撞上 PowerShell 的只读自动变量。
      const ps = 'tools/ui-kill-rcd.ps1';
      log('KILL-RCD ' + execSync('powershell -NoProfile -ExecutionPolicy Bypass -File "' + ps
        + '" -FixtureHome ' + HOME, { encoding: 'utf8' }).trim());
      await sleep(700);
      // 此刻这一栏站在某一层里（上一步刚进过 a）：上一级那一下重新列的是父层，而 rcd 已经不在了。
      await clickIfEnabled('上一级', 1); await sleep(2400);
    } },
  { id: 'remember-what-was-open', what: '写下"上次开在哪一面、右列哪一段"（重开那一格的前置；这一格不碰后台）', run: async () => {
      // 先站到对子那一面把右列翻到「日志」（那一段住在对子那一栏里，开在设置时它是隐藏的），
      // 再把这一面切到设置 —— 两件事各留一个"非默认"的值，重开之后才分得清是回来了还是碰巧。
      await click('配好的位置', 'any'); await sleep(1200);
      await clickRailTab('日志');
      await click('设置', 'any'); await sleep(1600);
      const face = String(await ev('(() => { const it=[...document.querySelectorAll(".ant-segmented-item")]'
        + '.find((e)=>/配好的位置|浏览文件|设置/.test(e.textContent||"") && e.className.includes("selected"));'
        + 'return it?it.textContent.trim():"没有那一面被选中"; })()'));
      if (!face.includes('设置')) throw new Error('切不到设置那一面，下一格就没得量：' + face);
      log('REMEMBER-SET 写下的是 面="' + face + '"');
    } },
  { id: 'restart-keeps-face-and-tab', needsReopen: true,
    what: '重开之后回到上次那一面与右列那一段，而模式那一格照旧从最轻的一档开始', run: async () => {
      // 顶栏那一条也有 Segmented（自动/浅色/深色），所以"哪一面"要按**这一面自己的三档**认，
      // 不能拿屏上第一个选中项 —— 那一拿就拿到主题（这一格第一遍就是这么红的）。
      const face = String(await ev('(() => { const it=[...document.querySelectorAll(".ant-segmented-item")]'
        + '.find((e)=>/配好的位置|浏览文件|设置/.test(e.textContent||"") && e.className.includes("selected"));'
        + 'return it?it.textContent.trim():"没有那一面被选中"; })()'));
      const tab = String(await ev('(() => { const c=document.querySelector(".ant-layout-content");'
        + 'const s=c?[...c.querySelectorAll(".ant-segmented-item")].find((e)=>e.className.includes("selected")):null;'
        + 'return s?s.textContent.trim():"读不到"; })()'));
      // 「要做什么」那一格在**停着的那一面**里（这一面开在设置），读 DOM 文字而不是宽度；
      // aria-label 挂在里面那枚 input 上（antd 的形状），所以两处都认一遍。
      // 2026-10-08：那一面不再是 `.ant-layout-sider` 的后代，选择条横跨整窗（见 `SEL_PAIR_BAR`）。
      // 2026-10-09：那一枚的 `aria-label` 从「模式」变成了「要做什么」，而这里**不能**直接用
      // `selectValue` —— 它按"看得见的"筛，而这一面此刻是 `display:none`（`selectValue` 会报
      // "找不到那一枚"，读着像选择条没挂上来）。所以这里照旧不过滤宽度，只换认的那一枚。
      const mode = String(await ev(`(() => {
        const all = [...document.querySelectorAll(${JSON.stringify(SEL_PAIR_BAR)})];
        const s = all.find((e) => (e.getAttribute('aria-label')
          || (e.querySelector('input') || {}).getAttribute?.('aria-label') || '') === '要做什么');
        if (!s) return '找不到那枚框（选择条上现有：'
          + all.map((e) => e.getAttribute('aria-label') || '?').join('/') + '）';
        // v6 里显示值住在 \`.ant-select-content\`（\`.ant-select-selection-item\` 是 v5 的类名）——
        // 与 \`selectValue\` 同一认法。只认一种就会读回"框里没有显示值"那句假红。
        const i = s.querySelector('.ant-select-content') || s.querySelector('.ant-select-selection-item');
        return i ? i.textContent.trim() : '框里没有显示值';
      })()`));
      log('REMEMBER 面="' + face + '" 段="' + tab + '" 模式="' + mode + '"');
      if (!face.includes('设置')) throw new Error('重开没回到上次那一面（开在="' + face + '"）');
      if (tab !== '日志') throw new Error('重开没回到右列上次那一段（停在="' + tab + '"）');
      if (!mode.includes('只补，不删')) throw new Error('模式那格跨重开记住了（那条安全规则被顺手扩到偏好层）：' + mode);
      await shot('restart-keeps-face-and-tab.png');
    } },
];

const from = Number(arg('from', 0));
const to = Number(arg('to', STEPS.length - 1));
let failed = 0;
let readoutFailed = 0;
for (let i = from; i <= to && i < STEPS.length; i++) {
  const st = STEPS[i];
  // 有的步骤只属于一套家（A 带排除规则 ⇒ 那句常驻解释才有；B 不带 ⇒ 「清空」点得动）。
  // 以前靠"记得传对 --to"来分，忘了就把另一档的场景报成 FAILED。现在写在各步自己身上：
  // 不对家的记 SKIP —— SKIP 也是一条读数，别让它悄悄消失。
  if (st.only && st.only !== HOME) { log(`STEP ${st.id} SKIP :: 只属 ${st.only} 那一套家（本次 HOME=${HOME}）`); continue; }
  // 有的步骤要**重开面板**才成立（不热重载这件事唯一的真证据：重启之后卡上才换成新值）。
  // 用 --reopen=1 单独跑那一格，别让它混在正常那一段里被"没重开"的窗口骗过 ——
  // 那种混跑会绿在错的理由上：旧窗口里读到旧值，恰好和"没重开"的预期一样。
  if (st.needsReopen && arg('reopen', '') !== '1') {
    log(`STEP ${st.id} SKIP :: 要重开面板（这一遍没带 --reopen=1）`); continue;
  }
  try {
    await mouseAway();
    // 每一格都从**干净的屏**开始量：上一层留下的层会盖住这一格的第一次点击。
    // 以前只对设置面那几格做（那一段红过），浏览器那一段留了一层没合，于是
    // 「一次撞名拒绝」这一格其实一直在读上一格留下的那一条 toast —— 它绿着，但绿在错的理由上。
    // 故意接上一层的那几格自己声明 keepsLayer。
    if (!st.keepsLayer) await closeStrayLayers(st.id);
    await st.run();
    await sleep(600);
    await shot(st.id + '.png');
    log(`STEP ${st.id} ok :: ${st.what}`);
  } catch (e) {
    // 栈也要进读数：只记 `String(e)` 的那一版，28 条步骤全报同一句 ReferenceError，
    // 却没人知道它是页面里哪一行抛的 —— 判定要能自己指到地方。
    const stack = String((e && e.stack) || e).split('\n').slice(1, 5).join(' | ');
    log(`STEP ${st.id} FAILED :: ${st.what} :: ${String(e).slice(0, 200)} :: ${stack}`);
    failed++;
    try { await shot(st.id + '-failed.png'); } catch (_) { /* 连屏都拍不到就算了 */ }
    continue;
  }
  // 读数与判定是两件事：上面那一条 `ok` 说的只是"这一步的断言过了"。
  // 以前 facts() 也在同一个 try 里，它一抛就把已经过了的一步也报成 FAILED ——
  // 于是一份"34 ok + 34 FAILED"的读数其实只有一处坏，而坏的地方在尺子上，不在界面上。
  try {
    await facts(st.id);
  } catch (e) {
    log(`FACTS-FAIL ${st.id} :: 断言过了，但这一步的读数没取到 :: ${String(e).slice(0, 160)}`);
    readoutFailed++;
  }
}
fs.writeFileSync(LOG, lines.join('\n') + '\n', 'utf8');
ws.close();
// 退出码要认得出这一遍红没红。以前它恒为 0（只打印"ran N steps"），于是"34 步全 FAILED"
// 那一遍在终端上看是绿的 —— 后台任务的 exit code 就此不再是证据。
const skipped = lines.filter((l) => l.includes(' SKIP ::')).length;
console.log(`ran ${to - from + 1} steps: 断言过 ${to - from + 1 - failed - skipped}, FAILED ${failed}, SKIP ${skipped}, 读数没取到 ${readoutFailed}`);
if (failed > 0) process.exitCode = 1;
