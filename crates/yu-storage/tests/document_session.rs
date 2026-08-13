use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use yu_core::{Revision, TextRange, Utf16Offset, Utf16Range};
use yu_editor::EditorCommand;
use yu_storage::{
    ClosePrompt, CloseRequest, CloseState, CloseStateMachine, CloseTransition, DiskState,
    DocumentEditorSession, DocumentSession, ExternalFileState, SaveOutcome, StorageError, Utf8Bom,
};

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

struct TestPath(PathBuf);

impl TestPath {
    fn new(label: &str) -> Self {
        let id = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("yu-storage-{label}-{}-{id}.md", std::process::id()));
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

fn empty_utf16() -> Utf16Range {
    Utf16Range::empty(Utf16Offset::ZERO)
}

fn bom_bytes(source: &str) -> Vec<u8> {
    [b"\xEF\xBB\xBF".as_slice(), source.as_bytes()].concat()
}

#[test]
fn open_preserves_bom_metadata_without_polluting_source_coordinates() {
    let path = TestPath::new("bom");
    fs::write(path.as_path(), bom_bytes("# 羽\n")).expect("write fixture");

    let session = DocumentSession::open(path.as_path()).expect("open UTF-8 BOM document");
    assert_eq!(session.bom(), Utf8Bom::Present);
    assert_eq!(session.editor().snapshot().as_str(), "# 羽\n");
    assert_eq!(
        session.editor().snapshot().len_bytes().get(),
        "# 羽\n".len() as u64
    );
    assert_eq!(session.revision(), Revision::INITIAL);
    assert!(!session.is_dirty());
    assert_eq!(
        session.disk_state().expect("disk state"),
        DiskState::Unchanged
    );
}

#[test]
fn save_is_atomic_and_reuses_bom_without_changing_editor_revision() {
    let path = TestPath::new("save");
    fs::write(path.as_path(), bom_bytes("hello")).expect("write fixture");
    let mut session = DocumentSession::open(path.as_path()).expect("open fixture");
    let initial = session.revision();

    session
        .execute(EditorCommand::insert_text("🙂"))
        .expect("insert should succeed");
    assert!(session.is_dirty());
    let revision = session.revision();
    let outcome = session.save().expect("save should succeed");
    assert_eq!(
        outcome,
        SaveOutcome::Saved {
            revision,
            bytes_written: 3 + "hello🙂".len(),
        }
    );
    assert_eq!(initial, Revision::INITIAL);
    assert_eq!(session.saved_revision(), revision);
    assert!(!session.is_dirty());
    assert_eq!(
        fs::read(path.as_path()).expect("read saved file"),
        bom_bytes("hello🙂")
    );
    let temp_prefix = format!(
        ".{}.yu-save-",
        path.as_path()
            .file_name()
            .expect("fixture filename")
            .to_string_lossy()
    );
    assert!(
        fs::read_dir(path.as_path().parent().expect("temp path parent"))
            .expect("read temp directory")
            .filter_map(Result::ok)
            .all(|entry| !entry
                .file_name()
                .to_string_lossy()
                .starts_with(&temp_prefix)),
        "atomic save must not leave a sibling temporary file"
    );
    assert_eq!(
        session.save().expect("clean save should be a no-op"),
        SaveOutcome::Unchanged { revision }
    );
}

#[test]
fn external_change_is_detected_and_never_overwritten() {
    let path = TestPath::new("conflict");
    fs::write(path.as_path(), b"original").expect("write fixture");
    let mut session = DocumentSession::open(path.as_path()).expect("open fixture");
    session
        .execute(EditorCommand::insert_text(" local"))
        .expect("local edit should succeed");
    fs::write(path.as_path(), b"external").expect("simulate external edit");

    assert_eq!(
        session.disk_state().expect("disk state"),
        DiskState::Changed
    );
    let error = session.save().expect_err("external edit must block save");
    assert!(matches!(
        error,
        StorageError::ExternalChange {
            state: ExternalFileState::Changed,
            ..
        }
    ));
    assert_eq!(
        fs::read(path.as_path()).expect("read external file"),
        b"external"
    );
    assert!(session.is_dirty());
}

#[test]
fn missing_file_is_a_conflict_after_open_but_new_session_can_save() {
    let path = TestPath::new("missing");
    fs::write(path.as_path(), b"source").expect("write fixture");
    let mut opened = DocumentSession::open(path.as_path()).expect("open fixture");
    fs::remove_file(path.as_path()).expect("remove fixture");
    opened
        .execute(EditorCommand::insert_text(" edit"))
        .expect("edit should succeed");
    assert_eq!(opened.disk_state().expect("disk state"), DiskState::Missing);
    assert!(matches!(
        opened.save().expect_err("missing target must block save"),
        StorageError::ExternalChange {
            state: ExternalFileState::Missing,
            ..
        }
    ));

    let mut new_session = DocumentSession::new(path.as_path(), "new source");
    assert!(new_session.is_dirty());
    assert_eq!(
        new_session.disk_state().expect("new disk state"),
        DiskState::Missing
    );
    let outcome = new_session.save().expect("new path should save");
    assert!(matches!(outcome, SaveOutcome::Saved { .. }));
    assert_eq!(
        fs::read(path.as_path()).expect("read new file"),
        b"new source"
    );
    assert!(!new_session.is_dirty());
}

#[test]
fn reload_requires_clean_revision_and_accepts_external_file_afterward() {
    let path = TestPath::new("reload");
    fs::write(path.as_path(), b"one").expect("write fixture");
    let mut session = DocumentSession::open(path.as_path()).expect("open fixture");
    session
        .execute(EditorCommand::insert_text(" local"))
        .expect("local edit should succeed");
    assert!(matches!(
        session.reload().expect_err("dirty reload must be blocked"),
        StorageError::UnsavedChanges { .. }
    ));

    session
        .execute(EditorCommand::undo())
        .expect("undo local edit");
    assert!(
        session.is_dirty(),
        "revision identity remains dirty after undo"
    );
    assert!(matches!(
        session
            .reload()
            .expect_err("revision-dirty reload must be blocked"),
        StorageError::UnsavedChanges { .. }
    ));

    let path = TestPath::new("reload-clean");
    fs::write(path.as_path(), b"one").expect("write fixture");
    let mut clean = DocumentSession::open(path.as_path()).expect("open fixture");
    fs::write(path.as_path(), b"two").expect("simulate external replacement");
    let outcome = clean
        .reload()
        .expect("clean reload should accept disk state");
    assert_eq!(outcome.revision, Revision::INITIAL);
    assert_eq!(outcome.bom, Utf8Bom::Absent);
    assert_eq!(clean.editor().snapshot().as_str(), "two");
    assert!(!clean.is_dirty());
}

#[test]
fn composition_is_transient_until_commit() {
    let path = TestPath::new("composition");
    fs::write(path.as_path(), "输入: ".as_bytes()).expect("write fixture");
    let mut session = DocumentSession::open(path.as_path()).expect("open fixture");
    let source = session.editor().snapshot();
    let end = source.len_bytes();
    session
        .begin_composition(TextRange::empty(end), "にほんご", empty_utf16())
        .expect("begin composition");
    session
        .update_composition("日本語", Utf16Range::empty(Utf16Offset::new(3)))
        .expect("update composition");
    assert!(!session.is_dirty());
    assert_eq!(session.revision(), Revision::INITIAL);
    session
        .commit_composition("日本語")
        .expect("commit composition");
    assert!(session.is_dirty());
    assert_eq!(session.editor().snapshot().as_str(), "输入: 日本語");
}

#[test]
fn invalid_utf8_is_rejected_before_editor_creation() {
    let path = TestPath::new("invalid-utf8");
    fs::write(path.as_path(), [0xFF, 0xFE]).expect("write invalid fixture");
    assert!(matches!(
        DocumentSession::open(path.as_path()).expect_err("invalid UTF-8 must fail"),
        StorageError::InvalidUtf8 { .. }
    ));
}

#[test]
fn close_request_uses_session_dirty_and_external_conflict_state() {
    let path = TestPath::new("close");
    fs::write(path.as_path(), b"source").expect("write fixture");
    let session = DocumentSession::open(path.as_path()).expect("open fixture");
    let mut close = CloseStateMachine::new();

    assert_eq!(
        session
            .close_request(&mut close)
            .expect("clean close request"),
        CloseRequest::CloseNow
    );
    assert_eq!(close.state(), CloseState::Closed);

    let mut session = DocumentSession::open(path.as_path()).expect("reopen fixture");
    session
        .execute(EditorCommand::insert_text(" local"))
        .expect("local edit should succeed");
    let mut close = CloseStateMachine::new();
    assert_eq!(
        session
            .close_request(&mut close)
            .expect("dirty close request"),
        CloseRequest::Prompt(ClosePrompt::SaveChanges)
    );
    assert_eq!(close.cancel(), Ok(CloseTransition::Cancelled));

    fs::write(path.as_path(), b"external").expect("simulate external replacement");
    assert_eq!(
        session
            .close_request(&mut close)
            .expect("conflict close request"),
        CloseRequest::Prompt(ClosePrompt::ExternalChange {
            state: ExternalFileState::Changed,
        })
    );
    assert_eq!(close.discard(), Ok(CloseTransition::Closed));
}

#[test]
fn unified_session_routes_edit_and_ime_through_one_source_revision() {
    let path = TestPath::new("unified");
    fs::write(path.as_path(), "输入: ").expect("write fixture");
    let mut session = DocumentEditorSession::open(path.as_path()).expect("open fixture");
    assert_eq!(session.revision(), Revision::INITIAL);
    assert_eq!(session.snapshot().as_str(), "输入: ");

    session
        .execute(EditorCommand::insert_text("🙂"))
        .expect("command should use canonical editor");
    assert_eq!(session.revision(), Revision::new(1));
    assert_eq!(session.snapshot().as_str(), "输入: 🙂");
    assert!(session.is_dirty());

    let end = session.snapshot().len_bytes();
    session
        .begin_composition(
            TextRange::empty(end),
            "にほんご",
            Utf16Range::empty(Utf16Offset::new(4)),
        )
        .expect("composition should share the editor");
    assert_eq!(session.revision(), Revision::new(1));
    assert!(session.composition().is_some());
    session
        .update_composition("日本語", Utf16Range::empty(Utf16Offset::new(3)))
        .expect("preedit update should remain transient");
    assert_eq!(session.snapshot().as_str(), "输入: 🙂");
    session
        .commit_composition("日本語")
        .expect("commit should create one transaction");
    assert_eq!(session.revision(), Revision::new(2));
    assert_eq!(session.snapshot().as_str(), "输入: 🙂日本語");
    assert!(session.is_dirty());
}

#[test]
fn unified_session_close_uses_same_dirty_and_external_state() {
    let path = TestPath::new("unified-close");
    fs::write(path.as_path(), "source").expect("write fixture");
    let mut session = DocumentEditorSession::open(path.as_path()).expect("open fixture");
    session
        .execute(EditorCommand::insert_text(" local"))
        .expect("edit should succeed");
    assert_eq!(
        session.close_request().expect("close request"),
        CloseRequest::Prompt(ClosePrompt::SaveChanges)
    );
    assert_eq!(
        session.cancel_close().expect("cancel close"),
        CloseTransition::Cancelled
    );
    fs::write(path.as_path(), "external").expect("simulate external replacement");
    assert_eq!(
        session.close_request().expect("conflict close request"),
        CloseRequest::Prompt(ClosePrompt::ExternalChange {
            state: ExternalFileState::Changed,
        })
    );
    assert!(matches!(
        session.save_close().expect_err("external save must fail"),
        StorageError::ExternalChange { .. }
    ));
    assert!(matches!(
        session.close_state(),
        yu_storage::CloseState::Prompting(ClosePrompt::ExternalChange { .. })
    ));
    assert_eq!(
        session
            .discard_close()
            .expect("discard should close the unified session"),
        CloseTransition::Closed
    );
}
