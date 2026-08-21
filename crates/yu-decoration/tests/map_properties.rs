//! **不变量 D3**：`map(&ChangeSet)` 必须使 DecorationSet 随 Transaction 正确
//! 迁移，边界 bias 显式声明。「此性质由 property-based 测试守护」是原文。
//!
//! # 为什么这里必须是 property test
//!
//! 迁移的错误不是「整体错位」那种一眼可见的错，而是「某个装饰的某一端在某种
//! 编辑下差一个字节」。这类错误的触发条件是编辑与装饰边界的相对位置——插在
//! 前面/后面/正好在边界上/跨过整条装饰/把它删光——组合起来几十种，手写用例
//! 一定会漏，而漏掉的那种表现是光标和高亮差一格，不报错。
//!
//! # 检查什么
//!
//! 每一步随机编辑之后：
//!
//! 1. **与 Anchor 一致。** 装饰的两端必须与「把同一个位置做成 Anchor 再迁移」
//!    落在同一处。这一条把装饰的边界语义钉死在既有的 Anchor 语义上
//!    （第 6.4 节要求「与既有 Anchor 语义对齐并交叉验证」）。
//! 2. **结构自洽。** 起点不超过终点、不越过文档末尾、视觉长度等于 source
//!    长度减去隐藏字节数。
//! 3. **round-trip 无损**（不变量 D4），在迁移之后仍然成立。

use proptest::prelude::*;
use yu_core::{Affinity, ByteOffset, Revision, TextAnchor, TextRange, VisualOffset};
use yu_decoration::{Bias, Decoration, DecorationRange, DecorationSet, StyleId};
use yu_text::{Edit, TextBuffer, Transaction};

/// 一次编辑：在 `at` 处删掉 `remove` 个字节，插入 `insert`。
#[derive(Clone, Debug)]
struct Step {
    at: usize,
    remove: usize,
    insert: String,
}

fn step_strategy() -> impl Strategy<Value = Step> {
    (
        0_usize..32,
        0_usize..8,
        prop::sample::select(vec!["", "x", "ab", "\n", "##", "*", "羽"]),
    )
        .prop_map(|(at, remove, insert)| Step {
            at,
            remove,
            insert: insert.to_owned(),
        })
}

/// 装饰的初始摆位。用 `(from, len, hides)` 生成，避免生成大量非法区间。
fn decorations_strategy() -> impl Strategy<Value = Vec<(usize, usize, bool)>> {
    prop::collection::vec((0_usize..30, 0_usize..6, any::<bool>()), 0..6)
}

fn build(source_len: u64, spec: &[(usize, usize, bool)]) -> DecorationSet {
    let ranges = spec.iter().filter_map(|&(from, len, hides)| {
        let from = ByteOffset::new((from as u64).min(source_len));
        let to = ByteOffset::new(((from.get()) + len as u64).min(source_len));
        let range = TextRange::new(from, to)?;
        let decoration = if hides {
            Decoration::Replace
        } else {
            Decoration::Mark { style: StyleId(1) }
        };
        Some(DecorationRange::new(range, decoration))
    });
    DecorationSet::new(Revision::INITIAL, ByteOffset::new(source_len), ranges)
}

/// 把一个位置按与 `map` 相同的 affinity 迁移。两者必须给出同一个答案。
fn map_offset(
    changes: &yu_text::ChangeSet,
    revision: Revision,
    offset: ByteOffset,
    affinity: Affinity,
) -> ByteOffset {
    changes
        .map_anchor(TextAnchor::new(revision, offset, affinity))
        .expect("同 revision 的锚点应当能迁移")
        .offset()
}

fn check_structure(set: &DecorationSet, label: &str) {
    let source_len = set.source_len();
    let mut hidden_bytes = 0_u64;
    let mut merged: Vec<(u64, u64)> = Vec::new();
    for entry in set.all() {
        assert!(
            entry.range.end() <= source_len,
            "{label}：装饰越过了文档末尾 {entry:?} / len {source_len:?}"
        );
        if !matches!(entry.decoration, Decoration::Replace) {
            continue;
        }
        let (from, to) = (entry.range.start().get(), entry.range.end().get());
        match merged.last_mut() {
            Some(last) if from <= last.1 => last.1 = last.1.max(to),
            _ => merged.push((from, to)),
        }
    }
    for (from, to) in merged {
        hidden_bytes += to - from;
    }
    assert_eq!(
        set.visual_len().get(),
        source_len.get() - hidden_bytes,
        "{label}：视觉长度与隐藏字节数对不上"
    );

    // round-trip 无损（不变量 D4）。
    for visual in 0..=set.visual_len().get() {
        for bias in [Bias::Before, Bias::After] {
            let source = set.visual_to_source(VisualOffset::new(visual), bias);
            assert_eq!(
                set.source_to_visual(source).get(),
                visual,
                "{label}：visual {visual} / {bias:?} 的 round-trip 丢了"
            );
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// 一连串随机编辑之后，装饰的每一端都必须与同位置的 Anchor 落在一处。
    #[test]
    fn decorations_migrate_exactly_like_anchors(
        spec in decorations_strategy(),
        steps in prop::collection::vec(step_strategy(), 1..8),
    ) {
        let source = "# Title\n\nSome *text* here.\nAnd more.\n";
        let mut buffer = TextBuffer::new(source.to_owned());
        let mut set = build(source.len() as u64, &spec);
        check_structure(&set, "初始");

        for (index, step) in steps.iter().enumerate() {
            let text = buffer.snapshot();
            let len = text.len_bytes().get() as usize;
            // 夹到合法的字符边界上；越界或半个字符的编辑不是这条性质要测的。
            let at = nearest_boundary(text.as_str(), step.at.min(len));
            let end = nearest_boundary(text.as_str(), (at + step.remove).min(len));
            let range = TextRange::new(
                ByteOffset::new(at as u64),
                ByteOffset::new(end as u64),
            ).expect("有序");

            let before = set.all().to_vec();
            let revision = buffer.revision();
            let transaction = Transaction::new(revision, [Edit::new(range, step.insert.as_str())]);
            let applied = buffer.apply(&transaction).expect("夹过的编辑应当合法");
            let changes = applied.change_set().clone();

            let mapped = set.map(&changes).expect("同 revision 应当能迁移");

            // 1. 与 Anchor 一致。
            let expected: Vec<TextRange> = before
                .iter()
                .filter_map(|entry| {
                    let start = map_offset(&changes, revision, entry.range.start(), Affinity::After);
                    let end = map_offset(&changes, revision, entry.range.end(), Affinity::Before);
                    let end = end.max(start);
                    let range = TextRange::new(start, end)?;
                    // 隐藏类装饰被删空之后会被丢掉，对照组也要照做。
                    if range.is_empty() && entry.decoration.hides_source() {
                        return None;
                    }
                    Some(range)
                })
                .collect();
            let actual: Vec<TextRange> = mapped
                .all()
                .iter()
                .map(|entry| entry.range)
                .collect();
            let mut expected_sorted = expected.clone();
            expected_sorted.sort_by_key(|range| (range.start().get(), range.end().get()));
            let mut actual_sorted = actual.clone();
            actual_sorted.sort_by_key(|range| (range.start().get(), range.end().get()));
            prop_assert_eq!(
                &actual_sorted,
                &expected_sorted,
                "第 {} 步之后装饰的位置与 Anchor 不一致",
                index
            );

            // 2/3. 结构自洽与 round-trip。
            check_structure(&mapped, &format!("第 {index} 步之后"));
            prop_assert_eq!(
                mapped.source_len().get(),
                buffer.snapshot().len_bytes().get(),
                "第 {} 步之后装饰集合记的文档长度与实际不符",
                index
            );
            set = mapped;
        }
    }
}

/// 把字节偏移夹到最近的（不大于它的）字符边界。
fn nearest_boundary(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while offset > 0 && !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}
