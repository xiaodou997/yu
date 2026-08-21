use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(unix)]
use std::os::unix::fs::{PermissionsExt, symlink};

use yu_core::{Revision, TextRange, Utf16Offset, Utf16Range};
use yu_editor::{CaretAffinity, EditorCommand, EditorSelection, ViewportSpan};
use yu_storage::{
    ClosePrompt, CloseRequest, CloseState, CloseStateMachine, CloseTransition, DiskState,
    DocumentEditorSession, DocumentSession, ExternalFileState, RecoveryError, RecoveryOutcome,
    RecoveryStore, SaveOutcome, StorageError, Utf8Bom,
};

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

/// 把光标移到文末。
///
/// 新文档的光标落在文首——打开文件应该看到开头。需要「在末尾追加」这个前提的
/// 用例必须自己建立它，而不是依赖一个隐含默认。
fn place_caret_at_end(snapshot: &yu_text::TextSnapshot) -> EditorSelection {
    EditorSelection::cursor(snapshot, snapshot.len_bytes(), CaretAffinity::Downstream)
        .expect("the end of a snapshot is a valid caret")
}

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

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new(label: &str) -> Self {
        let id = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("yu-storage-{label}-{}-{id}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("create test directory");
        Self(path)
    }

    fn as_path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
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
fn unified_session_exposes_viewport_without_a_second_editor_handle() {
    let path = TestPath::new("viewport");
    fs::write(path.as_path(), b"one\n\ntwo\n\nthree\n\nfour").expect("write fixture");
    let mut session = DocumentEditorSession::open(path.as_path()).expect("open fixture");

    let first = session
        .visible_blocks(ViewportSpan::new(0.0, 1.0))
        .expect("first viewport query should succeed");
    assert!(!first.blocks().is_empty());
    let entry_count = session.document().editor().viewport_stats().entries();
    assert_eq!(
        entry_count,
        session.document().editor().markdown().blocks().len()
    );

    session
        .execute(EditorCommand::insert_text("!"))
        .expect("edit should succeed");
    let second = session
        .visible_blocks(ViewportSpan::new(0.0, 1.0))
        .expect("mapped viewport query should succeed");
    assert_eq!(second.revision(), session.revision());
    assert_eq!(
        session.document().editor().viewport_stats().entries(),
        entry_count
    );
    assert!(session.document().editor().viewport_stats().remapped() > 0);
}

#[test]
fn recovery_round_trip_preserves_source_revision_and_bom() {
    let target = TestPath::new("recovery-round-trip");
    fs::write(target.as_path(), bom_bytes("source")).expect("write fixture");
    let root = TestDirectory::new("recovery-root");
    let store = RecoveryStore::new(root.as_path());
    let mut session = DocumentEditorSession::open(target.as_path()).expect("open fixture");
    let caret = place_caret_at_end(&session.snapshot());
    session.set_selection(caret).expect("caret at end");
    session
        .execute(EditorCommand::insert_text(" + 羽🙂"))
        .expect("edit should succeed");
    let revision = session.revision();

    let outcome = session
        .write_recovery(&store)
        .expect("recovery write should succeed");
    let recovery_path = match outcome {
        RecoveryOutcome::Written {
            path,
            revision: written_revision,
            bytes_written,
        } => {
            assert_eq!(written_revision, revision);
            assert!(bytes_written > session.snapshot().len_bytes().get() as usize);
            path
        }
        RecoveryOutcome::Cleared { .. } => panic!("dirty session must write recovery"),
    };
    assert!(recovery_path.is_file());
    assert_eq!(
        fs::read(target.as_path()).expect("read target"),
        bom_bytes("source")
    );

    let record = store
        .read(target.as_path())
        .expect("read recovery")
        .expect("record should exist");
    assert_eq!(record.target_path(), target.as_path());
    assert_eq!(record.source(), "source + 羽🙂");
    assert_eq!(record.revision(), revision);
    assert_eq!(record.saved_revision(), Revision::INITIAL);
    assert_eq!(record.bom(), Utf8Bom::Present);
    assert!(
        root.as_path()
            .read_dir()
            .expect("read recovery root")
            .filter_map(Result::ok)
            .all(|entry| !entry.file_name().to_string_lossy().contains(".tmp-"))
    );

    store.clear(target.as_path()).expect("clear recovery");
    assert!(
        store
            .read(target.as_path())
            .expect("read cleared recovery")
            .is_none()
    );
}

#[test]
fn clean_recovery_clears_stale_record_and_corruption_is_rejected() {
    let target = TestPath::new("recovery-corrupt");
    fs::write(target.as_path(), b"source").expect("write fixture");
    let root = TestDirectory::new("recovery-corrupt-root");
    let store = RecoveryStore::new(root.as_path());
    let mut dirty = DocumentSession::open(target.as_path()).expect("open fixture");
    dirty
        .execute(EditorCommand::insert_text(" edit"))
        .expect("edit should succeed");
    store.write(&dirty).expect("write recovery");

    let recovery_path = store.path_for(target.as_path()).expect("recovery path");
    let mut bytes = fs::read(&recovery_path).expect("read recovery bytes");
    let checksum_byte = bytes.len().checked_sub(1).expect("checksum exists");
    bytes[checksum_byte] ^= 1;
    fs::write(&recovery_path, bytes).expect("corrupt recovery");
    assert!(matches!(
        store.read(target.as_path()),
        Err(RecoveryError::InvalidFormat {
            reason: "checksum mismatch",
            ..
        })
    ));

    store.write(&dirty).expect("rewrite recovery");
    let clean = DocumentSession::open(target.as_path()).expect("open clean session");
    assert_eq!(
        clean
            .write_recovery(&store)
            .expect("clean session should clear recovery"),
        RecoveryOutcome::Cleared {
            path: recovery_path
        }
    );
    assert!(
        store
            .read(target.as_path())
            .expect("read cleared recovery")
            .is_none()
    );
}

#[test]
fn save_is_atomic_and_reuses_bom_without_changing_editor_revision() {
    let path = TestPath::new("save");
    fs::write(path.as_path(), bom_bytes("hello")).expect("write fixture");
    let mut session = DocumentSession::open(path.as_path()).expect("open fixture");
    let initial = session.revision();
    let caret = place_caret_at_end(&session.editor().snapshot());
    session.set_selection(caret).expect("caret at end");

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

#[cfg(unix)]
#[test]
fn symlink_save_updates_target_and_preserves_link_and_permissions() {
    let target = TestPath::new("symlink-target");
    let link = TestPath::new("symlink-link");
    fs::write(target.as_path(), b"source").expect("write symlink target");
    fs::set_permissions(target.as_path(), fs::Permissions::from_mode(0o640))
        .expect("set target permissions");
    symlink(target.as_path(), link.as_path()).expect("create symlink");

    let mut session = DocumentSession::open(link.as_path()).expect("open symlink");
    assert_eq!(session.path(), link.as_path());
    assert_eq!(
        session.storage_path(),
        fs::canonicalize(target.as_path()).expect("canonical target")
    );
    let caret = place_caret_at_end(&session.editor().snapshot());
    session.set_selection(caret).expect("caret at end");
    session
        .execute(EditorCommand::insert_text(" edit"))
        .expect("edit symlink target");
    session.save().expect("save through symlink");

    let link_metadata = fs::symlink_metadata(link.as_path()).expect("stat symlink");
    assert!(link_metadata.file_type().is_symlink());
    assert_eq!(
        fs::read_link(link.as_path()).expect("read symlink"),
        target.as_path()
    );
    assert_eq!(
        fs::read(target.as_path()).expect("read target"),
        b"source edit"
    );
    assert_eq!(
        fs::metadata(target.as_path())
            .expect("stat target")
            .permissions()
            .mode()
            & 0o777,
        0o640
    );
}

#[cfg(unix)]
#[test]
fn symlink_retarget_is_an_external_change() {
    let first_target = TestPath::new("symlink-retarget-first");
    let second_target = TestPath::new("symlink-retarget-second");
    let link = TestPath::new("symlink-retarget-link");
    fs::write(first_target.as_path(), b"first").expect("write first target");
    fs::write(second_target.as_path(), b"second").expect("write second target");
    symlink(first_target.as_path(), link.as_path()).expect("create symlink");

    let mut session = DocumentSession::open(link.as_path()).expect("open symlink");
    session
        .execute(EditorCommand::insert_text(" local"))
        .expect("edit symlink target");
    fs::remove_file(link.as_path()).expect("remove old symlink");
    symlink(second_target.as_path(), link.as_path()).expect("retarget symlink");

    assert_eq!(
        session.disk_state().expect("disk state"),
        DiskState::Changed
    );
    assert!(matches!(
        session.save().expect_err("retarget must block save"),
        StorageError::ExternalChange {
            state: ExternalFileState::Changed,
            ..
        }
    ));
    assert_eq!(
        fs::read(first_target.as_path()).expect("read first target"),
        b"first"
    );
    assert_eq!(
        fs::read(second_target.as_path()).expect("read second target"),
        b"second"
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

    let caret = place_caret_at_end(&session.snapshot());
    session.set_selection(caret).expect("caret at end");
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
    assert_eq!(session.composition_generation(), 1);
    assert_eq!(session.revision(), Revision::new(1));
    assert!(session.composition().is_some());
    session
        .update_composition("日本語", Utf16Range::empty(Utf16Offset::new(3)))
        .expect("preedit update should remain transient");
    assert_eq!(session.composition_generation(), 2);
    assert_eq!(session.snapshot().as_str(), "输入: 🙂");
    session
        .commit_composition("日本語")
        .expect("commit should create one transaction");
    assert_eq!(session.composition_generation(), 3);
    assert_eq!(session.revision(), Revision::new(2));
    assert_eq!(session.snapshot().as_str(), "输入: 🙂日本語");
    assert!(session.is_dirty());
}

#[test]
fn unified_session_composition_generation_rejects_late_native_state() {
    let path = TestPath::new("unified-generation");
    fs::write(path.as_path(), "source").expect("write fixture");
    let mut session = DocumentEditorSession::open(path.as_path()).expect("open fixture");
    let snapshot = session.snapshot();
    session
        .begin_composition(
            TextRange::empty(snapshot.len_bytes()),
            "にほん",
            Utf16Range::empty(Utf16Offset::new(3)),
        )
        .expect("begin composition");
    let first_generation = session.composition_generation();
    session
        .update_composition("日本", Utf16Range::empty(Utf16Offset::new(2)))
        .expect("update composition");
    assert_ne!(session.composition_generation(), first_generation);
    assert!(session.composition().is_some());
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
