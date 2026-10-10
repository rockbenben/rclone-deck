//! 浏览器视图与确认层的证人。口径与 `frontend_mirror.rs` 相同：本仓没有 JS 测试运行器，
//! 所以这些证人读**源文本**，真窗口里的那一份在验收任务里量。
//!
//! 这里钉的是两类会咬人的东西：
//! ① 界面把后台已经做完的判定**再算一遍**（规则条数、清单条数、比例）——算出来的那份是第二口径，
//!    而它错了没人知道（裁定 I5/R74）；
//! ② 界面上长出"覆盖"这一档。它不是审美问题：F10 量到 rc 对撞名回 200，然后按"大小 + modtime"
//!    决定跳过还是盖掉——一个"覆盖"开关背后是不可预期的丢数据（I2c/R73）。
//!
//! ④片把那一面拆成 `BrowserPanel`（编排：状态与判定接线）/ `BrowserPane`（一栏怎么摆）/
//! `LayerConfirm`（确认层）/ `ResultDock`（栏脚结果）。下面每条都按**内容归属**读那一份文件；
//! 跨两半的（布尔在一处算、句子挂在另一处）就两份都读、两边各钉一半 —— 拆分没有放松任何一条。

mod common;

use common::{count_in, read, strip_comments};

/// 拆开之后一件事常跨两份文件（判定在编排层算、形状在栏里摆）。两份一起扫，
/// 禁的那族写法在**任何一份**里冒出来都红。
fn both(pane: &str, panel: &str) -> String {
    format!("{}{}", read(pane), read(panel))
}

#[test]
fn the_browser_shows_the_backend_note_and_never_recomputes_a_verdict() {
    let pane = read("src/components/BrowserPane.tsx");
    // 两句都要：`listing.filteredNote` 钉这一份读的是后台那个字段（不是自己数规则），
    // `{listing.filteredNote}` 钉它**真的被渲染到了那一格上**。只钉前者的话，
    // 把渲染换成前端自己拼的一句固定串也不会红——那句 `filteredNote` 还在禁用提示里活着。
    assert!(pane.contains("listing.filteredNote"), "I8 那句话必须由后台给：{pane}");
    assert!(pane.contains("note={listing.filteredNote}"), "后台那句没有落在界面上，只在代码里被读过一次：{pane}");
    let t = both("src/components/BrowserPane.tsx", "src/components/BrowserPanel.tsx");
    assert!(t.contains("listing.canClearPrefix"), "能不能清空是后台的判定（R74），不许这里数规则：{t}");
    // 与 frontend_mirror.rs 同一条口径：压掉空白再扫，禁的是**动作**不是某一种拼写。
    let tight = t.split_whitespace().collect::<Vec<_>>().join("");
    for banned in ["doomed.length==", "doomed.length!=", "doomed.length>", "doomed.length<",
                   "entries.length==", "entries.length>", "Math.abs", "exclude.length"] {
        assert!(!tight.contains(banned), "浏览器在重算后台已经给出的判定（{banned}）：{tight}");
    }
    assert!(!t.contains("listed"), "`listed` 永不上界面");
}

/// 那一面**不许自己数对象**：`totalCount` 是后台数好的（只数文件、不数目录行），
/// 而 `entries.length` 把目录行也算进去 —— 于是「删掉这一层看到的 N 个」会报出一个比 doomed
/// 集合更大的数，那颗按钮点下去删的又不是那个数（终审抓的正是这一格：换一种拼写，
/// 上面那条禁比较式的证人一条都不红）。禁的是"读行数的这个动作"，不是某一种写法。
#[test]
fn the_visible_count_on_the_button_is_the_backends_number_not_a_row_count() {
    let t = both("src/components/BrowserPane.tsx", "src/components/BrowserPanel.tsx");
    assert!(t.contains("listing.totalCount"), "按钮上那个数必须由后台交出：{t}");
    let tight = t.split_whitespace().collect::<Vec<_>>().join("");
    // 换一种拼法也一样错：可选链、换个变量名、先 filter 再数。禁的是"数行数"这一族动作。
    for banned in ["entries.length", "entries?.length", "dataSource.length", "rows.length", "filtered.length"] {
        assert!(!tight.contains(banned),
            "数自己手里那份行数当\"看到的个数\"（{banned}）：{tight}");
    }
}

/// R54 + R74 那一格：`清空这一层` 点不动时，那句原因必须是**后台那一句**，而且它要挂在
/// disabled 按钮收得到悬停的那个形状上（Chromium 不给 disabled 控件派发 hover，见 `LockHint`）。
///
/// 这一条是评审轮补的：把 `<LockHint title={clearHint}>` 换成裸按钮、或把禁用条件从 `clearHint`
/// 换成 `busy`，前面几条**一条都不红** —— 那两个字段仍在别处各被读一次，够第一条绿。
/// "置灰 + Tooltip 里那句后台的话"这半条设计当时只写在证据文件里，没有闸门。
/// ④片拆成两份之后仍然两头各钉一半：布尔与句子都在编排层从后台字段读出来，
/// 而挂 Tooltip 与 `disabled` 的那个形状在栏里。
#[test]
fn the_greyed_clear_button_carries_the_backends_own_reason() {
    let panel = read("src/components/BrowserPanel.tsx");
    // 灰读布尔、Tooltip 读句子，**两者互不推断**：从 `canClearPrefix` 反推那句"为什么点不动"，
    // 等于假设"不能清空只可能是因为排除规则"，而站在根上那一档同样是不能（F12）。
    assert!(panel.contains("!listing.canClearPrefix"), "点不动与否必须由后台那个布尔决定：{panel}");
    assert!(panel.contains("listing.clearNote"), "点不动那一句必须由后台交出，不在这里拼：{panel}");
    let pane = read("src/components/BrowserPane.tsx");
    assert!(pane.contains("<LockHint title={noteTip(clearNote)}>"), "灰掉的那颗没挂上共用那只 Tooltip：{pane}");
    assert!(pane.contains("disabled={v.busy || v.clearBlocked}"), "那颗按钮的禁用不是由后台的判定决定的：{pane}");
    // 反面：把后台那个布尔再解释一遍（数它的长度、比它的真假、或拿句子当判定）。
    let t = both("src/components/BrowserPane.tsx", "src/components/BrowserPanel.tsx");
    let tight = t.split_whitespace().collect::<Vec<_>>().join("");
    for banned in ["canClearPrefix==", "canClearPrefix!=", "filteredNote.length", "clearNote.length",
                   "canClearPrefix?'':", "clearNote===''"] {
        assert!(!tight.contains(banned), "后台的判定在这里又被算了一遍（{banned}）：{tight}");
    }
}

#[test]
fn the_destructive_confirm_layer_and_the_view_switch_use_antd_components() {
    // 用户的明令：保留 antd 的标准组件，不自绘浮层替代 Modal/Tooltip 那一类。
    // 钉的是 **JSX 里的那一个**（`<Modal`）而不是组件名出现在文本里：只 `contains("Modal")` 的话，
    // 一行 `import { Modal }` 就能让它绿，而把浮层换成自绘 div 也不会红。
    let layer = read("src/components/LayerConfirm.tsx");
    assert!(layer.contains("<Modal"), "确认层不是 antd 的 Modal：{layer}");
    let pane = read("src/components/BrowserPane.tsx");
    for want in ["<Table", "<Tooltip"] {
        assert!(pane.contains(want), "那一栏少了 {want}：{pane}");
    }
    // 灰控件那句原因挂在共用的那只 antd Tooltip 上（Chromium 不为 disabled 控件派发 hover）。
    let hint = read("src/components/LockHint.tsx");
    assert!(hint.contains("<Tooltip"), "禁用原因是自绘浮层而不是 antd Tooltip：{hint}");
    // 分段控件住在 `Panel.tsx`：视图选择是面板那一层的事，
    // 钉在 BrowserPanel 里就等于钉一个永远不会出现在那里的组件名。
    // 标签说的是**这一面在干什么**（「对子」是黑话，「浏览器」第一眼像网页浏览器）。
    let panel = read("src/Panel.tsx");
    for want in ["<Segmented", "配好的位置", "浏览文件", "设置"] {
        assert!(panel.contains(want), "视图切换少了 {want}（档位名要说这一面在干什么，不许换回内部名词）：{panel}");
    }
    // 档数钉在**三**。这一条以前钉的是二：第四档「作业」当时被拦下，理由是"没有字段形状可读"
    // （spec §9）。现在多出来的那一档是「设置」，它有已批准的 spec（子项目 5，settings-view），
    // 所以判据跟着改到三，而不是松成"至少两档"那种永远不会红的形状。
    assert_eq!(count_in(&panel, "value: '"), 3,
        "档数不是三档了：再多一档得有自己的已批准 spec，不许悄悄长出来");
    // 那句"这是过滤后的视图"必须由**后台**那句给（同族第一条），这里只渲染。
    assert!(pane.contains("filteredNote") && !pane.contains("按面板的"),
        "界面里不该抄一份后台的文案：{pane}");
}

/// 栏上那句读数说「N 个文件 · M 个文件夹」，不再说「N 个对象」—— 数的是文件、行里却排着
/// 目录行，那句话一直在骗人（子项目 3 记下的缺口）。两个数必须由后台多交出来，
/// 前端不许自己数行（同一条禁 `entries.length` 的证人守着）。
/// 另一半：还没列出来时那颗按钮不许摆一个字母 `N` 给人看 —— 那是工程占位符，不是人话。
#[test]
fn the_pane_header_says_files_and_folders_not_objects() {
    // 先剥注释：这条禁的是界面上那句话，注释里提一句「N 个对象」是给人讲历史，不该红。
    let t = strip_comments(&read("src/components/BrowserPane.tsx"));
    assert!(t.contains("listing.fileCount") && t.contains("listing.dirCount"),
        "栏上那两个数没读后台的字段：{t}");
    let tight = t.split_whitespace().collect::<Vec<_>>().join("");
    assert!(!tight.contains("个对象"), "还在说「N 个对象」（数的是文件、排的是目录行）：{tight}");
    assert!(!tight.contains("看到的N个"), "按钮上还有那个字母 N：{tight}");
    assert!(t.contains("等这一层列出来才能删"), "那颗按钮空着时没说一句人话：{t}");
}

/// 「删掉这一层看到的 N 个」与「清空这一层（连里面的一起）」差一整棵子树：前者只动直接子项
/// （后台 `layer_files`，单层列举），后者是 `operations/purge`（后台 `prefix_files`，整棵前缀）。
/// 两颗都只写"这一层"的话，界面上看不出这个范围差 —— 而那正是操作员报上来的缺陷：
/// 列举还是递归的时候，"看到的 N 个"其实是整棵子树，按钮上的字和它动的范围不是一回事。
///
/// 另一半钉的是那句读数：单层之后 `totalBytes` 只是**直接子项**的大小，
/// 说成"这个前缀往下一共"就是把一个小数摆在大数的位置上。
#[test]
fn the_two_delete_buttons_each_say_their_own_range() {
    let t = strip_comments(&read("src/components/BrowserPane.tsx"));
    let tight = t.split_whitespace().collect::<Vec<_>>().join("");
    assert!(tight.contains("这一层{listing.fileCount}个文件"),
        "栏上那句读数没说清数的是哪一层：{tight}");
    assert!(tight.contains("删掉这一层看到的${listing.totalCount}个"),
        "删除那颗按钮没说自己的范围是这一层：{tight}");
    assert!(tight.contains("清空这一层（连里面的一起）"),
        "清空那颗没说它连文件夹里面的一起清掉（purge 删的是整棵前缀）：{tight}");
    // 读数不许再声称自己是"整个前缀往下"的总量。这两个词在剥掉注释之后不该出现
    // （今天唯一一处"一共"在注释里，讲的就是这件事）。
    for banned in ["一共", "总共"] {
        assert!(!tight.contains(banned),
            "界面上又出现了\"整个前缀往下一共\"那种说法（{banned}）——单层列举之后那个数只是这一层的：{tight}");
    }
}

/// 界面上不许有第二条"点一下就删"的路（R78）。
///
/// 判据是结构上的，扫的是 `api.ts` 里**每一个** `invoke` 的命令名：含破坏词的那一个必须是
/// `preview`（只发凭证，一个字节都不删），动手那条只能是 `browser_run`。
/// 命令层那一份同族证人在 `commands_surface.rs`；这一份钉的是前端有没有另开一条口子
/// —— 更早它就开着：`browserRmdir` 直接 invoke，点一下目录就没了。
#[test]
fn the_frontend_has_no_second_way_to_delete() {
    let api = strip_comments(&read("src/api.ts"));
    let mut names: Vec<String> = Vec::new();
    let mut rest = api.as_str();
    while let Some(i) = rest.find("invoke") {
        let after = &rest[i..];
        let Some(q) = after.find('\'') else { break };
        let Some(len) = after[q + 1..].find('\'') else { break };
        names.push(after[q + 1..q + 1 + len].to_string());
        rest = &after[q + 1 + len..];
    }
    // 反面半边：扫不出名字的话上面那圈就是空转，绿得没有意义。
    assert!(names.len() >= 10, "一个命令都没扫出来，这条证人在空转：{names:?}");
    for n in &names {
        let destructive = ["delete", "purge", "rmdir", "remove", "execute"].iter().any(|w| n.contains(w));
        if !destructive { continue; }
        assert!(n.contains("preview"),
            "前端有一条不经过预览就能删的路（{n}）：动手只能走 browser_run(token)");
    }
    assert!(names.iter().any(|n| n == "browser_run"), "动手那条入口不见了：{names:?}");

    // 删空目录那一颗确实接在确认层上（不是接在那个不发凭证的 `plain` 上）。
    let panel = strip_comments(&read("src/components/BrowserPanel.tsx"));
    let arm = panel.split("onRmdir:").nth(1)
        .unwrap_or_else(|| panic!("BrowserPanel 里找不到 onRmdir 那一支"));
    let arm = &arm[..arm.find("},").unwrap_or(arm.len())];
    assert!(arm.contains("openLayer") && arm.contains("api.browserPreviewRmdir"),
        "删空目录那一颗没走确认层：{arm}");
    assert!(!arm.contains("plain("), "删空目录那一颗还挂在不发凭证那条路上：{arm}");
}

/// 后台已经把拒绝拼成整句（撞名那句、动手前重列对不上那句、根与排除规则那两句、rmdir 非空的 rc 原文），
/// 前端只做展示。这一条钉的是"抄一份"这一族：界面里出现后台话术的任何一个片段，
/// 就说明这里有了第二份句子——后台改了措辞，界面上留着一句过期的解释。
///
/// 它**不**钉"确认后不关层"那一格：那要在真窗口里点一次撞名才看得见（Task 16 量），
/// 文本扫描造不出不骗人的证人。
#[test]
fn the_refusal_is_relayed_not_copied_into_the_ui() {
    let t = read("src/components/LayerConfirm.tsx");
    // 钉的是**只属于确认层那一个位置**的形状。先前这里钉 `<Alert type="error"`：那样式
    // 有三处（栏上列举失败、层里、名字框里），把层里那一整行删掉这条照样绿 —— 那是条假证人。
    // ④片之后那份句子从 `layerError` 变成层的 prop `error`，钉的还是同一件事：拒绝在层里渲染。
    // 错误话术分两层之后，那一份是 `{ short, detail }`，渲染走 `NoteLine`（摘要 + 「为什么？」）：
    // 只钉"渲染了 error"的话，把 detail 那半段丢掉也是绿的，所以两段都要钉到。
    assert!(t.contains("title=\"没改成\" description={<NoteLine note={error} />}"),
        "确认层里没有渲染拒绝的地方，或者那一份没走两段的渲染：{t}");
    assert!(t.contains("<Modal open={open}"), "那句拒绝没有挂在层里：{t}");
    let tight = t.split_whitespace().collect::<Vec<_>>().join("");
    for banned in ["已经有一个同名对象", "一个都没动", "不能打在"] {
        assert!(!tight.contains(banned), "界面抄了一份后台的拒绝句子（{banned}）：{tight}");
    }
}

/// 后台把"这一档有没有一份被核对过的清单"作为数据交出来（`RunReport.has_list`），而那一档不许
/// 出现"两个数一致"这半句 —— 把没核对过演成核对通过，是这一面最贵的一种说错话。建目录
/// 与一次复制走的都是这一档（真窗口里量到的是「清单上 0 个，后台报做完 0 个，两个数一致」）。
/// 删空目录**不再**在这一档里：它进了凭证路，清单上就是那一个目录名。
///
/// 钉的是**渲染表达式本身**（`{report.hasList && (`），不是"这句话出现在文件里"：注释里写一遍
/// 旧判据不该让这条变绿。反面那三条禁的是 TS 替后台做决定 —— 比那两个数、或从数上发明颜色。
#[test]
fn the_report_says_the_two_numbers_only_when_there_actually_was_a_list() {
    let t = read("src/components/BrowserPanel.tsx");
    assert!(t.contains("{report.hasList && ("), "那一档的分支不是由后台那个布尔决定的：{t}");
    assert!(t.contains("清单上 {report.confirmed} 个"), "那两个数没被渲染（证人要钉的是渲染，不是读取）：{t}");
    let tight = t.split_whitespace().collect::<Vec<_>>().join("");
    for banned in ["confirmed===0","confirmed>0","confirmed!==0","done===confirmed","report.done===report.confirmed"] {
        assert!(!tight.contains(banned), "TS 在替后台算那份核对（{banned}）：{tight}");
    }
    // 底色只读 `agrees`：没有清单那一档后台给的就是 true，TS 不需要再加一条规则。
    // ④片之后那一档经过 `tone` 一跳（编排层给 tone，`ResultDock` 把 tone 换成 Alert 的类型），
    // 所以两头都要钉：这里不许出现第二个判据，那里不许看见那两个数。
    assert!(tight.contains("report.agrees?('ok'asconst):('pending'asconst)"),
        "底色不再只读那个判定：{tight}");
    let dock = read("src/components/ResultDock.tsx");
    // 这一族禁的是"动作"，所以扫之前先剥注释：那份文件头的故事里就写着 `confirmed == 0`，
    // 拿它咬人是教人改注释，不是教人改代码（同一条口径见 `tests/common` 那段）。
    let dtight = strip_comments(&dock).split_whitespace().collect::<Vec<_>>().join("");
    assert!(dtight.contains("type={tone==='ok'?'success':tone==='refused'?'error':'warning'}"),
        "结果条的底色不再只读那一档：{dtight}");
    for banned in ["confirmed", "report.done", "doomed.length", "entries.length"] {
        assert!(!dtight.contains(banned), "结果条自己开始数东西了（{banned}）：{dtight}");
    }
}

/// 结果必须落在**发起它的那一栏脚下**，不是页面最底部。上一版它掉在视口外，
/// 我抄 DOM 读数时只抄了标题没抄正文，让一格真缺陷（`confirmed == 0` 却说"两个数一致"）
/// 混过了两轮评审 —— 所以这一格从此有闸门。
#[test]
fn the_result_of_a_pane_action_lands_at_the_foot_of_that_pane() {
    let dock = read("src/components/ResultDock.tsx");
    assert!(dock.contains("position: 'sticky'") && dock.contains("bottom: 0"),
        "结果条不是钉在这一栏脚下：{dock}");
    let pane = read("src/components/BrowserPane.tsx");
    assert!(pane.contains("<ResultDock"), "那一栏没挂结果条：{pane}");
    // 页面级那两条（一条常驻拒绝 + 一条报告）就是掉出视口的那对：拆到栏脚之后不许再长回来。
    let panel = read("src/components/BrowserPanel.tsx");
    for banned in ["<Alert showIcon type=\"error\" title=\"没改成\"", "type={report.agrees"] {
        assert!(!panel.contains(banned), "结果条又回到页面级了（{banned}）：{panel}");
    }
    // 而且那个结果**知道自己是谁发起的**：两栏共用一个槽位，没有 `side` 就会把左栏的结果摆到右栏脚下。
    assert!(panel.contains("result.side !== side"), "结果不再认它是哪一栏发起的：{panel}");
}

/// R57 那一族里最贵的一格：`message.error` 在 provider 摆错位置时**不是报错，是静默**
/// —— 全套类型与测试都绿，而界面对操作员一句话不说。runtime 的挂载顺序不在文本里，
/// 但**出事的那个形状在**：渲染 `<AntdApp>` 的那个组件自己同时调 `useApp()`。
#[test]
fn the_component_that_renders_the_provider_is_not_its_consumer() {
    // 只读 `export default function App` 之后的**函数体**。文件头那段注释讲的是这个形状的历史，
    // 拿整份文件扫会两头出错：注释里写出兄弟形状就假红，写出正确形状就假绿。
    let app = read("src/App.tsx");
    let from = app.find("export default function App").expect("App.tsx 里没有那个组件");
    let tight = app[from..].split_whitespace().collect::<Vec<_>>().join("");
    assert!(tight.contains("<AntdApp><Panel/>"), "provider 不再包着这一面：{tight}");
    assert!(!tight.contains("<Panel/><AntdApp>"),
        "`<AntdApp>` 又变成 `<Panel />` 的**兄弟**了：provider 不在消费者外面，antd 只给兜底上下文，\
         每一条 message.error 从此静默（真窗口里量过：toast 容器 0）：{tight}");
    // 消费者要在：只钉"形状对"不够，那把整个 toast 删掉也能绿。
    let panel = read("src/components/BrowserPanel.tsx");
    assert!(panel.contains("AntdApp.useApp()"), "这一面不再说拒绝的话了：{panel}");
}

/// 动成一次之后，上一次那句「这次没有改成」必须跟着作废。钉法是结构上的：
/// 成功与拒绝写的是**同一个槽位**（`result`），所以"报了成功却没退掉旧拒绝"那种形状
/// 在这份文件里根本摆不出来 —— 上一版它是两条 state（`report` + `actionError`），
/// 于是"写了报告忘了清拒绝"是写得出来、也真出过事的（删完三个对象之后界面说"什么都没改成"）。
#[test]
fn a_success_retires_the_previous_refusal() {
    let t = read("src/components/BrowserPanel.tsx");
    assert_eq!(count_in(&t, "setResult({ side, report:"), 1, "报告有了第二个写入口，作废那句就有人绕过去：{t}");
    assert_eq!(count_in(&t, "setResult({ side, refusal:"), 1, "拒绝有了第二个写入口：{t}");
    // 旧的两槽形状不许长回来：那正是"写了成功、忘了清拒绝"能成立的前提。
    assert!(!t.contains("setActionError"), "又变回两条 state 各写各的了：{t}");
    assert!(t.contains("const commit = useCallback"), "那两个动作不在同一个函数里：{t}");
    // 三个成功出口：不发凭证的那一批、走凭证的那一次、建目录。少一个 = 那一档不清旧拒绝。
    assert_eq!(count_in(&t, "commit(side, await"), 2, "少了一个成功出口走 commit：{t}");
    assert_eq!(count_in(&t, "commit(n.side, await"), 1, "建目录那一档没走同一个出口：{t}");
}

/// 拆分之前那四条界面改动各要一条闸门（当时它们**一条都不红**：删掉 `setPicked(null)`、
/// 删掉两处 `stopPropagation`、删掉 `far.listing` 判据、把那条 Alert 改回一条 toast，全绿）。
/// 钉法都是"这个形状只在这里出现一次"，多一处或少一处都红。
#[test]
fn a_success_retires_the_picked_source_and_the_row_buttons_do_not_select_it() {
    let t = read("src/components/BrowserPanel.tsx");
    // 退场这一件事必须**在写报告的那一个出口里**（换位置那一条是另一件事，它自己也要退场）。
    // 只钉"文件里出现过"不够：散到各个处理器里迟早漏一个，而那正是这一族的病。
    let from = t.find("const commit = useCallback").expect("BrowserPanel 里没有 commit 那个出口");
    let commit = &t[from..];
    let commit = commit.split("}, []);").next().unwrap();
    assert!(commit.contains("setPicked(null)"), "退场不在写报告的那一个出口里：{commit}");
    // 行里的按钮不冒泡到 `onRow`：两处动作格（目录行与文件行）都得有，缺一处就是半条规矩。
    let pane = read("src/components/BrowserPane.tsx");
    assert_eq!(count_in(&pane, "onClick={(e) => e.stopPropagation()}"), 2, "只有一格不冒泡：{pane}");
}

/// 「下载」的落点那一栏必须列得出来 —— 一栏标着「这一栏列不出来」的位置不是能说清落点的位置。
/// 判据要写在处理器里：`disabled` 是这一面今天恰好给的保护（与那四颗破坏性按钮同一条规矩）。
#[test]
fn the_download_landing_pane_must_have_a_listing_too() {
    let t = read("src/components/BrowserPanel.tsx");
    assert!(t.contains("if (here === null || there === null || far.listing === null) return;"),
        "处理器里的落点判据少了那一半：{t}");
    assert!(t.contains("const farReady = far.listing !== null && far.fs !== null;"),
        "提示那句不再看落点那一栏的读数：{t}");
}

/// 取不到位置那句要常驻，还要有一条不用重启面板就能再问一次的路；那颗按钮在请求回来之前按住
/// （连点会攒出一把并发请求，而先发的失败可能后到，把已经问成功的结果又盖回屏幕上）。
#[test]
fn a_failed_endpoint_lookup_stays_on_screen_with_one_way_to_retry() {
    let t = read("src/components/BrowserPanel.tsx");
    assert!(t.contains("{refsError !== null && ("), "取不到位置那句不再常驻：{t}");
    assert!(t.contains("disabled={refsLoading} onClick={loadRefs}"), "那颗重试按钮能连点：{t}");
    assert!(t.contains(".finally(() => setRefsLoading(false));"), "按下了没有放开的那一半：{t}");
    // 反面：退回"只飘一条 toast"。
    assert!(!t.contains("message.error(`取端点列表失败"), "又回到只飘一条 toast：{t}");
}

/// 界面文件递归扫，不点名。点名是一份**会过期的名单**：新增一个 `.tsx` 就自动免检，
/// 而这一条守的是 I2c/R73（撞名只能拒，因为 F10 量到 rc 对撞名回 200 再自己决定盖不盖）。
/// 禁词也不止一种说法：「跳过 / 替换」与「保留两份 / 强制」同样是那一档的入口。
/// 扫的是整份文件（**含注释**）：这些词写进注释也会红 —— 那是有意的 tripwire，
/// 换个人话措辞比留一条"这里不扫"的口子便宜。
#[test]
fn there_is_no_overwrite_choice_anywhere_in_the_ui() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut files = Vec::new();
    collect_ui(&root.join("src"), &mut files);
    assert!(files.len() >= 8, "递归扫 `src` 只找到 {} 个界面文件，扫描器本身坏了", files.len());
    for f in &files {
        let t = std::fs::read_to_string(f).unwrap_or_else(|e| panic!("读不到 {}：{e}", f.display()));
        // 词表与上面那句注释对齐：那一档的说法不止一种。这一条是**文案闸**——真正守住
        // "撞名只能拒"的闸门在后台（`refuse_if_target_exists` 与动手前那一次重列）。
        for banned in ["覆盖", "替换", "强制", "跳过", "盖掉", "保留两份", "overwrite", "force"] {
            assert!(!t.contains(banned), "{} 里出现了\"{banned}\"：撞名只能拒绝，界面上没有那一档：{t}",
                f.display());
        }
    }
}

fn collect_ui(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap_or_else(|e| panic!("读不到 {}：{e}", dir.display())) {
        let path = entry.expect("目录项读不出来").path();
        if path.is_dir() {
            collect_ui(&path, out);
        // 只扫 `.tsx`：那是会长出文案的地方。`.ts`（api/types/format）里出现那些词多半是注释
        // 或变量语义，扫它只会教会人把判据调松。
        } else if path.extension().map(|e| e == "tsx").unwrap_or(false) {
            out.push(path);
        }
    }
}

/// 空清单那一档在浏览器这层也要有自己的话，而不是「我知道，删这 0 个」配一句「这一步不可撤销」。
///
/// 0 是**真核对过、结果是零**（与 `RunReport.has_list` 同一族口径），不是"没得看"：
/// 可到达的那一档是"清空这一层而里面的东西全被排除规则挡着"。
/// 形状向对齐那层学（它早就有 0 那一档），复核那一句在两个分支外面继续说——
/// 空清单那一次照样要先过后台的重列核对。
#[test]
fn an_empty_list_in_the_browser_layer_says_there_is_nothing_to_move() {
    let t = strip_comments(&read("src/components/LayerConfirm.tsx"));
    assert!(t.contains("doomedCount === 0"), "这一层没有 0 那一档：{t}");
    assert!(t.contains("这一层没有要动的东西"), "0 那一档一个字都没说：{t}");
    let at = t.find("key=\"go\"").unwrap_or_else(|| panic!("页脚那颗批准的按钮不见了：{t}"));
    let arm: String = t[at..].chars().take(400).collect();
    assert!(arm.contains("这一份没有要动的，照它跑一次"),
        "0 那一档那颗还写着「删这 0 个」：{arm}");
    // 没清单那一档也不许演成"删这 0 个"（它连清单都还没有）。
    assert!(arm.contains("还没清单，先重新预览"), "没清单那一档没有自己的话：{arm}");
    // 0 那一档不许再挂着"不可撤销"那句：没东西可撤销。
    let zero_arm = t.find("doomedCount === 0").unwrap();
    let block: String = t[zero_arm..].chars().take(400).collect();
    assert!(!block.contains("这一步不可撤销"), "0 那一档还在说不可撤销：{block}");
}

/// 名字框里"点不动"要说得出为什么，而空值时那句回显不许摆一个空尾巴。
///
/// 同一个输入框以前两种待遇：带 `/` 会说（那句走 `plainNote` 进那只框下面的错误槽），
/// 全空却只是静默 `return`——那颗「就这么定」灰着，一个字都不说。判据本来就在界面手里，
/// 所以这一句由界面说是合法的（`DESIGN.md` 那句"后台没给才是后台的 bug，界面不设备用文案"
/// 说的是**后台该给的那些**）。
#[test]
fn an_empty_name_says_so_in_the_slot_that_already_exists() {
    let t = strip_comments(&read("src/components/BrowserPanel.tsx"));
    assert!(t.contains("名字还是空的"), "空名字那一次什么都没说（那颗灰着）：{t}");
    // 走现成那个错误槽（同一只框下面那条 Alert），不是新造浮层、也不是 title 属性。
    assert!(t.contains("setNamingError(plainNote('名字还是空的。'))"),
        "那句没走带 / 那一档同一个出口：{t}");
    assert!(t.contains("目录名里不要带 /"), "带 / 那一档的话不能被顺手删了：{t}");
    // 回显：改名那句要等有值才出现；建目录那句讲的是落点，空值也成立。
    let guard = t.find("naming.value.trim() !== ''").unwrap_or(usize::MAX);
    let echo = t.find("名字就是它在这一侧里的位置").unwrap_or(usize::MAX);
    assert!(guard < echo, "空值时那句回显还会摆出「a.webp → 」那种光尾巴：{t}");
    assert!(t.contains("新目录落在"), "建目录那句的落点回显丢了（它跟名字空不空无关）：{t}");
    // 这一条不许被做成"点得动但提交时炸"：那颗仍然在空值时是禁的，句子只是补上原因。
    assert!(t.contains("okButtonProps={{ disabled: busy || (naming?.value.trim() ?? '') === '' }}"),
        "空值时那颗放开到能点了：{t}");
}
