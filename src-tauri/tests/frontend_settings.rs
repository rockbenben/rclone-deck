//! 第三面「设置」那一张张面的证人。
//!
//! 口径与 `frontend_theme.rs` 相同（本仓没有 JS 测试运行器，界面形状只能读源文本），
//! 也都先 `strip_comments`：注释里提一句被禁的写法不该让证人红——那种红教人改注释。
//!
//! 这一面独有的两条是**顺序**与**在场**：连带那句要在保存**之前**看得见（等按钮灰了
//! 才说，人已经点过一次了），以及确认层里那一对完整位置必须由后台交出来（名字可以撞）。

mod common;

use common::{read, strip_comments, surface_text};

const PANEL: &str = "src/components/SettingsPanel.tsx";

/// 视图切换有第三档，而且那一段的名字是一个动作短语；这一面是三块卡片，自上而下。
#[test]
fn the_third_view_is_a_card_of_three_blocks_named_by_what_it_does() {
    let panel = strip_comments(&read("src/Panel.tsx"));
    assert!(panel.contains("'设置'"), "分段控件没有第三档：{panel}");
    assert!(panel.contains("'settings'"), "第三档没有自己的 view 值：{panel}");
    // 三面都挂着的显示条件必须按 view **等值**判；沿用"不是 browser 就显示"那种写法，
    // 第三档一出现就会把对子面一起显示出来（两面上同一个按钮，点哪个算哪个）。
    for arm in ["view === 'pairs'", "view === 'settings'"] {
        assert!(panel.contains(arm), "显示条件没按档位等值判（少了 {arm}）：{panel}");
    }
    assert!(!panel.contains("view !== 'browser'"),
        "还在用『不是浏览器那一面就显示对子』的写法：第三档会让两面同时显示");

    let t = surface_text(&strip_comments(&read(PANEL)));
    for block in ["这台机器上", "你的远端", "任务"] {
        assert!(t.contains(block), "少了一块卡片：{block}");
    }
    // 内部词不许当称呼（这一面最容易顺手把配置文件名与英文键名写上屏）。
    for banned in ["rcd", "token", "jobs.toml", "rclone.conf", "Scope"] {
        assert!(!t.contains(banned), "这一面的话里有 {banned}：{t}");
    }
}

/// 证人 5 的后半：exclude 那句连带在**卡片上**、在保存那颗之前，不是等按钮灰了才发现。
/// 判据是顺序而不是"某处有这个字符串"：挪进确认层就红（那是"点过之后才说"的形状）。
#[test]
fn the_exclude_side_effect_is_said_before_the_save_not_after() {
    let t = surface_text(&strip_comments(&read(PANEL)));
    // 「哪些不进桶」曾写「桶」（S3 行话），而它脚下那半句说的是「决定哪些东西不进去」——
    // 标签用行话、说明用白话，同一格两套词。2026-10-09 改成「哪些东西不进去」，两处同一个词。
    // 判据仍是**顺序**（那句连带必须在保存那颗之前），锚点跟着标签走。
    let at = t.find("哪些东西不进去").expect("exclude 那个字段没有中文说法");
    let note = t.find("看到的不是全部").expect("那句连带不在这一面上");
    let save = t.find("保存这几项").expect("保存那颗没了");
    assert!(at < note && note < save,
        "那句连带的位置不对（字段={at} 那句={note} 保存={save}）——它必须在按钮之前");
    // 另一半：flags 那句"要重开"也是同一族（后台交句子，界面只渲染，所以这里钉的是"有这一格"）。
    assert!(t.contains("重开"), "flags 那句连带不在这面上：{t}");
}

/// 「每次都要带的参数」那一格要点名这一族里**唯一**那枚改比法的例外，并把代价一起说。
/// 判据与 exclude 那一条同族：**在保存那颗之前**（填的人得在按下之前知道少掉了什么）。
///
/// 那串拼写 `--size-only` 必须在场：这一格填的就是拼写，只说人话不写它长什么样，
/// 人不知道该打什么 —— 而"配置难懂"正是操作员报过的那件事。
#[test]
fn the_flags_cell_names_the_one_exception_and_what_it_costs() {
    let t = surface_text(&strip_comments(&read(PANEL)));
    let at = t.find("每次都要带的参数").expect("flags 那个字段没有中文说法");
    let named = t.find("--size-only").expect("那一格没点名这枚例外");
    let save = t.find("保存这几项").expect("保存那颗没了");
    assert!(at < named && named < save,
        "那句说明的位置不对（字段={at} 那枚={named} 保存={save}）——要在按下之前说");
    assert!(t.contains("只比大小"), "没说出这一枚会把比法换成什么：{t}");
    assert!(t.contains("不会重传"), "代价没说出口：内容改了但大小没变的那些不会重传：{t}");
}

/// 保存成功那句走状态口径表第一档（「做完了 · …」），正文是后台那句「重开面板后生效。」。
/// 句子住两头发：后台拼（`edit.rs`），界面渲染（这一面）。只钉一头的话，另一头漂了没人知道。
#[test]
fn the_saved_slot_says_it_takes_effect_after_a_restart() {
    let edit = std::fs::read_to_string(
        concat!(env!("CARGO_MANIFEST_DIR"), "/src/config/edit.rs")).unwrap();
    assert!(edit.contains("重开面板后生效。"), "后台那句短槽没了：界面就没得显示");
    let t = surface_text(&strip_comments(&read(PANEL)));
    assert!(t.contains("做完了"), "保存成功那一条没走口径表第一档：{t}");
    // 口径词后面不许当场挂解释（同 `frontend_theme.rs` 那条判据）：这一面新加的那一格也不例外。
    for word in ["没改成", "这一栏列不出来"] {
        for seg in t.split(['。', '；', '\n']) {
            assert!(!seg.contains(&format!("{word}：")),
                "口径词「{word}」后面挂了半句解释（那半句属于展开那一格）：{seg}");
        }
    }
}

/// 密钥那一格：屏幕上只有那六个字与一条可复制的出路，**没有输入框**。
/// 判据是结构：那一格是自己的元件，元件体里不许有输入控件；元件之外也不许有人读密钥的值。
#[test]
fn the_secret_cell_has_no_input_box_and_the_surface_never_names_a_key() {
    let src = strip_comments(&read(PANEL));
    let t = surface_text(&src);
    assert!(t.contains("不回显"), "那一格没写「已配置，不回显」：{t}");
    // 键名一个都不出现（它们既不是可编辑项，也不该被界面提出来"是哪几项"）。
    for banned in ["access_key_id", "secret_access_key", "accessKey", "secretKey"] {
        assert!(!src.contains(banned), "界面上点了密钥的键名 {banned}");
    }
    // 为什么按元件判，不按"从那一句往后数 N 个字符"判：后者挪一次文件就失效，
    // 而失效的方向是**变绿**——那种判据留着等于没有。
    let body = fn_body(&src, "function SecretCell")
        .expect("密钥那一格没在自己的元件里：和别的字段混在一块儿，就迟早长出输入框");
    // 先证明这一把切片切到的是那一格的**本体**，再拿它判"里面没有输入框"：
    // 切片切错（比如只切到参数表）时那几条 assert 会全绿——那种绿是假的。
    assert!(body.contains("不回显") && body.contains("secrets.command"),
        "切片没切到密钥那一格的本体，下面那几条判据就没意义：{body}");
    for ctrl in ["<Input", "<Select", "TextArea", "AutoComplete"] {
        assert!(!body.contains(ctrl),
            "密钥那一格里长出了 {ctrl}：那等于给「改密钥」开了一条通路：{body}");
    }
    // 交出去的只有那一段引用；引用处不许顺手读值（值的通路只许有 `SecretCell` 那一条）。
    let rest = src.replace(body.as_str(), "");
    for read_form in ["secrets.note", "secrets.command", "secrets.present"] {
        assert!(!rest.contains(read_form), "SecretCell 之外还在读 {read_form}：密钥多了一条读法");
    }
    assert!(src.contains("<SecretCell secrets={block.secrets} />"),
        "那一格没接进远端卡片，屏上就没有那条出路：{src}");
}

/// 取出 `pattern` 那个函数从**函数体**的第一个 `{` 到配平的那个 `}` 的整段（含两端）。
/// 只为"某个元件的**内部**不许有某样东西"这类判据服务。
/// 参数表里那个 `{`（TS 的对象类型标注）不算函数体：它出现时括号还没合上。
fn fn_body(src: &str, pattern: &str) -> Option<String> {
    let at = src.find(pattern)?;
    let mut parens = 0i32;
    let mut open = None;
    for (i, c) in src[at..].char_indices() {
        match c {
            '(' => parens += 1,
            ')' => parens -= 1,
            '{' if parens == 0 => {
                open = Some(at + i);
                break;
            }
            _ => {}
        }
    }
    let open = open?;
    let mut depth = 0usize;
    for (i, c) in src[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(src[open..open + i + 1].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

/// 确认层：正文前两行是**完整路径**与那一**对**位置（都不是任务名），保存那颗的文案分两档。
#[test]
fn the_confirm_layer_leads_with_the_path_and_the_full_pairing() {
    let src = strip_comments(&read(PANEL));
    for field in ["plan.path", "plan.subject", "plan.changes", "plan.notes"] {
        assert!(src.contains(field), "确认层没读后台那个字段 {field}：句子就只能由界面自己编");
    }
    let t = surface_text(&src);
    assert!(t.contains("我知道，保存"), "确认那颗没有那一句：{t}");
    assert!(t.contains("我知道，删掉这条"), "删任务那一档要另一颗（与「保存」不是一颗）：{t}");
    // 差异为空 ⇒ 不给保存。判据：界面不许自己数 changes 来判"有没有东西"（那是第二份口径），
    // 后台已经用"发不出凭证"拦住了；界面上那颗的禁用条件读的是 plan 在不在。
    assert!(!t.contains("我知道，保存（0"), "按钮在零差异时还写着要保存：{t}");
    // 这一层里那张差异表**不分页**（一次配置的差异行个位数），所以不许长出 `pagination={{`。
    assert!(!src.contains("pagination={{"),
        "差异表挂了翻页器：这一族的表只有一处该分页，而这里连分页都不需要");
}

/// 任务那一块：每行两颗（改这条 / 删这条），新加一条在卡片头上；「这一条算哪类」那三个词
/// 读 `KIND_LABEL`（那张表是 Rust `JobKind::word()` 的镜像，R56），这里不许再发明一套。
#[test]
fn the_job_block_reuses_the_kind_vocabulary_and_offers_three_moves() {
    let src = strip_comments(&read(PANEL));
    assert!(src.contains("KIND_LABEL"), "kind 那三个词没读那张共用的词表：{src}");
    let t = surface_text(&src);
    for label in ["新加一条", "改这条", "删这条"] {
        assert!(t.contains(label), "任务那一块少了那颗：{label}");
    }
    // id 不可改：那一格要么不存在、要么禁用（改 id 等于把这一块的其余行指到另一条任务上）。
    let id_cell = src.split("这条的编号").nth(1).unwrap_or("");
    assert!(id_cell.contains("disabled"),
        "id 那一格点得动：改 id 会让这一块的其余行落到另一条任务上：{id_cell}");
    // 但「新加」那一次它必须点得动：无条件禁用的话，新条目永远交得出空编号，
    // 而后台那一条（`a_new_job_without_an_id_is_refused_and_writes_nothing`）会把这种意图拦在盘外
    // —— 两条判据合起来正好是"新加一条这条路走不通"，谁都不红，功能却是坏的。
    assert!(id_cell.contains("disabled={!job.isNew}"),
        "编号那一格没分「新加」与「已有」两档：{id_cell}");
    // 分档之外还得有写入口：`value` 挂上了而没有 `onChange`，那一格就永远填不进东西
    // ——真窗口跑出来过一次：新加一条交出去的编号恒为空串。
    assert!(id_cell.contains("onChange"),
        "编号那一格没有写入口，新加的条目根本没法带编号：{id_cell}");
}

/// 这一面新加的字段都读后台，界面不比较、不数行、不反推（`I5`）。
/// 判据：这一面里不许出现"自己判断"的那几种写法。
#[test]
fn this_surface_judges_nothing_itself() {
    let src = strip_comments(&read(PANEL));
    let tight: String = src.split_whitespace().collect();
    // 界面自己拼路径 / 猜端点 / 数差异行数，都是把后台的判定抄第二遍。
    for banned in ["startsWith('r2", ".split(\":\")", "changes.length === 0", "\\.length > 0 ? '会改"] {
        assert!(!tight.contains(&banned.replace(' ', "")),
            "界面里出现了自己判断的那族写法 {banned}：那些判定归后台");
    }
    // 后台交的两个"存在性"字段必须被读，而不是由界面拿正则再认一遍端点。
    assert!(tight.contains("rcloneBinExists") && tight.contains("logDirExists"),
        "存在性那两个字段没被读：那两句话就变成界面猜的");
}

/// 每一格要当场回答两个问题：**这一格决定什么**、**空着会怎样**。
///
/// 只说"这是什么"（`服务地址`、`区域`）就得让人自己推断填与不填的后果 —— 操作员的原话是
/// "配置难懂"。所以这两半句是 `Field` 的**必填**属性（少一格编译就红，不靠正则数），
/// 而这里钉的是两格真正写出来的话，免得把必填做成"参数在、话是空的"。
#[test]
fn every_field_answers_what_it_decides_and_what_blank_does() {
    let raw = read(PANEL);
    let said = surface_text(&strip_comments(&raw));
    assert!(raw.contains("decides: string") && raw.contains("ifBlank: string"),
        "Field 的这两半句要是可选参数，下一格漏了没人拦：{said}");
    for half in ["决定这个远端连哪台服务器", "空着就用 rclone 自己的默认",
                 "决定面板拿哪一份 rclone 起后台", "空着保存会被拦下"] {
        assert!(said.contains(half), "那一格的半句没写出来：{half}");
    }
    // 「这一格现在填的是文件里的那一份」这种复述不是新信息，不该占掉说明位。
    assert!(!said.contains("这一格现在填的是"), "说明位被复述占了：{said}");
}

/// rclone 自己那份 conf 的路径以前只在后台转了一圈就没上屏：想知道远端存在哪，
/// 只能自己猜目录。现在那一行露出来，并带一颗只开"它所在那一层"的按钮。
#[test]
fn the_screen_shows_rclones_own_file_and_offers_to_open_its_folder() {
    let raw = read(PANEL);
    let said = surface_text(&strip_comments(&raw));
    // 2026-10-09：那一行末尾的「在」撤了（**两份文件两行都**不带介词，路径就摆在称呼旁边）。
    // 判据的意图是"两个名字分得清"，不是那个介词 —— 旧写法只有 rclone 那一行带「在」，
    // 于是并排两行的语法不对称，其中一行还多了个什么信息都没加的字。钉的是名字本身。
    assert!(said.contains("rclone 自己的配置文件"),
        "没说清那是 rclone 的文件（面板自己那份是任务清单，两个名字混成一个就没人找得到）");
    // 反过来也一样要分得开：面板自己那份**不许**也叫「配置文件」。
    assert!(said.contains("任务清单文件"),
        "面板自己那份该叫「任务清单」：两份文件混成一个名字，操作员就找不到要找的那份");
    assert!(said.contains("打开这个目录"), "那颗按钮不在：{said}");
    // 那一格**渲染出来的**就是后台交的那一个（`copyable` 里也读它，但那不够：屏上写的字
    // 才是操作员抄下来去终端找的那一串）。
    assert!(raw.contains("{view.confPath}"), "屏上那一串不是后台交出来的路径：{raw}");
    assert!(raw.contains("view.confPath === ''"), "没按'问不出'那一档分岔：{raw}");
    // 那颗按钮自己不发请求：它喊的是**交进来的那一条通路**（和卡上「打开所在目录」同一条
    // `act('打开目录', …)`：失败时念出「打开目录失败 + 后台那句为什么」）。
    // 在这一面里直接 `invoke` 的那一颗，失败要么静默、要么顶着「没改成」那句不相干的话。
    assert!(raw.contains("onClick={props.onOpenFolder}"),
        "那颗按钮没挂在交进来的通路上（失败就没人说）：{raw}");
    assert!(!raw.contains("@tauri-apps/api"),
        "设置面绕过前端桥、自己直接发请求：{raw}");
}

/// 「任务」那一块按**同一对位置**分组显示（操作员为运行那一面定的规矩：「应该是选择框，
/// 而不是 N 条不同的条目」—— 那一面收了，这一面还平铺着十条，就是同一件事的两个说法）。
///
/// 判据是"键从哪儿来"，不是"有没有标题"：分组必须读后台那枚 `pairKey`，标题必须复用
/// 卡上那一对短名（`pairLabels`）。界面里再拿路径算一遍键，就是第二份口径 —— 那正是
/// 本仓反复红过的那族错（R69 的禁令）。
#[test]
fn the_jobs_block_is_grouped_by_the_backends_key_not_a_second_derivation() {
    let panel = strip_comments(&read(PANEL));
    assert!(panel.contains("groupDraftJobs("),
        "「任务」那一块还在一条一条平铺：{panel}");
    assert!(!panel.contains("view.jobs.map("),
        "还在直接遍历那一份平铺的清单（分组没接上渲染）：{panel}");

    let pairs = strip_comments(&read("src/pairs.ts"));
    assert!(pairs.contains("export function groupDraftJobs"), "分组函数不在 pairs.ts 里：{pairs}");
    let at = pairs.find("export function groupDraftJobs").unwrap();
    let body = &pairs[at..];
    // 判的是"每一行分组读的是谁"，不是"这函数里出现过 `j.pairKey`"：
    // 上一版那样写，把桶换成 `j.src` 而返回值里还留着一次 `j.pairKey`，证人照样绿 ——
    // 那是钉了一个永远成立的条件（H2 那把枪打出来就是这个形状）。
    let keying: Vec<&str> = body.lines()
        .filter(|l| l.contains("byKey.has(") || l.contains("byKey.set(")
            || l.contains("order.push(") || l.contains("byKey.get(j."))
        .collect();
    assert!(keying.len() >= 2, "分组那几行少到没在判任何东西：{keying:?}");
    // 判据写成 `find` 而不是 `for … filter { panic! }` 那个形状：后者在 clippy 的
    // never_loop 下是 deny（循环体永远 panic，它压根不循环），会让整个 test target 编不过。
    // 语义不变：命中第一行不合格的分组键就用它的原文报出来。
    if let Some(l) = keying.iter().find(|l| !l.contains("pairKey")) {
        panic!("这一行分组没读后台那枚键（{l}）");
    }
    assert!(body.contains("pairLabels("), "标题没复用卡上那一对短名（那是第二份说法）：{body}");
    // 界面自己认路径的那几种写法：出现即红（端点类型是数据，不是正则能判的东西）。
    for banned in ["indexOf(':')", "startsWith('[')", "/^[a-z0-9]+:/"] {
        assert!(!body.contains(banned), "分组里在拿路径自己判端点（{banned}）：{body}");
    }
}

/// 改了还没保存，要说得出**是哪几处**（操作员那句「我常用设置怎么没保存」）。
/// 实测的形状是：填进框里的字活着（切到别的面再回来还在），**重开面板才静默消失**，
/// 而屏上没有任何一处说"这些还没落盘"（界面上读不出"没落盘"这件事）。
/// 所以那句必须挂在三处草稿上，而不是常驻一句提醒 —— 常驻的那句会被读成噪声。
#[test]
fn the_settings_face_says_which_cells_are_unsaved() {
    let t = strip_comments(&read("src/components/SettingsPanel.tsx"));
    let tight = t.split_whitespace().collect::<Vec<_>>().join("");
    // 断言消息只摘前面一小段：整份文件插进 panic 消息，读的人只会翻过去（同仓那条 4 MB 的教训）。
    let clip: String = tight.chars().take(240).collect();
    assert!(tight.contains("还没保存"), "这一面没有那句\"改了还没保存\"：{clip}");
    // 三处草稿一处都不许漏：默认那几项、每个远端、开着的那张任务表单。
    for want in ["defaultsDirty", "remotesDirty", "jobDirty"] {
        assert!(tight.contains(want), "那份名单少了 {want} 那一处草稿：{clip}");
    }
    // 挂条件而不是常驻：判据是"有没有脏"，不是"这一面开着"。
    assert!(tight.contains("unsaved.length>0") || tight.contains("unsaved.length!==0"),
        "那句不是按\"有没有没保存的\"摆的（常驻的一句等于没有那句）：{clip}");
    // 那句要指得出路：说出保存那颗在哪儿，而不是只报"有 N 处"。
    assert!(tight.contains("重开面板就回到磁盘"), "那句没说出后果（重开会怎样）：{clip}");
}

/// 对账任务的对账基准由「算哪类」带出来，不再是一枚没有标签的「不对账」开关。
///
/// 旧形状：表单底部一枚裸 `Select`，选项是「不对账 / 这一条要对账」。选了「对账」却没拨它，
/// 保存时后台回「gate 任务 X 缺少 manifest」——一个普通用户既不知道 manifest 是什么，
/// 也不知道刚才那枚裸下拉就是它的开关。现在 `manifest` 跟着 kind 走，表单只给一句说明。
#[test]
fn a_gate_job_gets_its_manifest_from_the_kind_not_a_bare_toggle() {
    let t = strip_comments(&read(PANEL));
    assert!(t.contains("manifest: v === 'gate'"),
        "「对账」没有自动带上对账基准（用户还得自己找那枚开关）：{t}");
    assert!(t.contains("对账基准"), "对账任务没有一句说明基准从哪来：{t}");
    assert!(!t.contains("'不对账'"), "那枚没有标签的「不对账」开关又回来了：{t}");
    assert!(!t.contains("manifestField"), "旧的无标签开关处理器还在：{t}");
}

/// 新加一条时面板先替用户起一个编号，不让人凭空发明内部身份。
///
/// 编号会进日志文件名，普通用户没有理由自己想一个；字段仍然可编辑，想用短名就改。
/// 这一条钉的是"新表单打开时已经有值"和"临时卡带过来也有值"，不是钉具体生成的格式。
#[test]
fn a_new_job_starts_with_a_ready_id() {
    let t = strip_comments(&read(PANEL));
    assert!(t.contains("function suggestedJobId"), "没有生成建议编号的那一处：{t}");
    assert!(t.contains("id: suggestedJobId()"), "新加一条没带上建议编号：{t}");
    assert!(t.contains("id: p.id === '' ? suggestedJobId() : p.id"),
        "从临时卡带过来的表单没带上建议编号：{t}");
    assert!(t.contains("已经替你起了一个"), "用户不知道这个编号是替他起的、能不能改：{t}");
}

/// 设置面第一次打开时是在读文件，不是已经失败；读的过程中不许摆「再读一次」。
///
/// 冷启动实测：两份文件 + 两次只读 rclone 探针会超过 2 秒，旧界面在这段时间里只有
/// 「改设置」和一个「再读一次」按钮，用户会以为坏了或读到空配置。
#[test]
fn the_settings_face_says_it_is_loading_instead_of_offering_retry() {
    let t = strip_comments(&read(PANEL));
    assert!(t.contains("正在读那两份文件"), "第一次读的时候没有一个「正在读」：{t}");
    assert!(t.contains("error !== null && <Button"), "读失败时没有重试按钮（或反过来又常驻了）：{t}");
}
