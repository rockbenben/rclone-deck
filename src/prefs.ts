import { useEffect, useState } from 'react';

export const KEY_FACE = 'deck.view';
export const KEY_TAB = 'deck.tab';

/** 读一个存过的偏好：读不到（或存的不是名单里那一个）就当没存过。 */
export function readStored<T extends string>(key: string, allowed: readonly T[], fallback: T): T {
  try {
    const v = localStorage.getItem(key);
    return allowed.includes(v as T) ? (v as T) : fallback;
  } catch {
    return fallback;
  }
}

/** 写一个偏好：存不住就只生效这一次（换一件这件事不该因为存储不可用而被拒）。 */
export function writeStored(key: string, value: string): void {
  try {
    localStorage.setItem(key, value);
  } catch {
    // 存不住就只生效这一次：换一个面这件事不该因为存储不可用而被拒。
  }
}

/** 一个"初值从磁盘来、改的时候顺手写回去"的 state。 */
export function useStored<T extends string>(
  key: string, allowed: readonly T[], fallback: T,
): [T, (next: T) => void] {
  const [value, setValue] = useState<T>(() => readStored(key, allowed, fallback));
  return [value, (next: T) => { writeStored(key, next); setValue(next); }];
}

/** 结构化偏好通过调用方校验。读取或写入失败时，仍可在当前会话继续选择。 */
export function useStoredJson<T>(key: string, decode: (value: unknown) => T) {
  const [value, setValue] = useState(() => {
    try {
      return decode(JSON.parse(localStorage.getItem(key) ?? 'null'));
    } catch {
      return decode(null);
    }
  });
  useEffect(() => { writeStored(key, JSON.stringify(value)); }, [key, value]);
  return [value, setValue] as const;
}

export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}
