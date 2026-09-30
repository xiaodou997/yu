//! DirectWrite 的 `clusterMap` 反向翻译。
//!
//! `IDWriteTextAnalyzer::GetGlyphs` 给的是“UTF-16 文本位置 → cluster 首字形下标”。
//! Yu 的共享契约则要求每个字形携带它所属的完整 source cluster。关键点是：
//! **一个 cluster 可以对应多个字形**，因此这里产出 cluster 与 glyph-range 的关系，
//! 而不是强行给每个 glyph 切一个互不重叠的文本区间。
//!
//! 这个模块没有 DirectWrite 调用，所有边界条件都能在非 Windows 开发机执行。

use std::fmt;

use yu_core::TextDirection;

/// clusterMap 翻译失败。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClusterMapError {
    /// `clusterMap` 指向不存在的字形。
    GlyphIndexOutOfRange { text: usize, glyph: u16 },
    /// cluster 首字形下标与 run 方向不一致。
    NonMonotonic { text: usize },
    /// 字形数组有一段没有被任何 cluster 覆盖。
    UnmappedGlyph { glyph: usize },
    /// 文本与字形一边为空、一边非空。
    LengthMismatch { text_len: usize, glyph_count: usize },
}

impl fmt::Display for ClusterMapError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GlyphIndexOutOfRange { text, glyph } => write!(
                formatter,
                "cluster map entry {glyph} at text unit {text} is out of range"
            ),
            Self::NonMonotonic { text } => {
                write!(
                    formatter,
                    "cluster map is not monotonic for the run direction at text unit {text}"
                )
            }
            Self::UnmappedGlyph { glyph } => {
                write!(formatter, "glyph {glyph} is not covered by any cluster")
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

/// 一个逻辑 source cluster 及其在 DirectWrite 字形数组中的连续区间。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GlyphCluster {
    pub start_utf16: usize,
    pub end_utf16: usize,
    pub glyph_start: usize,
    pub glyph_end: usize,
}

/// 把 DirectWrite `clusterMap` 翻成逻辑 cluster。
///
/// 返回值按**源码逻辑顺序**排列；每个 `glyph_start..glyph_end` 是后端原生字形
/// 数组中的连续片段。LTR 时这些片段递增，RTL 时递减。一个片段可以包含多个
/// glyph，这正是 many-to-many shaping 的正常表达。
pub fn glyph_clusters(
    cluster_map: &[u16],
    glyph_count: usize,
    direction: TextDirection,
) -> Result<Vec<GlyphCluster>, ClusterMapError> {
    if cluster_map.is_empty() != (glyph_count == 0) {
        return Err(ClusterMapError::LengthMismatch {
            text_len: cluster_map.len(),
            glyph_count,
        });
    }
    if cluster_map.is_empty() {
        return Ok(Vec::new());
    }

    // 先把相邻、同 cluster 的 UTF-16 code unit 压成一个逻辑 cluster。
    let mut starts = Vec::<(usize, usize)>::new();
    let mut previous = None;
    for (text, &raw_glyph) in cluster_map.iter().enumerate() {
        let glyph = usize::from(raw_glyph);
        if glyph >= glyph_count {
            return Err(ClusterMapError::GlyphIndexOutOfRange {
                text,
                glyph: raw_glyph,
            });
        }
        if let Some(previous) = previous {
            let wrong_direction = match direction {
                TextDirection::Ltr => raw_glyph < previous,
                TextDirection::Rtl => raw_glyph > previous,
            };
            if wrong_direction {
                return Err(ClusterMapError::NonMonotonic { text });
            }
        }
        previous = Some(raw_glyph);
        if starts.last().is_none_or(|(_, start)| *start != glyph) {
            starts.push((text, glyph));
        }
    }

    match direction {
        TextDirection::Ltr if starts.first().is_some_and(|(_, glyph)| *glyph != 0) => {
            return Err(ClusterMapError::UnmappedGlyph { glyph: 0 });
        }
        TextDirection::Rtl if starts.last().is_some_and(|(_, glyph)| *glyph != 0) => {
            return Err(ClusterMapError::UnmappedGlyph { glyph: 0 });
        }
        _ => {}
    }

    let mut clusters = Vec::with_capacity(starts.len());
    for index in 0..starts.len() {
        let (start_utf16, glyph_start) = starts[index];
        let end_utf16 = starts
            .get(index + 1)
            .map_or(cluster_map.len(), |(text, _)| *text);
        let glyph_end = match direction {
            TextDirection::Ltr => starts
                .get(index + 1)
                .map_or(glyph_count, |(_, glyph)| *glyph),
            TextDirection::Rtl => {
                if index == 0 {
                    glyph_count
                } else {
                    starts[index - 1].1
                }
            }
        };
        if glyph_end <= glyph_start {
            return Err(ClusterMapError::UnmappedGlyph { glyph: glyph_start });
        }
        clusters.push(GlyphCluster {
            start_utf16,
            end_utf16,
            glyph_start,
            glyph_end,
        });
    }

    Ok(clusters)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cluster(
        start_utf16: usize,
        end_utf16: usize,
        glyph_start: usize,
        glyph_end: usize,
    ) -> GlyphCluster {
        GlyphCluster {
            start_utf16,
            end_utf16,
            glyph_start,
            glyph_end,
        }
    }

    #[test]
    fn one_to_one_ltr() {
        assert_eq!(
            glyph_clusters(&[0, 1, 2], 3, TextDirection::Ltr),
            Ok(vec![
                cluster(0, 1, 0, 1),
                cluster(1, 2, 1, 2),
                cluster(2, 3, 2, 3),
            ])
        );
    }

    #[test]
    fn ligature_is_many_text_units_to_one_glyph() {
        assert_eq!(
            glyph_clusters(&[0, 0], 1, TextDirection::Ltr),
            Ok(vec![cluster(0, 2, 0, 1)])
        );
    }

    #[test]
    fn one_cluster_may_expand_to_several_glyphs() {
        assert_eq!(
            glyph_clusters(&[0], 2, TextDirection::Ltr),
            Ok(vec![cluster(0, 1, 0, 2)])
        );
        assert_eq!(
            glyph_clusters(&[0, 0], 3, TextDirection::Ltr),
            Ok(vec![cluster(0, 2, 0, 3)])
        );
        assert_eq!(
            glyph_clusters(&[0, 1, 3], 4, TextDirection::Ltr),
            Ok(vec![
                cluster(0, 1, 0, 1),
                cluster(1, 2, 1, 3),
                cluster(2, 3, 3, 4),
            ])
        );
    }

    #[test]
    fn rtl_clusters_keep_logical_text_order_and_reverse_glyph_ranges() {
        assert_eq!(
            glyph_clusters(&[2, 1, 0], 3, TextDirection::Rtl),
            Ok(vec![
                cluster(0, 1, 2, 3),
                cluster(1, 2, 1, 2),
                cluster(2, 3, 0, 1),
            ])
        );
        assert_eq!(
            glyph_clusters(&[2, 0], 3, TextDirection::Rtl),
            Ok(vec![cluster(0, 1, 2, 3), cluster(1, 2, 0, 2)])
        );
    }

    #[test]
    fn emptiness_must_agree_on_both_sides() {
        assert_eq!(glyph_clusters(&[], 0, TextDirection::Ltr), Ok(Vec::new()));
        assert_eq!(
            glyph_clusters(&[], 1, TextDirection::Ltr),
            Err(ClusterMapError::LengthMismatch {
                text_len: 0,
                glyph_count: 1,
            })
        );
        assert_eq!(
            glyph_clusters(&[0], 0, TextDirection::Ltr),
            Err(ClusterMapError::LengthMismatch {
                text_len: 1,
                glyph_count: 0,
            })
        );
    }

    #[test]
    fn invalid_indices_and_direction_are_named() {
        assert_eq!(
            glyph_clusters(&[0, 5], 2, TextDirection::Ltr),
            Err(ClusterMapError::GlyphIndexOutOfRange { text: 1, glyph: 5 })
        );
        assert_eq!(
            glyph_clusters(&[1, 0], 2, TextDirection::Ltr),
            Err(ClusterMapError::NonMonotonic { text: 1 })
        );
        assert_eq!(
            glyph_clusters(&[0, 1], 2, TextDirection::Rtl),
            Err(ClusterMapError::NonMonotonic { text: 1 })
        );
        assert_eq!(
            glyph_clusters(&[1, 1], 2, TextDirection::Ltr),
            Err(ClusterMapError::UnmappedGlyph { glyph: 0 })
        );
        assert_eq!(
            glyph_clusters(&[1, 1], 2, TextDirection::Rtl),
            Err(ClusterMapError::UnmappedGlyph { glyph: 0 })
        );
    }

    #[test]
    fn every_accepted_map_covers_every_glyph_once() {
        let cases: [(&[u16], usize, TextDirection); 6] = [
            (&[0, 1, 2], 3, TextDirection::Ltr),
            (&[0, 0], 1, TextDirection::Ltr),
            (&[0], 2, TextDirection::Ltr),
            (&[0, 1, 3], 4, TextDirection::Ltr),
            (&[2, 1, 0], 3, TextDirection::Rtl),
            (&[2, 0], 3, TextDirection::Rtl),
        ];
        for (map, glyph_count, direction) in cases {
            let clusters = glyph_clusters(map, glyph_count, direction)
                .unwrap_or_else(|error| panic!("{map:?}/{direction:?}: {error}"));
            let mut covered = vec![false; glyph_count];
            for cluster in clusters {
                for slot in &mut covered[cluster.glyph_start..cluster.glyph_end] {
                    assert!(!*slot, "{map:?} overlaps a glyph");
                    *slot = true;
                }
            }
            assert!(covered.into_iter().all(|value| value), "{map:?}");
        }
    }
}
