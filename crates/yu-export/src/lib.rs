#![forbid(unsafe_code)]

//! Source-backed clipboard payloads for Yu.
//!
//! # Markdown 是正本，HTML 是给别人看的那一份
//!
//! 三种表示都从**同一段源码区间**产出：Markdown 与纯文本就是那段字节原样，
//! HTML 由 comrak 渲染。所以这个 crate 不做投影、不复制正文、不认识排版。
//!
//! # 为什么 HTML 走 comrak 而不是自己渲染（S7 第六刀）
//!
//! 这里以前有一份自研渲染器，建在 `yu_markdown::BlockKind` 上。它**不是「编辑
//! 器画成什么样」的另一种说法，而是第三个答案**，而且是三个里最差的一个：
//! 652 条 CommonMark 规范用例里，按标签间空白归一化之后它只对 163 条
//! （`html.rs` 643，comrak 652）。逐条看过的分歧分两类——
//!
//! - **它自己错**：`[label][missing]` 渲染成 `<p>label</p>`，**把 `[missing]`
//!   整段吞掉**；链接与图片的 title 一律丢掉。
//! - **它跟着编辑器一起短**：`***`、缩进代码块、HTML 块在 `BlockKind` 里没有
//!   变体，编辑器按不变量 I5 画成普通段落源码，它也照着导出。
//!
//! **导出的契约是「这段 Markdown 是什么意思」，不是「编辑器把它画成什么样」。**
//! 剪贴板是给别的 app 的，别的 app 认的是 CommonMark。所以第二类分歧是**有意
//! 留下的**：那几处「所见 ≠ 所拷」落在编辑器自己欠的账上（`BlockKind` 粗、
//! 块边界没合并），不由导出去迁就。不变量 F1（括号配对）与 F2（制表符）同理：
//! **它们是解析的偏差，导出不随它们偏**——这条登记在 invariants 第 F 节。
//!
//! # comrak 的开关照抄 Yu 自己的语法集合，不多不少
//!
//! [`options`] 只开 `tasklist` 与 `table`，因为 Yu 只有这两样 GFM 扩展。多开
//! 一样（比如删除线）会让导出认得编辑器不认得的语法，那是反方向的分叉。
//!
//! `render.unsafe = true`：另一条路是 `<!-- raw HTML omitted -->`，**静默删掉
//! 用户自己写在文档里的内容**——那正是这个项目最危险的失败模式。导入侧仍然
//! 拒绝原始 HTML，这个不对称是有意的：导出的是用户自己的文档，导入的是别人
//! 的 HTML。

use std::error::Error;
use std::fmt;

use yu_core::{Revision, TextRange};
use yu_text::{TextPositionError, TextSnapshot};

mod html_import;

pub use html_import::{HtmlImportError, import_html_fragment};

/// Pasteboard MIME/UTI names shared by native adapters.
pub const MARKDOWN_MIME: &str = "text/markdown";
pub const MARKDOWN_UTI: &str = "net.daringfireball.markdown";
pub const PLAIN_TEXT_MIME: &str = "text/plain;charset=utf-8";
pub const PLAIN_TEXT_UTI: &str = "public.utf8-plain-text";
pub const HTML_MIME: &str = "text/html";
pub const HTML_UTI: &str = "public.html";

/// Stable source formats that every native clipboard adapter must understand.
///
/// The MIME name is used by Windows/Linux/web-facing adapters; `uti()` is the
/// corresponding macOS pasteboard identifier. The payload order is deliberate:
/// Markdown is canonical, plain text is the lossless fallback, and HTML is a
/// derived semantic fragment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ClipboardFormat {
    Markdown,
    PlainText,
    Html,
}

impl ClipboardFormat {
    /// Formats published for every canonical source selection.
    pub const ALL: [Self; 3] = [Self::Markdown, Self::PlainText, Self::Html];

    #[must_use]
    pub const fn mime(self) -> &'static str {
        match self {
            Self::Markdown => MARKDOWN_MIME,
            Self::PlainText => PLAIN_TEXT_MIME,
            Self::Html => HTML_MIME,
        }
    }

    #[must_use]
    pub const fn uti(self) -> &'static str {
        match self {
            Self::Markdown => MARKDOWN_UTI,
            Self::PlainText => PLAIN_TEXT_UTI,
            Self::Html => HTML_UTI,
        }
    }
}

/// The three payloads published for one canonical source selection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipboardPayload {
    revision: Revision,
    source_range: TextRange,
    markdown: String,
    plain_text: String,
    html: String,
}

impl ClipboardPayload {
    #[must_use]
    pub fn revision(&self) -> Revision {
        self.revision
    }

    #[must_use]
    pub const fn source_range(&self) -> TextRange {
        self.source_range
    }

    /// Canonical Markdown source. This is the payload preferred by Yu and
    /// other Markdown-aware applications.
    #[must_use]
    pub fn markdown(&self) -> &str {
        &self.markdown
    }

    /// Plain-text fallback. It intentionally remains the selected source so a
    /// plain-text-only paste does not silently discard Markdown syntax.
    #[must_use]
    pub fn plain_text(&self) -> &str {
        &self.plain_text
    }

    /// Conservative semantic HTML fragment generated from the same source.
    #[must_use]
    pub fn html(&self) -> &str {
        &self.html
    }

    /// Returns the payload value for a stable native clipboard format.
    #[must_use]
    pub fn value(&self, format: ClipboardFormat) -> &str {
        match format {
            ClipboardFormat::Markdown => self.markdown(),
            ClipboardFormat::PlainText => self.plain_text(),
            ClipboardFormat::Html => self.html(),
        }
    }
}

/// Errors raised while exporting a revision-bound source range.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExportError {
    RevisionMismatch {
        snapshot: Revision,
        expected: Revision,
    },
    SourcePosition(TextPositionError),
}

impl fmt::Display for ExportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RevisionMismatch { snapshot, expected } => write!(
                formatter,
                "clipboard snapshot revision {snapshot:?} does not match expected {expected:?}"
            ),
            Self::SourcePosition(error) => error.fmt(formatter),
        }
    }
}

impl Error for ExportError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::SourcePosition(error) => Some(error),
            Self::RevisionMismatch { .. } => None,
        }
    }
}

impl From<TextPositionError> for ExportError {
    fn from(error: TextPositionError) -> Self {
        Self::SourcePosition(error)
    }
}

/// Exports one revision-bound source selection for a native clipboard.
pub fn export_clipboard(
    snapshot: &TextSnapshot,
    expected_revision: Revision,
    source_range: TextRange,
) -> Result<ClipboardPayload, ExportError> {
    if snapshot.revision() != expected_revision {
        return Err(ExportError::RevisionMismatch {
            snapshot: snapshot.revision(),
            expected: expected_revision,
        });
    }
    let markdown = slice(snapshot, source_range)?.to_owned();
    let plain_text = markdown.clone();
    let html = export_html_fragment(&markdown);
    Ok(ClipboardPayload {
        revision: expected_revision,
        source_range,
        markdown,
        plain_text,
        html,
    })
}

/// Exports a source string as an HTML fragment.
///
/// 这就是 comrak 那一层薄薄的包装：Yu 这一侧只负责**选项**，不负责渲染。
/// 选项的取法见模块文档。
#[must_use]
pub fn export_html_fragment(source: &str) -> String {
    comrak::markdown_to_html(source, &options())
}

/// comrak 的选项。**它必须与 Yu 自己认得的语法集合一致，不多不少。**
///
/// - `tasklist`：`yu-syntax` 无条件解析 GFM 任务项（`block.rs::is_task_marker`）。
/// - `table`：`yu-markdown` 有一个 `table` extension。
/// - **不开**删除线、autolink 之类：Yu 不认得它们，开了就是导出认得而编辑器
///   不认得，与「导出别去迁就编辑器」是反方向的同一个错。
/// - `unsafe`：见模块文档，另一条路会静默删内容。
fn options() -> comrak::Options<'static> {
    let mut options = comrak::Options::default();
    options.render.r#unsafe = true;
    options.extension.tasklist = true;
    options.extension.table = true;
    options
}

fn slice(snapshot: &TextSnapshot, range: TextRange) -> Result<&str, ExportError> {
    snapshot.utf16_offset(range.start())?;
    snapshot.utf16_offset(range.end())?;
    let start = usize::try_from(range.start().get()).expect("validated source offset fits usize");
    let end = usize::try_from(range.end().get()).expect("validated source offset fits usize");
    Ok(&snapshot.as_str()[start..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    use yu_core::ByteOffset;
    use yu_text::TextBuffer;

    fn whole_range(snapshot: &TextSnapshot) -> TextRange {
        TextRange::new(ByteOffset::ZERO, snapshot.len_bytes()).expect("whole range")
    }

    /// **这里没有「导出的 HTML 逐字节等于某个字面量」这种断言，是有意的。**
    ///
    /// S7 第六刀之前有 12 条那样的用例，断的是自研渲染器的输出。渲染换成
    /// comrak 之后，把它们原地改成 comrak 的新输出就是**在断言一个第三方库的
    /// 行为**——永远绿，什么都不证明。下面留下来的每一条，判据都在 Yu 自己
    /// 这一侧：契约（Revision / UTF-8 边界 / 格式映射）、Yu 认得的语法集合、
    /// 以及自家导入器接不接受。
    ///
    /// 逐字节那件事有人管：CommonMark 的 652 条棘轮
    /// （`yu-syntax/tests/commonmark_spec.rs`）与 comrak 差分
    /// （`yu-syntax/tests/differential.rs`）。**那两条走的是 `html.rs`，
    /// 与这里换掉的渲染器不是同一条路**，所以它们没有因此变成自证。
    #[test]
    fn clipboard_payload_keeps_source_and_derives_html_from_it() {
        let source =
            "# Yu\n\n**羽** [Rust](https://example.com)\n\n- [x] done\n\n```rust\n<&>\n```\n";
        let buffer = TextBuffer::new(source);
        let snapshot = buffer.snapshot();
        let payload = export_clipboard(&snapshot, snapshot.revision(), whole_range(&snapshot))
            .expect("clipboard export");

        // 正本是源码原样，一个字节都不重新序列化（不变量 A3）。
        assert_eq!(payload.markdown(), source);
        assert_eq!(payload.plain_text(), source);
        // HTML 是从同一段源码派生的，语义标签而不是投影出来的样式。
        assert!(payload.html().contains("<h1>Yu</h1>"));
        assert!(payload.html().contains("<strong>羽</strong>"));
        assert!(payload.html().contains("href=\"https://example.com\""));
        assert!(payload.html().contains("type=\"checkbox\""));
        // 代码块里的 `<&>` 必须是转义过的文本，不是标签。
        assert!(payload.html().contains("&lt;&amp;&gt;"));
    }

    #[test]
    fn clipboard_format_contract_maps_mime_uti_and_payloads() {
        assert_eq!(
            ClipboardFormat::ALL,
            [
                ClipboardFormat::Markdown,
                ClipboardFormat::PlainText,
                ClipboardFormat::Html
            ]
        );
        assert_eq!(ClipboardFormat::Markdown.mime(), MARKDOWN_MIME);
        assert_eq!(ClipboardFormat::Markdown.uti(), MARKDOWN_UTI);
        assert_eq!(ClipboardFormat::PlainText.mime(), PLAIN_TEXT_MIME);
        assert_eq!(ClipboardFormat::PlainText.uti(), PLAIN_TEXT_UTI);
        assert_eq!(ClipboardFormat::Html.mime(), HTML_MIME);
        assert_eq!(ClipboardFormat::Html.uti(), HTML_UTI);

        let buffer = TextBuffer::new("# Yu");
        let snapshot = buffer.snapshot();
        let payload = export_clipboard(&snapshot, snapshot.revision(), whole_range(&snapshot))
            .expect("clipboard export");
        assert_eq!(payload.value(ClipboardFormat::Markdown), "# Yu");
        assert_eq!(payload.value(ClipboardFormat::PlainText), "# Yu");
        assert_eq!(
            payload.value(ClipboardFormat::Html),
            payload.html(),
            "三种格式各自取自己那一份，取错了粘出来是另一种表示，不报错"
        );
    }

    #[test]
    fn clipboard_export_is_revision_and_utf8_boundary_bound() {
        let buffer = TextBuffer::new("羽🙂");
        let snapshot = buffer.snapshot();
        let whole = whole_range(&snapshot);
        assert!(matches!(
            export_clipboard(
                &snapshot,
                Revision::INITIAL.next().expect("next revision"),
                whole
            ),
            Err(ExportError::RevisionMismatch { .. })
        ));
        let invalid = TextRange::new(ByteOffset::new(1), ByteOffset::new(2)).expect("ordered");
        assert!(matches!(
            export_clipboard(&snapshot, snapshot.revision(), invalid),
            Err(ExportError::SourcePosition(
                TextPositionError::NotUtf8Boundary(_)
            ))
        ));
    }

    /// comrak 的扩展开关必须与 Yu 自己认得的语法一一对应。
    ///
    /// **判据是 Yu 的语法集合，不是 comrak 的输出格式**——两个方向各要一半：
    /// 开着的那两样必须真的生效（少开一个，任务项与表格会退化成普通文字，
    /// 不报错），没开的那些必须**不**生效（多开一个，导出认得而编辑器不认得，
    /// 用户拷出去多一层语义）。
    #[test]
    fn export_uses_exactly_the_extensions_yu_itself_parses() {
        // 开着的两样。
        let tasks = export_html_fragment("- [x] done\n");
        assert!(
            tasks.contains("type=\"checkbox\""),
            "任务项是 Yu 无条件解析的语法，导出必须认得：{tasks}"
        );
        let table = export_html_fragment("| a | b |\n| --- | --- |\n| 1 | 2 |\n");
        assert!(table.contains("<table>"), "Yu 有 table extension：{table}");

        // 没开的那些。每一条都是 Yu 的解析器里不存在的语法。
        for (source, forbidden, syntax) in [
            ("~~删掉~~\n", "<del>", "删除线"),
            ("www.example.com\n", "<a href", "裸域名 autolink"),
            ("foo[^1]\n\n[^1]: bar\n", "footnotes", "脚注"),
        ] {
            let html = export_html_fragment(source);
            assert!(
                !html.contains(forbidden),
                "{syntax}不在 Yu 的语法集合里，导出不该认得它：{html}"
            );
        }
    }

    /// 原始 HTML 原样穿过去，**不能变成 `<!-- raw HTML omitted -->`**。
    ///
    /// comrak 的 safe 模式把它换成一条注释，那是**静默删掉用户写在自己文档里
    /// 的内容**。导入侧照旧拒绝原始 HTML（`html_import` 的标签白名单），这个
    /// 不对称是有意的，理由在模块文档里。
    #[test]
    fn raw_html_is_passed_through_instead_of_silently_dropped() {
        let html = export_html_fragment("<div>raw</div>\n");
        assert!(html.contains("<div>raw</div>"), "{html}");
        assert!(!html.contains("omitted"), "{html}");
    }
}
