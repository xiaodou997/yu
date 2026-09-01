use std::fmt;

use crate::TextRange;
use crate::style::TextStyle;

/// Stable face identity carried by shaped glyph runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FontFaceId(u32);

impl FontFaceId {
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }

    #[must_use]
    pub const fn from_raw(value: u32) -> Self {
        Self(value)
    }
}

/// Stable glyph identity within a font face.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GlyphId(u32);

impl GlyphId {
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }

    #[must_use]
    pub const fn from_raw(value: u32) -> Self {
        Self(value)
    }
}

/// Direction passed through the shaping boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextDirection {
    Ltr,
    Rtl,
}

/// Script hint passed through the shaping boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Script {
    Common,
    Latin,
    Han,
    Japanese,
    Arabic,
    Devanagari,
    Unknown,
}

/// One positioned glyph with a source cluster range.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glyph {
    id: GlyphId,
    source: TextRange,
    advance: f32,
    x_offset: f32,
    y_offset: f32,
}

impl Glyph {
    #[must_use]
    pub const fn new(
        id: GlyphId,
        source: TextRange,
        advance: f32,
        x_offset: f32,
        y_offset: f32,
    ) -> Self {
        Self {
            id,
            source,
            advance,
            x_offset,
            y_offset,
        }
    }

    #[must_use]
    pub const fn id(self) -> GlyphId {
        self.id
    }

    #[must_use]
    pub const fn source(self) -> TextRange {
        self.source
    }

    #[must_use]
    pub const fn advance(self) -> f32 {
        self.advance
    }

    #[must_use]
    pub const fn x_offset(self) -> f32 {
        self.x_offset
    }

    #[must_use]
    pub const fn y_offset(self) -> f32 {
        self.y_offset
    }
}

/// A same-face shaped run. Source ranges are ordered and may span multiple
/// Unicode code points when a shaping engine forms a ligature or cluster.
///
/// **但一簇多形不行**：一个 run 里的字形区间必须首尾相接、不重叠、**非空**地
/// 铺满这个 run。完整条文与理由在 [`ShapingProvider`] 上，可执行的那一份是
/// [`crate::shaping_conformance`]。这句话以前不在这里，于是「多个 code point
/// 可以合成一形」看着像在说「也可以反过来」——第二个实现正是这么读的。
#[derive(Clone, Debug, PartialEq)]
pub struct GlyphRun {
    face: FontFaceId,
    source: TextRange,
    style: TextStyle,
    direction: TextDirection,
    script: Script,
    glyphs: Vec<Glyph>,
    advance: f32,
}

impl GlyphRun {
    #[must_use]
    pub fn new(
        face: FontFaceId,
        source: TextRange,
        style: TextStyle,
        direction: TextDirection,
        script: Script,
        glyphs: Vec<Glyph>,
    ) -> Self {
        let advance = glyphs.iter().map(|glyph| glyph.advance()).sum();
        Self {
            face,
            source,
            style,
            direction,
            script,
            glyphs,
            advance,
        }
    }

    #[must_use]
    pub const fn face(&self) -> FontFaceId {
        self.face
    }

    #[must_use]
    pub const fn source(&self) -> TextRange {
        self.source
    }

    #[must_use]
    pub const fn style(&self) -> TextStyle {
        self.style
    }

    #[must_use]
    pub const fn direction(&self) -> TextDirection {
        self.direction
    }

    #[must_use]
    pub const fn script(&self) -> Script {
        self.script
    }

    #[must_use]
    pub fn glyphs(&self) -> &[Glyph] {
        &self.glyphs
    }

    #[must_use]
    pub const fn advance(&self) -> f32 {
        self.advance
    }
}

/// Shaped output potentially split into fallback-face runs.
#[derive(Clone, Debug, PartialEq)]
pub struct ShapedText {
    source: TextRange,
    runs: Vec<GlyphRun>,
    advance: f32,
}

impl ShapedText {
    #[must_use]
    pub fn new(source: TextRange, runs: Vec<GlyphRun>) -> Self {
        let advance = runs.iter().map(GlyphRun::advance).sum();
        Self {
            source,
            runs,
            advance,
        }
    }

    #[must_use]
    pub const fn source(&self) -> TextRange {
        self.source
    }

    #[must_use]
    pub fn runs(&self) -> &[GlyphRun] {
        &self.runs
    }

    #[must_use]
    pub const fn advance(&self) -> f32 {
        self.advance
    }

    /// Scales deterministic shaped coordinates while retaining glyph/source
    /// identity. Real font backends should override `shape_scaled` and shape
    /// at the target point size so hinting and optical-size behavior remain
    /// native; this fallback keeps lightweight providers source-compatible.
    #[must_use]
    fn scaled(self, scale: f32) -> Self {
        let runs = self
            .runs
            .into_iter()
            .map(|run| {
                let glyphs = run
                    .glyphs
                    .into_iter()
                    .map(|glyph| Glyph {
                        advance: glyph.advance * scale,
                        x_offset: glyph.x_offset * scale,
                        y_offset: glyph.y_offset * scale,
                        ..glyph
                    })
                    .collect();
                GlyphRun::new(
                    run.face,
                    run.source,
                    run.style,
                    run.direction,
                    run.script,
                    glyphs,
                )
            })
            .collect();
        Self::new(self.source, runs)
    }
}

/// 布局层与字体后端之间的插口。
///
/// # 契约
///
/// 这十条以前**不写在这里**，而是散在调用方里——`yu-layout/src/block.rs` 的
/// tiling 门只表达了其中三条，`GlyphRun` 的文档一个字都没说不许一簇多形。
/// 类型上看不出来的东西正是第二个实现会撞的东西，所以 S7 第七刀把它们搬到了
/// 这里，并写成了可执行的 [`crate::shaping_conformance`]。
///
/// 给 `shape(text, source, style)`，其中 `source.len() == text.len()`
/// （调用方保证）。返回 `Ok(shaped)` 时必须满足：
///
/// - **C1** `shaped.source()` 等于请求的 `source`。
/// - **C2** 各 `GlyphRun::source` 按逻辑顺序首尾相接、不重叠，并集恰好等于
///   `source`。
/// - **C3** 一个 run 内各 [`Glyph::source`] 按逻辑顺序首尾相接、不重叠，并集
///   恰好等于该 run 的 `source`。**缺一段就是丢字，重一段就是重画，两样都不
///   panic。**
/// - **C4** 每个 [`Glyph::source`] 非空。C3 单独并不排除空区间——`from ==
///   cursor` 对空区间恒成立——而空区间会让布局层在 run 末尾越界 panic
///   （实测），在中间则多算一段 advance。**合起来 C3 + C4 就是「一簇一形」。**
/// - **C5** 每个 [`Glyph::source`] 的两端落在 `text` 的 UTF-8 字符边界上。
/// - **C6** `advance` 有限且非负，`x_offset` / `y_offset` 有限。
/// - **C7** 每个 run 的 `style()` 是请求的那个。
/// - **C8** 字形区间是 `source.start()` **加上**局部字节偏移。布局层今天总是
///   传零基 range（`block.rs` 的 `local_range`），所以忘了加基址在产品链路上
///   看不出来；契约不依赖调用方的这个习惯。
/// - **C9** 同一次请求重复调用给同一个答案。
/// - **C10** `shape_scaled(.., 1.0)` 等于 `shape`。
///
/// # 做不到就报错
///
/// 覆盖面不是契约的一部分：一个只排得了拉丁文的后端仍然合规，它对别的输入
/// 返回 `Err`。**不许为了凑满 C3 而伪造区间**——一簇多形时把多余的字形塞一个
/// 空区间会撞 C4，让它们重复簇首会撞 C3，把整簇并成一形是在少画字形。三条路
/// 都要显式选，选不了就返回 `Err`。
///
/// > `Err` 今天在产品链路上没有降级：它一路传成 `LayoutError::Shaping` →
/// > `EditorDocumentError::Layout` → `assemble_viewport_scene_*` 的 `?`，
/// > 于是那一整屏发不出来。**这条欠账已登记**（overview-v2 第 8 节 S7
/// > 第七刀的 spike 一节），不要靠伪造区间来绕开它。
pub trait ShapingProvider {
    type Error: fmt::Display;

    fn shape(
        &self,
        text: &str,
        source: TextRange,
        style: TextStyle,
    ) -> Result<ShapedText, Self::Error>;

    /// Shapes at a finite, positive scale relative to the provider's base
    /// font request. The layout boundary validates the scale before calling.
    /// Providers backed by a native font engine should override this method;
    /// the default is suitable for deterministic and benchmark shapers.
    fn shape_scaled(
        &self,
        text: &str,
        source: TextRange,
        style: TextStyle,
        scale: f32,
    ) -> Result<ShapedText, Self::Error> {
        self.shape(text, source, style)
            .map(|shaped| shaped.scaled(scale))
    }
}

/// 给一个 Unicode grapheme cluster 提供 advance。
///
/// 这个 trait 此前定义在 `yu-layout`，而实现它的是 `yu-font`——于是字体层必须
/// 反向依赖布局层（overview-v2 第 2.4 节）。它描述的是「量一个 cluster 有多宽」，
/// 属于字体契约，不属于布局。
pub trait ClusterMetrics {
    fn advance(&self, cluster: &str, style: TextStyle) -> f32;
}
