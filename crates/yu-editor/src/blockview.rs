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
    NoWidgets, StyleTable,
};
use yu_markdown::{BlockDecorations, BlockOrnament};
use yu_text::{ChangeSet, TextSnapshot};

use crate::blockinput::{BlockLayoutInput, BlockOrnaments};
use crate::geometry::{source_range_contains, upstream};
use crate::image::{ImagePlacement, build_image_placements, place_images_in_table};
use crate::table::{TableLayout, TableResizeCommit};
use crate::visual::VisualText;

/// 一个视觉簇：视觉几何加上它对应的源码。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlockCluster {
    source: TextRange,
    visual: VisualRange,
    line: usize,
    x: f32,
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
    /// 按度量排。列表标记只算宽度，不产字形。
    ///
    /// `visual` 必须是 `decorations` 投影出来的那一份——preedit 可以已经叠
    /// 在上面。两者对不上时 [`BlockLayoutInput`] 会拒绝。
    pub fn build<M: ClusterMetrics>(
        visual: &VisualText,
        decorations: &BlockDecorations,
        config: LayoutConfig,
        metrics: &M,
    ) -> Result<Self, LayoutError> {
        let input = BlockLayoutInput::from_decorations(decorations, visual, config, metrics)?;
        let layout = BlockLayout::build_all(
            input.layout_input(),
            config,
            input.styles(),
            &NoWidgets,
            input.line_styles(),
            metrics,
        )?;
        let table = table_of(decorations)
            .map(|table| {
                TableLayout::from_table(table, visual, &input, config, |text, source, style| {
                    crate::table::measure_with(text, source, style, metrics)
                })
            })
            .transpose()?;
        Self::assemble(visual, decorations, config, input, layout, table)
    }

    /// 按 shaping 后端排。列表标记的字形一并进字形流。
    pub fn build_shaped<S: ShapingProvider>(
        visual: &VisualText,
        decorations: &BlockDecorations,
        config: LayoutConfig,
        shaper: &S,
    ) -> Result<Self, LayoutError> {
        let input = BlockLayoutInput::from_decorations_shaped(decorations, visual, config, shaper)?;
        let layout = BlockLayout::build_shaped(
            input.layout_input(),
            config,
            input.styles(),
            &NoWidgets,
            input.line_styles(),
            shaper,
        )?;
        let table = table_of(decorations)
            .map(|table| {
                TableLayout::from_table(table, visual, &input, config, |text, source, style| {
                    crate::table::shape_with(text, source, style, shaper)
                })
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
    /// 表格块按网格算；其余按行盒累加，再让已经就绪的图片把它撑开——一张
    /// 解码后的图片可以比它那一行高，可滚动范围必须算上它，否则长文档尾部
    /// 滚不到。那个 bug 不报错。
    #[must_use]
    pub fn height(&self) -> f32 {
        let base = self
            .table
            .as_ref()
            .map_or_else(|| self.layout.height(), |table| table.bounds().height());
        self.images.iter().fold(base, |height, image| {
            height.max(image.bounds().y() + image.bounds().height())
        })
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
    /// 一处、点击落在另一处」——不 panic、不报错，只是不听话。S5 在 bidi
    /// 那一刀抓到过一次同样的毛病。
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
        let line = &self.lines[line_index];
        let mut visual = line.visual.start();
        let mut bias = Bias::Before;

        let content_start = self.content_start(line);
        if point.x() > content_start {
            let mut inside = false;
            for index in line.clusters.clone() {
                let cluster = self.clusters[index];
                if cluster.line_break {
                    continue;
                }
                if point.x() < cluster.x + cluster.width / 2.0 {
                    visual = cluster.visual.start();
                    bias = Bias::Before;
                    inside = true;
                    break;
                }
                visual = cluster.visual.end();
                bias = Bias::After;
            }
            // 落在行末：那个位置是**这一行**的末尾，不是下一行的开头。
            // 软换行处的两个位置分属 upstream 与 downstream（不变量 H5），
            // 报成 downstream 会让光标画到下一行去。
            if !inside && line_index + 1 < self.lines.len() {
                visual = self.line_content_visual_end(line);
                bias = Bias::Before;
            }
        }
        // 反过来的同一件事：落在行首的那个位置属于**这一行**，不是上一行的
        // 末尾。两条合起来才让 `hit` 与 `caret` 在软换行的两侧都对得上。
        if line_index > 0 && visual == line.visual.start() {
            bias = Bias::After;
        }
        let caret = self.caret_for_visual(visual, bias)?;
        Ok(BlockHit { caret, image: None })
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

    /// 解码后的图片尺寸到位，重算受影响的图片盒子（不变量 D7）。
    pub fn apply_image_intrinsic_sizes(
        &mut self,
        sizes: &[(TextRange, yu_layout::ImageIntrinsicSize)],
    ) -> Result<(), LayoutError> {
        for image in &mut self.images {
            if let Some((_, size)) = sizes
                .iter()
                .copied()
                .find(|(source, _)| *source == image.source())
            {
                image.apply_intrinsic_size(size, self.config)?;
            }
        }
        if let Some(table) = self.table.clone() {
            place_images_in_table(&mut self.images, &table)?;
        }
        Ok(())
    }

    /// 一行内容的起点 x：点在它左边时 caret 停在这里。
    ///
    /// 它必须与 [`BlockView::point_for_visual`] 在行首给出的位置一致——
    /// 两处各写一遍规则，就会「光标画在一处、点击落在另一处」。表格行的
    /// 内容从单元格的 padding 之后开始，普通行从行级缩进之后开始。
    fn content_start(&self, line: &BlockLine) -> f32 {
        if self.table.is_some() {
            return line
                .clusters
                .clone()
                .map(|index| self.clusters[index])
                .find(|cluster| !cluster.line_break)
                .map_or(0.0, |cluster| cluster.x);
        }
        self.layout
            .lines()
            .get(line.index)
            .map_or(0.0, yu_layout::LineBox::indent)
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
        if self.table.is_some() {
            return self.table_point_for_visual(visual, bias);
        }
        let caret = self.layout.caret(visual, affinity)?;
        Ok((caret.line(), caret.point()))
    }

    /// 表格里的 caret 位置来自网格，不是文字流。
    ///
    /// 文字流那条路（`BlockLayout::caret`）问的是排在文字流里的簇，而表格
    /// 的簇已经被搬进单元格了。两处必须走同一份簇，否则光标画在一个地方、
    /// 点击落在另一个地方。
    fn table_point_for_visual(
        &self,
        visual: VisualOffset,
        bias: Bias,
    ) -> Result<(usize, LayoutPoint), LayoutError> {
        let line_index = self.line_for_visual(visual, bias);
        let line = &self.lines[line_index];
        for index in line.clusters.clone() {
            let cluster = self.clusters[index];
            if visual <= cluster.visual.start() {
                return Ok((line_index, LayoutPoint::new(cluster.x, line.bounds.y())));
            }
            if visual < cluster.visual.end() {
                let x = match bias {
                    Bias::Before => cluster.x,
                    Bias::After => cluster.x + cluster.width,
                };
                return Ok((line_index, LayoutPoint::new(x, line.bounds.y())));
            }
            if visual == cluster.visual.end() {
                return Ok((
                    line_index,
                    LayoutPoint::new(cluster.x + cluster.width, line.bounds.y()),
                ));
            }
        }
        Ok((
            line_index,
            LayoutPoint::new(line.bounds.width(), line.bounds.y()),
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

    /// 把文字流的簇、字形与行搬进表格网格。
    ///
    /// 这段算法原样来自 v1 的 `LayoutSnapshot::apply_table_geometry`。它在
    /// 真实窗口里跑过，重写不会更对。S6 让表格变成真正的 block widget 时
    /// 它会被 widget 内部的逐单元格布局取代。
    fn apply_table_geometry(&mut self, table: &TableLayout) -> Result<(), LayoutError> {
        if table.revision() != self.revision() {
            return Err(LayoutError::Upstream(
                "table and text layout revisions differ".into(),
            ));
        }
        let original = self.clusters.clone();
        let mut targets = vec![None; self.clusters.len()];
        for cell in table.cells().iter().copied() {
            let mut x = cell.content_x();
            for (index, cluster) in original.iter().copied().enumerate() {
                if cluster.line_break || !source_range_contains(cell.source(), cluster.source()) {
                    continue;
                }
                if targets[index].is_some() {
                    return Err(LayoutError::Upstream(
                        "a visual cluster belongs to multiple table cells".into(),
                    ));
                }
                targets[index] = Some((cell.row(), cluster.x, x));
                self.clusters[index] = BlockCluster {
                    line: cell.row(),
                    x,
                    ..cluster
                };
                x += cluster.width;
            }
        }
        if self
            .clusters
            .iter()
            .zip(targets.iter())
            .any(|(cluster, target)| !cluster.line_break && target.is_none())
        {
            return Err(LayoutError::Upstream(
                "a table visual cluster has no source cell".into(),
            ));
        }

        let mut used = vec![false; self.clusters.len()];
        for glyph_index in 0..self.glyphs.len() {
            let original_glyph = self.glyphs[glyph_index];
            let Some((index, target)) = self
                .clusters
                .iter()
                .copied()
                .zip(targets.iter().copied())
                .enumerate()
                .find_map(|(index, (cluster, target))| {
                    (!used[index]
                        && target.is_some()
                        && cluster.source == original_glyph.source
                        && cluster.visual == original_glyph.visual)
                        .then_some((index, target))
                })
            else {
                return Err(LayoutError::Upstream(
                    "a shaped table glyph has no visual cluster".into(),
                ));
            };
            let (row, old_cluster_x, new_cluster_x) = target.expect("target checked above");
            used[index] = true;
            let old_baseline = self.baseline_for_flow_line(original_glyph.line);
            let y_offset = original_glyph.origin.y() - old_baseline;
            let x_offset = original_glyph.origin.x() - old_cluster_x;
            if !x_offset.is_finite() || !y_offset.is_finite() {
                return Err(LayoutError::InvalidPoint);
            }
            let new_baseline = table.row_height() * (row as f32 + 1.0);
            self.glyphs[glyph_index] = BlockGlyph {
                line: row,
                origin: LayoutPoint::new(new_cluster_x + x_offset, new_baseline + y_offset),
                ..original_glyph
            };
        }

        place_images_in_table(&mut self.images, table)?;

        let mut lines = Vec::with_capacity(table.row_sources().len());
        let mut cluster_start = 0;
        for (row, source) in table.row_sources().iter().copied().enumerate() {
            let start = cluster_start;
            if cluster_start < self.clusters.len() && self.clusters[cluster_start].line() < row {
                return Err(LayoutError::Upstream(
                    "table cluster lines are not ordered".into(),
                ));
            }
            while cluster_start < self.clusters.len() && self.clusters[cluster_start].line() == row
            {
                cluster_start += 1;
            }
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
            let visual = VisualRange::new(first.visual().start(), last.visual().end())
                .ok_or(LayoutError::OffsetOverflow)?;
            lines.push(BlockLine {
                index: row,
                source,
                visual,
                bounds: LayoutRect::new(
                    0.0,
                    first.bounds().y(),
                    table.bounds().width(),
                    table.row_height(),
                )?,
                baseline: table.row_height(),
                style: None,
                clusters: start..cluster_start,
            });
        }
        if cluster_start != self.clusters.len() {
            return Err(LayoutError::Upstream(
                "table cluster lines exceed table rows".into(),
            ));
        }
        self.lines = lines;
        Ok(())
    }

    fn baseline_for_flow_line(&self, index: usize) -> f32 {
        self.layout
            .lines()
            .get(index)
            .map_or(0.0, |line| line.bounds().y() + line.baseline())
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
        clusters.push(BlockCluster {
            source,
            visual: cluster.visual(),
            line: cluster.line(),
            x: cluster.x(),
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
