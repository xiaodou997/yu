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

/// 代码块（围栏与缩进共用）水平内边距，单位 pt。
///
/// 用途有两处，必须同源：断行宽度收窄为「列宽 − 2×本值」（
/// [`box_layout_config`]），背景矩形从内容盒每侧外扩本值（
/// [`code_block_background_rect`]）。M4 的背景填充按后者画。
pub const CODE_BLOCK_PADDING_X: f32 = 14.0;

/// 代码块（围栏与缩进共用）垂直内边距，**每侧** 5pt、上下共 10pt（照抄
/// Typora 的上下对称）。这 10pt 不计入代码文字的行盒，而是一次性折进块高
/// 贡献（见 `EditorDocument::block_box_height`）；背景矩形因此比内容高 10pt，
/// 文字在背景里垂直居中。
pub const CODE_BLOCK_PADDING_Y: f32 = 5.0;

/// 引用块水平内边距，单位 pt（照抄 Typora）。待遇与代码块相同：收窄断行宽度、
/// 内容右移、背景矩形每侧外扩（[`quote_block_background_rect`]）。
pub const QUOTE_BLOCK_PADDING_X: f32 = 12.0;

/// 正文行高倍率：段落、引用块、列表项共用。
///
/// Typora 正文 16–17px 配行高 1.6。它同时也是块间距表的「一行」的定义：
/// `BLOCK_SPACING_*` 里的 `1.0` = 一行正文高 = `line_height × 1.6`。
pub const LINE_HEIGHT_BODY: f32 = 1.6;

/// 代码块行高倍率。代码字号更小（等宽），行高反而略松：1.65 让长代码块不
/// 那么压迫，也是 Typora 系主题的常见取值。
pub const LINE_HEIGHT_CODE: f32 = 1.65;

/// 一、二级标题的行高倍率。大字号标题的行盒按正文字号推会过高，1.3 压到
/// 接近「字号 + 一点喘息」，h1/h2 共用。
pub const LINE_HEIGHT_HEADING_1_2: f32 = 1.3;

/// 三至六级标题的行高倍率。字号更接近正文，行盒可以略紧，1.25 统一四档
/// （h3–h6 的字号差由 [`heading_font_scale`] 体现，行高不再分档）。
pub const LINE_HEIGHT_HEADING_3_6: f32 = 1.25;

/// 标题字号倍率，按下标 0..6 对应 h1..h6（正文 = 1.0）。
///
/// h1 是正文的两倍；h2 起逐级收缩，h5/h6 已经只比正文大一点点——再往下
/// 就分不清是标题还是加粗段落了。排版上标题一律按 `Strong` 出字型
/// （`heading` extension 的语义，组装时统一替换，见 `blockinput.rs`）。
pub const HEADING_FONT_SCALE: [f32; 6] = [2.0, 1.6, 1.35, 1.2, 1.05, 1.05];

/// h1/h2 的 [`HEADING_FONT_SCALE`] 下标（含）。
const HEADING_LARGE_MAX_LEVEL: u8 = 2;

/// 标题字号倍率。`level` 必须在 1..=6，越界返回 `None`——调用方把它当
/// 配置错误处理，而不是默默落回正文。
#[must_use]
pub fn heading_font_scale(level: u8) -> Option<f32> {
    if !(1..=6).contains(&level) {
        return None;
    }
    Some(HEADING_FONT_SCALE[usize::from(level - 1)])
}

/// 标题行高倍率：h1/h2 用 [`LINE_HEIGHT_HEADING_1_2`]，h3–h6 用
/// [`LINE_HEIGHT_HEADING_3_6`]。`level` 越界返回 `None`。
#[must_use]
pub fn heading_line_height_scale(level: u8) -> Option<f32> {
    if !(1..=6).contains(&level) {
        return None;
    }
    Some(if level <= HEADING_LARGE_MAX_LEVEL {
        LINE_HEIGHT_HEADING_1_2
    } else {
        LINE_HEIGHT_HEADING_3_6
    })
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

/// 这一块是不是列表项（含任务项）。间距折叠用它识别「列表内连续 item」。
#[must_use]
pub const fn is_list_item(kind: BlockKind) -> bool {
    matches!(
        kind,
        BlockKind::ListItem { .. } | BlockKind::TaskListItem { .. }
    )
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

/// 按块类取段前/段后间距（正文行高的倍数）。
///
/// 取值的锚是 Typora 系主题在 16px 正文、行高 1.6（一行 ≈ 25.6px）下的
/// em 间距，换算成「行」的倍数：
///
/// | 块类                | 段前/段后（行） | ≈ px（@25.6px/行） | 对应 Typora 的 em 值  |
/// |--------------------|----------------|--------------------|-----------------------|
/// | h1                 | 0.90 / 0.45    | 23 / 12            | 1.4em / 0.75em        |
/// | h2                 | 0.75 / 0.40    | 19 / 10            | 1.2em / 0.6em         |
/// | h3                 | 0.65 / 0.35    | 17 / 9             | 1.0em / 0.55em        |
/// | h4                 | 0.55 / 0.30    | 14 / 8             | 0.9em / 0.5em         |
/// | h5                 | 0.50 / 0.25    | 13 / 6             | 0.8em / 0.4em          |
/// | h6                 | 0.45 / 0.20    | 12 / 5             | 0.7em / 0.3em         |
/// | 代码块（两种拼法）  | 0.60 / 0.60    | 15 / 15            | 约 1em                |
/// | 引用块             | 0.60 / 0.60    | 15 / 15            | 约 1em                |
/// | 列表项（列表边界用）| 0.40 / 0.40    | 10 / 10            | 约 0.65em             |
/// | 分隔线             | 0.80 / 0.80    | 20 / 20            | 约 1.25em             |
///
/// **Paragraph 是 0/0，这是有意设计**：源码空行本身是一块、有高（一行），
/// 已经提供了段落分隔；再给 Paragraph 加 margin 就是双倍间距。结构性块
/// （标题/代码/引用/分隔线/列表）的间距照常在空行之上叠加——空行给「一行」，
/// margin 给「结构感」，两者不冲突。
#[must_use]
pub const fn block_spacing(kind: BlockKind) -> BlockSpacing {
    match kind {
        BlockKind::Heading { level } => match level {
            1 => spacing(0.9, 0.45),
            2 => spacing(0.75, 0.4),
            3 => spacing(0.65, 0.35),
            4 => spacing(0.55, 0.3),
            5 => spacing(0.5, 0.25),
            // h6 与防御性兜底（level 已由解析器约束在 1..=6）。
            _ => spacing(0.45, 0.2),
        },
        BlockKind::FencedCodeBlock { .. } | BlockKind::IndentedCode => spacing(0.6, 0.6),
        BlockKind::BlockQuote { .. } => spacing(0.6, 0.6),
        BlockKind::ListItem { .. } | BlockKind::TaskListItem { .. } => spacing(0.4, 0.4),
        BlockKind::ThematicBreak => spacing(0.8, 0.8),
        BlockKind::Paragraph
        | BlockKind::BlankLine
        | BlockKind::ReferenceDefinition
        | BlockKind::HtmlBlock => spacing(0.0, 0.0),
    }
}

/// 相邻两块折叠后的间距（正文行高的倍数）：`max(after(上块), before(下块))`。
///
/// 唯一的特例：**列表内连续 item 之间间距为 0**——item 与 item 直接相邻
/// 时不产生间距（紧凑列表的 item 一行接一行）；item 的段前/段后只在列表
/// 边界上起作用（与相邻的非 item 块之间）。
///
/// 返回值折进块高时的约定见 `EditorDocument::block_box_height`：整条缝折在
/// **上块**的高度贡献里，高度索引的数学自动一致（光标/滚动/AX 全部消费
/// 高度，不需要任何调用方知道缝的存在）。
#[must_use]
pub fn collapsed_block_gap(upper: BlockKind, lower: BlockKind) -> f32 {
    if is_list_item(upper) && is_list_item(lower) {
        return 0.0;
    }
    block_spacing(upper).after.max(block_spacing(lower).before)
}

/// 这一块内容的水平内边距（pt）：代码块与引用块的内容让出内边距排布，
/// 背景矩形从内容盒外扩同一数值，视觉上就是「文字离背景边一个内边距」。
#[must_use]
pub const fn box_content_inset_x(kind: BlockKind) -> f32 {
    if is_code_block(kind) {
        CODE_BLOCK_PADDING_X
    } else if matches!(kind, BlockKind::BlockQuote { .. }) {
        QUOTE_BLOCK_PADDING_X
    } else {
        0.0
    }
}

/// 按块类收窄断行宽度：列宽两侧各让出一个水平内边距，内容从收窄后的
/// 左边缘排起（`blockinput.rs` 把 [`box_content_inset_x`] 加进缩进）。
/// 非代码/引用块原样返回。
///
/// 极窄的列（测试夹具、缩到极限的视口）放不下两个内边距：这时内边距让位，
/// 断行宽度至少留 1 个单位——`LayoutConfig` 要求宽度有限且为正，让到零或
/// 负数会把一个排版决定变成校验错误。
#[must_use]
pub fn box_layout_config(kind: BlockKind, config: LayoutConfig) -> LayoutConfig {
    let inset = box_content_inset_x(kind);
    if inset == 0.0 {
        return config;
    }
    config.with_max_width((config.max_width() - 2.0 * inset).max(1.0))
}

/// 代码块背景矩形（块内容坐标系，即与 `BlockLine::bounds` 同一空间）。
///
/// `column_width` 是**布局列宽**（`LayoutConfig::max_width`，未收窄的那一份），
/// `content_height` 是代码内容高（行盒累加，`BlockView::height`）。返回矩形
/// 铺满整列、上下各扩出 [`CODE_BLOCK_PADDING_Y`]：
///
/// - 水平：断行宽度收窄为「列宽 − 2×内边距」、内容右移一个内边距（见
///   [`box_layout_config`]），于是文字离背景左右边正好各 14pt——
///   「内容宽 + 2×水平内边距 = 列宽」是同一件事的两个算法。
/// - 垂直：块内容在这个矩形里从 [`content_origin_y`]（上内边距）起排，
///   上下对称。块贡献给视口的高度里还折着段间距（见
///   `EditorDocument::block_box_height`），那一段**不属于背景**——调用方
///   （M4）按 [`content_origin_y`] 平移内容、按本矩形画 `RoundedFillRect`。
///
/// # Errors
///
/// 几何参数非有限，或矩形越出 `Block` 空间的约束（宽/高须为正）。
pub fn code_block_background_rect(
    column_width: f32,
    content_height: f32,
) -> Result<LayoutRect, LayoutError> {
    Ok(LayoutRect::new(
        0.0,
        0.0,
        column_width,
        content_height + 2.0 * CODE_BLOCK_PADDING_Y,
    )?)
}

/// 引用块背景矩形（块内容坐标系），规则与 [`code_block_background_rect`] 的水
/// 平部分相同：铺满整列（内容因 [`QUOTE_BLOCK_PADDING_X`] 收窄 + 右移，离背
/// 景边各 12pt），高度就是内容高——引用块没有垂直内边距。
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

/// 块内容在**块盒**里的纵向起点：代码块从上内边距起排（上下对称的另一半在
/// 块高贡献里），其余块从 0 起排。
///
/// 消费块几何的两方要拿同一个数：把内容坐标（glyph/caret/选中/hit 的局部
/// y）换算成文档坐标时加上它（`EditorDocument::block_box_height` 的反向），
/// 画背景时按 [`code_block_background_rect`] 从 0 起画。漏掉它的表现是代码
/// 块里光标/选中整体上移 5pt，不报错。
#[must_use]
pub const fn content_origin_y(kind: BlockKind) -> f32 {
    if is_code_block(kind) {
        CODE_BLOCK_PADDING_Y
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Paragraph 的 0 间距是**有意设计**：源码空行本身是一块、有高，已经提供
    /// 段落分隔，再加 margin 就是双倍间距。这个 0 被拿掉（或手滑改成非 0）
    /// 时没有任何一条别的用例会红——常数没有断言就等于没有约定。
    #[test]
    fn paragraph_has_no_margins_by_design() {
        for kind in [
            BlockKind::Paragraph,
            BlockKind::BlankLine,
            BlockKind::ReferenceDefinition,
            BlockKind::HtmlBlock,
        ] {
            let spacing = block_spacing(kind);
            assert_eq!(
                (spacing.before, spacing.after),
                (0.0, 0.0),
                "{kind:?} 不自带段前段后间距"
            );
        }
    }

    /// 标题的段前段后随级别递减：h1 大、h6 小。
    #[test]
    fn heading_spacing_shrinks_with_level() {
        for level in 2..=6_u8 {
            let upper = block_spacing(BlockKind::Heading { level: level - 1 });
            let lower = block_spacing(BlockKind::Heading { level });
            assert!(
                upper.before > lower.before && upper.after > lower.after,
                "h{} 的间距要比 h{} 大",
                level - 1,
                level
            );
        }
    }

    /// 折叠规则：相邻两块取两半的 max，与顺序无关。
    #[test]
    fn adjacent_gap_collapses_to_the_larger_half() {
        let heading = BlockKind::Heading { level: 1 };
        // after(上块) 更大。
        assert_eq!(
            collapsed_block_gap(heading, BlockKind::Paragraph),
            block_spacing(heading).after
        );
        // before(下块) 更大。
        let heading2 = BlockKind::Heading { level: 2 };
        assert_eq!(
            collapsed_block_gap(BlockKind::Paragraph, heading2),
            block_spacing(heading2).before
        );
        // 两个非零中间取 max 而不是平均。
        let h4 = BlockKind::Heading { level: 4 };
        let code = BlockKind::IndentedCode;
        let (after_h4, before_code) = (block_spacing(h4).after, block_spacing(code).before);
        assert_eq!(
            collapsed_block_gap(h4, code),
            after_h4.max(before_code),
            "缝 = max(after(上), before(下))"
        );
    }

    /// 列表内连续 item 之间间距为 0，只有列表边界算间距。
    #[test]
    fn consecutive_list_items_have_no_gap() {
        let item = BlockKind::ListItem {
            ordered: false,
            depth: 0,
            marker: '-',
            start: 0,
        };
        let task = BlockKind::TaskListItem {
            ordered: false,
            depth: 0,
            marker: '-',
            start: 1,
            state: yu_markdown::TaskState::Todo,
        };
        assert_eq!(
            collapsed_block_gap(item, item),
            0.0,
            "item 挨着 item 没有缝"
        );
        assert_eq!(
            collapsed_block_gap(item, task),
            0.0,
            "普通项挨着任务项也没有缝"
        );
        // 列表边界：与非 item 块之间按表取间距。
        assert_eq!(
            collapsed_block_gap(item, BlockKind::Paragraph),
            block_spacing(item).after
        );
        assert_eq!(
            collapsed_block_gap(BlockKind::Paragraph, item),
            block_spacing(item).before
        );
    }

    /// 代码块背景铺满整列：断行宽度收窄 + 内容右移让文字离背景左右边各 14pt，
    /// 上下各扩 5pt。M4 的背景填充按这三个数画，写死它们是有意的。
    #[test]
    fn code_background_rect_covers_the_column_with_vertical_padding() {
        let rect = code_block_background_rect(400.0, 16.0).expect("矩形");
        assert_eq!(rect.x(), 0.0);
        assert_eq!(rect.y(), 0.0);
        assert_eq!(rect.width(), 400.0, "背景铺满整列");
        assert_eq!(rect.height(), 16.0 + 2.0 * CODE_BLOCK_PADDING_Y);
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
            assert_eq!(content_origin_y(code), CODE_BLOCK_PADDING_Y);
        }
        assert_eq!(content_origin_y(BlockKind::Paragraph), 0.0);
        assert_eq!(content_origin_y(BlockKind::BlockQuote { depth: 1 }), 0.0);
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
                400.0 - 2.0 * CODE_BLOCK_PADDING_X,
                "{code:?} 的断行宽度收窄两个水平内边距"
            );
        }
        assert_eq!(
            box_layout_config(BlockKind::BlockQuote { depth: 1 }, config).max_width(),
            400.0 - 2.0 * QUOTE_BLOCK_PADDING_X
        );
        assert_eq!(
            box_layout_config(BlockKind::Paragraph, config).max_width(),
            400.0,
            "段落不收窄"
        );
        // 列宽放不下两个内边距时至少留 1 个单位断行，而不是让 config 校验炸掉。
        let narrow = LayoutConfig::new(12.0, 10.0);
        assert_eq!(
            box_layout_config(BlockKind::IndentedCode, narrow).max_width(),
            1.0
        );
    }

    #[test]
    fn heading_scales_cover_six_levels_and_reject_the_rest() {
        assert_eq!(heading_font_scale(1), Some(2.0));
        assert_eq!(heading_font_scale(2), Some(1.6));
        assert_eq!(heading_font_scale(3), Some(1.35));
        assert_eq!(heading_font_scale(4), Some(1.2));
        assert_eq!(heading_font_scale(5), Some(1.05));
        assert_eq!(heading_font_scale(6), Some(1.05));
        assert_eq!(heading_font_scale(0), None);
        assert_eq!(heading_font_scale(7), None);
        assert_eq!(heading_line_height_scale(1), Some(LINE_HEIGHT_HEADING_1_2));
        assert_eq!(heading_line_height_scale(2), Some(LINE_HEIGHT_HEADING_1_2));
        for level in 3..=6 {
            assert_eq!(
                heading_line_height_scale(level),
                Some(LINE_HEIGHT_HEADING_3_6)
            );
        }
        assert_eq!(heading_line_height_scale(7), None);
    }
}
