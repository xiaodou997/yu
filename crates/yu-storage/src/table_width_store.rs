//! Application-owned presentation metadata, separate from Markdown bytes.
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use yu_core::{ByteOffset, TextRange};
use yu_editor::TableColumnWidthRecord;

use super::{DiskState, DocumentSession, StorageError, atomic_replace, current_fingerprint, fnv1a};

const MAGIC: &[u8; 8] = b"YUCOLW01";
const MAX_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct TableWidthStore {
    root: PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableWidthStoreOutcome {
    Missing,
    Invalid,
    Stale,
    Deferred,
    Restored(usize),
    Written,
    Unchanged,
}

impl TableWidthStore {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn path_for(&self, session: &DocumentSession) -> Result<PathBuf, StorageError> {
        let identity = identity(session)?;
        Ok(self
            .root
            .join(format!("{:016x}.yucolumns", fnv1a(identity.as_bytes()))))
    }

    /// Never persist proportions against unsaved or externally changed source.
    /// The host retries after a successful document save.
    pub fn write(&self, session: &DocumentSession) -> Result<TableWidthStoreOutcome, StorageError> {
        if session.is_dirty() || session.expected_file.is_none() {
            return Ok(TableWidthStoreOutcome::Deferred);
        }
        if session.disk_state()? != DiskState::Unchanged {
            return Ok(TableWidthStoreOutcome::Stale);
        }
        let path = self.path_for(session)?;
        let source = session.editor.snapshot();
        let records = session.editor.table_column_width_records();
        let previous = read_bounded(&path)?;
        if records.is_empty() && previous.is_none() {
            return Ok(TableWidthStoreOutcome::Unchanged);
        }
        let bytes =
            encode(identity(session)?, source.as_str().as_bytes(), &records).ok_or_else(|| {
                StorageError::io(
                    "encode table widths",
                    &path,
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        "table width metadata exceeds size limit",
                    ),
                )
            })?;
        if previous.as_deref() == Some(bytes.as_slice()) {
            return Ok(TableWidthStoreOutcome::Unchanged);
        }
        fs::create_dir_all(&self.root)
            .map_err(|error| StorageError::io("create table width directory", &self.root, error))?;
        atomic_replace(&path, &bytes, current_fingerprint(&path)?.as_ref())?;
        Ok(TableWidthStoreOutcome::Written)
    }

    /// Invalid caches are reported without modifying the editor or document.
    pub fn restore(
        &self,
        session: &mut DocumentSession,
    ) -> Result<TableWidthStoreOutcome, StorageError> {
        if session.is_dirty() || session.expected_file.is_none() {
            return Ok(TableWidthStoreOutcome::Deferred);
        }
        if session.disk_state()? != DiskState::Unchanged {
            return Ok(TableWidthStoreOutcome::Stale);
        }
        let path = self.path_for(session)?;
        let Some(bytes) = read_bounded(&path)? else {
            return Ok(TableWidthStoreOutcome::Missing);
        };
        let Some(record) = decode(&bytes) else {
            return Ok(TableWidthStoreOutcome::Invalid);
        };
        let source = session.editor.snapshot();
        if record.identity != identity(session)?.as_bytes()
            || record.source_len != source.as_str().len() as u64
            || record.source_hash != fnv1a(source.as_str().as_bytes())
        {
            return Ok(TableWidthStoreOutcome::Stale);
        }
        let count = record.widths.len();
        if session
            .editor
            .restore_table_column_width_records(&record.widths)
            .is_err()
        {
            return Ok(TableWidthStoreOutcome::Invalid);
        }
        Ok(TableWidthStoreOutcome::Restored(count))
    }
}

fn identity(session: &DocumentSession) -> Result<&str, StorageError> {
    session
        .storage_path
        .to_str()
        .ok_or_else(|| StorageError::InvalidPath(session.storage_path.clone()))
}

fn read_bounded(path: &Path) -> Result<Option<Vec<u8>>, StorageError> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(StorageError::io("open table widths", path, error)),
    };
    let mut bytes = Vec::new();
    file.take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| StorageError::io("read table widths", path, error))?;
    Ok(Some(bytes))
}

fn encode(identity: &str, source: &[u8], records: &[TableColumnWidthRecord]) -> Option<Vec<u8>> {
    let mut bytes = Vec::from(MAGIC.as_slice());
    bytes.extend_from_slice(&u32::try_from(identity.len()).ok()?.to_le_bytes());
    bytes.extend_from_slice(identity.as_bytes());
    bytes.extend_from_slice(&(source.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&fnv1a(source).to_le_bytes());
    bytes.extend_from_slice(&u32::try_from(records.len()).ok()?.to_le_bytes());
    for record in records {
        let size = 20_usize.checked_add(record.proportions.len().checked_mul(4)?)?;
        if bytes.len().checked_add(size)?.checked_add(8)? > MAX_BYTES {
            return None;
        }
        bytes.extend_from_slice(&record.delimiter.start().get().to_le_bytes());
        bytes.extend_from_slice(&record.delimiter.end().get().to_le_bytes());
        bytes.extend_from_slice(&u32::try_from(record.proportions.len()).ok()?.to_le_bytes());
        for ratio in &record.proportions {
            bytes.extend_from_slice(&ratio.to_bits().to_le_bytes());
        }
    }
    if bytes.len().checked_add(8)? > MAX_BYTES {
        return None;
    }
    let checksum = fnv1a(&bytes);
    bytes.extend_from_slice(&checksum.to_le_bytes());
    Some(bytes)
}

struct Decoded<'a> {
    identity: &'a [u8],
    source_len: u64,
    source_hash: u64,
    widths: Vec<TableColumnWidthRecord>,
}

fn take<'a>(bytes: &mut &'a [u8], count: usize) -> Option<&'a [u8]> {
    let (value, rest) = bytes.split_at_checked(count)?;
    *bytes = rest;
    Some(value)
}
fn u32_value(bytes: &mut &[u8]) -> Option<u32> {
    Some(u32::from_le_bytes(take(bytes, 4)?.try_into().ok()?))
}
fn u64_value(bytes: &mut &[u8]) -> Option<u64> {
    Some(u64::from_le_bytes(take(bytes, 8)?.try_into().ok()?))
}

fn decode(bytes: &[u8]) -> Option<Decoded<'_>> {
    if bytes.len() > MAX_BYTES || bytes.len() < 40 {
        return None;
    }
    let (mut body, checksum) = bytes.split_at(bytes.len() - 8);
    if fnv1a(body) != u64::from_le_bytes(checksum.try_into().ok()?) || take(&mut body, 8)? != MAGIC
    {
        return None;
    }
    let identity_len = u32_value(&mut body)? as usize;
    let identity = take(&mut body, identity_len)?;
    let source_len = u64_value(&mut body)?;
    let source_hash = u64_value(&mut body)?;
    let count = u32_value(&mut body)? as usize;
    if count > body.len() / 24 {
        return None;
    }
    let mut widths = Vec::with_capacity(count);
    for _ in 0..count {
        let start = u64_value(&mut body)?;
        let end = u64_value(&mut body)?;
        let count = u32_value(&mut body)? as usize;
        if count == 0 || count > body.len() / 4 {
            return None;
        }
        let mut proportions = Vec::with_capacity(count);
        for _ in 0..count {
            proportions.push(f32::from_bits(u32_value(&mut body)?));
        }
        widths.push(TableColumnWidthRecord {
            delimiter: TextRange::new(ByteOffset::new(start), ByteOffset::new(end))?,
            proportions,
        });
    }
    if !body.is_empty() {
        return None;
    }
    Some(Decoded {
        identity,
        source_len,
        source_hash,
        widths,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use yu_editor::{EditorCommand, LayoutConfig};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    const SOURCE: &[u8] = b"\xef\xbb\xbf| A | B |\r\n| --- | --- |\r\n| one | two |\r\n";
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "yu-width-store-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).expect("directory");
            Self(path)
        }
        fn open(&self, name: &str) -> DocumentSession {
            let path = self.0.join(name);
            fs::write(&path, SOURCE).expect("source");
            DocumentSession::open(path).expect("open")
        }
        fn store(&self) -> TableWidthStore {
            TableWidthStore::new(self.0.join("metadata"))
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn resize(session: &mut DocumentSession) {
        let mut layout = session
            .editor_mut()
            .block_layout_for_visual_state(0, LayoutConfig::new(500.0, 16.0))
            .expect("layout");
        layout.apply_table_column_resize(0, -70.0).expect("resize");
        session
            .editor_mut()
            .confirm_table_column_widths(0, layout.table().expect("table"))
            .expect("confirm");
    }

    #[test]
    fn reopening_restores_widths_without_rewriting_bom_crlf_or_adding_history() {
        let fixture = Fixture::new();
        let mut session = fixture.open("document.md");
        let store = fixture.store();
        assert_eq!(
            store.restore(&mut session).expect("missing"),
            TableWidthStoreOutcome::Missing
        );
        resize(&mut session);
        assert!(!session.is_dirty());
        let records = session.editor().table_column_width_records();
        assert_eq!(
            store.write(&session).expect("write"),
            TableWidthStoreOutcome::Written
        );
        assert_eq!(
            store.write(&session).expect("same"),
            TableWidthStoreOutcome::Unchanged
        );
        let mut reopened = DocumentSession::open(session.path()).expect("reopen");
        assert_eq!(
            store.restore(&mut reopened).expect("restore"),
            TableWidthStoreOutcome::Restored(1)
        );
        assert_eq!(reopened.editor().table_column_width_records(), records);
        assert!(!reopened.editor_mut().undo().expect("undo").changed());
        assert!(!reopened.is_dirty());
        assert_eq!(fs::read(session.path()).expect("disk"), SOURCE);
        reopened
            .editor_mut()
            .restore_table_column_width_records(&[])
            .expect("clear");
        store.write(&reopened).expect("persist clear");
        assert_eq!(
            store.restore(&mut session).expect("restore clear"),
            TableWidthStoreOutcome::Restored(0)
        );
        assert!(session.editor().table_column_width_records().is_empty());
        assert_eq!(
            fs::read_dir(&store.root).expect("entries").count(),
            1,
            "no abandoned temporary file"
        );
    }

    #[test]
    fn dirty_external_and_wrong_path_records_are_not_applied() {
        let fixture = Fixture::new();
        let mut session = fixture.open("document.md");
        let store = fixture.store();
        resize(&mut session);
        store.write(&session).expect("write");
        let path = store.path_for(&session).expect("path");
        let initial = fs::read(&path).expect("metadata");
        session
            .editor_mut()
            .execute(EditorCommand::insert_text("prefix\n\n"))
            .expect("edit");
        assert_eq!(
            store.write(&session).expect("defer"),
            TableWidthStoreOutcome::Deferred
        );
        assert_eq!(
            store.restore(&mut session).expect("defer restore"),
            TableWidthStoreOutcome::Deferred
        );
        assert_eq!(fs::read(&path).expect("untouched"), initial);
        assert_eq!(fs::read(session.path()).expect("source"), SOURCE);
        session.save().expect("save source");
        assert_eq!(
            store.restore(&mut session).expect("old fingerprint"),
            TableWidthStoreOutcome::Stale
        );
        store.write(&session).expect("write moved anchors");
        let mut reopened = DocumentSession::open(session.path()).expect("reopen");
        assert_eq!(
            store.restore(&mut reopened).expect("restore moved anchors"),
            TableWidthStoreOutcome::Restored(1)
        );
        assert_eq!(
            reopened.editor().table_column_width_records(),
            session.editor().table_column_width_records()
        );
        fs::write(session.path(), b"external replacement\n").expect("external");
        assert_eq!(
            store.write(&session).expect("external save"),
            TableWidthStoreOutcome::Stale
        );
        assert_eq!(
            store.restore(&mut reopened).expect("external restore"),
            TableWidthStoreOutcome::Stale
        );
        let mut other = fixture.open("other.md");
        fs::write(store.path_for(&other).expect("other path"), initial)
            .expect("copy mismatched cache");
        assert_eq!(
            store.restore(&mut other).expect("wrong identity"),
            TableWidthStoreOutcome::Stale
        );
        assert!(other.editor().table_column_width_records().is_empty());
    }

    #[test]
    fn malformed_and_oversized_records_leave_editor_untouched() {
        let fixture = Fixture::new();
        let mut session = fixture.open("document.md");
        let store = fixture.store();
        resize(&mut session);
        store.write(&session).expect("write");
        let path = store.path_for(&session).expect("path");
        let valid = fs::read(&path).expect("bytes");
        let expected = session.editor().table_column_width_records();
        let mut invalid = expected.clone();
        invalid[0].proportions = vec![f32::NAN, 0.5];
        let snapshot = session.editor().snapshot();
        let invalid_numbers = encode(
            identity(&session).expect("identity"),
            snapshot.as_str().as_bytes(),
            &invalid,
        )
        .expect("encoding");
        let mut trailing = valid[..valid.len() - 8].to_vec();
        trailing.push(1);
        let checksum = fnv1a(&trailing);
        trailing.extend_from_slice(&checksum.to_le_bytes());
        for bytes in [
            valid[..12].to_vec(),
            vec![0; MAX_BYTES + 1],
            invalid_numbers,
            trailing,
        ] {
            fs::write(&path, bytes).expect("corrupt");
            assert_eq!(
                store.restore(&mut session).expect("reject"),
                TableWidthStoreOutcome::Invalid
            );
            assert_eq!(session.editor().table_column_width_records(), expected);
            assert_eq!(fs::read(session.path()).expect("source"), SOURCE);
        }
        assert!(decode(&valid).is_some());
        for end in 0..valid.len() {
            assert!(decode(&valid[..end]).is_none());
        }
    }

    #[test]
    fn attached_store_follows_save_and_reload_without_text_edits_for_widths() {
        let fixture = Fixture::new();
        let mut session = fixture.open("attached.md");
        let root = fixture.0.join("attached-state");
        session.set_table_width_store(&root).expect("attach");
        session.save().expect("clean save");
        assert!(!root.exists(), "empty metadata must not create cache files");
        resize(&mut session);
        let expected = session.editor().table_column_width_records();
        assert!(matches!(
            session.save().expect("width-only save"),
            super::super::SaveOutcome::Unchanged { .. }
        ));
        session.reload().expect("reload");
        assert_eq!(session.editor().table_column_width_records(), expected);
        session
            .editor_mut()
            .execute(EditorCommand::MoveDocumentBoundary {
                end: false,
                extend: false,
            })
            .expect("document start");
        session
            .editor_mut()
            .execute(EditorCommand::insert_text("prefix\n\n"))
            .expect("edit");
        assert_eq!(
            session.persist_table_widths().expect("deferred"),
            TableWidthStoreOutcome::Deferred
        );
        session.save().expect("save text and metadata");
        let updated = session.editor().table_column_width_records();
        let mut reopened = DocumentSession::open(session.path()).expect("reopen");
        assert_eq!(
            reopened
                .set_table_width_store(root)
                .expect("attach reopened"),
            TableWidthStoreOutcome::Restored(1)
        );
        assert_eq!(reopened.editor().table_column_width_records(), updated);
        assert!(!reopened.is_dirty());
    }

    #[test]
    fn source_that_stops_being_a_table_does_not_export_orphaned_widths() {
        let fixture = Fixture::new();
        let mut session = fixture.open("structure.md");
        let root = fixture.0.join("state");
        session.set_table_width_store(&root).expect("attach");
        resize(&mut session);
        session.save().expect("save widths");
        session.reload().expect("reload");
        let revision = session.editor().revision();
        let header = TextRange::new(ByteOffset::ZERO, ByteOffset::new(9)).expect("header");
        session
            .editor_mut()
            .apply_transaction(&yu_text::Transaction::new(
                revision,
                [yu_text::Edit::new(header, "plain")],
            ))
            .expect("remove the two-column header");
        let layout = session
            .editor_mut()
            .block_layout_for_visual_state(0, LayoutConfig::new(500.0, 16.0))
            .expect("layout");
        assert!(
            layout.table().is_none(),
            "fixture must remove the parsed table"
        );
        assert!(session.editor().table_column_width_records().is_empty());
        session.save().expect("save changed structure");
        let mut reopened = DocumentSession::open(session.path()).expect("reopen");
        assert_eq!(
            reopened.set_table_width_store(root).expect("restore"),
            TableWidthStoreOutcome::Restored(0)
        );
        session.editor_mut().undo().expect("undo");
        assert_eq!(
            session.editor().table_column_width_records().len(),
            1,
            "undo retains the in-memory association"
        );
    }

    #[test]
    fn metadata_write_failure_does_not_touch_document() {
        let fixture = Fixture::new();
        let mut session = fixture.open("document.md");
        resize(&mut session);
        let root = fixture.0.join("not-a-directory");
        fs::write(&root, b"keep").expect("block directory");
        assert!(TableWidthStore::new(root.clone()).write(&session).is_err());
        assert_eq!(fs::read(session.path()).expect("source"), SOURCE);
        assert_eq!(fs::read(root).expect("root"), b"keep");
        assert!(!session.is_dirty());
    }
}
