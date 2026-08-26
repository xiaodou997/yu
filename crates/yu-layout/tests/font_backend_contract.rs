//! `yu-layout` 能消费真实字体后端产出的度量与 shaped run。
//!
//! 这两个用例此前住在 `yu-font` 里，于是字体 crate 的测试必须依赖 layout 与
//! projection——`yu-font` 只依赖 `yu-core`（不变量 E2）这条约束在测试目标下就
//! 不成立了。断言的内容是「布局能不能消费一个字体后端」，属于消费侧契约，
//! 因此放在 layout 这边。
//!
//! S5 之后布局的输入是「视觉文本 + 样式区间」，两个用例随之不再构造投影。

use std::sync::Arc;

use yu_core::{StyleId, VisualOffset, VisualRange};
use yu_font::{
    FontCoverage, FontDatabase, FontFaceSpec, FontMetrics, FontRequest, FontShaper, UnicodeRange,
};
use yu_layout::{
    BlockLayout, LayoutConfig, LayoutInput, NoLineStyles, NoWidgets, StyledRun, UniformStyleTable,
};

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

fn runs(text: &str) -> Vec<StyledRun> {
    let visual = VisualRange::new(
        VisualOffset::ZERO,
        VisualOffset::try_from(text.len()).expect("short"),
    )
    .expect("ordered");
    vec![StyledRun::new(visual, StyleId(0))]
}

#[test]
fn font_metrics_feed_the_existing_layout_contract() {
    let text = "ab";
    let metrics = FontMetrics::new(
        database(),
        FontRequest::new("Latin", 2.0).expect("request should be valid"),
    )
    .expect("metrics should build");
    let layout = BlockLayout::build(
        LayoutInput::new(text, &runs(text)),
        LayoutConfig::new(2.0, 1.0),
        &UniformStyleTable::default(),
        &metrics,
    )
    .expect("layout should consume font metrics");
    assert_eq!(layout.lines().len(), 1);
    assert_eq!(layout.lines()[0].width(), 2.0);
    assert_eq!(layout.clusters().len(), 2);
}

#[test]
fn font_shaper_feeds_shaping_aware_layout() {
    let text = "ab";
    let shaper = FontShaper::new(
        database(),
        FontRequest::new("Latin", 2.0).expect("request should be valid"),
    )
    .expect("shaper should build");
    let layout = BlockLayout::build_shaped(
        LayoutInput::new(text, &runs(text)),
        LayoutConfig::new(2.0, 1.0),
        &UniformStyleTable::default(),
        &NoWidgets,
        &NoLineStyles,
        &shaper,
    )
    .expect("layout should consume shaped runs");

    assert_eq!(layout.lines().len(), 1);
    assert_eq!(layout.lines()[0].width(), 2.0);
    assert_eq!(layout.clusters().len(), 2);
    // 字形与簇一一对应：一个后备字面上的 grapheme 也得画出来。
    assert_eq!(layout.glyphs().len(), 2);
}
