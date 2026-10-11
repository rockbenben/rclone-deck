import { Alert, Button, Card, Input, Listy, Modal, Select, Space, Table, Typography } from 'antd';
import { useCallback, useEffect, useState } from 'react';
import type { ReactNode } from 'react';

import * as api from '../api';
import { groupDraftJobs, intentOptions } from '../pairs';
import type { PairGroup } from '../pairs';
import { errNote } from '../errors';
import { NoteLine } from './NoteLine';

import type {
  Applied, ConfigChange, ConfigIntent, JobDraft, JobRow, RemoteBlock, SecretState, SettingsPlan, SettingsView, ShortNote,
} from '../types';
import { KIND_LABEL } from '../types';

/**
 * 这一条在卡上那一格里说的是哪一句 —— **与 `JobListView` 同一份**。
 *
 * 做法是把这一条填进它所属那一对的 `PairGroup`（后台带来的四个槽位只填它自己那一格，
 * 与 `groupJobs` 的填法一样），再调 `intentOptions`。所以设置这一面读到的
 * 就是卡上那一格的那句原话，不存在「设置一套说法、卡上另一套」。
 *
 * `group` 里其余三格留 `null` 是有意的：`intentOptions` 只用它算**这一条**的标签，
 * 而 `availableSlots` 会因此只列出这一条占的那一格 —— 列出来的正是这一条自己，
 * 不是那一对位置上所有能做的事（那几件在卡上是同一个选择框里的兄弟选项）。
 */
function intentPhraseFor(
  groups: { pairs: { group: PairGroup; jobs: JobRow[] }[] },
  j: JobRow,
): string {
  const entry = groups.pairs.find((p) => p.jobs.some((x) => x.id === j.id));
  if (entry === undefined) return j.name;   // 找不到所属那一对：交原名，不编一句
  const g: PairGroup = { ...entry.group };
  // 槽位类型是 `JobView`，而 `intentOptions` 只判它是不是 `null`（`slotJob` 返回
  // `JobView | null`，用的是 `=== null` 不是读字段）。这里放一个**空壳**占位：
  // 这一段要算的是「这一条属于哪一格、那一格怎么说」，不需要那条任务的任何字段。
  const placeholder = { id: j.id } as unknown as import('../types').JobView;
  // `mode` 由后台那一列判（copy=不删、sync=会删），槽位是这一条自己在配置里的那一格。
  if (j.jobKind === 'sync') {
    if (j.pairSlot === 'a') { g.mirrorA = placeholder; } else { g.mirrorB = placeholder; }
  } else if (j.pairSlot === 'a') { g.addA = placeholder; } else { g.addB = placeholder; }
  const opts = intentOptions(g);
  return opts.length > 0 ? opts[0].label : j.name;
}

/** 卡片一的那四项（一行一项的两个数组按数组交）。 */
type DefaultsDraft = { rcloneBin: string; logDir: string; flags: string[]; exclude: string[] };
/** 一块远端卡片：那四项。空串 = 不要那一行。 */
type RemoteDraft = { remoteType: string; provider: string; endpoint: string; region: string };
/** 正在编辑的那一条任务；`isNew` 决定发出去的是「新加一条」还是「改这条」。 */
type JobEdit = JobDraft & { isNew: boolean };

function defaultsOf(v: SettingsView): DefaultsDraft {
  return {
    rcloneBin: v.defaults.rcloneBin,
    logDir: v.defaults.logDir,
    flags: [...v.defaults.flags],
    exclude: [...v.defaults.exclude],
  };
}

function remoteOf(block: RemoteBlock): RemoteDraft {
  const read = (key: 'type' | 'provider' | 'endpoint' | 'region') =>
    block.fields.find((f) => f.key === key)?.value ?? '';
  return { remoteType: read('type'), provider: read('provider'), endpoint: read('endpoint'), region: read('region') };
}

/**
 * 一条任务里**能改的那七项**。写盘发的是这一份（`askJob`），比"改了还没保存"比的也是这一份 ——
 * 两处各挑一遍字段，漏一项的症状是"改了而这一面说没改"（那句"有 N 处还没保存"就此谎报）。
 */
function jobFields(j: JobDraft) {
  return {
    id: j.id, name: j.name, jobKind: j.jobKind,
    src: j.src, dst: j.dst, openDir: j.openDir, manifest: j.manifest,
  };
}

/** 「新加一条」刚打开时那份空表：新那条算不算"改了"，比的是它，不是空字符串
 *  （拿"每一项都非空"当判据会把 `jobKind` 的默认值也算成一次改动）。
 *  类型挂在 `jobFields` 的返回值上：那七项加一项，这里编译器就逼着补一项。 */
type JobFields = ReturnType<typeof jobFields>;
const BLANK_JOB: JobFields = {
  id: '', name: '', jobKind: 'copy', src: '', dst: '', openDir: '', manifest: false,
};

function labelOf(block: RemoteBlock, key: 'type' | 'provider' | 'endpoint' | 'region', fallback: string): string {
  return block.fields.find((f) => f.key === key)?.label ?? fallback;
}

/** 一行差异的"改成了什么"。三档的**判定**在后台（`kind` 是它算的），这里只有排版：
 *  「原来没有这一行」与「这次要拿走这一行」是渲染选择，不是新判定。 */
function rowOf(c: ConfigChange): string {
  if (c.kind === 'added') return `${c.after}（原来没有这一行）`;
  if (c.kind === 'removed') return `${c.before}（这次要拿走这一行）`;
  return `${c.before}  →  ${c.after}`;
}

/** 这一次要的是"拿走整块"还是"改几行"：只读后台给的那份差异的形状，界面不另判一次。
 *  这里不数行数：后台在差异为空时根本发不出凭证，所以能开到这一层就至少有一行。 */
function isRemoval(plan: SettingsPlan | null): boolean {
  return plan !== null && plan.changes.every((c) => c.kind === 'removed');
}

/**
 * 第三面：设置（2026-10-09 三面改名之后这一面是地方名；面里那张卡片仍叫「改设置」，
 * `DESIGN.md` 钉着它连同写盘那条「做完了 · 改设置：」，那是"正在做什么"而不是"这里是哪一面"）。
 * 三块卡片自上而下（这台机器上 / 你的远端 / 任务）。
 *
 * 这一面与另外两面的根本不同是它写的是操作员磁盘上的两份文件，而那些文件里满纸**理由注释**。
 * 所以这里的每条界面规矩都指向同一件事——按下去之前先看清"这次改哪几行"：
 *
 * - 差异行、拒绝句、连带说明全部由后台作为数据交出来，界面不比较两个值也不数行数；
 * - 保存 = 先要差异（一次 plan），确认层逐行列出，再凭那份一次性凭证落盘（一次 apply）；
 * - 密钥那几项没有输入框，屏上只有「已配置，不回显」与一条可复制的出路（见 `SecretCell`）；
 * - 写完不热重载：「重开面板后生效。」是后台那句，不是界面替它编的。
 *
 * 写法上有一条自律：**那颗按钮的名字必须是 JSX 文本，而它前面那个属性里不许有内联的
 * 带花括号的箭头**。文本证人的读法是把 `>` 到下一个 `<` 之间当文字，内联箭头会让那一段
 * 被当成表达式（`{ kind: … }` 里只有键名，按钮上那句人话对证人就隐身了）。上一版就是这么
 * 让「改这条」隐了身的。处理器一律提成具名的小函数，文案留在文字里，顺序也留在屏上的顺序里。
 */
export function SettingsPanel(props: {
  active: boolean;
  /** 从那张临时卡带过来的一格（R97「存成常用」）。它只**填进这张已有的表单**，
   *  落盘走的还是这里那三条：读现状 / 要差异 / 带凭证落盘 —— 界面上不存在第二条写得进的路。 */
  prefill: JobDraft | null;
  onOpenFolder: () => void;
}) {
  const [view, setView] = useState<SettingsView | null>(null);
  const [defaults, setDefaults] = useState<DefaultsDraft | null>(null);
  const [remotes, setRemotes] = useState<Record<string, RemoteDraft>>({});
  const [job, setJob] = useState<JobEdit | null>(null);
  const [plan, setPlan] = useState<SettingsPlan | null>(null);
  // 要差异或落盘被拦下时那句：层里留着（toast 自己会消失，而人正对着同一屏的差异决定要不要再来一次），
  // 而**没有层可开**的那一档落在卡片脚下。
  const [error, setError] = useState<ShortNote | null>(null);
  // 这一份凭证已经交出去了（后台取走即消费，而且排在重新比对之前）：层不关、差异留着、那颗不再亮。
  const [spent, setSpent] = useState(false);
  const [result, setResult] = useState<Applied | null>(null);
  const [busy, setBusy] = useState(false);

  const load = useCallback(async () => {
    setBusy(true);
    try {
      const v = await api.settingsCurrent();
      setView(v);
      setDefaults(defaultsOf(v));
      const next: Record<string, RemoteDraft> = {};
      for (const block of v.remotes) next[block.name] = remoteOf(block);
      setRemotes(next);
      setError(null);
    } catch (e) {
      // 读不出来就不摆半张面：那一档是「这一栏列不出来」，细节走「为什么？」。
      setError(errNote(e));
      setView(null);
    } finally {
      setBusy(false);
    }
  }, []);

  // 只在这一面被点亮、且还没读过时读一次：那三条动作读的是磁盘文件与一次 rclone 探针，
  // 不该跟着每秒一次的轮询跑。
  useEffect(() => {
    if (props.active && view === null && !busy) void load();
  }, [props.active, view, busy, load]);

  const ask = useCallback(async (intent: ConfigIntent) => {
    setBusy(true);
    setError(null);
    setSpent(false);
    try {
      setPlan(await api.settingsPlan(intent));
    } catch (e) {
      setPlan(null);
      setError(errNote(e));
    } finally {
      setBusy(false);
    }
  }, []);

  const commit = useCallback(async () => {
    if (plan === null) return;
    setBusy(true);
    try {
      const applied = await api.settingsApply(plan.token);
      setResult(applied);
      setPlan(null);
      setJob(null);
      await load();
    } catch (e) {
      setError(errNote(e));
      setSpent(true);
    } finally {
      setBusy(false);
    }
  }, [plan, load]);

  const closeLayer = useCallback(() => {
    setPlan(null);
    setSpent(false);
    setError(null);
  }, []);

  // ---- 下面这组只是"把要发出去的那一份意图凑齐"，判定一项都不在这里做。 ----

  const askDefaults = useCallback(() => {
    if (defaults === null) return;
    void ask({
      kind: 'defaults', rcloneBin: defaults.rcloneBin, logDir: defaults.logDir,
      flags: defaults.flags, exclude: defaults.exclude,
    });
  }, [ask, defaults]);

  const addJob = useCallback(() => {
    setJob({ id: '', name: '', jobKind: 'copy', src: '', dst: '', openDir: '', manifest: false, isNew: true });
  }, []);

  /** 卡上那颗「存成常用」把那一格交过来时，打开的就是**这一张**表单（R97）：
   *  编号与名字留着让人填（临时格那个名字是我们起的，存进配置的那一条该有他自己认得的名字）。
   *  `fromDraft` 记的是"这张表单是从那张卡来的"——那一句实话只在来的那一趟说，
   *  自己点「新加一条」时提它就是把不相干的两件事写在同一格上。 */
  const [fromDraft, setFromDraft] = useState(false);
  useEffect(() => {
    const p = props.prefill;
    if (p === null) return;
    setJob({ ...p, isNew: true });
    setFromDraft(true);
  }, [props.prefill]);

  const editJob = useCallback((j: JobDraft) => setJob({ ...j, isNew: false }), []);

  const removeJob = useCallback((j: JobDraft) => void ask({ kind: 'remove_job', id: j.id }), [ask]);

  const askJob = useCallback(() => {
    if (job === null) return;
    // 那七项只有一处挑法（`jobFields`）：这里与下面那句"改了还没保存"用的是同一份。
    const fields = jobFields(job);
    // 「新加一条」与「改这条」是两个意图，不是一个：合成一个会让打错的编号静默变成一条新任务。
    void ask(job.isNew ? { kind: 'job_add', ...fields } : { kind: 'job_set', ...fields });
  }, [ask, job]);

  const patchJob = useCallback((p: Partial<JobDraft>) => {
    setJob((prev) => (prev === null ? prev : { ...prev, ...p }));
  }, []);

  const patchDefaults = useCallback((p: Partial<DefaultsDraft>) => {
    setDefaults((d) => (d === null ? d : { ...d, ...p }));
  }, []);

  if (view === null) {
    return (
      <Card title="改设置" className="surface">
        <Space orientation="vertical" size="small" style={{ width: '100%' }}>
          {error !== null && (
            <Alert type="error" showIcon title="这一栏列不出来" description={<NoteLine note={error} />} />
          )}
          <Button onClick={load} loading={busy}>再读一次</Button>
        </Space>
      </Card>
    );
  }

  const saved = defaultsOf(view);
  // 「任务」那一块按**同一对位置**分组：键读后台那枚 `pairKey`，标题用卡上那一对的短名。
  // 十条平铺读起来是十个不相干的东西，而它们其实是几对位置各有两三个方向/模式 —— 
  // 运行那一面已经收成一张卡一对位置，这一面不能再各说一份（分组本身在 `pairs.ts`，
  // 那里钉的是「键从哪儿来」）。
  const groups = groupDraftJobs(view.jobs);

  // 一行还是那一条：两颗按钮、那张表单、写出去的那几项，全都照旧。
  //
  // **2026-10-11：任务名不再当标题**（这一条改的是「两处界面说了两种话」）。
  // 以前第一行大字是 `j.name`，而出厂那十条的名字里**烘焙着方向**
  // （「镜像 R2：以本地为主（删云端多余的）」）—— 可卡上早已把方向收成「要做什么」
  // 那一格了。于是同一个东西：卡上是「位置 + 要做什么」，这里是「一句裹着方向的名字」。
  // 人对着设置改一条，眼前那句话和卡上那一格对不上，就以为改的不是同一件事。
  //
  // 现在第一行读的是**卡上那一句本身**（`intentPhraseFor`，与卡上同一份 `intentOptions`），
  // 路径与配置里的原名收进「为什么？」—— 明面只说「要做什么」。
  const renderJobRow = (j: JobRow) => (
    <Space orientation="vertical" size={4} style={{ width: '100%' }}>
      <Space size={6} wrap>
        {/* 会删的那一档吃暖色 —— 与卡上「要做什么」那一枚框同一枚色、同一件事。
            这里不是新加一层危险提示，是**那一格颜色的回声**：两处说同一件事，
            颜色得是同一枚，否则读者要自己判断"这两处是不是一回事"。 */}
        <Typography.Text style={j.jobKind === 'sync' ? { color: 'var(--kind-warm)' } : undefined}>
          {intentPhraseFor(groups, j)}
        </Typography.Text>
      </Space>
      <NoteLine note={{
        short: '',
        detail: `${j.src} → ${j.dst}　（配置里叫「${j.name}」）`,
      }} />
      {job !== null && !job.isNew && job.id === j.id ? (
        <JobFields job={job} patch={patchJob} busy={busy}
          onSave={askJob} onCancel={() => setJob(null)} />
      ) : (
        <Space size={4} wrap>
          <Button size="small" disabled={busy} onClick={() => editJob(j)}>
            改这条
          </Button>
          <Button size="small" disabled={busy} onClick={() => removeJob(j)}>
            删这条
          </Button>
        </Space>
      )}
    </Space>
  );
  const defaultsDirty = JSON.stringify(defaults) !== JSON.stringify(saved);
  // 「改了还没保存」的那一份名单。三处草稿一处都不许漏（实测的形状：填进框里的字
  // 切到别的面再回来**还在**，重开面板才静默消失，而屏上没有任何一处说这些还没落盘）。
  // 比的是"草稿 vs 磁盘那一份"，两处都取自己那一面的现成形状：远端比 `remoteOf`，任务比 `jobFields`。
  // 那一块还没有草稿时按磁盘那份算（与 `RemoteCard` 拿的是同一个 `?? remoteOf(block)`）——
  // 少这一句兜底，一个还没被点开过的远端会被读成"改了没保存"。
  const remotesDirty = view.remotes.filter(
    (b) => JSON.stringify(remotes[b.name] ?? remoteOf(b)) !== JSON.stringify(remoteOf(b)));
  const jobDirty = job === null ? false : job.isNew
    ? JSON.stringify(jobFields(job)) !== JSON.stringify(BLANK_JOB)
    : (() => {
      const base = view.jobs.find((x) => x.id === job.id);
      // 磁盘上那一条没了（别人改了文件，或这一条正要新加）也算"改了没保存"——表单里那份还没落地。
      return base === undefined || JSON.stringify(jobFields(job)) !== JSON.stringify(jobFields(base));
    })();
  const unsaved: string[] = [];
  if (defaultsDirty) unsaved.push('默认那几项');
  for (const b of remotesDirty) unsaved.push(`远端 ${b.name}`);
  if (jobDirty) unsaved.push(job !== null && job.isNew ? '新加的那一条' : `任务 ${job !== null ? job.id : ''}`);

  return (
    <Space orientation="vertical" size="large" style={{ width: '100%' }}>
      {/* 有事在跑时后台会拒下每一次保存，并把是哪一条说出来。这句写在最上面：
          让人填了半天才发现"现在不能改"，等于让人白填一次。 */}
      {view.running.length > 0 && (
        <Alert type="warning" showIcon title="现在有事正在跑"
          description={`等它跑完再改：${view.running.join('、')}。`} />
      )}
      {/* 改了还没保存的那几处，趁这一面开着就说出来（操作员那句「我常用设置怎么没保存」）。
          不拦切面也不拦关窗 —— 拦不拦是另一件事；这里只负责让人知道"这些字目前只在内存里活着"。
          名单本身在上面算，三处草稿一处都不许漏。 */}
      {unsaved.length > 0 && (
        <Alert type="warning" showIcon
          title={`有 ${unsaved.length} 处改了还没保存：${unsaved.join('、')}`}
          description="重开面板就回到磁盘上那一份。要留下它们，按各自那一块下面的那颗保存——那一步会先把要改的那几行列给你看。" />
      )}
      {result !== null && (
        <Alert type="success" showIcon closable={{ onClose: () => setResult(null) }}
          title={`做完了 · 改设置：${result.path}`} description={<NoteLine note={result.note} />} />
      )}
      {error !== null && plan === null && (
        <Alert type="error" showIcon title="没改成" description={<NoteLine note={error} />} />
      )}

      <Card title="这台机器上" className="surface">
        <Space orientation="vertical" size="middle" style={{ width: '100%' }}>
          {/* 路径由后台交，界面只渲染（面板可以跑在别的家上，写死的那一句在夹具里就骗人）。
              两份文件各说各的话：以前只说一句「配置文件在 …」，指的却是面板自己那份，
              想知道 rclone 那些远端存在哪个人文件里的人因此找不到 —— "配置难懂"有一半就是这一处。
              文件名不写在称呼里（那两份文件的名字会随家变），Path 自己把那一项摆在同一行上。 */}
          <Space size={6} wrap>
            <Typography.Text type="secondary" className="fs-caption">任务清单文件</Typography.Text>
            <Typography.Text copyable={{ text: view.jobsPath }} className="fs-code">{view.jobsPath}</Typography.Text>
          </Space>
          <Space size={6} wrap>
            <Typography.Text type="secondary" className="fs-caption">rclone 自己的配置文件</Typography.Text>
            {view.confPath === ''
              ? <Typography.Text type="secondary" className="fs-caption">问不出（上面那一栏说了为什么）</Typography.Text>
              : <>
                <Typography.Text copyable={{ text: view.confPath }} className="fs-code">{view.confPath}</Typography.Text>
                {/* 只开它**所在的那一层**：那一个目录由后台再问一次 rclone 得到，界面不交路径进去
                    （交进去就等于让这一颗能开任意一个目录）。密钥的值不在这条通路上（它从来不上屏）。 */}
                <Button size="small" onClick={props.onOpenFolder}>
                  打开这个目录
                </Button>
              </>}
          </Space>
          <Field label="rclone 程序在哪" keyName="rclone_bin" value={defaults?.rcloneBin ?? ''}
            onChange={(v) => patchDefaults({ rcloneBin: v })}
            decides="决定面板拿哪一份 rclone 起后台" ifBlank="空着保存会被拦下"
            note={<BinReadout view={view} value={defaults?.rcloneBin ?? ''} />} />
          <Field label="日志放哪" keyName="log_dir" value={defaults?.logDir ?? ''}
            onChange={(v) => patchDefaults({ logDir: v })}
            decides="决定出问题时去哪找记录" ifBlank="空着保存会被拦下"
            note={view.defaults.logDirExists ? null : <NoteLine note={view.defaults.logDirNote} />} />
          <TagField label="每次都要带的参数" keyName="flags" values={defaults?.flags ?? []}
            placeholder="例如 --transfers，下一项填 32"
            onChange={(v) => patchDefaults({ flags: v })}
            decides="决定每一次跑都带上的那几项参数" ifBlank="空着就是不加任何参数"
            detail={<Typography.Text type="secondary" className="fs-caption">
              这些参数是后台启动时读的，要生效得重开面板。能填的只有「只改怎么跑、不改什么进桶」那一族；
              写错一项会在点保存时被拦下，并把那句为什么念给你听。
              这一族里只点名放行了一枚改比法的例外：--size-only，它让每一趟都只比大小，
              内容改了但大小没变的那些不会重传；开着它的时候，屏幕最上面那条会一直写着同一句。
            </Typography.Text>} />
          <TagField label="哪些东西不进去" keyName="exclude" values={defaults?.exclude ?? []}
            placeholder="例如 *.log；要连子树一起不进去就写 目录名/"
            onChange={(v) => patchDefaults({ exclude: v })}
            decides="决定哪些东西不进去" ifBlank="空着就是全都算进去"
            detail={<Typography.Text type="secondary" className="fs-caption">
              有排除规则时，界面上看到的不是全部，「清空这一层」也因此点不动。
            </Typography.Text>} />
          {/* 上面那句在保存那颗**之前**就摆在屏上：排除规则非空会同时让浏览器那一栏常驻
              "看到的不是全部"并让「清空这一层」点不动。等按钮灰了再说，人已经点过一次了。 */}
          <Button disabled={!defaultsDirty || busy} onClick={askDefaults}>
            保存这几项
          </Button>
        </Space>
      </Card>

      <Card title="你的远端" className="surface">
        <Space orientation="vertical" size="middle" style={{ width: '100%' }}>
          {/* 名字就是那份文件里有的那几段（后台只交段名，密钥的值在返回类型里就不存在）。
              段读不出来（坏了、或同名两段）时那句「这一栏列不出来」由后台带着原文来——
              界面不把它读成"那就没有远端"，也不把它读成"列出来了就没有事"。 */}
          {(view.confNote.short !== '' || view.confNote.detail !== '') && (
            <Alert type="error" showIcon title="这一栏列不出来" description={<NoteLine note={view.confNote} />} />
          )}
          {view.remotes.map((block) => (
            <RemoteCard key={block.name} block={block} busy={busy}
              draft={remotes[block.name] ?? remoteOf(block)}
              onDraft={(d) => setRemotes((r) => ({ ...r, [block.name]: d }))}
              onSave={ask} />
          ))}
        </Space>
      </Card>

      <Card title="任务" className="surface" extra={<Button size="small" disabled={job !== null || busy} onClick={addJob}>
        新加一条
      </Button>}>
        <Space orientation="vertical" size="middle" style={{ width: '100%' }}>
          {groups.pairs.map((g) => (
            <Space key={g.key} orientation="vertical" size={2} style={{ width: '100%' }}>
              {/* 这一句就是卡上位置下拉里的那一句（同一个键、同一份短名），这里不另起称呼。 */}
              <Typography.Text type="secondary" className="fs-caption">{g.label}</Typography.Text>
              <Listy<JobRow> items={g.jobs} rowKey={(j) => j.id} itemRender={renderJobRow} />
            </Space>
          ))}
          {groups.gates.length > 0 && (
            // 对账那几条不配对：卡上没有它们那一格，替它们编一个"这一对"就是说谎。
            <Space orientation="vertical" size={2} style={{ width: '100%' }}>
              <Typography.Text type="secondary" className="fs-caption">对账的那几条（只读，不动东西）</Typography.Text>
              <Listy<JobRow> items={groups.gates} rowKey={(j) => j.id} itemRender={renderJobRow} />
            </Space>
          )}
          {job !== null && job.isNew && (
            <Card size="small" title="新加一条">
              <Space orientation="vertical" size="small" style={{ width: '100%' }}>
                {fromDraft && (
                  // R97 那句实话：面板不热重载（这条是实测过的）。所以存完**当场**这一面还是那张
                  // 临时卡，重开面板它才换成一张配对卡。不说这一句，人就以为按下保存那张卡就变了。
                  <Alert type="info" showIcon
                    title="这条是从刚才那张临时卡带过来的：存完之后跑的那一面还是那张临时卡，重开面板它才作为一张配对卡出现。" />
                )}
                <JobFields job={job} patch={patchJob} busy={busy}
                  onSave={askJob} onCancel={() => { setJob(null); setFromDraft(false); }} />
              </Space>
            </Card>
          )}
        </Space>
      </Card>

      {/* 确认层：正文第一行是完整路径，第二行是那一**对**位置（或删除的那一条的现在那一对）。
          差异逐行照后台交出来的那份列，一行都不许界面自己补。 */}
      <Modal open={plan !== null} width={720} onCancel={closeLayer} title="会改这几行"
        footer={[
          <Button key="cancel" onClick={closeLayer}>取消</Button>,
          <Button key="go" type="primary" danger={isRemoval(plan)}
            disabled={plan === null || busy || spent} onClick={commit}>
            {/* 三档各有自己的话：花掉的凭证再按一次只会拿到同一句拒绝。 */}
            {spent ? '这一份已经交出去了，请关掉再点一次保存'
              : isRemoval(plan) ? '我知道，删掉这条' : '我知道，保存'}
          </Button>,
        ]}>
        {plan !== null && (
          <Space orientation="vertical" size="middle" style={{ width: '100%' }}>
            <Typography.Text copyable={{ text: plan.path }} className="fs-code">{plan.path}</Typography.Text>
            <Typography.Text type="secondary">{plan.subject}</Typography.Text>
            <Alert type="warning" showIcon title="这一步不可撤销"
              description="下面每一行都会照那样写进文件；没在这张表里的行一个字节都不动——那些注释是你自己写的理由，面板不重排也不删。" />
            <Table<ConfigChange> size="small" rowKey={diffRowKey} dataSource={plan.changes}
              pagination={false} columns={diffColumns} />
            {/* 连带说明照后台那份列：哪几句出现由这次真的动了哪一项决定（没做的事它不说）。 */}
            {plan.notes.map((n, i) => (
              <Alert key={i} type="info" showIcon title={n.short}
                description={n.detail === '' ? undefined : <NoteLine note={n} />} />
            ))}
            {error !== null && (
              <Alert type="error" showIcon title="没改成" description={<NoteLine note={error} />} />
            )}
            <Typography.Text type="secondary" className="fs-caption">
              保存之后这一面还是刚才那一份，重开面板才换成新的——面板只在启动那一次读它。
            </Typography.Text>
          </Space>
        )}
      </Modal>
    </Space>
  );
}

/** 差异表那两列的标题与那两格怎么排。判定（哪一行算加了/拿了）在后台，这里只有排版。 */
const diffColumns = [
  {
    title: '哪一项',
    key: 'label',
    render: (_: unknown, c: ConfigChange) => (
      <Space size={4} wrap>
        <Typography.Text>{c.label}</Typography.Text>
        <Typography.Text type="secondary" className="fs-code">{c.key}</Typography.Text>
      </Space>
    ),
  },
  {
    title: '这次改成什么',
    key: 'after',
    render: (_: unknown, c: ConfigChange) => (
      <Typography.Text code className="fs-code">{rowOf(c)}</Typography.Text>
    ),
  },
];

const diffRowKey = (c: ConfigChange) => `${c.key} ${c.before}`;

/**
 * 一行"说法 + 照抄的键名 + 输入框"。键名用代码体：那是操作员在自己文件里认的那一行。
 *
 * `decides` / `ifBlank` 是**必填**的：这一面被指"配置难懂"，病根是每格只说了"这是什么"，
 * 填与不填的后果得让人自己推。两半句写不全，编译器就不让这一格过关 —— 比任何数格子的证人可靠。
 */
function Field(props: {
  label: string; keyName: string; value: string;
  onChange: (v: string) => void;
  decides: string; ifBlank: string;
  /** **常驻**的读数（"你选的是 v1.75.1"、"这个目录现在不存在"）：它们会变，值得占那一行。 */
  note?: ReactNode;
  /** 查阅材料：收进「为什么？」，平时不占屏（见 `FieldNote`）。 */
  detail?: ReactNode;
}) {
  const { label, keyName, value, onChange, decides, ifBlank, note, detail } = props;
  return (
    <Space orientation="vertical" size={2} style={{ width: '100%' }}>
      <FieldLabel label={label} keyName={keyName} />
      <Input value={value} onChange={(e) => onChange(e.target.value)} />
      <FieldHelp decides={decides} ifBlank={ifBlank} detail={detail} />
      {note}
    </Space>
  );
}

/** 同一行形状，只是控件是"能一项项加、也能删掉"的那一种（flags 与 exclude 用）。 */
function TagField(props: {
  label: string; keyName: string; values: string[]; placeholder: string;
  onChange: (v: string[]) => void;
  decides: string; ifBlank: string;
  note?: ReactNode;
  detail?: ReactNode;
}) {
  const { label, keyName, values, placeholder, onChange, decides, ifBlank, note, detail } = props;
  return (
    <Space orientation="vertical" size={2} style={{ width: '100%' }}>
      <FieldLabel label={label} keyName={keyName} />
      <Select popupMatchSelectWidth={false} mode="tags" style={{ width: '100%' }} value={values} placeholder={placeholder} onChange={onChange} />
      <FieldHelp decides={decides} ifBlank={ifBlank} detail={detail} />
      {note}
    </Space>
  );
}

/** 一格的两个问题，一行灰字说完：这一格决定什么、空着会怎样。
 *  这两半句是**常驻**的（`decides` / `ifBlank` 编译期必填，证人钉着），但只有这一行 ——
 *  每格再挂一段三行的补充说明时，这一屏里字最多的东西就成了参考手册，而不是"改哪几行"。
 *  查阅材料接在**同一行**尾巴上（与 `NoteLine` 同一个形状：一句留明面，细节收进展开），
 *  另起一行的话，省下来的三行又变成了两行。 */
function FieldHelp(props: { decides: string; ifBlank: string; detail?: ReactNode }) {
  const [open, setOpen] = useState(false);
  const has = props.detail !== undefined && props.detail !== null;
  return (
    <div>
      <Typography.Text type="secondary" className="fs-caption">
        {props.decides}。{props.ifBlank}。
        {has && (
          <>
            {' '}
            <a onClick={() => setOpen(!open)}>{open ? '收起' : '为什么？'}</a>
          </>
        )}
      </Typography.Text>
      {has && open && <div>{props.detail}</div>}
    </div>
  );
}

function FieldLabel(props: { label: string; keyName: string }) {
  return (
    <Space size={6}>
      <Typography.Text>{props.label}</Typography.Text>
      <Typography.Text type="secondary" className="fs-code">{props.keyName}</Typography.Text>
    </Space>
  );
}

/** 一块远端卡片：那四项 + 密钥那一格（那格没有输入框，见 `SecretCell`）。 */
function RemoteCard(props: {
  block: RemoteBlock; draft: RemoteDraft; busy: boolean;
  onDraft: (d: RemoteDraft) => void;
  onSave: (intent: ConfigIntent) => void;
}) {
  const { block, draft, busy, onDraft, onSave } = props;
  const saved = remoteOf(block);
  const dirty = JSON.stringify(draft) !== JSON.stringify(saved);
  const patch = (p: Partial<RemoteDraft>) => onDraft({ ...draft, ...p });
  const save = () => onSave({
    kind: 'remote', remote: block.name, remoteType: draft.remoteType, provider: draft.provider,
    endpoint: draft.endpoint, region: draft.region,
  });
  return (
    <Space orientation="vertical" size={6} style={{ width: '100%' }} className="remote-card">
      <Typography.Text strong>{block.name}</Typography.Text>
      <Field label={labelOf(block, 'type', '用哪种存储')} keyName="type" value={draft.remoteType}
        onChange={(v) => patch({ remoteType: v })}
        decides="决定这一条用哪一种存储" ifBlank="空着保存会被拦下"
        detail={<Typography.Text type="secondary" className="fs-caption">
          换后端等于换一套说法：那几项密钥与新后端大概对不上，要用 rclone 自己的命令重新配一遍。
        </Typography.Text>} />
      <Field label={labelOf(block, 'provider', '服务商')} keyName="provider" value={draft.provider}
        onChange={(v) => patch({ provider: v })}
        decides="决定这一条走哪一家的接口" ifBlank="空着就是不要那一行，由后端自己认" />
      <Field label={labelOf(block, 'endpoint', '服务地址')} keyName="endpoint" value={draft.endpoint}
        onChange={(v) => patch({ endpoint: v })}
        decides="决定这个远端连哪台服务器" ifBlank="空着就用 rclone 自己的默认" />
      <Field label={labelOf(block, 'region', '区域')} keyName="region" value={draft.region}
        onChange={(v) => patch({ region: v })}
        decides="决定东西存在哪一片区域" ifBlank="空着就是不要那一行，由后端自己认" />
      <SecretCell secrets={block.secrets} />
      <Button disabled={!dirty || busy} onClick={save}>
        保存这几项
      </Button>
    </Space>
  );
}

/**
 * 密钥那一格。它单独成一个元件，是为了让证人按**结构**判"这一格里没有输入框"：
 * 用不着往后数固定个字符那种一挪文件就失效的判据。
 * 界面对密钥只有这一种通路——把 rclone 自己的命令交给人复制，值从来不上屏。
 */
function SecretCell(props: { secrets: SecretState }) {
  const { secrets } = props;
  return (
    <Space orientation="vertical" size={2} style={{ width: '100%' }}>
      <Space size={6} wrap>
        <Typography.Text>已配置，不回显</Typography.Text>
        <Typography.Text type="secondary" className="fs-caption">{secrets.note.short}</Typography.Text>
      </Space>
      <Typography.Text type="secondary" className="fs-caption">
        要换密钥请复制这条命令，在自己的终端上跑：
      </Typography.Text>
      <Typography.Text copyable={{ text: secrets.command }} className="fs-code">{secrets.command}</Typography.Text>
    </Space>
  );
}

/** 后台那两个存在性字段的读法，三档各一句；都不由界面反推。 */
function BinReadout(props: { view: SettingsView; value: string }) {
  const { view, value } = props;
  if (value !== view.defaults.rcloneBin) {
    // 换了一份文件：它的版本要下一次启动才问得到。这里说"还不知道"，
    // 不拿正在跑的那一份的版本顶它——那是两个不同的文件。
    return (
      <Typography.Text type="secondary" className="fs-caption">
        这一项要重开面板之后才知道是什么版本。
      </Typography.Text>
    );
  }
  if (!view.defaults.rcloneBinExists) {
    return (
      <Typography.Text type="secondary" className="fs-caption">
        这个位置没有可执行文件，保存时会被拦下。
      </Typography.Text>
    );
  }
  return (
    <Space size={6}>
      <Typography.Text type="secondary" className="fs-caption">读到的版本</Typography.Text>
      <Typography.Text className="fs-code">
        {view.defaults.rcloneBinVersion === '' ? '问不出它的版本' : `v${view.defaults.rcloneBinVersion}`}
      </Typography.Text>
    </Space>
  );
}

/** 任务那一块的字段。编号是身份不是名字：它点不动（要换编号就删了重建，两次确认各看一次差异）。 */
function JobFields(props: {
  job: JobEdit;
  patch: (p: Partial<JobDraft>) => void;
  busy: boolean;
  onSave: () => void;
  onCancel: () => void;
}) {
  const { job, patch, busy, onSave, onCancel } = props;
  const syncName = job.jobKind === 'sync';
  const kindField = (v: string) => patch({ jobKind: v as JobDraft['jobKind'] });
  const manifestField = (v: string) => patch({ manifest: v === 'localdir' });
  return (
    <Space orientation="vertical" size={6} style={{ width: '100%' }}>
      <Space orientation="vertical" size={2} style={{ width: '100%' }}>
        <Typography.Text>这条的编号</Typography.Text>
        {/* 编号只在**新加**那一次点得动：它是身份，不是名字。已经存在的那一条要换编号，
            等于把这一块的其余行指到另一条任务上——所以走"删了重建"，两次确认各看一次差异。 */}
        <Input value={job.id} disabled={!job.isNew} onChange={(e) => patch({ id: e.target.value })}
          placeholder={job.isNew ? '新加时才填，之后不改' : undefined} />
        <Typography.Text type="secondary" className="fs-caption">
          编号是这一条的身份。改它等于把同一批行落到另一条上：要换编号就删了重建。
        </Typography.Text>
      </Space>
      <Space orientation="vertical" size={2} style={{ width: '100%' }}>
        <Typography.Text>这一条算哪类</Typography.Text>
        <Select popupMatchSelectWidth={false} value={job.jobKind} style={{ width: '100%' }} onChange={kindField}
          options={(['sync', 'copy', 'gate'] as const).map((k) => ({ value: k, label: KIND_LABEL[k] }))} />
      </Space>
      <Space orientation="vertical" size={2} style={{ width: '100%' }}>
        <Typography.Text>这条叫什么名字</Typography.Text>
        <Input value={job.name} onChange={(e) => patch({ name: e.target.value })}
          placeholder={syncName ? '留空由面板起' : undefined} />
        {syncName && (
          <Typography.Text type="secondary" className="fs-caption">
            会删掉东西的那一条，名字里要有「删」这个字。
          </Typography.Text>
        )}
      </Space>
      <Field label="从哪里来" keyName="src" value={job.src} onChange={(v) => patch({ src: v })}
        decides="决定这一条从哪里读东西" ifBlank="空着保存会被拦下" />
      <Field label="到哪里去" keyName="dst" value={job.dst} onChange={(v) => patch({ dst: v })}
        decides="决定这一条把东西放到哪里" ifBlank="空着保存会被拦下"
        note={<Typography.Text type="secondary" className="fs-caption">
          会删东西的那一类要恰好一侧是云端的位置，另一侧是本地目录。
        </Typography.Text>} />
      <Field label="顺手打开哪个目录" keyName="open_dir" value={job.openDir}
        onChange={(v) => patch({ openDir: v })}
        decides="决定跑完之后顺手打开哪一个目录" ifBlank="空着就是不要那一行（那一行会被拿走）" />
      <Select popupMatchSelectWidth={false} value={job.manifest ? 'localdir' : 'none'} style={{ minWidth: 200 }}
        onChange={manifestField}
        options={[{ value: 'none', label: '不对账' }, { value: 'localdir', label: '这一条要对账' }]} />
      <Space size={4}>
        <Button disabled={busy} onClick={onSave}>保存这条</Button>
        <Button type="text" disabled={busy} onClick={onCancel}>不要了</Button>
      </Space>
    </Space>
  );
}
