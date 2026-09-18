//! CoreText owns line breaking and glyph positioning for an entire paragraph.
//! Only owned geometry leaves this operation; CTLine/CTRun never cross queues.

#![cfg(target_os = "macos")]

use super::*;
#[cfg(target_os = "macos")]
use objc2_core_foundation::CFMutableAttributedString;
#[cfg(target_os = "macos")]
use objc2_core_text::{
    CTParagraphStyle, CTParagraphStyleSetting, CTParagraphStyleSpecifier, CTRunDelegate,
    CTRunDelegateCallbacks, CTTypesetter, CTWritingDirection, kCTParagraphStyleAttributeName,
    kCTRunDelegateAttributeName,
};
use std::ffi::c_void;
use yu_core::{
    ParagraphCluster, ParagraphGlyph, ParagraphInput, ParagraphLayout, ParagraphLayoutProvider,
    ParagraphLine, ParagraphObjectBox, StyleId, TextAttrs, VisualOffset, VisualRange,
};

struct Unit {
    range: VisualRange,
    start: usize,
    end: usize,
    style: StyleId,
    attrs: TextAttrs,
    object: Option<usize>,
    spacer: Option<f32>,
    line_break: bool,
}

// Native UTF-16 range, directional run edges, font metrics, and the
// first/last visible unit boundaries after ignoring synthetic joiners.
type NativeRunEdge = (usize, usize, f64, f64, f32, f32, usize, usize);

struct InlineAccumulator {
    range: VisualRange,
    left: f32,
    right: f32,
    text_left: f32,
    text_right: f32,
    ascent: f32,
    descent: f32,
    inset_y: f32,
    has_text: bool,
}

struct ObjectMetrics {
    width: f64,
    ascent: f64,
    descent: f64,
}
unsafe extern "C-unwind" fn object_drop(pointer: NonNull<c_void>) {
    // SAFETY: each delegate owns exactly one allocation made below.
    drop(unsafe { Box::from_raw(pointer.as_ptr().cast::<ObjectMetrics>()) });
}
unsafe extern "C-unwind" fn object_width(pointer: NonNull<c_void>) -> f64 {
    unsafe { pointer.cast::<ObjectMetrics>().as_ref().width }
}
unsafe extern "C-unwind" fn object_ascent(pointer: NonNull<c_void>) -> f64 {
    unsafe { pointer.cast::<ObjectMetrics>().as_ref().ascent }
}
unsafe extern "C-unwind" fn object_descent(pointer: NonNull<c_void>) -> f64 {
    unsafe { pointer.cast::<ObjectMetrics>().as_ref().descent }
}

fn visual_range(start: usize, end: usize) -> Result<VisualRange, CoreTextShapeError> {
    VisualRange::new(
        VisualOffset::new(start as u64),
        VisualOffset::new(end as u64),
    )
    .ok_or(CoreTextShapeError::InvalidCoreTextRange)
}

impl ParagraphLayoutProvider for CoreTextShaper {
    fn layout(&self, input: &ParagraphInput<'_>) -> Result<ParagraphLayout, String> {
        layout(self, input).map_err(|error| error.to_string())
    }
}

// The fixed reference's normal prose wrapping keeps a solidus with a
// following Latin letter (WebKit issue 83934). This changes only native
// break opportunities, never source text or caret offsets.
fn joined_solidus_boundary(text: &str, offset: usize) -> bool {
    offset
        .checked_sub(1)
        .and_then(|before| text.as_bytes().get(before))
        == Some(&b'/')
        && text
            .get(offset..)
            .and_then(|tail| tail.chars().next())
            .is_some_and(|after| after <= '\u{ff}' && after.is_alphabetic())
}

fn layout(
    shaper: &CoreTextShaper,
    input: &ParagraphInput<'_>,
) -> Result<ParagraphLayout, CoreTextShapeError> {
    if !input.width.is_finite()
        || input.width <= 0.0
        || !input.indent.is_finite()
        || input.indent < 0.0
        || !input.line_height.is_finite()
        || input.line_height <= 0.0
    {
        return Err(CoreTextShapeError::InvalidViewportMetrics);
    }
    let mut text = String::with_capacity(input.text.len() + input.objects.len() * 3);
    let mut units = Vec::new();
    let mut utf16 = 0;
    let mut next_object = 0;
    let mut next_run = 0;
    // Preserve source break opportunities across synthetic decoration edges.
    // CoreText still chooses every line break and performs bidi ordering.
    let source_breaks: std::collections::BTreeSet<usize> =
        unicode_linebreak::linebreaks(input.text)
            .map(|(offset, _)| offset)
            .filter(|offset| !joined_solidus_boundary(input.text, *offset))
            .collect();
    let mut spans = std::collections::BTreeMap::new();
    for run in &input.runs {
        if let Some(id) = run
            .attrs
            .inline_box_id()
            .filter(|_| run.attrs.inline_inset() > 0.0)
        {
            let entry = spans.entry(id).or_insert((
                run.range.start(),
                run.range.end(),
                run.style,
                run.attrs,
            ));
            entry.1 = run.range.end();
        }
    }
    let mut edges: std::collections::BTreeMap<u64, Vec<(bool, StyleId, TextAttrs)>> =
        std::collections::BTreeMap::new();
    for (_, (start, end, style, attrs)) in spans {
        edges
            .entry(start.get())
            .or_default()
            .push((true, style, attrs));
        edges
            .entry(end.get())
            .or_default()
            .push((false, style, attrs));
    }
    for (byte, grapheme) in input
        .text
        .grapheme_indices(true)
        .chain(std::iter::once((input.text.len(), "")))
    {
        while next_run + 1 < input.runs.len()
            && input.runs[next_run].range.end().get() <= byte as u64
        {
            next_run += 1;
        }
        let (style, attrs) = input
            .runs
            .get(next_run)
            .map_or((StyleId(0), TextAttrs::default()), |run| {
                (run.style, run.attrs)
            });
        if let Some(mut pending) = edges.remove(&(byte as u64)) {
            pending.sort_by_key(|edge| edge.0); // close one box before opening the next
            for (before, edge_style, edge_attrs) in pending {
                // A word joiner binds each invisible edge to its content. These
                // UTF-16 units have no source characters and never become glyphs.
                let mut chars = if before {
                    vec!['\u{fffc}', '\u{2060}']
                } else {
                    vec!['\u{2060}', '\u{fffc}']
                };
                if byte > 0 && byte < input.text.len() && !source_breaks.contains(&byte) {
                    if before {
                        chars.insert(0, '\u{2060}');
                    } else {
                        chars.push('\u{2060}');
                    }
                }
                for ch in chars {
                    text.push(ch);
                    units.push(Unit {
                        range: VisualRange::empty(VisualOffset::new(byte as u64)),
                        start: utf16,
                        end: utf16 + 1,
                        style: edge_style,
                        attrs: edge_attrs,
                        object: None,
                        spacer: Some(if ch == '\u{fffc}' {
                            edge_attrs.inline_inset()
                        } else {
                            0.0
                        }),
                        line_break: false,
                    });
                    utf16 += 1;
                }
            }
        }
        if joined_solidus_boundary(input.text, byte) {
            text.push('\u{2060}');
            units.push(Unit {
                range: VisualRange::empty(VisualOffset::new(byte as u64)),
                start: utf16,
                end: utf16 + 1,
                style,
                attrs,
                object: None,
                spacer: Some(0.0),
                line_break: false,
            });
            utf16 += 1;
        }
        while let Some(object) = input.objects.get(next_object)
            && object.offset.get() == byte as u64
        {
            text.push('\u{fffc}');
            units.push(Unit {
                range: VisualRange::empty(object.offset),
                start: utf16,
                end: utf16 + 1,
                style,
                attrs,
                object: Some(next_object),
                spacer: None,
                line_break: false,
            });
            utf16 += 1;
            next_object += 1;
        }
        if !grapheme.is_empty() {
            let count = grapheme.encode_utf16().count();
            text.push_str(grapheme);
            units.push(Unit {
                range: visual_range(byte, byte + grapheme.len())?,
                start: utf16,
                end: utf16 + count,
                style,
                attrs,
                object: None,
                spacer: None,
                line_break: grapheme.chars().all(|c| {
                    matches!(
                        c,
                        '\n' | '\r' | '\u{b}' | '\u{c}' | '\u{85}' | '\u{2028}' | '\u{2029}'
                    )
                }),
            });
            utf16 += count;
        }
    }
    if next_object != input.objects.len() {
        return Err(CoreTextShapeError::InvalidCoreTextRange);
    }
    if units.is_empty() {
        let font = create_core_text_font(&shaper.request, shaper.font_source)?;
        let ascent = unsafe { font.ascent() } as f32;
        let descent = unsafe { font.descent() } as f32;
        let baseline = (input.line_height - ascent - descent) * 0.5 + ascent;
        return Ok(ParagraphLayout {
            lines: vec![ParagraphLine {
                range: VisualRange::empty(VisualOffset::ZERO),
                bounds: yu_core::Rect::new(0.0, 0.0, input.indent, input.line_height)
                    .map_err(|_| CoreTextShapeError::InvalidViewportMetrics)?,
                baseline,
                font_ascent: ascent,
            }],
            ..ParagraphLayout::default()
        });
    }
    let attributed = CFMutableAttributedString::new(None, 0)
        .ok_or(CoreTextShapeError::AttributedStringUnavailable)?;
    unsafe {
        CFMutableAttributedString::replace_string(
            Some(&attributed),
            CFRange {
                location: 0,
                length: 0,
            },
            Some(&CFString::from_str(&text)),
        );
    }
    let direction = match input.base_direction {
        yu_core::BaseDirection::Auto => CTWritingDirection::Natural,
        yu_core::BaseDirection::Ltr => CTWritingDirection::LeftToRight,
        yu_core::BaseDirection::Rtl => CTWritingDirection::RightToLeft,
    };
    let setting = CTParagraphStyleSetting {
        spec: CTParagraphStyleSpecifier::BaseWritingDirection,
        valueSize: std::mem::size_of::<CTWritingDirection>(),
        value: NonNull::from(&direction).cast(),
    };
    // SAFETY: CoreText copies the setting while its typed value is alive.
    // The attributed range covers the full native UTF-16 text, including objects.
    unsafe {
        let paragraph_style = CTParagraphStyle::new(&setting, 1);
        CFMutableAttributedString::set_attribute(
            Some(&attributed),
            CFRange {
                location: 0,
                length: utf16 as _,
            },
            Some(kCTParagraphStyleAttributeName),
            Some(paragraph_style.as_ref()),
        );
    }
    let mut primary_font_struts = Vec::new();
    let mut cursor = 0;
    while cursor < units.len() {
        let unit = &units[cursor];
        let mut end = cursor + 1;
        while end < units.len() && units[end].attrs == unit.attrs {
            end += 1;
        }
        let request = FontRequest::new(
            unit.attrs
                .font()
                .family()
                .unwrap_or(shaper.request.family()),
            shaper.request.size() * unit.attrs.size_scale(),
        )
        .map_err(|_| CoreTextShapeError::InvalidViewportMetrics)?;
        let styled = if unit.attrs.style().is_code()
            && (unit.attrs.font().family().is_some()
                || unit.attrs.font() == yu_core::ThemeFont::SystemMono)
        {
            style_font_request(&request, unit.attrs.style().without_code())
        } else {
            style_font_request(&request, unit.attrs.style())
        };
        let source = if unit.attrs.font() == yu_core::ThemeFont::SystemUi {
            CoreTextFontSource::SystemUi
        } else if unit.attrs.font() == yu_core::ThemeFont::SystemMono {
            CoreTextFontSource::SystemMono
        } else if unit.attrs.font().family().is_some() {
            CoreTextFontSource::RequestedFamily
        } else {
            shaper.font_source
        };
        let font = create_core_text_font(&styled, source)?;
        if matches!(
            input.font_strut_mode,
            yu_core::FontStrutMode::AllFonts
                | yu_core::FontStrutMode::DeclaredFonts
                | yu_core::FontStrutMode::CodeFont
                | yu_core::FontStrutMode::RelativeFonts
        ) && units[cursor..end]
            .iter()
            .any(|unit| unit.object.is_none() && unit.spacer.is_none())
        {
            let mut ascent = unsafe { font.ascent() } as f32;
            let descent = unsafe { font.descent() } as f32;
            // Local Courier's browser-compatible logical ascent differs from
            // its CoreText metrics. This changes the strut, never glyph outlines.
            // See WebKit FontCoreText.cpp, FontBase::platformInit.
            if matches!(
                input.font_strut_mode,
                yu_core::FontStrutMode::CodeFont | yu_core::FontStrutMode::RelativeFonts
            ) && unsafe { font.family_name() }.to_string() == "Courier"
            {
                ascent += ((ascent + descent) * 0.15).round();
            }
            let height = if input.font_strut_mode == yu_core::FontStrutMode::RelativeFonts {
                input.line_height * unit.attrs.size_scale()
            } else {
                input.line_height
            };
            let leading = (height - ascent - descent) * 0.5;
            primary_font_struts.push((
                unit.start,
                units[end - 1].end,
                ascent + leading,
                descent + leading,
            ));
        }
        unsafe {
            CFMutableAttributedString::set_attribute(
                Some(&attributed),
                CFRange {
                    location: unit.start as _,
                    length: (units[end - 1].end - unit.start) as _,
                },
                Some(kCTFontAttributeName),
                Some(font.as_ref()),
            );
        }
        let spacing = unit.attrs.letter_spacing()
            + super::synthetic_bold_offset(&font, unit.attrs.style().is_strong()) as f32;
        if spacing != 0.0 {
            let kern = objc2_core_foundation::CFNumber::new_f32(spacing);
            unsafe {
                CFMutableAttributedString::set_attribute(
                    Some(&attributed),
                    CFRange {
                        location: unit.start as _,
                        length: (units[end - 1].end - unit.start) as _,
                    },
                    Some(objc2_core_text::kCTKernAttributeName),
                    Some(kern.as_ref()),
                );
            }
        }
        cursor = end;
    }
    for unit in &units {
        let metrics = if let Some(index) = unit.object {
            let object = input.objects[index];
            ObjectMetrics {
                width: object.size.width() as f64,
                ascent: object.baseline as f64,
                descent: (object.size.height() - object.baseline) as f64,
            }
        } else if let Some(width) = unit.spacer {
            ObjectMetrics {
                width: width as f64,
                ascent: 0.0,
                descent: 0.0,
            }
        } else {
            continue;
        };
        let metrics = Box::new(metrics);
        let pointer = Box::into_raw(metrics).cast::<c_void>();
        let callbacks = CTRunDelegateCallbacks {
            version: 1,
            dealloc: Some(object_drop),
            getAscent: Some(object_ascent),
            getDescent: Some(object_descent),
            getWidth: Some(object_width),
        };
        let delegate = unsafe { CTRunDelegate::new(NonNull::from(&callbacks), pointer) };
        let Some(delegate) = delegate else {
            unsafe {
                object_drop(NonNull::new(pointer).expect("Box is non-null"));
            }
            return Err(CoreTextShapeError::InvalidGlyphRun);
        };
        unsafe {
            CFMutableAttributedString::set_attribute(
                Some(&attributed),
                CFRange {
                    location: unit.start as _,
                    length: 1,
                },
                Some(kCTRunDelegateAttributeName),
                Some(delegate.as_ref()),
            );
        }
    }
    let typesetter = unsafe { CTTypesetter::with_attributed_string(&attributed) };
    let mut result = ParagraphLayout::default();
    let base_font_strut = if matches!(
        input.font_strut_mode,
        yu_core::FontStrutMode::BaseFont
            | yu_core::FontStrutMode::AllFonts
            | yu_core::FontStrutMode::RelativeFonts
    ) {
        let font = create_core_text_font(&shaper.request, shaper.font_source)?;
        let ascent = unsafe { font.ascent() } as f32;
        let descent = unsafe { font.descent() } as f32;
        let leading = (input.line_height - ascent - descent) * 0.5;
        Some((ascent + leading, descent + leading))
    } else {
        None
    };
    let mut start = 0_usize;
    let mut y = 0.0;
    while start < utf16 {
        let width = (input.width - input.indent).max(1.0) as f64;
        let suggested = unsafe { typesetter.suggest_line_break(start as _, width) };
        let mut end = start + usize::try_from(suggested).unwrap_or(0);
        // An over-wide grapheme/object must still make progress without splitting it.
        if !units
            .iter()
            .any(|unit| unit.start >= start && unit.start < end && unit.spacer.is_none())
        {
            let first = units
                .iter()
                .find(|unit| unit.end > start && unit.spacer.is_none())
                .ok_or(CoreTextShapeError::InvalidCoreTextRange)?;
            end = first.end;
            for unit in units.iter().filter(|unit| unit.start >= first.end) {
                if unit.spacer.is_none()
                    || unit.attrs.inline_box_id() != first.attrs.inline_box_id()
                {
                    break;
                }
                end = unit.end;
            }
        }
        // CoreText may emergency-break inside a composed character or before
        // its closing decoration when the viewport is narrower than one glyph.
        if let Some(unit) = units.iter().find(|unit| unit.start < end && end < unit.end) {
            end = unit.end;
        }
        if let Some(last) = units
            .iter()
            .rev()
            .find(|unit| unit.start >= start && unit.end <= end && unit.spacer.is_none())
        {
            let suggested_end = end;
            for unit in units.iter().filter(|unit| unit.start >= suggested_end) {
                if unit.spacer.is_none()
                    || unit.attrs.inline_box_id() != last.attrs.inline_box_id()
                    || unit.range.start() != last.range.end()
                {
                    break;
                }
                end = unit.end;
            }
        }
        end = end.min(utf16);
        let line = unsafe {
            typesetter.line(CFRange {
                location: start as _,
                length: (end - start) as _,
            })
        };
        let advance = unsafe {
            line.typographic_bounds(
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        } as f32;
        let runs = unsafe { line.glyph_runs() };
        let runs: CFRetained<CFArray<CTRun>> = unsafe { CFRetained::cast_unchecked(runs) };
        let (mut font_ascent, mut font_descent) = (0.0_f32, 0.0_f32);
        for run in runs.iter() {
            let attributes = unsafe { run.attributes() };
            let attributes: &CFDictionary<CFString, CTFont> =
                unsafe { attributes.cast_unchecked() };
            let font = attributes
                .get(unsafe { kCTFontAttributeName })
                .ok_or(CoreTextShapeError::MissingRunFont)?;
            font_ascent = font_ascent.max(unsafe { font.ascent() } as f32);
            font_descent = font_descent.max(unsafe { font.descent() } as f32);
        }
        // Font ink may extend outside an explicit line-height. Position the
        // baseline with half-leading; by default only inline objects enlarge the
        // strut. Using CTLine's ascent+descent as a minimum silently enlarges
        // tight heading lines and makes theme geometry impossible to reproduce.
        let leading = (input.line_height - font_ascent - font_descent) * 0.5;
        let mut baseline = font_ascent + leading;
        let mut below = input.line_height - baseline;
        if let Some((above, under)) = base_font_strut {
            baseline = above;
            below = under;
            let first = primary_font_struts.partition_point(|&(_, to, _, _)| to <= start);
            let last = primary_font_struts.partition_point(|&(from, _, _, _)| from < end);
            for &(_, _, above, under) in &primary_font_struts[first..last] {
                baseline = baseline.max(above);
                below = below.max(under);
            }
        }
        if matches!(
            input.font_strut_mode,
            yu_core::FontStrutMode::DeclaredFonts | yu_core::FontStrutMode::CodeFont
        ) {
            let first = primary_font_struts.partition_point(|&(_, to, _, _)| to <= start);
            let last = primary_font_struts.partition_point(|&(from, _, _, _)| from < end);
            if let Some(&(_, _, above, under)) =
                primary_font_struts.get(first).filter(|_| first < last)
            {
                baseline = above;
                below = under;
                for &(_, _, above, under) in &primary_font_struts[first + 1..last] {
                    baseline = baseline.max(above);
                    below = below.max(under);
                }
            }
        }
        let line_index = result.lines.len();
        let first_unit = units.partition_point(|unit| unit.end <= start);
        let last_unit = units.partition_point(|unit| unit.start < end);
        let line_units = &units[first_unit..last_unit];
        for object in line_units
            .iter()
            .filter_map(|unit| unit.object.map(|index| input.objects[index]))
        {
            baseline = baseline.max(object.baseline);
            below = below.max(object.size.height() - object.baseline);
        }
        let height = baseline + below;
        let range = VisualRange::new(
            line_units[0].range.start(),
            line_units
                .last()
                .ok_or(CoreTextShapeError::InvalidCoreTextRange)?
                .range
                .end(),
        )
        .ok_or(CoreTextShapeError::InvalidCoreTextRange)?;
        result.lines.push(ParagraphLine {
            range,
            bounds: yu_core::Rect::new(0.0, y, input.indent + advance, height)
                .map_err(|_| CoreTextShapeError::InvalidViewportMetrics)?,
            baseline,
            font_ascent,
        });
        let mut run_edges = Vec::with_capacity(runs.len());
        let mut spacer_edges = std::collections::BTreeMap::new();
        let mut ligature_carets = std::collections::BTreeMap::new();
        for run in runs.iter() {
            let count = cf_index_to_usize(unsafe { run.glyph_count() })?;
            if count == 0 {
                continue;
            }
            let range = unsafe { run.string_range() };
            let mut positions = vec![CGPoint::ZERO; count];
            unsafe {
                run.positions(
                    CFRange {
                        location: 0,
                        length: 0,
                    },
                    NonNull::new(positions.as_mut_ptr()).expect("nonempty run"),
                );
            }
            let native_start = cf_index_to_usize(range.location)?;
            let native_end = native_start + cf_index_to_usize(range.length)?;
            let first_unit = units.partition_point(|unit| unit.end <= native_start);
            let last_unit = units.partition_point(|unit| unit.start < native_end);
            if units[first_unit..last_unit]
                .iter()
                .any(|unit| unit.spacer.is_some_and(|width| width > 0.0))
            {
                let mut indices = vec![0_isize; count];
                unsafe {
                    run.string_indices(
                        CFRange {
                            location: 0,
                            length: count as _,
                        },
                        NonNull::new(indices.as_mut_ptr()).expect("nonempty spacer run"),
                    );
                }
                for (&index, position) in indices.iter().zip(&positions) {
                    let index = cf_index_to_usize(index)?;
                    let unit_index = units.partition_point(|unit| unit.end <= index);
                    if let Some(unit) = units.get(unit_index)
                        && let Some(width) = unit.spacer.filter(|width| *width > 0.0)
                    {
                        let left = input.indent + position.x as f32;
                        spacer_edges.insert(unit.start, (left, left + width));
                    }
                }
            }
            let left = positions
                .iter()
                .map(|point| point.x)
                .fold(f64::INFINITY, f64::min);
            let width = unsafe {
                run.typographic_bounds(
                    CFRange {
                        location: 0,
                        length: 0,
                    },
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };
            let rtl = unsafe { run.status() }.contains(CTRunStatus::RightToLeft);
            let attributes = unsafe { run.attributes() };
            let attributes: &CFDictionary<CFString, CTFont> =
                unsafe { attributes.cast_unchecked() };
            let font = attributes
                .get(unsafe { kCTFontAttributeName })
                .ok_or(CoreTextShapeError::MissingRunFont)?;
            // CTLine can return the opposite run's edge for a boundary
            // inside an Arabic ligature next to synthetic joiners. Preserve
            // native ligature carets; fonts without GDEF carets divide only
            // that already-shaped glyph advance, never the run or paragraph.
            let mut indices = vec![0_isize; count];
            let mut advances = vec![CGSize::ZERO; count];
            let mut glyph_ids = vec![0 as CGGlyph; count];
            let all = CFRange {
                location: 0,
                length: count as _,
            };
            unsafe {
                run.string_indices(all, NonNull::new(indices.as_mut_ptr()).expect("indices"));
                run.advances(all, NonNull::new(advances.as_mut_ptr()).expect("advances"));
                run.glyphs(all, NonNull::new(glyph_ids.as_mut_ptr()).expect("glyphs"));
            }
            let mut starts = indices
                .iter()
                .copied()
                .map(cf_index_to_usize)
                .collect::<Result<Vec<_>, _>>()?;
            starts.push(native_end);
            starts.sort_unstable();
            starts.dedup();
            for (glyph_index, &index) in indices.iter().enumerate() {
                let from = cf_index_to_usize(index)?;
                let to = starts
                    .get(starts.partition_point(|start| *start <= from))
                    .copied()
                    .unwrap_or(native_end);
                let first_boundary = units.partition_point(|unit| unit.start <= from);
                let last_boundary = units.partition_point(|unit| unit.start < to);
                let boundaries: Vec<_> = units[first_boundary..last_boundary]
                    .iter()
                    .filter(|unit| unit.spacer.is_none() && unit.object.is_none())
                    .map(|unit| unit.start)
                    .collect();
                if boundaries.is_empty() || advances[glyph_index].width <= 0.0 {
                    continue;
                }
                let mut native_carets = vec![0.0; boundaries.len()];
                let found = unsafe {
                    font.ligature_caret_positions(
                        glyph_ids[glyph_index],
                        native_carets.as_mut_ptr(),
                        native_carets.len() as _,
                    )
                };
                let width = advances[glyph_index].width;
                native_carets.sort_by(f64::total_cmp);
                for (offset, boundary) in boundaries.iter().enumerate() {
                    let caret = if found == boundaries.len() as isize
                        && native_carets
                            .iter()
                            .all(|v| v.is_finite() && *v > 0.0 && *v < width)
                    {
                        native_carets[if rtl {
                            boundaries.len() - offset - 1
                        } else {
                            offset
                        }]
                    } else {
                        let fraction = (offset + 1) as f64 / (boundaries.len() + 1) as f64;
                        width * if rtl { 1.0 - fraction } else { fraction }
                    };
                    ligature_carets.insert(*boundary, positions[glyph_index].x + caret);
                }
            }
            let ascent = unsafe { font.ascent() } as f32;
            let descent = unsafe { font.descent() } as f32;
            let start = cf_index_to_usize(range.location)?;
            run_edges.push((
                start,
                start + cf_index_to_usize(range.length)?,
                if rtl { left + width } else { left },
                if rtl { left } else { left + width },
                ascent,
                descent,
                units[first_unit..last_unit]
                    .iter()
                    .find(|unit| unit.spacer != Some(0.0))
                    .map_or(native_start, |unit| unit.start),
                units[first_unit..last_unit]
                    .iter()
                    .rfind(|unit| unit.spacer != Some(0.0))
                    .map_or(native_end, |unit| unit.end),
            ));
        }
        let boundary_x = |index: usize, preferred: Option<f64>| {
            let mut secondary = 0.0;
            let primary = unsafe { line.offset_for_string_index(index as _, &mut secondary) };
            let offset = preferred.map_or(primary, |preferred| {
                if (secondary - preferred).abs() < (primary - preferred).abs() {
                    secondary
                } else {
                    primary
                }
            });
            offset as f32 + input.indent
        };
        let unit_boundary_x = |index: usize, edge: Option<&NativeRunEdge>| {
            if let Some(edge) = edge {
                // CoreText may collapse caret offsets at synthetic joiners
                // onto another bidi run. The run's glyph extent is exact at
                // its first/last visible unit, including ligature-safe width.
                if index <= edge.6 {
                    return input.indent + edge.2 as f32;
                }
                if index >= edge.7 {
                    return input.indent + edge.3 as f32;
                }
            }
            let x = boundary_x(index, edge.map(|edge| (edge.2 + edge.3) * 0.5));
            if let Some(edge) = edge
                && (x < input.indent + edge.2.min(edge.3) as f32 - 0.001
                    || x > input.indent + edge.2.max(edge.3) as f32 + 0.001)
                && let Some(caret) = ligature_carets.get(&index)
            {
                return input.indent + *caret as f32;
            }
            x
        };
        let mut inline_boxes: std::collections::BTreeMap<u64, Vec<InlineAccumulator>> =
            std::collections::BTreeMap::new();
        for unit in line_units {
            let Some(id) = unit
                .attrs
                .inline_box_id()
                .filter(|_| unit.attrs.inline_inset() > 0.0)
            else {
                continue;
            };
            if unit.line_break || unit.spacer == Some(0.0) {
                continue;
            }
            let edge = run_edges
                .iter()
                .find(|edge| edge.0 <= unit.start && unit.start < edge.1);
            let first_x = unit_boundary_x(unit.start, edge);
            let last_x = unit_boundary_x(unit.end.min(end), edge);
            // Synthetic padding must use its native glyph extent. CTLine's
            // dual caret offsets around a zero-width joiner can point across
            // an entire opposite-direction run, even though padding is 3pt.
            let (left, right) = if unit.spacer.is_some() {
                *spacer_edges
                    .get(&unit.start)
                    .ok_or(CoreTextShapeError::InvalidCoreTextRange)?
            } else {
                (first_x.min(last_x), first_x.max(last_x))
            };
            let mut fragment = InlineAccumulator {
                range: unit.range,
                left,
                right,
                text_left: f32::INFINITY,
                text_right: f32::NEG_INFINITY,
                ascent: 0.0,
                descent: 0.0,
                inset_y: unit.attrs.inline_inset_y(),
                has_text: false,
            };
            if unit.spacer.is_none() && unit.object.is_none() {
                fragment.has_text = true;
                fragment.text_left = fragment.text_left.min(left);
                fragment.text_right = fragment.text_right.max(right);
                fragment.ascent = fragment.ascent.max(edge.map_or(font_ascent, |edge| edge.4));
                fragment.descent = fragment
                    .descent
                    .max(edge.map_or(font_descent, |edge| edge.5));
            }
            inline_boxes.entry(id).or_default().push(fragment);
        }
        for (id, mut pieces) in inline_boxes {
            // Logical source adjacency does not imply visual adjacency after
            // bidi reordering. Merge only touching native visual intervals.
            pieces.sort_by(|a, b| a.left.total_cmp(&b.left).then(a.right.total_cmp(&b.right)));
            let mut fragments: Vec<InlineAccumulator> = Vec::new();
            for piece in pieces {
                if let Some(last) = fragments.last_mut()
                    && piece.left <= last.right + 0.001
                {
                    last.right = last.right.max(piece.right);
                    last.range = VisualRange::new(
                        last.range.start().min(piece.range.start()),
                        last.range.end().max(piece.range.end()),
                    )
                    .ok_or(CoreTextShapeError::InvalidCoreTextRange)?;
                    last.text_left = last.text_left.min(piece.text_left);
                    last.text_right = last.text_right.max(piece.text_right);
                    last.ascent = last.ascent.max(piece.ascent);
                    last.descent = last.descent.max(piece.descent);
                    last.inset_y = last.inset_y.max(piece.inset_y);
                    last.has_text |= piece.has_text;
                } else {
                    fragments.push(piece);
                }
            }
            for fragment in fragments {
                if !fragment.has_text {
                    continue;
                }
                result.inline_boxes.push(yu_core::InlineBoxFragment {
                    id,
                    range: fragment.range,
                    bounds: yu_core::Rect::new(
                        fragment.left,
                        y + baseline - fragment.ascent - fragment.inset_y,
                        fragment.right - fragment.left,
                        fragment.ascent + fragment.descent + fragment.inset_y * 2.0,
                    )
                    .map_err(|_| CoreTextShapeError::InvalidViewportMetrics)?,
                    left_edge: fragment.left < fragment.text_left - 0.001,
                    right_edge: fragment.right > fragment.text_right + 0.001,
                });
            }
        }
        for unit in line_units {
            if unit.spacer.is_some() {
                continue;
            }
            let edge = run_edges
                .iter()
                .find(|edge| edge.0 <= unit.start && unit.start < edge.1);
            let x = unit_boundary_x(unit.start, edge);
            if let Some(index) = unit.object {
                result.objects.push(ParagraphObjectBox {
                    index,
                    line: line_index,
                    x,
                    y: y + baseline - input.objects[index].baseline,
                });
                continue;
            }
            let right = unit_boundary_x(unit.end.min(end), edge);
            result.clusters.push(ParagraphCluster {
                range: unit.range,
                style: unit.style,
                line: line_index,
                leading: x,
                trailing: if unit.line_break { x } else { right },
                line_break: unit.line_break,
            });
        }
        for run in runs.iter() {
            let count = cf_index_to_usize(unsafe { run.glyph_count() })?;
            if count == 0 {
                continue;
            }
            let range = unsafe { run.string_range() };
            let native_start = cf_index_to_usize(range.location)?;
            let run_end = native_start + cf_index_to_usize(range.length)?;
            let first = units.partition_point(|unit| unit.end <= native_start);
            let unit = units
                .get(first)
                .ok_or(CoreTextShapeError::InvalidCoreTextRange)?;
            if unit.object.is_some() || unit.spacer.is_some() {
                continue;
            }
            let attributes = unsafe { run.attributes() };
            let attributes: &CFDictionary<CFString, CTFont> =
                unsafe { attributes.cast_unchecked() };
            let font = attributes
                .get(unsafe { kCTFontAttributeName })
                .ok_or(CoreTextShapeError::MissingRunFont)?;
            let face = shaper.face_id_with_weight(
                &font,
                shaper.request.size() * unit.attrs.size_scale(),
                unit.attrs.style().is_strong(),
            )?;
            let mut ids = vec![0 as CGGlyph; count];
            let mut positions = vec![CGPoint::ZERO; count];
            let mut indices = vec![0_isize; count];
            let all = CFRange {
                location: 0,
                length: count as _,
            };
            unsafe {
                run.glyphs(
                    all,
                    NonNull::new(ids.as_mut_ptr()).expect("nonempty glyphs"),
                );
                run.positions(
                    all,
                    NonNull::new(positions.as_mut_ptr()).expect("nonempty positions"),
                );
                run.string_indices(
                    all,
                    NonNull::new(indices.as_mut_ptr()).expect("nonempty indices"),
                );
            }
            let mut boundaries = indices
                .iter()
                .map(|index| cf_index_to_usize(*index))
                .collect::<Result<Vec<_>, _>>()?;
            boundaries.push(run_end);
            boundaries.sort_unstable();
            boundaries.dedup();
            for ((id, position), index) in ids.into_iter().zip(positions).zip(indices) {
                let index = cf_index_to_usize(index)?;
                let unit_index = units.partition_point(|unit| unit.end <= index);
                let unit = units
                    .get(unit_index)
                    .ok_or(CoreTextShapeError::InvalidCoreTextRange)?;
                if unit.object.is_some() || unit.spacer.is_some() || unit.line_break {
                    continue;
                }
                let next = boundaries.partition_point(|boundary| *boundary <= index);
                let glyph_end = *boundaries.get(next).unwrap_or(&run_end);
                let last = units
                    .partition_point(|unit| unit.start < glyph_end)
                    .saturating_sub(1);
                let range = VisualRange::new(unit.range.start(), units[last].range.end())
                    .ok_or(CoreTextShapeError::InvalidCoreTextRange)?;
                result.glyphs.push(ParagraphGlyph {
                    range,
                    style: unit.style,
                    line: line_index,
                    face,
                    glyph: GlyphId::from_raw(id as u32),
                    x: input.indent + position.x as f32,
                    y: y + baseline - position.y as f32,
                    size_scale: unit.attrs.size_scale(),
                });
            }
        }
        y += height;
        start = end;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(text: &str, width: f32) -> ParagraphInput<'_> {
        ParagraphInput {
            font_strut_mode: yu_core::FontStrutMode::RunMetrics,
            text,
            base_direction: yu_core::BaseDirection::Auto,
            width,
            indent: 0.0,
            line_height: 25.6,
            runs: vec![yu_core::ParagraphRun {
                range: visual_range(0, text.len()).expect("range"),
                style: StyleId(0),
                attrs: TextAttrs::default(),
            }],
            objects: vec![],
        }
    }

    #[test]
    fn declared_heading_strut_is_stable_across_cjk_fallback() {
        let shaper =
            CoreTextShaper::from_system(FontRequest::new("Helvetica Neue", 16.0).expect("font"))
                .expect("shaper");
        let mut measurements = Vec::new();
        for text in ["Heading", "Heading 中文", "中文"] {
            let mut input = request(text, 1000.0);
            input.line_height = 43.2;
            input.font_strut_mode = yu_core::FontStrutMode::DeclaredFonts;
            input.runs[0].attrs = TextAttrs::new(TextStyle::Strong)
                .with_font(yu_core::ThemeFont::HelveticaNeue)
                .with_size_scale(2.25)
                .expect("scale");
            let output = shaper.layout(&input).expect("layout");
            assert_eq!(output.lines.len(), 1);
            assert!((output.lines[0].bounds.height() - 43.2).abs() < 0.001);
            measurements.push(output.lines[0].baseline);
        }
        assert!(
            measurements
                .iter()
                .all(|v| (v - measurements[0]).abs() < 0.001)
        );
    }

    #[test]
    fn mixed_body_font_struts_accumulate_weight_and_code_metrics() {
        let shaper =
            CoreTextShaper::from_system(FontRequest::new("Helvetica Neue", 16.0).expect("font"))
                .expect("shaper");
        let mut input = request("abc", 500.0);
        input.line_height = 26.0;
        input.font_strut_mode = yu_core::FontStrutMode::AllFonts;
        input.runs = vec![
            yu_core::ParagraphRun {
                range: visual_range(0, 1).expect("range"),
                style: StyleId(0),
                attrs: TextAttrs::new(TextStyle::Plain)
                    .with_font(yu_core::ThemeFont::HelveticaNeue),
            },
            yu_core::ParagraphRun {
                range: visual_range(1, 2).expect("range"),
                style: StyleId(1),
                attrs: TextAttrs::new(TextStyle::Strong)
                    .with_font(yu_core::ThemeFont::HelveticaNeue),
            },
            yu_core::ParagraphRun {
                range: visual_range(2, 3).expect("range"),
                style: StyleId(2),
                attrs: TextAttrs::new(TextStyle::Code)
                    .with_font(yu_core::ThemeFont::Monaco)
                    .with_size_scale(0.875)
                    .expect("scale"),
            },
        ];
        let output = shaper.layout(&input).expect("layout");
        let mut above: f32 = 0.0;
        let mut below: f32 = 0.0;
        for (family, size, style) in [
            ("Helvetica Neue", 16.0, TextStyle::Plain),
            ("Helvetica Neue", 16.0, TextStyle::Strong),
            ("Monaco", 14.0, TextStyle::Plain),
        ] {
            let request = style_font_request(&FontRequest::new(family, size).expect("font"), style);
            let font =
                create_core_text_font(&request, CoreTextFontSource::RequestedFamily).expect("font");
            let baseline =
                (26.0 + unsafe { font.ascent() } as f32 - unsafe { font.descent() } as f32) / 2.0;
            above = above.max(baseline);
            below = below.max(26.0 - baseline);
        }
        assert_eq!(output.lines.len(), 1);
        assert!((output.lines[0].bounds.height() - above - below).abs() < 0.001);
        assert!(output.lines[0].bounds.height() > 26.5);
        assert!((output.lines[0].baseline - above).abs() < 0.001);
    }

    #[test]
    fn synthetic_code_weight_affects_typesetting_and_registered_faces() {
        let shaper =
            CoreTextShaper::from_system(FontRequest::new("Helvetica Neue", 16.0).expect("font"))
                .expect("shaper");
        let mut input = request("HH HH HH", 500.0);
        input.runs[0].attrs = TextAttrs::new(TextStyle::Code).with_font(yu_core::ThemeFont::Monaco);
        let plain = shaper.layout(&input).expect("plain");
        input.width = plain.lines[0].bounds.width() + 0.1;
        let plain = shaper.layout(&input).expect("plain exact width");
        assert_eq!(plain.lines.len(), 1);
        input.runs[0].attrs = input.runs[0]
            .attrs
            .with_style(TextStyle::CodeStrongEmphasis);
        let bold = shaper.layout(&input).expect("bold oblique");
        assert!(
            bold.lines.len() > 1,
            "synthetic advance must participate in native wrapping"
        );
        assert!(!bold.glyphs.is_empty());
        for glyph in bold.glyphs {
            assert!(
                shaper
                    .faces
                    .with_entry(glyph.face, |entry| entry.synthetic_bold
                        && unsafe { entry.font.0.matrix() }.c > 0.0)
                    .expect("table")
                    .expect("face")
            );
        }
    }

    #[test]
    fn base_font_strut_keeps_code_baseline_without_expanding_table_line() {
        for zoom in [0.75, 1.0, 1.5] {
            let shaper = CoreTextShaper::from_system(
                FontRequest::new("Open Sans", 16.0 * zoom).expect("font"),
            )
            .expect("shaper");
            let mut input = request("code", 500.0);
            input.line_height = 25.6 * zoom;
            input.font_strut_mode = yu_core::FontStrutMode::BaseFont;
            let body = shaper.layout(&input).expect("body");
            input.runs[0].attrs = TextAttrs::new(TextStyle::Code)
                .with_font(yu_core::ThemeFont::Courier)
                .with_size_scale(0.9)
                .expect("scale");
            let code = shaper.layout(&input).expect("code");
            assert!((code.lines[0].baseline - body.lines[0].baseline).abs() < 0.001);
            assert!((code.lines[0].bounds.height() - input.line_height).abs() < 0.001);
            input.font_strut_mode = yu_core::FontStrutMode::RunMetrics;
            let run_only = shaper.layout(&input).expect("run metrics");
            assert_eq!(code.clusters, run_only.clusters);
            assert!(
                (code.lines[0].baseline - run_only.lines[0].baseline).abs() > 0.01,
                "base={} run={}",
                code.lines[0].baseline,
                run_only.lines[0].baseline
            );
        }
    }

    #[test]
    fn relative_code_struts_preserve_body_baseline_without_fallback_expansion() {
        for zoom in [0.75, 1.0, 1.5] {
            let shaper = CoreTextShaper::from_system(
                FontRequest::new("Helvetica Neue", 16.0 * zoom).expect("font"),
            )
            .expect("shaper");
            let mut input = request("code", 500.0);
            input.line_height = 25.6 * zoom;
            input.font_strut_mode = yu_core::FontStrutMode::RelativeFonts;
            let body = shaper.layout(&input).expect("body");
            input.runs[0].attrs = TextAttrs::new(TextStyle::Code)
                .with_font(yu_core::ThemeFont::Courier)
                .with_size_scale(0.9)
                .expect("size");
            let relative = shaper.layout(&input).expect("relative code");
            assert!((relative.lines[0].baseline - body.lines[0].baseline).abs() < 0.001);
            input.font_strut_mode = yu_core::FontStrutMode::AllFonts;
            let absolute = shaper.layout(&input).expect("absolute code");
            assert!(relative.lines[0].bounds.height() < absolute.lines[0].bounds.height());
            assert_eq!(relative.clusters, absolute.clusters);
            let code_height = relative.lines[0].bounds.height();
            let attrs = input.runs[0].attrs;
            for text in ["中文", "👨‍👩‍👧‍👦"] {
                let mut mixed = request(text, 500.0);
                mixed.line_height = input.line_height;
                mixed.font_strut_mode = yu_core::FontStrutMode::RelativeFonts;
                mixed.runs[0].attrs = attrs;
                let result = shaper.layout(&mixed).expect("fallback code");
                assert!((result.lines[0].bounds.height() - code_height).abs() < 0.001);
                assert!((result.lines[0].baseline - relative.lines[0].baseline).abs() < 0.001);
            }
        }
    }

    #[test]
    fn absolute_font_struts_expand_code_without_using_fallback_ink() {
        for zoom in [0.75, 1.0, 1.5] {
            let shaper = CoreTextShaper::from_system(
                FontRequest::new("Helvetica Neue", 16.0 * zoom).expect("font"),
            )
            .expect("shaper");
            let mut input = request("abcd", 500.0);
            input.line_height = 26.0 * zoom;
            input.runs[0].attrs = TextAttrs::new(TextStyle::Code)
                .with_font(yu_core::ThemeFont::Monaco)
                .with_size_scale(0.875)
                .expect("scale");
            let exact = shaper.layout(&input).expect("exact");
            input.font_strut_mode = yu_core::FontStrutMode::AllFonts;
            let aligned = shaper.layout(&input).expect("aligned");
            let body = create_core_text_font(&shaper.request, shaper.font_source).expect("body");
            let mono = create_core_text_font(
                &FontRequest::new("Monaco", 14.0 * zoom).expect("mono"),
                CoreTextFontSource::RequestedFamily,
            )
            .expect("font");
            let baseline = |font: &CTFont| {
                let a = unsafe { font.ascent() } as f32;
                let d = unsafe { font.descent() } as f32;
                (input.line_height + a - d) * 0.5
            };
            let above = baseline(&body).max(baseline(&mono));
            let below =
                (input.line_height - baseline(&body)).max(input.line_height - baseline(&mono));
            assert!((aligned.lines[0].bounds.height() - above - below).abs() < 0.001);
            assert!(aligned.lines[0].bounds.height() > exact.lines[0].bounds.height() + 0.4 * zoom);
            assert_eq!(aligned.clusters, exact.clusters);
            for text in ["中文", "🪶", "👨‍👩‍👧‍👦"] {
                let mut input = request(text, 500.0);
                input.line_height = 26.0 * zoom;
                input.font_strut_mode = yu_core::FontStrutMode::AllFonts;
                let layout = shaper.layout(&input).expect("fallback glyphs");
                assert!((layout.lines[0].bounds.height() - 26.0 * zoom).abs() < 0.001);
            }
        }
    }

    #[test]
    fn explicit_paragraph_direction_preserves_run_direction_and_source_ranges() {
        let shaper =
            CoreTextShaper::from_system_ui(FontRequest::new("System UI", 16.0).expect("font"))
                .expect("shaper");
        for text in ["אבג def דהו xyz", "مرحبا world أهلا end"] {
            let mut input = request(text, 800.0);
            let mut orders = Vec::new();
            for direction in [
                yu_core::BaseDirection::Auto,
                yu_core::BaseDirection::Ltr,
                yu_core::BaseDirection::Rtl,
            ] {
                input.base_direction = direction;
                let layout = shaper.layout(&input).expect("paragraph");
                assert_eq!(layout.lines.len(), 1);
                assert_eq!(layout.clusters.len(), text.graphemes(true).count());
                let mut clusters: Vec<_> = layout.clusters.iter().collect();
                clusters.sort_by(|a, b| {
                    a.leading
                        .min(a.trailing)
                        .total_cmp(&b.leading.min(b.trailing))
                });
                let order: String = clusters
                    .iter()
                    .map(|c| &text[c.range.start().get() as usize..c.range.end().get() as usize])
                    .collect();
                let latin = layout
                    .clusters
                    .iter()
                    .find(|c| text[c.range.start().get() as usize..].starts_with(['d', 'w']))
                    .expect("Latin run");
                assert!(latin.leading < latin.trailing);
                assert!(layout.clusters.iter().any(|c| c.leading > c.trailing));
                orders.push(order);
            }
            assert_eq!(orders[0], orders[2], "Auto follows the initial RTL run");
            assert_ne!(orders[0], orders[1], "LTR must change paragraph embedding");
            assert!(orders[1].ends_with(if text.starts_with('א') { "xyz" } else { "end" }));
            assert_eq!(input.text, text);
        }
    }

    #[test]
    fn bidi_inline_background_never_bridges_unboxed_visual_text() {
        let shaper =
            CoreTextShaper::from_system_ui(FontRequest::new("System UI", 16.0).expect("font"))
                .expect("shaper");
        let mut split_cases = 0;
        for text in [
            "abc אבג def דהו xyz",
            "שלום abc עולם def",
            "hello مرحبا world أهلا",
        ] {
            let boundaries: Vec<_> = text
                .grapheme_indices(true)
                .map(|(i, _)| i)
                .chain(std::iter::once(text.len()))
                .collect();
            for (a, &start) in boundaries.iter().enumerate().take(boundaries.len() - 1) {
                for &end in boundaries.iter().skip(a + 1) {
                    let mut input = request(text, 600.0);
                    input.runs.clear();
                    for (from, to, boxed) in [
                        (0, start, false),
                        (start, end, true),
                        (end, text.len(), false),
                    ] {
                        if from == to {
                            continue;
                        }
                        let attrs = if boxed {
                            TextAttrs::default()
                                .with_inline_box_id(Some(7))
                                .with_inline_inset(3.0)
                                .expect("inset")
                        } else {
                            TextAttrs::default()
                        };
                        input.runs.push(yu_core::ParagraphRun {
                            range: visual_range(from, to).expect("range"),
                            style: StyleId(0),
                            attrs,
                        });
                    }
                    for width in [600.0, 80.0] {
                        input.width = width;
                        let layout = shaper.layout(&input).unwrap_or_else(|error| {
                            panic!("{text:?} {start}..{end} width={width}: {error}")
                        });
                        split_cases += usize::from(width == 600.0 && layout.inline_boxes.len() > 1);
                        assert_eq!(layout.clusters.len(), text.graphemes(true).count());
                        for cluster in &layout.clusters {
                            let line = &layout.lines[cluster.line];
                            let fragments: Vec<_> = layout
                                .inline_boxes
                                .iter()
                                .filter(|f| {
                                    f.bounds.y() < line.bounds.bottom()
                                        && f.bounds.bottom() > line.bounds.y()
                                })
                                .collect();
                            let left = cluster.leading.min(cluster.trailing);
                            let right = cluster.leading.max(cluster.trailing);
                            if right - left <= 0.001 {
                                continue;
                            }
                            if cluster.range.start().get() >= start as u64
                                && cluster.range.end().get() <= end as u64
                            {
                                assert!(
                                    fragments.iter().any(|f| f.bounds.x() <= left + 0.01
                                        && f.bounds.right() >= right - 0.01),
                                    "{text:?} {start}..{end} width={width}: missing code background {cluster:?}"
                                );
                            } else {
                                for fragment in fragments {
                                    let overlap = fragment.bounds.right().min(right)
                                        - fragment.bounds.x().max(left);
                                    assert!(
                                        overlap <= 0.01,
                                        "{text:?} range={start}..{end} width={width}: background {fragment:?} bridges plain cluster {cluster:?}"
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
        assert!(
            split_cases > 0,
            "must exercise a visually disjoint code span"
        );
    }

    #[test]
    fn inline_edges_change_native_width_without_adding_source_clusters_or_glyphs() {
        let shaper =
            CoreTextShaper::from_system_ui(FontRequest::new("System UI", 16.0).expect("font"))
                .expect("shaper");
        for text in ["code", "שלום", "é👨‍👩‍👧‍👦"] {
            let mut input = request(text, 600.0);
            let plain = shaper.layout(&input).expect("plain");
            input.runs[0].attrs = input.runs[0]
                .attrs
                .with_inline_box_id(Some(9))
                .with_inline_inset(3.0)
                .expect("inset")
                .with_inline_inset_y(1.0)
                .expect("inset");
            let boxed = shaper.layout(&input).expect("box");
            assert!(
                (boxed.lines[0].bounds.width() - plain.lines[0].bounds.width() - 6.0).abs() < 0.01,
                "{text}: {:?}",
                boxed.lines
            );
            assert_eq!(boxed.clusters.len(), plain.clusters.len());
            assert_eq!(boxed.glyphs.len(), plain.glyphs.len());
            assert_eq!(boxed.inline_boxes.len(), 1);
            let fragment = boxed.inline_boxes[0];
            assert!(fragment.left_edge && fragment.right_edge);
            assert_eq!(fragment.range, visual_range(0, text.len()).expect("range"));
            assert!((fragment.bounds.width() - boxed.lines[0].bounds.width()).abs() < 0.01);
            assert!(
                boxed
                    .glyphs
                    .iter()
                    .all(|g| g.range.end().get() <= text.len() as u64)
            );
            assert_eq!(input.text, text);
        }
    }

    #[test]
    fn inline_edges_wrap_with_their_content_and_adjacent_boxes_stay_distinct() {
        let shaper =
            CoreTextShaper::from_system_ui(FontRequest::new("System UI", 16.0).expect("font"))
                .expect("shaper");
        let mut input = request("aaaa bbbb", 600.0);
        let plain = shaper.layout(&input).expect("plain");
        input.width = plain.lines[0].bounds.width() + 0.1;
        input.runs = vec![
            yu_core::ParagraphRun {
                range: visual_range(0, 5).expect("range"),
                style: StyleId(0),
                attrs: TextAttrs::default(),
            },
            yu_core::ParagraphRun {
                range: visual_range(5, 9).expect("range"),
                style: StyleId(1),
                attrs: TextAttrs::default()
                    .with_inline_box_id(Some(1))
                    .with_inline_inset(5.0)
                    .expect("inset"),
            },
        ];
        let boxed = shaper.layout(&input).expect("wrapped");
        assert!(boxed.lines.len() > 1);
        assert!(
            boxed.lines.iter().all(|line| !line.range.is_empty()),
            "padding must not create empty source lines"
        );
        let mut adjacent = request("ab", 600.0);
        adjacent.runs = (0..2)
            .map(|n| yu_core::ParagraphRun {
                range: visual_range(n, n + 1).expect("range"),
                style: StyleId(n as u32),
                attrs: TextAttrs::default()
                    .with_inline_box_id(Some(n as u64))
                    .with_inline_inset(3.0)
                    .expect("inset"),
            })
            .collect();
        let geometry = shaper.layout(&adjacent).expect("adjacent");
        assert_eq!(geometry.inline_boxes.len(), 2);
        assert!(
            geometry
                .inline_boxes
                .iter()
                .all(|fragment| fragment.left_edge && fragment.right_edge)
        );
        let mut narrow = request("😀", 1.0);
        narrow.runs[0].attrs = adjacent.runs[0].attrs;
        let geometry = shaper.layout(&narrow).expect("over-wide cluster");
        assert!(
            geometry.lines.iter().all(|line| !line.range.is_empty()),
            "an over-wide emoji must keep its padding on the same source line"
        );
    }

    #[test]
    fn prose_slash_pairs_follow_reference_wrapping_without_changing_source() {
        let shaper =
            CoreTextShaper::from_system_ui(FontRequest::new("System UI", 16.0).expect("font"))
                .expect("shaper");
        for (text, prefix, expected_end) in [
            ("prefix alpha/beta end", "prefix alpha/", 7),
            ("prefix alpha/éclair end", "prefix alpha/", 7),
            ("prefix abcd-efgh end", "prefix abcd-", 12),
        ] {
            let width = shaper
                .layout(&request(prefix, 600.0))
                .expect("prefix")
                .lines[0]
                .bounds
                .width()
                + 0.1;
            let layout = shaper.layout(&request(text, width)).expect("paragraph");
            assert_eq!(
                layout.lines[0].range.end().get(),
                expected_end,
                "{text}: {:?}",
                layout.lines
            );
            assert_eq!(layout.clusters.len(), text.graphemes(true).count());
            assert!(layout.lines.iter().all(|line| !line.range.is_empty()));
            assert!(
                layout
                    .glyphs
                    .iter()
                    .all(|glyph| glyph.range.end().get() <= text.len() as u64)
            );
        }
    }

    #[test]
    fn slash_joiners_preserve_inline_edges_and_emergency_grapheme_wrapping() {
        let shaper =
            CoreTextShaper::from_system_ui(FontRequest::new("System UI", 16.0).expect("font"))
                .expect("shaper");
        let text = "prefix alpha/beta end";
        let prefix_width = shaper
            .layout(&request("prefix alpha/", 600.0))
            .expect("prefix")
            .lines[0]
            .bounds
            .width();
        for (start, end) in [(7, 13), (13, 17)] {
            let mut input = request(text, prefix_width + 6.1);
            input.runs = vec![
                yu_core::ParagraphRun {
                    range: visual_range(0, start).expect("range"),
                    style: StyleId(0),
                    attrs: TextAttrs::default(),
                },
                yu_core::ParagraphRun {
                    range: visual_range(start, end).expect("range"),
                    style: StyleId(1),
                    attrs: TextAttrs::default()
                        .with_inline_box_id(Some(1))
                        .with_inline_inset(3.0)
                        .expect("inset"),
                },
                yu_core::ParagraphRun {
                    range: visual_range(end, text.len()).expect("range"),
                    style: StyleId(0),
                    attrs: TextAttrs::default(),
                },
            ];
            let layout = shaper.layout(&input).expect("inline boundary");
            assert_eq!(
                layout.lines[0].range.end().get(),
                7,
                "joining crosses either inline edge"
            );
            assert_eq!(layout.clusters.len(), text.graphemes(true).count());
        }
        for text in ["alpha/beta/gamma", "a/é👨‍👩‍👧‍👦b/你", "office/affinity"] {
            for width in [1.0, 45.0] {
                let input = request(text, width);
                let layout = shaper.layout(&input).expect("narrow paragraph");
                assert!(layout.lines.iter().all(|line| !line.range.is_empty()));
                assert_eq!(layout.clusters.len(), text.graphemes(true).count());
                for line in &layout.lines {
                    assert!(text.is_char_boundary(line.range.start().get() as usize));
                    assert!(text.is_char_boundary(line.range.end().get() as usize));
                }
                if text == "alpha/beta/gamma" && width == 45.0 {
                    assert!(
                        layout.lines.len() < 6,
                        "emergency wrapping must fill lines, not emit one character each"
                    );
                }
            }
        }
        // CJK wrapping remains CoreText policy; its exact choice can vary
        // with native font fallback. Verify that we insert no Latin-specific
        // joiner at any boundary, rather than pinning a native break offset.
        let cjk = "prefix alpha/中文 end";
        assert!(
            cjk.char_indices()
                .all(|(offset, _)| !joined_solidus_boundary(cjk, offset))
        );
        for width in [45.0, 100.0] {
            let layout = shaper.layout(&request(cjk, width)).expect("CJK paragraph");
            assert_eq!(layout.clusters.len(), cjk.graphemes(true).count());
            assert_eq!(
                layout
                    .lines
                    .first()
                    .expect("first CJK line")
                    .range
                    .start()
                    .get(),
                0
            );
            assert_eq!(
                layout
                    .lines
                    .last()
                    .expect("last CJK line")
                    .range
                    .end()
                    .get(),
                cjk.len() as u64
            );
            for pair in layout.lines.windows(2) {
                assert_eq!(pair[0].range.end(), pair[1].range.start());
            }
        }
    }

    #[test]
    fn inline_decoration_does_not_introduce_a_break_inside_a_word() {
        let shaper =
            CoreTextShaper::from_system_ui(FontRequest::new("System UI", 16.0).expect("font"))
                .expect("shaper");
        let prefix = shaper.layout(&request("xx abc", 600.0)).expect("prefix");
        let mut input = request("xx abcdef", prefix.lines[0].bounds.width() + 3.1);
        input.runs = vec![
            yu_core::ParagraphRun {
                range: visual_range(0, 6).expect("range"),
                style: StyleId(0),
                attrs: TextAttrs::default(),
            },
            yu_core::ParagraphRun {
                range: visual_range(6, 9).expect("range"),
                style: StyleId(1),
                attrs: TextAttrs::default()
                    .with_inline_box_id(Some(1))
                    .with_inline_inset(3.0)
                    .expect("inset"),
            },
        ];
        let layout = shaper.layout(&input).expect("layout");
        assert_eq!(
            layout.lines[0].range,
            visual_range(0, 3).expect("space break")
        );
    }

    #[test]
    fn heading_tracking_changes_native_glyphs_and_editing_boundaries_together() {
        let shaper =
            CoreTextShaper::from_system_ui(FontRequest::new("System UI", 16.0).expect("font"))
                .expect("CoreText");
        let mut input = request("HHHHHH", 600.0);
        input.runs[0].attrs = TextAttrs::default().with_font(yu_core::ThemeFont::LucidaGrande);
        let normal = shaper.layout(&input).expect("normal");
        input.runs[0].attrs = input.runs[0]
            .attrs
            .with_letter_spacing(-1.5)
            .expect("tracking");
        let tight = shaper.layout(&input).expect("tracking");
        assert!(tight.lines[0].bounds.width() < normal.lines[0].bounds.width());
        assert!(tight.glyphs.last().expect("glyph").x < normal.glyphs.last().expect("glyph").x);
        assert!(
            tight.clusters.last().expect("cluster").trailing
                < normal.clusters.last().expect("cluster").trailing
        );
        assert_eq!(normal.lines[0].range, tight.lines[0].range);
    }

    #[test]
    fn yu_system_roles_keep_resolved_faces_for_mixed_text_and_rasterization() {
        let shaper = CoreTextShaper::from_system_ui(
            FontRequest::new("System UI", 16.0).expect("valid test setup"),
        )
        .expect("valid test setup");
        for role in [yu_core::ThemeFont::SystemUi, yu_core::ThemeFont::SystemMono] {
            let mut input = request("Yu 中文 e\u{301} 🙂 مرحبا", 600.0);
            input.line_height = 26.0;
            input.runs[0].attrs = TextAttrs::default().with_font(role);
            let paragraph = shaper.layout(&input).expect("valid test setup");
            assert!(!paragraph.glyphs.is_empty());
            for glyph in &paragraph.glyphs {
                let face = shaper
                    .faces
                    .with_entry(glyph.face, Clone::clone)
                    .expect("valid test setup")
                    .expect("valid test setup");
                let replay = shaper
                    .rasterizer()
                    .font_for_face(glyph.face, 16.0 * glyph.size_scale, 2.0)
                    .expect("valid test setup");
                assert_eq!(
                    unsafe { replay.post_script_name() }.to_string(),
                    unsafe { face.font.0.post_script_name() }.to_string()
                );
            }
        }
    }

    #[test]
    fn native_theme_fonts_survive_shaping_and_rasterization() {
        let shaper =
            CoreTextShaper::from_system_ui(FontRequest::new("System UI", 16.0).expect("font"))
                .expect("CoreText");
        for (font, scale, height, expected) in [
            (
                yu_core::ThemeFont::HelveticaNeue,
                1.0,
                26.0,
                "Helvetica Neue",
            ),
            (yu_core::ThemeFont::LucidaGrande, 2.5, 44.0, "Lucida Grande"),
            (yu_core::ThemeFont::Monaco, 0.875, 22.4, "Monaco"),
            (yu_core::ThemeFont::Courier, 0.9, 23.04, "Courier"),
        ] {
            let mut input = request("Native fonts", 600.0);
            input.line_height = height;
            input.runs[0].attrs = TextAttrs::default()
                .with_font(font)
                .with_size_scale(scale)
                .expect("size");
            let paragraph = shaper.layout(&input).expect("theme paragraph");
            assert_eq!(paragraph.lines.len(), 1);
            assert!((paragraph.lines[0].bounds.height() - height).abs() < 0.01);
            let glyph = &paragraph.glyphs[0];
            let face = shaper
                .faces
                .with_entry(glyph.face, Clone::clone)
                .expect("face table")
                .expect("resolved face");
            assert_eq!(unsafe { face.font.0.family_name() }.to_string(), expected);
            let key = GlyphRasterKey::new(glyph.face, glyph.glyph, 16.0 * glyph.size_scale)
                .expect("raster key");
            let rasterizer = shaper.rasterizer();
            let replay = rasterizer
                .font_for_face(key.face(), key.size(), 2.0)
                .expect("retained font");
            assert_eq!(unsafe { replay.family_name() }.to_string(), expected);
            assert!(
                !rasterizer
                    .rasterize(key)
                    .expect("glyph bitmap")
                    .bitmap()
                    .is_empty()
            );
        }
    }

    #[test]
    fn courier_code_baseline_stays_fixed_across_fallback_and_zoom() {
        for zoom in [0.75, 1.0, 1.5] {
            let size = 14.4 * zoom;
            let shaper =
                CoreTextShaper::from_system(FontRequest::new("Courier", size).expect("font"))
                    .expect("CoreText");
            let mut baselines = Vec::new();
            for text in ["plain ascii", "plain 中文", "emoji 👨‍👩‍👧‍👦"] {
                let mut input = request(text, 600.0);
                input.line_height = 23.04 * zoom;
                input.font_strut_mode = yu_core::FontStrutMode::CodeFont;
                let layout = shaper.layout(&input).expect("code paragraph");
                assert_eq!(layout.lines.len(), 1);
                assert!((layout.lines[0].bounds.height() - input.line_height).abs() < 0.001);
                baselines.push(layout.lines[0].baseline);
                assert!(!layout.glyphs.is_empty());
                assert!(!layout.clusters.is_empty());
            }
            assert!((baselines[0] - baselines[1]).abs() < 0.001);
            assert!((baselines[0] - baselines[2]).abs() < 0.001);
            if zoom == 1.0 {
                // Courier 14.4pt: native half-leading baseline 15.17625pt;
                // rounded 15% ascent adjustment adds 1pt to the baseline.
                assert!((baselines[0] - 16.17625).abs() < 0.001);
            }
        }
    }

    #[test]
    fn native_paragraph_preserves_complex_text_and_uses_real_baselines() {
        let shaper =
            CoreTextShaper::from_system_ui(FontRequest::new("System UI", 16.0).expect("font"))
                .expect("CoreText");
        let text = "office é 👨‍👩‍👧‍👦 العربية עברית 中文";
        let layout = shaper
            .layout(&request(text, 800.0))
            .expect("native paragraph");
        assert_eq!(layout.lines.len(), 1);
        assert_eq!(layout.clusters.len(), text.graphemes(true).count());
        assert!(!layout.glyphs.is_empty());
        assert!(layout.lines[0].baseline < layout.lines[0].bounds.height());
        assert!(
            layout
                .clusters
                .iter()
                .any(|cluster| cluster.leading > cluster.trailing)
        );
        assert_eq!(
            layout
                .clusters
                .last()
                .expect("last cluster")
                .range
                .end()
                .get(),
            text.len() as u64
        );
    }

    #[test]
    fn bidi_run_boundaries_do_not_create_document_wide_clusters() {
        let shaper = CoreTextShaper::from_system_ui(
            FontRequest::new("System UI", 16.0).expect("font request"),
        )
        .expect("CoreText");
        let text = "English العربية more עברית end";
        let layout = shaper
            .layout(&ParagraphInput {
                font_strut_mode: yu_core::FontStrutMode::RunMetrics,
                text,
                base_direction: yu_core::BaseDirection::Auto,
                runs: vec![],
                objects: vec![],
                width: 800.0,
                indent: 0.0,
                line_height: 25.6,
            })
            .expect("paragraph");
        assert_eq!(layout.lines.len(), 1);
        assert!(
            layout
                .clusters
                .iter()
                .any(|cluster| cluster.leading > cluster.trailing)
        );
        for cluster in &layout.clusters {
            assert!(
                (cluster.trailing - cluster.leading).abs() < 32.0,
                "a grapheme cannot span another bidi run: {cluster:?}"
            );
        }
    }

    #[test]
    fn native_paragraph_wraps_words_and_does_not_add_a_trailing_source_line() {
        let shaper =
            CoreTextShaper::from_system_ui(FontRequest::new("System UI", 16.0).expect("font"))
                .expect("CoreText");
        let wide = shaper
            .layout(&request("one two three\n", 800.0))
            .expect("wide paragraph");
        assert_eq!(wide.lines.len(), 1);
        let narrow = shaper
            .layout(&request("one two three\n", 65.0))
            .expect("narrow paragraph");
        assert!(narrow.lines.len() > 1);
        for line in &narrow.lines {
            assert!((line.bounds.height() - 25.6).abs() < 0.01);
            assert!(line.baseline > 0.0 && line.baseline < line.bounds.height());
        }
    }
}
