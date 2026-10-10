//! 钉住对齐确认话术里那句**前提**（裁定 R58 item 3，逐方向分开是 R60 补的）。
//!
//! 本仓库没有 JS 测试运行器（package.json 里只有 tsc/vite/cargo），所以这条证人直接读**源文本**：
//! 对齐的确认层必须原样携带下面两句前提——删掉半句、或改了措辞，这里就红。
//! R58 把**任务这一侧**删除的唯一入口收进了对齐，这两句是那个入口上**唯一**写明"哪一侧是全集"的地方，
//! 它比任何闸门都靠前：人是在读到这句话的瞬间决定点不点「我知道，删这 N 个」的。
//!
//! 话术搬家：R60 把这句前提从 `JobListView.tsx` 的 Popconfirm 搬进了 `MirrorConfirm.tsx`——
//! 那一档的 Popconfirm 根本不列清单，把前提写在那里等于把前提写在一个跳过确认的入口上。
//! 搬的时候**不是原样照搬**：句首补上了明确的方向主语（「本地是全集：」「云端是全集：」），
//! 因为两个方向共用一句就总有一个方向在骗人。下面一个方向钉一条证人。
//!
//! 为什么两句都要钉：曾经只有上传那一句（"本地没有的云端对象会被删掉…先跑一次下载"），
//! 而出厂四条里两条是下载方向的对齐——那里被删掉的是**本地**多出来的文件，同一句话在
//! 那个方向上是错的，人却正照着它按删除键。一个方向钉一句，另一个方向不许没证人。

/// 被读的文件：对齐的确认层（前提话术现在的住处）。
fn confirm_layer() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("src")
        .join("components")
        .join("MirrorConfirm.tsx");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("读不到 {}：{e}", path.display()))
}

/// 上传方向（本地目录 → 云端）：本地是全集，被删的是云端多出来的对象。
#[test]
fn the_mirror_confirmation_states_the_local_is_the_whole_set_precondition() {
    const SENTENCE: &str =
        "本地是全集：云端那些本地没有的对象会被删掉。如果你直接往云上传过图，先跑一次下载把本地补全，再对齐。";
    let text = confirm_layer();
    assert!(text.contains(SENTENCE),
        "上传方向的镜像丢了「本地是全集」这句前提（改文案就红）：{SENTENCE}");
}

/// 下载方向（云端 → 本地目录）：全集换到了云端，被删的是本地多出来的文件。
#[test]
fn the_mirror_confirmation_states_the_remote_is_the_whole_set_precondition() {
    const SENTENCE: &str =
        "云端是全集：本地那些云端没有的文件会被删掉。这批如果还没上传，先跑一次上传，再对齐。";
    let text = confirm_layer();
    assert!(text.contains(SENTENCE),
        "下载方向的镜像丢了「云端是全集」这句前提（改文案就红）：{SENTENCE}");
}

/// 那一层以前只说"拿走"，没说这一趟**同时**会把缺的那批补上/拿下来。
/// 操作员因此把它读成"这一档只能删东西"，宁可停在「先试跑」上：`r2-mirror-local-wins`
/// 两遍试跑、零次真跑（日志里全是 `dry_run=true`），那 17 个一直没上到云端。
/// 两句各钉一条，方向不许共用一句（共用一句就在另一个方向上说假话）。
#[test]
fn the_mirror_confirmation_says_the_same_run_also_fills_the_missing_side() {
    const SENTENCE: &str =
        "这一趟按对齐跑：不只拿走云端多余的那些，也会把本地有、云端没有的那批补上去。";
    let text = confirm_layer();
    assert!(text.contains(SENTENCE),
        "上传方向那一档没说这一趟也会把缺的补上去：{SENTENCE}");
    // 那句必须真的**渲染**出去：只留一个没人读的常量，等于给这条判据配了个假证人。
    assert!(text.contains("<Typography.Text>{alsoMoves}</Typography.Text>"),
        "那句写了却没挂到层里（屏上看不见）：{SENTENCE}");
}

/// 另一头：`src` 是云端时，"补"的方向反过来——这句在上传那一档上不许成立，也不许缺席。
#[test]
fn the_mirror_confirmation_names_the_download_direction_too() {
    const SENTENCE: &str =
        "这一趟按对齐跑：不只拿走本地多余的那些，也会把云端有、本地没有的那批拿下来。";
    let text = confirm_layer();
    assert!(text.contains(SENTENCE),
        "下载方向那一档没说这一趟也会把缺的拿下来：{SENTENCE}");
    assert!(text.contains("const alsoMoves = localWins(job)"),
        "这一句不按方向分叉（两个方向共用一句就有一个在说假话）：{SENTENCE}");
}
