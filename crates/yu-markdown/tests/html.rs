use yu_core::ByteOffset;
use yu_markdown::html::{HtmlTag, HtmlTagError};

#[test]
fn projected_html_sequence_expands_leaves_without_changing_parser_blocks() {
    use yu_markdown::BlockKind;
    use yu_text::TextBuffer;
    let source = "before\n\n<div><h2>Heading</h2><p>one</p><p>two</p></div>\n\nafter\n";
    let document = yu_markdown::parse(&TextBuffer::new(source).snapshot());
    let mut source_document = document.clone();
    source_document.set_source_mode(true);
    let raw = source_document
        .blocks()
        .iter()
        .map(|block| (block.kind(), block.range()))
        .collect::<Vec<_>>();
    let projected = document.html_regions().projected_blocks();
    assert_eq!(projected.len(), raw.len() + 2);
    assert_eq!(
        projected
            .iter()
            .filter(|block| block.kind() == BlockKind::Heading { level: 2 })
            .count(),
        1
    );
    assert!(
        !projected
            .iter()
            .any(|block| block.kind() == BlockKind::HtmlBlock)
    );
    let mut rebuilt = String::new();
    let mut previous = ByteOffset::ZERO;
    for block in projected.iter() {
        assert_eq!(block.range().start(), previous);
        rebuilt.push_str(
            &source[block.range().start().get() as usize..block.range().end().get() as usize],
        );
        previous = block.range().end();
    }
    assert_eq!(rebuilt, source);
    assert_eq!(
        source_document
            .blocks()
            .iter()
            .map(|block| (block.kind(), block.range()))
            .collect::<Vec<_>>(),
        raw
    );
}

#[test]
fn html_partition_queries_use_half_open_boundaries_and_reject_cross_leaf_ranges() {
    use yu_core::TextRange;
    use yu_text::TextBuffer;
    let source = "before\n\n<div><p>one</p><p>two</p></div>\n\nafter";
    let document = yu_markdown::parse(&TextBuffer::new(source).snapshot());
    let index = document.html_regions();
    let region = &index.regions[0];
    let parts = &region.model.as_ref().expect("model").partitions;
    assert_eq!(parts.len(), 2);
    assert!(std::ptr::eq(
        index.partition_for(parts[0].source).expect("first"),
        &parts[0]
    ));
    let boundary = parts[0].source.end();
    assert!(std::ptr::eq(
        index
            .partition_for(TextRange::empty(boundary))
            .expect("next"),
        &parts[1]
    ));
    let crossing =
        TextRange::new(parts[0].source.start(), parts[1].source.end()).expect("crossing");
    assert!(index.partition_for(crossing).is_none());
    assert!(
        index
            .partition_for(TextRange::empty(region.source.end()))
            .is_none()
    );
    assert!(
        index
            .partition_for(TextRange::empty(ByteOffset::ZERO))
            .is_none()
    );
}

#[test]
fn document_html_index_is_shared_and_rebuilt_after_unicode_prefix_edits() {
    use yu_core::TextRange;
    use yu_text::{Edit, TextBuffer, Transaction};
    let source = "前文🙂\n\n<div align='center'><p>first</p><p>second</p></div>\n\n```html\n<div>opaque</div>\n```\n";
    let mut buffer = TextBuffer::new(source);
    let document = yu_markdown::parse(&buffer.snapshot());
    let index = document.html_regions();
    assert_eq!(index.regions.len(), 1);
    let region = &index.regions[0];
    let model = region.model.as_ref().expect("HTML model");
    assert!(model.resolution.diagnostics.is_empty());
    assert_eq!(model.partitions.len(), 2);
    assert_eq!(model.partitions[0].source.start(), region.source.start());
    assert_eq!(model.partitions[1].source.end(), region.source.end());
    let mut cloned = document.clone();
    cloned.set_source_mode(true);
    assert!(std::ptr::eq(index, cloned.html_regions()));
    let prefix = "新🙂\n\n";
    let applied = buffer
        .apply(&Transaction::new(
            buffer.revision(),
            [Edit::new(TextRange::empty(ByteOffset::ZERO), prefix)],
        ))
        .expect("prefix edit");
    let changed =
        yu_markdown::parse_incremental(&document, applied.result_snapshot(), applied.change_set())
            .expect("incremental")
            .into_document();
    let full = yu_markdown::parse(applied.result_snapshot());
    assert_eq!(changed.html_regions(), full.html_regions());
    assert_eq!(
        changed.html_regions().regions[0].source.start().get(),
        region.source.start().get() + prefix.len() as u64
    );
    assert_eq!(document.source().as_str(), source);
    assert_eq!(index.revision, document.revision());
    assert_ne!(changed.html_regions().revision, index.revision);
}

#[test]
fn flow_partition_assigns_container_tags_and_gaps_without_extra_paragraphs() {
    use yu_core::TextRange;
    use yu_markdown::html::{HtmlFragment, partition_flow};
    let raw = "<div>\r\n<p>中文</p>\r\n\r\n<ul><li>one</li><li>two</li></ul>\r\n</div>";
    let fragment = HtmlFragment::parse(raw, ByteOffset::ZERO).expect("fragment");
    let leaves = fragment.flow_blocks(&fragment.resolve(raw), raw);
    let full = TextRange::new(ByteOffset::ZERO, ByteOffset::new(raw.len() as u64)).expect("range");
    let parts = partition_flow(&leaves, full).expect("partition");
    assert_eq!(parts.len(), 3);
    let mut joined = String::new();
    let mut cursor = ByteOffset::ZERO;
    for part in &parts {
        assert_eq!(part.source.start(), cursor);
        assert!(part.source.start() <= part.content.source.start());
        assert!(part.source.end() >= part.content.source.end());
        joined.push_str(&raw[part.source.start().get() as usize..part.source.end().get() as usize]);
        cursor = part.source.end();
    }
    assert_eq!(cursor, full.end());
    assert_eq!(joined, raw);
    assert_eq!(
        &raw[parts[0].source.start().get() as usize..parts[0].source.end().get() as usize],
        "<div>\r\n<p>中文</p>"
    );
}

#[test]
fn flow_partition_keeps_empty_containers_addressable_and_rejects_invalid_ranges() {
    use yu_core::TextRange;
    use yu_markdown::html::{HtmlFlowBlock, HtmlFlowError, HtmlFlowKind, partition_flow};
    let range = |a, b| TextRange::new(ByteOffset::new(a), ByteOffset::new(b)).expect("range");
    let whole = range(10, 30);
    let empty = partition_flow(&[], whole).expect("editable empty container");
    assert_eq!(empty[0].source, whole);
    assert_eq!(empty[0].content.kind, HtmlFlowKind::Source);
    let leaf = |source| HtmlFlowBlock {
        kind: HtmlFlowKind::Paragraph,
        source,
        owner: None,
    };
    assert_eq!(
        partition_flow(&[leaf(range(9, 20))], whole),
        Err(HtmlFlowError::OutsideSource)
    );
    assert_eq!(
        partition_flow(&[leaf(range(10, 20)), leaf(range(19, 25))], whole),
        Err(HtmlFlowError::OverlappingLeaves)
    );
}

#[test]
fn flow_leaves_keep_distinct_paragraphs_and_container_ancestry() {
    use yu_markdown::html::{HtmlFlowKind, HtmlFragment};
    let raw = "<div><h2>Heading</h2><p>first <b>bold</b></p><ul><li>one <em>item</em><ul><li>nested</li></ul></li></ul><p>last</p><table><tr><td>cell</td></tr></table></div>";
    let fragment = HtmlFragment::parse(raw, ByteOffset::ZERO).expect("fragment");
    let resolved = fragment.resolve(raw);
    let blocks = fragment.flow_blocks(&resolved, raw);
    assert_eq!(
        blocks.iter().map(|b| b.kind).collect::<Vec<_>>(),
        [
            HtmlFlowKind::Heading(2),
            HtmlFlowKind::Paragraph,
            HtmlFlowKind::Paragraph,
            HtmlFlowKind::Paragraph,
            HtmlFlowKind::Paragraph,
            HtmlFlowKind::Table
        ]
    );
    let text = blocks
        .iter()
        .map(|b| &raw[b.source.start().get() as usize..b.source.end().get() as usize])
        .collect::<Vec<_>>();
    assert_eq!(
        text,
        [
            "<h2>Heading</h2>",
            "<p>first <b>bold</b></p>",
            "one <em>item</em>",
            "nested",
            "<p>last</p>",
            "<table><tr><td>cell</td></tr></table>"
        ]
    );
    assert!(
        blocks
            .windows(2)
            .all(|pair| pair[0].source.end() <= pair[1].source.start())
    );
    let nested_owner = blocks[3].owner.expect("nested item");
    assert!(fragment.nodes[nested_owner].parent.is_some());
    assert_fragment_coverage(raw, &fragment);
}

#[test]
fn flow_keeps_invalid_subtrees_as_source_leaves() {
    use yu_markdown::html::{HtmlFlowKind, HtmlFragment};
    for raw in ["<div onclick='run()'><p>raw</p></div>", "<div><p>unclosed"] {
        let fragment = HtmlFragment::parse(raw, ByteOffset::ZERO).expect("fragment");
        let blocks = fragment.flow_blocks(&fragment.resolve(raw), raw);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].kind, HtmlFlowKind::Source);
        assert_eq!(blocks[0].source.end().get(), raw.len() as u64);
    }
}

#[test]
fn inline_containers_reject_block_children_without_browser_repair() {
    use yu_markdown::html::HtmlFragment;
    for raw in [
        "<p><div>block</div></p>",
        "<span><p>block</p></span>",
        "<b><ul><li>item</li></ul></b>",
        "<h2><table><tr><td>x</td></tr></table></h2>",
        "<a href='x'><details>body</details></a>",
    ] {
        let fragment = HtmlFragment::parse(raw, ByteOffset::ZERO).expect("source fragment");
        let resolved = fragment.resolve(raw);
        assert!(!resolved.diagnostics.is_empty(), "{raw}");
        assert!(resolved.elements.iter().all(Option::is_none));
        assert_fragment_coverage(raw, &fragment);
    }
    let raw = "<div><p>text <span><b>bold</b><a href='x'>link</a></span></p><ul><li><p>paragraph</p></li></ul></div>";
    let fragment = HtmlFragment::parse(raw, ByteOffset::ZERO).expect("valid fragment");
    assert!(fragment.resolve(raw).diagnostics.is_empty());
    assert_fragment_coverage(raw, &fragment);
}

#[test]
fn details_summary_is_unique_and_precedes_substantive_content() {
    use yu_markdown::html::HtmlFragment;
    for raw in [
        "<details><summary>One</summary><summary>Two</summary></details>",
        "<details><p>Body</p><summary>Late</summary></details>",
        "<details>Body<summary>Late</summary></details>",
    ] {
        let fragment = HtmlFragment::parse(raw, ByteOffset::ZERO).expect("fragment");
        let result = fragment.resolve(raw);
        assert!(!result.diagnostics.is_empty(), "{raw}");
        assert!(result.elements.iter().all(Option::is_none));
        assert_fragment_coverage(raw, &fragment);
    }
    for raw in [
        "<details> \n<!-- note --><summary>Title</summary><p>Body</p></details>",
        "<details><p>Body without explicit summary</p></details>",
        "<details><summary>Outer</summary><details><summary>Inner</summary>Body</details></details>",
    ] {
        let fragment = HtmlFragment::parse(raw, ByteOffset::ZERO).expect("fragment");
        assert!(fragment.resolve(raw).diagnostics.is_empty(), "{raw}");
        assert_fragment_coverage(raw, &fragment);
    }
}

#[test]
fn structural_resolution_rejects_orphan_cells_and_invalid_list_children() {
    use yu_markdown::html::{HtmlFragment, HtmlNodeKind};
    for raw in [
        "<td>orphan</td>",
        "<summary>orphan</summary>",
        "<ul><p>wrong child</p></ul>",
        "<table>loose text<tr><td>x</td></tr></table>",
    ] {
        let fragment = HtmlFragment::parse(raw, ByteOffset::ZERO).expect("fragment");
        let resolved = fragment.resolve(raw);
        assert!(!resolved.diagnostics.is_empty(), "{raw}");
        assert!(resolved.elements.iter().all(Option::is_none));
        assert_fragment_coverage(raw, &fragment);
    }
    let raw = "<div align='center'><ul><li>one</li></ul><table><tbody><tr><td>x</td></tr></tbody></table><details open><summary>More</summary><p>body</p></details></div>";
    let fragment = HtmlFragment::parse(raw, ByteOffset::ZERO).expect("fragment");
    let resolved = fragment.resolve(raw);
    assert!(resolved.diagnostics.is_empty());
    for (node, element) in fragment.nodes.iter().zip(&resolved.elements) {
        assert_eq!(
            element.is_some(),
            matches!(node.kind, HtmlNodeKind::Element { .. })
        );
    }
}

#[test]
fn unknown_or_invalid_attribute_subtrees_remain_source_while_valid_siblings_resolve() {
    use yu_markdown::html::{HtmlFragment, HtmlNodeKind, HtmlSemanticError};
    let raw = "<div onclick='run()'><b>raw</b></div><p>valid</p><script><i>text</i></script>";
    let fragment = HtmlFragment::parse(raw, ByteOffset::ZERO).expect("fragment");
    let resolved = fragment.resolve(raw);
    let resolved_names = fragment
        .nodes
        .iter()
        .zip(&resolved.elements)
        .filter_map(|(node, resolved)| {
            if resolved.is_none() {
                return None;
            }
            match &node.kind {
                HtmlNodeKind::Element { opening, .. } => Some(opening.name.as_str()),
                _ => None,
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(resolved_names, ["p"]);
    assert_eq!(resolved.diagnostics.len(), 2);
    assert!(
        resolved
            .diagnostics
            .iter()
            .any(|d| d.error == HtmlSemanticError::UnknownElement)
    );
    assert_fragment_coverage(raw, &fragment);
}

#[test]
fn finite_attributes_resolve_without_losing_original_spellings() {
    use yu_markdown::html::{HtmlAlignment, HtmlDimension};
    let raw = "<img src='../图&amp;片.png' alt='中文' width='50%' height='120.5px'>";
    let tag = HtmlTag::parse(raw, ByteOffset::ZERO).expect("image");
    let attrs = tag.resolve_attributes(raw).expect("finite attributes");
    assert_eq!(attrs.destination.as_deref(), Some("../图&片.png"));
    assert_eq!(attrs.alternate.as_deref(), Some("中文"));
    assert_eq!(attrs.width, Some(HtmlDimension::Percent(50.0)));
    assert_eq!(attrs.height, Some(HtmlDimension::Points(120.5)));
    assert_eq!(
        &raw[tag.source.start().get() as usize..tag.source.end().get() as usize],
        raw
    );
    for raw in ["<div align='CENTER'>", "<p style='text-align: center;'>"] {
        assert_eq!(
            HtmlTag::parse(raw, ByteOffset::ZERO)
                .expect("aligned")
                .resolve_attributes(raw)
                .expect("attributes")
                .alignment,
            Some(HtmlAlignment::Center)
        );
    }
    for raw in ["<details open>", "<details open='false'>"] {
        assert!(
            HtmlTag::parse(raw, ByteOffset::ZERO)
                .expect("details")
                .resolve_attributes(raw)
                .expect("boolean")
                .open
        );
    }
}

#[test]
fn unsupported_or_ambiguous_attributes_never_silently_disappear() {
    for raw in [
        "<div style='text-align:center;color:red'>",
        "<div align='left' style='text-align:right'>",
        "<img width='NaN'>",
        "<img width='0'>",
        "<img height='-4'>",
        "<img width='101%'>",
        "<img src='a' SRC='b'>",
        "<img loading='lazy'>",
        "<p onclick='run()'>",
        "<ol start='-1'>",
        "<td colspan='0'>",
    ] {
        let tag = HtmlTag::parse(raw, ByteOffset::ZERO).expect("retained source");
        assert!(tag.resolve_attributes(raw).is_err(), "{raw}");
        assert_eq!(tag.source.end().get(), raw.len() as u64);
    }
}

#[test]
fn native_semantic_candidates_exclude_executable_and_unknown_tags() {
    use yu_markdown::html::HtmlElementKind;
    for (raw, expected) in [
        ("<p>", HtmlElementKind::Paragraph),
        ("<h3>", HtmlElementKind::Heading(3)),
        ("<details open>", HtmlElementKind::Details),
        ("<td>", HtmlElementKind::DataCell),
    ] {
        assert_eq!(
            HtmlTag::parse(raw, ByteOffset::ZERO)
                .expect("tag")
                .semantic_kind(),
            Some(expected)
        );
    }
    for raw in ["<script>", "<style>", "<iframe>", "<custom>"] {
        assert_eq!(
            HtmlTag::parse(raw, ByteOffset::ZERO)
                .expect("retained tag")
                .semantic_kind(),
            None
        );
    }
    let tag = HtmlTag::parse("<img src='a' SRC='b' alt='text'>", ByteOffset::ZERO)
        .expect("duplicate retained");
    assert_eq!(tag.unique_attribute("src"), Err(HtmlTagError::Malformed));
    assert!(tag.unique_attribute("ALT").expect("unique").is_some());
    assert!(tag.unique_attribute("width").expect("missing").is_none());
}

fn assert_fragment_coverage(raw: &str, fragment: &yu_markdown::html::HtmlFragment) {
    use yu_markdown::html::HtmlNodeKind;
    let mut spans = Vec::new();
    for (id, node) in fragment.nodes.iter().enumerate() {
        for &child in &node.children {
            assert_eq!(fragment.nodes[child].parent, Some(id));
        }
        match &node.kind {
            HtmlNodeKind::Element { opening, closing } => {
                spans.push(opening.source);
                spans.extend(closing.iter().map(|tag| tag.source));
            }
            _ => spans.push(node.source),
        }
    }
    spans.sort_by_key(|range| range.start());
    let mut offset = 0;
    for span in spans {
        assert_eq!(span.start().get(), offset);
        offset = span.end().get();
    }
    assert_eq!(offset, raw.len() as u64);
}

#[test]
fn literal_comparisons_do_not_swallow_following_elements() {
    use yu_markdown::html::{HtmlFragment, HtmlNodeKind};
    let raw = "<p>中文 2 < 3 and << <b>bold</b> tail <</p>";
    let fragment = HtmlFragment::parse(raw, ByteOffset::ZERO).expect("fragment");
    assert!(fragment.diagnostics.is_empty());
    let names = fragment
        .nodes
        .iter()
        .filter_map(|node| match &node.kind {
            HtmlNodeKind::Element { opening, .. } => Some(opening.name.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(names, ["p", "b"]);
    assert_fragment_coverage(raw, &fragment);
}

#[test]
fn fragment_preserves_container_hierarchy_and_every_source_byte() {
    use yu_markdown::html::{HtmlFragment, HtmlNodeKind};
    let raw = "<div align='center'><h2>中文🙂</h2><p>A <b>bold</b><br>B</p><ul><li>one</li></ul><details open><summary>More</summary><table><tr><td>x</td></tr></table></details></div>";
    let fragment = HtmlFragment::parse(raw, ByteOffset::ZERO).expect("fragment");
    assert!(fragment.diagnostics.is_empty());
    assert_eq!(fragment.roots, [0]);
    assert_eq!(fragment.nodes[0].children.len(), 4);
    assert_eq!(fragment.nodes[0].source.end().get(), raw.len() as u64);
    let names = fragment
        .nodes
        .iter()
        .filter_map(|node| match &node.kind {
            HtmlNodeKind::Element { opening, .. } => Some(opening.name.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        [
            "div", "h2", "p", "b", "br", "ul", "li", "details", "summary", "table", "tr", "td"
        ]
    );
    assert_fragment_coverage(raw, &fragment);
}

#[test]
fn raw_elements_and_comments_do_not_create_active_inner_elements() {
    use yu_markdown::html::{HtmlFragment, HtmlNodeKind};
    let raw = "<!-- <img src=x> --><script>const x='<b>not formatting</b></scripture><i>still raw</i>';</script><style>p:before{content:'<i>'}</style><p>safe</p>";
    let fragment = HtmlFragment::parse(raw, ByteOffset::ZERO).expect("raw fragment");
    assert!(fragment.diagnostics.is_empty());
    let names = fragment
        .nodes
        .iter()
        .filter_map(|node| match &node.kind {
            HtmlNodeKind::Element { opening, .. } => Some(opening.name.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(names, ["script", "style", "p"]);
    assert_fragment_coverage(raw, &fragment);
}

#[test]
fn malformed_and_deep_fragments_keep_source_and_report_diagnostics() {
    use yu_markdown::html::{HtmlDiagnosticKind, HtmlFragment};
    for raw in [
        "<div><p>x</div></p>",
        "<p>unfinished",
        "<!-- unfinished",
        "<p x='broken>",
    ] {
        let fragment = HtmlFragment::parse(raw, ByteOffset::ZERO).expect("retained fragment");
        assert!(!fragment.diagnostics.is_empty(), "{raw}");
        assert_fragment_coverage(raw, &fragment);
    }
    let raw = format!("{}text{}", "<div>".repeat(1000), "</div>".repeat(1000));
    let fragment = HtmlFragment::parse(&raw, ByteOffset::ZERO).expect("bounded fragment");
    assert!(
        fragment
            .diagnostics
            .iter()
            .any(|d| d.kind == HtmlDiagnosticKind::LimitExceeded)
    );
    assert!(fragment.nodes.len() <= 129);
    assert_fragment_coverage(&raw, &fragment);
}

#[test]
fn html_tag_attributes_keep_exact_unicode_and_quoted_source_ranges() {
    let prefix = "前言🙂 ";
    let raw =
        "<IMG src='../图.png' alt=中文 title=\"x > y &amp; z\" width='120' data-extra disabled />";
    let source = format!("{prefix}{raw}");
    let tag = HtmlTag::parse(raw, ByteOffset::new(prefix.len() as u64)).expect("tag");
    assert_eq!(tag.name, "img");
    assert!(tag.is_void());
    assert!(tag.self_closing);
    assert!(!tag.closing);
    assert_eq!(
        tag.attributes
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>(),
        ["src", "alt", "title", "width", "data-extra", "disabled"]
    );
    let values = tag
        .attributes
        .iter()
        .map(|a| {
            a.value
                .map(|range| &source[range.start().get() as usize..range.end().get() as usize])
        })
        .collect::<Vec<_>>();
    assert_eq!(
        values,
        [
            Some("../图.png"),
            Some("中文"),
            Some("x > y &amp; z"),
            Some("120"),
            None,
            None
        ]
    );
    let attribute = &tag.attributes[0];
    assert_eq!(
        &source[attribute.source.start().get() as usize..attribute.source.end().get() as usize],
        "src='../图.png'"
    );
    assert_eq!(
        &source[tag.source.start().get() as usize..tag.source.end().get() as usize],
        raw
    );
}

#[test]
fn unknown_and_duplicate_attributes_are_retained_not_silently_selected() {
    let tag = HtmlTag::parse(
        "<details open OPEN='false' onclick='run()' style='color:red'>",
        ByteOffset::ZERO,
    )
    .expect("source metadata");
    assert_eq!(
        tag.attributes
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>(),
        ["open", "open", "onclick", "style"]
    );
    assert!(!tag.is_void());
    assert!(!tag.self_closing);
    let close = HtmlTag::parse("</DETAILS \t>", ByteOffset::ZERO).expect("closing");
    assert!(close.closing);
    assert_eq!(close.name, "details");
    assert!(close.attributes.is_empty());
}

#[test]
fn malformed_tags_and_offsets_are_rejected_without_guessing() {
    for raw in [
        "",
        "<>",
        "<1x>",
        "< b>",
        "<b",
        "</b x>",
        "</b/>",
        "<b x=>",
        "<b x='open>",
        "<b x='v'y='v'>",
        "<b x=`v`>",
        "<b x=a=b>",
        "<b x='v' extra\"bad>",
    ] {
        assert_eq!(
            HtmlTag::parse(raw, ByteOffset::ZERO),
            Err(HtmlTagError::Malformed),
            "{raw}"
        );
    }
    assert_eq!(
        HtmlTag::parse("<b>", ByteOffset::new(u64::MAX)),
        Err(HtmlTagError::OffsetOverflow)
    );
}

#[test]
fn html_presentation_matches_projected_leaves_and_nested_list_numbering() {
    use yu_markdown::PresentationKind as P;
    let source = "<div><h2>标题</h2><ol start='3'><li>one<ul><li>nested</li></ul></li><li>two</li></ol><p>end</p></div>";
    let document = yu_markdown::parse(&yu_text::TextBuffer::new(source).snapshot());
    let index = document.html_regions();
    let tree = index.presentation();
    let parts = &index.regions[0].model.as_ref().expect("model").partitions;
    assert_eq!(parts.len(), 5);
    let paths: Vec<_> = parts
        .iter()
        .map(|part| tree.path_for_range(part.source))
        .collect();
    let kinds = |i: usize| {
        paths[i]
            .iter()
            .map(|&id| tree.nodes()[id].kind)
            .collect::<Vec<_>>()
    };
    assert_eq!(kinds(0), [P::Document, P::Heading(2)]);
    assert_eq!(
        kinds(1),
        [
            P::Document,
            P::List {
                ordered: true,
                tight: true,
                start: 3
            },
            P::ListItem,
            P::Paragraph
        ]
    );
    assert_eq!(
        kinds(2),
        [
            P::Document,
            P::List {
                ordered: true,
                tight: true,
                start: 3
            },
            P::ListItem,
            P::List {
                ordered: false,
                tight: true,
                start: 1
            },
            P::ListItem,
            P::Paragraph
        ]
    );
    assert_eq!(tree.nodes()[paths[1][2]].item_number, Some(3));
    assert_eq!(tree.nodes()[paths[3][2]].item_number, Some(4));
    assert!(tree.is_tight_paragraph(*paths[1].last().expect("leaf")));
    for part in parts {
        let leaf = tree.leaf_for_range(part.source).expect("leaf");
        assert_eq!(tree.nodes()[leaf].source, part.source);
    }
    // Every exposed node belongs to the new tree; no detached old HTML leaf.
    for (id, node) in tree.nodes().iter().enumerate() {
        if let Some(parent) = node.parent {
            assert!(tree.nodes()[parent].children.contains(&id));
        }
        for &child in &node.children {
            assert_eq!(tree.nodes()[child].parent, Some(id));
        }
    }
    assert!(!tree.nodes().iter().any(|node| node.kind == P::Html));
    let shifted =
        yu_markdown::parse(&yu_text::TextBuffer::new(format!("prefix\n\n{source}")).snapshot());
    let next = shifted.html_regions();
    for (old, new) in parts
        .iter()
        .zip(&next.regions[0].model.as_ref().expect("model").partitions)
    {
        if matches!(
            old.content.kind,
            yu_markdown::html::HtmlFlowKind::Heading(_)
        ) {
            assert_ne!(
                tree.context_key(old.source),
                next.presentation().context_key(new.source)
            );
        } else {
            assert_eq!(
                tree.context_key(old.source),
                next.presentation().context_key(new.source)
            );
        }
    }
}

#[test]
fn html_explicit_list_paragraphs_keep_loose_spacing() {
    let document = yu_markdown::parse(
        &yu_text::TextBuffer::new("<ul><li><p>one</p><p>two</p></li></ul>").snapshot(),
    );
    let index = document.html_regions();
    let tree = index.presentation();
    for part in &index.regions[0].model.as_ref().expect("model").partitions {
        assert!(!tree.is_tight_paragraph(tree.leaf_for_range(part.source).expect("leaf")));
    }
}

#[test]
fn html_table_metadata_keeps_sections_mixed_headers_and_exact_cell_ranges() {
    use yu_markdown::html::{HtmlAlignment as A, HtmlTableSection as S};
    let source = "前文🙂\r\n\r\n<table align='center'><thead><tr><th>标题</th><th align='right'>B</th></tr></thead><tbody><tr align='left'><th>row</th><td><b>内容</b>&amp;</td></tr><tr><td></td></tr></tbody><tfoot><tr><td>尾</td><td>end</td></tr></tfoot></table>\r\n";
    let document = yu_markdown::parse(&yu_text::TextBuffer::new(source).snapshot());
    let model = document.html_regions().regions[0]
        .model
        .as_ref()
        .expect("model");
    let owner = model.partitions[0].content.owner.expect("table");
    let table = model.table(owner).expect("grid");
    assert_eq!(table.columns, 2);
    assert_eq!(table.rows.len(), 4);
    assert_eq!(
        table.rows.iter().map(|row| row.section).collect::<Vec<_>>(),
        [S::Head, S::Body, S::Body, S::Foot]
    );
    assert!(table.rows[0].cells.iter().all(|cell| cell.header));
    assert!(table.rows[1].cells[0].header);
    assert!(!table.rows[1].cells[1].header);
    assert_eq!(table.rows[0].cells[0].alignment, Some(A::Center));
    assert_eq!(table.rows[0].cells[1].alignment, Some(A::Right));
    assert_eq!(table.rows[1].cells[1].alignment, Some(A::Left));
    assert_eq!(
        table.rows[2].cells.len(),
        1,
        "ragged rows must not acquire synthetic source cells"
    );
    assert!(table.rows[2].cells[0].content.is_empty());
    let content = table.rows[1].cells[1].content;
    assert_eq!(
        &source[content.start().get() as usize..content.end().get() as usize],
        "<b>内容</b>&amp;"
    );
    for row in &table.rows {
        for cell in &row.cells {
            assert!(
                row.source.start() <= cell.source.start() && cell.source.end() <= row.source.end()
            );
            assert!(
                cell.source.start() < cell.content.start()
                    && cell.content.end() < cell.source.end()
            );
        }
    }
    assert_eq!(document.source().as_str(), source);
}

#[test]
fn html_table_has_no_implicit_header_and_rejects_unsupported_structure() {
    use yu_markdown::html::HtmlTableError;
    for (source, expected) in [
        ("<table><tr><td colspan='2'>x</td></tr></table>", None),
        (
            "<table><tr><td><table><tr><td>x</td></tr></table></td></tr></table>",
            Some(HtmlTableError::NestedTable),
        ),
        ("<table><tr></tr></table>", Some(HtmlTableError::EmptyTable)),
        ("<table><tr><td>x</td><td>y</td></tr></table>", None),
    ] {
        let document = yu_markdown::parse(&yu_text::TextBuffer::new(source).snapshot());
        let model = document.html_regions().regions[0]
            .model
            .as_ref()
            .expect("model");
        let owner = model.partitions[0].content.owner.expect("table source");
        match expected {
            Some(error) => assert_eq!(model.table(owner), Err(error)),
            None => assert!(
                model.table(owner).expect("grid").rows[0]
                    .cells
                    .iter()
                    .all(|cell| !cell.header)
            ),
        }
        assert_eq!(document.source().as_str(), source);
    }
}

#[test]
fn empty_html_list_items_keep_flow_leaves_and_ordered_numbers() {
    let source =
        "<ol start='3'><li></li><li><!--keep--></li><li><div> \n </div></li><li>tail</li></ol>";
    let document = yu_markdown::parse(&yu_text::TextBuffer::new(source).snapshot());
    let index = document.html_regions();
    let model = index.regions[0].model.as_ref().expect("model");
    assert_eq!(
        model.partitions.len(),
        4,
        "empty items must remain editable paragraphs"
    );
    let items: Vec<_> = index
        .presentation()
        .nodes()
        .iter()
        .filter(|node| node.kind == yu_markdown::PresentationKind::ListItem)
        .collect();
    assert_eq!(
        items
            .iter()
            .map(|node| node.item_number)
            .collect::<Vec<_>>(),
        [Some(3), Some(4), Some(5), Some(6)]
    );
    for part in &model.partitions {
        assert_eq!(
            part.content.kind,
            yu_markdown::html::HtmlFlowKind::Paragraph
        );
        assert!(index.presentation().leaf_for_range(part.source).is_some());
    }
    assert_eq!(document.source().as_str(), source);
}

#[test]
fn empty_nested_html_list_items_are_scoped_to_their_cell_and_do_not_overlap() {
    let source = "<div><ol><li>outside</li></ol><table><tr><td><ol start='5'><li><ul><li></li></ul></li><li></li></ol></td><td><ul><li><!--other--></li></ul></td></tr></table></div>";
    let document = yu_markdown::parse(&yu_text::TextBuffer::new(source).snapshot());
    let model = document.html_regions().regions[0]
        .model
        .as_ref()
        .expect("model");
    let table_owner = model
        .partitions
        .iter()
        .find(|p| p.content.kind == yu_markdown::html::HtmlFlowKind::Table)
        .and_then(|p| p.content.owner)
        .expect("table");
    let table = model.table(table_owner).expect("table metadata");
    assert_eq!(table.rows[0].cells[0].paragraphs.len(), 2);
    assert_eq!(table.rows[0].cells[1].paragraphs.len(), 1);
    for cell in &table.rows[0].cells {
        let parts = &model.cell_partitions[&cell.node];
        assert_eq!(
            parts.first().expect("first").source.start(),
            cell.content.start()
        );
        assert_eq!(parts.last().expect("last").source.end(), cell.content.end());
        for pair in parts.windows(2) {
            assert!(pair[0].content.source.end() <= pair[1].content.source.start());
        }
        assert!(
            parts
                .iter()
                .all(|p| cell.content.start() <= p.content.source.start()
                    && p.content.source.end() <= cell.content.end())
        );
    }
    assert_eq!(document.source().as_str(), source);
}

#[test]
fn html_cell_list_metadata_keeps_numbering_continuations_and_shifted_sources() {
    let source = "<table><tr><td><ol start='3'><li><p>first</p><p>continued</p><ul><li>nested</li></ul></li><li>last</li></ol></td></tr></table>";
    let document = yu_markdown::parse(&yu_text::TextBuffer::new(source).snapshot());
    let model = document.html_regions().regions[0]
        .model
        .as_ref()
        .expect("model");
    let table = model
        .native_table(model.partitions[0].content.owner.expect("table"))
        .expect("native");
    let parts = &table.rows[0].cells[0].paragraphs;
    assert_eq!(parts.len(), 4);
    assert_eq!(
        parts.iter().map(|part| part.list.len()).collect::<Vec<_>>(),
        [1, 1, 2, 1]
    );
    assert_eq!(parts[0].list[0].number, Some(3));
    assert!(parts[0].list[0].opening.is_some());
    assert!(!parts[0].list[0].tight);
    assert!(parts[1].list[0].opening.is_none());
    assert!(parts[2].list[0].opening.is_none());
    assert_eq!(parts[2].list[1].number, None);
    assert!(parts[2].list[1].opening.is_some());
    assert_eq!(parts[3].list[0].number, Some(4));
    let grid = yu_markdown::TableBlock::from_html(&table).expect("grid");
    let moved = grid.clone().shifted(11).expect("shift");
    let address = yu_markdown::TableCellAddress::new(0, 0);
    for (old, new) in grid
        .cell_paragraphs(address)
        .iter()
        .zip(moved.cell_paragraphs(address))
    {
        assert_eq!(new.source.start(), old.source.start() + 11);
        for (old, new) in old.list.iter().zip(&new.list) {
            assert_eq!(new.container.start(), old.container.start() + 11);
            assert_eq!(new.number, old.number);
            assert_eq!(
                new.opening.map(|r| r.start()),
                old.opening.map(|r| r.start() + 11)
            );
        }
    }
}

#[test]
fn source_table_grid_tracks_ragged_cell_owners_without_synthetic_content() {
    let source = "<table><tr><td>中文🙂</td><td>B</td></tr><tr><td>C</td></tr></table>\r\n";
    let document = yu_markdown::parse(&yu_text::TextBuffer::new(source).snapshot());
    let index = document.html_regions();
    let model = index.regions[0].model.as_ref().expect("model");
    let table = model
        .native_table(model.partitions[0].content.owner.expect("table owner"))
        .expect("table");
    assert_eq!(table.grid.columns, table.columns);
    assert_eq!(table.grid.rows, table.rows.len());
    assert_eq!(table.grid.slots, vec![Some(0), Some(1), Some(2), None]);
    let content: Vec<_> = table
        .grid
        .cells
        .iter()
        .map(|owner| {
            let cell = &table.rows[owner.source_row].cells[owner.source_cell];
            &source[cell.content.start().get() as usize..cell.content.end().get() as usize]
        })
        .collect();
    assert_eq!(content, ["中文🙂", "B", "C"]);
    assert_eq!(document.source().as_str(), source);
}

#[test]
fn html_spans_resolve_grid_owners_and_preserve_row_group_boundaries() {
    let source = "<table><tbody><tr><td colspan='2' rowspan='0'>中文🙂</td><td>B</td></tr><tr><td>C</td></tr></tbody><tbody><tr><td>D</td><td>E</td></tr></tbody></table>\r\n";
    let document = yu_markdown::parse(&yu_text::TextBuffer::new(source).snapshot());
    let index = document.html_regions();
    let model = index.regions[0].model.as_ref().expect("model");
    let owner = model.partitions[0].content.owner.expect("table");
    let table = model.table(owner).expect("span grid");
    assert_eq!(table.columns, 3);
    assert_eq!(
        table.grid.slots,
        vec![
            Some(0),
            Some(0),
            Some(1),
            Some(0),
            Some(0),
            Some(2),
            Some(3),
            Some(4),
            None
        ]
    );
    assert_eq!(table.grid.cells[0].rows, 2);
    assert_ne!(table.rows[1].group, table.rows[2].group);
    assert_eq!(table.rows[0].cells[0].span.rows, 0);
    let grid = yu_markdown::TableBlock::from_html(&table).expect("span adapter");
    let address = yu_markdown::TableCellAddress::new;
    let origin = address(0, 0);
    for covered in [origin, address(0, 1), address(1, 0), address(1, 1)] {
        assert_eq!(grid.cell_origin(covered), Some(origin));
        assert_eq!(grid.visible_cell(covered), grid.visible_cell(origin));
        assert_eq!(grid.html_cell_owner(covered).expect("owner").columns, 2);
    }
    assert_eq!(
        grid.visible_cell_for_source(source.find("C</td>").expect("C")),
        Some(address(1, 2))
    );
    assert_eq!(
        grid.next_visible_cell(address(0, 1)).expect("next").0,
        address(0, 2)
    );
    assert_eq!(
        grid.next_visible_cell(address(0, 2)).expect("next row").0,
        address(1, 2)
    );
    assert_eq!(
        grid.previous_visible_cell(address(1, 2))
            .expect("previous")
            .0,
        address(0, 2)
    );
    assert!(grid.previous_visible_cell(address(1, 1)).is_none());
    assert!(grid.visible_cell(address(2, 2)).is_none());
    assert_eq!(model.native_table(owner).expect("native span table"), table);
    assert_eq!(document.source().as_str(), source);
}

#[test]
fn malformed_table_spans_are_diagnosed_without_losing_tags() {
    use yu_markdown::html::HtmlTag;
    for raw in [
        "<td colspan='0'>",
        "<td colspan='1001'>",
        "<td rowspan='65535'>",
        "<td rowspan='-1'>",
        "<td colspan='+2'>",
        "<td colspan='1.5'>",
        "<td colspan='2' COLSPAN='3'>",
        "<div colspan='2'>",
    ] {
        let tag = HtmlTag::parse(raw, ByteOffset::ZERO).expect("source tag");
        assert!(tag.resolve_attributes(raw).is_err(), "{raw}");
        assert_eq!(tag.source.end().get() as usize, raw.len());
    }
}

#[test]
fn covered_grid_slots_resolve_styles_and_paragraphs_from_source_owner() {
    use yu_markdown::{TableAlignment, TableBlock, TableCellAddress};
    let source = "<table><tr><th rowspan='2' align='right'><b>字</b></th><td>A</td></tr><tr><td align='center'><p>B</p></td></tr></table>";
    let document = yu_markdown::parse(&yu_text::TextBuffer::new(source).snapshot());
    let index = document.html_regions();
    let model = index.regions[0].model.as_ref().expect("model");
    let table = model
        .table(model.partitions[0].content.owner.expect("owner"))
        .expect("table");
    let grid = TableBlock::from_html(&table).expect("adapter");
    let covered = TableCellAddress::new(1, 0);
    let shifted = TableCellAddress::new(1, 1);
    assert!(grid.cell_is_header(covered));
    assert!(!grid.cell_is_header(shifted));
    assert_eq!(grid.cell_alignment(covered), TableAlignment::Right);
    assert_eq!(grid.cell_alignment(shifted), TableAlignment::Center);
    let part = grid.cell_paragraphs(shifted).first().expect("paragraph");
    assert_eq!(&source[part.source.start()..part.source.end()], "B");
    assert_eq!(
        grid.cell_paragraphs(covered),
        grid.cell_paragraphs(TableCellAddress::new(0, 0))
    );
}

#[test]
fn merged_column_edits_change_each_source_owner_once_and_preserve_bytes() {
    fn edit(source: &str, column: usize, insert: bool) -> String {
        let document = yu_markdown::parse(&yu_text::TextBuffer::new(source).snapshot());
        let index = document.html_regions();
        let model = index.regions[0].model.as_ref().expect("model");
        let owner = model.partitions[0].content.owner.expect("table");
        let edits = model
            .column_edits(source, owner, column, insert)
            .expect("column plan");
        let mut result = source.to_owned();
        for (range, text) in edits.into_iter().rev() {
            result.replace_range(
                range.start().get() as usize..range.end().get() as usize,
                &text,
            );
        }
        let parsed = yu_markdown::parse(&yu_text::TextBuffer::new(&result).snapshot());
        let index = parsed.html_regions();
        let model = index.regions[0].model.as_ref().expect("result model");
        model
            .table(model.partitions[0].content.owner.expect("result table"))
            .expect("valid grid");
        result
    }
    let source = "<table><tr><td id='a' colspan='2' rowspan='2'><b>中文🙂</b></td><!--keep--><td>B</td></tr>\r\n<tr><td>C</td></tr><tr><td>D</td><td>E</td><td>F</td></tr></table>\r\n";
    let inserted = edit(source, 1, true);
    assert_eq!(
        inserted,
        source
            .replace("colspan='2'", "colspan='3'")
            .replace("<td>D</td>", "<td>D</td><td></td>")
    );
    assert_eq!(edit(&inserted, 1, false), source);
    assert_eq!(
        edit(source, 0, false),
        source
            .replace("colspan='2'", "colspan='1'")
            .replace("<td>D</td>", "")
    );
    assert_eq!(
        edit(source, 2, false),
        source
            .replace("<td>B</td>", "")
            .replace("<td>C</td>", "")
            .replace("<td>F</td>", "")
    );
    let boundary = edit(source, 0, true);
    assert_eq!(boundary, source.replace("<tr>", "<tr><td></td>"));
}

#[test]
fn deleting_rows_preserves_surviving_merged_content_and_groups() {
    fn delete(source: &str, row: usize) -> String {
        let document = yu_markdown::parse(&yu_text::TextBuffer::new(source).snapshot());
        let index = document.html_regions();
        let model = index.regions[0].model.as_ref().expect("model");
        let edits = model
            .delete_row_edits(
                source,
                model.partitions[0].content.owner.expect("table"),
                row,
            )
            .expect("plan");
        let mut result = source.to_owned();
        for (range, text) in edits.into_iter().rev() {
            result.replace_range(
                range.start().get() as usize..range.end().get() as usize,
                &text,
            );
        }
        let parsed = yu_markdown::parse(&yu_text::TextBuffer::new(&result).snapshot());
        let index = parsed.html_regions();
        let model = index.regions[0].model.as_ref().expect("result");
        model
            .table(model.partitions[0].content.owner.expect("table"))
            .expect("valid grid");
        result
    }
    let source = "<table><tbody><tr id='first'><td id='a' rowspan='3'><b>中文🙂</b></td><td rowspan='0'>B</td><td>X</td></tr>\r\n<tr><td>Y</td></tr><tr><td>Z</td></tr></tbody><tbody><tr><td>KEEP</td></tr></tbody></table>\r\n";
    assert_eq!(
        delete(source, 0),
        "<table><tbody>\r\n<tr><td id='a' rowspan='2'><b>中文🙂</b></td><td rowspan='0'>B</td><td>Y</td></tr><tr><td>Z</td></tr></tbody><tbody><tr><td>KEEP</td></tr></tbody></table>\r\n"
    );
    assert_eq!(
        delete(source, 1),
        source
            .replace("rowspan='3'", "rowspan='2'")
            .replace("<tr><td>Y</td></tr>", "")
    );
    assert_eq!(
        delete(source, 2),
        source
            .replace("rowspan='3'", "rowspan='2'")
            .replace("<tr><td>Z</td></tr>", "")
    );
}

#[test]
fn inserting_rows_adjusts_spans_without_crossing_source_groups() {
    fn insert(source: &str, row: usize, before: bool) -> String {
        let document = yu_markdown::parse(&yu_text::TextBuffer::new(source).snapshot());
        let index = document.html_regions();
        let model = index.regions[0].model.as_ref().expect("model");
        let edits = model
            .insert_row_edits(
                source,
                model.partitions[0].content.owner.expect("table"),
                row,
                before,
            )
            .expect("plan");
        let mut result = source.to_owned();
        for (range, text) in edits.into_iter().rev() {
            result.replace_range(
                range.start().get() as usize..range.end().get() as usize,
                &text,
            );
        }
        let document = yu_markdown::parse(&yu_text::TextBuffer::new(&result).snapshot());
        let index = document.html_regions();
        let model = index.regions[0].model.as_ref().expect("model");
        let table = model
            .table(model.partitions[0].content.owner.expect("table"))
            .expect("result grid");
        assert_eq!(table.columns, 3);
        result
    }
    let source = "<table><tbody><tr><td rowspan='2'>中文🙂</td><td rowspan='0'>B</td><td>X</td></tr>\r\n<tr><td>Y</td></tr></tbody><tbody><tr><td>KEEP</td></tr></tbody></table>";
    assert_eq!(
        insert(source, 1, true),
        source
            .replace("rowspan='2'", "rowspan='3'")
            .replace("<tr><td>Y", "<tr><td></td></tr>\r\n<tr><td>Y")
    );
    assert_eq!(
        insert(source, 1, false),
        source.replace(
            "<td>Y</td></tr>",
            "<td>Y</td></tr>\r\n<tr><td></td><td></td></tr>"
        )
    );
    assert_eq!(
        insert(source, 2, true),
        source.replace(
            "<tr><td>KEEP",
            "<tr><td></td><td></td><td></td></tr>\r\n<tr><td>KEEP"
        )
    );
    let overspan = source.replace("rowspan='2'", "rowspan='9'");
    assert_eq!(
        insert(&overspan, 1, false),
        overspan.replace("<td>Y</td></tr>", "<td>Y</td></tr>\r\n<tr><td></td></tr>")
    );
}

#[test]
fn merged_column_alignment_edits_owners_once_and_preserves_other_columns() {
    use yu_markdown::html::HtmlAlignment;
    fn apply(source: &str, alignment: Option<HtmlAlignment>) -> (String, usize) {
        let document = yu_markdown::parse(&yu_text::TextBuffer::new(source).snapshot());
        let index = document.html_regions();
        let model = index.regions[0].model.as_ref().expect("model");
        let edits = model
            .align_column_edits(
                source,
                model.partitions[0].content.owner.expect("table"),
                1,
                alignment,
            )
            .expect("alignment plan");
        let count = edits.len();
        let mut result = source.to_owned();
        for (range, text) in edits.into_iter().rev() {
            result.replace_range(
                range.start().get() as usize..range.end().get() as usize,
                &text,
            );
        }
        (result, count)
    }
    let source = "<table><tr><th colspan='2' rowspan='2' style='text-align:left'><b>中文🙂</b></th><td align='left'>KEEP</td></tr><tr><td>OTHER</td></tr><tr><td>A</td><td id='b'>B</td><td>C</td></tr></table>\r\n";
    let (aligned, count) = apply(source, Some(HtmlAlignment::Right));
    assert_eq!(count, 2);
    assert_eq!(
        aligned,
        source
            .replace("text-align:left", "text-align:right")
            .replace("id='b'", "id='b' align=\"right\"")
    );
    assert_eq!(
        apply(&aligned, Some(HtmlAlignment::Right)),
        (aligned.clone(), 0)
    );
    let (cleared, count) = apply(&aligned, None);
    assert_eq!(count, 2);
    assert!(cleared.contains("<td align='left'>KEEP</td>"));
    assert!(cleared.contains("<b>中文🙂</b>"));
    assert!(!cleared.contains("text-align:right"));
    assert!(!cleared.contains("align=\"right\""));
}

#[test]
fn merged_cell_replacements_deduplicate_covered_slots_and_reject_conflicting_data() {
    let source = "<table><tr><td colspan='2' rowspan='2'><b>中文🙂</b></td><td>B</td></tr><tr><td>C</td></tr></table>\r\n";
    let document = yu_markdown::parse(&yu_text::TextBuffer::new(source).snapshot());
    let index = document.html_regions();
    let model = index.regions[0].model.as_ref().expect("model");
    let owner = model.partitions[0].content.owner.expect("table");
    let edits = model
        .cell_content_edits(
            source,
            owner,
            &[
                (0, 0, "<i>新🙂</i>"),
                (0, 1, "<i>新🙂</i>"),
                (1, 0, "<i>新🙂</i>"),
                (1, 1, "<i>新🙂</i>"),
                (1, 2, "Z"),
            ],
        )
        .expect("one owner per body");
    assert_eq!(edits.len(), 2);
    let mut result = source.to_owned();
    for (range, text) in edits.into_iter().rev() {
        result.replace_range(
            range.start().get() as usize..range.end().get() as usize,
            &text,
        );
    }
    assert_eq!(
        result,
        source
            .replace("<b>中文🙂</b>", "<i>新🙂</i>")
            .replace(">C<", ">Z<")
    );
    assert!(
        model
            .cell_content_edits(source, owner, &[(0, 0, "A"), (1, 1, "B")])
            .is_err()
    );
    assert!(
        model
            .cell_content_edits(source, owner, &[(9, 0, "A")])
            .is_err()
    );
    assert_eq!(document.source().as_str(), source);
}

#[test]
fn splitting_merged_cells_preserves_content_identity_and_visual_grid() {
    let source = "<table><tr><td id='unique' colspan='2' rowspan='2' align='right'><b>中文🙂</b></td><!--keep--><td>B</td></tr>\r\n<tr><td>C</td></tr></table>\r\n";
    let document = yu_markdown::parse(&yu_text::TextBuffer::new(source).snapshot());
    let index = document.html_regions();
    let model = index.regions[0].model.as_ref().expect("model");
    let owner = model.partitions[0].content.owner.expect("table");
    let edits = model
        .split_cell_edits(source, owner, (1, 1, 1, 1))
        .expect("split covering owner");
    assert_eq!(edits.len(), 2);
    let mut result = source.to_owned();
    for (range, text) in edits.into_iter().rev() {
        result.replace_range(
            range.start().get() as usize..range.end().get() as usize,
            &text,
        );
    }
    assert_eq!(result.matches("id='unique'").count(), 1);
    assert_eq!(result.matches("<b>中文🙂</b>").count(), 1);
    assert!(result.contains("<!--keep-->"));
    assert!(result.contains("\r\n"));
    assert!(!result.contains("colspan"));
    assert!(!result.contains("rowspan"));
    let parsed = yu_markdown::parse(&yu_text::TextBuffer::new(&result).snapshot());
    let index = parsed.html_regions();
    let model = index.regions[0].model.as_ref().expect("split model");
    let owner = model.partitions[0].content.owner.expect("table");
    let table = model.native_table(owner).expect("ordinary grid");
    assert_eq!(table.columns, 3);
    let content: Vec<_> = table
        .rows
        .iter()
        .flat_map(|row| &row.cells)
        .map(|cell| &result[cell.content.start().get() as usize..cell.content.end().get() as usize])
        .collect();
    assert_eq!(content, ["<b>中文🙂</b>", "", "B", "", "", "C"]);
    assert!(
        model
            .split_cell_edits(&result, owner, (0, 0, 1, 2))
            .expect("already split")
            .is_empty()
    );
}

#[test]
fn splitting_interleaved_rowspans_orders_insertions_by_visual_column() {
    let source = "<table><tr><td>A</td><td id='b' rowspan='3' align='right'>B</td></tr><tr><td id='c' rowspan='2' align='left'>C</td></tr><tr></tr></table>";
    let document = yu_markdown::parse(&yu_text::TextBuffer::new(source).snapshot());
    let index = document.html_regions();
    let model = index.regions[0].model.as_ref().expect("model");
    let edits = model
        .split_cell_edits(
            source,
            model.partitions[0].content.owner.expect("table"),
            (0, 0, 2, 1),
        )
        .expect("split");
    let mut result = source.to_owned();
    for (range, text) in edits.into_iter().rev() {
        result.replace_range(
            range.start().get() as usize..range.end().get() as usize,
            &text,
        );
    }
    let parsed = yu_markdown::parse(&yu_text::TextBuffer::new(&result).snapshot());
    let index = parsed.html_regions();
    let model = index.regions[0].model.as_ref().expect("result model");
    let table = model
        .native_table(model.partitions[0].content.owner.expect("table"))
        .expect("grid");
    assert_eq!(
        table
            .rows
            .iter()
            .map(|row| row.cells.len())
            .collect::<Vec<_>>(),
        [2, 2, 2]
    );
    let content: Vec<_> = table
        .rows
        .iter()
        .flat_map(|row| &row.cells)
        .map(|cell| &result[cell.content.start().get() as usize..cell.content.end().get() as usize])
        .collect();
    assert_eq!(content, ["A", "B", "C", "", "", ""]);
    assert_eq!(
        table.rows[2].cells[0].alignment,
        Some(yu_markdown::html::HtmlAlignment::Left)
    );
    assert_eq!(
        table.rows[2].cells[1].alignment,
        Some(yu_markdown::html::HtmlAlignment::Right)
    );
    assert_eq!(result.matches("id='b'").count(), 1);
    assert_eq!(result.matches("id='c'").count(), 1);
}

#[test]
fn rectangular_paste_splits_only_intersected_owners_and_fills_sparse_prefixes() {
    fn paste(source: &str, start: (usize, usize), columns: usize, values: &[&str]) -> String {
        let document = yu_markdown::parse(&yu_text::TextBuffer::new(source).snapshot());
        let index = document.html_regions();
        let model = index.regions[0].model.as_ref().expect("model");
        let (range, text) = model
            .paste_cells_edit(
                source,
                model.partitions[0].content.owner.expect("owner"),
                start,
                columns,
                &values.iter().map(|v| v.to_string()).collect::<Vec<_>>(),
            )
            .expect("paste plan");
        let mut result = source.to_owned();
        result.replace_range(
            range.start().get() as usize..range.end().get() as usize,
            &text,
        );
        result
    }
    let source = "<table><tr><td id='a' colspan='2' rowspan='2'><b>KEEP🙂</b></td><td colspan='2'>OUTSIDE</td></tr><tr><td>C</td><td>D</td></tr><tr></tr></table>\r\n";
    let result = paste(source, (1, 0), 2, &["中文", "<i>新</i>"]);
    assert!(result.contains("<b>KEEP🙂</b>"));
    assert_eq!(result.matches("id='a'").count(), 1);
    assert!(result.contains("<td colspan='2'>OUTSIDE</td>"));
    assert!(result.ends_with("\r\n"));
    let result = paste(&result, (2, 2), 2, &["X", "Y"]);
    assert!(result.contains("<tr><td></td><td></td><td>X</td><td>Y</td></tr>"));
}

#[test]
fn single_value_fill_keeps_merged_owners_and_fills_missing_slots() {
    let source = "<table><tr><td id='a' colspan='2' rowspan='2'><b>中文🙂</b></td><td>B</td><td>KEEP</td></tr><tr></tr></table>\r\n";
    let document = yu_markdown::parse(&yu_text::TextBuffer::new(source).snapshot());
    let index = document.html_regions();
    let model = index.regions[0].model.as_ref().expect("model");
    let (range, result) = model
        .fill_cells_edit(
            source,
            model.partitions[0].content.owner.expect("owner"),
            (0, 0, 1, 2),
            "<i>新🙂</i>",
        )
        .expect("fill");
    assert_eq!(&source[range.end().get() as usize..], "\r\n");
    assert_eq!(
        result,
        "<table><tr><td id='a' colspan='2' rowspan='2'><i>新🙂</i></td><td><i>新🙂</i></td><td>KEEP</td></tr><tr><td><i>新🙂</i></td></tr></table>"
    );
    let parsed = yu_markdown::parse(&yu_text::TextBuffer::new(&result).snapshot());
    let index = parsed.html_regions();
    let model = index.regions[0].model.as_ref().expect("filled model");
    let table = model
        .table(model.partitions[0].content.owner.expect("owner"))
        .expect("filled grid");
    assert_eq!(
        table.grid.slots,
        vec![
            Some(0),
            Some(0),
            Some(1),
            Some(2),
            Some(0),
            Some(0),
            Some(3),
            None
        ]
    );
    assert_eq!(result.matches("id='a'").count(), 1);
}

#[test]
fn growing_paste_preserves_row_groups_and_extending_span_owners() {
    let source = "<table><tbody><tr><td rowspan='0'>FIRST</td><td>X</td></tr></tbody>\r\n<tbody><tr><td id='b' rowspan='0'>KEEP🙂</td><td>C</td></tr></tbody></table>\r\n";
    let document = yu_markdown::parse(&yu_text::TextBuffer::new(source).snapshot());
    let index = document.html_regions();
    let model = index.regions[0].model.as_ref().expect("model");
    let owner = model.partitions[0].content.owner.expect("table");
    let payload: Vec<_> = (0..9).map(|n| format!("值{n}")).collect();
    let (range, result) = model
        .paste_cells_edit(source, owner, (1, 1), 3, &payload)
        .expect("grow and paste");
    assert_eq!(&source[range.end().get() as usize..], "\r\n");
    assert!(result.contains("id='b' rowspan='0'>KEEP🙂"));
    let parsed = yu_markdown::parse(&yu_text::TextBuffer::new(&result).snapshot());
    let index = parsed.html_regions();
    let model = index.regions[0].model.as_ref().expect("grown model");
    let table = model
        .table(model.partitions[0].content.owner.expect("owner"))
        .expect("grid");
    assert_eq!((table.rows.len(), table.columns), (4, 4));
    assert_ne!(table.rows[0].group, table.rows[1].group);
    assert_eq!(table.rows[1].group, table.rows[3].group);
    let b = table.grid.slots[4].expect("B owner");
    assert_eq!(table.grid.slots[8], Some(b));
    assert_eq!(table.grid.slots[12], Some(b));
    assert_eq!(table.grid.cells[0].rows, 1);
    for (n, text) in payload.iter().enumerate() {
        let id = table.grid.slots[(1 + n / 3) * 4 + 1 + n % 3].expect("payload owner");
        let cell = table.grid.cells[id];
        let range = table.rows[cell.source_row].cells[cell.source_cell].content;
        assert_eq!(
            &result[range.start().get() as usize..range.end().get() as usize],
            text
        );
    }
}

#[test]
fn growing_table_rejects_unbounded_dimensions_before_allocating() {
    let source = "<table><tr><td>A</td></tr></table>";
    let document = yu_markdown::parse(&yu_text::TextBuffer::new(source).snapshot());
    let index = document.html_regions();
    let model = index.regions[0].model.as_ref().expect("model");
    let owner = model.partitions[0].content.owner.expect("owner");
    assert!(model.grow_table_edit(source, owner, usize::MAX, 2).is_err());
    assert!(model.grow_table_edit(source, owner, 1025, 1024).is_err());
    assert_eq!(document.source().as_str(), source);
}
