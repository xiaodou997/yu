//! extension 集合自己的用例。
//!
//! S6 前半段有一条差分（`yu-projection/tests/extension_parity.rs`）拿 v1 当
//! oracle 压住「非焦点时隐藏了哪些字节」，v1 删掉之后它没了，语料搬到了本
//! 文件末尾。这里压的是那条差分压不到的三件事，加上语料的结构性性质：
//!
//! 1. **焦点态。** 光标碰到语法就露出来。v1 的粒度与这里不同，比对不了。
//! 2. **id 表。** 装饰指向的 `StyleId` / `LineStyleId` 必须查得到——查不到
//!    不给默认值，直接 `None`。
//! 3. **验收本身。** S6 的验收是「新增一种语法的 diff 只落在 `yu-markdown`
//!    内，且 < 200 行」。这里真的加一种，看它需不需要动别的地方。

use yu_core::{ByteOffset, StyleId, TextAttrs, TextRange, TextStyle, WidgetId, WidgetSide};
use yu_decoration::LineStyleId;
use yu_markdown::{
    BlockContext, BlockDecorations, BlockOrnament, BlockWidget, DelimitedSpan, Extension,
    ExtensionOutput, ExtensionSet, ImageSpan, parse,
};
use yu_syntax::{NodeKind, parse as parse_syntax};
use yu_text::TextBuffer;

use std::sync::{Arc, Mutex};

fn offset(value: u64) -> ByteOffset {
    ByteOffset::new(value)
}

fn range(from: u64, to: u64) -> TextRange {
    TextRange::new(offset(from), offset(to)).expect("测试区间是升序的")
}

/// 跑一遍注册表，取第一个块的装饰。
fn decorate_with(
    extensions: &ExtensionSet,
    source: &str,
    active: Option<TextRange>,
) -> BlockDecorations {
    let buffer = TextBuffer::new(source.to_owned());
    let snapshot = buffer.snapshot();
    let document = parse(&snapshot);
    let tree = parse_syntax(&snapshot).expect("测试文档很短").into_tree();
    let block = document.blocks().get(0).expect("至少有一个块");
    extensions
        .decorate(&snapshot, &tree, block, active)
        .expect("装饰产出不该失败")
}

fn decorate(source: &str, active: Option<TextRange>) -> BlockDecorations {
    decorate_with(&ExtensionSet::markdown(), source, active)
}

/// 这个块上被隐藏的 source 区间，升序去重。
fn hidden(decorations: &BlockDecorations) -> Vec<(u64, u64)> {
    let mut ranges: Vec<_> = decorations
        .set()
        .all()
        .iter()
        .filter(|entry| entry.decoration.hides_source())
        .map(|entry| (entry.range.start().get(), entry.range.end().get()))
        .collect();
    ranges.sort_unstable();
    ranges.dedup();
    ranges
}

/// 同上，但把重叠与相邻的区间并起来。
///
/// 两个互不感知的 extension 可以盖在同一段 source 上（`- [x]` 就是），
/// 问「最后看不见哪些字节」时要看并集，不是看谁产了哪一条。
fn hidden_merged(decorations: &BlockDecorations) -> Vec<(u64, u64)> {
    let mut out: Vec<(u64, u64)> = Vec::new();
    for (from, to) in hidden(decorations) {
        match out.last_mut() {
            Some(last) if from <= last.1 => last.1 = last.1.max(to),
            _ => out.push((from, to)),
        }
    }
    out
}

// ---------------------------------------------------------------- 焦点态

#[test]
fn cursor_inside_emphasis_reveals_its_delimiters() {
    // `*斜体*`：光标停在内容里，两个 `*` 都要看得见。
    assert_eq!(hidden(&decorate("*斜体*", None)), vec![(0, 1), (7, 8)]);
    assert_eq!(hidden(&decorate("*斜体*", Some(range(3, 3)))), Vec::new());
}

/// 光标停在外边缘不算碰到它。
///
/// 用非严格包含的话，两段**相邻**语法之间那一处会让它们一起露出来——用户
/// 只把光标挪到两段之间，屏幕上凭空多出四个字符。
///
/// 语料用 `` `a`*b* `` 而不是 `*a**b*`：后者在 CommonMark 里是**一整段**
/// 强调（内容是 `a**b`），压不住「两段」这件事。
#[test]
fn a_cursor_on_the_outer_edge_does_not_reveal() {
    let both_hidden = vec![(0, 1), (2, 3), (3, 4), (5, 6)];
    assert_eq!(hidden(&decorate("`a`*b*", None)), both_hidden);
    assert_eq!(hidden(&decorate("`a`*b*", Some(range(3, 3)))), both_hidden);
    // 挪进第一段内部，只有第一段露出来。
    assert_eq!(
        hidden(&decorate("`a`*b*", Some(range(1, 1)))),
        vec![(3, 4), (5, 6)]
    );
}

/// 选区按相交算，不按严格包含。
#[test]
fn a_selection_reveals_every_span_it_touches() {
    assert_eq!(hidden(&decorate("`a`*b*", Some(range(0, 6)))), Vec::new());
}

/// 结构性前缀整块一起露出来：光标在这一行时用户要能看见 `##`。
#[test]
fn a_focused_block_reveals_its_structural_prefix() {
    assert_eq!(hidden(&decorate("## 标题", None)), vec![(0, 3)]);
    assert_eq!(hidden(&decorate("## 标题", Some(range(4, 4)))), Vec::new());

    assert_eq!(hidden(&decorate("> 引用", None)), vec![(0, 2)]);
    assert_eq!(hidden(&decorate("> 引用", Some(range(3, 3)))), Vec::new());

    // 列表的标记连替代呈现一起撤掉——否则光标停在一个看不见的 `-` 上，
    // 用户按退格会删掉一个他没看见的字符。
    assert_eq!(hidden(&decorate("- 项目", None)), vec![(0, 2)]);
    let focused = decorate("- 项目", Some(range(3, 3)));
    assert_eq!(hidden(&focused), Vec::new());
    assert!(
        focused.line_ornaments().is_empty(),
        "焦点列表项不该再画替代标记"
    );
}

// ---------------------------------------------------------------- 装饰内容

#[test]
fn heading_reports_its_level_not_its_font_size() {
    let decorations = decorate("### 三级", None);
    let ornaments = decorations.line_ornaments();
    assert_eq!(ornaments.len(), 1);
    assert_eq!(ornaments[0].1, &BlockOrnament::Heading { level: 3 });
}

/// 续行的 `>` 嵌在 `Paragraph` 里，不是 `Blockquote` 的直接子节点。按标记
/// 个数数层会把 `> a\n> b` 报成两层。
#[test]
fn quote_depth_counts_nesting_not_marks() {
    let one = decorate("> a\n> b", None);
    assert_eq!(
        one.line_ornaments()[0].1,
        &BlockOrnament::QuoteBar { depth: 1 }
    );
    assert_eq!(hidden(&one), vec![(0, 2), (4, 6)], "两行的前缀都要隐藏");

    let two = decorate("> > 两层", None);
    assert_eq!(
        two.line_ornaments()[0].1,
        &BlockOrnament::QuoteBar { depth: 2 }
    );
}

#[test]
fn ordered_lists_keep_their_number_bullets_get_a_dot() {
    let bullet = decorate("- 项目", None);
    let BlockOrnament::Marker(marker) = bullet.line_ornaments()[0].1 else {
        panic!("列表项该有一个标记装饰");
    };
    assert_eq!(marker.text(), "\u{2022}");
    assert_eq!(marker.indent(), 0);

    let ordered = decorate("1. 有序", None);
    let BlockOrnament::Marker(marker) = ordered.line_ornaments()[0].1 else {
        panic!("列表项该有一个标记装饰");
    };
    assert_eq!(marker.text(), "1.", "有序列表的编号是给人看的，原样搬过去");

    let indented = decorate("  - 缩进项", None);
    let BlockOrnament::Marker(marker) = indented.line_ornaments()[0].1 else {
        panic!("列表项该有一个标记装饰");
    };
    assert_eq!(marker.indent(), 2);
    assert_eq!(
        hidden(&indented),
        vec![(0, 4)],
        "行首缩进也是语法：缩进量单独报给上一层，留在视觉文本里就缩进两次"
    );
}

/// 代码块的内容不解析行内语法。这件事由树的形状保证，不靠任何人记得判断。
#[test]
fn code_content_keeps_its_asterisks() {
    let fenced = decorate("```\nlet x = *y;\n```", None);
    assert_eq!(
        hidden(&fenced),
        vec![(0, 4), (16, 19)],
        "只有两条围栏该消失，代码里的 `*` 一个都不能动"
    );

    // 空代码块没有 `CodeText`，开围栏后面那个换行符仍然要拿掉——留着它
    // 画面上就是一个空行。
    assert_eq!(hidden(&decorate("```\n```", None)), vec![(0, 4), (4, 7)]);
}

/// 硬换行拿掉的是换行符**前面**那一小段，换行符本身留着——布局按它强制换行。
#[test]
fn hard_breaks_hide_the_marker_not_the_newline() {
    assert_eq!(hidden(&decorate("行尾  \n第二行", None)), vec![(6, 8)]);
    assert_eq!(hidden(&decorate("行尾\\\n第二行", None)), vec![(6, 7)]);
    // 软换行没有东西要拿掉：行尾的空格是内容。
    assert_eq!(hidden(&decorate("行尾\n第二行", None)), Vec::new());
}

/// 任务项画成 `- ☐ 待办`，普通列表项画成 `• 项目`。
///
/// `- ` 前缀归 `list.rs` 管，而它按**块类型**只认 `ListItem`，认不到
/// `TaskListItem`——两个 extension 的定义域不相交，谁也不需要知道对方存在
/// （不变量 D6）。让 list 去问「有没有 task」才是相互感知。
#[test]
fn a_task_item_keeps_its_dash_a_plain_item_gets_a_bullet() {
    let task = decorate("- [ ] 待办", None);
    assert_eq!(
        hidden_merged(&task),
        vec![(2, 5)],
        "只有 `[ ]` 消失，`- ` 原样留着"
    );
    assert!(
        task.line_ornaments().is_empty(),
        "任务项不该有替代标记——有的话 `- ` 旁边会再多一个 `•`"
    );

    let plain = decorate("- 项目", None);
    assert_eq!(hidden_merged(&plain), vec![(0, 2)], "`- ` 换成替代标记");
    let BlockOrnament::Marker(marker) = plain.line_ornaments()[0].1 else {
        panic!("普通列表项该有一个标记装饰");
    };
    assert_eq!(marker.text(), "\u{2022}");
}

/// `[x]` 在树里还会被解析成一个 shortcut `Link`，于是 link 与 task 两个
/// extension 都往同一段 source 上盖隐藏。它们互不感知（不变量 D6），
/// 结果靠取并集收敛——这条用例钉住「并集恰好是整个 `[x]`」，不多不少。
#[test]
fn task_and_link_overlap_on_a_checked_box_without_fighting() {
    assert_eq!(hidden_merged(&decorate("- [x] 完成", None)), vec![(2, 5)]);
    assert_eq!(hidden_merged(&decorate("- [ ] 待办", None)), vec![(2, 5)]);
    // link 确实也插了一手：`[x]` 的两个 `LinkMark` 各自成一条装饰。
    assert_eq!(
        hidden(&decorate("- [x] 完成", None)),
        vec![(2, 3), (2, 5), (4, 5)]
    );
}

/// 复选框永远不露出来，焦点块也不例外。
///
/// 让它在光标经过时闪出一个 `[ ]`，用户会以为凭空多了两个字符。
#[test]
fn a_focused_task_still_hides_its_checkbox() {
    assert_eq!(
        hidden_merged(&decorate("- [ ] 待办", Some(range(8, 8)))),
        vec![(2, 5)]
    );
}

// ---------------------------------------------------------------- id 表

/// 查不到的 id 返回 `None`，不给默认值。
///
/// 「装饰产出与样式表脱节」的 bug 应该响。给个默认字型的话，它只会画得不对
/// ——而画得不对是这个项目最难发现的一类失败。
#[test]
fn unknown_ids_resolve_to_none_instead_of_a_default() {
    let decorations = decorate("*斜体*", None);
    assert_eq!(
        decorations.attrs(StyleId(0)),
        Some(TextAttrs::new(TextStyle::Emphasis))
    );
    assert_eq!(decorations.attrs(StyleId(9)), None);
    assert_eq!(decorations.ornament(LineStyleId(9)), None);
}

/// 每一条装饰指向的 id 都必须查得到。产出与表脱节是静默的。
#[test]
fn every_emitted_id_resolves() {
    for source in [
        "# 标题",
        "## 二级 *斜体*",
        "> 引用",
        "- 项目",
        "1. 有序",
        "- [ ] 待办",
        "```rust\nlet x = 1;\n```",
        "*a* **b** `c` [d](e) ![f](g) <http://h.i>",
    ] {
        let decorations = decorate(source, None);
        for entry in decorations.set().all() {
            match entry.decoration {
                yu_decoration::Decoration::Mark { style } => assert!(
                    decorations.attrs(style).is_some(),
                    "{source:?} 的 {style:?} 查不到字型"
                ),
                yu_decoration::Decoration::Line { style } => assert!(
                    decorations.ornament(style).is_some(),
                    "{source:?} 的 {style:?} 查不到行级装饰"
                ),
                _ => {}
            }
        }
    }
}

/// 装饰不得越出块。装饰是按块缓存的，越界会变成「改了这一块，另一块的样子
/// 也变了」——而按块缓存正好会把这件事藏起来。
#[test]
fn decorations_stay_inside_their_block() {
    let source = "# 标题\n\n> 引用\n\n- 项目\n\n*斜体*\n";
    let buffer = TextBuffer::new(source.to_owned());
    let snapshot = buffer.snapshot();
    let document = parse(&snapshot);
    let tree = parse_syntax(&snapshot).expect("测试文档很短").into_tree();
    let extensions = ExtensionSet::markdown();

    for block in document.blocks().iter() {
        let decorations = extensions
            .decorate(&snapshot, &tree, block, None)
            .expect("装饰产出不该失败");
        for entry in decorations.set().all() {
            assert!(
                entry.range.start() >= block.range().start()
                    && entry.range.end() <= block.range().end(),
                "{:?} 的装饰 {:?} 越出了块 {:?}",
                block.kind(),
                entry.range,
                block.range()
            );
        }
    }
}

// ------------------------------------------------------- S6 的验收本身

/// `==高亮==`：验收里点名的那种新语法。
///
/// 它只认识自己，只产出自己的装饰，拿不到别的 extension 的产出——所以整个
/// diff 就是这一个类型加注册表里一行。**这个文件之外一行都不用改。**
///
/// lezer 不认识 `==`，树里只有一个 `Paragraph`。所以它自己扫段落文本，
/// 这正是「新增一种语法」最不利的形状：连解析器都帮不上忙，也仍然不需要
/// 动 `yu-markdown` 之外的任何东西。
struct Highlight;

impl Extension for Highlight {
    fn name(&self) -> &'static str {
        "highlight"
    }

    fn decorate(&self, cx: &BlockContext<'_>, out: &mut ExtensionOutput) {
        for node in cx.nodes().filter(|node| node.kind() == NodeKind::Paragraph) {
            let Some(text) = cx.text(node.range()) else {
                continue;
            };
            let base = node.range().start().get();
            let mut marks = text.match_indices("==").map(|(index, _)| index as u64);
            while let (Some(open), Some(close)) = (marks.next(), marks.next()) {
                let style = out.style(TextAttrs::new(TextStyle::Code));
                if let Some(content) = TextRange::new(
                    ByteOffset::new(base + open + 2),
                    ByteOffset::new(base + close),
                ) {
                    out.mark(content, style);
                }
                if let Some(opening) = TextRange::new(
                    ByteOffset::new(base + open),
                    ByteOffset::new(base + open + 2),
                ) {
                    out.replace(opening);
                }
                if let Some(closing) = TextRange::new(
                    ByteOffset::new(base + close),
                    ByteOffset::new(base + close + 2),
                ) {
                    out.replace(closing);
                }
            }
        }
    }
}

#[test]
fn a_new_syntax_needs_nothing_outside_its_own_extension() {
    let extensions = ExtensionSet::empty().with(Highlight);
    let decorations = decorate_with(&extensions, "==高亮==", None);

    assert_eq!(
        hidden(&decorations),
        vec![(0, 2), (8, 10)],
        "两对 `==` 从视觉文本里消失，中间的内容留下"
    );

    // 单独跑一个 extension 时它的 id 从 0 开始——局部 id 空间。
    assert_eq!(
        decorations.attrs(StyleId(0)),
        Some(TextAttrs::new(TextStyle::Code))
    );
    assert!(
        !decorations.set().all().is_empty(),
        "新语法应当产出装饰，且不需要注册表之外的任何改动"
    );
}

/// 注册顺序只决定 id 怎么分，不决定装饰怎么定序（不变量 D6）。
///
/// 同**一组** extension 换个顺序注册，产出的隐藏区间必须一模一样。换了顺序
/// 就跟着变的话，那是 extension 之间在相互感知，而且是静默的那种：斜体会变成
/// 等宽，不报错。
#[test]
fn registration_order_does_not_change_what_gets_hidden() {
    let source = "*a* `b` *c* `d`";
    let forward = decorate_with(
        &ExtensionSet::empty().with(HidesEmphasis).with(HidesCode),
        source,
        None,
    );
    let backward = decorate_with(
        &ExtensionSet::empty().with(HidesCode).with(HidesEmphasis),
        source,
        None,
    );
    assert_eq!(hidden(&forward), hidden(&backward));
    assert_eq!(
        forward.set().all().len(),
        backward.set().all().len(),
        "定序由 order_key 全序钉死，条数不该随注册顺序变"
    );

    let ranges = hidden(&forward);
    assert_eq!(
        ranges,
        vec![
            (0, 1),
            (2, 3),
            (4, 5),
            (6, 7),
            (8, 9),
            (10, 11),
            (12, 13),
            (14, 15)
        ]
    );
}

/// 上一条用例的两个 extension。各自只认识一种定界符。
struct HidesEmphasis;

impl Extension for HidesEmphasis {
    fn name(&self) -> &'static str {
        "hides-emphasis"
    }

    fn decorate(&self, cx: &BlockContext<'_>, out: &mut ExtensionOutput) {
        hide_pairs(cx, out, NodeKind::Emphasis, NodeKind::EmphasisMark);
    }
}

struct HidesCode;

impl Extension for HidesCode {
    fn name(&self) -> &'static str {
        "hides-code"
    }

    fn decorate(&self, cx: &BlockContext<'_>, out: &mut ExtensionOutput) {
        hide_pairs(cx, out, NodeKind::InlineCode, NodeKind::CodeMark);
    }
}

fn hide_pairs(cx: &BlockContext<'_>, out: &mut ExtensionOutput, wanted: NodeKind, mark: NodeKind) {
    for node in cx.nodes().filter(|node| node.kind() == wanted) {
        let Some(span) = DelimitedSpan::of(node, |kind| kind == mark) else {
            continue;
        };
        out.replace(span.opening);
        out.replace(span.closing);
    }
}

// ------------------------------------------------- 变异验证补上的守护

/// 一个只做记录的 extension，用来看 `BlockContext` 到底给了什么。
///
/// 状态放在 `Arc` 里：注册表会取走 extension 的所有权，不共享就读不回来。
#[derive(Clone, Default)]
struct Recorder {
    root: Arc<Mutex<Option<NodeKind>>>,
    seen: Arc<Mutex<Vec<(NodeKind, u64, u64)>>>,
}

impl Extension for Recorder {
    fn name(&self) -> &'static str {
        "recorder"
    }

    fn decorate(&self, cx: &BlockContext<'_>, _out: &mut ExtensionOutput) {
        *self.root.lock().expect("测试里不会中毒") = Some(cx.syntax().kind());
        let mut seen = self.seen.lock().expect("测试里不会中毒");
        for node in cx.nodes() {
            seen.push((
                node.kind(),
                node.range().start().get(),
                node.range().end().get(),
            ));
        }
    }
}

/// 跑一个 extension，返回（它看到的根节点，它看到的全部节点，块的 range）。
fn record(extension: impl Extension + 'static, source: &str, block_index: usize) -> TextRange {
    let buffer = TextBuffer::new(source.to_owned());
    let snapshot = buffer.snapshot();
    let document = parse(&snapshot);
    let tree = parse_syntax(&snapshot).expect("测试文档很短").into_tree();
    let block = document.blocks().get(block_index).expect("块存在");
    ExtensionSet::empty()
        .with(extension)
        .decorate(&snapshot, &tree, block, None)
        .expect("装饰产出不该失败");
    block.range()
}

/// 块在语法树里必须定位到**它自己**那个节点，不能一路退到 `Document`。
///
/// 块的 range 带着行尾的换行符，语法节点不带（`# 标题\n` 的块是 0..10，
/// `AtxHeading1` 只有 0..9）。不修剪就包不住，于是每个块都退到根，`nodes()`
/// 再把整篇文档裁一遍——结果仍然对，只是每个块都要走整篇，长文档是
/// O(块数 × 文档长度)。**唯一的症状是慢**，所以只能这样断言着。
#[test]
fn a_block_locates_its_own_syntax_node_not_the_document_root() {
    let source = "# 标题\n\n> 引用\n\n- 项目\n\n```\n代码\n```\n";
    for (index, expected) in [
        (0, NodeKind::AtxHeading1),
        (2, NodeKind::Blockquote),
        (4, NodeKind::ListItem),
        (6, NodeKind::FencedCode),
    ] {
        let recorder = Recorder::default();
        record(recorder.clone(), source, index);
        let root = *recorder.root.lock().expect("测试里不会中毒");
        assert_eq!(
            root,
            Some(expected),
            "第 {index} 个块该定位到 {expected:?}，退到 {root:?} 说明修剪没生效"
        );
    }
}

/// `nodes()` 只给完整落在块内的节点。
///
/// 漏裁的后果是装饰跨到邻块上；装饰按块缓存，那会变成「改了这一块，另一块的
/// 样子也变了」。
#[test]
fn nodes_never_leave_the_block() {
    let source = "# 标题\n\n段落 *斜体*\n\n- 项目\n";
    for index in 0..5 {
        let recorder = Recorder::default();
        let range = record(recorder.clone(), source, index);
        for (kind, from, to) in recorder
            .seen
            .lock()
            .expect("测试里不会中毒")
            .iter()
            .copied()
        {
            assert!(
                from >= range.start().get() && to <= range.end().get(),
                "第 {index} 个块（{range:?}）拿到了块外的 {kind:?} {from}..{to}"
            );
        }
    }
}

/// 一个不守规矩的 extension 产出块外的装饰时，注册表要把它拦下来。
///
/// 这是 `nodes()` 之外的第二道防线。两道防线互相遮蔽——单独去掉任何一道，
/// 另一道都会把后果兜住，所以必须各有一条用例直接压着自己那一道。
struct OutOfBounds;

impl Extension for OutOfBounds {
    fn name(&self) -> &'static str {
        "out-of-bounds"
    }

    fn decorate(&self, cx: &BlockContext<'_>, out: &mut ExtensionOutput) {
        // 故意越界：从文档开头盖到块的末尾。
        if let Some(range) = TextRange::new(ByteOffset::new(0), cx.range().end()) {
            out.replace(range);
        }
        // 块内的这一条必须留下，否则分不清是拦住了还是整个丢了。
        out.replace(cx.range());
    }
}

#[test]
fn the_registry_drops_decorations_that_leave_their_block() {
    let source = "# 标题\n\n段落\n";
    let buffer = TextBuffer::new(source.to_owned());
    let snapshot = buffer.snapshot();
    let document = parse(&snapshot);
    let tree = parse_syntax(&snapshot).expect("测试文档很短").into_tree();
    let block = document.blocks().get(2).expect("第三个块存在");
    assert!(block.range().start().get() > 0, "这个块不从文档开头起");

    let decorations = ExtensionSet::empty()
        .with(OutOfBounds)
        .decorate(&snapshot, &tree, block, None)
        .expect("装饰产出不该失败");

    assert_eq!(
        hidden(&decorations),
        vec![(block.range().start().get(), block.range().end().get())],
        "越界那一条要被拦下，块内那一条要留着"
    );
}

/// 硬换行的行尾符可能是 `\r\n`，要按两个字节算。
///
/// 按一个字节算的话 `\r` 会留在视觉文本里：一个看不见但占位的字符。
#[test]
fn a_crlf_hard_break_hides_both_ending_bytes() {
    assert_eq!(hidden(&decorate("行尾  \r\n第二行", None)), vec![(6, 8)]);
    assert_eq!(hidden(&decorate("行尾\\\r\n第二行", None)), vec![(6, 7)]);
}

/// 链接正文按**正文**字型排，不继承外层。
///
/// 不显式说出来的话，装配层的「窄的赢」会让外层的 `Strong` 赢，
/// `**[文字](url)**` 里的链接正文就变粗了——画面变了，但不报错。
#[test]
fn link_text_does_not_inherit_the_surrounding_style() {
    let decorations = decorate("**[文字](目标)**", None);
    let marks: Vec<_> = decorations
        .set()
        .all()
        .iter()
        .filter_map(|entry| match entry.decoration {
            yu_decoration::Decoration::Mark { style } => {
                decorations.attrs(style).map(|attrs| (entry.range, attrs))
            }
            _ => None,
        })
        .collect();

    let link_text = range(3, 9);
    assert!(
        marks
            .iter()
            .any(|(covered, attrs)| *covered == link_text
                && *attrs == TextAttrs::new(TextStyle::Plain)),
        "链接正文该有一条自己的 Plain mark，实际是 {marks:?}"
    );
    assert!(
        marks
            .iter()
            .any(|(_, attrs)| *attrs == TextAttrs::new(TextStyle::Strong)),
        "外层的 Strong 仍然在，只是盖不住链接正文"
    );
}

/// 空白 run 跨过读窗口边界时也要数对。
///
/// `skip_spaces` 按 64 字节一段读，一次读到块末的话，一个五百行的引用块每个
/// `QuoteMark` 都要复制半个块。分段之后要保证跨段的 run 不会在段边界处停住
/// ——停住的话标题会顶着一串空格往右挪，不报错，只是画得不对。
#[test]
fn a_space_run_that_crosses_the_read_window_is_still_one_run() {
    for spaces in [1_usize, 63, 64, 65, 130] {
        let source = format!("#{}标题", " ".repeat(spaces));
        let decorations = decorate(&source, None);
        assert_eq!(
            hidden(&decorations),
            vec![(0, 1 + spaces as u64)],
            "{spaces} 个空格的前缀没有整段隐藏"
        );
    }
}

/// 往回扫同理。
#[test]
fn a_backward_space_run_that_crosses_the_read_window_is_still_one_run() {
    for spaces in [1_usize, 63, 64, 65, 130] {
        let source = format!("# 标题{}#", " ".repeat(spaces));
        let decorations = decorate(&source, None);
        let end = source.len() as u64;
        assert_eq!(
            hidden(&decorations).last().copied(),
            Some((end - 1 - spaces as u64, end)),
            "{spaces} 个空格加收尾 `#` 没有整段隐藏"
        );
    }
}

// -------------------------------------------------------------- 视觉物件

/// 这个块上的图片。
fn images(
    decorations: &BlockDecorations,
) -> Vec<(TextRange, TextRange, Option<TextRange>, Option<TextRange>)> {
    decorations
        .widgets()
        .iter()
        .copied()
        .map(|BlockWidget::Image(image)| {
            (
                image.source(),
                image.label(),
                image.destination(),
                image.reference(),
            )
        })
        .collect()
}

/// 这个块上每一条 widget 装饰：覆盖的 source 区间与它指向的那个物件。
fn widgets(decorations: &BlockDecorations) -> Vec<(TextRange, BlockWidget)> {
    decorations
        .set()
        .all()
        .iter()
        .filter_map(|entry| match entry.decoration {
            yu_decoration::Decoration::Widget { widget, .. } => {
                decorations.widget(widget).map(|found| (entry.range, found))
            }
            _ => None,
        })
        .collect()
}

/// 图片是一个 widget：整段 `![替代](目标)` 从视觉文本里消失，位置上留一个
/// 盒子。
///
/// 此前替代文字留在视觉文本里，图片盒子画在它上面——盒子有多宽由排出来的
/// 那几个簇说了算，于是同一张图在替代文字长短不同时宽度不一样，而两样都
/// 画出来。
#[test]
fn an_image_is_one_widget_over_its_whole_markup() {
    let decorations = decorate("![替代](图片)", None);
    assert_eq!(
        hidden(&decorations),
        vec![(0, 17)],
        "整段进 widget，一个字节都不留在视觉文本里"
    );
    let placed = widgets(&decorations);
    assert_eq!(placed.len(), 1, "一张图一个 widget，实际是 {placed:?}");
    assert_eq!(placed[0].0, range(0, 17));
    let BlockWidget::Image(image) = placed[0].1;
    assert_eq!(image.source(), range(0, 17));
    assert_eq!(image.label(), range(2, 8));
}

/// 光标进来时 widget 让位，源码原样露出来。
///
/// 不变量 D7 要求 widget 有「可编辑的源码回退」。回退就是这一条：和行内
/// 语法的定界符同一条规则，不是第二套呈现。盒子不让位的话，用户没法改自己
/// 写的那个 URL——图片压在上面。
#[test]
fn a_focused_image_gives_its_source_back_instead_of_a_widget() {
    let decorations = decorate("![替代](图片)", Some(range(5, 5)));
    assert!(hidden(&decorations).is_empty(), "整段源码都要看得见");
    assert!(widgets(&decorations).is_empty(), "露出源码时不再排盒子");
}

/// 图片之外的一个也产 widget 的 extension，用来考 widget id 的平移。
///
/// 它盖在块的头一个字节上。产的同样是 `BlockWidget::Image`——这个枚举现在
/// 只有一个变体，而这里考的是**id 怎么分**，不是它是什么。
struct Stamp;

impl Extension for Stamp {
    fn name(&self) -> &'static str {
        "stamp"
    }

    fn decorate(&self, cx: &BlockContext<'_>, out: &mut ExtensionOutput) {
        let start = cx.range().start();
        let Some(head) = TextRange::new(start, ByteOffset::new(start.get().saturating_add(1)))
        else {
            return;
        };
        let widget = out.widget(BlockWidget::Image(ImageSpan::new(head, head, None, None)));
        out.place_widget(head, widget, WidgetSide::Before);
    }
}

/// widget 的 id 与样式 id 一样是**局部**的，合并时整体平移。
///
/// 共用一个全局编号的话，`WidgetId(0)` 是谁的就取决于谁先跑——那正是 D6
/// 禁止的相互感知，而且是静默的那种：换个注册顺序，图片会画成另一个物件。
#[test]
fn widget_ids_are_rebased_per_extension() {
    let decorations = decorate_with(&ExtensionSet::markdown().with(Stamp), "![替代](图片)", None);
    let BlockWidget::Image(image) = decorations
        .widget(WidgetId(0))
        .expect("图片的 widget 排在前面，它是注册表里更早的那一个");
    assert_eq!(image.source(), range(0, 17));
    let BlockWidget::Image(stamp) = decorations
        .widget(WidgetId(1))
        .expect("后注册的 extension 的局部 0 号被平移成 1 号");
    assert_eq!(stamp.source(), range(0, 1));
    assert_eq!(decorations.widgets().len(), 2);

    // **装饰指的是哪一个**才是 id 平移真正管的事。只查表查不出来：表是按
    // 注册顺序拼的，平不平移都一样长、一样序。不平移的话两条装饰都指向
    // 0 号，图片与图章会画成同一个物件——不报错。
    let placed = widgets(&decorations);
    assert_eq!(
        placed,
        vec![
            (range(0, 1), BlockWidget::Image(stamp)),
            (range(0, 17), BlockWidget::Image(image)),
        ]
    );
}

/// 行内式图片：整段、替代文字、目标各自的区间。
///
/// 画图片盒子的那一层认的就是这三样，由 `Decoration::Widget` 的
/// `WidgetId` 指着。
#[test]
fn an_inline_image_is_annotated_with_its_label_and_destination() {
    let decorations = decorate("![替代](图片)", None);
    assert_eq!(
        images(&decorations),
        vec![(range(0, 17), range(2, 8), Some(range(10, 16)), None)]
    );
}

/// 引用式图片给的是标签，不是目标：目标要查 definition 才知道，而那是
/// 上一层的事（不变量 C6）。
#[test]
fn a_reference_image_is_annotated_with_its_label_instead() {
    let decorations = decorate("![替代][引用]", None);
    let annotated = images(&decorations);
    assert_eq!(annotated.len(), 1, "一张图，实际是 {annotated:?}");
    let (_, _, destination, reference) = annotated[0];
    assert_eq!(destination, None);
    assert!(reference.is_some(), "引用式必须给出标签");
}

/// shortcut 形式 `![替代]` 没有 `LinkLabel` 节点，标签就是替代文字本身。
#[test]
fn a_shortcut_image_falls_back_to_its_own_label() {
    let decorations = decorate("![替代]", None);
    let annotated = images(&decorations);
    assert_eq!(annotated.len(), 1, "一张图，实际是 {annotated:?}");
    let (_, label, destination, reference) = annotated[0];
    assert_eq!(destination, None);
    assert_eq!(reference, Some(label));
}

/// 表格的网格由 `BlockOrnament::Table` 带上来。
///
/// 它是**块级**的，所以走 `Decoration::Line` 那条已有的通道，不走
/// widget——表格的 widget 化还没做，见 overview 第 8 节 S6。
#[test]
fn a_table_carries_its_grid_as_a_block_ornament() {
    let decorations = decorate("a | b\n--- | ---\n1 | 2", None);
    let table = decorations
        .line_ornaments()
        .into_iter()
        .find_map(|(_, ornament)| match ornament {
            BlockOrnament::Table(table) => Some(table),
            _ => None,
        })
        .expect("表格块必须带上它的网格");
    assert_eq!(table.column_count(), 2);
    assert_eq!(table.body_row_count(), 1);
}

/// 竖线、单元格之间的空白、以及整行分隔行都不进视觉文本。
///
/// 取的是「单元格之间」而不是「竖线本身」：对齐的空格数量随内容变，逐个
/// 列举竖线会把它们留在视觉文本里，画面上是一排歪掉的单元格。
#[test]
fn a_table_hides_everything_that_is_not_cell_content() {
    let decorations = decorate("a | b\n--- | ---\n1 | 2", None);
    // `(5, 6)` 是表头那一行的行尾换行符，`(6, 16)` 是整行分隔行。两段相邻
    // 但分别产出——`hidden` 不合并，比对的是「谁产了什么」。
    assert_eq!(
        hidden(&decorations),
        vec![(1, 4), (5, 6), (6, 16), (17, 20)]
    );
}

/// 不是表格的段落一条表格装饰都不产。
///
/// `--- | ---` 这一行是表格的身份证明，缺了它整块就是普通段落。少了这条
/// 用例，「凡是含竖线的段落都当表格排」这种错法不会被任何断言抓住。
#[test]
fn a_paragraph_with_pipes_but_no_delimiter_row_is_not_a_table() {
    let decorations = decorate("a | b\nc | d", None);
    assert!(hidden(&decorations).is_empty(), "普通段落不该藏任何字节");
    assert!(
        decorations
            .line_ornaments()
            .iter()
            .all(|(_, ornament)| !matches!(ornament, BlockOrnament::Table(_))),
        "普通段落不该带表格网格"
    );
}

/// 光标进出表格不改变它的排法。
///
/// 行内语法的定界符碰到光标要露出来，竖线不用：竖线不是一段被藏起来的
/// 文字，而是整个块换了一种排法。跟着焦点变的话，表格会在光标进出时变成
/// 一堆文字又变回去。
#[test]
fn a_table_does_not_reveal_its_pipes_under_the_caret() {
    let source = "a | b\n--- | ---\n1 | 2";
    let focused = decorate(source, Some(range(0, 1)));
    let unfocused = decorate(source, None);
    assert_eq!(hidden(&focused), hidden(&unfocused));
}

// ---------------------------------------------------------------- 语料扫一遍

/// `extension_parity.rs` 的 48 份语料，原样搬过来。
///
/// 那条差分拿 v1 的 `BlockProjection` 当 oracle，逐字节比对「隐藏了哪些
/// 字节」，12 条登记差异全部是 v1 判错。**v1 删掉之后那个 oracle 就没有
/// 了**，语料留下来，断言换成下面这些自洽性质。
///
/// 它压不住「隐藏错了字节」——那件事现在的 oracle 是 CommonMark 官方用例
/// （`yu-syntax/tests/commonmark_spec.rs`，不变量 C7）加上面那些逐条钉死
/// 的用例。它压得住的是**越界与崩**：装饰跨到邻块上、id 指向表外、视觉
/// 文本与隐藏区间对不上。这三样都不 panic，只是画错。
const CORPUS: &[&str] = &[
    "# 标题\n",
    "> 引用\n",
    "- 项目\n",
    "段落\n",
    "*斜体*\n",
    "```rust\nlet x = 1;\n```\n",
    "```\n未闭合\n",
    "```\n```\n",
    "# 标题\n段落\n",
    "- 项目\n\n段落\n",
    "# 标题",
    "## 二级 *斜体* 标题",
    "# 标题 #",
    "#   多空格",
    "> 引用一层",
    "> > 引用两层",
    "> a\n> b",
    "1. 有序",
    "1) 圆括号",
    "  - 缩进项",
    "- a\n- b",
    "- [ ] 待办",
    "- [x] 完成",
    "普通段落 *斜体* 与 **粗体** 与 `代码`",
    "[文字](目标)",
    "[a][b]",
    "![替代](图片)",
    "<http://a.com>",
    "行尾硬换行  \n第二行",
    "行尾\\\n第二行",
    "中文 *强调* 与 emoji 🙂",
    "***both***",
    "***a***b",
    "**a*b***",
    "    indented *em*\n",
    "a\n\n    code *em*\n",
    "\tcode *em*\n",
    "~~~\nfenced *em*\n~~~\n",
    "``a `b` c``",
    "<!-- comment *em* -->",
    "<http://a.com/*b*>",
    "autolink <http://a.com/b>",
    "*a* <http://x.y> *b*",
    "[link *em*](/uri)",
    "![img](/uri)",
    "line *em*  \nnext",
    "a | b\n--- | ---\n1 | 2",
    "",
];

/// 整份文档逐块产一遍装饰。
fn decorate_every_block(source: &str) -> Vec<(TextRange, BlockDecorations)> {
    let buffer = TextBuffer::new(source.to_owned());
    let snapshot = buffer.snapshot();
    let document = parse(&snapshot);
    let tree = parse_syntax(&snapshot).expect("测试文档很短").into_tree();
    let extensions = ExtensionSet::markdown();
    document
        .blocks()
        .iter()
        .map(|block| {
            (
                block.range(),
                extensions
                    .decorate(&snapshot, &tree, block, None)
                    .expect("装饰产出不该失败"),
            )
        })
        .collect()
}

/// 一条装饰都不许越出它那个块。
///
/// 越界的后果是「改了这一块，另一块的样子也变了」——而装饰是按块缓存的，
/// 缓存会把它藏起来，直到某次编辑碰巧让两块一起重建才露出来。
#[test]
fn no_decoration_leaves_its_block_across_the_corpus() {
    for source in CORPUS {
        for (range, decorations) in decorate_every_block(source) {
            for entry in decorations.set().all() {
                assert!(
                    entry.range.start() >= range.start() && entry.range.end() <= range.end(),
                    "{source:?} 的块 {range:?} 产出了越界装饰 {:?}",
                    entry.range
                );
            }
        }
    }
}

/// 每条装饰指向的 id 都必须查得到。
#[test]
fn every_id_resolves_across_the_corpus() {
    for source in CORPUS {
        for (_, decorations) in decorate_every_block(source) {
            for entry in decorations.set().all() {
                match entry.decoration {
                    yu_decoration::Decoration::Mark { style } => assert!(
                        decorations.attrs(style).is_some(),
                        "{source:?} 的 {style:?} 查不到"
                    ),
                    yu_decoration::Decoration::Line { style } => assert!(
                        decorations.ornament(style).is_some(),
                        "{source:?} 的 {style:?} 查不到"
                    ),
                    yu_decoration::Decoration::Widget { widget, .. } => assert!(
                        decorations.widget(widget).is_some(),
                        "{source:?} 的 {widget:?} 查不到"
                    ),
                    yu_decoration::Decoration::Replace => {}
                }
            }
        }
    }
}

/// 视觉长度必须等于块长度减去被隐藏的字节，重叠只算一次。
///
/// 「哪些字节被隐藏」与「隐藏之后有多长」是两套算术：前者是装饰列表，后者
/// 是 `DecorationSet` 的映射索引。对不上的表现是画面比光标少几个字。
#[test]
fn the_visual_length_matches_the_hidden_bytes_across_the_corpus() {
    for source in CORPUS {
        for (range, decorations) in decorate_every_block(source) {
            let set = decorations.set();
            let visible = set
                .source_to_visual(range.end())
                .get()
                .saturating_sub(set.source_to_visual(range.start()).get());
            let mut spans: Vec<(u64, u64)> = set
                .all()
                .iter()
                .filter(|entry| entry.decoration.hides_source())
                .map(|entry| (entry.range.start().get(), entry.range.end().get()))
                .filter(|(from, to)| from < to)
                .collect();
            spans.sort_unstable();
            let mut hidden = 0_u64;
            let mut cursor = range.start().get();
            for (from, to) in spans {
                let from = from.max(cursor);
                if from < to {
                    hidden += to - from;
                    cursor = to;
                }
            }
            assert_eq!(
                range.len().saturating_sub(hidden),
                visible,
                "{source:?} 的块 {range:?} 视觉长度对不上"
            );
        }
    }
}
