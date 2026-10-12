// 纯英文界面文案扫描：ui-copy-scan.mjs 只收**含中文**的句子（`CN.test(s)`），
// 所以一条纯 ASCII 的界面话（`title="Delete"`、`<Button>Cancel</Button>`、空状态）
// 对那道闸门是隐形的。这个脚本补上那一半。
//
// 用法：
//   node tools/ascii-ui-scan.mjs                # 扫 src/
//   node tools/ascii-ui-scan.mjs 某个目录或文件   # 扫别处（给闸门当靶子时用）
//
// 退出码：报出条目就退 1，零条退 0（可直接当 CI 闸门用）。
//
// ── 三路候选，按"这一路有多像界面话"分别收紧 ─────────────────────────
// ① JSX 属性（title/label/placeholder/okText…）：**高**信号，单个词也算
// ② JSX 文本节点（>Delete</Button>）：**高**信号，单个词也算
// ③ 字符串字面量：**低**信号，必须是散文（≥2 词或命中单 UI 动词表）
//    —— 否则 'flex' / 'warning' / 'sticky' 这类样式与枚举会成片误报。
//
// 这三路的分档是 2026-10-12 第二版才有的：第一版把三路用同一把尺子，
// 于是①漏掉单个大写词（`Delete` 被 `words.length < 2` 跳过）、②整路没扫、
// ③的 ALLOW 词表又太宽（`copy path name` 这种整句由允许词拼成的会被放过）。
import fs from 'node:fs';
import path from 'node:path';

const ROOT = process.cwd();
const CN = /[\u4e00-\u9fff]/;

// 会出现在界面话里的英文功能/状态/否定词 —— 只用于③字符串字面量那一路
const PROSE = new RegExp(
  '\\b(no|not|none|nothing|empty|cannot|failed|failure|fails|error|success|cancel|save|saved|' +
  'delete|deleted|remove|removed|rename|renamed|upload|uploaded|download|downloaded|move|moved|' +
  'copy|copied|retry|refresh|reload|close|closed|open|opened|select|selected|choose|click|press|' +
  'loading|busy|ready|unknown|missing|invalid|file|files|folder|folders|directory|directories|' +
  'path|size|name|names|status|task|tasks|job|jobs|queue|running|paused|stopped|stop|done|' +
  'warning|confirm|confirmed|are|is|was|were|the|and|or|to|for|with|again|here|there|try|please)\\b',
  'i',
);


// 单个词就算界面话的高信号词表 —— 供①②两路用
const UI_WORD = new RegExp(
  '^(delete|cancel|ok|save|close|open|retry|refresh|reload|upload|download|rename|move|copy|' +
  'select|choose|loading|busy|ready|unknown|missing|invalid|none|nothing|empty|error|failed|' +
  'confirm|back|next|done|running|paused|stopped|stop|yes|no|add|edit|new|send|print|copy)$',
  'i',
);

// 明确不是散文的东西
const CSS_VALUE =
  /^(var\(--[\w-]+\)|#[\da-fA-F]{3,8}|[-\d.]+(px|rem|em|%|vh|vw|s|ms|deg|fr|ch|ex)|rgba?\(|hsla?\()/;
// 只有这一条形状过滤，且**只用在③字符串字面量那一路**（防 CSS 类名 `fs-code path-line` 误报）。
// 高信号的①属性 / ②JSX 文本节点一路**不做任何形状过滤** ——
// 第一版就是在这里栽的：它把 `Delete` 当成合法标识符放过去、把 `copy path name`
// 当成类名放过去，于是闸门对这两类真实漏翻全盲，却在 src/ 上报「零条」。
// 留着几个"看着像过滤器但没人调用"的大小写判据最容易把同一个 bug 请回来，所以不留。
const CLASS_SHAPE = /^[a-z][\w-]*(\s+[a-z][\w-]*)*$/;        // thumb-name fs-code

const ALLOWED_ATTR = new Set([
  'title', 'label', 'placeholder', 'okText', 'cancelText', 'emptyText',
  'description', 'message', 'tooltip', 'aria-label', 'alt', 'hint', 'reason',
  'header', 'footer', 'confirm', 'summary', 'extra', 'text',
]);
const NOT_TEXT_ATTR = new Set(['className', 'style', 'id', 'type', 'key', 'role', 'name', 'variant', 'size']);

function walk(target, out = []) {
  const st = fs.statSync(target);
  if (st.isFile()) return /\.(tsx?|jsx?)$/.test(target) ? [target] : out;
  for (const e of fs.readdirSync(target, { withFileTypes: true })) {
    const p = path.join(target, e.name);
    if (e.isDirectory()) walk(p, out);
    else if (/\.(tsx?|jsx?)$/.test(e.name)) out.push(p);
  }
  return out;
}

function stripComments(src) {
  return src
    .replace(/\/\*[\s\S]*?\*\//g, (m) => m.replace(/[^\n]/g, ' '))
    .replace(/(^|[^:])\/\/[^\n]*/g, (m, p1) => p1 + ' '.repeat(Math.max(0, m.length - p1.length)));
}

/** 把一行 JSX 里被标签包住的**文本节点**取出来。
 *  与 ui-copy-scan 同法：`{…}` 表达式整段换成占位符（按层配平），`<…>` 标签换成换行，
 *  余下的英文片段就是会渲染出来的字。 */
function jsxTextRuns(code) {
  let out = '';
  let depth = 0;
  for (const ch of code) {
    if (ch === '{') { depth++; if (depth === 1) out += '\u0001'; continue; }
    if (depth > 0) { if (ch === '}') { depth--; if (depth === 0) out += '\u0001'; } continue; }
    out += ch;
  }
  if (depth > 0) return [];
  const runs = [];
  for (const seg of out.replace(/<[^>\n]*>/g, '\n').split(/[\n\u0001]/)) {
    const s = seg.replace(/^[<>\s]+|[<>\s]+$/g, '').trim();
    if (s) runs.push(s);
  }
  return runs;
}

/** 这一行是不是 JSX？只对含真标签的行启用②那一路。
 *  不设这道闸时，`const i = name.lastIndexOf('/')` 这种**普通 TS 行**会被
 *  `jsxTextRuns` 整段当成"渲染出来的字"——它一个标签都没有，剥完什么都没剩下。
 *  src/ 一下报了 265 条，全是这一处。第三版才补上。 */
const JSX_TAG = /<\/?[A-Za-z][\w.]*(\s[^>]*)?\/?>/;

/** 代码形状（抄 ui-copy-scan 的 CODE_SHAPE，同一个仓里两把尺子得同一个量法）：
 *  调用/三元/ASCII 问号、运算符开头、JS 分隔符收尾、对象键开头 —— 这些不是句子。 */
const CODE_SHAPE = (s) =>
  /[()?]/.test(s) ||
  /^[*+\-/=&|!~^.]/.test(s) ||
  /[,;+]$/.test(s) ||
  /^[`'"].*[`'"]$/.test(s) ||
  /^[A-Za-z_$][\w$]*\s*:/.test(s);

/** 界面散文里不会出现这些 —— 出现就说明这是代码，不是给人读的字。
 *  为的是 TS **泛型**：`useState<AntdApp.useAppReturnType>(…)` 里那个 `<…>`
 *  长得和 JSX 标签一模一样，光靠 JSX_TAG 分不开（第三版踩过，src/Panel.tsx 两条）。
 *  逗号不算：英文句子本来就带逗号（"Upload failed, please try again"）。 */
const CODE_PUNCT = /[=()[\];]|=>|\?\?|&&|\|\||===|!==/;

const findings = [];
const roots = process.argv.slice(2);
const targets = roots.length ? roots : [path.join(ROOT, 'src')];

for (const file of targets.flatMap((t) => walk(path.resolve(ROOT, t)))) {
  const rel = path.relative(ROOT, file).replace(/\\/g, '/');
  const src = stripComments(fs.readFileSync(file, 'utf8'));
  src.split('\n').forEach((line, i) => {
    const isImport = /(^|[^\w.])import\s|^\s*export\s.*from\s/.test(line);

    const record = (kind, raw, high) => {
      let text = raw.trim().replace(/\s+/g, ' ');
      if (!text || CN.test(text) || isImport) return;
      if (CSS_VALUE.test(text)) return;
      if (/^[<{]|[}>]$/.test(text)) return;
      if (/^[,;:]/.test(text)) return;
      if (/\/\//.test(text) || /^[a-z]+:\/\//.test(text)) return;
      const words = text.split(' ').filter(Boolean);
      if (high) {
        // 高信号路：单个词看 UI 词表，多词散文看 PROSE，**不做形状过滤**。
        // JSX 文本节点是直接渲染出来的，`className` 才走 className=。
        // 形状过滤在这一路只会制造盲区（`Delete` 当标识符、`copy path name` 当类名）——
        // 第二版两处都栽在这。第三版把过滤全撤了。
        if (!PROSE.test(text) && !(words.length === 1 && UI_WORD.test(text))) return;
      } else {
        if (words.length === 1 && !UI_WORD.test(text)) return;
        if (!PROSE.test(text)) return;
      }
      findings.push({ rel, line: i + 1, kind, text });
    };

    // ① JSX 属性
    const attrRe = /\b([A-Za-z-]+)\s*=\s*(?:"([^"]*)"|'([^']*)'|\{"([^"]*)"\})/g;
    let m;
    while ((m = attrRe.exec(line))) {
      const name = m[1];
      if (!ALLOWED_ATTR.has(name)) continue;
      record(`attr ${name}`, m[2] ?? m[3] ?? m[4] ?? '', true);
    }

    // ② JSX 文本节点（只在这一行确实含 JSX 标签时才启用）
    if (JSX_TAG.test(line)) {
      for (const t of jsxTextRuns(line)) {
        if (CODE_SHAPE(t)) continue;
        if (CODE_PUNCT.test(t)) continue;
        record('jsx text', t, true);
      }
    }


    // ③ 字符串字面量（低信号路）
    const litRe = /(['"])((?:(?!\1).){2,})\1/g;
    while ((m = litRe.exec(line))) {
      const at = line.slice(0, m.index);
      if (/\b(className|style|id|type|key|role|variant)\s*=\s*$/.test(at)) continue;
      if (CLASS_SHAPE.test(m[2].trim())) continue;
      record('literal', m[2], false);
    }
  });
}

console.log('# 纯英文界面文案扫描\n');
if (!findings.length) {
  console.log(`（零条。扫了 ${targets.length} 个目标，没有会被读成英文的界面话。）\n`);
} else {
  const by = new Map();
  for (const f of findings) {
    if (!by.has(f.rel)) by.set(f.rel, []);
    by.get(f.rel).push(f);
  }
  for (const [rel, list] of by) {
    console.log(`#### ${rel}`);
    for (const f of list) console.log(`${String(f.line).padStart(5)}  [${f.kind}]  ${f.text}`);
    console.log('');
  }
  console.log(`小计：${findings.length} 条`);
  process.exitCode = 1;
}
