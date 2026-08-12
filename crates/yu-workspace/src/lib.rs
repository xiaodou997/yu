#![forbid(unsafe_code)]

//! Product-facing integration between the editor model and retained scenes.
//!
//! `yu-editor` owns canonical source, Markdown, viewport measurements and
//! block-local layout caches. This crate is the first layer allowed to combine
//! those results with `yu-scene`/`yu-render`; neither side needs to depend on
//! the other.

use std::error::Error;
use std::fmt;

use yu_core::Revision;
use yu_editor::{EditorDocument, EditorDocumentError, ShapingProvider, ViewportRect};
use yu_font::GlyphAtlas;
use yu_scene::{
    Rect, Rgba8, Scene, SceneBuilder, SceneError, ViewportBlockGeometry, ViewportSceneInput,
};

/// A validated scene together with the viewport metadata that produced it.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewportSceneFrame {
    input: ViewportSceneInput,
    scene: Scene,
}

impl ViewportSceneFrame {
    #[must_use]
    pub fn revision(&self) -> Revision {
        self.scene.revision()
    }

    #[must_use]
    pub fn input(&self) -> &ViewportSceneInput {
        &self.input
    }

    #[must_use]
    pub fn scene(&self) -> &Scene {
        &self.scene
    }

    #[must_use]
    pub fn into_parts(self) -> (ViewportSceneInput, Scene) {
        (self.input, self.scene)
    }
}

/// Errors raised while assembling an editor viewport into a retained scene.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ViewportSceneError {
    Document(EditorDocumentError),
    Scene(SceneError),
}

impl fmt::Display for ViewportSceneError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Document(error) => error.fmt(formatter),
            Self::Scene(error) => error.fmt(formatter),
        }
    }
}

impl Error for ViewportSceneError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Document(error) => Some(error),
            Self::Scene(error) => Some(error),
        }
    }
}

impl From<EditorDocumentError> for ViewportSceneError {
    fn from(error: EditorDocumentError) -> Self {
        Self::Document(error)
    }
}

impl From<SceneError> for ViewportSceneError {
    fn from(error: SceneError) -> Self {
        Self::Scene(error)
    }
}

/// Measures the current editor viewport, converts its block metadata into the
/// scene boundary, materializes matching shaped block layouts, and appends
/// them atomically through `SceneBuilder::append_viewport`.
pub fn assemble_viewport_scene<S: ShapingProvider>(
    document: &mut EditorDocument,
    viewport: ViewportRect,
    shaper: &S,
    font_size: f32,
    scene_viewport: Rect,
    atlas: &GlyphAtlas,
    color: Rgba8,
) -> Result<ViewportSceneFrame, ViewportSceneError> {
    let viewport_snapshot = document.visible_blocks_with_shaper(viewport, shaper)?;
    let revision = viewport_snapshot.revision();
    let config = document.viewport_config().layout();
    let geometries = viewport_snapshot
        .blocks()
        .iter()
        .copied()
        .map(|block| {
            ViewportBlockGeometry::new(
                revision,
                block.index(),
                block.source(),
                block.y(),
                block.height(),
                block.is_measured(),
                block.kind().viewport_tag(),
            )
        })
        .collect::<Result<Vec<_>, SceneError>>()?;
    let input = ViewportSceneInput::new(
        revision,
        viewport_snapshot.range().start()..viewport_snapshot.range().end(),
        viewport_snapshot.content_height(),
        geometries,
    )?;

    let mut layouts = Vec::with_capacity(viewport_snapshot.blocks().len());
    for block in viewport_snapshot.blocks() {
        layouts.push(
            document
                .block_layout_with_shaper(block.index(), config, shaper)?
                .clone(),
        );
    }
    let layout_refs = layouts.iter().collect::<Vec<_>>();
    let mut builder = SceneBuilder::new(revision, scene_viewport)?;
    builder.append_viewport(&input, &layout_refs, atlas, font_size, color)?;
    Ok(ViewportSceneFrame {
        input,
        scene: builder.finish(),
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use yu_editor::{LayoutConfig, ViewportConfig};
    use yu_font::{
        FontDatabase, FontFaceSpec, FontRequest, FontShaper, GlyphAtlasConfig, GlyphBitmap,
        GlyphMetrics, GlyphRasterKey, RasterizedGlyph,
    };
    use yu_render::RenderPlanBuilder;
    use yu_scene::{Primitive, Rgba8};

    use super::*;

    fn shaper(font_size: f32) -> FontShaper {
        let mut database = FontDatabase::new();
        database
            .register(FontFaceSpec::new("Test", 0.5))
            .expect("font face");
        FontShaper::new(
            Arc::new(database),
            FontRequest::new("Test", font_size).expect("font request"),
        )
        .expect("shaper")
    }

    fn atlas_for_document(
        document: &mut EditorDocument,
        viewport: ViewportRect,
        shaper: &FontShaper,
        font_size: f32,
    ) -> GlyphAtlas {
        let snapshot = document
            .visible_blocks_with_shaper(viewport, shaper)
            .expect("viewport");
        let config = document.viewport_config().layout();
        let mut atlas = GlyphAtlas::new(GlyphAtlasConfig::new(64, 64, 1).expect("atlas config"));
        for block in snapshot.blocks() {
            let layout = document
                .block_layout_with_shaper(block.index(), config, shaper)
                .expect("layout");
            for placement in layout.glyphs() {
                let key = GlyphRasterKey::new(placement.face(), placement.glyph(), font_size)
                    .expect("glyph key");
                if atlas.entry(key).is_none() {
                    let glyph = RasterizedGlyph::new(
                        key,
                        GlyphMetrics::new(0.0, 10.0, 7.0).expect("metrics"),
                        GlyphBitmap::new(2, 3, 2, vec![255; 6]).expect("bitmap"),
                    );
                    atlas.insert(glyph).expect("atlas insert");
                }
            }
        }
        atlas
    }

    #[test]
    fn editor_viewport_is_assembled_into_revision_bound_render_plan() {
        let font_size = 14.0;
        let shaper = shaper(font_size);
        let viewport = ViewportRect::new(0.0, 120.0);
        let mut document = EditorDocument::new("# title\n\nhello **world**");
        document
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(240.0, 20.0),
                20.0,
                0.0,
            ))
            .expect("viewport config");
        let atlas = atlas_for_document(&mut document, viewport, &shaper, font_size);
        let frame = assemble_viewport_scene(
            &mut document,
            viewport,
            &shaper,
            font_size,
            Rect::new(0.0, 0.0, 240.0, 120.0).expect("scene viewport"),
            &atlas,
            Rgba8::black(),
        )
        .expect("scene frame");

        assert_eq!(frame.revision(), document.revision());
        assert_eq!(frame.input().revision(), document.revision());
        assert_eq!(frame.input().blocks().len(), 3);
        assert!(!frame.scene().primitives().is_empty());
        let mut plans = RenderPlanBuilder::new();
        let plan = plans.build(frame.scene(), &atlas).expect("render plan");
        assert_eq!(plan.revision(), document.revision());
        assert_eq!(plan.commands().len(), frame.scene().primitives().len());
        assert!(
            plan.commands()
                .iter()
                .any(|command| matches!(command, yu_render::RenderCommand::Glyph { .. }))
        );
        assert!(
            frame
                .scene()
                .primitives()
                .iter()
                .all(|primitive| matches!(primitive, Primitive::Glyph(_)))
        );
    }

    #[test]
    fn missing_atlas_is_rejected_before_frame_publication() {
        let font_size = 14.0;
        let shaper = shaper(font_size);
        let viewport = ViewportRect::new(0.0, 80.0);
        let mut document = EditorDocument::new("paragraph");
        document
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(240.0, 20.0),
                20.0,
                0.0,
            ))
            .expect("viewport config");
        let empty_atlas = GlyphAtlas::new(GlyphAtlasConfig::new(64, 64, 1).expect("atlas config"));
        let error = assemble_viewport_scene(
            &mut document,
            viewport,
            &shaper,
            font_size,
            Rect::new(0.0, 0.0, 240.0, 80.0).expect("scene viewport"),
            &empty_atlas,
            Rgba8::black(),
        )
        .expect_err("missing glyph must fail");
        assert!(matches!(
            error,
            ViewportSceneError::Scene(SceneError::MissingGlyphAtlas(_))
        ));
    }
}
