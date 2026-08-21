//! `yu-layout` 能消费真实字体后端产出的度量与 shaped run。
//!
//! 这两个用例此前住在 `yu-font` 里，于是字体 crate 的测试必须依赖 layout 与
//! projection——`yu-font` 只依赖 `yu-core`（不变量 E2）这条约束在测试目标下就
//! 不成立了。断言的内容是「布局能不能消费一个字体后端」，属于消费侧契约，
//! 因此放在 layout 这边。

use std::sync::Arc;

use yu_core::{ByteOffset, TextRange};
use yu_font::{
    FontCoverage, FontDatabase, FontFaceSpec, FontMetrics, FontRequest, FontShaper, UnicodeRange,
};
use yu_layout::{LayoutConfig, LayoutSnapshot};
use yu_projection::Projection;
use yu_text::TextBuffer;

fn database() -> Arc<FontDatabase> {
    let mut database = FontDatabase::new();
    database
        .register(
            FontFaceSpec::new("Latin", 0.5).with_coverage(FontCoverage::Ranges(vec![
                UnicodeRange::new('a', 'z').expect("range should be valid"),
            ])),
        )
        .expect("Latin face should register");
    database
        .register(FontFaceSpec::new("Fallback", 1.0))
        .expect("fallback face should register");
    Arc::new(database)
}

#[test]
fn font_metrics_feed_the_existing_layout_contract() {
    let source = "ab";
    let buffer = TextBuffer::new(source);
    let snapshot = buffer.snapshot();
    let projection = Projection::inline(
        &snapshot,
        TextRange::new(ByteOffset::ZERO, ByteOffset::new(2)).expect("range should be valid"),
    )
    .expect("projection should build");
    let metrics = FontMetrics::new(
        database(),
        FontRequest::new("Latin", 2.0).expect("request should be valid"),
    )
    .expect("metrics should build");
    let layout = LayoutSnapshot::from_projection_with_metrics(
        &projection,
        LayoutConfig::new(2.0, 1.0),
        &metrics,
    )
    .expect("layout should consume font metrics");
    assert_eq!(layout.lines().len(), 1);
    assert_eq!(layout.lines()[0].width(), 2.0);
    assert_eq!(layout.clusters().len(), 2);
}

#[test]
fn font_shaper_feeds_shaping_aware_layout() {
    let source = "ab";
    let buffer = TextBuffer::new(source);
    let snapshot = buffer.snapshot();
    let projection = Projection::inline(
        &snapshot,
        TextRange::new(ByteOffset::ZERO, ByteOffset::new(2)).expect("range should be valid"),
    )
    .expect("projection should build");
    let shaper = FontShaper::new(
        database(),
        FontRequest::new("Latin", 2.0).expect("request should be valid"),
    )
    .expect("shaper should build");
    let layout = LayoutSnapshot::from_projection_with_shaper(
        &projection,
        LayoutConfig::new(2.0, 1.0),
        &shaper,
    )
    .expect("layout should consume shaped runs");

    assert_eq!(layout.lines().len(), 1);
    assert_eq!(layout.lines()[0].width(), 2.0);
    assert_eq!(layout.clusters().len(), 2);
}
