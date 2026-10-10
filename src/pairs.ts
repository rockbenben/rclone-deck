import type { JobRow, JobView } from './types';

/** 一张配对卡上的两个选择：模式（会不会删东西）与方向（哪一侧是全集）。
 *  这两个词是**界面上的说法**，不是后台的词：后台只有 kind（copy/sync）与槽位（a/b）。 */
export type Mode = 'add' | 'mirror';
export type Slot = 'a' | 'b';

/** 一对位置 + 四格里哪些真存在。缺的那格是 `null`，选择框里就不该出现它 ——
 *  框里给出一个按下去没有任务的选项，比"N 条条目"更坏。 */
export interface PairGroup {
  key: string;
  a: string;
  b: string;
  /** a 那一侧是不是本地目录（后台判的，界面不再拿路径正则认第二遍，裁定 R69）。 */
  aIsLocal: boolean;
  /** b 那一侧是不是本地目录。**必须两面各自一个标志**：只带 a 那一枚、另一面拿"取反"去推，
   *  两侧都是本地目录的那一对就会被推成"一本地一云端"，而那一对根本没有云端可说。
   *  （真窗口抓到过这个：夹具那对本地↔远端整个落进"两侧同型"那一支。）
   *  它从后台带出来的 `srcIsLocal`/`dstIsLocal` 读，不在这里判路径。 */
  bIsLocal: boolean;
  /** 那两侧**给人认的说法**（短名）：位置那一枚下拉里一行放不下两条绝对路径，
   *  而一屏里几对位置并排时，读的人靠的就是这一小截。它是**说法**，不是判据 ——
   *  判据是上面那两枚标志与后台算的 `pairKey`。撞名怎么退，见 `pairLabels`。 */
  aLabel: string;
  bLabel: string;
  addA: JobView | null;
  addB: JobView | null;
  mirrorA: JobView | null;
  mirrorB: JobView | null;
  /** 这一张卡上**有没有**临时格（成员里那几枚 `isDraft` 的或）。
   *  它是"删掉这一张临时卡"该不该摆出来的判据：一张卡可以是配置格与临时格混着的
   *  （配置里只占了两格，临时卡补另外两格），而收走的是那几格临时的。
   *  这里只做一个或：这一条是不是临时的由后台判（`Runner::is_draft`），界面不认编号前缀。 */
  hasDraft: boolean;
}

/** 划分而不是重算：`pairKey`/`pairSlot`/`aIsLocal` 全部读后台带出来的字段。
 *  这里若再判一次"哪侧是本地、谁排前面"，就是第二份口径 —— 它漂开的那一次，
 *  同一对目录会裂成两张卡，或者两张卡动的是同一棵树。
 *  对账（gate）守的是前缀（spec §5.2），不进配对卡：它由调用方单独一组摆。 */
export function groupJobs(jobs: JobView[]): { pairs: PairGroup[]; gates: JobView[] } {
  const byKey = new Map<string, PairGroup>();
  const gates: JobView[] = [];
  for (const j of jobs) {
    if (j.kind === 'gate') { gates.push(j); continue; }
    let g = byKey.get(j.pairKey);
    if (!g) {
      g = { key: j.pairKey, a: '', b: '', aIsLocal: false, bIsLocal: false,
        aLabel: '', bLabel: '',
        addA: null, addB: null, mirrorA: null, mirrorB: null, hasDraft: false };
      byKey.set(j.pairKey, g);
    }
    // 两条任务写同一对时，路径的先后以 aIsLocal 那一侧为准（后台已经这么排了）。
    const aSide = j.pairSlot === 'a' ? j.src : j.dst;
    const bSide = j.pairSlot === 'a' ? j.dst : j.src;
    if (g.a === '') {
      g.a = aSide; g.b = bSide;
      g.aIsLocal = j.aIsLocal;
      // b 那一面没有自己的字段（后台只交 aIsLocal），但它也不是"取反"推出来的：
      // 这一条任务的另一端点就是 b 侧，读后台给那一端判好的 `srcIsLocal`/`dstIsLocal`。
      g.bIsLocal = j.pairSlot === 'a' ? j.dstIsLocal : j.srcIsLocal;
    }
    if (j.kind === 'copy') { if (j.pairSlot === 'a') g.addA = j; else g.addB = j; }
    else { if (j.pairSlot === 'a') g.mirrorA = j; else g.mirrorB = j; }
    if (j.isDraft) g.hasDraft = true;
  }
  const pairs = [...byKey.values()];
  pairLabels(pairs);
  return { pairs, gates };
}

/** 把一条路径切成段。**不用正则**：R69 那条禁令禁的是"在界面里认路径判类型"，
 *  这里只切段、不判类型 —— 但正则一进来，下一个人就会把它当成判据用。 */
const segments = (p: string) => p.split('/').flatMap((s) => s.split('\\')).filter((s) => s !== '');

/** 一侧的短名：远端那一侧原样（`fxa:c` 本来就又短又认得出是谁）；
 *  本地那一侧从目录名起，`depth` 是往上一层数取的段数。 */
const labelAt = (p: string, isLocal: boolean, depth: number) => {
  if (!isLocal) return p;
  const segs = segments(p);
  return segs.slice(-depth).join('/') || p;
};

/** 一屏里每对位置各自的短名：先只取目录名，只要这一批里有重样的就**整体再往上一层**，
 *  三层之后还是分不出，就退回完整路径 —— 长一点只是难看，认不出是谁会点错家。 */
function pairLabels(groups: PairGroup[]) {
  for (let depth = 1; depth <= 3; depth++) {
    for (const g of groups) {
      g.aLabel = labelAt(g.a, g.aIsLocal, depth);
      g.bLabel = labelAt(g.b, g.bIsLocal, depth);
    }
    const keys = groups.map((g) => `${g.aLabel} ↔ ${g.bLabel}`);
    if (new Set(keys).size === keys.length) return;
  }
  for (const g of groups) { g.aLabel = g.a; g.bLabel = g.b; }
}

/** 设置面「任务」那一块的分组：键读后台那枚 `pairKey`，标题复用卡上那一对的短名
 *  （`pairLabels` 那一条规则），这里只是把**已经存在的分组**显示出来 —— 一行还是一行、
 *  按下去改的还是那一条，写盘路径一个字都不动。
 *
 *  对账那几条单独交出去：卡上没有它们那一格（闸门是只读对账，不在"这两个位置之间动东西"
 *  的四格里），把它们混进配对标题里等于替它们编一个"这一对"。
 *
 *  一处诚实的边界：短名的撞名升级是按**这一份清单里的对**算的。运行那一面多一张临时卡时，
 *  卡上那枚可能比这里深一层 —— 那是同一对位置的两种深度，不是两种说法，临时卡重开就没了。 */
export function groupDraftJobs(jobs: JobRow[]): { pairs: { key: string; label: string; jobs: JobRow[] }[]; gates: JobRow[] } {
  const gates: JobRow[] = [];
  const order: string[] = [];
  const byKey = new Map<string, JobRow[]>();
  for (const j of jobs) {
    if (j.jobKind === 'gate') { gates.push(j); continue; }
    if (!byKey.has(j.pairKey)) { byKey.set(j.pairKey, []); order.push(j.pairKey); }
    byKey.get(j.pairKey)!.push(j);
  }
  const groups: PairGroup[] = order.map((k) => {
    const js = byKey.get(k)!;
    const first = js[0];
    // a / b 两侧从这条任务的槽位与那两枚 is-local 标志里拿 —— 与 `groupJobs` 同一读法，
    // 界面不拿路径再判一次端点（R69 那条禁令）。
    const aSide = first.pairSlot === 'a' ? first.src : first.dst;
    const bSide = first.pairSlot === 'a' ? first.dst : first.src;
    return {
      key: k, a: aSide, b: bSide, aIsLocal: first.aIsLocal,
      bIsLocal: first.pairSlot === 'a' ? first.dstIsLocal : first.srcIsLocal,
      aLabel: '', bLabel: '',
      addA: null, addB: null, mirrorA: null, mirrorB: null, hasDraft: false,
    };
  });
  pairLabels(groups);
  return {
    pairs: groups.map((g) => ({ key: g.key, label: `${g.aLabel} ↔ ${g.bLabel}`, jobs: byKey.get(g.key)! })),
    gates,
  };
}

/** 这一格有没有任务。选择框的选项、按钮的可用性、以及"按下去动的是谁"都只问这一处。 */
export function slotJob(g: PairGroup, mode: Mode, slot: Slot): JobView | null {
  if (mode === 'add') return slot === 'a' ? g.addA : g.addB;
  return slot === 'a' ? g.mirrorA : g.mirrorB;
}

export function availableModes(g: PairGroup): Mode[] {
  return (['add', 'mirror'] as Mode[]).filter((m) => availableSlots(g, m).length > 0);
}

export function availableSlots(g: PairGroup, mode: Mode): Slot[] {
  return (['a', 'b'] as Slot[]).filter((s) => slotJob(g, mode, s) !== null);
}

/** 卡打开时站在哪一格：**默认「只补，不删」**，那一档没有才退到「让两边对齐（会删）」。
 *  不持久化这件事由调用方保证（会话内的 useState，重开面板回到默认）——
 *  记住一个"会删东西"的选择，等于让某天的第一次点击落在最重的那一档上。 */
export function defaultChoice(g: PairGroup): { mode: Mode; slot: Slot } {
  const modes = availableModes(g);
  for (const mode of (['add', 'mirror'] as Mode[])) {
    if (!modes.includes(mode)) continue;
    const slots = availableSlots(g, mode);
    if (slots.length > 0) return { mode, slot: slots[0] };
  }
  return { mode: 'add', slot: 'a' };
}

/** 方向那一格给人读的那一句，**跟着模式走**：同一格"谁在前"在两种模式里做的事不一样 ——
 *  「让两边对齐」那一档它是全集（另一边多出来的要拿走），「只补，不删」那一档它只是**从哪边补到哪边**。
 *  以前这一句只有一种写法（删…多余的），于是"不删"那一格的方向写着"删" —— 那正是操作员
 *  分不清"哪个是合适的"的那一种：界面上每一行都在说同一件最重的事。
 *  两侧里有一侧是本地才说"本地/云端"，两侧同型（都本地或都 remote）时说那两侧的**短名**
 *  （`pairLabels` 算好的）—— 说"云端"就是替配置说谎，而塞两条绝对路径会把那一格撑破。 */
export function directionLabel(g: PairGroup, slot: Slot, mode: Mode): string {
  const winner = slot === 'a' ? g.aLabel : g.bLabel;
  const other = slot === 'a' ? g.bLabel : g.aLabel;
  const winnerIsLocal = slot === 'a' ? g.aIsLocal : g.bIsLocal;
  const otherIsLocal = slot === 'a' ? g.bIsLocal : g.aIsLocal;
  if (winnerIsLocal === otherIsLocal) {
    return mode === 'add'
      ? `从 ${winner} 拿到 ${other}`
      : `以 ${winner} 为主（删 ${other} 多余的）`;
  }
  const w = winnerIsLocal ? '本地' : '云端';
  const o = winnerIsLocal ? '云端' : '本地';
  return mode === 'add' ? `从${w}补到${o}` : `以${w}为主（删${o}多余的）`;
}

export function modeLabel(mode: Mode): string {
  return mode === 'add' ? '只补，不删' : '让两边对齐（会删）';
}

/** 这一对位置上**实际存在**的那几格，每一格算成一句人话。
 *
 *  2026-10-09：选择条上原来是「模式」「方向」两枚框，而每一格的真名 = 模式 + 方向
 *  （`只补，不删` + `从本地补到云端`）。于是读者要**把两枚框的话拼起来**才知道这一格要干什么 ——
 *  而拼错了拼出来的还是一句通顺的话，这是最难发现的一类错。改成一枚框之后，
 *  一个选项**自己就是一个完整的意图**，不给人拼的机会。
 *
 *  `tone` 是危险程度信号：会删的那两档吃暖色（`MODE_VAR.mirror`），不删的吃中性墨。
 *  它跟着**这一格**走而不是跟着模式走 —— 一枚框里两种危险程度并排，颜色就是唯一的区分。
 *
 *  顺序：先不删的两档、后会删的两档，且每组内按槽位排 —— 列表**从上往下越来越重**，
 *  与 `defaultChoice` 的默认档（不删那档）一致，于是最上面那一项就是重开面板会站的那一格。
 */
export interface Intent {
  mode: Mode;
  slot: Slot;
  label: string;
  tone: string;
  /** 这一格会不会删东西 —— 选中的时候用来给那一枚框上色，别处不用。 */
  danger: boolean;
}

export function intentOptions(g: PairGroup): Intent[] {
  const out: Intent[] = [];
  for (const mode of (['add', 'mirror'] as Mode[])) {
    for (const slot of availableSlots(g, mode)) {
      out.push({
        mode,
        slot,
        label: `${modeLabel(mode)} —— ${directionLabel(g, slot, mode)}`,
        tone: mode === 'mirror' ? 'var(--kind-warm)' : 'var(--ink-2)',
        danger: mode === 'mirror',
      });
    }
  }
  return out;
}

/** 一格的稳定键。`mode:slot` 而不是数组下标：配置在两次轮询之间少了一格时，
 *  按下标会把选择指到**另一格**上去 —— 那一格是另一对方向上的事。 */
export const intentKey = (mode: Mode, slot: Slot): string => `${mode}:${slot}`;

/** ~~`summaryNumbers`~~ 2026-10-09 撤掉了。
 *
 *  它给的是**统计口径**：「待传 N · 云端多余 M · T 算的」。而这个面板存在的理由是
 * 「看清这一次会动哪些文件、**会不会删**，再按下去」—— 于是在**最要紧的那一处**（对齐确认层，
 * 按下去就要删东西）"云端多余"恰恰是**要删掉的那些**，却用了全屏最中性的词。
 * 两处显示它的地方（当前那一格 / 对齐确认层）因此各自按**后果口径**现说：
 * 「会传 N 个 · 会删 M 个」或「不会删」。它们读的是同一份 `SummaryView`，一个数都不许重算 ——
 * 要钉的是"念的是后台那份数据"，不是"两句措辞相同"（措辞按所在位置分叉：格上那一块是带颜色的
 * 排版，确认层里是嵌在一句话里的短读数）。留着一个没人调的 `summaryNumbers` 只会让下一个人
 * 以为还有第三种说法。 */
