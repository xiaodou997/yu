//! 与 `yu-projection` 的 source ↔ visual 差分。
//!
//! # 为什么这是 S4 最该先做的一件事
//!
//! 不变量 D4 要求「O(log n) 的双向映射且 round-trip 无损」。round-trip 是个
//! **自证**性质：一份始终把所有东西映射到 0 的实现也满足它。真正要问的是
//! 「映射到的位置对不对」，而这个问题需要 oracle。
//!
//! `yu-projection` 就是那个 oracle：v1 的投影实现，已经在产品里跑着，它的
//! `source_to_visual` / `visual_to_source` 正是 `yu-decoration` 要取代的东西。
//! S3 移植解析器时没有这个条件（扫描器与 CST 没有共同契约），S4 有。
//!
//! # 差分的形状
//!
//! 隐藏区间**从真实 Projection 里取**（`VisualRunKind::HiddenSyntax`），
//! 再原样喂给 `DecorationSet`。这样两边的输入完全一致，任何差异都只能来自
//! 映射本身——把「隐藏哪些字节」和「隐藏之后怎么映射」分开，是为了让失败
//! 能被归因。前一个问题由 S4 后半段的 decoration 产出器回答。
//!
//! 这条测试随 `yu-projection` 一起消失。

use yu_core::{ByteOffset, Revision, TextRange, VisualOffset};
use yu_decoration::{Bias, Decoration, DecorationRange, DecorationSet};
use yu_projection::{Projection, ProjectionBias, VisualRunKind};
use yu_text::TextBuffer;

/// 真实 Markdown 片段。挑的都是会产生隐藏语法的写法，并且刻意覆盖
/// 「隐藏区间贴着文档开头/结尾」「相邻的隐藏区间」这些边界。
const DOCUMENTS: &[&str] = &[
    "plain text with no syntax",
    "*emphasis* at the start",
    "trailing *emphasis*",
    "*whole line is emphasis*",
    "**strong** and *em* and `code`",
    "***both at once***",
    "a**b**c**d**e",
    "`code` `code` `code`",
    "text with \\*escaped\\* delimiters",
    "unmatched *delimiter stays visible",
    "中文 *强调* 与 emoji 🙂 *混排*",
    "**紧邻**`的`*三段*",
    "`` ` `` backtick inside code",
    "a *b* c\nd *e* f",
    "line one *em*\nline two `code`\nline three",
    "",
    "*",
    "**",
];

/// `source` 的全部 UTF-8 字符边界，含末尾。
fn char_boundaries(source: &str) -> Vec<u64> {
    source
        .char_indices()
        .map(|(index, _)| index as u64)
        .chain(std::iter::once(source.len() as u64))
        .collect()
}

fn projection_of(source: &str) -> Option<(TextBuffer, Projection)> {
    let buffer = TextBuffer::new(source.to_owned());
    let snapshot = buffer.snapshot();
    let range = TextRange::new(
        ByteOffset::ZERO,
        ByteOffset::try_from(source.len()).expect("测试文档很短"),
    )?;
    let projection = Projection::inline(&snapshot, range).ok()?;
    Some((buffer, projection))
}

/// 从 Projection 里取出被隐藏的 source 区间。
fn hidden_ranges(projection: &Projection) -> Vec<TextRange> {
    projection
        .runs()
        .iter()
        .filter(|run| run.kind() == VisualRunKind::HiddenSyntax)
        .map(|run| run.source())
        .filter(|range| !range.is_empty())
        .collect()
}

fn decoration_set(source_len: u64, hidden: &[TextRange]) -> DecorationSet {
    DecorationSet::new(
        Revision::INITIAL,
        ByteOffset::new(source_len),
        hidden
            .iter()
            .map(|range| DecorationRange::new(*range, Decoration::Replace)),
    )
}

/// 先确认两边对「视觉字节流有多长」的理解一致。
///
/// 这一条放在逐点比对之前：模型不一致时，逐点比对会给出一片看不懂的差异，
/// 而这里一句话就说清楚了是哪里对不上。
#[test]
fn the_two_models_agree_on_the_visual_length() {
    for source in DOCUMENTS {
        let Some((_buffer, projection)) = projection_of(source) else {
            continue;
        };
        let hidden = hidden_ranges(&projection);
        let hidden_bytes: u64 = hidden.iter().map(|range| range.len()).sum();
        let decorations = decoration_set(source.len() as u64, &hidden);

        assert_eq!(
            decorations.visual_len().get(),
            source.len() as u64 - hidden_bytes,
            "{source:?}：DecorationSet 的视觉长度应当就是 source 减去隐藏部分"
        );
        assert_eq!(
            decorations.visual_len(),
            projection.visual_len(),
            "{source:?}：两边的视觉长度不一致，隐藏区间之外还有别的东西影响投影"
        );
    }
}

/// source → visual 在每一个偏移上都必须一致。
#[test]
fn source_to_visual_matches_projection_at_every_offset() {
    for source in DOCUMENTS {
        let Some((_buffer, projection)) = projection_of(source) else {
            continue;
        };
        let hidden = hidden_ranges(&projection);
        let decorations = decoration_set(source.len() as u64, &hidden);

        // 只走字符边界：`yu-projection` 会拒绝落在字符中间的偏移，而
        // `DecorationSet` 不持有源码、做不了这个校验（见它的文档）。
        // 两边契约不同的地方不该拿来比对。
        for offset in char_boundaries(source) {
            let ours = decorations.source_to_visual(ByteOffset::new(offset));
            let theirs = projection
                .source_to_visual(ByteOffset::new(offset), ProjectionBias::After)
                .expect("整篇范围内的偏移都合法");
            assert_eq!(
                ours, theirs,
                "{source:?} 的 source {offset}：ours={ours:?} theirs={theirs:?}\n隐藏区间 {hidden:?}"
            );
        }
    }
}

/// visual → source 在每一个偏移、每一种 bias 上都必须一致。
///
/// 这个方向才是难的：一个视觉偏移可能对应一整段被隐藏的 source，
/// `Before` / `After` 选哪一端，而且连续的隐藏区间要一起跳过。
/// `yu-projection` 是在查询时沿着后继找的，这里改成构造期合并——
/// 两条不同的路必须给出同一个答案。
#[test]
fn visual_to_source_matches_projection_for_both_biases() {
    for source in DOCUMENTS {
        let Some((_buffer, projection)) = projection_of(source) else {
            continue;
        };
        let hidden = hidden_ranges(&projection);
        let decorations = decoration_set(source.len() as u64, &hidden);

        for visual in 0..=decorations.visual_len().get() {
            for (ours_bias, theirs_bias) in [
                (Bias::Before, ProjectionBias::Before),
                (Bias::After, ProjectionBias::After),
            ] {
                let ours = decorations.visual_to_source(VisualOffset::new(visual), ours_bias);
                let theirs = projection
                    .visual_to_source(VisualOffset::new(visual), theirs_bias)
                    .expect("投影长度之内的偏移都合法");
                assert_eq!(
                    ours, theirs,
                    "{source:?} 的 visual {visual} / {ours_bias:?}：\
                     ours={ours:?} theirs={theirs:?}\n隐藏区间 {hidden:?}"
                );
            }
        }
    }
}

/// 语料必须真的产生了隐藏区间，否则上面三条测试比的是一堆恒等映射。
///
/// 「差分测试通过了但其实什么都没测」是 S3 点名的危险类别，这里同样适用。
#[test]
fn the_corpus_actually_produces_hidden_syntax() {
    let with_hidden = DOCUMENTS
        .iter()
        .filter_map(|source| projection_of(source))
        .filter(|(_, projection)| !hidden_ranges(projection).is_empty())
        .count();
    assert!(
        with_hidden >= 10,
        "只有 {with_hidden} 份语料产生了隐藏语法，差分基本上在比恒等映射"
    );

    // 也要真的出现过「相邻的隐藏区间」，那是合并逻辑唯一的守护点。
    let with_adjacent = DOCUMENTS
        .iter()
        .filter_map(|source| projection_of(source))
        .filter(|(_, projection)| {
            let hidden = hidden_ranges(projection);
            hidden
                .windows(2)
                .any(|pair| pair[0].end() == pair[1].start())
        })
        .count();
    assert!(
        with_adjacent >= 1,
        "语料里没有相邻的隐藏区间，合并逻辑没有被差分覆盖到"
    );
}
