import { Breadcrumb } from 'antd';

/**
 * 可点的层级路径。它不是命令行：每一段都来自后台交出的那份清单（`prefix` 就是这些段拼出来的），
 * 点不动的段根本不存在 —— 没有"敲一个路径试试"这条路。
 *
 * 段与段之间点下去都是**换位置**（`onJump` 交回一个相对根的前缀，'' 就是根），
 * 不产生任何写动作，所以这里不需要确认层。
 */
export function Crumbs(props: { root: string; prefix: string; onJump: (prefix: string) => void }) {
  const { root, prefix, onJump } = props;
  const segs = prefix === '' ? [] : prefix.split('/');
  // 还没选位置时**什么都不摆**：那一格此刻已有三处说同一件事 —— 这一行（原来写「（还没选位置）」）、
  // 那枚框的占位（「选一个位置」）、以及框下面那句（「选一个位置，这一栏会列出那里有什么」）。
  // 三句里唯一带新信息的是最后那一句（它说了会发生什么），另两句是同一件事的复述。
  // 而且根还没定的时候这一行点下去也没有可跳的去处 —— 摆一个点不动的面包屑，比不摆更像在指路。
  if (root === '' && segs.length === 0) return null;
  return (
    <Breadcrumb
      items={[
        { title: <a onClick={() => onJump('')}>{root}</a> },
        ...segs.map((_, i) => ({
          title: <a onClick={() => onJump(segs.slice(0, i + 1).join('/'))}>{segs[i]}</a>,
        })),
      ]}
    />
  );
}
