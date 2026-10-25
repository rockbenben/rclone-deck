import { Alert, Button, Divider, Select, Space, Table, Typography } from 'antd';
import { useEffect, useState } from 'react';
import type { ReactNode } from 'react';

import { Crumbs } from './Crumbs';
import { NoteLine, noteTip } from './NoteLine';
import { HIT_OFF, LockHint } from './LockHint';
import { ResultDock } from './ResultDock';
import type { DockTone } from './ResultDock';
import { ThumbCell, looksLikeImage } from './ThumbCell';
import { plainNote } from '../errors';
import { baseName, formatBytes } from '../format';
import type { BrowserEntry, BrowserListing, ShortNote } from '../types';

const TABLE_PAGE = 100;

/** 网格一次摆多少格。200 是个折中：一屏满宽大约看 40–60 格，
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
 * 浏览器的一栏。DESIGN.md 的 Layout 那条把它钉成固定几层，自上而下：
 * 栏标题 + 面包屑 → 位置读数 → 那句提示 → 动作条（导航｜组织｜传输｜删除）→ 图片网格 → 表格 → 结果条。
 */
export interface PaneProps extends PaneView, PaneActions {
  /** 对面那一栏没站位置 —— 本栏占满整宽，图片按网格摆。
   *  两栏都在时（并排比）它还是表格：那时要的是左右对同一行，一行一个名字比网格好读。 */
  wide?: boolean;
  /** **只翻图，不动东西**：那一族动作收进一个折叠。
   *
   *  这一栏开着多数时候是为了翻那堆图片，而不是搬它。判据由**外面**给（进没进比对），
   *  不在这一层自己算 —— "翻图"与"没有可操作对象"两种情形会长出两套不同的收法，那样更怪。
   */
  compact?: boolean;
}

/** 动作的族：族与族之间画一条竖线（五颗平铺时"清空这一层"和"上一级"看着是同一级的事）。 */
type Group = 'organize' | 'move' | 'delete';

/** 一颗动作。折叠内外**共用这一份定义**：两处各写一遍就会长出两种说法与两种危险档。 */
interface Act {
  key: string;
  label: string;
  group: Group;
  /** 会拿走东西的那一族按契约走 `button-danger`（DESIGN 的 `button-primary` 那条）。 */
  danger?: boolean;
  off: boolean;
  /** 点不动的那句为什么；null = 这一颗此刻没有要说原因的（灰与原因各读各的字段）。 */
  why: ShortNote | null;
  run: () => void;
}

export function BrowserPane(v: PaneProps) {
  const { listing, hints, clearNote } = v;
  const a: PaneActions = v;
  const entries = listing?.entries ?? [];
  // 网格只在**满宽且本地**那一档摆。半宽时一屏只放得下三四张、看下去要横着拖好几回，
  // 而并排比要的是"左右对同一行"；云端那一栏根本没有可摆的图（rc 上没有只读前若干字节的
  // 出口，理由写在 `ThumbCell` 与 `runner::browser` 上），摆出来就是一屏「看不了」。
  const grid = v.wide === true && !v.isRemote
    ? entries.filter((e) => !e.isDir && looksLikeImage(e.name)) : [];
  const rest = grid.length > 0 ? entries.filter((e) => !grid.includes(e)) : entries;
  // 图片行被网格接走之后，表格里就没有它们那一行了 —— 而"改这一张 / 删这一张"是
  // 每一类对象都该有的一种处理。所以那一格点中之后的动作挂在动作条上（点中=要动的那个）。
  const picked = v.pickedName === null ? null : entries.find((e) => e.name === v.pickedName) ?? null;

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

  // 「上一件事还没跑完」这一句是界面自己的判据（忙碌态在界面手里，不在后台），
  // 所以句子也在界面手里 —— 与 `plainNote` 那一族同一口径，不是备用文案。
  const busyNote = plainNote('上一件事还没跑完');
  const acts: Act[] = [
    { key: 'mkdir', label: '新建目录', group: 'organize', off: v.busy, why: null, run: a.onMkdir },
    ...(v.isRemote ? [{
      key: 'push', label: '上传到这里', group: 'move' as Group,
      off: v.busy || hints.push.short !== '', why: hints.push, run: a.onPush,
    }] : []),
    {
      key: 'move', label: '移到这里', group: 'move',
      off: v.busy || hints.move.short !== '', why: hints.move, run: a.onMoveHere,
    },
    ...(picked === null || picked.isDir ? [] : [
      {
        key: 'rename', label: '改名这一个', group: 'move' as Group,
        off: v.busy, why: null, run: () => a.onRename(picked),
      },
      {
        key: 'delone', label: '删这一个', group: 'delete' as Group, danger: true,
        off: v.busy, why: null, run: () => a.onDeleteOne(picked),
      },
    ]),
    {
      // N 读后台按这一份清单算好的那个数，不在这里数行。还没列出来时不许摆一个数字占位符
      // 给人看（那一档那颗本来就是灰的，说一句人话就行）。「这一层」是范围，不是修辞：
      // 这颗只动直接子项，文件夹里面的东西归「清空这一层」。
      key: 'delvis', group: 'delete', danger: true,
      label: listing === null ? '等这一层列出来才能删' : `删掉这一层看到的 ${listing.totalCount} 个`,
      off: v.busy || hints.visible.short !== '', why: hints.visible, run: a.onDeleteVisible,
    },
    {
      // 与左边那颗的区别必须写在按钮上：这颗连文件夹里面的一起清掉（`purge` 删的是整棵前缀）。
      // 两颗都写"这一层"的话，范围差一整棵子树，而界面上看不出来。
      // 灰由后台那个布尔决定，Tooltip 由后台那两句决定，两件事各有各的来源；
      // "不能清空却没给句子"是后台的 bug（`clear_gates` 的证人钉着它），这里不设备用文案。
      key: 'clear', label: '清空这一层（连里面的一起）', group: 'delete', danger: true,
      off: v.busy || v.clearBlocked, why: clearNote, run: a.onPurge,
    },
  ];

  const renderAct = (r: Act) => {
    // 忙碌与"规则不让"两档都要有句子：读得到原因的那一颗才灰得有道理。
    const why = r.why !== null && r.why.short !== '' ? r.why : (v.busy ? busyNote : null);
    const tip = why === null ? null : noteTip(why);
    const btn = (
      <Button size="small" danger={r.danger} style={tip !== null ? HIT_OFF : undefined}
        disabled={r.off} onClick={r.run}>
        {r.label}
      </Button>
    );
    return tip === null
      ? <span key={r.key}>{btn}</span>
      : <LockHint key={r.key} title={tip}>{btn}</LockHint>;
  };
  const withDividers = (list: Act[]) => {
    const out: ReactNode[] = [];
    let prev: Group | null = null;
    for (const r of list) {
      if (prev !== null && r.group !== prev) out.push(<Divider key={`d-${r.key}`} vertical />);
      prev = r.group;
      out.push(renderAct(r));
    }
    return out;
  };

  return (
    <Space orientation="vertical" size={8} style={{ width: '100%' }}>
      <div>
        <Typography.Text strong>{v.title}</Typography.Text>
        <Crumbs root={v.rootLabel} prefix={v.prefix} onJump={a.onJump} />
      </div>
      <Space wrap size={4}>
        {/* 没有占位句：框下面那一句「选一个位置，这一栏会列出那里有什么。」已经说了同一件事，
            而框本身空着就是"没选"的样子（DESIGN 文案第 1 条：同一屏幕同一件事只说一遍）。 */}
        <Select popupMatchSelectWidth={false} style={{ minWidth: 170 }} value={v.fs ?? undefined}
          onChange={(next: string) => a.onSelectPosition(next)}
          options={v.choices.map((r) => ({ label: r.label, value: r.value }))} />
        {listing && <PanePath path={listing.path} />}
        {listing && (
          <Typography.Text type="secondary">
            {/* 三个数都是**这一层**的（后台只列一层，见 `browser_list`）：文件夹里还有多少东西不在这里说，
                进去才看得见。少了"这一层"三个字，这个字节数就会被读成"这个前缀往下一共多大"。
                等宽只给数字那一小块（`num`），整句走中文那一族 —— 把整行套进等宽族会连中文一起拖进去。 */}
            这一层 <span className="num">{listing.fileCount}</span> 个文件 ·
            {' '}<span className="num">{listing.dirCount}</span> 个文件夹 ·
            {' '}<span className="num">{formatBytes(listing.totalBytes)}</span>
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
        <Typography.Text type="secondary" className="fs-note">
          {v.choices.length === 0
            ? `这一栏还没有可选的${v.title}。`
            : '选一个位置，这一栏会列出那里有什么。'}
        </Typography.Text>
      ) : (
        <>
          <Space size={4} wrap>
            {/* 「上一级」在根上**不摆**：那里没有可去的地方，摆一颗灰着的、又要人为它编一句
                "已经在最上面"，不如不摆（与 `Crumbs` 空根时整行不出现同一形状）。 */}
            {v.prefix !== '' && (
              <Button size="small" disabled={v.busy} onClick={() => a.onJump(parentOf(v.prefix))}>
                上一级
              </Button>
            )}
            {!v.compact && withDividers(acts)}
          </Space>
          {/* 收起来的那一族给一颗粒在行上：折叠起来不等于找不着。 */}
          {v.compact && !showMove && (
            <Typography.Link className="fs-caption" onClick={() => setShowMove(true)}>
              要新建、传输或删除这一层里的东西？
            </Typography.Link>
          )}
          {v.compact && showMove && (
            <Space size={4} wrap>
              {withDividers(acts)}
              <Button size="small" type="text" onClick={() => setShowMove(false)}>收起</Button>
            </Space>
          )}
          {/* 图片网格（只有满宽且本地那一栏有）。`auto` 让它滚到眼前就取图，不用点。
              每格点一下仍然**选中那一个文件**（跨栏动作的源），与表格里点行是同一个动作 ——
              网格只是换了摆法，不是另一个入口；"要动那一个"的动作就在动作条上。 */}
          {grid.length > 0 && (
            <>
              <div className="thumb-grid">
                {shown.map((e) => (
                  <button key={e.name} type="button"
                    className={v.pickedName === e.name ? 'thumb-cell picked' : 'thumb-cell'}
                    title={e.name}
                    onClick={() => a.onPick(v.pickedName === e.name ? null : e.name)}>
                    <ThumbCell fs={v.fs ?? undefined} prefix={v.prefix} name={e.name} auto box={112} />
                    {/* 选中那一格要说得出"已选中"这句字，不能只靠边框与淡底（颜色不是唯一线索）。 */}
                    <span className="thumb-name fs-code">
                      {v.pickedName === e.name ? `已选中 · ${baseName(e.name)}` : baseName(e.name)}
                    </span>
                  </button>
                ))}
              </div>
              {/* 分批：上面那层 `IntersectionObserver` 治的是"**图**什么时候取"，这一层治的是
                  "**格子**什么时候存在"。两千张图一次铺 2000 个 button，DOM 本身就够卡一阵，
                  而其中一千九百多格在屏外、永远不会被看见。所以先摆 `GRID_BATCH` 格，
                  底下给一颗继续摆的按钮 —— 不做虚拟滚动是因为它要接一个新依赖、且滚动条长度会跳；
                  这一档没有滚动条（整页滚），加一颗按钮就够了，而翻到第 200 张时人早该换目录了。 */}
              {grid.length > shown.length && (
                <div style={{ textAlign: 'center', marginTop: 8 }}>
                  <Button size="small" onClick={() => setBatch((n) => n + GRID_BATCH)}>
                    再看 {Math.min(GRID_BATCH, grid.length - shown.length)} 张（还剩 {grid.length - shown.length} 张）
                  </Button>
                </div>
              )}
            </>
          )}
          {/* 网格把图片都收走了，下面这张表只剩目录与非图片。若它一个都没有，
              不要再摆一张「暂无数据」——同一屏上「这一层 3 个文件」与它互相打脸。
              真正空的目录（连网格都是空的）仍由这张表说「暂无数据」。 */}
          {(grid.length === 0 || rest.length > 0) && (
          <Table size="small" rowKey={(row) => row.name} dataSource={rest}
            loading={v.busy && listing === null}
            pagination={{ pageSize: TABLE_PAGE, hideOnSinglePage: true }}
            scroll={{ x: 'max-content' }}
            onRow={(row) => ({ onClick: () => a.onPick(row.isDir ? null : row.name) })}
            columns={[
              {
                title: '名字', dataIndex: 'name', key: 'name',
                render: (name: string, row) => (
                  <Space size={4}>
                    {/* 图与名字**同格、各写各的**：`ThumbCell` 只画那一格图（或那句状态），
                        名字由这一行写。以前是"图加载完了就把名字换成图"，于是本地这一栏
                        加载完的图行读不出是哪个文件 —— 而这一面头上那句明写着
                        「要动哪个名字就点哪个」，名字不在行上就点不出那个名字。
                        没有网格的形状（两栏并排、云端那一栏）也在同一格里直接摆图；
                        有网格时图行整批住在网格里，这一栏只剩目录与非图片，两边不重复画同一张图。 */}
                    {!v.isRemote && !row.isDir && (
                      <ThumbCell fs={v.fs ?? undefined} prefix={v.prefix} name={name} auto />
                    )}
                    <Typography.Text code className="fs-code">{name}</Typography.Text>
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
                    <Button size="small" danger disabled={v.busy || v.fs === null} onClick={() => a.onRmdir(row)}>
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
                    <Button size="small" danger disabled={v.busy || v.fs === null} onClick={() => a.onDeleteOne(row)}>
                      删这一个
                    </Button>
                  </Space>
                )),
              },
            ]}
          />
          )}
        </>
      )}
      {v.result && (
        <ResultDock tone={v.result.tone} title={v.result.title} detail={v.result.detail} logPath={v.result.logPath} />
      )}
    </Space>
  );
}

/** 位置读数：尾巴那个目录名要钉住（认家靠的就是它），所以三段 —— 头可缩可省略、尾不缩，
 *  全文挂**原生** `title`（不许收进浮层，规程 R68，与 `PairPath` 同一形状）。
 *  以前这一格走 antd 的 `ellipsis`（尾部省略）+ 浮层 Tooltip，于是消失的恰恰是尾巴。 */
function PanePath({ path }: { path: string }) {
  const i = path.lastIndexOf('/');
  const head = i < 0 ? path : path.slice(0, i + 1);
  const tail = i < 0 ? '' : path.slice(i + 1);
  return (
    <span className="fs-code path-line" style={{ maxWidth: 220 }}>
      <span className="path-head" title={path}>{head}</span>
      <span className="path-tail" title={path}>{tail}</span>
    </span>
  );
}

/** 回到上一段。这一面只拿得到字符串、不碰后台的形状（后台那份同名的在 `runner::browser::parent_of`，
 *  两处各判一次的是**显示**，写动作按的名字始终由后台那份交）。 */
function parentOf(name: string): string {
  const i = name.lastIndexOf('/');
  return i < 0 ? '' : name.slice(0, i);
}
