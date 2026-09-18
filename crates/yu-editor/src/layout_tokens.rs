//! 块级盒模型的排版常量：间距表、内边距、行高倍率（Typora 标杆）。
//!
//! # 为什么这些值住在这里
//!
//! 数值参考 Typora 主题（用户认可的标杆）：正文 16–17px、行高 1.6；代码块
//! 水平内边距 14pt、垂直 10pt；引用块水平内边距 12pt。依赖方向卡死了它们的
//! 住处：`yu-editor` 不依赖 `yu-workspace`（是反过来），而布局常数必须住在
//! `yu-editor` 或更低层——`yu-workspace`（M4）会反过来引用这里导出的常量
//! 与 helper，而不是自己再抄一份数值。
//!
//! # 单位约定
//!
//! - **内边距**是绝对的 pt 值（照抄 Typora），不进 `LayoutConfig.line_height`
//!   的倍数体系。
//! - **行高倍率**相对 `LayoutConfig.line_height`（见
//!   [`yu_layout::LineAttrs::line_height_scale`]）。
//! - **块间距**以「一行正文的高度」（`LayoutConfig.line_height` × 正文行高
//!   倍率 [`LINE_HEIGHT_BODY`]）为单位记一个 `f32` 倍数，折进块高时再乘回
//!   实际行高——这样换字号/行高配置时间距自动等比缩放。

use yu_layout::{LayoutConfig, LayoutError, LayoutRect};
use yu_markdown::BlockKind;

/// Resolved code box edges in logical points at the current zoom.
#[derive(Clone, Copy, Debug)]
pub struct CodeInsets {
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
}
impl CodeInsets {
    pub fn new(config: LayoutConfig) -> Self {
        let spec = config.theme().spec();
        let zoom = config.line_height() / spec.body_size;
        Self {
            left: (spec.code_padding_left + spec.code_border_width) * zoom,
            right: (spec.code_padding_right + spec.code_border_width) * zoom,
            top: (spec.code_padding_top + spec.code_border_width) * zoom,
            bottom: (spec.code_padding_bottom + spec.code_border_width) * zoom,
        }
    }
}

/// 正文行高倍率：段落、引用块、列表项共用。
///
/// Typora 正文 16–17px 配行高 1.6。它同时也是块间距表的「一行」的定义：
/// `BLOCK_SPACING_*` 里的 `1.0` = 一行正文高 = `line_height × 1.6`。
pub const LINE_HEIGHT_BODY: f32 = yu_core::ThemeSpec::GITHUB.body_line_ratio;

/// Resolve theme line height after zoom without accumulating per-line rounding.
/// Current fingerprinted Typora references retain fractional line heights.
/// Keep exact integers stable against floating-point multiplication error.
#[must_use]
pub fn resolved_line_height(config: LayoutConfig, scale: f32) -> f32 {
    let height = config.line_height() * scale;
    // Ratios such as Night H2's 1.875 / 1.63 must resolve to 30pt,
    // not 29pt because f32 multiplication landed one ULP below an integer.
    let nearest = height.round();
    let stable = if (height - nearest).abs() <= height.abs() * f32::EPSILON * 2.0 {
        nearest
    } else {
        height
    };
    stable.max(1.0)
}

/// 代码块行高倍率。代码字号更小（等宽），行高反而略松：1.65 让长代码块不
/// 那么压迫，也是 Typora 系主题的常见取值。
pub const LINE_HEIGHT_CODE: f32 =
    yu_core::ThemeSpec::GITHUB.code_line_ratio * yu_core::ThemeSpec::GITHUB.code_block_size_ratio;

/// 标题字号倍率，按下标 0..6 对应 h1..h6（正文 = 1.0）。
///
/// h1 是正文的两倍；h2 起逐级收缩，h5/h6 已经只比正文大一点点——再往下
/// 就分不清是标题还是加粗段落了。排版上标题一律按 `Strong` 出字型
/// （`heading` extension 的语义，组装时统一替换，见 `blockinput.rs`）。
pub const HEADING_FONT_SCALE: [f32; 6] = yu_core::ThemeSpec::GITHUB.heading_sizes;

/// 标题字号倍率。`level` 必须在 1..=6，越界返回 `None`——调用方把它当
/// 配置错误处理，而不是默默落回正文。
#[must_use]
pub fn heading_font_scale(level: u8) -> Option<f32> {
    if !(1..=6).contains(&level) {
        return None;
    }
    Some(HEADING_FONT_SCALE[usize::from(level - 1)])
}

/// Per-heading line ratio from the resolved native theme.
#[must_use]
pub fn heading_line_height_scale(level: u8) -> Option<f32> {
    if !(1..=6).contains(&level) {
        return None;
    }
    Some(yu_core::ThemeSpec::GITHUB.heading_lines[usize::from(level - 1)])
}

/// 这一块是不是代码块（围栏或缩进）。两种拼法是同一种东西——盒模型的
/// 内边距、行高、底色都走同一份待遇（与 `yu-workspace` 的
/// `viewport_block_background` 同一张表）。
#[must_use]
pub const fn is_code_block(kind: BlockKind) -> bool {
    matches!(
        kind,
        BlockKind::FencedCodeBlock { .. } | BlockKind::IndentedCode
    )
}

/// Shared container advances for paragraphs and published container borders.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ContainerMetrics {
    pub list_indent: f32,
    pub quote_indent: f32,
    pub quote_border: f32,
    pub quote_margin_left: f32,
}
impl ContainerMetrics {
    pub fn new(config: LayoutConfig) -> Self {
        let theme = config.theme().spec();
        let zoom = config.line_height() / theme.body_size;
        Self {
            list_indent: theme.list_indent * zoom,
            quote_indent: (theme.quote_margin_left
                + theme.quote_padding
                + theme.quote_border_width)
                * zoom,
            quote_margin_left: theme.quote_margin_left * zoom,
            quote_border: theme.quote_border_width * zoom,
        }
    }
}

/// 块的段前/段后间距，单位是「一行正文高」（`line_height × LINE_HEIGHT_BODY`）
/// 的倍数。
///
/// 折叠规则见 [`collapsed_block_gap`]。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlockSpacing {
    /// 段前间距倍数。
    pub before: f32,
    /// 段后间距倍数。
    pub after: f32,
}

const fn spacing(before: f32, after: f32) -> BlockSpacing {
    BlockSpacing { before, after }
}

/// Resolved block margins in body-line units. CSS rem/em values are
/// interpreted once here; source separator lines never contribute margins.
#[must_use]
pub fn block_spacing(kind: BlockKind, theme: yu_core::ThemeId) -> BlockSpacing {
    let spec = theme.spec();
    let body = spec.body_line_ratio;
    match kind {
        BlockKind::Heading { level } => {
            let index = usize::from(level.clamp(1, 6) - 1);
            spacing(
                spec.heading_margin_before[index] / body,
                spec.heading_margin_after[index] / body,
            )
        }
        BlockKind::FencedCodeBlock { .. } | BlockKind::IndentedCode => spacing(
            spec.code_margin_before / spec.body_size / body,
            spec.code_margin_after / spec.body_size / body,
        ),
        BlockKind::BlankLine | BlockKind::ReferenceDefinition | BlockKind::HtmlBlock => {
            spacing(0.0, 0.0)
        }
        _ => spacing(
            spec.paragraph_margin_before / body,
            spec.paragraph_margin / body,
        ),
    }
}

/// Collapse standalone block margins. Container-aware list spacing is resolved
/// from PresentationTree by LayoutContext, never from two BlockKind tags.
#[must_use]
pub fn collapsed_block_gap(upper: BlockKind, lower: BlockKind, theme: yu_core::ThemeId) -> f32 {
    block_spacing(upper, theme)
        .after
        .max(block_spacing(lower, theme).before)
}

/// Resolve quote margin specificity and first/last-child overrides from ancestry.
/// Values are unscaled logical points, independent of paragraph line struts.
pub(crate) fn quote_margin(
    tree: &yu_markdown::PresentationTree,
    id: usize,
    theme: yu_core::ThemeId,
    after: bool,
) -> f32 {
    use yu_markdown::PresentationKind;
    let node = &tree.nodes()[id];
    if let Some(parent) = node.parent.map(|id| &tree.nodes()[id]) {
        if !after
            && matches!(
                parent.kind,
                PresentationKind::Quote | PresentationKind::ListItem
            )
            && parent.children.first() == Some(&id)
        {
            return 0.0;
        }
        if after && parent.kind == PresentationKind::Quote && parent.children.last() == Some(&id) {
            return 0.0;
        }
    }
    let mut parent = node.parent;
    while let Some(id) = parent {
        let node = &tree.nodes()[id];
        if node.kind == PresentationKind::ListItem {
            return theme.spec().body_size;
        }
        parent = node.parent;
    }
    if after {
        theme.spec().quote_margin_after
    } else {
        theme.spec().quote_margin_before
    }
}

/// Left padding is an indent; the layout width already excludes right padding.
#[must_use]
pub fn box_content_inset_x(kind: BlockKind, config: LayoutConfig) -> f32 {
    if is_code_block(kind) {
        CodeInsets::new(config)
            .left
            .min((config.max_width() - 1.0).max(0.0))
    } else {
        0.0
    }
}

/// LayoutConfig width is the right edge, not text width: left padding is
/// subtracted by the paragraph's indent. Subtracting both edges here double
/// counts the left padding and causes premature wrapping.
#[must_use]
pub fn box_layout_config(kind: BlockKind, config: LayoutConfig) -> LayoutConfig {
    let right = if is_code_block(kind) {
        CodeInsets::new(config).right
    } else if matches!(kind, BlockKind::BlockQuote { .. }) {
        let spec = config.theme().spec();
        spec.quote_padding_right * config.line_height() / spec.body_size
    } else {
        0.0
    };
    config.with_max_width((config.max_width() - right).max(1.0))
}

/// Include the enclosing quote's right inset for every kind of descendant leaf.
#[must_use]
pub fn box_layout_config_with_quote(
    kind: BlockKind,
    config: LayoutConfig,
    in_quote: bool,
) -> LayoutConfig {
    let layout = box_layout_config(kind, config);
    if in_quote && !matches!(kind, BlockKind::BlockQuote { .. }) {
        let spec = config.theme().spec();
        layout.with_max_width(
            (layout.max_width() - spec.quote_padding_right * config.line_height() / spec.body_size)
                .max(1.0),
        )
    } else {
        layout
    }
}

/// Paint the same asymmetric vertical box used by the height index and caret.
pub fn code_block_background_rect(
    column_width: f32,
    content_height: f32,
    config: LayoutConfig,
) -> Result<LayoutRect, LayoutError> {
    let edges = CodeInsets::new(config);
    Ok(LayoutRect::new(
        0.0,
        0.0,
        column_width,
        content_height + edges.top + edges.bottom,
    )?)
}

/// Quote background spans the content column without vertical padding.
///
/// # Errors
///
/// 几何参数非有限，或矩形越出 `Block` 空间的约束（宽/高须为正）。
pub fn quote_block_background_rect(
    column_width: f32,
    content_height: f32,
) -> Result<LayoutRect, LayoutError> {
    Ok(LayoutRect::new(0.0, 0.0, column_width, content_height)?)
}

/// Shared top inset for painting, queries and document-to-local transforms.
#[must_use]
pub fn content_origin_y(kind: BlockKind, config: LayoutConfig) -> f32 {
    if is_code_block(kind) {
        CodeInsets::new(config).top
    } else {
        0.0
    }
}

#[must_use]
pub fn content_bottom_inset(kind: BlockKind, config: LayoutConfig) -> f32 {
    if is_code_block(kind) {
        CodeInsets::new(config).bottom
    } else if let BlockKind::Heading { level } = kind {
        config.theme().heading_border(level).0 * config.line_height()
            / config.theme().spec().body_size
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_struts_preserve_fractional_heights_and_margins() {
        let config = LayoutConfig::new(800.0, 16.0);
        assert_eq!(resolved_line_height(config, 1.6), 25.6);
        assert_eq!(resolved_line_height(config, 1.625), 26.0);
        for (theme, expected) in [
            (
                yu_core::ThemeSpec::GITHUB,
                [43.2, 34.3, 34.32, 28.0, 22.4, 22.4],
            ),
            (
                yu_core::ThemeSpec::NIGHT,
                [44.0, 30.0, 24.0, 22.0, 20.0, 16.0],
            ),
        ] {
            for (index, expected) in expected.into_iter().enumerate() {
                let actual = resolved_line_height(
                    config,
                    theme.heading_sizes[index] * theme.heading_lines[index],
                );
                assert!((actual - expected).abs() < 0.00001);
            }
        }
        // Layout struts do not change the rem-based paragraph margin.
        let margin = block_spacing(BlockKind::Paragraph, yu_core::ThemeId::Github).after;
        assert!((margin * 1.6 * 16.0 - 12.8).abs() < 0.001);
    }

    /// Source separators and semantic paragraph margins have different roles.
    #[test]
    fn source_separators_have_no_margin_but_paragraphs_do() {
        assert_eq!(
            block_spacing(BlockKind::Paragraph, yu_core::ThemeId::Github),
            spacing(0.5, 0.5)
        );
        for kind in [
            BlockKind::BlankLine,
            BlockKind::ReferenceDefinition,
            BlockKind::HtmlBlock,
        ] {
            let spacing = block_spacing(kind, yu_core::ThemeId::Github);
            assert_eq!(
                (spacing.before, spacing.after),
                (0.0, 0.0),
                "{kind:?} 不自带段前段后间距"
            );
        }
    }

    #[test]
    fn headings_use_each_themes_declared_margins() {
        for level in 1..=6 {
            let kind = BlockKind::Heading { level };
            let github = block_spacing(kind, yu_core::ThemeId::Github);
            assert_eq!(github, spacing(1.0 / 1.6, 1.0 / 1.6));
            let night = block_spacing(kind, yu_core::ThemeId::Night);
            assert_eq!(night.before, if level == 1 { 5.0 / 1.625 } else { 0.0 });
            assert_eq!(night.after * 1.625, if level == 6 { 0.75 } else { 1.5 });
        }
    }

    #[test]
    fn code_margins_collapse_with_neighbors_instead_of_adding() {
        for (theme, before, after) in [
            (yu_core::ThemeId::Github, 15.0, 15.0),
            (yu_core::ThemeId::Night, 24.0, 20.0),
        ] {
            let scale = theme.spec().body_size * theme.spec().body_line_ratio;
            assert!(
                (collapsed_block_gap(BlockKind::Paragraph, BlockKind::IndentedCode, theme) * scale
                    - before)
                    .abs()
                    < 0.001
            );
            assert!(
                (collapsed_block_gap(BlockKind::IndentedCode, BlockKind::Paragraph, theme) * scale
                    - after)
                    .abs()
                    < 0.001
            );
        }
    }

    /// 折叠规则：相邻两块取两半的 max，与顺序无关。
    #[test]
    fn adjacent_gap_collapses_to_the_larger_half() {
        let heading = BlockKind::Heading { level: 1 };
        // after(上块) 更大。
        assert_eq!(
            collapsed_block_gap(heading, BlockKind::Paragraph, yu_core::ThemeId::Github),
            block_spacing(heading, yu_core::ThemeId::Github)
                .after
                .max(block_spacing(BlockKind::Paragraph, yu_core::ThemeId::Github).before)
        );
        // before(下块) 更大。
        let heading2 = BlockKind::Heading { level: 2 };
        assert_eq!(
            collapsed_block_gap(BlockKind::Paragraph, heading2, yu_core::ThemeId::Github),
            block_spacing(heading2, yu_core::ThemeId::Github).before
        );
        // 两个非零中间取 max 而不是平均。
        let h4 = BlockKind::Heading { level: 4 };
        let code = BlockKind::IndentedCode;
        let (after_h4, before_code) = (
            block_spacing(h4, yu_core::ThemeId::Github).after,
            block_spacing(code, yu_core::ThemeId::Github).before,
        );
        assert_eq!(
            collapsed_block_gap(h4, code, yu_core::ThemeId::Github),
            after_h4.max(before_code),
            "缝 = max(after(上), before(下))"
        );
    }

    #[test]
    fn code_background_rect_covers_the_column_with_vertical_padding() {
        let rect =
            code_block_background_rect(400.0, 16.0, LayoutConfig::new(400.0, 16.0)).expect("矩形");
        assert_eq!(rect.x(), 0.0);
        assert_eq!(rect.y(), 0.0);
        assert_eq!(rect.width(), 400.0, "背景铺满整列");
        assert_eq!(rect.height(), 16.0 + 9.0 + 7.0);
    }

    #[test]
    fn quote_background_rect_covers_the_column() {
        let rect = quote_block_background_rect(400.0, 20.0).expect("矩形");
        assert_eq!(rect.x(), 0.0);
        assert_eq!(rect.width(), 400.0);
        assert_eq!(rect.height(), 20.0, "引用背景没有垂直内边距");
    }

    #[test]
    fn content_origin_y_insets_code_content() {
        for code in [
            BlockKind::FencedCodeBlock {
                marker: '`',
                closed: true,
            },
            BlockKind::IndentedCode,
        ] {
            assert_eq!(content_origin_y(code, LayoutConfig::new(400.0, 16.0)), 9.0);
        }
        assert_eq!(
            content_origin_y(BlockKind::Paragraph, LayoutConfig::new(400.0, 16.0)),
            0.0
        );
        assert_eq!(
            content_origin_y(
                BlockKind::BlockQuote { depth: 1 },
                LayoutConfig::new(400.0, 16.0)
            ),
            0.0
        );
    }

    /// 断行宽度只按代码块/引用块收窄，其余块类原样；极窄列下内边距让位。
    #[test]
    fn box_layout_config_narrows_code_and_quote_only() {
        let config = LayoutConfig::new(400.0, 10.0);
        for code in [
            BlockKind::FencedCodeBlock {
                marker: '`',
                closed: true,
            },
            BlockKind::IndentedCode,
        ] {
            assert_eq!(
                box_layout_config(code, config).max_width(),
                400.0 - 9.0 * 10.0 / 16.0,
                "{code:?} 的断行宽度收窄两个水平内边距"
            );
        }
        assert_eq!(
            box_layout_config(BlockKind::BlockQuote { depth: 1 }, config).max_width(),
            400.0 - 15.0 * 10.0 / 16.0
        );
        assert_eq!(
            box_layout_config(BlockKind::Paragraph, config).max_width(),
            400.0,
            "段落不收窄"
        );
        // 列宽放不下两个内边距时至少留 1 个单位断行，而不是让 config 校验炸掉。
        let narrow = LayoutConfig::new(4.0, 10.0);
        assert_eq!(
            box_layout_config(BlockKind::IndentedCode, narrow).max_width(),
            1.0
        );
    }

    #[test]
    fn heading_scales_cover_six_levels_and_reject_the_rest() {
        for (level, size, line) in [
            (1, 2.25, 1.2),
            (2, 1.75, 1.225),
            (3, 1.5, 1.43),
            (4, 1.25, 1.4),
            (5, 1.0, 1.4),
            (6, 1.0, 1.4),
        ] {
            assert_eq!(heading_font_scale(level), Some(size));
            assert_eq!(heading_line_height_scale(level), Some(line));
        }
        assert_eq!(heading_font_scale(0), None);
        assert_eq!(heading_font_scale(7), None);
        assert_eq!(heading_line_height_scale(7), None);
    }
}
