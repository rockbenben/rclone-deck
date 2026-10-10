import { Alert, Badge, Button, Segmented, Space, Tooltip, Typography } from 'antd';
import { useState } from 'react';

import { NoteLine } from './NoteLine';
import { useTheme, type ThemeMode } from '../theme';

import type { RcdStatus, ShortNote } from '../types';

/**
 * 顶栏回答四件事：**后台活着吗**、**有几件正在做**、**日志在哪儿**、
 * **这一台每一趟按什么算"变了"**。四个读数全部来自后台那一份（`activeCount` / `logDir` /
 * `sizeOnly`），界面一个都不自己数、不自己猜（`DESIGN.md` 的"顶栏那一条常驻读数"那条）。
 *
 * 版本/端口/进程是排障时才要的，收进「详情」。日志目录留在明面，而且不许被收起来 ——
 * 它是"面板跑在哪个家"的眼前证据，那两次误启动就是靠这一格当场发现的（操作规程 R68），
 * 收进 Tooltip 等于把那道规程拆掉。`frontend_theme.rs` 里那条证人就是钉这个的。
 *
 * 起不来那一档**照样**要把落点说出来：那一刻正是最需要知道"跑在哪个家"的时候，
 * 而旧写法在这一档把日志目录整项藏了（后台填空串、界面按"没有"处理）。
 */

/** 日志目录那一格。**尾部**省略（一整条串走 CSS 的 ellipsis，不是 `PairPath` 那种三段中截），
 *  全值在原生 `title` 上、复制入口照旧。
 *  为什么省略：这一串是全屏最长的东西，横着铺满会把「详情」挤到第二行 ——
 *  换行的结果就是"日志在哪儿"这条规程证据自己变得难找。
 *  为什么用原生 `title` 而不是 antd 的 `Tooltip`：那条证人钉的正是"别把它收进浮层"。 */
function LogDir({ status }: { status: RcdStatus }) {
  if (status.logDir === '') return null;
  return (
    <Typography.Text type="secondary" className="fs-note">
      日志目录{' '}
      <Typography.Text className="num" copyable={{ text: status.logDir }}>
        <span className="log-ellipsis" title={status.logDir}>{status.logDir}</span>
      </Typography.Text>
    </Typography.Text>
  );
}

/** 主题那颗切换口。点亮的是**选择**（自动/浅色/深色），颜色由生效值画 —— 分开是刻意的，见 `theme.ts`。
 *  三档都要在这里，少一档就等于替用户决定"跟随系统"这件事没有出口。 */
function ThemeSwitch() {
  const [, chosen, setChosen] = useTheme();
  return (
    <Segmented size="small" value={chosen} onChange={(v) => setChosen(v as ThemeMode)}
      options={[{ label: '自动', value: 'auto' }, { label: '浅色', value: 'light' }, { label: '深色', value: 'dark' }]} />
  );
}

export function RcdStatusBar({ status, pollError }: { status: RcdStatus | null; pollError: ShortNote | null }) {
  const [detail, setDetail] = useState(false);

  // 两档必须分开：**从没问到过**才是"正在连接"；问过而失败要说"还没连上"。
  // 旧写法只有前一档，于是请求一直失败时屏幕永远写着"正在连接后台…"，
  // 旁边还飘着一条说它失败的通知 —— 同一屏上两句互相打脸。
  //
  // 这一档是界面**唯一**自己下的判断：后台压根没被叫到，交不出任何东西。
  // 它不是"界面替后台推断状态"，而是"只有界面知道这件事"——`pollError` 由 Panel 交进来，
  // 句子仍是后台那一份（`errNote`），界面只决定摆不摆。
  if (!status) {
    return (
      <div className="surface bar">
        <Badge status="default" text={pollError === null ? '正在连接后台…' : '还没连上后台'} />
        {pollError !== null && <NoteLine note={pollError} />}
        <span className="bar-right"><ThemeSwitch /></span>
      </div>
    );
  }

  if (!status.ok) {
    return (
      <div className="surface bar">
        <Alert
          type="error"
          showIcon
          title="后台没能启动"
          description={
            <Space orientation="vertical" size={2}>
              {/* 那一句由后台分成两段交上来（`RcdStatus.startupError`）：顶栏是常驻面，
                  原文摆上去就等于界面上常驻一个内部名词。空的那一档也是后台给的句子。 */}
              {status.startupError !== null && <NoteLine note={status.startupError} />}
              <Typography.Text type="secondary" className="fs-note">
                <LogDir status={status} />
                {status.configPath !== '' && (
                  <>
                    {' '}配置文件在 <Typography.Text className="num" title={status.configPath}>{status.configPath}</Typography.Text>，
                    改完重开面板就行。
                  </>
                )}
              </Typography.Text>
            </Space>
          }
        />
        {/* 起不来那一档也要能换主题：眼睛不舒服的时候正好是这一屏，而这条规程与"后台活着吗"无关
            （同一族判据：那一格日志目录在这里也不藏）。 */}
        <span className="bar-right"><ThemeSwitch /></span>
      </div>
    );
  }

  return (
    <div className="surface bar">
      <Badge status="success" text="后台就绪" />
      {/* 后台活着 ≠ 现在能动。这个数是后台数好的（`activeCount`，去重后的持有者数）：
          界面数 `locks` 的键就是第二份判定，而一次动作可以申领多个键，数键会把一件事报成两件。 */}
      {status.activeCount > 0 && (
        <Typography.Text type="secondary" className="fs-note">{status.activeCount} 件正在做</Typography.Text>
      )}
      {/* 这一台面板每一趟**按什么算"变了"**（后台那枚布尔，判定在 `compares_by_size`）。
          只在开着那一档时出现：常驻一句没在跑的比法比不说更坏，所以判据是"有没有开"，与日志目录
          那一格同一族。为什么要在明面：只比大小会漏掉"内容改了但大小没变"的那一次改动，
          而漏掉的这件事只有当场说得出口，事后翻日志才追得回来。 */}
      {status.sizeOnly && (
        <Typography.Text type="secondary" className="fs-note">
          每一趟都只比大小：内容改了但大小没变的那些不会重传
        </Typography.Text>
      )}
      <Tooltip
        open={detail}
        onOpenChange={setDetail}
        title={<span className="num">rclone {status.rcloneVersion} · 端口 {status.port} · 进程 {status.pid}</span>}
      >
        {/* 不带 type="text"：它浮在同样无边框的文字之间时看着像个标签，没人知道能点。 */}
        <Button size="small">详情</Button>
      </Tooltip>
      {/* 主题那颗与日志目录一起归右边。契约写的是"顶栏**右侧**那颗主题切换口"，
          旧写法把它摆在「详情」后面（chrome 的中间），于是"我在哪一面"那一层里
          混进了两颗与这一面无关的控件。挪过去之后左边只说后台的事，右边只放可改的东西。 */}
      <span className="bar-right">
        <ThemeSwitch />
        <LogDir status={status} />
      </span>
    </div>
  );
}
