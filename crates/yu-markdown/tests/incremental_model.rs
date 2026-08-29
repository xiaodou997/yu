use yu_core::{ByteOffset, TextRange};
use yu_markdown::{MarkdownDocument, parse, parse_incremental};
use yu_text::{Edit, TextBuffer, Transaction, retained_snapshot_stats};

const INSERTIONS: [&str; 16] = [
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
    // Setext 的下划线：块的身份由树给之后，它是**唯一一种往回看**的分类
    // ——插进来会把上一行变成一个标题。增量复用的边界只往回退一个块，这两
    // 条插入串就是那条推理的用例。
    "\n===\n",
    "---\n",
    " ",
    "\r\n",
    "",
];

#[test]
fn incremental_parse_matches_full_parse_through_random_edits() {
    run_model();
}

fn run_model() {
    let mut seed = 0x5955_4d41_524b_444f_u64;
    let mut model =
        String::from("# Yu\n\nparagraph\n\nsetext\n===\n\n```rust\nfn main() {}\n```\n\nafter\n");
    let mut buffer = TextBuffer::new(model.clone());
    let mut document = parse(&buffer.snapshot());

    for step in 0..1_000 {
        let boundaries = char_boundaries(&model);
        let first = random_index(&mut seed, boundaries.len());
        let second = random_index(&mut seed, boundaries.len());
        let (start_index, end_index) = if first <= second {
            (first, second)
        } else {
            (second, first)
        };
        let start = boundaries[start_index];
        let mut end = boundaries[end_index];
        if model.len() > 8_192 && start == end {
            end = model.len();
        }
        let inserted = INSERTIONS[random_index(&mut seed, INSERTIONS.len())];
        let range = TextRange::new(
            ByteOffset::try_from(start).expect("test offset should fit u64"),
            ByteOffset::try_from(end).expect("test offset should fit u64"),
        )
        .expect("ordered offsets should form a range");
        let transaction = Transaction::new(buffer.revision(), [Edit::new(range, inserted)]);
        let applied = buffer
            .apply(&transaction)
            .expect("model-generated edit should apply");
        model.replace_range(start..end, inserted);

        let incremental =
            parse_incremental(&document, applied.result_snapshot(), applied.change_set())
                .expect("matching revisions should parse incrementally");
        let full = parse(applied.result_snapshot());
        assert_documents_equal(incremental.document(), &full, step);
        assert_eq!(applied.result_snapshot().as_str(), model);
        document = incremental.into_document();
    }
}

fn assert_documents_equal(incremental: &MarkdownDocument, full: &MarkdownDocument, step: usize) {
    assert_eq!(incremental, full, "diverged at step {step}");
    assert!(
        incremental.has_lossless_coverage(),
        "lost coverage at step {step}"
    );
}

fn char_boundaries(text: &str) -> Vec<usize> {
    text.char_indices()
        .map(|(index, _)| index)
        .chain(std::iter::once(text.len()))
        .collect()
}

fn random_index(seed: &mut u64, upper_bound: usize) -> usize {
    *seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
    ((*seed >> 32) as usize) % upper_bound
}

#[test]
fn deleting_fence_delimiters_propagates_to_eof() {
    for (source, needle) in [
        ("before\n\n```\ninside\n```\n\nafter\n", "```\ninside"),
        ("before\n\n```\ninside\n```\n\nafter\n", "```\n\nafter"),
    ] {
        let mut buffer = TextBuffer::new(source);
        let previous = parse(&buffer.snapshot());
        let start = source.find(needle).expect("fixture contains delimiter");
        let range = TextRange::new(
            ByteOffset::try_from(start).expect("offset should fit u64"),
            ByteOffset::try_from(start + 3).expect("offset should fit u64"),
        )
        .expect("ordered offsets should form a range");
        let transaction = Transaction::new(buffer.revision(), [Edit::new(range, "")]);
        let applied = buffer
            .apply(&transaction)
            .expect("delimiter deletion should apply");

        let incremental =
            parse_incremental(&previous, applied.result_snapshot(), applied.change_set())
                .expect("matching revisions should parse incrementally");
        let full = parse(applied.result_snapshot());

        assert_eq!(incremental.document(), &full);
        assert!(incremental.document().has_lossless_coverage());
        assert!(incremental.reparsed_range().end() == applied.result_snapshot().len_bytes());
    }
}

#[test]
fn local_edit_shares_persistent_prefix_and_shifted_suffix() {
    let source = "# one\n\nalpha\n\n# two\n\nomega\n";
    let mut buffer = TextBuffer::new(source);
    let previous_snapshot = buffer.snapshot();
    let previous = parse(&previous_snapshot);
    let insert_at = source.find("alpha").expect("fixture contains alpha") + 2;
    let transaction = Transaction::new(
        buffer.revision(),
        [Edit::new(
            TextRange::empty(ByteOffset::try_from(insert_at).expect("offset fits u64")),
            "羽",
        )],
    );
    let applied = buffer.apply(&transaction).expect("edit should apply");
    let materialized_before =
        retained_snapshot_stats(&[previous_snapshot.clone(), applied.result_snapshot().clone()])
            .materialized_buffers();

    let incremental = parse_incremental(&previous, applied.result_snapshot(), applied.change_set())
        .expect("matching revisions should parse incrementally");
    let full = parse(applied.result_snapshot());
    let shared = incremental
        .document()
        .blocks()
        .shared_blocks_with(previous.blocks());

    assert_eq!(incremental.document(), &full);
    assert_eq!(
        retained_snapshot_stats(&[previous_snapshot, applied.result_snapshot().clone()])
            .materialized_buffers(),
        materialized_before,
        "materialized source during convergence"
    );
    assert_eq!(incremental.reused_prefix_blocks(), 1);
    assert_eq!(incremental.reused_suffix_blocks(), 4);
    assert_eq!(shared, 5);
    assert!(
        incremental.reparsed_range().end() < applied.result_snapshot().len_bytes(),
        "scanned through EOF"
    );
    assert_eq!(incremental.document().block_storage_stats().segments(), 3);
    assert_eq!(
        incremental.document().block_storage_stats().allocations(),
        2
    );
}

#[test]
fn inserted_fence_prevents_false_hash_convergence() {
    let source = "before\n\ninside\n\n# repeated\n\ninside\n";
    let mut buffer = TextBuffer::new(source);
    let previous = parse(&buffer.snapshot());
    let fence_at = source.find("inside").expect("fixture contains paragraph");
    let transaction = Transaction::new(
        buffer.revision(),
        [Edit::new(
            TextRange::empty(ByteOffset::try_from(fence_at).expect("offset fits u64")),
            "```\n",
        )],
    );
    let applied = buffer.apply(&transaction).expect("edit should apply");
    let incremental = parse_incremental(&previous, applied.result_snapshot(), applied.change_set())
        .expect("matching revisions should parse incrementally");
    let full = parse(applied.result_snapshot());

    assert_eq!(incremental.document(), &full);
    assert_eq!(incremental.reused_suffix_blocks(), 0);
    assert_eq!(
        incremental.reparsed_range().end(),
        applied.result_snapshot().len_bytes()
    );
}

#[test]
fn container_marker_edit_matches_full_parse() {
    let source = "> quote\n\n- one\n  continuation\n  - nested\n- two\n\nafter\n";
    let mut buffer = TextBuffer::new(source);
    let previous = parse(&buffer.snapshot());
    let marker_at = source.find("- one").expect("fixture contains list marker");
    let transaction = Transaction::new(
        buffer.revision(),
        [Edit::new(
            TextRange::new(
                ByteOffset::try_from(marker_at).expect("offset fits u64"),
                ByteOffset::try_from(marker_at + 2).expect("offset fits u64"),
            )
            .expect("ordered marker range"),
            "1. ",
        )],
    );
    let applied = buffer
        .apply(&transaction)
        .expect("container marker edit should apply");
    let incremental = parse_incremental(&previous, applied.result_snapshot(), applied.change_set())
        .expect("matching revisions should parse incrementally");
    let full = parse(applied.result_snapshot());

    assert_eq!(incremental.document(), &full);
    assert!(incremental.document().has_lossless_coverage());
    assert_eq!(
        applied.result_snapshot().as_str(),
        source.replacen("- ", "1. ", 1)
    );
}

// ------------------------------------------------------------ 跟着文档的树

/// 增量解析出来的树必须与全量解析的逐节点相同（不变量 C3 在这一层的形状）。
///
/// `yu-syntax` 的差分测试压的是 `parse_with_fragments` 本身；这里压的是**接
/// 线**——fragment 是不是真的取自上一版文档的树、`ChangeSet` 有没有真的交给
/// 它。接错的后果不是崩，是树悄悄不对：复用了一段本不该复用的字节，而画面上
/// 只是某一块的语法没被藏掉。
#[test]
fn the_documents_tree_survives_incremental_edits() {
    let source = "# 标题\n\n段落一\n\n- [ ] 待办\n\n```rust\nfn main() {}\n```\n\n> 引用\n";
    for offset in [0_u64, 4, 12, 22, 34, 48] {
        for insert in ["x", "\n", "`", "> ", "- "] {
            let mut buffer = TextBuffer::new(source.to_owned());
            let document = yu_markdown::parse(&buffer.snapshot());
            let offset = offset.min(buffer.snapshot().len_bytes().get());
            let Some(range) = TextRange::new(ByteOffset::new(offset), ByteOffset::new(offset))
            else {
                continue;
            };
            let transaction = Transaction::new(buffer.revision(), [Edit::new(range, insert)]);
            let Ok(applied) = buffer.apply(&transaction) else {
                continue;
            };
            let incremental = yu_markdown::parse_incremental(
                &document,
                applied.result_snapshot(),
                applied.change_set(),
            )
            .expect("增量解析");
            let full = yu_markdown::parse(applied.result_snapshot());
            assert_eq!(
                incremental.document().tree(),
                full.tree(),
                "在 {offset} 处插入 {insert:?} 之后，增量的树与全量不一致"
            );
        }
    }
}

/// **J1 的可断言量**：一次单字符编辑重扫的字节数与文档大小无关。
///
/// 这条从 `yu-editor::DecorationCache` 搬过来——树跟着文档走之后，接线在这里。
///
/// 只断言「小于某个上界」是不够的：把上界定得足够大，退化成全量重扫也能过。
/// 所以同时断言它**不随文档增长**——文档大 8 倍，重扫的字节数不许跟着涨。
#[test]
fn one_edit_rescans_a_bounded_number_of_bytes() {
    /// 上限留到 256 是给块大小的余量。判据是它必须小到让「复用失效」一定越界
    /// ——最小的那份文档全量重扫就有三千多字节。
    const BUDGET: u32 = 256;

    fn many_blocks(blocks: usize) -> String {
        let mut source = String::new();
        for index in 0..blocks {
            source.push_str(&format!(
                "## Section {index}\n\nParagraph {index} with *emphasis* and `code`.\n\n"
            ));
        }
        source
    }

    let rescanned = |blocks: usize| -> u32 {
        let source = many_blocks(blocks);
        let mut buffer = TextBuffer::new(source.clone());
        let document = yu_markdown::parse(&buffer.snapshot());
        let middle = source
            .char_indices()
            .map(|(index, _)| index)
            .nth(source.len() / 2)
            .expect("文档够长");
        let at = ByteOffset::new(middle as u64);
        let range = TextRange::new(at, at).expect("空区间");
        let applied = buffer
            .apply(&Transaction::new(
                buffer.revision(),
                [Edit::new(range, "X")],
            ))
            .expect("插入");
        yu_markdown::parse_incremental(&document, applied.result_snapshot(), applied.change_set())
            .expect("增量解析")
            .document()
            .reparsed_bytes()
    };

    let small = rescanned(64);
    let large = rescanned(512);
    assert!(
        small <= BUDGET && large <= BUDGET,
        "单字符编辑重扫了 {small} / {large} 字节，超出上界 {BUDGET}"
    );
    assert!(
        large <= small.saturating_mul(2),
        "文档大 8 倍，重扫字节数从 {small} 涨到 {large}——复用没接上"
    );
}
