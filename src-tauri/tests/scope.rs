use rclone_deck_lib::endpoint::endpoint_of;
use rclone_deck_lib::scope::Scope;

#[test]
fn sibling_prefixes_that_merely_look_alike_do_not_conflict() {
    // I3 那一对：同一个远端里的 `img` 与 `imgprompt`。判成冲突 = 两条不相干的任务互相假死。
    let a = Scope::remote("r2:", "img");
    let b = Scope::remote("r2:", "imgprompt");
    assert!(!a.conflicts(&b), "{a:?} 与 {b:?} 不该互相挡");
    assert!(!b.conflicts(&a), "冲突判定必须是对称的");
}

#[test]
fn an_ancestor_prefix_always_conflicts_with_its_children() {
    let root = Scope::remote("r2:", "");
    let sub = Scope::remote("r2:", "img");
    let deep = Scope::remote("r2:", "img/2026");
    assert!(root.conflicts(&sub) && sub.conflicts(&root), "父子必须互斥");
    assert!(root.conflicts(&deep) && deep.conflicts(&root), "根要罩住所有更深的前缀");
    // 这条不是前两条的重复：前两条的某一侧 prefix 为空，这条两侧都非空，
    // 钉的是"祖先判定在非空 prefix 之间也成立"。
    assert!(sub.conflicts(&deep) && deep.conflicts(&sub), "相邻两层父子同样必须互斥");
    // 而相邻的**兄弟**不互斥。挑的这一对必须"真的共享一个前缀"，否则守不住"只看字符串前缀"那一侧：
    // `image` 并不以 `img` 开头（两个串共享的是 `im`，第三个字符 `g` 对 `a` 就分岔），
    // 只看前缀的写法对它照样返回"不是父子"，那句就成了装饰。
    // `a/bc` 以 `a/b` 开头，才是只看前缀就会判成父子的一对兄弟。
    assert!(!Scope::remote("r2:", "a/b").conflicts(&Scope::remote("r2:", "a/bc")),
        "以同一个前缀开头的兄弟不是父子：这是 lookalike");
}

#[test]
fn different_remotes_never_conflict_even_with_identical_prefixes() {
    assert!(!Scope::remote("r2:", "a").conflicts(&Scope::remote("qiniu:", "a")));
}

#[test]
fn separators_and_trailing_slashes_cannot_make_two_names_of_one_scope() {
    // 反斜杠与尾斜杠在 rc 那边是两种真实形状（子项目 1 §3.2 量过尾斜杠=空列表）。
    assert_eq!(Scope::remote("r2:", "a\\b"), Scope::remote("r2:", "a/b"));
    assert_eq!(Scope::remote("r2:", "a/b/"), Scope::remote("r2:", "a/b"));
    assert_eq!(Scope::remote("r2:", "/a/b"), Scope::remote("r2:", "a/b"));
    assert_eq!(Scope::remote("r2:", "a//b"), Scope::remote("r2:", "a/b"), "中间的空段也算两个名字");
}

#[test]
fn a_bucket_inside_the_fs_string_is_one_tree_with_its_subprefixes_but_not_with_its_neighbour() {
    // 走 of_endpoint 那一条真实入口：`r2:img` 这个串里 `img` 是桶名，也是远端内的第一段路径。
    let bucket = Scope::of_endpoint(&endpoint_of("r2:img"));
    let deeper = Scope::of_endpoint(&endpoint_of("r2:img/2026"));
    let neighbour = Scope::of_endpoint(&endpoint_of("r2:imgprompt"));
    assert_eq!(bucket.key(), "r2:#img", "根只到冒号为止，桶名进 prefix");
    assert!(bucket.conflicts(&deeper) && deeper.conflicts(&bucket));
    assert!(!bucket.conflicts(&neighbour) && !neighbour.conflicts(&bucket));
}

#[test]
fn local_roots_compare_case_insensitively_and_nesting_still_conflicts() {
    // Windows 的路径大小写不敏感；而"整条路径当根"会把 D:\Vault 与 D:\Vault\img 判成两棵不相干的树——
    // 往上一层写东西会动到下一层，那正是"两个写动作同时落进同一棵树"。
    use std::path::Path;
    assert_eq!(
        Scope::local(Path::new("D:\\Vault\\img")).key(),
        Scope::local(Path::new("d:\\vault\\img")).key(),
        "同一棵树不该有两个名字"
    );
    let top = Scope::local(Path::new("D:\\Vault"));
    let sub = Scope::local(Path::new("D:\\Vault\\img"));
    assert!(top.conflicts(&sub) && sub.conflicts(&top), "本地侧的父子也必须互斥");
    assert!(!top.conflicts(&Scope::local(Path::new("E:\\Vault"))), "另一个盘是另一棵树");
    // R84：非盘符形状（UNC）的根切在**第一段**，`\\srv\share` 与它的子层才是同一棵树；
    // 根若是整串，两者就成了两个不等的根 ⇒ 不冲突。
    // 那是"两个写动作同时落进同一棵树"的缝，方向危险（起点事故那一族），所以这条必须钉住。
    let unc = Scope::local(Path::new("\\\\srv\\share"));
    let unc_sub = Scope::local(Path::new("\\\\srv\\share\\img"));
    assert!(unc.conflicts(&unc_sub) && unc_sub.conflicts(&unc), "UNC 的父子也要互斥：{unc:?} / {unc_sub:?}");
    assert!(!unc.conflicts(&Scope::local(Path::new("\\\\anothersrv\\share"))), "另一个服务器是另一棵树");
}

#[test]
fn describe_reads_as_a_path_for_the_three_root_shapes() {
    // I1 要求"对哪个完整路径"回显得出来；`describe()` 是那句的唯一出处。
    // 本地根拼成 `d:vault` 会看着像一个远端名 —— 操作员按远端去理解就错了。
    use std::path::Path;
    assert_eq!(Scope::remote("r2:", "img/2026").describe(), "r2:img/2026");
    assert_eq!(Scope::remote("r2:", "").describe(), "r2:");
    assert_eq!(Scope::local(Path::new("D:\\Vault\\img")).describe(), "d:/vault/img");
    assert_eq!(Scope::local(Path::new("D:\\")).describe(), "d:");
    // 第三种根形（不以冒号结尾的本地首段：UNC 与相对路径）也必须有证人：它没有结尾冒号可依据，
    // 那句回显全靠"首段与其余段之间插斜杠"这一条规则，插法的分支删掉就退化成直接拼。
    assert_eq!(Scope::local(Path::new("\\\\srv\\share")).describe(), "srv/share");
    assert_eq!(Scope::local(Path::new("srv")).describe(), "srv");
}
