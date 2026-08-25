//! v2 的块布局：输入只有样式化的视觉文本。
//!
//! # 与 `LayoutSnapshot` 的关系
//!
//! [`LayoutSnapshot`](crate::LayoutSnapshot) 是 v1 的实现，它的输入是
//! `yu_projection::Projection`——一个认识标题、引用、列表标记、表格的类型。
//! 那正是 overview-v2 §2.1 点名的泄漏：布局层为了排版必须先认识 Markdown。
//!
//! 这个模块换掉输入契约。[`BlockLayout`] 只看见三样东西：
//!
//! - **视觉文本**：源码里没被隐藏的那些字节，已经拼成一段连续的 `&str`；
//! - **[`StyledRun`]**：视觉文本上的样式区间，样式是不透明的
//!   [`StyleId`]；
//! - **样式表**：[`StyleTable`]，把 `StyleId` 翻译成排版属性
//!   [`TextAttrs`]。
//!
//! 表由产出装饰的那一层填（`yu-markdown` 知道 `StyleId(3)` 是强调），
//! 这一层只会读到「斜体、1.0 倍字号」。这就是不变量 E1 在布局层的落法：
//! 不是「不写 markdown 这个词」，是**拿不到**判断 Markdown 语义所需的信息。
//!
//! # 坐标
//!
//! 输出里没有源码坐标，只有 [`VisualOffset`]。视觉↔源码是
//! `DecorationSet` 的双向映射（不变量 D4「这是投影映射链的唯一实现」），
//! 让布局也做一遍就会有第二套映射。调用方在拿到布局结果之后自己换算。
//!
//! # 断行
//!
//! 这一版是**按 grapheme 贪心**，与 `LayoutSnapshot` 同算法。UAX #14 的
//! 断行机会、CJK 禁则与 UAX #9 bidi 是 S5 后续几刀的事；先让新旧两条路在
//! 同一套规则下逐点可比，把「换了输入契约」与「换了断行算法」分成两次改动。

use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;
use yu_core::{CaretAffinity, ClusterMetrics, StyleId, TextAttrs, VisualOffset, VisualRange};

use crate::{LayoutConfig, LayoutError, LayoutPoint};

/// 把 [`StyleId`] 翻译成排版属性。
///
/// 实现住在产出装饰的那一层——只有它知道自己给 `StyleId(3)` 赋了什么含义。
/// 未知 id 返回 `None`，布局会报 [`LayoutError::UnknownStyle`] 而不是
/// 悄悄按默认字型排：一个「样式表没跟上装饰产出」的 bug 应该响，
/// 不应该只是画得不对。
pub trait StyleTable {
    fn attrs(&self, style: StyleId) -> Option<TextAttrs>;
}

/// 一张不区分 id 的样式表，全部返回同一组属性。
///
/// 给「还没有样式产出」的调用方与测试用。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct UniformStyleTable {
    attrs: TextAttrs,
}

impl UniformStyleTable {
    #[must_use]
    pub const fn new(attrs: TextAttrs) -> Self {
        Self { attrs }
    }
}

impl StyleTable for UniformStyleTable {
    fn attrs(&self, _style: StyleId) -> Option<TextAttrs> {
        Some(self.attrs)
    }
}

/// 视觉文本上的一段样式。
///
/// `visual` 是**视觉**字节区间，不是源码区间。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct StyledRun {
    visual: VisualRange,
    style: StyleId,
}

impl StyledRun {
    #[must_use]
    pub const fn new(visual: VisualRange, style: StyleId) -> Self {
        Self { visual, style }
    }

    #[must_use]
    pub const fn visual(self) -> VisualRange {
        self.visual
    }

    #[must_use]
    pub const fn style(self) -> StyleId {
        self.style
    }
}

/// 一个块的布局输入。
///
/// `runs` 必须**无缝铺满** `text`：从 0 开始、首尾相接、终点等于
/// `text.len()`。空文本对应空 run 列表。校验在 [`BlockLayout::build`] 里，
/// 不满足直接报错——一个漏掉半段文本的 run 列表会画出少了几个字的一行，
/// 而那既不 panic 也不报错。
#[derive(Clone, Copy, Debug)]
pub struct LayoutInput<'a> {
    text: &'a str,
    runs: &'a [StyledRun],
}

impl<'a> LayoutInput<'a> {
    #[must_use]
    pub const fn new(text: &'a str, runs: &'a [StyledRun]) -> Self {
        Self { text, runs }
    }

    #[must_use]
    pub const fn text(&self) -> &'a str {
        self.text
    }

    #[must_use]
    pub const fn runs(&self) -> &'a [StyledRun] {
        self.runs
    }
}

/// 视觉文本里的一个 grapheme 盒。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClusterBox {
    visual: VisualRange,
    line: usize,
    x: f32,
    width: f32,
    style: StyleId,
    line_break: bool,
}

impl ClusterBox {
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
    pub const fn style(self) -> StyleId {
        self.style
    }

    /// 这个盒子是不是一个强制换行符。强制换行占视觉字节但不占宽度。
    #[must_use]
    pub const fn is_line_break(self) -> bool {
        self.line_break
    }
}

/// 一条视觉行。
#[derive(Clone, Debug, PartialEq)]
pub struct LineBox {
    index: usize,
    visual: VisualRange,
    y: f32,
    width: f32,
    clusters: Range<usize>,
}

impl LineBox {
    #[must_use]
    pub const fn index(&self) -> usize {
        self.index
    }

    #[must_use]
    pub const fn visual(&self) -> VisualRange {
        self.visual
    }

    #[must_use]
    pub const fn y(&self) -> f32 {
        self.y
    }

    #[must_use]
    pub const fn width(&self) -> f32 {
        self.width
    }

    #[must_use]
    pub fn cluster_range(&self) -> Range<usize> {
        self.clusters.clone()
    }
}

/// 一个视觉偏移落在布局里的位置。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CaretBox {
    visual: VisualOffset,
    line: usize,
    point: LayoutPoint,
}

impl CaretBox {
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
}

/// 一个块布局好的样子。只有视觉坐标，没有源码坐标。
#[derive(Clone, Debug, PartialEq)]
pub struct BlockLayout {
    config: LayoutConfig,
    visual_len: VisualOffset,
    lines: Vec<LineBox>,
    clusters: Vec<ClusterBox>,
}

struct LineCursor {
    index: usize,
    visual_start: VisualOffset,
    width: f32,
    cluster_start: usize,
}

/// 第一遍度量出来的一个 grapheme，还没有被分配到行。
///
/// 度量与断行分成两遍，因为 UAX #14 的断行机会要在整段视觉文本上算，
/// 而它不认识 [`StyledRun`] 的边界。第一遍走 run 拿宽度，第二遍走断行机会
/// 分行。
struct Measured {
    visual: VisualRange,
    style: StyleId,
    advance: f32,
    /// 这个 grapheme 本身就是一个强制换行（UAX #14 的 BK / CR / LF / NL）。
    mandatory_break: bool,
    /// 全是空白。行尾的空白不参与「排不下」的判断，它们悬在行外。
    space: bool,
}

/// UAX #14 里构成强制换行的字符。
///
/// 注意这与 `docs/specs/coordinates.md` 的「源码逻辑行只按 LF 分隔」不冲突：
/// 那条说的是 `LineIndex`（源码行），这里说的是**视觉行**。软换行早就让两者
/// 不是一回事了；LS / PS / FF / NEL 只是又多了几个视觉行会断而源码行不断的
/// 位置。
fn is_mandatory_break_char(value: char) -> bool {
    matches!(
        value,
        '\n' | '\r' | '\u{b}' | '\u{c}' | '\u{85}' | '\u{2028}' | '\u{2029}'
    )
}

impl BlockLayout {
    /// 按 [`LayoutConfig`] 与调用方的度量把输入排成视觉行。
    pub fn build<T: StyleTable, M: ClusterMetrics>(
        input: LayoutInput<'_>,
        config: LayoutConfig,
        styles: &T,
        metrics: &M,
    ) -> Result<Self, LayoutError> {
        config.validate()?;
        let visual_len =
            VisualOffset::try_from(input.text.len()).map_err(|_| LayoutError::OffsetOverflow)?;
        validate_runs(input, visual_len)?;

        let measured = measure(input, styles, metrics)?;
        let segment_starts = segment_starts(input.text, &measured)?;

        let mut layout = Self {
            config,
            visual_len,
            lines: Vec::new(),
            clusters: Vec::new(),
        };
        let mut cursor = LineCursor {
            index: 0,
            visual_start: VisualOffset::ZERO,
            width: 0.0,
            cluster_start: 0,
        };
        let mut last_was_break = false;

        for segment in segment_starts.windows(2) {
            let (from, to) = (segment[0], segment[1]);
            // 这一段里到最后一个非空白 grapheme 为止的宽度。行尾空白不参与
            // 判断——UAX #14 的断行机会在空白**之后**，让空白把整个词挤到
            // 下一行是错的。
            let fit = measured[from..to]
                .iter()
                .rev()
                .skip_while(|cluster| cluster.space || cluster.mandatory_break)
                .map(|cluster| cluster.advance)
                .sum::<f32>();
            if cursor.width > 0.0 && cursor.width + fit > config.max_width() {
                layout.push_line(&cursor, measured[from].visual.start())?;
                cursor = layout.next_line(&cursor, measured[from].visual.start());
            }

            for cluster in &measured[from..to] {
                if cluster.mandatory_break {
                    layout.clusters.push(ClusterBox {
                        visual: cluster.visual,
                        line: cursor.index,
                        x: cursor.width,
                        width: 0.0,
                        style: cluster.style,
                        line_break: true,
                    });
                    layout.push_line(&cursor, cluster.visual.end())?;
                    cursor = layout.next_line(&cursor, cluster.visual.end());
                    last_was_break = true;
                    continue;
                }
                // 一个自身就超过整行宽度的「词」必须还能排出来：段内退回
                // 按 grapheme 断（UAX #14 允许的应急断行）。
                if !cluster.space
                    && cursor.width > 0.0
                    && cursor.width + cluster.advance > config.max_width()
                {
                    layout.push_line(&cursor, cluster.visual.start())?;
                    cursor = layout.next_line(&cursor, cluster.visual.start());
                }
                layout.clusters.push(ClusterBox {
                    visual: cluster.visual,
                    line: cursor.index,
                    x: cursor.width,
                    width: cluster.advance,
                    style: cluster.style,
                    line_break: false,
                });
                cursor.width += cluster.advance;
                last_was_break = false;
            }
        }

        if layout.lines.is_empty() || !last_was_break {
            layout.push_line(&cursor, visual_len)?;
        } else {
            let empty = cursor.visual_start;
            layout.push_line(
                &LineCursor {
                    width: 0.0,
                    ..cursor
                },
                empty,
            )?;
        }
        Ok(layout)
    }

    #[must_use]
    pub const fn config(&self) -> LayoutConfig {
        self.config
    }

    #[must_use]
    pub const fn visual_len(&self) -> VisualOffset {
        self.visual_len
    }

    #[must_use]
    pub fn lines(&self) -> &[LineBox] {
        &self.lines
    }

    #[must_use]
    pub fn clusters(&self) -> &[ClusterBox] {
        &self.clusters
    }

    /// 块的高度。
    #[must_use]
    pub fn height(&self) -> f32 {
        self.lines.len() as f32 * self.config.line_height()
    }

    /// 视觉偏移落在哪里。
    ///
    /// `affinity` 只在偏移正好落在软换行边界上时起作用：
    /// [`CaretAffinity::Upstream`] 给上一行的行末，
    /// [`CaretAffinity::Downstream`] 给下一行的行首（见
    /// `docs/specs/coordinates.md`）。
    ///
    /// 落在一个 grapheme **内部**的偏移不是合法的 caret 位置，返回该 grapheme
    /// 的左边缘。
    pub fn caret(
        &self,
        visual: VisualOffset,
        affinity: CaretAffinity,
    ) -> Result<CaretBox, LayoutError> {
        if visual > self.visual_len {
            return Err(LayoutError::VisualOutOfBounds(visual));
        }
        let line_index = self.line_for_visual(visual, affinity);
        let line = &self.lines[line_index];
        // 行内没有簇时 `width` 就是 0，循环不跑，x 自然落在行首。
        let mut x = line.width;
        for index in line.clusters.clone() {
            let cluster = self.clusters[index];
            // 落在簇左边缘或簇**内部**都取左边缘：grapheme 内部不是合法的
            // caret 位置，往右靠会把光标画进一个字里。
            if visual < cluster.visual.end() {
                x = cluster.x;
                break;
            }
            x = cluster.x + cluster.width;
        }
        Ok(CaretBox {
            visual,
            line: line_index,
            point: LayoutPoint::new(x, line.y),
        })
    }

    /// 一个 block 局部坐标点落在哪个视觉偏移上。
    pub fn hit(&self, point: LayoutPoint) -> Result<CaretBox, LayoutError> {
        let line_index = self.line_for_y(point.y());
        let line = &self.lines[line_index];
        let mut visual = line.visual.start();
        let mut x = line.width;

        if point.x() <= 0.0 {
            x = 0.0;
        } else {
            for index in line.clusters.clone() {
                let cluster = self.clusters[index];
                if cluster.line_break {
                    continue;
                }
                if point.x() < cluster.x + cluster.width / 2.0 {
                    visual = cluster.visual.start();
                    x = cluster.x;
                    break;
                }
                visual = cluster.visual.end();
                x = cluster.x + cluster.width;
            }
            if point.x() >= line.width {
                visual = self.line_content_visual_end(line);
                x = line.width;
            }
        }
        Ok(CaretBox {
            visual,
            line: line_index,
            point: LayoutPoint::new(x, line.y),
        })
    }

    fn next_line(&self, cursor: &LineCursor, visual_start: VisualOffset) -> LineCursor {
        LineCursor {
            index: cursor.index.saturating_add(1),
            visual_start,
            width: 0.0,
            cluster_start: self.clusters.len(),
        }
    }

    fn push_line(
        &mut self,
        cursor: &LineCursor,
        visual_end: VisualOffset,
    ) -> Result<(), LayoutError> {
        let visual =
            VisualRange::new(cursor.visual_start, visual_end).ok_or(LayoutError::OffsetOverflow)?;
        let y = cursor.index as f32 * self.config.line_height();
        if !y.is_finite() {
            return Err(LayoutError::InvalidPoint);
        }
        self.lines.push(LineBox {
            index: cursor.index,
            visual,
            y,
            width: cursor.width,
            clusters: cursor.cluster_start..self.clusters.len(),
        });
        Ok(())
    }

    fn line_for_y(&self, y: f32) -> usize {
        let raw = (y / self.config.line_height()).floor();
        if raw.is_sign_negative() {
            0
        } else {
            (raw as usize).min(self.lines.len().saturating_sub(1))
        }
    }

    fn line_for_visual(&self, visual: VisualOffset, affinity: CaretAffinity) -> usize {
        for (index, line) in self.lines.iter().enumerate() {
            if visual < line.visual.end()
                || (visual == line.visual.end()
                    && (affinity == CaretAffinity::Upstream || index + 1 == self.lines.len()))
            {
                return index;
            }
        }
        self.lines.len().saturating_sub(1)
    }

    fn line_content_visual_end(&self, line: &LineBox) -> VisualOffset {
        line.clusters
            .clone()
            .rev()
            .map(|index| self.clusters[index])
            .find_map(|cluster| {
                if cluster.line_break {
                    None
                } else {
                    Some(cluster.visual.end())
                }
            })
            .unwrap_or(line.visual.start())
    }
}

/// 第一遍：把每个 run 切成 grapheme 并量出宽度。
fn measure<T: StyleTable, M: ClusterMetrics>(
    input: LayoutInput<'_>,
    styles: &T,
    metrics: &M,
) -> Result<Vec<Measured>, LayoutError> {
    let mut measured = Vec::new();
    for run in input.runs() {
        let attrs = styles
            .attrs(run.style())
            .ok_or(LayoutError::UnknownStyle(run.style()))?;
        let start =
            usize::try_from(run.visual().start().get()).map_err(|_| LayoutError::OffsetOverflow)?;
        let end =
            usize::try_from(run.visual().end().get()).map_err(|_| LayoutError::OffsetOverflow)?;
        let text = input
            .text()
            .get(start..end)
            .ok_or(LayoutError::RunNotOnCharBoundary)?;
        for (local, cluster_text) in text.grapheme_indices(true) {
            let visual_start =
                VisualOffset::try_from(start + local).map_err(|_| LayoutError::OffsetOverflow)?;
            let visual_end = VisualOffset::try_from(start + local + cluster_text.len())
                .map_err(|_| LayoutError::OffsetOverflow)?;
            let visual =
                VisualRange::new(visual_start, visual_end).ok_or(LayoutError::OffsetOverflow)?;
            let mandatory_break = cluster_text.chars().all(is_mandatory_break_char);
            let advance = if mandatory_break {
                0.0
            } else {
                metrics.advance(cluster_text, attrs.style()) * attrs.size_scale()
            };
            if !advance.is_finite() || advance < 0.0 {
                return Err(LayoutError::InvalidMetrics(advance.to_bits()));
            }
            measured.push(Measured {
                visual,
                style: run.style(),
                advance,
                mandatory_break,
                space: !mandatory_break && cluster_text.chars().all(char::is_whitespace),
            });
        }
    }
    Ok(measured)
}

/// 第二遍的输入：每一段的第一个 grapheme 下标，首尾各一个哨兵。
///
/// 段界来自 UAX #14 的断行机会。两侧都是升序的，所以是一次归并走位。
///
/// 落不到 grapheme 边界上的机会会被丢掉——断行不能把一个 grapheme 劈开，
/// 劈开画出来是两个不相干的字形，不 panic 也不报错。这一条是防御性的：
/// 在采样过的 ZWJ 序列、区域指示符、组合记号、泰文与天城文样本上，
/// UAX #14 没有给出过落在 grapheme 内部的机会。
fn segment_starts(text: &str, measured: &[Measured]) -> Result<Vec<usize>, LayoutError> {
    if measured.is_empty() {
        return Ok(Vec::new());
    }
    let mut starts = vec![0_usize];
    let mut cluster = 0_usize;
    for (offset, _) in unicode_linebreak::linebreaks(text) {
        let offset = u64::try_from(offset).map_err(|_| LayoutError::OffsetOverflow)?;
        while cluster < measured.len() && measured[cluster].visual.start().get() < offset {
            cluster += 1;
        }
        if cluster >= measured.len() {
            break;
        }
        if measured[cluster].visual.start().get() == offset
            && cluster > *starts.last().expect("至少有哨兵 0")
        {
            starts.push(cluster);
        }
    }
    starts.push(measured.len());
    Ok(starts)
}

/// runs 必须无缝铺满视觉文本，且每个边界都在 UTF-8 字符边界上。
fn validate_runs(input: LayoutInput<'_>, visual_len: VisualOffset) -> Result<(), LayoutError> {
    let mut expected = VisualOffset::ZERO;
    for run in input.runs {
        if run.visual.start() != expected {
            return Err(LayoutError::RunsNotContiguous {
                expected,
                found: run.visual.start(),
            });
        }
        for offset in [run.visual.start().get(), run.visual.end().get()] {
            let index = usize::try_from(offset).map_err(|_| LayoutError::OffsetOverflow)?;
            if !input.text.is_char_boundary(index) {
                return Err(LayoutError::RunNotOnCharBoundary);
            }
        }
        expected = run.visual.end();
    }
    if expected != visual_len {
        return Err(LayoutError::RunsNotContiguous {
            expected: visual_len,
            found: expected,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{BlockLayout, LayoutInput, StyleTable, StyledRun, UniformStyleTable};
    use crate::{LayoutConfig, LayoutError, LayoutPoint, MonospaceMetrics};
    use yu_core::{CaretAffinity, StyleId, TextAttrs, TextStyle, VisualOffset, VisualRange};

    fn visual(start: u64, end: u64) -> VisualRange {
        VisualRange::new(VisualOffset::new(start), VisualOffset::new(end)).expect("有序")
    }

    fn plain(text: &str) -> Vec<StyledRun> {
        vec![StyledRun::new(visual(0, text.len() as u64), StyleId(0))]
    }

    fn build(text: &str, runs: &[StyledRun], width: f32) -> Result<BlockLayout, LayoutError> {
        BlockLayout::build(
            LayoutInput::new(text, runs),
            LayoutConfig::new(width, 1.0),
            &UniformStyleTable::default(),
            &MonospaceMetrics::default(),
        )
    }

    /// 一份漏掉半段文本的 run 列表会画出少了几个字的一行，既不 panic 也不
    /// 报错——这个项目最危险的失败模式。所以铺不满就是错误。
    #[test]
    fn runs_must_tile_the_visual_text() {
        let text = "abcdef";
        let short = vec![StyledRun::new(visual(0, 3), StyleId(0))];
        assert_eq!(
            build(text, &short, 80.0),
            Err(LayoutError::RunsNotContiguous {
                expected: VisualOffset::new(6),
                found: VisualOffset::new(3),
            })
        );

        let gapped = vec![
            StyledRun::new(visual(0, 2), StyleId(0)),
            StyledRun::new(visual(3, 6), StyleId(0)),
        ];
        assert_eq!(
            build(text, &gapped, 80.0),
            Err(LayoutError::RunsNotContiguous {
                expected: VisualOffset::new(2),
                found: VisualOffset::new(3),
            })
        );

        let overlapping = vec![
            StyledRun::new(visual(0, 4), StyleId(0)),
            StyledRun::new(visual(3, 6), StyleId(0)),
        ];
        assert!(matches!(
            build(text, &overlapping, 80.0),
            Err(LayoutError::RunsNotContiguous { .. })
        ));

        assert!(build(text, &plain(text), 80.0).is_ok());
        assert!(build("", &[], 80.0).is_ok());
    }

    #[test]
    fn run_boundaries_must_land_on_char_boundaries() {
        let text = "中文";
        let split = vec![
            StyledRun::new(visual(0, 1), StyleId(0)),
            StyledRun::new(visual(1, 6), StyleId(0)),
        ];
        assert_eq!(
            build(text, &split, 80.0),
            Err(LayoutError::RunNotOnCharBoundary)
        );
    }

    /// 装饰产出与样式表脱节必须响。按默认字型排会画出一份「看起来正常但
    /// 强调没生效」的画面，没有任何东西会拦住它。
    #[test]
    fn unknown_style_is_an_error_not_a_default() {
        struct OnlyZero;
        impl StyleTable for OnlyZero {
            fn attrs(&self, style: StyleId) -> Option<TextAttrs> {
                (style == StyleId(0)).then(TextAttrs::default)
            }
        }
        let text = "ab";
        let runs = vec![StyledRun::new(visual(0, 2), StyleId(7))];
        let built = BlockLayout::build(
            LayoutInput::new(text, &runs),
            LayoutConfig::new(80.0, 1.0),
            &OnlyZero,
            &MonospaceMetrics::default(),
        );
        assert_eq!(built, Err(LayoutError::UnknownStyle(StyleId(7))));
    }

    /// 字号倍率是布局层唯一知道的「这段比别处大」——标题靠它变大，而布局层
    /// 看不见「标题」。几何差分压不到它（那条差分的语料全是 1.0 倍），
    /// 所以在这里单独钉住。
    #[test]
    fn size_scale_multiplies_advance_and_therefore_wrapping() {
        struct Doubling;
        impl StyleTable for Doubling {
            fn attrs(&self, style: StyleId) -> Option<TextAttrs> {
                let attrs = TextAttrs::new(TextStyle::Plain);
                match style {
                    StyleId(0) => Some(attrs),
                    StyleId(1) => attrs.with_size_scale(2.0),
                    _ => None,
                }
            }
        }
        let text = "abcd";
        let config = LayoutConfig::new(4.0, 1.0);
        let metrics = MonospaceMetrics::default();

        let normal = BlockLayout::build(
            LayoutInput::new(text, &[StyledRun::new(visual(0, 4), StyleId(0))]),
            config,
            &Doubling,
            &metrics,
        )
        .expect("布局");
        assert_eq!(normal.lines().len(), 1);
        assert_eq!(normal.lines()[0].width(), 4.0);

        let doubled = BlockLayout::build(
            LayoutInput::new(text, &[StyledRun::new(visual(0, 4), StyleId(1))]),
            config,
            &Doubling,
            &metrics,
        )
        .expect("布局");
        assert_eq!(doubled.clusters()[0].width(), 2.0);
        assert_eq!(doubled.lines().len(), 2, "倍率翻倍后 4 个字排不进宽度 4");
    }

    #[test]
    fn caret_rejects_offsets_past_the_block() {
        let text = "abc";
        let layout = build(text, &plain(text), 80.0).expect("布局");
        assert_eq!(
            layout.caret(VisualOffset::new(4), CaretAffinity::Downstream),
            Err(LayoutError::VisualOutOfBounds(VisualOffset::new(4)))
        );
        assert!(
            layout
                .caret(VisualOffset::new(3), CaretAffinity::Downstream)
                .is_ok()
        );
    }

    /// 软换行边界上 affinity 决定 caret 画在上一行末还是下一行首。
    /// 这两个位置几何上差一整行高，混淆了不 panic，只是光标跳到别处。
    #[test]
    fn affinity_picks_the_visual_line_at_a_soft_wrap() {
        let text = "abcdef";
        let layout = build(text, &plain(text), 3.0).expect("布局");
        assert_eq!(layout.lines().len(), 2);
        let boundary = VisualOffset::new(3);
        let upstream = layout
            .caret(boundary, CaretAffinity::Upstream)
            .expect("caret");
        let downstream = layout
            .caret(boundary, CaretAffinity::Downstream)
            .expect("caret");
        assert_eq!(upstream.line(), 0);
        assert_eq!(upstream.point(), LayoutPoint::new(3.0, 0.0));
        assert_eq!(downstream.line(), 1);
        assert_eq!(downstream.point(), LayoutPoint::new(0.0, 1.0));
    }

    /// 把每一行的视觉区间还原成文本，便于把断行结果写成看得懂的期望。
    fn line_texts(text: &str, width: f32) -> Vec<String> {
        let layout = build(text, &plain(text), width).expect("布局");
        layout
            .lines()
            .iter()
            .map(|line| {
                let from = line.visual().start().get() as usize;
                let to = line.visual().end().get() as usize;
                text[from..to].to_owned()
            })
            .collect()
    }

    /// UAX #14：断在词之间，不再断在词中间。
    #[test]
    fn wraps_at_break_opportunities_not_inside_words() {
        assert_eq!(line_texts("one two", 5.0), ["one ", "two"]);
        assert_eq!(
            line_texts("re-usable words", 5.0),
            ["re-", "usabl", "e ", "words"]
        );
    }

    /// 行尾空白悬在行外：断行机会在空白**之后**，让空白把整个词挤到下一行
    /// 是错的。代价是这样的行宽度会超过 `max_width`，这是有意的。
    #[test]
    fn trailing_spaces_hang_past_the_line_width() {
        assert_eq!(line_texts("abc   def", 3.0), ["abc   ", "def"]);
        let layout = build("abc   def", &plain("abc   def"), 3.0).expect("布局");
        assert_eq!(layout.lines()[0].width(), 6.0, "三个空白悬在 3.0 宽的行外");

        // 这一条压的是「一段的空白算不算进排不排得下」——上面两条压不到，
        // 因为它们的那一段都是行首第一段，行宽为 0 时怎么算都放得下。
        // "bb " 的空白若参与判断，5 宽的行就会提前断成 ["aa ", "bb cc"]。
        assert_eq!(line_texts("aa bb cc", 5.0), ["aa bb ", "cc"]);
    }

    /// 一个比整行还宽的词仍然必须排得出来：段内退回按 grapheme 应急断行。
    /// 没有这一条，一串长 URL 会把整行撑出画面。
    #[test]
    fn an_over_wide_word_falls_back_to_grapheme_breaks() {
        assert_eq!(line_texts("abcdefgh", 3.0), ["abc", "def", "gh"]);
        assert_eq!(line_texts("don't break", 4.0), ["don'", "t ", "brea", "k"]);
    }

    /// CJK 禁则由 UAX #14 的默认规则覆盖，不另加 tailoring：
    /// 行首不出现 `、`「`」`，行末不出现 `「`。
    #[test]
    fn cjk_line_break_prohibitions_hold() {
        assert_eq!(
            line_texts("「あ」、「い」です。", 4.0),
            ["「あ」、", "「い」で", "す。"]
        );
        assert_eq!(
            line_texts("Hello, 世界！ ok", 4.0),
            ["Hell", "o, 世", "界！ ", "ok"]
        );
        // 表意文字之间可以自由断行。
        assert_eq!(
            line_texts("中文换行测试文字", 3.0),
            ["中文换", "行测试", "文字"]
        );
    }

    /// 窄到一行放不下「开括号 + 一个字」时只能应急断行，`、` 会落到行首。
    /// 记下来是因为它看起来像禁则失效，其实是宽度不够——UAX #14 允许在
    /// 没有任何合法断点时断在 grapheme 边界。
    #[test]
    fn prohibitions_yield_to_emergency_breaks_when_nothing_fits() {
        assert_eq!(
            line_texts("「あ」、「い」です。", 3.0),
            ["「あ」", "、", "「い」", "です。"]
        );
    }

    /// UAX #14 的强制换行不只有 LF。这与 `docs/specs/coordinates.md` 的
    /// 「源码逻辑行只按 LF 分隔」不冲突：那条说的是 `LineIndex`（源码行），
    /// 这里是**视觉行**，软换行早就让两者不是一回事了。
    #[test]
    fn mandatory_breaks_cover_the_whole_uax14_set() {
        for text in [
            "a\nb",
            "a\r\nb",
            "a\rb",
            "a\u{b}b",
            "a\u{c}b",
            "a\u{85}b",
            "a\u{2028}b",
            "a\u{2029}b",
        ] {
            let layout = build(text, &plain(text), 80.0).expect("布局");
            assert_eq!(layout.lines().len(), 2, "{text:?} 应当强制换成两行");
            let breaks: Vec<_> = layout
                .clusters()
                .iter()
                .filter(|cluster| cluster.is_line_break())
                .collect();
            assert_eq!(breaks.len(), 1, "{text:?} 的换行簇");
            assert_eq!(breaks[0].width(), 0.0, "{text:?} 的换行簇不占宽度");
        }
    }

    /// 断行不得把一个 grapheme 劈开。ZWJ 序列是最容易被劈的一类：
    /// 劈开之后画出来是两个不相干的 emoji，不 panic、不报错。
    #[test]
    fn breaking_never_splits_a_grapheme() {
        let text = "\u{1f469}\u{200d}\u{1f4bb}\u{1f469}\u{200d}\u{1f4bb}";
        let layout = build(text, &plain(text), 1.0).expect("布局");
        assert_eq!(layout.clusters().len(), 2);
        assert_eq!(layout.lines().len(), 2);
        for line in layout.lines() {
            let from = line.visual().start().get() as usize;
            let to = line.visual().end().get() as usize;
            assert!(
                text[from..to].chars().count() == 3,
                "每一行应当正好是一个完整的 ZWJ 序列"
            );
        }
    }

    /// grapheme 内部的视觉偏移不是合法 caret，取所在 grapheme 的左边缘，
    /// 不得落在字中间。
    #[test]
    fn offsets_inside_a_grapheme_snap_to_its_left_edge() {
        let text = "中文";
        let layout = build(text, &plain(text), 80.0).expect("布局");
        let inside = layout
            .caret(VisualOffset::new(1), CaretAffinity::Downstream)
            .expect("caret");
        assert_eq!(inside.point(), LayoutPoint::new(0.0, 0.0));
        let boundary = layout
            .caret(VisualOffset::new(3), CaretAffinity::Downstream)
            .expect("caret");
        assert_eq!(boundary.point(), LayoutPoint::new(1.0, 0.0));
    }
}
