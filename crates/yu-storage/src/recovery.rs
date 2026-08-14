use std::error::Error;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use yu_core::Revision;

use super::{DocumentSession, Utf8Bom, fnv1a};

const MAGIC: &[u8; 8] = b"YURECOV1";
const FORMAT_VERSION: u16 = 1;
const CHECKSUM_BYTES: usize = std::mem::size_of::<u64>();
const MAX_RECOVERY_FILE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_TARGET_PATH_BYTES: u64 = 1024 * 1024;
static RECOVERY_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A caller-owned directory containing crash-recovery envelopes.
///
/// The store does not run a timer or retain an editor. The product shell may
/// call [`DocumentSession::write_recovery`] after its own debounce policy,
/// while recovery discovery remains an explicit read-and-decide operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryStore {
    root: PathBuf,
}

impl RecoveryStore {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns the deterministic envelope path for a target document.
    pub fn path_for(&self, target: &Path) -> Result<PathBuf, RecoveryError> {
        let target_text = target_text(target)?;
        let key = fnv1a(target_text.as_bytes());
        Ok(self.root.join(format!("{key:016x}.yurecovery")))
    }

    /// Writes a dirty source snapshot or removes a stale record for a clean
    /// session. The target file is never modified.
    pub fn write(&self, session: &DocumentSession) -> Result<RecoveryOutcome, RecoveryError> {
        let path = self.path_for(session.path())?;
        if !session.is_dirty() {
            self.clear_path(&path)?;
            return Ok(RecoveryOutcome::Cleared { path });
        }

        let record = RecoveryRecord {
            target_path: session.path().to_path_buf(),
            source: session.editor().snapshot().as_str().to_owned(),
            revision: session.revision(),
            saved_revision: session.saved_revision(),
            bom: session.bom(),
        };
        let encoded = encode(&record)?;
        write_atomic(&path, &encoded)?;
        Ok(RecoveryOutcome::Written {
            path,
            revision: record.revision,
            bytes_written: encoded.len(),
        })
    }

    /// Reads and validates the recovery record associated with `target`.
    /// Returning a record does not mutate or replace the target document.
    pub fn read(&self, target: &Path) -> Result<Option<RecoveryRecord>, RecoveryError> {
        let path = self.path_for(target)?;
        let metadata = match fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(RecoveryError::io("stat recovery", path, error)),
        };
        if metadata.len() > MAX_RECOVERY_FILE_BYTES {
            return Err(RecoveryError::TooLarge {
                path,
                bytes: metadata.len(),
            });
        }
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(RecoveryError::io("read recovery", path, error)),
        };
        let record = decode(&path, &bytes)?;
        if record.target_path != target {
            return Err(RecoveryError::TargetMismatch {
                path,
                expected: target.to_path_buf(),
                actual: record.target_path,
            });
        }
        Ok(Some(record))
    }

    /// Removes the recovery record associated with `target`. Missing records
    /// are already clear and therefore succeed.
    pub fn clear(&self, target: &Path) -> Result<(), RecoveryError> {
        let path = self.path_for(target)?;
        self.clear_path(&path)
    }

    fn clear_path(&self, path: &Path) -> Result<(), RecoveryError> {
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(RecoveryError::io(
                "remove recovery",
                path.to_path_buf(),
                error,
            )),
        }
    }
}

/// A validated source candidate loaded from a recovery envelope.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryRecord {
    target_path: PathBuf,
    source: String,
    revision: Revision,
    saved_revision: Revision,
    bom: Utf8Bom,
}

impl RecoveryRecord {
    #[must_use]
    pub fn target_path(&self) -> &Path {
        &self.target_path
    }

    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    #[must_use]
    pub const fn saved_revision(&self) -> Revision {
        self.saved_revision
    }

    #[must_use]
    pub const fn bom(&self) -> Utf8Bom {
        self.bom
    }
}

/// Result of one explicit recovery write/clear operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecoveryOutcome {
    Written {
        path: PathBuf,
        revision: Revision,
        bytes_written: usize,
    },
    Cleared {
        path: PathBuf,
    },
}

/// Errors raised while encoding, validating or atomically storing recovery
/// envelopes.
#[derive(Debug)]
pub enum RecoveryError {
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    InvalidPath(PathBuf),
    InvalidFormat {
        path: PathBuf,
        reason: &'static str,
    },
    TargetMismatch {
        path: PathBuf,
        expected: PathBuf,
        actual: PathBuf,
    },
    TooLarge {
        path: PathBuf,
        bytes: u64,
    },
}

impl RecoveryError {
    fn io(operation: &'static str, path: PathBuf, source: io::Error) -> Self {
        Self::Io {
            operation,
            path,
            source,
        }
    }
}

impl fmt::Display for RecoveryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io {
                operation,
                path,
                source,
            } => write!(formatter, "{operation} {}: {source}", path.display()),
            Self::InvalidPath(path) => {
                write!(
                    formatter,
                    "recovery path is not valid UTF-8: {}",
                    path.display()
                )
            }
            Self::InvalidFormat { path, reason } => {
                write!(
                    formatter,
                    "invalid recovery envelope {}: {reason}",
                    path.display()
                )
            }
            Self::TargetMismatch {
                path,
                expected,
                actual,
            } => write!(
                formatter,
                "recovery envelope {} targets {}, expected {}",
                path.display(),
                actual.display(),
                expected.display()
            ),
            Self::TooLarge { path, bytes } => write!(
                formatter,
                "recovery envelope {} is too large ({bytes} bytes)",
                path.display()
            ),
        }
    }
}

impl Error for RecoveryError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::InvalidPath(_)
            | Self::InvalidFormat { .. }
            | Self::TargetMismatch { .. }
            | Self::TooLarge { .. } => None,
        }
    }
}

fn target_text(target: &Path) -> Result<&str, RecoveryError> {
    target
        .to_str()
        .ok_or_else(|| RecoveryError::InvalidPath(target.to_path_buf()))
}

fn encode(record: &RecoveryRecord) -> Result<Vec<u8>, RecoveryError> {
    let target = target_text(&record.target_path)?.as_bytes();
    let source = record.source.as_bytes();
    let target_len = u64::try_from(target.len()).map_err(|_| RecoveryError::TooLarge {
        path: record.target_path.clone(),
        bytes: target.len() as u64,
    })?;
    let source_len = u64::try_from(source.len()).map_err(|_| RecoveryError::TooLarge {
        path: record.target_path.clone(),
        bytes: source.len() as u64,
    })?;
    if target_len > MAX_TARGET_PATH_BYTES {
        return Err(RecoveryError::TooLarge {
            path: record.target_path.clone(),
            bytes: target_len,
        });
    }
    let payload_len =
        target_len
            .checked_add(source_len)
            .ok_or_else(|| RecoveryError::TooLarge {
                path: record.target_path.clone(),
                bytes: u64::MAX,
            })?;
    if payload_len > MAX_RECOVERY_FILE_BYTES {
        return Err(RecoveryError::TooLarge {
            path: record.target_path.clone(),
            bytes: payload_len,
        });
    }

    let mut bytes = Vec::with_capacity(
        MAGIC.len()
            + 2
            + 2
            + (std::mem::size_of::<u64>() * 4)
            + target.len()
            + source.len()
            + CHECKSUM_BYTES,
    );
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    bytes.push(match record.bom {
        Utf8Bom::Absent => 0,
        Utf8Bom::Present => 1,
    });
    bytes.push(0);
    bytes.extend_from_slice(&record.revision.get().to_le_bytes());
    bytes.extend_from_slice(&record.saved_revision.get().to_le_bytes());
    bytes.extend_from_slice(&target_len.to_le_bytes());
    bytes.extend_from_slice(&source_len.to_le_bytes());
    bytes.extend_from_slice(target);
    bytes.extend_from_slice(source);
    bytes.extend_from_slice(&fnv1a(&bytes).to_le_bytes());
    Ok(bytes)
}

fn decode(path: &Path, bytes: &[u8]) -> Result<RecoveryRecord, RecoveryError> {
    let file_bytes = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if file_bytes > MAX_RECOVERY_FILE_BYTES {
        return Err(RecoveryError::TooLarge {
            path: path.to_path_buf(),
            bytes: file_bytes,
        });
    }
    if bytes.len() < MAGIC.len() + 2 + 2 + (std::mem::size_of::<u64>() * 4) + CHECKSUM_BYTES {
        return Err(RecoveryError::InvalidFormat {
            path: path.to_path_buf(),
            reason: "truncated header",
        });
    }
    let checksum_start = bytes.len() - CHECKSUM_BYTES;
    let expected_checksum =
        read_u64(&bytes[checksum_start..]).ok_or_else(|| RecoveryError::InvalidFormat {
            path: path.to_path_buf(),
            reason: "missing checksum",
        })?;
    if fnv1a(&bytes[..checksum_start]) != expected_checksum {
        return Err(RecoveryError::InvalidFormat {
            path: path.to_path_buf(),
            reason: "checksum mismatch",
        });
    }

    let mut cursor = 0_usize;
    if take(bytes, &mut cursor, MAGIC.len()) != Some(MAGIC.as_slice()) {
        return Err(RecoveryError::InvalidFormat {
            path: path.to_path_buf(),
            reason: "magic mismatch",
        });
    }
    let version = take(bytes, &mut cursor, 2)
        .and_then(read_u16)
        .ok_or_else(|| invalid_format(path, "missing version"))?;
    if version != FORMAT_VERSION {
        return Err(invalid_format(path, "unsupported version"));
    }
    let bom = match take(bytes, &mut cursor, 1).and_then(|value| value.first().copied()) {
        Some(0) => Utf8Bom::Absent,
        Some(1) => Utf8Bom::Present,
        _ => return Err(invalid_format(path, "invalid BOM flag")),
    };
    if take(bytes, &mut cursor, 1) != Some(&[0]) {
        return Err(invalid_format(path, "reserved header byte is not zero"));
    }
    let revision = Revision::new(
        take(bytes, &mut cursor, 8)
            .and_then(read_u64)
            .ok_or_else(|| invalid_format(path, "missing revision"))?,
    );
    let saved_revision = Revision::new(
        take(bytes, &mut cursor, 8)
            .and_then(read_u64)
            .ok_or_else(|| invalid_format(path, "missing saved revision"))?,
    );
    let target_len = take(bytes, &mut cursor, 8)
        .and_then(read_u64)
        .ok_or_else(|| invalid_format(path, "missing target length"))?;
    let source_len = take(bytes, &mut cursor, 8)
        .and_then(read_u64)
        .ok_or_else(|| invalid_format(path, "missing source length"))?;
    if target_len > MAX_TARGET_PATH_BYTES {
        return Err(RecoveryError::TooLarge {
            path: path.to_path_buf(),
            bytes: target_len,
        });
    }
    let payload_len = target_len
        .checked_add(source_len)
        .ok_or_else(|| invalid_format(path, "payload length overflow"))?;
    if payload_len > MAX_RECOVERY_FILE_BYTES {
        return Err(RecoveryError::TooLarge {
            path: path.to_path_buf(),
            bytes: payload_len,
        });
    }
    let expected_end = cursor
        .checked_add(
            usize::try_from(payload_len).map_err(|_| RecoveryError::TooLarge {
                path: path.to_path_buf(),
                bytes: payload_len,
            })?,
        )
        .ok_or_else(|| invalid_format(path, "payload offset overflow"))?;
    if expected_end != checksum_start {
        return Err(invalid_format(path, "payload length does not match file"));
    }
    let target_len =
        usize::try_from(target_len).map_err(|_| invalid_format(path, "target length overflow"))?;
    let source_len =
        usize::try_from(source_len).map_err(|_| invalid_format(path, "source length overflow"))?;
    let target_bytes = take(bytes, &mut cursor, target_len)
        .ok_or_else(|| invalid_format(path, "missing target path"))?;
    let source_bytes = take(bytes, &mut cursor, source_len)
        .ok_or_else(|| invalid_format(path, "missing source"))?;
    let target = String::from_utf8(target_bytes.to_vec())
        .map_err(|_| invalid_format(path, "target path is not UTF-8"))?;
    let source = String::from_utf8(source_bytes.to_vec())
        .map_err(|_| invalid_format(path, "source is not UTF-8"))?;
    Ok(RecoveryRecord {
        target_path: PathBuf::from(target),
        source,
        revision,
        saved_revision,
        bom,
    })
}

fn invalid_format(path: &Path, reason: &'static str) -> RecoveryError {
    RecoveryError::InvalidFormat {
        path: path.to_path_buf(),
        reason,
    }
}

fn take<'a>(bytes: &'a [u8], cursor: &mut usize, length: usize) -> Option<&'a [u8]> {
    let end = cursor.checked_add(length)?;
    let value = bytes.get(*cursor..end)?;
    *cursor = end;
    Some(value)
}

fn read_u16(bytes: &[u8]) -> Option<u16> {
    bytes.try_into().ok().map(u16::from_le_bytes)
}

fn read_u64(bytes: &[u8]) -> Option<u64> {
    bytes.try_into().ok().map(u64::from_le_bytes)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), RecoveryError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|error| {
        RecoveryError::io("create recovery directory", parent.to_path_buf(), error)
    })?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| RecoveryError::InvalidPath(path.to_path_buf()))?;
    let counter = RECOVERY_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp_path = parent.join(format!(".{file_name}.tmp-{}-{counter}", std::process::id()));
    let mut guard = RecoveryTempGuard::new(temp_path.clone());
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp_path)
        .map_err(|error| {
            RecoveryError::io("create recovery temporary file", temp_path.clone(), error)
        })?;
    file.write_all(bytes).map_err(|error| {
        RecoveryError::io("write recovery temporary file", temp_path.clone(), error)
    })?;
    file.sync_all().map_err(|error| {
        RecoveryError::io("sync recovery temporary file", temp_path.clone(), error)
    })?;
    drop(file);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temp_path, fs::Permissions::from_mode(0o600)).map_err(|error| {
            RecoveryError::io("set recovery permissions", temp_path.clone(), error)
        })?;
    }
    fs::rename(&temp_path, path)
        .map_err(|error| RecoveryError::io("atomic rename recovery", path.to_path_buf(), error))?;
    guard.disarm();
    Ok(())
}

struct RecoveryTempGuard {
    path: PathBuf,
    armed: bool,
}

impl RecoveryTempGuard {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for RecoveryTempGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_file(&self.path);
        }
    }
}
