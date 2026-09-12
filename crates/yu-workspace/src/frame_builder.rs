//! 视口帧的准备：走一遍可见字形把它们栅格化进 CPU atlas，然后发布一帧。
//!
//! 这一层原来住在 `yu-render-macos`（`CoreTextViewportFrameBuilder`，462 行），
//! 而它**真正跟 macOS 有关的只有两处**：字段的类型是 `CoreTextShaper`，以及
//! `self.shaper.rasterizer()` 那一行。[`yu_font::RasterizingShaper`] 把后者
//! 写进了类型，于是整块逻辑可以泛型化搬到工作区层——第二端不必抄第二遍。
//!
//! builder 不拥有窗口、GPU 纹理或编辑器文档。它只保留跨帧要复用的东西：平台
//! shaper、CPU atlas、render plan 的页指纹缓存、发布序号。

use std::error::Error;
use std::fmt;

use yu_assets::{ImageIntrinsicPublication, ImagePublication, ImageRequestPriority};
use yu_editor::{EditorDocument, EditorDocumentError, EditorRenderLayout, EditorRenderSnapshot};
use yu_font::{
    AtlasError, GlyphAtlas, GlyphAtlasConfig, GlyphRasterKey, GlyphRasterizer, RasterizingShaper,
};
use yu_render::RenderPlanBuilder;

use crate::{
    FrameBuildRequest, ViewportFramePublication, ViewportFramePublisher, ViewportPublishError,
    ViewportRenderConfig,
};

/// Immutable source and visual state for a frame preparation job. Document
/// reconstruction and visibility discovery happen in `publish_owned`, on the
/// worker; capture never borrows AppKit or shares a mutable editor.
#[derive(Debug)]
pub struct ViewportFrameBuildInput {
    pub request: FrameBuildRequest,
    pub document: EditorRenderSnapshot,
    pub image_publications: Vec<ImagePublication>,
    pub image_intrinsics: Vec<ImageIntrinsicPublication>,
}

/// Result returned by a background preparation job.  The request is carried
/// beside the publication so the owner can reject stale work before upload or
/// presentation.
#[derive(Debug)]
pub struct ViewportFrameBuildOutput {
    pub layout: EditorRenderLayout,
    pub request: FrameBuildRequest,
    pub publication: ViewportFramePublication,
    /// CPU glyph atlas state produced alongside the publication.  The main
    /// thread adopts it before GPU upload so plan page references always point
    /// at the pixels that were prepared by the worker.
    pub atlas: GlyphAtlas,
    /// Visibility was computed by the worker using the same shaper/config.
    pub viewport_blocks: Vec<(usize, ImageRequestPriority)>,
    /// Resource payloads used by this exact publication, for owner validation.
    pub image_publications: Vec<ImagePublication>,
    pub image_intrinsics: Vec<ImageIntrinsicPublication>,
}

/// 准备一帧时可能出现的错误。
///
/// `E` 是后端栅格化器自己的错误类型——布局层与工作区层都不解释它，只原样带上去
/// 交给调用方映射（平台侧的状态码表是唯一知道该怎么翻译它的地方）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ViewportFrameBuildError<E> {
    InvalidConfig(&'static str),
    Document(EditorDocumentError),
    Raster(E),
    Atlas(AtlasError),
    Publish(ViewportPublishError),
}

impl<E: fmt::Display> fmt::Display for ViewportFrameBuildError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig(message) => formatter.write_str(message),
            Self::Document(error) => error.fmt(formatter),
            Self::Raster(error) => error.fmt(formatter),
            Self::Atlas(error) => error.fmt(formatter),
            Self::Publish(error) => error.fmt(formatter),
        }
    }
}

impl<E: Error + 'static> Error for ViewportFrameBuildError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Document(error) => Some(error),
            Self::Raster(error) => Some(error),
            Self::Atlas(error) => Some(error),
            Self::Publish(error) => Some(error),
            Self::InvalidConfig(_) => None,
        }
    }
}

impl<E> From<EditorDocumentError> for ViewportFrameBuildError<E> {
    fn from(error: EditorDocumentError) -> Self {
        Self::Document(error)
    }
}

impl<E> From<AtlasError> for ViewportFrameBuildError<E> {
    fn from(error: AtlasError) -> Self {
        Self::Atlas(error)
    }
}

impl<E> From<ViewportPublishError> for ViewportFrameBuildError<E> {
    fn from(error: ViewportPublishError) -> Self {
        Self::Publish(error)
    }
}

/// 跨帧存活的准备状态：平台 shaper、CPU atlas、页指纹缓存、发布缓存。
///
/// 这些对象都不进入 canonical 的编辑器文档。
#[derive(Debug)]
pub struct ViewportFrameBuilder<S> {
    shaper: S,
    atlas: GlyphAtlas,
    render_plans: RenderPlanBuilder,
    publisher: ViewportFramePublisher,
    config: ViewportRenderConfig,
}

type BuildError<S> =
    ViewportFrameBuildError<<<S as RasterizingShaper>::Rasterizer as GlyphRasterizer>::Error>;

impl<S: RasterizingShaper> ViewportFrameBuilder<S> {
    /// 用一个已经配置好的平台 shaper 建一个 builder。
    pub fn with_shaper(
        shaper: S,
        config: ViewportRenderConfig,
        atlas_config: GlyphAtlasConfig,
    ) -> Result<Self, BuildError<S>> {
        Self::with_shaper_and_initial_serial(shaper, config, atlas_config, 0)
    }

    /// 建一个发布序号接着上一个 builder 走的 builder。新 builder 的 shaping 与
    /// atlas 状态仍然是全新的，跨重建带过来的只有发布身份。
    pub fn with_shaper_and_initial_serial(
        shaper: S,
        config: ViewportRenderConfig,
        atlas_config: GlyphAtlasConfig,
        initial_serial: u64,
    ) -> Result<Self, BuildError<S>> {
        check_font_size(&shaper, config)?;
        let mut render_plans = RenderPlanBuilder::new();
        render_plans
            .set_raster_scale(config.raster_scale())
            .map_err(|_| {
                ViewportFrameBuildError::InvalidConfig(
                    "viewport render config has an invalid raster scale",
                )
            })?;
        Ok(Self {
            shaper,
            atlas: GlyphAtlas::new(atlas_config).with_raster_scale(config.raster_scale()),
            render_plans,
            publisher: ViewportFramePublisher::with_next_serial(initial_serial),
            config,
        })
    }

    /// 准备并发布当前文档的视口。
    pub fn publish(
        &mut self,
        document: &mut EditorDocument,
    ) -> Result<ViewportFramePublication, BuildError<S>> {
        self.publish_with_images(document, &[])
    }

    /// 用已经就绪的图片尺寸准备一帧。图片字节仍然归平台缓存所有；进入
    /// 布局/场景计算的只有尺寸。
    pub fn publish_with_images(
        &mut self,
        document: &mut EditorDocument,
        image_publications: &[ImagePublication],
    ) -> Result<ViewportFramePublication, BuildError<S>> {
        self.publish_with_images_and_intrinsics(document, image_publications, &[])
    }

    /// 用已就绪的像素、以及像素被淘汰之后仍然留着的固有尺寸准备一帧。
    pub fn publish_with_images_and_intrinsics(
        &mut self,
        document: &mut EditorDocument,
        image_publications: &[ImagePublication],
        image_intrinsics: &[ImageIntrinsicPublication],
    ) -> Result<ViewportFramePublication, BuildError<S>> {
        let raster_start = std::time::Instant::now();
        self.rasterize_visible_glyphs(document)?;
        if std::env::var_os("YU_RENDER_TIMING").is_some() {
            println!(
                "yu-render-metric event=preparation_rasterization duration_ms={:.6}",
                raster_start.elapsed().as_secs_f64() * 1000.0
            );
        }
        let publish_start = std::time::Instant::now();
        self.publisher
            .publish_with_images_and_intrinsics(
                document,
                self.config,
                &self.shaper,
                &self.atlas,
                &mut self.render_plans,
                image_publications,
                image_intrinsics,
            )
            .map_err(ViewportFrameBuildError::Publish)
            .inspect(|_| {
                if std::env::var_os("YU_RENDER_TIMING").is_some() {
                    println!(
                        "yu-render-metric event=preparation_publication duration_ms={:.6}",
                        publish_start.elapsed().as_secs_f64() * 1000.0
                    );
                }
            })
    }

    /// Consumes an owned worker input and publishes one revision-bound frame.
    pub fn publish_owned(
        &mut self,
        input: ViewportFrameBuildInput,
    ) -> Result<ViewportFrameBuildOutput, BuildError<S>> {
        let ViewportFrameBuildInput {
            request,
            document,
            image_publications,
            image_intrinsics,
        } = input;
        let mut document = document.into_document()?;
        self.publish_owned_document(request, &mut document, image_publications, image_intrinsics)
    }

    /// Publishes using a document retained by the same worker between jobs.
    /// The document never crosses the worker boundary; this only avoids
    /// reparsing and rebuilding its layout cache for a scroll-only request.
    pub fn publish_owned_document(
        &mut self,
        request: FrameBuildRequest,
        document: &mut EditorDocument,
        image_publications: Vec<ImagePublication>,
        image_intrinsics: Vec<ImageIntrinsicPublication>,
    ) -> Result<ViewportFrameBuildOutput, BuildError<S>> {
        // Worker outputs may be dropped or superseded before reaching the GPU.
        // Every owned publication carries the pages it needs; the GPU atlas
        // deduplicates by fingerprint only after receiving the payload.
        self.render_plans.reset();
        let publication = self.publish_with_images_and_intrinsics(
            document,
            &image_publications,
            &image_intrinsics,
        )?;
        let viewport = self.config.viewport();
        let viewport_blocks = publication
            .frame()
            .scene()
            .input()
            .blocks()
            .iter()
            .map(|block| {
                let priority = if block.y() + block.height() > viewport.scroll_y()
                    && block.y() < viewport.scroll_y() + viewport.height()
                {
                    ImageRequestPriority::Visible
                } else {
                    ImageRequestPriority::Overscan
                };
                (block.index(), priority)
            })
            .collect();
        Ok(ViewportFrameBuildOutput {
            layout: document.render_layout(),
            request,
            publication,
            atlas: self.atlas.clone(),
            viewport_blocks,
            image_publications,
            image_intrinsics,
        })
    }

    /// 当前视口/overscan 窗口里的块下标。图片资源调度器用它，好让屏幕外的
    /// 图片不进 解码队列。
    pub fn visible_block_indices(
        &self,
        document: &mut EditorDocument,
    ) -> Result<Vec<usize>, EditorDocumentError> {
        Ok(self
            .viewport_image_blocks(document)?
            .into_iter()
            .map(|(index, _)| index)
            .collect())
    }

    /// 带显式请求优先级的视口/overscan 块下标。分类用的是渲染器消费的同一份
    /// 文档空间几何，所以调度器不会另建一份 HeightIndex，也不会扫全文。
    pub fn viewport_image_blocks(
        &self,
        document: &mut EditorDocument,
    ) -> Result<Vec<(usize, ImageRequestPriority)>, EditorDocumentError> {
        let snapshot = document
            .visible_blocks_with_visual_state_and_shaper(self.config.viewport(), &self.shaper)?;
        let viewport = self.config.viewport();
        let visible_top = viewport.scroll_y();
        let visible_bottom = visible_top + viewport.height();
        Ok(snapshot
            .blocks()
            .iter()
            .map(|block| {
                let block_bottom = block.y() + block.height();
                let priority = if block_bottom > visible_top && block.y() < visible_bottom {
                    ImageRequestPriority::Visible
                } else {
                    ImageRequestPriority::Overscan
                };
                (block.index(), priority)
            })
            .collect())
    }

    /// 换一套视口/场景输入，保留 shaper、CPU atlas、页指纹缓存与发布序号。
    /// 字号是 glyph raster key 的一部分，所以字号变了调用方必须重建 builder，
    /// 而不是让一个 host 状态里混着两个字号。
    pub fn update_config(&mut self, config: ViewportRenderConfig) -> Result<bool, BuildError<S>> {
        check_font_size(&self.shaper, config)?;
        let changed = self.config != config;
        self.config = config;
        Ok(changed)
    }

    #[must_use]
    pub const fn config(&self) -> ViewportRenderConfig {
        self.config
    }

    /// 借出跨帧存活的 shaper 供宿主查度量。shaper 仍然归 builder 所有：调用方
    /// 换不掉它，也搬不动它背后的原生状态。
    #[must_use]
    pub const fn shaper(&self) -> &S {
        &self.shaper
    }

    #[must_use]
    pub fn atlas(&self) -> &GlyphAtlas {
        &self.atlas
    }

    /// Replace the CPU atlas with the state returned by an owned worker build.
    /// The publication and atlas are a matched pair; keeping this operation on
    /// the builder makes that pairing explicit at the publication boundary.
    pub fn replace_atlas(&mut self, atlas: GlyphAtlas) {
        self.atlas = atlas;
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

    /// A diagnostic publisher and worker may take turns using the same host.
    /// Start after its accepted serial without discarding CPU atlas contents.
    pub fn ensure_publication_serial(&mut self, serial: u64) {
        self.publisher.next_serial = self.publisher.next_serial.max(serial);
    }

    fn rasterize_visible_glyphs(
        &mut self,
        document: &mut EditorDocument,
    ) -> Result<(), BuildError<S>> {
        let visibility_start = std::time::Instant::now();
        let viewport = document
            .visible_blocks_with_visual_state_and_shaper(self.config.viewport(), &self.shaper)?;
        if std::env::var_os("YU_RENDER_TIMING").is_some() {
            println!(
                "yu-render-metric event=preparation_visibility duration_ms={:.6} blocks={}",
                visibility_start.elapsed().as_secs_f64() * 1000.0,
                viewport.blocks().len()
            );
        }
        let glyph_start = std::time::Instant::now();
        let layout_config = document.viewport_config().layout();
        let rasterizer = self.shaper.rasterizer();
        for block in viewport.blocks() {
            let layout = document.block_layout_for_visual_state_with_shaper(
                block.index(),
                layout_config,
                &self.shaper,
            )?;
            for placement in layout.glyphs() {
                // 按物理像素取样：Retina 上 raster_scale 是 backing scale，
                // 后端会把 atlas 矩形除回逻辑坐标。否则 1x 纹理会被拉伸到 2x
                // 而发虚。
                // size 保持逻辑尺寸——它决定 optical size 变体，必须与 shaping
                // 时一致；栅格倍率是独立维度，参与缓存键并只影响绘制分辨率。
                let key = GlyphRasterKey::new(
                    placement.face(),
                    placement.glyph(),
                    self.config.font_size() * placement.size_scale(),
                )
                .and_then(|key| key.with_raster_scale(self.config.raster_scale()))
                .map_err(|_| {
                    ViewportFrameBuildError::InvalidConfig(
                        "glyph raster key has an invalid font size or raster scale",
                    )
                })?;
                if self.atlas.entry(key).is_none() {
                    self.atlas.insert(
                        rasterizer
                            .rasterize(key)
                            .map_err(ViewportFrameBuildError::Raster)?,
                    )?;
                }
            }
        }
        if std::env::var_os("YU_RENDER_TIMING").is_some() {
            println!(
                "yu-render-metric event=preparation_glyph_loop duration_ms={:.6}",
                glyph_start.elapsed().as_secs_f64() * 1000.0
            );
        }
        Ok(())
    }
}

/// 视口字号必须与 shaper 的字号一致。
///
/// 不一致时 atlas 里的字形与排出来的度量属于两个字号——不报错，只是画面糊。
/// 构造与 `update_config` 走同一份检查，两处各写一遍正是这条最容易漂开的地方。
fn check_font_size<S: RasterizingShaper>(
    shaper: &S,
    config: ViewportRenderConfig,
) -> Result<(), BuildError<S>> {
    let font_size = config.font_size();
    if !font_size.is_finite() || font_size <= 0.0 {
        return Err(ViewportFrameBuildError::InvalidConfig(
            "viewport font size must be finite and positive",
        ));
    }
    if (shaper.font_request().size() - font_size).abs() > 0.001 {
        return Err(ViewportFrameBuildError::InvalidConfig(
            "shaper size must match viewport font size",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use yu_core::{ShapedText, ShapingProvider, TextRange, TextStyle};
    use yu_editor::{LayoutConfig, ViewportConfig, ViewportSpan};
    use yu_font::{
        FontDatabase, FontFaceSpec, FontMetricKey, FontMetricsSnapshot, FontRequest, FontShaper,
        GlyphBitmap, GlyphMetrics, RasterizedGlyph, ShapeError,
    };
    use yu_scene::{Rect, Rgba8};

    use super::*;

    /// 一个会数数的假栅格化器。
    ///
    /// 判据不是「atlas 里有多少字形」——那是 atlas 自己的账；是**后端被问了
    /// 几次**。少了 `atlas.entry(key).is_none()` 那道门，前者一模一样而后者
    /// 每帧都在涨。
    #[derive(Clone, Debug)]
    struct CountingRasterizer {
        calls: Arc<AtomicUsize>,
        fail: bool,
    }

    impl GlyphRasterizer for CountingRasterizer {
        type Error = ShapeError;

        fn font_metrics(&self, key: FontMetricKey) -> Result<FontMetricsSnapshot, Self::Error> {
            FontMetricsSnapshot::new(key.size(), 0.0, 0.0, 1000)
                .map_err(|error| ShapeError::Backend(Arc::from(error.to_string())))
        }

        fn rasterize(&self, key: GlyphRasterKey) -> Result<RasterizedGlyph, Self::Error> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                return Err(ShapeError::Backend(Arc::from("rasterizer refuses")));
            }
            Ok(RasterizedGlyph::new(
                key,
                GlyphMetrics::new(0.0, 10.0, 7.0).expect("metrics"),
                GlyphBitmap::new(2, 3, 2, vec![255; 6]).expect("bitmap"),
            ))
        }
    }

    #[derive(Clone, Debug)]
    struct CountingShaper {
        inner: FontShaper,
        calls: Arc<AtomicUsize>,
        fail: bool,
    }

    impl CountingShaper {
        fn new(font_size: f32, fail: bool) -> Self {
            let mut database = FontDatabase::new();
            database
                .register(FontFaceSpec::new("Test", 0.5))
                .expect("font face");
            Self {
                inner: FontShaper::new(
                    Arc::new(database),
                    FontRequest::new("Test", font_size).expect("font request"),
                )
                .expect("shaper"),
                calls: Arc::new(AtomicUsize::new(0)),
                fail,
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    impl ShapingProvider for CountingShaper {
        type Error = ShapeError;

        fn shape(
            &self,
            text: &str,
            source: TextRange,
            style: TextStyle,
        ) -> Result<ShapedText, Self::Error> {
            self.inner.shape(text, source, style)
        }

        fn shape_scaled(
            &self,
            text: &str,
            source: TextRange,
            style: TextStyle,
            scale: f32,
        ) -> Result<ShapedText, Self::Error> {
            self.inner.shape_scaled(text, source, style, scale)
        }
    }

    impl RasterizingShaper for CountingShaper {
        type Rasterizer = CountingRasterizer;

        fn font_request(&self) -> &FontRequest {
            self.inner.request()
        }

        fn rasterizer(&self) -> Self::Rasterizer {
            CountingRasterizer {
                calls: Arc::clone(&self.calls),
                fail: self.fail,
            }
        }
    }

    fn document(text: &str) -> EditorDocument {
        let mut document = EditorDocument::new(text);
        document
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(240.0, 20.0),
                20.0,
                0.0,
            ))
            .expect("viewport config");
        document
    }

    fn config(font_size: f32) -> ViewportRenderConfig {
        ViewportRenderConfig::new(
            ViewportSpan::new(0.0, 200.0),
            font_size,
            Rect::new(0.0, 0.0, 240.0, 200.0).expect("scene viewport"),
            Rgba8::black(),
        )
    }

    #[test]
    fn publication_serial_floor_survives_diagnostic_worker_handoffs() {
        let mut builder = ViewportFrameBuilder::with_shaper_and_initial_serial(
            CountingShaper::new(14.0, false),
            config(14.0),
            GlyphAtlasConfig::default(),
            10,
        )
        .unwrap();
        let mut document = document("hello");
        builder.ensure_publication_serial(3);
        assert_eq!(builder.publish(&mut document).unwrap().serial(), 11);
        builder.ensure_publication_serial(20);
        assert_eq!(builder.publish(&mut document).unwrap().serial(), 21);
        builder.ensure_publication_serial(4);
        assert_eq!(builder.publish(&mut document).unwrap().serial(), 22);
    }

    #[test]
    fn dropped_owned_publication_does_not_consume_gpu_uploads() {
        let shaper = CountingShaper::new(14.0, false);
        let counter = Arc::clone(&shaper.calls);
        let mut builder = ViewportFrameBuilder::with_shaper(
            shaper,
            config(14.0).with_raster_scale(2.0),
            GlyphAtlasConfig::default(),
        )
        .unwrap();
        let make_input = |generation| {
            let document = document("hello");
            let key = crate::FrameBuildKey::new(
                document.revision().get(),
                0,
                vec![document.selection()],
                0,
                None,
                crate::Appearance::Light,
                crate::FrameGeometry::new(14.0, 240.0, 0.0, 200.0, 240.0, 200.0, 2.0).unwrap(),
            );
            ViewportFrameBuildInput {
                request: FrameBuildRequest::new(key, generation),
                document: document.capture_render_snapshot(),
                image_publications: vec![],
                image_intrinsics: vec![],
            }
        };
        let dropped = builder.publish_owned(make_input(1)).unwrap();
        let pages = dropped.publication.frame().plan().uploads().to_vec();
        assert!(!pages.is_empty());
        let raster_calls = counter.load(Ordering::SeqCst);
        drop(dropped); // The consumer never saw these page payloads.
        let delivered = builder.publish_owned(make_input(2)).unwrap();
        assert_eq!(delivered.publication.frame().plan().uploads(), pages);
        assert_eq!(counter.load(Ordering::SeqCst), raster_calls);
        assert_eq!(delivered.atlas.raster_scale(), 2.0);
    }

    #[test]
    fn visible_glyphs_are_rasterized_once_and_reused_across_frames() {
        let font_size = 14.0;
        let shaper = CountingShaper::new(font_size, false);
        let counter = Arc::clone(&shaper.calls);
        let mut builder = ViewportFrameBuilder::with_shaper(
            shaper,
            config(font_size),
            GlyphAtlasConfig::new(64, 64, 2).expect("atlas config"),
        )
        .expect("builder");
        let mut document = document("# title\n\nhello **world**");

        builder.publish(&mut document).expect("first publication");
        let first = counter.load(Ordering::SeqCst);
        assert!(first > 0);
        assert!(builder.atlas_glyph_count() > 0);

        builder.publish(&mut document).expect("second publication");
        assert_eq!(
            counter.load(Ordering::SeqCst),
            first,
            "同一份文档再发一帧不该再问后端要字形"
        );

        document
            .execute(yu_editor::EditorCommand::insert_text("Zq"))
            .expect("edit");
        builder.publish(&mut document).expect("edited publication");
        assert!(
            counter.load(Ordering::SeqCst) > first,
            "新字形必须真的栅格化一次"
        );
    }

    /// 后端排不出/画不出来时，错误原样带上去，不悄悄少画一个字形。
    #[test]
    fn a_refusing_rasterizer_surfaces_its_own_error() {
        let font_size = 14.0;
        let shaper = CountingShaper::new(font_size, true);
        let mut builder = ViewportFrameBuilder::with_shaper(
            shaper,
            config(font_size),
            GlyphAtlasConfig::new(64, 64, 2).expect("atlas config"),
        )
        .expect("builder");
        let mut document = document("hello");

        let error = builder
            .publish(&mut document)
            .expect_err("rasterizer fails");
        assert!(matches!(error, ViewportFrameBuildError::Raster(_)));
    }

    /// 字号必须与 shaper 一致——**构造与 `update_config` 是同一份检查**。
    ///
    /// 两处各写一遍是这条最容易漂开的地方：漏掉后一处的表现是「改字号之后
    /// atlas 里的字形与排出来的度量属于两个字号」，不报错，只是画面糊。
    #[test]
    fn viewport_font_size_must_match_the_shaper_at_construction_and_on_update() {
        let font_size = 14.0;
        let shaper = CountingShaper::new(font_size, false);
        assert!(matches!(
            ViewportFrameBuilder::with_shaper(
                shaper.clone(),
                config(font_size + 2.0),
                GlyphAtlasConfig::new(64, 64, 2).expect("atlas config"),
            )
            .expect_err("mismatched size"),
            ViewportFrameBuildError::InvalidConfig(_)
        ));

        let mut builder = ViewportFrameBuilder::with_shaper(
            shaper,
            config(font_size),
            GlyphAtlasConfig::new(64, 64, 2).expect("atlas config"),
        )
        .expect("builder");
        assert!(
            !builder
                .update_config(config(font_size))
                .expect("same config")
        );
        assert!(matches!(
            builder
                .update_config(config(font_size + 2.0))
                .expect_err("mismatched size"),
            ViewportFrameBuildError::InvalidConfig(_)
        ));
    }

    #[test]
    fn publication_serial_continues_after_a_rebuilt_builder() {
        let font_size = 14.0;
        let shaper = CountingShaper::new(font_size, false);
        let mut builder = ViewportFrameBuilder::with_shaper_and_initial_serial(
            shaper.clone(),
            config(font_size),
            GlyphAtlasConfig::new(64, 64, 2).expect("atlas config"),
            7,
        )
        .expect("builder");
        let mut document = document("hello");
        assert_eq!(
            builder
                .publish(&mut document)
                .expect("publication")
                .serial(),
            8
        );
        assert_eq!(builder.shaper().calls(), builder.shaper().calls());
    }
}
