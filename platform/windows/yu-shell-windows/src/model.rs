use std::path::{Path, PathBuf};

use yu_storage::{
    CloseRequest, CloseStateError, CloseTransition, DocumentEditorSession, SaveOutcome,
    StorageError,
};
use yu_workspace::Appearance;

use crate::{Locale, Strings};

const DEFAULT_DPI: u32 = 96;
const UNTITLED_PLACEHOLDER: &str = "Untitled.md";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SidebarMode {
    Hidden,
    #[default]
    Files,
    Outline,
    Search,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowMetrics {
    width_px: u32,
    height_px: u32,
    dpi: u32,
}

impl Default for WindowMetrics {
    fn default() -> Self {
        Self {
            width_px: 1200,
            height_px: 800,
            dpi: DEFAULT_DPI,
        }
    }
}

impl WindowMetrics {
    #[must_use]
    pub const fn new(width_px: u32, height_px: u32, dpi: u32) -> Self {
        Self {
            width_px,
            height_px,
            dpi: if dpi == 0 { DEFAULT_DPI } else { dpi },
        }
    }

    #[must_use]
    pub const fn width_px(self) -> u32 {
        self.width_px
    }

    #[must_use]
    pub const fn height_px(self) -> u32 {
        self.height_px
    }

    #[must_use]
    pub const fn dpi(self) -> u32 {
        self.dpi
    }

    #[must_use]
    pub fn scale(self) -> f32 {
        self.dpi as f32 / DEFAULT_DPI as f32
    }

    #[must_use]
    pub fn logical_width(self) -> f32 {
        self.width_px as f32 / self.scale()
    }

    #[must_use]
    pub fn logical_height(self) -> f32 {
        self.height_px as f32 / self.scale()
    }

    #[must_use]
    pub fn px(self, logical: f32) -> i32 {
        (logical * self.scale()).round() as i32
    }
}

#[derive(Debug)]
pub struct DocumentSlot {
    identity: u64,
    session: DocumentEditorSession,
    untitled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveAction {
    Saved(SaveOutcome),
    NeedsDestination,
}

impl DocumentSlot {
    fn next_identity() -> u64 {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }

    pub const fn identity(&self) -> u64 {
        self.identity
    }

    #[must_use]
    pub fn new_untitled() -> Self {
        Self {
            identity: Self::next_identity(),
            session: DocumentEditorSession::new(PathBuf::from(UNTITLED_PLACEHOLDER), ""),
            untitled: true,
        }
    }

    pub fn open(path: impl Into<PathBuf>) -> Result<Self, StorageError> {
        Ok(Self {
            identity: Self::next_identity(),
            session: DocumentEditorSession::open(path)?,
            untitled: false,
        })
    }

    #[must_use]
    pub const fn session(&self) -> &DocumentEditorSession {
        &self.session
    }

    pub fn session_mut(&mut self) -> &mut DocumentEditorSession {
        &mut self.session
    }

    #[must_use]
    pub const fn is_untitled(&self) -> bool {
        self.untitled
    }

    #[must_use]
    pub fn display_name(&self, strings: Strings) -> &str {
        if self.untitled {
            strings.untitled()
        } else {
            self.session
                .path()
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(strings.untitled())
        }
    }

    pub fn save(&mut self) -> Result<SaveAction, StorageError> {
        if self.untitled {
            return Ok(SaveAction::NeedsDestination);
        }
        self.session.save().map(SaveAction::Saved)
    }

    pub fn save_as(
        &mut self,
        path: impl Into<PathBuf>,
        replace_existing: bool,
    ) -> Result<SaveOutcome, StorageError> {
        let outcome = self.session.save_as(path, replace_existing)?;
        self.untitled = false;
        Ok(outcome)
    }

    pub fn close_request(&mut self) -> Result<CloseRequest, StorageError> {
        self.session.close_request()
    }

    pub fn cancel_close(&mut self) -> Result<CloseTransition, CloseStateError> {
        self.session.cancel_close()
    }

    pub fn discard_close(&mut self) -> Result<CloseTransition, CloseStateError> {
        self.session.discard_close()
    }

    pub fn save_close(&mut self) -> Result<CloseTransition, StorageError> {
        self.session.save_close()
    }

    pub fn save_as_close(
        &mut self,
        path: impl Into<PathBuf>,
        replace_existing: bool,
    ) -> Result<CloseTransition, StorageError> {
        self.session.save_as_close(path, replace_existing)
    }
}

#[derive(Debug)]
pub struct ShellState {
    locale: Locale,
    appearance: Appearance,
    sidebar: SidebarMode,
    metrics: WindowMetrics,
    document: DocumentSlot,
}

impl ShellState {
    #[must_use]
    pub fn new(locale: Locale) -> Self {
        Self {
            locale,
            appearance: Appearance::Light,
            sidebar: SidebarMode::Files,
            metrics: WindowMetrics::default(),
            document: DocumentSlot::new_untitled(),
        }
    }

    pub fn from_path(locale: Locale, path: &Path) -> Result<Self, StorageError> {
        let mut state = Self::new(locale);
        state.document = DocumentSlot::open(path)?;
        Ok(state)
    }

    #[must_use]
    pub const fn locale(&self) -> Locale {
        self.locale
    }

    #[must_use]
    pub const fn strings(&self) -> Strings {
        self.locale.strings()
    }

    #[must_use]
    pub const fn appearance(&self) -> Appearance {
        self.appearance
    }

    pub fn set_appearance(&mut self, appearance: Appearance) {
        self.appearance = appearance;
    }

    #[must_use]
    pub const fn sidebar(&self) -> SidebarMode {
        self.sidebar
    }

    pub fn set_sidebar(&mut self, sidebar: SidebarMode) {
        self.sidebar = sidebar;
    }

    pub fn toggle_sidebar(&mut self) {
        self.sidebar = if self.sidebar == SidebarMode::Hidden {
            SidebarMode::Files
        } else {
            SidebarMode::Hidden
        };
    }

    #[must_use]
    pub const fn metrics(&self) -> WindowMetrics {
        self.metrics
    }

    pub fn set_metrics(&mut self, metrics: WindowMetrics) {
        self.metrics = metrics;
    }

    #[must_use]
    pub const fn document(&self) -> &DocumentSlot {
        &self.document
    }

    pub fn document_mut(&mut self) -> &mut DocumentSlot {
        &mut self.document
    }

    pub fn new_document(&mut self) {
        self.document = DocumentSlot::new_untitled();
    }

    pub fn open_document(&mut self, path: impl Into<PathBuf>) -> Result<(), StorageError> {
        self.document = DocumentSlot::open(path)?;
        Ok(())
    }

    pub fn replace_document(&mut self, document: DocumentSlot) {
        self.document = document;
    }

    #[must_use]
    pub fn window_title(&self) -> String {
        let strings = self.strings();
        let dirty = if self.document.session().is_dirty() {
            " •"
        } else {
            ""
        };
        format!(
            "{}{} — {}",
            self.document.display_name(strings),
            dirty,
            strings.app_name()
        )
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use yu_storage::ClosePrompt;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("yu-shell-windows-{}-{name}", std::process::id()))
    }

    #[test]
    fn dpi_metrics_preserve_logical_size() {
        let hundred = WindowMetrics::new(1200, 800, 96);
        let one_fifty = WindowMetrics::new(1800, 1200, 144);
        assert_eq!(hundred.logical_width(), one_fifty.logical_width());
        assert_eq!(hundred.logical_height(), one_fifty.logical_height());
        assert_eq!(one_fifty.px(240.0), 360);
    }

    #[test]
    fn untitled_never_saves_to_the_placeholder_without_save_as() {
        let mut document = DocumentSlot::new_untitled();
        assert_eq!(
            document.save().expect("save decision"),
            SaveAction::NeedsDestination
        );
        assert!(document.session().is_dirty());
    }

    #[test]
    fn save_as_establishes_real_document_identity() {
        let path = temp_path("save-as.md");
        let _ = fs::remove_file(&path);
        let mut document = DocumentSlot::new_untitled();
        document.save_as(&path, false).expect("save as");
        assert!(!document.is_untitled());
        assert_eq!(document.session().path(), path.as_path());
        assert!(!document.session().is_dirty());
        assert_eq!(fs::read_to_string(&path).expect("saved source"), "");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn open_uses_the_shared_storage_session_and_clean_close_is_immediate() {
        let path = temp_path("open.md");
        fs::write(&path, "# 羽\n").expect("fixture");
        let mut document = DocumentSlot::open(&path).expect("open");
        assert_eq!(document.session().snapshot().as_str(), "# 羽\n");
        assert_eq!(
            document.close_request().expect("close"),
            CloseRequest::CloseNow
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn new_document_close_uses_the_shared_close_state_machine() {
        let mut document = DocumentSlot::new_untitled();
        assert_eq!(
            document.close_request().expect("close"),
            CloseRequest::Prompt(ClosePrompt::SaveChanges)
        );
        assert_eq!(
            document.cancel_close().expect("cancel"),
            CloseTransition::Cancelled
        );
    }

    #[test]
    fn title_tracks_locale_dirty_and_document_identity() {
        let mut state = ShellState::new(Locale::SimplifiedChinese);
        assert_eq!(state.window_title(), "未命名 • — Yu");

        let path = temp_path("title.md");
        fs::write(&path, "hello").expect("fixture");
        state.open_document(&path).expect("open");
        let expected = format!(
            "{} — Yu",
            path.file_name()
                .and_then(|name| name.to_str())
                .expect("utf-8 temp name")
        );
        assert_eq!(state.window_title(), expected);
        let _ = fs::remove_file(path);
    }
}
