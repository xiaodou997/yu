//! `ShapingProvider` 契约的一致性套件。
//!
//! 契约条文写在 [`crate::ShapingProvider`] 的文档上，这里是它可执行的那一份。
//! **它是普通 API 而不是 `#[cfg(test)]`**：六个实现散在六个 crate 里
//! （`yu-core` / `yu-font` / `yu-layout` / `yu-editor` / `yu-bench` /
//! `yu-font-macos`），只有 0 层这一个 crate 是它们全都已经依赖的。放进
//! `yu-font` 会逼 `yu-editor` 与 `yu-bench` 各加一条产品依赖边——为了测试
//! 脚手架去改依赖图，方向是反的。
//!
//! **这套东西检查的是「返回 `Ok` 时内容必须满足什么」，不检查覆盖面。**
//! 一个只支持拉丁文的后端仍然是合规后端，它对别的输入返回 `Err`。覆盖面是
//! 产品决定，不是 seam 契约。为了不让「永远返回 `Err`」也算通过，语料分成
//! 两档：[`Coverage::Required`] 的必须 shape 得出来，[`Coverage::Optional`]
//! 的可以拒。
//!
//! **不要指望语料压住「一簇多形」。** S7 第七刀的 spike 在本机拿 35 个语料
//! 跑真的 `CoreTextShaper`，一次都没让两个字形拿到同一个起点——会出现它的
//! 脚本全部先被 `CTRunStatus` 拒了。压那一条只能靠故意违约的 mock，本模块
//! 自己的用例就是这么写的。

use core::fmt;

use crate::style::TextStyle;
use crate::{ByteOffset, ShapingProvider, TextRange};

/// 一条语料对后端的最低要求。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Coverage {
    /// 必须 shape 成功。拒绝它就是不合规。
    Required,
    /// 可以返回 `Err`（后端做不到）。返回 `Ok` 就要满足全部条款。
    Optional,
}

/// 一条语料。
#[derive(Clone, Copy, Debug)]
pub struct ConformanceCase {
    pub name: &'static str,
    pub text: &'static str,
    pub coverage: Coverage,
}

/// 被违反的那一条。条文见 [`crate::ShapingProvider`]。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Clause {
    /// C1 `shaped.source()` 必须等于请求的 range。
    SourceIdentity,
    /// C2 run 的 source 必须首尾相接、不重叠地铺满请求的 range。
    RunsTileRequest,
    /// C3 字形的 source 必须首尾相接、不重叠地铺满它所在的 run。
    GlyphsTileRun,
    /// C4 字形的 source 不得为空。
    EmptyGlyphRange,
    /// C5 字形的 source 两端必须落在 UTF-8 字符边界上。
    CharBoundary,
    /// C6 advance 必须有限且非负，偏移必须有限。
    Metrics,
    /// C7 run 的 style 必须是请求的那个。
    StylePassthrough,
    /// C8 字形的 source 是请求 range 的起点加上局部偏移。
    BaseOffset,
    /// C9 同一次请求重复调用必须给同一个答案。
    Determinism,
    /// C10 `shape_scaled(.., 1.0)` 必须等于 `shape`。
    UnitScale,
    /// `Coverage::Required` 的语料被拒了。
    RequiredCaseRejected,
}

impl Clause {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SourceIdentity => "C1 source identity",
            Self::RunsTileRequest => "C2 runs tile the request",
            Self::GlyphsTileRun => "C3 glyphs tile the run",
            Self::EmptyGlyphRange => "C4 no empty glyph range",
            Self::CharBoundary => "C5 char boundary",
            Self::Metrics => "C6 finite metrics",
            Self::StylePassthrough => "C7 style passthrough",
            Self::BaseOffset => "C8 base offset",
            Self::Determinism => "C9 determinism",
            Self::UnitScale => "C10 unit scale",
            Self::RequiredCaseRejected => "required case rejected",
        }
    }
}

/// 一处违约。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Violation {
    pub case: &'static str,
    pub style: TextStyle,
    pub clause: Clause,
    pub detail: String,
}

impl fmt::Display for Violation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "[{}] {:?} {}: {}",
            self.case,
            self.style,
            self.clause.as_str(),
            self.detail
        )
    }
}

const fn required(name: &'static str, text: &'static str) -> ConformanceCase {
    ConformanceCase {
        name,
        text,
        coverage: Coverage::Required,
    }
}

const fn optional(name: &'static str, text: &'static str) -> ConformanceCase {
    ConformanceCase {
        name,
        text,
        coverage: Coverage::Optional,
    }
}

const CASES: &[ConformanceCase] = &[
    required("empty", ""),
    required("single-ascii", "a"),
    required("ascii-word", "hello"),
    required("ascii-spaces", "ab cd  ef"),
    required("ligature-candidate", "fi"),
    required("combining", "e\u{0301}"),
    required("precomposed", "\u{1ec7}"),
    required("cjk", "\u{4e2d}\u{6587}"),
    required("mixed-latin-cjk", "a\u{4e2d}b"),
    required("emoji", "\u{1f600}"),
    required("emoji-zwj", "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}"),
    required("regional-indicator", "\u{1f1e8}\u{1f1f3}"),
    required("nbsp", "a\u{a0}b"),
    required("tab", "a\tb"),
    optional("hard-break", "a\nb"),
    optional("hebrew", "\u{05d0}\u{05b8}"),
    optional("arabic", "\u{0644}\u{0627}"),
    optional("devanagari", "\u{0915}\u{094d}\u{0937}\u{093f}"),
    optional("tamil", "\u{0b95}\u{0bca}"),
    optional("thai-sara-am", "\u{0e01}\u{0e33}"),
    optional("myanmar", "\u{1000}\u{1031}"),
    optional("tibetan", "\u{0f40}\u{0f7c}"),
    optional("khmer", "\u{1780}\u{17d2}\u{1783}"),
    optional("hangul-jamo", "\u{1100}\u{1161}\u{11a8}"),
];

/// 契约语料。
///
/// `Required` 那一档刻意只放「一簇一形一定成立」的东西：拉丁、组合记号、
/// CJK、emoji 序列、空白。`Optional` 那一档放的是 S7 第七刀 spike 实测被
/// CoreText 拒掉的脚本——它们留在这里是为了「一旦哪天 shape 得出来，条款
/// 立刻开始管它」，不是为了要求谁支持。
#[must_use]
pub fn cases() -> &'static [ConformanceCase] {
    CASES
}

/// 契约要在这几种 style 上都成立——style 是请求的一部分，后端换字重换斜体
/// 时 fallback 会走到别的 face 上。
const STYLES: [TextStyle; 4] = [
    TextStyle::Plain,
    TextStyle::Strong,
    TextStyle::Emphasis,
    TextStyle::Code,
];

/// 第二次请求用的基址。**不是 0**：`ShapingProvider` 的 range 参数今天由
/// 布局层以零基传入（`yu-layout` 的 `local_range`），于是「后端有没有把
/// 起点加回去」在产品链路上永远看不出来。C8 就是压这一条的。
const ALTERNATE_BASE: u64 = 97;

/// 把一个后端过一遍契约。返回空表示合规。
#[must_use]
pub fn audit<P: ShapingProvider>(provider: &P) -> Vec<Violation> {
    let mut violations = Vec::new();
    for case in cases() {
        for style in STYLES {
            audit_case(provider, *case, style, &mut violations);
        }
    }
    violations
}

fn range_at(base: u64, len: usize) -> Option<TextRange> {
    let len = u64::try_from(len).ok()?;
    TextRange::new(
        ByteOffset::new(base),
        ByteOffset::new(base.checked_add(len)?),
    )
}

fn audit_case<P: ShapingProvider>(
    provider: &P,
    case: ConformanceCase,
    style: TextStyle,
    violations: &mut Vec<Violation>,
) {
    let mut push = |clause: Clause, detail: String| {
        violations.push(Violation {
            case: case.name,
            style,
            clause,
            detail,
        });
    };
    let Some(source) = range_at(0, case.text.len()) else {
        push(
            Clause::SourceIdentity,
            "语料长度放不进 TextRange".to_owned(),
        );
        return;
    };
    let shaped = match provider.shape(case.text, source, style) {
        Ok(shaped) => shaped,
        Err(error) => {
            if case.coverage == Coverage::Required {
                push(Clause::RequiredCaseRejected, error.to_string());
            }
            return;
        }
    };

    // C1
    if shaped.source() != source {
        push(
            Clause::SourceIdentity,
            format!("要的是 {source:?}，回的是 {:?}", shaped.source()),
        );
    }

    // C2 / C3 / C4 / C5 / C6 / C7
    let mut cursor = source.start().get();
    for (index, run) in shaped.runs().iter().enumerate() {
        if run.source().start().get() != cursor {
            push(
                Clause::RunsTileRequest,
                format!(
                    "run#{index} 从 {} 开始，上一段停在 {cursor}",
                    run.source().start().get()
                ),
            );
        }
        if run.style() != style {
            push(
                Clause::StylePassthrough,
                format!(
                    "run#{index} 的 style 是 {:?}，请求的是 {style:?}",
                    run.style()
                ),
            );
        }
        let mut glyph_cursor = run.source().start().get();
        for (glyph_index, glyph) in run.glyphs().iter().enumerate() {
            let from = glyph.source().start().get();
            let to = glyph.source().end().get();
            if from != glyph_cursor {
                push(
                    Clause::GlyphsTileRun,
                    format!(
                        "run#{index} glyph#{glyph_index} 从 {from} 开始，上一个停在 {glyph_cursor}"
                    ),
                );
            }
            if to > run.source().end().get() {
                push(
                    Clause::GlyphsTileRun,
                    format!(
                        "run#{index} glyph#{glyph_index} 到 {to}，run 只到 {}",
                        run.source().end().get()
                    ),
                );
            }
            if to == from {
                push(
                    Clause::EmptyGlyphRange,
                    format!("run#{index} glyph#{glyph_index} 的 source 是空的（{from}..{to}）"),
                );
            }
            for (label, offset) in [("start", from), ("end", to)] {
                let local = offset.checked_sub(source.start().get());
                let on_boundary = local
                    .and_then(|local| usize::try_from(local).ok())
                    .is_some_and(|local| case.text.is_char_boundary(local));
                if !on_boundary {
                    push(
                        Clause::CharBoundary,
                        format!(
                            "run#{index} glyph#{glyph_index} 的 {label}={offset} 不在字符边界上"
                        ),
                    );
                }
            }
            if !glyph.advance().is_finite() || glyph.advance() < 0.0 {
                push(
                    Clause::Metrics,
                    format!(
                        "run#{index} glyph#{glyph_index} 的 advance 是 {}",
                        glyph.advance()
                    ),
                );
            }
            if !glyph.x_offset().is_finite() || !glyph.y_offset().is_finite() {
                push(
                    Clause::Metrics,
                    format!(
                        "run#{index} glyph#{glyph_index} 的偏移是 ({}, {})",
                        glyph.x_offset(),
                        glyph.y_offset()
                    ),
                );
            }
            glyph_cursor = to;
        }
        if glyph_cursor != run.source().end().get() {
            push(
                Clause::GlyphsTileRun,
                format!(
                    "run#{index} 的字形停在 {glyph_cursor}，run 到 {}",
                    run.source().end().get()
                ),
            );
        }
        cursor = run.source().end().get();
    }
    if cursor != source.end().get() {
        push(
            Clause::RunsTileRequest,
            format!("run 停在 {cursor}，请求到 {}", source.end().get()),
        );
    }

    // C9
    match provider.shape(case.text, source, style) {
        Ok(again) if again == shaped => {}
        Ok(_) => push(
            Clause::Determinism,
            "同一次请求两次给了不同的答案".to_owned(),
        ),
        Err(error) => push(
            Clause::Determinism,
            format!("第一次成功第二次失败：{error}"),
        ),
    }

    // C10
    match provider.shape_scaled(case.text, source, style, 1.0) {
        Ok(unit) if unit == shaped => {}
        Ok(_) => push(
            Clause::UnitScale,
            "shape_scaled(1.0) 与 shape 给了不同的答案".to_owned(),
        ),
        Err(error) => push(
            Clause::UnitScale,
            format!("shape_scaled(1.0) 失败：{error}"),
        ),
    }

    // C8
    let Some(moved_source) = range_at(ALTERNATE_BASE, case.text.len()) else {
        return;
    };
    match provider.shape(case.text, moved_source, style) {
        Err(error) => push(
            Clause::BaseOffset,
            format!("换一个基址就 shape 不出来了：{error}"),
        ),
        Ok(moved) => {
            if moved.source() != moved_source {
                push(
                    Clause::SourceIdentity,
                    format!("要的是 {moved_source:?}，回的是 {:?}", moved.source()),
                );
            }
            let flatten = |text: &crate::ShapedText| -> Vec<(u64, u64)> {
                text.runs()
                    .iter()
                    .flat_map(|run| run.glyphs())
                    .map(|glyph| (glyph.source().start().get(), glyph.source().end().get()))
                    .collect()
            };
            let base = flatten(&shaped);
            let shifted = flatten(&moved);
            if base.len() != shifted.len() {
                push(
                    Clause::BaseOffset,
                    format!("换基址后字形数从 {} 变成 {}", base.len(), shifted.len()),
                );
            } else if let Some((index, (want, got))) = base
                .iter()
                .zip(shifted.iter())
                .map(|(a, b)| ((a.0 + ALTERNATE_BASE, a.1 + ALTERNATE_BASE), (b.0, b.1)))
                .enumerate()
                .find(|(_, (want, got))| want != got)
            {
                push(
                    Clause::BaseOffset,
                    format!("glyph#{index} 应该是 {want:?}，实际是 {got:?}"),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shaping::{FontFaceId, Glyph, GlyphId, GlyphRun, Script, ShapedText, TextDirection};

    /// 一 `char` 一形、老实把基址加回去的合规后端。
    struct Conforming;

    impl ShapingProvider for Conforming {
        type Error = String;

        fn shape(
            &self,
            text: &str,
            source: TextRange,
            style: TextStyle,
        ) -> Result<ShapedText, Self::Error> {
            if text.is_empty() {
                return Ok(ShapedText::new(source, Vec::new()));
            }
            let glyphs = text
                .char_indices()
                .map(|(start, character)| {
                    let from = source.start().get() + start as u64;
                    let to = from + character.len_utf8() as u64;
                    Ok(Glyph::new(
                        GlyphId::from_raw(1),
                        TextRange::new(ByteOffset::new(from), ByteOffset::new(to)).ok_or("有序")?,
                        1.0,
                        0.0,
                        0.0,
                    ))
                })
                .collect::<Result<Vec<_>, String>>()?;
            Ok(ShapedText::new(
                source,
                vec![GlyphRun::new(
                    FontFaceId::from_raw(0),
                    source,
                    style,
                    TextDirection::Ltr,
                    Script::Latin,
                    glyphs,
                )],
            ))
        }
    }

    /// 按一张「每个字形的局部区间」表产出，用来造各种违约。
    struct Scripted(fn(&str) -> Vec<(u64, u64)>);

    impl ShapingProvider for Scripted {
        type Error = String;

        fn shape(
            &self,
            text: &str,
            source: TextRange,
            style: TextStyle,
        ) -> Result<ShapedText, Self::Error> {
            let glyphs = (self.0)(text)
                .into_iter()
                .map(|(from, to)| {
                    Ok(Glyph::new(
                        GlyphId::from_raw(1),
                        TextRange::new(
                            ByteOffset::new(source.start().get() + from),
                            ByteOffset::new(source.start().get() + to),
                        )
                        .ok_or("有序")?,
                        1.0,
                        0.0,
                        0.0,
                    ))
                })
                .collect::<Result<Vec<_>, String>>()?;
            Ok(ShapedText::new(
                source,
                vec![GlyphRun::new(
                    FontFaceId::from_raw(0),
                    source,
                    style,
                    TextDirection::Ltr,
                    Script::Latin,
                    glyphs,
                )],
            ))
        }
    }

    fn clauses(violations: &[Violation]) -> Vec<Clause> {
        let mut clauses: Vec<Clause> = violations
            .iter()
            .map(|violation| violation.clause)
            .collect();
        clauses.dedup();
        clauses.sort_by_key(|clause| clause.as_str());
        clauses.dedup();
        clauses
    }

    #[test]
    fn a_conforming_backend_passes() {
        let violations = audit(&Conforming);
        assert!(violations.is_empty(), "{violations:#?}");
    }

    /// 空区间是 DirectWrite 那条 `empty-at-end` 反转策略的产物，而
    /// `yu-layout` 那道 tiling 门对它恒不成立——套件必须自己抓住它。
    #[test]
    fn an_empty_glyph_range_is_a_violation() {
        let violations = audit(&Scripted(|text| {
            let len = text.len() as u64;
            if len == 0 {
                return Vec::new();
            }
            vec![(0, len), (len, len)]
        }));
        assert!(
            clauses(&violations).contains(&Clause::EmptyGlyphRange),
            "{violations:#?}"
        );
    }

    /// 一簇多形照 HarfBuzz 的做法反转出来就是「两个字形同一个起点」。
    #[test]
    fn a_repeated_cluster_start_is_a_violation() {
        let violations = audit(&Scripted(|text| {
            let len = text.len() as u64;
            if len == 0 {
                return Vec::new();
            }
            vec![(0, len), (0, len)]
        }));
        assert!(
            clauses(&violations).contains(&Clause::GlyphsTileRun),
            "{violations:#?}"
        );
    }

    /// 忽略 `source` 的起点——`yu-layout` 今天总是传零基，所以产品链路上
    /// 这一条永远看不出来。
    #[test]
    fn ignoring_the_requested_base_offset_is_a_violation() {
        struct ZeroBased;
        impl ShapingProvider for ZeroBased {
            type Error = String;
            fn shape(
                &self,
                text: &str,
                source: TextRange,
                style: TextStyle,
            ) -> Result<ShapedText, Self::Error> {
                if text.is_empty() {
                    return Ok(ShapedText::new(source, Vec::new()));
                }
                let glyphs = text
                    .char_indices()
                    .map(|(start, character)| {
                        Ok(Glyph::new(
                            GlyphId::from_raw(1),
                            TextRange::new(
                                ByteOffset::new(start as u64),
                                ByteOffset::new((start + character.len_utf8()) as u64),
                            )
                            .ok_or("有序")?,
                            1.0,
                            0.0,
                            0.0,
                        ))
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                Ok(ShapedText::new(
                    source,
                    vec![GlyphRun::new(
                        FontFaceId::from_raw(0),
                        source,
                        style,
                        TextDirection::Ltr,
                        Script::Latin,
                        glyphs,
                    )],
                ))
            }
        }
        let violations = audit(&ZeroBased);
        assert!(
            clauses(&violations).contains(&Clause::BaseOffset),
            "{violations:#?}"
        );
    }

    /// 把 `Required` 的语料拒掉不算合规，否则「永远返回 Err」就通过了。
    #[test]
    fn rejecting_a_required_case_is_a_violation() {
        struct AlwaysFails;
        impl ShapingProvider for AlwaysFails {
            type Error = String;
            fn shape(
                &self,
                _text: &str,
                _source: TextRange,
                _style: TextStyle,
            ) -> Result<ShapedText, Self::Error> {
                Err("做不了".to_owned())
            }
        }
        let violations = audit(&AlwaysFails);
        assert_eq!(clauses(&violations), vec![Clause::RequiredCaseRejected]);
    }

    /// 只在 `Optional` 语料上失败仍然合规——否则一个只支持拉丁与 CJK 的
    /// 后端会被这套东西判成不合规，而覆盖面本来就不是 seam 契约的内容。
    #[test]
    fn rejecting_only_optional_cases_still_conforms() {
        struct OptionalHostile;
        impl ShapingProvider for OptionalHostile {
            type Error = String;
            fn shape(
                &self,
                text: &str,
                source: TextRange,
                style: TextStyle,
            ) -> Result<ShapedText, Self::Error> {
                let optional = cases()
                    .iter()
                    .any(|case| case.coverage == Coverage::Optional && case.text == text);
                if optional {
                    return Err("这个后端做不了".to_owned());
                }
                Conforming.shape(text, source, style)
            }
        }
        assert!(audit(&OptionalHostile).is_empty());
    }

    #[test]
    fn styles_are_passed_through() {
        struct AlwaysPlain;
        impl ShapingProvider for AlwaysPlain {
            type Error = String;
            fn shape(
                &self,
                text: &str,
                source: TextRange,
                _style: TextStyle,
            ) -> Result<ShapedText, Self::Error> {
                Conforming.shape(text, source, TextStyle::Plain)
            }
        }
        let violations = audit(&AlwaysPlain);
        assert!(
            clauses(&violations).contains(&Clause::StylePassthrough),
            "{violations:#?}"
        );
    }
}
