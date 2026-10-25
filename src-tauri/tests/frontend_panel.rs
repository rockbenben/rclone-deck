//! 面板**管道**的证人（不是话术）。口径与 `frontend_copy.rs` / `frontend_mirror.rs` 相同：
//! 本仓没有 JS 测试运行器，所以这些证人读源文本，真窗口里的那一份在验收任务里量。
//!
//! 这里钉的是一条会咬人的时序：轮询的 catch 每秒跑一次，所以要有一道"别说第二遍"的门。

mod common;

use common::{count_in, read, strip_comments, surface_text};

/// 那道门以前是个**布尔**，只在一次轮询成功之后重新武装。同一段故障里原因从 A 变成 B 时
/// （还是读不到，但错换了），第二句永远说不出口，操作员对着的还是第一句——而那一句已经
/// 不代表当下。这条被记成 P1 写在 `docs/superpowers/evidence/build-notes.md`。
///
/// 现在门里存的是**上一次那句原因**：同因不刷屏，换因要说。
/// 钉的是形状而不是某句拼写：布尔那一族写法（`useRef(false)`、把门`= true`）一回来就红，
/// 而话术怎么改、变量怎么改名都不影响这条。
#[test]
fn a_changed_poll_failure_reason_gets_spoken_again() {
    let t = read("src/Panel.tsx");
    // 门必须拿"这一次的原因"和"上一次的原因"比，而不是问"报过没有"。
    assert!(t.contains("!== lastPollError.current"),
        "轮询失败的门没有和上一次的原因比较：换了的第二句又说不出口");
    // 反面：布尔门那一族不许回来。这两条不是装饰——把实现改回布尔就是同时踩中这两条。
    for banned in ["useRef(false)", "pollErrorReported", "lastPollError.current = true"] {
        assert!(!t.contains(banned), "面板里又出现了布尔式的门（{banned}）：同一段故障里换原因就静默");
    }
    // 成功那一轮必须把原因清空，否则"恢复过一次、又坏了"也会被当成同因吞掉。
    assert!(t.contains("lastPollError.current = null"),
        "成功的一轮没有把记住的原因清掉：恢复后再坏就没人说了");
}

/// 卡上任何一颗按钮都要**先选中那张卡**再发请求。
///
/// 右栏读的是 `selected`：不选中的话，"开始传输"就在看不见的地方发生了 —— 操作员只看得见
/// 所有按钮一起变灰（`busy` 是全局的），右栏还在讲上一张卡的事。报的就是这个，
/// 而当时那一整遍交互验收里没有一步是"站在别的卡上按这一张的按钮"。
///
/// 判据是结构上的：那个列表元素里不许再出现直接调 `act(` 的写法，四颗动手的按钮都走
/// `cardAct`；而 `cardAct` 自己得真的先选中 —— 只钉名字的话，把它改成空壳也是绿的。
#[test]
fn every_card_action_selects_its_own_card_first() {
    let t = read("src/Panel.tsx");
    let from = t.find("<JobListView").expect("Panel 里找不到那个任务列表元素");
    let to = t[from..].find("/>").map(|i| from + i).expect("那个元素没有收尾");
    let el = &t[from..to];
    assert!(!el.contains("void act("),
        "卡上又有一颗按钮绕过「先选中」直接发请求：{el}");
    for arm in ["onTrialRun=", "onStart=", "onGate=", "onOpenDir="] {
        let i = el.find(arm).unwrap_or_else(|| panic!("那一颗按钮不在了：{arm}"));
        let rest = &el[i..];
        // 取到下一个同级 prop 之前（多行的 `onGate` 也吃得下：它内层缩进更深）。
        let end = rest[1..].find("\n              on").map(|j| j + 1).unwrap_or(rest.len());
        let body = &rest[..end];
        // 「先选中」有两条合法的路：`cardAct`，以及对账那一条自己的 `runGate`
        // （它除了选中还要把右列翻到装报告的那一段，见下面那条证人）。
        // 认这两条**而不是认任意一个函数名**：下面逐条钉这两个定义里都有 `setSelectedId(id)`。
        assert!(body.contains("cardAct(") || body.contains("runGate"),
            "{arm} 那一支没有先选中那张卡：{body}");
    }
    let def = t.find("const cardAct").expect("找不到 cardAct 的定义");
    // 按 ASCII 锚点切，不按字节数：定义上面那段中文注释里随便一个字节偏移都会切在多字节中间。
    let rest = &t[def..];
    let end = rest.find("}, [act]);").map(|i| i + "}, [act]);".len()).unwrap_or(rest.len());
    let body = &rest[..end];
    assert!(body.contains("setSelectedId(id)"), "cardAct 没有先选中：{body}");
    assert!(body.contains("act(label, fn)"), "cardAct 没有把请求交给那条统一的入口：{body}");
    // 第二条路自己也要钉住：`runGate` 里必须有那一句选中，否则上面那条"认函数名"就是空账。
    let gate_def = t.find("const runGate").expect("找不到 runGate 的定义");
    let grest = &t[gate_def..];
    let gend = grest.find("}, [act]);").map(|i| i + "}, [act]);".len()).unwrap_or(grest.len());
    let gbody = &grest[..gend];
    assert!(gbody.contains("setSelectedId(id)"), "runGate 没有先选中那张卡：{gbody}");
}

/// 轮询只给**选中的那一个**任务读日志（状态仍然全员：卡面那颗灰不灰靠它）。
/// 旧形状是每一轮对每个任务发两个请求，其中 `tailLog` 那一路真的开文件读 8 KiB
/// （`TaskLog::tail`），而界面只读 `logs[selectedId]` 那一份——N 份文本里 N−1 份一次都没被看过，
/// 每秒付一次这个钱。
///
/// 钉的是范围，不是行数：`tailLog` 必须存在、必须在 `jobs.map` 那个循环**外面**、
/// 而且这里的依赖里必须有 `selectedId`（切卡立刻补读靠的就是这条依赖重建 `refresh`）。
#[test]
fn the_poll_tails_only_the_selected_task_log() {
    let t = strip_comments(&read("src/Panel.tsx"));
    let from = t.find("const refresh").expect("找不到那条轮询");
    let body = &t[from..];
    let end = body.find("}, [say").unwrap_or(body.len());
    let scope = &body[..end];
    assert_eq!(count_in(scope, "api.tailLog("), 1, "尾巴那一路有不止一个调用点：{scope}");
    let loop_at = scope.find("j.map(async (job) => {").expect("找不到那个每任务循环");
    let loop_body = &scope[loop_at..];
    let loop_end = loop_body.find("}));").unwrap_or(loop_body.len());
    assert!(!loop_body[..loop_end].contains("tailLog"),
        "每一轮又在给每个任务开文件读日志（只有选中那一份会被看见）：{}", &loop_body[..loop_end]);
    assert!(scope.contains("selectedId"), "尾巴不再按选中范围读：{scope}");
    assert!(t.contains("}, [say, selectedId]"),
        "refresh 不随选中重建：切到一张卡上要等下一轮才看得到它的日志：{t}");
    // 状态那一族请求仍然全员（这条改动不许顺手把它也收掉——卡面靠它）。
    assert!(scope.contains("api.jobState(job.id)"), "卡面那三份状态读数被顺手裁掉了：{scope}");
}

/// 日志面必须说得出"这一屏是哪个文件的尾巴"。
/// 日志**每次动作新开一个带时间戳的文件**（`TaskLog::create` 那个 `{job_id}-{时间}.log`），
/// 所以停在"完整内容在日志文件里"就等于没指——而这条命令为了读尾巴本来已经把路径拿到了，
/// 旧写法只用一下又扔掉。
#[test]
fn the_log_face_says_which_file_it_is_the_tail_of() {
    let cmd = read("src-tauri/src/commands.rs");
    let from = cmd.find("pub async fn tail_log").expect("找不到 tail_log");
    let body = &cmd[from..];
    let end = body.find("\n}").unwrap_or(body.len());
    assert!(body[..end].contains("Result<LogTail"),
        "尾巴又只剩纯文本了：路径在这一行里本来就已经拿到：{body}");
    assert!(body[..end].contains("log_path"), "那份路径没有作为数据走出去：{body}");

    let rail = strip_comments(&read("src/components/JobContextPanel.tsx"));
    assert!(rail.contains("log.logPath"), "日志面没把那个文件摆出来：{rail}");
    // 复制入口 + 中段省略 + 原生 title（与顶栏那一格同一个形状；`DESIGN.md`：路径不是密码）。
    assert!(rail.contains("copyable={{ text: log.logPath }}"), "那个路径没有可复制的入口：{rail}");
    assert!(rail.contains("log-ellipsis"), "那串全屏最长的路径没走省略，而是被塞成一整行：{rail}");
    // 界面不许自己拼日志文件名（那是坐标系的第二份口径，而命名规则在 `TaskLog::create` 里）。
    // 扫的是**会摆上界面的那部分**（`surface_text`）：注释里举那个文件名是给人看的，不该红。
    let shown = surface_text(&rail);
    assert!(!shown.contains(".log"), "日志面在界面里拼文件名：{shown}");
}

/// 跑完对账要把右列翻到**装它结果的那一段**。
/// 四段以前只有盘点那一段会自己翻（`openInventory` 里那一句 `setRailTab('diff')`），
/// 对账的两个入口都不翻：报告写进 `gate[id]` 之后屏幕上一个字都不变，而人刚点的那一句结论
/// 在另一段里。这一条钉的就是"那两个入口都记得翻面"。
/// 试跑与传输**不该**翻（它们的读数就住在进度那一段），所以这里同时钉翻面只出现一次。
#[test]
fn running_the_check_lands_on_the_face_that_holds_its_report() {
    let t = strip_comments(&read("src/Panel.tsx"));
    assert!(t.contains("setRailTab('check')"),
        "跑完对账没有把右列翻到「对账」那一段（报告掉在看不见的第二段里）：{t}");
    // 一条动作一处实现：`api.runGate` 只许出现在 `runGate` 那一个定义里。
    // 两处各写一份闭包，翻面这种细节就会只补上一处（以前正是这样）。
    assert_eq!(count_in(&t, "api.runGate("), 1, "检查有两个调用点，那两个入口会各自漂：{t}");
    assert_eq!(count_in(&t, "setRailTab('check')"), 1,
        "翻面那句出现了不止一处：一条动作两处实现：{t}");
}

/// 面板开在哪一面、右列停在哪一段 —— 这两件是"你上次在看什么"，重开该回来
/// （操作员那句「我常用设置怎么没保存」里的那一条）。
/// 记的**只有这两件**：模式与方向那两格仍然不许跨重开 —— 那条安全规则由
/// `frontend_pairs.rs::the_mode_choice_never_outlives_the_session` 钉着，这里钉的是
/// "别把它顺手扩到这一层来"，所以键的名单里出现 mode/slot 就是红。
#[test]
fn the_face_and_the_rail_tab_survive_a_restart_and_only_those_two() {
    let prefs = strip_comments(&read("src/prefs.ts"));
    assert!(prefs.contains("deck.view") && prefs.contains("deck.tab"),
        "面与段没走持久层：{prefs}");
    for banned in ["deck.mode", "deck.slot", "deck.pair"] {
        assert!(!prefs.contains(banned), "{banned} 进来了：那一格跨重开记住 = 第一次点击可能落在最重那一档");
    }
    let panel = strip_comments(&read("src/Panel.tsx"));
    let tight = panel.split_whitespace().collect::<Vec<_>>().join("");
    assert_eq!(count_in(&tight, "useStored("), 2,
        "面与段不是各接一处持久层（数到 {} 处）：{tight}", count_in(&tight, "useStored("));
    // 旧的写死初值不许留着：留着就是"存的那一份根本没人读"。
    assert!(!tight.contains("useState<View>('pairs')"), "那一面还写死从配对开始：持久层是摆设");
    assert!(!tight.contains("useState<Tab>('progress')"), "右列那段还写死从进度开始：持久层是摆设");
}

/// 传输 / 试跑 / 对齐跑完之后要说一声，但**不翻右列**。
///
/// 这三格的结果住在「进度」；人从「看差异」按「开始」时，如果既没翻面也没有提示，
/// 屏幕上会一个字都不变。既有判据（`running_the_check_lands_on_the_face_that_holds_its_report`）
/// 钉着试跑/传输不翻，所以这里用一条会自己消失的成功提示补上反馈，而不是改那条口径。
#[test]
fn a_finished_transfer_says_so_without_moving_the_rail() {
    let t = strip_comments(&read("src/Panel.tsx"));
    assert!(t.contains("notifyDone("), "跑完没有一声确认：{t}");
    // `runJob` / `runMirror` 的 rclone 失败是 **resolved 的 `Failed`**，不是 reject；
    // 完成提示必须读终态，不能只看 `act` 的 promise 有没有落地。
    assert!(t.contains("if (r.ok) done?.(r.value)"), "完成提示没有区分成功/失败：{t}");
    assert!(t.contains("if (isDone(value) && value.dry_run) notifyDone('试跑', true)"),
        "试跑完成提示没读 Done + dry_run：{t}");
    assert!(t.contains("if (isDone(value) && !value.dry_run) notifyDone('传输')"),
        "传输完成提示没读 Done：{t}");
    assert!(t.contains("if (r.ok && isDone(r.value)) notifyDone('对齐')"),
        "对齐完成提示没读 Done：{t}");
    assert!(t.contains("finished = await api.runMirror") && t.contains("return finished;"),
        "对齐把 runMirror 的终态丢了，`.then` 永远读不到 Done：{t}");
    // 口径不变：不许为了这条提示顺手把右列翻走。
    assert_eq!(count_in(&t, "setRailTab('progress')"), 0,
        "试跑/传输开始翻右列了（那条口径另有用例钉着，别在这里改）：{t}");
}
