pub mod commands;
pub mod config;
pub mod endpoint;
pub mod error;
pub mod gate;
pub mod moment;
pub mod mutex;
pub mod note;
pub mod pair;
pub mod preview;
pub mod prune;
pub mod rc;
pub mod rcd;
pub mod runner;
pub mod scope;
pub mod state;

use std::sync::Arc;

use state::AppState;
use tauri::Manager;

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let app_handle = app.handle().clone();
            let state = tauri::async_runtime::block_on(async move {
                match state::bootstrap().await {
                    Ok(runner) => AppState::new(Some(runner), None),
                    Err(e) => AppState::new(None, Some(e.reason())),
                }
            });
            app_handle.manage(state);
            // 配置写的凭证表：与 `AppState` 分开的第二份管理状态。
            // 它不是 `AppState` 的一个字段，因为那是两件事——"这台面板现在能不能用"（`AppState`）
            // 与"有没有一份看过差异的写在等着落盘"（这张表）。塞在一起会让读侧一坏，
            // 凭证表也跟着说"没有后台就不给写"，而配置写在后台起不来的时候照样要看差异。
            app_handle.manage(config::edit::WriteRegistry::default());
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                let state = window.state::<AppState>();
                if let Some(runner) = state.take_runner() {
                    // `take_runner()` 只在这里用（carry #4 的例外说明）：关窗要把 `Runner` 连它
                    // 内层的 `Arc<RcdHandle>` 一起收走，才能调 `shutdown_rcd`（它按值收 `Runner`），
                    // 而不是让命令继续共享它。正常关窗时这是最后一个引用，`try_unwrap` 解得出。
                    // 若仍有在途命令握着克隆（一条 `run_job` 正 await 传输），解不出就**不阻塞**等它：
                    // `RcdHandle` 故意不在 `Drop` 里杀进程，rcd 作为孤儿留下，由下次启动的三重身份
                    // 认领回收（见 rcd 模块头）。崩溃本来就走不了这条路，同一套孤儿认领兜底。
                    if let Ok(owned) = Arc::try_unwrap(runner) {
                        // 收尾失败不许 `let _ =`（本项目的既有规矩）：这条真失败的意思是
                        // rcd 还在这个端口上听着。它不是数据危险——下次启动会按三重身份把它**终结后重启**
                        // （spec §3.1 情形 2 定的是"不复用旧实例"，Task 12 量过：那会打断一个还在跑的传输），
                        // 但"我按了关闭、它还活着"这件事必须说出来，而不是静默。
                        // release 是 GUI 子系统，stderr 没有落点，所以只在 debug 构建里说
                        // （与 `mutex.rs` 绊线同一口径）。
                        if let Err(e) = tauri::async_runtime::block_on(owned.shutdown_rcd()) {
                            #[cfg(debug_assertions)]
                            eprintln!("rcd 收尾失败：{e}——默认端口 上可能还挂着，下次启动会按三重身份终结并重启");
                            // release 构建里这个绑定没人用（打印被 cfg 掉了），留着它就是
                            // `cargo check --release` 的一条警告——而这条命令没有 CI 会跑，
                            // 所以只能在这里主动消费掉，与 `mutex.rs` 的 `let _ = &note;` 同一手法。
                            #[cfg(not(debug_assertions))]
                            let _ = &e;
                        }
                    }
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_jobs,
            // 「换一对位置」那两条：立一张只在内存里的卡、收起它（R93/R98）。
            // 它们不碰任何一棵树，所以不带 token；而**删除的入口一条都没多**——
            // 临时格实删走的还是下面那对 `preview_mirror` → `run_mirror`。
            commands::pair_draft,
            commands::drop_draft,
            commands::rcd_status,
            commands::job_state,
            commands::run_job,
            // 对齐那三条：预览交回逐条名单与一次性 token；「不预览直接批」走 `confirm_mirror`
            // （它一次列举都不发，也一个东西都不删）；动手那一条只认 token。
            // 表里没有第三条能删东西的路（`tests/commands_surface.rs` 读这张表）。
            commands::preview_mirror,
            commands::confirm_mirror,
            commands::run_mirror,
            commands::run_gate,
            commands::run_inventory,
            commands::preview_local_image,
            commands::preview_pane_image,
            commands::tail_log,
            commands::open_dir,
            // 浏览器读侧那一对：只列东西、不动东西，命令表里"能删的那一条"的形状一个字没变
            //（这张表由 `tests/commands_surface.rs` 逐字读，所以这里不许出现别的入口名）。
            commands::browser_remotes,
            commands::browser_list,
            // 浏览器第一个会写动作的入口：只写被点中的那一个对象，撞名在写入之前由面板拦下。
            // 它进的是 `copyfile` 这一条具名通路，不是整棵树的 `copy`/`sync`（裁定 F2/I4）。
            commands::browser_copy,
            // 删除这一族：预览发凭证，动手只认 token（R78）。表里没有第三条"直接删"的路。
            commands::browser_preview_delete,
            commands::browser_run,
            // 目录这一族：建与删空都不发凭证（幂等 / 只删得掉空的），清空整个前缀走预览→动手。
            commands::browser_mkdir,
            commands::browser_preview_rmdir,
            commands::browser_preview_purge,
            // 改名与移动：预览发凭证（两端都进 scopes），动手仍然只走 browser_run(token)。
            commands::browser_preview_move,
            // 设置面那三条（子项目 5）：读现状 / 要差异 / 带凭证落盘。
            // 与上面两对同一族判据：**能写的那一条只认 token**，这张表里没有第二条写得进
            // 那两份文件的路（`tests/commands_surface.rs` 逐字读这张表，也数这一族有几条）。
            commands::settings_current,
            commands::open_config_folder,
            commands::settings_plan,
            commands::settings_apply,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
