use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use yu_editor::EditorCommand;
use yu_storage::DocumentSession;

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "yu-image-save-as-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("old/assets")).expect("old directory");
        fs::create_dir_all(root.join("new")).expect("new directory");
        Self(root)
    }
    fn old(&self) -> PathBuf {
        self.0.join("old/note.md")
    }
    fn new_path(&self) -> PathBuf {
        self.0.join("new/copy.md")
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn image_save_as_preserves_source_bytes_resources_and_history_in_both_modes() {
    for source_mode in [false, true] {
        let f = Fixture::new();
        let body = "\u{feff}# 文档\r\n\r\n![中文](assets/%E5%9B%BE%20%25.png)\r\n\r\n![引用][image]\r\n\r\n[image]: assets/%E5%9B%BE%20%25.png\r\n";
        fs::write(f.old(), body).expect("document");
        fs::write(f.0.join("old/assets/图 %.png"), b"unchanged image bytes").expect("image");
        let mut session = DocumentSession::open(f.old()).expect("open");
        session
            .editor_mut()
            .set_source_mode(source_mode)
            .expect("mode");
        session
            .editor_mut()
            .execute(EditorCommand::insert_text("edited "))
            .expect("edit");
        let expected = format!("\u{feff}{}", session.editor().snapshot().as_str());
        let history = session.editor().history_stats();
        session
            .save_as(f.new_path(), false)
            .expect("portable save as");
        assert_eq!(fs::read(f.old()).expect("old source"), body.as_bytes());
        assert_eq!(
            fs::read(f.new_path()).expect("new source"),
            expected.as_bytes()
        );
        assert_eq!(
            fs::read(f.0.join("new/assets/图 %.png")).expect("new image"),
            b"unchanged image bytes"
        );
        assert_eq!(session.editor().history_stats(), history);
        session.editor_mut().undo().expect("undo after Save As");
        assert_eq!(
            session.editor().snapshot().as_str(),
            body.trim_start_matches('\u{feff}')
        );
        session.save().expect("save undo");
    }
}

#[test]
fn image_conflict_never_overwrites_or_publishes_a_new_document() {
    let f = Fixture::new();
    fs::create_dir(f.0.join("new/assets")).expect("destination");
    fs::write(f.old(), "![a](assets/a.png) ![b](assets/b.png)").expect("document");
    fs::write(f.0.join("old/assets/a.png"), b"a").expect("image a");
    fs::write(f.0.join("old/assets/b.png"), b"b").expect("image b");
    fs::write(f.0.join("new/assets/b.png"), b"unrelated").expect("conflict");
    let mut session = DocumentSession::open(f.old()).expect("open");
    assert!(session.save_as(f.new_path(), false).is_err());
    assert_eq!(session.path(), f.old());
    assert!(!f.new_path().exists());
    assert!(!f.0.join("new/assets/a.png").exists());
    assert_eq!(
        fs::read(f.0.join("new/assets/b.png")).expect("existing"),
        b"unrelated"
    );
}

#[test]
fn identical_destination_is_reused_and_missing_resources_do_not_change_identity() {
    let f = Fixture::new();
    fs::create_dir(f.0.join("new/assets")).expect("destination");
    for side in ["old", "new"] {
        fs::write(f.0.join(side).join("assets/a.png"), b"same").expect("image");
    }
    fs::write(f.old(), "![a](assets/a.png)").expect("document");
    let mut session = DocumentSession::open(f.old()).expect("open");
    session
        .save_as(f.new_path(), false)
        .expect("reuse same bytes");
    fs::remove_file(f.0.join("old/assets/a.png")).expect("missing source image");
    let mut missing = DocumentSession::open(f.old()).expect("open missing image document");
    assert!(missing.save_as(f.0.join("new/other.md"), false).is_err());
    assert_eq!(missing.path(), f.old());
    assert!(!f.0.join("new/other.md").exists());
}

#[test]
fn image_syntax_inside_code_does_not_copy_a_resource() {
    let f = Fixture::new();
    let body = "`![inline](assets/missing.png)`\n\n```md\n![fenced](assets/missing.png)\n```\n";
    fs::write(f.old(), body).expect("document");
    let mut session = DocumentSession::open(f.old()).expect("open");
    session
        .save_as(f.new_path(), false)
        .expect("save code examples");
    assert_eq!(
        fs::read_to_string(f.new_path()).expect("saved source"),
        body
    );
    assert!(!f.0.join("new/assets").exists());
}

#[cfg(unix)]
#[test]
fn destination_symlink_cannot_redirect_resource_writes() {
    let f = Fixture::new();
    fs::create_dir(f.0.join("outside")).expect("outside");
    std::os::unix::fs::symlink(f.0.join("outside"), f.0.join("new/assets")).expect("symlink");
    fs::write(f.old(), "![a](assets/a.png)").expect("document");
    fs::write(f.0.join("old/assets/a.png"), b"a").expect("image");
    let mut session = DocumentSession::open(f.old()).expect("open");
    assert!(session.save_as(f.new_path(), false).is_err());
    assert!(!f.0.join("outside/a.png").exists());
    assert!(!f.new_path().exists());
}

#[test]
fn save_as_keeps_images_reachable_by_undo_after_their_source_is_removed() {
    let f = Fixture::new();
    let body = "![a](assets/a.png)";
    fs::write(f.old(), body).expect("document");
    fs::write(f.0.join("old/assets/a.png"), b"undo image").expect("image");
    let mut session = DocumentSession::open(f.old()).expect("open");
    let snapshot = session.editor().snapshot();
    let selected = yu_editor::EditorSelection::range(
        &snapshot,
        yu_core::ByteOffset::ZERO,
        snapshot.len_bytes(),
        yu_editor::CaretAffinity::Downstream,
    )
    .expect("selection");
    session
        .editor_mut()
        .set_selection(selected)
        .expect("select image");
    session
        .editor_mut()
        .execute(EditorCommand::DeleteSelections)
        .expect("remove image");
    session
        .save_as(f.new_path(), false)
        .expect("save blank document");
    session.editor_mut().undo().expect("restore image");
    assert_eq!(session.editor().snapshot().as_str(), body);
    assert_eq!(
        fs::read(f.0.join("new/assets/a.png")).expect("undo resource survives Save As"),
        b"undo image"
    );
}

#[test]
fn newly_inserted_then_undone_image_can_be_redone_after_save_as() {
    let f = Fixture::new();
    fs::write(f.old(), "text").expect("document");
    fs::write(f.0.join("old/assets/new.png"), b"new image").expect("image");
    let mut session = DocumentSession::open(f.old()).expect("open");
    session
        .editor_mut()
        .execute(EditorCommand::PasteFragments(vec![
            "![new](assets/new.png)".into(),
        ]))
        .expect("insert image");
    session.editor_mut().undo().expect("undo image");
    session
        .save_as(f.new_path(), false)
        .expect("save after undo");
    session.editor_mut().redo().expect("redo image");
    assert_eq!(
        session.editor().snapshot().as_str(),
        "![new](assets/new.png)text"
    );
    assert_eq!(
        fs::read(f.0.join("new/assets/new.png")).expect("redo resource"),
        b"new image"
    );
}

#[test]
fn image_property_edit_save_as_reopen_and_undo_keep_resource_and_source() {
    let f = Fixture::new();
    let original = "\u{feff}# 中文\r\n\r\n![图](<assets/a&amp;b.png> 'title')\r\n\r\nKEEP\r\n";
    fs::write(f.old(), original).expect("image save-as fixture");
    fs::write(f.0.join("old/assets/a&b.png"), b"image bytes").expect("image save-as fixture");
    let mut session = DocumentSession::open(f.old()).expect("image save-as fixture");
    let range = session
        .editor()
        .image_references()
        .expect("image save-as fixture")[0]
        .source;
    let mut properties = session
        .editor()
        .image_properties(range)
        .expect("image save-as fixture");
    assert_eq!(properties.destination, "assets/a&b.png");
    properties.width = Some(160);
    properties.height = Some(80);
    session
        .editor_mut()
        .update_image_properties(&properties)
        .expect("image save-as fixture");
    let edited = session.editor().snapshot().as_str().to_owned();
    session
        .save_as(f.new_path(), false)
        .expect("image save-as fixture");
    assert_eq!(
        fs::read(f.0.join("new/assets/a&b.png")).expect("image save-as fixture"),
        b"image bytes"
    );
    let reopened = DocumentSession::open(f.new_path()).expect("image save-as fixture");
    assert_eq!(reopened.editor().snapshot().as_str(), edited);
    let reopened_range = reopened
        .editor()
        .image_references()
        .expect("image save-as fixture")[0]
        .source;
    let properties = reopened
        .editor()
        .image_properties(reopened_range)
        .expect("image save-as fixture");
    assert_eq!((properties.width, properties.height), (Some(160), Some(80)));
    session
        .editor_mut()
        .undo()
        .expect("undo image property edit");
    session.save().expect("image save-as fixture");
    assert_eq!(
        fs::read(f.new_path()).expect("image save-as fixture"),
        original.as_bytes()
    );
    assert_eq!(
        fs::read(f.old()).expect("image save-as fixture"),
        original.as_bytes()
    );
}

#[test]
fn missing_image_stays_missing_without_blocking_source_save_as() {
    for source_mode in [false, true] {
        let f = Fixture::new();
        let body =
            "\u{feff}# 原文\r\n\r\n![缺失](assets/missing.png) ![现有](assets/available.png)\r\n";
        fs::write(f.old(), body).expect("source");
        fs::write(f.0.join("old/assets/available.png"), b"original resource").expect("image");
        let mut session = DocumentSession::open(f.old()).expect("open");
        session
            .editor_mut()
            .set_source_mode(source_mode)
            .expect("mode");
        let revision = session.revision();
        let history = session.editor().history_stats();
        session
            .save_as(f.new_path(), false)
            .expect("preserve missing reference");
        assert_eq!(fs::read(f.new_path()).expect("saved"), body.as_bytes());
        assert_eq!(fs::read(f.old()).expect("original"), body.as_bytes());
        assert!(!f.0.join("new/assets/missing.png").exists());
        assert_eq!(
            fs::read(f.0.join("new/assets/available.png")).expect("copied"),
            b"original resource"
        );
        assert_eq!(session.revision(), revision);
        assert_eq!(session.editor().history_stats(), history);
        let reopened = DocumentSession::open(f.new_path()).expect("reopen");
        assert_eq!(
            reopened.editor().snapshot().as_str(),
            body.trim_start_matches('\u{feff}')
        );
    }
}

#[cfg(unix)]
#[test]
fn missing_image_never_rebinds_to_a_dangling_destination_symlink() {
    let f = Fixture::new();
    fs::create_dir(f.0.join("new/assets")).expect("assets");
    fs::write(f.old(), "![missing](assets/missing.png)").expect("source");
    std::os::unix::fs::symlink("elsewhere.png", f.0.join("new/assets/missing.png"))
        .expect("dangling link");
    let mut session = DocumentSession::open(f.old()).expect("open");
    assert!(session.save_as(f.new_path(), false).is_err());
    assert!(!f.new_path().exists());
    assert_eq!(session.path(), f.old());
    assert_eq!(
        fs::read_link(f.0.join("new/assets/missing.png")).expect("untouched link"),
        PathBuf::from("elsewhere.png")
    );
}

#[test]
fn parent_relative_images_shared_by_both_documents_need_no_copy_or_rewrite() {
    let f = Fixture::new();
    fs::create_dir(f.0.join("shared")).expect("shared directory");
    fs::write(f.0.join("shared/图.png"), b"shared original").expect("resource");
    let body =
        "\u{feff}![shared](../shared/%E5%9B%BE.png)\r\n![missing](../shared/missing.png)\r\n";
    fs::write(f.old(), body).expect("source");
    let mut session = DocumentSession::open(f.old()).expect("open");
    session
        .save_as(f.new_path(), false)
        .expect("same resolved references");
    assert_eq!(
        fs::read(f.new_path()).expect("saved source"),
        body.as_bytes()
    );
    assert_eq!(
        fs::read(f.0.join("shared/图.png")).expect("original resource"),
        b"shared original"
    );
    assert!(!f.0.join("shared/missing.png").exists());
    assert!(!f.0.join("new/assets").exists());
}

#[test]
fn parent_relative_images_copy_to_corresponding_sibling_without_rewriting_history() {
    let f = Fixture::new();
    fs::create_dir_all(f.0.join("isolated/notes")).expect("new document directory");
    fs::create_dir(f.0.join("shared")).expect("source resources");
    fs::write(f.0.join("shared/a.png"), b"original").expect("resource");
    fs::write(f.old(), "![a](../shared/a.png)").expect("source");
    let target = f.0.join("isolated/notes/copy.md");
    let mut session = DocumentSession::open(f.old()).expect("open");
    session
        .editor_mut()
        .execute(EditorCommand::insert_text("前言\n"))
        .expect("edit");
    let expected = session.editor().snapshot().as_str().to_owned();
    let history = session.editor().history_stats();
    session
        .save_as(&target, false)
        .expect("parent-relative migration");
    assert_eq!(fs::read_to_string(&target).expect("saved"), expected);
    assert_eq!(
        fs::read(f.0.join("isolated/shared/a.png")).expect("copied"),
        b"original"
    );
    assert_eq!(session.editor().history_stats(), history);
    session.editor_mut().undo().expect("undo before save as");
    assert_eq!(
        session.editor().snapshot().as_str(),
        "![a](../shared/a.png)"
    );
    session.save().expect("save undo");
    assert_eq!(
        DocumentSession::open(&target)
            .expect("reopen")
            .editor()
            .snapshot()
            .as_str(),
        "![a](../shared/a.png)"
    );
    assert_eq!(
        fs::read(f.0.join("shared/a.png")).expect("original preserved"),
        b"original"
    );
}

#[test]
fn parent_relative_conflict_preserves_source_history_and_all_resources() {
    let f = Fixture::new();
    fs::create_dir_all(f.0.join("isolated/notes")).expect("create fixture directory");
    fs::create_dir_all(f.0.join("isolated/shared")).expect("create fixture directory");
    fs::create_dir(f.0.join("shared")).expect("create fixture directory");
    fs::write(f.0.join("shared/a.png"), b"a").expect("write fixture bytes");
    fs::write(f.0.join("shared/b.png"), b"b").expect("write fixture bytes");
    fs::write(f.0.join("isolated/shared/b.png"), b"unrelated").expect("write fixture bytes");
    let body = "![a](../shared/a.png) ![b](../shared/b.png)";
    fs::write(f.old(), body).expect("write fixture bytes");
    let mut session = DocumentSession::open(f.old()).expect("open source document");
    let history = session.editor().history_stats();
    let target = f.0.join("isolated/notes/copy.md");
    assert!(session.save_as(&target, false).is_err());
    assert_eq!(session.path(), f.old());
    assert_eq!(session.editor().snapshot().as_str(), body);
    assert_eq!(session.editor().history_stats(), history);
    assert!(!target.exists());
    assert!(!f.0.join("isolated/shared/a.png").exists());
    assert_eq!(
        fs::read(f.0.join("isolated/shared/b.png")).expect("read preserved bytes"),
        b"unrelated"
    );
}

#[cfg(unix)]
#[test]
fn parent_relative_destination_symlink_cannot_redirect_writes() {
    let f = Fixture::new();
    fs::create_dir_all(f.0.join("isolated/notes")).expect("create fixture directory");
    fs::create_dir(f.0.join("shared")).expect("create fixture directory");
    fs::create_dir(f.0.join("outside")).expect("create fixture directory");
    fs::write(f.0.join("shared/a.png"), b"a").expect("write fixture bytes");
    std::os::unix::fs::symlink(f.0.join("outside"), f.0.join("isolated/shared"))
        .expect("create destination symlink");
    fs::write(f.old(), "![a](../shared/a.png)").expect("write fixture bytes");
    let mut session = DocumentSession::open(f.old()).expect("open source document");
    let target = f.0.join("isolated/notes/copy.md");
    assert!(session.save_as(&target, false).is_err());
    assert!(!target.exists());
    assert!(!f.0.join("outside/a.png").exists());
}

#[test]
fn parent_relative_undone_image_remains_available_to_redo_after_save_as() {
    let f = Fixture::new();
    fs::create_dir_all(f.0.join("isolated/notes")).expect("create fixture directory");
    fs::create_dir(f.0.join("shared")).expect("create fixture directory");
    fs::write(f.0.join("shared/图 %.png"), b"redo bytes").expect("write fixture bytes");
    fs::write(f.old(), "\u{feff}正文\r\n").expect("write fixture bytes");
    let mut session = DocumentSession::open(f.old()).expect("open source document");
    session
        .editor_mut()
        .execute(EditorCommand::insert_text(
            "![图](../shared/%E5%9B%BE%20%25.png)\r\n",
        ))
        .expect("insert image reference");
    let inserted = session.editor().snapshot().as_str().to_owned();
    session.editor_mut().undo().expect("undo image insertion");
    let target = f.0.join("isolated/notes/copy.md");
    session
        .save_as(&target, false)
        .expect("save parent-relative references");
    assert_eq!(
        fs::read(&target).expect("read preserved bytes"),
        "\u{feff}正文\r\n".as_bytes()
    );
    session.editor_mut().redo().expect("redo image insertion");
    assert_eq!(session.editor().snapshot().as_str(), inserted);
    assert_eq!(
        fs::read(f.0.join("isolated/shared/图 %.png")).expect("read preserved bytes"),
        b"redo bytes"
    );
    session.save().expect("save restored source");
    assert_eq!(
        fs::read(&target).expect("read preserved bytes"),
        format!("\u{feff}{inserted}").as_bytes()
    );
}

#[test]
fn missing_parent_relative_image_preserves_bom_crlf_without_creating_directories() {
    let f = Fixture::new();
    fs::create_dir_all(f.0.join("isolated/notes")).expect("create fixture directory");
    let body = "\u{feff}![missing](../shared/missing.png)\r\n";
    fs::write(f.old(), body).expect("write fixture bytes");
    let mut session = DocumentSession::open(f.old()).expect("open source document");
    let target = f.0.join("isolated/notes/copy.md");
    session
        .save_as(&target, false)
        .expect("save parent-relative references");
    assert_eq!(
        fs::read(&target).expect("read preserved bytes"),
        body.as_bytes()
    );
    assert!(!f.0.join("isolated/shared").exists());
    assert_eq!(
        fs::read(f.old()).expect("read preserved bytes"),
        body.as_bytes()
    );
}
