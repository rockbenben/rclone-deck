import { Alert, Input, Modal, Select, Space, Typography } from 'antd';
import { useEffect, useState } from 'react';

import * as api from '../api';
import { errNote } from '../errors';
import { NoteLine } from './NoteLine';
import type { RemoteRef, ShortNote } from '../types';

/** 一行的两份值：`pick` 是挑中的那个起点（只为让选择框有个位置），`value` 是框里那几个字。
 *  **交出去的是 `value`，一个字节都不加工**：候选给的是根（`r2:img`、`D:/vault/img-r2`），
 *  要在根下面再下一层由操作员自己在框里写。这里补斜杠、拼冒号，就是第二处解释同一个坐标系
 *  （浏览器那一面早写着"界面不许自己拼路径"，同一件事）。 */
interface Row { pick: string | null; value: string }

const EMPTY: Row = { pick: null, value: '' };

/**
 * 「换一对位置」那一层（spec §3，裁定 R93/R94）：两处位置交出去，立一张只活在这一趟的卡。
 *
 * 它不解释"哪一侧是全集"——那是卡上那两格的事，这里只管把两处位置交出去。
 * 它也**不自己判**那两个位置合不合规矩（同一处、认不出、那一格已经占着）：判据在后台，
 * 那句拒绝由后台说，这一层负责把它摆在人正对着的那两个框下面。
 */
export function PairDraftLayer(props: {
  open: boolean;
  onClose: () => void;
  /** 立成了：交回后台算的那把 `pairKey`（与 `JobView.pairKey` 同一个来源，界面不自己拼）。 */
  onCreated: (pairKey: string) => void;
}) {
  const [refs, setRefs] = useState<RemoteRef[]>([]);
  const [refsError, setRefsError] = useState<ShortNote | null>(null);
  const [left, setLeft] = useState<Row>(EMPTY);
  const [right, setRight] = useState<Row>(EMPTY);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<ShortNote | null>(null);

  // 每次开层都重新问一次候选、并清空上一回那两个值。留着旧值按下去，动的是没人刚看过的位置
  // （与对齐确认层"每次都重新列一遍"同一个理由）。
  useEffect(() => {
    if (!props.open) return;
    setLeft(EMPTY); setRight(EMPTY); setError(null);
    let alive = true;
    api.browserRemotes()
      .then((r) => { if (alive) { setRefs(r); setRefsError(null); } })
      .catch((e) => { if (alive) setRefsError(errNote(e)); });
    return () => { alive = false; };
  }, [props.open]);

  const submit = async () => {
    setBusy(true);
    setError(null);
    try {
      props.onCreated(await api.pairDraft(left.value, right.value));
    } catch (e) {
      // 拒绝留在这一层里说，不只飘一条 toast 就完（spec §3 最后那行）：toast 自己会消失，
      // 而人此刻正对着这两个框决定要改成什么。
      setError(errNote(e));
    } finally {
      setBusy(false);
    }
  };

  // 一行的两份控件。分组名说的是"这一行是什么"（与本地相对的那个坐标），所以用**远端**不用云端
  // （`DESIGN.md` 那条词表：说"东西在哪一侧"才用云端）。名字与值都由后台交，这里不重排、不补冒号。
  const row = (label: string, value: Row, set: (r: Row) => void) => (
    <Space orientation="vertical" size={4} style={{ width: '100%' }}>
      <Typography.Text type="secondary" className="fs-caption">{label}</Typography.Text>
      {/* `wrap={false}`：名字与那两个框在 560 宽里放不下时会各掉一行，读的人得自己找回配对的框
          （卡上那两格量到过同一件事，同一层里不留第二种排版）。 */}
      <Space size={4} wrap={false} style={{ width: '100%' }}>
        <Select popupMatchSelectWidth={false} size="small" value={value.pick ?? undefined} placeholder="挑个起点" showSearch
          aria-label={`${label}的起点`} style={{ minWidth: 168 }}
          options={[
            { label: '远端', options: refs.filter((r) => !r.isLocal).map((r) => ({ value: r.id, label: r.label })) },
            { label: '本地', options: refs.filter((r) => r.isLocal).map((r) => ({ value: r.id, label: r.label })) },
          ]}
          onChange={(id) => set({ pick: id, value: id })} />
        <Input size="small" value={value.value} aria-label={label} className="fs-code"
          placeholder="要动的那一处，可直接改写" style={{ flex: 1 }}
          onChange={(e) => set({ pick: value.pick, value: e.target.value })} />
      </Space>
    </Space>
  );

  return (
    <Modal open={props.open} width={560} title="换一对位置" onCancel={props.onClose}
      onOk={() => void submit()} okText="就这么定" cancelText="取消"
      okButtonProps={{ loading: busy }}>
      <Space orientation="vertical" size={8} style={{ width: '100%' }}>
        <Typography.Text type="secondary" className="fs-note">
          两处位置各写一个：先挑一个起点，再在框里改成要动的那一处。
        </Typography.Text>
        {row('这一侧', left, setLeft)}
        {row('那一侧', right, setRight)}
        {refsError && (
          <Alert type="error" showIcon title="取那些位置失败" description={<NoteLine note={refsError} />} />
        )}
        {error && (
          <Alert type="error" showIcon title="没改成" description={<NoteLine note={error} />} />
        )}
        {/* 这句必须在这里说：这一张卡不写得进那两份文件，而"关掉就没了"是它与设计语言里
            那几张配好的卡唯一的区别（R96/R97：格上那句「这张卡上有没写进配置的格子」）。 */}
        <Typography.Text type="secondary" className="fs-note">
          这张卡只活在这一趟面板里：关掉就没了，也没有写进那两份文件。
          要常用它，立起来之后按卡上那颗「存成常用（写进配置）」—— 它会把这张卡带进
          设置里的「新加一条」，在那里确认一次才真的写进配置。
        </Typography.Text>
      </Space>
    </Modal>
  );
}
