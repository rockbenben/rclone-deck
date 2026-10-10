import type { ShortNote } from './types';

/**
 * 后台交出来的错误是**两段**：`short` 给人看，`detail` 是排障原文（Rust 侧 `AppError::note`，
 * 判据在 `src-tauri/src/note.rs`）。
 *
 * 以前这里是 `String(e)`：invoke 拒绝的就是后台 Display 那根字符串，于是界面上出现
 * `rc 调用 operations/list 失败：error in ListJSON: … AccessDenied …`。前端那条
 * "界面不许出现内部名词"的证人扫的是**字面量**，运行时拼进来的这一族它一条也扫不到。
 *
 * 认不出这个形状时退回整串——那是 JS 侧自己抛的（比如 fetch 挂了、读不到 undefined 的属性），
 * 不是后台的话；那种时候编一句"这一步没做成"反而把真信息藏了。
 */
export function errNote(e: unknown): ShortNote {
  if (e !== null && typeof e === 'object') {
    const n = e as { short?: unknown; detail?: unknown };
    if (typeof n.short === 'string') {
      return { short: n.short, detail: typeof n.detail === 'string' ? n.detail : '' };
    }
  }
  return { short: String(e), detail: '' };
}

/** 界面自己那句话（不是后台交来的）：一句人话，外加一句可选的展开。
 *  用在命名框与"点不动"那几条本地校验上——它们的判据就在界面手里，没有原文可附，
 *  但**条件本身**值得单独一行（`DESIGN.md`：明面那一格 ≤16 字，细节收进展开）。 */
export function plainNote(short: string, detail = ''): ShortNote {
  return { short, detail };
}

/** "同一段故障里别刷屏"那道门的比较键（裁定 R57 第三条：同因不重复，**换因要再说一次**）。
 *  两段都要进键：只比 `short` 的话，两次不同的 403（不同位置）会被当成同因吞掉第二次。 */
export function noteKey(n: ShortNote): string {
  return n.detail === '' ? n.short : `${n.short}\u0000${n.detail}`;
}
