//! 文本证人的共用小工具。
//!
//! 本仓没有 JS 测试运行器，所以"界面长什么样"这类判据只能读源文本来钉。
//! 读文本最容易犯的错是**因错的理由变红**：注释里提一句被禁的写法，证人就红在一份
//! 永远不会上界面的文字上——那种红教人改注释而不是改代码，比不红更坏。
//! 所以这里的每一条判据都从 `strip_comments` 开始。

#![allow(dead_code)] // 每个测试二进制各自编译一份，用不到的那几条不算死码

use std::path::PathBuf;

pub fn read(rel: &str) -> String {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("读不到 {}：{e}", p.display()))
}

/// 剥 `//` 与 `/* */`。够用即可：本仓的注释里不出现 JSX 属性写法，剥错也不会把代码当注释。
/// （真正的解析要引 JS 工具进来，那是另一件事，这条闸门不值那个价。）
pub fn strip_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut it = src.chars().peekable();
    while let Some(c) = it.next() {
        if c == '/' {
            match it.peek() {
                Some('/') => {
                    it.next();
                    while let Some(n) = it.next() {
                        if n == '\n' {
                            out.push(n);
                            break;
                        }
                    }
                    continue;
                }
                Some('*') => {
                    it.next();
                    while let Some(n) = it.next() {
                        if n == '*' && it.peek() == Some(&'/') {
                            it.next();
                            break;
                        }
                    }
                    continue;
                }
                _ => {}
            }
        }
        out.push(c);
    }
    out
}

/// 数一段文本里出现了多少次某个形状。钉"这个出口只有一个"用的就是它。
pub fn count_in(hay: &str, needle: &str) -> usize {
    hay.matches(needle).count()
}

/// 只留下**界面会说的话**：字符串字面量的内容 + JSX 里两个标签之间的那段文字。
///
/// 为什么要有这一份：内部名词那条禁令管的是"操作员看见的词"。拿整份文件扫，
/// `RcdStatusBar`、`p.token` 这些**标识符**会先红 —— 那种红教人改组件名，不教人改文案，
/// 比不红更坏（同一条口径：证人不要因错的理由变红）。
pub fn surface_text(src: &str) -> String {
    let b: Vec<char> = src.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c == '\'' || c == '"' || c == '`' {
            i = push_literal(&b, i, &mut out);
            continue;
        }
        if c == '>' {
            let mut j = i + 1;
            let mut seg = String::new();
            while j < b.len() && b[j] != '<' {
                seg.push(b[j]);
                j += 1;
            }
            // 带 { 的那段是表达式不是文字（`>{a.label}<` 这类），整段不收；
            // 但表达式**里面**引号括起来的还是要上界面的话 —— 表标题那种
            // `columns={[{ title: '缺这个文件' }]}` 就住在这里。以前这里整段跳过去，
            // 于是那一族的文案对证人完全隐形（拿变异枪打出来的：把已改的
            // 「缺这个对象」改回「缺这个文件」，证人照样绿）。
            if seg.contains('{') {
                let sb: Vec<char> = seg.chars().collect();
                let mut k = 0;
                while k < sb.len() {
                    let d = sb[k];
                    if d == '\'' || d == '"' || d == '`' {
                        k = push_literal(&sb, k, &mut out);
                        continue;
                    }
                    k += 1;
                }
            } else {
                out.push(' ');
                out.push_str(&seg);
            }
            i = j;
            continue;
        }
        i += 1;
    }
    out
}

/// 从 `i`（必须指向一个引号）起把那段字面量抄进 `out`，返回引号之后的位置。
/// 反斜杠按转义处理（`\n` 不进文本），闭合引号找不到就抄到结尾。
fn push_literal(b: &[char], i: usize, out: &mut String) -> usize {
    let q = b[i];
    let mut j = i + 1;
    out.push(' ');
    while j < b.len() && b[j] != q {
        if b[j] == '\\' {
            j += 1;
            if j < b.len() { out.push(b[j]); }
        } else {
            out.push(b[j]);
        }
        j += 1;
    }
    j + 1
}

/// 切出源文本里每一段 `<open …>…</close>`（自闭合 `<open …/>` 也算一段）。
/// 只为"某样东西不许藏在这个元件里"这类判据服务：判据落在结构上，
/// 而不是落在"不许出现某串固定文字"上——后者换个写法就绕过去了。
pub fn tag_spans(t: &str, open: &str, close: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(h) = t[at..].find(open) {
        let start = at + h;
        let gt = match t[start..].find('>') {
            Some(g) => start + g,
            None => break,
        };
        if t[start..gt].ends_with('/') {
            out.push((start, gt + 1));
            at = gt + 1;
            continue;
        }
        match t[gt..].find(close) {
            Some(c) => {
                let end = gt + c + close.len();
                out.push((start, end));
                at = end;
            }
            None => break,
        }
    }
    out
}
