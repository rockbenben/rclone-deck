use std::collections::BTreeMap;
use std::path::Path;

use serde::Serialize;

use crate::config::edit;
use crate::error::AppError;
use crate::gate::GateReport;
use crate::prune::PruneReport;
use crate::runner::browser::{BrowserListing, BrowserPreview, RemoteRef, RunReport};
use crate::runner::tasklog::TaskLog;
use crate::runner::{PreviewView, RunError, TaskState};
use crate::state::{AppState, JobView, RcdStatus};

const TAIL_BYTES: u64 = 8 * 1024;

/// 日志面要的两件事一起交：**尾巴那一段**，和**它是哪一个文件**。
///
/// 每次动作都新开一个带时间戳的日志文件（`TaskLog::create` 那个 `{job_id}-{YYYYmmdd-HHMMSS}.log`），
/// 所以"完整内容在日志文件里"那一句不带着文件名就指不动 —— 而这条命令本来就为了读尾巴
/// 把路径拿到了手（`Runner::last_log`），旧写法只用了一下就扔掉、只把文本送出去。
/// 路径归后台说：界面不许自己按任务 id 与时间拼文件名（同一坐标系的第二份口径）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogTail {
    pub text: String,
    /// 空串 = 这个任务还没跑过任何东西（那一档界面不显示这一项）。
    pub log_path: String,
}

/// 界面上那些卡：配置文件里那些，加上**这一趟临时立起来的那些**（裁定 R93）。
///
/// 这里交出去的不等于那份文件。设置面读的还是文件（`settings_current` 那条），因为"存成常用"
/// 之后那条任务要真的落盘才算立起来；而跑的那一面读的是这张合成表，所以一张没存过的卡
/// 也能跑。两条读法各自的名字都在这里写清楚，别让人按"左列 = 文件"去推下一件事（spec §8）。
#[tauri::command]
pub async fn get_jobs(state: tauri::State<'_, AppState>) -> Result<Vec<JobView>, AppError> {
    let runner = state.runner()?;
    // locked_by 只从登记表派生，绝不从 `TaskState::Unknown` 的 reason
    // 文本里猜"锁没锁"。`TaskState::Unknown` 有五处构造点：提交没答复、`已提交` 那行日志写不下去、
    // 轮询连续读不到 `job/status`、限流警告那行日志写不下去 —— 这四处的键**都还握着**（guard 在
    // `Inflight` 手里，没确证结束就不放开）；只有第五处（确证结束之后那次 `core/stats` 读失败）
    // 不锁，因为那一刻已经 `disarm` 了。reason 里的散文不是可解析的信号，一律不猜。
    // 登记表本身已经把"键还占着没有"说清楚了：被 `abandoned` 存起来的 guard 仍在表里，
    // 已 `disarm` 的终态则早就不在。
    //
    // 问的是"**谁正挡着这条任务要写的那两棵树**"（`Runner::blocking_owner`，裁定 R92），不是"这组键里
    // 有没有一条**等号**登记"。锁早在 R90 就改成按 `Scope` 的包含关系拒绝，读侧若还按等号查，
    // "占着父目录的那次写"底下那一层就会显示成没锁——按钮亮着、点下去拿到 `Busy`，
    // 界面那句「正在跑：谁」与拒绝的理由从此两说一套。判据与拒绝侧同一个（`mutex::conflicts_with`）。
    // 同一趟还带上**上一次盘点**的那两个数与时刻（`Runner::last_summary`，spec §3）：
    // 界面只显示它，绝不因为"没有"就自己发起一次列举 —— 那正是"每次开窗几十次计费 LIST"的来路。
    Ok(runner.jobs().iter().map(|j| JobView::new(
        j, runner.blocking_owner(j), runner.last_summary(&j.id), runner.is_draft(&j.id))).collect())
}

/// 「换一对位置」：把两处位置立成一张临时卡，交回那张卡的 `pair_key`（界面拿它把右列指过去）。
///
/// 这一条**不写任何文件**：立起来的卡片只活在这一趟面板里（R93/R96），要常用得走设置面那三条
/// 里那条唯一写得进的路（R97「存成常用」跳的就是它）。哪一格缺、两处是不是同一棵树、
/// 那一档能不能建，全在 `Runner::pair_draft` 里说；这里再判一遍就是第二处话术。
#[tauri::command]
pub async fn pair_draft(
    state: tauri::State<'_, AppState>,
    src: String,
    dst: String,
) -> Result<String, AppError> {
    let runner = state.runner()?;
    Ok(runner.pair_draft(&src, &dst)?)
}

/// 收起一张临时卡。正在跑（以及没确证跑完）的那一张会被拒（R98），那句话也在 runner 里说。
#[tauri::command]
pub async fn drop_draft(
    state: tauri::State<'_, AppState>,
    pair_key: String,
) -> Result<(), AppError> {
    let runner = state.runner()?;
    Ok(runner.drop_draft(&pair_key).await?)
}

#[tauri::command]
pub async fn rcd_status(state: tauri::State<'_, AppState>) -> Result<RcdStatus, AppError> {
    // 顶栏是常驻面：那一句在这里就分成两段（人话 + 原文），不把内部名词摆上去。
    let startup_error = state.startup_error().map(|s| crate::note::note_for_text(&s));
    let Ok(runner) = state.runner() else {
        return Ok(RcdStatus {
            ok: false, port: 0, pid: 0,
            rclone_version: String::new(), execute_id: String::new(),
            // 后台**没**说出原因时也不留空：界面替它编一句就是第二份话术，
            // 而"没说出原因"那四个字到界面上就断了，操作员对着它没有下一步。
            startup_error: startup_error
                .or_else(|| Some(crate::note::note_for_text("后台没说为什么不行了。"))),
            // 后台没起来时**照样要把落点交出去**：那一刻正是操作员最需要知道"面板跑在哪个家"
            // 的时候（操作规程 R68 就是靠这一格当场发现两次误启动的），而旧写法在这里填空串、
            // 界面就按"没有"把它藏了 —— 最需要它的时候它不在。
            // 读得到配置就给，读不到（配置本身就是起不来的原因）才留空串，界面仍不说。
            locks: BTreeMap::new(),
            log_dir: crate::config::AppConfig::load_jobs().ok()
                .and_then(|f| crate::config::AppConfig::from_defaults(&f.defaults).ok())
                .map(|c| c.log_dir.display().to_string())
                .unwrap_or_default(),
            config_path: crate::config::AppConfig::jobs_toml_path()
                .map(|p| p.display().to_string()).unwrap_or_default(),
            active_count: 0,
            // 后台没起来 = 没有任何一趟在按某种比法跑。这一档填 false 让顶栏不说那句，
            // 而不是拿配置文件里写着的那一枚去预告"下次会这样"——屏上常驻一句没在跑的比法就是骗。
            size_only: false,
        });
    };
    let rcd = runner.rcd_info();
    let locks = runner.locks().await;
    // 去重后的**持有者**数，不是键数：一次动作可以申领多个键（`acquire_scopes`），
    // 数键会把一件事报成两件（口径写在 `RcdStatus.active_count` 上）。
    let mut owners: Vec<&String> = locks.values().collect();
    owners.sort();
    owners.dedup();
    Ok(RcdStatus {
        ok: true,
        port: rcd.port,
        pid: rcd.pid,
        rclone_version: rcd.rclone_version,
        execute_id: rcd.execute_id,
        startup_error,
        active_count: owners.len(),
        locks,
        log_dir: runner.log_dir(),
        config_path: crate::config::AppConfig::jobs_toml_path()
            .map(|p| p.display().to_string()).unwrap_or_default(),
        // 这一枚读的是**这台面板启动时**那一份 flags（与 rcd 的 argv 同源），不是磁盘上那份：
        // 面板不热重载，改完没重开之前，在跑的仍是旧比法，屏上说的就得是旧的。
        size_only: runner.size_only(),
    })
}

#[tauri::command]
pub async fn job_state(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<TaskState, AppError> {
    Ok(state.runner()?.state(&id).await)
}

#[tauri::command]
pub async fn run_job(
    state: tauri::State<'_, AppState>,
    id: String,
    dry_run: bool,
) -> Result<TaskState, AppError> {
    let runner = state.runner()?;
    // 哪一类任务能从这个入口跑，由 runner 那一侧的门说（`runner::transport_kind_gate`，
    // `run_copy` 的第一件事）。这里以前自己判一遍，于是同一件事有了两处话术、两处会各自漂移，
    // 而"只能跑复制或同步任务"那句在对齐的实删被挡下之后就成了假话——现在只转发 runner 说的。
    // `dry_run` 是 runner 自己的布尔（它翻成 `_config={"dryRun":true}`），不是转发给 rcd 的
    // `_config` flag；`_config` 永不进提交参数。返回的 `TaskState` 若为 `Failed { error }`，
    // 前端配 `tail_log(id)` 同时拿到 rclone 的报错原文与那份逐条日志（删除清单/真实数字只在日志里）。
    Ok(runner.run_copy(&id, dry_run).await?)
}

/// 对齐的第一步：只列不删，交回"会被删掉"的每一行，并登记一次性 token。
///
/// 这一对命令是**任务这一侧**删除的唯一入口（`run_job` 那一档对对齐的实删只会拒绝；浏览器那一面
/// 的按名删除 / `purge` / `rmdir` 走 `browser_*` 那几条，不经过这里）。这里没有任何
/// 判断——哪一侧是全集、两道拒绝、逐条比对，全在 `Runner` 里说；命令层再判一遍就是第二处话术，
/// 两处各自漂移过一次（这里以前写着"这个入口只能跑复制或同步任务"，而对齐的实删随后就被拒了）。
#[tauri::command]
pub async fn preview_mirror(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<PreviewView, AppError> {
    Ok(state.runner()?.preview_mirror(&id).await?)
}

/// 对齐的「不预览直接批」：登记一份**不带逐条名单**的确认，交回一次性凭证。
/// 它一个东西都不删，也一次列举都不发 —— 比例与空侧那两道拒绝留在 `run_mirror` 动手之前，
/// 一道都没挪到这里（挪过来就变成"确认时判一次、动手时不判"，那是假担保）。
#[tauri::command]
pub async fn confirm_mirror(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<String, AppError> {
    Ok(state.runner()?.confirm_mirror(&id).await?)
}

/// 对齐的第二步：拿那份预览的 token 才能真跑。`token` 是必填参数而不是可选——
/// 少了它，这条命令就成了表里第三个能删东西的入口（`tests/commands_surface.rs` 钉这个形状）。
#[tauri::command]
pub async fn run_mirror(    state: tauri::State<'_, AppState>,
    id: String,
    token: String,
) -> Result<TaskState, AppError> {
    Ok(state.runner()?.run_mirror(&id, &token).await?)
}

#[tauri::command]
pub async fn run_gate(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<GateReport, AppError> {
    Ok(state.runner()?.run_gate(&id).await?)
}

#[tauri::command]
pub async fn run_inventory(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<PruneReport, AppError> {
    Ok(state.runner()?.run_inventory(&id).await?)
}

#[tauri::command]
pub async fn tail_log(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<LogTail, AppError> {
    let runner = state.runner()?;
    let Some(path) = runner.last_log(&id).await else {
        // 还没跑过任何东西：两段都是空的，界面按"没有"处理（不显示路径那一格）。
        return Ok(LogTail { text: String::new(), log_path: String::new() });
    };
    let text = TaskLog::tail(&path, TAIL_BYTES).map_err(|e| AppError::Io(e.to_string()))?;
    Ok(LogTail { text, log_path: path.display().to_string() })
}

#[tauri::command]
pub async fn open_dir(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<(), AppError> {
    let runner = state.runner()?;
    let dir = runner.job(&id)
        .and_then(|j| j.open_dir.clone())
        .ok_or_else(|| AppError::Run(RunError::Invariant(format!("任务 {id} 没有配 open_dir"))))?;
    if !Path::new(&dir).is_dir() {
        return Err(AppError::Run(RunError::Invariant(format!("{dir} 不存在"))));
    }
    // explorer 即使成功也返回 1，所以只看有没有拉起来，不看退出码。
    std::process::Command::new("explorer").arg(&dir).spawn()
        .map_err(|e| AppError::Io(format!("打不开 {dir}：{e}")))?;
    Ok(())
}

/// 浏览器左列：rclone 自己报的远端（F3，不解析 `rclone.conf`）∪ 任务里出现过的本地目录。
///
/// 这一对命令是**只读**的：列一个前缀不发写方法、不拿锁（要拿锁的是写动作，裁定 R93），
/// 所以它们不会让任何一棵树变忙，也不会留下副作用。判定（能不能清空、那句"这不是全部"、
/// 能力位那段说明）全在 `runner::browser` 里做完，这里只转发——命令层再判一遍就是第二处话术。
#[tauri::command]
pub async fn browser_remotes(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<RemoteRef>, AppError> {
    Ok(state.runner()?.browser_remotes().await?)
}

/// 列一个前缀（一次递归列举）。`fs` 是 `browser_remotes` 交出的那个 id 原样传回，
/// `prefix` 是清单里某个条目的名字（以那个 id 本身为根时传空串）。
#[tauri::command]
pub async fn browser_list(
    state: tauri::State<'_, AppState>,
    fs: String,
    prefix: String,
) -> Result<BrowserListing, AppError> {
    Ok(state.runner()?.browser_list(&fs, &prefix).await?)
}

/// 下载 / 另存为 / 上传：一条 `copyfile`，只动被点中的那一个对象。
///
/// 两端都交**完整的位置对**（`fs` + 那个对象在 `fs` 根内的相对名），名字由界面从清单里拿：
/// 后台不猜"要点到哪儿"，猜错就是往另一个对象上写（I2c）。目标位置已经有同名对象时
/// 面板在写入之前拦下（R73：没有覆盖这一档，rc 对撞名回的是 200，F10）。
#[tauri::command]
pub async fn browser_copy(
    state: tauri::State<'_, AppState>,
    src_fs: String,
    src_remote: String,
    dst_fs: String,
    dst_remote: String,
) -> Result<RunReport, AppError> {
    Ok(state.runner()?
        .browser_copy(&src_fs, &src_remote, &dst_fs, &dst_remote)
        .await?)
}

/// "删掉这一层看到的 N 个"（R75）与"删这一个"的**预览**：交出的是一份清单加一张凭证，
/// 一个字节都不删（R80：领凭证的命令不算动手）。`names` 为空 = 当前前缀下看到的全部。
#[tauri::command]
pub async fn browser_preview_delete(
    state: tauri::State<'_, AppState>,
    fs: String,
    prefix: String,
    names: Vec<String>,
) -> Result<BrowserPreview, AppError> {
    Ok(state.runner()?.browser_preview_delete(&fs, &prefix, names).await?)
}

/// 改名 / 移动到目录 / 跨远端移动的**预览**（发凭证，什么都没搬）。
/// 破坏性在 `move` 这一档是真的（源会没），所以两件事排在前面：
/// 撞名先拒（F10：move 撞同名会删源而目标一字不改），凭证的 scopes 覆盖**两端**。
#[tauri::command]
pub async fn browser_preview_move(
    state: tauri::State<'_, AppState>,
    src_fs: String,
    src_remote: String,
    dst_fs: String,
    dst_remote: String,
) -> Result<BrowserPreview, AppError> {
    Ok(state.runner()?
        .browser_preview_move(&src_fs, &src_remote, &dst_fs, &dst_remote)
        .await?)
}


/// 建一个目录。**幂等**（F14 实测：已存在再点一次还是 200），所以第二次点下去不报错。
#[tauri::command]
pub async fn browser_mkdir(
    state: tauri::State<'_, AppState>,
    fs: String,
    remote: String,
) -> Result<RunReport, AppError> {
    Ok(state.runner()?.browser_mkdir(&fs, &remote).await?)
}

/// 删一个**空**目录的**预览**（发凭证，一个字节都不删）。
/// 这一档以前是直接动的：那是当时界面上唯一一条不经过清单与凭证的删除路径，
/// 而 R78 那句"没有第二条直接删的路"在它身上是假的。现在动手只走 `browser_run`。
#[tauri::command]
pub async fn browser_preview_rmdir(
    state: tauri::State<'_, AppState>,
    fs: String,
    remote: String,
) -> Result<BrowserPreview, AppError> {
    Ok(state.runner()?.browser_preview_rmdir(&fs, &remote).await?)
}

/// 清空整个前缀的**预览**（发凭证，一个字节都不删）。两道闸在前面：排除规则非空即拒
/// （R74/I8，F11：purge 不受过滤器管，它删的比显示的多），清的就是整个 `fs` 即拒
/// （F12：alias→本地时连目录本身一起删）。
#[tauri::command]
pub async fn browser_preview_purge(
    state: tauri::State<'_, AppState>,
    fs: String,
    prefix: String,
) -> Result<BrowserPreview, AppError> {
    Ok(state.runner()?.browser_preview_purge(&fs, &prefix).await?)
}

/// **唯一的动手入口**（R78）：只认 token，做什么由那份预览自己说。
/// 界面上没有第二条"直接删"的路——多一条，凭证这套机制就整体失效（它挡的是跳过预览的那一步）。
#[tauri::command]
pub async fn browser_run(
    state: tauri::State<'_, AppState>,
    token: String,
) -> Result<RunReport, AppError> {
    Ok(state.runner()?.browser_run(&token).await?)
}

// ---- 设置面那三条（子项目 5）----
//
// 这一族与上面那两对（对齐、浏览器）共用同一套形状：**看一遍 → 交出一份凭证 → 只认凭证动手**。
// 差别在"看的那一份"是什么：对齐看的是会被删掉的那些名字，这里看的是**这次要改哪几行**。

/// 设置面的读侧。**不**要求后台可用：这一面读的是两份文件本身，远端的名字就是那份
/// `rclone.conf` 里有的那几段（`edit::section_names`）——只有一个来源，不另起一份名单。
/// 段读不出来（坏了、或有同名两段）那一档由 `confNote` 交出「这一栏列不出来」+ 原文，
/// 不是"你的配置里没有远端"。
#[tauri::command]
pub async fn settings_current(state: tauri::State<'_, AppState>)
    -> Result<edit::SettingsView, AppError> {
    let files = edit::env_from_config()?;
    let running = running_names(&state).await;
    // `current()` 是同步的，里面有两个只读探针（问配置文件在哪、问这份 rclone 的版本）。
    // 不占 async worker：挪到 spawn_blocking 里。
    Ok(tokio::task::spawn_blocking(move || edit::current(&files, running))
        .await.map_err(|e| AppError::Io(e.to_string()))??)
}

/// 「打开这个目录」那一颗：开的是 **rclone 自己说的那份 conf 所在的层**。
///
/// 路径不接界面交进来的任何东西（那是"让它指哪开哪"），每次点都重新问一次 `rclone config file`
/// —— 问不出（或它只给出一个没有目录的名字）就不开，也不按 `%APPDATA%` 猜一个：
/// 那种猜法在跑夹具的那一遍会把操作员真文件的那一层开出来。
#[tauri::command]
pub async fn open_config_folder() -> Result<(), AppError> {
    let files = edit::env_from_config()?;
    let bin = files.rclone_bin.clone();
    let conf = tokio::task::spawn_blocking(move || edit::conf_path_of(&bin))
        .await.map_err(|e| AppError::Io(e.to_string()))?
        .ok_or_else(|| AppError::Config(edit::EditError::Refused(
            "问不出 rclone 的配置文件在哪，所以那一层也不去开。".to_string())))?;
    let Some(folder) = edit::config_folder_of(&conf.display().to_string()) else {
        return Err(AppError::Config(edit::EditError::Refused(
            format!("它说出的那一个（{conf:?}）里没有目录，不知道要开哪一层。"))));
    };
    if !Path::new(&folder).is_dir() {
        return Err(AppError::Run(RunError::Invariant(format!("{folder} 不存在"))));
    }
    // explorer 即使成功也返回 1，所以只看有没有拉起来，不看退出码（与 `open_dir` 同一条）。
    std::process::Command::new("explorer").arg(&folder).spawn()
        .map_err(|e| AppError::Io(format!("打不开 {folder}：{e}")))?;
    Ok(())
}

/// 要差异：①有没有事正在跑 → ②造新字节 + 装载侧校验 → ③"只改了这些行"，然后登记凭证。
/// 一个字节都还没写。
#[tauri::command]
pub async fn settings_plan(
    state: tauri::State<'_, AppState>,
    registry: tauri::State<'_, edit::WriteRegistry>,
    intent: edit::Intent,
) -> Result<edit::PlanView, AppError> {
    let files = edit::env_from_config()?;
    let running = running_names(&state).await;
    Ok(registry.plan(&intent, &files, &running).await?)
}

/// **唯一**能改这两份文件的入口（与 `browser_run` 同一族规矩）：只认 token。
/// 后台会重新算一遍方案并与凭证里那份逐字节比对，对不上就不写。
#[tauri::command]
pub async fn settings_apply(
    state: tauri::State<'_, AppState>,
    registry: tauri::State<'_, edit::WriteRegistry>,
    token: String,
) -> Result<edit::Applied, AppError> {
    let files = edit::env_from_config()?;
    let running = running_names(&state).await;
    Ok(registry.apply(&token, &files, &running).await?)
}

/// 有几件事正占着锁。后台没起来 = 这台面板上什么都没跑 ⇒ 空表（那句话由 `refuse_while_running` 说）。
///
/// 交的是**持有者的名字**（`Runner::locks()` 的值），与对齐那一句「正在跑：谁」同一个来源；
/// 去重排序后交给设置面点名用。
async fn running_names(state: &tauri::State<'_, AppState>) -> Vec<String> {
    let Ok(runner) = state.runner() else { return Vec::new() };
    let mut names: Vec<String> = runner.locks().await.values().cloned().collect();
    names.sort();
    names.dedup();
    names
}
