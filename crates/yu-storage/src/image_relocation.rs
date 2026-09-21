//! Copy relative resources before publishing a Save As document. No Markdown
//! rewriting, no overwrite of unrelated resources, and no renderer dependency.
use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

use yu_assets::ImageLocation;
use yu_editor::ImageReference;

use super::{FileFingerprint, StorageError};

pub(super) struct ImageRelocation {
    created: Vec<(PathBuf, FileFingerprint)>,
    directories: Vec<PathBuf>,
    missing: Vec<(PathBuf, PathBuf)>,
    committed: bool,
}

impl ImageRelocation {
    pub fn prepare<'a>(
        old: &Path,
        new: &Path,
        images: &[ImageReference],
        retained: impl Iterator<Item = &'a str>,
    ) -> Result<Self, StorageError> {
        let mut pending = Self {
            created: Vec::new(),
            directories: Vec::new(),
            missing: Vec::new(),
            committed: false,
        };
        let old_parent = parent(old)?;
        let new_parent = parent(new)?;
        if old_parent == new_parent {
            return Ok(pending);
        }
        let mut destinations = BTreeSet::new();
        let mut copies = Vec::new();
        // Validate every destination before creating any resource.
        for (destination, required) in images
            .iter()
            .map(|image| (image.destination_text.as_str(), true))
            .chain(retained.map(|destination| (destination, false)))
        {
            if destination.contains("://") || destination.starts_with("data:") {
                continue;
            }
            let decoded = match ImageLocation::resolve("document.md", destination) {
                Ok(path) => path,
                Err(_) if !required => continue,
                Err(error) => {
                    return Err(StorageError::io(
                        "decode image path",
                        old,
                        io::Error::new(io::ErrorKind::InvalidData, error),
                    ));
                }
            };
            if decoded.path().is_absolute() {
                continue;
            }
            let source = old_parent.join(decoded.path());
            // Shared parent references need neither a copy nor a rewrite.
            if decoded
                .path()
                .components()
                .any(|part| part == Component::ParentDir)
                && same_resource_location(&source, &new_parent.join(decoded.path()))
            {
                continue;
            }
            let (resource_root, relative) = match resource_destination(&new_parent, decoded.path())
            {
                Ok(destination) => destination,
                Err(_) if !required => continue,
                Err(error) => return Err(error),
            };
            let target = resource_root.join(&relative);
            if !destinations.insert(target.clone()) {
                continue;
            }
            check_parents(&resource_root, &relative)?;
            let bytes = match read_image(&source) {
                Ok(bytes) => bytes,
                Err(_) if !required => continue,
                Err(StorageError::Io { source: error, .. })
                    if error.kind() == io::ErrorKind::NotFound =>
                {
                    // A pre-existing broken reference must not prevent saving
                    // the user's source. Preserve it only if it remains broken
                    // at the new location: never bind it to an unrelated file.
                    match fs::symlink_metadata(&target) {
                        Err(error) if error.kind() == io::ErrorKind::NotFound => {
                            pending.missing.push((source, target));
                            continue;
                        }
                        Ok(_) => {
                            return Err(StorageError::io(
                                "missing source image would bind to an existing destination",
                                &target,
                                io::Error::from(io::ErrorKind::AlreadyExists),
                            ));
                        }
                        Err(error) => {
                            return Err(StorageError::io(
                                "inspect missing image destination",
                                &target,
                                error,
                            ));
                        }
                    }
                }
                Err(error) => return Err(error),
            };
            match fs::symlink_metadata(&target) {
                Ok(metadata) => {
                    if metadata.file_type().is_symlink()
                        || !metadata.is_file()
                        || read_image(&target)? != bytes
                    {
                        return Err(StorageError::io(
                            "image destination already exists with different contents",
                            &target,
                            io::Error::from(io::ErrorKind::AlreadyExists),
                        ));
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    let metadata = fs::metadata(&source).map_err(|error| {
                        StorageError::io("inspect source image", &source, error)
                    })?;
                    copies.push((
                        resource_root,
                        target,
                        source,
                        FileFingerprint::from_bytes(&bytes, &metadata),
                    ));
                }
                Err(error) => {
                    return Err(StorageError::io(
                        "inspect image destination",
                        &target,
                        error,
                    ));
                }
            }
        }
        for (resource_root, target, source, expected) in copies {
            let bytes = read_image(&source)?;
            let metadata = fs::metadata(&source)
                .map_err(|error| StorageError::io("inspect source image", &source, error))?;
            if FileFingerprint::from_bytes(&bytes, &metadata) != expected {
                return Err(StorageError::io(
                    "source image changed during Save As",
                    &source,
                    io::Error::from(io::ErrorKind::Interrupted),
                ));
            }
            let mut directory = resource_root.clone();
            let relative = target
                .strip_prefix(&resource_root)
                .expect("validated resource parent");
            if let Some(parent) = relative.parent() {
                for component in parent.components() {
                    directory.push(component);
                    match fs::create_dir(&directory) {
                        Ok(()) => pending.directories.push(directory.clone()),
                        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                            let metadata = fs::symlink_metadata(&directory).map_err(|error| {
                                StorageError::io("inspect image directory", &directory, error)
                            })?;
                            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                                return Err(StorageError::InvalidPath(directory));
                            }
                        }
                        Err(error) => {
                            return Err(StorageError::io(
                                "create image directory",
                                &directory,
                                error,
                            ));
                        }
                    }
                }
            }
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)
                .map_err(|error| {
                    StorageError::io("create image without overwriting", &target, error)
                })?;
            if let Err(error) = file.write_all(&bytes).and_then(|()| file.sync_all()) {
                drop(file);
                let _ = fs::remove_file(&target);
                return Err(StorageError::io("copy image", &target, error));
            }
            let metadata = file
                .metadata()
                .map_err(|error| StorageError::io("inspect copied image", &target, error))?;
            pending
                .created
                .push((target, FileFingerprint::from_bytes(&bytes, &metadata)));
        }
        Ok(pending)
    }

    /// Recheck missing references immediately before publishing Markdown.
    /// Another process may have restored the source or occupied the target
    /// while the remaining images were being copied.
    pub fn validate_before_publish(&self) -> Result<(), StorageError> {
        for (source, target) in &self.missing {
            for (path, metadata) in [
                (source, fs::metadata(source)),
                (target, fs::symlink_metadata(target)),
            ] {
                match metadata {
                    Err(error) if error.kind() == io::ErrorKind::NotFound => (),
                    Ok(_) => {
                        return Err(StorageError::io(
                            "missing image changed during Save As",
                            path,
                            io::Error::from(io::ErrorKind::Interrupted),
                        ));
                    }
                    Err(error) => {
                        return Err(StorageError::io("recheck missing image", path, error));
                    }
                }
            }
        }
        Ok(())
    }

    pub fn commit(&mut self) {
        self.committed = true;
    }
}

impl Drop for ImageRelocation {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        for (path, expected) in &self.created {
            // Never remove an externally replaced file during rollback.
            if read_image(path)
                .ok()
                .zip(fs::symlink_metadata(path).ok())
                .is_some_and(|(bytes, metadata)| {
                    !metadata.file_type().is_symlink()
                        && FileFingerprint::from_bytes(&bytes, &metadata) == *expected
                })
            {
                let _ = fs::remove_file(path);
            }
        }
        for directory in self.directories.iter().rev() {
            let _ = fs::remove_dir(directory); // only empty directories we created
        }
    }
}

fn parent(path: &Path) -> Result<PathBuf, StorageError> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::canonicalize(parent)
        .map_err(|error| StorageError::io("resolve document directory", parent, error))
}

fn same_resource_location(source: &Path, target: &Path) -> bool {
    fn identity(path: &Path) -> Option<PathBuf> {
        match fs::canonicalize(path) {
            Ok(path) => Some(path),
            // Preserve a shared missing image only when its actual containing
            // directory resolves identically. Do not invent missing ancestors.
            Err(error) if error.kind() == io::ErrorKind::NotFound => Some(
                fs::canonicalize(path.parent()?)
                    .ok()?
                    .join(path.file_name()?),
            ),
            Err(_) => None,
        }
    }
    matches!((identity(source), identity(target)), (Some(source), Some(target)) if source == target)
}

/// Resolve leading parent components against the canonical document directory.
/// Keep the written URI unchanged, including in undo history. Interior `..`
/// is rejected rather than lexically cancelling a potentially symlinked path.
fn resource_destination(root: &Path, path: &Path) -> Result<(PathBuf, PathBuf), StorageError> {
    let mut root = root.to_owned();
    let mut relative = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => relative.push(part),
            Component::CurDir => (),
            Component::ParentDir if relative.as_os_str().is_empty() => {
                if !root.pop() {
                    return Err(StorageError::InvalidPath(path.to_owned()));
                }
            }
            _ => return Err(StorageError::InvalidPath(path.to_owned())),
        }
    }
    if relative.as_os_str().is_empty() {
        return Err(StorageError::InvalidPath(path.to_owned()));
    }
    Ok((root, relative))
}

fn check_parents(root: &Path, relative: &Path) -> Result<(), StorageError> {
    let mut path = root.to_owned();
    if let Some(parent) = relative.parent() {
        for component in parent.components() {
            path.push(component);
            match fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => (),
                Ok(_) => return Err(StorageError::InvalidPath(path)),
                Err(error) if error.kind() == io::ErrorKind::NotFound => (),
                Err(error) => {
                    return Err(StorageError::io("inspect image directory", &path, error));
                }
            }
        }
    }
    Ok(())
}

fn read_image(path: &Path) -> Result<Vec<u8>, StorageError> {
    const LIMIT: u64 = 128 * 1024 * 1024;
    let file = fs::File::open(path).map_err(|error| StorageError::io("read image", path, error))?;
    let metadata = file
        .metadata()
        .map_err(|error| StorageError::io("inspect image", path, error))?;
    if !metadata.is_file() || metadata.len() > LIMIT {
        return Err(StorageError::io(
            "image is not a regular file within the resource budget",
            path,
            io::Error::from(io::ErrorKind::InvalidData),
        ));
    }
    let mut bytes = Vec::new();
    file.take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| StorageError::io("read image", path, error))?;
    if bytes.len() as u64 > LIMIT {
        return Err(StorageError::InvalidPath(path.to_owned()));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use yu_core::{ByteOffset, TextRange};

    #[test]
    fn uncommitted_resources_roll_back_without_deleting_external_replacements() {
        let root = std::env::temp_dir().join(format!(
            "yu-image-rollback-{}-{}",
            std::process::id(),
            super::super::TEMP_FILE_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("old/assets")).expect("old resources");
        fs::create_dir(root.join("new")).expect("new document directory");
        fs::write(root.join("old/assets/a.png"), b"a").expect("first image");
        fs::write(root.join("old/assets/b.png"), b"b").expect("second image");
        let references = ["assets/a.png", "assets/b.png"].map(|destination| ImageReference {
            source: TextRange::empty(ByteOffset::ZERO),
            label: TextRange::empty(ByteOffset::ZERO),
            destination: TextRange::empty(ByteOffset::ZERO),
            destination_text: destination.to_owned(),
        });
        {
            let _pending = ImageRelocation::prepare(
                &root.join("old/note.md"),
                &root.join("new/copy.md"),
                &references,
                std::iter::empty(),
            )
            .expect("stage resources");
            assert!(root.join("new/assets/a.png").is_file());
            // Simulate a document-save failure plus an external resource edit.
            fs::write(root.join("new/assets/b.png"), b"external replacement")
                .expect("external edit");
        }
        assert!(!root.join("new/assets/a.png").exists());
        assert_eq!(
            fs::read(root.join("new/assets/b.png")).expect("external resource"),
            b"external replacement"
        );
        assert_eq!(
            fs::read(root.join("old/assets/a.png")).expect("original image"),
            b"a"
        );
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn missing_resource_recheck_rejects_files_created_during_preparation() {
        for changed_side in ["old", "new"] {
            let root = std::env::temp_dir().join(format!(
                "yu-image-missing-race-{}-{}",
                std::process::id(),
                super::super::TEMP_FILE_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            fs::create_dir(&root).expect("private root");
            fs::create_dir_all(root.join("old/assets")).expect("source directory");
            fs::create_dir_all(root.join("new/assets")).expect("target directory");
            fs::write(root.join("old/assets/valid.png"), b"valid resource").expect("source");
            let references =
                ["assets/missing.png", "assets/valid.png"].map(|destination| ImageReference {
                    source: TextRange::empty(ByteOffset::ZERO),
                    label: TextRange::empty(ByteOffset::ZERO),
                    destination: TextRange::empty(ByteOffset::ZERO),
                    destination_text: destination.to_owned(),
                });
            let pending = ImageRelocation::prepare(
                &root.join("old/note.md"),
                &root.join("new/copy.md"),
                &references,
                std::iter::empty(),
            )
            .expect("prepare");
            assert!(pending.validate_before_publish().is_ok());
            let external = root.join(changed_side).join("assets/missing.png");
            fs::write(&external, b"external file").expect("external restoration");
            assert!(pending.validate_before_publish().is_err());
            drop(pending);
            assert!(!root.join("new/assets/valid.png").exists());
            assert_eq!(
                fs::read(&external).expect("preserve external file"),
                b"external file"
            );
            fs::remove_dir_all(root).expect("cleanup");
        }
    }
}
