//! 语法树自身的不变量（C1、C2、C4）。
//!
//! `commonmark_spec.rs` 比的是渲染成 HTML 之后的结果，它管不到「节点的
//! source range 对不对」——而 Yu 用的正是 range：投影、隐藏语法、把编辑落回
//! 源码，全都靠它。HTML 相同而 range 错位是完全可能的，那正是这一轮点名的
//! 「静默地做错事」。
//!
//! 这里把规范的 652 条输入当语料，对每一棵树逐条检查结构性质。

use std::path::PathBuf;

use yu_syntax::{NodeKind, Tree, parse};

fn spec_inputs() -> Vec<(usize, String)> {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../third_party/commonmark/spec.json");
    let raw = std::fs::read(&path).expect("规范用例应该在仓库里");
    let parsed: serde_json::Value = serde_json::from_slice(&raw).expect("合法 JSON");
    parsed
        .as_array()
        .expect("顶层是数组")
        .iter()
        .map(|value| {
            (
                value["example"].as_u64().unwrap_or(0) as usize,
                value["markdown"].as_str().unwrap_or("").to_owned(),
            )
        })
        .collect()
}

/// 深度优先遍历，回调收到 (节点, 绝对起点, 绝对终点, 深度)。
fn walk(tree: &Tree, from: u32, depth: usize, visit: &mut impl FnMut(&Tree, u32, u32, usize)) {
    visit(tree, from, from + tree.len_bytes(), depth);
    for index in 0..tree.child_count() {
        let (child, position) = tree.child(index).expect("下标来自 child_count");
        walk(child, from + position, depth + 1, visit);
    }
}

/// **C1**：range 有序、有效，且不超出源码。
///
/// 「有效」在这里是三条：起点不大于终点、子节点被父节点完全包住、
/// 兄弟节点按起点升序且不交叉。第三条尤其重要——交叉的 range 会让装饰阶段
/// 的区间合并给出没有意义的结果，而它不会 panic。
#[test]
fn ranges_are_ordered_valid_and_within_the_source() {
    for (number, source) in spec_inputs() {
        let parsed = parse(source.as_str()).expect("规范用例都很短");
        let tree = parsed.tree();
        let len = u32::try_from(source.len()).expect("规范用例都很短");
        assert_eq!(
            tree.kind(),
            NodeKind::Document,
            "用例 #{number} 的根不是 Document"
        );

        walk(tree, 0, 0, &mut |node, from, to, _| {
            assert!(
                from <= to,
                "用例 #{number}：{} 的 range 倒置",
                node.kind().name()
            );
            assert!(
                to <= len,
                "用例 #{number}：{} 的终点 {to} 超出源码长度 {len}",
                node.kind().name()
            );
            assert!(
                source.is_char_boundary(from as usize) && source.is_char_boundary(to as usize),
                "用例 #{number}：{} 的 range {from}..{to} 落在 UTF-8 字符中间",
                node.kind().name()
            );

            let mut previous_end = from;
            for index in 0..node.child_count() {
                let (child, position) = node.child(index).expect("下标来自 child_count");
                let child_from = from + position;
                let child_to = child_from + child.len_bytes();
                assert!(
                    child_from >= previous_end,
                    "用例 #{number}：{} 的第 {index} 个子节点 {} 与前一个兄弟交叉",
                    node.kind().name(),
                    child.kind().name()
                );
                assert!(
                    child_to <= to,
                    "用例 #{number}：{} 的子节点 {} 超出父节点范围",
                    node.kind().name(),
                    child.kind().name()
                );
                previous_end = child_to;
            }
        });
    }
}

/// **C2**：lossless。
///
/// 定义（不变量 C2）是「相邻节点之间的 gap 必须可由 position 精确推导，
/// 且推导结果与原始字节完全一致」，不要求每个字节都有节点。这里就按这个
/// 定义验：沿着树把节点与 gap 依次拼起来，必须字节级还原源码。
#[test]
fn source_is_recoverable_from_positions_alone() {
    for (number, source) in spec_inputs() {
        let parsed = parse(source.as_str()).expect("规范用例都很短");
        let rebuilt = rebuild(parsed.tree(), 0, &source);
        assert_eq!(
            rebuilt, source,
            "用例 #{number}：从树的 position 推不回原始字节"
        );
    }
}

/// 只用 position 与源码切片重建文本：叶子节点取自己的切片，非叶子节点
/// 取「子节点 + 它们之间的 gap」。
fn rebuild(tree: &Tree, from: u32, source: &str) -> String {
    let to = from + tree.len_bytes();
    if tree.child_count() == 0 {
        return source[from as usize..to as usize].to_owned();
    }
    let mut out = String::new();
    let mut cursor = from;
    for index in 0..tree.child_count() {
        let (child, position) = tree.child(index).expect("下标来自 child_count");
        let child_from = from + position;
        if child_from > cursor {
            out.push_str(&source[cursor as usize..child_from as usize]);
        }
        out.push_str(&rebuild(child, child_from, source));
        cursor = (child_from + child.len_bytes()).max(cursor);
    }
    if cursor < to {
        out.push_str(&source[cursor as usize..to as usize]);
    }
    out
}

/// **C5**：未闭合或畸形的 Markdown 不得丢内容，也不得凭空造语义节点。
///
/// 上一条已经保证了「不丢内容」（能字节级还原）。这里补另一半：一段没有
/// 任何 Markdown 语法的纯文本，不应该解析出除 Paragraph 之外的东西。
#[test]
fn malformed_input_keeps_its_bytes_and_invents_nothing() {
    let cases = [
        "```rust\nunclosed fence\n",
        "> quote without end",
        "- item\n  - nested\n    unfinished",
        "[link](unclosed\n",
        "***\n**bold without end\n",
        "| a | b |\n| - |\n",
        "\u{feff}bom then text\n",
        "text with \0 nul\n",
    ];
    for source in cases {
        let parsed = parse(source).expect("短输入");
        assert_eq!(
            rebuild(parsed.tree(), 0, source),
            source,
            "{source:?} 的字节没被完整保留"
        );
    }

    // 纯文本不该长出语义节点。
    let plain = "just words and numbers 123 across\ntwo lines\n";
    let parsed = parse(plain).expect("短输入");
    assert_eq!(parsed.tree().to_sexp(), "Document(Paragraph)");
}

/// **C4**：parser 不复制正文，节点只通过 range 引用源码。
///
/// 分两条可证的性质：
///
/// 1. **树不借用源码。** 下面的块里 `String` 在块结束时被释放，而树还活着。
///    这是编译期事实：只要 `Tree` 里出现一个 `&str`，这段代码就编译不过。
/// 2. **树的规模只由结构决定，与正文长度无关。** 同样结构、正文长度差三个
///    数量级的两份文档，节点数必须完全相同。会复制正文的实现做不到这一点
///    ——它要么把文本切成多个节点，要么让某个节点持有随长度增长的数据。
#[test]
fn nodes_carry_ranges_not_text() {
    let tree = {
        let owned = String::from("# heading\n\nparagraph with *emphasis*\n");
        parse(owned.as_str()).expect("短输入").into_tree()
        // `owned` 在这里被释放。
    };
    assert_eq!(tree.kind(), NodeKind::Document);
    assert!(tree.child_count() > 0);

    let short = format!("# t\n\n*{}*\n", "a".repeat(8));
    let long = format!("# t\n\n*{}*\n", "a".repeat(8 * 1_000));
    let short_tree = parse(short.as_str()).expect("短输入").into_tree();
    let long_tree = parse(long.as_str()).expect("长输入").into_tree();
    assert_eq!(
        node_count(&short_tree),
        node_count(&long_tree),
        "正文长了 1000 倍，节点数却变了——说明正文进了树"
    );
    assert_eq!(short_tree.to_sexp(), long_tree.to_sexp());
    assert_ne!(short_tree.len_bytes(), long_tree.len_bytes());
}

fn node_count(tree: &Tree) -> usize {
    let mut count = 0_usize;
    walk(tree, 0, 0, &mut |_, _, _, _| count += 1);
    count
}

/// 标记节点必须精确覆盖语法字符本身。
///
/// 这是「隐藏未聚焦的 Markdown 语法」唯一能依赖的东西：装饰阶段拿这些 range
/// 去做 `Replace`，多一个字节就吃掉正文，少一个字节就漏出语法。
#[test]
fn syntax_marks_cover_exactly_the_syntax_characters() {
    let cases: &[(&str, &[(NodeKind, &str)])] = &[
        ("## title\n", &[(NodeKind::HeaderMark, "##")]),
        (
            "*em*\n",
            &[(NodeKind::EmphasisMark, "*"), (NodeKind::EmphasisMark, "*")],
        ),
        (
            "**strong**\n",
            &[
                (NodeKind::EmphasisMark, "**"),
                (NodeKind::EmphasisMark, "**"),
            ],
        ),
        ("> quoted\n", &[(NodeKind::QuoteMark, ">")]),
        ("- item\n", &[(NodeKind::ListMark, "-")]),
        ("12. item\n", &[(NodeKind::ListMark, "12.")]),
        (
            "`code`\n",
            &[(NodeKind::CodeMark, "`"), (NodeKind::CodeMark, "`")],
        ),
        (
            "```rust\nx\n```\n",
            &[
                (NodeKind::CodeMark, "```"),
                (NodeKind::CodeInfo, "rust"),
                (NodeKind::CodeMark, "```"),
            ],
        ),
        (
            "[text](/url \"t\")\n",
            &[
                (NodeKind::LinkMark, "["),
                (NodeKind::LinkMark, "]"),
                (NodeKind::LinkMark, "("),
                (NodeKind::Url, "/url"),
                (NodeKind::LinkTitle, "\"t\""),
                (NodeKind::LinkMark, ")"),
            ],
        ),
        ("title\n=====\n", &[(NodeKind::HeaderMark, "=====")]),
    ];

    for (source, expected) in cases {
        let parsed = parse(*source).expect("短输入");
        let mut found: Vec<(NodeKind, &str)> = Vec::new();
        walk(parsed.tree(), 0, 0, &mut |node, from, to, _| {
            if matches!(
                node.kind(),
                NodeKind::HeaderMark
                    | NodeKind::QuoteMark
                    | NodeKind::ListMark
                    | NodeKind::LinkMark
                    | NodeKind::EmphasisMark
                    | NodeKind::CodeMark
                    | NodeKind::CodeInfo
                    | NodeKind::LinkTitle
                    | NodeKind::Url
            ) {
                found.push((node.kind(), &source[from as usize..to as usize]));
            }
        });
        assert_eq!(
            found,
            expected.to_vec(),
            "{source:?} 的语法标记覆盖范围不对"
        );
    }
}

/// 硬换行的行尾符可以是 `\n`、`\r\n` 或单独的 `\r`。
///
/// 只认 `\n` 的话，CRLF 文档里的硬换行整个失效：`HardBreak` 节点不存在，
/// 两个尾随空格变成可见内容，换行也不再是硬的。不报错，只是画面不对——
/// 而 Windows 上存的文件全是 CRLF。CommonMark 的 spec 用例只用 `\n`，
/// 压不住这一条，所以它在这里。
#[test]
fn hard_breaks_accept_every_line_ending() {
    let cases: &[(&str, &[(u32, u32)])] = &[
        ("a  \nb", &[(1, 4)]),
        ("a  \r\nb", &[(1, 5)]),
        ("a  \rb", &[(1, 4)]),
        ("a\\\nb", &[(1, 3)]),
        ("a\\\r\nb", &[(1, 4)]),
        // 一个空格不够，两个才算硬换行。
        ("a \nb", &[]),
        ("a \r\nb", &[]),
    ];

    for (source, expected) in cases {
        let parsed = parse(*source).expect("短输入");
        let mut found: Vec<(u32, u32)> = Vec::new();
        walk(parsed.tree(), 0, 0, &mut |node, from, to, _| {
            if node.kind() == NodeKind::HardBreak {
                found.push((from, to));
            }
        });
        assert_eq!(found, expected.to_vec(), "{source:?} 的硬换行范围不对");
    }
}
