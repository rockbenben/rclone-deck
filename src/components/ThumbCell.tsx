import { Image, Space, Typography } from 'antd';
import { useEffect, useState } from 'react';

import { previewLocalImage, previewPaneImage } from '../api';

/** 只认这一族扩展名，与后端 `IMAGE_EXTS` 同一张表（两边不一致就是"点得动但读不出"）。 */
const IMAGE_EXTS = ['.png', '.jpg', '.jpeg', '.gif', '.webp', '.bmp', '.avif'];

const looksLikeImage = (name: string): boolean => {
  const lower = name.toLowerCase();
  return IMAGE_EXTS.some((e) => lower.endsWith(e));
};

/**
 * 一格缩略图。
 *
 * **两种来源，一个组件**：
 * - `jobId`（差异那一面）：点一下「看看」才取 —— 那一面一屏 20 行，点哪张取哪张。
 * - `fs`/`prefix`（浏览面板本地那一栏）：`auto` 为真时**直接取**。浏览是翻文件夹，
 *   翻的目的就是看，一列名字等于让人自己去资源管理器开一遍；而本地文件在磁盘上，
 *   读它不花钱（不下载、不问远端）。所以那一栏**直接摆图**，不等点击。
 *
 * 云端那一栏**不摆**：`fs` 是远端坐标，本机没有那个文件，看它得先下载一次 ——
 * 那是另一件还没做的事，这里不能摆一个点得动却拉不动的入口。
 */
export function ThumbCell({ jobId, name, fs, prefix = '', auto = false }:
  { jobId?: string; name: string; fs?: string; prefix?: string; auto?: boolean }) {
  const [url, setUrl] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  const [loading, setLoading] = useState(false);

  const fetchImage = async () => {
    setLoading(true);
    try {
      // 两条路都由后台定根 —— 这里只交坐标，不交路径。
      const data = jobId !== undefined
        ? await previewLocalImage(jobId, name)
        : await previewPaneImage(fs as string, prefix, name);
      setUrl(data);
      return data;
    } catch {
      // 拒绝的话术在后台（那才是有信息的那一版：为什么不给读）。
      // 屏上只说"看不了"，细节是操作员要看时才问的事。
      setFailed(true);
      return null;
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    // 换行（换 job / 翻页 / 换目录）就丢掉上一张：那是一份 base64 字符串，
    // 留着只是白占内存。
    setUrl(null);
    setFailed(false);
    setLoading(false);
    if (auto) { void fetchImage(); }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [jobId, name, fs, prefix, auto]);

  if (!looksLikeImage(name)) {
    // 不是图片就没有缩略图——**不摆一个占位方块**，那一格要短就短。
    return <Typography.Text className="fs-code">{name}</Typography.Text>;
  }

  if (failed) {
    // 读不出来时退回名字，而不是留一个破图：这一格的信息是名字，不是"这里有张图"。
    return (
      <Space size={4}>
        <Typography.Text type="secondary" className="fs-caption">看不了</Typography.Text>
        <Typography.Text className="fs-code">{name}</Typography.Text>
      </Space>
    );
  }

  if (url === null) {
    return (
      <Space size={4}>
        {auto ? (
          <Typography.Text type="secondary" className="fs-caption">读着…</Typography.Text>
        ) : (
          <Typography.Text
            className="fs-code" role="button" style={{ cursor: loading ? 'default' : 'pointer' }}
            onClick={() => { if (!loading) { void fetchImage(); } }}
          >
            {loading ? '读着…' : '看看'}
          </Typography.Text>
        )}
        <Typography.Text className="fs-code" type="secondary">{name}</Typography.Text>
      </Space>
    );
  }

  return (
    <Image
      src={url} alt={name} width={72} height={72} className="thumb-preview"
      // 圆角与裁切在 `styles.css` 的 `.thumb-preview` 里，不在这儿写 `style`：
      // 全仓的圆角都在那一个文件（组件里写圆角是 `frontend_theme.rs` 禁的那件事），
      // 而且 `ThumbCell` 已登记进那道闸的名单——它现在看得见这个文件了。
      // 点开看大图：缩略图那 72px 只是"是不是这一张"的答案，看清全貌才是目的。
      preview={{ mask: '看大图' }}
    />
  );
}