import { Alert, Button, Divider, Select, Space, Table, Tooltip, Typography } from 'antd';
import { useEffect, useState } from 'react';
import type { ReactNode } from 'react';

import { Crumbs } from './Crumbs';
import { NoteLine, noteTip } from './NoteLine';
import { HIT_OFF, LockHint } from './LockHint';
import { ResultDock } from './ResultDock';
import type { DockTone } from './ResultDock';
import { ThumbCell, looksLikeImage } from './ThumbCell';
import { formatBytes } from '../format';
import type { BrowserEntry, BrowserListing, ShortNote } from '../types';

const TABLE_PAGE = 100;

/** 网格一次摆多少格（2026-10-11）。** 200 是个折中：一屏满宽大约看 40–60 格，
 *  所以首批已经把整屏铺满还多留了几屏；再多只是白铺 DOM，再少会让人连点「再看」。 */
const GRID_BATCH = 200;

/** 这一栏摆什么、点下去叫谁，全在这里；一栏自己不知道另一栏的存在。
 *  位置、清单、每颗按钮"点不动的那句原因"都是外面算好交进来的 —— 那些判据有后台的份，
 *  不在这一面重算（`I5`）。 */
export interface PaneView {
  title: string;
  /** 这一栏是不是远端那一侧。判据从外面给：不许从"标题写着远端"反推（那是拿显示当数据）。 */
  isRemote: boolean;
  rootLabel: string;
  choices: { label: string; value: string }[];
  fs: string | null;
  prefix: string;
  listing: BrowserListing | null;
  listError: ShortNote | null;
  busy: boolean;
  /** 这一栏里被点中的那一个文件（跨栏动作的源），null = 没点中。 */
  pickedName: string | null;
  hints: { download: ShortNote; push: ShortNote; move: ShortNote; visible: ShortNote };
  /** 「清空这一层」点不动那两句（后台一次算完；能清空时两段都是空串）。 */
  clearNote: ShortNote;
  clearBlocked: boolean;
  result: { tone: DockTone; title: string; detail: ReactNode; logPath: string } | null;
}

/** 点下去叫谁。`BrowserPane` 不知道另一栏的存在，也不知道后台的形状。 */
export interface PaneActions {
  onSelectPosition: (fs: string) => void;
  onJump: (prefix: string) => void;
  onEnter: (name: string) => void;
  onPick: (name: string | null) => void;
  onMkdir: () => void;
  onRename: (row: BrowserEntry) => void;
  onPush: () => void;
  onMoveHere: () => void;
  onDownload: (row: BrowserEntry) => void;
  onDeleteOne: (row: BrowserEntry) => void;
  onDeleteVisible: () => void;
  onPurge: () => void;
  onRmdir: (row: BrowserEntry) => void;
}

/**
 * 浏览器的一栏。DESIGN.md 的 Layout 那条把它钉成固定五层，自上而下：
 * 栏标题 + 面包屑 → 位置读数 → 那句提示 → 动作条（导航｜组织｜传输｜删除）→ 表格 → 结果条。
 */
export interface PaneProps extends PaneView, PaneActions {
  /** 对面那一栏没站位置 —— 本栏占满整宽，图片按网格摆（2026-10-11）。
   *  两栏都在时它还是半宽表格：并排比的时候一行一个名字比网格好读（要左右对同一行）。 */
  wide?: boolean;
  /** **只翻图，不动东西**（2026-10-11）。
   *
   *  这一栏开着多数时候是为了翻那堆图片，而不是搬它。那一排「新建目录 / 上传到这里 /
   * 移到这里 / 删掉这一层看到的 N 个 / 清空这一层」在翻图时一个都用不上，却占了半屏 ——
   * 所以收进一个折叠里。行内的「改名 / 删这一个 / 进入」也跟着收：它们是**动某一个**，
   * 与翻图无关。留一颗粒在行上，折叠起来也不至于找不着。
   *
   * 判据由**外面**给（进没进比对），不在这一层自己算 —— "翻图"与"没有可操作对象"
   * 两种情形会长出两套不同的收法，那样更怪。
   */
  compact?: boolean;
}

export function BrowserPane(v: PaneProps) {
  const { listing, hints, clearNote } = v;
  const a: PaneActions = v;
  const entries = listing?.entries ?? [];
  // 满宽那一栏：图片按网格摆、目录与非图片另列一表（2026-10-11）。
  //
  // 为什么要分两半而不是一张混合网格：目录点一下要进进去，网格里给目录一格位置
  // 是把"能进去的"和"能看的"混成同一排；而非图片没有缩略图，摆进网格就是一片灰。
  // 分开之后网格里**每一格都是一张能看的图**——这一面开着是为了翻图，不是为了读文件清单。
  // 网格的判据从"满宽且本地"放宽到"本地 **或** 远端"（2026-10-11 加了云端预览）：
  // 两栏都在时（并排比）仍然走表格 —— 那时要的是左右对同一行；
  // 只有一栏时（翻图）才走网格，而翻图这一面本地云端都可能是它。
  const grid = entries.filter((e) => !e.isDir && looksLikeImage(e.name));
  const rest = grid.length > 0 ? entries.filter((e) => !grid.includes(e)) : entries;

  // 分批摆的那一档。换目录/换位置时归零：留着上一处的批数，进新目录第一屏就摆两三千格。
  const [batch, setBatch] = useState(GRID_BATCH);
  const gridKey = `${v.fs ?? ''}|${v.prefix}`;
  const [lastKey, setLastKey] = useState(gridKey);
  if (lastKey !== gridKey) { setLastKey(gridKey); setBatch(GRID_BATCH); }
  const shown = grid.slice(0, batch);

  // `compact` 时那族"动文件"的动作有没有展开。翻图时默认收起（默认站得住的是"看"），
  // 换目录/换位置都收回 —— 上一次展开的是**上一个地方**的意图，带着过来只会多占一屏。
  const [showMove, setShowMove] = useState(false);
  useEffect(() => { setShowMove(false); }, [v.fs, v.prefix]);
  return (
    <Space orientation="vertical" size={8} style={{ width: '100%' }}>
      <div>
        <Typography.Text strong>{v.title}</Typography.Text>
        <Crumbs root={v.rootLabel} prefix={v.prefix} onJump={a.onJump} />
      </div>
      <Space wrap size={4}>
        <Select popupMatchSelectWidth={false} style={{ minWidth: 170 }} placeholder="选一个位置" value={v.fs ?? undefined}
          onChange={(next: string) => a.onSelectPosition(next)}
          options={v.choices.map((r) => ({ label: r.label, value: r.value }))} />
        {listing && (
          <Tooltip title={listing.path}>
            <Typography.Text type="secondary" ellipsis className="fs-code" style={{ maxWidth: 220 }}>
              {listing.path}
            </Typography.Text>
          </Tooltip>
        )}
        {listing && (
          <Typography.Text type="secondary" className="num">
            {/* 三个数都是**这一层**的（后台只列一层，见 `browser_list`）：文件夹里还有多少东西不在这里说，
                进去才看得见。少了"这一层"三个字，这个字节数就会被读成"这个前缀往下一共多大"。 */}
            这一层 {listing.fileCount} 个文件 · {listing.dirCount} 个文件夹 · {formatBytes(listing.totalBytes)}
          </Typography.Text>
        )}
      </Space>
      {v.listError !== null && (
        <Alert type="error" showIcon title="这一栏列不出来" description={<NoteLine note={v.listError} />} />
      )}
      {/* I8：那句"这里显示的不是全部"两段都由后台拼好，这里只决定先露摘要。能力位同理（只解释、不禁用）。 */}
      {listing && <NoteLine note={listing.filteredNote} tone="warning" />}
      {listing && <NoteLine note={listing.capabilities} />}
      {/* 动作条与表格**都等"选了一个位置"才摆**。以前它们无条件在屏上，于是刚打开这一面
          每一栏先摆出五颗灰按钮，其中一颗的**名字**就是一句解释（「等这一层列出来才能删」）
          —— 那句话是 Tooltip 的内容，被写成按钮文字之后一屏有两排点不动的东西，
          而"这一栏还没选位置"这件事反而没有一句正经话在说。
          判据仍在 `hints` 那一组（每颗点不动的理由一个字没少），这里只是不提前摆那一排。 */}
      {v.fs === null ? (
        <Typography.Text type="secondary" className="fs-note">选一个位置，这一栏会列出那里有什么。</Typography.Text>
      ) : (
        <>
      {/* 动作按族分组（导航｜组织｜传输｜删除）：五颗平铺时"清空这一层"和"上一级"看着是同一级的事。
          **`compact`（只翻图）时把"组织/传输/删除"整族收进一个折叠**（2026-10-11）——
          翻图时一个都用不上却占半屏。导航（「上一级」）留在外面：它是**翻图本身要用的**，
          收进去等于让人为了进两层目录先点一次折叠。 */}
      <Space size={4} wrap>
        <Button size="small" disabled={v.busy || v.prefix === ''} onClick={() => a.onJump(parentOf(v.prefix))}>
          上一级
        </Button>
        {!v.compact && (
        <>
        <Divider vertical />
        <Button size="small" disabled={v.busy || v.fs === null} onClick={a.onMkdir}>新建目录</Button>
        <Divider vertical />
        {v.isRemote && (
          <LockHint title={noteTip(hints.push)}>
            <Button size="small" style={hints.push.short ? HIT_OFF : undefined}
              disabled={v.busy || hints.push.short !== ''}
              onClick={a.onPush}>
            上传到这里
            </Button>
          </LockHint>
        )}
        <LockHint title={noteTip(hints.move)}>
          <Button size="small" style={hints.move.short ? HIT_OFF : undefined}
            disabled={v.busy || hints.move.short !== ''}
            onClick={a.onMoveHere}>
            移到这里
          </Button>
        </LockHint>
        <Divider vertical />
        <LockHint title={noteTip(hints.visible)}>
          <Button size="small" style={hints.visible.short ? HIT_OFF : undefined}
            disabled={v.busy || hints.visible.short !== ''}
            onClick={a.onDeleteVisible}>
            {/* N 读后台按这一份清单算好的那个数，不在这里数行。还没列出来时不许摆一个字母占位符
                给人看（子项目 3 记下的缺口）：那颗本来就是灰的，说一句人话就行。
                「这一层」是范围，不是修辞：这颗只动直接子项，文件夹里面的东西归「清空这一层」。 */}
            {listing === null ? '等这一层列出来才能删' : `删掉这一层看到的 ${listing.totalCount} 个`}
          </Button>
        </LockHint>
        <LockHint title={noteTip(clearNote)}>
          {/* 灰由后台那个布尔决定，Tooltip 由后台那两句决定，两件事各有各的来源。
              "不能清空却没给句子"是后台的 bug，`clear_gates` 的证人钉着它；这里不设备用文案 ——
              编一句"为什么点不动"正是这一面最不该做的事。 */}
          <Button size="small" danger style={v.clearBlocked ? HIT_OFF : undefined}
            disabled={v.busy || v.clearBlocked} onClick={a.onPurge}>
            {/* 与左边那颗的区别必须写在按钮上：这颗连文件夹里面的一起清掉（`operations/purge`
                删的是整棵前缀，后台给它列的也是整棵那份清单）。两颗都写"这一层"的话，
                范围差一整棵子树，而界面上看不出来。 */}
            清空这一层（连里面的一起）
          </Button>
        </LockHint>
        </>
        )}
      </Space>
      {/* 收起来的那一族给一颗粒在行上：折叠起来不等于找不着。 */}
      {v.compact && (
        <Typography.Link className="fs-caption" onClick={() => setShowMove(true)}>
          要新建、传输或删除这一层里的东西？
        </Typography.Link>
      )}
      {v.compact && showMove && (
        <Space size={4} wrap style={{ marginTop: 4 }}>
          <Button size="small" disabled={v.busy} onClick={a.onMkdir}>新建目录</Button>
          <LockHint title={noteTip(hints.move)}>
            <Button size="small" style={hints.move.short ? HIT_OFF : undefined}
              disabled={v.busy || hints.move.short !== ''} onClick={a.onMoveHere}>移到这里</Button>
          </LockHint>
          <LockHint title={noteTip(hints.visible)}>
            <Button size="small" danger style={hints.visible.short ? HIT_OFF : undefined}
              disabled={v.busy || hints.visible.short !== ''} onClick={a.onDeleteVisible}>
              删掉这一层看到的 {listing?.totalCount ?? 0} 个
            </Button>
          </LockHint>
          <LockHint title={noteTip(clearNote)}>
            <Button size="small" danger style={v.clearBlocked ? HIT_OFF : undefined}
              disabled={v.busy || v.clearBlocked} onClick={a.onPurge}>清空这一层</Button>
          </LockHint>
          <Button size="small" type="text" onClick={() => setShowMove(false)}>收起</Button>
        </Space>
      )}
      {/* 图片网格（只有满宽的本地那一栏有）。`auto` 让它一摆出来就取图，不用点。
          每格点一下仍然**选中那一个文件**（跨栏动作的源），与表格里点行是同一个动作 ——
          网格只是换了摆法，不是另一个入口，所以别的东西（上传到这里 / 移到这里）照旧能用。 */}
      {grid.length > 0 && (
        <>
        <div className="thumb-grid">
          {shown.map((e) => (
            <button key={e.name} type="button"
              className={v.pickedName === e.name ? 'thumb-cell picked' : 'thumb-cell'}
              title={e.name}
              onClick={() => a.onPick(v.pickedName === e.name ? null : e.name)}>
              <ThumbCell fs={v.fs ?? undefined} prefix={v.prefix} name={e.name} auto box={112} remote={v.isRemote} />
              <span className="thumb-name">{e.name}</span>
            </button>
          ))}
        </div>
        {/* 分批：上面那层 `IntersectionObserver` 治的是"**图**什么时候取"，这一层治的是
            "**格子**什么时候存在"。两千张图一次铺 2000 个 button，DOM 本身就够卡一阵，
            而其中一千九百多格在屏外、永远不会被看见。
            所以先摆 `GRID_BATCH` 格，底下给一颗继续摆的按钮——不做虚拟滚动是因为它要接一个
            新依赖、且滚动条长度会跳；这一档没有滚动条（整页滚），加一颗按钮就够了，
            而翻到第 200 张时人早该换目录了。 */}
        {grid.length > shown.length && (
          <div style={{ textAlign: 'center', marginTop: 8 }}>
            <Button size="small" onClick={() => setBatch((n) => n + GRID_BATCH)}>
              再看 {Math.min(GRID_BATCH, grid.length - shown.length)} 张（还剩 {grid.length - shown.length} 张）
            </Button>
          </div>
        )}
        </>
      )}
      <Table size="small" rowKey={(row) => row.name}        dataSource={rest}
        pagination={{ pageSize: TABLE_PAGE, hideOnSinglePage: true }}
        scroll={{ x: 'max-content' }}
        onRow={(row) => ({ onClick: () => a.onPick(row.isDir ? null : row.name) })}
        columns={[
          {
            title: '名字', dataIndex: 'name', key: 'name',
            render: (name: string, row) => (
              <Space size={4}>
                {/* 网格已经列过的那一批，这一格**不再画一遍图**：同一张图在屏上出现两次，
                    第一次那一格就白占了地方。`grid` 里有它就只给名字（它仍可被选中）。
                    没有网格的形状（两栏并排、云端那一栏）才在这一格里直接摆图。 */}
                {grid.includes(row) ? (
                  <Typography.Text code className="fs-code">{name}</Typography.Text>
                ) : (!v.isRemote && !row.isDir) ? (
                  <ThumbCell fs={v.fs ?? undefined} prefix={v.prefix} name={name} auto />
                ) : (
                  <Typography.Text code className="fs-code">{name}</Typography.Text>
                )}
                {row.isDir && <Typography.Text type="secondary">（目录）</Typography.Text>}
                {v.pickedName === row.name && (
                  <Typography.Text type="success">已选中</Typography.Text>
                )}
              </Space>
            ),
          },
          {
            title: '大小（字节）', dataIndex: 'size', key: 'size', width: 100,
            // 数字成列（DESIGN.md 的 metric-md）：只有数值那一格等宽，行名与标签不走等宽。
            render: (size: number, row) => (row.isDir ? '' : <span className="num">{size}</span>),
          },
          {
            title: '动作', key: 'act',
            // 行里的按钮不冒泡到 `onRow`：点「删这一个」不该顺手把那一行设成跨栏动作的源。
            render: (_: unknown, row) => (row.isDir ? (
              <Space size={4} wrap onClick={(e) => e.stopPropagation()}>
                <Button size="small" disabled={v.busy} onClick={() => a.onEnter(row.name)}>进入</Button>
                <Button size="small" disabled={v.busy || v.fs === null} onClick={() => a.onRmdir(row)}>
                  删空目录
                </Button>
              </Space>
            ) : (
              <Space size={4} wrap onClick={(e) => e.stopPropagation()}>
                {v.isRemote && (
                <LockHint title={noteTip(hints.download)}>
                  <Button size="small" style={hints.download.short ? HIT_OFF : undefined}
                    disabled={v.busy || hints.download.short !== ''} onClick={() => a.onDownload(row)}>
                      下载
                    </Button>
                  </LockHint>
                )}
                <Button size="small" disabled={v.busy} onClick={() => a.onRename(row)}>改名</Button>
                <Button size="small" disabled={v.busy || v.fs === null} onClick={() => a.onDeleteOne(row)}>
                  删这一个
                </Button>
              </Space>
            )),
          },
        ]}
      />
        </>
      )}
      {v.result && (
        <ResultDock tone={v.result.tone} title={v.result.title} detail={v.result.detail} logPath={v.result.logPath} />
      )}
    </Space>
  );
}

/** 回到上一段。这一面只拿得到字符串、不碰后台的形状（后台那份同名的在 `runner::browser::parent_of`，
 *  两处各判一次的是**显示**，写动作按的名字始终由后台那份交）。 */
function parentOf(name: string): string {
  const i = name.lastIndexOf('/');
  return i < 0 ? '' : name.slice(0, i);
}
