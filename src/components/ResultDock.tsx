import { Alert, Typography } from 'antd';
import type { ReactNode } from 'react';

/** 一次动作的结果落在哪一档。口径表那三档：`做完了` / `没改成` / `还没清单`。
 *  设置面与浏览器两面都按第一档写「做完了 · <动作>：<位置>」；后台报的数与清单对不上那一档
 *  不走这个前缀（那是"做完了但没核对上"，由正文那句去说）。不许再长第四种说法。 */
export type DockTone = 'ok' | 'refused' | 'pending';

/** 完整路径在复制里给全，屏上只留最后两段 —— 一栏最小 380px 塞不下一条 Windows 长路径，
 *  而这一格是"结果"，不该被一条路径撑成三行。 */
function tailOf(p: string): string {
  const parts = p.split(/[\\/]/);
  return parts.length <= 2 ? p : parts.slice(-2).join('/');
}

/**
 * 一次动作的结果，钉在**发起它的那一栏脚下**。
 *
 * 上一版它是页面级的一条 Alert，掉在视口外：验收时我只抄了标题没抄正文，
 * 于是"`confirmed == 0` 却说两个数一致"那一格真缺陷混过了两轮评审。
 * 所以这一格从此钉在栏脚（`position: sticky` + `bottom: 0`），并且有证人守着
 * （`frontend_browser.rs` 的 `the_result_of_a_pane_action_lands_at_the_foot_of_that_pane`）。
 *
 * 这里不拼任何判定：`tone`、`title`、`detail` 全是外面交进来的，
 * `detail` 里那些分支（有没有那份清单）读的也是后台作为数据交出来的布尔。
 */
export function ResultDock(props: {
  tone: DockTone;
  title: string;
  detail: ReactNode;
  logPath: string;
}) {
  const { tone, title, detail, logPath } = props;
  if (title === '') return null;
  return (
    <div style={{ position: 'sticky', bottom: 0, paddingTop: 8, background: 'var(--surface)' }}>
      <Alert
        showIcon
        type={tone === 'ok' ? 'success' : tone === 'refused' ? 'error' : 'warning'}
        title={title}
        description={
          <>
            {detail}
            {logPath !== '' && (
              <div>
                <Typography.Text type="secondary" className="fs-note">
                  日志{' '}
                  <Typography.Text className="num" copyable={{ text: logPath }}>{tailOf(logPath)}</Typography.Text>
                </Typography.Text>
              </div>
            )}
          </>
        }
      />
    </div>
  );
}
