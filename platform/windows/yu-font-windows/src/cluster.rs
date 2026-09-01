//! DirectWrite 的 `clusterMap` 反过来读：文本 → 字形，变成字形 → 文本。
//!
//! **这个模块里一行 DirectWrite 调用都没有，也不需要 Windows 才能跑。**
//! 那是有意的，理由与 S7 第七刀 b 把 `cluster_spans` 从 `shape_run` 里抽出来
//! 是同一条：真实后端造不出让两种取法分开的输入，判据必须落在纯函数上，
//! 反向验证才有手段。这里更极端一点——开发机上**根本没有** DirectWrite。
//!
//! # 两边的方向是相反的
//!
//! CoreText 的 `CTRunGetStringIndices` 是**字形 → 文本**：第 i 个字形从文本
//! 的哪里来。DirectWrite 的 `GetGlyphs` 给的 `clusterMap` 是**文本 → 字形**：
//! `clusterMap[i]` 是文本位置 `i` 所属那一簇的**首个字形下标**，索引单位是
//! UTF-16 code unit。
//!
//! 官方文档对它只有一句「contains the mapping from character ranges to glyph
//! ranges」，说不清 `clusterMap[i]` 到底装什么。实际语义要从实现里读：
//! HarfBuzz 的 DirectWrite shaper 把它当 `log_clusters` 用。
//!
//! # 一簇多形没有第三条路
//!
//! 一个簇映射到多个字形时（天城文、部分 emoji 序列），后续那些字形在
//! `clusterMap` 里**根本不出现**。S7 第七刀 spike 把三种凑法各跑了一遍：
//!
//! | 凑法 | 赔掉什么 |
//! | --- | --- |
//! | 重复起点（HarfBuzz 式） | 重画——`yu-layout` 的 tiling 门直接拒 |
//! | 末尾补空区间 | **越界 panic**（实测），中间的则凭空多一段 advance |
//! | 并成一形 | 丢字形 |
//!
//! 契约（E7）因此要求**后端做不到就报错，不许伪造区间**。报错之后
//! `yu-layout` 走 I5 的两级降级（逐簇重试 → 替代字形），那是布局层的事。
//!
//! # 这一层不做 UTF-16 → UTF-8 的换算
//!
//! 反转的产出是 **UTF-16 code unit 区间**；换成 `Glyph::source` 要的字节区间
//! 由 [`yu_font::Utf16Map`] 做（它在 S7 第七刀 b 从 `yu-font-macos` 提到
//! `yu-font`，正是为了这一天）。两件事分开是因为它们各自会错得不一样：
//! 反转错了是字形与文本对不上，换算错了是把一个字符劈成两半。

use std::fmt;

/// 反转失败的原因。每一种都对应契约里的一条，不是「解析出错」。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClusterMapError {
    /// 一个簇映射到多个字形。契约 C3 + C4 要求一簇一形，凑不出来就报错。
    MultiGlyphCluster { glyph: usize },
    /// `clusterMap` 指向的字形下标越界。
    GlyphIndexOutOfRange { text: usize, glyph: u16 },
    /// 簇的首字形下标非单调递减。DirectWrite 在 LTR 下不会这样，
    /// 出现了说明这个 run 不是我们以为的那种。
    NonMonotonic { text: usize },
    /// 有字形没有任何文本位置映射到它，而它也不是某一簇的后续字形。
    UnmappedGlyph { glyph: usize },
    /// `clusterMap` 是空的但字形不是，或者反过来。
    LengthMismatch { text_len: usize, glyph_count: usize },
}

impl fmt::Display for ClusterMapError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MultiGlyphCluster { glyph } => write!(
                formatter,
                "cluster maps to more than one glyph at glyph {glyph}"
            ),
            Self::GlyphIndexOutOfRange { text, glyph } => write!(
                formatter,
                "cluster map entry {glyph} at text unit {text} is out of range"
            ),
            Self::NonMonotonic { text } => {
                write!(
                    formatter,
                    "cluster map is not monotonic at text unit {text}"
                )
            }
            Self::UnmappedGlyph { glyph } => {
                write!(formatter, "glyph {glyph} has no text mapped to it")
            }
            Self::LengthMismatch {
                text_len,
                glyph_count,
            } => write!(
                formatter,
                "cluster map covers {text_len} text units but there are {glyph_count} glyphs"
            ),
        }
    }
}

impl std::error::Error for ClusterMapError {}

/// 一个字形覆盖的 UTF-16 区间，半开。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GlyphSpan {
    pub start_utf16: usize,
    pub end_utf16: usize,
}

/// 把 `clusterMap`（文本 → 字形）反过来，得到每个字形覆盖的 UTF-16 区间。
///
/// 产出的区间**首尾相接、不重叠、非空**地铺满 `[0, cluster_map.len())`
/// ——那正是契约 C3 与 C4 合起来的那一条，也是 `yu-layout` 的 tiling 门会
/// 再查一遍的东西。凑不出来就返回 `Err`，**不伪造**。
pub fn glyph_spans(
    cluster_map: &[u16],
    glyph_count: usize,
) -> Result<Vec<GlyphSpan>, ClusterMapError> {
    if cluster_map.is_empty() != (glyph_count == 0) {
        return Err(ClusterMapError::LengthMismatch {
            text_len: cluster_map.len(),
            glyph_count,
        });
    }
    if cluster_map.is_empty() {
        return Ok(Vec::new());
    }

    // 每个字形的起点：第一个映射到它的文本位置。
    let mut starts: Vec<Option<usize>> = vec![None; glyph_count];
    let mut previous: Option<u16> = None;
    for (text, &glyph) in cluster_map.iter().enumerate() {
        let index = usize::from(glyph);
        if index >= glyph_count {
            return Err(ClusterMapError::GlyphIndexOutOfRange { text, glyph });
        }
        if let Some(previous) = previous
            && glyph < previous
        {
            return Err(ClusterMapError::NonMonotonic { text });
        }
        previous = Some(glyph);
        if starts[index].is_none() {
            starts[index] = Some(text);
        }
    }

    // 没有任何文本映射到它的字形，就是「一簇多形」里那些多出来的。
    //
    // **这里是这个模块唯一真正做判断的地方。** 三种凑法（重复起点 /
    // 空区间 / 并成一形）都能让下面这个循环产出点什么，而三种都是错的
    // ——分别是重画、越界 panic、丢字形。契约说的是报错。
    for (glyph, start) in starts.iter().enumerate() {
        if start.is_none() {
            // 第 0 个字形没人映射说明 clusterMap 根本没覆盖开头；
            // 其余位置没人映射说明前一个簇产出了不止一个字形。
            return Err(if glyph == 0 {
                ClusterMapError::UnmappedGlyph { glyph }
            } else {
                ClusterMapError::MultiGlyphCluster { glyph }
            });
        }
    }

    let mut spans = Vec::with_capacity(glyph_count);
    for glyph in 0..glyph_count {
        let start = starts[glyph].expect("checked above");
        let end = if glyph + 1 < glyph_count {
            starts[glyph + 1].expect("checked above")
        } else {
            cluster_map.len()
        };
        // 非空是契约 C4。等于或小于都说明两个字形抢同一个起点，
        // 那是「重复起点」那条凑法的形状。
        if end <= start {
            return Err(ClusterMapError::MultiGlyphCluster { glyph });
        }
        spans.push(GlyphSpan {
            start_utf16: start,
            end_utf16: end,
        });
    }
    Ok(spans)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spans(pairs: &[(usize, usize)]) -> Vec<GlyphSpan> {
        pairs
            .iter()
            .map(|&(start_utf16, end_utf16)| GlyphSpan {
                start_utf16,
                end_utf16,
            })
            .collect()
    }

    /// 一对一：三个 code unit 三个字形。
    #[test]
    fn one_to_one_tiles_the_run() {
        assert_eq!(
            glyph_spans(&[0, 1, 2], 3),
            Ok(spans(&[(0, 1), (1, 2), (2, 3)]))
        );
    }

    /// 连字：两个 code unit 并成一个字形。**这一种必须过**——它是拉丁文里
    /// 的常态（`fi`、`->`），不是边角情形。
    #[test]
    fn a_ligature_is_one_glyph_covering_several_code_units() {
        assert_eq!(glyph_spans(&[0, 0], 1), Ok(spans(&[(0, 2)])));
        assert_eq!(
            glyph_spans(&[0, 0, 1, 2, 2], 3),
            Ok(spans(&[(0, 2), (2, 3), (3, 5)]))
        );
    }

    /// **一簇多形报错，不凑。** 这是这个模块存在的理由。
    ///
    /// `[0]` 配两个字形：第 1 个字形没有任何文本映射到它。三种凑法分别会
    /// 产出 `(0,1),(0,1)`（重画）、`(0,1),(1,1)`（末尾空区间，实测让
    /// `yu-layout` 越界 panic）、只产 `(0,1)`（丢字形）。全错。
    #[test]
    fn a_cluster_with_several_glyphs_is_an_error_not_a_guess() {
        assert_eq!(
            glyph_spans(&[0], 2),
            Err(ClusterMapError::MultiGlyphCluster { glyph: 1 })
        );
        // 中间的一簇多形同样要报错——它不 panic，是三种里最容易溜过去的。
        assert_eq!(
            glyph_spans(&[0, 1, 3], 4),
            Err(ClusterMapError::MultiGlyphCluster { glyph: 2 })
        );
        // M:N。
        assert_eq!(
            glyph_spans(&[0, 0], 3),
            Err(ClusterMapError::MultiGlyphCluster { glyph: 1 })
        );
    }

    /// 空 run 是合法的；一边空一边不空不是。
    #[test]
    fn emptiness_must_agree_on_both_sides() {
        assert_eq!(glyph_spans(&[], 0), Ok(Vec::new()));
        assert_eq!(
            glyph_spans(&[], 1),
            Err(ClusterMapError::LengthMismatch {
                text_len: 0,
                glyph_count: 1,
            })
        );
        assert_eq!(
            glyph_spans(&[0], 0),
            Err(ClusterMapError::LengthMismatch {
                text_len: 1,
                glyph_count: 0,
            })
        );
    }

    /// 越界与非单调各自报自己的名字——排查时要知道是后端给错了下标，
    /// 还是这个 run 根本不是 LTR。
    #[test]
    fn a_broken_cluster_map_names_its_own_problem() {
        assert_eq!(
            glyph_spans(&[0, 5], 2),
            Err(ClusterMapError::GlyphIndexOutOfRange { text: 1, glyph: 5 })
        );
        assert_eq!(
            glyph_spans(&[1, 0], 2),
            Err(ClusterMapError::NonMonotonic { text: 1 })
        );
        // 开头就没人映射到 0 号字形：clusterMap 没覆盖 run 的开头。
        assert_eq!(
            glyph_spans(&[1, 1], 2),
            Err(ClusterMapError::UnmappedGlyph { glyph: 0 })
        );
    }

    /// **产出必须真的铺满**，而不只是「看起来对」。
    ///
    /// 这条是拿契约本身当判据：任何一个 `Ok` 的结果，区间必须首尾相接、
    /// 非空、并且末尾正好等于文本长度。一般式，不是某一个语料。
    #[test]
    fn every_accepted_cluster_map_tiles_the_run() {
        let maps: [(&[u16], usize); 5] = [
            (&[0, 1, 2], 3),
            (&[0, 0], 1),
            (&[0, 0, 1, 2, 2], 3),
            (&[0], 1),
            (&[0, 1, 1, 2, 3, 3, 3], 4),
        ];
        for (map, glyphs) in maps {
            let spans = glyph_spans(map, glyphs)
                .unwrap_or_else(|error| panic!("{map:?} 应当合规：{error}"));
            assert_eq!(spans.len(), glyphs, "{map:?}");
            let mut cursor = 0;
            for span in &spans {
                assert_eq!(span.start_utf16, cursor, "{map:?} 没有首尾相接");
                assert!(span.end_utf16 > span.start_utf16, "{map:?} 有空区间");
                cursor = span.end_utf16;
            }
            assert_eq!(cursor, map.len(), "{map:?} 没有铺满");
        }
    }
}
