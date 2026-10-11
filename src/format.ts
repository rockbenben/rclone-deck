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
