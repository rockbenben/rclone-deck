import { useState } from 'react';

/**
 * 「你上次在看什么」——面板开在哪一面、右列停在哪一段。这两件重开该回来。
 *
 * 只有这两件。模式与方向那两格**不在这里记**：记住一个"会删东西"的选择，等于让某天的
 * 第一次点击落在最重的那一档上（那条规则由 `frontend_pairs.rs` 的
 * `the_mode_choice_never_outlives_the_session` 钉着）。这里的键名单里出现 mode / slot，
 * 本文件的证人就红 —— 那是把安全规则顺手扩到偏好层来。
 *
 * 口径抄 `theme.ts` 那一套（存不下就只生效这一次，不因为存储不可用而拒掉界面）：
 * 两处用同一句"读不到就当没存过"，将来加第三件偏好时也照这一份抄，而不是各写一套 try/catch。
 */
export const KEY_FACE = 'deck.view';
export const KEY_TAB = 'deck.tab';

function readStored<T extends string>(key: string, allowed: readonly T[], fallback: T): T {
  try {
    const v = localStorage.getItem(key);
    return allowed.includes(v as T) ? (v as T) : fallback;
  } catch {
    return fallback;
  }
}

function writeStored(key: string, value: string): void {
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
