import { Alert, Button, Modal, Space, Table, Typography } from 'antd';

import { NoteLine } from './NoteLine';

import type { BrowserPreview, ShortNote } from '../types';

/** 确认层里那份逐条清单：每页列多少个名字（与对齐那一层同一个显示预算）。
 *  这里用**分页**而不是"截断 + 其余去日志"：分页让每一个名字都在这一层里够得着，
 *  而"只列前 20 个、剩下的信那个数"正是 spec §9 点名不要的那种半截操作。 */
const LIST_BUDGET = 20;

/**
 * 破坏性动作的确认层（R78：一次动作只有这一个动手入口）。
 *
 * 三条不能弯的：① 正文第一行就是这个动作落到的完整位置（后台拼好的 `preview.target`，I1）；
 * ② 拒绝**不关层** —— 那句拒绝要说在操作员正对着的这份清单上，撞名那一次还要能就地改名字再来一遍；
 * ③ 交出去的是这一份预览的一次性 token，所以吃过一次拒绝的凭证（`spent`）不再亮着。
 */
export function LayerConfirm(props: {
  open: boolean;
  busy: boolean;
  preview: BrowserPreview | null;
  deletion: boolean;
  /** 非空 = 这一次是改名来的，撞名之后要能就地换一个名字再来一遍。 */
  canRenameAgain: boolean;
  spent: boolean;
  error: ShortNote | null;
  onRepreview: () => void;
  onCancel: () => void;
  onConfirm: () => void;
  onRenameAgain: () => void;
}) {
  const { open, busy, preview: p, deletion, canRenameAgain, spent, error } = props;
  // 复核那一句与对齐那层共用开头半句（「确认之后后台会重新列一遍」），后半各说各自真做的事：
  // 这一层再看的是**那一个位置**现在有什么，不是拿一份逐条清单去比集合。
  // 旧写法是「再看一次那个位置」——"看一次"比"重新列一遍"弱，人会读成瞄一眼。
  const recheck = '确认之后后台会重新列一遍那个位置，跟这份对不上就不会动手。';
  const footer = [
    <Button key="again" loading={busy} onClick={props.onRepreview}>重新预览</Button>,
    ...(canRenameAgain ? [
      <Button key="rename" onClick={props.onRenameAgain}>换个名字再来一次</Button>,
    ] : []),
    <Button key="cancel" onClick={props.onCancel}>取消</Button>,
    <Button key="go" danger type="primary" disabled={p === null || busy || spent} onClick={props.onConfirm}>
      {/* 三档各有自己的话。0 那一档向镜像层学：它不是"没得删所以别按"，而是一次**真的没有
          东西可动**的跑法（照它跑一次也改不了什么），所以那颗照旧能按、话说清后果为零。
          以前这里会写着「我知道，删这 0 个」并挂着「这一步不可撤销」——同一屏两句互相打脸。
          那个数读后台的 `doomedCount`：这一层的清单是按页翻的，数组在界面手里不代表全集。 */}
      {p === null ? '还没清单，先重新预览'
        : spent ? '这一份已经交出去了，再来一次要看新的清单'
        : p.doomedCount === 0 ? '这一份没有要动的，照它跑一次'
        : `我知道，${deletion ? '删这' : '搬走这'} ${p.doomedCount} 个`}
    </Button>,
  ];
  return (
    <Modal open={open} width={720} title={p ? p.action : '还没清单'} onCancel={props.onCancel} footer={footer}>
      <Space orientation="vertical" size="middle" style={{ width: '100%' }}>
        {p && <Typography.Text code>{p.target}</Typography.Text>}
        {/* 那一档长什么样由"这份清单有几个"决定，而三档共用的复核话只有一份（写三遍就会漂成三种）。
            空清单不是"没得看"：它是**真核对过、结果是零**（与 `RunReport.has_list` 同一族口径），
            所以那一档不再说「这一步不可撤销」——没东西可撤销——而复核那一句仍然要说。 */}
        {p === null || p.doomedCount === 0 ? (
          <Alert type={p === null ? 'warning' : 'success'} showIcon
            title={p === null ? '还没清单' : '这一层没有要动的东西。照它跑一次也不会改任何东西。'}
            description={p === null ? '点下面「重新预览」列一遍；列不出来时会说什么原因。' : recheck} />
        ) : (
          <Alert type="warning" showIcon
            title={deletion ? '这一步不可撤销' : '源位置会空出来'}
            description={recheck} />
        )}
        {error && <Alert type="error" showIcon title="没改成" description={<NoteLine note={error} />} />}
        {p && <NoteLine note={p.filteredNote} tone="warning" />}
        {p && (
          <Table size="small" rowKey={(row) => row.name}
            dataSource={p.doomed.map((name) => ({ name }))}
            pagination={{ pageSize: LIST_BUDGET, hideOnSinglePage: true }}
            columns={[{
              title: `这些名字就是要动的那些（共 ${p.doomedCount} 个，每页列 ${LIST_BUDGET} 个）`,
              dataIndex: 'name', key: 'name',
              render: (name: string) => <Typography.Text code className="fs-code">{name}</Typography.Text>,
            }]}
          />
        )}
        {p && (
          <Typography.Text type="secondary" className="fs-note">
            这一份的占比是 {p.doomedRatio}（照实报，它不是放行或挡住的依据）。
            上面列的名字就是后台动手时要核对的那些。
          </Typography.Text>
        )}
      </Space>
    </Modal>
  );
}
