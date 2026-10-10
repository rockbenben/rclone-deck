//! 「开始」这个入口受理哪一类任务（裁定 R60 的那道闸）。
//!
//! 为什么这里只有纯函数那一半：`run_job` 是个 Tauri 命令，签名里的 `tauri::State` 在测试里
//! 构造不出来，所以闸必须装在能测的地方（纯函数 + `Runner::run_copy` 自己把门），命令层只转发。
//! "门真的装在 `run_copy` 上、被拒的动作不留副作用"那一半在 `tests/runner_local.rs`——
//! 那边有现成的 rcd 夹具。
//!
//! 后面三条证人钉的是命令层的**形状**：命令表里躺着什么、能删的那条命令要什么、
//! 前端喊的是哪个名字。第二句扫的是确认执行那一路的拒绝话术：同一个规矩，另一批句子。

use rclone_deck_lib::config::jobs::JobKind;
use rclone_deck_lib::runner::{
    mirror_confirmation_voided_refusal, mirror_list_changed_refusal, mirror_side_refused_refusal,
    mirror_wrong_kind_refusal, transport_kind_gate,
};

/// 读后台/前端源文本：下面三条证人钉的都是"形状"——命令表里有什么、命令签名要什么、
/// 前端调的是哪个名字。这些东西没有一处运行时断言管得住，只有源文本本身是权威。
fn read_src(rel: &str) -> String {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("读不到 {}：{e}", p.display()))
}

const MIRROR: &str = "镜像 R2：以本地为主（删云端多余的）";

/// 话术里不许出现的东西：Rust 枚举名、内部命令名（R56 第 2 条：那是给操作员看的句子）。
/// 「让两边对齐」**已经从这份名单里出去了**：那个按钮现在就挂在对齐行上，拒绝话术指着它
/// 才是说真话。指着界面上没有的入口——不管是还没有的还是已经改名的——都是一次没人接得住的拒绝。
const BANNED: &[&str] = &[
    "Sync", "Copy", "Gate", "JobKind", "run_copy", "run_job", "run_mirror", "preview_mirror",
];

/// 对齐真正存在的入口，拒绝话术必须指得出它。
const MIRROR_ENTRY: &str = "让两边对齐";

#[test]
fn the_gate_lets_copy_and_the_trial_run_through_and_stops_a_real_mirror() {
    // 复制：两个布尔都放行（`dry_run` 对 copy 只是"别写真东西"，不涉及删除范围）。
    transport_kind_gate(&JobKind::Copy, "R2 图床上传", false).unwrap();
    transport_kind_gate(&JobKind::Copy, "R2 图床上传", true).unwrap();
    // 对齐的只读那一趟放行：它删不了任何东西。放行它不再是"给操作员留一条交叉核对的路" ——
    // 那颗入口 2026-10-05 已从对齐这一格撤了，对数仍然跑，只是跑在 `tests/mirror_runner.rs` 里。
    transport_kind_gate(&JobKind::Sync, MIRROR, true).unwrap();

    let err = transport_kind_gate(&JobKind::Sync, MIRROR, false).unwrap_err();
    // 认**分类那个词**，不认"镜像"二字：这条任务的名字本身就叫「镜像 R2：…」，
    // 拿 `contains("镜像")` 判"句子说得出这是哪一档"是名字蒙过去的 —— 话术哪天改口它照样绿。
    assert!(err.contains("对齐任务"), "要说出这是对齐任务，而不是把任务名念一遍：{err}");
    // 这句里**不许**再指着卡上已经没有的那颗只读按钮（「看看会动什么」原名「先试跑」，
    // 2026-10-09 改的字面；「先看一遍」是中间一版的叫法）。指着一个界面上没有的入口
    // = 一次没人接得住的拒绝，那正是这一族的病（句子指在"看"那一颗）。三种拼法都钉住：
    // 旧名留着是为了**改名之后**这一条仍然拦得住指回旧按钮的话术。
    assert!(!err.contains("看看会动什么") && !err.contains("先看一遍") && !err.contains("先试跑"),
        "那句还指着卡上已经没有的一颗：{err}");
    assert!(err.contains("预览") || err.contains("清单"), "要说明实删为什么被拒：{err}");
    assert!(err.contains(MIRROR_ENTRY), "要指得出真正存在的那个入口，而不是背一句口号：{err}");
    for b in BANNED {
        assert!(!err.contains(b), "镜像那档的拒绝话术漏了内部名词 {b}：{err}");
    }

    // 对账类那一档：词来自 `JobKind::word()`（人话词表的唯一出处），不是枚举的 Debug 写法。
    let g = transport_kind_gate(&JobKind::Gate, "ideahub 封面对账", false).unwrap_err();
    assert!(g.contains(JobKind::Gate.word()), "那句子里的人话词不是 word() 给的：{g}");
    assert!(g.contains("ideahub 封面对账"), "要点名是哪个任务：{g}");
    assert!(!g.contains("看看会动什么") && !g.contains("先看一遍") && !g.contains("先试跑"),
        "这一句也顺带指着那颗已经不存在的按钮：{g}");
    assert!(g.contains(MIRROR_ENTRY), "镜像那一路要说得出今天真正的那个入口：{g}");
    for b in BANNED {
        assert!(!g.contains(b), "检查那档的拒绝话术漏了内部名词 {b}：{g}");
    }
}

/// 确认执行那一路的拒绝话术（`run_mirror` 流程中间的四句，抽成纯函数就是为了这里够得着）。
/// 上面那份扫描原本只覆盖 `transport_kind_gate` 自己的两句，剩下那几句等于从没被这条规矩管过，
/// 而恰恰是它们最容易顺手把内部名词写进去——它们要解释"为什么没有一个硬删的按钮"。
#[test]
fn the_mirror_refusals_never_leak_internal_names() {
    let all = [
        mirror_wrong_kind_refusal(MIRROR, &JobKind::Copy.word()),
        mirror_confirmation_voided_refusal(MIRROR),
        mirror_list_changed_refusal(MIRROR, 3, 4),
        mirror_side_refused_refusal(MIRROR, true, false),
    ];
    for err in &all {
        for b in BANNED {
            assert!(!err.contains(b), "镜像的拒绝话术漏了内部名词 {b}：{err}");
        }
    }
    // 每一句拒绝都得给出路（回到预览），而不是只说不。
    for err in &all[1..] {
        assert!(err.contains("重新预览"), "那句拒绝没指出下一步：{err}");
    }
    assert!(all[1].contains("不作数"), "作废那句必须说得出作废：{}", all[1]);
    assert!(all[2].contains("清单变了"), "比对那句必须说得出清单变了：{}", all[2]);
    assert!(all[3].contains("复核"), "复核那两句要说得出这是复核：{}", all[3]);
    // 词表那条（`DESIGN.md`）：档位名叫「配好的位置」，**句子**里说「这两个位置」。
    // 「这一对目录」是内部坐标系（那两个锁键）的说法，操作员在屏上从没见到"一对"这个东西。
    for err in &all {
        assert!(!err.contains("这一对目录"),
            "那句拒绝说的是「这一对目录」，该说「这两个位置」：{err}");
    }
}

/// 命令表的形状（spec §4.1：删除只有「预览 → 确认」这一对入口）。
///
/// 为什么钉源文本而不是钉行为：`generate_handler![...]` 是前端唯一能调到后台的口子，
/// "这张表里没有第三条能删东西的路"这件事，运行时没有任何断言管得住——多写一条命令、
/// 少写一条命令，程序都照样编译。所以这里直接读那张表。
#[test]
fn the_handler_table_exposes_the_mirror_pair_and_nothing_that_deletes_on_its_own() {
    let src = read_src("src/lib.rs");
    for name in ["commands::preview_mirror", "commands::run_mirror"] {
        assert!(src.contains(name), "命令表里少了 {name}：镜像的那条路在界面上就是断的");
    }
    // 表里不许躺着任何"自己就删"的入口（跳过预览与确认的那种）。
    // 判据从"名字里不许有破坏词"升级成两条（裁定 R79 + R80）：
    // ① 只发凭证的名字（含 `preview`）不动手——它一个字节都不删；
    // ② 剩下的含破坏词的名字必须**只凭 token 动手**。
    // 为什么必须升级：旧那句在"删除通路只有一条"的世界里才与意图等价，而 R78 加了第二条
    // （`browser_run`）——照旧字扫，`browser_preview_delete` 一进表就把这条证人红在自己的新命令上。
    // 改名绕过扫描、或往禁词里加例外，都是把钉子拆了（下一个 `browser_purge_now` 会从底下滑过去）。
    // "能不带 token 就改变状态"的那份**封闭名单**在同文件下面那条新证人里，这里不重复一份。
    let commands = read_src("src/commands.rs");
    for banned in ["delete", "purge", "execute", "force", "abort"] {
        for chunk in src.split("commands::").skip(1) {
            let name = chunk.split(',').next().unwrap_or("").trim().to_string();
            if !name.contains(banned) || name.contains("preview") { continue; }
            let after = commands.split(&format!("pub async fn {name}")).nth(1)
                .unwrap_or_else(|| panic!("表里的 {name} 在 commands.rs 里找不到实现"));
            let signature = &after[..after.find('{').unwrap_or(after.len())];
            assert!(signature.contains("token: String"),
                "命令表里的 {name} 含破坏词 {banned} 却不带 token：删除只能走预览→确认");
        }
    }
}

/// R78 的结构证人：命令表里"能动手"的入口只认 token。
///
/// 上面那条禁词扫描钉的是**名字**，这条钉的是**结构**——两种都要：只留一个的话，
/// 将来谁加一个 `browser_delete_now(fs, remote)`，若它的名字恰好不在禁词表里（比如写成
/// `browser_drop`），就既没有禁词也没有 token。
#[test]
fn every_browser_entry_that_can_destroy_something_requires_a_token() {
    let table = read_src("src/lib.rs");
    let commands = read_src("src/commands.rs");
    // 动手的两条都必须伸手要 token：对齐那条与浏览器那条。
    for cmd in ["run_mirror", "browser_run"] {
        let after = commands.split(&format!("pub async fn {cmd}")).nth(1)
            .unwrap_or_else(|| panic!("命令 {cmd} 不在了"));
        let signature = &after[..after.find('{').unwrap_or(after.len())];
        assert!(signature.contains("token: String"), "{cmd} 必须只凭 token 动手：{signature}");
    }
    // 而且不许存在第二个"不带 token 就能自己动手"的入口。判据分三步，缺一步就漏：
    // ① 含 `preview` 的名字**不算动手**（R80：它只发凭证）；
    // ② 能改变东西、又不带 token 的入口是一份**点名**的封闭清单，每条带理由
    //   （与 I4"命令面是封闭集合"同族）；往表里加第三条"动手"的路，必须先来改这份清单并说明凭什么。
    // ③ 清单本身也要被核对：里面那条命令若已经带上了 token，就是清单在骗人。
    //
    // 清单只列**已存在**的命令（裁定 R103）：Task 11 那时只有 `browser_copy`，Task 12 的那两条
    // 随它们自己那批一起加。写进不存在的名字钉不住任何东西——证人会 panic 在
    // "名单说它能不带 token 动手，可它在 commands.rs 里找不到实现"，那是空转不是红。
    //
    // `browser_rmdir` 曾经在第三行，理由是"rc 只让空目录删得掉，够不到任何没核对过的东西"。
    // 操作员那条"所有删除必须先提示"把它请进了凭证路（`browser_preview_rmdir` +
    // `browser_run`）：那个理由说的是**后果**有限，说的是不是**要不要先问一句**——而它是当时
    // 界面上唯一一条点一下就删的路。名单里现在只剩两条非破坏的写。
    let acting_without_token: &[(&str, &str)] = &[
        ("browser_copy", "非破坏：源保留，撞名在写入之前由面板拦下（I2c/F10）"),
        ("browser_mkdir", "只增，且幂等（F14 实测：已存在再建一次还是 200）"),
    ];
    for (name, reason) in acting_without_token {
        let after = commands.split(&format!("pub async fn {name}")).nth(1)
            .unwrap_or_else(|| panic!("名单说 {name} 能不带 token 动手，可它在 commands.rs 里找不到"));
        let signature = &after[..after.find('{').unwrap_or(after.len())];
        assert!(!signature.contains("token: String"), "{name} 已经带 token 了，把它从名单里删掉：{signature}");
        assert!(!reason.is_empty(), "名单里每一条都要有理由：{name}");
    }
    for chunk in table.split("commands::").skip(1) {
        let name = chunk.split(',').next().unwrap_or("").trim().to_string();
        let acts = ["delete", "purge", "remove", "rmdir", "execute", "force", "abort"]
            .iter().any(|w| name.contains(w));
        if !acts || name.contains("preview") || acting_without_token.iter().any(|(n, _)| *n == name) {
            continue;
        }
        let after = commands.split(&format!("pub async fn {name}")).nth(1)
            .unwrap_or_else(|| panic!("表里的 {name} 在 commands.rs 里找不到实现"));
        let signature = &after[..after.find('{').unwrap_or(after.len())];
        assert!(signature.contains("token: String"),
            "{name} 既不领凭证、也不在封闭名单里：动手只能走 browser_run(token)");
    }
}

/// 「不预览直接批」那一步**既不动手也不列举**。
///
/// 为什么值得钉：这一条存在的理由就是"按下确认不该先付一趟两侧全量 LIST"。
/// 将来谁在里面顺手加一次 `listed_doomed` 想让拒绝更早说话，这条不变量就悄悄回来了 ——
/// 而界面上只会表现为"又得等一会儿才动"，行为测试全绿。所以钉的是**它没有调用谁**。
/// 另一半同样要紧：动手那一条仍然只认 token，多一条命令不等于多一条删东西的路。
#[test]
fn the_confirmation_step_lists_nothing_and_deletes_nothing() {
    let src = read_src("src/runner/mod.rs");
    let after = src.split("pub async fn confirm_mirror").nth(1)
        .unwrap_or_else(|| panic!("Runner 里没有 confirm_mirror：那一步在界面上就是断的"));
    // 切到**这个函数自己的收尾花括号**为止：按"下一个 `pub async fn`"切会把下一条命令的
    // 文档注释也圈进来（那里面写着 `transfer(...)`），于是这条证人红在一段说明文字上。
    let body = &after[..after.find("\n    }").unwrap_or(after.len())];
    for banned in ["listed_doomed(", "side_set(", "transfer(", "sync/sync", "prune::plan("] {
        assert!(!body.contains(banned),
            "确认那一步里出现了 {banned}：它要么在列举要么在动手：{body}");
    }
    // 登记的仍然是 `Option<BTreeSet>`，而它的值来自 `peek_seen`（那份**方向敏感**的缓存）：
    // `Some` = 人在「看差异」里逐条看过，`None` = 没人看过、用空集冒充是**另一种意思**
    // （看过且那份是空的）。所以这里钉的是"它仍由 `peek_seen` 交出、仍不是就地算的"，
    // 而不是钉某一个字面拼写 —— 2026-10-09 把它从写死的 `doomed: None` 换成
    // "有看过的那份就用、没有就 `None`"之后，**「没看过」这一档仍然是 `None`**。
    assert!(body.contains("peek_seen("),
        "确认那一步不再从「看差异」那份缓存取基准了：{body}");
    assert!(body.contains("doomed: seen") || body.contains("doomed,"),
        "凭证登记的那一份没接到 peek_seen 的结果上：{body}");
    // 反过来钉住那条不能破的：`None` 那一档必须还在（人没看过名单时不得拿空集冒充）。
    assert!(!body.contains("BTreeSet::new()") && !body.contains("BTreeSet::from(["),
        "「没看过名单」被换成了空集冒充：空集说的是\"看过一份，那份是空的\"：{body}");
    let run = src.split("pub async fn run_mirror").nth(1)
        .unwrap_or_else(|| panic!("Runner 里没有 run_mirror"));
    assert!(run.contains("consume(token)"), "动手那一条不再先吃凭证：{run}");
    let table = read_src("src/lib.rs");
    assert!(table.contains("commands::confirm_mirror"), "命令表里没有 confirm_mirror：{table}");
}

/// 命令签名：**确认那一条必须伸手要 token**，预览那一条不要。
///
/// 这一条钉的是"没有 token 就删不了"这条不变量在命令层也成立：`Runner::run_mirror` 里那次
/// `remove(token)` 只挡住拿到 token 的调用方，而调用方是命令层写的——命令层不收 token，
/// 它就只会传一个空串进去，那条路每次都拒绝，整个对齐功能反而是死的。
/// 两层各自守各自那一段，缺一条就没人守。
#[test]
fn the_mirror_run_command_cannot_be_reached_without_a_confirmation_token() {
    let src = read_src("src/commands.rs");
    let after = src.split("pub async fn run_mirror").nth(1)
        .unwrap_or_else(|| panic!("命令层没有 run_mirror 这个函数"));
    let signature = &after[..after.find('{').unwrap_or(after.len())];
    assert!(signature.contains("token: String"),
        "确认执行那条命令不要 token，等于给界面留了一个跳过预览就删的入口：{signature}");

    let after = src.split("pub async fn preview_mirror").nth(1)
        .unwrap_or_else(|| panic!("命令层没有 preview_mirror 这个函数"));
    let signature = &after[..after.find('{').unwrap_or(after.len())];
    assert!(signature.contains("id: String"), "预览要按任务列清单，得知道是哪个任务：{signature}");
    assert!(!signature.contains("token"), "预览是只读的，它不该要一份确认：{signature}");
}

/// 前端那一头的桥：喊的名字必须就是表里那两个，实删那一条必须把 token 一路带到命令层。
/// 命令名是 Rust 与 TS 之间唯一的契约——拼错一个字母不是"少个功能"，是一次运行时报错，
/// 而这条界面上没有任何测试会替你按一次按钮。
#[test]
fn the_frontend_bridge_calls_the_same_two_commands_by_name() {
    let ts = read_src("../src/api.ts");
    assert!(ts.contains("'preview_mirror'"), "前端没有预览这条路：要删只能从冷点的「开始」走：{ts}");
    assert!(ts.contains("'run_mirror'"), "前端没有确认执行这条路：预览出来的清单没人消费：{ts}");
    let run = ts.split("'run_mirror'").next().unwrap_or("");
    let decl = run.rsplit("export const").next().unwrap_or(run);
    assert!(decl.contains("token"), "实删那一条在前端就不要 token：确认这一步能在界面上被绕过：{decl}");
    assert!(!ts.contains("'run_prune") && !ts.contains("'delete"),
        "前端还留着一个能自己删东西的命令：{ts}");
}

/// 设置面那三条的形状：**能落盘的那一条只认 token**，另外两条不动磁盘。
/// 与对齐那一族同一套判据（扫源文本；这张表多一条少一条程序都照样编译）。
#[test]
fn the_settings_commands_take_a_token_only_for_the_write() {
    let src = read_src("src/commands.rs");
    for cmd in ["settings_current", "settings_plan", "settings_apply"] {
        assert!(src.contains(&format!("pub async fn {cmd}")), "命令层没有 {cmd}");
    }
    let signature = |name: &str| -> String {
        let after = src.split(&format!("pub async fn {name}")).nth(1)
            .unwrap_or_else(|| panic!("命令 {name} 找不到实现"));
        let end = after.find('{').unwrap_or(after.len());
        after[..end].to_string()
    };
    assert!(signature("settings_apply").contains("token: String"),
        "落盘那一条不要凭证，等于给界面留了一个跳过差异就写的入口：{}", signature("settings_apply"));
    for read_only in ["settings_current", "settings_plan"] {
        let s = signature(read_only);
        assert!(!s.contains("token"), "{read_only} 是只读的，它不该要一份确认：{s}");
    }
    // 而且写盘不在命令层：命令层自己不许出现写文件的那一族。
    let body = src.split("pub async fn settings_apply").nth(1).unwrap_or("");
    for needle in ["fs::write(", "File::create(", "fs::rename("] {
        assert!(!body.contains(needle), "命令层自己写了文件（{needle}）：写通路只有一条");
    }
}

/// 设置面读侧的远端名单**只有一个来源**：那一份 `rclone.conf` 自己的段名。
///
/// 这一条是拿真窗口的事故钉的：以前这里把浏览器那一栏的候选当名单递进去，而那些候选是
/// **位置**（`fxa:a`、`fxa:b`），每一段因此"找不到"——远端卡片明明列出来了，脚边却同时挂着
/// 「这一栏列不出来」。两句同时说的根因是"有两个来源"，所以判据落在结构上：不许再借别处。
#[test]
fn the_settings_read_side_has_exactly_one_source_for_remote_names() {
    let src = read_src("src/commands.rs");
    let body = src.split("pub async fn settings_current").nth(1).unwrap_or("")
        .split("\n}").next().unwrap_or("");
    assert!(!body.contains("browser_remotes"),
        "settings_current 又去借浏览器那一栏的候选当远端名单了：{body}");
    assert!(body.contains("edit::current(&files, running)"),
        "读侧的入参不该再有第四份名单：{body}");
    let edit = read_src("src/config/edit.rs");
    assert!(edit.contains("section_names(&lines)"),
        "段名应由那份文件自己报（section_names），否则名字与段可能对不上");
}

/// 命令表里设置面**只有三条**，凭证表挂在 tauri 的管理状态上。
#[test]
fn the_handler_table_exposes_exactly_three_settings_entries() {
    let table = read_src("src/lib.rs");
    for name in ["commands::settings_current", "commands::settings_plan", "commands::settings_apply"] {
        assert!(table.contains(name), "命令表里少了 {name}：这一面在界面上就是断的");
    }
    assert_eq!(table.matches("commands::settings_").count(), 3,
        "表里的设置面条数不是三条：加一条之前先回来看这条证人凭什么拦");
    assert!(table.contains("config::edit::WriteRegistry"),
        "凭证表没挂进 tauri 的管理状态：那三条命令拿不到同一张表");
    for banned in ["settings_reset", "settings_reload", "settings_write", "settings_save_now"] {
        assert!(!table.contains(banned), "命令表里长出了别的路：{banned}");
    }
}

/// 前端那一头：喊的名字必须就是表里那三个，**落盘那一条把 token 一路带到命令层**。
/// 命令名是 Rust 与 TS 之间唯一的契约——拼错一个字母不是"少个功能"，是一次运行时报错，
/// 而这一面没有任何测试会替你按一次保存。
#[test]
fn the_frontend_bridge_calls_the_three_settings_commands_by_name() {
    let ts = read_src("../src/api.ts");
    for name in ["'settings_current'", "'settings_plan'", "'settings_apply'"] {
        assert!(ts.contains(name), "前端没有 {name} 这条路：设置面在界面上就是死的");
    }
    assert_eq!(ts.matches("'settings_").count(), 3,
        "前端的 settings_ 调用不是三条：多一条就是第二条写得进配置的路");
    let after = ts.split("'settings_apply'").next().unwrap_or("");
    let decl = after.rsplit("export const").next().unwrap_or(after);
    assert!(decl.contains("token"), "落盘那一条在前端就不带凭证：{decl}");
    // 配置文件的路径归后台交（R68 那一族：面板可以跑在别的家上），前端不许自己指。
    assert!(!ts.contains("rclone.conf") && !ts.contains("jobs.toml"),
        "前端自己指配置文件：{ts}");
}

/// 形状漂了要红：TS 的字段名必须就是 Rust `#[serde(rename_all = "camelCase")]` 交出来的那些。
/// 线上传的是字段不是类型名，所以钉字段；类型名两边可以不同（TS 侧带前缀防撞名）。
#[test]
fn the_ts_settings_shapes_use_the_same_field_names_as_rust() {
    let ts = read_src("../src/types.ts");
    let want: &[(&str, &[&str])] = &[
        ("JobDraft", &["jobKind", "openDir", "manifest", "src", "dst", "id", "name"]),
        // 任务那一块的两条通路（新增 / 改这条）与远端那四项，token 值都在 TS 里点名：
        // 拼错一个就不会报错，只会让后台收到一个认不出的意图。
        ("ConfigIntent", &["job_add", "job_set", "remove_job", "remote", "rcloneBin",
                           "logDir", "remoteType", "openDir", "jobKind"]),
        ("SettingsPlan", &["token", "path", "subject", "changes", "notes"]),
        ("SettingsView", &["jobsPath", "confPath", "confNote", "defaults", "remotes", "jobs", "running"]),
        ("DefaultsBlock", &["rcloneBin", "rcloneBinExists", "rcloneBinVersion",
                            "logDir", "logDirExists", "logDirNote", "flags", "exclude"]),
        ("SecretState", &["present", "note", "command"]),
    ];
    for (iface, fields) in want {
        let block = ts_shape(&ts, iface).unwrap_or_else(|| panic!("TS 里没有 {iface} 这个形状"));
        assert!(!block.trim().is_empty(), "{iface} 是空的：那它什么也没钉");
        for f in *fields {
            assert!(block.contains(f), "{iface} 里没有字段 {f}（Rust 交的是这个 camelCase 拼法）：{block}");
        }
    }
    // 密钥那一条的反面：TS 侧**不许**出现任何能装值的字段名。
    let secret = ts_shape(&ts, "SecretState").unwrap();
    for banned in ["value", "accessKey", "secretAccessKey"] {
        assert!(!secret.contains(banned), "SecretState 里长出了能装值的字段 {banned}：{secret}");
    }
}

/// 取出一个 TS 形状的声明体：`interface X {…}` 或 `type X = …;`（联合类型那种）。
/// 一段声明到下一个空行为止——**不**按 `}` 收尾，因为联合类型的第一个 `}` 只是第一个成员，
/// 按它截会让 `ConfigIntent` 那类断言悄悄只查到一半（这种"扫到一半就当扫过"是恒真的另一种）。
///
/// 两条自我检查，都来自本机踩过的形状：
/// - 工作树是 CRLF（`core.autocrlf=true`），按 `"\n\n"` 找边界会一个都找不到 ⇒ 整份文件
///   被当成一个形状，任何字段名都"在里面找到了"。所以先归一化，再断言**截断了**。
/// - 段里不许再出现第二个 `export`：出现了就是没截断。
fn ts_shape(ts: &str, name: &str) -> Option<String> {
    let ts = ts.replace("\r\n", "\n");
    for anchor in [format!("interface {name}"), format!("type {name} ")] {
        let Some(at) = ts.find(&anchor) else { continue };
        let rest = &ts[at..];
        let end = rest.find("\n\n").unwrap_or(rest.len());
        let block = rest[..end].to_string();
        let tail = block.strip_prefix(anchor.as_str()).unwrap_or(&block);
        assert!(!tail.contains("export "),
            "{name} 的形状没在边界处截断——这个证人在扫整份文件，它绿了不算证据：{block}");
        assert!(end < rest.len(), "{name} 那段之后没有空行：文件形状变了，这段扫的是文件尾巴");
        return Some(block);
    }
    None
}

/// 「打开 rclone 的配置目录」那颗按钮：路径必须由后台**问出来**，不能由界面交进来。
///
/// 交进来就是"能喊它开任意一个目录"的那条通路 —— 这一颗只该开一个地方，
/// 而那个地方是 rclone 自己说的那份 conf 所在的层（`conf_path_of` + `config_folder_of`）。
#[test]
fn the_config_folder_command_asks_rclone_instead_of_trusting_the_screen() {
    let reg = read_src("src/lib.rs");
    assert!(reg.contains("open_config_folder"),
        "命令没注册进 invoke_handler：前端喊它也接不到");

    let src = read_src("src/commands.rs");
    let body = src.split("pub async fn open_config_folder").nth(1).unwrap_or("")
        .split("\n}").next().unwrap_or("");
    assert!(!body.is_empty(), "命令还没有：{src}");
    assert!(body.contains("conf_path_of("),
        "路径不是问 rclone 要的（空着不开、猜的开错地方）：{body}");
    assert!(body.contains("config_folder_of("),
        "取所在目录这一步没走那条纯函数：{body}");
    for handed in ["path: String", "dir: String", "folder: String"] {
        assert!(!body.contains(handed),
            "界面把路径交进来 = 让它指哪开哪，这一颗没这个权限：{body}");
    }
    // 开出去的那一步只看"有没有拉起来"，不看退出码（explorer 成功也返回 1，一看就把成功读成失败）。
    // 钉的是这一步的形状，不是它用哪个程序名 —— 注释里也写着 explorer，扫"有没有这个词"会绿得没道理。
    assert!(body.contains("is_dir()"),
        "没先确认那一层真的在：把一个不在的路径交给 explorer，开出来的是'文档'：{body}");
    assert!(body.contains(".spawn()"),
        "没有走那条'拉起就算、不等它退出'的通路：{body}");
    assert!(!body.contains(".status()") && !body.contains(".wait()"),
        "在那里等退出码：成功的那一次会被读成失败：{body}");

    // 前端那一头：走 api.ts 一条**具名出口**，并由 `Panel.tsx` 接进 `act('打开目录', …)`。
    // 少这一环，那颗按钮点了就是静默（或直接 `invoke`），失败时没人说话。
    // 按"这一行是 export const"取，而不是全文找那个命令名 —— 注释掉的那一行也算找到过。
    let ts = read_src("../src/api.ts");
    let bridge = ts.lines()
        .find(|l| l.trim_start().starts_with("export const openConfigFolder"))
        .unwrap_or("");
    assert!(bridge.contains("'open_config_folder'"),
        "前端桥里少了那条具名出口（按钮喊不到东西）：{ts}");
    assert!(!bridge.contains(", {"), "调用带了参数（界面交路径 = 让它指哪开哪）：{bridge}");
    let panel = read_src("../src/Panel.tsx");
    assert!(panel.contains("openConfigFolder"), "那条路没接进面板：{panel}");
}

/// 「换一对位置」那两条命令：表里有，壳里**只转发不判断**。
///
/// 为什么还要钉这一遍（`draft_pairs.rs` 已经钉过 runner 那一侧的行为）：立卡与收卡的判据
/// （哪一格缺、正在跑就不许收、那几句拒绝）全在 `Runner` 里说；壳里若再判一遍，同一件事就有
/// 两处话术、两处会各自漂——这个形状在本仓栽过一次（`run_job` 头上那段就是那次留下的）。
#[test]
fn the_handler_table_exposes_the_two_draft_entries_and_the_shell_only_forwards() {
    let table = read_src("src/lib.rs");
    for name in ["commands::pair_draft", "commands::drop_draft"] {
        assert!(table.contains(name), "命令表里少了 {name}：立卡与收卡在界面上就是断的");
    }
    let commands = read_src("src/commands.rs");
    for (name, call) in [("pair_draft", "runner.pair_draft("), ("drop_draft", "runner.drop_draft(")] {
        let after = commands.split(&format!("pub async fn {name}")).nth(1)
            .unwrap_or_else(|| panic!("表里的 {name} 在 commands.rs 里找不到实现"));
        let body = &after[..after.find("\n}").unwrap_or(after.len())];
        assert!(body.contains(call), "{name} 没把这件事交给 runner：{body}");
        for banned in ["if ", "Invariant", "format!(", "draft_seq", "check_positions"] {
            assert!(!body.contains(banned), "{name} 的壳里长出了判断（{banned}）：{body}");
        }
        // 这两条都不碰任何一棵树：不动对象、不删东西，所以也不该伸手要凭证。
        // 「带 token 才是动手」那条判据（上面两条证人）覆盖的是删除通路，把它们套到这里
        // 等于给一个只改内存卡片的入口配一份它用不上的确认。
        assert!(!body.contains("token"), "{name} 不该带凭证：它一个对象都不动：{body}");
        assert!(!body.contains("dry_run"), "{name} 不是传输入口：{body}");
    }
    // 删除那一对的形状一字未改：立一张卡不新增第三条能删的路（临时格的实删走的还是
    // `preview_mirror` → `run_mirror`，R95 靠的就是"同一对入口、同一个登记处"）。
    assert!(!table.contains("commands::run_draft"),
        "表里长出只给临时格动手的入口：删除只有预览→确认那一对");
    assert!(!table.contains("commands::preview_draft"),
        "表里长出只给临时格发凭证的入口：预览那一条本来就吃 id");

    // 收卡那一条要**等到底**并把错误原样交出去。少了 `.await`，这句照样编译 —— 它只是造出一个
    // 没人 poll 的 Future（一条警告），而界面上那颗「删掉这张临时卡」从此静默不动，卡留在原处。
    // 这一族"看起来成功、其实什么都没做"的口子在本仓按事故记过账，所以钉在这里而不是只靠真窗口。
    let after = commands.split("pub async fn drop_draft").nth(1).unwrap_or("");
    let body = &after[..after.find("\n}").unwrap_or(after.len())];
    assert!(body.contains(".await?"), "收卡那条没有等到底（错误也就没交出去）：{body}");
}

/// `get_jobs` 现在交的是**配置 + 这一趟的临时卡**，那句话得写在它自己头上。
/// 钉这一句的理由不是文档癖：任何"左列 = 那份文件"的隐含假设都会在这条命令上失效（spec §8），
/// 而失效的那一次看起来像"界面上多了一条没配过的任务"，没人会说得出为什么。
#[test]
fn get_jobs_says_its_output_is_no_longer_just_the_file() {
    let commands = read_src("src/commands.rs");
    let at = commands.find("pub async fn get_jobs").expect("get_jobs 不在了");
    // 按**行**取那一段，不按字节：这份文件里全是中文，`before.len()-900` 那个下标
    // 落进一个三字节汉字的中间就是 panic（拿变异枪打 D3 时才撞出来——偏移一挪就炸，
    // 炸的还是证人自己，不是它要钉的那件事）。
    let window = commands[..at].lines().rev().take(25).collect::<Vec<_>>().join("
");
    assert!(window.contains("临时"), "get_jobs 没说明它现在也交临时卡：{window}");
    // 设置面读的是那一份文件，不读这张合成表（`SettingsView` 那条路）：
    // 两处都读合成表的话，"存成常用"就会把一条没落盘的临时任务当成已经存过。
    // 判据写成 `.jobs()` 而不是 `runner.jobs()`：合成表从哪一头读来都算读错（`state.runner()?.jobs()`
    // 是更顺手的那种写法），钉死一种拼法就是给另一种留门。
    let settings = commands.split("pub async fn settings_current").nth(1).unwrap_or("")
        .split("\n}").next().unwrap_or("");
    assert!(!settings.contains(".jobs()"),
        "设置面改读合成表了：它读的是那份文件，不是界面上那些卡：{settings}");
}

/// 前端那一头：喊的名字必须就是表里那两个。命令名是 Rust 与 TS 之间唯一的契约——
/// 拼错一个字母不是"少一个功能"，是一次运行时报错，而立卡这条路没有任何测试会替你按一次。
#[test]
fn the_frontend_bridge_calls_the_two_draft_commands_by_name() {
    let ts = read_src("../src/api.ts");
    for (export, name) in [("pairDraft", "'pair_draft'"), ("dropDraft", "'drop_draft'")] {
        let line = ts.lines()
            .find(|l| l.trim_start().starts_with(&format!("export const {export}")))
            .unwrap_or_else(|| panic!("前端桥里少了具名出口 {export}：那颗按钮喊不到东西"));
        assert!(line.contains(name), "{export} 喊的名字不是表里那一个：{line}");
    }
    // 参数按位置交回，返回的是那张卡的 pair_key（后台算的那把尺子，前端不自己拼）。
    let draft = ts.lines().find(|l| l.trim_start().starts_with("export const pairDraft")).unwrap_or("");
    assert!(draft.contains("src") && draft.contains("dst"), "两条位置都得交回去：{draft}");
    assert!(draft.contains("<string>"), "pair_key 在前端被当成别的形状：{draft}");
}
