#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use std::io::{self, BufRead, Write};
use std::sync::mpsc;
use std::time::Duration;
use yu_document_renderer::{Kind, MAX_SOURCE_BYTES, RenderStyle, VectorOutput, render_styled};

// JSON escaping can expand a source byte to six transport bytes.
const MAX_MESSAGE_BYTES: usize = MAX_SOURCE_BYTES * 6 + 4096;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    id: u64,
    document: u64,
    revision: u64,
    kind: Kind,
    source: String,
    #[serde(default)]
    style: RenderStyle,
}
#[derive(Serialize)]
struct Response {
    id: u64,
    document: u64,
    revision: u64,
    #[serde(flatten)]
    result: Outcome,
}
#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum Outcome {
    Ready { vector: VectorOutput },
    Failed { diagnostic: String },
}

fn read_message(reader: &mut impl BufRead) -> io::Result<Option<Vec<u8>>> {
    let mut bytes = Vec::new();
    loop {
        let part = reader.fill_buf()?;
        if part.is_empty() {
            return if bytes.is_empty() {
                Ok(None)
            } else {
                Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "Incomplete render request",
                ))
            };
        }
        let end = part.iter().position(|b| *b == b'\n').map(|n| n + 1);
        let count = end.unwrap_or(part.len());
        if bytes.len() + count > MAX_MESSAGE_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Render request exceeds transport limit",
            ));
        }
        bytes.extend_from_slice(&part[..count]);
        reader.consume(count);
        if end.is_some() {
            return Ok(Some(bytes));
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // The host cancels running jobs by terminating this disposable helper.
    // No file writes or shared mutable document state exist in this process.
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("yu-render-input".into())
        .spawn(move || {
            let mut input = io::stdin().lock();
            loop {
                match read_message(&mut input) {
                    Ok(Some(bytes)) => {
                        if sender.send(Ok(bytes)).is_err() {
                            break;
                        }
                    }
                    Ok(None) => break,
                    Err(error) => {
                        let _ = sender.send(Err(error));
                        break;
                    }
                }
            }
        })?;
    let mut output = io::stdout().lock();
    while let Ok(message) = receiver.recv_timeout(Duration::from_secs(60)) {
        let request: Request = serde_json::from_slice(&message?)?;
        let result = match render_styled(request.kind, &request.source, request.style) {
            Ok(vector) => Outcome::Ready { vector },
            Err(diagnostic) => Outcome::Failed { diagnostic },
        };
        serde_json::to_writer(
            &mut output,
            &Response {
                id: request.id,
                document: request.document,
                revision: request.revision,
                result,
            },
        )?;
        output.write_all(b"\n")?;
        output.flush()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn framed_requests_are_bounded_and_not_combined() {
        let mut input = io::Cursor::new(b"first\nsecond\n");
        assert_eq!(
            read_message(&mut input).expect("read").expect("line"),
            b"first\n"
        );
        assert_eq!(
            read_message(&mut input).expect("read").expect("line"),
            b"second\n"
        );
        assert!(read_message(&mut input).expect("eof").is_none());
        assert!(read_message(&mut io::Cursor::new(vec![b'x'; MAX_MESSAGE_BYTES + 1])).is_err());
        assert!(read_message(&mut io::Cursor::new(b"partial")).is_err());
    }
}
