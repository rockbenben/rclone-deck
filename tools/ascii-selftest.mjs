// ascii-ui-scan 的自检：那道闸门**必须仍然抓得到东西**。
//
// 为什么要有它：这道闸门第一版和第二版都能在真代码上报零条，看着"绿得很干净"，
// 可它对 `<Button>Delete</Button>`、`copy path name` 这类真实漏翻是**全盲**的
// （单大写词被 `words.length < 2` 跳过，JSX 文本节点整路没扫）。
// 零条既可能是"干净"，也可能是"瞎了"——从输出上看不出区别。
// 于是把一份**故意写错**的英文文案钉在 tools/ascii-selftest/deliberate.tsx，
// 这条命令断言"扫它必须红且条数不减"。哪天闸门退化了，红的是这一条，不是线上那次。
//
// 用法：npm run copy:ascii:selftest
import { spawnSync } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const SCANNER = path.join(ROOT, 'tools', 'ascii-ui-scan.mjs');
const FIXTURE = path.join(ROOT, 'tools', 'ascii-selftest');

// 靶子里刻意埋的每一类漏翻，少抓到一类就说明这一路又退化了
const EXPECT = [
  'Delete',                        // JSX 文本节点里的单个大写词
  'Cancel',                        // 同上
  'Loading',                       // 同上
  'No results found',              // JSX 文本节点里的多词散文
  'Upload failed, please try again', // 带逗号的散文（逗号不能拿来判"这是代码"）
  'copy path name',                // 全小写多词 —— CLASS_SHAPE 那类形状过滤的盲区
  'Refresh',                       // 单个大写词的 title=
  'No data',                       // 空状态 description=
  'Copy the file name to clipboard', // title= 里的长散文
  'Confirm',                       // okText=
  'Nothing here yet',              // emptyText=
];

const run = spawnSync(process.execPath, [SCANNER, FIXTURE], {
  cwd: ROOT,
  encoding: 'utf8',
});

if (run.error) {
  console.error(`自检跑不起来：${run.error.message}`);
  process.exit(1);
}

const out = `${run.stdout ?? ''}${run.stderr ?? ''}`;
const missing = EXPECT.filter((s) => !out.includes(s));

console.log('# ascii-ui-scan 自检\n');
if (run.status === 0) {
  console.log('✗ 闸门没抓到靶子里的任何一条 —— 它已经瞎了（扫真代码时那个“零条”是假的）。');
  console.log(out);
  process.exit(1);
}
if (missing.length) {
  console.log(`✗ 退化了：靶子里这 ${missing.length} 类没被抓到 ——`);
  for (const s of missing) console.log(`  - ${s}`);
  console.log(`\n${out}`);
  process.exit(1);
}

const count = (out.match(/^\s+\d+\s+\[/gm) ?? []).length;
console.log(`✓ 闸门仍然抓得到：${EXPECT.length} 类漏翻全部命中（共报 ${count} 条，退出码 ${run.status}）。`);
