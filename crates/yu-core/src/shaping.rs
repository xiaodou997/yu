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

/// One positioned glyph associated with one source cluster range.
///
/// Several consecutive glyphs may carry the same non-empty source range when a
/// shaping engine expands one cluster to several glyphs. Glyph order is the
/// backend's native drawing order; it is not required to match logical source
/// order for RTL runs.
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

/// A same-face shaped run.
///
/// [`Glyph::source`] names the complete source cluster for each glyph. Several
/// adjacent glyphs may therefore share the same source range (one cluster to
/// many glyphs), and RTL backends may return those cluster ranges in reverse
/// source order. The distinct cluster ranges still have to cover this run
/// exactly once in logical source order. The executable form of that contract
/// lives in [`crate::shaping_conformance`].
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
/// 这十条以前**不写在这里**，而是散在调用方里。S7 第七刀先把它们搬到这里；
/// Windows 第一组随后把 C3 从旧的“一簇一形”限制升级成当前的 cluster-group
/// 契约，并同步进可执行的 [`crate::shaping_conformance`]。
///
/// 给 `shape(text, source, style)`，其中 `source.len() == text.len()`
/// （调用方保证）。返回 `Ok(shaped)` 时必须满足：
///
/// - **C1** `shaped.source()` 等于请求的 `source`。
/// - **C2** 各 `GlyphRun::source` 按逻辑顺序首尾相接、不重叠，并集恰好等于
///   `source`。
/// - **C3** 一个 run 内的字形按**簇**关联源码：同一簇可以有一个或多个连续
///   字形，簇内所有 [`Glyph::source`] 完全相同。去重后的簇区间按逻辑源码顺序
///   首尾相接、不重叠，并集恰好等于该 run 的 `source`。字形数组本身可以保持
///   后端原生绘制顺序，因此 RTL run 的簇区间允许按源码逆序出现；同一簇的字形
///   不能被别的簇穿插。
/// - **C4** 每个 [`Glyph::source`] 非空。多字形簇通过共享同一个非空 source
///   range 表达，禁止用空区间给“多出来”的字形占位。
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
/// 返回 `Err`。**不许为了凑满 C3 而伪造区间**：多字形簇必须让这些字形共享同
/// 一个真实、非空的 source cluster；不能塞空区间，也不能把字形丢掉。
///
/// > 后端返回 `Err` 时，`yu-layout` 会逐 cluster 重试并最终用 U+FFFD 可见
/// > 降级；**契约违约的 `Ok` 结果不会走这条路**，仍然是硬错误。覆盖面不足与
/// > 返回错误几何/cluster 不能混为一谈。
pub trait ShapingProvider {
    type Error: fmt::Display;

    /// Production native backends expose complete paragraph layout. Providers
    /// used by deterministic metrics tests may keep the reference line engine.
    /// A native provider error is propagated, never retried through that engine.
    fn paragraph_provider(&self) -> Option<&dyn crate::ParagraphLayoutProvider> {
        None
    }

    /// Font ascent and descent in logical units, at the same scale as shaping.
    /// Reference providers can omit metrics and retain their deterministic strut.
    fn font_metrics(
        &self,
        _face: FontFaceId,
        _scale: f32,
    ) -> Result<Option<(f32, f32)>, Self::Error> {
        Ok(None)
    }

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
