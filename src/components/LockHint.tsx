import { Tooltip } from 'antd';
import type { CSSProperties, ReactNode } from 'react';

/**
 * 灰控件自己说清楚「为什么点不动」（裁定 R54 第二条）。`title` 为空时原样返回，不加任何包装。
 * `title` 收 ReactNode：那句「为什么」在后台是两段（摘要 + 细节），Tooltip 这一面就是它的展开处。
 *
 * 外层这层 `span` 不是装饰：Chromium 不为 disabled 表单控件派发 mouseover/mouseenter，而这一版
 * @rc-component/trigger 没有 pointer-events 补丁——Tooltip 直接挂在 disabled 的 Button 上，
 * 真鼠标悬停永远弹不出来（用 CDP 可信 mouseMoved 量过：可见提示数 0，只有 JS 合成事件才弹）。
 * 配合下面 `HIT_OFF` 把按钮的命中关掉，事件落到 span 上，antd 标准 `Tooltip` 才真的工作。
 * 按钮本来就被禁用，关命中不损失任何操作。不换自绘浮层：本项目只允许 antd 标准件。
 */
export function LockHint({ title, children }: { title?: ReactNode | null; children: ReactNode }) {
  if (!title) return <>{children}</>;
  return (
    <Tooltip title={title}>
      <span style={{ display: 'inline-block', cursor: 'not-allowed' }}>{children}</span>
    </Tooltip>
  );
}

/** 锁定时挂在按钮上：让命中测试落到 LockHint 的 span，Tooltip 才收得到真悬停。 */
export const HIT_OFF: CSSProperties = { pointerEvents: 'none' };
