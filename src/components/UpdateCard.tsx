import { Alert, Button, Card, Progress, Space, Switch, Typography } from 'antd';
import { useEffect } from 'react';

import { formatBytes } from '../format';
import {
  checkForUpdate, downloadAndInstall, loadCurrentVersion, setAuto, useUpdates,
} from '../updates';
import { NoteLine } from './NoteLine';

/**
 * 设置面里的「版本与更新」那一张卡。
 *
 * 这一面是**面板唯一一处会联网**的地方，而它**默认关着**（`src/updates.ts` 里那一档）：
 * 关着的时候这一张卡只报当前版本与那两颗按钮，屏上一个包都不下。
 *
 * 两条规矩在这一张卡上是看得见的：
 *
 * 1. **问与下是分开的两颗按钮。**「立即检查」只问一句（有没有、有哪个版本），
 *    下载与安装是「下载并安装」那一颗的事 —— 人没按就不下、不装。
 * 2. **问了之后要不要说话，看是谁问的。**人按的那一次要么说"已经是最新"、要么把
 *    出了什么事说出来；面板自己问的那一次不出声（没人问，就不该吵）。
 */
export function UpdateCard() {
  const s = useUpdates();

  // 面板现在跑着的是哪个版本：问应用自己，不联网。
  useEffect(() => { void loadCurrentVersion(); }, []);

  // 百分比只在问得到总量时才有；问不到就只报已经下了多少（`formatBytes` 给的是人读的写法）。
  const percent = s.total !== null && s.total > 0
    ? Math.min(100, Math.round((s.downloaded / s.total) * 100))
    : 0;
  const size = s.total !== null
    ? `${formatBytes(s.downloaded)} / ${formatBytes(s.total)}`
    : formatBytes(s.downloaded);

  return (
    <Card title="版本与更新" className="surface">
      <Space orientation="vertical" size="middle" style={{ width: '100%' }}>
        <Space size={6} wrap>
          <Typography.Text type="secondary" className="fs-caption">当前版本</Typography.Text>
          <Typography.Text className="fs-code">{s.current}</Typography.Text>
        </Space>

        <Space size={8} wrap>
          <Switch checked={s.auto} onChange={setAuto} disabled={s.checking || s.downloading} />
          <Typography.Text>每次打开时检查新版本</Typography.Text>
        </Space>
        {/* 这一句是这一张卡存在的理由：面板平时不联网，开了这一档才会去问一句。 */}
        <Typography.Text type="secondary" className="fs-caption">
          面板平时不联网；只有这一项会去问发布页有没有新版，关着就一次也不问。
        </Typography.Text>

        <div>
          <Button onClick={() => void checkForUpdate(true)} loading={s.checking} disabled={s.downloading}>
            立即检查
          </Button>
        </div>

        {s.downloading && (
          <Progress percent={percent} status="active"
            format={() => <Typography.Text className="fs-code">{size}</Typography.Text>} />
        )}

        {/* 有新版时只说"有"与版本号：装不装是下面那颗按钮的事。
            下着的时候这一块**不撤**（撤了又来一次，读起来像"刚才那一下白下了"），
            只是那颗按钮灰着——下与装都还没完。 */}
        {s.version !== '' && (
          <Alert type="info" showIcon title="有新版了"
            description={
              <Space orientation="vertical" size={4} style={{ width: '100%' }}>
                <Space size={6} wrap>
                  <Typography.Text>版本</Typography.Text>
                  <Typography.Text className="fs-code">{s.version}</Typography.Text>
                </Space>
                <Button type="primary" disabled={s.downloading} onClick={() => void downloadAndInstall()}>
                  下载并安装
                </Button>
              </Space>
            } />
        )}

        {s.note !== null && (
          <Alert type="info" showIcon title={s.note.short}
            description={s.note.detail === '' ? undefined : s.note.detail} />
        )}

        {s.error !== null && (
          <Alert type="error" showIcon title={s.error.title}
            description={<NoteLine note={s.error.note} />} />
        )}
      </Space>
    </Card>
  );
}
