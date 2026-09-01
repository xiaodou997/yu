//! 把 DirectWrite 交回来的那几个平铺数组拼成一个 [`GlyphRun`]。
//!
//! **这一层也没有一行 DirectWrite 调用。** 它拿的是「已经从 COM 里取出来的
//! 普通数组」：`clusterMap`、字形 id、advance、offset。分开的理由与
//! [`crate::cluster`] 同一条——真正会错的是**翻译**，而翻译在哪台机器上都能
//! 跑；调用 COM 的那一步在开发机上根本不存在。
//!
//! 契约里有两条只在这一层看得出来：
//!
//! - **C8（基址）**：字形区间是 `source.start()` 加上局部偏移。产品链路上
//!   布局层永远传零基，所以「有没有把起点加回去」在真实调用里没有差别
//!   ——conformance 套件换第二个基址再问一遍来压它，而这里的用例直接用非零
//!   基址造。
//! - **UTF-16 → UTF-8**：DirectWrite 的索引单位是 code unit，`Glyph::source`
//!   要的是字节。代理对低位不是一个字节边界，落在那里必须失败而不是就近取整
//!   ——把一个字符劈成两半不报错。这一步交给 [`Utf16Map`]。

use yu_core::{Glyph, GlyphId, GlyphRun, Script, TextDirection, TextRange, TextStyle};
use yu_font::{FontFaceId, Utf16Map};

use crate::cluster::{ClusterMapError, glyph_spans};

/// 拼装失败的原因。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunAssemblyError {
    /// `clusterMap` 反不过来。原因见 [`ClusterMapError`]。
    ClusterMap(ClusterMapError),
    /// 字形 id / advance / offset 三个数组长度对不上。
    ArrayLengthMismatch {
        glyphs: usize,
        advances: usize,
        offsets: usize,
    },
    /// UTF-16 区间落在代理对中间，或者越出了这段文本。
    NotAUtf16Boundary { start: usize, end: usize },
    /// advance 或 offset 不是有限值。
    NonFiniteMetric,
}

impl std::fmt::Display for RunAssemblyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ClusterMap(error) => error.fmt(formatter),
            Self::ArrayLengthMismatch {
                glyphs,
                advances,
                offsets,
            } => write!(
                formatter,
                "glyph arrays disagree: {glyphs} ids, {advances} advances, {offsets} offsets"
            ),
            Self::NotAUtf16Boundary { start, end } => write!(
                formatter,
                "UTF-16 range {start}..{end} is not a byte boundary of this run"
            ),
            Self::NonFiniteMetric => formatter.write_str("glyph advance or offset is not finite"),
        }
    }
}

impl std::error::Error for RunAssemblyError {}

impl From<ClusterMapError> for RunAssemblyError {
    fn from(error: ClusterMapError) -> Self {
        Self::ClusterMap(error)
    }
}

/// DirectWrite 一次 `GetGlyphs` + `GetGlyphPlacements` 之后拿到的那一份平铺
/// 结果。字段就是 COM 那几个 out 参数，顺序也一样。
#[derive(Clone, Copy, Debug)]
pub struct ShapedArrays<'a> {
    /// `clusterMap[i]`：文本第 i 个 code unit 所属那一簇的首字形下标。
    pub cluster_map: &'a [u16],
    pub glyph_ids: &'a [u16],
    pub advances: &'a [f32],
    /// `(x, y)`，DirectWrite 的 `DWRITE_GLYPH_OFFSET`。
    pub offsets: &'a [(f32, f32)],
}

/// 把一次 shaping 的结果拼成一个 run。
///
/// `text` 是这个 run 的文本，`source` 是它整体在调用方坐标系里的位置。
pub fn assemble_run(
    face: FontFaceId,
    text: &str,
    source: TextRange,
    style: TextStyle,
    direction: TextDirection,
    script: Script,
    arrays: ShapedArrays<'_>,
) -> Result<GlyphRun, RunAssemblyError> {
    let glyph_count = arrays.glyph_ids.len();
    if arrays.advances.len() != glyph_count || arrays.offsets.len() != glyph_count {
        return Err(RunAssemblyError::ArrayLengthMismatch {
            glyphs: glyph_count,
            advances: arrays.advances.len(),
            offsets: arrays.offsets.len(),
        });
    }

    let spans = glyph_spans(arrays.cluster_map, glyph_count)?;
    let utf16 = Utf16Map::new(text);
    let mut glyphs = Vec::with_capacity(glyph_count);
    for (index, span) in spans.iter().enumerate() {
        let range = utf16
            .range(span.start_utf16, span.end_utf16, source)
            .ok_or(RunAssemblyError::NotAUtf16Boundary {
                start: span.start_utf16,
                end: span.end_utf16,
            })?;
        let advance = arrays.advances[index];
        let (x_offset, y_offset) = arrays.offsets[index];
        if !advance.is_finite() || !x_offset.is_finite() || !y_offset.is_finite() {
            return Err(RunAssemblyError::NonFiniteMetric);
        }
        glyphs.push(Glyph::new(
            GlyphId::from_raw(u32::from(arrays.glyph_ids[index])),
            range,
            advance,
            x_offset,
            y_offset,
        ));
    }
    Ok(GlyphRun::new(
        face, source, style, direction, script, glyphs,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use yu_core::ByteOffset;

    fn source(start: u64, len: u64) -> TextRange {
        TextRange::new(ByteOffset::new(start), ByteOffset::new(start + len)).expect("range")
    }

    fn arrays<'a>(
        cluster_map: &'a [u16],
        glyph_ids: &'a [u16],
        advances: &'a [f32],
        offsets: &'a [(f32, f32)],
    ) -> ShapedArrays<'a> {
        ShapedArrays {
            cluster_map,
            glyph_ids,
            advances,
            offsets,
        }
    }

    /// **C8：区间要以 `source.start()` 为基址，不是零基。**
    ///
    /// 布局层今天永远传零基，所以漏掉这一步在产品链路上一点差别都没有
    /// ——这条用例直接用非零基址造，是唯一能把它压住的形状。
    #[test]
    fn glyph_ranges_are_offset_by_the_requested_base() {
        let run = assemble_run(
            FontFaceId::from_raw(3),
            "abc",
            source(97, 3),
            TextStyle::Plain,
            TextDirection::Ltr,
            Script::Latin,
            arrays(
                &[0, 1, 2],
                &[10, 11, 12],
                &[5.0, 5.0, 5.0],
                &[(0.0, 0.0); 3],
            ),
        )
        .expect("run");

        let ranges: Vec<(u64, u64)> = run
            .glyphs()
            .iter()
            .map(|glyph| (glyph.source().start().get(), glyph.source().end().get()))
            .collect();
        assert_eq!(ranges, vec![(97, 98), (98, 99), (99, 100)]);
        assert_eq!(run.advance(), 15.0);
    }

    /// 多字节字符：一个字形覆盖三个 UTF-8 字节但只有一个 code unit。
    #[test]
    fn multibyte_characters_map_through_code_units_not_bytes() {
        let run = assemble_run(
            FontFaceId::from_raw(1),
            "羽a",
            source(0, 4),
            TextStyle::Plain,
            TextDirection::Ltr,
            Script::Han,
            arrays(&[0, 1], &[7, 8], &[12.0, 6.0], &[(0.0, 0.0); 2]),
        )
        .expect("run");
        let ranges: Vec<(u64, u64)> = run
            .glyphs()
            .iter()
            .map(|glyph| (glyph.source().start().get(), glyph.source().end().get()))
            .collect();
        assert_eq!(ranges, vec![(0, 3), (3, 4)]);
    }

    /// 代理对：一个 emoji 是两个 code unit、一个字形。
    ///
    /// clusterMap 是 `[0, 0]`——两个 code unit 同属一簇，与连字同形。
    #[test]
    fn a_surrogate_pair_is_one_cluster_of_two_code_units() {
        let run = assemble_run(
            FontFaceId::from_raw(1),
            "🙂",
            source(0, 4),
            TextStyle::Plain,
            TextDirection::Ltr,
            Script::Common,
            arrays(&[0, 0], &[42], &[16.0], &[(0.0, 0.0)]),
        )
        .expect("run");
        assert_eq!(run.glyphs().len(), 1);
        assert_eq!(run.glyphs()[0].source().start().get(), 0);
        assert_eq!(run.glyphs()[0].source().end().get(), 4);
    }

    /// 三个数组长度对不上要立刻说出来，而不是按最短的那个截断。
    ///
    /// 截断不报错——它产出一个「合法」的 run，只是少了几个字形，而
    /// `yu-layout` 的 tiling 门会把它报成「run 没铺满」，排查方向指向布局层。
    #[test]
    fn disagreeing_array_lengths_are_named_here_not_downstream() {
        assert_eq!(
            assemble_run(
                FontFaceId::from_raw(1),
                "ab",
                source(0, 2),
                TextStyle::Plain,
                TextDirection::Ltr,
                Script::Latin,
                arrays(&[0, 1], &[1, 2], &[5.0], &[(0.0, 0.0); 2]),
            ),
            Err(RunAssemblyError::ArrayLengthMismatch {
                glyphs: 2,
                advances: 1,
                offsets: 2,
            })
        );
    }

    /// 一簇多形从 [`crate::cluster`] 一路传上来，不在这里被吞掉。
    #[test]
    fn a_multi_glyph_cluster_stays_an_error_at_this_layer_too() {
        assert!(matches!(
            assemble_run(
                FontFaceId::from_raw(1),
                "a",
                source(0, 1),
                TextStyle::Plain,
                TextDirection::Ltr,
                Script::Devanagari,
                arrays(&[0], &[1, 2], &[5.0, 5.0], &[(0.0, 0.0); 2]),
            ),
            Err(RunAssemblyError::ClusterMap(
                ClusterMapError::MultiGlyphCluster { .. }
            ))
        ));
    }

    /// 非有限的度量不许进 run——它一路飘到布局里会变成 NaN 宽度。
    #[test]
    fn non_finite_metrics_are_rejected() {
        assert_eq!(
            assemble_run(
                FontFaceId::from_raw(1),
                "a",
                source(0, 1),
                TextStyle::Plain,
                TextDirection::Ltr,
                Script::Latin,
                arrays(&[0], &[1], &[f32::NAN], &[(0.0, 0.0)]),
            ),
            Err(RunAssemblyError::NonFiniteMetric)
        );
    }
}
