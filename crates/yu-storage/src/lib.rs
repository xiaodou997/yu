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

use yu_core::{ByteOffset, Revision, TextRange, Utf16Range};
use yu_editor::{
    BlockProjection, BlockView, CaretScrollRequest, CommandResult, CompositionError,
    CompositionOverlay, EditorCommand, EditorDocument, EditorDocumentError, EditorSelection,
    KeyEvent, KeyRouteResult, LayoutConfig, MonospaceMetrics, Projection, ShapingProvider,
    ViewportConfig, ViewportSnapshot, ViewportSpan,
};
use yu_text::{AppliedTransaction, TextSnapshot, Transaction};

static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

mod close;
mod recovery;
mod watch;

pub use close::{
    ClosePrompt, CloseRequest, CloseState, CloseStateError, CloseStateMachine, CloseTransition,
};
pub use recovery::{RecoveryError, RecoveryOutcome, RecoveryRecord, RecoveryStore};
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
    storage_path: PathBuf,
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
            .field("storage_path", &self.storage_path)
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
        let storage_path = canonical_storage_path(&path)?.ok_or_else(|| {
            StorageError::io(
                "canonicalize",
                &path,
                io::Error::from(io::ErrorKind::NotFound),
            )
        })?;
        let (source, bom, fingerprint) = read_document(&storage_path)?;
        let editor = EditorDocument::new(source);
        let saved_revision = editor.revision();
        Ok(Self {
            path,
            storage_path,
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
        let path = path.into();
        let editor = EditorDocument::new(source);
        let saved_revision = editor.revision();
        Self {
            storage_path: path.clone(),
            path,
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

    /// Returns the canonical target used for reads, fingerprints and atomic
    /// replacement. For a normal file this is its absolute canonical path;
    /// for a symlink it is the link target, while [`Self::path`] remains the
    /// user-facing path.
    #[must_use]
    pub fn storage_path(&self) -> &Path {
        &self.storage_path
    }

    #[must_use]
    pub fn editor(&self) -> &EditorDocument {
        &self.editor
    }

    /// Returns the canonical editor mutably for product-internal integration
    /// layers that assemble revision-bound layout/scene snapshots. The caller
    /// must keep all mutations on the same session boundary; no second editor
    /// or source is created.
    pub fn editor_mut(&mut self) -> &mut EditorDocument {
        &mut self.editor
    }

    /// Returns the canonical source selection owned by the editor.
    #[must_use]
    pub fn selection(&self) -> EditorSelection {
        self.editor.selection()
    }

    /// Builds the current revision's source-backed inline projection through
    /// the editor-owned projection cache. The returned value is an owned
    /// snapshot so a native FFI caller cannot retain a Rust borrow across
    /// another session operation.
    pub fn inline_projection(&mut self) -> Result<Projection, StorageError> {
        let snapshot = self.editor.snapshot();
        let source_range = TextRange::new(ByteOffset::ZERO, snapshot.len_bytes())
            .expect("a snapshot's full range is always ordered");
        self.editor
            .projection(source_range)
            .cloned()
            .map_err(StorageError::Editor)
    }

    /// Builds the selection-bound visual projection used by the native mirror
    /// and retained renderer. It bypasses Revision-only caches so a caret move
    /// can reveal inline syntax without mutating canonical source.
    pub fn inline_projection_for_visual_state(&self) -> Result<Projection, StorageError> {
        let snapshot = self.editor.snapshot();
        let source_range = TextRange::new(ByteOffset::ZERO, snapshot.len_bytes())
            .expect("a snapshot's full range is always ordered");
        self.editor
            .projection_with_selection_reveal(source_range)
            .map_err(StorageError::Editor)
    }

    /// Builds an owned metrics layout for the current full-source projection.
    ///
    /// This is intentionally a diagnostic/native bridge operation rather than
    /// a second layout cache. The editor remains the owner of projections and
    /// source; callers receive a revision-bound snapshot that can answer
    /// source↔visual↔point queries without retaining a borrow into the editor.
    pub fn inline_layout(&mut self, config: LayoutConfig) -> Result<BlockView, StorageError> {
        let projection = BlockProjection::Inline(self.inline_projection()?);
        BlockView::build(
            &projection,
            config,
            &MonospaceMetrics::new(config.default_advance()),
        )
        .map_err(|error| StorageError::Editor(EditorDocumentError::Layout(error)))
    }

    /// Builds the current transient composition over the same full-source
    /// projection used by native layout diagnostics. The preedit is visual
    /// only: the canonical buffer, Markdown CST and source Revision remain
    /// unchanged until `commit_composition` succeeds.
    pub fn composition_projection(&mut self) -> Result<Projection, StorageError> {
        let overlay = self
            .editor
            .composition()
            .cloned()
            .ok_or(StorageError::Editor(
                EditorDocumentError::CompositionNotActive,
            ))?;
        let snapshot = self.editor.snapshot();
        let source_range = TextRange::new(ByteOffset::ZERO, snapshot.len_bytes())
            .expect("a snapshot's full range is always ordered");
        let base = self
            .editor
            .projection(source_range)
            .map_err(StorageError::Editor)?
            .clone();
        base.with_composition(
            overlay.replacement_range(),
            overlay.text().to_owned(),
            overlay.selection_bytes(),
        )
        .map_err(|error| StorageError::Editor(error.into()))
    }

    /// Returns the number of parser-owned blocks in the current source
    /// revision. Block indices are stable only for this revision and must be
    /// paired with the revision returned by the session.
    #[must_use]
    pub fn block_count(&self) -> usize {
        self.editor.markdown().blocks().len()
    }

    /// Returns source-backed metadata for one parser-owned block. The scalar
    /// kind is the stable Markdown block tag used by the native boundary; the
    /// Rust enum itself never crosses the FFI.
    #[must_use]
    pub fn block_metadata(&self, index: usize) -> Option<(TextRange, u8)> {
        self.editor
            .markdown()
            .blocks()
            .get(index)
            .map(|block| (block.range(), block.kind().viewport_tag()))
    }

    /// Builds an owned projection for one parser-owned block. The projection
    /// remains tied to the current editor revision and is safe to hand to a
    /// synchronous native snapshot query.
    pub fn block_projection(&mut self, index: usize) -> Result<BlockProjection, StorageError> {
        self.editor
            .block_projection(index)
            .cloned()
            .map_err(StorageError::Editor)
    }

    pub fn block_projection_for_visual_state(
        &self,
        index: usize,
    ) -> Result<BlockProjection, StorageError> {
        if self.editor.selection_reveal_block_index() == Some(index) {
            self.editor
                .block_projection_with_selection_reveal(index)
                .map_err(StorageError::Editor)
        } else {
            let block = self
                .editor
                .markdown()
                .blocks()
                .get(index)
                .ok_or(StorageError::Editor(
                    EditorDocumentError::BlockOutOfBounds {
                        index,
                        blocks: self.editor.markdown().blocks().len(),
                    },
                ))?;
            BlockProjection::from_block_with_definitions(
                &self.editor.snapshot(),
                block,
                self.editor.markdown().reference_definitions(),
            )
            .map_err(|error| StorageError::Editor(error.into()))
        }
    }

    /// Builds an owned metrics layout for one parser-owned block. The block
    /// index and returned snapshot are tied to the current source Revision;
    /// no layout object is retained by the file session.
    pub fn block_layout(
        &mut self,
        index: usize,
        config: LayoutConfig,
    ) -> Result<BlockView, StorageError> {
        self.editor
            .block_layout(index, config)
            .cloned()
            .map_err(StorageError::Editor)
    }

    /// Builds an owned shaped layout for one parser-owned block. The shaper is
    /// platform-owned and never becomes part of the canonical document state.
    pub fn block_layout_with_shaper<S: ShapingProvider>(
        &mut self,
        index: usize,
        config: LayoutConfig,
        shaper: &S,
    ) -> Result<BlockView, StorageError> {
        self.editor
            .block_layout_with_shaper(index, config, shaper)
            .cloned()
            .map_err(StorageError::Editor)
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

    /// Measures the current visible block window through the same document
    /// session that owns source, dirty state and file conflicts.
    pub fn visible_blocks(
        &mut self,
        viewport: ViewportSpan,
    ) -> Result<ViewportSnapshot, StorageError> {
        self.editor
            .visible_blocks(viewport)
            .map_err(StorageError::Editor)
    }

    /// Measures the current visible block window with an explicit shaping
    /// provider. The editor owns viewport estimates and measurements.
    pub fn visible_blocks_with_shaper<S: ShapingProvider>(
        &mut self,
        viewport: ViewportSpan,
        shaper: &S,
    ) -> Result<ViewportSnapshot, StorageError> {
        self.editor
            .visible_blocks_with_shaper(viewport, shaper)
            .map_err(StorageError::Editor)
    }

    /// Resolves the focus caret through the same shaped viewport policy used
    /// by `visible_blocks_with_shaper`, returning an absolute document-space
    /// scroll target without changing canonical source state.
    pub fn caret_scroll_request_with_shaper<S: ShapingProvider>(
        &mut self,
        viewport: ViewportSpan,
        margin: f32,
        shaper: &S,
    ) -> Result<CaretScrollRequest, StorageError> {
        self.editor
            .caret_scroll_request_with_shaper(viewport, margin, shaper)
            .map_err(StorageError::Editor)
    }

    /// Replaces the viewport policy without changing the canonical source.
    pub fn set_viewport_config(&mut self, config: ViewportConfig) -> Result<(), StorageError> {
        self.editor
            .set_viewport_config(config)
            .map_err(|error| StorageError::Editor(EditorDocumentError::Viewport(error)))
    }

    #[must_use]
    pub fn viewport_config(&self) -> ViewportConfig {
        self.editor.viewport_config()
    }

    /// Writes or clears a caller-scheduled crash-recovery snapshot without
    /// changing the canonical source, Revision or dirty boundary.
    pub fn write_recovery(&self, store: &RecoveryStore) -> Result<RecoveryOutcome, RecoveryError> {
        store.write(self)
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
        let current_path = canonical_storage_path(&self.path)?;
        let current = match current_path {
            None => None,
            Some(current_path) if current_path != self.storage_path => {
                return Ok(DiskState::Changed);
            }
            Some(current_path) => current_fingerprint(&current_path)?,
        };
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

    /// Executes a vertical caret movement using a caller-owned shaping
    /// provider. Source, selection, preferred-X and history remain owned by
    /// the editor; only the layout used for this command is shaped.
    pub fn move_vertical_with_shaper<S: ShapingProvider>(
        &mut self,
        up: bool,
        extend: bool,
        config: LayoutConfig,
        shaper: &S,
    ) -> Result<CommandResult, StorageError> {
        self.editor
            .move_vertical_with_shaper(up, extend, config, shaper)
            .map_err(StorageError::Editor)
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
        atomic_replace(&self.storage_path, &bytes)?;
        let metadata = fs::metadata(&self.storage_path)
            .map_err(|source| StorageError::io("stat", &self.storage_path, source))?;
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
        let storage_path = canonical_storage_path(&self.path)?.ok_or_else(|| {
            StorageError::io(
                "canonicalize",
                &self.path,
                io::Error::from(io::ErrorKind::NotFound),
            )
        })?;
        let (source, bom, fingerprint) = read_document(&storage_path)?;
        self.editor
            .reset_source(source)
            .map_err(StorageError::Editor)?;
        self.storage_path = storage_path;
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

    /// Gives an integration layer temporary mutable access to the canonical
    /// document while retaining the unified session as the ownership boundary.
    pub fn document_mut(&mut self) -> &mut DocumentSession {
        &mut self.document
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

    /// Returns an owned source-backed projection from the same editor session
    /// that owns source, revision, selection and history.
    pub fn inline_projection(&mut self) -> Result<Projection, StorageError> {
        self.document.inline_projection()
    }

    pub fn inline_projection_for_visual_state(&self) -> Result<Projection, StorageError> {
        self.document.inline_projection_for_visual_state()
    }

    pub fn inline_layout(&mut self, config: LayoutConfig) -> Result<BlockView, StorageError> {
        self.document.inline_layout(config)
    }

    pub fn composition_projection(&mut self) -> Result<Projection, StorageError> {
        self.document.composition_projection()
    }

    #[must_use]
    pub fn block_count(&self) -> usize {
        self.document.block_count()
    }

    #[must_use]
    pub fn block_metadata(&self, index: usize) -> Option<(TextRange, u8)> {
        self.document.block_metadata(index)
    }

    pub fn block_projection(&mut self, index: usize) -> Result<BlockProjection, StorageError> {
        self.document.block_projection(index)
    }

    pub fn block_projection_for_visual_state(
        &self,
        index: usize,
    ) -> Result<BlockProjection, StorageError> {
        self.document.block_projection_for_visual_state(index)
    }

    pub fn block_layout(
        &mut self,
        index: usize,
        config: LayoutConfig,
    ) -> Result<BlockView, StorageError> {
        self.document.block_layout(index, config)
    }

    pub fn block_layout_with_shaper<S: ShapingProvider>(
        &mut self,
        index: usize,
        config: LayoutConfig,
        shaper: &S,
    ) -> Result<BlockView, StorageError> {
        self.document
            .block_layout_with_shaper(index, config, shaper)
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

    /// Executes a vertical caret movement through the unified file-backed
    /// session using a caller-owned shaped layout provider.
    pub fn move_vertical_with_shaper<S: ShapingProvider>(
        &mut self,
        up: bool,
        extend: bool,
        config: LayoutConfig,
        shaper: &S,
    ) -> Result<CommandResult, StorageError> {
        self.document
            .move_vertical_with_shaper(up, extend, config, shaper)
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

    /// Measures the current visible block window without creating a second
    /// editor handle outside the unified product session.
    pub fn visible_blocks(
        &mut self,
        viewport: ViewportSpan,
    ) -> Result<ViewportSnapshot, StorageError> {
        self.document.visible_blocks(viewport)
    }

    pub fn visible_blocks_with_shaper<S: ShapingProvider>(
        &mut self,
        viewport: ViewportSpan,
        shaper: &S,
    ) -> Result<ViewportSnapshot, StorageError> {
        self.document.visible_blocks_with_shaper(viewport, shaper)
    }

    /// Resolves the focus caret through the file-backed session's shaped
    /// viewport state. The returned target is document-space and revision
    /// bound; it never mutates source or selection.
    pub fn caret_scroll_request_with_shaper<S: ShapingProvider>(
        &mut self,
        viewport: ViewportSpan,
        margin: f32,
        shaper: &S,
    ) -> Result<CaretScrollRequest, StorageError> {
        self.document
            .caret_scroll_request_with_shaper(viewport, margin, shaper)
    }

    pub fn set_viewport_config(&mut self, config: ViewportConfig) -> Result<(), StorageError> {
        self.document.set_viewport_config(config)
    }

    #[must_use]
    pub fn viewport_config(&self) -> ViewportConfig {
        self.document.viewport_config()
    }

    /// Writes or clears a recovery snapshot through the unified product
    /// session; no second storage/editor handle is created.
    pub fn write_recovery(&self, store: &RecoveryStore) -> Result<RecoveryOutcome, RecoveryError> {
        self.document.write_recovery(store)
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

fn canonical_storage_path(path: &Path) -> Result<Option<PathBuf>, StorageError> {
    match fs::canonicalize(path) {
        Ok(path) => Ok(Some(path)),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(StorageError::io("canonicalize", path, source)),
    }
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
