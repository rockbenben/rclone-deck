import { Space, Typography } from 'antd';
import { useState } from 'react';
import type { ReactNode } from 'react';

import type { ShortNote } from '../types';

/**
 * 一句人话 + 一个「为什么？」。摘要与细节两段都由后台拼好（spec §4.1-1），
 * 这里只决定先露哪一段 —— 不许在这里拆句，也不许从别的字段反推第二句（`I5`）。
 *
 * 为什么不是 Tooltip：这一句是**常驻**的解释（栏上那条），悬停才看得见等于没有。
 */
export function NoteLine(props: { note: ShortNote; tone?: 'warning' | 'secondary' }) {
  const [open, setOpen] = useState(false);
  const { note } = props;
  if (note.short === '') return null;
  return (
    <div>
      <Typography.Text type={props.tone ?? 'secondary'} className="fs-note">{note.short}</Typography.Text>
      {note.detail !== '' && (
        <>
          <Typography.Text type="secondary" className="fs-caption">
            <a onClick={() => setOpen(!open)}> 为什么？</a>
          </Typography.Text>
          {open && <div><Typography.Text type="secondary" className="fs-note">{note.detail}</Typography.Text></div>}
        </>
      )}
    </div>
  );
}

/** Tooltip 那一面就是那两段的展开处：摘要一行、细节一行。
 *  两段都空 ⇒ 交回 null，`LockHint` 就不包那层壳（能清空时不该有一句"为什么点不动"挂着）。 */
export function noteTip(n: ShortNote): ReactNode {
  if (n.short === '') return null;
  return (
    <Space orientation="vertical" size={2}>
      <Typography.Text className="fs-note">{n.short}</Typography.Text>
      {n.detail !== '' && <Typography.Text className="fs-note">{n.detail}</Typography.Text>}
    </Space>
  );
}
