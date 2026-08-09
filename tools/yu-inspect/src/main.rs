#![forbid(unsafe_code)]

use std::env;
use std::error::Error;
use std::fs;
use std::io;
use std::path::PathBuf;

use yu_markdown::parse;
use yu_text::TextBuffer;

fn main() -> Result<(), Box<dyn Error>> {
    let path = input_path()?;
    let source = fs::read_to_string(&path)?;
    let buffer = TextBuffer::new(source);
    let snapshot = buffer.snapshot();
    let document = parse(&snapshot);

    println!("file: {}", path.display());
    println!("revision: {}", document.revision().get());
    println!("bytes: {}", document.source_len().get());
    println!("blocks: {}", document.blocks().len());
    println!("lossless coverage: {}", document.has_lossless_coverage());

    for (index, block) in document.blocks().iter().enumerate() {
        println!(
            "{index:>4}  {:>8}..{:<8}  {:?}",
            block.range().start().get(),
            block.range().end().get(),
            block.kind()
        );
    }

    if !document.has_lossless_coverage() {
        return Err(io::Error::other("syntax ranges do not cover the source").into());
    }
    Ok(())
}

fn input_path() -> Result<PathBuf, Box<dyn Error>> {
    let mut arguments = env::args_os();
    let executable = arguments.next().unwrap_or_default();
    let Some(path) = arguments.next() else {
        let executable = PathBuf::from(executable);
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("usage: {} <markdown-file>", executable.display()),
        )
        .into());
    };
    if arguments.next().is_some() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "yu-inspect accepts exactly one input file",
        )
        .into());
    }
    Ok(PathBuf::from(path))
}
