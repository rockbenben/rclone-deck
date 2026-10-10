//! 对齐相关界面话术的证人（裁定 R60）。口径与 `frontend_copy.rs` 相同：本仓没有 JS 测试运行器，
//! 所以这些证人读**源文本**；真窗口里的那一份在验收任务里量。
//!
//! 这里钉的不是措辞好不好看，是"界面上同时出现两句互相打脸的话"这一族：
//! 「试跑完成，没有改动任何文件」与「这次对齐删除了 N 个文件」曾经在同一屏上并存，
//! 因为删除那一档没查 `dry_run`——而拒绝实删的那句话术，正好把人往「先试跑」指。

mod common;

use common::{count_in, read, strip_comments, surface_text};

#[test]
fn a_trial_run_never_says_it_deleted_something() {
    let t = read("src/components/JobDetailPanel.tsx");
    assert!(t.contains("dryRun"), "详情面板没有区分试跑与实跑");
    // 实删那一档必须被 !dryRun 限定，否则试跑的 deletes 计数会被说成"删除了 N 个"
    assert!(t.contains("!dryRun && stats.deletes > 0 && kind === 'sync'"),
        "镜像的实删话术没被 dry_run 限定住");
    // 试跑那一档必须单独存在，并且说的是"会删"而不是"删了"
    assert!(t.contains("dryRun && kind === 'sync'"), "试跑的镜像读数没有自己那一档");
    assert!(t.contains("会删掉") && t.contains("一个都没真删"),
        "试跑的话术必须是将来式：{t}");
}

#[test]
fn the_trial_run_count_is_presented_as_an_independent_reading() {
    // 试跑报出的 `deletes` 是 rclone 自己算的，与面板的集合差互相独立——
    // 这是 spec §4.1 留着的那条交叉核对，界面上要看得出这是两个来源，不是一个数的复读。
    let t = read("src/components/JobDetailPanel.tsx");
    assert!(t.contains("rclone 自己算的"), "没说清这个数字来自哪一侧：{t}");
    assert!(t.contains("应该相等"), "没有把'两个数该相等'这件可核对的事说出来");
}

/// 钉到**具体那一档**的源码区间：从 `const mirrorPrimary` 那一句到它自己的 `];`。
/// 锚点用声明名而不是某串 JSX —— 任务行换过好几次形状（③片把 `List` 换成 `Listy`、
/// 三个 kind 各抽成一档、后来又换成"一张卡一对位置 + 两个选择框"），
/// JSX 那串每次都没了，而这条证人要钉的东西一条都没变。
/// 为什么不能拿整个文件判"没有 `onStart`"：复制那一档用它是合法的（冷点即跑就是设计），
/// 按整文件判就成了一条永远为真的断言——把那行代码加回对齐那一格也不会红。
fn mirror_arm(t: &str) -> &str {
    const START: &str = "const mirrorPrimary";
    let from = t
        .find(START)
        .unwrap_or_else(|| panic!("界面里找不到镜像那一档（{START}）：卡面换了形状就该来看这条证人"));
    let rest = &t[from..];
    let to = rest.find("];").unwrap_or(rest.len());
    &rest[..to]
}

#[test]
fn the_confirm_layer_lists_names_not_just_a_count() {
    let t = strip_comments(&read("src/components/MirrorConfirm.tsx"));
    assert!(t.contains("preview.doomed"), "确认层没把逐条名字渲染出来");
    // 每一个名字都要在这层里**够得着**：分页，而不是"截断 + 其余去日志"。
    // 后者是这一仓点名不要的半截操作（`LayerConfirm.tsx` 头上那一段就是这么写的），
    // 而它以前恰好住在这一层——所以这一条判据当时钉的是反面形状。换掉它的理由在那一层里：
    // 截断之后只剩一个计数，等于让人对着数字点头；而把人指去日志那一段时，模态正盖在上面。
    assert!(t.contains("pagination={{ pageSize: LIST_BUDGET"),
        "清单不是分页列全的：截到前若干行、剩下的把人指去别处，就是让人对着数字点头");
    assert!(!t.contains(".slice(0, LIST_BUDGET)"),
        "又回到「只列前 N 个」那一半：剩下的名字在这层里够不着");
    assert!(t.contains("LIST_BUDGET = 20"), "钉的是预算值 20 与它确实被使用，不是钉某个语法形状");
    assert!(t.contains("共") && t.contains("每页"),
        "超出那一页时必须说出总数与每页几个（那两个数是核对用的，不是装饰）");
    assert!(!t.contains("其余在这个任务日志"), "又把剩下的名字送到模态后面那一面去了");
    assert!(t.contains("重新列一遍"), "要说清确认后还会复核，否则这句话是空头承诺");
}

/// 钉住「让两边对齐」这条拒绝的出路：拒绝与清单都只在确认层里渲染，所以这颗按钮必须
/// 走开层那一条路（`openMirror`），不能裸调只落状态的 `doPreview`——层没打开时，
/// 拒绝落进一个没挂载的组件里，等于没有拒绝。
///
/// 这条证人不是空的：把那一行的 `onPreviewMirror={openMirror}` 换回 `onPreviewMirror={doPreview}`，
/// 第一条断言立刻红；把 `openMirror` 里开层那一步（`setPreviewFor(id)`）拿掉、把
/// `error={previewError}` 从传给层的 props 里删掉、或把层里渲染拒绝的那一档摘掉，
/// 分别红第二、三、四条。四条各钉链路一环，少任何一环拒绝就到不了眼前。
#[test]
fn the_preview_button_opens_the_layer_that_shows_the_refusal() {
    let panel = read("src/Panel.tsx");
    assert!(panel.contains("onPreviewMirror={openMirror}"),
        "镜像行的「让两边对齐」没接在开层处理器上：拒绝会落进没挂载的确认层里，没人看得见");
    // 开层处理器必须真的设置 `previewFor`——确认层就挂在这个状态上，不设置它就没有地方渲染。
    let from = panel.find("const openMirror")
        .expect("Panel.tsx 里找不到 openMirror：开层的路换了名字就来看这条证人");
    let body = &panel[from..from + panel[from..].find("}, [").unwrap_or(panel.len() - from)];
    assert!(body.contains("setPreviewFor(id)"), "openMirror 没有把确认层打开：{body}");
    // 拒绝状态要传得进层，否则层里那一档永远拿到 null。
    assert!(panel.contains("error={previewError}"), "previewError 没有传进确认层");
    // 层里必须真有把拒绝说出来的那一档，且描述就是那句拒绝本身。
    // 错误话术分两层之后那一份是 `{ short, detail }`，渲染走 `NoteLine`（摘要 + 「为什么？」）：
    // 只钉"渲染了 error"的话，把 detail 那半段丢掉也是绿的，所以钉到渲染表达式本身。
    let confirm = read("src/components/MirrorConfirm.tsx");
    assert!(confirm.contains("<Alert type=\"error\"") && confirm.contains("description={<NoteLine note={error} />}"),
        "确认层里没有渲染拒绝的地方，或者那一份没走两段的渲染：清单与拒绝必须出自同一层");
}

#[test]
fn the_confirm_copy_states_the_premise_for_each_direction() {
    let t = read("src/components/MirrorConfirm.tsx");
    assert!(t.contains("本地是全集"), "上传镜像的前提话术丢了");
    assert!(t.contains("云端是全集"), "下载镜像的前提话术丢了");
    // 两句必须**分别**挂在方向上：合成一句就只有一个方向说真话，而错的有一句要让人照着点删除。
    assert!(t.contains("localWins(job)"), "前提不是按方向挑出来的：{t}");
    assert!(t.contains("空目录"), "sync 还删空目录，清单是文件级的——这句不许少");
    // 那颗要说出后果，而且说的是后台那两个数（"我知道"那层复述去掉了：全程只问一次）。
    assert!(t.contains("删这 ${preview.doomedCount} 个"), "确认按钮没带着后台那个数：{t}");
    assert!(t.contains("假设值"), "比例阈值是假设值，不许界面把它说成量出来的");
}

#[test]
fn a_mirror_row_cannot_be_started_cold() {
    let t = read("src/components/JobListView.tsx");
    let arm = mirror_arm(&t);
    assert!(arm.contains("onPreviewMirror"), "镜像行没有预览入口");
    assert!(!arm.contains("onStart"), "镜像那一档还在用冷点的 onStart：删除可以不预览就跑");
    // 卡上不放红色按钮：一个按下去什么都不删的 danger 按钮，练的是"红色可以随便点"。
    // 批准删除的那个红色按钮在确认层的页脚，所以下面那半截把它钉在那里。
    assert!(!arm.contains("danger"), "镜像那一格上有红色按钮：批准的按钮该只在确认层里");
    let confirm = read("src/components/MirrorConfirm.tsx");
    let footer_start = confirm.find("footer=").expect("确认层里没有 footer 数组：按钮换了位置就来看这条证人");
    let footer = &confirm[footer_start..];
    assert!(footer.contains("danger"), "确认层页脚没有 danger：批准删除的那个按钮要看得出后果");
    // 反面：复制那一档照样能冷点开始。少了这一句，`!arm.contains` 那条会因为整条 onStart
    // 从界面上消失而一起变绿——那是另一种"什么都删不了"的错。
    assert!(t.contains("props.onStart"), "复制行也不能冷点开始了，这不对：{t}");
}

/// 一张卡是一对位置、方向与模式是卡上两格选择之后，确认层必须自己把**那两条完整位置**、
/// 以及"删的是哪一侧"摆回眼前 —— 那是按红色那颗之前最后一处看得见它们的地方。
/// （旧形状不需要这一句：一行就是一条任务，`src → dst` 那行字就在脚下。）
/// 判据钉的是**拼装顺序**：修剪的对象取 `job.dst`、全集取 `job.src`（对齐的 src 是全集），
/// 而不是界面再拿 `localWins` 判一次"哪侧是本地所以删哪侧"——那一判就是第二份口径。
#[test]
fn the_layer_restates_both_positions_and_which_side_gets_trimmed() {
    let t = strip_comments(&read("src/components/MirrorConfirm.tsx"));
    let s = surface_text(&t);
    assert!(t.contains("job.src") && t.contains("job.dst"), "层里没把那两条完整位置摆出来：{t}");
    assert!(s.contains("把 ${job.dst} 修剪成 ${job.src} 那样"),
        "那一句的顺序不是「删 dst 多出来的」（镜像里 src 才是全集）：{s}");
    assert!(s.contains("多出来的删掉") && s.contains("缺的补上"),
        "那一句只说了删、没说这一趟同时补：{s}");
}

/// 详情面板只**并列摆出**这两个数，不在 TypeScript 里算判定（裁定 R66）。
///
/// 为什么这里不许出现 `previewed !== stats.deletes` 那样的比较：不一致的那一次在 Rust 里
/// 已经被 `transfer_invariant_violation` 变成了 `TaskState::Failed`，一个 `Done` 里永远
/// 不可能带着不相等的两个数——写出来是一段永不执行的代码，还把"判定作为数据走出去"这条
/// 规矩破掉了（同一类问题见 `src/types.ts` 里 `verdict` 的注释）。
/// **这一段不是待补的 TODO，别把比较加回来。**
#[test]
fn the_detail_panel_shows_both_numbers_and_derives_no_verdict() {
    let t = read("src/components/JobDetailPanel.tsx");
    assert!(t.contains("previewed"), "预览数没上界面：TaskState.done.previewed 是唯一的事后凭证");
    assert!(t.contains("stats.deletes"), "实际删除的数没上界面");
    assert!(t.contains("核对"), "要说清这两个数是拿来互相核对的");
    // 判定由 Rust 返回的 state 说；这里连"比一下"都不许出现。
    // 扫的是**去掉空白之后**的文本：`previewed!==x`、`previewed\n  == x`、`previewed != x`
    // 这些写法在源码里长得各不相同，而规矩只有一条。逐字符列清单必漏，压平一次就全拦住。
    // 算术也算拦住：`const d = previewed - stats.deletes; if (d !== 0)` 比的是差值而不是那两个数，
    // 破的是同一条规矩（判定在前端重算），却躲得过只盯比较符的扫描。
    // 说清这条证人**拦不住**什么：先把两个数搬进别的变量名、再在别处比，文本扫描看不见——
    // 真正兜底的是 Rust 的 `transfer_invariant_violation`（不等就变 Failed），这条只是围栏。
    let tight = t.split_whitespace().collect::<Vec<_>>().join("");
    for banned in [
        "previewed!=", "previewed==", "previewed>", "previewed<",
        "deletes!=", "deletes==",
        "previewed-", "-previewed", "previewed+", "previewed*", "previewed/",
        "deletes-", "-deletes", "Math.abs",
    ] {
        assert!(!tight.contains(banned), "界面在重算后台已经给出的判定（{banned}）：那个比较永不成立");
    }
    assert!(!t.contains("listed"), "界面永远不许出现 listed");
}

/// 吃过一次拒绝的那份确认不许再亮着——与浏览器那一层同一个形状。
///
/// 后台那份授权是**取走即消费**，而且排在拿锁、复核、逐条比对**之前**
/// （`Runner::run_mirror` 里那句"授权先吃掉"：拿不到锁也不能把授权留着，否则等锁的人
/// 可以拿旧清单插队）。所以一次拒绝之后，同一个 token 再按一次只会拿到同一句拒绝。
/// 浏览器那层为这一档备着 `spent`（层不关、清单留着让人对着看、那颗红的不亮），
/// 对齐这层以前没有——屏幕上红按钮还亮着，等于界面邀请人再按一次注定的失败。
///
/// 这一档改过**范围**：`spent` 现在只锁"看过名单"那一条。
/// 没名单那一条（默认那一条）没有陈旧可言 —— 再按一次就是现登记一份新凭证，
/// 那句拒绝自己也这么说（"请重新确认一次"）。
///
/// 原来这里还钉着「还没清单，先重新预览」那一臂（"没有清单"那颗不许读成"删 0 个"）。
/// 那一臂随"必须先有清单才让按"整条一起退了；接住同一句判据的是**两臂各自存在**：
/// 没名单那一臂说的是"这一趟先核对一遍"，而 `doomedCount === 0` 那一臂说的是"没有要删的"。
/// 合成一臂就等于让"没人看过"与"看过且是空的"共用一句话，那正是这一条要避免的形状。
#[test]
fn a_spent_mirror_confirmation_is_not_offered_again() {
    let confirm = strip_comments(&read("src/components/MirrorConfirm.tsx"));
    let at = confirm.find("key=\"go\"").unwrap_or_else(|| panic!("页脚那颗批准的按钮不见了：{confirm}"));
    let arm: String = confirm[at..].chars().take(520).collect();
    assert!(arm.contains("spent"), "这一份已经交出去了那一档整个不见了：{arm}");
    // 必须由它**禁用**，不是只在句子里提一句；而它只禁"看过名单"那一档。
    assert!(arm.contains("disabled={loading || (spent && preview !== null)}"),
        "凭证花掉了那颗红的还亮着，或者把没名单那一档也一起锁死了：{arm}");
    assert!(arm.contains("照对齐跑（这一趟先核对一遍）"),
        "没名单那一档没有自己的话（它不许读成「按下去删 0 个」）：{arm}");
    assert!(arm.contains("preview.doomedCount === 0"),
        "「看过且一份都没多出来」那一臂没了：{arm}");
    let panel = strip_comments(&read("src/Panel.tsx"));
    assert!(panel.contains("setPreviewSpent((p) => ({ ...p, [id]: true }))"),
        "拒绝那一路没把这一份标成已交出去：{panel}");
    // 重新列过一份就要放开：否则那颗永远是灭的。
    assert!(panel.contains("setPreviewSpent((p) => ({ ...p, [id]: false }))"),
        "重列之后那一颗还背着已交出的标记：{panel}");
    // 而"没名单"那一档的凭证是**现登记**的：发起点只许有一处（两处就有两处话术）。
    assert_eq!(count_in(&panel, "api.confirmMirror("), 1,
        "登记确认这一步有了第二个发起点：{panel}");
}

/// 「确认之后还要复核」这一份担保在两层里是**同一句前缀 + 各自的后半**。
///
/// 后半必须各自说准，因为后台做的确实是两件事：对齐那侧两侧各列一次、比的是**集合相等**
/// （`fresh != stored.doomed`），浏览器那侧再看的是**那一个位置**现在有什么。
/// 硬合成一句就有一层在骗人；两句差太远（"重新列一遍并逐条比对" vs "再看一次那个位置"）
/// 又会让人把后者读成"瞄一眼"。
#[test]
fn the_recheck_sentence_is_one_prefix_and_two_truths() {
    let mirror = strip_comments(&read("src/components/MirrorConfirm.tsx"));
    let layer = strip_comments(&read("src/components/LayerConfirm.tsx"));
    for half in [mirror.clone(), layer.clone()] {
        assert!(half.contains("确认之后后台会重新列一遍"),
            "那句复核又各写各的了（担保本身是同一件事）：{half}");
    }
    assert!(mirror.contains("逐条比对"), "镜像那侧的集合比对被写成看了一眼：{mirror}");
    assert!(!layer.contains("再看一次那个位置"), "浏览器那侧的复核又退回「看一眼」：{layer}");
    // 「那条线是假设值」要留着：它告诉人"这一档被拒不是我的东西真的危险"，是可行动的；
    // 后面那半句"量到真实分布之前会一直偏保守"说的是我们还没量——操作员对它做不了任何事，
    // 而 `DESIGN.md` 把这一类推理点名留在日志和文档里（那一条举的例子就是这句原话）。
    assert!(mirror.contains("假设值"), "阈值是假设值这件事不许被说成量出来的：{mirror}");
    assert!(!mirror.contains("量到真实分布"),
        "那条线还没标定这件事又回到删除确认层上了：{mirror}");
}

/// 两层那句「我知道，删这 N 个」与标题里的那个数，读的是**后台交出来的数**，
/// 不是界面拿数组长度数出来的行数（`I5` 那一族：数数也归后台）。
///
/// 为什么这条现在就要钉：确认层的显示是按条截的（`LIST_BUDGET`），那份数组在界面手里
/// 只代表"显示出来的那一段"。今天整份发上来所以两个数恰好相等，而哪天改成截断着发，
/// 界面上那句就会悄悄变成"前 20 个"——而它是删除的批准语。
#[test]
fn the_confirm_layers_read_the_backend_count_not_a_row_count() {
    for file in ["src/components/MirrorConfirm.tsx", "src/components/LayerConfirm.tsx"] {
        let t = strip_comments(&read(file));
        let at = t.find("key=\"go\"").unwrap_or_else(|| panic!("{file}：页脚那颗批准的按钮不见了：{t}"));
        let arm: String = t[at..].chars().take(400).collect();
        assert!(arm.contains("doomedCount"), "{file}：批准那颗上的数还是数组长度：{arm}");
        assert!(!arm.contains("doomed.length"), "{file}：批准那颗还在数行数：{arm}");
        // 标题里那个数也一样（标题是"会删掉 N 个（目标端共 M 个）"那一句）。
        let head = t.find("<Modal").unwrap_or_else(|| panic!("{file}：找不到那一层：{t}"));
        let title: String = t[head..].chars().take(200).collect();
        assert!(!title.contains("doomed.length"), "{file}：标题上的数是数出来的行数：{title}");
    }
    // 两份预览答复都要把这个数作为字段交出来。
    for rel in ["src-tauri/src/runner/mod.rs", "src-tauri/src/runner/browser.rs"] {
        assert!(read(rel).contains("pub doomed_count: usize"), "{rel} 没把那个数作为数据交出去：{rel}");
    }
    // 而上线的两份形状里都得带着它（一份漏了，界面那一侧就又得回去数行数）。
    let types = read("src/types.ts");
    assert_eq!(types.matches("doomedCount").count(), 2, "两份上线形状里有一份没带那个数：{types}");
}

/// 卡上那颗现在叫「让两边对齐」：操作员报的是"两颗都不像跑"，而"跑"这个意图应当在卡上读得出来
/// —— 修法不是把它改成「开始传输」（那颗按下去什么都不传，改了名就是造一枚假按钮，
/// 而"看着像开始、按下去不是"正是这个面板要防的第一号形状），是给它一个说真话的名字。
#[test]
fn the_mirror_row_names_the_run_entry_without_promising_a_transfer() {
    let list = read("src/components/JobListView.tsx");
    let t = surface_text(&strip_comments(&list));
    assert!(t.contains("让两边对齐"), "镜像行上没有那颗说得出'跑'的按钮：{t}");
    assert!(!t.contains("看看要删什么"), "旧名字还留在卡上（拒绝话术与出厂注释指的都是新名字）：{t}");
    // 名字不许越界：`add` 那一档的真传那颗仍叫「开始」，两个词不能互换。
    assert!(t.contains("开始"), "复制那一档的真传那颗被一起改掉了：{t}");
}

/// 层里那颗红色的必须把**两半**都说出来：只说删多少，那一层读起来就"只能删东西"，
/// 于是没人按它（实测：那 17 个连着三天没上去）。
/// 那个数读后台交出来的 `preview.pending`，不是界面拿 `srcCount - (dstCount - doomedCount)` 减出来的。
///
/// 同一轮操作员第二句是"不用再问一次"：那句「**我知道**，删这 N 个」要人先复述一遍授权，
/// 而这一层已经是全程唯一的那次询问（卡上那颗只是把清单摊开）。所以留两个数、去掉那层复述。
#[test]
fn the_confirm_button_says_both_halves_of_the_run() {
    let confirm = read("src/components/MirrorConfirm.tsx");
    let t = surface_text(&strip_comments(&confirm));
    assert!(confirm.contains("preview.pending"), "那个数不是后台交的那一个：{confirm}");
    assert!(t.contains("删这 "), "红色的那颗没带着删除数：{t}");
    assert!(t.contains("补上那 "), "红色的那颗只说删，不说这一趟同时补什么：{t}");
    assert!(!t.contains("我知道"), "那一层复述又回来了：全程只该问一次（操作员原话）：{t}");
}

/// 「会删除」那一格卡上**只剩一颗**按钮：「先试跑」撤了（同样两趟列举、同样的钱和 46 秒，
/// 交回来的是这份清单的真子集）。这一条钉的是"撤干净"——留着它就是让人在两颗里挑哪颗会真跑，
/// 而"两条独立算路对数"那条仪器住在 `tests/mirror_runner.rs`，不靠操作员每次点一遍。
#[test]
fn the_mirror_card_carries_a_single_button() {
    let list = read("src/components/JobListView.tsx");
    // `mirror_arm` 切的是**代码**（它找的是声明名），所以这里喂剥掉注释的那一份而不是屏上文字。
    let code = strip_comments(&list);
    let arm = mirror_arm(&code);
    // 三种拼法都钉（旧名留着，是为了让**改名之后**这一条仍然拦得住把按钮叫回旧名的写法）：
    assert!(!arm.contains("看看会动什么") && !arm.contains("先看一遍") && !arm.contains("先试跑"),
        "对齐那一格上还有试跑那颗：那是一趟只给计数的重复列举：{arm}");
    assert_eq!(arm.matches("act(").count(), 1, "对齐那一格不止一颗按钮了（并排就会有人按错那颗）：{arm}");
    assert!(arm.contains("让两边对齐"), "对齐那一格唯一那颗不叫「让两边对齐」：{arm}");
    // 反面：「只补，不删」那一格照旧两颗并排（那张没有逐条清单，试跑是它唯一的"会动哪些"）。
    let add_from = code.find("const addPrimary").expect("界面里找不到「只补，不删」那一档");
    let add_arm = &code[add_from..add_from + code[add_from..].find("];").unwrap()];
    assert!(add_arm.contains("看看会动什么") && add_arm.contains("开始"),
        "只补那一档被顺手一起撤了：{add_arm}");
}

/// 等待要在屏上说话——而且**每一段说各自那一句**。
///
/// 这里以前钉的是按钮上那两句（「正在列清单…」/「正在核对清单…」）。
/// 那天之后"列清单"从按下之前的过路费变成了这一跑自己的第一段，于是接住这句话的地方
/// 从按钮移到了右列：`TaskState::Running { phase: listing }`。
/// 三条判据，缺一条就有一个形状在骗人：
/// ① 按钮在 `loading` 那一臂上说话，且它排在各档话**之前**（否则正在算的时候还在叫人家去按）；
/// ② 右列对"列举那一段"有自己的一句话；
/// ③ 那句「传输进行中，关窗不会中止它」**只许**站在 `transferring` 那一臂上 ——
///   列举那一段一个东西都还没改，关窗什么都不会打断，把那句挂在前一段就是善意的谎。
#[test]
fn the_layer_says_what_it_is_doing_while_it_is_doing_it() {
    let confirm = strip_comments(&read("src/components/MirrorConfirm.tsx"));
    let footer = &confirm[confirm.find("footer=").expect("确认层里没有 footer 数组")..];
    let t = surface_text(footer);
    assert!(t.contains("正在核对清单"), "等待没在钮上说话：{t}");
    let loading = footer.find("loading ?").or_else(|| footer.find("loading &&"))
        .expect("那句话没挂在 loading 那一臂上（写死就是永远在转）");
    let quiet = footer.find("照对齐跑").expect("找不到'没名单'那一臂");
    assert!(loading < quiet, "loading 那一臂排在各档话后面：正在算的时候还是会叫人家去按那颗");

    let rail = strip_comments(&read("src/components/JobDetailPanel.tsx"));
    assert!(rail.contains("phase === 'listing'"),
        "列举那一段在右列没有自己的一句话（那几十秒屏上是死的）：{rail}");
    assert!(rail.contains("正在核对清单"), "列举那一段没说出它在干什么：{rail}");
    assert!(rail.contains("phase === 'transferring'"),
        "「别关窗」那一句没挂在传输那一臂上：{rail}");
    // 顺序也要判：`传输进行中` 那一段必须排在 `transferring` 那一臂之后，
    // 挂在 listing 之前就是每一段都在说后一段的话。
    let listing = rail.find("phase === 'listing'").unwrap();
    let transferring = rail.find("phase === 'transferring'").unwrap();
    assert!(listing < transferring, "两臂顺序倒了（后一句会盖住前一段）：{listing} {transferring}");
    assert_eq!(count_in(&rail, "传输进行中"), 1,
        "「传输进行中」出现了不止一处，那一句就不只属于传输那一段：{rail}");
}

/// 走线两边同名：Rust 的 `PreviewView.pending` ↔ TS 的 `pending`。
/// 少一边就是前端读到 `undefined`，而 `undefined > 0` 是 false —— 那句话会**静默地不说**。
#[test]
fn the_pending_count_reaches_the_wire_under_that_name() {
    let rs = read("src-tauri/src/runner/mod.rs");
    assert!(rs.contains("pub pending: usize"), "后台那份预览里没有这个数：{rs}");
    let ts = read("src/types.ts");
    assert!(ts.contains("pending: number"), "TS 那一侧没接这个字段：{ts}");
}
