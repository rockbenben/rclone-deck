import { invoke } from '@tauri-apps/api/core';

import type {
  Applied, BrowserListing, BrowserPreview, ConfigIntent, GateReport, JobView, LogTail,
  PreviewView, PruneReport, RcdStatus, RemoteRef, RunReport, SettingsPlan, SettingsView, TaskState,
} from './types';

export const getJobs = () => invoke<JobView[]>('get_jobs');
// 「换一对位置」那两条（R93/R96/R98）：立一张只在内存里的卡、收起它。
// `pairDraft` 交回的是后台算好的 `pairKey`（与 JobView.pairKey 同一把尺子），界面拿它把右列指过去；
// 它**不写得进任何文件**，要常用得走下面那三条设置面的路（R97「存成常用」跳的就是那一条）。
// 这两条都不碰任何一棵树，所以也没有删除通路被多加一条——临时格实删走的还是上面那对对齐入口。
export const pairDraft = (src: string, dst: string) => invoke<string>('pair_draft', { src, dst });
export const dropDraft = (pairKey: string) => invoke<void>('drop_draft', { pairKey });
export const rcdStatus = () => invoke<RcdStatus>('rcd_status');
export const jobState = (id: string) => invoke<TaskState>('job_state', { id });
export const runJob = (id: string, dryRun: boolean) => invoke<TaskState>('run_job', { id, dryRun });
export const runGate = (id: string) => invoke<GateReport>('run_gate', { id });
// R58：盘点只有这一个入口（只读）。执行命令 `run_prune_execute` 已随删除路径一起移除。
export const runInventory = (id: string) => invoke<PruneReport>('run_inventory', { id });

/**
 * 读一张**本地**图片，交成 `data:` URL。
 *
 * 注意签名里只有 `id` 与 `name`——**没有路径**。根由后台按 `id` 反查任务自己定，
 * 越权的形状（`..`、盘符、目录外）一律在 Rust 侧拒（见 `runner::local_image_data_url` 上那三道闸）。
 * 前端要是能传路径，这一条就变成任意文件读取器了。
 */
export const previewLocalImage = (id: string, name: string) =>
  invoke<string>('preview_local_image', { id, name });

/**
 * 读浏览面板**当前那一栏**里的一张本地图。
 *
 * `fs` 是后台交出来的位置坐标（`local_fs_id` 编出的 `D:/pics` 那种形状），`prefix` 是
 * 这一栏的导航状态，两者都由界面持有但都不是用户可以乱填的路径 —— 后台把它们反解回
 * `Path` 再过那三道闸。**不传绝对路径**，理由同 `previewLocalImage`。
 */
export const previewPaneImage = (fs: string, prefix: string, name: string) =>
  invoke<string>('preview_pane_image', { fs, prefix, name });

/**
 * 远端那一栏的缩略图（云端预览）。
 *
 * 后台用 `operations/cat` 并让**取内容的那一侧就截断**（`count` 被翻成 Range 头下发），
 * 所以这不是「把整张原图拉回来再裁」—— 一张 20 MB 的图只下前若干 KB。
 * 同样**不传绝对路径**：名字必须落在 `prefix` 那一层之下，后台会判。
 */
export const previewRemoteThumb = (fs: string, prefix: string, name: string) =>
  invoke<string>('preview_remote_thumb', { fs, prefix, name });
// 删除只有这一对入口：`runJob(id, false)` 对对齐任务会被后台拒绝（那句话术在 runner 里）。
// `runMirror` 的 token 必须来自**刚刚那一次**登记：带着逐条名单的那份出自 `previewMirror`，
// 没看名单的那份出自 `confirmMirror`。它单次消费，后台还会重列一遍（看过的那份逐条比对），
// 对不上就不动。这里不缓存、不猜、不传空串。「看差异」那份是另一条路（下面 `confirmMirror`）。
export const previewMirror = (id: string) => invoke<PreviewView>('preview_mirror', { id });
export const runMirror = (id: string, token: string) => invoke<TaskState>('run_mirror', { id, token });
// 「不预览直接批」那一条：登记一份**不带逐条名单**的确认，交回一次性凭证。它一个东西都不删，
// 也一次列举都不发（两道拒绝留在动手之前，一道都没挪到这一步）。
export const confirmMirror = (id: string) => invoke<string>('confirm_mirror', { id });
export const tailLog = (id: string) => invoke<LogTail>('tail_log', { id });
export const openDir = (id: string) => invoke<void>('open_dir', { id });
// 浏览器读侧（Task 8）：这一对只列东西，不动东西。`fs` 必须是 `browserRemotes()` 交出的那个 id
// 原样传回——本地那几行的 id 是后台按 F16 转换过的拼法，界面不许自己拼路径或改斜杠。
export const browserRemotes = () => invoke<RemoteRef[]>('browser_remotes');
export const browserList = (fs: string, prefix: string) => invoke<BrowserListing>('browser_list', { fs, prefix });
// 浏览器写动作（Task 10）：两端都交**完整的位置对**——`fs` 是左列交出的 id 原样传回，
// `remote` 是清单里那一行的 `name`（它已经是 fs 根内的完整相对名，不是只显示的最后一段）。
// 界面不许自己拼路径、补冒号或改斜杠，也不许替操作员猜目标名字：撞名由后台在写入之前拦下。
export const browserCopy = (srcFs: string, srcRemote: string, dstFs: string, dstRemote: string) =>
  invoke<RunReport>('browser_copy', { srcFs, srcRemote, dstFs, dstRemote });
// 目录这一族（Task 12）：建目录是"看得见就点得动"的那一条，不发凭证；
// 删空目录、清空整个前缀都是**预览**，动手仍然只走 browserRun(token)（R78）。
export const browserMkdir = (fs: string, remote: string) => invoke<RunReport>('browser_mkdir', { fs, remote });
export const browserPreviewRmdir = (fs: string, remote: string) => invoke<BrowserPreview>('browser_preview_rmdir', { fs, remote });
export const browserPreviewPurge = (fs: string, prefix: string) => invoke<BrowserPreview>('browser_preview_purge', { fs, prefix });
// 改名 / 移动（Task 13）：预览发凭证，两端各交完整的位置对；动手只走 browserRun(token)。
export const browserPreviewMove = (srcFs: string, srcRemote: string, dstFs: string, dstRemote: string) =>
  invoke<BrowserPreview>('browser_preview_move', { srcFs, srcRemote, dstFs, dstRemote });
// 删除这一族（Task 11）：预览发清单 + 凭证，动手只认 token。
// `names` 为空数组 = 「删掉这一层看到的 N 个」；非空 = 只删点中的那几个。
// 界面上不存在第三条「直接删」的路径：多一条，凭证这套机制整体失效（R78）。
export const browserPreviewDelete = (fs: string, prefix: string, names: string[]) =>
  invoke<BrowserPreview>('browser_preview_delete', { fs, prefix, names });
export const browserRun = (token: string) => invoke<RunReport>('browser_run', { token });

// 设置面那三条：读现状 / 要差异 / 带凭证落盘。与上面那两对同一族规矩——
// **写得进那两份文件的通路只有 `settingsApply` 这一条，而且它只认 `settingsPlan` 交回来的那一个 token**。
// 一次算完的是一整块卡片（差异行由后台数），不是一个字段发一次：界面要能回答"这次改哪几行"。
export const settingsCurrent = () => invoke<SettingsView>('settings_current');
export const settingsPlan = (intent: ConfigIntent) => invoke<SettingsPlan>('settings_plan', { intent });
// `token` 单次消费，而且落盘前后台会重新算一遍与这份逐字节比对：磁盘被别的东西改过就不写。
export const settingsApply = (token: string) => invoke<Applied>('settings_apply', { token });

// 「打开这个目录」：不带任何参数——那一层由后台问 rclone 自己得到（`open_dir` 是另一条，它开的是任务里那一格）。
export const openConfigFolder = () => invoke<void>('open_config_folder');
