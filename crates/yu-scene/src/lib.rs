#![forbid(unsafe_code)]

//! Revision-bound retained drawing data for Yu Editor.
//!
//! A scene contains only owned geometry, colors and glyph-atlas placements. It
//! deliberately does not contain source text, layout caches, pixels, native
//! window objects or GPU handles. A renderer can therefore consume a scene on
//! its own thread and discard it when its source revision is stale.

use std::error::Error;
use std::fmt;

use yu_core::{GeometryError, Revision, TextRange};
use yu_font::{AtlasEntry, GlyphRasterKey};

mod viewport;

pub use viewport::{ViewportBlockGeometry, ViewportSceneInput};

/// 文档坐标系里的点与矩形。
///
/// 实现在 `yu-core`，空间是 [`yu_core::Document`]：原点是文档内容左上角，
/// 单位是逻辑像素，**不含**滚动位移。用别名而不是自己再写一份：算术只写一
/// 遍，而空间由类型参数带着走——把 block 局部矩形直接当成场景矩形是编译错
/// 误，唯一的通道是 [`yu_core::Rect::translate_into`]。
pub type Point = yu_core::Point<yu_core::Document>;

/// 文档坐标系里的矩形。见 [`Point`]。
pub type Rect = yu_core::Rect<yu_core::Document>;

/// Packed non-premultiplied sRGB color used by scene primitives.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rgba8(u32);

impl Rgba8 {
    #[must_use]
    pub const fn new(red: u8, green: u8, blue: u8, alpha: u8) -> Self {
        Self(u32::from_be_bytes([red, green, blue, alpha]))
    }

    #[must_use]
    pub const fn packed(self) -> u32 {
        self.0
    }

    #[must_use]
    pub const fn red(self) -> u8 {
        self.0.to_be_bytes()[0]
    }

    #[must_use]
    pub const fn green(self) -> u8 {
        self.0.to_be_bytes()[1]
    }

    #[must_use]
    pub const fn blue(self) -> u8 {
        self.0.to_be_bytes()[2]
    }

    #[must_use]
    pub const fn alpha(self) -> u8 {
        self.0.to_be_bytes()[3]
    }

    #[must_use]
    pub const fn white() -> Self {
        Self::new(255, 255, 255, 255)
    }

    #[must_use]
    pub const fn black() -> Self {
        Self::new(0, 0, 0, 255)
    }
}

impl Default for Rgba8 {
    fn default() -> Self {
        Self::black()
    }
}

/// A glyph draw operation referencing an entry in a separate CPU/GPU atlas.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlyphPrimitive {
    atlas: AtlasEntry,
    origin: Point,
    color: Rgba8,
    bounds: Rect,
}

impl GlyphPrimitive {
    pub fn new(atlas: AtlasEntry, origin: Point, color: Rgba8) -> Result<Self, SceneError> {
        let rect = atlas.rect();
        // bounds 在构造时算好并校验，`bounds()` 因此永远合法。原来的做法是
        // 用结构体字面量绕过校验、再让每个使用点自己 `validate()`——漏掉一
        // 个使用点就是一个画错但不报错的字形。
        let bounds = Rect::new(
            origin.x() + atlas.metrics().bearing_x(),
            origin.y() - atlas.metrics().bearing_y(),
            rect.width() as f32,
            rect.height() as f32,
        )?;
        Ok(Self {
            atlas,
            origin,
            color,
            bounds,
        })
    }

    #[must_use]
    pub const fn atlas(self) -> AtlasEntry {
        self.atlas
    }

    #[must_use]
    pub const fn key(self) -> GlyphRasterKey {
        self.atlas.key()
    }

    #[must_use]
    pub const fn origin(self) -> Point {
        self.origin
    }

    #[must_use]
    pub const fn color(self) -> Rgba8 {
        self.color
    }

    /// Returns the visual bounds in baseline-oriented scene coordinates.
    #[must_use]
    pub const fn bounds(self) -> Rect {
        self.bounds
    }
}

/// A source-independent image draw operation.
///
/// `resource` is a stable `yu-assets::ImageKey::fingerprint()` supplied by
/// the host. The scene carries only that scalar identity and a fallback
/// color; decoded pixels and GPU textures remain owned by the platform
/// backend.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImagePrimitive {
    resource: u64,
    bounds: Rect,
    fallback: Rgba8,
}

impl ImagePrimitive {
    #[must_use]
    pub const fn new(resource: u64, bounds: Rect, fallback: Rgba8) -> Self {
        Self {
            resource,
            bounds,
            fallback,
        }
    }

    #[must_use]
    pub const fn resource(self) -> u64 {
        self.resource
    }

    #[must_use]
    pub const fn bounds(self) -> Rect {
        self.bounds
    }

    #[must_use]
    pub const fn fallback(self) -> Rgba8 {
        self.fallback
    }
}

/// A source-backed SVG embedded resource draw operation.
///
/// The scene intentionally carries only the resource identity and intrinsic
/// dimensions. SVG markup remains owned by the backend-neutral render-plan
/// upload, so the retained scene cannot accidentally become a second document
/// source of truth. `kind` is the stable wire tag from `yu-assets` (Math is 0,
/// Mermaid is 1) without making the scene depend on a concrete asset cache.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EmbeddedSvgPrimitive {
    resource: u64,
    generation: u64,
    kind: u8,
    source: TextRange,
    bounds: Rect,
    width: u32,
    height: u32,
    fallback: Rgba8,
}

impl EmbeddedSvgPrimitive {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub const fn new(
        resource: u64,
        generation: u64,
        kind: u8,
        source: TextRange,
        bounds: Rect,
        width: u32,
        height: u32,
        fallback: Rgba8,
    ) -> Self {
        Self {
            resource,
            generation,
            kind,
            source,
            bounds,
            width,
            height,
            fallback,
        }
    }

    #[must_use]
    pub const fn resource(self) -> u64 {
        self.resource
    }

    #[must_use]
    pub const fn generation(self) -> u64 {
        self.generation
    }

    #[must_use]
    pub const fn kind(self) -> u8 {
        self.kind
    }

    #[must_use]
    pub const fn source(self) -> TextRange {
        self.source
    }

    #[must_use]
    pub const fn bounds(self) -> Rect {
        self.bounds
    }

    #[must_use]
    pub const fn width(self) -> u32 {
        self.width
    }

    #[must_use]
    pub const fn height(self) -> u32 {
        self.height
    }

    #[must_use]
    pub const fn fallback(self) -> Rgba8 {
        self.fallback
    }
}

/// 一块装饰画的是什么层。
///
/// 后端今天把每一层都落成一个实心矩形；把层次留在 retained scene 里，是为了
/// 让原生诊断、选中与 Accessibility 分辨得出「这是边框还是填充」，而不必
/// 回头去解析文档。
///
/// 它是**渲染中立**的词汇：`Border` 就是一条边框，不管它属于表格、任务框
/// 还是别的什么。此前这里有三套按语法命名的 primitive（表格 / 引用条 /
/// 任务框），一种语法一条全链路，正是 overview-v2 §2.1 点名的泄漏。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OrnamentRole {
    /// 衬在内容底下的一块底色。
    Background,
    /// 盖在内容上的一块填充（选中高亮之类）。
    Fill,
    /// 一条边框线。
    Border,
    /// 一条贴着内容左侧或上方的装饰条。
    Bar,
    /// 一个记号（勾、点）。
    Mark,
}

/// 一块 source-backed 的装饰矩形。
///
/// `source` 指着它对应的那段源码——一个单元格、一段被引用的正文、一个
/// `[x]` 标记。几何与颜色都由调用方给：这一层只负责把它们留在场景里并
/// 参与 damage 计算。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OrnamentPrimitive {
    source: TextRange,
    bounds: Rect,
    color: Rgba8,
    role: OrnamentRole,
}

impl OrnamentPrimitive {
    #[must_use]
    pub const fn new(source: TextRange, bounds: Rect, color: Rgba8, role: OrnamentRole) -> Self {
        Self {
            source,
            bounds,
            color,
            role,
        }
    }

    #[must_use]
    pub const fn source(self) -> TextRange {
        self.source
    }

    #[must_use]
    pub const fn bounds(self) -> Rect {
        self.bounds
    }

    #[must_use]
    pub const fn color(self) -> Rgba8 {
        self.color
    }

    #[must_use]
    pub const fn role(self) -> OrnamentRole {
        self.role
    }
}

/// Semantic role for transient editor chrome retained with a visual frame.
///
/// These roles remain distinct in the scene even though the current renderer
/// lowers them to solid rectangles. Platform hosts can therefore prove that a
/// submitted frame owns the selection/caret pixels without inferring meaning
/// from color or geometry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EditorDecorationPrimitiveRole {
    Selection,
    Caret,
    CompositionCaret,
    /// 一处搜索命中。
    ///
    /// 它与 `Selection` 同类，**不是 Decoration**：不变量 D1 管的是文字自己
    /// 的视觉表现（藏语法、换控件、改字型），而这三个都是**盖在文字上、由
    /// 非文档状态驱动的矩形**，不改任何字节的样式。装饰的三张表里也没有一张
    /// 能表达「一段文字底下画一块颜色」。
    SearchMatch,
    /// 光标正落在上面的那一处命中。
    SearchCurrent,
}

/// One source-backed selection or caret rectangle.
///
/// `source` is the canonical selection intersection for a selection layer and
/// an empty range at the canonical focus/replacement boundary for a caret.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EditorDecorationPrimitive {
    source: TextRange,
    bounds: Rect,
    color: Rgba8,
    role: EditorDecorationPrimitiveRole,
}

impl EditorDecorationPrimitive {
    #[must_use]
    pub const fn new(
        source: TextRange,
        bounds: Rect,
        color: Rgba8,
        role: EditorDecorationPrimitiveRole,
    ) -> Self {
        Self {
            source,
            bounds,
            color,
            role,
        }
    }

    #[must_use]
    pub const fn source(self) -> TextRange {
        self.source
    }

    #[must_use]
    pub const fn bounds(self) -> Rect {
        self.bounds
    }

    #[must_use]
    pub const fn color(self) -> Rgba8 {
        self.color
    }

    #[must_use]
    pub const fn role(self) -> EditorDecorationPrimitiveRole {
        self.role
    }
}

/// 一个要画的字形：字面、字形 id、block 局部的基线左端、字号倍率。
///
/// 场景层要的只有这四样。它不认识布局的盒子类型，也不认识源码坐标——
/// 那些属于上面那层（不变量 E1、E2）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneGlyph {
    face: yu_font::FontFaceId,
    glyph: yu_font::GlyphId,
    origin: yu_core::Point<yu_core::Block>,
    size_scale: f32,
}

impl SceneGlyph {
    #[must_use]
    pub const fn new(
        face: yu_font::FontFaceId,
        glyph: yu_font::GlyphId,
        origin: yu_core::Point<yu_core::Block>,
        size_scale: f32,
    ) -> Self {
        Self {
            face,
            glyph,
            origin,
            size_scale,
        }
    }

    #[must_use]
    pub const fn face(self) -> yu_font::FontFaceId {
        self.face
    }

    #[must_use]
    pub const fn glyph(self) -> yu_font::GlyphId {
        self.glyph
    }

    #[must_use]
    pub const fn origin(self) -> yu_core::Point<yu_core::Block> {
        self.origin
    }

    #[must_use]
    pub const fn size_scale(self) -> f32 {
        self.size_scale
    }
}

/// 一个可见块在这一帧里要画的东西。
///
/// 画家顺序就是字段顺序：底色 → 装饰 → 字形 → 图片 → 覆盖层。
///
/// 装饰在字形**之前**，所以单元格底色盖不住它自己的文字；图片在字形之后，
/// 所以一张就绪的图盖得住它替代的那段文本；覆盖层在最后，给那些必须压在
/// 文字上面的控件（任务框之类）。两个位置都留着不是为了对称——把控件挪到
/// 文字下面去不会报错，只是画面变了，而那种变化只有真实窗口看得见。
#[derive(Clone, Copy, Debug)]
pub struct ViewportBlockContent<'a> {
    revision: Revision,
    source: TextRange,
    glyphs: &'a [SceneGlyph],
    fill: Option<Rgba8>,
    ornaments: &'a [OrnamentPrimitive],
    images: &'a [ImagePrimitive],
    overlays: &'a [OrnamentPrimitive],
}

impl<'a> ViewportBlockContent<'a> {
    #[must_use]
    pub const fn new(revision: Revision, source: TextRange, glyphs: &'a [SceneGlyph]) -> Self {
        Self {
            revision,
            source,
            glyphs,
            fill: None,
            ornaments: &[],
            images: &[],
            overlays: &[],
        }
    }

    /// 整块的底色。铺满视口宽度，衬在所有内容底下。
    #[must_use]
    pub const fn with_fill(mut self, fill: Option<Rgba8>) -> Self {
        self.fill = fill;
        self
    }

    /// 已经搬到文档坐标的装饰矩形。
    #[must_use]
    pub const fn with_ornaments(mut self, ornaments: &'a [OrnamentPrimitive]) -> Self {
        self.ornaments = ornaments;
        self
    }

    /// 已经搬到文档坐标的图片。
    #[must_use]
    pub const fn with_images(mut self, images: &'a [ImagePrimitive]) -> Self {
        self.images = images;
        self
    }

    /// 压在这一块所有内容之上的装饰。
    #[must_use]
    pub const fn with_overlays(mut self, overlays: &'a [OrnamentPrimitive]) -> Self {
        self.overlays = overlays;
        self
    }
}

/// One retained scene primitive. Insertion order is the painter's order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Primitive {
    FillRect { bounds: Rect, color: Rgba8 },
    Glyph(GlyphPrimitive),
    Image(ImagePrimitive),
    EmbeddedSvg(EmbeddedSvgPrimitive),
    Ornament(OrnamentPrimitive),
    EditorDecoration(EditorDecorationPrimitive),
}

impl Primitive {
    #[must_use]
    pub fn bounds(self) -> Rect {
        match self {
            Self::FillRect { bounds, .. } => bounds,
            Self::Glyph(glyph) => glyph.bounds(),
            Self::Image(image) => image.bounds(),
            Self::EmbeddedSvg(svg) => svg.bounds(),
            Self::Ornament(ornament) => ornament.bounds(),
            Self::EditorDecoration(decoration) => decoration.bounds(),
        }
    }
}

/// Damage rectangles for one scene build. Adjacent or overlapping rectangles
/// are merged; when the budget is exceeded, all damage is collapsed to bounds.
#[derive(Clone, Debug, PartialEq)]
pub struct DamageSet {
    max_rects: usize,
    rects: Vec<Rect>,
}

impl DamageSet {
    pub fn new(max_rects: usize) -> Result<Self, SceneError> {
        if max_rects == 0 {
            return Err(SceneError::InvalidDamageBudget);
        }
        Ok(Self {
            max_rects,
            rects: Vec::new(),
        })
    }

    #[must_use]
    pub const fn max_rects(&self) -> usize {
        self.max_rects
    }

    #[must_use]
    pub fn rects(&self) -> &[Rect] {
        &self.rects
    }

    #[must_use]
    pub fn bounds(&self) -> Option<Rect> {
        self.rects.iter().copied().reduce(Rect::union)
    }

    pub fn add(&mut self, rect: Rect) -> Result<(), SceneError> {
        if rect.is_empty() {
            return Ok(());
        }
        let mut merged = rect;
        let mut index = 0;
        while index < self.rects.len() {
            if self.rects[index].intersects_or_touches(merged) {
                merged = merged.union(self.rects.remove(index));
            } else {
                index += 1;
            }
        }
        self.rects.push(merged);
        if self.rects.len() > self.max_rects {
            let bounds = self.bounds().ok_or(SceneError::InvalidDamageBudget)?;
            self.rects.clear();
            self.rects.push(bounds);
        }
        Ok(())
    }

    pub fn clear(&mut self) {
        self.rects.clear();
    }
}

impl Default for DamageSet {
    fn default() -> Self {
        Self {
            max_rects: 64,
            rects: Vec::new(),
        }
    }
}

/// Errors raised while constructing a retained scene.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SceneError {
    Geometry(GeometryError),
    InvalidGeometry(&'static str),
    InvalidViewportInput(&'static str),
    InvalidDamageBudget,
    PrimitiveLimitExceeded,
    InvalidEmbeddedDimensions {
        width: u32,
        height: u32,
    },
    RevisionMismatch {
        scene: Revision,
        layout: Revision,
    },
    ViewportRevisionMismatch {
        expected: Revision,
        actual: Revision,
    },
    ViewportSourceMismatch,
    InvalidFontSize(u32),
    MissingGlyphAtlas(GlyphRasterKey),
}

impl fmt::Display for SceneError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Geometry(error) => error.fmt(formatter),
            Self::InvalidGeometry(message) => formatter.write_str(message),
            Self::InvalidViewportInput(message) => formatter.write_str(message),
            Self::InvalidDamageBudget => formatter.write_str("damage budget must be positive"),
            Self::PrimitiveLimitExceeded => formatter.write_str("scene primitive limit exceeded"),
            Self::InvalidEmbeddedDimensions { width, height } => write!(
                formatter,
                "embedded SVG dimensions must be positive, got {width}x{height}"
            ),
            Self::RevisionMismatch { scene, layout } => write!(
                formatter,
                "scene revision {scene:?} does not match layout revision {layout:?}"
            ),
            Self::ViewportRevisionMismatch { expected, actual } => write!(
                formatter,
                "viewport revision {actual:?} does not match expected {expected:?}"
            ),
            Self::ViewportSourceMismatch => {
                formatter.write_str("viewport block source range does not match layout")
            }
            Self::InvalidFontSize(size) => {
                write!(
                    formatter,
                    "invalid scene font size {}",
                    f32::from_bits(*size)
                )
            }
            Self::MissingGlyphAtlas(key) => write!(
                formatter,
                "layout references missing atlas glyph {}",
                key.glyph().get()
            ),
        }
    }
}

impl From<GeometryError> for SceneError {
    fn from(error: GeometryError) -> Self {
        Self::Geometry(error)
    }
}

impl Error for SceneError {}

/// A retained, source-revision-bound drawing scene.
#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    revision: Revision,
    viewport: Rect,
    primitives: Vec<Primitive>,
    damage: DamageSet,
}

impl Scene {
    #[must_use]
    pub fn revision(&self) -> Revision {
        self.revision
    }

    #[must_use]
    pub const fn viewport(&self) -> Rect {
        self.viewport
    }

    #[must_use]
    pub fn primitives(&self) -> &[Primitive] {
        &self.primitives
    }

    #[must_use]
    pub fn damage(&self) -> &DamageSet {
        &self.damage
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.primitives.is_empty()
    }
}

/// 把 block 局部矩形搬到文档坐标。`origin` 是该 block 左上角在文档中的位置。
///
/// 原来这里有两个函数：一个收 `LayoutRect`，一个收 `Rect`——后者的入参其实
/// 也是 block 局部坐标，只是被当成文档坐标构造出来的，两个空间在类型上分不
/// 开。现在 `LayoutRect` 就是 `Rect<Block>`，两者合而为一。
pub fn translate_block_rect(
    rect: yu_layout::LayoutRect,
    origin: Point,
) -> Result<Rect, SceneError> {
    Ok(rect.translate_into(origin)?)
}

/// Builds a scene while keeping primitive and damage order deterministic.
#[derive(Clone, Debug)]
pub struct SceneBuilder {
    revision: Revision,
    viewport: Rect,
    primitives: Vec<Primitive>,
    damage: DamageSet,
    max_primitives: usize,
}

impl SceneBuilder {
    pub fn new(revision: Revision, viewport: Rect) -> Result<Self, SceneError> {
        Ok(Self {
            revision,
            viewport,
            primitives: Vec::new(),
            damage: DamageSet::default(),
            max_primitives: 1_000_000,
        })
    }

    pub fn with_damage_budget(mut self, max_rects: usize) -> Result<Self, SceneError> {
        self.damage = DamageSet::new(max_rects)?;
        Ok(self)
    }

    #[must_use]
    pub const fn with_primitive_limit(mut self, max_primitives: usize) -> Self {
        self.max_primitives = max_primitives;
        self
    }

    pub fn push(&mut self, primitive: Primitive) -> Result<u32, SceneError> {
        if self.primitives.len() >= self.max_primitives {
            return Err(SceneError::PrimitiveLimitExceeded);
        }
        if let Primitive::EmbeddedSvg(svg) = primitive
            && (svg.width() == 0 || svg.height() == 0)
        {
            return Err(SceneError::InvalidEmbeddedDimensions {
                width: svg.width(),
                height: svg.height(),
            });
        }
        let index =
            u32::try_from(self.primitives.len()).map_err(|_| SceneError::PrimitiveLimitExceeded)?;
        self.primitives.push(primitive);
        self.damage.add(primitive.bounds())?;
        Ok(index)
    }

    pub fn fill_rect(&mut self, bounds: Rect, color: Rgba8) -> Result<u32, SceneError> {
        self.push(Primitive::FillRect { bounds, color })
    }

    pub fn glyph(&mut self, glyph: GlyphPrimitive) -> Result<u32, SceneError> {
        self.push(Primitive::Glyph(glyph))
    }

    pub fn image(&mut self, image: ImagePrimitive) -> Result<u32, SceneError> {
        self.push(Primitive::Image(image))
    }

    pub fn embedded_svg(&mut self, svg: EmbeddedSvgPrimitive) -> Result<u32, SceneError> {
        self.push(Primitive::EmbeddedSvg(svg))
    }

    pub fn ornament(&mut self, ornament: OrnamentPrimitive) -> Result<u32, SceneError> {
        self.push(Primitive::Ornament(ornament))
    }

    pub fn editor_decoration(
        &mut self,
        decoration: EditorDecorationPrimitive,
    ) -> Result<u32, SceneError> {
        self.push(Primitive::EditorDecoration(decoration))
    }

    /// 把一组字形追加进场景。
    ///
    /// `origin` 是这个 block 左上角在文档坐标里的位置；字形的坐标是 block
    /// 局部的，只有这里把它们搬过去。atlas 查表在改动场景**之前**全部做完，
    /// 所以一次失败不会留下画了一半的块。
    pub fn append_glyphs(
        &mut self,
        glyphs: &[SceneGlyph],
        atlas: &yu_font::GlyphAtlas,
        font_size: f32,
        color: Rgba8,
        origin: Point,
    ) -> Result<usize, SceneError> {
        let primitives = self.collect_glyphs(glyphs, atlas, font_size, color, origin)?;
        self.commit_glyphs(primitives)
    }

    /// 一帧里所有可见块，一次事务提交。
    ///
    /// 每个块的内容由调用方装配好（[`ViewportBlockContent`]）：底色、装饰、
    /// 字形、图片。**这一层不知道那些装饰是什么语法**——它只按画家顺序摆
    /// 矩形和字形（不变量 E1）。
    ///
    /// revision、源码范围、atlas 查表、几何与 primitive 预算全部在改动场景
    /// 之前校验完；一个过期或半成品的视口不可能只发布出它的前一半。
    pub fn append_viewport(
        &mut self,
        input: &ViewportSceneInput,
        blocks: &[ViewportBlockContent<'_>],
        atlas: &yu_font::GlyphAtlas,
        font_size: f32,
        color: Rgba8,
    ) -> Result<usize, SceneError> {
        if input.revision() != self.revision {
            return Err(SceneError::ViewportRevisionMismatch {
                expected: self.revision,
                actual: input.revision(),
            });
        }
        if blocks.len() != input.blocks().len() {
            return Err(SceneError::InvalidViewportInput(
                "viewport block count must match input blocks",
            ));
        }

        let mut primitives = Vec::new();
        for (geometry, content) in input.blocks().iter().copied().zip(blocks) {
            if geometry.revision() != self.revision {
                return Err(SceneError::ViewportRevisionMismatch {
                    expected: self.revision,
                    actual: geometry.revision(),
                });
            }
            if content.revision != geometry.revision() {
                return Err(SceneError::RevisionMismatch {
                    scene: geometry.revision(),
                    layout: content.revision,
                });
            }
            if content.source != geometry.source() {
                return Err(SceneError::ViewportSourceMismatch);
            }
            let origin = Point::new(0.0, geometry.y());
            if let Some(fill) = content.fill {
                primitives.push(Primitive::FillRect {
                    bounds: Rect::new(
                        self.viewport.x(),
                        geometry.y(),
                        self.viewport.width(),
                        geometry.height(),
                    )?,
                    color: fill,
                });
            }
            for ornament in content.ornaments {
                primitives.push(Primitive::Ornament(*ornament));
            }
            primitives.extend(
                self.collect_glyphs(content.glyphs, atlas, font_size, color, origin)?
                    .into_iter()
                    .map(Primitive::Glyph),
            );
            primitives.extend(content.images.iter().copied().map(Primitive::Image));
            for overlay in content.overlays {
                primitives.push(Primitive::Ornament(*overlay));
            }
        }
        self.commit_primitives(primitives)
    }

    fn collect_glyphs(
        &self,
        glyphs: &[SceneGlyph],
        atlas: &yu_font::GlyphAtlas,
        font_size: f32,
        color: Rgba8,
        origin: Point,
    ) -> Result<Vec<GlyphPrimitive>, SceneError> {
        if !origin.is_finite() {
            return Err(SceneError::InvalidGeometry(
                "block origin must contain finite coordinates",
            ));
        }
        if !font_size.is_finite() || font_size <= 0.0 {
            return Err(SceneError::InvalidFontSize(font_size.to_bits()));
        }

        let mut primitives = Vec::with_capacity(glyphs.len());
        for placement in glyphs.iter().copied() {
            let glyph_size = font_size * placement.size_scale();
            let key = GlyphRasterKey::new(placement.face(), placement.glyph(), glyph_size)
                .map_err(|_| SceneError::InvalidFontSize(glyph_size.to_bits()))?;
            let entry = atlas.entry(key).ok_or(SceneError::MissingGlyphAtlas(key))?;
            primitives.push(GlyphPrimitive::new(
                entry,
                Point::new(
                    origin.x() + placement.origin().x(),
                    origin.y() + placement.origin().y(),
                ),
                color,
            )?);
        }
        Ok(primitives)
    }

    fn commit_glyphs(&mut self, glyphs: Vec<GlyphPrimitive>) -> Result<usize, SceneError> {
        self.commit_primitives(glyphs.into_iter().map(Primitive::Glyph).collect())
    }

    fn commit_primitives(&mut self, primitives: Vec<Primitive>) -> Result<usize, SceneError> {
        if primitives.len() > self.max_primitives.saturating_sub(self.primitives.len()) {
            return Err(SceneError::PrimitiveLimitExceeded);
        }
        let new_len = self
            .primitives
            .len()
            .checked_add(primitives.len())
            .ok_or(SceneError::PrimitiveLimitExceeded)?;
        if !primitives.is_empty() && u32::try_from(new_len.saturating_sub(1)).is_err() {
            return Err(SceneError::PrimitiveLimitExceeded);
        }

        let mut damage = self.damage.clone();
        for primitive in &primitives {
            damage.add(primitive.bounds())?;
        }
        let count = primitives.len();
        self.primitives.extend(primitives);
        self.damage = damage;
        Ok(count)
    }

    #[must_use]
    pub fn finish(self) -> Scene {
        Scene {
            revision: self.revision,
            viewport: self.viewport,
            primitives: self.primitives,
            damage: self.damage,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yu_core::{ByteOffset, TextRange};
    use yu_font::{
        GlyphAtlas, GlyphAtlasConfig, GlyphBitmap, GlyphMetrics, GlyphRasterKey, RasterizedGlyph,
    };

    fn atlas_entry(glyph: u32) -> AtlasEntry {
        let key = GlyphRasterKey::new(
            yu_font::FontFaceId::from_raw(1),
            yu_font::GlyphId::from_raw(glyph),
            12.0,
        )
        .expect("key");
        let bitmap = GlyphBitmap::new(2, 3, 2, vec![255; 6]).expect("bitmap");
        let metrics = GlyphMetrics::new(1.0, 9.0, 8.0).expect("metrics");
        let glyph = RasterizedGlyph::new(key, metrics, bitmap);
        let mut atlas = GlyphAtlas::new(GlyphAtlasConfig::new(16, 16, 1).expect("config"));
        atlas.insert(glyph).expect("atlas entry")
    }

    #[test]
    fn damage_merges_touching_rectangles_and_collapses_over_budget() {
        let mut damage = DamageSet::new(2).expect("budget");
        damage
            .add(Rect::new(0.0, 0.0, 10.0, 10.0).expect("rect"))
            .expect("damage");
        damage
            .add(Rect::new(10.0, 0.0, 4.0, 4.0).expect("touching rect"))
            .expect("damage");
        assert_eq!(damage.rects().len(), 1);
        assert_eq!(damage.bounds().expect("bounds").width(), 14.0);

        damage
            .add(Rect::new(100.0, 100.0, 2.0, 2.0).expect("rect"))
            .expect("damage");
        damage
            .add(Rect::new(200.0, 200.0, 2.0, 2.0).expect("rect"))
            .expect("damage");
        assert_eq!(damage.rects().len(), 1);
        assert_eq!(damage.bounds().expect("bounds").right(), 202.0);
    }

    #[test]
    fn scene_keeps_revision_order_and_glyph_bounds() {
        let viewport = Rect::new(0.0, 0.0, 640.0, 480.0).expect("viewport");
        let mut builder = SceneBuilder::new(Revision::new(7), viewport)
            .expect("builder")
            .with_damage_budget(8)
            .expect("damage budget");
        let glyph = GlyphPrimitive::new(atlas_entry(1), Point::new(10.0, 20.0), Rgba8::white())
            .expect("glyph bounds");
        builder
            .fill_rect(Rect::new(0.0, 0.0, 4.0, 4.0).expect("rect"), Rgba8::black())
            .expect("rect primitive");
        builder.glyph(glyph).expect("glyph primitive");
        let scene = builder.finish();
        assert_eq!(scene.revision(), Revision::new(7));
        assert_eq!(scene.primitives().len(), 2);
        assert_eq!(scene.primitives()[1], Primitive::Glyph(glyph));
        let bounds = glyph.bounds();
        assert_eq!(bounds.x(), 11.0);
        assert_eq!(bounds.y(), 11.0);
        assert_eq!(bounds.width(), 2.0);
        assert_eq!(bounds.height(), 3.0);
    }

    #[test]
    fn invalid_geometry_is_rejected_before_scene_publication() {
        assert!(Rect::new(0.0, 0.0, f32::NAN, 1.0).is_err());
        // 非有限的点自己是合法值（hit-test 可以问任何位置），但它做不出矩形。
        assert!(!Point::new(f32::INFINITY, 0.0).is_finite());
        assert!(
            GlyphPrimitive::new(
                atlas_entry(1),
                Point::new(f32::INFINITY, 0.0),
                Rgba8::white()
            )
            .is_err()
        );
        assert_eq!(DamageSet::new(0), Err(SceneError::InvalidDamageBudget));
    }

    /// 装饰的顺序就是插入顺序，几何搬到文档坐标，身份留着源码范围。
    ///
    /// 这里不再有表格：`yu-scene` 已经不认识它了（不变量 E1）。表格的网格
    /// 由 `yu-workspace` 算完再交进来，那一段的用例住在那里。
    #[test]
    fn ornaments_keep_source_identity_and_painter_order() {
        let revision = Revision::new(5);
        let table = TextRange::new(ByteOffset::new(0), ByteOffset::new(30)).expect("table");
        let cell = TextRange::new(ByteOffset::new(2), ByteOffset::new(3)).expect("cell");
        let mut builder =
            SceneBuilder::new(revision, Rect::new(0.0, 0.0, 40.0, 40.0).expect("viewport"))
                .expect("builder");
        let header = OrnamentPrimitive::new(
            cell,
            Rect::new(10.0, 20.0, 6.0, 2.0).expect("bounds"),
            Rgba8::new(235, 238, 244, 255),
            OrnamentRole::Background,
        );
        let border = OrnamentPrimitive::new(
            table,
            Rect::new(10.0, 20.0, 1.0, 6.0).expect("bounds"),
            Rgba8::new(150, 155, 165, 255),
            OrnamentRole::Border,
        );
        builder.ornament(header).expect("header");
        builder.ornament(border).expect("border");
        let scene = builder.finish();
        assert_eq!(
            scene.primitives(),
            &[Primitive::Ornament(header), Primitive::Ornament(border)]
        );
        assert_eq!(scene.damage().rects().len(), 1);
        assert_eq!(scene.revision(), revision);
    }

    #[test]
    fn viewport_images_are_appended_after_block_content() {
        let revision = Revision::new(4);
        let source_range = TextRange::new(ByteOffset::ZERO, ByteOffset::ZERO).expect("range");
        let geometry = ViewportBlockGeometry::new(revision, 0, source_range, 0.0, 10.0, true, 0)
            .expect("geometry");
        let input = ViewportSceneInput::new(revision, 0..1, 10.0, vec![geometry]).expect("input");
        let bounds = Rect::new(4.0, 0.0, 32.0, 10.0).expect("image bounds");
        let image = ImagePrimitive::new(42, bounds, Rgba8::new(232, 234, 238, 255));
        let ornament = OrnamentPrimitive::new(
            source_range,
            Rect::new(0.0, 0.0, 8.0, 10.0).expect("bar"),
            Rgba8::new(176, 181, 190, 255),
            OrnamentRole::Bar,
        );
        let atlas = GlyphAtlas::new(GlyphAtlasConfig::default());
        let mut builder =
            SceneBuilder::new(revision, Rect::new(0.0, 0.0, 80.0, 10.0).expect("viewport"))
                .expect("builder");
        // 画家顺序：装饰在字形之前，图片在字形之后。这一块没有字形，所以
        // 剩下装饰在前、图片在后。
        let count = builder
            .append_viewport(
                &input,
                &[ViewportBlockContent::new(revision, source_range, &[])
                    .with_ornaments(std::slice::from_ref(&ornament))
                    .with_images(std::slice::from_ref(&image))],
                &atlas,
                12.0,
                Rgba8::black(),
            )
            .expect("append image");
        assert_eq!(count, 2);
        assert_eq!(
            builder.finish().primitives(),
            &[Primitive::Ornament(ornament), Primitive::Image(image)]
        );
    }

    #[test]
    fn embedded_svg_primitive_keeps_source_identity_and_rejects_empty_dimensions() {
        let revision = Revision::new(11);
        let viewport = Rect::new(0.0, 0.0, 320.0, 200.0).expect("viewport");
        let source = TextRange::new(ByteOffset::new(4), ByteOffset::new(12)).expect("source");
        let bounds = Rect::new(12.0, 18.0, 160.0, 80.0).expect("bounds");
        let fallback = Rgba8::new(238, 239, 244, 255);
        let svg = EmbeddedSvgPrimitive::new(0xfeed_beef, 3, 0, source, bounds, 640, 320, fallback);
        let mut builder = SceneBuilder::new(revision, viewport).expect("builder");
        builder.embedded_svg(svg).expect("embedded SVG");
        let scene = builder.finish();
        assert_eq!(scene.primitives(), &[Primitive::EmbeddedSvg(svg)]);
        assert_eq!(svg.source(), source);
        assert_eq!(svg.resource(), 0xfeed_beef);
        assert_eq!(svg.width(), 640);
        assert_eq!(svg.height(), 320);

        let invalid =
            EmbeddedSvgPrimitive::new(0xfeed_beef, 4, 0, source, bounds, 0, 320, fallback);
        let mut invalid_builder = SceneBuilder::new(revision, viewport).expect("builder");
        assert_eq!(
            invalid_builder.embedded_svg(invalid),
            Err(SceneError::InvalidEmbeddedDimensions {
                width: 0,
                height: 320,
            })
        );
    }
}
