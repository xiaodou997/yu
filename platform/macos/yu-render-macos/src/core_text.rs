//! macOS 这一侧的视口帧准备：把泛型的 [`ViewportFrameBuilder`] 定在
//! `CoreTextShaper` 上。
//!
//! 准备逻辑本身（走一遍可见字形栅格化进 CPU atlas、发布一帧）住在
//! `yu-workspace`——它一个 CoreText 调用都没有。这里只剩下两件事：说出这个
//! 平台用的是哪个 shaper，以及拿真的 CoreText 把整条路跑一遍。后者是这个
//! 模块存在的理由：`yu-workspace` 的用例只能用 mock 后端，而
//! 「真 CoreText 排出来的字形真的栅格化得进 atlas」只有在这一层问得出来。

use yu_font_macos::{CoreTextRasterError, CoreTextShaper};
use yu_workspace::{ViewportFrameBuildError, ViewportFrameBuilder};

/// CoreText 后端的视口帧 builder。
pub type CoreTextViewportFrameBuilder = ViewportFrameBuilder<CoreTextShaper>;

/// 上面那个 builder 会返回的错误。
pub type CoreTextViewportFrameError = ViewportFrameBuildError<CoreTextRasterError>;

#[cfg(test)]
mod tests {
    use super::*;
    use yu_core::{ByteOffset, TextRange, Utf16Offset, Utf16Range};
    use yu_editor::{EditorCommand, EditorDocument, ViewportConfig, ViewportSpan};
    use yu_font::{FontRequest, GlyphAtlasConfig};
    use yu_scene::{Rect, Rgba8};
    use yu_workspace::ViewportRenderConfig;

    #[test]
    fn core_text_builder_reuses_atlas_and_render_upload_state() {
        let font_size = 16.0;
        let request = FontRequest::new(".SFNS-Regular", font_size).expect("font request");
        let shaper = CoreTextShaper::from_system_ui(request).expect("CoreText shaper");
        let metrics = shaper.viewport_metrics("A羽🙂").expect("CoreText metrics");
        let mut document = EditorDocument::new("# 羽🙂\n\nhello **world**");
        document
            .set_viewport_config(ViewportConfig::new(
                yu_editor::LayoutConfig::new(320.0, metrics.line_height()),
                28.0,
                0.0,
            ))
            .expect("viewport config");
        let config = ViewportRenderConfig::new(
            ViewportSpan::new(0.0, 240.0),
            font_size,
            Rect::new(0.0, 0.0, 320.0, 480.0).expect("scene viewport"),
            Rgba8::black(),
        );
        let mut builder =
            CoreTextViewportFrameBuilder::with_shaper(shaper, config, GlyphAtlasConfig::default())
                .expect("frame builder");

        let first = builder.publish(&mut document).expect("first publication");
        assert_eq!(first.revision(), document.revision());
        assert!(!first.frame().plan().commands().is_empty());
        assert!(!first.frame().plan().uploads().is_empty());
        let page_count = builder.atlas_page_count();
        let glyph_count = builder.atlas_glyph_count();
        assert!(page_count > 0);
        assert!(glyph_count > 0);
        assert!(builder.atlas_bytes() > 0);

        let second = builder.publish(&mut document).expect("cached publication");
        assert_eq!(second.revision(), first.revision());
        assert_eq!(second.serial(), first.serial() + 1);
        assert!(second.frame().plan().uploads().is_empty());
        assert_eq!(builder.atlas_page_count(), page_count);
        assert_eq!(builder.atlas_glyph_count(), glyph_count);

        let source = document.snapshot();
        let composition_start = source.as_str().find("world").expect("composition target");
        let composition_end = composition_start + "world".len();
        document
            .begin_composition(
                TextRange::new(
                    ByteOffset::new(composition_start as u64),
                    ByteOffset::new(composition_end as u64),
                )
                .expect("composition range"),
                "日本🙂",
                Utf16Range::empty(Utf16Offset::new(2)),
            )
            .expect("composition");
        let composed = builder
            .publish(&mut document)
            .expect("composition publication");
        assert_eq!(composed.revision(), document.revision());
        assert_ne!(
            composed.frame().plan().commands().len(),
            second.frame().plan().commands().len()
        );
        assert!(builder.atlas_glyph_count() >= glyph_count);

        assert!(document.cancel_composition());
        let cancelled = builder.publish(&mut document).expect("cancel publication");
        assert_eq!(cancelled.revision(), document.revision());
        assert_eq!(document.composition(), None);

        document
            .execute(EditorCommand::insert_text("界"))
            .expect("document edit");
        let third = builder.publish(&mut document).expect("edited publication");
        assert_eq!(third.revision(), document.revision());
        assert!(!third.frame().plan().uploads().is_empty());
        assert!(builder.atlas_glyph_count() > glyph_count);
    }
}
