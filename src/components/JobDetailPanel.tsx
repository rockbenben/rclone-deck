import { Alert, Descriptions, Progress, Space, Table, Typography } from 'antd';

import { NoteLine } from './NoteLine';

import { formatBytes, formatDuration, formatEta, formatFileChanges, formatSpeed } from '../format';
import type { JobView, TaskState } from '../types';

function pct(done: number, total: number): number {
  if (!total) return 0;
  return Math.min(100, Math.round((done / total) * 100));
}

export function JobDetailPanel({
  job, state,
}: {
  job: JobView;
  state: TaskState | undefined;
}) {
  const { kind } = job;
  const stats = state?.state === 'running' ? state.stats : null;
  const dryRun = state?.state === 'running' ? state.dry_run : false;
  return (
    <Space orientation="vertical" size="middle" style={{ width: '100%' }}>
      {(state === undefined || state.state === 'idle') && (
        <NoteLine tone="secondary" note={{ short: '这个任务还没有运行记录。', detail: '' }} />
      )}
      {state?.state === 'unknown' && (
        <Alert type="warning" showIcon title="这个任务现在什么状态说不准"
          description={`${state.reason}。别当成成功，去日志里看清楚再决定要不要重跑。`} />
      )}
      {state?.state === 'failed' && (
        // 那一句由后台分好两段（`ShortNote`）：这一臂的两个生产者是 rclone 的原话和我们的
        // 不变量句，直接摆出来就是"界面上写着 operation error S3 … StatusCode: 403"那种事。
        <Alert type="error" showIcon title="任务失败" description={<NoteLine note={state.error} />} />
      )}
      {state?.state === 'done' && (
        <>
          <Alert type={state.dry_run ? 'info' : 'success'} showIcon
            title={state.dry_run ? '模拟完成，文件未改动' : kind === 'sync' ? '同步完成' : '复制完成'}
            description={<>
              <div>{formatFileChanges(state)}</div>
              {typeof state.previewed === 'number' && <div>执行前清单有 {state.previewed} 个待删除文件</div>}
            </>} />
          <Descriptions size="small" column={1} bordered>
            <Descriptions.Item label="来源"><span className="num">{job.src}</span></Descriptions.Item>
            <Descriptions.Item label="目标"><span className="num">{job.dst}</span></Descriptions.Item>
            <Descriptions.Item label="用时">{formatDuration(state.stats.elapsedTime)}</Descriptions.Item>
          </Descriptions>
          <ChangedFiles label={state.dry_run ? '预计复制的文件（新增或覆盖）' : '已复制的文件（新增或覆盖）'}
            names={state.copied_files} count={state.stats.transfers} />
          <ChangedFiles label={state.dry_run ? '预计删除的文件' : '已从目标删除的文件'}
            names={state.deleted_files} count={state.stats.deletes} />
        </>
      )}
      {state?.state === 'queued' && <Alert type="info" showIcon title="排队中" />}
      {state?.state === 'running' && state.phase === 'listing' && (
        // 列举那一段一个东西都还没改，所以「别关窗，它在后台继续跑」那一句在这里是假的：
        // 那时关窗什么都不会打断。分开说，每一段说各自那一句。
        <Alert type="info" showIcon title="正在核对清单"
          description="正在比较来源和目标的文件，核对后才会开始同步。" />
      )}
      {state?.state === 'running' && state.phase === 'transferring' && (
        // 裁定 R54 第一条 + R55 的更正：这里必须写**实测到的行为**，不是愿望。
        // Task 12 量过：关窗确实不中止已经送出去的传输，但重开的面板会把那个还在跑的后台进程
        // 直接终止再重启一个——"接管"做不到，因为每次启动的凭据都是新生成的随机值、从不落盘，
        // 新面板手里没有旧进程的口令（spec §3.1 情形 2 定的就是终止并重启）。
        // 不用拦关窗做二次确认——webview 先死的时候 JS 跑不动，失效形状变成「窗口关不掉」，
        // 比没提示更糟；孤儿 rcd 由下次启动按三重身份认领回收（Task 7 已建好并测过）。
        // 刻意用 info 而不是 error/warning：这不是故障，是这一态本来就该说清楚的一句话。
        <Alert type="info" showIcon title={dryRun ? '正在模拟文件变更' : '传输进行中'}
          description={dryRun ? '正在统计预计复制和删除的文件。' :
            '关掉窗口不会中止传输，它会在后台继续跑；但下次打开面板时，会先把那个还在跑的后台进程终止、' +
            '再重启一个新的，所以想让这一次跑完就别关窗。被这样打断的那一次，日志会停在「已提交」那一行，' +
            '不会有最后的结果。'
          } />
      )}
      {stats?.retryError && (
        <Alert type="warning" showIcon title="云端在限流"
          description="rclone 正在自行重试，不是卡住了。" />
      )}
      {stats?.fatalError && (
        <Alert type="error" showIcon title="出现了不可恢复的错误" />
      )}
      {stats && !dryRun && stats.deletes > 0 && kind === 'copy' && (
        <Alert type="error" showIcon title="这次传输删除了文件"
          description={`删除 ${stats.deletes} 个文件、${stats.deletedDirs} 个目录。传输类任务不该有删除，请立刻检查配置。`} />
      )}
      {stats && (
        <>
          <Progress
            percent={pct(stats.bytes, stats.totalBytes)}
            status="active"
          />
          <Descriptions size="small" column={2} bordered>
            {/* 读数一律等宽：数字跳动时列不抖（DESIGN.md 的 metric-md）。 */}
            {/* 试跑一个字节都没写：读数仍是 rclone 算出来的计划数，但标签不能说「已传」。
                「做完了 · 这一趟没动任何文件」与「已传 1/1」并排时，后者会被读成真的传了。 */}
            <Descriptions.Item label={dryRun ? '会传 / 共' : '已传 / 共'}>
              <span className="num">{stats.transfers} / {stats.totalTransfers} 个文件</span>
            </Descriptions.Item>
            <Descriptions.Item label="已核对 / 共">
              <span className="num">{stats.checks} / {stats.totalChecks} 个</span>
            </Descriptions.Item>
            <Descriptions.Item label={dryRun ? '要动的数据量' : '数据量'}>
              <span className="num">{formatBytes(stats.bytes)} / {formatBytes(stats.totalBytes)}</span>
            </Descriptions.Item>
            <Descriptions.Item label="速度"><span className="num">{formatSpeed(stats.speed)}</span></Descriptions.Item>
            <Descriptions.Item label="剩余时间"><span className="num">{formatEta(stats.eta)}</span></Descriptions.Item>
            <Descriptions.Item label="已用时">
              <span className="num">{formatDuration(stats.elapsedTime)}</span>
            </Descriptions.Item>
            <Descriptions.Item label="出错"><span className="num">{stats.errors}</span></Descriptions.Item>
            <Descriptions.Item label={dryRun ? '预计删除' : '已删除'}><span className="num">{stats.deletes}</span></Descriptions.Item>
          </Descriptions>
        </>
      )}
    </Space>
  );
}

function ChangedFiles({ label, names = [], count }: { label: string; names?: string[]; count: number }) {
  if (count === 0) return null;
  return (
    <section aria-label={label}>
      <Typography.Text strong>{label}</Typography.Text>
      {names.length < count && (
        <div><Typography.Text type="secondary">
          {names.length === 0 ? '本次未取得文件名。' : `取得 ${names.length} 个文件名，共 ${count} 个文件。`}
        </Typography.Text></div>
      )}
      {names.length > 0 && (
        <Table size="small" showHeader={false} rowKey="path"
          pagination={{ pageSize: 10, hideOnSinglePage: true, showSizeChanger: false }}
          dataSource={names.map((path) => ({ path }))}
          columns={[{ title: '相对路径', dataIndex: 'path', className: 'fs-code' }]} />
      )}
    </section>
  );
}
