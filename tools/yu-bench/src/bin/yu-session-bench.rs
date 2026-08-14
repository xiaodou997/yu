#![forbid(unsafe_code)]

//! Headless Phase 2 vertical slice benchmark.
//!
//! The workload intentionally goes through the same `DocumentEditorSession`
//! boundary used by the native host: open a UTF-8 Markdown file, select a
//! source range, execute an insert command (which updates the editor and
//! incremental Markdown state), save atomically, and reload the clean file.

use std::env;
use std::error::Error;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use yu_core::ByteOffset;
use yu_editor::{CaretAffinity, EditorCommand, EditorSelection};
use yu_storage::{DocumentEditorSession, SaveOutcome};

const SECTION: &str = "# Yu\n\nA paragraph with **strong text**, 中文 and emoji 🙂.\n\n```rust\nfn main() {}\n```\n\n";
const INSERTIONS: [&str; 6] = ["羽", "Yu", "🙂", "e\u{301}", "\n", "**"];

fn main() -> Result<(), Box<dyn Error>> {
    let configuration = Configuration::from_arguments()?;
    let source = fixture(configuration.size_mib);
    let (script, expected_source) = random_edit_script(&source, configuration.random_edits);
    let document = TemporaryDocument::create(&source)?;

    println!("Yu DocumentSession vertical slice");
    println!("fixture: {}", document.path.display());
    println!("document bytes: {}", source.len());
    println!("open iterations: {}", configuration.iterations);
    println!("random edits: {}", script.len());

    let mut open_samples = Vec::with_capacity(configuration.iterations);
    for _ in 0..configuration.iterations {
        let start = Instant::now();
        let session = DocumentEditorSession::open(&document.path)?;
        open_samples.push(start.elapsed());
        if session.snapshot().as_str() != source {
            return Err(io::Error::other("open snapshot differs from fixture").into());
        }
        std::hint::black_box(session.revision());
    }

    let mut session = DocumentEditorSession::open(&document.path)?;
    if session.is_dirty() {
        return Err(io::Error::other("fresh session unexpectedly dirty").into());
    }

    let mut edit_samples = Vec::with_capacity(script.len());
    let edit_start = Instant::now();
    for edit in &script {
        let snapshot = session.snapshot();
        let anchor = ByteOffset::try_from(edit.start)
            .map_err(|_| io::Error::other("edit start exceeds ByteOffset"))?;
        let focus = ByteOffset::try_from(edit.end)
            .map_err(|_| io::Error::other("edit end exceeds ByteOffset"))?;
        let selection =
            EditorSelection::range(&snapshot, anchor, focus, CaretAffinity::Downstream)?;

        let command_start = Instant::now();
        session.set_selection(selection)?;
        let result = session.execute(EditorCommand::insert_text(edit.inserted))?;
        edit_samples.push(command_start.elapsed());
        if !result.changed() {
            return Err(io::Error::other("insert command unexpectedly made no change").into());
        }
    }
    let edit_total = edit_start.elapsed();

    let actual_snapshot = session.snapshot();
    if actual_snapshot.as_str() != expected_source {
        return Err(io::Error::other("session edit result differs from model").into());
    }
    if !session.is_dirty() {
        return Err(io::Error::other("edited session unexpectedly clean").into());
    }

    let save_start = Instant::now();
    let save = session.save()?;
    let save_time = save_start.elapsed();
    let saved_bytes = match save {
        SaveOutcome::Saved { bytes_written, .. } => bytes_written,
        SaveOutcome::Unchanged { .. } => {
            return Err(io::Error::other("edited session reported unchanged save").into());
        }
    };
    if session.is_dirty() {
        return Err(io::Error::other("session stayed dirty after save").into());
    }

    let reload_start = Instant::now();
    session.reload()?;
    let reload_time = reload_start.elapsed();
    if session.snapshot().as_str() != expected_source {
        return Err(io::Error::other("reload snapshot differs from saved source").into());
    }

    println!();
    println!("open median: {:?}", median(&mut open_samples));
    println!("edit total: {:?}", edit_total);
    println!(
        "edit mean: {:?}",
        edit_total / u32::try_from(script.len()).unwrap_or(u32::MAX)
    );
    println!("edit command median: {:?}", median(&mut edit_samples));
    println!("save: {:?} ({saved_bytes} bytes)", save_time);
    println!("reload: {:?}", reload_time);
    println!("final revision: {:?}", session.revision());
    println!("final bytes: {}", expected_source.len());
    println!("result: ok");

    Ok(())
}

struct TemporaryDocument {
    path: PathBuf,
}

impl TemporaryDocument {
    fn create(source: &str) -> Result<Self, io::Error> {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = env::temp_dir().join(format!(
            "yu-session-bench-{}-{stamp}.md",
            std::process::id()
        ));
        fs::write(&path, source)?;
        Ok(Self { path })
    }
}

impl Drop for TemporaryDocument {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[derive(Clone, Copy, Debug)]
struct Configuration {
    size_mib: usize,
    iterations: usize,
    random_edits: usize,
}

impl Configuration {
    fn from_arguments() -> Result<Self, Box<dyn Error>> {
        let mut configuration = Self {
            size_mib: 1,
            iterations: 8,
            // Random Markdown edits can intentionally trigger fence/state
            // propagation to EOF. Keep the default bounded; callers can
            // increase this when running a stress workload explicitly.
            random_edits: 4,
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
                "--iterations" => configuration.iterations = positive_number(&value, &argument)?,
                "--random-edits" => {
                    configuration.random_edits = positive_number(&value, &argument)?;
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
    start: usize,
    end: usize,
    inserted: &'static str,
}

fn random_edit_script(source: &str, count: usize) -> (Vec<ScriptEdit>, String) {
    let mut seed = 0x5955_5345_5353_494f_u64;
    let mut model = source.to_owned();
    let mut script = Vec::with_capacity(count);
    for _ in 0..count {
        let mut start = random_index(&mut seed, model.len() + 1);
        start = nearest_char_boundary(&model, start);
        let requested_end = start + random_index(&mut seed, 65);
        let mut end = requested_end.min(model.len());
        end = nearest_char_boundary(&model, end);
        let inserted = INSERTIONS[random_index(&mut seed, INSERTIONS.len())];
        script.push(ScriptEdit {
            start,
            end,
            inserted,
        });
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
    SECTION.repeat(requested_bytes.div_ceil(SECTION.len()))
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
