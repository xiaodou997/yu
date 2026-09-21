//! `![替代文字](目标)` 与它的引用形式。
//!
//! 图片是一个 widget（第 3 节的对照表）：不聚焦时整段 `![替代](目标)` 由
//! [`Decoration::Widget`] 覆盖，从视觉文本里消失，位置上留一个盒子。盒子
//! **多大**不在这里——那要资源解码后的固有尺寸与 `LayoutConfig`，是
//! `yu-editor` 与 workspace 的事。这里只说「这一段是一张图，它的替代文字
//! 与目标各在哪」。
//!
//! # 光标进来时 widget 让位
//!
//! 与行内语法的定界符同一条规则（[`reveals`]）：光标碰到这一段时整段源码
//! 原样露出来，可编辑。不变量 D7 要求 widget 的资源失败时「保留可编辑的
//! 源码回退」——回退就是这一条，不是第二套呈现。
//!
//! [`Decoration::Widget`]: yu_decoration::Decoration::Widget

use yu_core::{TextAttrs, TextRange, TextStyle, WidgetSide};
use yu_syntax::NodeKind;

use super::SyntaxNode;
use super::{
    BlockContext, BlockWidget, DelimitedSpan, Extension, ExtensionOutput, ImageSpan, reveals,
};

pub struct Image;

impl Extension for Image {
    fn name(&self) -> &'static str {
        "image"
    }

    fn decorate(&self, cx: &BlockContext<'_>, out: &mut ExtensionOutput) {
        for image in collect_node_images(cx.nodes(), cx.source, false) {
            if reveals(cx.active(), image.source()) {
                let style = out.style(TextAttrs::new(TextStyle::Plain));
                out.mark(image.label(), style);
                continue;
            }
            if image.reference().is_some_and(|label| !cx.resolves(label)) {
                continue;
            }
            let widget = out.widget(BlockWidget::Image(image));
            out.place_widget(image.source(), widget, WidgetSide::Before);
        }
    }
}

fn child_range(node: SyntaxNode<'_>, kind: NodeKind) -> Option<TextRange> {
    node.children()
        .find(|child| child.kind() == kind)
        .map(SyntaxNode::range)
}

/// The semantic image catalog uses the same delimiter interpretation as the
/// decoration extension, without populating layout or decoration caches.
pub fn image_spans(
    document: &crate::MarkdownDocument,
    source: &yu_text::TextSnapshot,
    range: Option<TextRange>,
) -> Vec<ImageSpan> {
    let Some(tree) = document.tree() else {
        return Vec::new();
    };
    let root = SyntaxNode::new(tree, 0);
    let nodes = match range {
        Some(range) => root.descendants_in(range),
        None => root.descendants(),
    };
    let mut images = collect_node_images(nodes, source, true);
    for region in &document.html_regions().regions {
        if range.is_some_and(|range| {
            region.source.end() <= range.start() || region.source.start() >= range.end()
        }) {
            continue;
        }
        if let Ok(model) = &region.model {
            images.extend(
                model
                    .image_spans(source.as_str())
                    .into_iter()
                    .filter(|image| {
                        range.is_none_or(|range| {
                            image.source().start() >= range.start()
                                && image.source().end() <= range.end()
                        })
                    }),
            );
        }
    }
    images.sort_by_key(|image| image.source().start());
    images
}

fn collect_node_images<'a>(
    nodes: impl Iterator<Item = SyntaxNode<'a>>,
    source: &yu_text::TextSnapshot,
    skip_blocks: bool,
) -> Vec<ImageSpan> {
    let mut images = Vec::new();
    let mut stack: Vec<(String, bool)> = Vec::new();
    for node in nodes {
        if node.kind() == NodeKind::HtmlBlock && skip_blocks {
            continue;
        }
        if node.kind() == NodeKind::HtmlTag {
            let Some(raw) = source
                .as_str()
                .get(node.start() as usize..node.end() as usize)
            else {
                continue;
            };
            let Ok(tag) =
                crate::html::HtmlTag::parse(raw, yu_core::ByteOffset::new(u64::from(node.start())))
            else {
                continue;
            };
            if tag.closing {
                if stack.pop().is_some_and(|(name, _)| name != tag.name) {
                    stack.clear();
                }
                continue;
            }
            let allowed = stack.last().is_none_or(|(_, allowed)| *allowed)
                && tag.resolve_attributes(source.as_str()).is_ok();
            if !tag.self_closing && !tag.is_void() {
                stack.push((tag.name.clone(), allowed));
            }
            if !allowed {
                continue;
            }
        } else if stack.last().is_some_and(|(_, allowed)| !allowed) {
            continue;
        }
        images.extend(node_images(node, source));
    }
    images
}

fn image_span(node: SyntaxNode<'_>) -> Option<ImageSpan> {
    if node.kind() != NodeKind::Image {
        return None;
    }
    let span = DelimitedSpan::of(node, |kind| kind == NodeKind::LinkMark)?;
    Some(ImageSpan::new(
        node.range(),
        span.content,
        child_range(node, NodeKind::Url),
        span.reference_label(node),
    ))
}

fn node_images(node: SyntaxNode<'_>, source: &yu_text::TextSnapshot) -> Vec<ImageSpan> {
    if let Some(image) = image_span(node) {
        return vec![image];
    }
    if !matches!(node.kind(), NodeKind::HtmlTag | NodeKind::HtmlBlock) {
        return Vec::new();
    }
    let Some(raw) = source
        .as_str()
        .get(node.start() as usize..node.end() as usize)
    else {
        return Vec::new();
    };
    let mut images = Vec::new();
    let mut cursor = 0;
    // Only consecutive img tags at a syntax-owned HTML span. In particular,
    // never scan strings inside script, comments, attributes or fenced code.
    loop {
        while raw
            .as_bytes()
            .get(cursor)
            .is_some_and(u8::is_ascii_whitespace)
        {
            cursor += 1;
        }
        let rest = &raw[cursor..];
        let Some(image) = html_image_span(rest, u64::from(node.start()) + cursor as u64) else {
            break;
        };
        cursor += image.source().len() as usize;
        images.push(image);
        if cursor >= raw.len() {
            break;
        }
    }
    images
}

/// Shared source-preserving img interpretation for inline and block HTML.
pub(crate) fn html_image_span(raw: &str, base: u64) -> Option<ImageSpan> {
    let tag = crate::image_markup::parse_image_tag(raw)?;
    let destination = tag.attribute("src")?.value.clone()?;
    if destination.is_empty() {
        return None;
    }
    let width = tag.dimension(raw, "width").ok()?;
    let height = tag.dimension(raw, "height").ok()?;
    let range = |range: std::ops::Range<usize>| {
        TextRange::new(
            yu_core::ByteOffset::new(base + range.start as u64),
            yu_core::ByteOffset::new(base + range.end as u64),
        )
        .expect("validated tag range")
    };
    let label = tag
        .attribute("alt")
        .and_then(|a| a.value.clone())
        .unwrap_or(0..0);
    Some(
        ImageSpan::new(
            range(0..tag.length),
            range(label),
            Some(range(destination)),
            None,
        )
        .with_html_dimensions(width, height),
    )
}

/// Resolve source syntax before the resource layer decodes the URI.
pub fn image_destination_text(
    source: &yu_text::TextSnapshot,
    image: ImageSpan,
    definitions: &crate::ReferenceDefinitionIndex,
) -> Option<String> {
    let destination = image.destination().or_else(|| {
        image
            .reference()
            .and_then(|label| definitions.lookup(source, label))
            .map(|definition| definition.destination())
    })?;
    let raw = source
        .as_str()
        .get(destination.start().get() as usize..destination.end().get() as usize)?;
    let raw = if image.is_html() {
        raw
    } else {
        raw.strip_prefix('<')
            .and_then(|value| value.strip_suffix('>'))
            .unwrap_or(raw)
    };
    Some(crate::image_markup::decode_image_text(raw, image.is_html()))
}

/// The title belongs to the image or its resolved definition, never another
/// adjacent link. Callers use this when serializing an individual image edit.
pub fn image_title_text(
    document: &crate::MarkdownDocument,
    source: &yu_text::TextSnapshot,
    image: ImageSpan,
) -> Option<String> {
    if image.is_html() {
        let raw = source
            .as_str()
            .get(image.source().start().get() as usize..image.source().end().get() as usize)?;
        let tag = crate::image_markup::parse_image_tag(raw)?;
        return tag
            .value(raw, "title")
            .map(|value| crate::image_markup::decode_image_text(value, true));
    }
    let range = image
        .reference()
        .and_then(|label| document.reference_definitions().lookup(source, label))
        .map_or(image.source(), |definition| definition.source());
    SyntaxNode::new(document.tree()?, 0)
        .descendants_in(range)
        .find(|node| node.kind() == NodeKind::LinkTitle)
        .and_then(|node| {
            source
                .as_str()
                .get(node.start() as usize + 1..node.end() as usize - 1)
        })
        .map(|value| crate::image_markup::decode_image_text(value, false))
}
