//! Immutable source ↔ visual interval index. Both hidden syntax and nonempty
//! replacement atoms use the same prefix coordinates and O(log n) queries.
//! Byte/character boundary validation belongs to the source-owning projection.
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Bias {
    Before,
    #[default]
    After,
}

#[derive(Clone, Copy, Debug)]
struct ProjectedSpan {
    from: u64,
    to: u64,
    visual_from: u64,
    visual_to: u64,
}

#[derive(Clone)]
pub(crate) struct ProjectionIndex {
    spans: Arc<[ProjectedSpan]>,
    source_len: u64,
    visual_len: u64,
}

impl ProjectionIndex {
    #[cfg(test)]
    pub(crate) fn build(hidden: &[(u64, u64)], source_len: u64) -> Self {
        Self::build_projected(hidden.iter().map(|&(from, to)| (from, to, 0)), source_len)
    }

    /// Sorted, nonoverlapping source intervals paired with replacement byte
    /// lengths. Zero length hides source; positive length is an atomic substitute.
    /// Adjacent intervals are legal, including runs of zero-width substitutions.
    pub(crate) fn build_projected(
        spans: impl IntoIterator<Item = (u64, u64, u64)>,
        source_len: u64,
    ) -> Self {
        let mut source_cursor = 0;
        let mut visual_cursor = 0_u64;
        let mut projected = Vec::new();
        for (from, to, length) in spans {
            assert!(
                source_cursor <= from && from < to && to <= source_len,
                "projection intervals must be ordered, nonempty and within source"
            );
            let visual_from = visual_cursor
                .checked_add(from - source_cursor)
                .expect("projection byte length overflow");
            let visual_to = visual_from
                .checked_add(length)
                .expect("projection byte length overflow");
            projected.push(ProjectedSpan {
                from,
                to,
                visual_from,
                visual_to,
            });
            source_cursor = to;
            visual_cursor = visual_to;
        }
        Self {
            spans: projected.into(),
            source_len,
            visual_len: visual_cursor
                .checked_add(source_len - source_cursor)
                .expect("projection byte length overflow"),
        }
    }

    #[cfg(test)]
    pub(crate) fn source_len(&self) -> u64 {
        self.source_len
    }

    pub(crate) fn visual_len(&self) -> u64 {
        self.visual_len
    }

    /// Source interiors map to the start of their replacement atom; the source
    /// end maps to its visual end. No second offset adjustment exists downstream.
    pub(crate) fn visual_for_source(&self, source: u64) -> u64 {
        let source = source.min(self.source_len);
        let index = self.spans.partition_point(|span| span.to <= source);
        if let Some(span) = self.spans.get(index)
            && source >= span.from
        {
            return span.visual_from;
        }
        if index == 0 {
            source
        } else {
            let previous = self.spans[index - 1];
            previous.visual_to + (source - previous.to)
        }
    }

    /// Exact visible atom boundaries remain exact. Interior visual offsets use
    /// bias to choose a source edge. At collapsed runs Before chooses the first
    /// source edge and After the last, without scanning adjacent intervals.
    pub(crate) fn source_for_visual(&self, visual: u64, bias: Bias) -> u64 {
        let visual = visual.min(self.visual_len);
        let index = self.spans.partition_point(|span| match bias {
            Bias::Before => span.visual_to < visual,
            Bias::After => span.visual_to <= visual,
        });
        if let Some(span) = self.spans.get(index)
            && visual >= span.visual_from
        {
            if visual == span.visual_from {
                return span.from;
            }
            if visual == span.visual_to {
                return span.to;
            }
            return match bias {
                Bias::Before => span.from,
                Bias::After => span.to,
            };
        }
        if index == 0 {
            visual
        } else {
            let previous = self.spans[index - 1];
            previous.to + (visual - previous.visual_to)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Bias, ProjectionIndex};

    /// 独立的线性参照实现；二分索引必须与它逐点一致。
    ///
    /// 留着它是因为「O(log n) 的查询」和「显然正确的线性扫」是两份独立的
    /// 推理，下面的用例拿它们互相校验。只有一份实现的时候，写错了没人知道。
    fn reference_visual_for_source(hidden: &[(u64, u64)], source_len: u64, source: u64) -> u64 {
        let source = source.min(source_len);
        let mut removed = 0_u64;
        for &(from, to) in hidden {
            if source <= from {
                break;
            }
            removed += source.min(to) - from;
        }
        source - removed
    }

    fn reference_source_for_visual(
        hidden: &[(u64, u64)],
        source_len: u64,
        visual: u64,
        bias: Bias,
    ) -> u64 {
        // 所有映射到 `visual` 的 source 偏移构成一个闭区间，取它的两端。
        let mut first = None;
        let mut last = 0_u64;
        for source in 0..=source_len {
            if reference_visual_for_source(hidden, source_len, source) == visual {
                if first.is_none() {
                    first = Some(source);
                }
                last = source;
            }
        }
        match bias {
            Bias::Before => first.unwrap_or(0),
            Bias::After => last,
        }
    }

    fn check(hidden: &[(u64, u64)], source_len: u64) {
        let index = ProjectionIndex::build(hidden, source_len);
        assert_eq!(index.source_len(), source_len);
        for source in 0..=source_len {
            assert_eq!(
                index.visual_for_source(source),
                reference_visual_for_source(hidden, source_len, source),
                "source {source} 在 {hidden:?} / len {source_len} 上不一致"
            );
        }
        for visual in 0..=index.visual_len() {
            for bias in [Bias::Before, Bias::After] {
                assert_eq!(
                    index.source_for_visual(visual, bias),
                    reference_source_for_visual(hidden, source_len, visual, bias),
                    "visual {visual} / {bias:?} 在 {hidden:?} / len {source_len} 上不一致"
                );
            }
        }
    }

    #[test]
    fn binary_index_matches_the_linear_reference() {
        check(&[], 0);
        check(&[], 10);
        check(&[(0, 3)], 10);
        check(&[(0, 10)], 10);
        check(&[(2, 5)], 10);
        check(&[(7, 10)], 10);
        check(&[(0, 2), (5, 7)], 10);
        check(&[(1, 2), (3, 4), (5, 6), (7, 8)], 9);
    }

    /// 足够多的区间覆盖二分查找的各级边界。
    #[test]
    fn binary_search_is_exercised_across_many_intervals() {
        let hidden: Vec<(u64, u64)> = (0..200)
            .map(|index| (index * 5 + 1, index * 5 + 3))
            .collect();
        let source_len = 200 * 5 + 4;
        check(&hidden, source_len);
    }

    /// round-trip 无损（不变量 D4）：任何 visual 偏移换成 source 再换回来，
    /// 必须回到原处。
    #[test]
    fn visual_round_trips_through_source() {
        let hidden = [(0, 2), (5, 7), (11, 15)];
        let index = ProjectionIndex::build(&hidden, 20);
        for visual in 0..=index.visual_len() {
            for bias in [Bias::Before, Bias::After] {
                let source = index.source_for_visual(visual, bias);
                assert_eq!(
                    index.visual_for_source(source),
                    visual,
                    "visual {visual} / {bias:?} round-trip 丢了"
                );
            }
        }
    }

    #[test]
    fn replacement_boundaries_and_adjacent_hidden_runs_have_exact_bias() {
        let index = ProjectionIndex::build_projected(
            [
                (0, 2, 0),
                (2, 6, 1),
                (6, 8, 0),
                (8, 10, 0),
                (10, 12, 4),
                (13, 15, 0),
            ],
            16,
        );
        assert_eq!(index.visual_len(), 7);
        let source_to_visual = [0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 5, 6, 6, 6, 7];
        for (source, visual) in source_to_visual.into_iter().enumerate() {
            assert_eq!(index.visual_for_source(source as u64), visual);
        }
        let before = [0, 6, 10, 10, 10, 12, 13, 16];
        let after = [2, 10, 12, 12, 12, 12, 15, 16];
        for visual in 0..=7 {
            assert_eq!(
                index.source_for_visual(visual, Bias::Before),
                before[visual as usize]
            );
            assert_eq!(
                index.source_for_visual(visual, Bias::After),
                after[visual as usize]
            );
        }
        assert_eq!(index.visual_for_source(u64::MAX), 7);
        assert_eq!(index.source_for_visual(u64::MAX, Bias::After), 16);
    }

    #[test]
    fn adjacent_hidden_intervals_share_the_full_collapsed_boundary() {
        let index = ProjectionIndex::build_projected([(1, 3, 0), (3, 5, 0), (5, 7, 0)], 8);
        assert_eq!(index.visual_len(), 2);
        assert_eq!(index.source_for_visual(1, Bias::Before), 1);
        assert_eq!(index.source_for_visual(1, Bias::After), 7);
        for source in 1..=7 {
            assert_eq!(index.visual_for_source(source), 1);
        }
    }

    #[test]
    fn replacing_the_entire_source_keeps_both_visual_edges() {
        let index = ProjectionIndex::build_projected([(0, 4, 1)], 4);
        for bias in [Bias::Before, Bias::After] {
            assert_eq!(index.source_for_visual(0, bias), 0);
            assert_eq!(index.source_for_visual(1, bias), 4);
        }
        assert_eq!(index.visual_for_source(3), 0);
        assert_eq!(index.visual_for_source(4), 1);
        let clone = index.clone();
        assert_eq!(clone.source_for_visual(1, Bias::After), 4);
    }
}
