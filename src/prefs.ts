import { useState } from 'react';

/**
 * 「你上次在看什么」——面板开在哪一面、右列停在哪一段。这两件重开该回来。
 *
 * 这两件之外，**只有更新那一档**也走这一层（`src/updates.ts` 用下面这两个读写函数存它）：
 * 它同样要跨重开记住，否则"开了"每次开面板都被退回关着，那一档就等于没有。
 * 模式与方向那两格**不在这里记**：记住一个"会删东西"的选择，等于让某天的
 * 第一次点击落在最重的那一档上（那条规则由 `frontend_pairs.rs` 的
 * `the_mode_choice_never_outlives_the_session` 钉着）。这里的键名单里出现 mode / slot，
 * 本文件的证人就红 —— 那是把安全规则顺手扩到偏好层来。
 *
 * 口径抄 `theme.ts` 那一套（存不下就只生效这一次，不因为存储不可用而拒掉界面）：
 * 两处用同一句"读不到就当没存过"，加新偏好时也照这一份抄，而不是各写一套 try/catch。
 */
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
