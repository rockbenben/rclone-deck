import { Alert, Descriptions, Progress, Space } from 'antd';

import { NoteLine } from './NoteLine';

import { formatBytes, formatDuration, formatEta, formatSpeed } from '../format';
import { KIND_LABEL } from '../types';
import type { JobKind, StatsView, TaskState } from '../types';

function pct(done: number, total: number): number {
  if (!total) return 0;
  return Math.min(100, Math.round((done / total) * 100));
}

/** 进度面：**只在选中了任务之后被调用**（"没选中"那一句归 `JobContextPanel` 统一说，四段同一句）。
 *  所以这里不再自己演一个空态，也不再收任务 id —— 这一面读的全是 `state` 那一份读数。 */
export function JobDetailPanel({
  kind, state,
}: {
  kind: JobKind;
  state: TaskState | undefined;
}) {
  const stats: StatsView | null =
    state?.state === 'running' ? state.stats
    : state?.state === 'done' ? state.stats
    : null;
  // 试跑的 `deletes` 是 rclone 自己**算出来会删**的数量，一个都没真删。
  // 不区分这一档，同一屏上会同时出现「试跑完成，没有改动任何文件」和「这次对齐删除了 N 个文件」。
  // `running` 也带 `dry_run`：试跑从第一帧起就是计划数，不能等终态才改口。
  const dryRun =
    state?.state === 'running' ? state.dry_run
    : state?.state === 'done' ? state.dry_run
    : false;
  // 后台带出来的预览数（`TaskState::Done.previewed`）。`null` = 这一次不是走对齐确认进来的，
  // **不许**当 0 用，也不许在这里拿它跟 `stats.deletes` 比——判定是 Rust 的活：
  // 两者不等的这一次早就被 `transfer_invariant_violation` 变成 `Failed` 了，`Done` 里根本等不到。
  const previewed = state?.state === 'done' ? state.previewed : null;

  return (
    <Space orientation="vertical" size="middle" style={{ width: '100%' }}>
      {/* 「还没跑过」这一档与差异 / 对账两段走**同一个形状**（一句明面 + 「为什么？」展开）。
          以前这一段一个字都没有，而它是四段里默认停着的那一段 —— 于是一块 700×400 的卡片
          在打开面板的头几秒里是全屏最大的一块空白，"还没跑过"被读成了"没画完"。
          走 `NoteLine` 而不是 `Empty` 图标：四段共用一种空态（`frontend_pairs` 的证人钉着）。 */}
      {(state === undefined || state.state === 'idle') && (
        <NoteLine tone="secondary" note={{
          // 明面只说状态，**不指路**：「看差异」那颗在两侧同型的那一格根本不在卡上
          // （`canInventory` 读的是端点类型），指过去就是对着空气找下一步；
          // 而它在的那一格里，那颗就摆在左边 —— 屏上指着一颗就在眼前的话，读者要多走一步。
          short: '这一格还没跑过。',
          detail: '「看看会动什么」只回答「要不要动」，回答不了「差在哪几个」——那一份名字在「差异」那一段，'
            + '到那一段上算一次，它只列不动，不真传。第一次跑建议就停在那一步。',
        }} />
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
        <Alert type={state.dry_run ? 'info' : 'success'} showIcon
          // 口径表第一档：「做完了 · <动作>」。以前这一档自己造了两个说法（「任务完成」与
          // 「试跑完成，没有改动任何文件」），而设置面与浏览器那两处早就照表写了 ——
          // 同一个"成了"在屏上有三种词。试跑那一档说的是"没动文件"这件事，它不是另一种"完成"。
          title={state.dry_run ? '做完了 · 这一趟没动任何文件' : `做完了 · ${KIND_LABEL[kind]}`}
          // 试跑答的是"要不要动"（计数），答不了"差在哪几个"（名字）。操作员报的那一类里
          // 点了「看看会动什么」之后找不到差异，就是因为这一句没把路指出来 —— 而"差异"那一面是**另一份数据**
          // （面板自己算的集合差），不会跟着试跑自动填上。这里指路，不代替它算。
          description={state.dry_run
            ? '它只回答「要不要动」，回答不了「差在哪几个」。要看那几个的名字，翻上面「差异」那一段点「看差异」——那一步只列不动。'
            : undefined} />
      )}
      {state?.state === 'queued' && <Alert type="info" showIcon title="排队中" />}
      {state?.state === 'running' && state.phase === 'listing' && (
        // 列举那一段一个东西都还没改，所以「别关窗，它在后台继续跑」那一句在这里是假的：
        // 那时关窗什么都不会打断。分开说，每一段说各自那一句。
        <Alert type="info" showIcon title="正在核对清单"
          description="这一步只读：它把这两个位置各列一遍，算出哪些多出来、哪些还缺着。列完才轮到那两条拒绝决定删不删。" />
      )}
      {state?.state === 'running' && state.phase === 'transferring' && (
        // 裁定 R54 第一条 + R55 的更正：这里必须写**实测到的行为**，不是愿望。
        // Task 12 量过：关窗确实不中止已经送出去的传输，但重开的面板会把那个还在跑的后台进程
        // 直接终止再重启一个——"接管"做不到，因为每次启动的凭据都是新生成的随机值、从不落盘，
        // 新面板手里没有旧进程的口令（spec §3.1 情形 2 定的就是终止并重启）。
        // 不用拦关窗做二次确认——webview 先死的时候 JS 跑不动，失效形状变成「窗口关不掉」，
        // 比没提示更糟；孤儿 rcd 由下次启动按三重身份认领回收（Task 7 已建好并测过）。
        // 刻意用 info 而不是 error/warning：这不是故障，是这一态本来就该说清楚的一句话。
        <Alert type="info" showIcon title="传输进行中"
          description={
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
      {stats && dryRun && kind === 'sync' && (
        // 试跑时 rclone 报的 `deletes` 是**它自己算出来会删**的数量。这是与面板集合差互相独立的
        // 一条读数（spec §4.1 的交叉核对）：把它显示成"会删掉 N 个"，而不是"删掉了 N 个"。
        // 这一档今天从界面上走不到：对齐那一格只剩「让两边对齐」一颗，它发的是带确认的实删，
        // 而「看看会动什么」只在「只补，不删」那一格（kind 是 copy）。留着是因为后台仍会答
        // `dry_run` 为真的 sync 任务（老状态、夹具、以及测试侧那条对数的腿），不是给操作员按的。
        <Alert type="info" showIcon
          title={`试跑算出这次会删掉 ${stats.deletes} 个文件、${stats.deletedDirs} 个目录，一个都没真删`}
          description="这是 rclone 自己算的数，与面板清单预览列出的那个数互相核对：同一批东西时应该相等，不等就说明两边看的不是同一批东西。" />
      )}
      {stats && !dryRun && stats.deletes > 0 && kind === 'sync' && (
        // 对齐任务删目标端多出来的东西是它该干的活——把这条报成"请检查配置"是假警报。
        <Alert type="info" showIcon title={`这次对齐删除了 ${stats.deletes} 个文件、${stats.deletedDirs} 个目录`}
          description="对齐就是按本地清单把目标端多出来的部分去掉，这一次删除在预期之内。" />
      )}
      {stats && !dryRun && kind === 'sync' && (
        // 两个数**并列摆出来**：一个是你确认过的那份清单有几个，一个是 rclone 真删了几个。
        // 这里不比大小、不下结论（裁定 R66）——不等的那一次在后台就已经是失败，那句话由上面
        // 「任务失败」那一条带着 `error` 原文说出来；在这里再算一遍是一段永不执行的代码。
        <Alert type="info" showIcon
          title={`给你看的那份：${previewed ?? '—（这一趟不是走对齐确认进来的）'}，真删的：${stats.deletes} 个`}
          description="预览那个数是面板自己列清单算的，删除那个数是 rclone 报的——两条路互相独立，摆在一起就是核对。对不上的话这里不会有绿色。" />
      )}

      {stats && (
        <>
          <Progress
            percent={pct(stats.bytes, stats.totalBytes)}
            status={state?.state === 'failed' ? 'exception' : 'active'}
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
            <Descriptions.Item label="删除"><span className="num">{stats.deletes}</span></Descriptions.Item>
          </Descriptions>
        </>
      )}
    </Space>
  );
}
