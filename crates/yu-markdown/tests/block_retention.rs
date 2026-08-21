use yu_core::{ByteOffset, TextRange};
use yu_markdown::{BlockCompactionPolicy, parse, parse_incremental, retained_markdown_stats};
use yu_text::{Edit, TextBuffer, Transaction};

#[test]
fn retained_stats_deduplicate_shared_block_allocations() {
    let mut buffer = TextBuffer::new("# Yu\n\nbody\n");
    let first = parse(&buffer.snapshot());
    let transaction = Transaction::new(buffer.revision(), std::iter::empty::<Edit>());
    let applied = buffer.apply(&transaction).expect("empty edit should apply");
    let second = parse_incremental(&first, applied.result_snapshot(), applied.change_set())
        .expect("matching revisions should parse incrementally")
        .into_document();

    let stats = retained_markdown_stats(&[first.clone(), second]);
    let blocks = stats.blocks();

    assert_eq!(stats.documents(), 2);
    assert_eq!(blocks.sequences(), 2);
    assert_eq!(blocks.block_references(), first.blocks().len() * 2);
    assert_eq!(blocks.segment_tables(), 1);
    assert_eq!(blocks.block_allocations(), 1);
    assert_eq!(blocks.block_records(), first.blocks().len());
}

#[test]
fn explicit_compaction_packs_a_segmented_document() {
    let source = "# one\n\nalpha\n\n# two\n\nomega\n";
    let mut buffer = TextBuffer::new(source);
    let previous = parse(&buffer.snapshot());
    let insert_at = source.find("alpha").expect("fixture contains alpha") + 2;
    let transaction = Transaction::new(
        buffer.revision(),
        [Edit::new(
            TextRange::empty(ByteOffset::try_from(insert_at).expect("offset fits u64")),
            "Yu",
        )],
    );
    let applied = buffer.apply(&transaction).expect("edit should apply");
    let mut document =
        parse_incremental(&previous, applied.result_snapshot(), applied.change_set())
            .expect("matching revisions should parse incrementally")
            .into_document();
    let semantic_copy = document.clone();
    let policy = BlockCompactionPolicy::new(2, 8, usize::MAX).expect("valid policy");

    assert_eq!(document.block_storage_stats().segments(), 3);
    assert!(document.needs_block_compaction(policy));
    assert!(document.compact_blocks_if_needed(policy));
    assert_eq!(document, semantic_copy);
    assert_eq!(document.block_storage_stats().segments(), 1);
    assert_eq!(document.block_storage_stats().allocations(), 1);
    assert_eq!(
        document.block_storage_stats().retained_records(),
        document.blocks().len()
    );
    assert_eq!(document.blocks().shared_blocks_with(previous.blocks()), 0);
    assert!(!document.compact_blocks());
}

#[test]
fn retention_amplification_recommends_compaction_after_large_delete() {
    let source = "\n".repeat(10_000);
    let mut buffer = TextBuffer::new(source);
    let previous = parse(&buffer.snapshot());
    let transaction = Transaction::new(
        buffer.revision(),
        [Edit::new(
            TextRange::new(ByteOffset::ZERO, ByteOffset::new(9_000))
                .expect("fixture deletion is ordered"),
            "",
        )],
    );
    let applied = buffer.apply(&transaction).expect("delete should apply");
    let mut document =
        parse_incremental(&previous, applied.result_snapshot(), applied.change_set())
            .expect("matching revisions should parse incrementally")
            .into_document();
    let before = document.block_storage_stats();

    assert_eq!(before.blocks(), 1_000);
    assert_eq!(before.retained_records(), 10_000);
    assert_eq!(before.reclaimable_records(), 9_000);
    assert!(document.needs_block_compaction(BlockCompactionPolicy::default()));
    assert!(document.compact_blocks_if_needed(BlockCompactionPolicy::default()));
    assert_eq!(document.block_storage_stats().retained_records(), 1_000);
}

#[test]
fn idle_compaction_bounds_segments_through_a_long_edit_session() {
    let mut model = "# heading\n\nparagraph\n\n".repeat(256);
    let mut buffer = TextBuffer::new(model.clone());
    let mut document = parse(&buffer.snapshot());
    let policy = BlockCompactionPolicy::new(16, 8, 128).expect("valid test policy");
    let mut compactions = 0;

    for step in 0..500 {
        let offset = (step * 7_919) % (model.len() + 1);
        let inserted = match step % 3 {
            0 => "x",
            1 => "\n",
            _ => "# ",
        };
        let range = TextRange::empty(ByteOffset::try_from(offset).expect("offset fits u64"));
        let transaction = Transaction::new(buffer.revision(), [Edit::new(range, inserted)]);
        let applied = buffer
            .apply(&transaction)
            .expect("session edit should apply");
        model.insert_str(offset, inserted);

        document = parse_incremental(&document, applied.result_snapshot(), applied.change_set())
            .expect("matching revisions should parse incrementally")
            .into_document();
        if document.compact_blocks_if_needed(policy) {
            compactions += 1;
        }
        assert!(
            document.block_storage_stats().segments() <= policy.max_segments(),
            "exceeded policy at step {step}"
        );

        if step.is_multiple_of(25) {
            assert_eq!(document, parse(applied.result_snapshot()));
        }
    }

    assert!(compactions > 0, "never compacted");
    assert_eq!(buffer.snapshot().as_str(), model);
    assert_eq!(document, parse(&buffer.snapshot()));
    assert!(document.has_lossless_coverage());
}
