import { Alert, Button, Space, Table, Typography } from 'antd';

import { HIT_OFF, LockHint } from './LockHint';
import { NoteLine } from './NoteLine';

import type { PruneReport } from '../types';

/**
 * 「差异」那一面——只读盘点视图（裁定 R58）。
 * 这里**没有任何删除控件**：不勾选、不确认、不删除。曾经这一屏是 prune 的
 * 勾选 + 删除入口，但用户存在直接上传到云端的对象，这份清单分不出"改名垃圾"
 * 与"云-only 正式资产"，给它配删除按钮就是把误删做成一键。删除只走对齐任务。
 *
 * 说明的摆法照 `DESIGN.md` 那两条：一句人话留在明面，细节收进「为什么？」展开。
 * 以前这一屏从上到下有四段解释（一个标题、一条四行的 info Alert、一段三行的空态、
 * 两张表的列标题各把方向复述一遍），同一件事在半屏里说四遍；而那个标题
 * 是全仓唯一一处 `Typography.Title`，渲染出来 20px —— 契约只有五档字阶（18/15/13.5/12.5/11），
 * 它是第六档。段控件上已经写着「差异」，这一面不再自带标题。
 */
export function InventoryPanel({
  report, loading, off, lockHint, onRefresh, deletesLocal,
}: {
  report: PruneReport | null;
  loading: boolean;
  /** 这一对目录正被别的任务占着（后台交的数据）。卡上那颗因为同一份数据是灰的，
   *  这一段以前不认它：亮着，点下去必然吃一条拒绝。 */
  off: boolean;
  lockHint: string | null;
  onRefresh: () => void;
  /** 这一格站在**哪个方向**：对齐要删掉的是目标端（dst）那一侧。
   *  2026-10-09 起这一面在**两个方向**都摆得出来（原先只给「以本地为主」），
   *  而两个方向里"要删的那一栏"不是同一栏 —— 不标出来，操作员就得自己推哪一栏会被删。
   *  `true` = 这一格会删**本机**上的东西（以云端为主）。
   *  判据读后台已经判好的 `dstIsLocal`（`JobView` 的两枚 is-local 标志，裁定 R69），
   *  这一面不拿路径再认一遍端点类型。 */
  deletesLocal: boolean;
}) {
  return (
    <Space orientation="vertical" size="middle" style={{ width: '100%' }}>
      <Space>
        <LockHint title={lockHint}>
          <Button loading={loading} disabled={off} style={off ? HIT_OFF : undefined}
            onClick={onRefresh}>列一遍</Button>
        </LockHint>
        {report && (
          <Typography.Text type="secondary">
            本地 {report.localCount} 个，云端 {report.remoteCount} 个 · {report.measuredAt} 算的
          </Typography.Text>
        )}
      </Space>

      {/* 这一次顺手落的那份名单（`runner::lists`）：路径由后台交，界面只渲染。
          它存在的意义就是"拿这份文件去跟两周前那份 diff"，所以这一行必须**看得见、可复制**——
          藏在日志里等于没给。空串那一档（被拒绝的那一次）什么都不露。 */}
      {report && report.listFile !== '' && (
        <Space size={6} wrap>
          <Typography.Text type="secondary" className="fs-caption">名单存成了</Typography.Text>
          <Typography.Text copyable={{ text: report.listFile }} title={report.listFile}
            className="fs-code">{report.listFile}</Typography.Text>
        </Space>
      )}

      {/* 「只给你看」这一句是常驻解释，明面留一句、细节进展开。内容一个字没少：
          它说的还是"这里分不出垃圾与正式对象，所以不放删除按钮"。 */}
      <NoteLine note={{
        short: '这里只给你看，要删请走对齐任务。',
        detail: '这份清单是「云端有、本地没有」的对象。看不出来哪些是改名留下的垃圾、'
          + '哪些是直接传到云上的正式对象，所以这里不放删除按钮。'
          + '真要删，去跑对齐任务——它的确认会先说清楚会删掉什么。',
      }} />

      {!report && !loading && (
        <NoteLine note={{
          short: '还没算过。',
          // 只指这一面上真有的那一颗（「列一遍」）。以前拿「先试跑」作对比，而对齐那一档卡上
          // 已经没有那颗了 —— 指一个不存在的入口，等于让人对着空气找下一步。
          detail: '这一面要按「列一遍」单独算一次才填得上：跑一次传输、开一次清单都不顺带把它填上。',
        }} />
      )}

      {report?.emptyBasis && (
        // 后台在空基准时直接返回错误，正常路径到不了这里；这一条是"万一上游漏了，
        // UI 也不把整桶渲染成一份可信清单"的第二层闸（裁定 R29）。
        <Alert type="error" showIcon
          title="这份清单不可信"
          description={`面板这次在本地一个文件都没扫到，而云端有 ${report.remoteCount} 个。\
差额自然就是「全部」。这几乎一定是源目录指错了或忽略规则放宽过头了，先修好对账基准。`} />
      )}

      {/* 两张表各自判断，不合成一条"两边都齐"：两个空集是三种不同的形状。
          列标题只写方向本身（「本地有、云端没有」）：以前那两串是"名字 + 括号里把名字解释一遍"，
          同一件事在一行标题里说两遍，而方向本身就是这里唯一要说的信息。 */}
      {report && !report.emptyBasis && report.pendingUpload.length === 0 && (
        <Alert type="success" showIcon title="本地也没有云端还缺的对象" />
      )}

      {report && !report.emptyBasis && report.orphans.length === 0 && (
        <Alert type="success" showIcon title="云端没有本地找不到的对象" />
      )}

      {report && !report.emptyBasis && report.pendingUpload.length > 0 && (
        <Table
          size="small"
          rowKey={(k) => k}
          pagination={{ pageSize: 20, showSizeChanger: false, hideOnSinglePage: true }}
          dataSource={report.pendingUpload}
          columns={[{
            // 标题外面包一个 `span`，**不是**用 `columns[].style`：后者不是 antd `ColumnType` 的
            // 属性，会被**静默丢掉** —— 量出来的现象是两个标题颜色一模一样，而读代码看不出原因。
            // 暖色是全屏唯一那枚 `--kind-warm`，不是新颜色；只在这一格要删本机的那一栏出现。
            title: <span style={deletesLocal ? { color: 'var(--kind-warm)' } : undefined}>本地有、云端没有</span>,
            dataIndex: undefined,
            render: (_, k) => k,
          }]}
        />
      )}

      {report && !report.emptyBasis && report.orphans.length > 0 && (
        <Table
          size="small"
          rowKey={(k) => k}
          pagination={{ pageSize: 20, showSizeChanger: false, hideOnSinglePage: true }}
          dataSource={report.orphans}
          columns={[{
            // 反过来：云端这一栏只在「以本地为主」那一个方向上才是删除候选。
            // 标错方向比不标更坏 —— 所以两种情况都要显式给值，不靠"没有 style 就是默认"。
            title: <span style={deletesLocal ? undefined : { color: 'var(--kind-warm)' }}>云端有、本地没有</span>,
            dataIndex: undefined,
            render: (_, k) => k,
          }]}
        />
      )}
    </Space>
  );
}
