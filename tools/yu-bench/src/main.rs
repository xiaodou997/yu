#![forbid(unsafe_code)]

use std::env;
use std::error::Error;
use std::io;
use std::time::{Duration, Instant};

use yu_core::{ByteOffset, TextRange};
use yu_markdown::parse;
use yu_text::{Edit, TextBuffer, Transaction};

const SECTION: &str = "# Yu\n\nA paragraph with **strong text**, 中文 and emoji 🙂.\n\n```rust\nfn main() {}\n```\n\n";

fn main() -> Result<(), Box<dyn Error>> {
    let configuration = Configuration::from_arguments()?;
    let source = fixture(configuration.size_mib);
    let mut buffer = TextBuffer::new(source);
    let snapshot = buffer.snapshot();

    let warmup = parse(&snapshot);
    if !warmup.has_lossless_coverage() {
        return Err(io::Error::other("warmup parse lost source coverage").into());
    }

    let mut parse_samples = Vec::with_capacity(configuration.iterations);
    for _ in 0..configuration.iterations {
        let start = Instant::now();
        let document = parse(&snapshot);
        parse_samples.push(start.elapsed());
        std::hint::black_box(document);
    }

    let middle = nearest_char_boundary(snapshot.as_str(), snapshot.as_str().len() / 2);
    let insert_range = TextRange::empty(
        ByteOffset::try_from(middle).map_err(|_| io::Error::other("fixture is too large"))?,
    );
    let mut edit_samples = Vec::with_capacity(configuration.iterations);
    for _ in 0..configuration.iterations {
        let transaction = Transaction::new(buffer.revision(), [Edit::new(insert_range, "羽")]);
        let start = Instant::now();
        let applied = buffer.apply(&transaction)?;
        buffer.apply(applied.inverse())?;
        edit_samples.push(start.elapsed());
    }

    println!("Yu Phase 1 reference workload");
    println!("backend: flat Arc<str> reference (not production)");
    println!("document bytes: {}", snapshot.as_str().len());
    println!("blocks: {}", warmup.blocks().len());
    println!("iterations: {}", configuration.iterations);
    println!("full block scan median: {:?}", median(&mut parse_samples));
    println!(
        "middle insert + inverse median: {:?}",
        median(&mut edit_samples)
    );

    Ok(())
}

#[derive(Clone, Copy, Debug)]
struct Configuration {
    size_mib: usize,
    iterations: usize,
}

impl Configuration {
    fn from_arguments() -> Result<Self, Box<dyn Error>> {
        let mut configuration = Self {
            size_mib: 1,
            iterations: 20,
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

fn median(samples: &mut [Duration]) -> Duration {
    samples.sort_unstable();
    samples[samples.len() / 2]
}
