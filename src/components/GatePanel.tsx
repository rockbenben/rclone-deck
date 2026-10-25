import { Alert, Button, Space, Table, Typography } from 'antd';

import { HIT_OFF, LockHint } from './LockHint';
import { NoteLine } from './NoteLine';

import { KIND_LABEL } from '../types';
import type { GateReport, JobKind } from '../types';

export function GatePanel({
  report, running, off, lockHint, onRun, jobKind,
}: {
  report: GateReport | null;
  running: boolean;
  /** 这一对目录正被别的任务占着（后台交的数据）。以前这一段不认它：那颗按钮亮着，
   *  点下去必然吃一条拒绝，而卡上同一颗是灰的、还写着「正在写这棵目录树：谁」。 */
  off: boolean;
  lockHint: string | null;
  onRun: () => void;
  /** 右列这一格现在说的是哪条任务。对账任务与复制/对齐任务是**分开的两类**：
   *  `run_gate` 只认 `kind = gate`，在一条复制任务上摆「开始对账」等于摆一颗必然被拒的按钮。 */
  jobKind: JobKind;
}) {
  // **这一格不是对账任务时不摆那颗按钮**（2026-10-24 用户实测：按下去只说「这是复制任务」）。
  // 对账任务在左列下面单独一行；要新建一条到设置里「新加一条」。这段话说的是去处，不是拒绝。
  if (jobKind !== 'gate') {
    return (
      <NoteLine note={{
        short: `这一条是${KIND_LABEL[jobKind]}任务，不是对账任务。`,
        detail: '对账要用单独建的「对账」任务：在左列下面选那一条，或到「设置」里的「任务」新加一条。'
          + '这一页现在没有可以按的对账动作。',
      }} />
    );
  }
  // **结论**只从 Rust 给的 verdict 读，不从 `missing.length` 反推 —— 空集是第三种状态，
  // 「一个都没缺」和「没得比」必须长得不一样（裁定 R29）。
  // 同样不许在这里补一个 `report.expectedCount === 0` 的判断（裁定 R56 item 1）：
  // 「桶里也有东西没扫到」这一档的判归 Rust 所有，判定必须作为数据走出去，不是让界面再算一遍。
  const passed = report?.verdict === 'pass';
  const noBasis = report?.verdict === 'empty_basis';

  return (
    <Space orientation="vertical" size="middle" style={{ width: '100%' }}>
      <Space>
        <LockHint title={lockHint}>
          <Button type="primary" loading={running} disabled={off}
            style={off ? HIT_OFF : undefined} onClick={onRun}>开始对账</Button>
        </LockHint>
        {report && (
          <Typography.Text type="secondary">
            应有 {report.expectedCount} 个，实际 {report.actualCount} 个 · {report.measuredAt} 查的
          </Typography.Text>
        )}
      </Space>

      {/* 「还没查过」是这一段自己的空态（与「差异」那一段同一形状：一句 + 展开）。
          以前这里一个字都没有：四段里只有这一段空着的时候不说话，而"报告还没生成"与
          "报告生成了、结论是都齐"在界面上长得一模一样。 */}
      {!report && !running && (
        <NoteLine note={{
          short: '还没查过。',
          detail: '这一面要跑一次才有结论：它把本地目录里那些文件名与网盘里现有的对一遍，'
            + '缺了会点名列出来。跑过的那一份不会自己重算——本地又动了就得再跑一次。',
        }} />
      )}

      {report && (
        <Alert
          type={passed ? 'success' : noBasis ? 'warning' : 'error'}
          showIcon
          title={passed
            ? '都在，可以发布'
            : noBasis
              ? '没得比'
              : `少了 ${report.missing.length} 个，先别发布`}
          description={passed
            ? undefined
            : noBasis
              ? '这不是「都齐了」。这次面板在本地一个文件都没扫到，所以它并不知道文章引用了什么，\
也就没法说「全都传上去了」。先检查这个任务的源目录，以及配置里的忽略规则是不是放宽过头了。'
              : '下面这些对象已经被引用了，但还没传到网盘。现在发布，用户会看到 404，而且这个 404 会被边缘缓存一年。'}
        />
      )}

      {report && report.missing.length > 0 && (
        <Table
          size="small"
          pagination={{ pageSize: 20, showSizeChanger: false, hideOnSinglePage: true }}
          dataSource={report.missing.map((k) => ({ key: k }))}
          columns={[{ title: '缺这个对象', dataIndex: 'key' }]}
        />
      )}

      {report && report.caseConflicts.length > 0 && (
        <Alert
          type="warning"
          showIcon
          title="有几组文件名只差大小写"
          description={
            <ul style={{ margin: 0, paddingLeft: 18 }}>
              {report.caseConflicts.map((g) => (
                <li key={g.join('|')}>{g.join('  ↔  ')}</li>
              ))}
            </ul>
          }
        />
      )}
    </Space>
  );
}
