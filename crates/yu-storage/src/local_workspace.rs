//! Bounded, metadata-only local workspace discovery shared by native shells.
//! No document contents, private project files or persistent search database.
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::time::{Duration, Instant};

const MAX_FILES: usize = 5_000;
const MAX_ENTRIES: usize = 50_000;
const MAX_DEPTH: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceFile {
    pub path: PathBuf,
    pub relative: String,
}

#[derive(Debug)]
pub struct WorkspaceIndex {
    pub root: PathBuf,
    pub files: Vec<WorkspaceFile>,
    pub truncated: bool,
    pub skipped: usize,
}

impl WorkspaceIndex {
    pub fn scan(root: &Path) -> io::Result<Self> {
        Self::scan_with(root, &AtomicBool::new(false), MAX_FILES, MAX_ENTRIES)
    }

    fn scan_with(
        root: &Path,
        cancel: &AtomicBool,
        file_limit: usize,
        entry_limit: usize,
    ) -> io::Result<Self> {
        let root = root.canonicalize()?;
        if !root.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Workspace is not a directory",
            ));
        }
        let mut index = Self {
            root: root.clone(),
            files: Vec::new(),
            truncated: false,
            skipped: 0,
        };
        let mut pending = vec![(root.clone(), 0)];
        let started = Instant::now();
        let mut visited = 0;
        'scan: while let Some((directory, depth)) = pending.pop() {
            if cancel.load(Ordering::Relaxed) {
                return Err(io::ErrorKind::Interrupted.into());
            }
            let entries = match std::fs::read_dir(&directory) {
                Ok(entries) => entries,
                Err(error) if directory == root => return Err(error),
                Err(_) => {
                    index.skipped += 1;
                    continue;
                }
            };
            let mut children = Vec::new();
            for entry in entries {
                if cancel.load(Ordering::Relaxed) {
                    return Err(io::ErrorKind::Interrupted.into());
                }
                if visited >= entry_limit
                    || index.files.len() >= file_limit
                    || started.elapsed() >= Duration::from_secs(2)
                {
                    index.truncated = true;
                    break 'scan;
                }
                visited += 1;
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(_) => {
                        index.skipped += 1;
                        continue;
                    }
                };
                let name = entry.file_name();
                let Some(name) = name.to_str() else {
                    index.skipped += 1;
                    continue;
                };
                if name.starts_with('.') {
                    continue;
                }
                let metadata = match entry.metadata() {
                    Ok(value) => value,
                    Err(_) => {
                        index.skipped += 1;
                        continue;
                    }
                };
                // Never follow symlinks or Windows junction/reparse directories.
                if metadata.file_type().is_symlink() {
                    continue;
                }
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    if metadata.file_attributes() & 0x400 != 0 {
                        continue;
                    }
                }
                let path = entry.path();
                if metadata.is_dir() {
                    if [
                        "node_modules",
                        "target",
                        "build",
                        "dist",
                        "DerivedData",
                        "artifacts",
                    ]
                    .iter()
                    .any(|excluded| name.eq_ignore_ascii_case(excluded))
                    {
                        continue;
                    }
                    if depth >= MAX_DEPTH {
                        index.truncated = true;
                        continue;
                    }
                    children.push((path, depth + 1));
                } else if metadata.is_file() && Self::supports(&path) {
                    if path.to_str().is_none() {
                        index.skipped += 1;
                        continue;
                    }
                    let relative = path
                        .strip_prefix(&root)
                        .expect("enumerated child")
                        .to_string_lossy()
                        .replace('\\', "/");
                    index.files.push(WorkspaceFile { path, relative });
                }
            }
            children.sort_by(|a, b| b.0.cmp(&a.0));
            pending.extend(children);
        }
        index.files.sort_by(|a, b| {
            a.relative
                .to_lowercase()
                .cmp(&b.relative.to_lowercase())
                .then(a.relative.cmp(&b.relative))
        });
        Ok(index)
    }

    pub fn supports(path: &Path) -> bool {
        path.extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                ["md", "markdown", "mdown", "txt"]
                    .iter()
                    .any(|value| extension.eq_ignore_ascii_case(value))
            })
    }

    /// Return deterministic filename-first matches, then path/subsequence matches.
    pub fn search(&self, query: &str, limit: usize) -> Vec<&WorkspaceFile> {
        let query = query.trim().to_lowercase().replace('\\', "/");
        if query.chars().count() > 256 {
            return Vec::new();
        }
        let mut hits: Vec<_> = self
            .files
            .iter()
            .filter_map(|file| {
                let path = file.relative.to_lowercase();
                let name = path.rsplit('/').next().unwrap_or(&path);
                let score = if query.is_empty() {
                    0
                } else if name == query {
                    0
                } else if name.starts_with(&query) {
                    10
                } else if name.contains(&query) {
                    20
                } else if path.contains(&query) {
                    30
                } else if query
                    .split_whitespace()
                    .all(|token| subsequence(token, &path))
                {
                    50
                } else {
                    return None;
                };
                Some((score, file))
            })
            .collect();
        // Stable sorting preserves the index's deterministic path order for ties.
        hits.sort_by_key(|(score, _)| *score);
        hits.into_iter()
            .take(limit.min(200))
            .map(|(_, file)| file)
            .collect()
    }

    /// Revalidate a selected path immediately before opening it. A symlink swapped
    /// after enumeration cannot silently escape the chosen workspace.
    pub fn resolve(&self, path: &Path) -> io::Result<PathBuf> {
        let path = path.canonicalize()?;
        if !path.starts_with(&self.root) || !path.is_file() || !Self::supports(&path) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "File is outside this workspace",
            ));
        }
        Ok(path)
    }
}

fn subsequence(needle: &str, haystack: &str) -> bool {
    let mut chars = haystack.chars();
    needle
        .chars()
        .all(|wanted| chars.by_ref().any(|actual| actual == wanted))
}

/// A cancellable scan owns no UI handles. Drop cancels without blocking the UI;
/// native hosts poll on their event loop and own all query/result presentation.
pub struct WorkspaceScan {
    cancel: Arc<AtomicBool>,
    receiver: mpsc::Receiver<io::Result<WorkspaceIndex>>,
}
impl WorkspaceScan {
    pub fn start(root: PathBuf) -> io::Result<Self> {
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&cancel);
        let (sender, receiver) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("yu-workspace-scan".into())
            .spawn(move || {
                let result = WorkspaceIndex::scan_with(&root, &flag, MAX_FILES, MAX_ENTRIES);
                let _ = sender.send(result);
            })?;
        Ok(Self { cancel, receiver })
    }
    pub fn poll(&self) -> Option<io::Result<WorkspaceIndex>> {
        match self.receiver.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Err(io::Error::other("Workspace scan stopped")))
            }
        }
    }
}
impl Drop for WorkspaceScan {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "yu-workspace-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).expect("fixture");
            Self(path)
        }
        fn file(&self, name: &str) {
            let path = self.0.join(name);
            std::fs::create_dir_all(path.parent().expect("parent")).expect("directory");
            std::fs::write(path, [0xff, 0xfe, 0x00]).expect("metadata scan must not read text");
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn workspace_discovers_nested_markdown_without_reading_contents() {
        let fixture = Fixture::new();
        for name in [
            "README.MD",
            "docs/羽🙂.markdown",
            "draft.mdown",
            "notes.txt",
            "picture.png",
            ".hidden.md",
            "node_modules/x.md",
            "target/x.md",
        ] {
            fixture.file(name);
        }
        let index = WorkspaceIndex::scan(&fixture.0).expect("scan");
        assert_eq!(index.files.len(), 4);
        assert!(!index.truncated);
        assert_eq!(index.search("羽🙂", 50).len(), 1);
        assert_eq!(index.search("docs 羽", 50).len(), 1);
        assert_eq!(index.search("rdm", 50)[0].relative, "README.MD");
        assert!(index.search("not-present", 50).is_empty());
        assert_eq!(index.search("", 2).len(), 2);
    }
    #[test]
    fn workspace_ranks_filename_before_path() {
        let fixture = Fixture::new();
        for name in ["readme.md", "readme/other.md", "x-readme.md"] {
            fixture.file(name);
        }
        let index = WorkspaceIndex::scan(&fixture.0).expect("scan");
        let hits = index.search("README", 20);
        assert_eq!(hits[0].relative, "readme.md");
        assert_eq!(hits[1].relative, "x-readme.md");
        assert_eq!(hits[2].relative, "readme/other.md");
    }
    #[test]
    fn workspace_budgets_and_cancellation_are_explicit() {
        let fixture = Fixture::new();
        for n in 0..12 {
            fixture.file(&format!("{n}.md"));
        }
        let index =
            WorkspaceIndex::scan_with(&fixture.0, &AtomicBool::new(false), 3, 100).expect("scan");
        assert_eq!(index.files.len(), 3);
        assert!(index.truncated);
        assert!(WorkspaceIndex::scan_with(&fixture.0, &AtomicBool::new(true), 100, 100).is_err());
        assert!(WorkspaceIndex::scan(&fixture.0.join("missing")).is_err());
    }
    #[test]
    fn workspace_rejects_stale_or_outside_selection() {
        let fixture = Fixture::new();
        fixture.file("one.md");
        let other = Fixture::new();
        other.file("outside.md");
        let index = WorkspaceIndex::scan(&fixture.0).expect("scan");
        assert!(index.resolve(&other.0.join("outside.md")).is_err());
        assert!(index.resolve(&index.files[0].path).is_ok());
        std::fs::remove_file(&index.files[0].path).expect("remove");
        assert!(index.resolve(&index.files[0].path).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn workspace_never_follows_symlink_directories_or_files() {
        let fixture = Fixture::new();
        fixture.file("one.md");
        std::os::unix::fs::symlink(&fixture.0, fixture.0.join("loop")).expect("link");
        std::os::unix::fs::symlink(fixture.0.join("one.md"), fixture.0.join("alias.md"))
            .expect("link");
        let index = WorkspaceIndex::scan(&fixture.0).expect("scan");
        assert_eq!(index.files.len(), 1);
    }
    #[test]
    fn workspace_async_scan_returns_the_same_sorted_snapshot() {
        let fixture = Fixture::new();
        fixture.file("z.md");
        fixture.file("a.md");
        let scan = WorkspaceScan::start(fixture.0.clone()).expect("worker");
        let deadline = Instant::now() + Duration::from_secs(5);
        let index = loop {
            if let Some(result) = scan.poll() {
                break result.expect("scan");
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(index.files[0].relative, "a.md");
    }
}
