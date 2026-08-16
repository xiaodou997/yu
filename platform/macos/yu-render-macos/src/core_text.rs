//! CoreText-backed preparation of revision-bound workspace frames.
//!
//! This module is the first Rust-owned connection between the macOS shaping
//! adapter and the Metal backend. It keeps a CoreText shaper, CPU glyph atlas,
//! render-plan fingerprint cache and workspace publisher alive across frames;
//! none of those objects become part of the canonical editor document.

use std::error::Error;
use std::fmt;

use yu_editor::{EditorDocument, EditorDocumentError};
use yu_font::{
    AtlasError, FontRequest, GlyphAtlas, GlyphAtlasConfig, GlyphRasterKey, GlyphRasterizer,
};
use yu_font_macos::{CoreTextFontError, CoreTextRasterError, CoreTextShaper};
use yu_render::RenderPlanBuilder;
use yu_workspace::{
    ViewportFramePublication, ViewportFramePublisher, ViewportPublishError, ViewportRenderConfig,
};

use crate::{
    MetalAtlas, MetalFrameRenderer, MetalSurface, MetalUploader, MetalViewportHostError,
    MetalViewportHostSession, MetalViewportHostSubmission,
};

/// Errors raised while preparing a CoreText-backed workspace frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CoreTextViewportFrameError {
    InvalidConfig(&'static str),
    Font(CoreTextFontError),
    Document(EditorDocumentError),
    Raster(CoreTextRasterError),
    Atlas(AtlasError),
    Publish(ViewportPublishError),
    Host(MetalViewportHostError),
}

impl fmt::Display for CoreTextViewportFrameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig(message) => formatter.write_str(message),
            Self::Font(error) => error.fmt(formatter),
            Self::Document(error) => error.fmt(formatter),
            Self::Raster(error) => error.fmt(formatter),
            Self::Atlas(error) => error.fmt(formatter),
            Self::Publish(error) => error.fmt(formatter),
            Self::Host(error) => error.fmt(formatter),
        }
    }
}

impl Error for CoreTextViewportFrameError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Font(error) => Some(error),
            Self::Document(error) => Some(error),
            Self::Raster(error) => Some(error),
            Self::Atlas(error) => Some(error),
            Self::Publish(error) => Some(error),
            Self::Host(error) => Some(error),
            Self::InvalidConfig(_) => None,
        }
    }
}

impl From<CoreTextFontError> for CoreTextViewportFrameError {
    fn from(error: CoreTextFontError) -> Self {
        Self::Font(error)
    }
}

impl From<EditorDocumentError> for CoreTextViewportFrameError {
    fn from(error: EditorDocumentError) -> Self {
        Self::Document(error)
    }
}

impl From<CoreTextRasterError> for CoreTextViewportFrameError {
    fn from(error: CoreTextRasterError) -> Self {
        Self::Raster(error)
    }
}

impl From<AtlasError> for CoreTextViewportFrameError {
    fn from(error: AtlasError) -> Self {
        Self::Atlas(error)
    }
}

impl From<ViewportPublishError> for CoreTextViewportFrameError {
    fn from(error: ViewportPublishError) -> Self {
        Self::Publish(error)
    }
}

impl From<MetalViewportHostError> for CoreTextViewportFrameError {
    fn from(error: MetalViewportHostError) -> Self {
        Self::Host(error)
    }
}

/// Persistent Rust preparation state for a macOS CoreText viewport.
///
/// The builder owns no window, Metal texture or editor document. It only
/// retains the platform shaper, CPU atlas, page-fingerprint builder and
/// publication cache needed to prepare successive revision-bound frames.
#[derive(Debug)]
pub struct CoreTextViewportFrameBuilder {
    shaper: CoreTextShaper,
    atlas: GlyphAtlas,
    render_plans: RenderPlanBuilder,
    publisher: ViewportFramePublisher,
    config: ViewportRenderConfig,
}

impl CoreTextViewportFrameBuilder {
    /// Creates a builder from an already configured CoreText shaper.
    pub fn with_shaper(
        shaper: CoreTextShaper,
        config: ViewportRenderConfig,
        atlas_config: GlyphAtlasConfig,
    ) -> Result<Self, CoreTextViewportFrameError> {
        Self::with_shaper_and_initial_serial(shaper, config, atlas_config, 0)
    }

    /// Creates a builder whose first publication continues after an existing
    /// builder serial. The new builder still starts with fresh shaping and
    /// atlas state; only the publication identity is carried across a host
    /// rebuild.
    pub fn with_shaper_and_initial_serial(
        shaper: CoreTextShaper,
        config: ViewportRenderConfig,
        atlas_config: GlyphAtlasConfig,
        initial_serial: u64,
    ) -> Result<Self, CoreTextViewportFrameError> {
        let font_size = config.font_size();
        if !font_size.is_finite() || font_size <= 0.0 {
            return Err(CoreTextViewportFrameError::InvalidConfig(
                "CoreText viewport font size must be finite and positive",
            ));
        }
        if (shaper.request().size() - font_size).abs() > 0.001 {
            return Err(CoreTextViewportFrameError::InvalidConfig(
                "CoreText shaper size must match viewport font size",
            ));
        }
        Ok(Self {
            shaper,
            atlas: GlyphAtlas::new(atlas_config),
            render_plans: RenderPlanBuilder::new(),
            publisher: ViewportFramePublisher::with_next_serial(initial_serial),
            config,
        })
    }

    /// Creates a builder backed by the AppKit/CoreText system UI font.
    pub fn from_system_ui(
        request: FontRequest,
        config: ViewportRenderConfig,
        atlas_config: GlyphAtlasConfig,
    ) -> Result<Self, CoreTextViewportFrameError> {
        let shaper = CoreTextShaper::from_system_ui(request)?;
        Self::with_shaper(shaper, config, atlas_config)
    }

    /// Prepares and publishes the current document viewport.
    pub fn publish(
        &mut self,
        document: &mut EditorDocument,
    ) -> Result<ViewportFramePublication, CoreTextViewportFrameError> {
        self.rasterize_visible_glyphs(document)?;
        self.publisher
            .publish(
                document,
                self.config,
                &self.shaper,
                &self.atlas,
                &mut self.render_plans,
            )
            .map_err(CoreTextViewportFrameError::Publish)
    }

    /// Updates the viewport/scene inputs while retaining the CoreText shaper,
    /// CPU atlas, page-fingerprint cache and publication serial. The font
    /// size is part of a glyph raster key, so callers must recreate the
    /// builder when it changes rather than mixing sizes in one host state.
    pub fn update_config(
        &mut self,
        config: ViewportRenderConfig,
    ) -> Result<bool, CoreTextViewportFrameError> {
        let font_size = config.font_size();
        if !font_size.is_finite() || font_size <= 0.0 {
            return Err(CoreTextViewportFrameError::InvalidConfig(
                "CoreText viewport font size must be finite and positive",
            ));
        }
        if (self.shaper.request().size() - font_size).abs() > 0.001 {
            return Err(CoreTextViewportFrameError::InvalidConfig(
                "CoreText shaper size must match viewport font size",
            ));
        }
        let changed = self.config != config;
        self.config = config;
        Ok(changed)
    }

    /// Prepares, accepts and submits one current document frame through the
    /// existing revision-aware Metal host state machine.
    pub fn publish_and_submit(
        &mut self,
        document: &mut EditorDocument,
        host: &mut MetalViewportHostSession,
        renderer: &mut MetalFrameRenderer,
        surface: &MetalSurface,
        uploader: &mut MetalUploader,
        metal_atlas: &mut MetalAtlas,
    ) -> Result<MetalViewportHostSubmission, CoreTextViewportFrameError> {
        let publication = self.publish(document)?;
        host.accept_publication(publication)?;
        host.submit(renderer, surface, uploader, metal_atlas)
            .map_err(CoreTextViewportFrameError::Host)
    }

    #[must_use]
    pub const fn config(&self) -> ViewportRenderConfig {
        self.config
    }

    /// Borrows the persistent CoreText shaper for host-side metrics checks.
    /// The shaper remains owned by this builder; callers cannot replace it or
    /// move native CoreText state across the boundary.
    #[must_use]
    pub const fn shaper(&self) -> &CoreTextShaper {
        &self.shaper
    }

    #[must_use]
    pub fn atlas(&self) -> &GlyphAtlas {
        &self.atlas
    }

    #[must_use]
    pub fn atlas_page_count(&self) -> usize {
        self.atlas.page_count()
    }

    #[must_use]
    pub fn atlas_glyph_count(&self) -> usize {
        self.atlas.len()
    }

    #[must_use]
    pub const fn atlas_bytes(&self) -> usize {
        self.atlas.bytes()
    }

    #[must_use]
    pub fn last_publication(&self) -> Option<&ViewportFramePublication> {
        self.publisher.last_publication()
    }

    fn rasterize_visible_glyphs(
        &mut self,
        document: &mut EditorDocument,
    ) -> Result<(), CoreTextViewportFrameError> {
        let viewport = if document.composition().is_some() {
            document
                .visible_blocks_with_composition_and_shaper(self.config.viewport(), &self.shaper)?
        } else {
            document.visible_blocks_with_shaper(self.config.viewport(), &self.shaper)?
        };
        let layout_config = document.viewport_config().layout();
        let rasterizer = self.shaper.rasterizer();
        let composition_blocks = document.composition_block_range();
        for block in viewport.blocks() {
            let layout = if composition_blocks
                .as_ref()
                .is_some_and(|span| span.contains(&block.index()))
            {
                document.block_layout_with_composition_and_shaper(
                    block.index(),
                    layout_config,
                    &self.shaper,
                )?
            } else {
                document
                    .block_layout_with_shaper(block.index(), layout_config, &self.shaper)?
                    .clone()
            };
            for placement in layout.glyphs() {
                let key = GlyphRasterKey::new(
                    placement.face(),
                    placement.glyph(),
                    self.config.font_size(),
                )
                .map_err(|_| {
                    CoreTextViewportFrameError::InvalidConfig(
                        "CoreText glyph raster key has an invalid font size",
                    )
                })?;
                if self.atlas.entry(key).is_none() {
                    self.atlas.insert(rasterizer.rasterize(key)?)?;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yu_core::{ByteOffset, TextRange, Utf16Offset, Utf16Range};
    use yu_editor::{EditorCommand, ViewportConfig, ViewportRect};
    use yu_scene::{Rect, Rgba8};

    #[test]
    fn core_text_builder_reuses_atlas_and_render_upload_state() {
        let font_size = 16.0;
        let request = FontRequest::new(".SFNS-Regular", font_size).expect("font request");
        let shaper = CoreTextShaper::from_system_ui(request).expect("CoreText shaper");
        let metrics = shaper.viewport_metrics("A羽🙂").expect("CoreText metrics");
        let mut document = EditorDocument::new("# 羽🙂\n\nhello **world**");
        document
            .set_viewport_config(ViewportConfig::new(
                yu_editor::LayoutConfig::new(320.0, metrics.line_height()),
                28.0,
                0.0,
            ))
            .expect("viewport config");
        let config = ViewportRenderConfig::new(
            ViewportRect::new(0.0, 240.0),
            font_size,
            Rect::new(0.0, 0.0, 320.0, 480.0).expect("scene viewport"),
            Rgba8::black(),
        );
        let mut builder =
            CoreTextViewportFrameBuilder::with_shaper(shaper, config, GlyphAtlasConfig::default())
                .expect("frame builder");

        let first = builder.publish(&mut document).expect("first publication");
        assert_eq!(first.revision(), document.revision());
        assert!(!first.frame().plan().commands().is_empty());
        assert!(!first.frame().plan().uploads().is_empty());
        let page_count = builder.atlas_page_count();
        let glyph_count = builder.atlas_glyph_count();
        assert!(page_count > 0);
        assert!(glyph_count > 0);
        assert!(builder.atlas_bytes() > 0);

        let second = builder.publish(&mut document).expect("cached publication");
        assert_eq!(second.revision(), first.revision());
        assert_eq!(second.serial(), first.serial() + 1);
        assert!(second.frame().plan().uploads().is_empty());
        assert_eq!(builder.atlas_page_count(), page_count);
        assert_eq!(builder.atlas_glyph_count(), glyph_count);

        let source = document.snapshot();
        let composition_start = source.as_str().find("world").expect("composition target");
        let composition_end = composition_start + "world".len();
        document
            .begin_composition(
                TextRange::new(
                    ByteOffset::new(composition_start as u64),
                    ByteOffset::new(composition_end as u64),
                )
                .expect("composition range"),
                "日本🙂",
                Utf16Range::empty(Utf16Offset::new(2)),
            )
            .expect("composition");
        let composed = builder
            .publish(&mut document)
            .expect("composition publication");
        assert_eq!(composed.revision(), document.revision());
        assert_ne!(
            composed.frame().plan().commands().len(),
            second.frame().plan().commands().len()
        );
        assert!(builder.atlas_glyph_count() >= glyph_count);

        assert!(document.cancel_composition());
        let cancelled = builder.publish(&mut document).expect("cancel publication");
        assert_eq!(cancelled.revision(), document.revision());
        assert_eq!(document.composition(), None);

        document
            .execute(EditorCommand::insert_text("界"))
            .expect("document edit");
        let third = builder.publish(&mut document).expect("edited publication");
        assert_eq!(third.revision(), document.revision());
        assert!(!third.frame().plan().uploads().is_empty());
        assert!(builder.atlas_glyph_count() > glyph_count);
    }
}
