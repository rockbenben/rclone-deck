import { Image, Typography } from 'antd';
import { useEffect, useRef, useState } from 'react';

import { previewLocalImage, previewPaneImage } from '../api';
import { errNote } from '../errors';
import type { ShortNote } from '../types';

/** 只认这一族扩展名，与后端 `IMAGE_EXTS` 同一张表（两边不一致就是"点得动但读不出"）。 */
const IMAGE_EXTS = ['.png', '.jpg', '.jpeg', '.gif', '.webp', '.bmp', '.avif'];

/** 只认这一族扩展名，与后端 `IMAGE_EXTS` 同一张表（两边不一致就是"点得动但读不出"）。
 *  `export`：浏览面板把图片与非图片分开摆时要问同一句 —— 两处各写一张表，
 *  漂开的那一次是"有的进网格、有的进列表"，而人只会以为这是本来就那样。 */
export const looksLikeImage = (name: string): boolean => {
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
 * 云端那一栏**不摆**：那一侧的对象在本机没有，而 rc 接口上没有"只读前若干字节"的出口
 * （实测那五个方法都没注册），要画这一格就得整张下载。所以这里不摆一个点得动却拉不动的入口。
 */
export function ThumbCell({ jobId, name, fs, prefix = '', auto = false, box = 72 }:
  { jobId?: string; name: string; fs?: string; prefix?: string; auto?: boolean; box?: number }) {
  const [url, setUrl] = useState<string | null>(null);
  // 「看不了」要说得出为什么，而那一句只有后台有（超上限 / 形状画不出来 / 读不出来）。
  // 界面不自己分类，也不设备用文案：把后台那两段原样留着，格子上给短的那句。
  const [why, setWhy] = useState<ShortNote | null>(null);
  const [loading, setLoading] = useState(false);
  const [seen, setSeen] = useState(false);
  const ref = useRef<HTMLSpanElement | null>(null);

  const fetchImage = async () => {
    setLoading(true);
    try {
      // 两条路都由后台定根 —— 这里只交坐标，不交路径。
      const data = jobId !== undefined
        ? await previewLocalImage(jobId, name)
        : await previewPaneImage(fs as string, prefix, name);
      setUrl(data);
      return data;
    } catch (e) {
      setWhy(errNote(e));
      return null;
    } finally {
      setLoading(false);
    }
  };

  /**
   * **滚到眼前才取**（2026-10-11，这是"分批加载"的那一半）。
   *
   * 原来 `auto` 是挂载就取，于是打开一个有两千张图的目录会同时发两千发
   * `preview_pane_image`，每发一份 base64 都进内存（上限 8 MB/张）—— 页面先卡死，
   * 然后内存爆掉。`IntersectionObserver` 让**看得见的那几十格**去取，看不见的一格不发。
   *
   * `rootMargin` 提前一屏取：往下滚的那一下要立刻有图，等滚到才发请求会看见一片空白。
   */
  useEffect(() => {
    if (!auto || seen || url !== null || why !== null) return;
    const el = ref.current;
    if (el === null) return;
    if (typeof IntersectionObserver === 'undefined') {
      // 没有观察器就退回"立刻取"：那是最糟的一档，但比一格都不画强。
      setSeen(true);
      void fetchImage();
      return;
    }
    const io = new IntersectionObserver((entries) => {
      if (!entries.some((e) => e.isIntersecting)) return;
      io.disconnect();          // 只取一次
      setSeen(true);
      void fetchImage();
    }, { rootMargin: '400px' });
    io.observe(el);
    return () => io.disconnect();
  }, [auto, seen, url, why, jobId, name, fs, prefix]);

  useEffect(() => {
    // 换行（换 job / 翻页 / 换目录）就丢掉上一张：那是一份 base64 字符串，
    // 留着只是白占内存。`seen` 也一起归零 —— 新的这一格要重新决定"该不该取"。
    setUrl(null);
    setWhy(null);
    setLoading(false);
    setSeen(false);
  }, [jobId, name, fs, prefix]);

  // 这些分支只画「那一格的图」或「那句状态」，**不写文件名**。
  // 名字归每一格自己写（网格下面那行 `.thumb-name`、表格里那一行的 `{name}`、
  // 差异面那一行的 `{k}`）：组件里再写一遍，图一加载完就会把行里那个名字**换掉**——
  // 真窗口量到的就是这个形状：本地那一栏加载完的图行一个字都没有，而这一面自己那句是
  // 「要动哪个名字就点哪个」，名字没了就点不出那个名字；差异面那一格更重，
  // 「这些名字就是要删的那些」在那一行上读不到名字。两处各写一次就一定会漂。
  if (!looksLikeImage(name)) {
    // 不是图片就没有缩略图——**不摆一个占位方块**，那一格要短就短。
    return null;
  }

  if (why !== null) {
    // 读不出来时交回那两个字，而不是留一个破图：这一格的信息是"这张看不了"，
    // 名字那一行由格子的外面写。那一句为什么**由后台说**（超上限 / 形状画不出来 /
    // 读不出来是三种不同的处置），界面不替它分类成"太大"那种两个字 —— 那是第二份口径。
    // 全文挂原生 title（规程 R68）。
    return (
      <Typography.Text type="secondary" className="fs-caption"
        title={why.detail === '' ? why.short : `${why.short}\n${why.detail}`}>
        看不了
      </Typography.Text>
    );
  }

  // 网格那一格要在**观察器的观察点上**是那一方块，别的都是普通的行内容。
  const inner = url !== null ? (
    <Image
      src={url} alt={name} width={box} height={box} className="thumb-preview"
      // 圆角与裁切在 `styles.css` 的 `.thumb-preview` 里，不在这儿写 `style`：
      // 全仓的圆角都在那一个文件（组件里写圆角是 `frontend_theme.rs` 禁的那件事），
      // 而且 `ThumbCell` 已登记进那道闸的名单——它现在看得见这个文件了。
      // 点开看大图：网格里那几格只是"有哪些图"的答案，看清全貌才是目的。
      preview={{ mask: '看大图' }}
    />
  ) : auto ? (
    <Typography.Text type="secondary" className="fs-caption">
      {loading ? '读着…' : ''}
    </Typography.Text>
  ) : (
    <Typography.Text
      className="fs-code" role="button" style={{ cursor: loading ? 'default' : 'pointer' }}
      onClick={() => { if (!loading) { void fetchImage(); } }}
    >
      {loading ? '读着…' : '看看'}
    </Typography.Text>
  );

  if (!auto) return inner;
  // `auto` 那一档：外面套一个**占满**观察点的 `span`，否则观察的是内容高度
  // —— 内容还没取到时是空的（高度 0），永远不进视口，于是那一格永远不取。
  return (
    <span ref={ref} style={{ display: 'block', width: '100%', minHeight: box }}>
      {inner}
    </span>
  );
}