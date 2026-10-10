// 界面文案清单：只报**界面会说的话**（字符串字面量与 JSX 文本里的中文），注释与测试文件不算。
// 用法：node tools/ui-copy-scan.mjs > docs/superpowers/evidence/ui-copy-inventory.md
// 改一条就在 docs/superpowers/evidence/ui-copy-diff.md 里记一条（旧句 → 新句 → 依据）。
//
// 退出码（2026-10-09 补）：**标点有违规就退 1**，清单条数不算违规。
// 之前这一条永远是 0，于是 `DESIGN.md`「标点那条机械保证零违规」只能靠人读输出，
// CI 挂不上；现在 `npm run copy:scan` 就能当闸门用。用 `process.exitCode` 而不是
// `process.exit()`，是为了让重定向到文件时 stdout 先冲干净再退。
import fs from 'node:fs';
import path from 'node:path';

const CN = /[\u4e00-\u9fff]/;

/** 从一行 TSX 代码里取出**会显示出来的文本**（不是字符串字面量，那一路单独收）。
 *  做法：先把 `{…}` 表达式整段换成占位符（大括号按层配平，`style={{…}}` 是两层的），
 *  再把 `<…>` 标签换成换行，剩下的中文片段就是 JSX 文本。
 *  为什么非要这一路：上一版只认 `>中文<` 同一行那种，于是跨行的 JSX 文本
 *  （「改完重开面板就行。」）和夹着表达式的那种（「新目录落在：{…}」）根本没进过清单 ——
 *  逐句过文案的底稿里缺的正好是要过的那几句。
 *  这一版改的是**筛子的形状**：旧版把带引号的片段整族筛掉，为的是躲开
 *  `title: '名字',`、`? '都在，可以发布'` 这类代码行（它们由字符串字面量那一路收，
 *  这里再收一遍就是假条目）。代价是真写在标签之间的 `用 ASCII 直引号括中文` 一起消失，
 *  而末节的标点判据正是扫这些片段的 —— 清单报"零条"，屏上却可能有一句违规。
 *  试过把大括号深度跨行接起来（以为"这行还在表达式里"就足以认出代码行）：量出来是**吃掉真文案**。
 *  组件体自己那个 `{` 要到文件末尾才闭，于是整个 JSX 区都在深度 ≥1 里，
 *  跨行接续把 `上传到这里`、`取消` 这样的真句子成段抹掉（609 条掉到 551，掉的几乎全是界面话）。
 *  所以深度仍按行归零，认代码靠**代码形状**：括号、三元、运算符开头、JS 分隔符收尾、对象键开头。
 *  引号本身不再是嫌疑，标点判据才看得见它。 */
const CODE_SHAPE = (s) =>
  /[()?]/.test(s) ||                  // 调用、三元、ASCII 问号：界面句子不用这三个字符
  /^[*+\-/=&|!~^.]/.test(s) ||        // 以运算符或结构符开头（拼接的续行、条件分支）
  /[,;+]$/.test(s) ||                 // 以 JS 分隔符或续接的 `+` 收尾
  /^[`'"].*[`'"]$/.test(s) ||         // 整段被引号包住：那是字符串字面量，那一路已经收过一遍
  /^[A-Za-z_$][\w$]*\s*:/.test(s);    // `title:` / `short:` 这种对象键开头

function jsxTextRuns(code) {
  let out = '';
  let depth = 0;
  for (const ch of code) {
    if (ch === '{') {
      depth++;
      // `${…}` 那个 `$` 是模板字符串的语法，不是文案的一部分
      if (depth === 1) { if (out.endsWith('$')) out = out.slice(0, -1); out += '\u0001'; }
      continue;
    }
    if (depth > 0) {
      if (ch === '}') {
        depth--;
        if (depth === 0) out += '\u0001';
      }
      continue;
    }
    out += ch;
  }
  if (depth > 0) return [];   // 这一行的表达式没闭合：整行留给后面的行，别把半截当句子
  const runs = [];
  for (const seg of out.replace(/<[^>\n]*>/g, '\n').split(/[\n\u0001]/)) {
    // 跨行的开标签会在文本行留一个光 `>`（`… onClick={x}>列一遍`），先剥掉再判。
    const s = seg.replace(/^[<>\s]+|(?:<)[^<>]*$/g, '').replace(/^[,，。；;：:、]+/, '').trim();
    if (CN.test(s) && !/[=<>{}`]/.test(s) && !CODE_SHAPE(s)) runs.push(s);
  }
  return runs;
}

/** 取出这一行里的字符串字面量。
 *  不用一条正则扫过去：那种写法把 `'` 和 `"` 当成同一种界，于是
 *  `'这不是"都齐了"。这次面板在本地…'` 会在第一个直双引号处断开，清单里只剩「都齐了」三个字 ——
 *  而这一句正好是"文案里混用直引号"那一族的本体，底稿把它切碎了就没法逐句过。
 *  这里按**同种引号配对**扫：从开引号找到同种的闭引号，中间另一种引号算正文。
 *  返回 `open`：这一行末尾有半个字符串没闭合（行尾那个 `\` 是续行），交给下一行接着读 ——
 *  本项目好几条长话术就是这么写的（GatePanel 那句"这不是"都齐了""），不接就等于整条丢掉。 */
function stringLiterals(code) {
  const out = [];
  for (let i = 0; i < code.length; i++) {
    const q = code[i];
    if (q !== "'" && q !== '"' && q !== '`') continue;
    let j = i + 1;
    let body = '';
    let closed = false;
    for (; j < code.length; j++) {
      const c = code[j];
      if (c === '\\') { body += c + (code[j + 1] ?? ''); j++; continue; }
      if (c === q) { closed = true; break; }
      if (c === '\n') break;
      body += c;
    }
    if (!closed) {
      // 走到行尾都没闭合：末尾那个 `\` 是续行 ⇒ 这句话还没完，把已读到的内容交出去接着读
      if (body.endsWith('\\')) return { out, open: { q, body: body.slice(0, -1) } };
      continue;
    }
    if (CN.test(body)) out.push(unesc(body));
    i = j;
  }
  return { out, open: null };
}

/** 字符串里的转义（`\"`、`\\`）还原成人眼读到的样子。 */
const unesc = (t) => t.replace(/\\(["'`\\])/g, '$1');

/** 续行的那一半：从下一行开头找到同种闭引号。返回 null = 这一行也还没闭合。 */
function closeCarry(q, rest) {
  let body = '';
  for (let j = 0; j < rest.length; j++) {
    const c = rest[j];
    if (c === '\\') { body += c + (rest[j + 1] ?? ''); j++; continue; }
    if (c === q) return { text: body, tail: rest.slice(j + 1) };
    if (c === '\n') break;
    body += c;
  }
  return null;
}

function walk(dir, out) {
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) walk(p, out);
    else if (/\.(tsx?|rs)$/.test(e.name) && !p.includes(`${path.sep}target${path.sep}`)) out.push(p);
  }
  return out;
}

/** 行级剥注释，带一个**跨行状态**：块注释（`/* … *\/`，含 JSX 的 `{/* … *\/}`）没在本行闭合时，
 *  后面每一行都算注释，直到看见闭合。上一版只看行首，于是 JSX 注释体里那些中文
 *  （本项目大量用 `{/* 为什么这么摆 *\/}` 讲理由）被当成界面会说的话收了进来 —— 474 条里有一大截是这种噪声，
 *  而这份清单是逐句过文案的底稿，底稿不干净就每条都要先人肉判一次"这是话还是注释"。
 *
 *  仍是清单不是闸门：字符串里出现 `//` 会被截半句（那种行少一条，人对着代码复核）。 */
function makeStripper() {
  let inBlock = false;
  return function codePart(line) {
    let work = line;
    if (inBlock) {
      const close = work.indexOf('*/');
      if (close < 0) return null;
      inBlock = false;
      work = work.slice(close + 2);
    }
    const trimmed = work.trim();
    const opens = (s) => {
      const at = s.indexOf('/*');
      if (at < 0) return false;
      return !s.slice(at + 2).includes('*/');
    };
    if (/^(\/\/|\{?\/\*|\*)/.test(trimmed)) {
      if (opens(trimmed)) inBlock = true;
      return null;
    }
    const slash = work.indexOf('//');
    if (slash >= 0) work = work.slice(0, slash);
    const block = work.indexOf('/*');
    if (block >= 0) {
      if (opens(work.slice(block))) inBlock = true;
      work = work.slice(0, block);
    }
    return work;
  };
}

const lines = [];
// 标点两族的尺子，跟着清单一起报（口径与 `DESIGN.md` 的「标点」那条同源）：
//  引用一个说法 = 「」，不许用 ASCII 直引号；中文句子中的冒号 = 全角 ：
// 两样东西不参与：反引号里的（那是**照抄的真字符**：配置写法、rclone 的原话），
// 以及 `${…}` / `{…}` 那种插值（里面是代码，不是文案）。
const punct = [];
const punctuationCheck = (rel, line, text) => {
  const prose = text.replace(/`[^`]*`/g, '\u0002').replace(/\$\{[^}]*\}/g, '\u0003').replace(/\{[^}]*\}/g, '\u0003');
  for (const q of ['"', "'"]) {
    const parts = prose.split(q);
    for (let k = 1; k < parts.length - 1; k += 2) {
      if (CN.test(parts[k])) punct.push(`${rel}:${line}  直引号括中文  ${q}${parts[k]}${q}`);
    }
    if (parts.length % 2 === 0) punct.push(`${rel}:${line}  落单的直引号  ${prose.slice(0, 60)}`);
  }
  for (let k = 1; k < prose.length; k++) {
    if (prose[k] === ':' && CN.test(prose[k - 1])) {
      punct.push(`${rel}:${line}  半角冒号  …${prose.slice(Math.max(0, k - 14), k + 10).replace(/\s+/g, ' ')}`);
    }
  }
};

lines.push('# 界面文案清单（待改清单）');
lines.push('');
lines.push('生成：`node tools/ui-copy-scan.mjs`。改一条就在 `ui-copy-diff.md` 里记一条（旧句 → 新句 → 撞哪条裁定）。');
lines.push('口径：只收**界面会说的话** —— 字符串字面量与 JSX 文本里的中文；注释（含跨行的块注释与 JSX 注释）、测试、日志行不算。');
lines.push('用 `+` 拼起来的一句话合成一条（拼接是行宽的事，不是句子的事）；行号取那一句开头那一行。');
lines.push('');

for (const group of [
  // `.ts` 也要收：那句「正在跑：X」现在住在 `types.ts` 的 `runningHint` 里（卡面与右列共用一处），
  // 上一版只扫 `.tsx`，于是"界面会说的话"少了一整族——句子搬进 .ts 而清单没跟上。
  { label: '前端（src/**/*.tsx 与 src/**/*.ts）', dir: 'src', filter: (f) => /\.tsx?$/.test(f) },
  { label: '后端交给界面的句子（src-tauri/src/**/*.rs）', dir: 'src-tauri/src', filter: (f) => f.endsWith('.rs') },
]) {
  lines.push('## ' + group.label);
  lines.push('');
  let total = 0;
  for (const f of walk(group.dir, []).filter(group.filter).sort()) {
    // 这份仓在 Windows 上，源码是 CRLF：不先剥掉行尾的 `\r`，"行尾续行反斜杠"就永远差一个字符
    const src = fs.readFileSync(f, 'utf8').split('\n').map((l) => l.replace(/\r$/, ''));
    const codePart = makeStripper();
    const hits = [];
    let inTests = false;
    // 半个字符串（行尾续行反斜杠）从上一行带过来的：{ q, body, line }
    let carry = null;
    src.forEach((line, i) => {
      const s = line.trim();
      if (f.endsWith('.rs')) {
        if (/^(mod|#\[cfg\(test\)\])/.test(s) && s.includes('tests')) inTests = true;
        if (inTests) { codePart(line); return; }
      }
      const code = codePart(line);
      if (code === null) return;
      let rest = code;
      if (carry) {
        const done = closeCarry(carry.q, code);
        if (done === null) {
          carry.body += code.replace(/\\\s*$/, '');
          return;
        }
        const text = carry.body + done.text;
        if (CN.test(text)) hits.push({ line: carry.line, text: unesc(text), pending: false });
        carry = null;
        rest = done.tail;
      }
      const trimmed = rest.trim();
      // 这一行以 `+` 收尾 = 那句话还没说完；下一行以 `+` 或引号开头 = 接着说。
      const continues = /\+\s*$/.test(trimmed);
      const found = stringLiterals(rest);
      if (found.open) carry = { ...found.open, line: i + 1 };
      for (const text of found.out) {
        const glue = /^(?:\+\s*)?['"`]/.test(trimmed) && hits.length > 0 && hits[hits.length - 1].pending;
        if (glue) {
          const last = hits[hits.length - 1];
          last.text += text;
          last.pending = continues;
          continue;
        }
        hits.push({ line: i + 1, text, pending: continues });
      }
      if (f.endsWith('.tsx')) {
        // 同一行的几段合成一条，中间用 ` … ` 标出"这里插了一个值"：
        // 「应有 / 个，实际 / 个 · / 查的」拆成四条就没法读，而那句本来是一句。
        const runs = jsxTextRuns(rest);
        if (runs.length) hits.push({ line: i + 1, text: runs.join(' … '), pending: false });
      }
    });
    if (!hits.length) continue;
    lines.push('#### ' + f.replace(/\\/g, '/') + '  (' + hits.length + ' 条)');
    for (const h of hits) {
      total++;
      const shown = h.text.replace(/\s+/g, ' ');
      punctuationCheck(f.replace(/\\/g, '/'), h.line, shown);
      lines.push('    ' + String(h.line).padStart(4) + '  ' + shown);
    }
    lines.push('');
  }
  lines.push('小计：' + total + ' 条');
  lines.push('');
}
lines.push('## 标点（直引号括中文 / 中文句中的半角冒号）');
lines.push('');
lines.push('判据与豁免见 `DESIGN.md` 的「标点」那条；反引号里照抄的字符与 `${…}` 插值不参与。');
lines.push('');
lines.push(punct.length
  ? punct.map((p) => '- ' + p).join('\n')
  : '（零条。出现新条目就按那条改：引用一个说法用「」，中文句子中的冒号用全角。）');
process.stdout.write(lines.join('\n') + '\n');

// 闸门：标点有违规就退 1（清单条数不算，那不是违规，是活儿还没干完）。
// 报告已经完整写到 stdout 了，这里只改退出码。
process.exitCode = punct.length ? 1 : 0;
