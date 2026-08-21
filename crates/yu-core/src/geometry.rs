//! 视觉坐标：一套实现，多个用类型区分的坐标空间。
//!
//! 源码坐标（`ByteOffset` / `TextRange` / `Utf16Offset` / `LineIndex`）早就
//! 收敛在 `position.rs`。视觉坐标没有：`yu-layout` 与 `yu-scene` 各自写了一
//! 份结构完全相同的 `Point` / `Rect`，`yu-editor` 与平台层则直接散着
//! `x/y/width/height: f32` 四元组。
//!
//! 但把它们合成**一个** `Rect` 是错的。它们本来就不是同一个东西：
//!
//! - `yu-layout` 的矩形以 block 左上角为原点；
//! - `yu-scene` 的矩形以文档左上角为原点；
//! - 平台 damage scissor 用的是物理像素。
//!
//! 两类真实事故都出在这条缝上：`768b5e3` 把 CTLine 的绝对坐标当成了 run 内
//! 的相对坐标，`5fac1fe` 在已经是逻辑坐标的位置上又乘了一次 backing scale。
//! 两次都不报错，只是画错，都要靠真实窗口才能发现。
//!
//! 所以这里收敛的是**实现**，不是**空间**：`Point<S>` / `Size<S>` /
//! `Rect<S>` 只写一遍算术与校验，空间进入类型。把 block 局部矩形传给要文档
//! 坐标的函数不再是一个「看起来对」的调用，而是编译不过。跨空间只有两条路：
//! [`Rect::translate_into`]（平移原点）与 [`Rect::scale`]（换单位），两者都
//! 必须显式写出来。

use std::error::Error;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;

/// 一个视觉坐标空间。
///
/// 实现者都是零大小的标记类型，不构造实例，只作为 `Point` / `Rect` 的类型
/// 参数存在。
pub trait CoordinateSpace: 'static {
    /// 出现在 `Debug` 输出里，便于在日志中分辨两个数值相同但空间不同的矩形。
    const NAME: &'static str;

    /// 该空间对矩形的**额外**约束。
    ///
    /// 通用约束（四个分量有限、宽高非负、右下角有限）由 [`Rect::new`] 保证，
    /// 不必在这里重复。默认没有额外约束。
    #[must_use]
    fn accepts_rect(_x: f32, _y: f32, _width: f32, _height: f32) -> bool {
        true
    }
}

/// Block 局部坐标：原点是该 block 的左上角，单位是逻辑像素。
///
/// 额外约束是 `x >= 0 && y >= 0 && height > 0`：block 局部坐标不会落在自己
/// 左上角之外，而这里的每个矩形都是要画出来的盒子（图片、表格单元、引用条、
/// 代码块背景），零高度的盒子是构造错误而不是空盒子。
///
/// 注意约束只加在矩形上，不加在点上：hit-test 的入参可以落在 block 之外，
/// 那是「没命中」，不是非法输入。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Block;

impl CoordinateSpace for Block {
    const NAME: &'static str = "block";

    fn accepts_rect(x: f32, y: f32, _width: f32, height: f32) -> bool {
        x >= 0.0 && y >= 0.0 && height > 0.0
    }
}

/// 文档坐标：原点是文档内容的左上角，单位是逻辑像素。
///
/// 这是 scene 与 RenderPlan 使用的空间。它**不含**滚动位移——滚动是渲染时
/// 的整帧平移，不改变 primitive 的坐标。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Document;

impl CoordinateSpace for Document {
    const NAME: &'static str = "document";
}

/// 物理像素：drawable 表面上的实际像素，原点是表面左上角。
///
/// 只有平台后端该用它——damage scissor、纹理尺寸、栅格化目标。逻辑坐标乘上
/// [`Scale<Document, Device>`](Scale) 才能得到它，没有别的路径。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Device;

impl CoordinateSpace for Device {
    const NAME: &'static str = "device";
}

/// 构造几何量时被拒绝的原因。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeometryError {
    /// 某个分量是 NaN 或无穷，或者右下角溢出成了无穷。
    NotFinite(&'static str),
    /// 宽或高为负。
    NegativeExtent(&'static str),
    /// 违反了该坐标空间自己的约束，见 [`CoordinateSpace::accepts_rect`]。
    OutsideSpace(&'static str),
    /// 缩放因子不是有限正数。
    InvalidScale,
}

impl fmt::Display for GeometryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFinite(space) => {
                write!(formatter, "{space} geometry must be finite")
            }
            Self::NegativeExtent(space) => {
                write!(formatter, "{space} geometry must have non-negative extent")
            }
            Self::OutsideSpace(space) => {
                write!(
                    formatter,
                    "geometry is outside the {space} coordinate space"
                )
            }
            Self::InvalidScale => formatter.write_str("scale must be finite and positive"),
        }
    }
}

impl Error for GeometryError {}

/// 一个坐标空间里的点，单位由空间决定。
///
/// `PhantomData<fn() -> S>` 而不是 `PhantomData<S>`：前者不会让 `Point` 的
/// 自动 trait（`Send` / `Sync`）随标记类型变化，标记类型也不必是 `Copy`。
pub struct Point<S> {
    x: f32,
    y: f32,
    space: PhantomData<fn() -> S>,
}

impl<S: CoordinateSpace> Point<S> {
    /// 构造一个点。不做有限性检查——点可以合法地落在任何地方（hit-test 的
    /// 入参就可能在内容之外），非有限值由消费方按自己的语义拒绝。
    #[must_use]
    pub const fn new(x: f32, y: f32) -> Self {
        Self {
            x,
            y,
            space: PhantomData,
        }
    }

    pub const ORIGIN: Self = Self::new(0.0, 0.0);

    #[must_use]
    pub const fn x(self) -> f32 {
        self.x
    }

    #[must_use]
    pub const fn y(self) -> f32 {
        self.y
    }

    #[must_use]
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }

    /// 换单位。`scale` 显式写出「从哪个空间到哪个空间」。
    #[must_use]
    pub fn scale<T: CoordinateSpace>(self, scale: Scale<S, T>) -> Point<T> {
        Point::new(self.x * scale.factor(), self.y * scale.factor())
    }
}

/// 一个坐标空间里的尺寸。
pub struct Size<S> {
    width: f32,
    height: f32,
    space: PhantomData<fn() -> S>,
}

impl<S: CoordinateSpace> Size<S> {
    pub fn new(width: f32, height: f32) -> Result<Self, GeometryError> {
        if !width.is_finite() || !height.is_finite() {
            return Err(GeometryError::NotFinite(S::NAME));
        }
        if width < 0.0 || height < 0.0 {
            return Err(GeometryError::NegativeExtent(S::NAME));
        }
        Ok(Self {
            width,
            height,
            space: PhantomData,
        })
    }

    #[must_use]
    pub const fn width(self) -> f32 {
        self.width
    }

    #[must_use]
    pub const fn height(self) -> f32 {
        self.height
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.width == 0.0 || self.height == 0.0
    }

    pub fn scale<T: CoordinateSpace>(self, scale: Scale<S, T>) -> Result<Size<T>, GeometryError> {
        Size::new(self.width * scale.factor(), self.height * scale.factor())
    }
}

/// 一个坐标空间里的半开矩形。
pub struct Rect<S> {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    space: PhantomData<fn() -> S>,
}

impl<S: CoordinateSpace> Rect<S> {
    /// 构造一个矩形，先查通用约束再查该空间自己的约束。
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Result<Self, GeometryError> {
        if !x.is_finite() || !y.is_finite() || !width.is_finite() || !height.is_finite() {
            return Err(GeometryError::NotFinite(S::NAME));
        }
        if width < 0.0 || height < 0.0 {
            return Err(GeometryError::NegativeExtent(S::NAME));
        }
        // 右下角单独查一次：两个有限的数相加仍可能溢出成无穷，而后续的
        // 相交与并集判断全都用右下角。
        if !(x + width).is_finite() || !(y + height).is_finite() {
            return Err(GeometryError::NotFinite(S::NAME));
        }
        if !S::accepts_rect(x, y, width, height) {
            return Err(GeometryError::OutsideSpace(S::NAME));
        }
        Ok(Self {
            x,
            y,
            width,
            height,
            space: PhantomData,
        })
    }

    #[must_use]
    pub const fn x(self) -> f32 {
        self.x
    }

    #[must_use]
    pub const fn y(self) -> f32 {
        self.y
    }

    #[must_use]
    pub const fn width(self) -> f32 {
        self.width
    }

    #[must_use]
    pub const fn height(self) -> f32 {
        self.height
    }

    #[must_use]
    pub fn right(self) -> f32 {
        self.x + self.width
    }

    #[must_use]
    pub fn bottom(self) -> f32 {
        self.y + self.height
    }

    #[must_use]
    pub fn origin(self) -> Point<S> {
        Point::new(self.x, self.y)
    }

    #[must_use]
    pub fn is_empty(self) -> bool {
        self.width == 0.0 || self.height == 0.0
    }

    /// 点是否落在矩形内，含四条边。非有限的点永远不命中。
    #[must_use]
    pub fn contains(self, point: Point<S>) -> bool {
        point.is_finite()
            && point.x() >= self.x
            && point.x() <= self.right()
            && point.y() >= self.y
            && point.y() <= self.bottom()
    }

    /// 包含两者的最小矩形。
    ///
    /// 不返回 `Result`：两个已经合法的矩形的并集，四个分量都在它们的凸包里，
    /// 必然满足通用约束。空间自己的额外约束同理——`accepts_rect` 只允许收紧
    /// 到「原点非负、高度为正」这类对并集封闭的条件。
    #[must_use]
    pub fn union(self, other: Self) -> Self {
        let left = self.x.min(other.x);
        let top = self.y.min(other.y);
        let right = self.right().max(other.right());
        let bottom = self.bottom().max(other.bottom());
        Self {
            x: left,
            y: top,
            width: right - left,
            height: bottom - top,
            space: PhantomData,
        }
    }

    /// 是否相交或相切。相切算命中，因为 damage 区域按边界对齐时不能漏掉。
    #[must_use]
    pub fn intersects_or_touches(self, other: Self) -> bool {
        self.x <= other.right()
            && other.x <= self.right()
            && self.y <= other.bottom()
            && other.y <= self.bottom()
    }

    /// 平移到另一个坐标空间：`origin` 是本空间的原点在目标空间中的位置。
    ///
    /// 这是「block 局部 → 文档」唯一的合法通道。写出这一行就等于声明了
    /// 「我知道我在换坐标系，而且我知道这个 block 的原点在哪」。
    pub fn translate_into<T: CoordinateSpace>(
        self,
        origin: Point<T>,
    ) -> Result<Rect<T>, GeometryError> {
        Rect::new(
            self.x + origin.x(),
            self.y + origin.y(),
            self.width,
            self.height,
        )
    }

    /// 换单位，典型用途是逻辑像素 → 物理像素。
    pub fn scale<T: CoordinateSpace>(self, scale: Scale<S, T>) -> Result<Rect<T>, GeometryError> {
        Rect::new(
            self.x * scale.factor(),
            self.y * scale.factor(),
            self.width * scale.factor(),
            self.height * scale.factor(),
        )
    }
}

/// 两个坐标空间之间的换算因子。
///
/// 方向写在类型里，所以「乘了两次 backing scale」这类错误不再是一个能通过
/// 编译的表达式：`Rect<Device>` 上没有 `scale::<Device>` 可用。
pub struct Scale<From, To> {
    factor: f32,
    spaces: PhantomData<fn() -> (From, To)>,
}

impl<From: CoordinateSpace, To: CoordinateSpace> Scale<From, To> {
    pub fn new(factor: f32) -> Result<Self, GeometryError> {
        if !factor.is_finite() || factor <= 0.0 {
            return Err(GeometryError::InvalidScale);
        }
        Ok(Self {
            factor,
            spaces: PhantomData,
        })
    }

    #[must_use]
    pub const fn factor(self) -> f32 {
        self.factor
    }

    /// 反向换算。
    #[must_use]
    pub fn inverse(self) -> Scale<To, From> {
        Scale {
            // factor 已经保证是有限正数，倒数同样是有限正数。
            factor: 1.0 / self.factor,
            spaces: PhantomData,
        }
    }
}

// --- 手写的 trait 实现 ---
//
// derive 会给标记类型加上 `S: Clone` 之类的约束，而标记类型只是个空间名字，
// 不该被要求实现这些。`PhantomData<fn() -> S>` 本身对任何 S 都是 Copy 的。

macro_rules! impl_copy_clone {
    ($name:ident<$($param:ident),+>) => {
        impl<$($param),+> Clone for $name<$($param),+> {
            fn clone(&self) -> Self {
                *self
            }
        }

        impl<$($param),+> Copy for $name<$($param),+> {}
    };
}

impl_copy_clone!(Point<S>);
impl_copy_clone!(Size<S>);
impl_copy_clone!(Rect<S>);
impl_copy_clone!(Scale<From, To>);

impl<S> PartialEq for Point<S> {
    fn eq(&self, other: &Self) -> bool {
        self.x == other.x && self.y == other.y
    }
}

impl<S> PartialEq for Size<S> {
    fn eq(&self, other: &Self) -> bool {
        self.width == other.width && self.height == other.height
    }
}

impl<S> PartialEq for Rect<S> {
    fn eq(&self, other: &Self) -> bool {
        self.x == other.x
            && self.y == other.y
            && self.width == other.width
            && self.height == other.height
    }
}

impl<From, To> PartialEq for Scale<From, To> {
    fn eq(&self, other: &Self) -> bool {
        self.factor == other.factor
    }
}

impl<S> Hash for Scale<S, S> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.factor.to_bits().hash(state);
    }
}

impl<S: CoordinateSpace> fmt::Debug for Point<S> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Point<{}>({}, {})", S::NAME, self.x, self.y)
    }
}

impl<S: CoordinateSpace> fmt::Debug for Size<S> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "Size<{}>({}x{})",
            S::NAME,
            self.width,
            self.height
        )
    }
}

impl<S: CoordinateSpace> fmt::Debug for Rect<S> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "Rect<{}>({}, {}, {}x{})",
            S::NAME,
            self.x,
            self.y,
            self.width,
            self.height
        )
    }
}

impl<From: CoordinateSpace, To: CoordinateSpace> fmt::Debug for Scale<From, To> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "Scale<{} -> {}>({})",
            From::NAME,
            To::NAME,
            self.factor
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_rejects_non_finite_and_negative_extent() {
        assert_eq!(
            Rect::<Document>::new(f32::NAN, 0.0, 1.0, 1.0),
            Err(GeometryError::NotFinite("document"))
        );
        assert_eq!(
            Rect::<Document>::new(0.0, 0.0, -1.0, 1.0),
            Err(GeometryError::NegativeExtent("document"))
        );
    }

    #[test]
    fn rect_rejects_a_bottom_right_that_overflows_to_infinity() {
        // 四个分量各自有限，右下角却不是——相交与并集判断全都建立在右下角上。
        assert_eq!(
            Rect::<Document>::new(f32::MAX, 0.0, f32::MAX, 1.0),
            Err(GeometryError::NotFinite("document"))
        );
    }

    #[test]
    fn block_space_keeps_the_rules_yu_layout_used_to_hand_write() {
        // 这套规则是从 yu-layout 原来手写的 LayoutRect::new 搬过来的，
        // 逐条钉住，免得日后有人「顺手放宽」。
        assert!(Rect::<Block>::new(0.0, 0.0, 1.0, 1.0).is_ok());
        // 零宽度是合法的（空单元格），零高度不是（画不出来的盒子）。
        assert!(Rect::<Block>::new(0.0, 0.0, 0.0, 1.0).is_ok());
        assert!(Rect::<Block>::new(0.0, 0.0, 1.0, 0.0).is_err());
        assert!(Rect::<Block>::new(-0.1, 0.0, 1.0, 1.0).is_err());
        assert!(Rect::<Block>::new(0.0, -0.1, 1.0, 1.0).is_err());
        assert!(Rect::<Block>::new(0.0, 0.0, -1.0, 1.0).is_err());
        assert!(Rect::<Block>::new(f32::NAN, 0.0, 1.0, 1.0).is_err());
        assert!(Rect::<Block>::new(f32::MAX, 0.0, f32::MAX, 1.0).is_err());
        // 点不受空间约束：hit-test 可以问 block 之外的位置，那是「没命中」。
        assert!(Point::<Block>::new(-5.0, -5.0).is_finite());
    }

    #[test]
    fn block_space_rejects_what_document_space_accepts() {
        // 同样的四个数，在两个空间里的合法性不同——这正是空间进类型的理由。
        assert!(Rect::<Document>::new(-1.0, 0.0, 2.0, 2.0).is_ok());
        assert_eq!(
            Rect::<Block>::new(-1.0, 0.0, 2.0, 2.0),
            Err(GeometryError::OutsideSpace("block"))
        );

        assert!(Rect::<Document>::new(0.0, 0.0, 2.0, 0.0).is_ok());
        assert_eq!(
            Rect::<Block>::new(0.0, 0.0, 2.0, 0.0),
            Err(GeometryError::OutsideSpace("block"))
        );
    }

    #[test]
    fn translate_into_moves_a_block_rect_to_document_coordinates() {
        let block = Rect::<Block>::new(1.0, 2.0, 10.0, 4.0).expect("valid block rect");
        let document = block
            .translate_into(Point::<Document>::new(100.0, 200.0))
            .expect("translated rect stays valid");

        assert_eq!(document.x(), 101.0);
        assert_eq!(document.y(), 202.0);
        assert_eq!(document.width(), 10.0);
        assert_eq!(document.height(), 4.0);
    }

    #[test]
    fn scale_round_trips_through_its_inverse() {
        let logical = Rect::<Document>::new(3.0, 5.0, 7.0, 9.0).expect("valid document rect");
        let scale = Scale::<Document, Device>::new(2.0).expect("valid scale");
        let physical = logical.scale(scale).expect("scaled rect stays valid");

        assert_eq!(physical.x(), 6.0);
        assert_eq!(physical.width(), 14.0);
        assert_eq!(
            physical.scale(scale.inverse()).expect("inverse is valid"),
            logical
        );
    }

    #[test]
    fn scale_rejects_non_positive_factors() {
        assert_eq!(
            Scale::<Document, Device>::new(0.0),
            Err(GeometryError::InvalidScale)
        );
        assert_eq!(
            Scale::<Document, Device>::new(-1.0),
            Err(GeometryError::InvalidScale)
        );
    }

    #[test]
    fn union_covers_both_inputs_and_stays_in_the_space() {
        let left = Rect::<Block>::new(0.0, 0.0, 4.0, 4.0).expect("valid");
        let right = Rect::<Block>::new(10.0, 2.0, 2.0, 6.0).expect("valid");
        let union = left.union(right);

        assert_eq!(union.x(), 0.0);
        assert_eq!(union.y(), 0.0);
        assert_eq!(union.right(), 12.0);
        assert_eq!(union.bottom(), 8.0);
        // union 不返回 Result，所以它必须自己保证结果仍然是这个空间的合法值。
        assert!(Block::accepts_rect(
            union.x(),
            union.y(),
            union.width(),
            union.height()
        ));
    }

    #[test]
    fn contains_includes_edges_and_never_matches_non_finite_points() {
        let rect = Rect::<Document>::new(0.0, 0.0, 4.0, 4.0).expect("valid");

        assert!(rect.contains(Point::new(0.0, 0.0)));
        assert!(rect.contains(Point::new(4.0, 4.0)));
        assert!(!rect.contains(Point::new(4.1, 0.0)));
        assert!(!rect.contains(Point::new(f32::NAN, 0.0)));
    }

    #[test]
    fn debug_output_names_the_space() {
        let block = Rect::<Block>::new(0.0, 0.0, 1.0, 1.0).expect("valid");
        let document = Rect::<Document>::new(0.0, 0.0, 1.0, 1.0).expect("valid");

        // 数值相同、空间不同——日志里必须分得出来。
        assert_ne!(format!("{block:?}"), format!("{document:?}"));
        assert_eq!(format!("{block:?}"), "Rect<block>(0, 0, 1x1)");
    }
}
