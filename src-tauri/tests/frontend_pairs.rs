//! 对子视图的几条规矩：右列不许整块替换（看差异时也得能翻进度）、当前那一格的次要动作进「更多」、
//! 位置与方向的说法只许有一处、按钮不许长回危险色。
//!
//! 口径与 `frontend_theme.rs` 相同：读源文本、判据落在结构上。**大多数**条先剥注释再比，
//! 少数几条（暖色那一枚的落点、按钮颗数那类）故意读原文 —— 它们要认的就是写在代码里的
//! 那段注释或那串字面量，剥掉就没得认了。新加判据时记得选对读法。

mod common;

use common::{count_in, read, strip_comments, surface_text};

/// 右列是"同一份上下文换着看"，不是"根据 kind 决定给不给看"。
/// 旧形状是 `selected?.kind === 'gate' ? <GatePanel/> : <JobDetailPanel/>`：
/// 选中"对账"类任务就翻不到进度与日志，而人看完报告第一个动作就是想知道刚才发生了什么。
#[test]
fn the_right_rail_segments_instead_of_replacing() {
    let t = strip_comments(&read("src/components/JobContextPanel.tsx"));
    for want in ["'progress'", "'diff'", "'check'", "'log'"] {
        assert!(t.contains(want), "少一段 {want}：{t}");
    }
    assert!(!t.contains("kind === 'gate' ?"), "又在按 kind 整块替换：{t}");
    // 四段都挂着、只换显示：卸载重建会把这一屏的状态弄没（与"两面都挂着"同一个理由）。
    let tight = t.split_whitespace().collect::<Vec<_>>().join("");
    let kept = tight.matches("display:tab===").count();
    assert_eq!(kept, 4, "有一段不是挂着换显示，而是被换进换出（切到 {kept} 段）：{tight}");
}

/// 卡面上直接摆出来的动作要少：**打开所在目录**归收纳口。
/// 钉的是**数据流**，不是字面标签：卡面上的按钮是 `{a.label}` 渲染出来的，所以"这句标签
/// 有没有出现在 `<Button>` 里"这种判据拦不住真正的回退 —— 把那颗动作从 `rest` 挪进
/// `...Primary` 数组，它就又回到卡面上了，而字面标签一条都没动（这一版证人第一稿就是这么虚的，
/// 是拿 G 枪打出来的）。所以这里判的是：**那颗动作被登记进哪个数组**。
///
/// **2026-10-08：「看差异」从收纳口提到了卡面上**，这一条跟着收窄。理由不是"按钮少一颗更好看"：
/// 它是这一格**唯一只读**的动作，也是动手之前该先按的那一颗（README 第 4 步「第一次跑建议就停在这儿」）。
/// 收在「更多」里的那一次，格上那句「差异还没算过：点「看差异」算一次」指着一颗**看不见**的按钮，
/// 而同时看得见的那颗是「开始传输」—— 把只读的那条藏起来、把会动数据的那条摆出来，顺序是反的。
/// 收纳口还留着（打开所在目录 / 删掉这张临时卡），它仍然是" seldom 用但常驻"的那一格的去处。
///
/// 这一版还修了证人自己一个洞：原来判的是 `hits.any(|i| i >= rest_at && i <= menu_at)`，
/// 那是个**位置区间**，不是"登记在哪个数组里" —— 把一颗动作从 `rest` 挪到 `face`
/// （而 `face` 恰好就写在 `rest` 与 `</Dropdown>` 之间），这个断言照样绿。
/// 证人因为错误的原因变绿比变红更糟，所以下面改成**按数组切开再判**：看它落在 `constrest`
/// 与 `constface` 之间（收纳口）还是 `constface` 之后（卡面）。
#[test]
fn secondary_task_actions_live_in_a_menu_not_on_the_card() {
    let t = strip_comments(&read("src/components/JobListView.tsx"));
    let tight = t.split_whitespace().collect::<Vec<_>>().join("");
    assert!(tight.contains("<Dropdown"), "没有收纳口：{tight}");
    assert!(tight.contains("更多"), "收纳口不叫「更多」：{tight}");
    // 断言消息只摘开头一小段：整份压扁后的文件插进 panic 消息，读的人只会翻过去。
    let hint = |s: &str| -> String { s.chars().take(160).collect() };
    let rest_at = tight.find("constrest").unwrap_or_else(|| panic!("找不到喂给菜单的那个数组：{tight}"));
    let face_at = tight.find("constface").unwrap_or_else(|| panic!("找不到摆上卡面那个数组：{tight}"));
    assert!(rest_at < face_at, "两个数组的顺序反了（菜单那个在卡面之后）：{tight}");
    let primary_arms: Vec<(usize, usize)> = ["constgatePrimary", "constmirrorPrimary", "constaddPrimary"]
        .iter()
        .map(|anchor| {
            let from = tight.find(anchor)
                .unwrap_or_else(|| panic!("卡面主操作那一档不见了（{anchor}）：{}", hint(&tight)));
            let tail = &tight[from..];
            let len = tail.find("];").unwrap_or(tail.len());
            (from, from + len)
        })
        .collect();
    let find_key = |key: &str| -> Vec<usize> { tight.match_indices(key).map(|(i, _)| i).collect() };

    // 打开所在目录：留在收纳口，且不许混进主操作那一档。
    let key = "'opendir'";
    let hits = find_key(key);
    assert!(!hits.is_empty(), "这颗动作整个不见了（{key}）：{}", hint(&tight));
    assert!(hits.iter().any(|i| *i >= rest_at && *i < face_at),
        "「打开所在目录」不在收纳口那个数组里（{}..{}）：{}", rest_at, face_at, hint(&tight));
    for (s, e) in &primary_arms {
        assert!(!tight[*s..*e].contains(key),
            "「{key}」被登记进了卡面主操作那一档（{}..{}）：它又直接摊在界面上了", *s, *e);
    }

    // 看差异：现在**要**在卡面上（在 `constface` 之后，且在 `*Primary` 之外 ——
    // 它是只读的入口，不是会动数据的那一档，所以不许混进 Primary）。
    let key = "'inventory'";
    let hits = find_key(key);
    assert!(!hits.is_empty(), "这颗动作整个不见了（{key}）：{}", hint(&tight));
    assert!(hits.iter().any(|i| *i > face_at),
        "「看差异」又回到收纳口里去了：格上那句「点「看差异」算一次」会指着一颗看不见的按钮");
    for (s, e) in &primary_arms {
        assert!(!tight[*s..*e].contains(key),
            "「{key}」被登记进了卡面主操作那一档（{}..{}）：它和会动数据的那一颗混成一档了", *s, *e);
    }
}

/// 格上那句**后果**：两个数、"会不会删"、还有"几点算的"都从后台读，而且**不在列表渲染里发起盘点**。
/// （④片之前它住在 `frontend_browser.rs` 里；按内容归属搬到这里 —— 断言一个字没改。）
///
/// 2026-10-09 起这一句说的是**后果**而不是统计：面板存在的理由是「看清这一次会动哪些文件、
/// **会不会删**，再按下去」，而旧措辞「云端多余 N」在对齐模式下恰恰指**要删掉的那些**，
/// 却用了全屏最中性的词。于是这里多钉一条：那一格必须把"会不会删"**正面说出来**，
/// 且删除的那一份吃危险色（与 `MODE_VAR` 同一枚）。
#[test]
fn the_pair_row_shows_the_last_difference_and_when_it_was_measured() {
    let t = read("src/components/JobListView.tsx");
    // `job.lastSummary` 而不是 `lastSummary`：前者是那一行**读**了这个字段，后者一个 import
    // 或一个没人调的函数就能满足。
    assert!(t.contains("job.lastSummary"), "行上没有摘要：{t}");
    assert!(t.contains("measuredAt"), "没有'几点算的'，旧读数就会被当成现状：{t}");
    assert!(t.contains("{summaryLine(job, mode, dstIsLocal)}"), "那句摘要是写出来的，不是从字段读出来的：{t}");
    // 「会不会删」这一问要有正面答案，两档都算数：删的那一档说"会删 N"，不删的那一档说"不会删"。
    assert!(t.contains("会删") && t.contains("不会删"),
        "格上没正面回答「会不会删」——这一句是这个面板存在的理由：{t}");
    // 删的那一份吃危险色，且用的是**全屏唯一那枚暖色**（另起一个红就是第二个危险信号源）。
    assert!(t.contains("MODE_VAR.mirror"), "会删那一份没吃危险色：{t}");
    // **「会删 N 个」必须说清删的是哪一边**（2026-10-09）：删云端上的 N 个与删**本机硬盘上**的
    // N 个完全不是一回事 —— 后者没有任何撤销，而两者在屏上原本长得一模一样。
    // 方向读的是选择条那枚框已经摆着的结果（`dstIsLocal`），不许在格子里再拿路径判一遍（R69）。
    // **按结构判，不按字面**，而且**锚在 `summaryLine` 的函数体里**：「会删」这个词全仓最先出现
    // 的是文件顶部注释里那一句「颜色按会不会删给」，拿全文件第一个 `会删` 去找只会读到注释
    // （钉字面会永远红，而这一条要拦的是"两边说成同一个词"那种真的回退）。
    let fn_at = t.find("function summaryLine").expect("找不到 summaryLine：那一格的后果句就在它里面");
    let body = &t[fn_at..];
    let body = &body[..body.find("\nfunction ").unwrap_or(body.len())];
    assert!(body.contains("dstIsLocal"),
        "「会删 N 个」没说是删哪一边（没有读方向那枚框的结果）：{body}");
    assert!(body.contains("本机") && body.contains("云端"),
        "「会删」那半句没有按本机/云端分叉：{body}");
    // 空基准那一档不许跟着这次重写一起没了：本地是空的 ⇒ 这两个数不可信，而它恰恰是最容易
    // 出事的那种（空的一侧 + 对齐）。上一版摘要把它带在句尾，重写成排版时漏过一次。
    assert!(t.contains("emptyBasis"), "空基准那一档丢了（本地是空的，这两个数不可信）：{t}");
    // 摘要不自动刷（spec §3）：这里不许存在任何"没有就顺手算一次"的写法。
    assert!(!t.contains("runInventory(") && !t.contains("inventory()"),
        "列表渲染里发起了盘点 = 每次开窗几十次计费 LIST：{t}");
}

/// 对账那一段的标题取那张词表（`KIND_LABEL` = Rust `JobKind::word()` 的镜像，裁定 R56）：
/// 这里再发明一个说法就多一处会漂的地方。
///
/// **这一条原来叫 `the_list_puts_the_deleting_capable_cards_before_the_read_only_ones`**，
/// 里面四条判据在"那一面收成下拉 + 当前那一格"之后各自去了哪里（逐条对过才改的名）：
/// ① 能删东西的排在只读的之前 → `the_pairs_face_has_one_selector_per_attribute_and_no_card_list`
///    （拿 `aria-label="位置"` 与对账那一段的先后钉：同一件事换了形状，判据没丢）；
/// ③ 颜色信号一处一次 → 同上一条里那枚暖色的计数（它认的是 `var(--kind-warm)` 在 `intentOptions`
///    里恰好一处；`MODE_VAR.mirror` 那一枚由上面「删除的那一份吃危险色」那条钉）；
/// ④ 两段都真切出来 → 同上一条（`groupJobs(jobs)` 与对账那一段）。
/// 「只看」那枚筛选器整个没了：它的活（从一堆里挑一对）现在由位置那一枚下拉干，
/// 留着就是第四枚下拉 —— 操作员那句"相同属性的下拉应该只有一个"第一条就冲着它。
#[test]
fn the_gate_section_takes_its_name_from_the_shared_word_list() {
    let t = strip_comments(&read("src/components/JobListView.tsx"));
    assert!(t.contains("KIND_LABEL.gate"),
        "对账那一段的标题不再取那张词表（那就是第二份说法）：{t}");
}

/// 试跑答"要不要动"（计数），差异答"差在哪几个"（名字）——两者是**两份独立算路**，
/// 不许混为一谈，也不许靠解析 rclone 日志来补（日志格式随版本漂，而那是新造一份判定口径）。
/// 所以试跑那句下面要指路，而差异那一面的空态要说清它得单独算一次。
#[test]
fn the_dry_run_points_at_the_diff_instead_of_pretending_to_answer_it() {
    let detail = strip_comments(&read("src/components/JobDetailPanel.tsx"));
    assert!(detail.contains("翻上面「差异」那一面点「列一遍」"),
        "试跑那句没把路指出来（报的就是找不到差异）：{detail}");
    assert!(detail.contains("试跑完成，没有改动任何文件"), "试跑那句标题丢了：{detail}");
    // 不许出现"自动跑一次盘点"这种联动：那是每次试跑多打一次远端列举（钱与等待），
    // 而且会把两份独立读数混成一份。判据落在"有没有在这里调用盘点"。
    assert!(!detail.contains("runInventory"), "试跑那一面自己发起了盘点：{detail}");
    let inv = strip_comments(&read("src/components/InventoryPanel.tsx"));
    // 空态那句只指**这一面上真有**的那一颗（「列一遍」）。以前它拿「先试跑」作对比 —— 那颗后来
    // 改名「看看会动什么」，而对齐那一格 2026-10-05 起只剩一颗，指过去就是对着空气找下一步。
    assert!(inv.contains("「列一遍」单独算一次"), "差异那一面的空态没把路指在自己这一面上：{inv}");
    assert!(!inv.contains("看看会动什么") && !inv.contains("先看一遍") && !inv.contains("先试跑"),
        "空态那句又去指那颗可能不在卡上的按钮：{inv}");
    assert!(inv.contains("顺带"), "没点出'别的动作不会替它算'：{inv}");
}

/// 右列必须说得出"这一屏在说哪张卡"。
/// 四段读的全是选中那一条的上下文，而左列的选中态只有一层底色：没有这一行，
/// "刚点的那颗按钮在右列演的是哪条任务"就得回头数底色（报的 P1 是同一条的另一半——
/// 那一条修的是点按钮要先把卡立起来，这一条修的是立起来之后得看得见是哪张）。
#[test]
fn the_rail_says_which_task_it_is_showing() {
    let t = strip_comments(&read("src/components/JobContextPanel.tsx"));
    // 名字来自 `job.name`（后台字段），不是界面自己从 id 或路径推的。
    let at = t.find("job.name").unwrap_or_else(|| panic!("右列没有把任务名摆出来：{t}"));
    // 摆在那四个面**共用的那一层**（段控件之前），不是塞进某一段里——
    // 塞进一段就只有一段说得出是哪张卡。未选中那半边也有一处 `<Segmented`，所以取**最后**那一处。
    let seg = t.rfind("<Segmented").unwrap_or_else(|| panic!("段控件不见了：{t}"));
    assert!(at < seg, "任务名不是整列共用的那一行：{t}");
    // 反向也钉：四个面里不许各藏一份名字（那就是同一件事说四遍）。
    for face in ["'progress'", "'diff'", "'check'", "'log'"] {
        let head = t.find(&format!("display: tab === {face}"))
            .unwrap_or_else(|| panic!("少一段 {face}：{t}"));
        let body = &t[head..];
        let end = body.find("</div>").unwrap_or(body.len());
        assert!(!body[..end].contains("job.name"), "{face} 那一段里又自己抄了一份任务名：{t}");
    }
    // `src → dst` 不在这里重复：卡上已经写着了（重复就是同一屏两遍）。
    assert!(!t.contains("job.src"), "右列把卡上那行路径又抄了一遍：{t}");
}

/// 当前那一格只交**完整那两条路径**，任务名不在那里复述（操作员批准动的是卡上那一行）。
/// 病根是出厂那些名字自己把方向写进了字符串（「对齐 R2：以本地为主（删云端多余的）」），
/// 而同一格里隔两行就有「模式」+「方向」两枚框在说同一件事 —— 名字把它说第三遍。
/// 那条裁定（`DESIGN.md` Known Gaps）说的是**配置里的身份不改**，该动的是这一行。
/// 名字不是没地方去了：右列开头那一行（上一条钉着它）、设置面那一块、日志与确认层都拿它认这场动作；
/// 临时那一格也不靠这一行说自己是临时的（位置框里那句「（临时）」与格上那句"重开面板就没了"都在，
/// 那几处标志由 `a_draft_card_says_its_temporary_and_offers_the_two_exits_only_there` 钉着）。
/// **撤的是配对那一格上的**名字：对账那几行没有方向框，名字就是它们唯一的称呼，留在那儿。
#[test]
fn the_pair_cell_shows_the_paths_and_leaves_the_name_to_the_rail() {
    let t = strip_comments(&read("src/components/JobListView.tsx"));
    let from = t.find("const renderCell").expect("找不到当前那一格那一段");
    let rest = &t[from..];
    let gate_at = rest.find("const renderGate")
        .unwrap_or_else(|| panic!("找不到对账那一行的那一段（撤名字时把它一起动了？）：{rest}"));
    let cell = &rest[..gate_at];
    assert!(!cell.contains("job.name"),
        "当前那一格又自己抄了一份任务名（它两行之内就有模式与方向两枚框）：{cell}");
    // 撤名字不许顺手把"挑错一家靠全文认"那半句一起撤掉：那两条完整路径必须还在这一格里。
    //
    // 2026-10-08：这一行从单段 `{job.src} → {job.dst}` 换成了 `PairPath`（三段式**截中段**）。
    // 改的只是**怎么截** —— 尾部截断会把 `…\Local\Temp\d…` 留在屏上，认不出是哪一家，
    // 而这一行存在的理由正是"挑错一家靠全文认"。所以判据从"那个字面串在"改成
    // "两条路径的**字段**都还在，而且都经由那个会截中段的组件"：
    // 两处（当前那一格与对账那一行）都指同一个组件，这一处抄不动、也不会抄漏一边。
    assert!(cell.contains("<PairPath src={job.src} dst={job.dst} />"),
        "格上那两条完整路径不见了：{cell}");
    assert!(!cell.contains("job.src} → {job.dst}"),
        "这一格又自己拼回了那个单段字面串（那样会退回尾部截断）：{cell}");
    let comp = &t[..t.find("const renderCell").expect("找不到当前那一格那一段")];
    let comp = &comp[comp.find("function PairPath").expect("找不到那个截中段的组件")..];
    let comp = &comp[..comp.find("\n}").unwrap_or(comp.len())];
    assert!(comp.contains("path-head") && comp.contains("path-tail")
        && comp.contains("title={src}") && comp.contains("title={dst}"),
        "PairPath 不是截中段、或者没把全值挂上 title：{comp}");
    // 范围钉在这里：对账那一行**还**读名字（它没有那两枚框，名字是它唯一的称呼）。
    assert!(rest[gate_at..].contains("{job.name}"), "对账那一行的名字也被撤了：{rest}");
}

/// 四段的"现在没有内容"必须**同一句、同一形状**。
/// 旧形状是四种：一段一个 `Empty` 图标 + 一句"从左边选一个任务"、两段一行次要文字、
/// 一段三行长文、一段一个字都没有。同一个屏幕的四个面把同一件事说成四种样子，
/// 人就得以四种方式各认一次——而"这一段空着"与"这一段没有这一档"是两种读法，
/// 只有第一种该出现。
#[test]
fn the_four_faces_share_one_empty_state() {
    let rail = strip_comments(&read("src/components/JobContextPanel.tsx"));
    let tight = rail.split_whitespace().collect::<Vec<_>>().join("");
    // 未选中那一句只在右列那一层出现**一次**（每段自己再说一遍就是四种说法回来了）。
    assert_eq!(count_in(&rail, "先选一个任务"), 1, "「先选一个任务」出现了不止一处：{rail}");
    assert!(tight.contains("if(job===null)"), "没有选中任务时不进四段这一档：{rail}");
    // 进度面不再自己演空态：`Empty` 整个从界面消失（它以前是四段里唯一的图标版）。
    assert!(!rail.contains("Empty") , "右列又请回了图标版空态：{rail}");
    let detail = strip_comments(&read("src/components/JobDetailPanel.tsx"));
    assert!(!detail.contains("Empty"), "进度面还在自己演空态（于是四段有两种形状）：{detail}");
    // 而"选中了、这一段还没算过"在两处有数据的面都得说一句（差异 / 对账），
    // 走同一个渲染器：一句明面 + 细节进展开。
    for (file, want) in [
        ("src/components/InventoryPanel.tsx", "还没算过。"),
        ("src/components/GatePanel.tsx", "还没查过。"),
    ] {
        let t = strip_comments(&read(file));
        assert!(t.contains(want), "{file} 的空态没说出它是还没跑过：{t}");
        assert!(t.contains("<NoteLine"), "{file} 的说明没走一句 + 展开那个形状：{t}");
    }
}

/// 右列那两颗"现在就重算一次"的按钮必须受**同一道锁**约束，并说出同一句原因。
/// 后台的 `run_gate` / `run_inventory` 第一件事就是申领那两棵树的锁（`Runner::acquire`），
/// 所以以前从右列点下去不是"能不能点"的问题，而是必然吃一条拒绝——而卡上同一颗是灰的、
/// 还写着「正在跑：谁」。`DESIGN.md` 那句"每个'点不动'都读后台给的那句原因"只兑现了一半。
#[test]
fn the_rail_buttons_obey_the_same_lock_as_the_card() {
    let rail = strip_comments(&read("src/components/JobContextPanel.tsx"));
    assert!(rail.contains("job.lockedBy"), "右列整个不认那道锁：{rail}");
    assert!(rail.contains("runningHint(job)"), "右列自己另编了一句锁的话：{rail}");
    for file in ["src/components/GatePanel.tsx", "src/components/InventoryPanel.tsx"] {
        let t = strip_comments(&read(file));
        assert!(t.contains("<LockHint"), "{file}：那颗按钮灰着却说不出为什么：{t}");
        assert!(t.contains("disabled={off}"), "{file}：那颗按钮没受锁约束：{t}");
    }
    // 「正在跑：X」全仓只拼一处（`types.ts` 的 `runningHint`）：两处现编就是两处会各自漂的话术。
    let types = strip_comments(&read("src/types.ts"));
    assert_eq!(count_in(&types, "正在跑："), 1, "那句锁的文案有两处拼装：{types}");
}

/// 那一句"还没算过"的空态之外，卡上那颗入口的名字要跟**段名**一致。
/// 同一个面以前有四个称呼（段名「差异」、那颗「本地没有的云端对象」、面上的标题、那颗「列一遍」），
/// 而那颗按下去做的事就是"翻到「差异」那一段并算一次"——名字跟着段名走，人学一次词能用四处。
/// 钉的是**那颗登记进收纳口时带的那个标签**与摘要句引用它，不是钉一串死字。
#[test]
fn the_entry_that_opens_the_diff_face_is_named_after_that_face() {
    let t = strip_comments(&read("src/components/JobListView.tsx"));
    let from = t.find("act('inventory'").unwrap_or_else(|| panic!("那颗只读盘点入口不见了：{t}"));
    let label: String = t[from..].chars().take(120).collect();
    assert!(label.contains("'看差异'"), "那颗不叫「看差异」（它开的就是「差异」那一段）：{label}");
    assert!(!t.contains("'本地没有的云端对象'"), "旧那个九字称呼回来了（同一段的第四种叫法）：{t}");
    // 「还没算过」那一档**不再指路**（2026-10-09）：它曾写「点「看差异」算一次」，而那一颗
    // 2026-10-08 就从收纳口提到了卡面上 —— 屏上指着一颗就在眼前的话，读者要多走一步。
    // 这一档现在只说状态（「还没算过差异」），那颗按钮自己站在这一格里，名字仍然是「看差异」。
    assert!(!t.contains("点「看差异」算一次"),
        "没算过那一档又去指路（那一颗现在就在卡面上，指路反而多一步）：{t}");
    assert!(t.contains("还没算过差异"), "没算过这一档没了：{t}");
}

/// 对账报告也带着**它是几点查的**，而那一档的形状全仓只有一处定义（`src-tauri/src/moment.rs`）。
///
/// 两个理由叠在一起：
/// ① 与盘点那句"几点算的"同族，而对账交出去的是**发布许可**——一次旧许可被当成现状
///   就是一次带着 404 的发布（`frontend_pairs.rs` 上面那条给盘点写的理由原样适用于这里）。
/// ② 界面不许把日期再解释成"今天"：那是拿浏览器的时钟去解释后台那一刻（面板可以整夜开着），
///   也正是 `I5` 那条"判定与解释作为数据走出去"。所以后台交的是成品字符串，TS 只渲染。
#[test]
fn the_check_report_carries_the_moment_and_only_one_place_formats_it() {
    let gate = read("src-tauri/src/gate.rs");
    assert!(gate.contains("pub measured_at: String"), "检查报告没有那一个时刻：{gate}");
    assert!(gate.contains("moment::now_label()"), "时刻不是取自那一处唯一的格式化：{gate}");
    // 盘点那一族也走同一处，且谁都不许再自己拼那一档日期。
    let prune = read("src-tauri/src/prune.rs");
    assert!(prune.contains("moment::now_label()"), "盘点的时刻又自己拼了一份：{prune}");
    for rel in ["src-tauri/src/gate.rs", "src-tauri/src/prune.rs", "src-tauri/src/runner/mod.rs"] {
        let t = strip_comments(&read(rel));
        assert!(!t.contains("%m-%d %H:%M"), "{rel} 里还留着自己拼的那一档日期：{t}");
    }
    // 那个字段要**在 GateReport 那一段里**上线：整份 types.ts 里出现一次不算（SummaryView 也带它）。
    let types = read("src/types.ts");
    let report = types.split("export interface GateReport").nth(1).unwrap_or("")
        .split('}').next().unwrap_or("");
    assert!(report.contains("measuredAt"), "检查报告的那个时刻没按上线的形状交出来：{report}");
    let panel = strip_comments(&read("src/components/GatePanel.tsx"));
    assert!(panel.contains("report.measuredAt"), "界面没把「几点查的」摆出来：{panel}");
}

/// 「差异」那一面的解释不许长回半屏。

/// 契约那两条：`DESIGN.md` 的"一句人话留在明面，细节收进展开"，以及"五档字阶不许有第六档"。
/// 第六档以前真的在：这一面自带一个 `Typography.Title level={4}`，而 antd 的字号梯
/// （`genFontSizes.js`，基值取我们的 `fontSize: 13.5`）把 heading level 4 算成 **20px**——
/// 18/15/13.5/12.5/11 之外的一档，且它是全仓唯一一处 `Title`。
#[test]
fn the_diff_face_explains_itself_in_one_line_and_no_sixth_type_step() {
    let t = strip_comments(&read("src/components/InventoryPanel.tsx"));
    assert!(!t.contains("Typography.Title"), "这一面又给自己加了一个标题（段控件已经写着「差异」）：{t}");
    assert!(!t.contains("Alert type=\"info\""), "解释又变回一整块 Alert（明面只能一句，细节进展开）：{t}");
    // 明面那句短到能一眼读完，长的那半段进了展开。
    assert!(t.contains("这里只给你看，要删请走对齐任务。"), "常驻那句的解释丢了：{t}");
    // 列标题只写方向本身，不再"名字 + 括号把名字解释一遍"。
    assert!(t.contains("本地有、云端没有"), "两列的方向标签不见了：{t}");
    assert!(t.contains("云端有、本地没有"), "两列的方向标签不见了：{t}");
    assert!(!t.contains("还没上传的（"), "列标题又复述了一遍方向：{t}");
    assert!(!t.contains("本地没有的云端对象（"), "列标题又复述了一遍方向：{t}");
}

/// 「看看会动什么」（原名「先试跑」）与真执行并排时最容易按错：一颗什么都不传，另一颗真传。所以**并排的那一格**要有一句
/// 当场说得清的"它不动数据"，而不是等人按完再解释。
///
/// 操作员报"跑之前不该再让我预览一遍、不该问两次"。查完账，**「会删除」那一格的「先试跑」撤了**
/// （它列的是同样两趟、花同样的钱和 46 秒，交回来的是「让两边对齐」那份清单的**真子集**：只有计数，
/// 没有名字也没有占比；spec §4.1 留它要的"两条独立算路对数"早就长在 `tests/mirror_runner.rs` 里，
/// 那是我们的验收活，不该让操作员每次跑之前替他付一遍）。于是：
/// - 这句只挂在 `mode === 'add'` 那一格里（对齐那一格只剩一颗按钮，没有并排就没有可错的地方）；
/// - 旧 shape 是"按 kind 分组，注挂在分组标题下面"（当时挂在每张卡脚下就是十二行灰字）；
///   现在一张卡就是一对位置，注跟着**这一张卡站在哪一格**走，一格一句，同样不重复。
#[test]
fn the_trial_button_says_out_loud_that_it_transfers_nothing() {
    let raw = read("src/components/JobListView.tsx");
    let t = surface_text(&strip_comments(&raw));
    // 断言消息只摘前面一小段：整份文件插进 panic 消息，读的人只会翻过去（同仓那条 4 MB 的教训）。
    let hint: String = t.chars().take(160).collect();
    assert!(t.contains("看看会动什么"), "那颗本来就不在：{hint}");
    assert!(t.contains("「看看会动什么」只告诉你会动哪些，不真传；要把东西放上去，按「开始」。"),
        "「只补，不删」那一格的注没整句写出来（并排时按错的就是它）：{hint}");
    assert_eq!(t.matches("只告诉你会动哪些，不真传").count(), 1,
        "这句又出现了两处：镜像那一格卡上只剩一颗按钮，没有可错的地方，不该还跟着说一句");
    // 判据落在**结构**上：这一句由"这一格是什么模式" gate 住，不是每张卡无条件摆一份。
    let code = strip_comments(&raw);
    assert_eq!(count_in(&code, "ADD_MODE_NOTE"), 2,
        "这句不再只有一处定义 + 一处摆上屏（第三处就是同一句说两遍）：{code}");
    // 2026-10-08：它从**常驻在卡上的一行字**改成挂在「先试跑」那颗自己的悬停上。
    // 改的只有"什么时候说"，没改"说什么"：它说的是那两颗的区别（并排时最容易按错），
    // 不是这一格的状态，而常驻那一行让"现在是什么状态"这一层被两行解释压着。
    // 所以判据从「它渲染在 `{mode === 'add' && ...}` 那一格里」换成「它是 'trial' 那颗
    // 的 hint，且那个 hint 只在 add 模式下才给」—— 对齐那一档仍然不带着它。
    let trial_at = code.find("act('trial'").expect("找不到试跑那一颗");
    let trial_line = &code[trial_at..trial_at + code[trial_at..].find("),\n").unwrap_or(160)];
    assert!(trial_line.contains("mode === 'add' ? ADD_MODE_NOTE"),
        "那句注没有跟着试跑那颗走，或没有挂在'只补，不删'这一格上：{trial_line}");
    assert!(trial_line.contains("'看看会动什么'"), "这一条说的不是试跑那颗：{trial_line}");
    // 「会删除」那一档的源码区间里没有这一句（挂错了地方就是让人以为按下去还会先看清清单）。
    let from = code.find("const mirrorPrimary").expect("找不到「会删除」那一档");
    let arm = &code[from..from + code[from..].find("];").unwrap()];
    assert!(!arm.contains("只会告诉你会动哪些"), "镜像那一档自己带着这句：{arm}");
}

/// 浏览面板：**默认看本地 + 对面空时满宽 + 本地图走网格**（2026-10-11）。
///
/// 三条是一件事：这一面开着多数时候不是为了左右比，就是为了翻本地那一堆图。
/// 原来两栏都空着（人得先挑位置才看得见第一张图）、各占半宽（一行只放得下三四张图）、
/// 图藏在表格里（要一行一行扫名字）—— 三样都把"翻图"这件事变重了。
#[test]
fn browsing_defaults_to_local_and_gives_it_the_whole_width() {
    let raw = read("src/components/BrowserPanel.tsx");
    let t = strip_comments(&raw);

    // 默认站到第一个本地目录，且**不**自动站云端那一栏（替人挑"要比谁"比多一次点击更贵）
    assert!(t.contains("r.find((x) => x.isLocal)"),
        "打开这一面没有默认站到本地：{t}");
    assert!(t.contains("p.local.fs !== null"),
        "默认那一步会覆盖操作员已经挑好的位置：{t}");

    // **本地为主、云端是可切换的比对**（2026-10-11 再改一版）：单边占满并收起动作族，
    // 进比对才并排且把动作放出来。原来那条「对面空就满宽」只是形状的一半，
    // 另一半是"对面空**且**没进比对时那一栏是 compact" —— 翻图时那族按钮一个都用不上。
    assert!(t.contains("wide={!comparing}") && t.contains("compact={!comparing}"),
        "单边/比对两档没有分别给满宽与收起的 compact：{t}");

    // 进比对的入口在**顶部**，明说它是什么；退出的那颗也在。
    assert!(t.contains("和云端比一比") && t.contains("不比了，回去看本地"),
        "进/出比对的两颗按钮不在：翻图与比对之间的那一步得是看得见的：{t}");

    // 进比对时云端站到**本地这一栏配对的那一头**，不是随便挑一个远端：
    // 这一面存在的意义就是比同一个位置的两面，随便挑一个等于把要比的两堆放到一起。
    assert!(t.contains("pairedWith"),
        "进比对时没有读配对关系：{t}");
}

#[test]
fn the_local_column_shows_photos_as_a_grid() {
    let raw = read("src/components/BrowserPane.tsx");
    let t = strip_comments(&raw);
    assert!(t.contains("looksLikeImage"),
        "图片与非图片没有分开：网格里每格都得是一张能看的图：{t}");
    // 网格里已经列过的，表格里**不再摆一遍**（同一张图在屏上出现两次是白占地方）。
    // 钉的是那份排除本身 —— 以前这里钉的是渲染分支里的一句 `grid.includes(row)`，
    // 而那句在 `rest` 已经排除之后恒为假，钉着它等于给一条走不到的分支作保。
    assert!(t.contains("!grid.includes(e)"),
        "网格列过的图在表格里又摆了一遍：{t}");
    // 网格与表格式共用 `ThumbCell`，而不是另写一个 —— 两处各画一次，
    // "点得动但读不出"的那一格就会出现两次（一次能看、一次不能）。
    assert!(t.contains("thumb-grid") && t.contains("<ThumbCell"),
        "网格没有复用同一个缩略图组件：{t}");
    // 点一格仍然是"选中那一个文件"（跨栏动作的源），网格不是另一个入口
    assert!(t.contains("a.onPick("),
        "网格那一格不与表格共用那个选中动作：上传/移到这里会断：{t}");
    // **只有满宽且本地那一档摆网格**。半宽时一屏只放得下三四张，而并排比要的是左右对同一行；
    // 云端那一栏没有可摆的图（rc 上没有只读前若干字节的出口），摆出来就是一屏「看不了」。
    // 这一条钉的是那个判据：`wide` 必须**被读**，只传不读就是孤儿 prop（实测过：两栏并排时
    // 本地那一栏照样摆了 3 列网格，而注释与 CSS 都写着"满宽时才有"）。
    assert!(t.contains("v.wide === true && !v.isRemote"),
        "网格没有等「满宽且本地」这一判据：{t}");
    // 图片行被网格接走之后，"改这一张 / 删这一张"必须有个去处（否则界面上只剩
    // 「删掉这一层看到的 N 个」与「清空这一层」两颗比它重一整棵子树的按钮）。
    assert!(t.contains("改名这一个") && t.contains("删这一个"),
        "点中一张图之后没有逐个处理的动作：{t}");
    assert!(t.contains("const picked ="),
        "那一格点中的东西没有回到动作条上：{t}");
}

/// 浏览面板的两栏**联动**（2026-10-11）。
///
/// 这一面开着是为了**比两边差在哪**，而"差在哪"只在「同一位置的本地 ↔ 同一位置的云端」
/// 这一对上成立。两边各挑各的位置时屏幕并排着，可它们根本不在同一处 —— 看着是对比，
/// 其实摆在一起比的是两堆无关的东西。
///
/// 这一条钉三件：
/// 1. 挑位置时**对面自动站到配对的那一头**（读后台的 `pairedWith`，界面不自己猜）；
/// 2. **导航也联动** —— 往里走一层要两边一起走，只动一边走两层就站在不同深度上了；
/// 3. 上一级（面包屑那条路）同样联动，否则会留下一道"能跳过去、跳完就跑偏"的缝。
#[test]
fn the_two_browser_columns_follow_one_another() {
    let t = strip_comments(&read("src/components/BrowserPanel.tsx"));
    // 配对关系读后台交出来的那一枚，界面不认路径、不猜
    assert!(t.contains("pairedWith"),
        "联动没有读后台的配对关系：界面就得自己猜「哪个本地配哪个远端」，那是第二套口径：{t}");
    // 挑位置时对面跟上
    assert!(t.contains("onSelectPosition"),
        "换位置那一处不见了：{t}");
    // 导航两处都要跟（进目录 + 面包屑），少一处就留下一道缝
    let enter = t.matches("panesRef.current.remote").count();
    assert!(enter >= 3, "联动只在挑位置时跟，导航（进入 / 面包屑）没跟：{t}");
}

/// 设置里的「任务」那一栏**不许拿任务名当标题**（2026-10-11）。
///
/// 出厂那十条的名字里烘焙着方向（「镜像 R2：以本地为主（删云端多余的）」），而卡上
/// 早已把方向收成「要做什么」那一格。于是同一个东西：卡上是「位置 + 要做什么」，
/// 设置里是「一句裹着方向的名字」—— 人在设置里改一条，眼前那句话和卡上那一格对不上，
/// 就以为改的不是同一件事。
///
/// 这一条钉的是"设置那一行读的是**卡上那一句**"：它必须走 `intentOptions`（同一份），
/// 而不是把 `j.name` 或 `KIND_LABEL[j.jobKind]` 拼一遍。
#[test]
fn the_settings_task_row_speaks_the_same_language_as_the_card() {
    let raw = read("src/components/SettingsPanel.tsx");
    let t = strip_comments(&raw);
    assert!(t.contains("intentPhraseFor(groups, j)"),
        "任务那一行没有读卡上那一句（intentPhraseFor）：两处界面会各说一套：{t}");
    // 与卡上同一份，不是另写一遍
    assert!(t.contains("intentOptions(g)"),
        "它自己另写了一套说法：应该调 `pairs.ts` 那一份：{t}");
    // 旧写法不许回来：任务名回到第一行大字，等于把烘焙着方向的名字又摆回明面。
    assert!(!t.contains("<Typography.Text strong>{j.name}</Typography.Text>"),
        "任务名又成了标题：那正是方向被烘焙进字符串的那一格：{t}");
    // 会删的那一档吃暖色 —— 与卡上那一枚框同一枚色
    assert!(t.contains("j.jobKind === 'sync' ? { color: 'var(--kind-warm)' }"),
        "会删那一档没吃暖色：两处说同一件事，颜色却不是同一枚：{t}");
}

/// 临时卡那颗按钮**要说结果**（2026-10-11）。
///
/// 「存成常用」这四个字是人话，可它要人自己推一步才知道按下去会写进配置 ——
/// 而这一颗按下去正是 R97 那条路。这条钉住"结果写在按钮上"。
#[test]
fn the_draft_button_says_what_it_will_do() {
    let raw = read("src/components/JobListView.tsx");
    let t = strip_comments(&raw);
    assert!(t.contains("存成常用（写进配置）"),
        "那颗按钮没说自己会写进配置：{t}");
    // 临时卡那一层也要把"关掉就没了"与"怎么常用"说全
    let layer = strip_comments(&read("src/components/PairDraftLayer.tsx"));
    assert!(layer.contains("写进配置"),
        "临时卡那一层没说清常用它要经过哪一步：{layer}");
}

/// 盘点顺手落的那份名单，路径要在「差异」那一面露出来（可复制）。
/// 藏在日志里等于没给：操作员要的就是"拉一次存成文件，之后自己比对"，
/// 而这条通路唯一的入口就是这一行字。
#[test]
fn the_difference_face_shows_where_the_name_list_was_written() {
    let raw = read("src/components/InventoryPanel.tsx");
    let t = surface_text(&strip_comments(&raw));
    assert!(raw.contains("report.listFile"),
        "路径没读后台交出来的那一个（界面自己拼一个就等于猜）：{raw}");
    assert!(t.contains("名单存成了"), "屏上没说这一份落在哪儿：{t}");
    assert!(raw.contains("copyable"), "那一串不能复制：拿去比对还得手抄：{raw}");
}

/// 「两边都有、但不是同一份」这一类必须在界面上**露出来**，而且要带方向（2026-10-11）。
///
/// 这一类是集合差看不见的：两边都有的名字在 `local \ remote` 与 `remote \ local` 里同时消失，
/// 于是「云端那份被换过、你本地还是旧的」在盘点里一个字都不提，跑一次 `sync` 就被静默覆盖
/// （后台那条线是 `prune::diverged`，判据见 `tests/gate.rs` 里那几条证人）。
///
/// 所以这一条钉的是三件事：
/// 1. 读的是**后台交上来的那份**（界面自己重算就是第二套口径，R69 同一条禁令）；
/// 2. **有方向那一列** —— 只说"这两个不一样"没法做决定，得知道按上传会覆盖哪一边；
/// 3. `newer` 为空那一格写的是「看不出」而不是留空 —— 留空会被读成"没这一行信息"，
///    而它其实是"两边都不足以判断"，那恰恰是最该说清的一种形状。
#[test]
fn the_difference_face_lists_files_that_exist_on_both_sides_with_their_direction() {
    let raw = read("src/components/InventoryPanel.tsx");
    let t = surface_text(&strip_comments(&raw));

    assert!(raw.contains("report") && raw.contains("diverged"),
        "没读后台交上来的那一份：{raw}");
    // 方向那一列，且 `remote` 那一格点了出来 —— 它是「按上传会覆盖它」的那一格，
    // 界面上必须与 `local` 视觉上分得开。
    assert!(t.contains("哪边比较新"), "没说要人决定哪边留：{t}");
    assert!(t.contains("云端那份"), "云端更新的那一格没在屏上说出后果：{t}");
    assert!(t.contains("看不出"), "说不出方向的那一格得说「看不出」，不能留空：{t}");
    // 缺席防御：这一份是后端新加的字段，`undefined.length` 是 TypeError 而不是"没有差异"。
    assert!(raw.contains("?? []"),
        "直接读 `.length` 而不兜底：字段缺席时是崩溃，不是空清单：{raw}");
    // 而且它必须**常驻一句话解释**——这一类为什么不在前两张表里，是最反直觉的一处。
    assert!(t.contains("两边都有"),
        "这一类为什么单列一句都没说：{t}");
}

// ---- 一张卡 = 一对位置，方向与模式是卡上的两个选择框 ----
//
// 操作员：「以本地为主、以镜像为主，应该是选择框，这样更直观吧，而不是 N 条不同的条目」。
// 数过之后他是对的：出厂那 10 条 = 2 对位置 × {上传、下载、对齐两向} + 2 条对账，
// 方向被当成了身份，所以"哪条是合适的"在界面上没有答案。

/// 分组与选择框的选项都只读后台带出来的三枚字段；选项由"这一格有没有任务"过滤出来。
#[test]
fn a_card_is_one_position_pair_and_its_selectors_list_only_what_exists() {
    let raw = read("src/pairs.ts");
    let code = strip_comments(&raw);
    for f in ["pairKey", "pairSlot", "aIsLocal"] {
        assert!(code.contains(f), "分组没读后台带出来的 {f}：前端再判一次就是第二份口径");
    }
    // 不许在 TypeScript 里再认一遍路径 —— 裁定 R69 就是被三份正则坑出来的。
    assert!(!code.contains(".match(") && !code.contains("RegExp"),
        "pairs.ts 里出现了路径正则：哪一侧是本地该读 aIsLocal");
    assert!(code.contains("slotJob(g, mode, s) !== null"),
        "选项不是从真实任务过滤出来的：写死的 2×2 会给出按下去没有任务的选项");
    let def = &code[code.find("export function defaultChoice").expect("找不到 defaultChoice")..];
    let add = def.find("'add'").expect("defaultChoice 里没有'add'");
    let mirror = def.find("'mirror'").expect("defaultChoice 里没有'mirror'");
    assert!(add < mirror, "默认那一档不是「只补，不删」：重开面板会停在会删东西的那一格");
    // 方向那句在两侧同型时不许说"云端"（复制允许两侧都本地、也允许两侧都 remote）。
    let dir_from = code.find("export function directionLabel").expect("找不到 directionLabel");
    let dir_to = code.find("export function modeLabel").expect("找不到 modeLabel");
    let dir = &code[dir_from..dir_to];
    assert!(dir.contains("aIsLocal"), "方向那句没读那枚标志：它会在两侧都本地时说出'云端'");
    // 两侧的类型必须**各自一个标志**：拿"另一面 = a 面取反"去推，两侧都本地的那一对会被推成
    // 一本地一云端（真窗口抓到的正是这一族，只不过方向反过来：本地↔远端落进了同型那一支）。
    // 两侧的类型必须**各自一个标志**，而且那两句要互为镜像（站 a 就拿 a 面，站 b 就拿 b 面）：
    // 拿"另一面 = a 面取反"去推，两侧都本地的那一对会被推成一本地一云端
    // （真窗口抓到过同一族，只是方向反过来：本地↔远端落进了"两侧同型"那一支）。
    assert!(dir.contains("slot === 'a' ? g.aIsLocal : g.bIsLocal")
        && dir.contains("slot === 'a' ? g.bIsLocal : g.aIsLocal"),
        "两面的类型不是各自读各自那枚标志（或那两句不再互为镜像）：{dir}");
    assert!(!dir.contains("!g.aIsLocal"), "另一面的类型是取反推出来的：{dir}");
    // 两档模式的名字也钉在这里：界面上只有 `modeLabel` 这一处取名，词就只该在这一处出现。
    let mlabel = &code[code.find("export function modeLabel").expect("找不到 modeLabel")..];
    // 2026-10-09 词表迁移：`只放上去（不删）` → `只补，不删`、`会删除（镜像）` → `让两边对齐（会删）`。
    // 这一条要判的是"**只有这一处取名**"，所以新名断言 + 旧名禁掉，两头都钉。
    assert!(mlabel.contains("只补，不删") && mlabel.contains("让两边对齐"),
        "两档模式的名字不是这一处给的：{mlabel}");
    assert!(!mlabel.contains("只放上去") && !mlabel.contains("会删除（镜像）"),
        "旧拼法又回来了：{mlabel}");
    assert!(dir.contains("为主（删") && dir.contains("多余的"),
        "「让两边对齐」那一格的方向话不见了：{dir}");
    // 方向那一句必须**跟着模式走**：「只补，不删」那一格里它是"从哪边补到哪边"，
    // 不许沿用"删…多余的" —— 一格说法就是两种模式共用那句最重的话（操作员分不清哪个合适的来源之一）。
    assert!(dir.contains("从") && dir.contains("补到"),
        "「只补，不删」那一格的方向没有自己的说法：{dir}");
    // 反向钉：那两句**各自都要按模式分叉**（两侧同型那一支 + 本地/云端那一支 = 两处）。
    // 把任何一处的 add 那一支删掉，就是"两种模式共用一句最重的话"，这里数的是分叉点本身。
    assert_eq!(dir.matches("mode === 'add'").count(), 2,
        "方向那两句不再两处都按模式分叉（合并成一格说法就是回到旧形状）：{dir}");
    assert_eq!(dir.matches('删').count(), 2,
        "「删」在方向那一句里出现的次数不是 2（镜像那两格各一次）：那一格又在替不删东西的模式说话：{dir}");
}

/// 模式与方向的选择**不跨重开面板记住**：记住一个"会删东西"的选择，等于让某天的第一次点击
/// 落在最重的那一档上。这里钉的是持久化的三条通路都没被用。
#[test]
fn the_mode_choice_never_outlives_the_session() {
    let list = strip_comments(&read("src/components/JobListView.tsx"));
    for w in ["localStorage", "sessionStorage", "document.cookie", "invoke("] {
        assert!(!list.contains(w), "卡上的选择走了持久层或自己发命令（{w}）：模式必须每次从「只补，不删」开始");
    }
    assert!(list.contains("useState"), "选择没有存在组件状态里：那它靠什么记住这一张卡当前是哪一格？");
}

/// 当前那一格脚下的按钮按**模式**摆：「只补，不删」那格两颗（看看会动什么 + 开始），
/// 「让两边对齐」那格只有一颗（它自己就叫「让两边对齐」），而且界面上任何一格都不许出现 danger/实心删除。
/// 两格的名字与方向那两句都不许在这一面现编：取 `pairs.ts` 的那两处
/// （`a_card_is_one_position_pair_and_its_selectors_list_only_what_exists` 钉的是词只在那一处）。
///
/// **原来这一条叫 `the_card_buttons_follow_the_mode_and_nothing_deletes_from_the_card`**，
/// 里面那条"选择框那一行必须挡住冒泡"随卡列表一起退了 —— 它当时防的是：antd 的下拉是 portal，
/// 事件走的却是 React 树，点中一格选项的那发 click 会冒到**那张卡自己**那层 `onSelect(job.id)` 上，
/// 而闭包里那个 `job` 还是换之前那一条（真窗口抓到过"卡换了、右列没换"）。
/// 现在那一面上没有卡可以冒了：三个下拉都在顶部那一行，脚下没有 `onSelect` 的祖先。
/// 接管它的是"换格必须同时把右列指过去"那两支：本条下面的 `choose` 与
/// `a_cell_is_always_selected_and_the_rail_follows_it` 里的 `onSelect(picked.job.id)`。
#[test]
fn the_cell_buttons_follow_the_mode_and_nothing_deletes_from_the_face() {
    let list = strip_comments(&read("src/components/JobListView.tsx"));
    let t = surface_text(&list);
    // 「开始」是 `t.contains("开始")` 就够的那一颗 —— 它是整份源码里出现得最多的两个字，
    // 所以这条判据钉的是**它在不在**，不是"出现几次"（那会是另一条判据，且必然是假红）。
    assert!(t.contains("让两边对齐") && t.contains("开始"), "两格各自那颗入口都没了：{t}");
    // 2026-10-09（模式+方向并成一枚「要做什么」之后）：这一格叫什么由 `pairs.ts` 的
    // `intentOptions` **一处**算出来，这一面只许调它。判据因此挪到那一处上去：
    // 原来查的是"这一面有没有用 modeLabel/directionLabel/availableSlots/slotJob"，
    // 现在查的是"这一面调了 intentOptions" + "intentOptions 内部用的是那几把尺子" ——
    // 两次都查，是为了**这一面**与 **`pairs.ts`** 两处各断一次：只查一头的话，
    // 有人把 intentOptions 改成一个自己现编名字的空壳，另一头照样绿。
    assert!(list.contains("intentOptions"),
        "这一面没有走 intentOptions：这一格的真名只该在 `pairs.ts` 一处算：{list}");
    let pairs_src = read("src/pairs.ts");
    let body = &pairs_src[pairs_src.find("export function intentOptions").expect("找不到 intentOptions")..];
    for helper in ["modeLabel", "directionLabel", "availableSlots"] {
        assert!(body.contains(helper), "intentOptions 内部没有用 {helper}：那是把\"这一格有什么\"在第二处算了一遍");
    }
    // 反向钉（枪打出来的：只留 `modeLabel` 那一条时，把「只看」的选项换成字面"只放上去（不删）"照样绿）：
    // 这一面**不许自己现编那两档的名字**，一张词表只许有一处字面。
    assert!(!t.contains("只放上去") && !t.contains("会删除（镜像）"),
        "这一面写死了一份模式的名字：那张词表在 `pairs.ts` 的 `modeLabel`，两处就是两份会漂的口径：{t}");
    // 删除的批准仍然只在确认层里：这一面不许有红色那颗。
    assert!(!list.contains("danger"), "界面上出现了红色按钮：批准删除的那颗只能在确认层的页脚");
    // 分组那一段读的是 groupJobs，不再按 kind 出一条一条（按 kind 出就是"方向当身份"那一版）。
    assert!(list.contains("groupJobs"), "这一面还是按 kind 一条一条：那 10 条就还是 10 行");
    assert!(list.contains("g.key"), "分组没按配对键走：{list}");
    // 换选择必须**同时**把右列指过去：右列讲的要是当前这一格那条任务（同族那条）。
    let choose = &list[list.find("const choose = ").expect("找不到换模式/换方向那处理器")..];
    let body = &choose[..choose.find("};").unwrap_or(choose.len())];
    assert!(body.contains("setChoice") && body.contains("onSelect("),
        "换那一格只记了选择、没把右列指过去：{body}");
    // 名字与它的框必须捆成一格（那一行放不下三格时只能整格换，实拍量过：
    // 「方向」留在上一行、它的框掉到下一行就是名字与值分家）。三枚下拉各一颗。
    //
    // 2026-10-08 简化那一轮换掉了实现，但**没换掉这条意图**：原来是三颗 `wrap={false}` 的
    // `Space`（标签在框左边），那一轮改成三枚框横跨整窗、标签**在上方**，靠 `.pair-field` 那个
    // 纵向 flex 把两者钉在同一格里。横向 Space 那一套随左列一起搬走了 ——
    // 340px 装不下三枚横向的框，它们本来就被折成了三行（每行各一枚，名字与框仍然同格）。
    // 所以这里钉的换成"三枚各在一颗纵向 Field 里"：标签与框仍在同一个盒子，名字与值不会分家。
    // 2026-10-09（模式+方向并成一枚之后）这里从三枚变两枚，**要守的性质没变**：
    // 每一枚框仍然各自被捆进一颗纵向 Field，标签与框不会分家。
    assert_eq!(count_in(&list, "<Field label="), 2, "两枚下拉没有各捆成一格：{list}");
    let field = &list[list.find("function Field(").expect("找不到那颗纵向捆扎的 Field")..];
    let field = &field[..field.find("\n}").unwrap_or(field.len())];
    assert!(field.contains("pair-field") && field.contains("label") && field.contains("{children}"),
        "Field 没有把标签与控件放进同一个盒子（名字与值会分家）：{field}");
}

/// 「换一对位置」那颗按钮：跑的那一面就能立一张卡，不必先去「设置」写一条再重开面板。
///
/// 判据落在**它站在哪一行**上，不只是"这个字在文件里出现过"：它如果掉到每一格里，
/// 一列位置就成了 N 颗同名按钮（操作员要的是"少几条目"，不是"多一屏按钮"），
/// 而 spec §6 明写它是那一面的一条全局动作。
/// 锚点跟着上一版换过：原来那一段从「只看」那个标签起、到第一列卡
/// （`<Listy<PairGroup>`）还没为止；现在那一列卡没了，改成"顶部那一行那几枚下拉之内、
/// 当前那一格之前"。
#[test]
fn the_pairs_face_has_one_button_to_pick_a_new_pair() {
    let t = strip_comments(&read("src/components/JobListView.tsx"));
    let tight = t.split_whitespace().collect::<Vec<_>>().join("");
    let clip = tight.chars().take(200).collect::<String>();
    assert!(tight.contains("换一对位置"), "跑的那一面没有立卡的那一颗：{clip}");
    let pos_at = tight.find("aria-label=\"位置\"")
        .unwrap_or_else(|| panic!("找不到位置那一枚下拉：{clip}"));
    let cell_at = tight.find("renderCell(shown)")
        .unwrap_or_else(|| panic!("找不到当前那一格那一段：{clip}"));
    let row = &tight[pos_at..cell_at];
    let row_head = row.chars().take(200).collect::<String>();
    assert!(row.contains("换一对位置"), "那颗不在选择条那一行里：{row_head}");
    assert_eq!(tight.matches("换一对位置").count(), 1, "这一颗在界面上不止一处：{clip}");
    // 它自己不立卡：立卡那一次调用与它带回来的那句拒绝都住在**这一层**里（`PairDraftLayer`），
    // 外面那一层只管开与关 —— 两处发起点就有两处话术。
    assert!(tight.contains("onNewPair()"), "那颗没接到外面：{clip}");
    assert!(!tight.contains("pairDraft"), "这一面自己去立卡了：那是第二处发起点");
}

/// 「换一对位置」那一层：两行位置、候选来自后台、拒绝写在层里。
///
/// 三条各钉一件事：
/// - 候选必须由后台交（`browser_remotes` 那一份，与浏览器那面同一来源）。前端列常量
///   就是第二份"你有哪些位置"的口径，而它会漂到操作员看不见自己真在用的那朵云。
/// - 路径框是**可编辑**的：候选只到根，而常要用的是根下面那一层（`r2:img/ideahub`）。
/// - 拒绝落在层里，不是只飘一条 toast：与对齐确认层同一条口径——看不见的拒绝等于没有拒绝。
#[test]
fn the_draft_layer_takes_two_positions_and_shows_the_refusal_itself() {
    let t = strip_comments(&read("src/components/PairDraftLayer.tsx"));
    let tight = t.split_whitespace().collect::<Vec<_>>().join("");
    let clip = tight.chars().take(240).collect::<String>();
    assert!(tight.contains("api.browserRemotes("), "位置候选不是问后台要的：{clip}");
    // 两行各有"一个选择框 + 一个可编辑的框"。判据落在**一行那个 helper 被调了两次**上，
    // 不是"文件里躺着两份 `<Input>`"：抄两份就是两份会各自漂的排版，而同一个层里的两行该同形。
    assert_eq!(count_in(&tight, "<Input"), 1, "路径框被抄成两份（一行一份、由同一个 helper 画）：{clip}");
    assert_eq!(count_in(&tight, "<Select"), 1, "位置选择框被抄成两份：{clip}");
    for label in ["这一侧", "那一侧"] {
        assert!(tight.contains(&format!("row('{label}'")), "那一行没被画出来：{label}");
    }
    // 可及名由行标签带进那两个框（读屏里不许剩"编辑框 1/2"这种无名控件）。
    assert!(tight.contains("aria-label={label}"), "框没有可及名：{clip}");
    assert!(tight.contains("就这么定"), "确认那颗用的不是已有词表里那一颗：{clip}");
    assert!(tight.contains("取消"), "缺一颗退出的：{clip}");
    assert!(tight.contains("api.pairDraft("), "那颗按下去没人去立卡：{clip}");
    // 拒绝要在层里说，而且说的**是这一次立卡的拒绝**（上面那枚"候选没取到"的错误不算：
    // 第一稿只判"文件里有 `<Alert type=\"error\"`"，拿枪把这一枚整个删掉它照样绿——那是假钉子）。
    let at = tight.find("{error&&")
        .unwrap_or_else(|| panic!("立卡被拒那一句不在层里说，只飘了 toast：{clip}"));
    let seg: String = tight[at..].chars().take(220).collect();
    assert!(seg.contains("<Alerttype=\"error\""), "那条拒绝没走错误那一档：{seg}");
    assert!(seg.contains("note={error}"), "错误那一枚显示的不是这一次拒绝：{seg}");
    // 具体远端名不许写进这一层：那是一份会漂的位置清单（操作员加一朵云就少一个候选）。
    for banned in ["r2:", "qiniu", "decklocal"] {
        assert!(!t.contains(banned), "这一层里写死了位置（{banned}）");
    }
    // 也不许自己拼端点：框里写的是什么就交什么（拼 '/' 与补冒号是第二处解释同一个坐标系）。
    for spliced in ["+\"/\"", "+'/'", ".id+\"/\"", ".id+'/'+"] {
        assert!(!tight.contains(spliced), "这一层在拼路径（{spliced}）：交出去的东西应当就是框里那几个字");
    }
}

/// 两边接线的归属：层由 Panel 拿着，**"这张卡现在说的是哪一格"仍然只有 JobListView 一份判据**。
///
/// 最后那一条是这一片真正的危险：立完卡要把右列指过去，而"指过去"要挑一格。
/// Panel 若为了挑那一格去读 `defaultChoice`/`slotJob`，同一对位置就有两处选择口径，
/// 漂开的那一次就是卡上写着「只放上去」、右列演的是「会删除」——量到过的那个形状。
#[test]
fn the_panel_holds_the_layer_but_the_card_still_owns_its_cell() {
    let panel = strip_comments(&read("src/Panel.tsx"));
    let list = strip_comments(&read("src/components/JobListView.tsx"));
    let tight = panel.split_whitespace().collect::<Vec<_>>().join("");
    let clip = tight.chars().take(240).collect::<String>();
    assert!(tight.contains("<PairDraftLayer"), "那一层没挂进面板：它开不了");
    // 立卡那一次调用与它带回来的那句拒绝**住在层里**（与 `BrowserPanel` 的命名层同一个主人）：
    // 面板若也去调一次，就有两处发起点，而拒绝只会在其中一处出现。
    assert!(!tight.contains("api.pairDraft("), "面板自己去立卡了：那一趟与该说的拒绝要在层里说话");
    // 成功之后要把右列指过去（新卡立在看不见的那一列里，右列还讲上一张卡的事 = 操作员会
    // 对着两张卡按下一颗按钮）。
    assert!(tight.contains("onCreated"), "层立完了卡没人接：{clip}");
    assert!(tight.contains("focusPair"), "新卡没有落脚处：右列还讲上一张卡的事");
    for not_here in ["defaultChoice", "slotJob("] {
        assert!(!tight.contains(not_here),
            "面板自己挑那一格了（{not_here}）：那一判归卡面那层，两处各挑一次就会各说一套");
    }
    // 卡面那一层接得住这个指点：它读 focusPair，并用**自己那份** `pick()` 换成任务 id。
    assert!(list.contains("focusPair"), "卡面不认这个指哪：新卡立起来而右列不动");
}

/// 临时卡说清自己是临时的，并且**只有它能**被存成常用或删掉。
///
/// 那三处的判据都读后台带出来的那枚 `isDraft`（上一证人钉的是标志本身，这条钉的是界面上
/// 真的按它摆）：
/// - 「重开面板就没了」那一句（R96）不许只记在我们心里；
/// - 「存成常用」只在**当前这一格**是临时的那一条上（配置格本来就常用着）；
/// - 「删掉这张临时卡」收的是这一张卡上那几格临时的，所以整张卡有临时格才摆得出来（R98 那句
///   "正在跑就拒"由后台说，界面不设备用文案）。
#[test]
fn a_draft_card_says_its_temporary_and_offers_the_two_exits_only_there() {
    let t = strip_comments(&read("src/components/JobListView.tsx"));
    let tight = t.split_whitespace().collect::<Vec<_>>().join("");
    let clip = tight.chars().take(240).collect::<String>();
    assert!(tight.contains("重开面板就没了"), "那句没说：那张卡读起来就像已经存好了：{clip}");
    assert!(tight.contains("存成常用"), "少那颗把这一格送去已有表单的：{clip}");
    assert!(tight.contains("删掉这张临时卡"), "少那颗删卡的：{clip}");
    // 两处各有各的判据：一颗看**当前那一格**，一颗看**这一张卡上有没有**临时格。
    assert!(tight.contains("job.isDraft"), "「存成常用」没读当前那一格那枚标志：{clip}");
    assert!(tight.contains("foldable"), "「删掉这张临时卡」没读整张卡那枚标志：{clip}");
    // 两处各有各的判据：那一句挂在这张卡**有没有**临时格上，那一颗挂在**当前这一格**是不是临时的上。
    // 钉形状而不是只钉"读没读那两枚标志"：把句子改成常驻，配置卡就被说成没存进配置（操作员会
    // 去点「存成常用」存一条已经存过的），那是这句话唯一另一种错法。
    assert!(tight.contains("g.hasDraft&&("), "那句挂在别的东西上或干脆常驻：配置卡也被说成临时的：{clip}");
    assert!(tight.contains("job.isDraft?["), "那颗没挂在当前这一格上：{clip}");
    // 「删掉这张临时卡」交回的是那把键（后台算的那一份），不是任务 id：一张卡上那几格一起收。
    assert!(tight.contains("onFoldPair(job.pairKey)"), "收的是哪一张卡说不清：{clip}");
    // 卡面上不许长出第二条写得进配置的路：那颗只把那一格交出去（R97）。
    for banned in ["settingsApply", "settingsPlan", "settings_apply", "settings_plan"] {
        assert!(!tight.contains(banned), "卡面上长出了第二条写得进配置的路（{banned}）");
    }
}

/// 那一枚标志是后台判的，界面只渲染（裁定 R69 的同一条口径落在临时卡上）。
/// 缺任何一处接线，剩下的那一处就开始猜——而猜的方向是把一条真配置项当成临时卡收掉。
#[test]
fn the_draft_flag_travels_as_data_and_nobody_reguesses_it() {
    let drafts = strip_comments(&read("src-tauri/src/runner/drafts.rs"));
    assert!(drafts.contains("pub fn is_draft(&self, id: &str) -> bool"),
        "后台没有那一条查询：这一判没地方长");
    let state = strip_comments(&read("src-tauri/src/state.rs"));
    assert!(state.contains("pub is_draft: bool"), "JobView 没带出那枚标志");
    let cmds = strip_comments(&read("src-tauri/src/commands.rs"));
    assert!(cmds.contains("runner.is_draft("), "get_jobs 没把这一判交给后台");
    let ts = read("src/types.ts");
    assert!(ts.contains("isDraft: boolean"), "TS 形状里没有它：前端只能自己猜");
    // 分组那一层对它的处理只能是"成员里有人是临时的"这一句或：判据本身不许在这里重算。
    let pair_ts = strip_comments(&read("src/pairs.ts"));
    assert!(pair_ts.contains("j.isDraft"), "那一层的 hasDraft 不是从成员那枚标志或出来的：那是第二份判据");
    // 编号前缀是我们内部的说法（日志文件名要靠它认卡），不是给界面用的判据。
    for f in ["src/components/JobListView.tsx", "src/pairs.ts", "src/Panel.tsx"] {
        assert!(!strip_comments(&read(f)).contains("adhoc"), "{f} 在按编号前缀认临时卡");
    }
}

/// 「存成常用」**不许长出第二条写得进配置的路**（R97）：它只把那一格交进设置面已有的那张表单。
/// 判据钉在三处：卡面不喊设置面的命令、面板只切面 + 预填、设置面接住那份预填并说出那句实话。
#[test]
fn saving_a_draft_card_reuses_the_existing_settings_form() {
    let card = strip_comments(&read("src/components/JobListView.tsx"));
    for banned in ["settings_apply", "settingsApply", "settings_plan", "settingsPlan"] {
        assert!(!card.contains(banned), "卡面上长出了第二条写得进配置的路（{banned}）");
    }
    let panel = strip_comments(&read("src/Panel.tsx"));
    assert!(panel.contains("prefill"), "面板没有把那一格交给设置面：「存成常用」点了就是静默");
    assert!(!panel.contains("settingsApply"), "面板自己去落盘了：那条路只归设置面");
    let settings = strip_comments(&read("src/components/SettingsPanel.tsx"));
    assert!(settings.contains("prefill"), "设置面不接那份预填：那张表单不会被打开");
    // 那句实话（R97）：存完之后这一面还是那张临时卡，重开面板它才作为配对卡出现。
    assert!(settings.contains("它才作为一张配对卡出现"), "存成常用之后那句实话没说：{settings}");
}

// ==== 那一面收成"三个下拉 + 当前那一格"（操作员那句：
//      「我希望有多个下拉可以选，而且相同属性的下拉应该只有一个」）
//
// 实测的形状是病根：夹具里铺五对位置，屏上就数到 **11 个下拉**（`只看` 一个 + 每张卡自带
// 模式/方向 各一个），一列下来 `只放上去（不删）` 重复四遍，两侧都本地的那一张还在方向框里
// 塞了两条完整绝对路径。下面三条钉的是改完之后必须成立的三件事，先立起来让它们红。

/// 那一面**每个属性只有一个下拉**：位置一枚、**要做什么**一枚，卡列表整段撤掉。
/// 「只看」那颗跟着没了 —— 它的活（从一堆里挑出一对）现在由位置下拉干，
/// 留着就是第三枚下拉，而且挑出来的还是看不见的那一列。
///
/// **2026-10-09：三枚并成两枚。** 「模式」+「方向」两枚的话要**拼起来**才是这一格的真名，
/// 而拼错拼出来的仍是一句通顺的话 —— 读者没有任何地方能发现自己拼错了。改成一枚「要做什么」
/// 之后，一个选项自己就是一个完整意图（「只补，不删 —— 从本地补到云端」）。所以这条的
/// 判据从"三枚"改成"两枚"，**要守的性质没变**：同一属性仍然只有一枚下拉。
#[test]
fn the_pairs_face_has_one_selector_per_attribute_and_no_card_list() {
    let t = strip_comments(&read("src/components/JobListView.tsx"));
    for name in ["位置", "要做什么"] {
        let anchor = format!("aria-label=\"{name}\"");
        assert_eq!(count_in(&t, &anchor), 1,
            "「{name}」那一枚下拉不止一个（相同属性的下拉就该只有一个）：{t}");
    }
    // 反向钉：并成一枚之后，**旧的两枚不许悄悄长回来**（它们各自都会被当成新的属性）。
    for gone in ["模式", "方向"] {
        let anchor = format!("aria-label=\"{gone}\"");
        assert_eq!(count_in(&t, &anchor), 0, "「{gone}」那一枚又长回来了：{t}");
    }
    // 卡列表整段没了：不是"留着但藏起来"。
    assert!(!t.contains("items={pairRows}"), "配对那一段还在按一列卡摆：{t}");
    assert!(!t.contains("<Listy<PairGroup>"), "卡列表只是换了变量名，还挂着：{t}");
    assert!(!t.contains("useState<Filter>"), "「只看」那枚筛选器回来了（它就是第四枚下拉）：{t}");
    // 对账那一小段仍然在，而且排在当前那一格**之后**：能删东西的那一格先看见，
    // 只读的对账垫底（这条判据从"两段卡的先后"搬到这里，没丢）。
    let cell = t.find("aria-label=\"位置\"").expect("位置那一枚下拉不见了");
    let gates_at = t.find("items={gates}").unwrap_or_else(|| panic!("对账那一段没了：{t}"));
    assert!(cell < gates_at, "对账排到了当前那一格前面：{t}");
    // 两段都是真的按数据切出来的：配对走 groupJobs 的产物，对账走它交回的 gates。
    assert!(t.contains("groupJobs(jobs)"), "没走 groupJobs：那一列任务就还是各自一家：{t}");
    assert!(t.contains("var(--kind-warm)"), "会删那一档没有走暖色（色板里的哪一枚？）：{t}");
    // 2026-10-09：危险程度那一笔颜色跟着「要做什么」一起搬进了 `pairs.ts` 的 `intentOptions`
    // （一枚框里两档并排，颜色是唯一能一眼分出"哪两项会删"的地方，所以它必须只有一处出处）。
    // 判据跟着搬：这一面**读** `intentOptions` 给的 `tone`，而那一处**只有一枚**暖色。
    assert!(t.contains("current?.tone") || t.contains("current.tone"),
        "选择条没有读 intentOptions 给的 tone：颜色要重新变成这一面自己算的第二份口径：{t}");
    let pairs_src = read("src/pairs.ts");
    let body = &pairs_src[pairs_src.find("export function intentOptions").expect("找不到 intentOptions")..];
    assert_eq!(count_in(body, "var(--kind-warm)"), 1, "暖色在 intentOptions 里不止一处：{body}");
}

/// 位置那一枚下拉里，每一项说的是**那一对**，而且说的是认得出的短名：
/// 本地那一侧取目录名，远端那一侧原样（`fxa:c` 本来就短）。两侧同型时方向那一句
/// 也走同一份短名 —— 以前它把两条绝对路径塞进一个窄框，量出来放不下。
/// 撞名要自己往上加层：`a/img` 与 `b/img` 短名都是 `img` 时，读的人分不出是谁。
#[test]
fn the_position_selector_labels_each_pair_by_a_short_unique_name() {
    let pair_ts = strip_comments(&read("src/pairs.ts"));
    assert!(pair_ts.contains("aLabel") && pair_ts.contains("bLabel"),
        "那一份短名没作为数据带出去：卡面与方向句只能各自现编（那是第二份口径）");
    assert!(pair_ts.contains("fn pairLabels") || pair_ts.contains("function pairLabels"),
        "没有那一处统一算短名的地方：撞名退回长写这条规则就没人管");
    // 撞名要加深，不是一路用短名到底。
    assert!(pair_ts.contains("depth"), "短名没有'撞了就再上一层'那一档：{pair_ts}");
    // 钉的是**离开那一圈的唯一出口是"比过这一批短名不重样"**，不是"`Set(` 这个字还在文件里"：
    // 后者拦不住"把比较结果存进一个变量、然后无条件 return"那一发（串还在原地，规则已经没了）。
    assert!(pair_ts.contains("=== keys.length) return"),
        "短名加深那一圈不再由'比过重样'决定什么时候收（撞名也只用目录名）：{pair_ts}");
    assert!(pair_ts.contains("g.aLabel = g.a"),
        "层数用尽没退回完整路径（认不出是谁会点错家）：{pair_ts}");
    // 方向那一句的两侧同型那一支读的是短名，不是原始路径。
    let dir_from = pair_ts.find("export function directionLabel").expect("找不到 directionLabel");
    let dir = &pair_ts[dir_from..];
    assert!(dir.contains("slot === 'a' ? g.aLabel : g.bLabel")
        && dir.contains("slot === 'a' ? g.bLabel : g.aLabel"),
        "方向那一句不再按槽位读那两枚短名（或那两句不再互为镜像）：{dir}");
    assert!(!dir.contains("? g.a :") && !dir.contains("? g.b :"),
        "方向那一句又去读整条原始路径了（那就是窄框里塞两条绝对路径）：{dir}");
    // 短名是**说法**，不是判据：认路径在哪一侧仍然只读后台那两枚标志。
    assert!(!pair_ts.contains(".match(") && !pair_ts.contains("RegExp"),
        "pairs.ts 里出现了路径正则：哪一侧是本地该读 aIsLocal（R69 那条禁令仍然算数）");
    // 短名只说**一遍**：位置那一枚下拉的每一项就是它；格子里不再复述一次标题，
    // 而是交出完整那两条路径（挑错一家靠全文认，不靠猜那一小截）。
    let view = strip_comments(&read("src/components/JobListView.tsx"));
    assert!(view.contains("${p.aLabel} ↔ ${p.bLabel}"), "位置那一项不是用短名摆的：{view}");
    assert!(!view.contains("<span>{g.aLabel}"), "格子里又把短名复述了一遍（下拉刚说过）：{view}");
    assert!(view.contains("job.src") && view.contains("job.dst"),
        "短名换掉了标题，但完整那一对位置没在这一格里写出来：{view}");
    // 被占着那一档：**两枚框照选照点**。仓里那条判据是"要人听得见后台那句为什么，
    // 而不是点不动又没人说话"（那颗「删掉这张临时卡」同一条），灰的只该是**那颗动作**。
    // 锚点跟着 2026-10-08 那一轮换过：`selectorRow` 那个函数没有了，那几枚框现在直接写在
    // `JobListView` 里（它就是那条选择条本身），下一段是 `PairCell`。
    let row = &view[view.find("export function JobListView").expect("找不到那一条选择条")..];
    let row_end = row.find("export function PairCell").unwrap_or(row.len());
    assert!(!row[..row_end].contains("disabled={locked}"),
        "选择条那两枚框因为'这一格被占着'被灰掉了（越了那条判据）：{row}");
}

/// 永远有一格被选中：默认站在第一对上，换位置/换模式/换方向都跟着把右列指过去。
/// 「先选一个任务」那一档在这一面不该可达（列表撤了之后，没有'没选'这个状态）。
#[test]
fn a_cell_is_always_selected_and_the_rail_follows_it() {
    let t = strip_comments(&read("src/components/JobListView.tsx"));
    // 没选过的时候有默认：第一对。
    assert!(t.contains("?? pairs[0]"), "没有'还没选过就站在第一对'那一档：{t}");
    // 当前那一格算出来之后要把右列指过去（点按钮先立起那张卡那条判据的延续）。
    assert!(t.contains("onSelect(picked.job.id)"), "选中态没跟着这一格走：右列会讲上一家的事：{t}");
    // 那一格不存在了（配置在两次轮询之间被改过）要退回默认档，不许静默空着。
    assert!(t.contains("defaultChoice(g)"), "缺档时没有退回默认那一格：{t}");
    // 仍然不许把选择记进持久层（记住一个会删东西的选择 = 第一次点击落在最重那一档）。
    for w in ["localStorage", "sessionStorage", "document.cookie"] {
        assert!(!t.contains(w), "选择走了持久层（{w}）：模式必须每次从「只补，不删」开始");
    }
}
