#![forbid(unsafe_code)]

//! macOS file-watch event normalization for Yu document sessions.
//!
//! This crate deliberately does not own a thread, run loop, file descriptor,
//! FSEvents stream, or AppKit window. A native shell may use FSEvents or a
//! DispatchSource vnode watcher and forward its callbacks here. The adapter
//! translates native bitmasks into the platform-neutral `yu-storage` event,
//! then applies the shared debounce policy. The session still performs the
//! authoritative fingerprint check after [`MacosFileWatchAdapter::poll`].

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use yu_storage::{FileWatchCheck, FileWatchDebouncer, FileWatchEvent, FileWatchReason};

/// FSEvents flags used to classify an item notification.
pub mod fsevents_flags {
    pub const MUST_SCAN_SUB_DIRS: u32 = 0x0000_0001;
    pub const USER_DROPPED: u32 = 0x0000_0002;
    pub const KERNEL_DROPPED: u32 = 0x0000_0004;
    pub const EVENT_IDS_WRAPPED: u32 = 0x0000_0008;
    pub const HISTORY_DONE: u32 = 0x0000_0010;
    pub const ROOT_CHANGED: u32 = 0x0000_0020;
    pub const MOUNT: u32 = 0x0000_0040;
    pub const UNMOUNT: u32 = 0x0000_0080;
    pub const ITEM_CREATED: u32 = 0x0000_0100;
    pub const ITEM_REMOVED: u32 = 0x0000_0200;
    pub const INODE_META_MOD: u32 = 0x0000_0400;
    pub const ITEM_RENAMED: u32 = 0x0000_0800;
    pub const ITEM_MODIFIED: u32 = 0x0000_1000;
    pub const FINDER_INFO_MOD: u32 = 0x0000_2000;
    pub const ITEM_CHANGE_OWNER: u32 = 0x0000_4000;
    pub const ITEM_XATTR_MOD: u32 = 0x0000_8000;
    pub const ITEM_IS_FILE: u32 = 0x0001_0000;
    pub const ITEM_IS_DIR: u32 = 0x0002_0000;
    pub const ITEM_IS_SYMLINK: u32 = 0x0004_0000;
    pub const OWN_EVENT: u32 = 0x0008_0000;
    pub const ITEM_IS_HARDLINK: u32 = 0x0010_0000;
    pub const ITEM_IS_LAST_HARDLINK: u32 = 0x0020_0000;
    pub const ITEM_CLONED: u32 = 0x0040_0000;
}

/// DispatchSource vnode flags used by a file descriptor watcher.
pub mod dispatch_vnode_flags {
    pub const DELETE: usize = 0x0000_0001;
    pub const WRITE: usize = 0x0000_0002;
    pub const EXTEND: usize = 0x0000_0004;
    pub const ATTRIB: usize = 0x0000_0008;
    pub const LINK: usize = 0x0000_0010;
    pub const RENAME: usize = 0x0000_0020;
    pub const REVOKE: usize = 0x0000_0040;
}

/// Native source that generated one macOS callback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MacosWatchSource {
    Fsevents,
    DispatchSourceVnode,
}

/// A native event before it is translated into Yu's platform-neutral event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MacosNativeFileEvent {
    source: MacosWatchSource,
    flags: usize,
}

impl MacosNativeFileEvent {
    #[must_use]
    pub const fn fsevents(flags: u32) -> Self {
        Self {
            source: MacosWatchSource::Fsevents,
            flags: flags as usize,
        }
    }

    #[must_use]
    pub const fn dispatch_source_vnode(flags: usize) -> Self {
        Self {
            source: MacosWatchSource::DispatchSourceVnode,
            flags,
        }
    }

    #[must_use]
    pub const fn source(self) -> MacosWatchSource {
        self.source
    }

    #[must_use]
    pub const fn flags(self) -> usize {
        self.flags
    }

    #[must_use]
    pub fn reason(self) -> FileWatchReason {
        match self.source {
            MacosWatchSource::Fsevents => fsevents_reason(self.flags as u32),
            MacosWatchSource::DispatchSourceVnode => dispatch_vnode_reason(self.flags),
        }
    }
}

/// macOS-specific adapter around the shared Yu file-watch debouncer.
#[derive(Clone, Debug)]
pub struct MacosFileWatchAdapter {
    path: PathBuf,
    debouncer: FileWatchDebouncer,
}

impl MacosFileWatchAdapter {
    #[must_use]
    pub fn new(path: impl Into<PathBuf>, debounce: Duration) -> Self {
        let path = path.into();
        Self {
            debouncer: FileWatchDebouncer::new(path.clone(), debounce),
            path,
        }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub const fn debounce(&self) -> Duration {
        self.debouncer.debounce()
    }

    /// Forwards one FSEvents callback into the shared debounce state.
    pub fn observe_fsevents(
        &mut self,
        event_path: impl Into<PathBuf>,
        flags: u32,
        now: Instant,
    ) -> bool {
        self.observe(
            event_path.into(),
            MacosNativeFileEvent::fsevents(flags),
            now,
        )
    }

    /// Forwards one DispatchSource vnode callback into the shared debounce
    /// state.
    pub fn observe_dispatch_source_vnode(
        &mut self,
        event_path: impl Into<PathBuf>,
        flags: usize,
        now: Instant,
    ) -> bool {
        self.observe(
            event_path.into(),
            MacosNativeFileEvent::dispatch_source_vnode(flags),
            now,
        )
    }

    /// Forwards a pre-classified native callback into the shared debounce
    /// state. Native source ownership and callback threading remain outside
    /// this crate.
    pub fn observe(
        &mut self,
        event_path: PathBuf,
        event: MacosNativeFileEvent,
        now: Instant,
    ) -> bool {
        self.debouncer
            .observe(&FileWatchEvent::new(event_path, event.reason()), now)
    }

    /// Returns one debounced check request after the quiet period.
    pub fn poll(&mut self, now: Instant) -> Option<FileWatchCheck> {
        self.debouncer.poll(now)
    }

    /// Flushes a pending request while the native watcher is being stopped.
    pub fn flush(&mut self) -> Option<FileWatchCheck> {
        self.debouncer.flush()
    }
}

fn fsevents_reason(flags: u32) -> FileWatchReason {
    use fsevents_flags as flag;
    let ambiguous = flag::MUST_SCAN_SUB_DIRS
        | flag::USER_DROPPED
        | flag::KERNEL_DROPPED
        | flag::EVENT_IDS_WRAPPED
        | flag::ROOT_CHANGED
        | flag::MOUNT
        | flag::UNMOUNT;
    if flags & ambiguous != 0 {
        return FileWatchReason::Unknown;
    }
    if flags & flag::ITEM_REMOVED != 0 {
        return FileWatchReason::Removed;
    }
    if flags & flag::ITEM_RENAMED != 0 {
        return FileWatchReason::Renamed;
    }
    if flags & flag::ITEM_CREATED != 0 {
        return FileWatchReason::Created;
    }
    let modified = flag::INODE_META_MOD
        | flag::ITEM_MODIFIED
        | flag::FINDER_INFO_MOD
        | flag::ITEM_CHANGE_OWNER
        | flag::ITEM_XATTR_MOD
        | flag::ITEM_CLONED;
    if flags & modified != 0 {
        return FileWatchReason::Modified;
    }
    FileWatchReason::Unknown
}

fn dispatch_vnode_reason(flags: usize) -> FileWatchReason {
    use dispatch_vnode_flags as flag;
    if flags & flag::REVOKE != 0 {
        return FileWatchReason::Unknown;
    }
    if flags & flag::DELETE != 0 {
        return FileWatchReason::Removed;
    }
    if flags & flag::RENAME != 0 {
        return FileWatchReason::Renamed;
    }
    if flags & (flag::WRITE | flag::EXTEND | flag::ATTRIB | flag::LINK) != 0 {
        return FileWatchReason::Modified;
    }
    FileWatchReason::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fsevents_mapping_prioritizes_file_lifecycle_and_rejects_ambiguous_flags() {
        assert_eq!(
            MacosNativeFileEvent::fsevents(fsevents_flags::ITEM_RENAMED).reason(),
            FileWatchReason::Renamed
        );
        assert_eq!(
            MacosNativeFileEvent::fsevents(fsevents_flags::ITEM_REMOVED).reason(),
            FileWatchReason::Removed
        );
        assert_eq!(
            MacosNativeFileEvent::fsevents(fsevents_flags::ITEM_MODIFIED).reason(),
            FileWatchReason::Modified
        );
        assert_eq!(
            MacosNativeFileEvent::fsevents(
                fsevents_flags::MUST_SCAN_SUB_DIRS | fsevents_flags::ITEM_MODIFIED
            )
            .reason(),
            FileWatchReason::Unknown
        );
    }

    #[test]
    fn dispatch_vnode_mapping_handles_atomic_replace_signals() {
        assert_eq!(
            MacosNativeFileEvent::dispatch_source_vnode(dispatch_vnode_flags::RENAME).reason(),
            FileWatchReason::Renamed
        );
        assert_eq!(
            MacosNativeFileEvent::dispatch_source_vnode(dispatch_vnode_flags::DELETE).reason(),
            FileWatchReason::Removed
        );
        assert_eq!(
            MacosNativeFileEvent::dispatch_source_vnode(dispatch_vnode_flags::WRITE).reason(),
            FileWatchReason::Modified
        );
        assert_eq!(
            MacosNativeFileEvent::dispatch_source_vnode(dispatch_vnode_flags::REVOKE).reason(),
            FileWatchReason::Unknown
        );
    }

    #[test]
    fn adapter_filters_paths_and_debounces_native_events() {
        let path = PathBuf::from("/tmp/note.md");
        let mut adapter = MacosFileWatchAdapter::new(path.clone(), Duration::from_millis(20));
        let start = Instant::now();
        assert!(!adapter.observe_fsevents("/tmp/other.md", fsevents_flags::ITEM_MODIFIED, start));
        assert!(adapter.observe_dispatch_source_vnode(
            path.clone(),
            dispatch_vnode_flags::RENAME,
            start
        ));
        assert!(adapter.poll(start + Duration::from_millis(19)).is_none());
        let check = adapter
            .poll(start + Duration::from_millis(20))
            .expect("debounced watcher check");
        assert_eq!(check.path(), path);
        assert_eq!(check.reason(), FileWatchReason::Renamed);
    }
}
