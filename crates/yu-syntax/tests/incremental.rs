//! **C3**：`incremental_parse(edit(old))` 必须与 `full_parse(new)` 等价。
//!
//! 不变量原文写明「等价性由差分测试守护，不由人工推理保证」。这份文件就是
//! 那个守护。它的形状来自 `crates/yu-markdown/tests/incremental_model.rs`
//! ——那是 v1 时期为扫描器建的安全网，随机编辑 1000 步，每步比对增量与全量。
//!
//! 与那份的两点不同：
//!
//! 1. **语料更宽。** 除了随机编辑，还拿 CommonMark 规范的 652 条输入逐条做
//!    「在每个位置插一个字符」的穷举，因为规范用例恰好是容器嵌套、围栏、
//!    引用定义这些最容易在增量时错的构造的集合。
//! 2. **同时守重解析上界。** 等价只说明结果对，不说明代价对。J1 要求
//!    「编辑只重解析受影响范围」，这里把它变成一个可断言的字节数。

use yu_core::{ByteOffset, TextRange};
use yu_syntax::{Tree, TreeFragment, parse, parse_with_fragments};
use yu_text::{Edit, TextBuffer, Transaction};

/// 一步编辑：在 `range` 处替换成 `insert`，返回新文本、新树与增量解析读到的字节数。
struct Step {
    text: String,
    incremental: Tree,
    fragments: Vec<TreeFragment>,
    reparsed_bytes: u32,
}

fn apply(
    buffer: &mut TextBuffer,
    fragments: &[TreeFragment],
    range: TextRange,
    insert: &str,
) -> Step {
    let transaction = Transaction::new(buffer.revision(), [Edit::new(range, insert)]);
    let applied = buffer.apply(&transaction).expect("编辑应当合法");
    let snapshot = applied.result_snapshot();
    let moved = TreeFragment::apply_change_set(fragments, applied.change_set());
    let parsed = parse_with_fragments(snapshot, &moved).expect("测试文档不会超长");
    Step {
        text: snapshot.as_str().to_owned(),
        fragments: TreeFragment::from_tree(parsed.tree()),
        reparsed_bytes: parsed.reparsed_bytes(),
        incremental: parsed.into_tree(),
    }
}

fn range_of(start: usize, end: usize) -> TextRange {
    TextRange::new(
        ByteOffset::try_from(start).expect("测试偏移不会溢出"),
        ByteOffset::try_from(end).expect("测试偏移不会溢出"),
    )
    .expect("有序偏移构成合法 range")
}

/// 与 `yu-markdown` 那份模型测试同一组插入串：覆盖多字节、emoji、
/// 各类块起始标记、换行与 CRLF。
const INSERTIONS: [&str; 14] = [
    "羽",
    "🙂",
    "#",
    "```",
    "~~~\n",
    "> ",
    "- ",
    "1. ",
    "  - ",
    "\n",
    "\n# title\n",
    " ",
    "\r\n",
    "",
];

fn next_random(seed: &mut u64) -> u64 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    *seed
}

fn random_index(seed: &mut u64, length: usize) -> usize {
    if length == 0 {
        0
    } else {
        usize::try_from(next_random(seed) % length as u64).unwrap_or(0)
    }
}

fn char_boundaries(text: &str) -> Vec<usize> {
    text.char_indices()
        .map(|(index, _)| index)
        .chain(std::iter::once(text.len()))
        .collect()
}

#[test]
fn incremental_matches_full_parse_through_random_edits() {
    let mut seed = 0x5955_4d41_524b_444f_u64;
    let mut model = String::from(
        "# Yu\n\nparagraph\n\n```rust\nfn main() {}\n```\n\n> quote\n> more\n\n- a\n- b\n\n[r]: /url\n\nsee [r]\n",
    );
    let mut buffer = TextBuffer::new(model.clone());
    let mut fragments = {
        let parsed = parse(&buffer.snapshot()).expect("初始文档不会超长");
        TreeFragment::from_tree(parsed.tree())
    };

    for step_index in 0..1_000 {
        let boundaries = char_boundaries(&model);
        let first = random_index(&mut seed, boundaries.len());
        let second = random_index(&mut seed, boundaries.len());
        let (low, high) = if first <= second {
            (first, second)
        } else {
            (second, first)
        };
        let start = boundaries[low];
        let mut end = boundaries[high];
        // 别让模型无限膨胀。
        if model.len() > 8_192 && start == end {
            end = model.len();
        }
        let insert = INSERTIONS[random_index(&mut seed, INSERTIONS.len())];

        let step = apply(&mut buffer, &fragments, range_of(start, end), insert);
        model.replace_range(start..end, insert);
        assert_eq!(
            step.text, model,
            "第 {step_index} 步：缓冲区与模型文本不一致"
        );

        let full = parse(step.text.as_str())
            .expect("测试文档不会超长")
            .into_tree();
        assert_trees_equal(&step.incremental, &full, &step.text, step_index);
        fragments = step.fragments;
    }
}

/// 规范用例当语料：对每一条输入，在每一个字符边界上插一个字符，
/// 逐一验证增量与全量等价。
///
/// 随机编辑走的是「一份文档连续演化」的路径，容易在同一片区域里打转；
/// 这一条相反，是宽而浅的穷举，覆盖 652 种结构各自的每一个位置。
#[test]
fn incremental_matches_full_parse_on_every_spec_input() {
    for (number, source) in spec_inputs() {
        if source.is_empty() {
            continue;
        }
        for insert in ["x", "\n", "#", "`", ">", "-", "*", "]"] {
            for offset in char_boundaries(&source) {
                let mut buffer = TextBuffer::new(source.clone());
                let fragments = {
                    let parsed = parse(&buffer.snapshot()).expect("规范用例都很短");
                    TreeFragment::from_tree(parsed.tree())
                };
                let step = apply(&mut buffer, &fragments, range_of(offset, offset), insert);
                let full = parse(step.text.as_str())
                    .expect("规范用例都很短")
                    .into_tree();
                assert_trees_equal(
                    &step.incremental,
                    &full,
                    &step.text,
                    number * 10_000 + offset,
                );
            }
        }
    }
}

/// 删除也要验：删掉一个围栏定界符会让它之后的所有内容改变含义，
/// 是增量复用最容易多复用的场景。
#[test]
fn deleting_a_fence_delimiter_propagates_to_the_end_of_the_document() {
    let source = "intro\n\n```\ncode\n```\n\ntail *em*\n\n## heading\n";
    let mut buffer = TextBuffer::new(source.to_owned());
    let mut fragments = {
        let parsed = parse(&buffer.snapshot()).expect("短文档");
        TreeFragment::from_tree(parsed.tree())
    };

    let closing = source.rfind("```").expect("有闭合围栏");
    let step = apply(&mut buffer, &fragments, range_of(closing, closing + 3), "");
    let full = parse(step.text.as_str()).expect("短文档").into_tree();
    assert_trees_equal(&step.incremental, &full, &step.text, 0);
    // 闭合围栏没了，后面的一切都被吸进代码块。
    assert_eq!(
        step.incremental.to_sexp(),
        "Document(Paragraph,FencedCode(CodeMark,CodeText))",
        "删掉闭合定界符之后，文档尾部应该整体成为未闭合代码块"
    );

    // 再把它加回去，结构必须完全恢复。
    fragments = step.fragments;
    let step = apply(&mut buffer, &fragments, range_of(closing, closing), "```");
    let full = parse(step.text.as_str()).expect("短文档").into_tree();
    assert_trees_equal(&step.incremental, &full, &step.text, 1);
    assert_eq!(step.text, source);
}

/// **J1 的量化上界**：一次单字符编辑重新扫描的字节数，必须与文档大小无关。
///
/// 断言的是字节数而不是耗时。耗时随机器和负载浮动，拿它当门禁只会得到一条
/// 时不时变红的检查，然后被调松到失去意义；字节数对同样的输入永远给同样的
/// 答案，退化时一定是真的退化。
#[test]
fn a_single_character_edit_rescans_a_bounded_number_of_bytes() {
    /// 一次单字符编辑允许重新扫描的字节数上限。
    ///
    /// 实测值是 66 字节（约等于被改动的那一个块），对三种文档大小都一样。
    /// 上限定在 256 是留给块大小波动的余量，不是「反正差不多」——判据是它
    /// 必须小到让「退化成全量重扫」一定越界：最小的那份文档就有 3,628 字节，
    /// 一旦复用失效，实测会跳到 3,373。
    const BUDGET: u32 = 256;

    for blocks in [64_usize, 256, 1_024] {
        let mut source = String::new();
        for index in 0..blocks {
            source.push_str(&format!(
                "## Section {index}\n\nParagraph {index} with *emphasis* and `code`.\n\n"
            ));
        }
        let total = u32::try_from(source.len()).expect("测试文档不会超长");
        let mut buffer = TextBuffer::new(source.clone());
        let fragments = {
            let parsed = parse(&buffer.snapshot()).expect("测试文档不会超长");
            TreeFragment::from_tree(parsed.tree())
        };

        // 改文档正中间的一个字符。
        let middle = char_boundaries(&source)[blocks / 2 * 20];
        let step = apply(&mut buffer, &fragments, range_of(middle, middle), "X");
        let full = parse(step.text.as_str())
            .expect("测试文档不会超长")
            .into_tree();
        assert_trees_equal(&step.incremental, &full, &step.text, 0);

        assert!(
            step.reparsed_bytes <= BUDGET,
            "{blocks} 个块（{total} 字节）的文档里改一个字符，重新扫描了 \
             {} 字节，超出上界 {BUDGET}",
            step.reparsed_bytes
        );
    }
}

/// 上界不能只在小文档上成立。这条检查它确实与文档大小**无关**：
/// 文档大 16 倍，重扫字节数不得跟着涨。
#[test]
fn the_rescan_bound_does_not_grow_with_the_document() {
    let measure = |blocks: usize| -> u32 {
        let mut source = String::new();
        for index in 0..blocks {
            source.push_str(&format!("## S{index}\n\nP{index} text here.\n\n"));
        }
        let mut buffer = TextBuffer::new(source.clone());
        let fragments = {
            let parsed = parse(&buffer.snapshot()).expect("测试文档不会超长");
            TreeFragment::from_tree(parsed.tree())
        };
        let middle = char_boundaries(&source)[blocks / 2 * 10];
        apply(&mut buffer, &fragments, range_of(middle, middle), "X").reparsed_bytes
    };

    let small = measure(64);
    let large = measure(1_024);
    assert!(
        large <= small * 2,
        "文档大了 16 倍，单字符编辑的重扫字节数从 {small} 涨到 {large}——\
         说明复用没生效，退化成了全量重扫"
    );
}

// ---------------------------------------------------------------------------

fn assert_trees_equal(incremental: &Tree, full: &Tree, source: &str, step: usize) {
    if incremental != full {
        panic!(
            "第 {step} 步：增量与全量不一致\n源码 {source:?}\n增量 {}\n全量 {}",
            incremental.to_sexp(),
            full.to_sexp()
        );
    }
    assert_eq!(
        incremental.len_bytes() as usize,
        source.len(),
        "第 {step} 步：树的长度与源码不符"
    );
}

fn spec_inputs() -> Vec<(usize, String)> {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../third_party/commonmark/spec.json");
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
