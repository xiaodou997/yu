#![forbid(unsafe_code)]

//! Headless Markdown document sessions and UTF-8 file persistence.
//!
//! `DocumentSession` keeps the file boundary deliberately smaller than the
//! editor model: the [`yu_editor::EditorDocument`] remains the only canonical
//! source, while this crate tracks the path, BOM representation, disk
//! fingerprint and revision last known to be saved.

use std::error::Error;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

use yu_core::{Revision, TextRange, Utf16Range};
use yu_editor::{
    CommandResult, CompositionError, CompositionOverlay, EditorCommand, EditorDocument,
    EditorDocumentError, EditorSelection, KeyEvent, KeyRouteResult,
};
use yu_text::{AppliedTransaction, TextSnapshot, Transaction};

static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

mod close;
mod watch;

pub use close::{
    ClosePrompt, CloseRequest, CloseState, CloseStateError, CloseStateMachine, CloseTransition,
};
pub use watch::{FileWatchCheck, FileWatchDebouncer, FileWatchEvent, FileWatchReason};

/// Whether a loaded UTF-8 file contained the standard UTF-8 BOM.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Utf8Bom {
    #[default]
    Absent,
    Present,
}

impl Utf8Bom {
    fn prefix(self) -> &'static [u8] {
        match self {
            Self::Absent => &[],
            Self::Present => b"\xEF\xBB\xBF",
        }
    }
}

/// The observed state of the session path compared with the last loaded or
/// saved file fingerprint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExternalFileState {
    Changed,
    Missing,
}

/// Result of checking the path without mutating the session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiskState {
    Unchanged,
    Changed,
    Missing,
}

/// Result of an explicit save operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveOutcome {
    /// The source was written using an atomic sibling-file replacement.
    Saved {
        revision: Revision,
        bytes_written: usize,
    },
    /// The session was clean and the path still matched its known fingerprint.
    Unchanged { revision: Revision },
}

/// Result of reloading the path into the canonical editor source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReloadOutcome {
    pub revision: Revision,
    pub bom: Utf8Bom,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FileFingerprint {
    length: u64,
    modified: Option<SystemTime>,
    content_hash: u64,
}

impl FileFingerprint {
    fn from_bytes(bytes: &[u8], metadata: &fs::Metadata) -> Self {
        Self {
            length: metadata.len(),
            modified: metadata.modified().ok(),
            content_hash: fnv1a(bytes),
        }
    }
}

/// A file-backed editor session with revision-bound dirty and conflict state.
pub struct DocumentSession {
    path: PathBuf,
    editor: EditorDocument,
    bom: Utf8Bom,
    saved_revision: Revision,
    expected_file: Option<FileFingerprint>,
}

impl fmt::Debug for DocumentSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DocumentSession")
            .field("path", &self.path)
            .field("revision", &self.editor.revision())
            .field("saved_revision", &self.saved_revision)
            .field("bom", &self.bom)
            .field("dirty", &self.is_dirty())
            .finish_non_exhaustive()
    }
}

impl DocumentSession {
    /// Opens an existing UTF-8 Markdown file. A UTF-8 BOM is tracked as file
    /// metadata and is restored on save; it is not inserted into Markdown
    /// source coordinates.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, StorageError> {
        let path = path.into();
        let (source, bom, fingerprint) = read_document(&path)?;
        let editor = EditorDocument::new(source);
        let saved_revision = editor.revision();
        Ok(Self {
            path,
            editor,
            bom,
            saved_revision,
            expected_file: Some(fingerprint),
        })
    }

    /// Creates a new unsaved session. The path must not be replaced silently:
    /// if a file appears before the first save, save returns an external-change
    /// conflict instead of overwriting it.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>, source: impl Into<String>) -> Self {
        let editor = EditorDocument::new(source);
        let saved_revision = editor.revision();
        Self {
            path: path.into(),
            editor,
            bom: Utf8Bom::Absent,
            saved_revision,
            expected_file: None,
        }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub fn editor(&self) -> &EditorDocument {
        &self.editor
    }

    /// Returns the canonical source selection owned by the editor.
    #[must_use]
    pub fn selection(&self) -> EditorSelection {
        self.editor.selection()
    }

    /// Returns the active transient IME overlay, if any.
    #[must_use]
    pub fn composition(&self) -> Option<&CompositionOverlay> {
        self.editor.composition()
    }

    /// Replaces the canonical source selection after the editor validates its
    /// revision and UTF-8 boundaries.
    pub fn set_selection(&mut self, selection: EditorSelection) -> Result<(), StorageError> {
        self.editor
            .set_selection(selection)
            .map_err(|error| StorageError::Editor(EditorDocumentError::Selection(error)))
    }

    /// Resolves a native key through the canonical editor command route.
    pub fn route_key(&mut self, event: KeyEvent) -> Result<KeyRouteResult, StorageError> {
        self.editor.route_key(event).map_err(StorageError::Editor)
    }

    /// Reports whether a command is available without mutating editor state.
    #[must_use]
    pub fn command_available(&self, command: &EditorCommand) -> bool {
        self.editor.command_available(command)
    }

    #[must_use]
    pub fn revision(&self) -> Revision {
        self.editor.revision()
    }

    #[must_use]
    pub fn saved_revision(&self) -> Revision {
        self.saved_revision
    }

    #[must_use]
    pub fn bom(&self) -> Utf8Bom {
        self.bom
    }

    /// Dirty is revision-bound. Undoing back to the same bytes still leaves a
    /// session dirty until an explicit save establishes the new revision as
    /// the persisted boundary.
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.expected_file.is_none() || self.revision() != self.saved_revision
    }

    /// Compares the current path with the fingerprint captured on open/save.
    /// This is read-only and does not refresh the expected fingerprint.
    pub fn disk_state(&self) -> Result<DiskState, StorageError> {
        let current = current_fingerprint(&self.path)?;
        Ok(match (&self.expected_file, current) {
            (None, None) => DiskState::Missing,
            (None, Some(_)) => DiskState::Changed,
            (Some(_), None) => DiskState::Missing,
            (Some(expected), Some(current)) if expected == &current => DiskState::Unchanged,
            (Some(_), Some(_)) => DiskState::Changed,
        })
    }

    /// Returns an external-change reason suitable for product-shell prompts.
    ///
    /// A new session starts with an intentionally absent expected fingerprint:
    /// a missing path is still the expected state, while a path that appears
    /// before the first save is treated as a conflict. A clean opened session
    /// may close without prompting because it has no local source to lose;
    /// callers that need to refresh it can use [`Self::reload`].
    pub fn external_file_state(&self) -> Result<Option<ExternalFileState>, StorageError> {
        Ok(match (self.expected_file.is_some(), self.disk_state()?) {
            (_, DiskState::Unchanged) | (false, DiskState::Missing) => None,
            (_, DiskState::Changed) => Some(ExternalFileState::Changed),
            (true, DiskState::Missing) => Some(ExternalFileState::Missing),
        })
    }

    /// Evaluates a close request without dropping or mutating the document.
    ///
    /// The state machine owns only close intent; the caller remains responsible
    /// for invoking [`Self::save`] after a save prompt and dropping the session
    /// only after [`CloseStateMachine::save_succeeded`] or
    /// [`CloseStateMachine::discard`].
    pub fn close_request(
        &self,
        state: &mut CloseStateMachine,
    ) -> Result<CloseRequest, StorageError> {
        let external_change = self.external_file_state()?;
        Ok(state.request_close(self.is_dirty(), external_change))
    }

    /// Executes a permanent editor command through the canonical source.
    pub fn execute(&mut self, command: EditorCommand) -> Result<CommandResult, StorageError> {
        self.editor.execute(command).map_err(StorageError::Editor)
    }

    /// Applies an external transaction through the same source/parser/history
    /// path used by editor commands.
    pub fn apply_transaction(
        &mut self,
        transaction: &Transaction,
    ) -> Result<AppliedTransaction, StorageError> {
        self.editor
            .apply_transaction(transaction)
            .map_err(StorageError::Editor)
    }

    /// Starts a transient IME composition without changing dirty state.
    pub fn begin_composition(
        &mut self,
        replacement_range: TextRange,
        text: impl Into<std::sync::Arc<str>>,
        selection_utf16: Utf16Range,
    ) -> Result<(), StorageError> {
        self.editor
            .begin_composition(replacement_range, text, selection_utf16)
            .map_err(StorageError::Editor)
    }

    pub fn update_composition(
        &mut self,
        text: impl Into<std::sync::Arc<str>>,
        selection_utf16: Utf16Range,
    ) -> Result<(), StorageError> {
        self.editor
            .update_composition(text, selection_utf16)
            .map_err(StorageError::Editor)
    }

    pub fn commit_composition(
        &mut self,
        committed_text: impl Into<std::sync::Arc<str>>,
    ) -> Result<AppliedTransaction, StorageError> {
        self.editor
            .commit_composition(committed_text)
            .map_err(StorageError::Editor)
    }

    #[must_use]
    pub fn cancel_composition(&mut self) -> bool {
        self.editor.cancel_composition()
    }

    /// Saves the canonical source via a same-directory temporary file and
    /// atomic rename. A changed/missing target is never overwritten.
    pub fn save(&mut self) -> Result<SaveOutcome, StorageError> {
        match self.disk_state()? {
            DiskState::Unchanged => {}
            DiskState::Changed => {
                return Err(StorageError::ExternalChange {
                    path: self.path.clone(),
                    state: ExternalFileState::Changed,
                });
            }
            DiskState::Missing if self.expected_file.is_some() => {
                return Err(StorageError::ExternalChange {
                    path: self.path.clone(),
                    state: ExternalFileState::Missing,
                });
            }
            DiskState::Missing => {}
        }

        if !self.is_dirty() {
            return Ok(SaveOutcome::Unchanged {
                revision: self.revision(),
            });
        }

        let bytes = serialize_source(self.editor.snapshot().as_str(), self.bom);
        atomic_replace(&self.path, &bytes)?;
        let metadata = fs::metadata(&self.path)
            .map_err(|source| StorageError::io("stat", &self.path, source))?;
        self.expected_file = Some(FileFingerprint::from_bytes(&bytes, &metadata));
        self.saved_revision = self.revision();
        Ok(SaveOutcome::Saved {
            revision: self.saved_revision,
            bytes_written: bytes.len(),
        })
    }

    /// Reloads the file only when the editor has no unsaved revision.
    pub fn reload(&mut self) -> Result<ReloadOutcome, StorageError> {
        if self.is_dirty() {
            return Err(StorageError::UnsavedChanges {
                path: self.path.clone(),
            });
        }
        let (source, bom, fingerprint) = read_document(&self.path)?;
        self.editor
            .reset_source(source)
            .map_err(StorageError::Editor)?;
        self.bom = bom;
        self.saved_revision = self.editor.revision();
        self.expected_file = Some(fingerprint);
        Ok(ReloadOutcome {
            revision: self.saved_revision,
            bom,
        })
    }
}

/// The single Rust-owned product session for a file-backed editor.
///
/// `DocumentSession` owns the canonical `EditorDocument`, file fingerprint,
/// BOM and save boundary. This facade adds close lifecycle and makes command,
/// selection and composition operations available through one object so a
/// native host never needs separate storage and editor handles.
#[derive(Debug)]
pub struct DocumentEditorSession {
    document: DocumentSession,
    close: CloseStateMachine,
    composition_generation: u64,
}

impl DocumentEditorSession {
    /// Opens an existing UTF-8 Markdown file as one unified session.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, StorageError> {
        Ok(Self {
            document: DocumentSession::open(path)?,
            close: CloseStateMachine::new(),
            composition_generation: 0,
        })
    }

    /// Creates a new unsaved unified editor session.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>, source: impl Into<String>) -> Self {
        Self {
            document: DocumentSession::new(path, source),
            close: CloseStateMachine::new(),
            composition_generation: 0,
        }
    }

    #[must_use]
    pub fn document(&self) -> &DocumentSession {
        &self.document
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        self.document.path()
    }

    #[must_use]
    pub fn snapshot(&self) -> TextSnapshot {
        self.document.editor().snapshot()
    }

    #[must_use]
    pub fn revision(&self) -> Revision {
        self.document.revision()
    }

    #[must_use]
    pub fn saved_revision(&self) -> Revision {
        self.document.saved_revision()
    }

    #[must_use]
    pub fn bom(&self) -> Utf8Bom {
        self.document.bom()
    }

    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.document.is_dirty()
    }

    pub fn disk_state(&self) -> Result<DiskState, StorageError> {
        self.document.disk_state()
    }

    pub fn external_file_state(&self) -> Result<Option<ExternalFileState>, StorageError> {
        self.document.external_file_state()
    }

    #[must_use]
    pub fn close_state(&self) -> CloseState {
        self.close.state()
    }

    #[must_use]
    pub fn selection(&self) -> EditorSelection {
        self.document.selection()
    }

    #[must_use]
    pub fn composition(&self) -> Option<&CompositionOverlay> {
        self.document.composition()
    }

    /// Monotonically identifies the transient composition state owned by this
    /// session. Native text-input mirrors must include it with every marked
    /// text update so a late callback cannot mutate a newer composition.
    #[must_use]
    pub fn composition_generation(&self) -> u64 {
        self.composition_generation
    }

    pub fn execute(&mut self, command: EditorCommand) -> Result<CommandResult, StorageError> {
        self.document.execute(command)
    }

    pub fn route_key(&mut self, event: KeyEvent) -> Result<KeyRouteResult, StorageError> {
        self.document.route_key(event)
    }

    #[must_use]
    pub fn command_available(&self, command: &EditorCommand) -> bool {
        self.document.command_available(command)
    }

    pub fn set_selection(&mut self, selection: EditorSelection) -> Result<(), StorageError> {
        self.document.set_selection(selection)
    }

    pub fn begin_composition(
        &mut self,
        replacement_range: TextRange,
        text: impl Into<std::sync::Arc<str>>,
        selection_utf16: Utf16Range,
    ) -> Result<(), StorageError> {
        let result = self
            .document
            .begin_composition(replacement_range, text, selection_utf16);
        if result.is_ok() {
            self.composition_generation = self.composition_generation.wrapping_add(1);
        }
        result
    }

    pub fn update_composition(
        &mut self,
        text: impl Into<std::sync::Arc<str>>,
        selection_utf16: Utf16Range,
    ) -> Result<(), StorageError> {
        let result = self.document.update_composition(text, selection_utf16);
        if result.is_ok() {
            self.composition_generation = self.composition_generation.wrapping_add(1);
        }
        result
    }

    pub fn commit_composition(
        &mut self,
        committed_text: impl Into<std::sync::Arc<str>>,
    ) -> Result<AppliedTransaction, StorageError> {
        let result = self.document.commit_composition(committed_text);
        if result.is_ok() {
            self.composition_generation = self.composition_generation.wrapping_add(1);
        }
        result
    }

    #[must_use]
    pub fn cancel_composition(&mut self) -> bool {
        let cancelled = self.document.cancel_composition();
        if cancelled {
            self.composition_generation = self.composition_generation.wrapping_add(1);
        }
        cancelled
    }

    pub fn save(&mut self) -> Result<SaveOutcome, StorageError> {
        self.document.save()
    }

    pub fn reload(&mut self) -> Result<ReloadOutcome, StorageError> {
        let outcome = self.document.reload()?;
        self.composition_generation = self.composition_generation.wrapping_add(1);
        Ok(outcome)
    }

    pub fn close_request(&mut self) -> Result<CloseRequest, StorageError> {
        self.document.close_request(&mut self.close)
    }

    pub fn cancel_close(&mut self) -> Result<CloseTransition, CloseStateError> {
        self.close.cancel()
    }

    pub fn save_close(&mut self) -> Result<CloseTransition, StorageError> {
        if !matches!(self.close.state(), CloseState::Prompting(_)) {
            return Err(StorageError::CloseState(CloseStateError::NotPrompting));
        }
        match self.document.save() {
            Ok(_) => self
                .close
                .save_succeeded()
                .map_err(StorageError::CloseState),
            Err(error @ StorageError::ExternalChange { state, .. }) => {
                let _ = self.close.save_failed_external(state);
                Err(error)
            }
            Err(error) => Err(error),
        }
    }

    pub fn discard_close(&mut self) -> Result<CloseTransition, CloseStateError> {
        self.close.discard()
    }

    pub fn save_failed_external(
        &mut self,
        state: ExternalFileState,
    ) -> Result<CloseTransition, CloseStateError> {
        self.close.save_failed_external(state)
    }
}

fn read_document(path: &Path) -> Result<(String, Utf8Bom, FileFingerprint), StorageError> {
    let bytes = fs::read(path).map_err(|source| StorageError::io("read", path, source))?;
    let metadata = fs::metadata(path).map_err(|source| StorageError::io("stat", path, source))?;
    let (source_bytes, bom) = if bytes.starts_with(Utf8Bom::Present.prefix()) {
        (&bytes[Utf8Bom::Present.prefix().len()..], Utf8Bom::Present)
    } else {
        (&bytes[..], Utf8Bom::Absent)
    };
    let source =
        String::from_utf8(source_bytes.to_vec()).map_err(|source| StorageError::InvalidUtf8 {
            path: path.to_path_buf(),
            source,
        })?;
    Ok((source, bom, FileFingerprint::from_bytes(&bytes, &metadata)))
}

fn current_fingerprint(path: &Path) -> Result<Option<FileFingerprint>, StorageError> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(StorageError::io("read", path, source)),
    };
    let metadata = fs::metadata(path).map_err(|source| StorageError::io("stat", path, source))?;
    Ok(Some(FileFingerprint::from_bytes(&bytes, &metadata)))
}

fn serialize_source(source: &str, bom: Utf8Bom) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(bom.prefix().len() + source.len());
    bytes.extend_from_slice(bom.prefix());
    bytes.extend_from_slice(source.as_bytes());
    bytes
}

fn atomic_replace(path: &Path, bytes: &[u8]) -> Result<(), StorageError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| StorageError::InvalidPath(path.to_path_buf()))?;
    let counter = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp_path = parent.join(format!(
        ".{file_name}.yu-save-{}-{counter}.tmp",
        std::process::id()
    ));
    let existing_permissions = fs::metadata(path)
        .ok()
        .map(|metadata| metadata.permissions());
    let mut guard = TempFileGuard::new(temp_path.clone());
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp_path)
        .map_err(|source| StorageError::io("create temporary save", &temp_path, source))?;
    file.write_all(bytes)
        .map_err(|source| StorageError::io("write temporary save", &temp_path, source))?;
    file.sync_all()
        .map_err(|source| StorageError::io("sync temporary save", &temp_path, source))?;
    drop(file);
    if let Some(permissions) = existing_permissions {
        fs::set_permissions(&temp_path, permissions)
            .map_err(|source| StorageError::io("set temporary permissions", &temp_path, source))?;
    }
    fs::rename(&temp_path, path)
        .map_err(|source| StorageError::io("atomic rename", path, source))?;
    guard.disarm();
    Ok(())
}

struct TempFileGuard {
    path: PathBuf,
    armed: bool,
}

impl TempFileGuard {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 14_695_981_039_346_656_037_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(1_099_511_628_211_u64);
    }
    hash
}

/// Errors at the file/session boundary.
#[derive(Debug)]
pub enum StorageError {
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    InvalidUtf8 {
        path: PathBuf,
        source: std::string::FromUtf8Error,
    },
    InvalidPath(PathBuf),
    ExternalChange {
        path: PathBuf,
        state: ExternalFileState,
    },
    UnsavedChanges {
        path: PathBuf,
    },
    CloseState(CloseStateError),
    Editor(EditorDocumentError),
}

impl StorageError {
    fn io(operation: &'static str, path: &Path, source: io::Error) -> Self {
        Self::Io {
            operation,
            path: path.to_path_buf(),
            source,
        }
    }
}

impl fmt::Display for StorageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io {
                operation,
                path,
                source,
            } => write!(formatter, "{operation} {}: {source}", path.display()),
            Self::InvalidUtf8 { path, .. } => {
                write!(formatter, "file {} is not valid UTF-8", path.display())
            }
            Self::InvalidPath(path) => write!(formatter, "cannot save path {}", path.display()),
            Self::ExternalChange { path, state } => {
                write!(
                    formatter,
                    "file {} changed externally ({state:?})",
                    path.display()
                )
            }
            Self::UnsavedChanges { path } => {
                write!(
                    formatter,
                    "cannot reload {} with unsaved changes",
                    path.display()
                )
            }
            Self::CloseState(error) => write!(formatter, "invalid close transition: {error:?}"),
            Self::Editor(error) => error.fmt(formatter),
        }
    }
}

impl Error for StorageError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::InvalidUtf8 { source, .. } => Some(source),
            Self::Editor(error) => Some(error),
            Self::InvalidPath(_)
            | Self::ExternalChange { .. }
            | Self::UnsavedChanges { .. }
            | Self::CloseState(_) => None,
        }
    }
}

impl From<CloseStateError> for StorageError {
    fn from(error: CloseStateError) -> Self {
        Self::CloseState(error)
    }
}

impl From<EditorDocumentError> for StorageError {
    fn from(error: EditorDocumentError) -> Self {
        Self::Editor(error)
    }
}

impl From<CompositionError> for StorageError {
    fn from(error: CompositionError) -> Self {
        Self::Editor(EditorDocumentError::Composition(error))
    }
}
