import { Segmented, Space, Typography } from 'antd';

import { GatePanel } from './GatePanel';
import { InventoryPanel } from './InventoryPanel';
import { JobDetailPanel } from './JobDetailPanel';
import type { GateReport, JobView, LogTail, PruneReport, TaskState } from '../types';
import { runningHint } from '../types';

export type Tab = 'progress' | 'diff' | 'check' | 'log';

/** 右列那四段（导出是给 `Panel.tsx` 的持久层校验用的：**名单只有一份**，
 *  这里摆的段与那里认的值不能各抄一份 —— 抄两份的下一档就是"存了但开不出来"）。 */
export const TABS: { label: string; value: Tab }[] = [
  { label: '进度', value: 'progress' },
  { label: '差异', value: 'diff' },
  { label: '对账', value: 'check' },
  { label: '日志', value: 'log' },
];

/**
 * 右列：同一份上下文的四个面，**换着看不换内容**。
 *
 * 旧形状是 `selected?.kind === 'gate' ? <GatePanel/> : <JobDetailPanel/>` —— 选中"检查"类任务
 * 就翻不到进度与日志，而人看完报告第一个动作就是想知道刚才发生了什么。所以四段**一直挂着**，
 * 只用 display 切换（与浏览器/对子两面挂着同一个理由：卸载重建会把这一屏的状态弄没）。
 *
 * `tab` 由外面管着，不在这里 `useState`：卡上那颗「看差异」既要算差异、也要把右列翻到「差异」
 * 那一面 —— 段落在本组件里发明的话，那颗按钮就指挥不动它。
 *
 * 这一列**开头那一行说的是哪张卡**：四段读的都是选中那一条的上下文，而左列的选中态只有一层底色。
 * 没有这一行，"刚点的那颗按钮在右列演的是哪条任务"就得回头数底色。名字取 `job.name`（后台字段），
 * `src → dst` 不在这里重复 —— 卡上已经写着了。
 * 反过来那条也一样成立，而那条分工是：**名字只在这一行说一遍**。
 * 卡上曾经跟着抄一行，而出厂那些名字把方向写进了字符串，于是同一件事在格上隔两行说了三遍
 * （模式一枚框、方向一枚框、名字那一行）。撤的是那一行，不是那份身份 —— 判据在
 * `frontend_pairs.rs` 的 `the_rail_says_which_task_it_is_showing` 与
 * `the_pair_cell_shows_the_paths_and_leaves_the_name_to_the_rail` 两条手里，各钉一面。
 *
 * 没选中任务时不进四段，只有一句「先选一个任务。」：**四段共用这一句、这一个形状**。
 * 以前每段各说各的（一段一个图标 + 一句"从左边选一个任务"、两段一行次要文字、
 * 一段三行长文、一段一个字都没有）—— 同一个屏幕的四个面把"现在没有对象"说成四种样子，
 * 人就得以四种方式各认一次。段控件本身还在：它说的是"这一屏分几段"，与有没有对象无关。
 */
export function JobContextPanel({
  job, state, log, inventory, gate, busy, tab, onTab, onRunInventory, onRunGate,
}: {
  job: JobView | null;
  state: TaskState | undefined;
  log: LogTail | null;
  inventory: PruneReport | null;
  gate: GateReport | null;
  busy: boolean;
  tab: Tab;
  onTab: (next: Tab) => void;
  onRunInventory: () => void;
  onRunGate: () => void;
}) {
  if (job === null) {
    return (
      <Space orientation="vertical" size="middle" style={{ width: '100%' }}>
        <Segmented value={tab} onChange={(v) => onTab(v as Tab)} options={TABS} />
        <Typography.Text type="secondary" className="fs-note">先选一个任务。</Typography.Text>
      </Space>
    );
  }
  // 这一段那两颗"现在就重算一次"的按钮与卡面上同一颗**共用同一道闸**：
  // 后台的 `run_gate` / `run_inventory` 第一件事就是申领那两棵树的锁，所以以前从右列点下去
  // 不是"能不能点"的问题，而是必然吃一条拒绝——而卡面那一颗是灰的、还写着谁挡着。
  // `lockedBy` 就是那份数据（`Runner::blocking_owner` 按 Scope 包含关系算出来的持有者名）。
  const locked = job.lockedBy !== null;
  const lockedHint = runningHint(job);
  return (
    <Space orientation="vertical" size="middle" style={{ width: '100%' }}>
      <Typography.Text strong>{job.name}</Typography.Text>
      <Segmented value={tab} onChange={(v) => onTab(v as Tab)} options={TABS} />
      <div style={{ display: tab === 'progress' ? 'block' : 'none' }}>
        <JobDetailPanel kind={job.kind} state={state} />
      </div>
      <div style={{ display: tab === 'diff' ? 'block' : 'none' }}>
        <InventoryPanel report={inventory} loading={busy} off={locked} lockHint={lockedHint}
          deletesLocal={job.dstIsLocal} jobId={job.id}
          onRefresh={onRunInventory} />
      </div>
      <div style={{ display: tab === 'check' ? 'block' : 'none' }}>
        <GatePanel report={gate} running={busy} off={locked} lockHint={lockedHint} onRun={onRunGate} />
      </div>
      <div style={{ display: tab === 'log' ? 'block' : 'none' }}>
        {/* 那一句以前停在"完整内容在日志文件里"就没了：日志**每次动作新开一个带时间戳的文件**
            （`TaskLog::create`），所以那一格不指着哪一个文件，就等于没指。路径由后台交
            （`tail_log` 为了读尾巴本来就把路径拿到了），界面不拼文件名；中段省略 + 原生
            `title` 全文 + 复制入口，与顶栏那一格同一个形状（`DESIGN.md`：路径不是密码）。 */}
        <Typography.Text type="secondary" className="fs-note">
          {log !== null && log.logPath !== '' ? '最近这一段 · 完整内容在这个文件 ' : '最近这一段'}
        </Typography.Text>
        {log !== null && log.logPath !== '' && (
          <Typography.Text className="num" copyable={{ text: log.logPath }}>
            <span className="log-ellipsis" title={log.logPath}>{log.logPath}</span>
          </Typography.Text>
        )}
        <pre className="log-tail">{log === null || log.text === '' ? '还没有日志。' : log.text}</pre>
      </div>
    </Space>
  );
}
