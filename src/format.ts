import type { BrowserEntry, TaskState } from './types';

/** 相对名的最后一段。清单行上的名字是**根内相对名**（`pics/2026/a.webp`），
 *  而界面上"这一层里有什么"那一格只该摆最后那一段 —— 前面那几段面包屑已经说过了。
 *  两处都要用（动作条算落点、网格那一格写名字），所以只写在这一处。 */
export function baseName(name: string): string {
  const i = name.lastIndexOf('/');
  return i < 0 ? name : name.slice(i + 1);
}

export function formatBytes(n: number): string {
  if (!Number.isFinite(n) || n <= 0) return '0 B';
  const units = ['B', 'KiB', 'MiB', 'GiB', 'TiB'];
  const i = Math.min(units.length - 1, Math.floor(Math.log2(n) / 10));
  const v = n / 2 ** (10 * i);
  return `${i === 0 ? v.toFixed(0) : v.toFixed(v < 10 ? 2 : 1)} ${units[i]}`;
}

export function formatFileChanges({ stats, dry_run }: Extract<TaskState, { state: 'done' }>): string {
  const { transfers, bytes, deletes, deletedDirs } = stats;
  if (transfers === 0 && deletes === 0 && deletedDirs === 0) {
    return dry_run ? '没有需要复制或删除的文件' : '本次没有复制或删除文件';
  }
  const copied = `${dry_run ? '预计复制' : '已复制'} ${transfers} 个文件`;
  const deleted = `${dry_run ? '预计删除' : '已删除'} ${deletes} 个文件`;
  return [
    transfers > 0 ? `${copied}（${formatBytes(bytes)}）` : copied,
    deletedDirs > 0 ? `${deleted}、${deletedDirs} 个目录` : deleted,
  ].join(' · ');
}

export function formatSpeed(bytesPerSec: number): string {
  if (!Number.isFinite(bytesPerSec) || bytesPerSec <= 0) return '—';
  return `${formatBytes(bytesPerSec)}/s`;
}

export function formatEta(sec: number | null): string {
  if (sec === null || !Number.isFinite(sec) || sec < 0) return '—';
  return formatDuration(sec);
}

export function formatDuration(sec: number): string {
  const s = Math.max(0, Math.round(sec));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const r = s % 60;
  if (h > 0) return `${h} 小时 ${m} 分`;
  if (m > 0) return `${m} 分 ${r} 秒`;
  return `${r} 秒`;
}

/** 与后台的图片扩展名列表一致，供缩略图和网格共用。 */
const IMAGE_EXTS = ['.png', '.jpg', '.jpeg', '.gif', '.webp', '.bmp', '.avif'];

export function looksLikeImage(name: string): boolean {
  const lower = name.toLowerCase();
  return IMAGE_EXTS.some((ext) => lower.endsWith(ext));
}

export function classifyBrowserEntries(entries: BrowserEntry[], showGrid: boolean) {
  const grid: BrowserEntry[] = [];
  if (!showGrid) return { grid, rest: entries };

  const rest: BrowserEntry[] = [];
  for (const entry of entries) {
    if (!entry.isDir && looksLikeImage(entry.name)) grid.push(entry);
    else rest.push(entry);
  }
  return { grid, rest };
}
