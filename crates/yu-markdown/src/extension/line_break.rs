//! 硬换行。
//!
//! 换行符本身留在视觉文本里——布局按它强制换行。要拿掉的是它**前面**那
//! 一小段：两个尾随空格，或者一个反斜杠。它们是语法，不是内容。
//!
//! 软换行没有 `HardBreak` 节点，也没有东西要拿掉：行尾的空格是内容。

use yu_core::{ByteOffset, TextRange};
use yu_syntax::NodeKind;

use super::{BlockContext, Extension, ExtensionOutput};
use crate::reference::read_range;

pub struct LineBreak;

impl Extension for LineBreak {
    fn name(&self) -> &'static str {
        "line-break"
    }

    fn decorate(&self, cx: &BlockContext<'_>, out: &mut ExtensionOutput) {
        for node in cx.nodes() {
            if node.kind() != NodeKind::HardBreak {
                continue;
            }
            if let Some(prefix) = break_prefix(cx, node.range()) {
                out.replace(prefix);
            }
        }
    }
}

/// 硬换行节点里除去行尾符本身的那一段。
///
/// 行尾符可能是 `\r\n`，所以要看最后两个字节，不能一律按一个算——按一个算
/// 的话 `\r` 会留在视觉文本里，排出来是一个看不见但占位的字符。
fn break_prefix(cx: &BlockContext<'_>, range: TextRange) -> Option<TextRange> {
    let end = range.end().get();
    let tail_start = end
        .checked_sub(2)
        .filter(|candidate| *candidate >= range.start().get())
        .or_else(|| end.checked_sub(1))?;
    let tail = TextRange::new(ByteOffset::new(tail_start), range.end())?;
    let ending_len = if read_range(cx.source(), tail)?.as_slice() == b"\r\n" {
        2
    } else {
        1
    };
    let ending_start = ByteOffset::new(end.checked_sub(ending_len)?);
    if ending_start < range.start() {
        return None;
    }
    TextRange::new(range.start(), ending_start).filter(|prefix| !prefix.is_empty())
}
