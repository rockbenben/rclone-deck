import { Button, Dropdown, Listy, Select, Space, Tag, Typography } from 'antd';
import { CloudUploadOutlined, DiffOutlined, FolderOpenOutlined, SwapOutlined, SyncOutlined } from '@ant-design/icons';
import type { ReactNode } from 'react';
import { useEffect, useRef, useState } from 'react';

import { HIT_OFF, LockHint } from './LockHint';
import type { Mode, PairGroup, Slot } from '../pairs';
import { defaultChoice, groupJobs, intentKey, intentOptions, slotJob } from '../pairs';
import type { JobView, TaskState } from '../types';
import { canInventory, KIND_LABEL, runningHint } from '../types';

// 颜色按**会不会删**给，不按 kind 给：同一对位置两种模式都摆得出来，"配对"本身没有危险程度，
// 站在哪一格才有。值取自 CSS 变量，与 `theme.ts` 色板同名同值（组件里不写字面色值这条判据不动）。
const MODE_VAR: Record<Mode, string> = { add: 'var(--ink-2)', mirror: 'var(--kind-warm)' };
const INK = 'var(--ink-2)';

/** 「只补，不删」那一档下面的一句，**挂在「看看会动什么」那颗的悬停上，不常驻在卡上**。
 *  它要说的话一个字没少（并排两颗时最容易按错的就是这两颗），但它说的是"这两颗的区别"，
 *  不是"这一格的状态" —— 常驻那一行等于让一句解释每一趟都占掉两行，而那两行本该给读数。
 *  所以它跟着那颗按钮走：鼠标在上面时说得清，平时不占地方。
 *  「会删除」那一档没有这一句：那一格只剩一颗，没有并排就没有可错的地方。
 *
 *  改名（2026-10-09）：`先试跑` → `看看会动什么`，`开始传输` → `开始`。
 *  「试跑」听着像**真的要跑一次**，而这颗只列不动；「传输」是术语，而卡上已经说了
 *  这一趟动的是哪一侧。这两个是全仓少数**不与后台词表焊在一起**的动作名（后台从不按
 *  这两个名字发话），所以只动字面就够。**没顺手改的**是「看差异」—— 它是只读盘点的
 *  入口，后台 `runner/mod.rs:823` 的日志按「看差异」那一份取基准，改它要连后台一起动。 */
const ADD_MODE_NOTE = '「看看会动什么」只告诉你会动哪些，不真传；要把东西放上去，按「开始」。';

/** 这一格**这一次会怎么样** —— 不是这一格"有几个东西"的统计，是"按下去会发生什么"的后果。
 *
 *  这一句是整个面板存在的理由（README 第一行：看清这一次会动哪些文件、**会不会删**，再按下去）。
 *  旧写法给的是统计口径「待传 2 · 云端多余 0」：其中**「云端多余」恰恰是对齐模式下要删掉的那些**，
 *  却用了全屏最中性的词 —— 于是一个会删东西的工具，问「会不会删」时屏上没有任何一处正面回答。
 *  这里换成后果口径，并按模式分叉：会删的那一份自己占一格、吃危险色，删不掉的那一次说「不会删」。
 *
 *  **「会删 N 个」还必须说清删的是哪一边**（2026-10-09）：同一个「会删 3 个」，
 *  删云端上的 3 个和删**你自己硬盘上**的 3 个完全不是一回事 —— 后者没有任何撤销。
 *  判据是方向那枚框已经摆着的那句话（`directionLabel` 里的「以…为主（删…多余的）」），
 *  这里**读它的方向**而不是重算一遍：哪一侧是全集已经由后台那两枚 `isLocal` 标志判过了。
 *
 *  **「会传 N 个」必须把两边都有但对不上的那批一起算进去**（`willUpload`）：只数"本地有、云端没有"
 *  的话，一次会把云端那份换掉的 copy 会显示成「会传 0 个」——操作员据此按下去，被换掉的是他
 *  没被告知的那部分。下载方向同理，后台已经按 dst 的方向算好，界面只读。`willUpload` 宁可多报
 *  （`Unknown`、size-only 下同大小的也在内）；真正会换掉目标端的那批看 `replaces`。
 *  `replaces` 单独说一句，因为"新增"与"换掉原有的"不是同一件事；它是后台判定的**真会被重传**
 *  的那一批（`Unknown` 与 size-only 下大小没变的不算），界面不许自己从 `diverged` 再算一遍。
 * 这几个数、`emptyBasis`、`measuredAt` 全部照读后台（`JobView.lastSummary`），一个都不许重算。
 * `null` 那一档说"还没算过"，但**不解释怎么算** —— 「看差异」就在这一格上，指着一颗看得见的按钮说它是虚的。
 * 这一句也**不**负责去算：列表渲染里发起盘点就是每次开窗几十次计费 LIST（spec §3 那句"列举不自动跑"）。
 */
function summaryLine(job: JobView, mode: Mode, dstIsLocal: boolean): ReactNode {
  const s = job.lastSummary;
  if (s === null) {
    return <Typography.Text type="secondary" className="fs-note">还没算过差异</Typography.Text>;
  }
  const num = (n: number) => <span className="num">{n} 个</span>;
  return (
    <Space orientation="vertical" size={2} style={{ width: '100%' }}>
      <Space size={8} wrap>
        <Typography.Text className="fs-note">会传 {num(s.willUpload)}</Typography.Text>
        {mode === 'mirror'
          ? <Typography.Text className="fs-note" style={{ color: MODE_VAR.mirror }}>
            会删{dstIsLocal ? '本机' : '云端'} {num(dstIsLocal ? s.pendingUpload : s.orphans)}
          </Typography.Text>
          : <Typography.Text type="secondary" className="fs-note">不会删</Typography.Text>}
        <Typography.Text type="secondary" className="fs-caption">{s.measuredAt} 算的</Typography.Text>
      </Space>
      {/* 换掉原有的那份不是"新增"：两边都有、这一趟会重传时，按下去会改写目标端那一份。
          这句话只说后果，不替操作员判哪边更新——「差异」那一段才说得出哪边新，判不出时也会明说。 */}
      {s.replaces > 0 && (
        <Typography.Text type="secondary" className="fs-caption">
          其中 {num(s.replaces)}：两边都有，目标端那份会被这一趟换掉
        </Typography.Text>
      )}
      {/* `emptyBasis`：本地那一侧是空的，于是这两个数**不可信**。它必须跟着这两个数一起摆 ——
          把它撤掉的话，屏上会并排着"会删 0 个"与两个没有意义的数，而这一格恰恰是最容易出事的
          那种（空的一侧 + 镜像 = 一次什么都删不掉的空跑，或者更糟）。措辞照读后台，不改。 */}
      {s.emptyBasis && (
        <Typography.Text type="secondary" className="fs-caption">本地是空的，这两个数不可信</Typography.Text>
      )}
    </Space>
  );
}

/** 当前那一格脚下的一颗动作。`strong` 只有"开始传输"用得上：界面上不许有红色或实心的一键删除
 * （`mirrorPrimary` 那一段说清了为什么）。`hint` 是那颗**自己**的悬停说明，不是常驻文字。 */
interface CardAction {
  key: string;
  label: string;
  run: () => void;
  off: boolean;
  strong: boolean;
  hint?: ReactNode;
  /** 可选的图标，纯装饰（`CardAction` 那一行）。 */
  icon?: ReactNode;
}

/** 上一件事还没跑完时挂在灰钮上的那一句。它不算"备用文案"：忙碌态是这一面自己的判据
 *  （后台从没说过"别按"），所以按 DESIGN 的分工，句子也归界面这一层。 */
const BUSY_NOTE = '上一件事还没跑完';

const act = (key: string, label: string, run: () => void, off: boolean, strong = false,
  hint?: ReactNode, icon?: ReactNode): CardAction => ({ key, label, run, off, strong, hint, icon });

/** 卡面上那几颗动作的图标（2026-10-09，从操作员那份 AHK 脚本里吸收过来的那一处优势）。
 *
 * 脚本用的是 emoji（☁️⬇️⬆️📂），这里用 antd 矢量图标：离线可用、跨机器一致、
 * 跟着主题走深浅色。**只给"看得出来的动作"** —— 传/对齐/看/换/开目录，
 * 不给每一颗都加：满屏图标和满屏文字是同一种噪音。
 * 图标是**装饰**，动作的名字一个字没少（`a.label` 仍是 `<Button>` 里的文字），
 * 所以屏上认不出图标的人和认得出的人拿到的是同一份信息。 */
const ICON = {
  up: <CloudUploadOutlined />,
  mirror: <SyncOutlined />,
  diff: <DiffOutlined />,
  swap: <SwapOutlined />,
  folder: <FolderOpenOutlined />,
};

/** 那一对完整路径，**截中段**（`DESIGN.md` 的 Typography 那一条：「过长时截断中段并给 Tooltip 全文」）。
 *
 *  认家靠的是**尾巴那个目录名**。尾部截断把 `…\Local\Temp\d…` 留在屏上 —— 认不出是哪一家，
 *  而这一行存在的理由恰恰是"挑错一家靠全文认"（那一段钉名字撤不撤的判据）。
 *  三段纯 CSS：头是可缩可省略的那一段，箭头与尾钉住不缩，全值挂原生 `title`
 *  （与 `log-ellipsis` 同一个形状：那一格不许被收进浮层，规程 R68）。
 */
function PairPath({ src, dst }: { src: string; dst: string }) {
  return (
    <div className="fs-code path-line">
      <span className="path-head" title={src}>{src}</span>
      <span className="path-arrow">→</span>
      <span className="path-tail" title={dst}>{dst}</span>
    </div>
  );
}

export interface JobActions {
  onTrialRun: (id: string) => void;
  onStart: (id: string) => void;
  /** 打开对齐的确认层：里面是逐条清单，确认交出去的是那份预览的一次性 token。 */
  onPreviewMirror: (id: string) => void;
  onGate: (id: string) => void;
  onInventory: (id: string) => void;
  onOpenDir: (id: string) => void;
}

interface Props extends JobActions {
  jobs: JobView[];
  states: Record<string, TaskState>;
  selectedId: string | null;
  busy: boolean;
  onSelect: (id: string) => void;
  /** 「换一对位置」那颗：立卡那一趟与它带回来的拒绝都住在层里，这里只负责把它喊开。 */
  onNewPair: () => void;
  /** 把当前这一格送去设置面那张**已有的**「新加一条」表单（R97）。
   *  这一层自己不写得进任何文件：第二条写入通路就是第二条会各自漂的确认流程。 */
  onSaveDraft: (job: JobView) => void;
  /** 收起这一对位置上那几格临时的（R98）。交回的是那把键不是任务 id：一张卡一起收。 */
  onFoldPair: (pairKey: string) => void;
  /** 刚立起来的那一对的 `pairKey`：指一次就完（`askedFor` 记着已经指过的键）。 */
  focusPair: string | null;
}
/** 「站在哪一对位置 + 那一格是什么」的**唯一一份状态**。
 *
 *  它原来住在 `JobListView` 自己身上，于是三枚下拉只能跟着格一起挤在左列里 —— 那一列只有
 *  340px，三枚框加三行标签一行也放不下，实测被折成**三行**，"选哪一对位置"读起来像一张
 *  竖排的表。判据「同一属性的下拉只该有一个」是对的，但它没有说这三枚必须挤在一列里。
 *  现在它抬成一处，选择条横跨整窗、格与右列并排 —— 两边读的是同一份状态，判定仍只有一处。 */
export function usePairChoice(jobs: JobView[], focusPair: string | null, onSelect: (id: string) => void) {
  const { pairs, gates } = groupJobs(jobs);
  /** 站在哪一对位置上。没选过就站在第一对 —— 这一面**永远有一格被选中**：
   *  列表撤掉之后只剩当前那一格，"没选"会让右列没有主人（那句「先选一个任务」不该在这一面可达）。 */
  const [pairKey, setPairKey] = useState<string | null>(null);
  const shown = pairs.find((g) => g.key === pairKey) ?? pairs[0];
  /** 每一对站在哪一格（模式 + 方向）。**只活在本次会话里**：记住一个"会删东西"的选择，
   *  等于让某天的第一次点击落在最重的那一档上，所以重开面板一定回到「只补，不删」。 */
  const [choice, setChoice] = useState<Record<string, { mode: Mode; slot: Slot }>>({});

  /** 这一对现在说的是**哪条任务**：读选择，缺的那一格退回默认（配置在两次轮询之间被改过
   *  就会缺）。四格里哪一格有任务由后台带出来的 `pairKey`/`pairSlot` 决定，这里不重算。 */
  const pick = (g: PairGroup) => {
    const wanted = choice[g.key] ?? defaultChoice(g);
    const job = slotJob(g, wanted.mode, wanted.slot);
    if (job !== null) return { mode: wanted.mode, slot: wanted.slot, job };
    const dflt = defaultChoice(g);
    const fallback = slotJob(g, dflt.mode, dflt.slot);
    return fallback === null ? null : { mode: dflt.mode, slot: dflt.slot, job: fallback };
  };

  /** 换模式 / 换方向：除了记住这一格，还要把右列跟着指过去 —— 右列讲的必须是当前这一格那条任务，
   *  不然翻了方向而进度与差异还在讲上一格（"刚点的那颗在右列演的是哪条"是同族病）。 */
  const choose = (mode: Mode, slot: Slot) => {
    if (shown === undefined) return;
    const job = slotJob(shown, mode, slot);
    if (job === null) return;
    setChoice((c) => ({ ...c, [shown.key]: { mode, slot } }));
    onSelect(job.id);
  };

  /** 外面那一句「刚立起来一张卡」（`focusPair`）翻成"站到那一对上"。
   *  `askedFor` 记的是已经接过的键：轮询每秒重渲染一次，不记就会把操作员随后换的位置抢回去。 */
  const askedFor = useRef<string | null>(null);
  useEffect(() => {
    const key = focusPair;
    if (key === null || askedFor.current === key) return;
    if (!pairs.some((g) => g.key === key)) return;   // 那一批还没到（下一拍轮询会送来）
    askedFor.current = key;
    setPairKey(key);
  });

  /** **这一格真的换了**，才把右列指过去。"换了"只由 `pick()` 说了算：外面那一层不许自己
   *  再挑一次默认格，两处各挑一次，漂开的那一次就是格上写着「只补，不删」、右列演的是「让两边对齐（会删）」
   *  （量到过）。门放在这里而不是放在"每次渲染都对一遍 selectedId"：
   *  对一遍就会把点中对账那一行的选择抢回来 —— 对账不是这一格。 */
  const pointed = useRef<string | null>(null);
  useEffect(() => {
    const picked = shown === undefined ? null : pick(shown);
    if (picked === null || pointed.current === picked.job.id) return;
    pointed.current = picked.job.id;
    onSelect(picked.job.id);
  });

  return { pairs, gates, shown, pick, choose, pairKey, setPairKey };
}

type Choice = ReturnType<typeof usePairChoice>;

/** 选择条上的一格：标签在上、框在下。三枚并排时**标签成列**才读得出谁说的是谁，
 *  以前标签在框左边，折行之后就成了三段各说各话的竖排文字。 */
function Field({ label, tone, children }: { label: string; tone?: string; children: ReactNode }) {
  return (
    <div className="pair-field">
      <Typography.Text className="fs-caption" style={tone !== undefined ? { color: tone } : undefined}>
        {label}
      </Typography.Text>
      {children}
    </div>
  );
}

/**
 * 位置 / **要做什么** 两枚选择条，横跨整窗。
 *
 * 一枚属性一枚框是操作员那句「相同属性的下拉应该只有一个」的直接结果：
 * 以前每张卡自带两枚，五对位置就是十枚，一列下来「只补，不删」重复四遍，
 * 读的人分不清哪一枚说的是哪一家。
 *
 * **2026-10-09：「模式」「方向」两枚并成一枚「要做什么」。** 原来那一格的真名是两枚框的话拼起来
 * 才会出现（`只补，不删` + `从本地补到云端`），而拼错拼出来的仍是一句通顺的话 —— 读者没有
 * 任何地方能发现自己拼错了。并成一枚之后**一个选项自己就是一个完整意图**，不给人拼的机会。
 * 代价是选项变长（所以 `popupMatchSelectWidth={false}` 让它按内容开），换来的是少一次判断。
 *
 * 危险的那两档吃暖色：这是唯一能一眼分出"哪两项会删东西"的地方，所以**不给它们红底、不给它们
 * 放在最后一位之外** —— 顺序本身（不删的在上）已经是第二道提示。
 */
export function JobListView(props: Props & { choice: Choice }) {
  const { shown, pick, choose, setPairKey, pairs } = props.choice;
  // 没有任何一对位置时**也要把这条选择条摆出来**：那颗「换一对位置」就在这一行里。
  // 旧写法整段 `return null`，于是删掉最后一对位置之后，这一面只剩右列一句「先选一个任务」——
  // 立新卡的那颗按钮跟着那条不存在的卡一起消失了。
  const picked = shown === undefined ? null : pick(shown);
  if (shown !== undefined && picked === null) return null;
  const intents = shown === undefined ? [] : intentOptions(shown);
  const current = picked === null ? undefined : intents.find((i) => i.mode === picked.mode && i.slot === picked.slot);
  return (
    <div className="surface bar pair-bar">
      <Field label="位置">
        <Select popupMatchSelectWidth={false} value={shown?.key} aria-label="位置"
          placeholder={shown === undefined ? '还没有位置' : undefined}
          disabled={shown === undefined}
          className="pair-select" onChange={(k) => setPairKey(k)}
          options={pairs.map((p) => ({
            value: p.key,
            // 短名由 `pairs.ts` 那一处统一算（撞名会自己往上加层）；这里只读。
            // 临时那几格要在名字里说出来：位置挑错了要能看出来这一家不是配置里的。
            label: `${p.aLabel} ↔ ${p.bLabel}${p.hasDraft ? '（临时）' : ''}`,
          }))} />
      </Field>
      {/* 「要做什么」那两个字那一格的颜色说的就是**当前这一格**会不会删东西
          （暖色 = 会删）。一屏只有一枚这样的框，所以它不需要再写一遍"会删除"三遍。 */}
      <Field label="要做什么" tone={current?.tone}>
        <Select<string> popupMatchSelectWidth={false}
          value={picked === null ? undefined : intentKey(picked.mode, picked.slot)} aria-label="要做什么"
          placeholder={picked === null ? '先挑一对位置' : undefined}
          disabled={picked === null}
          className="pair-select" onChange={(k) => {
            const hit = intents.find((i) => intentKey(i.mode, i.slot) === k);
            if (hit) choose(hit.mode, hit.slot);
          }}
          options={intents.map((i) => ({ value: intentKey(i.mode, i.slot), label: i.label }))} />
      </Field>
      {/* 这一颗是**那一面的一条全局动作**，不是每一格里的一颗：它自己不立卡（`onNewPair()`），
          因为立卡那趟带回来的那句拒绝要在层里说——两处发起点就会有两处话术。 */}
      <Button className="pair-bar-act" icon={ICON.swap} onClick={() => props.onNewPair()}>换一对位置</Button>
    </div>
  );
}

/**
 * 左列：**当前那一格** + 对账那一小段。
 *
 * 这一格只说三件事，说完就停：动的是哪两个位置（完整路径，认家靠全文）、这一格有几格没存过、
 * 以及那一排动作。以前它还常驻一句「先试跑」的解释与一句方向复述，于是"现在是什么状态"
 * 这一层被三行说明压着 —— 一屏里字最多的地方，说的却不是当前状态。
 */
export function PairCell(props: Props & { choice: Choice }) {
  const { states, selectedId, busy, onSelect } = props;
  const { gates, shown, pick } = props.choice;
  const stateOf = (job: JobView) => states[job.id];

  /** 当前那一格脚下的一排：主操作摊在面上，只读的那两颗收进「更多」。
   *  `foldable` = 这一对上有临时格：那一来「删掉这张临时卡」要摆得出来（收的是那几格临时的，
   *  交回的是那把键 —— 一起收，不是逐格收）。它不因为"这一对正被什么占着"而灭：
   *  正在跑的时候是**后台**拒的，那句"等它跑完再收"要人听得见，而不是点不动又没人说话。 */
  const cellButtons = (job: JobView, primary: CardAction[], foldable = false) => {
    const locked = job.lockedBy !== null;
    const lockedHint = runningHint(job);
    // 「看差异」从收纳口提到卡面上：它是这一格**唯一只读**的动作，也是动手之前该先按的那一颗
    // （README 第 4 步「第一次跑建议就停在这儿」）。收在「更多」里的那一次，格上那句
    // 「差异还没算过：点「看差异」算一次」指着一颗看不见的按钮，而同时看得见的那一颗
    // 是会删东西的 —— 把只读的那条藏起来、把危险的那条摆出来，顺序是反的。
    const rest: CardAction[] = [
      // 「收起来」→「删掉这张临时卡」（2026-10-09）。后台 `drop_draft` 是把那几格**从内存里
      // 移除**（`runner/drafts.rs`），不是收进哪儿去；卡上自己也写着「重开面板就没了」。
      // 「收起」暗示"回头还翻得出来"，而它其实没了 —— 一个说反了的名字比没有更坏。
      //
      // 为什么不叫光秃秃的「删除」：这个仓里「删除」= 删**用户的数据**，删东西要走逐条清单
      // + 一次确认。一颗没点明对象的「删除」在菜单里会让人以为要删文件。**点明对象**，
      // 两个顾虑就都没了：动词是你要的那个，被删的到底是什么一目了然。
      ...(foldable ? [act('fold', '删掉这张临时卡', () => props.onFoldPair(job.pairKey), busy)] : []),
      // R58：只读盘点。src 是本地目录、dst 是 remote 的任务都能问一句
      // "云端有哪些本地没有的对象"；这一项只是打开只读视图，不触发任何删除。
      // 这颗的名字跟着**段名**走（右列那一段就叫「差异」，`DESIGN.md` 钉着四段名）：
      // 以前它叫「本地没有的云端对象」，而同一个面在仓里另有三个称呼。
      ...(job.openDir
        ? [act('opendir', '打开所在目录', () => props.onOpenDir(job.id), false)]
        : []),
    ];
    const face: CardAction[] = [
      ...primary,
      ...(canInventory(job)
        ? [act('inventory', '看差异', () => props.onInventory(job.id), busy || locked, false, undefined, ICON.diff)]
        : []),
      // 「存成常用」摊在面上而不是收进「更多」：它是这一格唯一那条"以后还能这么跑"的路，
      // 而它读的是**当前这一格**（配置格本来就常用着，不需要这一颗）。
      // 按钮名说**结果**不说动作：「存成常用」这四个字要人自己推一步才知道它会写进配置，
      // 而这一颗按下去正是要把临时格写进 `jobs.toml`（R97）。短了不行，长到读不成一句也不行。
      ...(job.isDraft ? [act('save', '存成常用（写进配置）', () => props.onSaveDraft(job), busy)] : []),
    ];
    return (
      <Space wrap size={4}>
        {face.map((a) => {
          // 灰着的那颗必须说得出为什么（R54），而"为什么"有三个来源，优先级是：
          // ① 被别的任务占着（后台那句，它比这颗自己的说明更要紧——人此刻问的是"为什么点不动"）；
          // ② 这颗自己的功能说明（`a.hint`，常驻）；
          // ③ 只是上一件事还没跑完（这一面自己的判据，所以句子也在这一面，见 `plainNote`）。
          // 以前 ③ 那一档什么都没有：一次传输在跑，屏上同时灰五颗，只有正在跑的那张卡说了一句。
          const why = (a.off && locked && lockedHint !== null)
            ? lockedHint
            : (a.hint ?? (a.off ? BUSY_NOTE : null));
          // `HIT_OFF` 的条件是"这一颗挂着 Tooltip"，不是"它没有自己的说明"：
          // Chromium 不给 disabled 控件派发 hover，少了那层让位，真鼠标悬停永远弹不出来。
          const btn = (
            <Button size="small" type={a.strong ? 'primary' : 'default'}
              style={why !== null ? HIT_OFF : undefined} disabled={a.off} onClick={a.run}>
              {a.icon !== undefined && <span className="act-icon">{a.icon}</span>}
              {a.label}
            </Button>
          );
          return why === null
            ? <span key={a.key}>{btn}</span>
            : <LockHint key={a.key} title={why}>{btn}</LockHint>;
        })}
        {rest.length > 0 && (
          <Dropdown
            trigger={['click']}
            menu={{
              items: rest.map((r) => ({
                key: r.key,
                label: r.label,
                disabled: r.off,
                onClick: r.run,
              })),
            }}
          >
            <Button size="small">更多</Button>
          </Dropdown>
        )}
      </Space>
    );
  };

  // 主操作按**模式**分两档，档名是给证人看的锚点（`frontend_mirror.rs` 的 `mirror_arm`）：
  // 「会删除」那一档里出现 `onStart` 就是"可以不预览直接删"，所以那一档单独切出来钉。
  const mirrorPrimary = (id: string, off: boolean): CardAction[] => [
    // 这一档只有**一颗**，它打开的就是那层清单 —— 所以这里不放红色按钮：
    // 一个按下去什么都不删的红色按钮，练的是"红色可以随便点"。
    // 真正批准删除的那颗红色在确认层的页脚（`MirrorConfirm`），
    // 而点下去之后后台还要重列一遍逐条比对，不一致就不动。
    act('preview', '让两边对齐', () => props.onPreviewMirror(id), off, false, undefined, ICON.mirror),
  ];
  const addPrimary = (id: string, off: boolean, mode: Mode): CardAction[] => [
    // 那句解释挂在**这颗自己**的悬停上（`ADD_MODE_NOTE`），不占卡上那两行常驻文字：
    // 它说的是这两颗的区别，不是这一格的状态，而对齐那一档没有可错的地方所以不带它。
    act('trial', '看看会动什么', () => props.onTrialRun(id), off, false, mode === 'add' ? ADD_MODE_NOTE : undefined),
    act('start', '开始', () => props.onStart(id), off, true, undefined, ICON.up),
  ];
  const gatePrimary = (id: string, off: boolean): CardAction[] => [
    act('gate', '开始对账', () => props.onGate(id), off),
  ];

  /** 当前那一格：它说的是哪条任务、完整那一对位置、这一格自己的读数与动作。
   *  认家靠**完整那两条路径**（挑错一家靠的是全文认，不是猜那一小截）；短名与名字各只在一个地方说，
   *  一个在位置那枚框里、一个在右列那一行上，这一格不替它们复述。 */
  const renderCell = (g: PairGroup) => {
    const picked = pick(g);
    if (picked === null) return null;      // 一对位置连一格任务都没有：这一对不出现（位置框里也不该有它）
    const { mode, job } = picked;
    const locked = job.lockedBy !== null;
    const state = stateOf(job);
    const running = state?.state === 'running' || state?.state === 'queued';
    const disabled = busy || locked || running;
    const lockedHint = runningHint(job);
    const primary = job.kind === 'sync' ? mirrorPrimary(job.id, disabled) : addPrimary(job.id, disabled, mode);
    // 对齐要删的是**目标端**那一侧（`doomed = dst \ src`）。哪一端是 dst 由选择条那枚方向框
    // 说了算，这里读后台已经判好的 `dstIsLocal`，不重算一次（裁定 R69：哪一侧是本地
    // 由 `aIsLocal`/`bIsLocal` 判，界面再拿路径认一遍就是第二份口径）。
    const dstIsLocal = picked.slot === 'a' ? g.bIsLocal : g.aIsLocal;
    return (
      // 这一块的类名是**验收驱动器读的锚点**：量它得有个不依赖文字的说法
      //（改名、换措辞都不该让那一格忽然量不到）。
      <Space orientation="vertical" size={8} style={{ width: '100%' }} className="pairs-cell">
        {/* 这一格只交**完整那两条路径**。名字不在这里说（它在右列开头那一行），
            方向不在这里说（它在选择条上那枚框里），模式也不在这里说（它那枚框的字会变色）。 */}
        <PairPath src={job.src} dst={job.dst} />
        {locked && <Tag color="red">{lockedHint}</Tag>}
        {/* 「会不会删」这一问，屏上有一处正面回答：这一块自己占一格、底下垫一层底色，
            是整格里唯一"读它就知道按下去会发生什么"的地方（R96 那句临时格的话排在它后面 ——
            那句说的是"这一格存在多久"，不是"这一格会做什么"，两件事不能抢同一个位置）。 */}
        {canInventory(job) && (
          <div className="consequence">{summaryLine(job, mode, dstIsLocal)}</div>
        )}
        {/* R96：这一对上那几格不写得进那两份文件，界面上必须把这句话说出来，而不是只记在我们心里。
            措辞按"这一对上**有**没存过的格子"说而不是"整家是临时的"：一对位置可以是配置格与
            临时格混着的（配置只占了两格，临时卡补另外两格），后一种说法在这种位置上就是假的。 */}
        {g.hasDraft && (
          <Typography.Text type="secondary" className="fs-caption">
            这张卡上有没写进配置的格子：重开面板就没了
          </Typography.Text>
        )}
        {cellButtons(job, primary, g.hasDraft)}
      </Space>
    );
  };

  const renderGate = (job: JobView) => {
    const locked = job.lockedBy !== null;
    const lockedHint = runningHint(job);
    return (
      <div
        onClick={() => onSelect(job.id)}
        style={{
          cursor: 'pointer',
          background: selectedId === job.id ? 'var(--sel-bg)' : undefined,
        }}
      >
        <Space orientation="vertical" size={4} style={{ width: '100%' }}>
          <Space wrap size={4}>
            <span>{job.name}</span>
            {locked && <Tag color="red">{lockedHint}</Tag>}
          </Space>
          <PairPath src={job.src} dst={job.dst} />
          {cellButtons(job, gatePrimary(job.id, busy || locked))}
        </Space>
      </div>
    );
  };

  // 当前那一格**排在对账之前**：能删东西的那一格先看见，只读的对账垫底（顺序就是危险程度的顺序）。
  // 对账仍然一列摆开 —— 它是只读对账，不是"某一对位置的一格"，混进位置那枚下拉
  // 就等于把两种危险程度摆在同一个选择里。
  return (
    <Space orientation="vertical" size={8} style={{ width: '100%' }}>
      {shown !== undefined && renderCell(shown)}
      {gates.length > 0 && (
        <div className="cell-divider">
          <Tag style={{ color: INK, background: 'transparent' }}>{KIND_LABEL.gate}</Tag>
          <Listy<JobView> items={gates} rowKey={(j) => j.id} itemRender={renderGate} />
        </div>
      )}
    </Space>
  );
}
