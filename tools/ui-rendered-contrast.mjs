// 深色交付的那一半判据：antd 暗色算法**实际渲染出来**的色值，拿来算 WCAG 比值。
//
// 为什么不看色板：`palette.dark` 那一份由 `tools/ui-contrast.mjs` 算过了，全部达标。
// 没量过的是运行时 —— antd 的 darkAlgorithm 会派生出一批不在色板里的颜色
// （次要文字、描边、浮层底、被派生成文字的主色那一档最容易掉）。那种色只能从
// 真窗口的 getComputedStyle 里取，所以这里连的是面板自己的 CDP。
//
// 做法：对每个取样点，取元素自己的 color，再往上找**第一个不透明**的 background-color
// 当底 —— 半透明的层（选中底、遮罩）不能当分母，否则算出来的是假达标。
//   node .superpowers/scratch/dark-contrast.mjs > docs/superpowers/evidence/ui/dark-contrast.md
import { cdpEndpoint } from './cdp-endpoint.mjs';

const ENDPOINT = cdpEndpoint();

// 界面整修 / 设置面那两档主题的实际渲染对比度。
//   node tools/ui-rendered-contrast.mjs --view=设置 --theme=深色 > docs/superpowers/evidence/ui/contrast-settings-dark.md
// 取样点分两族：下面那份表**每一格都要求在某一屏里被量到**（"这一屏没有这个元素"是没量到，
// 不是达标）。跑的人按 --view 换屏，两档主题各一遍。
//   --layer=保存|删这条  把设置面那一层的差异表与那两颗按钮也量一遍（层的形成按下面 fillLayer 那段）。
//   --mode=会删除        对子那一面先站到那一档上再量（危险程度那枚暖色只在它站着的那一格出现）。
const arg = (name, dflt) => {
  const hit = process.argv.find((a) => a.startsWith('--' + name + '='));
  return hit ? hit.split('=').slice(1).join('=') : dflt;
};
const VIEW = arg('view', '');
const THEME = arg('theme', '');
const LAYER = arg('layer', '');
// 「有危险程度的那一档」不会自己站在屏上：对子那一面默认停在「只放上去」，那枚暖色就不出现。
// `--mode=会删除` 把它逼出来（只改 React 的 state，一个字节不落盘）。
const MODE = arg('mode', '');

// 取样点：选择器 + 这一格在界面上是干什么的 + 按哪一档要求判（AA 正文 4.5 / 大字与非文字 3）。
const SPOTS = [
  ['body', '窗口底上的正文', 4.5],
  ['.surface', '卡片那一层上的正文', 4.5],
  ['.ant-typography-secondary', '次要说明句', 4.5],
  ['.fs-caption', '短注 / 时间 / 标签（最小的那一档）', 4.5],
  ['.ant-btn-primary', '主按钮上的字', 4.5],
  // 确认层里那两颗：一颗是普通主按钮（改配置），一颗在会拿走东西的那一判里变红（danger）。
  // 它们只在层开着时存在（`--layer=` 那一遍），普通三屏的报告里这两格就是"这一屏没有"。
  ['.ant-btn-primary.ant-btn-dangerous', '确认层里"会拿走东西"那一颗（红底白字）', 4.5],
  ['.ant-modal .ant-btn-default', '确认层里"取消"那一颗', 4.5],
  ['.ant-modal-title', '确认层那句标题（说的是哪几行）', 4.5],
  ['.ant-segmented-item-selected', '被选中的那一段', 4.5],
  ['.ant-table-thead .ant-table-cell', '表头', 4.5],
  ['.ant-table-tbody tr:not(.ant-table-placeholder) .ant-table-cell', '表格正文（有数据那一行）', 4.5],
  ['.ant-table-placeholder', '表格空着那一句（它也在传递信息）', 4.5],
  // 第四个字段是伪类：`:hover` 那份是鼠标压在按钮上时**真会显示**的颜色，只判静止态会放过一档常见失败。
  ['.ant-btn-text[disabled], .ant-btn[disabled]', '禁用按钮（WCAG 1.4.3 豁免不活动的控件：参考值，不判过不过）', 0],
  // `.ant-typography-warning` 这一档以前在这里：全仓扫一遍没有任何一处用 `Typography type="warning"`
  //（警示走的是 `Alert type="warning"`，那是下面 `@alerts` 量的），留着一个永远"这一屏没有"的取样点
  // 等于给报告里掺一格没人读的"没量到"。
  ['@tags', '分组标题那颗标签（三颗各量）', 4.5],
  ['.ant-segmented', '分段控件的底', 3],
  // ---- 设置面那一屏新增的几族 ----
  // 这一面是"字段 + 说明句 + 差异表"那一族排版，上面那份表一条都没盖到：
  // 输入框里的值与操作员自己写的那些字，是最容易只判色板、不判组件的三格。
  ['.ant-card-head-title', '卡片标题（那三块的名称）', 4.5],
  ['.ant-input', '输入框里那串值（路径与地址都住在这里）', 4.5],
  // 提示语是 `::placeholder` 那一族：它比正文淡一档，是最容易"色板过了、组件没过"的一格。
  // 伪元素选择器不能进 querySelector，所以第四个字段才是伪类（与主按钮 hover 同一个通路）。
  ['.ant-input', '输入框空着时那句提示', 4.5, '::placeholder'],
  ['.fs-code', '代码体的路径与键名', 4.5],
  // 层级路径上那几段也是"主色当文字用"的一族。把"只量这一屏看得见的那一格"这条补上之后
  // 才发现这一族一直没被读过 —— 上一版读到的都是别的屏藏在那儿的那一格。
  ['.ant-breadcrumb a', '层级路径上那一段可点的名字', 4.5],
  // 四档语义底各量一遍：`.ant-alert-description` 那种"取第一个"的写法只量到屏上恰好排在最前
  // 的那一档（做完了），另外三档的底与字色没进过账 —— 而这一族偏偏是"色板过了、组件没过"的老位置。
  ['@alerts', '语义底那一格（做完了 / 没改成 / 警示 / 信息，各量标题与正文）', 4.5],
  // v6 把显示值那枚类名从 `.ant-select-selection-item` 改成了 `.ant-select-content`
  // （2026-10-11 真窗口量的：旧名 0 枚、新名 9 枚）。旧名留在这里的话这一格永远报
  // "这一屏没有这个元素"，而那份报告整体是绿的 —— 静默失明的取样点比没有取样点更坏。
  ['.ant-select-content', 'tags 那一族里的一枚（flags 与 exclude）', 4.5],
  // 「打开这个目录」那一颗与「保存这几项」是同一族：**普通**那一档按钮（既不是主色也不是禁用），
  // 它吃掉的是 antd 从控件底派生出来的字色 —— 上面那份表里 primary 与 disabled 各有一格，
  // 静止态的普通按钮从来没被量过。
  ['.ant-btn-default:not([disabled])', '普通那一颗按钮上的字（静止态）', 4.5],
  ['.ant-listy-item', '任务那一块的每一行（antd v6 的 Listy，行类名带 y）', 4.5],
];

const HARVEST = ((spots) => {
  const rgbaOf = (c) => {
    const m = c.match(/rgba?\(([^)]+)\)/);
    if (!m) return null;
    const p = m[1].split(',').map((s) => parseFloat(s.trim()));
    return [p[0], p[1], p[2], p.length > 3 ? p[3] : 1];
  };
  const opaque = (c) => {
    const v = rgbaOf(c);
    if (!v) return c !== 'transparent';
    return v[3] > 0.99;
  };
  // 底要**把整条半透明链合成完**再交出去。上一版只取"第一个不透明的祖先"，
  // 于是选中行/悬试行那一层 `colorFillAlter`（半透明青）根本不参与 ——
  // 前景的 alpha 早就知道要先合成，背景的 alpha 却漏了，同一族谎。
  const bgOf = (el) => {
    const chain = [];
    for (let n = el; n; n = n.parentElement) {
      const v = rgbaOf(getComputedStyle(n).backgroundColor);
      if (!v || v[3] === 0) continue;
      chain.push(v);
      if (v[3] > 0.99) break;
    }
    let acc = [255, 255, 255];
    for (let i = chain.length - 1; i >= 0; i--) {
      const [r, g, b, a] = chain[i];
      acc = [r * a + acc[0] * (1 - a), g * a + acc[1] * (1 - a), b * a + acc[2] * (1 - a)];
    }
    return 'rgb(' + acc.map((v) => Math.round(v)).join(', ') + ')';
  };
  // 不过的那一格要能自己说清"这一层底是谁给的"：把参与合成的那条链原样带出去。
  // 没有这一列，报告里就只剩一个算出来的数，读的人得回代码里一层层找是谁涂的。
  const bgChainOf = (el) => {
    const names = [];
    for (let n = el; n; n = n.parentElement) {
      const c = getComputedStyle(n).backgroundColor;
      const v = rgbaOf(c);
      if (v && v[3] === 0) continue;
      const cls = (n.className && String(n.className).trim().split(/\s+/)[0]) || n.tagName.toLowerCase();
      names.push(cls + ':' + c);
      if (v && v[3] > 0.99) break;
    }
    return names.join(' < ');
  };
  const hasOwnText = (el) => [...el.childNodes].some((n) => n.nodeType === 3 && n.textContent.trim() !== '');
  function firstTextLeaf(el) {
    // 容器的 computed color **不是**人眼读到的那行字的颜色。上一版就是取了
    // `.ant-table-placeholder` 这个 td 的 color（ink4，3.30），报了个假失败 ——
    // 真正显示的那句 `.ant-empty-description` 是 ink3，5.00，本来就过。
    const stack = [el];
    while (stack.length) {
      const n = stack.shift();
      if (hasOwnText(n)) return n;
      stack.push(...n.children);
    }
    return el;
  }
  const textLeaf = (el) => firstTextLeaf(el);

  return spots.flatMap(([sel, what, want, pseudo]) => {
    // `@tags`：界面上有几颗分组标签就量几颗。颜色信号是设计里故意的（会删那一族排最前 + 唯一暖色），
    // 所以不能靠"第一颗"外推，三颗各有各的预设色。
    if (sel === '@tags') {
      const seen = new Map();
      document.querySelectorAll('.ant-tag').forEach((t) => {
        const cs = getComputedStyle(t);
        const key = cs.color + '|' + bgOf(t);
        if (!seen.has(key)) seen.set(key, { label: (t.textContent || '').trim().slice(0, 16) });
      });
      return [...seen.entries()].map(([key, v]) => {
        const [fg, bg] = key.split('|');
        return { label: '@tags ' + v.label, what: what + '「' + v.label + '」', want, fg, bg, fontSize: 12, weight: 400 };
      });
    }
    // `@alerts`：四档语义底各量两格（标题与正文）。隐藏的那些也收：面板三面一直挂着，
    // 另一面的那一档此刻只是 display:none，它的**色**与露出来时是同一份规则。
    if (sel === '@alerts') {
      const seen = new Set();
      const rows2 = [];
      document.querySelectorAll('.ant-alert').forEach((a) => {
        const variant = (a.className.match(/ant-alert-(success|error|warning|info)/) || [])[1] || '?';
        for (const [role, raw] of [['标题', a.querySelector('.ant-alert-title')],
                                   ['正文', a.querySelector('.ant-alert-description')]]) {
          if (!raw) continue;
          const leaf = textLeaf(raw);
          const cs = getComputedStyle(leaf);
          const bg = bgOf(leaf);
          const key = variant + role + cs.color + bg + cs.fontSize;
          if (seen.has(key)) continue;
          seen.add(key);
          rows2.push({ label: '@alerts ' + variant + ' ' + role,
            what: what + '「' + variant + '」那一档的' + role, want,
            fg: cs.color, bg, fontSize: parseFloat(cs.fontSize), weight: Number(cs.fontWeight) });
        }
      });
      return rows2;
    }
    // **只量这一屏看得见的那一格。** 三面面板一直挂在 DOM 里，换屏只是 display:none：
    // 上一版用 `querySelector(sel)` 拿第一枚匹配，于是"屏=设置"那份报告里
    // 「表格正文（有数据那一行）」读到的其实是浏览器那一面被藏起来的表 —— 18 格全是 `w:0 h:0`，
    // 报告却写着"过"。同一个类名在隐藏时算出来的色确实一样，但这句话报的是"这一屏量到了"，
    // 而这一屏根本没有那格字给人看；拿藏起来的格子冒充量过，与假达标同一种坏。
    const onScreen = (e) => {
      if (!e.getClientRects().length || !e.getBoundingClientRect().width) return false;
      // 层开着的时候，层**外面**那些字被半透明遮罩压住：它不是"这一屏看得见的那一格"，
      // 而遮罩是兄弟节点不是祖先，背景链合成也追不到它。所以那一遍只量层里的那些格。
      const wrap = [...document.querySelectorAll('.ant-modal-wrap')]
        .find((w) => w.getClientRects().length && getComputedStyle(w).visibility !== 'hidden');
      return !wrap || wrap.contains(e);
    };
    const hits = [...document.querySelectorAll(sel)].filter(onScreen);
    if (!hits.length) return [{ label: sel, what, want, missing: true }];
    // 同一屏上这一族往往有几十格，**只取第一格会把坏的那几格藏起来**（`.fs-caption` 在浏览器
    // 那一面就有两种底：一种在卡片上、一种在被选中的那一段旁边）。所以全量、按色分组，
    // 每组带着"这一组有几格、量的是哪一句"出去 —— 一句都藏不住，读的人也能核对取的是哪一片。
    const groups = new Map();
    for (const raw of hits) {
      const el = textLeaf(raw);
      const cs = getComputedStyle(el, pseudo || null);
      let fg = cs.color;
      // 图标/容器自己常常 color:rgba(0,0,0,0) —— 那不是要读的文字，往下找一个真有字的后代。
      if (/rgba\([^)]*,\s*0\)/.test(fg)) {
        for (const kid of el.querySelectorAll('*')) {
          const k = getComputedStyle(kid).color;
          if (!/rgba\([^)]*,\s*0\)/.test(k)) { fg = k; break; }
        }
      }
      const bg = bgOf(el);
      const key = fg + '|' + bg + '|' + cs.fontSize + '|' + cs.fontWeight;
      const text = pseudo ? '(伪元素 ' + pseudo + ')' : (el.textContent || '').replace(/\s+/g, ' ').trim().slice(0, 24);
      const g = groups.get(key);
      if (g) { g.count++; continue; }
      groups.set(key, { fg, bg, fontSize: parseFloat(cs.fontSize), weight: Number(cs.fontWeight),
        count: 1, text: text || '(空)', chain: bgChainOf(el) });
    }
    return [...groups.values()].map((g) => ({
      label: sel + (pseudo ? ' ' + pseudo : ''), what, want, ...g,
    }));
  });
});

async function main() {
  const targets = await (await fetch(ENDPOINT)).json();
  const page = targets.find((t) => t.type === 'page' && !t.url.startsWith('devtools://'));
  if (!page) throw new Error('没有可驱动的页面');
  const ws = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((ok, bad) => { ws.onopen = ok; ws.onerror = bad; });
  const send = (method, params) => new Promise((resolve, reject) => {
    const id = Math.floor(Math.random() * 1e9);
    const onMsg = (ev) => {
      const msg = JSON.parse(ev.data);
      if (msg.id !== id) return;
      ws.removeEventListener('message', onMsg);
      msg.error ? reject(new Error(JSON.stringify(msg.error))) : resolve(msg.result);
    };
    ws.addEventListener('message', onMsg);
    ws.send(JSON.stringify({ id, method, params }));
  });
  const r0 = await send('Runtime.evaluate', { expression: '1+1', returnByValue: true });
  if (r0.result.value !== 2) throw new Error('CDP 通道没通');

  // 先按 --theme / --view 站到要量的那一屏上：量的必须是**那一面自己的**元素，
  // 而不是"面板恰好开在哪一屏"。切换走可信鼠标事件（合成 click 在 antd 的 Segmented 上
  // 也能通，但那一档已经不是这把尺子要证明的东西了）。
  const clickAt = async (x, y) => {
    await send('Input.dispatchMouseEvent', { type: 'mouseMoved', x, y });
    await send('Input.dispatchMouseEvent', { type: 'mousePressed', x, y, button: 'left', clickCount: 1 });
    await send('Input.dispatchMouseEvent', { type: 'mouseReleased', x, y, button: 'left', clickCount: 1 });
  };
  // 按屏上那行字找到按钮再点它（可信事件）。找不到就抛，别往下量一份"层开着"的假读数。
  const clickVisible = async (text) => {
    const got = await send('Runtime.evaluate', { expression: `(() => {
      const norm = (e) => (e.textContent || '').replace(/\\s+/g, '');
      const want = ${JSON.stringify(text)}.replace(/\\s+/g, '');
      const pool = [...document.querySelectorAll('.ant-modal .ant-btn, .ant-btn')]
        .filter((e) => e.getClientRects().length && e.getBoundingClientRect().width)
        .filter((e) => !e.disabled)
        .filter((e) => norm(e).includes(want));
      if (!pool.length) return { err: '没有那颗按钮：' + want + '，屏上按钮='
        + [...document.querySelectorAll('.ant-btn')].map((e) => norm(e)).filter(Boolean).slice(0, 12).join(',') };
      const b = pool[0];
      b.scrollIntoView({ block: 'center' });
      const q = b.getBoundingClientRect();
      return { x: q.x + q.width / 2, y: q.y + q.height / 2 };
    })()`, returnByValue: true });
    const pt = got.result.value;
    if (!pt || pt.err) throw new Error('点不到「' + text + '」 :: ' + JSON.stringify(pt));
    await clickAt(pt.x, pt.y);
  };
  const clickSeg = async (want, settle) => {
    const js = `(() => {
      const norm = (e) => (e.textContent || '').replace(/\\s+/g, '');
      const it = [...document.querySelectorAll('.ant-segmented-item')]
        .find((e) => norm(e).includes(${JSON.stringify(want)}));
      if (!it) return { err: '没有那一段：' + ${JSON.stringify(want)} + ' 全部='
        + [...document.querySelectorAll('.ant-segmented-item')].map((e) => norm(e)).join(',') };
      // 先把它滚回视野再量坐标：上一版没滚，前一步骤留下的滚动位置让顶栏那一段量到 **负 y**
      //（y=-439），CDP 往那个坐标发的事件谁都不接，于是"浅色/深色"两份报告一模一样还都写着过 ——
      // 那种绿比红坏，它谎报的是"两档都量过了"。
      it.scrollIntoView({ block: 'center' });
      const b = it.getBoundingClientRect();
      return { x: b.x + b.width / 2, y: b.y + b.height / 2, on: it.className.includes('selected'), w: b.width };
    })()`;
    const got = await send('Runtime.evaluate', { expression: js, returnByValue: true });
    const pt = got.result.value;
    if (!pt || pt.err) throw new Error('切屏失败 :: ' + JSON.stringify(pt));
    if (pt.on) return;                       // 已经站在这一段上，别重复点（点了也不动，白等）
    await clickAt(pt.x, pt.y);
    await new Promise((ok) => setTimeout(ok, settle));
  };
  const pickOption = async (which, want) => {
    // 开一开顶部那一行第 `which` 枚下拉、点中写着 `want` 的那一格。量色要的是**那一档真的在屏上**，
    // 不是代码里写了那一档 —— 与 `--layer=` 同一资格：把要量的那一格逼出来，别的什么都不动。
    const box = await send('Runtime.evaluate', { expression: `(() => {
      const all = [...document.querySelectorAll('.ant-layout-sider .ant-select')]
        .filter((e) => e.getBoundingClientRect().width);
      const s = all[${which}];
      if (!s) return { err: '左栏没有第 ' + (${which} + 1) + ' 枚框（看得见 ' + all.length + ' 枚）' };
      s.scrollIntoView({ block: 'center' });
      const b = s.getBoundingClientRect();
      return { x: b.x + b.width / 2, y: b.y + b.height / 2 };
    })()`, returnByValue: true });
    const p = box.result.value;
    if (!p || p.err) throw new Error('开不了那一枚下拉 :: ' + JSON.stringify(box.result));
    await clickAt(p.x, p.y);
    await new Promise((ok) => setTimeout(ok, 700));
    const opt = await send('Runtime.evaluate', { expression: `(() => {
      const it = [...document.querySelectorAll('.ant-select-dropdown:not(.ant-select-dropdown-hidden) .ant-select-item')]
        .find((e) => (e.textContent || '').includes(${JSON.stringify(want)}));
      if (!it) return { err: '框里没有写着「' + ${JSON.stringify(want)} + '」的那一格' };
      const b = it.getBoundingClientRect();
      return { x: b.x + b.width / 2, y: b.y + b.height / 2 };
    })()`, returnByValue: true });
    const q = opt.result.value;
    if (!q || q.err) throw new Error('点不到那一格选项 :: ' + JSON.stringify(opt.result));
    await clickAt(q.x, q.y);
    await new Promise((ok) => setTimeout(ok, 900));
  };
  if (THEME) await clickSeg(THEME, 900);
  if (VIEW) await clickSeg(VIEW, VIEW === '设置' ? 2600 : 1200);
  // 「有数据那一行」不会自己长出来。上一版六份报告里那一格全写着"这一屏没有这个元素"，
  // 而报告读起来像"这一档没人管"——其实只是没人把它逼出来：
  // 设置面那张差异表要有人改一格才出行，浏览器那一面的表要挑一个位置才列得出东西。
  // 这里量的只是**渲染出来的色**，改动全留在 React 的 state 里：不点保存，一个字节都不落盘
  //（写侧的验证在 `tools/ui-acceptance.mjs` 那边，而且那一侧只打 %TEMP% 夹具）。
  let dropdown = null;   // 浏览器那一屏在下拉开着时量到的那一枚候选（见 fillTable 里那段）
  const fillTable = async () => {
    // 对子那一面先站到「对账」那一段：那颗主按钮（`GatePanel.tsx` 的 `type="primary"`）住在这一段里，
    // 默认站的「进度」上没有它 —— 上一版"量到了"的那格主按钮，其实是别的屏上被藏着的那一枚。
    if (VIEW === '配好的位置') {
      // 危险程度那枚暖色住在「要做什么」那一格的标签上（顶部那一行第 2 枚）：默认站在
      // 「只补，不删」时它是中性色，量到的就不是那一档。
      if (MODE) await pickOption(1, MODE);
      await clickSeg('对账', 1200); return;
    }
    // 设置面那张差异表**不在这里逼**：它住在确认层里，要按「保存」才出，而这一把尺子的
    // 全部资格是"只看不写"——按下会引出写入流程的按钮，不该由量色的脚本代按。
    // 那一行的色与浏览器/对子那两面的表是同一族组件、同一份令牌，量在浏览器那一屏即算盖到。
    if (VIEW !== '浏览文件') return;
    const box = await send('Runtime.evaluate', { expression: `(() => {
      const s = [...document.querySelectorAll('.ant-select')]
        .find((e) => e.getBoundingClientRect().width);
      if (!s) return { err: '这一屏没有可选的位置' };
      s.scrollIntoView({ block: 'center' });
      const b = s.getBoundingClientRect();
      return { x: b.x + b.width / 2, y: b.y + b.height / 2 };
    })()`, returnByValue: true });
    const p = box.result.value;
    if (!p || p.err) throw new Error('浏览器那一面的位置框找不到 :: ' + JSON.stringify(box.result));
    await clickAt(p.x, p.y);
    await new Promise((ok) => setTimeout(ok, 800));
    // 下拉**开着的那一瞬间**量一枚候选：它的底是 `colorBgElevated` 那一族（模态、抽屉、气泡卡片同一枚种子），
    // 而那族只在点开时看得见 —— 挑完选项就关上了，事后再量等于没量。
    dropdown = (await send('Runtime.evaluate', { expression: `(() => {
      const dd = [...document.querySelectorAll('.ant-select-dropdown')]
        .find((e) => e.getClientRects().length && e.getBoundingClientRect().width);
      const o = dd && [...dd.querySelectorAll('.ant-select-item-option')]
        .find((e) => e.getBoundingClientRect().width);
      if (!o) return { err: '下拉没开或没有候选可读' };
      const cs = getComputedStyle(o);
      let bg = '';
      for (let n = o; n; n = n.parentElement) {
        const m = (getComputedStyle(n).backgroundColor || '').match(/rgba?\\(([^)]+)\\)/);
        if (!m) continue;
        const parts = m[1].split(',').map((s) => parseFloat(s.trim()));
        if ((parts.length > 3 ? parts[3] : 1) > 0.99) { bg = 'rgb(' + parts.slice(0, 3).map((v) => Math.round(v)).join(', ') + ')'; break; }
      }
      return { fg: cs.color, bg, fontSize: parseFloat(cs.fontSize), text: (o.textContent || '').trim().slice(0, 24) };
    })()`, returnByValue: true })).result.value;
    const opt = await send('Runtime.evaluate', { expression: `(() => {
      const o = [...document.querySelectorAll('.ant-select-item-option')]
        .filter((e) => e.getBoundingClientRect().width);
      if (!o.length) return { err: '下拉没开或这一栏没有候选' };
      // 带反斜杠的那一枚是本地目录（它不依赖后台在线）；都没有就拿第一枚。
      const pick = o.find((e) => (e.textContent || '').includes('\\\\')) || o[0];
      pick.scrollIntoView({ block: 'center' });
      const b = pick.getBoundingClientRect();
      return { x: b.x + b.width / 2, y: b.y + b.height / 2, text: (pick.textContent || '').slice(0, 50) };
    })()`, returnByValue: true });
    const q = opt.result.value;
    if (!q || q.err) throw new Error('浏览器那一面挑不出位置 :: ' + JSON.stringify(opt.result));
    await clickAt(q.x, q.y);
  };
  await fillTable();
  // 确认层里那几格以前没人量过：那张差异表、那颗「确认改这几行」/「确认删」的按钮，
  // 要按「保存」或「删这条」才出现。按下去的是**开层**那一下 —— 写盘要的是层里另一颗「确认」，
  // 而"取消不动字节"是 `tools/ui-acceptance.mjs` 里 `settings-cancel-keeps-bytes` 钉过的；
  // 量完这一遍立刻按「取消」合上。
  const openLayer = async () => {
    if (LAYER === '保存') {
      const r = await send('Runtime.evaluate', { expression: `(() => {
        const box = [...document.querySelectorAll('.ant-space-vertical')]
          .filter((s) => s.getBoundingClientRect().width)
          .find((s) => (s.textContent || '').replace(/\\s+/g, '').startsWith('服务地址')
            && s.querySelector('input.ant-input'));
        if (!box) return { err: '没有服务地址那一格' };
        const i = box.querySelector('input.ant-input');
        const set = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set;
        set.call(i, 'https://contrast-probe.invalid');
        i.dispatchEvent(new Event('input', { bubbles: true }));
        return { read: i.value };
      })()`, returnByValue: true });
      const v = r.result.value;
      if (!v || v.err) throw new Error('改不出差异 :: ' + JSON.stringify(r.result));
      await new Promise((ok) => setTimeout(ok, 500));
      await clickVisible('保存这几项');
    } else if (LAYER === '删这条') {
      await clickVisible('删这条');
    } else {
      throw new Error('--layer 只认 保存 / 删这条');
    }
    await new Promise((ok) => setTimeout(ok, 1100));
    const open = await send('Runtime.evaluate', {
      expression: '(() => [...document.querySelectorAll(".ant-modal-wrap")]'
        + '.some((w) => w.getClientRects().length && getComputedStyle(w).visibility !== "hidden") ? 1 : 0)()',
      returnByValue: true,
    });
    if (open.result.value !== 1) throw new Error('按了「' + LAYER + '」层没开，这一份数不能交');
  };
  if (LAYER) await openLayer();
  // 表格那一格要等**列出数据**才量得到：两栏的列表是挂载之后才发的，1.2 秒不够就会得到
  // "这一屏没有这个元素"，而那份报告读起来像"这一档没人管"——其实只是没等。
  // 等的必须是**这一屏看得见**的行：`querySelectorAll` 数的是 DOM，另外两面的表也挂在里面，
  // 上一版因此"等到了行"（其实等到的是别的面的陈行），本面那条列表还没回来就去量 ——
  // 于是浏览器那一屏报出 15 格量到、12 格没量到，而紧接着热一遍重跑同一屏是 29 格。
  const countRows = async () => (await send('Runtime.evaluate', {
    expression: '(() => { const vis=(e)=>e.getClientRects().length&&e.getBoundingClientRect().width;'
      + 'const wrap=[...document.querySelectorAll(".ant-modal-wrap")]'
      + '.find((x)=>x.getClientRects().length&&getComputedStyle(x).visibility!=="hidden");'
      + 'const cells=[...document.querySelectorAll(".ant-table-tbody tr:not(.ant-table-placeholder) .ant-table-cell")].filter(vis);'
      + 'return (wrap?cells.filter((e)=>wrap.contains(e)):cells).length; })()',
    returnByValue: true,
  })).result.value;
  const waitRows = async (rounds) => {
    for (let i = 0; i < rounds; i++) {
      if (await countRows() > 0) return true;
      await new Promise((ok) => setTimeout(ok, 500));
    }
    return false;
  };
  if (!(await waitRows(24))) {
    // 冷启动那一档：面板自己拉起的后台（rcd）第一次列远程位置要等它起来，12 秒不够就整屏读成"没量到"
    //（实测过：同一份产物、同一屏，一遍 15 格量到、紧接着重跑 29 格）。
    // 所以再挑一次位置、再等一轮；还不行就照实让报告说"这一屏没量到"。
    if (VIEW === '浏览文件') {
      console.error('RETRY 第一轮等到 12 秒还没列出数据，再挑一次位置接着等');
      await fillTable();
      await waitRows(32);
    }
  }
  // 主题真切过去了没有：拿窗口底那一层的亮度判。
  // 上一版就是栽在这里——顶栏在视口外，事件发到了一个负 y 的坐标上，于是"深色"那份报告
  // 其实是浅色的数（两档全过），而那份绿看着像量过两遍。
  const canvas = await send('Runtime.evaluate', {
    expression: '(() => { const c = getComputedStyle(document.body).backgroundColor;'
      + 'const m = c.match(/\\d+/g); if (!m) return { err: "读不到底色：" + c };'
      + 'return { rgb: c, lum: (0.2126*m[0] + 0.7152*m[1] + 0.0722*m[2]) / 255 }; })()',
    returnByValue: true,
  });
  const cv = canvas.result.value;
  if (!cv || cv.err) throw new Error('底色读不到，主题那一档没法判 :: ' + JSON.stringify(cv));
  if (THEME === '深色' && cv.lum > 0.5) {
    throw new Error(`说是要量深色，窗口底却还是亮的（${cv.rgb}）—— 主题没真切过去，别交这份数`);
  }
  if (THEME === '浅色' && cv.lum <= 0.5) {
    throw new Error(`说是要量浅色，窗口底是暗的（${cv.rgb}）—— 主题没真切过去，别交这份数`);
  }
  console.log('CANVAS ' + JSON.stringify(cv) + ' 主题=' + (THEME || '当前'));

  // 鼠标压在表格第一行上再取色：那一层的底是**半透明**的（antd 的 rowHoverBg = `colorFillAlter`），
  // 压在行上是屏上最常见的一帧，而上一版的 `bgOf` 只取"第一个不透明祖先"，那层整个不参与 ——
  // 前景的 alpha 知道先合成，背景的 alpha 却漏了，是同一族谎。
  if (VIEW === '浏览文件') {
    const cell = await send('Runtime.evaluate', {
      expression: '(() => { const c=[...document.querySelectorAll(".ant-table-tbody tr:not(.ant-table-placeholder) .ant-table-cell")]'
        + '.find((e)=>e.getClientRects().length&&e.getBoundingClientRect().width);'
        + 'if(!c)return {}; c.scrollIntoView({block:"center"}); const b=c.getBoundingClientRect();'
        + 'return {x:b.x+b.width/2,y:b.y+b.height/2}; })()',
      returnByValue: true,
    });
    const hp = cell.result.value;
    if (hp && hp.x) {
      // 先点中那一行（选中态的那层底也是半透明的），再把鼠标压在它上面：
      // 「选中 + 悬停」是屏上叠得最狠的一帧，只量静止行等于放过它。
      await clickAt(hp.x, hp.y);
      await new Promise((ok) => setTimeout(ok, 500));
      await send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: hp.x, y: hp.y, buttons: 0 });
      await new Promise((ok) => setTimeout(ok, 400));
    }
  }
  const r = await send('Runtime.evaluate', {
    expression: `(${HARVEST.toString()})(${JSON.stringify(SPOTS)})`,
    returnByValue: true,
  });
  if (r.exceptionDetails) throw new Error(JSON.stringify(r.exceptionDetails));
  const rows = r.result.value;
  if (dropdown && !dropdown.err) {
    rows.push({ label: '.ant-select-dropdown .ant-select-item-option',
      what: '下拉里那一枚候选（开着的时候量的）', want: 4.5,
      fg: dropdown.fg, bg: dropdown.bg, fontSize: dropdown.fontSize, weight: 400,
      count: 1, text: dropdown.text,
      chain: '底 = colorBgElevated（种子，管着模态/抽屉/下拉这一族）' });
  } else if (VIEW === '浏览文件') {
    rows.push({ label: '.ant-select-dropdown .ant-select-item-option',
      what: '下拉里那一枚候选（开着的时候量的）', want: 4.5, missing: true });
  }

  // 主按钮的 hover 态**真把鼠标移上去**再读。第一版用的是 `getComputedStyle(el, ':hover')`，
  // 而 Chromium 那里只认伪元素：color 拿回来是空串，于是算出一个没有根据的比值 ——
  // 假失败和假达标一样坏，它会把人支去改一个本来就对的颜色。
  const box = await send('Runtime.evaluate', {
    expression: '(()=>{const wrap=[...document.querySelectorAll(".ant-modal-wrap")]'
      + '.find((w)=>w.getClientRects().length&&getComputedStyle(w).visibility!=="hidden");'
      + 'const all=[...document.querySelectorAll(".ant-btn-primary")]'
      + '.filter((e)=>e.getClientRects().length&&e.getBoundingClientRect().width);'
      + 'const b=(wrap?all.filter((e)=>wrap.contains(e)):all)[0];if(!b)return "";'
      + 'const q=b.getBoundingClientRect();const cs=getComputedStyle(b);'
      + 'return JSON.stringify({x:q.x+q.width/2,y:q.y+q.height/2,bg:cs.backgroundColor,fg:cs.color});})()',
    returnByValue: true,
  });
  if (box.result.value) {
    const rest = JSON.parse(box.result.value);
    await send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: rest.x, y: rest.y, buttons: 0 });
    await new Promise((ok) => setTimeout(ok, 400));
    const hov = await send('Runtime.evaluate', {
      expression: '(()=>{const wrap=[...document.querySelectorAll(".ant-modal-wrap")]'
        + '.find((w)=>w.getClientRects().length&&getComputedStyle(w).visibility!=="hidden");'
        + 'const all=[...document.querySelectorAll(".ant-btn-primary")]'
        + '.filter((e)=>e.getClientRects().length&&e.getBoundingClientRect().width);'
        + 'const b=(wrap?all.filter((e)=>wrap.contains(e)):all)[0];'
        + 'if(!b)return {};const cs=getComputedStyle(b);return {fg:cs.color,bg:cs.backgroundColor};})()',
      returnByValue: true,
    });
    const moved = !!hov.result.value.bg && hov.result.value.bg !== rest.bg;
    rows.push(moved
      ? { label: '.ant-btn-primary:hover', what: '主按钮压住时（真 hover）', want: 4.5,
        fg: hov.result.value.fg, bg: hov.result.value.bg, fontSize: 13, weight: 500,
        count: 1, text: '（鼠标压住的那一颗主按钮）',
        chain: 'button:hover backgroundColor=' + hov.result.value.bg + '（静止时 ' + rest.bg + '）' }
      // 底色没变 = 鼠标没真压上去（那一屏没有这颗按钮，或它刚被重排移动过）。
      // 报"过"就是拿没量到的东西冒充量过 —— 与假达标一样坏。
      : { label: '.ant-btn-primary:hover', what: '主按钮压住时（真 hover）', want: 4.5, missing: true });
    await send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: 4, y: 4 });
  }

  const parse = (c) => {
    const m = c.match(/rgba?\(([^)]+)\)/);
    if (!m) return [0, 0, 0, 1];
    const p = m[1].split(',').map((s) => parseFloat(s.trim()));
    return [p[0], p[1], p[2], p.length > 3 ? p[3] : 1];
  };
  /**
   * 半透明的前景必须**先合成到底上**再算亮度。antd 的暗色文字大量是 `rgba(ink, .45)` 这类
   * 派生值，直接丢掉 alpha 会把 0.25 的淡字当成实色算出 13.73 —— 那是假达标，
   * 而这把尺子的全部意义就是别谎报。
   */
  const composite = (fg, bg) => {
    const [r, g, b, a] = parse(fg);
    const [br, bg2, bb] = parse(bg);
    return [r * a + br * (1 - a), g * a + bg2 * (1 - a), b * a + bb * (1 - a)];
  };
  const lum = (c) => {
    const [r, g, b] = (typeof c === 'string' ? parse(c) : c).slice(0, 3);
    return [r, g, b]
      .map((v) => v / 255)
      .map((v) => (v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4))
      .reduce((acc, v, i) => acc + [0.2126, 0.7152, 0.0722][i] * v, 0);
  };
  const ratio = (fg, bg) => {
    const [x, y] = [lum(composite(fg, bg)), lum(bg)].sort((m, n) => n - m);
    return (x + 0.05) / (y + 0.05);
  };

  console.log(`# 真窗口实际渲染色的对比度（AA）· 屏=${VIEW || '当前那屏'}${LAYER ? ' · 层=' + LAYER : ''} · 主题=${THEME || '当前那一档'}\n`);
  console.log('| 取样点 | 界面是什么 | 量的是哪一句（同色并入一格） | 前景 | 背景 | 实测 | 要求 | 判定 |');
  console.log('|---|---|---|---|---|---|---|---|');
  let bad = 0;
  let measured = 0;
  let absent = 0;
  const fails = [];
  for (const row of rows) {
    const { label, what, want } = row;
    if (row.missing) {
      // 这一格在**当前这一屏**上不存在（表格面与卡片面不是同一屏）。它不是不达标，是没量到：
      // 混进"不达标"的计数会让那个数字说谎，所以每个取样点要求在两屏里至少被量到一次，
      // 由跑的人核对，而不是由这里冒充。
      console.log(`| \`${label}\` | ${what} | — | — | — | — | ${want || '参考'} | 这一屏没有这个元素 |`);
      absent++;
      continue;
    }
    measured++;
    const seen = (row.text || '—') + (row.count > 1 ? `（+${row.count - 1} 格同色）` : '');
    // AA 大字档：>=18.66px 且加粗，或 >=24px，可以按 3:1 判。
    const large = row.fontSize >= 24 || (row.fontSize >= 18.66 && row.weight >= 700);
    const need = large ? Math.min(want, 3) : want;
    const r = ratio(row.fg, row.bg);
    if (need === 0) {
      console.log(`| \`${label}\` | ${what} | ${seen} | ${row.fg} | ${row.bg} | ${r.toFixed(2)} | 豁免 | 参考（判据不认这一档） |`);
      continue;
    }
    const ok = r >= need;
    if (!ok) { bad++; fails.push({ label, what, seen, fg: row.fg, bg: row.bg, r: r.toFixed(2), chain: row.chain }); }
    console.log(`| \`${label}\` | ${what} | ${seen} | ${row.fg} | ${row.bg} | ${r.toFixed(2)} | ${need}${large ? '（大字档）' : ''} | ${ok ? '过' : '**不过**'} |`);
  }
  console.log('');
  // 不达标的那一格把"这一层底是谁涂的"打出来：一个算出来的数没法修，一条链才能。
  if (fails.length) {
    console.log('不达标那一格的底，从字所在的元素往上到第一个不透明层：\n');
    for (const f of fails) console.log(`- \`${f.label}\`「${f.seen}」${f.fg} 压 ${f.bg} = ${f.r}  ←  ${f.chain || '(没取到链)'}`);
    console.log('');
  }
  // "全部 N 个取样点达标"里如果混着"这一屏没量到"，那句就是谎 —— 分开报。
  console.log(bad === 0
    ? `量到的 ${measured} 个取样点全部达标；另有 ${absent} 个这一屏没量到（换一屏或先让它列出数据）。`
    : `**${bad} 个取样点不达标**（见上表加粗）；量到 ${measured} 个，${absent} 个这一屏没量到。`);
  // 量完把层合上：写盘要的是层里另一颗「确认」，这一遍按的是「取消」。
  // 没关上就抛 —— 留着开着的层会让下一遍把所有取样点读成"层外面那些字看不见"，
  // 那种坏是静默的，宁可当场红。
  if (LAYER) {
    await clickVisible('取消');
    await new Promise((ok) => setTimeout(ok, 900));
    const still = await send('Runtime.evaluate', {
      expression: '(() => [...document.querySelectorAll(".ant-modal-wrap")]'
        + '.some((w) => w.getClientRects().length && getComputedStyle(w).visibility !== "hidden") ? 1 : 0)()',
      returnByValue: true,
    });
    if (still.result.value === 1) throw new Error('按了「取消」层还开着 —— 这一遍的数可用，但别接着往下跑');
    console.log('LAYER-CLOSED 按「取消」合上了（没写盘：写要的是层里那颗「确认」）');
  }
  ws.close();
}

main().catch((e) => { console.error('FAILED', e.message); process.exit(1); });
