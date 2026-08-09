use yu_core::{ByteOffset, TextRange};
use yu_markdown::{MarkdownDocument, parse, parse_incremental};
use yu_text::{Edit, StorageBackend, TextBuffer, Transaction, retained_snapshot_stats};

const INSERTIONS: [&str; 10] = [
    "羽",
    "🙂",
    "#",
    "```",
    "~~~\n",
    "\n",
    "\n# title\n",
    " ",
    "\r\n",
    "",
];

#[test]
fn incremental_parse_matches_full_parse_through_random_edits() {
    for backend in StorageBackend::ALL {
        run_model(backend);
    }
}

fn run_model(backend: StorageBackend) {
    let mut seed = 0x5955_4d41_524b_444f_u64;
    let mut model = String::from("# Yu\n\nparagraph\n\n```rust\nfn main() {}\n```\n\nafter\n");
    let mut buffer = TextBuffer::with_backend(model.clone(), backend);
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
        assert_documents_equal(incremental.document(), &full, backend, step);
        assert_eq!(applied.result_snapshot().as_str(), model);
        document = incremental.into_document();
    }
}

fn assert_documents_equal(
    incremental: &MarkdownDocument,
    full: &MarkdownDocument,
    backend: StorageBackend,
    step: usize,
) {
    assert_eq!(
        incremental, full,
        "backend {backend} diverged at step {step}"
    );
    assert!(
        incremental.has_lossless_coverage(),
        "backend {backend} lost coverage at step {step}"
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
    for backend in StorageBackend::ALL {
        let source = "# one\n\nalpha\n\n# two\n\nomega\n";
        let mut buffer = TextBuffer::with_backend(source, backend);
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
        let materialized_before = retained_snapshot_stats(&[
            previous_snapshot.clone(),
            applied.result_snapshot().clone(),
        ])
        .materialized_buffers();

        let incremental =
            parse_incremental(&previous, applied.result_snapshot(), applied.change_set())
                .expect("matching revisions should parse incrementally");
        let full = parse(applied.result_snapshot());
        let shared = incremental
            .document()
            .blocks()
            .shared_blocks_with(previous.blocks());

        assert_eq!(incremental.document(), &full, "backend {backend}");
        assert_eq!(
            retained_snapshot_stats(&[previous_snapshot, applied.result_snapshot().clone()])
                .materialized_buffers(),
            materialized_before,
            "backend {backend} materialized source during convergence"
        );
        assert_eq!(incremental.reused_prefix_blocks(), 1, "backend {backend}");
        assert_eq!(incremental.reused_suffix_blocks(), 4, "backend {backend}");
        assert_eq!(shared, 5, "backend {backend}");
        assert!(
            incremental.reparsed_range().end() < applied.result_snapshot().len_bytes(),
            "backend {backend} scanned through EOF"
        );
        assert_eq!(
            incremental.document().block_storage_stats().segments(),
            3,
            "backend {backend}"
        );
        assert_eq!(
            incremental.document().block_storage_stats().allocations(),
            2,
            "backend {backend}"
        );
    }
}

#[test]
fn inserted_fence_prevents_false_hash_convergence() {
    for backend in StorageBackend::ALL {
        let source = "before\n\ninside\n\n# repeated\n\ninside\n";
        let mut buffer = TextBuffer::with_backend(source, backend);
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
        let incremental =
            parse_incremental(&previous, applied.result_snapshot(), applied.change_set())
                .expect("matching revisions should parse incrementally");
        let full = parse(applied.result_snapshot());

        assert_eq!(incremental.document(), &full, "backend {backend}");
        assert_eq!(incremental.reused_suffix_blocks(), 0, "backend {backend}");
        assert_eq!(
            incremental.reparsed_range().end(),
            applied.result_snapshot().len_bytes(),
            "backend {backend}"
        );
    }
}
