//! 一个块排完之后的样子：视觉几何 + 源码坐标 + 装饰。
//!
//! # 它取代了什么
//!
//! v1 的 `yu_layout::LayoutSnapshot` 同时干三件事：排文字、解释 Markdown
//! （标题字号、引用 gutter、列表标记、表格网格、图片盒子）、把视觉坐标换算
//! 回源码坐标。第二件事让布局层必须认识 Markdown——不变量 E1 禁止的那件事。
//!
//! v2 把这三件事分开：
//!
//! - 排文字是 [`BlockLayout`] 的事，它只看见视觉文本与不透明的样式 id；
//! - 解释 Markdown 是 [`BlockLayoutInput`] 的事（本 crate），它把语法翻译成
//!   字号倍率、缩进、网格；
//! - 换算源码坐标是 `DecorationSet` 的事——[`BlockView`] 隔着
//!   [`VisualText`] 问它，自己不再实现一遍（不变量 D4「这是投影映射链的
//!   唯一实现」）。
//!
//! [`BlockView`] 是把这三样拼起来的那个东西，也是产品侧唯一要打交道的类型。
//!
//! # 为什么它有自己的一套盒子
//!
//! [`BlockLayout`] 的输出**只有视觉坐标**，这是有意的。产品侧要的是源码
//! 坐标（选中、编辑、Accessibility 都按源码走），所以这里把每个盒子补上
//! 它的源码区间，得到 [`BlockCluster`] / [`BlockGlyph`] / [`BlockLine`]。
//! 补的时候问的是装饰集合的双向映射，不是自己再算一遍。

use std::ops::Range;

use yu_core::{
    ByteOffset, CaretAffinity, ClusterMetrics, FontFaceId, GlyphId, LineStyleId, Revision,
    ShapingProvider, TextRange, TextStyle, VisualOffset, VisualRange,
};
use yu_decoration::Bias;
use yu_layout::{
    BlockLayout, HeightIndex, HeightIndexError, LayoutConfig, LayoutError, LayoutPoint, LayoutRect,
    StyleTable, WidgetMeasure,
};
use yu_markdown::{BlockDecorations, BlockOrnament};
use yu_text::{ChangeSet, TextSnapshot};

use crate::blockinput::{BlockLayoutInput, BlockOrnaments};
use crate::geometry::upstream;
use crate::image::{ImagePlacement, build_image_placements, build_table_image_placements};
use crate::table::{TableLayout, TableResizeCommit};
use crate::visual::VisualText;
use crate::widget::{BlockWidgets, ImageSize, constraints_of};

/// 一个视觉簇：视觉几何加上它对应的源码。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlockCluster {
    source: TextRange,
    visual: VisualRange,
    line: usize,
    x: f32,
    y: f32,
    width: f32,
    style: TextStyle,
    line_break: bool,
}

impl BlockCluster {
    #[must_use]
    pub const fn source(self) -> TextRange {
        self.source
    }

    #[must_use]
    pub const fn visual(self) -> VisualRange {
        self.visual
    }

    #[must_use]
    pub const fn line(self) -> usize {
        self.line
    }

    #[must_use]
    pub const fn x(self) -> f32 {
        self.x
    }

    /// 簇所在那条**文字行**的上沿，在 block 局部坐标里。
    ///
    /// 它不总是 `lines()[line()].y()`：表格的一条 [`BlockLine`] 是一个网格
    /// 行，而格子里的内容可以换行——第二行的簇与第一行同属一个网格行，y
    /// 却差一个行高。少了这个字段，格内换行之后光标与选中高亮会画在这一格
    /// 的第一行上，不报错，只是画错。
    #[must_use]
    pub const fn y(self) -> f32 {
        self.y
    }

    #[must_use]
    pub const fn width(self) -> f32 {
        self.width
    }

    #[must_use]
    pub const fn style(self) -> TextStyle {
        self.style
    }

    #[must_use]
    pub const fn is_line_break(self) -> bool {
        self.line_break
    }
}

/// 一个排好位置的字形，附带它的源码区间。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlockGlyph {
    face: FontFaceId,
    glyph: GlyphId,
    source: TextRange,
    visual: VisualRange,
    line: usize,
    origin: LayoutPoint,
    style: TextStyle,
    size_scale: f32,
}

impl BlockGlyph {
    #[must_use]
    pub const fn face(self) -> FontFaceId {
        self.face
    }

    #[must_use]
    pub const fn glyph(self) -> GlyphId {
        self.glyph
    }

    #[must_use]
    pub const fn source(self) -> TextRange {
        self.source
    }

    #[must_use]
    pub const fn visual(self) -> VisualRange {
        self.visual
    }

    #[must_use]
    pub const fn line(self) -> usize {
        self.line
    }

    /// 字形基线左端在 block 空间里的位置。
    #[must_use]
    pub const fn origin(self) -> LayoutPoint {
        self.origin
    }

    #[must_use]
    pub const fn style(self) -> TextStyle {
        self.style
    }

    /// 相对 shaper 基准字号的倍率。栅格化按它选字号。
    #[must_use]
    pub const fn size_scale(self) -> f32 {
        self.size_scale
    }
}

/// 一条视觉行。
#[derive(Clone, Debug, PartialEq)]
pub struct BlockLine {
    index: usize,
    source: TextRange,
    visual: VisualRange,
    bounds: LayoutRect,
    baseline: f32,
    style: Option<LineStyleId>,
    clusters: Range<usize>,
}

impl BlockLine {
    #[must_use]
    pub const fn index(&self) -> usize {
        self.index
    }

    #[must_use]
    pub const fn source(&self) -> TextRange {
        self.source
    }

    #[must_use]
    pub const fn visual(&self) -> VisualRange {
        self.visual
    }

    #[must_use]
    pub const fn bounds(&self) -> LayoutRect {
        self.bounds
    }

    #[must_use]
    pub const fn y(&self) -> f32 {
        self.bounds.y()
    }

    #[must_use]
    pub const fn width(&self) -> f32 {
        self.bounds.width()
    }

    /// 行高。有 widget 的行比 `line_height` 高，所以它不是常数。
    #[must_use]
    pub const fn height(&self) -> f32 {
        self.bounds.height()
    }

    #[must_use]
    pub const fn baseline(&self) -> f32 {
        self.baseline
    }

    /// 这一行的行级样式 id，原样带出来给绘制方查表。
    #[must_use]
    pub const fn style(&self) -> Option<LineStyleId> {
        self.style
    }

    #[must_use]
    pub fn cluster_range(&self) -> Range<usize> {
        self.clusters.clone()
    }
}

/// caret 落在哪。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlockCaret {
    source: ByteOffset,
    visual: VisualOffset,
    line: usize,
    point: LayoutPoint,
    bias: Bias,
}

impl BlockCaret {
    #[must_use]
    pub const fn source(self) -> ByteOffset {
        self.source
    }

    #[must_use]
    pub const fn visual(self) -> VisualOffset {
        self.visual
    }

    #[must_use]
    pub const fn line(self) -> usize {
        self.line
    }

    #[must_use]
    pub const fn point(self) -> LayoutPoint {
        self.point
    }

    #[must_use]
    pub const fn bias(self) -> Bias {
        self.bias
    }
}

/// 一次点击落在哪。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlockHit {
    caret: BlockCaret,
    image: Option<TextRange>,
}

impl BlockHit {
    #[must_use]
    pub const fn source(self) -> ByteOffset {
        self.caret.source
    }

    #[must_use]
    pub const fn visual(self) -> VisualOffset {
        self.caret.visual
    }

    #[must_use]
    pub const fn line(self) -> usize {
        self.caret.line
    }

    #[must_use]
    pub const fn point(self) -> LayoutPoint {
        self.caret.point
    }

    #[must_use]
    pub const fn bias(self) -> Bias {
        self.caret.bias
    }

    /// 点在某张图片上时给出它的源码区间。
    #[must_use]
    pub const fn image(self) -> Option<TextRange> {
        self.image
    }

    pub(crate) const fn image_hit(
        source: ByteOffset,
        visual: VisualOffset,
        line: usize,
        point: LayoutPoint,
        bias: Bias,
        image: TextRange,
    ) -> Self {
        Self {
            caret: BlockCaret {
                source,
                visual,
                line,
                point,
                bias,
            },
            image: Some(image),
        }
    }
}

/// 一个块排完之后的全部结果。
#[derive(Clone, Debug)]
pub struct BlockView {
    visual: VisualText,
    decorations: BlockDecorations,
    config: LayoutConfig,
    input: BlockLayoutInput,
    layout: BlockLayout,
    lines: Vec<BlockLine>,
    clusters: Vec<BlockCluster>,
    glyphs: Vec<BlockGlyph>,
    images: Vec<ImagePlacement>,
    table: Option<TableLayout>,
}

impl BlockView {
    /// 按度量排，图片一律画 placeholder。
    ///
    /// 命中测试、Accessibility、纯度量排版走这一条：它们不关心图片解码没有。
    pub fn build<M: ClusterMetrics>(
        visual: &VisualText,
        decorations: &BlockDecorations,
        config: LayoutConfig,
        metrics: &M,
    ) -> Result<Self, LayoutError> {
        Self::build_with_images(visual, decorations, config, metrics, &[])
    }

    /// 按度量排。列表标记只算宽度，不产字形。
    ///
    /// `visual` 必须是 `decorations` 投影出来的那一份——preedit 可以已经叠
    /// 在上面。两者对不上时 [`BlockLayoutInput`] 会拒绝。
    ///
    /// `sizes` 是已经解码到位的图片，没列进来的画 placeholder（不变量 D7）。
    pub fn build_with_images<M: ClusterMetrics>(
        visual: &VisualText,
        decorations: &BlockDecorations,
        config: LayoutConfig,
        metrics: &M,
        sizes: &[ImageSize],
    ) -> Result<Self, LayoutError> {
        let input = BlockLayoutInput::from_decorations(decorations, visual, config, metrics)?;
        let widgets = BlockWidgets::new(decorations.widgets(), sizes);
        let layout = BlockLayout::build_all(
            input.layout_input(),
            config,
            input.styles(),
            &widgets,
            input.line_styles(),
            metrics,
        )?;
        let table = table_of(decorations)
            .map(|table| {
                TableLayout::from_table(
                    table,
                    visual,
                    &input,
                    &widgets,
                    config,
                    &crate::table::MetricsCells(metrics),
                )
            })
            .transpose()?;
        Self::assemble(visual, decorations, config, input, layout, table)
    }

    /// 按 shaping 后端排，图片一律画 placeholder。
    pub fn build_shaped<S: ShapingProvider>(
        visual: &VisualText,
        decorations: &BlockDecorations,
        config: LayoutConfig,
        shaper: &S,
    ) -> Result<Self, LayoutError> {
        Self::build_shaped_with_images(visual, decorations, config, shaper, &[])
    }

    /// 按 shaping 后端排。列表标记的字形一并进字形流。
    pub fn build_shaped_with_images<S: ShapingProvider>(
        visual: &VisualText,
        decorations: &BlockDecorations,
        config: LayoutConfig,
        shaper: &S,
        sizes: &[ImageSize],
    ) -> Result<Self, LayoutError> {
        let input = BlockLayoutInput::from_decorations_shaped(decorations, visual, config, shaper)?;
        let widgets = BlockWidgets::new(decorations.widgets(), sizes);
        let layout = BlockLayout::build_shaped(
            input.layout_input(),
            config,
            input.styles(),
            &widgets,
            input.line_styles(),
            shaper,
        )?;
        let table = table_of(decorations)
            .map(|table| {
                TableLayout::from_table(
                    table,
                    visual,
                    &input,
                    &widgets,
                    config,
                    &crate::table::ShapedCells(shaper),
                )
            })
            .transpose()?;
        Self::assemble(visual, decorations, config, input, layout, table)
    }

    fn assemble(
        visual: &VisualText,
        decorations: &BlockDecorations,
        config: LayoutConfig,
        input: BlockLayoutInput,
        layout: BlockLayout,
        table: Option<TableLayout>,
    ) -> Result<Self, LayoutError> {
        let clusters = source_backed_clusters(visual, &layout, input.styles())?;
        let glyphs = source_backed_glyphs(&layout, &clusters, input.ornaments())?;
        let lines = flow_lines(visual, &layout)?;
        let mut view = Self {
            visual: visual.clone(),
            decorations: decorations.clone(),
            config,
            input,
            layout,
            lines,
            clusters,
            glyphs,
            images: Vec::new(),
            table,
        };
        view.images = build_image_placements(&view)?;
        if let Some(table) = view.table.clone() {
            view.apply_table_geometry(&table)?;
        }
        Ok(view)
    }

    #[must_use]
    pub fn revision(&self) -> Revision {
        self.visual.revision()
    }

    #[must_use]
    pub fn source_range(&self) -> TextRange {
        self.visual.source_range()
    }

    #[must_use]
    pub fn visual_len(&self) -> VisualOffset {
        self.visual.visual_len()
    }

    #[must_use]
    pub const fn config(&self) -> LayoutConfig {
        self.config
    }

    /// 排文字的那一层。`yu-scene` 只需要这个加上字形。
    #[must_use]
    pub const fn layout(&self) -> &BlockLayout {
        &self.layout
    }

    /// 这个块的视觉字节流与它到源码的映射。
    #[must_use]
    pub const fn visual(&self) -> &VisualText {
        &self.visual
    }

    /// 这个块的装饰产出：装饰集合、两张 id 表、语义标注。
    #[must_use]
    pub const fn decorations(&self) -> &BlockDecorations {
        &self.decorations
    }

    #[must_use]
    pub fn lines(&self) -> &[BlockLine] {
        &self.lines
    }

    #[must_use]
    pub fn clusters(&self) -> &[BlockCluster] {
        &self.clusters
    }

    #[must_use]
    pub fn glyphs(&self) -> &[BlockGlyph] {
        &self.glyphs
    }

    #[must_use]
    pub fn images(&self) -> &[ImagePlacement] {
        &self.images
    }

    #[must_use]
    pub const fn table(&self) -> Option<&TableLayout> {
        self.table.as_ref()
    }

    /// 「长什么样」那部分装饰：标题级别、列表标记、引用竖条。
    #[must_use]
    pub const fn ornaments(&self) -> &BlockOrnaments {
        self.input.ornaments()
    }

    /// 块的高度。
    ///
    /// 表格块按网格算；其余按行盒累加。图片不需要在这里单独补一次：一张
    /// 解码后的图片撑高的是它所在的那**一行**，而行高本来就进了累加
    /// （`BlockLayout::height`）。此前图片是排完之后另贴上去的盒子，行不
    /// 知道它有多高，可滚动范围因此要在这里补一次 max——那个补丁随
    /// widget 化一起没了。
    #[must_use]
    pub fn height(&self) -> f32 {
        self.table
            .as_ref()
            .map_or_else(|| self.layout.height(), |table| table.bounds().height())
    }

    /// 每条视觉行的高度，给视口的高度索引用。
    pub fn height_index(&self) -> Result<HeightIndex, HeightIndexError> {
        HeightIndex::new(self.lines.iter().map(BlockLine::height))
    }

    /// 一次源码编辑之后这个块还在不在。改到块里就整块作废。
    ///
    /// 编辑落在块外时块里每一个偏移挪同样多，所以这里只做一次平移：装饰、
    /// 视觉文本、簇、字形、行、图片、表格网格全部按同一个常量走。让它们各自
    /// 问一遍锚点也对，只是把一个常量算了几百遍——而算错其中一份的表现是
    /// 「点击落在别处」，不报错。
    pub fn map_through(
        &self,
        changes: &ChangeSet,
        snapshot: &TextSnapshot,
    ) -> Result<Option<Self>, LayoutError> {
        if snapshot.revision() != changes.after() {
            return Err(LayoutError::Upstream(
                "快照与变更集的 Revision 对不上".into(),
            ));
        }
        let Some(delta) = self.visual.shift_through(changes).map_err(upstream)? else {
            return Ok(None);
        };
        let decorations = self
            .decorations
            .shifted(delta, snapshot.revision(), snapshot.len_bytes())
            .map_err(upstream)?;
        let visual = VisualText::new(snapshot, decorations.range(), decorations.set().clone())
            .map_err(upstream)?;
        let shift = |range: TextRange| shift_range(range, delta);
        let mut view = Self {
            visual,
            decorations,
            ..self.clone()
        };
        view.table = view
            .table
            .as_ref()
            .map(|table| table.shifted(delta))
            .transpose()?;
        view.clusters = view
            .clusters
            .iter()
            .copied()
            .map(|cluster| {
                Ok(BlockCluster {
                    source: shift(cluster.source)?,
                    ..cluster
                })
            })
            .collect::<Result<Vec<_>, LayoutError>>()?;
        view.glyphs = view
            .glyphs
            .iter()
            .copied()
            .map(|glyph| {
                Ok(BlockGlyph {
                    source: shift(glyph.source)?,
                    ..glyph
                })
            })
            .collect::<Result<Vec<_>, LayoutError>>()?;
        view.lines = view
            .lines
            .iter()
            .cloned()
            .map(|line| {
                Ok(BlockLine {
                    source: shift(line.source)?,
                    ..line
                })
            })
            .collect::<Result<Vec<_>, LayoutError>>()?;
        view.images = view
            .images
            .iter()
            .copied()
            .map(|image| image.shifted(delta))
            .collect::<Result<Vec<_>, LayoutError>>()?;
        Ok(Some(view))
    }

    /// 源码偏移落在哪。
    pub fn caret_for_source(
        &self,
        source: ByteOffset,
        bias: Bias,
    ) -> Result<BlockCaret, LayoutError> {
        let visual = self
            .visual
            .source_to_visual(source, bias)
            .map_err(upstream)?;
        self.caret_for_visual(visual, bias)
    }

    /// 视觉偏移落在哪。
    ///
    /// composition 的多个视觉边界有意映射到同一段 canonical 替换范围，所以
    /// 从源码那一侧查不回一个 preedit 内部的 caret——这个入口是给它准备的。
    pub fn caret_for_visual(
        &self,
        visual: VisualOffset,
        bias: Bias,
    ) -> Result<BlockCaret, LayoutError> {
        let source = match self
            .table
            .as_ref()
            .and_then(|table| table.source_for_visual_hit(&self.visual, visual))
        {
            Some(source) => source,
            None => self
                .visual
                .visual_to_source(visual, bias)
                .map_err(upstream)?,
        };
        let (line, point) = self.point_for_visual(visual, bias)?;
        Ok(BlockCaret {
            source,
            visual,
            line,
            point,
            bias,
        })
    }

    /// 一个 block 局部坐标点落在哪。
    ///
    /// 这里只决定**落在哪个视觉偏移上**；几何位置回头问
    /// [`BlockView::caret_for_visual`]。两处各写一遍规则，就会「光标画在
    /// 一处、点击落在另一处」——不 panic、不报错，只是不听话。
    ///
    /// # 文字流那条路交给 `BlockLayout::hit`
    ///
    /// 它枚举这一行**所有 caret 位置**取离点击处最近的一个，用的是与
    /// [`BlockLayout`] 画 caret 同一条规则。这里曾经自己按 x 从左到右扫簇、
    /// 「过了中点算下一个」——那条规则默认 x 随逻辑顺序单调递增，**bidi
    /// 重排之后不成立**：`abc مرحبا def` 里点在阿拉伯语段落上，光标会跳到
    /// 十个像素以外的另一个位置。S5 登记过这一项，源码映射收敛成一套之后
    /// 两条就能合流。
    ///
    /// 表格不走那条路：它的簇已经被搬进单元格了（见
    /// [`BlockView::apply_table_geometry`]），文字流的 x 对不上网格的 x。
    /// 表格那条与 [`BlockView::table_point_for_visual`] 配对，两处必须
    /// 走同一份簇。
    pub fn hit_test(&self, point: LayoutPoint) -> Result<BlockHit, LayoutError> {
        if !point.is_finite() {
            return Err(LayoutError::InvalidPoint);
        }
        if let Some(image) = self
            .images
            .iter()
            .find(|image| image.bounds().contains(point))
        {
            return Ok(image.hit(point));
        }
        let line_index = self.line_for_y(point.y());
        // 有两处的「落在哪一侧」行的规则（不变量 H5）分不出来，因为那两处
        // 的两个位置**不在同一个 x 上**，而 H5 管的是软换行的两侧（同一个
        // x）：widget 的左右两沿差着整个盒子的宽度，相邻两格的交界差着一整
        // 段被隐藏的竖线与空白。这两处由下面那一层直接给出。
        let (visual, forced) = match self.table.as_ref() {
            Some(table) => self.table_visual_for_point(table, point)?,
            None => {
                let hit = self.layout.hit(point)?;
                (hit.visual(), widget_bias(hit.widget_affinity()))
            }
        };
        let bias = forced.unwrap_or_else(|| self.hit_bias(line_index, visual));
        let caret = self.caret_for_visual(visual, bias)?;
        Ok(BlockHit { caret, image: None })
    }

    /// 表格里的点落在哪个视觉偏移上：找到格子，再问那一格自己的布局。
    ///
    /// 此前这里是一段手写的「按 x 从左往右扫、过了中点算下一个」——那是
    /// 文字流那一路在第六刀丢掉的规则，表格这边留了一份。两份规则会分叉，
    /// 而分叉的表现是「光标画在一处、点击落在另一处」。现在每一格有自己的
    /// [`BlockLayout`]，命中就是它的 `hit`，与画 caret 用的是同一条规则。
    fn table_visual_for_point(
        &self,
        table: &TableLayout,
        point: LayoutPoint,
    ) -> Result<(VisualOffset, Option<Bias>), LayoutError> {
        let Some((cell, layout)) = table.cell_at(point) else {
            return Ok((self.visual_len(), None));
        };
        let local = LayoutPoint::new(
            point.x() - cell.content_x(),
            (point.y() - cell.bounds().y()).max(0.0),
        );
        let hit = layout.hit(local)?;
        let visual = VisualOffset::new(
            hit.visual()
                .get()
                .saturating_add(cell.visual().start().get()),
        );
        // 三条规则按这个顺序：widget 的哪一沿最具体，其次是格子的边界，
        // 最后是格**内**的软换行。
        //
        // 相邻两格的内容在视觉字节流里是**紧挨着**的（中间的竖线与空白全被
        // 隐藏了），所以「上一格的末尾」与「下一格的开头」是同一个偏移。
        // 分开它们的只有 bias：`Before` 解析到上一格内容的末尾，`After`
        // 解析到下一格内容的开头。点在哪一格是这里唯一知道的事——猜错就是
        // 「点第二列，光标停在第一列」。
        //
        // 格**内**的软换行是另一回事：那两个位置属于同一格，由格子自己的
        // 布局说了算（不变量 H5）。这里一律 `Before` 的话，点在第二行会把
        // 光标画到第一行的末尾去。
        let bias = widget_bias(hit.widget_affinity()).unwrap_or_else(|| {
            if hit.visual() == VisualOffset::ZERO {
                Bias::After
            } else if hit.visual() == layout.visual_len() {
                Bias::Before
            } else {
                match hit.line_affinity() {
                    CaretAffinity::Upstream => Bias::Before,
                    CaretAffinity::Downstream => Bias::After,
                }
            }
        });
        Ok((visual, Some(bias)))
    }

    /// 落在软换行两侧的那个位置归哪一行（不变量 H5）。
    ///
    /// 行首那个位置属于**这一行**，不是上一行的末尾；行末那个位置属于这一
    /// 行，不是下一行的开头。报错一边，光标就画到另一行去——不 panic、
    /// 不报错，只是不听话。
    ///
    /// 只有最后一行的内容末尾之后才是 `After`：那里没有「下一行的行首」可
    /// 以抢这个位置。空行没有内容末尾，留在 `Before`。
    fn hit_bias(&self, line_index: usize, visual: VisualOffset) -> Bias {
        let line = &self.lines[line_index];
        if line_index > 0 && visual == line.visual.start() {
            return Bias::After;
        }
        if line_index + 1 < self.lines.len() {
            return Bias::Before;
        }
        if visual > line.visual.start() && visual == self.line_content_visual_end(line) {
            Bias::After
        } else {
            Bias::Before
        }
    }

    /// 会话内的表格列宽调整。canonical 源码与缓存布局都不动。
    pub fn apply_table_column_resize(
        &mut self,
        index: usize,
        delta: f32,
    ) -> Result<(), LayoutError> {
        let table = self
            .table
            .as_ref()
            .ok_or(LayoutError::Upstream("block is not a table".into()))?
            .resized_columns(index, delta)?;
        self.apply_table_geometry(&table)?;
        self.table = Some(table);
        Ok(())
    }

    /// 应用一次表格拖拽的提交结果。
    pub fn apply_table_resize(&mut self, commit: TableResizeCommit) -> Result<(), LayoutError> {
        if commit.revision() != self.revision() {
            return Err(LayoutError::Upstream(
                "table resize commit is bound to another revision".into(),
            ));
        }
        match commit.target() {
            crate::table::TableResizeTarget::Column { index } => {
                self.apply_table_column_resize(index, commit.delta())
            }
            crate::table::TableResizeTarget::Row { .. } => Ok(()),
        }
    }

    /// 这份排好的布局是不是欠着一个现在能量出来的 widget。
    ///
    /// 不变量 D7 的后半句：资源就绪之后受影响的块要重排一次。判据是「**还
    /// 在画 placeholder** 的那些 widget 里，有没有哪一个现在量得出真尺寸」
    /// ——只朝一个方向走，所以一个不带尺寸表的调用方（命中测试）不会把带
    /// 尺寸的那一份挤掉。
    #[must_use]
    pub fn needs_widget_rebuild(&self, sizes: &[ImageSize]) -> bool {
        let constraints = constraints_of(self.config);
        let widgets = BlockWidgets::new(self.decorations.widgets(), sizes);
        self.layout
            .pending_widgets()
            .into_iter()
            .filter_map(|widget| widgets.measure(widget, constraints))
            .any(|measurement| measurement.is_ready())
    }

    /// 一行里最后一个**文字**簇的视觉末尾。换行符不算内容。
    fn line_content_visual_end(&self, line: &BlockLine) -> VisualOffset {
        line.clusters
            .clone()
            .rev()
            .map(|index| self.clusters[index])
            .find_map(|cluster| (!cluster.line_break).then(|| cluster.visual.end()))
            .unwrap_or(line.visual.start())
    }

    fn line_for_y(&self, y: f32) -> usize {
        for (index, line) in self.lines.iter().enumerate() {
            if y < line.bounds.y() + line.bounds.height() {
                return index;
            }
        }
        self.lines.len().saturating_sub(1)
    }

    fn point_for_visual(
        &self,
        visual: VisualOffset,
        bias: Bias,
    ) -> Result<(usize, LayoutPoint), LayoutError> {
        if visual > self.visual_len() {
            return Err(LayoutError::VisualOutOfBounds(visual));
        }
        let affinity = match bias {
            Bias::Before => CaretAffinity::Upstream,
            Bias::After => CaretAffinity::Downstream,
        };
        if let Some(table) = self.table.as_ref() {
            return self.table_point_for_visual(table, visual, bias);
        }
        let caret = self.layout.caret(visual, affinity)?;
        Ok((caret.line(), caret.point()))
    }

    /// 表格里的 caret 位置来自那一格自己的布局。
    ///
    /// 与 [`BlockView::table_visual_for_point`] 是同一份布局的两个方向。
    /// 两处走同一份，「光标画在一处、点击落在另一处」才不可能发生。
    fn table_point_for_visual(
        &self,
        table: &TableLayout,
        visual: VisualOffset,
        bias: Bias,
    ) -> Result<(usize, LayoutPoint), LayoutError> {
        let affinity = match bias {
            Bias::Before => CaretAffinity::Upstream,
            Bias::After => CaretAffinity::Downstream,
        };
        let Some((cell, layout)) = table.cell_for_visual(visual, bias) else {
            let line = self.line_for_visual(visual, bias);
            let bounds = self.lines[line].bounds;
            return Ok((line, LayoutPoint::new(bounds.width(), bounds.y())));
        };
        let local = VisualOffset::new(
            visual
                .get()
                .saturating_sub(cell.visual().start().get())
                .min(layout.visual_len().get()),
        );
        let caret = layout.caret(local, affinity)?;
        Ok((
            cell.row(),
            LayoutPoint::new(
                cell.content_x() + caret.point().x(),
                cell.bounds().y() + caret.point().y(),
            ),
        ))
    }

    fn line_for_visual(&self, visual: VisualOffset, bias: Bias) -> usize {
        for (index, line) in self.lines.iter().enumerate() {
            if visual < line.visual.end()
                || (visual == line.visual.end()
                    && (bias == Bias::Before || index + 1 == self.lines.len()))
            {
                return index;
            }
        }
        self.lines.len().saturating_sub(1)
    }

    /// 用表格各格自己排出来的几何取代文字流那一份。
    ///
    /// 此前这里做的是**搬运**：整块排成一条线性流，再把排好的簇按源码区间
    /// 分派进格子、逐个改 x。那条流按整块宽度断行，格子里放不下的内容不会
    /// 重排，于是后一列的内容压在前一列上。现在每一格有自己的
    /// [`BlockLayout`]（零基），这里只把它平移到格子的位置上。
    ///
    /// 一条 [`BlockLine`] 仍然是**一个网格行**，不是一条文字行：同一条文字
    /// 行跨越几个格子时，它在视觉字节流里不是连续的一段，而 `BlockLine` 的
    /// `visual` 必须是。格内换行体现为这一行更高，簇各自带着自己的 `y`。
    fn apply_table_geometry(&mut self, table: &TableLayout) -> Result<(), LayoutError> {
        if table.revision() != self.revision() {
            return Err(LayoutError::Upstream(
                "table and text layout revisions differ".into(),
            ));
        }
        let styles = self.input.styles();
        let mut clusters = Vec::new();
        let mut glyphs = Vec::new();
        let mut lines: Vec<BlockLine> = Vec::with_capacity(table.rows().len());
        let mut row_start = 0_usize;
        let mut current_row: Option<usize> = None;

        for (index, cell) in table.cells().iter().copied().enumerate() {
            let layout = table
                .cell_layouts()
                .get(index)
                .ok_or(LayoutError::Upstream("table cell has no layout".into()))?;
            if current_row != Some(cell.row()) {
                if let Some(row) = current_row {
                    lines.push(self.table_line(table, row, row_start..clusters.len())?);
                    row_start = clusters.len();
                }
                current_row = Some(cell.row());
            }
            let origin = LayoutPoint::new(cell.content_x(), cell.bounds().y());
            let base = cell.visual().start();
            let first_cluster = clusters.len();
            for cluster in layout.clusters() {
                let line = layout
                    .lines()
                    .get(cluster.line())
                    .ok_or(LayoutError::Upstream(
                        "table cell cluster has no line".into(),
                    ))?;
                let visual = shift_visual(cluster.visual(), base)?;
                let start = self
                    .visual
                    .visual_to_source(visual.start(), Bias::After)
                    .map_err(upstream)?;
                let end = self
                    .visual
                    .visual_to_source(visual.end(), Bias::Before)
                    .map_err(upstream)?;
                let style = styles
                    .attrs(cluster.style())
                    .ok_or(LayoutError::UnknownStyle(cluster.style()))?
                    .style();
                clusters.push(BlockCluster {
                    source: TextRange::new(start, end.max(start))
                        .ok_or(LayoutError::OffsetOverflow)?,
                    visual,
                    line: cell.row(),
                    x: origin.x() + cluster.x(),
                    y: origin.y() + line.bounds().y(),
                    width: cluster.width(),
                    style,
                    line_break: cluster.is_line_break(),
                });
            }
            for glyph in layout.glyphs() {
                let visual = shift_visual(glyph.visual(), base)?;
                let cluster = clusters[first_cluster..]
                    .iter()
                    .find(|cluster| cluster.visual == visual)
                    .ok_or(LayoutError::Shaping(
                        "a shaped table glyph has no visual cluster".into(),
                    ))?;
                let point = LayoutPoint::new(
                    origin.x() + glyph.origin().x(),
                    origin.y() + glyph.origin().y(),
                );
                if !point.is_finite() {
                    return Err(LayoutError::InvalidPoint);
                }
                glyphs.push(BlockGlyph {
                    face: glyph.face(),
                    glyph: glyph.glyph(),
                    source: cluster.source,
                    visual,
                    line: cell.row(),
                    origin: point,
                    style: cluster.style,
                    size_scale: glyph.size_scale(),
                });
            }
        }
        if let Some(row) = current_row {
            lines.push(self.table_line(table, row, row_start..clusters.len())?);
        }

        self.clusters = clusters;
        self.glyphs = glyphs;
        self.lines = lines;
        self.images = build_table_image_placements(self, table)?;
        Ok(())
    }

    /// 一个网格行对应的那条 [`BlockLine`]。
    fn table_line(
        &self,
        table: &TableLayout,
        row: usize,
        clusters: Range<usize>,
    ) -> Result<BlockLine, LayoutError> {
        let geometry = table
            .rows()
            .get(row)
            .copied()
            .ok_or(LayoutError::Upstream("table row has no geometry".into()))?;
        let mut row_cells = table
            .cells()
            .iter()
            .copied()
            .filter(|cell| cell.row() == row);
        let first = row_cells
            .clone()
            .next()
            .ok_or(LayoutError::Upstream("table row has no cells".into()))?;
        let last = row_cells
            .next_back()
            .ok_or(LayoutError::Upstream("table row has no cells".into()))?;
        let source = table
            .row_sources()
            .get(row)
            .copied()
            .ok_or(LayoutError::Upstream("table row has no source".into()))?;
        Ok(BlockLine {
            index: row,
            source,
            visual: VisualRange::new(first.visual().start(), last.visual().end())
                .ok_or(LayoutError::OffsetOverflow)?,
            bounds: LayoutRect::new(0.0, geometry.y(), table.bounds().width(), geometry.height())?,
            baseline: self.config.line_height(),
            style: None,
            clusters,
        })
    }
}

/// 给每个视觉簇补上它的源码区间。
///
/// 问的是装饰集合的双向映射，不是自己再算一遍——不变量 D4 说投影映射链
/// 只有一个实现。簇的起点取 `After`、终点取 `Before`：一个簇不该把它两边
/// 被隐藏的语法也吞进来，否则选中一个字会连带选中它旁边的 `*`。
fn source_backed_clusters(
    visual: &VisualText,
    layout: &BlockLayout,
    styles: &crate::blockinput::BlockStyleTable,
) -> Result<Vec<BlockCluster>, LayoutError> {
    let mut clusters = Vec::with_capacity(layout.clusters().len());
    for cluster in layout.clusters() {
        let start = visual
            .visual_to_source(cluster.visual().start(), Bias::After)
            .map_err(upstream)?;
        let end = visual
            .visual_to_source(cluster.visual().end(), Bias::Before)
            .map_err(upstream)?;
        let source = TextRange::new(start, end.max(start)).ok_or(LayoutError::OffsetOverflow)?;
        // 字型取**解释之后**的那个，不是 run 自己声明的那个：标题把每一段
        // 都排成粗体，栅格化必须按实际用的字面来，否则字画得比量出来的窄。
        let style = styles
            .attrs(cluster.style())
            .ok_or(LayoutError::UnknownStyle(cluster.style()))?
            .style();
        let y = layout
            .lines()
            .get(cluster.line())
            .map_or(0.0, |line| line.bounds().y());
        clusters.push(BlockCluster {
            source,
            visual: cluster.visual(),
            line: cluster.line(),
            x: cluster.x(),
            y,
            width: cluster.width(),
            style,
            line_break: cluster.is_line_break(),
        });
    }
    Ok(clusters)
}

/// 给每个字形补上源码区间，并把列表标记的字形并进来。
///
/// 标记的字形排在最前面：它画在 gutter 里，而 gutter 在内容左边。
fn source_backed_glyphs(
    layout: &BlockLayout,
    clusters: &[BlockCluster],
    ornaments: &BlockOrnaments,
) -> Result<Vec<BlockGlyph>, LayoutError> {
    let mut glyphs = Vec::with_capacity(layout.glyphs().len());
    if let Some(marker) = ornaments.marker()
        && let Some(shaped) = marker.shaped()
    {
        let baseline = layout
            .lines()
            .first()
            .map_or(0.0, |line| line.bounds().y() + line.baseline());
        let mut x = marker.x();
        for run in shaped.runs() {
            for glyph in run.glyphs() {
                let origin = LayoutPoint::new(x + glyph.x_offset(), baseline + glyph.y_offset());
                if !origin.is_finite() {
                    return Err(LayoutError::InvalidPoint);
                }
                glyphs.push(BlockGlyph {
                    face: run.face(),
                    glyph: glyph.id(),
                    source: marker.source(),
                    visual: VisualRange::empty(VisualOffset::ZERO),
                    line: 0,
                    origin,
                    style: TextStyle::Plain,
                    size_scale: 1.0,
                });
                x += glyph.advance();
            }
        }
    }
    for glyph in layout.glyphs() {
        let cluster = clusters
            .iter()
            .find(|cluster| cluster.visual == glyph.visual())
            .ok_or(LayoutError::Shaping(
                "a shaped glyph has no visual cluster".into(),
            ))?;
        glyphs.push(BlockGlyph {
            face: glyph.face(),
            glyph: glyph.glyph(),
            source: cluster.source,
            visual: glyph.visual(),
            line: glyph.line(),
            origin: glyph.origin(),
            style: cluster.style,
            size_scale: glyph.size_scale(),
        });
    }
    Ok(glyphs)
}

/// 给每条视觉行补上它覆盖的源码区间。
///
/// 一行的源码从上一行的结尾接着算，到它视觉结尾对应的源码为止；最后一行
/// 一直算到块的结尾。这样**每一个源码字节都恰好属于一行**，包括被隐藏的
/// 语法标记——代码围栏的收尾那一行看起来是空的，但它拥有 ``` 那几个字节，
/// 少了这一条，按行查源码就会漏掉它们。
fn flow_lines(visual: &VisualText, layout: &BlockLayout) -> Result<Vec<BlockLine>, LayoutError> {
    let block = visual.source_range();
    let mut lines = Vec::with_capacity(layout.lines().len());
    let mut start = block.start();
    let count = layout.lines().len();
    for line in layout.lines() {
        let end = if line.index() + 1 == count {
            block.end()
        } else {
            visual
                .visual_to_source(line.visual().end(), Bias::Before)
                .map_err(upstream)?
        };
        let end = end.max(start).min(block.end());
        lines.push(BlockLine {
            index: line.index(),
            source: TextRange::new(start, end).ok_or(LayoutError::OffsetOverflow)?,
            visual: line.visual(),
            bounds: line.bounds(),
            baseline: line.baseline(),
            style: line.style(),
            clusters: line.cluster_range(),
        });
        start = end;
    }
    Ok(lines)
}

/// 这个块带着的表格网格，没有就是 `None`。
fn table_of(decorations: &BlockDecorations) -> Option<&yu_markdown::TableBlock> {
    decorations
        .line_styles()
        .iter()
        .find_map(|ornament| match ornament {
            BlockOrnament::Table(table) => Some(table),
            _ => None,
        })
}

/// widget 的哪一沿翻成 bias。没有 widget 参与就是 `None`。
const fn widget_bias(affinity: Option<CaretAffinity>) -> Option<Bias> {
    match affinity {
        Some(CaretAffinity::Upstream) => Some(Bias::Before),
        Some(CaretAffinity::Downstream) => Some(Bias::After),
        None => None,
    }
}

/// 一段零基的视觉区间平移到块空间里的 `base` 上。
fn shift_visual(range: VisualRange, base: VisualOffset) -> Result<VisualRange, LayoutError> {
    let shift = |offset: VisualOffset| -> Option<VisualOffset> {
        offset.get().checked_add(base.get()).map(VisualOffset::new)
    };
    shift(range.start())
        .zip(shift(range.end()))
        .and_then(|(start, end)| VisualRange::new(start, end))
        .ok_or(LayoutError::OffsetOverflow)
}

/// 一段源码区间平移 `delta` 个字节。
pub(crate) fn shift_range(range: TextRange, delta: i64) -> Result<TextRange, LayoutError> {
    let shift = |offset: ByteOffset| -> Option<ByteOffset> {
        u64::try_from(i64::try_from(offset.get()).ok()?.checked_add(delta)?)
            .ok()
            .map(ByteOffset::new)
    };
    shift(range.start())
        .zip(shift(range.end()))
        .and_then(|(start, end)| TextRange::new(start, end))
        .ok_or(LayoutError::OffsetOverflow)
}
