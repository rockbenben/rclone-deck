//! 预览登记处本身的证人（`src-tauri/src/preview.rs`）。
//!
//! 这里不起 daemon：登记表管的只是"谁看过哪一份清单、那份清单关于哪些位置"，
//! 那些判断（单次消费、按 Scope 作废、发新预览只顶掉同一组位置）都在纯逻辑这一层就判得出来。
//! 端到端那一半（预览 → 中途写一次 → 确认被作废）在 `tests/mirror_runner.rs`，那边一条判据都不许改。

use std::collections::BTreeSet;

use rclone_deck_lib::preview::{PreviewIntent, PreviewRegistry, StoredPreview};
use rclone_deck_lib::scope::Scope;

fn stored(intent: PreviewIntent, scopes: &[&str], names: &[&str]) -> StoredPreview {
    StoredPreview {
        intent,
        // 简报这句写的是 `names.iter().cloned()`：`names` 是 `&[&str]`，cloned 交出的是 `&str`，
        // 而 `BTreeSet<String>` 没有 `FromIterator<&str>`（实测 E0277）⇒ 换成 `to_string()`，
        // 集合形状不变。
        doomed: Some(names.iter().map(|s| s.to_string()).collect::<BTreeSet<String>>()),
        scopes: scopes.iter().map(|p| Scope::remote("decklocal:", p)).collect(),
    }
}

/// 「批准这一趟、没看逐条名单」那份凭证。
/// 空集**不是**这一档：空集说的是"看过一份名单，那份名单是空的"，那是合法形状。
fn stored_without_a_list(intent: PreviewIntent, scopes: &[&str]) -> StoredPreview {
    StoredPreview { intent, doomed: None, scopes: scopes.iter().map(|p| Scope::remote("decklocal:", p)).collect() }
}

#[tokio::test]
async fn a_confirmation_without_a_list_is_voided_by_a_write_just_like_one_with_a_list() {    // 新形状最容易漏的一条：它没有名单，所以作废机器只能靠 scopes 认它。
    let r = PreviewRegistry::default();
    let t = r.issue(stored_without_a_list(
        PreviewIntent::Mirror { job_id: "m".into() }, &["imgprompt"])).await;
    r.invalidate(&[Scope::remote("decklocal:", "imgprompt")]).await;
    assert!(r.consume(&t).await.is_none(),
        "写过之后的这份确认还作数，等于让人批过一次就永远批着");
}

#[tokio::test]
async fn a_token_is_spent_once_and_the_second_ask_finds_nothing() {
    let r = PreviewRegistry::default();
    let t = r.issue(stored(PreviewIntent::Mirror { job_id: "m".into() }, &[""], &["o1"])).await;
    assert!(r.consume(&t).await.is_some());
    assert!(r.consume(&t).await.is_none(), "同一份凭证花第二次 = 同一份清单被确认两次");
}

#[tokio::test]
async fn invalidation_reaches_deeper_previews_and_spares_unrelated_ones() {
    let r = PreviewRegistry::default();
    let whole = r.issue(stored(PreviewIntent::Mirror { job_id: "a".into() }, &[""], &["x"])).await;
    let sub = r.issue(stored(PreviewIntent::Mirror { job_id: "b".into() }, &["imgprompt"], &["y"])).await;
    let other = r.issue(stored(PreviewIntent::Mirror { job_id: "c".into() }, &["other"], &["z"])).await;
    // 有人往 imgprompt 里写了东西：整桶那份与子目录那份都不该再作数，另一份不受影响。
    r.invalidate(&[Scope::remote("decklocal:", "imgprompt")]).await;
    assert!(r.consume(&whole).await.is_none() && r.consume(&sub).await.is_none());
    assert!(r.consume(&other).await.is_some(), "把不相干的凭证一起清掉会让操作员白看一次清单");
}

#[tokio::test]
async fn a_lookalike_prefix_does_not_invalidate_its_neighbour() {
    let r = PreviewRegistry::default();
    let img = r.issue(stored(PreviewIntent::Mirror { job_id: "a".into() }, &["img"], &["x"])).await;
    let prompt = r.issue(stored(PreviewIntent::Mirror { job_id: "b".into() }, &["imgprompt"], &["y"])).await;
    r.invalidate(&[Scope::remote("decklocal:", "img")]).await;
    assert!(r.consume(&img).await.is_none());
    assert!(r.consume(&prompt).await.is_some(), "Task 3 那条 lookalike 规矩在这里也得站住");
}

#[tokio::test]
async fn supersede_only_replaces_the_same_scope_set() {
    // 发新预览**不是写**：它只该顶掉"同一组位置的旧清单"，不该顺手作废别人正在核对的那一份。
    // 这条钉的是发凭证处那一次调用的原语义（同配对只留最新一份）没被顺手放宽成 invalidate。
    let r = PreviewRegistry::default();
    let pair = r.issue(stored(PreviewIntent::Mirror { job_id: "a".into() }, &["", "sub"], &["x"])).await;
    let nested = r.issue(stored(PreviewIntent::Mirror { job_id: "b".into() }, &["sub"], &["y"])).await;
    r.supersede(&[Scope::remote("decklocal:", ""), Scope::remote("decklocal:", "sub")]).await;
    assert!(r.consume(&pair).await.is_none(), "同一组 scope 的旧预览被新的顶掉");
    assert!(r.consume(&nested).await.is_some(), "scope 集合不等的那份不该被一次读操作作废");
}

#[tokio::test]
async fn a_token_carries_the_intent_it_was_issued_for() {
    // R78：界面上只有一个"动手"入口，所以"这份授权做什么"必须在 token 里，而不是在参数里。
    let r = PreviewRegistry::default();
    let t = r.issue(stored(PreviewIntent::DeleteNames { fs: "decklocal:".into(), names: vec!["a.webp".into()] }, &[""], &["a.webp"])).await;
    let s = r.consume(&t).await.expect("凭证要在");
    assert_eq!(s.intent, PreviewIntent::DeleteNames { fs: "decklocal:".into(), names: vec!["a.webp".into()] });
}

#[tokio::test]
#[should_panic(expected = "空 scopes")]
async fn an_empty_scope_set_is_refused_at_issue_because_it_can_never_be_voided() {
    // 空 `scopes` 的那份凭证**作废不掉**：`invalidate` 对空集的 `any()` 恒假，`supersede` 又只被
    // 另一个空集顶掉，于是"往任何位置写过东西"都动不了它 —— 它会一直躺在登记表上等一次实删。
    // 今天的唯一调用方（`preview_mirror`）永远带两个 Scope，所以这一格现在咬不到人；
    // 而 `PreviewIntent` 除 `Mirror` 外那四个变体都要从浏览器侧发（Task 9、11–13 那几条动作），
    // `issue` 是它们共同的入口，到那时"忘了带 Scope"就是静默的 fail-open。
    // 规矩长在 `PreviewRegistry::issue` 的边界上，这条证人举的是它**真的咬人**：panic 而不是"登记下来但没人管"。
    let r = PreviewRegistry::default();
    r.issue(stored(PreviewIntent::Mirror { job_id: "m".into() }, &[], &["o1"])).await;
}

/// 「看差异」看过的那份删除候选（2026-10-09）当基准：**方向敏感**。
///
/// 同一对位置两个方向的 `doomed` 互为补集 —— 拿反方向那份当基准，动手前那次比对会拿
/// 两个不相交的集合比相等，**多半每次都拒**。所以这条钉的是"反方向取不到"，而不是"正方向取得到"：
/// 只有前者才是这条出口真正的风险（前者取错 = 对齐永远跑不了，后者取不到 = 退回到没看名单那一档，安全）。
#[tokio::test]
async fn the_seen_list_is_never_handed_out_for_the_opposite_direction() {
    let r = PreviewRegistry::default();
    let src = Scope::remote("decklocal:", "img");
    let dst = Scope::remote("decklocal:", "img/2026");
    let seen: BTreeSet<String> = ["a.webp", "b.webp"].iter().map(|s| s.to_string()).collect();
    r.remember_seen(&src, &dst, seen).await;

    assert!(r.peek_seen(&src, &dst).await.is_some(), "正方向反而取不到：基准丢了，这一条出口就没意义了");
    assert!(r.peek_seen(&dst, &src).await.is_none(),
        "反方向取到了同一份集合：两个方向的 doomed 互为补集，拿它比相等会每次都拒");
    // 位置不同的两棵树之间也不许串：这不是同一个方向的同一件事。
    let elsewhere = Scope::remote("decklocal:", "other");
    assert!(r.peek_seen(&src, &elsewhere).await.is_none(), "基准串到别的树上了");
}

/// `peek` **不消费** —— 这一张表是缓存不是授权：取过一次还能再取，取不消费不产生任何副作用。
/// 钉的是"它不是凭证"这个性质：真凭证是 `consume`（取走即消费），这里不是。
#[tokio::test]
async fn peeking_the_seen_list_is_not_a_consumption() {
    let r = PreviewRegistry::default();
    let src = Scope::remote("decklocal:", "img");
    let dst = Scope::remote("decklocal:", "img/2026");
    r.remember_seen(&src, &dst, ["x.webp".to_string()].into_iter().collect()).await;
    let first = r.peek_seen(&src, &dst).await;
    let second = r.peek_seen(&src, &dst).await;
    assert!(first.is_some() && second.is_some(), "peek 消费掉了：这一张表是缓存不是授权");
}

/// 超上限的那一份**不记**，而不是记一半 —— 几十万条名字留在内存里是这个进程付不起的
/// （对照：`last_summaries` 只留计数是刻意的）。钉的是"忘了记"而不是"记坏了"：
/// 忘了记的后果是退回没看名单那一档（更弱但安全），记坏了才是拿一份残缺的集合当基准。
#[tokio::test]
async fn an_oversized_seen_list_is_remembered_as_nothing_rather_than_partly() {
    let r = PreviewRegistry::default();
    let src = Scope::remote("decklocal:", "img");
    let dst = Scope::remote("decklocal:", "img/2026");
    let huge: BTreeSet<String> = (0..60_000).map(|i| format!("o{i}")).collect();
    r.remember_seen(&src, &dst, huge).await;
    assert!(r.peek_seen(&src, &dst).await.is_none(), "超上限那份被截着记下来了：那是拿残缺集合当基准");
}