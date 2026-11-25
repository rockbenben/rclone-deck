import { App as AntdApp, Button, Layout, Segmented } from 'antd';
import { CloudSyncOutlined, FolderOpenOutlined, SettingOutlined, SwapOutlined } from '@ant-design/icons';
import { useCallback, useEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';

import * as api from './api';
import { errNote, noteKey, plainNote } from './errors';
import { formatFileChanges } from './format';
import { BrowserPanel } from './components/BrowserPanel';
import { JobContextPanel, TABS } from './components/JobContextPanel';
import { JobListView, PairCell, usePairChoice } from './components/JobListView';
import { MirrorConfirm } from './components/MirrorConfirm';
import { PairDraftLayer } from './components/PairDraftLayer';
import { KEY_FACE, KEY_TAB, useStored } from './prefs';
import { RcdStatusBar } from './components/RcdStatusBar';
import { SettingsPanel } from './components/SettingsPanel';
import { autoCheck } from './updates';
import type { GateReport, JobDraft, JobView, LogTail, PreviewView, PruneReport, RcdStatus, ShortNote, TaskState } from './types';

const POLL_MS = 1000;

/** `runJob` / `runMirror` 交回的是**终态数据**：rclone 失败时也是 `Ok(Failed)`。
 *  完成提示只认这一档，不能把「promise 没 reject」当成「做完了」。 */
function isDone(value: unknown): value is Extract<TaskState, { state: 'done' }> {
  return typeof value === 'object' && value !== null && (value as TaskState).state === 'done';
}

/** 顶栏下面那三面。显示条件一律按它**等值**判：两面共用"不是那一面就显示这一面"那种写法，
 *  第三面一出现就会与对子面同时显示（两面上各有一套按钮，点下去动的是看不见的地方）。 */
type View = 'pairs' | 'browser' | 'settings';

/** 那三档的**名单只有一份**：Segmented 摆的是它，持久层校验存的也是它。
 *  两处各抄一份，下一档加进来就会有一处忘改 —— 那一处的症状是"存了却开不出来"。 */
const FACES: { label: string; value: View; icon: ReactNode }[] = [
  { label: '文件任务', value: 'pairs', icon: <SwapOutlined /> },
  { label: '浏览文件', value: 'browser', icon: <FolderOpenOutlined /> },
  { label: '设置', value: 'settings', icon: <SettingOutlined /> },
];

/**
 * 面板本体：所有状态与回调都在这里，`useApp()` 也在这里——它在 `<AntdApp>` 的**里面**
 * （见 `App.tsx`），所以拿到的是真上下文，`message.error` 真的是函数（裁定 R57）。
 */
export function Panel() {
  const { notification } = AntdApp.useApp();
  /** 那句飘出来的话：**标题是人话，正文才是原文**。
   *  用 `notification` 而不是 `message` 是因为它有两段（title + description）——
   *  后台交出来的错误就是两段（`errNote`），塞进一根字符串就又回到"界面上只有 rclone 的原话"。
   *  它仍然来自 `useApp()`，所以 R57 那个形状没动（provider 在外、消费者在内）。 */
  const say = useCallback((label: string, note: ShortNote) => {
    notification.error({
      message: `${label}失败`,
      description: note.detail === '' ? note.short : `${note.short}\n${note.detail}`,
      duration: 10,
    });
  }, [notification]);
  const [status, setStatus] = useState<RcdStatus | null>(null);
  const [jobs, setJobs] = useState<JobView[]>([]);
  const [states, setStates] = useState<Record<string, TaskState>>({});
  const [logs, setLogs] = useState<Record<string, LogTail>>({});
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  // 顶栏下面那三档：**对子**（任务行 + 这一对目录的上下文）、**浏览器**（按名字动的双栏）、
  // **设置**（那两份文件）。只两档不是漏了一档：以前第三档「作业」的全部依据是 `job/list`
  // 的字段形状，而它没量过（spec §9），放一个空的第三档就是占位符——现在这第三档是有内容的。
  // 这一档**跨重开记住**（操作员那句「我常用设置怎么没保存」）—— 它是"你上次在看什么"，
  // 不碰任何东西。会删东西的那两格仍然不记，见 `prefs.ts` 上面那一段。
  const [view, setView] = useStored(KEY_FACE, FACES.map((f) => f.value), 'pairs');
  const [gate, setGate] = useState<Record<string, GateReport>>({});
  const [inventory, setInventory] = useState<Record<string, PruneReport>>({});
  // 右列现在翻到哪一段。它归这里管，不归 `JobContextPanel` 自己发明：卡上那颗「看差异」
  // 既要算差异、也要把右列翻到「差异」那一面。这一段也跟着上一件一起记住。
  const [railTab, setRailTab] = useStored(KEY_TAB, TABS.map((t) => t.value), 'progress');
  // 对齐那一对按钮的状态（spec §4.1）：`preview` 按任务存着最近一次预览的清单与 token，
  // `previewFor` 是确认层现在开在哪个任务上，`previewError` 留在层里说（不要只飘一条 toast 就完）。
  const [preview, setPreview] = useState<Record<string, PreviewView>>({});
  // 这一份确认**是否已经交出去了**。后台那份授权取走即消费，而且排在拿锁、复核、比对**之前**
  // （`Runner::run_mirror` 里那句"授权先吃掉"）——所以吃过一次拒绝的那个 token 再按一次，
  // 拿到的必然是同一句「这一份确认不作数」。浏览器那一层早就备着这一档（层不关、清单留着、
  // 那颗红的不亮），对齐这一层以前没有：屏幕上红按钮还亮着，等于邀请人再按一次注定的失败。
  const [previewSpent, setPreviewSpent] = useState<Record<string, boolean>>({});
  const [previewFor, setPreviewFor] = useState<string | null>(null);
  const [previewError, setPreviewError] = useState<ShortNote | null>(null);
  // 「新建临时任务」那一层开不开，以及刚立起来的那张卡要指到哪（`pairKey`，后台算的那把尺子）。
  // 归这里管是因为右列读的是 `selectedId`：新卡立在看不见的那一列里、右列还讲上一张卡的事，
  // 操作员就会对着两张卡按下一颗按钮。挑哪一格仍然由卡面那层自己说（见 `JobListView` 的 `pick`）。
  const [draftOpen, setDraftOpen] = useState(false);
  const [focusPair, setFocusPair] = useState<string | null>(null);
  const [rootsRevision, setRootsRevision] = useState(0);
  // 「存成常用」带进设置面那张**已有的**「新加一条」表单的那一格（R97）。
  // 这里一份文件都不写：写得进那条路只有设置面那三条（读现状 / 要差异 / 带凭证落盘），
  // 多一条就是第二份确认流程，而两份确认流程迟早有一份忘记问人。
  const [prefill, setPrefill] = useState<JobDraft | null>(null);
  // 轮询失败的「已经报过」门（裁定 R57 第三条）：这一条 catch 每秒跑一次，不加门就是一句拒绝刷屏。
  // 门里存的是**上一次那句原因**而不是一个布尔：同一个原因每秒重复不许再喊，但原因换了
  // （还是读不到，错却变成另一个）就必须重新说一句——用布尔的那版只在轮询成功后重新武装，
  // 于是同一段故障里第二句永远说不出口，操作员对着的是第一句，而那句已经不代表当下。
  const lastPollError = useRef<string | null>(null);
  /** 那一次失败的**内容**（ref 里存的是比较键）。顶栏靠它分"正在连接"与"还没连上"两档：
   *  `status` 只在轮询成功时才落，请求本身失败时它永远是 null —— 旧写法于是一直写着
   *  "正在连接后台…"，旁边还飘着一条说它失败的通知，同一屏上两句互相打脸。 */
  const [pollError, setPollError] = useState<ShortNote | null>(null);

  const refreshRound = useCallback(async () => {
    const statusRead = api.rcdStatus().then((value) => {
      setStatus(value);
      setPollError(null);
      return value;
    }, (error: unknown) => {
      setPollError(errNote(error));
      throw error;
    });
    const tasksRead = (async () => {
      const j = await api.getJobs();
      setJobs(j);
      const next: Record<string, TaskState> = {};
      const stateReads = await Promise.allSettled(j.map(async (job) => {
        // 状态**每个任务都要**：卡面那颗灰不灰、「正在写这棵目录树：X」那一句都读它。
        next[job.id] = await api.jobState(job.id);
      }));
      const failed = stateReads.find((result) => result.status === 'rejected');
      if (failed !== undefined) throw failed.reason;
      setStates(next);
      // 日志只读**选中那一个**：那一段尾巴只有右列用得上，而旧写法每一轮给每个任务
      // 都开一次文件读 8 KiB —— N 份文本里 N−1 份一次都没被看过（每秒都在付这个钱）。
      // 这一句挂在 `selectedId` 上，切卡会重建 `refresh`，于是"选中谁就立刻补读一次"
      // 是同一条依赖顺带做到的：翻到日志那一段时看到的不是上一秒的旧尾巴。
      if (selectedId !== null && j.some((job) => job.id === selectedId)) {
        const tail = await api.tailLog(selectedId);
        setLogs((prev) => ({ ...prev, [selectedId]: tail }));
      }
    })();
    const [statusResult, tasksResult] = await Promise.allSettled([statusRead, tasksRead]);
    const taskError = statusResult.status === 'fulfilled' && statusResult.value.ok && statusResult.value.alive
      && tasksResult.status === 'rejected' ? errNote(tasksResult.reason) : null;
    const note = statusResult.status === 'rejected' ? errNote(statusResult.reason) : taskError;
    if (note === null) {
      lastPollError.current = null;
      return;
    }
    const reason = noteKey(note);
    if (reason !== lastPollError.current) {
      lastPollError.current = reason;
      say(statusResult.status === 'rejected' ? '向后台要状态' : '读取任务', note);
    }
  }, [say, selectedId]);

  const latestRound = useRef(refreshRound);
  const inFlight = useRef<Promise<void> | null>(null);
  const refresh = useCallback(async () => {
    // An explicit refresh waits for the old round, then shares one fresh round.
    await inFlight.current;
    if (inFlight.current === null) {
      inFlight.current = latestRound.current().finally(() => { inFlight.current = null; });
    }
    await inFlight.current;
  }, []);

  useEffect(() => {
    latestRound.current = refreshRound;
    void refresh();
  }, [refreshRound, refresh]);

  useEffect(() => {
    const timer = window.setInterval(() => {
      if (inFlight.current === null) void refresh();
    }, POLL_MS);
    return () => window.clearInterval(timer);
  }, [refresh]);

  // 更新检查：默认**关着**（那一档存在 `src/updates.ts` 那一层里），所以这一段平时什么也不做。
  // 开了才在面板起来时问一次；问出什么由下面那条通知说——**不下载也不装**，
  // 下不下是设置面那张卡上「下载并安装」那一颗的事。
  useEffect(() => {
    void autoCheck().then((r) => {
      if (r.status !== 'available') return;
      notification.info({
        message: '有新版了',
        description: `版本 ${r.version}。到「设置」那面可以下载安装。`,
        duration: 10,
      });
    });
  }, [notification]);

  const act = useCallback(async <T,>(
    label: string,
    fn: () => Promise<T>,
  ): Promise<{ ok: true; value: T } | { ok: false }> => {
    setBusy(true);
    try {
      const value = await fn();
      await refresh();
      return { ok: true, value };
    } catch (e) {
      say(label, errNote(e));
      await refresh();
      return { ok: false };
    } finally {
      setBusy(false);
    }
  }, [say, refresh]);

  const selected = jobs.find((j) => j.id === selectedId) ?? null;

  /** 「站在哪一对位置 + 那一格是什么」这一份状态归这里，因为**选择条与当前那一格是两块屏**：
   *  它抬上来之后两处读的是同一份（以前它住在格那一个组件里，三枚框只能跟着格挤在 340px 的
   *  左列里，实测被折成三行）。判定仍然只有一处 —— 抬的是"谁持有"，不是"判几次"。 */
  const choice = usePairChoice(jobs, focusPair, setSelectedId);

  const notifyDone = useCallback((id: string, label: string, result: Extract<TaskState, { state: 'done' }>) => {
    const job = jobs.find((item) => item.id === id);
    notification.open({
      key: `result-${id}`,
      type: result.dry_run ? 'info' : 'success',
      title: result.dry_run ? '模拟完成，文件未改动' : `${label}完成`,
      description: formatFileChanges(result),
      duration: 8,
      actions: job && <Button size="small" onClick={() => {
        setView('pairs');
        choice.selectJob(job);
        setRailTab('progress');
        notification.destroy(`result-${id}`);
      }}>查看文件明细</Button>,
    });
  }, [jobs, notification, setView, setRailTab, choice.selectJob]);

  /** 卡上任何一颗按钮都**先把那张卡立起来**，再发请求。
   *  右栏读的是 `selected`：不选中的话，"开始"就在看不见的地方发生了 ——
   *  操作员只看得见所有按钮一起变灰（`busy` 是全局的），而右栏还在讲上一张卡的事。
   *  操作员报的就是这个。`openInventory`/`openMirror` 本来就自己选中，
   *  它们不走这里（各自还要顺带翻右列那一面）。 */
  const cardAct = useCallback(<T,>(id: string, label: string, fn: () => Promise<T>, done?: (value: T) => void) => {
    setSelectedId(id);
    void act(label, fn).then((r) => { if (r.ok) done?.(r.value); });
  }, [act]);

  const runTransfer = useCallback((id: string, dryRun: boolean) => {
    const label = dryRun ? '模拟复制' : '复制';
    notification.destroy(`result-${id}`);
    cardAct(id, label, async () => {
      const value = await api.runJob(id, dryRun);
      setStates((prev) => ({ ...prev, [id]: value }));
      return value;
    }, (value) => {
      if (isDone(value) && value.dry_run === dryRun) {
        notifyDone(id, label, value);
      } else if (value.state === 'failed') {
        say(label, value.error);
      } else if (value.state === 'unknown') {
        notification.warning({ message: `${label}结果未知`, description: value.reason, duration: 10 });
      }
    });
  }, [cardAct, notification, notifyDone, say]);

  /** 对账的两个入口（卡上那颗 + 右列那一段里那颗）走**同一条路**：选中、把右列翻到「对账」那一段、跑。
   *  跑完不翻面等于把报告掉在看不见的第二段里：四段以前只有盘点那一段会自己翻（`openInventory`），
   *  点完「开始对账」屏幕上一个字都不变，而结果确实在那儿——只是在另一段。
   *  试跑与传输**不翻**：它们的读数就住在进度那一段（`TaskState` 是它们的数据）。
   *  以前这两处是两份一模一样的闭包，现在是一份：一个动作两处实现就是两处会各自漂的话术。 */
  const runGate = useCallback((id: string) => {
    setSelectedId(id);
    setRailTab('check');
    void act('对账', async () => {
      const report = await api.runGate(id);
      setGate((g) => ({ ...g, [id]: report }));
    });
  }, [act]);
  // 确认层开在哪个任务上：`previewFor` 只是那个 id，层要的是任务行本身（名字与两侧路径 ——
  // 前提话术按方向分叉靠它）。现从 `jobs` 里找而不是存一份 `JobView`：每轮刷新都换成新对象。
  const previewJob = previewFor ? jobs.find((j) => j.id === previewFor) ?? null : null;

  // 点「看差异」：选中该任务、把右列翻到「差异」、列一遍。
  // 只读，没有随执行作废的清单要记（那一份归对齐确认层管，见下面 `preview`）。
  const openInventory = useCallback((id: string) => {
    setSelectedId(id);
    setRailTab('diff');
    void act('盘点', async () => {
      const report = await api.runInventory(id);
      setInventory((p) => ({ ...p, [id]: report }));
    });
  }, [act]);

  // 预览这一步只列不删。它自己捕获错误并落进确认层——拒绝（空 src、占比守卫、
  // 清单变了、这一对目录正被别的任务占着）要是只飘一条 toast 就被人在弹窗里看不见，
  // 那等于没有拒绝，而看不见的拒绝正是"以为没危险"的起点。所以「确认单向同步」不裸调它，
  // 走下面 `openMirror` 那条开层的路：清单与拒绝都必须在层里呈现出来。
  const doPreview = useCallback(async (id: string) => {
    setBusy(true);
    setPreviewError(null);
    try {
      // 先 `await` 再落 state：函数式更新器必须是同步的，`await` 写进它里面就是这里的语法错误
      // （同 `onGate` 那条注释说的同一件事）。
      const view = await api.previewMirror(id);
      setPreview((prev) => ({ ...prev, [id]: view }));
      // 新清单 = 新凭证：上一份"已经交出去了"那一档跟着作废，否则重列之后那颗还是灭的。
      setPreviewSpent((p) => ({ ...p, [id]: false }));
      await refresh();
    } catch (e) {
      // 失败的这一次没有清单，也没有可交出去的 token：把可能残留的旧那份丢掉。
      setPreview((prev) => { const n = { ...prev }; delete n[id]; return n; });
      setPreviewError(errNote(e));
    } finally {
      setBusy(false);
    }
  }, [refresh]);

  // 卡上那颗「确认单向同步」走的就是这里：**打开同步确认窗口**（开层不再顺手列一遍，见下面那三行）。
  // 它自己不删任何东西：层里没有 token 就没有可交出去的凭证。
  const openMirror = useCallback((id: string) => {
    setSelectedId(id);
    setPreviewFor(id);
    setPreviewError(null);
    // 开层**不再顺手列一遍**。以前这里 `void doPreview(id)`，于是按下「确认单向同步」
    // 先要等两侧各列举一次（真库上几十秒）才按得动红色那颗 —— 而那一趟不是担保，
    // 动手前那一次重列与那两道拒绝才是。想看逐条名单，层里那颗「生成删除清单」就是它，一秒不少。
  }, [doPreview]);

  /** 「存成常用」：把**当前这一格**的位置交进设置面那张已有的表单，并切到那一面（R97）。
   *  名字与编号留给操作员填：临时格的名字是我们起的（「临时（没存进配置）」），
   *  存进配置的那一条该有他自己认得的名字。这里一个字节都不写。 */
  const saveDraft = useCallback((job: JobView) => {
    setPrefill({
      id: '', name: '', jobKind: job.kind, src: job.src, dst: job.dst,
      openDir: job.openDir ?? '', manifest: false,
    });
    setView('settings');
  }, []);

  /** 「删掉这张临时卡」：把这一张卡上那几格临时的收掉。正在跑的时候后台会拒，并把为什么说出来；
   *  界面不设备用文案，也不因为"大概正忙着"就提前把那颗按钮灭掉。 */
  const foldPair = useCallback((pairKey: string) => {
    void act('收起这张卡', async () => {
      await api.dropDraft(pairKey);
      setRootsRevision((revision) => revision + 1);
      // 收完把"指哪一张"那条指令也撤了：同一对位置再立一次时拿的是同一把键，
      // 不撤就会因为"这张已经指过了"而不把右列指过去。
      setFocusPair(null);
    });
  }, [act]);

  const closeMirror = useCallback(() => {
    setPreviewFor(null);
    setPreviewError(null);   // 关掉之后不该在下一个任务的确认层里留下上一任务的拒绝
  }, []);

  return (
    <Layout className="deck-shell">
      <header className="deck-header">
        <div className="deck-brand">
          <span className="deck-mark" aria-hidden="true"><CloudSyncOutlined /></span>
          <span>rclone-deck</span>
        </div>
        <nav aria-label="工作区">
          <Segmented className="deck-nav" value={view} onChange={(v) => setView(v as View)} options={FACES} />
        </nav>
      </header>
      <div className="deck-status"><RcdStatusBar status={status} pollError={pollError} /></div>
      <main className="deck-workspace">
        {/* 切换时保留浏览位置。新建或收起草稿后更新可选位置。 */}
        <div style={{ display: view === 'pairs' ? undefined : 'none' }} className="pairs-layout">
          <h1 className="workspace-title">文件任务</h1>
          {/* 「选哪一对位置」横跨整窗，下面才是当前那一格与它的上下文。
              以前这两件事挤在同一个 340px 的左列里，三枚框加三行标签一行也放不下，被折成三行 ——
              一眼看过去像一张竖排的表，而它问的其实是"我在哪"。 */}
          <JobListView
            choice={choice}
            jobs={jobs}
            states={states}
            selectedId={selectedId}
            busy={busy}
            onSelect={(id) => setSelectedId(id)}
            onNewPair={() => setDraftOpen(true)}
            focusPair={focusPair}
            onSaveDraft={saveDraft}
            onFoldPair={foldPair}
            onTrialRun={(id) => runTransfer(id, true)}
            onStart={(id) => runTransfer(id, false)}
            onPreviewMirror={openMirror}
            onGate={runGate}
            onInventory={openInventory}
            onOpenDir={(id) => cardAct(id, '打开目录', () => api.openDir(id))}
          />
          <Layout hasSider className="pairs-split">
            <Layout.Sider width="100%" className="surface pairs-aside">
              <PairCell
                choice={choice}
                jobs={jobs}
                states={states}
                selectedId={selectedId}
                busy={busy}
                onSelect={(id) => {
                  setSelectedId(id);
                  setRailTab(choice.gates.some((job) => job.id === id) ? 'check' : 'progress');
                }}
                onNewPair={() => setDraftOpen(true)}
                focusPair={focusPair}
                onSaveDraft={saveDraft}
                onFoldPair={foldPair}
                onTrialRun={(id) => runTransfer(id, true)}
                onStart={(id) => runTransfer(id, false)}
                onPreviewMirror={openMirror}
                onGate={runGate}
                onInventory={openInventory}
                onOpenDir={(id) => cardAct(id, '打开目录', () => api.openDir(id))}
              />
            </Layout.Sider>
            <Layout.Content className="surface pairs-main">
              <JobContextPanel
                job={selected}
                state={selectedId ? states[selectedId] : undefined}
                log={selected ? logs[selected.id] ?? null : null}
                inventory={selected ? inventory[selected.id] ?? null : null}
                gate={selected ? gate[selected.id] ?? null : null}
                busy={busy}
                tab={railTab}
                onTab={setRailTab}
                onRunInventory={() => { if (selected) openInventory(selected.id); }}
                onRunGate={() => { if (selected) runGate(selected.id); }}
              />
            </Layout.Content>
          </Layout>
        </div>
        <div style={{ display: view === 'browser' ? 'block' : 'none' }}>
          <BrowserPanel rootsRevision={rootsRevision} />
        </div>
        {/* 第三面：那两份文件。它不轮询，也不在挂载之外自己发请求（读一次 = 读两份文件 +
            一次问 rclone 的只读探针），所以挂着不比藏起来更贵；切走再回来还站在原来那一格。
            「打开这个目录」接的是卡上「打开所在目录」同一条通路：失败了由 `act` 念出
            「打开目录失败 + 后台那句为什么」，而不是让这一面顶着一句不相干的「没改成」。 */}
        <div style={{ display: view === 'settings' ? 'block' : 'none' }}>
          <h1 className="workspace-title settings-title">设置</h1>
          <SettingsPanel active={view === 'settings'}
            prefill={prefill}
            onOpenFolder={() => void act('打开目录', () => api.openConfigFolder())} />
        </div>
      </main>
      {previewJob && (
        <MirrorConfirm
          job={previewJob}
          preview={preview[previewJob.id] ?? null}
          loading={busy}
          error={previewError}
          spent={previewJob ? previewSpent[previewJob.id] === true : false}
          onCancel={closeMirror}
          onPreview={doPreview}
          onConfirm={(id, token) => void act('同步', async () => {
            notification.destroy(`result-${id}`);
            let finished: TaskState;
            try {
              // 没名单那一档：凭证要现登记（发起点只有这一处，两处就有两处话术）。
              finished = await api.runMirror(id, token ?? await api.confirmMirror(id));
              setStates((prev) => ({ ...prev, [id]: finished }));
            } catch (e) {
              // 后台那句拒绝（清单变了 / 这份确认不作数 / 复核不过）要留在眼前这份清单上：
              // toast 自己会消失，而人正对着同一屏的清单决定要不要再来一次。
              setPreviewError(errNote(e));
              // 但这一份凭证确实已经花掉了（后台先吃掉再判），所以那颗红的不再亮着：
              // 再来一次的路只有「生成删除清单」。
              setPreviewSpent((p) => ({ ...p, [id]: true }));
              throw e;   // 仍然交给 act 报失败：那一条 toast 与 busy 的收尾是它管的。
            }
            setPreviewSpent((p) => ({ ...p, [id]: true }));
            if (!isDone(finished)) {
              setPreviewError(finished.state === 'failed' ? finished.error
                : finished.state === 'unknown' ? plainNote('执行结果未知', finished.reason)
                : plainNote('同步未完成，请重新生成删除清单。'));
              return finished;
            }
            closeMirror();
            // 这份确认已经交出去了（单次消费），留着它就是一份再也换不来任何东西的旧清单。
            setPreview((p) => { const n = { ...p }; delete n[id]; return n; });
            // 终态必须交回去给下面的 `.then` 判完成；丢了它，成功提示就永远读不到 Done。
            return finished;
          }).then((r) => { if (r.ok && isDone(r.value)) notifyDone(id, '同步', r.value); })}
        />
      )}
      {/* 「新建临时任务」那一层。立卡那一次调用与它带回来的那句拒绝都住在层里（同一屏上看得见，
          人才知道要改哪一格）——这里只接两件事：关掉它，并把右列指到刚立起来的那张卡。
          立刻刷新一次，不等那一秒的轮询：spec §3 要的是"同一拍里卡就出现"。 */}
      <PairDraftLayer open={draftOpen} onClose={() => setDraftOpen(false)}
        onCreated={(pairKey) => {
          setDraftOpen(false);
          setFocusPair(pairKey);
          setRootsRevision((revision) => revision + 1);
          void refresh();
        }} />
    </Layout>
  );
}
