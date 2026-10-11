//! 设计契约的闸门（`DESIGN.md` 说"不写死字号与颜色"，这里就扫它）。
//!
//! 口径与 `frontend_pairs.rs` 相同（共用 `tests/common`）：本仓没有 JS 测试运行器，
//! 所以这些证人读**源文本**。
//! 但**先剥注释** —— 这一族最容易犯的错是"因错的理由变红"：
//! 注释里提一句被禁的写法，证人就红在一份不会上界面的文字上。那种红教人把注释改掉，
//! 而不是把代码改对，所以它比不红更坏。

mod common;

use common::{read, strip_comments, surface_text, tag_spans};

/// 会上界面的那一张张面。下面那三条文案判据（内部名词 / 称呼 / 口径词）扫的是同一批文件，
/// 名单放一处，免得出现"改了词表但只查了三个面"的那种洞。
const SURFACE_FILES: &[&str] = &[
    "src/Panel.tsx", "src/components/BrowserPanel.tsx", "src/components/BrowserPane.tsx",
    "src/components/LayerConfirm.tsx", "src/components/ResultDock.tsx", "src/components/Crumbs.tsx",
    "src/components/JobListView.tsx", "src/components/JobContextPanel.tsx",
    "src/components/JobDetailPanel.tsx", "src/components/RcdStatusBar.tsx",
    "src/components/MirrorConfirm.tsx", "src/components/InventoryPanel.tsx",
    "src/components/GatePanel.tsx", "src/components/LockHint.tsx",
    // 第三面（设置）：上面那三条文案判据要连着它一起扫，否则"改了词表但只查了几个面"
    // 那个洞就在这里重开。
    "src/components/SettingsPanel.tsx",
    // 预览那一格（2026-10-11）：名单是**写死的**，新组件不在上面就是免检——
    // 那道闸的判据（内部名词 / 称呼 / 硬编码视觉值）对一个没登记的文件一律失明。
    // 登记它比在组件里写一句"我是安全的"便宜。
    "src/components/ThumbCell.tsx",
    // 「换一对位置」那一层：口径表那几条词（第四档的标题、被拒绝那档的「没改成」）
    // 在它里面各写了一份，而名单是写死的 —— 不在上面的文件，那道闸对它一律失明。
    "src/components/PairDraftLayer.tsx",
];

/// 组件与页面里不许出现写死的字号 / 颜色 / 圆角：它们必须来自 `theme.ts` 或 CSS 变量。
/// `theme.ts` 与 `styles.css` 自己**是**令牌的定义处，所以不在扫描范围内。
#[test]
fn components_carry_no_hard_coded_visual_values() {
    for rel in ["src/Panel.tsx", "src/components/BrowserPanel.tsx", "src/components/JobListView.tsx",
                "src/components/JobDetailPanel.tsx", "src/components/JobContextPanel.tsx",
                "src/components/BrowserPane.tsx", "src/components/LayerConfirm.tsx",
                "src/components/ResultDock.tsx", "src/components/Crumbs.tsx", "src/components/NoteLine.tsx",
                "src/components/GatePanel.tsx",
                "src/components/InventoryPanel.tsx", "src/components/MirrorConfirm.tsx",
                "src/components/RcdStatusBar.tsx", "src/components/LockHint.tsx",
                "src/components/SettingsPanel.tsx", "src/components/ThumbCell.tsx"] {
        let t = strip_comments(&read(rel));
        let tight: String = t.split_whitespace().collect();
        for banned in ["fontSize:", "color:\"#", "background:\"#", "borderRadius:"] {
            assert!(!tight.contains(banned),
                "{rel} 里写死了视觉值（{banned}）——它该取令牌：{tight}");
        }
    }
}

/// 顶栏那格"日志在哪儿"是本项目的操作规程，不是装饰：那两次误启动就是靠它一眼发现的。
/// 所以它必须**留在明面上**，被收进 Tooltip 或抽屉 = 违规。
///
/// 判据是结构上的：把源文本里每一段 `<Tooltip …>…</Tooltip>`（含自闭合那种）切出来，
/// 一段都不许提到 `status.logDir`。上一版这里写的是"不许出现某串固定文字"，
/// 那种断言几乎不会红 —— 换个写法就绕过去了。
#[test]
fn the_log_directory_stays_on_the_surface_of_the_bar() {
    let t = strip_comments(&read("src/components/RcdStatusBar.tsx"));
    assert!(t.contains("status.logDir"), "顶栏不再读日志目录：{t}");
    for (s, e) in tag_spans(&t, "<Tooltip", "</Tooltip>") {
        assert!(!t[s..e].contains("status.logDir"),
            "日志目录被塞进 Tooltip 了：{}", &t[s..e]);
    }
    let tight = t.split_whitespace().collect::<Vec<_>>().join("");
    assert!(tight.contains("copyable={{text:status.logDir}}"),
        "日志目录还在，但那个复制入口没了：{tight}");
    // 版本/端口/进程是排障信息，收在"详情"那一句里。
    assert!(t.contains("详情"), "没有那个收起次要读数的入口：{t}");
}

/// 卡上那一点颜色只许取 CSS 变量，而且**整面只有一枚暖色**。
///
/// 两个理由各管一半：
/// * 库的预设色名（blue / volcano / green）是"预设字压自己那档淡底"，真窗口按实际渲染值量
///   只有 3.37–4.37，两档主题都不过 AA。取变量才能跟着色板走，改色也只改一处。
/// * 以前那是三枚 kind 标签（`KIND_VAR`）；现在一张卡是一对位置，颜色跟着
///   **这一格会不会删**走（`MODE_VAR`）。换的是形状，判据没换：暖色必须是唯一那一枚，
///   因为那是"会删东西"的信号职责，多一处暖色就没有信号了。
/// 判据是结构上的：把那张取色映射读出来数，不是找某句固定文案。
#[test]
fn the_group_labels_take_palette_vars_and_only_one_stays_warm() {
    let t = strip_comments(&read("src/components/JobListView.tsx"));
    let start = t.find("MODE_VAR").expect("MODE_VAR 这张取色映射没了（卡面又换形状了？）");
    let rest = &t[start..];
    let block = &rest[..rest.find('}').map(|i| i + 1).expect("MODE_VAR 的右花括号找不到")];
    for preset in ["\"blue\"", "\"volcano\"", "\"green\"", "'blue'", "'volcano'", "'green'"] {
        assert!(!block.contains(preset),
            "标签又用回库的预设色名了（那几档压自己的淡底不过 AA）：{block}");
    }
    let vars: Vec<&str> = block.split(',').filter(|p| p.contains("var(--")).collect();
    assert_eq!(vars.len(), 2, "两格模式没都取 CSS 变量：{block}");
    let warm = vars.iter().filter(|p| p.contains("--kind-warm")).count();
    assert_eq!(warm, 1, "暖色必须只给「会删除」那一格，现在数到 {warm} 枚：{block}");
    // 整面一枚：段标题那些标签走中性墨，别再长第二处暖色。
    assert_eq!(t.matches("--kind-warm").count(), 1, "暖色在这张卡上出现了不止一处：{t}");
    assert!(t.contains("const INK = 'var(--ink-2)'"), "段标题那两枚没取中性墨：{t}");
}

/// 深色已放出来，所以这一条钉的从"不许出现"换成**形状**：
/// * 切换口在界面上，而且**只在一处** —— 第二处就是第二个口径，两边会各自点亮不同的档；
/// * kill-switch 还要咬得住：把 `DARK_IS_SHIPPED` 翻回 false，`resolveTheme` 必须只会给浅色，
///   这样"深色没通过"不依赖有没有人记得把那颗控件藏起来（旧的那条证人犯的正是这个错）。
/// * 「选择」与「生效值」是两个值：`auto` 那一档必须能被点亮，同时画的是系统给的那一头。
#[test]
fn the_dark_switch_sits_in_one_place_and_the_kill_switch_still_bits() {
    let theme = read("src/theme.ts");
    assert!(theme.contains("export const DARK_IS_SHIPPED = true"),
        "深色这一档的常量没翻开：{theme}");
    assert!(theme.contains("if (!DARK_IS_SHIPPED) return 'light'"),
        "kill-switch 只是句空话：`resolveTheme` 没有读它，翻回 false 也照样出深色");
    // 三档都在，且少一档就等于替用户决定"跟随系统"没有出口。
    let bar = strip_comments(&read("src/components/RcdStatusBar.tsx"));
    for label in ["自动", "浅色", "深色"] {
        assert!(bar.contains(label), "切换口少了「{label}」那一档：{bar}");
    }
    assert!(bar.contains("useTheme"), "顶栏没接主题状态：{bar}");

    // 只许一处：扫全部会上一界面的文件。
    let hosts: Vec<&str> = SURFACE_FILES.iter()
        .filter(|rel| strip_comments(&read(rel)).contains("ThemeSwitch"))
        .map(|rel| *rel)
        .collect();
    assert_eq!(hosts, vec!["src/components/RcdStatusBar.tsx"],
        "主题切换口出现了第二处（或顶栏那处没了）：{hosts:?}");
}

/// 界面不许出现内部名词。扫的是**剥注释之后**的源码：注释里写 `rcd` 是给人看的，不该红
/// （同一条口径见 `tests/common` 那段——证人别因错的理由变红）。
#[test]
fn internal_names_stay_out_of_the_surface() {
    for rel in SURFACE_FILES {
        let t = surface_text(&strip_comments(&read(rel)));
        for banned in ["rcd", "copyfile", "purge", "token", "Scope", "FilterAware"] {
            assert!(!t.contains(banned), "{rel} 的界面文本里有内部名词 {banned}：{rel}");
        }
    }
}

/// 词表那条规则（`DESIGN.md`）：**桶里那样东西 = 对象，本地那样东西 = 文件，
/// 清单上看得见的那一行 = 名字**。
///
/// 这里不钉某两句的逐字——逐字钉换个写法就绕过去了，而这条要拦的是**称呼串了**。
/// 判据落在句子级：一句会上界面的话，只要提到云端那一侧（云端 / 远端 / 桶）又用了"文件"，
/// 就必须同句点明那说的是**本地**的那些，否则它就是拿"文件"去叫桶里的对象。
///
/// 反例是真出现过的两处：对账面的表标题「缺这个文件」（缺的是桶里的
/// 对象），和差异面那句「哪些是直接传到云上的正式文件」。
/// 而「这一层 N 个文件 · M 个文件夹」保持"文件"是对的——那一栏两侧都只数文件行。
#[test]
fn the_far_side_is_never_called_a_file() {
    // 一般的：一句里提到云端那一侧又用"文件"，必须同句点明说的是本地那些。
    for rel in SURFACE_FILES {
        let t = surface_text(&strip_comments(&read(rel)));
        for sent in t.split(['。', '；', '\n']) {
            let sent = sent.trim();
            let talks_far = ["云端", "远端", "桶"].iter().any(|w| sent.contains(w));
            if talks_far && sent.contains("文件") {
                assert!(sent.contains("本地"),
                    "{rel} 把云端那一侧叫成\"文件\"了（词表：那叫对象）：{sent}");
            }
        }
    }
    // 更严的一档：对账面与差异面**整面说的都是桶里那些东西**，那里出现的"文件"要么是
    // 指本地（同句必须写出"本地"），要么就是说漏了。这一档才拦得住表标题那种没有上下文的
    // 短格子——「缺这个文件」四个字里一个方向词都没有，上面那条一般判据对它失明。
    for rel in ["src/components/GatePanel.tsx", "src/components/InventoryPanel.tsx"] {
        let t = surface_text(&strip_comments(&read(rel)));
        for sent in t.split(['。', '；', '\n', ' ']) {
            let sent = sent.trim();
            if sent.contains("文件") && !sent.contains("本地") && !sent.contains("文件名") {
                panic!("{rel} 整面说的都是桶里的对象，这一格却写\"文件\"：{sent}");
            }
        }
    }
}

/// 词表那条（`DESIGN.md`）：**档位名说「配好的位置」，句子说「这两个位置」**。
/// 「这一对目录」是内部坐标系（那两个锁键）的说法在屏上留下了脚印——操作员从没在这上面
/// 见过"一对"这个东西，而锁的粒度本来就是**一棵树**（裁定 R92）。
/// 档位名不在这一条里：它是四个字的档名，与另一档「浏览文件」成对。
#[test]
fn the_two_sides_are_called_places_in_a_sentence() {
    let mut corpus = Vec::new();
    for rel in SURFACE_FILES {
        corpus.push((rel.to_string(), surface_text(&strip_comments(&read(rel)))));
    }
    // 后台那两句（作废、清单变了）与 Unknown 那句都在 `runner/mod.rs` 的字面量里。
    corpus.push(("src-tauri/src/runner/mod.rs".to_string(),
        surface_text(&strip_comments(&read("src-tauri/src/runner/mod.rs")))));
    for (rel, t) in &corpus {
        assert!(!t.contains("这一对目录"),
            "{rel} 的句子里说的是「这一对目录」，词表那条要它说「这两个位置」");
    }
    // 反向半边：改口之后那三句得真的在屏上说话（不是整句被删掉了）。
    let mirror = surface_text(&strip_comments(&read("src/components/MirrorConfirm.tsx")));
    assert!(mirror.contains("这两个位置"), "齐了那一句不再说「这两个位置」：改没了？");
}

/// 明面短槽（`plainNote('…')` 与 `short: '…'`）里那句必须 ≤16 字——`DESIGN.md` 那条管的正是
/// 这一格（`description` 长是合规的，所以句长那一族 186 条里绝大多数不该动；该动的是这五格）。
///
/// 计数只数**汉字**，`${…}` 里的东西不算：那一格插的是后台交来的名字（「远端」还是「本地目录」），
/// 长度不在界面手里，把它的长度算进这一条会在别的组合下假红。
#[test]
fn the_short_slots_stay_within_one_breath() {
    const MARKS: [(&str, char); 3] = [
        ("plainNote('", '\''), ("plainNote(`", '`'), ("short: '", '\''),
    ];
    let mut worst = (0usize, String::new(), String::new());
    let mut seen = 0usize;
    for rel in SURFACE_FILES {
        let t = strip_comments(&read(rel));
        for (mark, close) in MARKS {
            let mut at = 0;
            while let Some(h) = t[at..].find(mark) {
                let start = at + h + mark.len();
                let Some(end) = t[start..].find(close) else { break };
                let body = &t[start..start + end];
                seen += 1;
                let spoken: String = {
                    let mut out = String::new();
                    let mut depth = 0usize;
                    let mut prev_dollar = false;
                    for c in body.chars() {
                        if c == '$' { prev_dollar = true; continue; }
                        if prev_dollar && c == '{' { depth += 1; prev_dollar = false; continue; }
                        prev_dollar = false;
                        if depth > 0 { if c == '}' { depth -= 1; } continue; }
                        out.push(c);
                    }
                    out
                };
                let han = spoken.chars().filter(|c| *c >= '\u{4e00}' && *c <= '\u{9fff}').count();
                assert!(han <= 16,
                    "{rel} 的短槽那句有 {han} 个字（契约上限 16，细节收进展开）：{spoken}");
                if han > worst.0 { worst = (han, rel.to_string(), spoken.clone()); }
                at = start + end;
            }
        }
    }
    // 尺子先证明自己读到了东西：一格都没读到就是判据在空转，不是文案合规。
    assert!(seen >= 6, "只读到 {seen} 个短槽——这一族的形状大概换了写法：{worst:?}");
    assert!(worst.0 >= 12,
        "最长那一格才 {} 字？尺子可能根本没量到长句：{:?}", worst.0, worst.1);
}

/// 只有一行时不许挂着翻页器：每一处 `pagination={{…}}` 都必须写 `hideOnSinglePage: true`。
///
/// 判据落在"每一处都查"而不是"钉某几张表"：这一族以前就是漏在没人看的那三张上
/// （对账面那张缺失清单 + 差异面那两张），而第四张表将来加进来时，钉逐表的证人不会拦住它。
#[test]
fn every_table_pager_hides_itself_on_a_single_page() {
    let mut seen = 0usize;
    for rel in SURFACE_FILES {
        let t = strip_comments(&read(rel));
        let mut at = 0;
        while let Some(h) = t[at..].find("pagination={{") {
            let start = at + h;
            let Some(end) = t[start..].find("}}") else {
                panic!("{rel} 有一处 pagination 没闭合，判据落不到它身上");
            };
            let span = &t[start..start + end + 2];
            seen += 1;
            assert!(span.contains("hideOnSinglePage: true"),
                "{rel} 这张表的翻页器在只有一行时还会挂着：{span}");
            at = start + end;
        }
    }
    // 尺子先证明它读到了东西：一处都没找到就是判据在空转（改名或换了写法）。
    assert!(seen >= 5, "只找到 {seen} 处 pagination——判据的锚点大概已经换了形状");
}

/// 状态口径表那五个词，在明面那一格**只能是词本身**。

///
/// 「没得比：本地一个文件都没找到」那种写法是把两格合成了一格：冒号后那半句属于
/// 「为什么？」展开（`DESIGN.md` 的"一句人话 ≤16 字，细节收进展开"）。
/// 上一版屏上就是这么写的，而它是这一族里唯一一处——其余四个词都干净，所以这条判据
/// 不是摆设：把任何一个词后面再挂上"：解释"都会红。
#[test]
fn the_failure_words_stay_words_without_an_inline_gloss() {
    for rel in SURFACE_FILES {
        let t = surface_text(&strip_comments(&read(rel)));
        for word in ["没改成", "还没清单", "任务失败", "没得比", "这一栏列不出来"] {
            // 判"词 + 冒号"这一族，而不是判某一句：字面量在 `surface_text` 里是空格连着排的，
            // 拿 `starts_with` 会漏（前面那格的尾巴还挂在同一段里）—— 上一版就是这么绿的。
            for seg in t.split(['。', '；', '\n']) {
                let gloss = format!("{word}：");
                if seg.contains(&gloss) {
                    panic!("{rel} 在口径词「{word}」后面当场挂了半句解释（那半句属于展开那一格）：{seg}");
                }
            }
        }
    }
}

/// 配置文件那条路径不许写死在界面里：面板可以跑在别的家上（夹具规程 R68），
/// 写死的那一句在夹具里就在骗人。它必须由后台作为数据交出来（`RcdStatus.configPath`）。
///
/// 渲染点从 `Panel.tsx` 搬进了顶栏（起不来那一条与"改完重开面板就行"是同一件事，
/// 分成两块飘着像两条无关的通知）——所以这条扫的是**两份**，判据没放松：谁渲染都行，
/// 只要读的是后台那个字段，而且两份里都不许留一个写死的路径。
#[test]
fn the_config_path_comes_from_the_backend_not_a_hard_coded_string() {
    let bar = strip_comments(&read("src/components/RcdStatusBar.tsx"));
    assert!(bar.contains("status.configPath"), "那句提示不再读后台交出来的路径：{bar}");
    for rel in ["src/Panel.tsx", "src/components/RcdStatusBar.tsx"] {
        let t = strip_comments(&read(rel));
        assert!(!t.contains("%APPDATA%"), "界面里还写死着配置路径：%APPDATA% —— 那个家在夹具里根本不存在：{t}");
        assert!(!t.contains("jobs.toml"), "路径由后台交，界面里不许留文件名（留了就等于允许它再被拼回去）：{t}");
    }
}

/// 顶栏那格"有几件正在做"读的是后台数好的那个字段（`activeCount`）。
///
/// 为什么不许界面自己数：`RcdStatus.locks` 是按 `Scope` 分的键，一次动作可以申领多个键
/// （`acquire_scopes`），数键会把**一件事**报成两件 —— 那就是第二份判定（I5），
/// 而它错了没人知道。禁的是"数这份映射"这一族动作，不是某一种写法。
#[test]
fn the_active_count_is_the_backends_number_not_a_key_count() {
    let t = strip_comments(&read("src/components/RcdStatusBar.tsx"));
    assert!(t.contains("status.activeCount"), "顶栏不再读后台那个数：{t}");
    let tight = t.split_whitespace().collect::<Vec<_>>().join("");
    for banned in ["Object.keys(status.locks)", "Object.keys(locks)", "status.locks)", ".length>0&&"] {
        assert!(!tight.contains(banned),
            "顶栏在自己数那份登记表（{banned}）：一次动作可以申领多个键，数键会把一件事报成两件：{tight}");
    }
    assert!(!t.contains("status.locks"), "顶栏不该整份拿那份登记表，它要的就是那个数：{t}");
}

/// 开着「只比大小」那一档时顶栏要常驻一句，而且必须把**代价**一起说出来：
/// 只报一个比法的名字，操作员不知道少掉了什么（这一面被指"配置难懂"，病根就在这类只说名字的句子）。
///
/// 判据读的是后台那枚布尔（`status.sizeOnly`）：界面从 flags 里自己猜 = 第二份判定。
/// 反面同样钉 —— 不开着时那句不许在场，常驻一句没在跑的比法比不说更坏（这一档靠"整项不渲染"，
/// 与日志目录那一格同一族判据）。
#[test]
fn the_bar_says_which_basis_every_run_is_using() {
    let t = strip_comments(&read("src/components/RcdStatusBar.tsx"));
    assert!(t.contains("status.sizeOnly"), "那句没读后台那枚布尔，界面在自己猜比法：{t}");
    let words = surface_text(&t);
    assert!(words.contains("只比大小"), "开着那一档时屏上没有说出这个比法：{words}");
    assert!(words.contains("不会重传"),
        "只说了比法、没说代价。内容改了但大小没变的那些不会重传，这一句必须一起在场：{words}");
}

/// "正在连接"与"还没连上"必须是两档：`status` 只在轮询成功时才落，请求本身失败时它永远是
/// null —— 旧写法于是一直写着"正在连接后台…"，旁边还飘着一条说它失败的通知，
/// 同一屏上两句互相打脸（操作员报的）。
#[test]
fn the_bar_tells_a_failed_poll_apart_from_still_connecting() {
    let t = strip_comments(&read("src/components/RcdStatusBar.tsx"));
    assert!(t.contains("还没连上后台"), "问过而失败那一档没有自己的话：{t}");
    assert!(t.contains("正在连接后台"), "还没问过那一档的话丢了：{t}");
    // 分档的依据是 Panel 交进来的那一份错误，不是界面自己计时或猜。
    assert!(t.contains("pollError === null"), "两档不是按\"有没有那次失败\"分的：{t}");
    let panel = strip_comments(&read("src/Panel.tsx"));
    assert!(panel.contains("pollError={pollError}"), "那一份错误没交进顶栏：{panel}");
    assert!(panel.contains("setPollError(null)"), "成功的一轮没把它清掉：恢复之后还挂着旧那句：{panel}");
}

/// 令牌只在一个模块里，且 `App` 真的把它接上了 `ConfigProvider`。
/// 后半条钉的是这条形状本身：provider 直接包着这一面，且 `App` 自己不调 `useApp()`。
#[test]
fn the_theme_is_one_module_and_the_app_wires_it() {
    let theme = read("src/theme.ts");
    assert!(theme.contains("colorPrimary"), "主题里没有 antd 的主色令牌：{theme}");
    assert!(theme.contains("export function antdTheme"), "令牌不在一个模块里：{theme}");
    let app = read("src/App.tsx");
    let from = app.find("export default function App").expect("App.tsx 里没有那个组件");
    let body = app[from..].split_whitespace().collect::<Vec<_>>().join("");
    assert!(body.contains("theme={antdTheme("), "ConfigProvider 没接主题：{body}");
    assert!(body.contains("<AntdApp><Panel/>"), "provider 不再包着这一面：{body}");
}

/// 每一枚下拉框展开的那一列，宽度必须**按内容**给，不许跟框一样宽
/// （真窗口逐枚量的：antd 默认 `popupMatchSelectWidth: true`，弹层宽 = 框宽，
/// 而「只放上去（不删）」那一格需要 108px、132 的框里只剩 100 —— 永远读不全，
/// 选项又没带 title，悬停也读不回来。操作员报的"下拉框无法完整展示"就是这一处）。
/// 这条钉的是**每一枚都带着**：漏一枚，那一枚自己回去切字。
#[test]
fn every_select_lets_its_list_wide_enough_for_the_words() {
    for rel in ["src/components/BrowserPane.tsx", "src/components/JobListView.tsx",
                "src/components/PairDraftLayer.tsx", "src/components/SettingsPanel.tsx"] {
        let t = strip_comments(&read(rel));
        let n = t.matches("<Select").count();
        let w = t.matches("popupMatchSelectWidth={false}").count();
        assert!(n > 0, "{rel} 里一枚下拉框都没有了：那这条判据要跟着改，不许空转");
        assert_eq!(n, w, "{rel} 有 {n} 枚下拉框、只有 {w} 枚让列表按内容宽：剩下的会把选项切掉");
    }
}
