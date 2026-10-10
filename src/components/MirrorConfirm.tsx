import { Alert, Button, Modal, Space, Table, Typography } from 'antd';

import { NoteLine } from './NoteLine';

import type { JobView, PreviewView, ShortNote } from '../types';

/** src 是本地目录 = 以本地为主（删云端多余的）；src 是 remote = 以云端为主（删本地多余的）。
 *  判据来自后台：`JobView.srcIsLocal` 是 Rust 的 `endpoint_of` 算好后带上线的字段，这里不许
 *  再拿正则认一遍端点（裁定 R69）——自己判的那份会把 UNC、盘符相对路径认成本地以外的东西，
 *  而前提是"云端是全集"的那句话配上的内核其实在删云端多余的对象。 */
function localWins(job: JobView): boolean {
  return job.srcIsLocal;
}

/** 显示预算：确认层里**每一页**列这么多名字（spec §4.1 第 2 步）。
 *  列不全是**显示**的限制，不是省略证据——所以这是分页的页数，不是截断的口子：
 *  剩下的名字翻页就在这一层里看得见，不必关掉这层去别处找。 */
const LIST_BUDGET = 20;

/**
 * 对齐的确认层（spec §4.1 第 2 步）。它只在该任务预览过之后有内容可显示。
 *
 * 两条批准的路：
 *  · 默认这一条**不带名单** —— 凭证出自 `confirm_mirror`，动手那一刻列出来的就是准，
 *    守这一趟的是"占比过半"与"这一侧空着"那两道拒绝；
 *  · 点过「重新预览」再批的那一条带着名单，凭证出自 `preview_mirror`，后台拿到锁之后
 *    会重列一次并**逐条比对**，对不上就不动手。
 *  上面那句"这里显示的清单可能已经过期"只适用于第二条，而它必须写在界面上 ——
 *  它是"你看到的就是被删的"那个担保的全部内容，也是它唯一的来源。
 */
export function MirrorConfirm(props: {
  job: JobView;
  preview: PreviewView | null;
  loading: boolean;
  error: ShortNote | null;
  /** 这一份凭证是否已经交出去过（后台取走即消费，而且排在复核之前）。
   *  与浏览器那一层同一档：层不关、清单留着让人对着看，但那颗红的不亮着。 */
  spent: boolean;
  onCancel: () => void;
  onPreview: (id: string) => void;
  /** `token` 为 `null` 说的是「没看过逐条名单，批的是这一趟」——那种凭证要现登记，
   *  所以这一步交给外面去拿（发起点只有一处，话术也就只有一处）。 */
  onConfirm: (id: string, token: string | null) => void;
}) {
  const { job, preview, loading, error, spent } = props;
  // 前提按方向分叉（R60）：哪一侧是全集，被删掉的就是另一侧多出来的东西。
  // 这两句各钉一条证人（src-tauri/tests/frontend_copy.rs），改文案就红——合成一句就有一个方向在骗人。
  const premise = localWins(job)
    ? '本地是全集：云端那些本地没有的对象会被删掉。如果你直接往云上传过图，先跑一次下载把本地补全，再对齐。'
    : '云端是全集：本地那些云端没有的文件会被删掉。这批如果还没上传，先跑一次上传，再对齐。';
  // 这一层以前只说"拿走"，没说这一趟**同时**会把缺的那一批补上/拿下 —— 于是它读起来像"这一档只能删东西"。
  // 操作员照着这个读就不按红色的那颗：`r2-mirror-local-wins` 两遍试跑、零次真跑（日志全是 `dry_run=true`），
  // 该上传的 17 个一直没上去。这句里一个数字都不写：它是"还没清单"那一档也说得出的话。
  // 红色的那颗现在带两个数（`doomedCount` 与 `pending` 都读后台交出来的，界面不自己减），
  // 因为按下去之前那一刻，人要看的就是"这一趟两头各动多少"。
  const alsoMoves = localWins(job)
    ? '这一趟按对齐跑：不只拿走云端多余的那些，也会把本地有、云端没有的那批补上去。'
    : '这一趟按对齐跑：不只拿走本地多余的那些，也会把云端有、本地没有的那批拿下来。';
  const doomed = preview?.doomed ?? [];

  return (
    <Modal open width={720}
      title={preview ? `会删掉 ${preview.doomedCount} 个（目标端共 ${preview.dstCount} 个）` : '确认这一趟对齐'}
      onCancel={props.onCancel}
      footer={[
        <Button key="again" onClick={() => props.onPreview(job.id)} loading={loading}>
          重新预览
        </Button>,
        <Button key="cancel" onClick={props.onCancel}>取消</Button>,
        <Button key="go" danger type="primary"
          disabled={loading || (spent && preview !== null)}
          onClick={() => props.onConfirm(job.id, preview?.token ?? null)}>
          {/* 三档话各说各的（默认那一档是"没有名单"）：
              · 没看过名单 —— 批的是**这一趟**，数还没列，所以这里**不编一个数**；
              · 看过名单 —— 说得出删多少、补多少，那颗红的亮着就是在说"这些名字我看过"；
              · 名单是空的 —— 照它跑一次，它同样要先过动手前那两道拒绝。
              `spent` 只锁"看过名单"那一档：那份已经交出去了，再按只会拿到同一句拒绝；
              没名单那一档没有陈旧可言 —— 再确认一次就是一份新凭证。
              两趟等待（列清单 / 按下去之后后台重列比对）都在这颗上说话：以前列举途中它写着
              「还没清单，先重新预览」，等于人家正在算的时候叫他再去按一颗。 */}
          {loading ? '正在核对清单…'
            : spent && preview ? '这一份已经交出去了，要看新的名单就再预览一次'
            : !preview ? '照对齐跑（这一趟先核对一遍）'
            : preview.doomedCount === 0 ? '这一份没有要删的，照它跑一次'
            // 那个数读后台交出来的 `doomedCount`，不读数组长度：确认层显示上是按页截的，
            // 而"删这 N 个"说的是后台动手时要核对的那一整份（数行数归界面就是第二份口径）。
            : `删这 ${preview.doomedCount} 个`
              + (preview.pending > 0 ? `，补上那 ${preview.pending} 个` : '')}
        </Button>,
      ]}
    >
      <Space orientation="vertical" size="middle" style={{ width: '100%' }}>
        {/* 那两条完整位置必须由这一层**自己**说出来：左列现在一张卡是一对位置、方向是卡上的一格选择，
            旧形状里"这一行脚下那一句 `src → dst`"没有了 —— 而按红色那颗之前，这是最后一处
            能看见"动的到底是哪两棵树、修剪的是哪一侧"的地方（换成选择框之后更要看得见）。
            顺序读的是后台那条不变量（镜像的 src 是全集、dst 是被修剪的那一侧），不在这里再判一次方向：
            那一判就是第二份口径，而它漂开的那一次正好删掉没人看过的那一侧。 */}
        <Alert type="warning" showIcon title="这一步不可撤销"
          description={<Space orientation="vertical" size={2} style={{ width: '100%' }}>
            <Typography.Text>
              {`这一趟把 ${job.dst} 修剪成 ${job.src} 那样：多出来的删掉，缺的补上。`}
            </Typography.Text>
            <Typography.Text>{premise}</Typography.Text>
            <Typography.Text>{alsoMoves}</Typography.Text>
          </Space>} />
        <Alert type="info" showIcon
          title={preview
            // 占比原样带着后台给的那一串（"3/4"）：这里一除，判据就变成界面解释的了（R29 同一条口径）。
            // 「那条线是假设值」留着——它是可行动的（被挡住时知道那不是自己的东西真的危险）；
            // 原来跟在后面的那半句"量到真实分布之前会一直偏保守"删了：那说的是我们还没量，
            // 操作员对它做不了任何事，而 `DESIGN.md` 的时间/口径那一节把这一类推理点名留在日志和文档里。
            // 同一族的另一层（`LayerConfirm`）早就只说"照实报，它不是放行或挡住的依据"。
            ? `还可能删掉若干空目录——清单只列文件。这次要删的占比 ${preview.doomedRatio}；挡住一半的那条线是假设值（偏保守），它挡住的时候会说出来。`
            : '还可能删掉若干空目录——清单只列文件。挡住一半的那条线是假设值（偏保守），它挡住的时候会说出来。'} />
        {error && <Alert type="error" showIcon title="没改成" description={<NoteLine note={error} />} />}
        {!preview && (
          // 这一档必须自己把"你批的是什么"说完：没有名单不等于没有担保 ——
          // 动手前那两道拒绝（占比过半 / 这一侧空着）照旧在，而想看逐条名字有一条现成的路。
          // 那两个数**不在这里现算**：读的是后台摘要那一份（`summaryNumbers` 只有一处说法）。
          // 但措辞是**后果口径**而不是统计口径 —— 这一层是全app最要紧的一刻（"按下去就要删东西"），
          // 而「云端多余」在这个位置上恰恰是**要删掉的那些**，用中性词说它等于把最重的那件事说轻了。
          // 卡面上那一格（`JobListView` 的 `summaryLine`）已经是这个口径，两处说同一件事用同一个词。
          <Alert type="info" showIcon title="这一趟还没列逐条名单"
            description={<Space orientation="vertical" size={2} style={{ width: '100%' }}>
              <Typography.Text>
                按下去之后，后台先把这两个位置各列一遍：多出来的删掉、缺的补上。
                要删的占到一半以上，或者这一侧一个对象都没有，它就拒绝并说出为什么。
              </Typography.Text>
              <Typography.Text type="secondary">
                {job.lastSummary
                  ? `上次核对：会传 ${job.lastSummary.pendingUpload} 个 · 会删
                    ${job.lastSummary.dstIsLocal ? job.lastSummary.pendingUpload : job.lastSummary.orphans} 个`
                    + ` · ${job.lastSummary.measuredAt} 算的`
                  : '这两个位置还没核对过。要先看逐条名字，点「重新预览」。'}
              </Typography.Text>
            </Space>} />
        )}
        {preview && preview.doomedCount === 0 && (
          <Alert type="success" showIcon title="这两个位置现在是齐的，没有要删的东西。" />
        )}
        {preview && doomed.length > 0 && (
          // 分页列全，而不是"截断 + 其余去日志"：每一个名字都要在这层里**够得着**。
          // 旧写法只列前 20 行，剩下的人被指去右列「日志」那一段——而此刻模态正盖在上面，
          // 要看就得先关掉这层、先放弃眼前这份清单；那一面以前还不说日志文件叫什么。
          // 浏览器那一层（`LayerConfirm`）头上写的就是同一仓的口径：那是 spec §9 点名不要的
          // 半截操作。这里一直是那个反面，现在两边同一形状（antd 的标准 `Table` + 分页）。
          <Table size="small" rowKey={(k) => k}
            dataSource={doomed}
            pagination={{ pageSize: LIST_BUDGET, hideOnSinglePage: true }}
            columns={[{
              title: `这些名字就是要删的那些（共 ${preview.doomedCount} 个，每页列 ${LIST_BUDGET} 个）`,
              dataIndex: undefined, key: 'name',
              render: (_: unknown, k: string) => <Typography.Text code className="fs-code">{k}</Typography.Text>,
            }]}
          />
        )}
        {/* 这一句对两个分支都成立：清单为空时「照它跑一次」同样要先过后台的重列比对，
            所以复核的话要说在分支外面，不能只在有东西可删时才出现。
            前半句「重新列一遍并逐条比对」是这一层做的事（两侧各列一次，集合不等就不动），
            与浏览器那层共用的只有开头那半句——那一层核对的是"那个位置现在有什么"，
            不是同一件事，硬写成同一句就是在其中一层骗人。
            没有清单那一支**不再复述算法**（2026-10-09）：它原写「先列一遍这两个位置」，
            而正上方那条「这一趟还没列逐条名单」已经逐字说了同一件事 —— 同一层里说两遍，
            第二遍只会被读成另一件事。这一支只留**担保**：你按下去之后才列，那两条拒绝照样生效。 */}
        <Typography.Text type="secondary">
          {preview ? '确认之后后台会重新列一遍并逐条比对，跟这份对不上就不会动手。'
                   : '按下去之后才开始列；上面那两条拒绝照样生效。'}
        </Typography.Text>
      </Space>
    </Modal>
  );
}
