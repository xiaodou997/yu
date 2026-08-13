use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// A coarse event produced by a platform file watcher.
///
/// The event is intentionally not treated as proof that the file changed.
/// `DocumentSession::disk_state` remains the authority after the debounce
/// window, because atomic saves and editor-specific watcher events can report
/// several intermediate names for one logical replacement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileWatchReason {
    Created,
    Modified,
    Removed,
    Renamed,
    Unknown,
}

/// A platform watcher notification associated with one path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileWatchEvent {
    path: PathBuf,
    reason: FileWatchReason,
}

impl FileWatchEvent {
    #[must_use]
    pub fn new(path: impl Into<PathBuf>, reason: FileWatchReason) -> Self {
        Self {
            path: path.into(),
            reason,
        }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub const fn reason(&self) -> FileWatchReason {
        self.reason
    }
}

/// The debounced result that asks a session to compare its file fingerprint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileWatchCheck {
    path: PathBuf,
    reason: FileWatchReason,
}

impl FileWatchCheck {
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub const fn reason(&self) -> FileWatchReason {
        self.reason
    }
}

#[derive(Clone, Copy, Debug)]
struct PendingCheck {
    reason: FileWatchReason,
    deadline: Instant,
}

/// Coalesces noisy platform events for one document path.
///
/// It has no thread, timer, or operating-system dependency. A macOS FSEvents,
/// DispatchSource, or future cross-platform adapter forwards events through
/// [`Self::observe`] and schedules its own wake-up for the returned deadline.
/// Calling [`Self::poll`] after that deadline yields one check request.
#[derive(Clone, Debug)]
pub struct FileWatchDebouncer {
    path: PathBuf,
    debounce: Duration,
    pending: Option<PendingCheck>,
}

impl FileWatchDebouncer {
    #[must_use]
    pub fn new(path: impl Into<PathBuf>, debounce: Duration) -> Self {
        Self {
            path: path.into(),
            debounce,
            pending: None,
        }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub const fn debounce(&self) -> Duration {
        self.debounce
    }

    /// Records a matching event and resets the debounce deadline.
    ///
    /// Returns `false` for an unrelated path. The caller can use the boolean
    /// to avoid scheduling a platform timer for another document.
    pub fn observe(&mut self, event: &FileWatchEvent, now: Instant) -> bool {
        if event.path != self.path {
            return false;
        }
        let deadline = now.checked_add(self.debounce).unwrap_or(now);
        self.pending = Some(match self.pending {
            Some(previous) => PendingCheck {
                reason: stronger_reason(previous.reason, event.reason),
                deadline,
            },
            None => PendingCheck {
                reason: event.reason,
                deadline,
            },
        });
        true
    }

    /// Returns a check request only after the quiet period has elapsed.
    pub fn poll(&mut self, now: Instant) -> Option<FileWatchCheck> {
        let pending = self.pending?;
        if now < pending.deadline {
            return None;
        }
        self.pending = None;
        Some(FileWatchCheck {
            path: self.path.clone(),
            reason: pending.reason,
        })
    }

    /// Immediately emits the pending check, useful when a watcher is stopped
    /// while a document is being closed.
    pub fn flush(&mut self) -> Option<FileWatchCheck> {
        let pending = self.pending.take()?;
        Some(FileWatchCheck {
            path: self.path.clone(),
            reason: pending.reason,
        })
    }
}

fn stronger_reason(left: FileWatchReason, right: FileWatchReason) -> FileWatchReason {
    if reason_priority(right) > reason_priority(left) {
        right
    } else {
        left
    }
}

const fn reason_priority(reason: FileWatchReason) -> u8 {
    match reason {
        // An ambiguous/drop notification must win over a more specific event
        // in the same burst: the caller should not infer a lifecycle reason
        // after the native watcher has told us its history is incomplete.
        FileWatchReason::Unknown => 5,
        FileWatchReason::Modified => 1,
        FileWatchReason::Created => 2,
        FileWatchReason::Removed => 3,
        FileWatchReason::Renamed => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unrelated_events_are_ignored_and_matching_events_debounce() {
        let path = PathBuf::from("/tmp/note.md");
        let other = PathBuf::from("/tmp/other.md");
        let mut debouncer = FileWatchDebouncer::new(path.clone(), Duration::from_millis(50));
        let start = Instant::now();

        assert!(!debouncer.observe(
            &FileWatchEvent::new(other, FileWatchReason::Modified),
            start
        ));
        assert!(debouncer.observe(
            &FileWatchEvent::new(path.clone(), FileWatchReason::Modified),
            start
        ));
        assert!(debouncer.poll(start + Duration::from_millis(49)).is_none());
        assert_eq!(
            debouncer.poll(start + Duration::from_millis(50)),
            Some(FileWatchCheck {
                path,
                reason: FileWatchReason::Modified,
            })
        );
    }

    #[test]
    fn bursts_reset_deadline_and_keep_the_strongest_reason() {
        let path = PathBuf::from("note.md");
        let mut debouncer = FileWatchDebouncer::new(path.clone(), Duration::from_secs(1));
        let start = Instant::now();
        debouncer.observe(
            &FileWatchEvent::new(path.clone(), FileWatchReason::Modified),
            start,
        );
        debouncer.observe(
            &FileWatchEvent::new(path.clone(), FileWatchReason::Removed),
            start + Duration::from_millis(500),
        );
        assert!(
            debouncer
                .poll(start + Duration::from_millis(1_499))
                .is_none()
        );
        assert_eq!(
            debouncer.poll(start + Duration::from_millis(1_500)),
            Some(FileWatchCheck {
                path,
                reason: FileWatchReason::Removed,
            })
        );
    }

    #[test]
    fn ambiguous_event_wins_over_specific_event_in_one_burst() {
        let path = PathBuf::from("note.md");
        let mut debouncer = FileWatchDebouncer::new(path.clone(), Duration::from_secs(1));
        let start = Instant::now();
        debouncer.observe(
            &FileWatchEvent::new(path.clone(), FileWatchReason::Modified),
            start,
        );
        debouncer.observe(
            &FileWatchEvent::new(path.clone(), FileWatchReason::Unknown),
            start + Duration::from_millis(1),
        );
        assert_eq!(
            debouncer.poll(start + Duration::from_millis(1_001)),
            Some(FileWatchCheck {
                path,
                reason: FileWatchReason::Unknown,
            })
        );
    }

    #[test]
    fn flush_emits_pending_check_once() {
        let path = PathBuf::from("note.md");
        let mut debouncer = FileWatchDebouncer::new(path.clone(), Duration::from_secs(5));
        debouncer.observe(
            &FileWatchEvent::new(path.clone(), FileWatchReason::Renamed),
            Instant::now(),
        );
        assert_eq!(
            debouncer.flush(),
            Some(FileWatchCheck {
                path,
                reason: FileWatchReason::Renamed,
            })
        );
        assert!(debouncer.flush().is_none());
    }
}
