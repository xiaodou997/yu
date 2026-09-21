//! File identity changes preserve the canonical editor and its undo history.
use std::fs;
use std::path::PathBuf;

use super::{
    CloseStateMachine, DocumentEditorSession, DocumentSession, ExternalFileState, FileFingerprint,
    RecoveryRecord, SaveOutcome, StorageError, atomic_replace, canonical_storage_path,
    current_fingerprint, serialize_source,
};

impl DocumentSession {
    /// Save to an explicitly selected destination. Existing files require the
    /// caller's overwrite confirmation. Failure before replacement leaves the
    /// session identity, source and history untouched.
    pub fn save_as(
        &mut self,
        path: impl Into<PathBuf>,
        replace_existing: bool,
    ) -> Result<SaveOutcome, StorageError> {
        let path = path.into();
        let name = path
            .file_name()
            .ok_or_else(|| StorageError::InvalidPath(path.clone()))?;
        let storage_path = match canonical_storage_path(&path)? {
            Some(existing) => existing,
            None => {
                let parent = path
                    .parent()
                    .filter(|p| !p.as_os_str().is_empty())
                    .unwrap_or_else(|| std::path::Path::new("."));
                fs::canonicalize(parent)
                    .map_err(|error| StorageError::io("resolve save directory", parent, error))?
                    .join(name)
            }
        };
        if self.expected_file.is_some() && storage_path == self.storage_path {
            // "Save As" to this document must not bypass its conflict check.
            return self.save();
        }
        let expected = current_fingerprint(&storage_path)?;
        if expected.is_some() && !replace_existing {
            return Err(StorageError::ExternalChange {
                path,
                state: ExternalFileState::Changed,
            });
        }
        let images = self.editor.image_references()?;
        let mut resources = super::image_relocation::ImageRelocation::prepare(
            &self.path,
            &path,
            &images,
            self.editor.retained_image_destinations(),
        )?;
        let bytes = serialize_source(self.editor.snapshot().as_str(), self.bom);
        resources.validate_before_publish()?;
        atomic_replace(&storage_path, &bytes, expected.as_ref())?;
        resources.commit();
        let metadata = fs::metadata(&storage_path)
            .map_err(|error| StorageError::io("stat saved destination", &storage_path, error))?;
        self.path = path;
        self.storage_path = storage_path;
        self.expected_file = Some(FileFingerprint::from_bytes(&bytes, &metadata));
        self.saved_revision = self.revision();
        self.recovered_dirty = false;
        // Column metadata is derived and persisted separately by the host;
        // its failure must not turn an already committed path change into an
        // apparent failure that would leave the native window on the old URL.
        Ok(SaveOutcome::Saved {
            revision: self.saved_revision,
            bytes_written: bytes.len(),
        })
    }
}

impl DocumentEditorSession {
    pub fn save_as(
        &mut self,
        path: impl Into<PathBuf>,
        replace_existing: bool,
    ) -> Result<SaveOutcome, StorageError> {
        self.document.save_as(path, replace_existing)
    }

    /// Recovery restores the original disk baseline, never the fingerprint of
    /// a possibly newer external file. Recovery itself performs no file write.
    #[must_use]
    pub fn recover(record: RecoveryRecord) -> Self {
        Self {
            document: record.into_document(),
            close: CloseStateMachine::new(),
            composition_generation: 0,
        }
    }

    /// A cancelled application-wide quit reopens every provisional close
    /// decision, including clean windows and earlier approved discards.
    pub fn abort_close(&mut self) {
        self.close = CloseStateMachine::new();
    }
}
