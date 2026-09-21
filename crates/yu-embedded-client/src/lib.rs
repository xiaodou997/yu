#![forbid(unsafe_code)]
//! The app-side client intentionally has no Typst/Mermaid dependency.
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc,
};
use std::time::{Duration, Instant};
use yu_assets::{EmbeddedRenderPayload, EmbeddedRenderRequest};

const MAX_RESPONSE: usize = yu_assets::EMBEDDED_SVG_MAX_MARKUP_BYTES * 6 + 4096;

#[derive(Clone)]
pub struct RenderControl {
    revision: Arc<AtomicU64>,
    alive: Arc<AtomicBool>,
}
impl Default for RenderControl {
    fn default() -> Self {
        Self {
            revision: Arc::new(AtomicU64::new(0)),
            alive: Arc::new(AtomicBool::new(true)),
        }
    }
}
impl RenderControl {
    pub fn set_revision(&self, revision: u64) {
        self.revision.store(revision, Ordering::Release);
    }
    pub fn close(&self) {
        self.alive.store(false, Ordering::Release);
    }
    /// Whether a pending or running job still belongs to the live presentation.
    pub fn is_current(&self, revision: u64) -> bool {
        self.alive.load(Ordering::Acquire) && self.revision.load(Ordering::Acquire) == revision
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum RenderFailure {
    Cancelled,
    Worker(String),
    InvalidSource(String),
}
struct Process {
    child: Arc<Mutex<Child>>,
    input: ChildStdin,
    output: mpsc::Receiver<Result<Vec<u8>, String>>,
}
impl Drop for Process {
    fn drop(&mut self) {
        terminate_child(&self.child);
    }
}
// All kill/reap operations keep the same Child under one lock, avoiding raw
// PID signaling races when an exited child's PID is reused by the OS.
fn terminate_child(child: &Mutex<Child>) {
    let mut child = child.lock().unwrap_or_else(|error| error.into_inner());
    if !matches!(child.try_wait(), Ok(Some(_))) {
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn reap_closed_output(child: &Mutex<Child>) {
    // EOF can precede the kernel's exit notification slightly. Give normal
    // shutdown a short grace period, without holding the cancellation lock.
    for _ in 0..5 {
        {
            let mut child = child.lock().unwrap_or_else(|error| error.into_inner());
            if matches!(child.try_wait(), Ok(Some(_))) {
                return;
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    // A process with a broken response stream can no longer serve requests.
    terminate_child(child);
}

pub struct NativeRendererClient {
    path: PathBuf,
    document: u64,
    sequence: u64,
    process: Option<Process>,
}
impl NativeRendererClient {
    pub fn new(path: PathBuf, document: u64) -> Self {
        Self {
            path,
            document,
            sequence: 0,
            process: None,
        }
    }
    pub fn process_id(&self) -> Option<u32> {
        self.process.as_ref().and_then(|process| {
            let mut child = process
                .child
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            match child.try_wait() {
                Ok(Some(_)) => None,
                _ => Some(child.id()),
            }
        })
    }
    fn start(&mut self) -> Result<(), RenderFailure> {
        if let Some(process) = &mut self.process {
            let status = process
                .child
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .try_wait();
            match status {
                Ok(None) => return Ok(()),
                Ok(Some(_)) => self.process = None,
                Err(error) => return Err(RenderFailure::Worker(error.to_string())),
            }
        }
        let mut child = Command::new(&self.path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| RenderFailure::Worker(e.to_string()))?;
        let input = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let (sender, output) = mpsc::sync_channel(1);
        let child = Arc::new(Mutex::new(child));
        let reader_child = child.clone();
        let thread = std::thread::Builder::new()
            .name("yu-render-output".into())
            .spawn(move || {
                let mut reader = BufReader::new(stdout);
                loop {
                    let line = read_response(&mut reader);
                    let failed = line.is_err();
                    if failed {
                        reap_closed_output(&reader_child);
                    }
                    if sender.send(line).is_err() || failed {
                        break;
                    }
                }
            });
        if let Err(error) = thread {
            terminate_child(&child);
            return Err(RenderFailure::Worker(error.to_string()));
        }
        self.process = Some(Process {
            child,
            input,
            output,
        });
        Ok(())
    }
    pub fn render(
        &mut self,
        request: &EmbeddedRenderRequest,
        control: &RenderControl,
    ) -> Result<EmbeddedRenderPayload, RenderFailure> {
        let revision = request.revision().get();
        if !control.is_current(revision) {
            return Err(RenderFailure::Cancelled);
        }
        self.start()?;
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or_else(|| RenderFailure::Worker("Job identity exhausted".into()))?;
        let id = self.sequence;
        let message = serde_json::json!({"id":id, "document":self.document, "revision":revision,
            "kind":request.kind().language(), "source":request.source(),
            "style":{"font_milli":request.style().font_milli(),"foreground":request.style().foreground(),"dark":request.style().dark(),"display":request.style().display(),"reference_day":request.style().reference_day()}});
        let process = self.process.as_mut().expect("started");
        if let Err(error) =
            writeln!(process.input, "{message}").and_then(|()| process.input.flush())
        {
            self.process = None;
            return Err(RenderFailure::Worker(error.to_string()));
        }
        let deadline = Instant::now() + Duration::from_secs(30);
        let result = loop {
            if !control.is_current(revision) {
                break Err(RenderFailure::Cancelled);
            }
            if Instant::now() >= deadline {
                break Err(RenderFailure::Worker("Render deadline exceeded".into()));
            }
            match process.output.recv_timeout(Duration::from_millis(10)) {
                Ok(Ok(bytes)) => break decode_response(&bytes, id, self.document, revision),
                Ok(Err(error)) => break Err(RenderFailure::Worker(error)),
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    break Err(RenderFailure::Worker("Renderer exited".into()));
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        };
        // Recheck after decoding: a revision may advance while a response is read.
        let result = if control.is_current(revision) {
            result
        } else {
            Err(RenderFailure::Cancelled)
        };
        if matches!(
            result,
            Err(RenderFailure::Cancelled | RenderFailure::Worker(_))
        ) {
            self.process = None;
        }
        result
    }
}
fn read_response(reader: &mut impl BufRead) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    loop {
        let part = reader.fill_buf().map_err(|e| e.to_string())?;
        if part.is_empty() {
            return Err("Renderer output closed".into());
        }
        let end = part.iter().position(|b| *b == b'\n').map(|n| n + 1);
        let count = end.unwrap_or(part.len());
        if bytes.len() + count > MAX_RESPONSE {
            return Err("Renderer output too large".into());
        }
        bytes.extend_from_slice(&part[..count]);
        reader.consume(count);
        if end.is_some() {
            return Ok(bytes);
        }
    }
}
fn decode_response(
    bytes: &[u8],
    id: u64,
    document: u64,
    revision: u64,
) -> Result<EmbeddedRenderPayload, RenderFailure> {
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|e| RenderFailure::Worker(e.to_string()))?;
    for (key, expected) in [("id", id), ("document", document), ("revision", revision)] {
        if value[key].as_u64() != Some(expected) {
            return Err(RenderFailure::Worker(format!("Mismatched {key}")));
        }
    }
    match value["status"].as_str() {
        Some("failed") => Err(RenderFailure::InvalidSource(
            value["diagnostic"]
                .as_str()
                .unwrap_or("Render failed")
                .into(),
        )),
        Some("ready") => {
            let vector = &value["vector"];
            let dimension = |key| {
                vector[key]
                    .as_u64()
                    .filter(|n| *n > 0 && *n <= u64::from(yu_assets::EMBEDDED_SVG_MAX_DIMENSION))
                    .map(|n| n as u32)
            };
            let (Some(width), Some(height), Some(svg)) = (
                dimension("width"),
                dimension("height"),
                vector["svg"].as_str(),
            ) else {
                return Err(RenderFailure::Worker("Invalid vector response".into()));
            };
            if svg.len() > yu_assets::EMBEDDED_SVG_MAX_MARKUP_BYTES {
                return Err(RenderFailure::Worker("Oversized vector".into()));
            }
            let baseline = vector["baseline_milli"]
                .as_u64()
                .filter(|value| *value <= u64::from(height) * 1000)
                .ok_or_else(|| RenderFailure::Worker("Invalid vector baseline".into()))?;
            EmbeddedRenderPayload::svg(width, height, svg)
                .and_then(|payload| payload.with_baseline(baseline as u32))
                .map_err(|e| RenderFailure::Worker(e.to_string()))
        }
        _ => Err(RenderFailure::Worker("Unknown response status".into())),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_other_jobs_documents_and_revisions() {
        let bytes = br#"{"id":1,"document":2,"revision":3,"status":"ready","vector":{"width":12,"height":20,"baseline_milli":16000,"svg":"<svg/>"}}"#;
        assert!(decode_response(bytes, 1, 2, 3).is_ok());
        for (id, doc, rev) in [(2, 2, 3), (1, 3, 3), (1, 2, 4)] {
            assert!(decode_response(bytes, id, doc, rev).is_err());
        }
    }
    #[test]
    fn construction_is_lazy_and_control_is_shared() {
        let client = NativeRendererClient::new(PathBuf::from("/not-a-renderer"), 1);
        assert!(client.process_id().is_none());
        let control = RenderControl::default();
        let worker = control.clone();
        control.set_revision(5);
        assert!(worker.is_current(5));
        assert!(!worker.is_current(4));
        control.close();
        assert!(!worker.is_current(5));
    }
}
