use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};

use yu_core::Revision;
use yu_storage::{ClosePrompt, CloseRequest, CloseStateError, DocumentEditorSession, StorageError};
use yu_text::TextSnapshot;

/// Stable identity of one open tab inside a [`Workspace`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TabId(u64);

impl TabId {
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// One tab owns exactly one unified `DocumentEditorSession`.
#[derive(Debug)]
pub struct WorkspaceTab {
    id: TabId,
    session: DocumentEditorSession,
}

impl WorkspaceTab {
    #[must_use]
    pub const fn id(&self) -> TabId {
        self.id
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        self.session.path()
    }

    #[must_use]
    pub fn revision(&self) -> Revision {
        self.session.revision()
    }

    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.session.is_dirty()
    }

    /// Returns a revision-bound snapshot without creating a second source.
    #[must_use]
    pub fn snapshot(&self) -> TextSnapshot {
        self.session.snapshot()
    }

    #[must_use]
    pub const fn session(&self) -> &DocumentEditorSession {
        &self.session
    }

    pub fn session_mut(&mut self) -> &mut DocumentEditorSession {
        &mut self.session
    }
}

/// Result of opening or creating one tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpenTabResult {
    id: TabId,
    reused: bool,
}

impl OpenTabResult {
    #[must_use]
    pub const fn id(self) -> TabId {
        self.id
    }

    #[must_use]
    pub const fn reused(self) -> bool {
        self.reused
    }
}

/// The product-shell decision for a pending dirty/conflicted tab close.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseAction {
    Save,
    Discard,
    Cancel,
}

/// Outcome after resolving a pending close prompt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseResult {
    Closed { id: TabId, action: CloseAction },
    Cancelled { id: TabId },
}

/// The first close request for a tab. Clean tabs are removed immediately;
/// dirty/conflicted tabs remain until [`Workspace::resolve_close`] is called.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkspaceCloseRequest {
    Closed { id: TabId },
    Prompt { id: TabId, prompt: ClosePrompt },
    AlreadyClosed { id: TabId },
}

/// Errors raised by workspace/tab lifecycle operations.
#[derive(Debug)]
pub enum WorkspaceError {
    Storage(StorageError),
    CloseState(CloseStateError),
    TabNotFound(TabId),
    TabIdOverflow,
}

impl fmt::Display for WorkspaceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Storage(error) => error.fmt(formatter),
            Self::CloseState(error) => {
                write!(formatter, "invalid workspace close state: {error:?}")
            }
            Self::TabNotFound(id) => write!(formatter, "workspace tab {:?} was not found", id),
            Self::TabIdOverflow => formatter.write_str("workspace tab id overflowed"),
        }
    }
}

impl Error for WorkspaceError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Storage(error) => Some(error),
            Self::CloseState(_) | Self::TabNotFound(_) | Self::TabIdOverflow => None,
        }
    }
}

impl From<StorageError> for WorkspaceError {
    fn from(error: StorageError) -> Self {
        Self::Storage(error)
    }
}

impl From<CloseStateError> for WorkspaceError {
    fn from(error: CloseStateError) -> Self {
        Self::CloseState(error)
    }
}

/// Headless workspace ownership and tab lifecycle.
///
/// The workspace stores sessions, not another source model. A native window
/// or future visual surface can borrow the active tab and publish revision-
/// bound data without taking ownership of Markdown text.
#[derive(Debug)]
pub struct Workspace {
    next_tab_id: u64,
    active: Option<TabId>,
    tabs: Vec<WorkspaceTab>,
}

impl Default for Workspace {
    fn default() -> Self {
        Self::new()
    }
}

impl Workspace {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            next_tab_id: 1,
            active: None,
            tabs: Vec::new(),
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.tabs.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    #[must_use]
    pub const fn active_tab_id(&self) -> Option<TabId> {
        self.active
    }

    #[must_use]
    pub fn active_tab(&self) -> Option<&WorkspaceTab> {
        self.active.and_then(|id| self.tab_by_id(id))
    }

    pub fn active_tab_mut(&mut self) -> Option<&mut WorkspaceTab> {
        let id = self.active?;
        self.tab_by_id_mut(id)
    }

    pub fn tab(&self, id: TabId) -> Result<&WorkspaceTab, WorkspaceError> {
        self.tab_by_id(id).ok_or(WorkspaceError::TabNotFound(id))
    }

    pub fn tab_mut(&mut self, id: TabId) -> Result<&mut WorkspaceTab, WorkspaceError> {
        self.tab_by_id_mut(id)
            .ok_or(WorkspaceError::TabNotFound(id))
    }

    pub fn tab_ids(&self) -> impl Iterator<Item = TabId> + '_ {
        self.tabs.iter().map(WorkspaceTab::id)
    }

    pub fn set_active(&mut self, id: TabId) -> Result<(), WorkspaceError> {
        self.tab(id)?;
        self.active = Some(id);
        Ok(())
    }

    /// Opens an existing path, reusing an already open exact path.
    pub fn open_path(&mut self, path: impl Into<PathBuf>) -> Result<OpenTabResult, WorkspaceError> {
        let path = path.into();
        if let Some(tab) = self.tabs.iter().find(|tab| tab.path() == path) {
            let id = tab.id();
            self.active = Some(id);
            return Ok(OpenTabResult { id, reused: true });
        }
        let session = DocumentEditorSession::open(path)?;
        self.insert_session(session)
    }

    /// Creates a new unsaved session at `path` without duplicating source.
    pub fn new_document(
        &mut self,
        path: impl Into<PathBuf>,
        source: impl Into<String>,
    ) -> Result<OpenTabResult, WorkspaceError> {
        let session = DocumentEditorSession::new(path, source);
        self.insert_session(session)
    }

    /// Requests a close. Clean tabs are removed immediately; dirty/conflicted
    /// tabs return their prompt and remain addressable by the same `TabId`.
    pub fn request_close(&mut self, id: TabId) -> Result<WorkspaceCloseRequest, WorkspaceError> {
        let request = self.tab_mut(id)?.session_mut().close_request()?;
        match request {
            CloseRequest::CloseNow => {
                self.remove_tab(id)?;
                Ok(WorkspaceCloseRequest::Closed { id })
            }
            CloseRequest::Prompt(prompt) => Ok(WorkspaceCloseRequest::Prompt { id, prompt }),
            CloseRequest::AlreadyClosed => {
                self.remove_tab(id)?;
                Ok(WorkspaceCloseRequest::AlreadyClosed { id })
            }
        }
    }

    /// Resolves a prompt without removing a tab before save/discard succeeds.
    pub fn resolve_close(
        &mut self,
        id: TabId,
        action: CloseAction,
    ) -> Result<CloseResult, WorkspaceError> {
        match action {
            CloseAction::Cancel => {
                self.tab_mut(id)?.session_mut().cancel_close()?;
                Ok(CloseResult::Cancelled { id })
            }
            CloseAction::Save => {
                self.tab_mut(id)?.session_mut().save_close()?;
                self.remove_tab(id)?;
                Ok(CloseResult::Closed { id, action })
            }
            CloseAction::Discard => {
                self.tab_mut(id)?.session_mut().discard_close()?;
                self.remove_tab(id)?;
                Ok(CloseResult::Closed { id, action })
            }
        }
    }

    fn insert_session(
        &mut self,
        session: DocumentEditorSession,
    ) -> Result<OpenTabResult, WorkspaceError> {
        let id = TabId(self.next_tab_id);
        self.next_tab_id = self
            .next_tab_id
            .checked_add(1)
            .ok_or(WorkspaceError::TabIdOverflow)?;
        self.tabs.push(WorkspaceTab { id, session });
        self.active = Some(id);
        Ok(OpenTabResult { id, reused: false })
    }

    fn remove_tab(&mut self, id: TabId) -> Result<WorkspaceTab, WorkspaceError> {
        let index = self
            .tabs
            .iter()
            .position(|tab| tab.id() == id)
            .ok_or(WorkspaceError::TabNotFound(id))?;
        let removed = self.tabs.remove(index);
        if self.active == Some(id) {
            self.active = self
                .tabs
                .get(index)
                .or_else(|| self.tabs.last())
                .map(WorkspaceTab::id);
        }
        Ok(removed)
    }

    fn tab_by_id(&self, id: TabId) -> Option<&WorkspaceTab> {
        self.tabs.iter().find(|tab| tab.id() == id)
    }

    fn tab_by_id_mut(&mut self, id: TabId) -> Option<&mut WorkspaceTab> {
        self.tabs.iter_mut().find(|tab| tab.id() == id)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use yu_editor::{CaretAffinity, EditorCommand, EditorSelection};
    use yu_storage::{ClosePrompt, ExternalFileState, StorageError};

    use super::*;

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TestPath(PathBuf);

    impl TestPath {
        fn new(label: &str) -> Self {
            let id = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "yu-workspace-{label}-{}-{id}.md",
                std::process::id()
            ));
            let _ = fs::remove_file(&path);
            Self(path)
        }

        fn as_path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestPath {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }

    #[test]
    fn opening_an_existing_path_reuses_one_session_and_activates_it() {
        let first = TestPath::new("first");
        let second = TestPath::new("second");
        fs::write(first.as_path(), b"one").expect("write first");
        fs::write(second.as_path(), b"two").expect("write second");
        let mut workspace = Workspace::new();

        let first_open = workspace.open_path(first.as_path()).expect("open first");
        let second_open = workspace.open_path(second.as_path()).expect("open second");
        assert!(!first_open.reused());
        assert!(!second_open.reused());
        assert_eq!(workspace.len(), 2);
        assert_eq!(workspace.active_tab_id(), Some(second_open.id()));

        workspace
            .set_active(first_open.id())
            .expect("activate first");
        let reused = workspace.open_path(first.as_path()).expect("reuse first");
        assert!(reused.reused());
        assert_eq!(reused.id(), first_open.id());
        assert_eq!(workspace.len(), 2);
        assert_eq!(workspace.active_tab_id(), Some(first_open.id()));
    }

    #[test]
    fn clean_close_removes_tab_and_rehomes_active_tab() {
        let first = TestPath::new("clean-first");
        let second = TestPath::new("clean-second");
        fs::write(first.as_path(), b"one").expect("write first");
        fs::write(second.as_path(), b"two").expect("write second");
        let mut workspace = Workspace::new();
        let first_open = workspace.open_path(first.as_path()).expect("open first");
        let second_open = workspace.open_path(second.as_path()).expect("open second");

        assert_eq!(
            workspace
                .request_close(second_open.id())
                .expect("close second"),
            WorkspaceCloseRequest::Closed {
                id: second_open.id()
            }
        );
        assert_eq!(workspace.len(), 1);
        assert_eq!(workspace.active_tab_id(), Some(first_open.id()));
        assert!(matches!(
            workspace.tab(second_open.id()),
            Err(WorkspaceError::TabNotFound(id)) if id == second_open.id()
        ));
    }

    #[test]
    fn dirty_close_can_cancel_then_save_without_removing_the_tab_early() {
        let path = TestPath::new("dirty-close");
        fs::write(path.as_path(), b"source").expect("write fixture");
        let mut workspace = Workspace::new();
        let opened = workspace.open_path(path.as_path()).expect("open fixture");
        {
            // 新文档的光标落在文首；这里断言的是「在末尾追加」，前提要自己建立。
            let session = workspace.tab_mut(opened.id()).expect("tab").session_mut();
            let snapshot = session.snapshot();
            let caret =
                EditorSelection::cursor(&snapshot, snapshot.len_bytes(), CaretAffinity::Downstream)
                    .expect("caret at end");
            session.set_selection(caret).expect("caret at end");
        }
        workspace
            .tab_mut(opened.id())
            .expect("tab")
            .session_mut()
            .execute(EditorCommand::insert_text(" edit"))
            .expect("edit");

        assert_eq!(
            workspace.request_close(opened.id()).expect("request close"),
            WorkspaceCloseRequest::Prompt {
                id: opened.id(),
                prompt: ClosePrompt::SaveChanges,
            }
        );
        assert_eq!(workspace.len(), 1);
        assert_eq!(
            workspace
                .resolve_close(opened.id(), CloseAction::Cancel)
                .expect("cancel close"),
            CloseResult::Cancelled { id: opened.id() }
        );
        assert_eq!(workspace.len(), 1);

        workspace.request_close(opened.id()).expect("request again");
        assert_eq!(
            workspace
                .resolve_close(opened.id(), CloseAction::Save)
                .expect("save close"),
            CloseResult::Closed {
                id: opened.id(),
                action: CloseAction::Save,
            }
        );
        assert!(workspace.is_empty());
        assert_eq!(
            fs::read(path.as_path()).expect("read saved source"),
            b"source edit"
        );
    }

    #[test]
    fn external_conflict_blocks_save_but_discard_closes_without_overwrite() {
        let path = TestPath::new("conflict-close");
        fs::write(path.as_path(), b"source").expect("write fixture");
        let mut workspace = Workspace::new();
        let opened = workspace.open_path(path.as_path()).expect("open fixture");
        workspace
            .tab_mut(opened.id())
            .expect("tab")
            .session_mut()
            .execute(EditorCommand::insert_text(" local"))
            .expect("edit");
        fs::write(path.as_path(), b"external").expect("external replacement");

        assert_eq!(
            workspace.request_close(opened.id()).expect("request close"),
            WorkspaceCloseRequest::Prompt {
                id: opened.id(),
                prompt: ClosePrompt::ExternalChange {
                    state: ExternalFileState::Changed,
                },
            }
        );
        assert!(matches!(
            workspace.resolve_close(opened.id(), CloseAction::Save),
            Err(WorkspaceError::Storage(StorageError::ExternalChange { .. }))
        ));
        assert_eq!(workspace.len(), 1);
        workspace
            .resolve_close(opened.id(), CloseAction::Discard)
            .expect("discard conflict");
        assert!(workspace.is_empty());
        assert_eq!(
            fs::read(path.as_path()).expect("read external source"),
            b"external"
        );
    }
}
