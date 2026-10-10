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
