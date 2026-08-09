use yu_core::{ByteOffset, TextRange, Utf16Offset};
use yu_text::{Edit, StorageBackend, TextBuffer, TextSummary, Transaction};

const INSERTIONS: [&str; 7] = ["羽", "Yu", "🙂", "e\u{301}", "\n", "**", ""];

#[test]
fn deterministic_random_edits_match_string_model_and_inverse() {
    for backend in StorageBackend::ALL {
        run_model(backend);
    }
}

fn run_model(backend: StorageBackend) {
    let mut seed = 0x5955_4544_4954_4f52_u64;
    let mut model = String::from("# 羽\n\nHello, 世界🙂\n");
    let mut buffer = TextBuffer::with_backend(model.clone(), backend);

    for step in 0..2_000 {
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

        // Keep the model bounded while still exercising insert, delete and replace.
        if model.len() > 4_096 && start == end {
            end = *boundaries
                .last()
                .expect("a string always has a final boundary");
        }

        let inserted = INSERTIONS[random_index(&mut seed, INSERTIONS.len())];
        let range = TextRange::new(
            ByteOffset::try_from(start).expect("test offset should fit u64"),
            ByteOffset::try_from(end).expect("test offset should fit u64"),
        )
        .expect("ordered boundaries should form a valid range");
        let transaction = Transaction::new(buffer.revision(), [Edit::new(range, inserted)]);

        let before = model.clone();
        model.replace_range(start..end, inserted);
        let applied = buffer
            .apply(&transaction)
            .expect("model-generated transaction should apply");
        assert_snapshot_matches_model(&buffer, &model, backend, step);

        if step % 5 == 0 {
            buffer
                .apply(applied.inverse())
                .expect("generated inverse should apply");
            model = before;
            assert_snapshot_matches_model(&buffer, &model, backend, step);
        }
    }
}

fn assert_snapshot_matches_model(
    buffer: &TextBuffer,
    model: &str,
    backend: StorageBackend,
    step: usize,
) {
    let snapshot = buffer.snapshot();
    assert_eq!(
        snapshot.as_str(),
        model,
        "backend {backend} content failed at step {step}"
    );
    assert_eq!(
        snapshot.summary(),
        TextSummary::from_text(model),
        "backend {backend} summary failed at step {step}"
    );

    if step.is_multiple_of(31) {
        let probe = model
            .char_indices()
            .nth(model.chars().count() / 2)
            .map_or(model.len(), |(byte, _)| byte);
        let expected_utf16 = model[..probe].encode_utf16().count() as u64;
        let byte = ByteOffset::try_from(probe).expect("test offset should fit u64");
        let utf16 = Utf16Offset::new(expected_utf16);
        assert_eq!(snapshot.utf16_offset(byte), Ok(utf16));
        assert_eq!(snapshot.byte_offset_for_utf16(utf16), Ok(byte));
    }
}

fn char_boundaries(text: &str) -> Vec<usize> {
    text.char_indices()
        .map(|(index, _)| index)
        .chain(std::iter::once(text.len()))
        .collect()
}

fn random_index(seed: &mut u64, upper_bound: usize) -> usize {
    *seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
    let value = (*seed >> 32) as usize;
    value % upper_bound
}
