#![cfg(target_os = "windows")]
#![allow(non_snake_case)]

use std::fmt;
use std::mem::ManuallyDrop;
use std::ptr;
use std::sync::{Arc, Mutex};

use unicode_bidi::{BidiClass, bidi_class};
use windows::Win32::Foundation::{BOOL, DWRITE_E_NOCOLOR, RECT};
use windows::Win32::Graphics::DirectWrite::*;
use windows::core::{ComObject, PCWSTR, Result as WinResult, implement};
use yu_core::{GlyphRun, Script, ShapedText, ShapingProvider, TextDirection, TextRange, TextStyle};
use yu_font::{
    FaceTableError, FontFaceId, FontMetricKey, FontMetricsCache, FontMetricsSnapshot, FontRequest,
    FontSlant, FontWeight, GlyphBitmap, GlyphMetrics, GlyphRasterKey, GlyphRasterizer,
    RasterDataError, RasterizedGlyph, RasterizingShaper, ShapeError, SharedFaceTable, Utf16Map,
};

use crate::run::{RunAssemblyError, ShapedArrays, assemble_run};

const LOCALE: &[u16] = &[
    b'e' as u16,
    b'n' as u16,
    b'-' as u16,
    b'u' as u16,
    b's' as u16,
    0,
];

#[derive(Clone)]
pub struct DirectWriteFace {
    face: IDWriteFontFace,
    family: Arc<str>,
    face_name: Arc<str>,
    size_factor: f32,
}

impl fmt::Debug for DirectWriteFace {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DirectWriteFace")
            .field("family", &self.family)
            .field("face_name", &self.face_name)
            .field("size_factor", &self.size_factor)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug)]
pub enum DirectWriteError {
    Windows(Arc<str>),
    FaceTable(FaceTableError),
    Run(RunAssemblyError),
    Raster(RasterDataError),
    UnknownFace(FontFaceId),
    OffsetOverflow,
    InvalidAnalysis(&'static str),
}

impl fmt::Display for DirectWriteError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Windows(message) => formatter.write_str(message),
            Self::FaceTable(error) => error.fmt(formatter),
            Self::Run(error) => error.fmt(formatter),
            Self::Raster(error) => error.fmt(formatter),
            Self::UnknownFace(face) => write!(formatter, "unknown DirectWrite face {}", face.get()),
            Self::OffsetOverflow => formatter.write_str("DirectWrite text offset overflowed"),
            Self::InvalidAnalysis(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for DirectWriteError {}

impl From<windows::core::Error> for DirectWriteError {
    fn from(error: windows::core::Error) -> Self {
        Self::Windows(Arc::from(error.to_string()))
    }
}

impl From<FaceTableError> for DirectWriteError {
    fn from(error: FaceTableError) -> Self {
        Self::FaceTable(error)
    }
}

impl From<RunAssemblyError> for DirectWriteError {
    fn from(error: RunAssemblyError) -> Self {
        Self::Run(error)
    }
}

impl From<RasterDataError> for DirectWriteError {
    fn from(error: RasterDataError) -> Self {
        Self::Raster(error)
    }
}

#[derive(Clone, Copy, Debug)]
struct ScriptSpan {
    start: u32,
    len: u32,
    analysis: DWRITE_SCRIPT_ANALYSIS,
}

#[derive(Clone, Copy, Debug)]
struct BidiSpan {
    start: u32,
    len: u32,
    level: u8,
}

#[implement(IDWriteTextAnalysisSource)]
struct AnalysisSource {
    text: Vec<u16>,
    locale: Vec<u16>,
    direction: DWRITE_READING_DIRECTION,
}

impl IDWriteTextAnalysisSource_Impl for AnalysisSource_Impl {
    fn GetTextAtPosition(
        &self,
        textposition: u32,
        textstring: *mut *mut u16,
        textlength: *mut u32,
    ) -> WinResult<()> {
        let position = usize::try_from(textposition).unwrap_or(usize::MAX);
        unsafe {
            if position >= self.text.len() {
                *textstring = ptr::null_mut();
                *textlength = 0;
            } else {
                *textstring = self.text.as_ptr().add(position).cast_mut();
                *textlength = u32::try_from(self.text.len() - position).unwrap_or(u32::MAX);
            }
        }
        Ok(())
    }

    fn GetTextBeforePosition(
        &self,
        textposition: u32,
        textstring: *mut *mut u16,
        textlength: *mut u32,
    ) -> WinResult<()> {
        let position = usize::try_from(textposition)
            .unwrap_or(usize::MAX)
            .min(self.text.len());
        unsafe {
            if position == 0 {
                *textstring = ptr::null_mut();
                *textlength = 0;
            } else {
                *textstring = self.text.as_ptr().cast_mut();
                *textlength = u32::try_from(position).unwrap_or(u32::MAX);
            }
        }
        Ok(())
    }

    fn GetParagraphReadingDirection(&self) -> DWRITE_READING_DIRECTION {
        self.direction
    }

    fn GetLocaleName(
        &self,
        textposition: u32,
        textlength: *mut u32,
        localename: *mut *mut u16,
    ) -> WinResult<()> {
        let position = usize::try_from(textposition).unwrap_or(usize::MAX);
        unsafe {
            *textlength = if position < self.text.len() {
                u32::try_from(self.text.len() - position).unwrap_or(u32::MAX)
            } else {
                0
            };
            *localename = self.locale.as_ptr().cast_mut();
        }
        Ok(())
    }

    fn GetNumberSubstitution(
        &self,
        textposition: u32,
        textlength: *mut u32,
        numbersubstitution: *mut Option<IDWriteNumberSubstitution>,
    ) -> WinResult<()> {
        let position = usize::try_from(textposition).unwrap_or(usize::MAX);
        unsafe {
            *textlength = if position < self.text.len() {
                u32::try_from(self.text.len() - position).unwrap_or(u32::MAX)
            } else {
                0
            };
            *numbersubstitution = None;
        }
        Ok(())
    }
}

#[implement(IDWriteTextAnalysisSink)]
struct AnalysisSink {
    scripts: Arc<Mutex<Vec<ScriptSpan>>>,
    bidis: Arc<Mutex<Vec<BidiSpan>>>,
}

impl IDWriteTextAnalysisSink_Impl for AnalysisSink_Impl {
    fn SetScriptAnalysis(
        &self,
        textposition: u32,
        textlength: u32,
        scriptanalysis: *const DWRITE_SCRIPT_ANALYSIS,
    ) -> WinResult<()> {
        if scriptanalysis.is_null() {
            return Ok(());
        }
        let analysis = unsafe { *scriptanalysis };
        if let Ok(mut spans) = self.scripts.lock() {
            spans.push(ScriptSpan {
                start: textposition,
                len: textlength,
                analysis,
            });
        }
        Ok(())
    }

    fn SetLineBreakpoints(
        &self,
        _textposition: u32,
        _textlength: u32,
        _linebreakpoints: *const DWRITE_LINE_BREAKPOINT,
    ) -> WinResult<()> {
        Ok(())
    }

    fn SetBidiLevel(
        &self,
        textposition: u32,
        textlength: u32,
        _explicitlevel: u8,
        resolvedlevel: u8,
    ) -> WinResult<()> {
        if let Ok(mut spans) = self.bidis.lock() {
            spans.push(BidiSpan {
                start: textposition,
                len: textlength,
                level: resolvedlevel,
            });
        }
        Ok(())
    }

    fn SetNumberSubstitution(
        &self,
        _textposition: u32,
        _textlength: u32,
        _numbersubstitution: Option<&IDWriteNumberSubstitution>,
    ) -> WinResult<()> {
        Ok(())
    }
}

fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn dwrite_weight(weight: FontWeight) -> DWRITE_FONT_WEIGHT {
    match weight {
        FontWeight::Normal => DWRITE_FONT_WEIGHT_NORMAL,
        FontWeight::Medium => DWRITE_FONT_WEIGHT_MEDIUM,
        FontWeight::Bold => DWRITE_FONT_WEIGHT_BOLD,
    }
}

fn dwrite_style(slant: FontSlant) -> DWRITE_FONT_STYLE {
    match slant {
        FontSlant::Upright => DWRITE_FONT_STYLE_NORMAL,
        FontSlant::Italic => DWRITE_FONT_STYLE_ITALIC,
        FontSlant::Oblique => DWRITE_FONT_STYLE_OBLIQUE,
    }
}

fn styled_font(request: &FontRequest, style: TextStyle) -> Result<FontRequest, DirectWriteError> {
    let family = if style.is_code() {
        "Cascadia Mono"
    } else {
        request.family()
    };
    let mut styled = FontRequest::new(family, request.size())
        .map_err(|error| DirectWriteError::Windows(Arc::from(error.to_string())))?
        .with_weight(request.weight())
        .with_slant(request.slant());
    if style.is_strong() {
        styled = styled.with_weight(FontWeight::Bold);
    }
    if style.is_emphasis() {
        styled = styled.with_slant(FontSlant::Italic);
    }
    Ok(styled)
}

fn localized_string(strings: &IDWriteLocalizedStrings) -> Result<String, DirectWriteError> {
    let length = unsafe { strings.GetStringLength(0)? };
    let mut buffer =
        vec![0_u16; usize::try_from(length).map_err(|_| DirectWriteError::OffsetOverflow)? + 1];
    unsafe { strings.GetString(0, &mut buffer)? };
    buffer.truncate(usize::try_from(length).map_err(|_| DirectWriteError::OffsetOverflow)?);
    Ok(String::from_utf16_lossy(&buffer))
}

fn font_identity(font: &IDWriteFont) -> Result<(String, String), DirectWriteError> {
    let family = unsafe {
        let family = font.GetFontFamily()?;
        localized_string(&family.GetFamilyNames()?)?
    };
    let face = localized_string(&unsafe { font.GetFaceNames()? })?;
    Ok((family, face))
}

fn classify_script(text: &str) -> Script {
    let Some(ch) = text.chars().find(|ch| !ch.is_whitespace()) else {
        return Script::Common;
    };
    let value = ch as u32;
    match value {
        0x0041..=0x024f | 0x1e00..=0x1eff => Script::Latin,
        0x0600..=0x06ff | 0x0750..=0x077f | 0x08a0..=0x08ff => Script::Arabic,
        0x0900..=0x097f => Script::Devanagari,
        0x3040..=0x30ff | 0x31f0..=0x31ff => Script::Japanese,
        0x3400..=0x4dbf | 0x4e00..=0x9fff | 0xf900..=0xfaff => Script::Han,
        _ if ch.is_ascii_punctuation() || ch.is_ascii_digit() => Script::Common,
        _ => Script::Unknown,
    }
}

fn paragraph_direction(text: &str) -> DWRITE_READING_DIRECTION {
    for ch in text.chars() {
        match bidi_class(ch) {
            BidiClass::L => return DWRITE_READING_DIRECTION_LEFT_TO_RIGHT,
            BidiClass::R | BidiClass::AL => return DWRITE_READING_DIRECTION_RIGHT_TO_LEFT,
            _ => {}
        }
    }
    DWRITE_READING_DIRECTION_LEFT_TO_RIGHT
}

fn sub_source(source: TextRange, start: usize, end: usize) -> Result<TextRange, DirectWriteError> {
    let start = source
        .start()
        .checked_add(u64::try_from(start).map_err(|_| DirectWriteError::OffsetOverflow)?)
        .ok_or(DirectWriteError::OffsetOverflow)?;
    let end = source
        .start()
        .checked_add(u64::try_from(end).map_err(|_| DirectWriteError::OffsetOverflow)?)
        .ok_or(DirectWriteError::OffsetOverflow)?;
    TextRange::new(start, end).ok_or(DirectWriteError::OffsetOverflow)
}

fn analysis_boundaries(length: u32, scripts: &[ScriptSpan], bidis: &[BidiSpan]) -> Vec<u32> {
    let mut boundaries = vec![0, length];
    for span in scripts {
        boundaries.push(span.start);
        boundaries.push(span.start.saturating_add(span.len).min(length));
    }
    for span in bidis {
        boundaries.push(span.start);
        boundaries.push(span.start.saturating_add(span.len).min(length));
    }
    boundaries.sort_unstable();
    boundaries.dedup();
    boundaries
}

fn analysis_for(
    position: u32,
    scripts: &[ScriptSpan],
    bidis: &[BidiSpan],
) -> Result<(DWRITE_SCRIPT_ANALYSIS, u8), DirectWriteError> {
    let script = scripts
        .iter()
        .find(|span| position >= span.start && position < span.start.saturating_add(span.len))
        .map(|span| span.analysis)
        .ok_or(DirectWriteError::InvalidAnalysis(
            "DirectWrite did not analyze the script run",
        ))?;
    let bidi = bidis
        .iter()
        .find(|span| position >= span.start && position < span.start.saturating_add(span.len))
        .map(|span| span.level)
        .unwrap_or(0);
    Ok((script, bidi))
}

#[derive(Clone)]
pub struct DirectWriteShaper {
    factory: IDWriteFactory2,
    analyzer: IDWriteTextAnalyzer,
    collection: IDWriteFontCollection,
    fallback: IDWriteFontFallback,
    request: FontRequest,
    faces: SharedFaceTable<DirectWriteFace>,
}

impl fmt::Debug for DirectWriteShaper {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DirectWriteShaper")
            .field("request", &self.request)
            .finish_non_exhaustive()
    }
}

impl DirectWriteShaper {
    pub fn new(request: FontRequest) -> Result<Self, DirectWriteError> {
        let factory: IDWriteFactory2 = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };
        let analyzer = unsafe { factory.CreateTextAnalyzer()? };
        let fallback = unsafe { factory.GetSystemFontFallback()? };
        let mut collection = None;
        unsafe { factory.GetSystemFontCollection(&mut collection, false)? };
        let collection = collection.ok_or(DirectWriteError::InvalidAnalysis(
            "DirectWrite did not return the system font collection",
        ))?;
        Ok(Self {
            factory,
            analyzer,
            collection,
            fallback,
            request,
            faces: SharedFaceTable::new(),
        })
    }

    fn base_font(&self, request: &FontRequest) -> Result<IDWriteFont, DirectWriteError> {
        let family_name = wide_null(request.family());
        let mut index = 0_u32;
        let mut exists = BOOL(0);
        unsafe {
            self.collection.FindFamilyName(
                PCWSTR(family_name.as_ptr()),
                &mut index,
                &mut exists,
            )?;
        }
        let family = if exists.as_bool() {
            unsafe { self.collection.GetFontFamily(index)? }
        } else {
            let segoe = wide_null("Segoe UI");
            unsafe {
                self.collection
                    .FindFamilyName(PCWSTR(segoe.as_ptr()), &mut index, &mut exists)?;
            }
            if !exists.as_bool() {
                return Err(DirectWriteError::InvalidAnalysis(
                    "DirectWrite could not resolve the requested or Segoe UI family",
                ));
            }
            unsafe { self.collection.GetFontFamily(index)? }
        };
        unsafe {
            Ok(family.GetFirstMatchingFont(
                dwrite_weight(request.weight()),
                DWRITE_FONT_STRETCH_NORMAL,
                dwrite_style(request.slant()),
            )?)
        }
    }

    fn face_id(
        &self,
        font: &IDWriteFont,
        size_factor: f32,
    ) -> Result<(FontFaceId, IDWriteFontFace), DirectWriteError> {
        let (family, face_name) = font_identity(font)?;
        let native = unsafe { font.CreateFontFace()? };
        let key = format!(
            "{family}\u{1f}{face_name}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}",
            unsafe { font.GetWeight() }.0,
            unsafe { font.GetStyle() }.0,
            unsafe { font.GetStretch() }.0,
            size_factor.to_bits(),
        );
        let id = self.faces.id_for(&key, || DirectWriteFace {
            face: native.clone(),
            family: Arc::from(family.as_str()),
            face_name: Arc::from(face_name.as_str()),
            size_factor,
        })?;
        Ok((id, native))
    }

    fn analyze(
        &self,
        text: &str,
        utf16: &[u16],
    ) -> Result<(IDWriteTextAnalysisSource, Vec<ScriptSpan>, Vec<BidiSpan>), DirectWriteError> {
        let scripts = Arc::new(Mutex::new(Vec::new()));
        let bidis = Arc::new(Mutex::new(Vec::new()));
        let source: IDWriteTextAnalysisSource = ComObject::new(AnalysisSource {
            text: utf16.to_vec(),
            locale: LOCALE.to_vec(),
            direction: paragraph_direction(text),
        })
        .into_interface();
        let sink: IDWriteTextAnalysisSink = ComObject::new(AnalysisSink {
            scripts: Arc::clone(&scripts),
            bidis: Arc::clone(&bidis),
        })
        .into_interface();
        let length = u32::try_from(utf16.len()).map_err(|_| DirectWriteError::OffsetOverflow)?;
        unsafe {
            self.analyzer.AnalyzeScript(&source, 0, length, &sink)?;
            self.analyzer.AnalyzeBidi(&source, 0, length, &sink)?;
        }
        let mut scripts = scripts
            .lock()
            .map_err(|_| DirectWriteError::InvalidAnalysis("script analysis state poisoned"))?
            .clone();
        let mut bidis = bidis
            .lock()
            .map_err(|_| DirectWriteError::InvalidAnalysis("bidi analysis state poisoned"))?
            .clone();
        scripts.sort_by_key(|span| span.start);
        bidis.sort_by_key(|span| span.start);
        Ok((source, scripts, bidis))
    }

    fn shape_native(
        &self,
        text: &str,
        source: TextRange,
        style: TextStyle,
        font_request: FontRequest,
    ) -> Result<ShapedText, DirectWriteError> {
        if text.is_empty() {
            return Ok(ShapedText::new(source, Vec::new()));
        }
        let utf16: Vec<u16> = text.encode_utf16().collect();
        let utf16_map = Utf16Map::new(text);
        let (analysis_source, scripts, bidis) = self.analyze(text, &utf16)?;
        let length = u32::try_from(utf16.len()).map_err(|_| DirectWriteError::OffsetOverflow)?;
        let boundaries = analysis_boundaries(length, &scripts, &bidis);
        let family_name = wide_null(font_request.family());
        let base_font = self.base_font(&font_request)?;
        let mut runs = Vec::<(u32, GlyphRun)>::new();

        for window in boundaries.windows(2) {
            let analysis_start = window[0];
            let analysis_end = window[1];
            if analysis_start >= analysis_end {
                continue;
            }
            let (script_analysis, bidi_level) = analysis_for(analysis_start, &scripts, &bidis)?;
            let direction = if bidi_level % 2 == 1 {
                TextDirection::Rtl
            } else {
                TextDirection::Ltr
            };
            let mut position = analysis_start;
            while position < analysis_end {
                let requested = analysis_end - position;
                let mut mapped_len = 0_u32;
                let mut mapped_font = None;
                let mut mapped_scale = 1.0_f32;
                unsafe {
                    self.fallback.MapCharacters(
                        &analysis_source,
                        position,
                        requested,
                        &self.collection,
                        PCWSTR(family_name.as_ptr()),
                        dwrite_weight(font_request.weight()),
                        dwrite_style(font_request.slant()),
                        DWRITE_FONT_STRETCH_NORMAL,
                        &mut mapped_len,
                        &mut mapped_font,
                        &mut mapped_scale,
                    )?;
                }
                if !mapped_scale.is_finite() || mapped_scale <= 0.0 {
                    return Err(DirectWriteError::InvalidAnalysis(
                        "DirectWrite fallback returned an invalid scale",
                    ));
                }
                if mapped_len == 0 {
                    mapped_len = requested;
                }
                mapped_len = mapped_len.min(requested);
                let font = mapped_font.unwrap_or_else(|| base_font.clone());
                let (face_id, font_face) = self.face_id(&font, mapped_scale)?;

                let end = position
                    .checked_add(mapped_len)
                    .ok_or(DirectWriteError::OffsetOverflow)?;
                let byte_start = utf16_map
                    .byte_offset(
                        usize::try_from(position).map_err(|_| DirectWriteError::OffsetOverflow)?,
                    )
                    .ok_or(DirectWriteError::InvalidAnalysis(
                        "DirectWrite fallback split a surrogate pair",
                    ))?;
                let byte_end = utf16_map
                    .byte_offset(
                        usize::try_from(end).map_err(|_| DirectWriteError::OffsetOverflow)?,
                    )
                    .ok_or(DirectWriteError::InvalidAnalysis(
                        "DirectWrite fallback split a surrogate pair",
                    ))?;
                let run_text =
                    text.get(byte_start..byte_end)
                        .ok_or(DirectWriteError::InvalidAnalysis(
                            "DirectWrite fallback did not land on UTF-8 boundaries",
                        ))?;
                let run_source = sub_source(source, byte_start, byte_end)?;
                let run_utf16 = &utf16[usize::try_from(position)
                    .map_err(|_| DirectWriteError::OffsetOverflow)?
                    ..usize::try_from(end).map_err(|_| DirectWriteError::OffsetOverflow)?];
                let text_len =
                    u32::try_from(run_utf16.len()).map_err(|_| DirectWriteError::OffsetOverflow)?;
                let max_glyphs = text_len
                    .checked_mul(4)
                    .and_then(|value| value.checked_add(16))
                    .ok_or(DirectWriteError::OffsetOverflow)?
                    .max(16);
                let mut cluster_map = vec![0_u16; run_utf16.len()];
                let mut text_props =
                    vec![DWRITE_SHAPING_TEXT_PROPERTIES::default(); run_utf16.len()];
                let mut glyph_ids = vec![
                    0_u16;
                    usize::try_from(max_glyphs)
                        .map_err(|_| DirectWriteError::OffsetOverflow)?
                ];
                let mut glyph_props =
                    vec![DWRITE_SHAPING_GLYPH_PROPERTIES::default(); glyph_ids.len()];
                let mut actual_glyphs = 0_u32;
                unsafe {
                    self.analyzer.GetGlyphs(
                        PCWSTR(run_utf16.as_ptr()),
                        text_len,
                        &font_face,
                        false,
                        direction == TextDirection::Rtl,
                        &script_analysis,
                        PCWSTR(LOCALE.as_ptr()),
                        None::<&IDWriteNumberSubstitution>,
                        None,
                        None,
                        0,
                        max_glyphs,
                        cluster_map.as_mut_ptr(),
                        text_props.as_mut_ptr(),
                        glyph_ids.as_mut_ptr(),
                        glyph_props.as_mut_ptr(),
                        &mut actual_glyphs,
                    )?;
                }
                let actual =
                    usize::try_from(actual_glyphs).map_err(|_| DirectWriteError::OffsetOverflow)?;
                glyph_ids.truncate(actual);
                glyph_props.truncate(actual);
                let mut advances = vec![0_f32; actual];
                let mut offsets = vec![DWRITE_GLYPH_OFFSET::default(); actual];
                unsafe {
                    self.analyzer.GetGlyphPlacements(
                        PCWSTR(run_utf16.as_ptr()),
                        cluster_map.as_ptr(),
                        text_props.as_mut_ptr(),
                        text_len,
                        glyph_ids.as_ptr(),
                        glyph_props.as_ptr(),
                        actual_glyphs,
                        &font_face,
                        font_request.size() * mapped_scale,
                        false,
                        direction == TextDirection::Rtl,
                        &script_analysis,
                        PCWSTR(LOCALE.as_ptr()),
                        None,
                        None,
                        0,
                        advances.as_mut_ptr(),
                        offsets.as_mut_ptr(),
                    )?;
                }
                let offsets: Vec<(f32, f32)> = offsets
                    .into_iter()
                    .map(|offset| (offset.advanceOffset, -offset.ascenderOffset))
                    .collect();
                let script = classify_script(run_text);
                let run = assemble_run(
                    face_id,
                    run_text,
                    run_source,
                    style,
                    direction,
                    script,
                    ShapedArrays {
                        cluster_map: &cluster_map,
                        glyph_ids: &glyph_ids,
                        advances: &advances,
                        offsets: &offsets,
                    },
                )?;
                runs.push((position, run));
                position = end;
            }
        }
        runs.sort_by_key(|(start, _)| *start);
        Ok(ShapedText::new(
            source,
            runs.into_iter().map(|(_, run)| run).collect(),
        ))
    }
}

impl ShapingProvider for DirectWriteShaper {
    type Error = ShapeError;

    fn font_metrics(
        &self,
        face: FontFaceId,
        scale: f32,
    ) -> Result<Option<(f32, f32)>, Self::Error> {
        let metrics = (|| -> Result<_, DirectWriteError> {
            let entry = self
                .faces
                .with_entry(face, Clone::clone)?
                .ok_or(DirectWriteError::UnknownFace(face))?;
            let mut native = DWRITE_FONT_METRICS::default();
            unsafe { entry.face.GetMetrics(&mut native) };
            if native.designUnitsPerEm == 0 || !scale.is_finite() || scale <= 0.0 {
                return Err(DirectWriteError::InvalidAnalysis(
                    "invalid font metrics scale",
                ));
            }
            let factor = self.request.size() * entry.size_factor * scale
                / f32::from(native.designUnitsPerEm);
            Ok((
                f32::from(native.ascent) * factor,
                f32::from(native.descent) * factor,
            ))
        })()
        .map_err(|error| ShapeError::Backend(Arc::from(error.to_string())))?;
        Ok(Some(metrics))
    }

    fn shape(
        &self,
        text: &str,
        source: TextRange,
        style: TextStyle,
    ) -> Result<ShapedText, Self::Error> {
        let font = styled_font(&self.request, style)
            .map_err(|error| ShapeError::Backend(Arc::from(error.to_string())))?;
        self.shape_native(text, source, style, font)
            .map_err(|error| ShapeError::Backend(Arc::from(error.to_string())))
    }

    fn shape_scaled(
        &self,
        text: &str,
        source: TextRange,
        style: TextStyle,
        scale: f32,
    ) -> Result<ShapedText, Self::Error> {
        if !scale.is_finite() || scale <= 0.0 {
            return Err(ShapeError::Backend(Arc::from(
                "invalid DirectWrite shaping scale",
            )));
        }
        let base = styled_font(&self.request, style)
            .map_err(|error| ShapeError::Backend(Arc::from(error.to_string())))?;
        let font = FontRequest::new(base.family(), base.size() * scale)
            .map_err(|error| ShapeError::Backend(Arc::from(error.to_string())))?
            .with_weight(base.weight())
            .with_slant(base.slant());
        self.shape_native(text, source, style, font)
            .map_err(|error| ShapeError::Backend(Arc::from(error.to_string())))
    }
}

#[derive(Clone)]
pub struct DirectWriteGlyphRasterizer {
    factory: IDWriteFactory2,
    faces: SharedFaceTable<DirectWriteFace>,
    metrics: Arc<Mutex<FontMetricsCache>>,
}

impl fmt::Debug for DirectWriteGlyphRasterizer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("DirectWriteGlyphRasterizer(..)")
    }
}

impl DirectWriteGlyphRasterizer {
    fn color_bitmap(
        &self,
        run: &DWRITE_GLYPH_RUN,
    ) -> Result<Option<(GlyphBitmap, RECT)>, DirectWriteError> {
        let layers = match unsafe {
            self.factory.TranslateColorGlyphRun(
                0.0,
                0.0,
                run,
                None,
                DWRITE_MEASURING_MODE_NATURAL,
                None,
                0,
            )
        } {
            Ok(layers) => layers,
            Err(error) if error.code() == DWRITE_E_NOCOLOR => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let mut bitmaps = Vec::new();
        let mut extent: Option<RECT> = None;
        while unsafe { layers.MoveNext()? }.as_bool() {
            let current = unsafe { layers.GetCurrentRun()? };
            if current.is_null() {
                return Err(DirectWriteError::InvalidAnalysis("null color glyph layer"));
            }
            let layer = unsafe { &*current };
            let analysis = unsafe {
                self.factory.CreateGlyphRunAnalysis(
                    &layer.glyphRun,
                    None,
                    DWRITE_RENDERING_MODE_NATURAL_SYMMETRIC,
                    DWRITE_MEASURING_MODE_NATURAL,
                    DWRITE_GRID_FIT_MODE_DEFAULT,
                    DWRITE_TEXT_ANTIALIAS_MODE_GRAYSCALE,
                    layer.baselineOriginX,
                    layer.baselineOriginY,
                )?
            };
            let bounds = unsafe { analysis.GetAlphaTextureBounds(DWRITE_TEXTURE_ALIASED_1x1)? };
            let width = (bounds.right - bounds.left).max(0) as usize;
            let height = (bounds.bottom - bounds.top).max(0) as usize;
            let count = width
                .checked_mul(height)
                .filter(|count| *count <= 16 * 1024 * 1024)
                .ok_or(DirectWriteError::OffsetOverflow)?;
            if count == 0 {
                continue;
            }
            let mut coverage = vec![0; count];
            unsafe {
                analysis.CreateAlphaTexture(DWRITE_TEXTURE_ALIASED_1x1, &bounds, &mut coverage)?;
            }
            extent = Some(match extent {
                None => bounds,
                Some(old) => RECT {
                    left: old.left.min(bounds.left),
                    top: old.top.min(bounds.top),
                    right: old.right.max(bounds.right),
                    bottom: old.bottom.max(bounds.bottom),
                },
            });
            // 0xffff denotes the application's foreground. Color glyphs share
            // a theme-independent atlas; their default foreground is black.
            let color = if layer.paletteIndex == 0xffff {
                [0.0, 0.0, 0.0, 1.0]
            } else {
                [
                    layer.runColor.r,
                    layer.runColor.g,
                    layer.runColor.b,
                    layer.runColor.a,
                ]
            };
            bitmaps.push((bounds, coverage, color));
        }
        let Some(bounds) = extent else {
            return Ok(None);
        };
        let width = (bounds.right - bounds.left) as u32;
        let height = (bounds.bottom - bounds.top) as u32;
        let bytes = (width as usize)
            .checked_mul(height as usize)
            .and_then(|n| n.checked_mul(4))
            .filter(|n| *n <= 64 * 1024 * 1024)
            .ok_or(DirectWriteError::OffsetOverflow)?;
        let mut rgba = vec![0_u8; bytes];
        for (layer, coverage, color) in bitmaps {
            let layer_width = (layer.right - layer.left) as usize;
            for (index, coverage) in coverage.into_iter().enumerate() {
                let x = (layer.left - bounds.left) as usize + index % layer_width;
                let y = (layer.top - bounds.top) as usize + index / layer_width;
                let offset = (y * width as usize + x) * 4;
                let alpha = coverage as f32 / 255.0 * color[3].clamp(0.0, 1.0);
                for channel in 0..4 {
                    let source = if channel == 3 {
                        alpha
                    } else {
                        color[channel].clamp(0.0, 1.0) * alpha
                    };
                    rgba[offset + channel] = (source * 255.0
                        + rgba[offset + channel] as f32 * (1.0 - alpha))
                        .round()
                        .clamp(0.0, 255.0) as u8;
                }
            }
        }
        Ok(Some((
            GlyphBitmap::new_rgba(width, height, width * 4, rgba)?,
            bounds,
        )))
    }

    fn face(&self, id: FontFaceId) -> Result<DirectWriteFace, DirectWriteError> {
        self.faces
            .with_entry(id, Clone::clone)?
            .ok_or(DirectWriteError::UnknownFace(id))
    }

    fn design_metrics(
        face: &IDWriteFontFace,
        glyph: u16,
    ) -> Result<DWRITE_GLYPH_METRICS, DirectWriteError> {
        let mut metrics = DWRITE_GLYPH_METRICS::default();
        unsafe { face.GetDesignGlyphMetrics(&glyph, 1, &mut metrics, false)? };
        Ok(metrics)
    }
}

impl GlyphRasterizer for DirectWriteGlyphRasterizer {
    type Error = DirectWriteError;

    fn font_metrics(&self, key: FontMetricKey) -> Result<FontMetricsSnapshot, Self::Error> {
        if let Some(metrics) = self
            .metrics
            .lock()
            .map_err(|_| DirectWriteError::InvalidAnalysis("font metrics cache poisoned"))?
            .get(key)
        {
            return Ok(metrics);
        }
        let entry = self.face(key.face())?;
        let face = &entry.face;
        let mut native = DWRITE_FONT_METRICS::default();
        unsafe { face.GetMetrics(&mut native) };
        let units = u32::from(native.designUnitsPerEm);
        if units == 0 {
            return Err(DirectWriteError::InvalidAnalysis(
                "DirectWrite font has zero design units per em",
            ));
        }
        let scale = key.size() * entry.size_factor / units as f32;
        let metrics = FontMetricsSnapshot::new(
            f32::from(native.ascent) * scale,
            f32::from(native.descent) * scale,
            f32::from(native.lineGap) * scale,
            units,
        )?;
        self.metrics
            .lock()
            .map_err(|_| DirectWriteError::InvalidAnalysis("font metrics cache poisoned"))?
            .insert(key, metrics);
        Ok(metrics)
    }

    fn rasterize(&self, key: GlyphRasterKey) -> Result<RasterizedGlyph, Self::Error> {
        let glyph = u16::try_from(key.glyph().get())
            .map_err(|_| DirectWriteError::InvalidAnalysis("DirectWrite glyph id exceeds u16"))?;
        let entry = self.face(key.face())?;
        let face = &entry.face;
        let mut font_metrics = DWRITE_FONT_METRICS::default();
        unsafe { face.GetMetrics(&mut font_metrics) };
        let units = u32::from(font_metrics.designUnitsPerEm);
        if units == 0 {
            return Err(DirectWriteError::InvalidAnalysis(
                "DirectWrite font has zero design units per em",
            ));
        }
        let physical_size = key.size() * entry.size_factor * key.raster_scale();
        let design = Self::design_metrics(face, glyph)?;
        let physical_advance = design.advanceWidth as f32 * physical_size / units as f32;
        let glyphs = [glyph];
        let advances = [physical_advance];
        let offsets = [DWRITE_GLYPH_OFFSET::default()];
        let mut run = DWRITE_GLYPH_RUN {
            fontFace: ManuallyDrop::new(Some(face.clone())),
            fontEmSize: physical_size,
            glyphCount: 1,
            glyphIndices: glyphs.as_ptr(),
            glyphAdvances: advances.as_ptr(),
            glyphOffsets: offsets.as_ptr(),
            isSideways: BOOL(0),
            bidiLevel: 0,
        };
        let color = self.color_bitmap(&run);
        if !matches!(color, Ok(None)) {
            unsafe { ManuallyDrop::drop(&mut run.fontFace) };
            let (bitmap, bounds) =
                color?.ok_or(DirectWriteError::InvalidAnalysis("missing color bitmap"))?;
            let metrics =
                GlyphMetrics::new(bounds.left as f32, -(bounds.top as f32), physical_advance)?;
            return Ok(RasterizedGlyph::new(key, metrics, bitmap));
        }
        let analysis = unsafe {
            self.factory.CreateGlyphRunAnalysis(
                &run,
                None,
                DWRITE_RENDERING_MODE_NATURAL_SYMMETRIC,
                DWRITE_MEASURING_MODE_NATURAL,
                DWRITE_GRID_FIT_MODE_DEFAULT,
                DWRITE_TEXT_ANTIALIAS_MODE_GRAYSCALE,
                0.0,
                0.0,
            )
        };
        // CreateGlyphRunAnalysis 已经完成对输入 run 的消费；后面的 bounds/texture
        // 查询即使失败，也不能让这个临时 COM face 因 ManuallyDrop 而泄漏。
        unsafe { ManuallyDrop::drop(&mut run.fontFace) };
        let analysis = analysis?;
        // Yu 的普通文字 atlas 在共享层保存的是单通道覆盖率（进入 atlas 后扩成
        // premultiplied RGBA）。不要先申请 ClearType 3x1 再把 RGB 子像素压成
        // 一个 alpha；那会把 Windows 私有的子像素语义带进共享 seam。
        let bounds: RECT = unsafe { analysis.GetAlphaTextureBounds(DWRITE_TEXTURE_ALIASED_1x1)? };
        let width = (bounds.right - bounds.left).max(0) as u32;
        let height = (bounds.bottom - bounds.top).max(0) as u32;
        let pixel_count = usize::try_from(width)
            .ok()
            .and_then(|w| usize::try_from(height).ok()?.checked_mul(w))
            .ok_or(DirectWriteError::OffsetOverflow)?;
        let mut coverage = vec![0_u8; pixel_count];
        if !coverage.is_empty() {
            unsafe {
                analysis.CreateAlphaTexture(DWRITE_TEXTURE_ALIASED_1x1, &bounds, &mut coverage)?;
            }
        }
        let bitmap = GlyphBitmap::new(width, height, width, coverage)?;
        let metrics =
            GlyphMetrics::new(bounds.left as f32, -(bounds.top as f32), physical_advance)?;
        Ok(RasterizedGlyph::new(key, metrics, bitmap))
    }
}

impl RasterizingShaper for DirectWriteShaper {
    type Rasterizer = DirectWriteGlyphRasterizer;

    fn font_request(&self) -> &FontRequest {
        &self.request
    }

    fn rasterizer(&self) -> Self::Rasterizer {
        DirectWriteGlyphRasterizer {
            factory: self.factory.clone(),
            faces: self.faces.clone(),
            metrics: Arc::new(Mutex::new(FontMetricsCache::new())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yu_core::{ByteOffset, GlyphRun, TextStyle};

    fn source(text: &str) -> TextRange {
        TextRange::new(ByteOffset::ZERO, ByteOffset::new(text.len() as u64)).expect("valid source")
    }

    #[test]
    fn direct_write_conforms_to_the_shared_shaping_contract() {
        let request = FontRequest::new("Segoe UI", 13.0).expect("font request");
        let shaper = DirectWriteShaper::new(request).expect("DirectWrite");
        let violations = yu_core::shaping_conformance::audit(&shaper);
        assert!(violations.is_empty(), "{violations:#?}");
    }

    #[test]
    fn direct_write_shapes_rtl_and_complex_script_runs() {
        let request = FontRequest::new("Segoe UI", 16.0).expect("font request");
        let shaper = DirectWriteShaper::new(request).expect("DirectWrite");

        for text in [
            "שלום",
            "مرحبا",
            "हिन्दी",
            "שָׁלוֹם",
            "السَّلَامُ",
            "abc שלום 😀 مرحبا xyz",
        ] {
            let shaped = ShapingProvider::shape(&shaper, text, source(text), TextStyle::Plain)
                .unwrap_or_else(|error| panic!("{text:?}: {error}"));
            assert!(!shaped.runs().is_empty(), "{text:?}");
            assert!(
                shaped
                    .runs()
                    .iter()
                    .flat_map(GlyphRun::glyphs)
                    .next()
                    .is_some(),
                "{text:?}"
            );
        }

        let rtl = ShapingProvider::shape(&shaper, "שלום", source("שלום"), TextStyle::Plain)
            .expect("Hebrew");
        assert!(
            rtl.runs()
                .iter()
                .any(|run| run.direction() == TextDirection::Rtl),
            "{rtl:#?}"
        );
    }

    #[test]
    fn rasterizer_resolves_faces_minted_by_the_shaper() {
        let request = FontRequest::new("Segoe UI", 16.0).expect("font request");
        let shaper = DirectWriteShaper::new(request).expect("DirectWrite");
        let rasterizer = shaper.rasterizer();
        let text = "A中";
        let shaped =
            ShapingProvider::shape(&shaper, text, source(text), TextStyle::Plain).expect("shape");

        for run in shaped.runs() {
            let metrics =
                rasterizer.font_metrics(FontMetricKey::new(run.face(), 16.0).expect("key"));
            assert!(metrics.is_ok(), "face {:?}: {metrics:?}", run.face());
            for glyph in run.glyphs() {
                let raster = rasterizer
                    .rasterize(
                        GlyphRasterKey::new(run.face(), glyph.id(), 16.0).expect("glyph key"),
                    )
                    .unwrap_or_else(|error| panic!("face {:?}: {error}", run.face()));
                assert!(raster.metrics().advance_x().is_finite());
            }
        }
    }

    #[test]
    fn paragraph_direction_uses_the_first_strong_character() {
        assert_eq!(
            paragraph_direction("  שלום abc"),
            DWRITE_READING_DIRECTION_RIGHT_TO_LEFT
        );
        assert_eq!(
            paragraph_direction("123 abc שלום"),
            DWRITE_READING_DIRECTION_LEFT_TO_RIGHT
        );
        assert_eq!(
            paragraph_direction("123!?"),
            DWRITE_READING_DIRECTION_LEFT_TO_RIGHT
        );
    }

    #[test]
    fn native_color_emoji_retains_palette_and_scales_with_dpi() {
        let shaper = DirectWriteShaper::new(FontRequest::new("Segoe UI", 24.0).expect("font"))
            .expect("DirectWrite");
        let rasterizer = shaper.rasterizer();
        for text in ["😀", "👩‍💻", "👍🏽"] {
            let range = TextRange::new(
                yu_core::ByteOffset::ZERO,
                yu_core::ByteOffset::new(text.len() as u64),
            )
            .expect("range");
            let shaped = shaper.shape(text, range, TextStyle::Plain).expect("shape");
            let mut colorful = 0;
            for run in shaped.runs() {
                for glyph in run.glyphs() {
                    let key = GlyphRasterKey::new(run.face(), glyph.id(), 24.0).expect("key");
                    let raster = rasterizer.rasterize(key).expect("raster");
                    let bitmap = raster.bitmap();
                    if bitmap.is_color() {
                        assert!(
                            bitmap
                                .pixels()
                                .as_chunks::<4>()
                                .0
                                .iter()
                                .any(|p| p[3] > 32 && (p[0] != p[1] || p[1] != p[2])),
                            "palette missing: {text}"
                        );
                        assert!(
                            bitmap
                                .pixels()
                                .as_chunks::<4>()
                                .0
                                .iter()
                                .all(|p| p[..3].iter().all(|c| *c <= p[3])),
                            "not premultiplied"
                        );
                        let high = rasterizer
                            .rasterize(key.with_raster_scale(2.0).expect("scale"))
                            .expect("2x raster");
                        assert!(high.bitmap().is_color());
                        assert!(high.bitmap().width() >= bitmap.width() * 3 / 2);
                        assert!(high.bitmap().height() >= bitmap.height() * 3 / 2);
                        colorful += 1;
                    }
                }
            }
            assert!(colorful > 0, "No native color glyph for {text}");
        }
    }
}
