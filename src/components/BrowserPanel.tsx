import { Alert, App as AntdApp, Button, Input, Modal, Space, Typography } from 'antd';
import { useCallback, useEffect, useRef, useState } from 'react';

import * as api from '../api';
import { errNote, plainNote } from '../errors';
import { BrowserPane } from './BrowserPane';
import { LayerConfirm } from './LayerConfirm';
import { NoteLine } from './NoteLine';
import type { BrowserEntry, BrowserListing, BrowserPreview, RemoteRef, RunReport, ShortNote } from '../types';

type Side = 'local' | 'remote';

const other = (side: Side): Side => (side === 'local' ? 'remote' : 'local');
const SIDE_TITLE: Record<Side, string> = { local: '本地目录', remote: '远端' };

/** 一栏站的位置：发进后台的那个 `fs`（后台给什么就传回什么，见 `RemoteRef.id`）+ 相对根的前缀。 */
interface PaneState { fs: string | null; prefix: string; listing: BrowserListing | null }

const EMPTY_PANE: PaneState = { fs: null, prefix: '', listing: null };

/** 相对名的唯一一处拼接。两个输入都是后台交出的形状：`dir` 是这一栏的导航状态（它的每个片段
 *  都来自清单行的 `name`），`name` 是行上那个完整相对名或操作员敲进去的一段。
 *  这里只做"落在谁下面"这一件事——不补冒号、不改斜杠、不认端点，那些判据都在后台那一处。 */
function under(dir: string, name: string): string { return dir === '' ? name : `${dir}/${name}`; }
function baseOf(name: string): string { const i = name.lastIndexOf('/'); return i < 0 ? name : name.slice(i + 1); }

/** 一次动作的结果，记着**是谁发起的**：它要落回那一栏脚下（上一版它是页面级一条 Alert，
 *  掉在视口外，于是有一格真缺陷混过了两轮评审）。 */
type Result = { side: Side; report: RunReport } | { side: Side; refusal: ShortNote };

/** 确认层开着的那一份。`make` 就是"再看一次"要调的那一句：第一次打开与「重新预览」走同一条路，
 *  所以清单与拒绝一定出自同一个入口、同一句话。 */
interface Layer {
  side: Side;
  make: () => Promise<BrowserPreview>;
  preview: BrowserPreview | null;
  /** 这一份是在删东西（页脚那颗按钮要说对动词），还是在搬东西。 */
  deletion: boolean;
  /** 非空 = 这一次是改名来的，撞名之后要能就地换一个名字再来一遍。 */
  renaming: { side: Side; src: string; typed: string } | null;
  /** token 已经被吃掉一次（动手前被拒也算）：这一份不能再按第二次。
   *  后台先消费凭证再拿锁，所以按第二次换来的不是同一个拒绝，是一句新的「不作数」。 */
  spent: boolean;
}

interface Naming { side: Side; kind: 'mkdir' | 'rename'; src: string; value: string }

/** 后台那两段的"没有那一句"。界面不许替它编一句备用文案（那是从"能不能"反推"为什么"）。 */
const EMPTY_NOTE: ShortNote = { short: '', detail: '' };

/**
 * 编排层：两栏的状态、跨栏的源（`picked`）、确认层与命名框都归这里。
 * 一栏长什么样在 `BrowserPane`，确认层长什么样在 `LayerConfirm` —— 这一面只决定"谁把什么交给谁"。
 */
export function BrowserPanel() {
  const { notification } = AntdApp.useApp();
  /** 与 `Panel.tsx` 那个同形状：标题是人话，正文才是原文（后台交出来的两段，见 `errNote`）。
   *  仍然走 `useApp()`，R57 那个 provider 形状没动。 */
  const say = useCallback((label: string, note: ShortNote) => {
    notification.error({
      message: label,
      description: note.detail === '' ? note.short : `${note.short}\n${note.detail}`,
      duration: 10,
    });
  }, [notification]);
  const [refs, setRefs] = useState<RemoteRef[]>([]);
  // 取不到位置时那句要**留在屏幕上**并且给一条重试的路：这一面挂着之后只在挂载时问过一次的形状，
  // 冷启动那一下 rcd 还没起来，两个下拉框就会永远空着 —— 而"这台机器上没有位置"与"刚才没问到"
  // 在一条已经消失的 toast 之后长得一模一样（与 `listErrors` 同一族理由）。
  const [refsError, setRefsError] = useState<ShortNote | null>(null);
  // 那颗「重新取一次」在请求回来之前要按住：连点会攒出一把并发请求，而先发的失败可能
  // 后到 —— 已经问成功的结果又被旧错误盖回屏幕上。
  const [refsLoading, setRefsLoading] = useState(false);
  const [panes, setPanes] = useState<Record<Side, PaneState>>({ local: EMPTY_PANE, remote: EMPTY_PANE });
  /** 跨栏动作（上传到这里 / 移到这里）的源：操作员在某一栏点中的那一个文件。 */
  const [picked, setPicked] = useState<{ side: Side; name: string } | null>(null);
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<Result | null>(null);
  const [layer, setLayer] = useState<Layer | null>(null);
  const [layerError, setLayerError] = useState<ShortNote | null>(null);
  const [naming, setNaming] = useState<Naming | null>(null);
  const [namingError, setNamingError] = useState<ShortNote | null>(null);
  // 一次列不出来、一次被拒，那句原因要**留在屏幕上**：那条 toast 十几秒就自己没了（`duration: 10`），
  // 而那时界面剩下的
  // 是一栏空的表 —— "这一层是空的"与"这一层列不出来"在只有一条 toast 的界面里长得一模一样。
  const [listErrors, setListErrors] = useState<Record<Side, ShortNote | null>>({ local: null, remote: null });
  // 动作跑完之后刷新两栏，用的是**当下**站的位置；回调里那份 `panes` 是这次点击开始时的旧副本。
  const panesRef = useRef(panes);
  panesRef.current = panes;

  /** 动成了一次 = 上一次那句「没改成」就作废。少这一步，屏幕会在刚删完东西之后说什么都没改成。 */
  const commit = useCallback((side: Side, r: RunReport) => {
    // 点中的那一个可能已经被这次动作搬走或删掉了：留着它，对面那颗「上传到这里」/「移到这里」
    // 就亮着并指向一份已经不存在的清单（后台会拦，但那一格界面自己说不清）。
    setPicked(null);
    setResult({ side, report: r });
  }, []);

  const refused = useCallback((side: Side, reason: ShortNote) => setResult({ side, refusal: reason }), []);

  const loadRefs = useCallback(() => {
    setRefsLoading(true);
    api.browserRemotes()
      .then((r) => { setRefs(r); setRefsError(null); })
      // 两栏的端点都由后台交出，取不到就是没有：这里不猜，也不拿任务里那两条路径凑一份。
      // 但"没问到"要说在屏幕上，并且留一条重新问一次的路。
      .catch((e) => setRefsError(errNote(e)))
      .finally(() => setRefsLoading(false));
  }, []);

  useEffect(() => { loadRefs(); }, [loadRefs]);

  /** 换位置 = 重新问一次那一层。列不出来要把原因留在原地，不许读成"这一层是空的"。 */
  const go = useCallback(async (side: Side, next: PaneState) => {
    setPanes((p) => ({ ...p, [side]: { ...next, listing: null } }));
    if (next.fs === null) return;
    try {
      const listing = await api.browserList(next.fs, next.prefix);
      setPanes((p) => ({ ...p, [side]: { ...next, listing } }));
      setListErrors((p) => ({ ...p, [side]: null }));
    } catch (e) {
      // 后台那句原文（远端名打错的 500、点段那句）留在这栏杆上，清单那一格明确是空的。
      // 两条都说一遍是有意的：栏上那句是"这一栏现在没有读数"的常驻解释，toast 是给
      // 焦点在别处（另一栏、确认层里）的操作员的现场提示 —— 少任何一条都会让"列不出来"
      // 与"这一层是空的"在界面上长成同一个样子。
      const note = errNote(e);
      setListErrors((p) => ({ ...p, [side]: note }));
      setPanes((p) => ({ ...p, [side]: { ...next, listing: null } }));
      say(`${SIDE_TITLE[side]}那一栏列不出来`, note);
    }
  }, [say]);

  /** 动过东西之后两栏都重新问一次：作废是后台的事，这里的清单只是它的一份副本。 */
  const reloadBoth = useCallback(async () => {
    const now = panesRef.current;
    await Promise.all([go('local', now.local), go('remote', now.remote)]);
  }, [go]);

  /** 不发凭证的动作（建目录、下载、上传）：跑完刷新两栏，结果落在那句报告里。 */
  const plain = useCallback(async (side: Side, label: string, fn: () => Promise<RunReport>) => {
    setBusy(true);
    try {
      commit(side, await fn());
      await reloadBoth();
    } catch (e) {
      // 后台那两段照原样交出去（短的那句给人看，原文挂在「为什么？」后面），并且留在那一栏脚下。
      const note = errNote(e);
      refused(side, note);
      say(`${label}失败`, note);
    } finally {
      setBusy(false);
    }
  }, [commit, say, refused, reloadBoth]);

  /** 打开确认层。**拒绝也要开层**：层没打开时，那句拒绝落进一个没挂载的地方，等于没有拒绝。 */
  const openLayer = useCallback(async (side: Side, make: () => Promise<BrowserPreview>, deletion: boolean,
                                       renaming: { side: Side; src: string; typed: string } | null) => {
    setBusy(true);
    setLayerError(null);
    const base: Layer = { side, make, preview: null, deletion, renaming, spent: false };
    setLayer(base);
    try {
      setLayer({ ...base, preview: await make() });
    } catch (e) {
      setLayerError(errNote(e));
    } finally {
      setBusy(false);
    }
  }, []);

  const closeLayer = useCallback(() => {
    setLayer(null);
    setLayerError(null);
  }, []);

  /** 唯一的动手入口：交出去的是这一份预览的一次性 token（R78）。界面上没有第二条"直接删"的路。 */
  const confirmLayer = useCallback(async () => {
    const l = layer;
    if (l === null || l.preview === null) return;
    const p = l.preview;
    const side = l.side;
    setBusy(true);
    setLayerError(null);
    try {
      commit(side, await api.browserRun(p.token));
      closeLayer();
      await reloadBoth();
    } catch (e) {
      // **不关层**：那句拒绝要说在操作员正对着的这份清单上；撞名那一次还要能改名字再来一遍。
      // 但这份凭证确实已经交出去了（后台先消费再拿锁），所以那颗红按钮不再亮着。
      setLayer((l) => (l === null ? l : { ...l, spent: true }));
      const note = errNote(e);
      setLayerError(note);
      refused(side, note);
    } finally {
      setBusy(false);
    }
  }, [closeLayer, commit, layer, refused, reloadBoth]);

  const submitNaming = useCallback(async () => {
    const n = naming;
    if (n === null) return;
    const pane = panes[n.side];
    const here = pane.fs;
    const value = n.value.trim();
    if (here === null) return;
    if (value === '') {
      // 空名字与"带 /"是同一类输入问题，走同一个出口（那只框下面已经有的错误槽）。
      // 以前这一支是**静默 return**：那颗「就这么定」灰着、一个字都不说，而带 / 那一档会说——
      // 同一个输入框两种待遇。`DESIGN.md` 那条"每个'点不动'都要读得出为什么"在这里的
      // 合法形状就是这个错误槽（判据在界面手里，所以句子也在界面手里，见 `plainNote` 那段）。
      setNamingError(plainNote('名字还是空的。'));
      return;
    }
    if (n.kind === 'mkdir' && value.includes('/')) {
      // 框里要的是一个目录名，不是一条路径：带 / 的写法会把东西建到别的那一层去，
      // 而下面那句回显说的是「建在这里」。明面那一格只留这七个字，条件走「为什么？」。
      setNamingError(plainNote('目录名里不要带 /', '要建在别的那一层，请先回到那一层再建。'));
      return;
    }
    setBusy(true);
    setNamingError(null);
    try {
      if (n.kind === 'mkdir') {
        commit(n.side, await api.browserMkdir(here, under(pane.prefix, value)));
        setNaming(null);
        await reloadBoth();
      } else {
        // 改名就是"换一个名字的**移动**"，所以它走凭证，不开第二条写路径（R78）。
        // 框里就是那个对象在这一侧根内的**完整相对名**：动最后一段就是就地改名，动前面几段
        // 就是搬去别的层（那也是一种移动，一样走凭证与撞名预检）。只填最后一段的话，
        // 清单里出现 `sub/c.webp` 时提交会算回同一个名字 —— 一次什么都不动的"成功"。
        const dst = value;
        if (dst === n.src) { setNamingError(plainNote('名字一个字都没变', '这一趟没有要动的东西。')); return; }
        setNaming(null);
        await openLayer(n.side, () => api.browserPreviewMove(here, n.src, here, dst), false,
          { side: n.side, src: n.src, typed: value });
      }
    } catch (e) {
      // 名字留在框里，改一改就能再来一次——那句拒绝要说在这儿，不能只飘一条 toast。
      setNamingError(errNote(e));
    } finally {
      setBusy(false);
    }
  }, [commit, naming, openLayer, panes, reloadBoth]);

  /** 结果只回给发起它的那一栏。`hasList` 那一档不出现那两个数（把没核对过演成核对通过，
   *  是这一面最贵的一种说错话）；底色只读后台的 `agrees`，这里不比那两个数。 */
  const resultFor = (side: Side) => {
    if (result === null || result.side !== side) return null;
    if ('refusal' in result) {
      return { tone: 'refused' as const, title: '没改成', detail: <NoteLine note={result.refusal} />, logPath: '' };
    }
    const report = result.report;
    return {
      tone: report.agrees ? ('ok' as const) : ('pending' as const),
      // 口径表第一档：「做完了 · <动作>：<完整位置>」（设置面那条早照做了；`frontend_settings.rs`
      // 钉的是设置面，这一面以前漏了那一顶 —— 三档词表在 ResultDock 头上）。两个数没对上那一档
      // 不抢"做完了"这句：它由下面那半句去说，标题不替后台做判定。
      title: report.agrees
        ? `做完了 · ${report.action}：${report.target}`
        : `${report.action}：${report.target}`,
      detail: (
        <>
          {report.hasList && (
            <Typography.Text className="fs-note">
              清单上 {report.confirmed} 个，后台报做完 {report.done} 个，
              {report.agrees ? '两个数一致。' : '两个数不一致——这一次以日志为准。'}
            </Typography.Text>
          )}
          {report.note !== '' && <div><Typography.Text className="fs-note">{report.note}</Typography.Text></div>}
        </>
      ),
      logPath: report.logPath,
    };
  };

  /** 一栏摆什么、点不动的那句原因、结果那一格——都在这里算好交下去：`BrowserPane` 不参与判定。 */
  const viewFor = (side: Side) => {
    const pane = panes[side];
    const far = panes[other(side)];
    const listing = pane.listing;
    const here = pane.fs;
    const ready = listing !== null && here !== null;
    // 每一颗按钮的"点不动"都要有一句原因（R54），而那些原因全是**这一面自己**的状态；
    // 清空那一颗例外：判定与句子都在后台一次算完（`clear_gates` 交出的两个字段）。
    // 灰读布尔、Tooltip 读句子，**两者互不推断** —— 从"能不能"反推"为什么"等于假设
    // "不能只可能是因为规则"，而根那一档同样是不能。
    // 落点那一侧必须**列得出来**：一栏标着「这一栏列不出来」的位置，不是一个能说清落点的位置。
    const farReady = far.listing !== null && far.fs !== null;
    const incoming = picked !== null && picked.side !== side ? picked : null;
    // 四句都是 `ShortNote`：明面那一格只放"去哪儿"，条件（列不列得出来）走 Tooltip 的第二个字。
    const hints = {
      download: ready && farReady ? EMPTY_NOTE
        : plainNote(`先在「${SIDE_TITLE[other(side)]}」那一栏选个位置`, '那一栏要列得出东西，落点才说得清。'),
      push: incoming === null || !ready ? plainNote('先在对面那一栏点中一个文件') : EMPTY_NOTE,
      move: picked === null || !ready ? plainNote('先在任意一栏点中一个文件') : EMPTY_NOTE,
      visible: ready ? EMPTY_NOTE : plainNote('先选一个位置，等这一层列出东西'),
    };
    return {
      title: SIDE_TITLE[side],
      isRemote: side === 'remote',
      rootLabel: refs.find((r) => r.id === here)?.label ?? '',
      choices: refs.filter((r) => (side === 'local' ? r.isLocal : !r.isLocal))
        .map((r) => ({ label: r.label, value: r.id })),
      fs: pane.fs,
      prefix: pane.prefix,
      listing,
      listError: listErrors[side],
      busy,
      pickedName: picked !== null && picked.side === side ? picked.name : null,
      hints,
      // 点不动那两句原样交下去（两段都在后台拼好）；没选位置时它自己会空，界面上就不挂 Tooltip。
      clearNote: listing !== null && here !== null ? listing.clearNote : EMPTY_NOTE,
      clearBlocked: !ready || !listing.canClearPrefix,
      result: resultFor(side),
    };
  };

  /** 这一栏上的动作：全部在这里绑好 `side` 与落点，`BrowserPane` 只管叫谁。
   *  每个处理器里都**再判一次位置**：判据写在处理器里，不靠按钮今天恰好 `disabled` ——
   *  前者是结构上不给第二种答案的写法，后者只是这一面此刻给的保护。 */
  const actionsFor = (side: Side) => {
    const pane = panes[side];
    const far = panes[other(side)];
    const here = pane.fs;
    const srcPane = picked === null ? null : panes[picked.side].fs;
    const incoming = picked !== null && picked.side !== side ? picked : null;
    return {
      onSelectPosition: (fs: string) => { setPicked(null); void go(side, { fs, prefix: '', listing: null }); },
      onJump: (prefix: string) => void go(side, { ...pane, prefix }),
      onEnter: (name: string) => void go(side, { ...pane, prefix: name }),
      onPick: (name: string | null) => setPicked(name === null ? null : { side, name }),
      onMkdir: () => { setNamingError(null); setNaming({ side, kind: 'mkdir', src: '', value: '' }); },
      onRename: (row: BrowserEntry) => {
        setNamingError(null);
        setNaming({ side, kind: 'rename', src: row.name, value: row.name });
      },
      onPush: () => {
        if (here === null || incoming === null || srcPane === null) return;
        const drop = under(pane.prefix, baseOf(incoming.name));
        void plain(side, '上传到这里', () => api.browserCopy(srcPane, incoming.name, here, drop));
      },
      onMoveHere: () => {
        if (here === null || picked === null || srcPane === null) return;
        const drop = under(pane.prefix, baseOf(picked.name));
        void openLayer(side, () => api.browserPreviewMove(srcPane, picked.name, here, drop), false, null);
      },
      onDownload: (row: BrowserEntry) => {
        const there = far.fs;
        if (here === null || there === null || far.listing === null) return;
        const drop = under(far.prefix, baseOf(row.name));
        void plain(side, '下载', () => api.browserCopy(here, row.name, there, drop));
      },
      onDeleteOne: (row: BrowserEntry) => {
        if (here === null) return;
        void openLayer(side, () => api.browserPreviewDelete(here, pane.prefix, [row.name]), true, null);
      },
      onDeleteVisible: () => {
        if (here === null || pane.listing === null) return;
        void openLayer(side, () => api.browserPreviewDelete(here, pane.prefix, []), true, null);
      },
      onPurge: () => {
        if (here === null || pane.listing === null || !pane.listing.canClearPrefix) return;
        void openLayer(side, () => api.browserPreviewPurge(here, pane.prefix), true, null);
      },
      onRmdir: (row: BrowserEntry) => {
        if (here === null) return;
        // 也走确认层：删空目录以前是"点一下就删"，那是这一面上唯一一条不经过清单与凭证的
        // 删除路径（R78 那句话在它身上是假的）。空不空由后台在发凭证之前判，非空根本发不出来。
        void openLayer(side, () => api.browserPreviewRmdir(here, row.name), true, null);
      },
    };
  };

  const p = layer?.preview ?? null;

  return (
    <Space orientation="vertical" size={12} style={{ width: '100%' }}>
      <Typography.Text type="secondary" className="fs-note">
        这一面只有动作，没有命令行：点中哪个名字，就动哪个名字。删东西的那几颗先列清单，
        确认交出去的是一次性的凭证，后台动手前还要再看一次。
      </Typography.Text>
      {refsError !== null && (
        <Alert showIcon type="error" title="取不到可以打开的位置" description={
          <Space orientation="vertical" size={4}>
            <NoteLine note={refsError} />
            <Button size="small" disabled={refsLoading} onClick={loadRefs}>重新取一次</Button>
          </Space>
        } />
      )}
      <div style={{ display: 'flex', gap: 16, alignItems: 'flex-start' }}>
        <div className="surface" style={{ flex: 1, minWidth: 0, padding: 12 }}>
          <BrowserPane {...viewFor('local')} {...actionsFor('local')} />
        </div>
        <div className="surface" style={{ flex: 1, minWidth: 0, padding: 12 }}>
          <BrowserPane {...viewFor('remote')} {...actionsFor('remote')} />
        </div>
      </div>

      <LayerConfirm
        open={layer !== null}
        busy={busy}
        preview={p}
        deletion={layer?.deletion ?? true}
        canRenameAgain={(layer?.renaming ?? null) !== null}
        spent={layer?.spent ?? false}
        error={layerError}
        onRepreview={() => { if (layer) void openLayer(layer.side, layer.make, layer.deletion, layer.renaming); }}
        onCancel={closeLayer}
        onConfirm={() => void confirmLayer()}
        onRenameAgain={() => {
          const r = layer?.renaming ?? null;
          if (r === null) return;
          // 那句拒绝带进下一个框里：换了名字也要看得见上一句为什么没过。
          // 位置信息原样带回去，名字留着让人改。
          setNaming({ side: r.side, kind: 'rename', src: r.src, value: r.typed });
          setNamingError(layerError);
          closeLayer();
        }}
      />

      <Modal open={naming !== null} width={480} title={naming?.kind === 'mkdir' ? '新建目录' : '改名'}
        onCancel={() => setNaming(null)} onOk={() => void submitNaming()}
        okText="就这么定" cancelText="取消"
        okButtonProps={{ disabled: busy || (naming?.value.trim() ?? '') === '' }}>
        <Space orientation="vertical" size={8} style={{ width: '100%' }}>
          <Input value={naming?.value ?? ''} placeholder={naming?.kind === 'mkdir' ? '新目录的名字' : '新名字'}
            onChange={(e) => setNaming((n) => (n ? { ...n, value: e.target.value } : n))}
            onPressEnter={() => void submitNaming()} />
          {/* 回显分两条：建目录那句说的就是**落点**，名字空着它也成立；改名那句是
              `旧名字 → 新名字`，新名字还没填就摆出一个空尾巴（`a.webp → `）是在骗人，
              所以那一整句等到有值才出现。 */}
          {naming && naming.kind === 'mkdir' && (
            <Typography.Text type="secondary" className="fs-note">
              新目录落在：{panes[naming.side].listing?.path ?? '（还没选位置）'}
            </Typography.Text>
          )}
          {naming && naming.kind === 'rename' && naming.value.trim() !== '' && (
            <Typography.Text type="secondary" className="fs-note">
              名字就是它在这一侧里的位置：{naming.src} → {naming.value}
            </Typography.Text>
          )}
          {namingError && <Alert type="error" showIcon title="没改成" description={<NoteLine note={namingError} />} />}
        </Space>
      </Modal>
    </Space>
  );
}
