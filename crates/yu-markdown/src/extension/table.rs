//! GFM 表格。
//!
//! 竖线、单元格之间的空白、以及那一整行 `--- | ---` 都不进视觉文本；剩下的
//! 是各个单元格的内容，首尾相接。**光是这样排不成一张表**——网格几何要
//! `LayoutConfig` 才算得出来，那是 `yu-editor` 的事。这里除了隐藏区间之外
//! 只多说一句话：这个块是一张表，它的行列在这些 source 区间上，由
//! [`BlockOrnament::Table`] 带上去。
//!
//! # 为什么不看 `active`
//!
//! 行内语法的定界符在光标碰到它时要露出来，表格的竖线不用：竖线不是「一段
//! 被藏起来的文字」，而是整个块换了一种排法。露出来会让表格在光标进出时
//! 变成一堆文字又变回去。v1 的 `TableProjection` 也是这么做的。
//!
//! # 为什么还不是 widget
//!
//! 第 3 节的对照表说表格终局是一个 block widget。那要求布局层能问 widget
//! 要尺寸（§5.3），而 `BlockLayout` 现在拿的是 `NoWidgets`。在那之前，
//! 表格的几何仍然由 `yu-editor::TableLayout` 按这里给出的网格算。

use yu_core::{ByteOffset, TextRange};

use super::{BlockContext, BlockOrnament, Extension, ExtensionOutput};
use crate::block_sequence::BlockKind;
use crate::table::{TableBlock, TableCellRange, parse_table_in_snapshot};

pub struct Table;

impl Extension for Table {
    fn name(&self) -> &'static str {
        "table"
    }

    fn decorate(&self, cx: &BlockContext<'_>, out: &mut ExtensionOutput) {
        // 表格在块序列里就是一个段落——`BlockKind` 没有 `Table`。语法树也
        // 认不出来（`yu-syntax` 没有 GFM 的表格 extension），所以这一种语法
        // 的结构只能来自 `parse_table_in_snapshot`。
        if cx.block().kind() != BlockKind::Paragraph {
            return;
        }
        let Some(table) = parse_table_in_snapshot(cx.source(), cx.range()) else {
            return;
        };
        for range in hidden_ranges(&table) {
            out.replace(range);
        }
        let style = out.line_style(BlockOrnament::Table(table));
        out.line(cx.range(), style);
    }
}

/// 单元格内容之外的一切：竖线、单元格周围的空白、行尾的换行符，以及整行
/// 分隔行。
///
/// 算法照搬 v1 的 `table_projection_hidden_ranges`：逐行走一遍，把游标与
/// 下一个单元格起点之间的那一段藏起来。取「单元格之间」而不是「竖线本身」，
/// 是因为对齐的空格数量随内容变，逐个列举竖线会把它们留在视觉文本里。
fn hidden_ranges(table: &TableBlock) -> Vec<TextRange> {
    let mut hidden = Vec::new();
    if let Some(row) = table.row_source_range(0) {
        append_row(row, table.header(), &mut hidden);
    }
    if let Some(delimiter) = table.delimiter_source_range()
        && let Some(range) = text_range(delimiter)
    {
        hidden.push(range);
    }
    for (index, cells) in table.rows().iter().enumerate() {
        if let Some(row) = table.row_source_range(index.saturating_add(2)) {
            append_row(row, cells, &mut hidden);
        }
    }
    hidden
}

fn append_row(row: TableCellRange, cells: &[TableCellRange], hidden: &mut Vec<TextRange>) {
    let Some(row) = text_range(row) else {
        return;
    };
    let mut cursor = row.start();
    for cell in cells {
        let Some(cell) = text_range(*cell) else {
            continue;
        };
        // 单元格必须落在它那一行里并且升序。错位时跳过这一格而不是把整行
        // 藏掉：藏掉的后果是一整行内容凭空消失，不报错。
        if cell.start() < row.start() || cell.end() > row.end() || cell.start() < cursor {
            continue;
        }
        if let Some(gap) = TextRange::new(cursor, cell.start()) {
            hidden.push(gap);
        }
        cursor = cell.end();
    }
    if let Some(tail) = TextRange::new(cursor, row.end()) {
        hidden.push(tail);
    }
}

fn text_range(range: TableCellRange) -> Option<TextRange> {
    let start = u64::try_from(range.start()).ok()?;
    let end = u64::try_from(range.end()).ok()?;
    TextRange::new(ByteOffset::new(start), ByteOffset::new(end))
}
