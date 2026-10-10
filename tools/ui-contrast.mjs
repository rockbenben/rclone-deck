// 对比度实测：把 DESIGN.md 的色板按 WCAG 2.1 相对亮度算一遍配对比值。
// 为什么用算的而不是截图：这一条问的是"这两块颜色放在一起差多少"，那是色值的函数，
// 截图只会把它再量化一次还引入抗锯齿误差。真窗口那侧另有闸门（截图 + DOM 读数）。
// 用法：node tools/ui-contrast.mjs > docs/superpowers/evidence/ui-contrast.md
import fs from 'node:fs';

// 色值只有一份：从 `src/theme.ts` 里读出来，不在这里再抄一遍（抄了就会有第二份口径）。
function readPalette() {
  const src = fs.readFileSync('src/theme.ts', 'utf8');
  const out = {};
  for (const theme of ['light', 'dark']) {
    const block = src.split(theme + ': {')[1].split('}')[0];
    const p = {};
    for (const m of block.matchAll(/(\w+):\s*'(#[0-9A-Fa-f]{6})'/g)) p[m[1]] = m[2];
    if (Object.keys(p).length < 12) throw new Error('从 theme.ts 里没读全 ' + theme + ' 的色板');
    out[theme] = p;
  }
  return out;
}
const palette = readPalette();

const lum = (hex) => {
  const c = hex.replace('#', '');
  const [r, g, b] = [0, 2, 4].map((i) => parseInt(c.slice(i, i + 2), 16) / 255)
    .map((v) => (v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4));
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
};
const ratio = (a, b) => {
  const [x, y] = [lum(a), lum(b)].sort((m, n) => n - m);
  return (x + 0.05) / (y + 0.05);
};

// 配对：谁摆在谁上面。名字与 DESIGN.md 的 Colors 一节对齐。
const PAIRS = [
  ['ink', 'canvas', 4.5, '正文摆在窗口底'],
  ['ink', 'surface', 4.5, '正文摆在卡片上'],
  ['ink2', 'surface', 4.5, '次要文字（说明句）'],
  ['ink3', 'surface', 4.5, '短注 / 时间 / 标签'],
  // 四档成片淡底各量一遍：那是屏上唯一成片的语义底，正文（ink3 那一档）压在它上面，不是压在白卡片上。
  // 卡片上过了不等于那一格过了 —— 浅色的 success 派生底就是这么被抓到 2.54 的。
  ['ink3', 'successBand', 4.5, '「做完了」那一格的小字'],
  ['ink3', 'dangerBand', 4.5, '「没改成」那一格的小字'],
  ['ink3', 'warningBand', 4.5, '警示那一格的小字'],
  ['ink3', 'infoBand', 4.5, '解释那一格的小字'],
  ['ink2', 'successBand', 4.5, '「做完了」那一格的正文'],
  // 控件底那一档（`surfaceMuted`）也是字真正压着的底：日志尾巴（ink2）在它上面，
  // 而**表格悬停/选中那一行叠出来就是它**（ink3）。这一对以前不在表里，
  // 于是色板级"全过"而真窗口里深色那一档实测 4.04 —— 卡片上过了不代表所有底上过了。
  ['ink3', 'surfaceMuted', 4.5, '压在控件底/悬停行那一档上的小字'],
  ['ink2', 'surfaceMuted', 4.5, '日志尾巴那一串（就在控件底上）'],
  ['ink4', 'surface', 3, '禁用文字（只要求 3:1，它不是要人读的）'],
  ['primary', 'surface', 4.5, '可点的事与"做完了"'],
  ['onPrimary', 'primary', 4.5, '主按钮上的字'],
  // 交互两档也进表：这一档以前交给算法派生，结果比静止色更亮、白字压上去不过线。
  // 色板级只是前置检查，真窗口那次量的是实际渲染值（DESIGN.md 的 Colors 一节写着这条口径）。
  ['onPrimary', 'primaryHover', 4.5, '主按钮压住时'],
  ['onPrimary', 'primaryActive', 4.5, '主按钮按下去时'],
  ['warning', 'surface', 4.5, '看一眼那一档'],
  ['danger', 'surface', 4.5, '被拒 / 删除族'],
  ['hairline', 'surface', 1, '描边：只要求看得见，不当文字算'],
];

const lines = [];
lines.push('# 色板对比度实测（WCAG 2.1 相对亮度）');
lines.push('');
lines.push('生成：`node tools/ui-contrast.mjs`。阈值那一列写的是**这条要求达到的比**，不是实测值。');
lines.push('');
let fail = 0;
for (const theme of ['light', 'dark']) {
  lines.push('## ' + theme);
  lines.push('');
  lines.push('| 配对 | 前景 | 背景 | 实测 | 要求 | 说法 |');
  lines.push('|---|---|---|---|---|---|');
  for (const [fg, bg, want, note] of PAIRS) {
    const r = ratio(palette[theme][fg], palette[theme][bg]);
    const ok = r >= want;
    if (!ok) fail++;
    lines.push(`| ${fg} on ${bg} | ${palette[theme][fg]} | ${palette[theme][bg]} | ${r.toFixed(2)} | ${want} | ${ok ? '过' : '**不过**'} · ${note} |`);
  }
  lines.push('');
}
lines.push(fail === 0 ? '全部配对达标。' : `有 ${fail} 条不达标（见上表的加粗）。`);
lines.push('');
lines.push('这份报告只是**前置检查**：色板过 AA 不代表组件过 —— 组件库会自己派生一批不在色板里的颜色'
  + '（次要文字、占位、按钮上的字、预设色名的标签）。交付判据是另一把尺子：'
  + '`npm run contrast:rendered`，在真窗口取 `getComputedStyle` 的实际渲染值算（半透明前景先合成到底上）。');
process.stdout.write(lines.join('\n') + '\n');
