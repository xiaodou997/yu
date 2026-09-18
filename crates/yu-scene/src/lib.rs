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

/// 圆角矩形的软阴影参数。
///
/// `blur` 就是 fragment shader 高斯衰减的 σ（逻辑像素）：阴影浓度按
/// `exp(-d² / 2σ²)` 衰减，d 是到圆角矩形边缘的带符号距离。`offset_x` /
/// `offset_y` 把阴影整体平移（正值向右 / 向下）。`blur` 为 0 时按无阴影
/// 处理——σ = 0 的高斯没有定义，硬边投影不是这个结构要表达的东西。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shadow {
    offset_x: f32,
    offset_y: f32,
    blur: f32,
    color: Rgba8,
}

impl Shadow {
    /// 高斯 σ 之外仍把阴影算作「到达」的距离，以 σ 的倍数计。
    ///
    /// 取 3σ：该处衰减到 e⁻⁹ ≈ 1.2e-4，8-bit 颜色下不可见。**这一份常数
    /// 同时决定 scene damage 的外扩与 Metal 绘制 quad 的外扩**，两边必须
    /// 一致：damage 大于实际绘制范围会擦掉没有命令重绘的像素，小于则留下
    /// 阴影残影。
    pub const EXTENT_SIGMAS: f32 = 3.0;

    pub fn new(offset_x: f32, offset_y: f32, blur: f32, color: Rgba8) -> Result<Self, SceneError> {
        if !offset_x.is_finite() || !offset_y.is_finite() {
            return Err(SceneError::InvalidGeometry(
                "shadow offset must contain finite coordinates",
            ));
        }
        if !blur.is_finite() || blur < 0.0 {
            return Err(SceneError::InvalidGeometry(
                "shadow blur must be finite and non-negative",
            ));
        }
        Ok(Self {
            offset_x,
            offset_y,
            blur,
            color,
        })
    }

    #[must_use]
    pub const fn offset_x(self) -> f32 {
        self.offset_x
    }

    #[must_use]
    pub const fn offset_y(self) -> f32 {
        self.offset_y
    }

    #[must_use]
    pub const fn blur(self) -> f32 {
        self.blur
    }

    #[must_use]
    pub const fn color(self) -> Rgba8 {
        self.color
    }

    /// 阴影在几何 bounds 之外四边各自的最大延伸（左、上、右、下，逻辑像素）。
    ///
    /// 衰减 σ = blur，超过 `EXTENT_SIGMAS` × σ 截断；offset 再把阴影整体
    /// 平移，所以四边不对称——阴影向左最多探出 `extent - min(0, ox)`，向右
    /// 最多探出 `extent + max(0, ox)`，上下同理。blur 为 0 时不画阴影
    /// （见 [`Shadow`]），外扩为零。
    #[must_use]
    pub fn outset(&self) -> (f32, f32, f32, f32) {
        if self.blur == 0.0 {
            return (0.0, 0.0, 0.0, 0.0);
        }
        let extent = Self::EXTENT_SIGMAS * self.blur;
        (
            extent - self.offset_x.min(0.0),
            extent - self.offset_y.min(0.0),
            extent + self.offset_x.max(0.0),
            extent + self.offset_y.max(0.0),
        )
    }

    /// 把几何 bounds 外扩到阴影的最大到达范围。
    pub fn expand_bounds(&self, bounds: Rect) -> Result<Rect, SceneError> {
        let (left, top, right, bottom) = self.outset();
        Ok(Rect::new(
            bounds.x() - left,
            bounds.y() - top,
            bounds.width() + left + right,
            bounds.height() + top + bottom,
        )?)
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
    corner_radius: f32,
}

impl ImagePrimitive {
    #[must_use]
    pub const fn new(resource: u64, bounds: Rect, fallback: Rgba8) -> Self {
        Self {
            resource,
            bounds,
            fallback,
            // 默认 0 = 直角，即 M2 之前的行为；圆角图片由调用方显式开启。
            corner_radius: 0.0,
        }
    }

    /// 图片四角的裁剪圆角（逻辑像素）。0 = 直角（现状）。
    ///
    /// 取负或非有限值在渲染后端被拒绝；scene 层不重复校验，与 `radius` 在
    /// `RoundedFillRect` 上的处理一致。
    #[must_use]
    pub const fn with_corner_radius(mut self, corner_radius: f32) -> Self {
        self.corner_radius = corner_radius;
        self
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

    #[must_use]
    pub const fn corner_radius(self) -> f32 {
        self.corner_radius
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
pub enum OrnamentShape {
    Rectangle,
    Rounded {
        radius: f32,
    },
    /// Three vertices relative to bounds; stroke and caps stay inside bounds.
    Polyline {
        points: [Point; 3],
        width: f32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OrnamentPrimitive {
    source: TextRange,
    bounds: Rect,
    color: Rgba8,
    role: OrnamentRole,
    shape: OrnamentShape,
}

impl OrnamentPrimitive {
    #[must_use]
    pub const fn new(source: TextRange, bounds: Rect, color: Rgba8, role: OrnamentRole) -> Self {
        Self {
            source,
            bounds,
            color,
            role,
            shape: OrnamentShape::Rectangle,
        }
    }

    #[must_use]
    pub const fn with_shape(mut self, shape: OrnamentShape) -> Self {
        self.shape = shape;
        self
    }
    #[must_use]
    pub const fn shape(self) -> OrnamentShape {
        self.shape
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

/// 一个要画的字形：字面、字形 id、block 局部的基线左端、字号倍率，以及一个
/// 可选的颜色覆盖。
///
/// 场景层要的只有这几样。它不认识布局的盒子类型，也不认识源码坐标——那些属于
/// 上面那层（不变量 E1、E2）。
///
/// # 那个 `Option<Rgba8>`
///
/// 一帧本来只有一个正文颜色，由 [`SceneBuilder::append_viewport`] 的 `color`
/// 参数给（S7 第五刀之前，从装饰到这里没有任何一层带着「这个字什么颜色」）。
/// 代码块高亮要给**字形本身**上色，而那是不变量 D1 管的事——文字自己的视觉
/// 表现——所以颜色一路从装饰走下来，到这里才落地。
///
/// 写成 `Option` 而不是「每个字形都带一份 `Rgba8`」有两条理由：**没有覆盖**与
/// **覆盖成正文色**是两件不同的事（前者跟着主题走，后者钉死），而绝大多数
/// 字形属于前者；另外，`None` 让「上一层忘了传颜色」退化成现状而不是一片黑。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneGlyph {
    face: yu_font::FontFaceId,
    glyph: yu_font::GlyphId,
    origin: yu_core::Point<yu_core::Block>,
    size_scale: f32,
    color: Option<Rgba8>,
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
            color: None,
        }
    }

    /// 覆盖这一帧的正文颜色。
    #[must_use]
    pub const fn with_color(mut self, color: Rgba8) -> Self {
        self.color = Some(color);
        self
    }

    #[must_use]
    pub const fn color(self) -> Option<Rgba8> {
        self.color
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
/// 画家顺序就是字段顺序：装饰 → 字形 → 图片 → 覆盖层。
///
/// 装饰在字形**之前**，所以引用竖条、表格网格衬在文字底下；图片在字形之后，
/// 所以一张就绪的图盖得住它替代的那段文本；覆盖层在最后，给那些必须压在
/// 文字上面的控件（任务框之类）。两个位置都留着不是为了对称——把控件挪到
/// 文字下面去不会报错，只是画面变了，而那种变化只有真实窗口看得见。
///
/// 块背景（代码灰底、引用蓝底）**不在**这里：它曾经是 `with_fill` 一块铺满
/// 视口宽的直角矩形，M4 起改成列宽圆角矩形，几何只有拼装的上一层知道（
/// `layout_tokens` 的 helper + 布局配置），由 `yu-workspace` 直接发
/// `RoundedFillRect`。
#[derive(Clone, Copy, Debug)]
pub struct ViewportBlockContent<'a> {
    revision: Revision,
    source: TextRange,
    glyphs: &'a [SceneGlyph],
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
            ornaments: &[],
            images: &[],
            overlays: &[],
        }
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
    FillRect {
        bounds: Rect,
        color: Rgba8,
    },
    /// 圆角矩形填充，可带软阴影。
    ///
    /// M2 交付的底座能力，供给行内代码 chip、圆角代码块背景、引用块与圆角
    /// 图片背景。`bounds()` 只报几何矩形；阴影会画到它之外——外扩量见
    /// [`Shadow::outset`]，damage 必须走 [`Primitive::damage_bounds`]。
    RoundedFillRect {
        bounds: Rect,
        radius: f32,
        color: Rgba8,
        shadow: Option<Shadow>,
    },
    Glyph(GlyphPrimitive),
    Image(ImagePrimitive),
    EmbeddedSvg(EmbeddedSvgPrimitive),
    Ornament(OrnamentPrimitive),
    EditorDecoration(EditorDecorationPrimitive),
}

impl Primitive {
    /// 几何 bounds。注意 [`Primitive::RoundedFillRect`] 的阴影会画到这个
    /// 矩形之外（外扩量见 [`Shadow::outset`]）：参与 damage 的调用方必须改用
    /// [`Primitive::damage_bounds`]，否则 retained target 会留下阴影残影。
    #[must_use]
    pub fn bounds(self) -> Rect {
        match self {
            Self::FillRect { bounds, .. } => bounds,
            Self::RoundedFillRect { bounds, .. } => bounds,
            Self::Glyph(glyph) => glyph.bounds(),
            Self::Image(image) => image.bounds(),
            Self::EmbeddedSvg(svg) => svg.bounds(),
            Self::Ornament(ornament) => ornament.bounds(),
            Self::EditorDecoration(decoration) => decoration.bounds(),
        }
    }

    /// 参与 damage 的 bounds：几何 bounds 外扩阴影的最大到达范围。
    ///
    /// 除 `RoundedFillRect` 外的图元没有 bounds 之外的绘制，damage_bounds
    /// 等于 `bounds()`。
    pub fn damage_bounds(self) -> Result<Rect, SceneError> {
        match self {
            Self::RoundedFillRect {
                bounds,
                shadow: Some(shadow),
                ..
            } => shadow.expand_bounds(bounds),
            Self::Ornament(ornament) => {
                match ornament.shape() {
                    OrnamentShape::Rounded { radius } if !radius.is_finite() || radius < 0.0 => {
                        return Err(SceneError::InvalidGeometry("invalid ornament radius"));
                    }
                    OrnamentShape::Polyline { points, width }
                        if !width.is_finite()
                            || width <= 0.0
                            || points.iter().any(|p| {
                                !p.x().is_finite()
                                    || !p.y().is_finite()
                                    || p.x() < width / 2.0
                                    || p.y() < width / 2.0
                                    || p.x() > ornament.bounds().width() - width / 2.0
                                    || p.y() > ornament.bounds().height() - width / 2.0
                            }) =>
                    {
                        return Err(SceneError::InvalidGeometry(
                            "stroke must stay inside ornament bounds",
                        ));
                    }

                    _ => {}
                }
                Ok(ornament.bounds())
            }
            _ => Ok(self.bounds()),
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
        if let Primitive::RoundedFillRect { radius, .. } = primitive
            && (!radius.is_finite() || radius < 0.0)
        {
            // 半径是裸字段，构造入口收在这里；shadow 本身由 Shadow::new 校验过。
            return Err(SceneError::InvalidGeometry(
                "corner radius must be finite and non-negative",
            ));
        }
        let index =
            u32::try_from(self.primitives.len()).map_err(|_| SceneError::PrimitiveLimitExceeded)?;
        self.primitives.push(primitive);
        // damage 用 damage_bounds：阴影画在几何 bounds 之外，漏掉会留残影。
        self.damage.add(primitive.damage_bounds()?)?;
        Ok(index)
    }

    pub fn fill_rect(&mut self, bounds: Rect, color: Rgba8) -> Result<u32, SceneError> {
        self.push(Primitive::FillRect { bounds, color })
    }

    pub fn rounded_fill_rect(
        &mut self,
        bounds: Rect,
        radius: f32,
        color: Rgba8,
        shadow: Option<Shadow>,
    ) -> Result<u32, SceneError> {
        self.push(Primitive::RoundedFillRect {
            bounds,
            radius,
            color,
            shadow,
        })
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
    /// 每个块的内容由调用方装配好（[`ViewportBlockContent`]）：装饰、字形、
    /// 图片。**这一层不知道那些装饰是什么语法**——它只按画家顺序摆
    /// 矩形和字形（不变量 E1）。块背景由调用方自己发 `RoundedFillRect`，
    /// 不进这条路（见 [`ViewportBlockContent`] 的文档）。
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
                // 字形自己带的颜色赢过这一帧的正文颜色。没带就是没带——
                // 见 [`SceneGlyph`] 上关于这个 `Option` 的那一段。
                placement.color().unwrap_or(color),
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
            // 与 push 同一条不变量：damage 覆盖阴影外扩，不只几何 bounds。
            damage.add(primitive.damage_bounds()?)?;
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

    /// 阴影外扩公式是 damage 与 Metal quad 共用的唯一依据，逐边钉死：
    /// 每边先外扩 3σ（σ = blur），再按 offset 的符号一边加一边抵消。
    #[test]
    fn shadow_outset_grows_with_blur_and_shifts_with_offset() {
        let shadow = Shadow::new(4.0, -2.0, 10.0, Rgba8::black()).expect("shadow");
        assert_eq!(
            shadow.outset(),
            (30.0 - 0.0, 30.0 + 2.0, 30.0 + 4.0, 30.0 - 0.0)
        );

        // offset 反号时加减速对调：正 offset 扩张右/下、负 offset 扩张左/上。
        let flipped = Shadow::new(-4.0, 2.0, 10.0, Rgba8::black()).expect("shadow");
        assert_eq!(
            flipped.outset(),
            (30.0 + 4.0, 30.0 - 0.0, 30.0 - 0.0, 30.0 + 2.0)
        );

        // offset 为零时四边对称；blur 为零时没有外扩。
        let centered = Shadow::new(0.0, 0.0, 10.0, Rgba8::black()).expect("shadow");
        assert_eq!(centered.outset(), (30.0, 30.0, 30.0, 30.0));
        let plain = Shadow::new(3.0, 3.0, 0.0, Rgba8::black()).expect("shadow");
        assert_eq!(plain.outset(), (0.0, 0.0, 0.0, 0.0));
    }

    #[test]
    fn shadow_rejects_non_finite_offset_and_negative_blur() {
        assert_eq!(
            Shadow::new(f32::NAN, 0.0, 4.0, Rgba8::black()),
            Err(SceneError::InvalidGeometry(
                "shadow offset must contain finite coordinates"
            ))
        );
        assert_eq!(
            Shadow::new(0.0, f32::INFINITY, 4.0, Rgba8::black()),
            Err(SceneError::InvalidGeometry(
                "shadow offset must contain finite coordinates"
            ))
        );
        assert_eq!(
            Shadow::new(0.0, 0.0, -1.0, Rgba8::black()),
            Err(SceneError::InvalidGeometry(
                "shadow blur must be finite and non-negative"
            ))
        );
    }

    /// `bounds()` 只报几何矩形，damage 必须覆盖阴影探出的部分——否则 retained
    /// target 里旧阴影没人擦、新阴影没人画。两边读的是同一份 outset 公式。
    #[test]
    fn rounded_fill_rect_damage_bounds_cover_shadow_extent() {
        let bounds = Rect::new(40.0, 50.0, 20.0, 10.0).expect("bounds");
        let shadow = Shadow::new(4.0, -2.0, 10.0, Rgba8::new(0, 0, 0, 128)).expect("shadow");
        let primitive = Primitive::RoundedFillRect {
            bounds,
            radius: 6.0,
            color: Rgba8::white(),
            shadow: Some(shadow),
        };

        assert_eq!(primitive.bounds(), bounds);
        let damage = primitive.damage_bounds().expect("damage bounds");
        assert_eq!(damage.x(), 40.0 - 30.0);
        assert_eq!(damage.y(), 50.0 - 32.0);
        assert_eq!(damage.right(), 60.0 + 34.0);
        assert_eq!(damage.bottom(), 60.0 + 30.0);

        // 无阴影时 damage_bounds 退化为几何 bounds。
        let plain = Primitive::RoundedFillRect {
            bounds,
            radius: 6.0,
            color: Rgba8::white(),
            shadow: None,
        };
        assert_eq!(plain.damage_bounds().expect("damage bounds"), bounds);
    }

    #[test]
    fn scene_expands_damage_for_shadow_and_rejects_invalid_radius() {
        let revision = Revision::new(9);
        let viewport = Rect::new(0.0, 0.0, 200.0, 100.0).expect("viewport");
        let shadow = Shadow::new(4.0, -2.0, 10.0, Rgba8::new(0, 0, 0, 128)).expect("shadow");
        let mut builder = SceneBuilder::new(revision, viewport).expect("builder");
        builder
            .rounded_fill_rect(
                Rect::new(40.0, 50.0, 20.0, 10.0).expect("bounds"),
                6.0,
                Rgba8::white(),
                Some(shadow),
            )
            .expect("rounded fill");

        let damage = builder.finish().damage().bounds().expect("damage");
        assert_eq!(damage.x(), 10.0);
        assert_eq!(damage.y(), 18.0);
        assert_eq!(damage.right(), 94.0);
        assert_eq!(damage.bottom(), 90.0);

        let mut invalid = SceneBuilder::new(revision, viewport).expect("builder");
        assert_eq!(
            invalid.rounded_fill_rect(
                Rect::new(0.0, 0.0, 10.0, 10.0).expect("bounds"),
                f32::NAN,
                Rgba8::white(),
                None,
            ),
            Err(SceneError::InvalidGeometry(
                "corner radius must be finite and non-negative"
            ))
        );
        let mut negative = SceneBuilder::new(revision, viewport).expect("builder");
        assert_eq!(
            negative.rounded_fill_rect(
                Rect::new(0.0, 0.0, 10.0, 10.0).expect("bounds"),
                -2.0,
                Rgba8::white(),
                None,
            ),
            Err(SceneError::InvalidGeometry(
                "corner radius must be finite and non-negative"
            ))
        );
    }

    /// 图片圆角默认 0（M2 之前的行为），with_corner_radius 只是改写默认值。
    #[test]
    fn image_corner_radius_defaults_to_square_corners() {
        let bounds = Rect::new(4.0, 0.0, 32.0, 10.0).expect("image bounds");
        let square = ImagePrimitive::new(42, bounds, Rgba8::new(232, 234, 238, 255));
        assert_eq!(square.corner_radius(), 0.0);
        let rounded = square.with_corner_radius(8.0);
        assert_eq!(rounded.corner_radius(), 8.0);
        assert_eq!(square.corner_radius(), 0.0, "builder 不改写原值");
        assert_eq!(rounded.resource(), 42);
        assert_eq!(rounded.bounds(), bounds);
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
