import { getVersion } from '@tauri-apps/api/app';
import { useSyncExternalStore } from 'react';

import { errNote, plainNote } from './errors';
import { readStored, writeStored } from './prefs';
import type { ShortNote } from './types';

/**
 * 更新检查 —— 面板**唯一**一处会联网的地方，而且**默认关着**。
 *
 * 三条规矩（与作者另一个应用 QuickText 的「检查更新」同一口径）：
 *
 * 1. **默认关**。问不问由设置面那个开关决定（存不存得住都按"关"算，走 `prefs.ts` 那一层）。
 * 2. **自动检查只问一次**，而且只在面板起来时问；**问完什么都不下载**。
 * 3. **下载与安装都要人按**。有新版时只说"有新版"与版本号，下载是另一颗按钮的事。
 *
 * 这一份状态住在**模块级**而不是组件里：自动检查在面板起来时跑（那时人在哪一面都不知道），
 * 手动检查与那颗下载按钮在设置面那一张卡上 —— 两处读的是同一份。各写一份 state
 * 就是两处会各自漂的话术（同一族毛病见 `Panel.tsx` 里 `railTab` 那段注释）。
 */

/** 「每次打开时检查新版本」这一档的键。 */
const KEY_AUTO = 'deck.update.auto';
const AUTO_ON = 'on';
const AUTO_OFF = 'off';

/** 出了什么事：屏上那一句（`title`）+ 展开里那一句（`note`，两段由 `errNote` 给）。 */
export type UpdateError = { title: string; note: ShortNote };

export type UpdateState = {
  /** 自动检查那一档：开着才在面板起来时问一次。默认关。 */
  auto: boolean;
  /** 正在问（问的时候那几颗按钮都点不动）。 */
  checking: boolean;
  /** 正在下。 */
  downloading: boolean;
  /** 已经下了多少字节。 */
  downloaded: number;
  /** 一共多少字节（问不到就是 null，那就没有百分比可算）。 */
  total: number | null;
  /** 有新版时的那个版本号；空串 = 没有。 */
  version: string;
  /** 面板现在跑着的那个版本号。 */
  current: string;
  /** 一句人话（"已经是最新"那一类）。空 = 没什么可说的。 */
  note: ShortNote | null;
  /** 出了什么事（问不到 / 下不动 / 装不上）。 */
  error: UpdateError | null;
};

const initial: UpdateState = {
  auto: readStored(KEY_AUTO, [AUTO_ON, AUTO_OFF], AUTO_OFF) === AUTO_ON,
  checking: false,
  downloading: false,
  downloaded: 0,
  total: null,
  version: '',
  current: '',
  note: null,
  error: null,
};

let state: UpdateState = initial;
const listeners = new Set<() => void>();

function set(patch: Partial<UpdateState>): void {
  state = { ...state, ...patch };
  for (const l of listeners) l();
}

function subscribe(l: () => void): () => void {
  listeners.add(l);
  return () => { listeners.delete(l); };
}

/** 读这一份状态。组件订阅它，状态一变就重画。 */
export function useUpdates(): UpdateState {
  return useSyncExternalStore(subscribe, () => state);
}

/** 面板现在跑着的是哪个版本：问应用自己，不联网。 */
export async function loadCurrentVersion(): Promise<void> {
  try {
    set({ current: await getVersion() });
  } catch {
    // 问不到就空着：版本号只是屏上那一行读数，问不到不该挡住别的。
  }
}

/** 这一次问的结果。`skipped` = 这一趟压根没问（没开那一档，或已经问过了）。 */
export type UpdateOutcome =
  | { status: 'up-to-date' }
  | { status: 'available'; version: string }
  | { status: 'failed'; error: UpdateError }
  | { status: 'skipped' };

/**
 * 问一次有没有新版。
 *
 * `manual` 是"人按了"与"面板自己问的"那一条分界：**只有手动那一次会说话**
 * （已经是最新 / 出了什么事），自动那一次不出声 —— 没人问，就不该吵。
 */
export async function checkForUpdate(manual: boolean): Promise<UpdateOutcome> {
  set({ checking: true, note: null, error: null });
  try {
    const { check } = await import('@tauri-apps/plugin-updater');
    const update = await check();
    if (update === null) {
      set({ checking: false, version: '', note: manual ? plainNote('已经是最新') : null });
      return { status: 'up-to-date' };
    }
    set({
      checking: false,
      version: update.version,
      current: update.currentVersion,
      note: null,
    });
    return { status: 'available', version: update.version };
  } catch (e) {
    const error: UpdateError = { title: '检查更新失败', note: errNote(e) };
    set({ checking: false, error: manual ? error : null });
    return { status: 'failed', error };
  }
}

/** 自动检查每次开面板只问一次：StrictMode 会把 effect 跑两遍，而问一次是要联网的。 */
let autoChecked = false;

/** 面板起来时那一次：没开那一档就什么都不做，问过了也不问第二次。 */
export async function autoCheck(): Promise<UpdateOutcome> {
  if (!state.auto) return { status: 'skipped' };
  if (autoChecked) return { status: 'skipped' };
  autoChecked = true;
  return checkForUpdate(false);
}

/** 那一档开关。开了之后当场的这一次也要问（人刚按了，总得有个说法）。 */
export function setAuto(on: boolean): void {
  writeStored(KEY_AUTO, on ? AUTO_ON : AUTO_OFF);
  set({ auto: on });
  if (on) void checkForUpdate(true);
}

/**
 * 下载并装上。**只有人按了才走这一条。**
 *
 * 装的时候这个进程就没了：Windows 那个安装器接管之后会把面板重开
 * （插件源码里 `/R` 那一档），所以 `install()` 之后的那几行多半走不到。
 */
export async function downloadAndInstall(): Promise<void> {
  set({ downloading: true, downloaded: 0, total: null, note: null, error: null });
  try {
    const { check } = await import('@tauri-apps/plugin-updater');
    const update = await check();
    if (update === null) {
      set({ downloading: false, note: plainNote('已经是最新') });
      return;
    }
    // 重新问了一次，所以屏上那个版本号跟着这一次的走（两次之间发了新版的话，
    // 装的就是屏上写着的那一个，不是一句过期的话）。
    set({ version: update.version, current: update.currentVersion });
    let downloaded = 0;
    let total: number | null = null;
    await update.download((event) => {
      if (event.event === 'Started') {
        total = event.data.contentLength ?? null;
      } else if (event.event === 'Progress') {
        downloaded += event.data.chunkLength;
      }
      set({ downloaded, total });
    });
    set({ downloading: false, note: plainNote('正在装新版本', '装完这个面板会自己重开。') });
    await update.install();
  } catch (e) {
    set({ downloading: false, error: { title: '装新版本失败', note: errNote(e) } });
  }
}
