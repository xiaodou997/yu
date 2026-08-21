#![forbid(unsafe_code)]

use std::env;
use std::error::Error;
use std::io;
use std::time::{Duration, Instant};

use yu_core::{ByteOffset, TextRange};
use yu_markdown::{
    BlockCompactionPolicy, MarkdownRetentionStats, parse, parse_incremental,
    retained_markdown_stats,
};
use yu_text::{Edit, StorageBackend, TextBuffer, Transaction, retained_snapshot_stats};

const SECTION: &str = "# Yu\n\nA paragraph with **strong text**, 中文 and emoji 🙂.\n\n```rust\nfn main() {}\n```\n\n";
const INSERTIONS: [&str; 6] = ["羽", "Yu", "🙂", "e\u{301}", "\n", "**"];

fn main() -> Result<(), Box<dyn Error>> {
    let configuration = Configuration::from_arguments()?;
    let source = fixture(configuration.size_mib);
    let (random_script, expected_random_result) =
        random_edit_script(&source, configuration.random_edits);

    println!("Yu Phase 1 storage comparison");
    println!("document bytes: {}", source.len());
    println!("timing iterations: {}", configuration.iterations);
    println!("random edits: {}", configuration.random_edits);
    println!("retained snapshots: {}", configuration.retained_snapshots);

    for backend in StorageBackend::ALL {
        run_backend(
            backend,
            &source,
            &random_script,
            &expected_random_result,
            configuration,
        )?;
    }

    Ok(())
}

fn run_backend(
    backend: StorageBackend,
    source: &str,
    random_script: &[ScriptEdit],
    expected_random_result: &str,
    configuration: Configuration,
) -> Result<(), Box<dyn Error>> {
    let construct_start = Instant::now();
    let mut buffer = TextBuffer::with_backend(source, backend);
    let construct_time = construct_start.elapsed();
    let initial_stats = buffer.storage_stats();
    let middle = nearest_char_boundary(source, source.len() / 2);
    let middle_offset =
        ByteOffset::try_from(middle).map_err(|_| io::Error::other("fixture is too large"))?;
    let insert_range = TextRange::empty(middle_offset);

    let mut snapshot_samples = Vec::with_capacity(configuration.iterations);
    for _ in 0..configuration.iterations {
        let start = Instant::now();
        let snapshot = buffer.snapshot();
        snapshot_samples.push(start.elapsed());
        std::hint::black_box(snapshot.storage_stats());
    }

    let structured_snapshot = buffer.snapshot();
    let mut chunk_scan_samples = Vec::with_capacity(configuration.iterations);
    for _ in 0..configuration.iterations {
        let start = Instant::now();
        let bytes = structured_snapshot
            .chunks()
            .map(|chunk| chunk.text().len())
            .sum::<usize>();
        chunk_scan_samples.push(start.elapsed());
        if bytes != source.len() {
            return Err(io::Error::other("chunk cursor lost source bytes").into());
        }
    }

    let mut coordinate_samples = Vec::with_capacity(configuration.iterations);
    for _ in 0..configuration.iterations {
        let start = Instant::now();
        let utf16 = structured_snapshot.utf16_offset(middle_offset)?;
        let line = structured_snapshot.line_index(middle_offset)?;
        std::hint::black_box(structured_snapshot.byte_offset_for_utf16(utf16)?);
        std::hint::black_box(structured_snapshot.line_start(line)?);
        coordinate_samples.push(start.elapsed());
    }

    let parse_snapshot = buffer.snapshot();
    let materialized_before =
        retained_snapshot_stats(std::slice::from_ref(&parse_snapshot)).materialized_buffers();
    let warmup = parse(&parse_snapshot);
    if !warmup.has_lossless_coverage() {
        return Err(io::Error::other("warmup parse lost source coverage").into());
    }
    let mut parse_samples = Vec::with_capacity(configuration.iterations);
    for _ in 0..configuration.iterations {
        let start = Instant::now();
        let document = parse(&parse_snapshot);
        parse_samples.push(start.elapsed());
        std::hint::black_box(document);
    }
    let materialized_after =
        retained_snapshot_stats(std::slice::from_ref(&parse_snapshot)).materialized_buffers();
    if materialized_after != materialized_before {
        return Err(io::Error::other("block parser materialized a contiguous source copy").into());
    }

    let cold_snapshot = buffer.snapshot();
    let materialize_start = Instant::now();
    std::hint::black_box(cold_snapshot.as_str());
    let materialize_time = materialize_start.elapsed();

    let incremental_measurements = [
        benchmark_incremental(
            backend,
            source,
            "near-start",
            nearest_char_boundary(source, source.len() / 100),
            configuration.iterations,
        )?,
        benchmark_incremental(backend, source, "middle", middle, configuration.iterations)?,
        benchmark_incremental(
            backend,
            source,
            "near-end",
            nearest_char_boundary(source, source.len() * 99 / 100),
            configuration.iterations,
        )?,
    ];
    let session = benchmark_incremental_session(
        backend,
        source,
        random_script,
        expected_random_result,
        configuration.retained_snapshots,
    )?;

    let mut edit_samples = Vec::with_capacity(configuration.iterations);
    for _ in 0..configuration.iterations {
        let transaction = Transaction::new(buffer.revision(), [Edit::new(insert_range, "羽")]);
        let start = Instant::now();
        let applied = buffer.apply(&transaction)?;
        buffer.apply(applied.inverse())?;
        edit_samples.push(start.elapsed());
    }

    let mut random_buffer = TextBuffer::with_backend(source, backend);
    let mut retained_snapshots = Vec::with_capacity(configuration.retained_snapshots);
    retained_snapshots.push(random_buffer.snapshot());
    let retention_stride = configuration
        .random_edits
        .div_ceil(configuration.retained_snapshots.saturating_sub(1).max(1));
    let random_start = Instant::now();
    for (index, edit) in random_script.iter().enumerate() {
        let transaction = Transaction::new(
            random_buffer.revision(),
            [Edit::new(edit.range, edit.inserted)],
        );
        let applied = random_buffer.apply(&transaction)?;
        let completed = index + 1;
        if retained_snapshots.len() < configuration.retained_snapshots
            && (completed.is_multiple_of(retention_stride) || completed == random_script.len())
        {
            retained_snapshots.push(applied.result_snapshot().clone());
        }
    }
    let random_time = random_start.elapsed();
    let random_snapshot = random_buffer.snapshot();
    if random_snapshot.as_str() != expected_random_result {
        return Err(io::Error::other(format!("{backend} random edit result mismatch")).into());
    }
    let mut fragmented_chunk_samples = Vec::with_capacity(configuration.iterations);
    for _ in 0..configuration.iterations {
        let start = Instant::now();
        let bytes = random_snapshot
            .chunks()
            .map(|chunk| chunk.text().len())
            .sum::<usize>();
        fragmented_chunk_samples.push(start.elapsed());
        if bytes != expected_random_result.len() {
            return Err(io::Error::other("fragmented chunk cursor lost source bytes").into());
        }
    }

    let round_trip_stats = buffer.storage_stats();
    let random_stats = random_buffer.storage_stats();
    let retention = retained_snapshot_stats(&retained_snapshots);
    println!();
    println!("backend: {backend}");
    println!("  construct: {construct_time:?}");
    println!("  snapshot median: {:?}", median(&mut snapshot_samples));
    println!(
        "  chunk cursor scan median: {:?}",
        median(&mut chunk_scan_samples)
    );
    println!(
        "  coordinate round-trip median: {:?}",
        median(&mut coordinate_samples)
    );
    println!("  first contiguous view: {materialize_time:?}");
    println!("  full block scan median: {:?}", median(&mut parse_samples));
    for measurement in incremental_measurements {
        println!(
            "  incremental {} median: {:?} (reparsed-bytes={} reused-prefix={} reused-suffix={} shared-blocks={} segments={})",
            measurement.label,
            measurement.median,
            measurement.reparsed_bytes,
            measurement.reused_prefix_blocks,
            measurement.reused_suffix_blocks,
            measurement.shared_blocks,
            measurement.segments,
        );
    }
    println!(
        "  incremental session parse: {:?} total / {:?} mean (reparsed-bytes={} max-segments={} final-segments={})",
        session.parse_time,
        session.parse_time / u32::try_from(random_script.len()).unwrap_or(u32::MAX),
        session.reparsed_bytes,
        session.max_segments,
        session.final_segments,
    );
    println!(
        "  idle block compaction: {:?} total / {:?} max (runs={} rewritten-blocks={} segment-threshold={})",
        session.compaction_time,
        session.max_compaction_time,
        session.compactions,
        session.rewritten_blocks,
        BlockCompactionPolicy::default().max_segments(),
    );
    let retained_blocks = session.retention.blocks();
    println!(
        "  retained markdown estimate: {} (documents={} block-allocations={} block-records={} block-bytes={} segment-tables={} segments={} segment-bytes={})",
        human_bytes(session.retention.estimated_bytes()),
        session.retention.documents(),
        retained_blocks.block_allocations(),
        retained_blocks.block_records(),
        retained_blocks.block_record_bytes(),
        retained_blocks.segment_tables(),
        retained_blocks.segments(),
        retained_blocks.segment_bytes(),
    );
    println!(
        "  middle insert + inverse median: {:?}",
        median(&mut edit_samples)
    );
    println!("  random edit total: {random_time:?}");
    println!(
        "  random edit mean: {:?}",
        random_time / u32::try_from(random_script.len()).unwrap_or(u32::MAX)
    );
    println!(
        "  fragmented chunk scan median: {:?}",
        median(&mut fragmented_chunk_samples)
    );
    println!("  initial structure: chunks={}", initial_stats.chunks());
    println!(
        "  after repeated insert/inverse: chunks={}",
        round_trip_stats.chunks()
    );
    println!("  after random edits: chunks={}", random_stats.chunks());
    println!(
        "  retained allocation estimate: {} (snapshots={} snapshot-bytes={} nodes={} node-bytes={} auxiliary={} auxiliary-bytes={} text-buffers={} text-bytes={} materialized-buffers={} materialized-bytes={})",
        human_bytes(retention.estimated_bytes()),
        retention.snapshots(),
        retention.snapshot_bytes(),
        retention.nodes(),
        retention.node_bytes(),
        retention.auxiliary_allocations(),
        retention.auxiliary_bytes(),
        retention.text_buffers(),
        retention.text_bytes(),
        retention.materialized_buffers(),
        retention.materialized_bytes(),
    );

    Ok(())
}

#[derive(Clone, Copy, Debug)]
struct IncrementalMeasurement {
    label: &'static str,
    median: Duration,
    reparsed_bytes: u64,
    reused_prefix_blocks: usize,
    reused_suffix_blocks: usize,
    shared_blocks: usize,
    segments: usize,
}

fn benchmark_incremental(
    backend: StorageBackend,
    source: &str,
    label: &'static str,
    offset: usize,
    iterations: usize,
) -> Result<IncrementalMeasurement, Box<dyn Error>> {
    let mut buffer = TextBuffer::with_backend(source, backend);
    let previous = parse(&buffer.snapshot());
    let range = TextRange::empty(
        ByteOffset::try_from(offset).map_err(|_| io::Error::other("fixture is too large"))?,
    );
    let transaction = Transaction::new(buffer.revision(), [Edit::new(range, "\n# incremental\n")]);
    let applied = buffer.apply(&transaction)?;
    let expected = parse(applied.result_snapshot());
    let mut samples = Vec::with_capacity(iterations);
    let mut measurement = IncrementalMeasurement {
        label,
        median: Duration::ZERO,
        reparsed_bytes: 0,
        reused_prefix_blocks: 0,
        reused_suffix_blocks: 0,
        shared_blocks: 0,
        segments: 0,
    };

    for _ in 0..iterations {
        let start = Instant::now();
        let result = parse_incremental(&previous, applied.result_snapshot(), applied.change_set())?;
        samples.push(start.elapsed());
        if result.document() != &expected {
            return Err(io::Error::other("incremental parse diverged from full parse").into());
        }
        measurement.reparsed_bytes = result.reparsed_range().len();
        measurement.reused_prefix_blocks = result.reused_prefix_blocks();
        measurement.reused_suffix_blocks = result.reused_suffix_blocks();
        measurement.shared_blocks = result
            .document()
            .blocks()
            .shared_blocks_with(previous.blocks());
        measurement.segments = result.document().block_storage_stats().segments();
        std::hint::black_box(result);
    }
    measurement.median = median(&mut samples);
    Ok(measurement)
}

#[derive(Clone, Copy, Debug)]
struct IncrementalSessionMeasurement {
    parse_time: Duration,
    compaction_time: Duration,
    max_compaction_time: Duration,
    compactions: usize,
    rewritten_blocks: usize,
    reparsed_bytes: u64,
    max_segments: usize,
    final_segments: usize,
    retention: MarkdownRetentionStats,
}

fn benchmark_incremental_session(
    backend: StorageBackend,
    source: &str,
    script: &[ScriptEdit],
    expected_result: &str,
    retained_documents: usize,
) -> Result<IncrementalSessionMeasurement, Box<dyn Error>> {
    let mut buffer = TextBuffer::with_backend(source, backend);
    let mut document = parse(&buffer.snapshot());
    let policy = BlockCompactionPolicy::default();
    let mut history = Vec::with_capacity(retained_documents);
    history.push(document.clone());
    let retention_stride = script
        .len()
        .div_ceil(retained_documents.saturating_sub(1).max(1));
    let mut parse_time = Duration::ZERO;
    let mut compaction_time = Duration::ZERO;
    let mut max_compaction_time = Duration::ZERO;
    let mut compactions = 0;
    let mut rewritten_blocks = 0_usize;
    let mut reparsed_bytes = 0_u64;
    let mut max_segments = document.block_storage_stats().segments();

    for (index, edit) in script.iter().enumerate() {
        let transaction =
            Transaction::new(buffer.revision(), [Edit::new(edit.range, edit.inserted)]);
        let applied = buffer.apply(&transaction)?;

        let parse_start = Instant::now();
        let incremental =
            parse_incremental(&document, applied.result_snapshot(), applied.change_set())?;
        parse_time += parse_start.elapsed();
        reparsed_bytes = reparsed_bytes.saturating_add(incremental.reparsed_range().len());
        document = incremental.into_document();
        let block_stats = document.block_storage_stats();
        max_segments = max_segments.max(block_stats.segments());

        if policy.should_compact(block_stats) {
            rewritten_blocks = rewritten_blocks.saturating_add(document.blocks().len());
            let compact_start = Instant::now();
            if document.compact_blocks_if_needed(policy) {
                let elapsed = compact_start.elapsed();
                compaction_time += elapsed;
                max_compaction_time = max_compaction_time.max(elapsed);
                compactions += 1;
            }
        }

        let completed = index + 1;
        if history.len() < retained_documents
            && (completed.is_multiple_of(retention_stride) || completed == script.len())
        {
            history.push(document.clone());
        }
    }

    if buffer.snapshot().as_str() != expected_result {
        return Err(
            io::Error::other(format!("{backend} incremental session text mismatch")).into(),
        );
    }
    if document != parse(&buffer.snapshot()) {
        return Err(
            io::Error::other(format!("{backend} incremental session parse mismatch")).into(),
        );
    }
    if retained_documents == 1 {
        history.clear();
        history.push(document.clone());
    }

    Ok(IncrementalSessionMeasurement {
        parse_time,
        compaction_time,
        max_compaction_time,
        compactions,
        rewritten_blocks,
        reparsed_bytes,
        max_segments,
        final_segments: document.block_storage_stats().segments(),
        retention: retained_markdown_stats(&history),
    })
}

#[derive(Clone, Copy, Debug)]
struct Configuration {
    size_mib: usize,
    iterations: usize,
    random_edits: usize,
    retained_snapshots: usize,
}

impl Configuration {
    fn from_arguments() -> Result<Self, Box<dyn Error>> {
        let mut configuration = Self {
            size_mib: 1,
            iterations: 20,
            random_edits: 2_000,
            retained_snapshots: 8,
        };
        let mut arguments = env::args().skip(1);

        while let Some(argument) = arguments.next() {
            let value = arguments.next().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("missing value after {argument}"),
                )
            })?;
            match argument.as_str() {
                "--size-mib" => configuration.size_mib = positive_number(&value, &argument)?,
                "--iterations" => {
                    configuration.iterations = positive_number(&value, &argument)?;
                }
                "--random-edits" => {
                    configuration.random_edits = positive_number(&value, &argument)?;
                }
                "--retained-snapshots" => {
                    configuration.retained_snapshots = positive_number(&value, &argument)?;
                }
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("unknown argument {argument}"),
                    )
                    .into());
                }
            }
        }

        Ok(configuration)
    }
}

#[derive(Clone, Copy, Debug)]
struct ScriptEdit {
    range: TextRange,
    inserted: &'static str,
}

fn random_edit_script(source: &str, count: usize) -> (Vec<ScriptEdit>, String) {
    let mut seed = 0x5955_4245_4e43_4821_u64;
    let mut model = source.to_owned();
    let mut script = Vec::with_capacity(count);

    for _ in 0..count {
        let mut start = random_index(&mut seed, model.len() + 1);
        start = nearest_char_boundary(&model, start);
        let requested_end = start + random_index(&mut seed, 65);
        let mut end = requested_end.min(model.len());
        end = nearest_char_boundary(&model, end);
        let inserted = INSERTIONS[random_index(&mut seed, INSERTIONS.len())];
        let range = TextRange::new(
            ByteOffset::try_from(start).expect("benchmark offset should fit u64"),
            ByteOffset::try_from(end).expect("benchmark offset should fit u64"),
        )
        .expect("benchmark boundaries should be ordered");
        script.push(ScriptEdit { range, inserted });
        model.replace_range(start..end, inserted);
    }

    (script, model)
}

fn positive_number(value: &str, argument: &str) -> Result<usize, Box<dyn Error>> {
    let parsed: usize = value.parse()?;
    if parsed == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{argument} must be greater than zero"),
        )
        .into());
    }
    Ok(parsed)
}

fn fixture(size_mib: usize) -> String {
    let requested_bytes = size_mib.saturating_mul(1024 * 1024);
    let repetitions = requested_bytes.div_ceil(SECTION.len());
    SECTION.repeat(repetitions)
}

fn nearest_char_boundary(text: &str, mut candidate: usize) -> usize {
    while !text.is_char_boundary(candidate) {
        candidate -= 1;
    }
    candidate
}

fn random_index(seed: &mut u64, upper_bound: usize) -> usize {
    *seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
    ((*seed >> 32) as usize) % upper_bound
}

fn median(samples: &mut [Duration]) -> Duration {
    samples.sort_unstable();
    samples[samples.len() / 2]
}

fn human_bytes(bytes: usize) -> String {
    const MEBIBYTE: f64 = (1024 * 1024) as f64;
    format!("{:.2} MiB", bytes as f64 / MEBIBYTE)
}
