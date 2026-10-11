export type JobKind = 'copy' | 'sync' | 'gate';

/** 词表以 Rust 的 `src-tauri/src/config/jobs.rs` 里 `JobKind::word()` 为准（裁定 R56 item 2）——
 *  同一个词同时出现在 runner/commands 的拒绝话术里，界面这里是它的镜像，改一边就要改另一边。
 *  分工：`copy`/`sync`/`gate` 这三个 token 是分支用的，不许跟着文案漂。
 *  R58：`prune` 这一档没有了——清理降级成只读盘点，删除只走 sync。 */
export const KIND_LABEL: Record<JobKind, string> = {
  copy: '复制',
  sync: '让两边对齐（会删）',
  // 2026-10-09：`检查` → `对账`。这不是造新词，是**让孤立的归队**：`对账` 本来就是
  // 这一族在别处的说法（测试夹具那份清单的任务名 `ideahub 封面对账`、设置面「对账的那几条」、
  // 报错「闸门无从对账」——14 行代码都在用），只有这个词表和右列那一段写着「检查」。
  // 而右列已经有一段叫「差异」，两个都像"比一比"——分不清的那一处就是这么来的。
  // **动词用法不动**：`先检查 src 指向与 exclude 是否过宽` 那些是"去看一眼"的意思，
  // 换成「先对账」句子就不通。判据是**这个词在句子里当名词还是当动词**。
  gate: '对账',
};

/** 盘点视图（「本地和云端差在哪」）对哪些任务有意义：**两侧一本地一远端**。
 *  这只是界面侧的**显隐**判断，权威在后台——`run_inventory` 两端同型时会报错并说出是哪一种形状。
 *  两侧的端点类型都由 Rust 的 `endpoint_of` 判定后作为字段带出（裁定 R69）：界面不许再拿正则
 *  自己认一遍盘符/具名 remote——同一条判据在仓里存过两份口径，而 UNC、盘符相对路径这类手写
 *  端点会让两份分开。
 *
 *  **两个方向都要给**（2026-10-09）：原来写死 `srcIsLocal && !dstIsLocal`，
 *  于是「以云端为主」那一格——**删本机硬盘、没有任何撤销**的那一档——恰恰是没有盘点的那一格。
 *  安全上的不对称正好是反的。判据改成"两侧不同型"：`XOR` 就是这句话，方向本身在 `PairCell` 里
 *  读同一批字段（哪一端是 dst），不重算。 */
export function canInventory(job: JobView): boolean {
  return job.srcIsLocal !== job.dstIsLocal;
}

/** 「正在跑：X」那一句。持有者名字是后台按 Scope 包含关系算好交出来的（`JobView.lockedBy`），
 *  这里只做一件事实上的排版：把那个名字放进句子。
 *  卡面与右列四段共用这一个函数——那句"为什么点不动"以前在卡面里现编，
 *  右列那两颗按钮因此完全没受锁约束（点下去才吃到后台的拒绝）。 */
export function runningHint(job: JobView): string | null {
  return job.lockedBy === null ? null : `正在跑：${job.lockedBy}`;
}

/** 与 Rust 的 StatsView 逐字段对齐。rclone 那个「扫到了多少条」的计数不在这里，
 *  也不许上界面——它被读成「在传一万张图」就是一次事故的根因（spec §9 第三行）。 */
export interface StatsView {
  transfers: number;
  totalTransfers: number;
  checks: number;
  totalChecks: number;
  bytes: number;
  totalBytes: number;
  speed: number;
  eta: number | null;
  errors: number;
  deletes: number;
  deletedDirs: number;
  retryError: boolean;
  fatalError: boolean;
  elapsedTime: number;
}

/** 与 Rust 的 TaskState 逐变体对齐（`#[serde(tag = "state", rename_all = "snake_case")]`）：
 *  变体名是 snake_case，变体**字段**不改名，所以是 `dry_run` 而不是 `dryRun`。 */
export type TaskState =
  | { state: 'idle' }
  | { state: 'queued' }
  // `phase` 那一档说的是：列清单那一段一个东西都还没改，而「传输进行中，别关窗」
  // 在那段时间里是个善意的谎。`jobid` 跟着变成可空 —— 列举那一段还没有 rc 的 job 可指，
  // 填个 0 就是让下一个人拿它去轮询一个不存在的 job。
  | { state: 'running'; phase: 'listing' | 'transferring'; jobid: number | null; stats: StatsView | null }
  // `previewed` = 这次实删照着预览的那份清单有几个，由后台算好带出来，前端不许自己重算判定
  // （裁定 R66）。`null` 的意思是**这次不是走对齐确认进来的**（复制、或任何一次试跑），
  // 不是"预览了 0 个"——把 null 当 0 就是把"没人核对过"读成"核对过、没问题"。
  | { state: 'done'; jobid: number; stats: StatsView; dry_run: boolean; previewed: number | null }
  // 终态失败那一句由后台分成两段（人话 + 原文）：这一臂的两个生产者是 rclone 的原话与
  // 我们自己的不变量句，都不该原样端上界面（裁定同 `ShortNote` 那一段）。
  | { state: 'failed'; jobid: number; error: ShortNote }
  | { state: 'unknown'; reason: string };

export interface JobView {
  id: string;
  name: string;
  kind: JobKind;
  src: string;
  dst: string;
  openDir: string | null;
  lockedBy: string | null;
  /** 端点类型由后台的 `endpoint_of` 判定后带出，界面只读，不再自己正则识别（裁定 R69）。 */
  srcIsLocal: boolean;
  dstIsLocal: boolean;
  /** 这一条是不是**这一趟临时立起来**的那一张卡上的一格（裁定 R96/R93）。
   *  界面上三处读它：那句"重开面板就没了"、「存成常用」、「删掉这张临时卡」。
   *  判定在后台（`Runner::is_draft` 只认那张内存表）——编号前缀 `adhoc-` 是给日志文件名用的
   *  （出事时认得出哪张卡哪一格，R99），拿它当判据就是第二份口径，而猜错的方向是收掉一条真配置项。 */
  isDraft: boolean;
  /** 上一次盘点的两个数与它是几点算的（spec §3）。`null` = 这条任务还没盘过：
   *  界面要说"还没算过"，不许显示 0/0，也不许因为缺了就去发起一次列举（裁定 R71 那半句）。 */
  lastSummary: SummaryView | null;
  /** 这一条属于哪**对**位置（后台把两侧各走一遍 `Scope` 的键、排序后拼接）。
   *  方向与 kind 都不进这个键 —— 一张卡是一对位置，方向/模式是卡上的选择
   *  （操作员那句「应该是选择框，而不是 N 条不同的条目」）。
   *  分组只读它，不在前端再算一遍：那把键问的"是不是同一对"与锁问的是同一件事。 */
  pairKey: string;
  /** 这一条在这对里站 a 还是 b（后台 `pair_slot_of` 算的，与 pairKey 同一条规则）。
   *  选择条上那枚「要做什么」把 (kind, 槽位) 摆成四格完整意图，缺的那格不出现。 */
  pairSlot: 'a' | 'b';
  /** a 那一侧是不是本地目录（后台判的）。卡上那两句「以本地为主 / 以云端为主」
   *  读它挑标签，不再拿路径正则认一遍（裁定 R69）；两侧同型时为 false。 */
  aIsLocal: boolean;
}

/** 行上那一句摘要（Rust 侧 `runner::SummaryView`）。两个数与时刻都由后台算，前端只显示：
 *  自己重算就是第二份口径，而"几点算的"这句话一漂，旧读数就会被当成现状。
 *  `orphans` / `pendingUpload` 是按**位置**分的两半（云端多出来的 / 本地多出来的），不是方向；
 *  「这一档要删的是哪一半」由 `dstIsLocal` 说（与 `JobView` 上那两枚 `isLocal` 同一判据）。 */
export interface SummaryView {
  pendingUpload: number;
  orphans: number;
  /** 这一次 dst 那一侧是不是本地目录（后台判的，随摘要一起带出来）。「这一档要删的是上面
   *  哪一个数」读它，界面不许拿路径再判一遍（R69）。 */
  dstIsLocal: boolean;
  /** 空基准那次的诊断随集合一起带出（R29）：为真时那两个数不可信。 */
  emptyBasis: boolean;
  /** 例：`10-02 14:05`（带日期：面板可能整夜开着）。 */
  measuredAt: string;
}

export interface RcdStatus {
  ok: boolean;
  port: number;
  pid: number;
  rcloneVersion: string;
  executeId: string;
  /** 起不来那一句，**后台已经分成两段**（人话 + 原文）。顶栏是常驻面，
   *  原文直接摆上去就等于界面上常驻一个内部名词。null = 后台起来了。 */
  startupError: ShortNote | null;
  locks: Record<string, string>;
  /** 日志目录。空串 = 连配置都读不到（后台没起来时会**试着**从配置读它：那一刻正是最需要
   *  知道"面板跑在哪个家"的时候），这时顶栏不显示这一项。 */
  logDir: string;
  /** 配置文件在哪（空串 = 还不知道）。那句"改完重开面板"以前把路径写死在界面里，
   *  而面板可以跑在别的家上（夹具规程 R68）—— 路径由后台交，界面只渲染。 */
  configPath: string;
  /** 现在有几件事正在做。**后台数好的**：界面数 `locks` 的键就是第二份判定（I5），
   *  而键按 Scope 分、一次动作可以申领多个键，数键会把一件事报成两件。0 时那一格整段不渲染。 */
  activeCount: number;
  /** 这一台面板每一趟**按什么算"变了"**：`--size-only` 那枚例外开着就是 `true`。
   *  判定在后台（`config::jobs::compares_by_size`，读启动时那一份 flags），界面只决定说不说：
   *  那串 flags 从来没过屏，猜一份就是第二份口径。`false` 时顶栏那句整项不出现
   *  （常驻一句没在跑的比法，比不说更坏）。 */
  sizeOnly: boolean;
}

/** 日志面那一份（Rust 侧 `commands::LogTail`）：尾巴 + **它是哪一个文件**。
 *  每次动作新开一个带时间戳的日志文件，所以那句"完整内容在日志文件里"必须带着文件名才指得动；
 *  路径由后台交（`Runner::last_log` 就是为读尾巴已经把路径拿到了），界面不许自己拼。
 *  两个字段都可能是空串：空 = 这个任务还没跑过任何东西。 */
export interface LogTail {
  text: string;
  logPath: string;
}

export interface GateReport {
  // 结论由 Rust 给（'pass' | 'missing' | 'empty_basis'）。
  // **结论**不许从 `missing.length` 反推 —— 空集是第三种状态，那是绕过 Rust 判断的唯一方式
  // （裁定 R29）。`missing` 交的是**未截断的全集**（按条截断只发生在删除类清单那一族），
  // 所以标题里"少了 N 个"数的是这份数据的条数：那是显示，不是第二个判定。
  verdict: 'pass' | 'missing' | 'empty_basis';
  missing: string[];
  caseConflicts: string[][];
  expectedCount: number;
  actualCount: number;
  /** 这一份结论是几点查的（后台格式化好的那一档：今天 14:22 / 昨天 21:40 / 10-01 09:12）。
   *  它交出去的是**成品字符串**：界面再把它解析成"今天"就是拿浏览器的时钟解释后台那一刻，
   *  而要守的正是"一次旧许可被当成现状"这一条（与下面 `PruneReport.measuredAt` 同族）。 */
  measuredAt: string;
}

/** 盘点报告（Rust 侧 `prune::PruneReport`——内核名保留，裁定 R58 点名留下 `prune::orphans`）。
 *  只读：这份数据喂「差异」那一面，界面上不存在任何能触发删除的控件。 */
export interface PruneReport {
  // true = 本地清单是空的而桶里非空：orphans 此刻等于整个桶，那份结论不可信。
  emptyBasis: boolean;
  orphans: string[];
  /** 本地有、云端没有 = 还没上传的（spec §4.2）。绝不是删除理由，只是看得见 copy 要传什么。 */
  pendingUpload: string[];
  localCount: number;
  remoteCount: number;
  /** 这一次的两个位置里 dst 那一侧是不是本地目录（后台按端点类型判好带出来的）。
   *  `orphans` / `pendingUpload` 按位置命名，「这一档删的是哪一半」读这一枚，不许在界面再判路径。 */
  dstIsLocal: boolean;
  /** 这一次盘是几点算的（后台早就带出来了，只是这一面以前没读它——只有左列卡上那句摘要在用）。 */
  measuredAt: string;
  /** 这一次顺手落的那份名单文件在哪（`runner::lists`）。空串 = 没落盘（被拒绝的那一次就是空串）。
   *  路径由后台定：界面不许自己拼一个（那等于猜日志目录与文件名两处口径）。 */
  listFile: string;
  /**
   * 「两边都有、但不是同一份」的那一批。**它不在上面那两个差集里** —— 两边都有的名字在两个差集中
   * 同时消失，所以这一类是集合差看不见的（`prune.rs` 的注释与 `tests/gate.rs` 里那条证人钉着）。
   * 它也是唯一一类跑一次 `sync` 就会被静默冲掉的，所以单独上线。
   *
   * `verdict` 只有 `differs` / `unknown` 两个值，**没有 `same`**：相同的那些根本不该占一行。
   * `newer` 为 `null` 时与 `verdict: 'unknown'` 同时出现 —— **看不出谁新就不许给方向**。
   */
  diverged: DivergedEntry[];
}

/** 一条「两边都在、但不是同一份」。 */
export interface DivergedEntry {
  /** 相对**本地**根的名字（与 `orphans` 同一坐标系）。 */
  name: string;
  /** `differs` = 对得上不上且说得出谁新；`unknown` = 对不上但说不出（时间戳缺或相等）。 */
  verdict: 'differs' | 'unknown';
  /** `remote` = 云端那份更新 —— **按上传会冲掉它**，最要紧的一格；`null` = 说不出。 */
  newer: 'remote' | 'local' | null;
}

/** 一次对齐预览（Rust 侧 `runner::PreviewView`）。`doomed` 是逐条、**相对目标端自己的根**的名字，
 *  也就是 rclone 会删掉、操作员在目标端看得见的那个名字，不需要再拼桶前缀。
 *  `token` 是"人已经看过这一份清单"的唯一凭证：单次消费，跑过一次、或这一对目录上动过别的任务
 *  就作废。界面不许缓存它跨任务复用，也不许在没有 token 的情况下调实删。 */
export interface PreviewView {
  token: string;
  /** 这份清单有几个（后台数好的）。界面那句「删这 N 个」读它，
   *  不读下面那份数组的长度——那是"数行数"那一族，而清单哪天截断着发，界面上的数就悄悄变了。 */
  doomedCount: number;
  doomed: string[];
  /** 同一趟会**补上**的那些有几个（Rust 侧 `MirrorPlan::pending`，后台算的那个数）。
   *  0 = 这一趟没有要补的。界面不许拿两侧计数自己减出它（DESIGN.md 那条"不在 TS 里比较两个数"）。 */
  pending: number;
  srcCount: number;
  dstCount: number;
  listRequests: number;
  /** "3/4"：比例作为字符串出来，前端不许自己解释这个判据（阈值是后台的假设值）。 */
  doomedRatio: string;
}

/** 浏览器左列的一行（Rust 侧 `runner::browser::RemoteRef`）。
 *  `id` 是**发进 rclone 的那个 `fs`**，后台给什么就传回什么：远端那几行带结尾冒号
 *  （`config/listremotes` 回来的是不带冒号的纯名字，F3/F16），本地那几行是 F16 量过的那种
 *  拼法（正斜杠的盘符绝对路径）。界面不许自己拼路径、补冒号或改斜杠——那是在第二处
 *  解释同一个坐标系。 */
export interface RemoteRef {
  id: string;
  label: string;
  isLocal: boolean;
  /** 配对的那一头（2026-10-11，联动对比视图靠它）。交的是**同一个形状的 id**
   * （能直接喂回 `browser_list`），不是给人看的短名；任务里没有这一对时是空串。 */
  pairedWith: string;
}

/** 清单里的一行（Rust 侧 `BrowserEntry`）。`name` 是这个对象在**那一侧根内**的完整相对名
 *  （不是只显示最后一段：同一层里两个不同名字可以共享最后那一段，而写动作按名字执行），
 *  它与写动作要发的那个 `remote` 是同一个串。 */
export interface BrowserEntry {
  name: string;
  size: number;
  isDir: boolean;
}

/** 一句人话 + 可展开的细节。两段都由后台拼：界面只决定哪段先露出来，不许自己拆句（I5）。 */
export interface ShortNote {
  short: string;
  detail: string;
}

/** 列一个前缀的答复（Rust 侧 `BrowserListing`）。三样解释性内容都在后台拼好了：
 *  `filteredNote`（I8：这不是全部，还有几条规则在藏着）、`capabilities`（F6/I5：只解释、
 *  **不禁用**，取不到就是空串）、`canClearPrefix` + `clearNote`（「清空这一层」那两道闸的
 *  **判定**与**那句原因**，同一处算完，见 `clear_gates`）。
 *  前端只许读，不许再从规则条数、能力位或"能不能"自己推任何一个结论（裁定 I5）。 */
export interface BrowserListing {
  /** 这一份清单是关于哪个位置的完整回显（I1）。 */
  path: string;
  entries: BrowserEntry[];
  filteredNote: ShortNote;
  capabilities: ShortNote;
  canClearPrefix: boolean;
  /** 点不动那一句（能清空时两段都是空串）。灰与句子都读这两个字段，不互相推断。 */
  clearNote: ShortNote;
  /** 这一份清单里那些对象的用量（后台按已列到的那一份算，F16）。 */
  totalBytes: number;
  totalCount: number;
  /** 「N 个对象」数的是文件、行里排着目录行，所以那两个数由后台一起交出来。
   *  界面**不许**自己数行（`frontend_browser.rs` 那条禁 `entries.length` 一族的证人守着）。 */
  fileCount: number;
  dirCount: number;
}


/** 一份删除预览（Rust 侧 `runner::browser::BrowserPreview`）：清单 + 一次性的凭证。
 *  `doomed` 就是操作员看到的那一批名字（fs 根内的完整相对名），动手时逐条核对的也是它；
 *  `doomedRatio` 照实报（"3/3" 这种 100% 的形状**不是**拒绝的理由，I2b）；
 *  `filteredNote` 说清有几条排除规则在藏着东西（I8）。
 *  动手只有一个入口 `browserRun(token)`（R78）：做什么由这份凭证自己说。 */
export interface BrowserPreview {
  token: string;
  action: string;
  target: string;
  /** 同上：那句「删这 N 个，补上那 M 个」与列标题里的总数都读这个数。 */
  doomedCount: number;
  doomed: string[];
  totalInPrefix: number;
  doomedRatio: string;
  filteredNote: ShortNote;
}

/** 一次动作跑完之后后台交回来的那一份（Rust 侧 `runner::browser::RunReport`）。
 *  `hasList` 说**这一档有没有一份被核对过的清单**：建目录与一次复制没有，
 *  所以「清单上 N 个 / 两个数一致」那半句整段不出现 —— 把没核对过演成核对通过是这一面最贵的
 *  一种说错话。空清单（`hasList` 为真而 `confirmed` 为 0）是真核对过、结果是零，那半句要说。
 *  `agrees` 是后台做完的**判定**（I7），界面只许读它，不许自己拿两个数再比一次（裁定 I5）。
 *  `logPath` 是那一次动作自己的日志文件——出事时操作员第一个问的是"去哪儿看"。 */
export interface RunReport {
  action: string;
  /** 这次动作落到的完整位置（I1）。 */
  target: string;
  /** 有没有一份被核对过的清单（见上面那段）。渲染那一档只看它，不看数。 */
  hasList: boolean;
  confirmed: number;
  done: number;
  agrees: boolean;
  note: string;
  logPath: string;
}

// ---- 设置面（子项目 5）----
//
// 这一族的类型名在 TS 侧带前缀防撞名（Rust 那边叫 `Intent` / `Change` / `PlanView`），
// **字段名**与 Rust 的 camelCase 逐字对齐：线上传的是字段不是类型名，
// 漂一个字母不会报错，只会让那一格拿到 undefined，而这是界面上最难发现的一种坏。

/** 后台为每一条算好的「这一条站哪一对、站这一对的哪一侧」——与运行那一面 `JobView`
 *  同一组字段、同一份口径（`pair_key_of` / `pair_slot_of` / `endpoint_of`）。
 *  设置面「任务」那一块按 `pairKey` 分组，标题用卡上那一对的短名；它们不参与任何一次写盘。 */
export interface JobPair {
  pairKey: string;
  pairSlot: 'a' | 'b';
  srcIsLocal: boolean;
  dstIsLocal: boolean;
  aIsLocal: boolean;
}

/** 设置面列表里的那一行 = 能改的那几项 + 它所属的那一对。 */
export type JobRow = JobDraft & JobPair;

/** 读侧的整面（Rust 侧 `edit::SettingsView`）。
 *  两份文件的路径都由后台交（顶栏那句「配置文件在」同一族）：界面不许自己指。 */
export interface SettingsView {
  jobsPath: string;
  /** 空串 = 问不出（那一句在 `confNote` 里）。 */
  confPath: string;
  /** 空 = 那一栏正常；非空 = 「这一栏列不出来」+ 原文。 */
  confNote: ShortNote;
  defaults: DefaultsBlock;
  remotes: RemoteBlock[];
  /** 每一条都带着后台算好的那枚配对键（见 `JobPair`），那一块按它分组显示。 */
  jobs: JobRow[];
  /** 有事在跑时后台不但不写，还要说得出是哪一条：这一列就是那几句里的名字。 */
  running: string[];
}

export interface DefaultsBlock {
  rcloneBin: string;
  rcloneBinExists: boolean;
  /** 空串 = 问不出它的版本。不许拿正在跑的那一份的版本顶它：那是两个不同的文件。 */
  rcloneBinVersion: string;
  logDir: string;
  /** 读侧不创建目录也不试写；「还没有」那一句在 `logDirNote` 里，"写不进去"由启动路径说。 */
  logDirExists: boolean;
  logDirNote: ShortNote;
  flags: string[];
  exclude: string[];
}

/** 允许改的四项。这四个 token 是分支用的，与那三句人话（`label`）不是一件事。 */
export interface RemoteField {
  key: 'type' | 'provider' | 'endpoint' | 'region';
  label: string;
  value: string;
}

/** 密钥项：**只有键名**，值在 Rust 的那个类型里就不存在（不是"发之前擦掉"）。 */
export interface SecretState {
  present: string[];
  note: ShortNote;
  /** 可复制的出路文本。 */
  command: string;
}

export interface RemoteBlock {
  name: string;
  fields: RemoteField[];
  secrets: SecretState;
}

/** 任务块里能改的那几项。`id` 是身份，界面上不许改（要换 id 就删了重建，两次确认）。 */
export interface JobDraft {
  id: string;
  name: string;
  jobKind: JobKind;
  src: string;
  dst: string;
  /** 空串 = 没有这一行（保存时会把那一行拿走）。 */
  openDir: string;
  manifest: boolean;
}

/** 一次要写的东西（Rust 侧 `edit::Intent`）。`kind` 那五个 token 与文案不是一件事。
 *  「新加一条」与「改这条」是两个 token，不是一个：合成一条会让打错的 id 静默变成一条新任务。 */
export type ConfigIntent =
  | { kind: 'defaults'; rcloneBin: string; logDir: string; flags: string[]; exclude: string[] }
  | { kind: 'job_add'; id: string; name: string; jobKind: JobKind; src: string; dst: string;
      openDir: string; manifest: boolean }
  | { kind: 'job_set'; id: string; name: string; jobKind: JobKind; src: string; dst: string;
      openDir: string; manifest: boolean }
  | { kind: 'remove_job'; id: string }
  | { kind: 'remote'; remote: string; remoteType: string; provider: string; endpoint: string;
      region: string };

/** 一行差异。判定（哪一行算改了 / 加了 / 拿走了）全在后台，这一面只渲染这三个档。 */
export interface ConfigChange {
  key: string;
  label: string;
  before: string;
  after: string;
  kind: 'changed' | 'added' | 'removed';
}

/** 一份待落盘的方案（Rust 侧 `edit::PlanView`）。`path` 是确认层正文第一行。 */
export interface SettingsPlan {
  token: string;
  path: string;
  /** 任务那一档是那**对**完整位置（名字可以撞，配对才是身份）。 */
  subject: string;
  changes: ConfigChange[];
  /** 连带说明：flags 那句、exclude 那句、改后端那句、删对账那句。 */
  notes: ShortNote[];
}

/** 落盘之后的那一句（Rust 侧 `edit::Applied`）。 */
export interface Applied { path: string; note: ShortNote }
