import { Alert, Badge, Button, Segmented, Space, Tooltip, Typography } from 'antd';
import type { ReactNode } from 'react';

import { NoteLine } from './NoteLine';
import { useTheme, type ThemeMode } from '../theme';

import type { RcdStatus, ShortNote } from '../types';

function LogDir({ status }: { status: RcdStatus }) {
  if (status.logDir === '') return null;
  return (
    <Typography.Text type="secondary" className="deck-log fs-note">
      日志目录{' '}
      <Typography.Text className="deck-log-path num" copyable={{ text: status.logDir }}>
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
    <Segmented className="deck-theme" aria-label="显示主题" value={chosen} onChange={(v) => setChosen(v as ThemeMode)}
      options={[{ label: '自动', value: 'auto' }, { label: '浅色', value: 'light' }, { label: '深色', value: 'dark' }]} />
  );
}

export function RcdStatusBar({ status, pollError, children }: {
  status: RcdStatus | null;
  pollError: ShortNote | null;
  children: ReactNode;
}) {
  const unavailable = status !== null && (!status.ok || !status.alive);

  return (
    <>
      <header className="deck-header">
        <div className="deck-toolbar">
          {children}
          <div className="deck-tools">
            {status === null && (
              <Badge status={pollError === null ? 'default' : 'error'}
                text={pollError === null ? '正在连接后台…' : '还没连上后台'} />
            )}
            {status !== null && !unavailable && (
              <div className="deck-health">
                <Badge status="success" text="后台就绪" />
                {status.activeCount > 0 && (
                  <Typography.Text type="secondary" className="fs-note">{status.activeCount} 件正在做</Typography.Text>
                )}
                <Tooltip trigger={['hover', 'focus']}
                  title={<span className="num">rclone {status.rcloneVersion} · 端口 {status.port} · 进程 {status.pid}</span>}>
                  <Button size="small">详情</Button>
                </Tooltip>
              </div>
            )}
            {status !== null && <LogDir status={status} />}
            <ThemeSwitch />
          </div>
        </div>
      </header>
      {status === null && pollError !== null && (
        <div className="deck-notice"><NoteLine note={pollError} /></div>
      )}
      {unavailable && (
        <div className="deck-notice">
          <Alert type="error" showIcon title={status.ok ? '后台已经断开' : '后台没能启动'}
            description={
              <Space orientation="vertical" size={2}>
                {status.startupError !== null && <NoteLine note={status.startupError} />}
                {status.configPath !== '' && (
                  <Typography.Text type="secondary" className="fs-note">
                    配置文件在 <Typography.Text className="num" title={status.configPath}>{status.configPath}</Typography.Text>，
                    {status.ok ? '重开面板会重新起一个。' : '改完重开面板就行。'}
                  </Typography.Text>
                )}
              </Space>
            }
          />
        </div>
      )}
      {status !== null && !unavailable && status.sizeOnly && (
        <div className="deck-notice">
          <Typography.Text type="secondary" className="fs-note">
            每一趟都只比大小：内容改了但大小没变的那些不会重传
          </Typography.Text>
        </div>
      )}
    </>
  );
}
