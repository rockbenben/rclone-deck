// 界面整修的验收夹具：两套家目录（A 带排除规则、B 不带），全在 %TEMP% 里，零云端。
// 每个验收场景有自己的子目录，互不消耗。
import fs from 'node:fs';
import path from 'node:path';
import { tmpdir } from 'node:os';
import { execFileSync } from 'node:child_process';

// rclone 不是一个固定路径，是**这台机器上的某一个可执行文件**。原来这里写死了作者那台的
// WinGet 链接，别人克隆下来直接跑会在"rclone 不动"上死掉，而那形状和"夹具铺错了"一模一样。
// 口径跟着仓里已有的 `rc-contract-probe.mjs` 走：`RCLONE_BIN` 显式指定，否则在 PATH 上找。
//
// **但必须解析成绝对路径再写进 jobs.toml**，不能把裸名 `rclone` 写进去：面板的
// `AppConfig::from_defaults` 明确拒绝非绝对路径（"本机 PATH 上有两个 rclone，靠 PATH 顺序
// 选是赌博"—— 那条规矩是这个仓明文写下的，不是疏漏）。所以"可移植"的做法是**在工具里
// 把 PATH 解成绝对路径**，而不是把这份判断推给面板。
function resolveRclone() {
  const given = process.env.RCLONE_BIN;
  if (given) return given;
  for (const [cmd, args] of [['where.exe', ['rclone']], ['where', ['rclone']], ['which', ['rclone']]]) {
    try {
      const out = execFileSync(cmd, args, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] });
      const first = out.split(/\r?\n/).map((s) => s.trim()).filter(Boolean)[0];
      if (first && fs.existsSync(first)) return first;
    } catch { /* 这个探测方式这台机器上没有，试下一个 */ }
  }
  throw new Error(
    'PATH 上找不到 rclone。装一个，或者设 RCLONE_BIN 指向它的完整路径 ——\n' +
    '（不能靠裸名：面板要求 rclone_bin 是绝对路径，它不赌 PATH 顺序。）');
}
const RCLONE = resolveRclone();
// `%TEMP%` 走 Node 的 tmpdir()，不是抄一条绝对路径。
const TMP = tmpdir();
// --home A|B 只铺一套；不给参数就两套都铺（驱动的两段各用一套）。
const ONLY = (process.argv.find((a) => a.startsWith('--home=')) || '').split('=')[1] || '';
const want = (name) => ONLY === '' || ONLY === name;

const real = path.join(process.env.APPDATA, 'rclone-deck');
if (fs.existsSync(real)) {
  for (const f of fs.readdirSync(real)) {
    const st = fs.statSync(path.join(real, f));
    console.log('REALHOME ' + f + ' size=' + st.size + ' mtime=' + st.mtime.toISOString());
  }
} else {
  console.log('REALHOME absent');
}
// 设置面那一条通路会写 `rclone.conf`（rclone 自己的那份文件），所以操作员那一份的
// 大小与时刻要进读数：验收跑完之后同一个数还得回来，否则就是这一遍动了真文件。
for (const [name, p] of [['REALCONF', path.join(process.env.APPDATA, 'rclone', 'rclone.conf')]]) {
  if (fs.existsSync(p)) {
    const st = fs.statSync(p);
    console.log(name + ' size=' + st.size + ' mtime=' + st.mtime.toISOString());
  } else {
    console.log(name + ' absent');
  }
}

function w(p, s) { fs.mkdirSync(path.dirname(p), { recursive: true }); fs.writeFileSync(p, s, 'utf8'); }
function b(n) { return 'x'.repeat(n); }

function build(home, remoteName, bucket, opts) {
  // 先清干净再铺：验收要能一遍遍重跑，每一遍的起点得是同一个形状
  //（否则上一遍下载/搬走的东西会留在原地，撞名那一格就会打在错的地方）。
  fs.rmSync(home, { recursive: true, force: true });
  fs.mkdirSync(path.join(home, 'logs'), { recursive: true });
  const fwd = (p) => p.replace(/\\/g, '/');
  const B = (rel) => bucket + '/' + rel;

  // 左栏的候选：任务里出现过的本地目录（`browser_remotes` 就是从这里取的）。
  w(`${home}/local/a.webp`, b(30));
  w(`${home}/local/b.webp`, b(40));
  w(`${home}/local/only-local.webp`, b(50));
  w(`${home}/local/note.log`, b(7));
  w(`${home}/local/sub/c.webp`, b(20));
  w(`${home}/mirror-src/m1.webp`, b(11));
  w(`${home}/mirror-src/m2.webp`, b(12));
  w(`${home}/mirror-src/new.webp`, b(13));
  // 设置面「改这条」那一步把对齐的源换到这一个：存在的目录才配当"另一个源"，
  // 不然验收证的是"面板能把一份坏配置写进去"，而那不是这件事。
  w(`${home}/mirror-src-changed/m1.webp`, b(11));
  w(`${home}/mirror-src-changed/m2.webp`, b(12));

  // a：过滤后的视图 + 上传撞名的落点（note.log 被 `*.log` 藏着）
  w(B('a/a.webp'), b(30));
  w(B('a/b.webp'), b(40));
  w(B('a/orphan.webp'), b(21));
  w(B('a/note.log'), b(7));
  // b：对齐那一行的两侧（本地多 new、远端多 gone ⇒ 摘要 1/1）
  w(B('b/m1.webp'), b(11));
  w(B('b/m2.webp'), b(12));
  w(B('b/gone.webp'), b(22));
  // c：一次真的复制的两侧（文件多一些，好让"正在写同一棵树"那一句量得到）
  for (let i = 0; i < opts.bulk; i++) w(`${home}/local2/u${i}.webp`, b(16));
  for (let i = 0; i < opts.bulk; i += 2) w(B(`c/u${i}.webp`), b(16));
  // del1 / delN：删这一个、删掉看到的 N 个
  w(B('del1/keep.webp'), b(9));
  w(B('del1/drop.webp'), b(9));
  for (const n of ['d1.webp', 'd2.webp', 'd3.webp']) w(B('delN/' + n), b(9));
  // grey：清空这一层（A 有规则 ⇒ 置灰；B 无规则 ⇒ 真能清）
  w(B('grey/x.webp'), b(5));
  w(B('grey/y.webp'), b(6));
  w(B('grey/deep/z.webp'), b(7));
  // mk：建目录、删空目录、删非空被拒
  w(B('mk/empty/keepinside/.keep'), '');
  fs.rmSync(`${B('mk/empty/keepinside')}`, { recursive: true });   // 留一个真正的空目录
  fs.mkdirSync(`${B('mk/empty')}`, { recursive: true });
  w(B('mk/full/something.webp'), b(9));
  // rn：改名与改名撞名
  w(B('rn/old-name.webp'), b(17));
  w(B('rn/taken.webp'), b(17));
  // mv：移进来的目标
  fs.mkdirSync(B('mv'), { recursive: true });

  // 设置面的写入实验打在这一份上：段内有注释、有密钥形状的哨兵值、有白名单之外的键。
  // 形状照真机量到的样子（真 conf 全 CRLF、段内有注释与明文密钥、还有 `chunk_size` 这种
  // 面板不许碰的键）。哨兵值在 alias 这一段里对 rclone 是无害的额外键（实测：`lsf` 与
  // `config file` 都不报错），但对证人有用——屏上出现 `SENTINEL` 就等于密钥上了界面。
  const conf = [
    `[${remoteName}]`,
    'type = alias',
    `remote = ${fwd(bucket)}`,
    '# 这行注释是操作员自己写的理由，任何一次保存都不许吃掉',
    'access_key_id = SENTINEL-AK-' + remoteName,
    'secret_access_key = SENTINEL-SK-' + remoteName,
    'endpoint = https://example.invalid',
    'chunk_size = 4194304',
    '',
  ].join('\r\n') + '\r\n';
  w(path.join(home, 'conf', 'rclone.conf'), conf);

  // 路径一律写成 TOML 的**字面字符串**（单引号）：双引号里 `\Users` 会被当成 unicode 转义读，
  // 整份 jobs.toml 解析失败（"too few unicode value digits"），而那种失败在界面上只是"暂无数据"。
  const P = (o) => "'" + home.replace(/\//g, '\\') + o + "'";
  const jobs = [
    // 这两行是"理由注释"的形状：面板的写路径按行改，没点名的行要逐字节回来。
    '# 这一段是操作员自己写的理由，任何一次保存都不许吃掉。',
    '# 第二行留着：只保住第一行等于没验证"连续多行"那一档。',
    '[defaults]',
    `rclone_bin = '${RCLONE}'`,
    `exclude = ${JSON.stringify(opts.exclude)}`,
    `log_dir = ${P('\\logs')}`,
    '',
    '[[job]]',
    'id = "fx-copy"',
    '# 这一条是批量上传用的，别改它的目的',
    'name = "图床批量上传（夹具）"',
    'kind = "copy"',
    `src = ${P('\\local2')}`,
    `dst = "${remoteName}:c"`,
    '',
    '[[job]]',
    'id = "fx-copy-down"',
    '# 这一条与上面那一条是**同一对位置的反方向**：左列现在一张卡就是一对位置，',
    '# 两个方向是卡上那一格选择。铺两条，"换一格屏上真的换了"这件事才量得到。',
    'name = "图床批量下载（夹具）"',
    'kind = "copy"',
    `src = "${remoteName}:c"`,
    `dst = ${P('\\local2')}`,
    '',
    '[[job]]',
    'id = "fx-sync-c"',
    '# 这一条与上面两条是**同一对位置**，只是模式不同：一张卡上于是有了「只补，不删」两格与',
    '# 「让两边对齐（会删）」一格。每一档的选项自己就是一句完整的话，这一条就是让那件事量得出来的。',
    'name = "封面修剪（会删，夹具）"',
    'kind = "sync"',
    `src = ${P('\\local2')}`,
    `dst = "${remoteName}:c"`,
    '',
    '[[job]]',
    'id = "fx-sync"',
    'name = "封面对齐（会删，夹具）"',
    'kind = "sync"',
    `src = ${P('\\mirror-src')}`,
    `dst = "${remoteName}:b"`,
    '',
    '[[job]]',
    'id = "fx-gate"',
    'name = "封面对账（夹具）"',
    'kind = "gate"',
    `src = ${P('\\local')}`,
    `dst = "${remoteName}:a"`,
    'manifest = { kind = "localdir" }',
    '',
  ].join('\n');
  w(path.join(home, 'jobs.toml'), jobs);
  // 字节基线：设置面每一步验收都要能证明"只改了被点名的那一行"。
  // `latin1` 是唯一能一个字节对一个字符来回搬的编码（utf-8 会把哨兵之外的字节合成多字节字符，
  // 而基线要比的是字节）。驱动那边用 `Buffer.from(s, 'latin1')` 还原后逐字节比。
  const base = {
    jobs: fs.readFileSync(path.join(home, 'jobs.toml')).toString('latin1'),
    conf: fs.readFileSync(path.join(home, 'conf', 'rclone.conf')).toString('latin1'),
  };
  w(path.join(home, 'settings-baseline.json'), JSON.stringify(base));
  console.log('HOME ' + home + '  exclude=' + JSON.stringify(opts.exclude) + '  bulk=' + opts.bulk);
  console.log('CONF_BYTES ' + Buffer.byteLength(base.conf) + '  JOBS_BYTES ' + Buffer.byteLength(base.jobs));
}

// 家目录**统一用 path.join 拼**（反斜杠）。原来这里是 `TMP + '/deck-ui-A'`，拼出的是
// `...\Temp/deck-ui-A`；而 `ui-launch.cmd` 给面板的 `RCLONE_DECK_HOME` 与
// `ui-acceptance.mjs` 算出来的都是 `...\Temp\deck-ui-A`。两者不是同一个字符串 ——
// 驱动器里「屏上这条路径 == 我算的那个家」的比对会全红，而那种红的读法是"界面坏了"，
// 其实只是斜杠方向不一样。三处必须同一种拼法。
if (want('A')) { const h = path.join(TMP, 'deck-ui-A'); build(h, 'fxa', path.join(h, 'bucket'), { exclude: ['*.log'], bulk: 20000 }); }
if (want('B')) { const h = path.join(TMP, 'deck-ui-B'); build(h, 'fxb', path.join(h, 'bucket'), { exclude: [], bulk: 4 }); }
console.log('BUILT');
