use yu_core::{ByteOffset, TextRange};
use yu_editor::{DecorationCache, VisualText};
use yu_text::TextBuffer;

fn projected(source: &str, active: Option<TextRange>) -> String {
    let snapshot = TextBuffer::new(source).snapshot();
    let markdown = yu_markdown::parse(&snapshot);
    let mut cache = DecorationCache::default();
    let mut output = String::new();
    for block in markdown.blocks().iter() {
        let decorations = cache.decorate(&markdown, block, active).expect("decorate");
        output.push_str(
            VisualText::new(&snapshot, block.range(), decorations.set().clone())
                .expect("visual")
                .text(),
        );
    }
    assert_eq!(markdown.source().as_str(), source);
    output
}

#[test]
fn inline_html_formatting_is_nested_case_insensitive_and_source_preserving() {
    assert_eq!(
        projected(
            "Before <STRONG>中文 <em>italic</em></STRONG> <mark>x<sup>2</sup></mark> H<sub>2</sub>O.",
            None
        ),
        "Before 中文 italic x2 H2O."
    );
    assert_eq!(
        projected(
            "Before <b>**bold** &amp; [link](https://example.com)</b>.",
            None
        ),
        "Before bold & link."
    );
}

#[test]
fn unsupported_attributes_unknown_containers_and_broken_tags_stay_editable() {
    for source in [
        "Before <b class='x'>raw</b>.",
        "Before <b onclick='run()'>raw</b>.",
        "Before <unknown><b>raw</b></unknown>.",
        "Before <b><i>raw</b></i>.",
        "Before <b>unclosed.",
        "Before <b/>raw</b>.",
        "Before <code><b>raw</b></code>.",
    ] {
        assert_eq!(projected(source, None), source, "{source}");
    }
    assert_eq!(
        projected("Before `<b>code</b>` $<b>math</b>$.", None),
        "Before <b>code</b> ."
    );
}

#[test]
fn active_html_format_range_reveals_tags_without_rewriting_content() {
    let source = "Before <b>中文</b> after <i>other</i>.";
    let position = source.find("中文").expect("content");
    let active = TextRange::empty(ByteOffset::new(position as u64));
    assert_eq!(
        projected(source, Some(active)),
        "Before <b>中文</b> after other."
    );
}

fn structural_projection(source: &str, active: Option<TextRange>) -> Vec<String> {
    let snapshot = TextBuffer::new(source).snapshot();
    let document = yu_markdown::parse(&snapshot);
    let mut results = Vec::new();
    for region in &document.html_regions().regions {
        let model = region.model.as_ref().expect("html model");
        for part in &model.partitions {
            let mut output = yu_markdown::ExtensionOutput::default();
            let handled = model.decorate_paragraph(source, part, active, &mut output);
            let set = yu_decoration::DecorationSet::new(
                snapshot.revision(),
                ByteOffset::new(source.len() as u64),
                output.ranges().iter().cloned(),
            );
            let visual = VisualText::new(&snapshot, part.source, set).expect("projection");
            if !handled {
                assert_eq!(
                    visual.text(),
                    &source[part.source.start().get() as usize..part.source.end().get() as usize]
                );
            }
            let mut previous = 0;
            let raw = &source[part.source.start().get() as usize..part.source.end().get() as usize];
            for boundary in raw
                .char_indices()
                .map(|(i, _)| i)
                .chain(std::iter::once(raw.len()))
            {
                let mapped = visual
                    .canonical_source_to_visual(ByteOffset::new(
                        part.source.start().get() + boundary as u64,
                    ))
                    .get();
                assert!(mapped >= previous && mapped <= visual.text().len() as u64);
                previous = mapped;
            }
            assert_eq!(previous, visual.text().len() as u64);
            results.push(visual.text().to_owned());
        }
    }
    assert_eq!(document.source().as_str(), source);
    results
}

#[test]
fn structural_html_projects_separate_paragraphs_with_entities_and_collapsed_spaces() {
    let source = "<div>\n<h2> 标题 &amp; 中文 </h2>\n<p>  hello &#32;<b> world </b> \t<em> again</em><br>next&nbsp;line </p>\n<p>第三段😀</p>\n</div>\n";
    assert_eq!(
        structural_projection(source, None),
        [
            "标题 & 中文",
            "hello world again\nnext\u{a0}line",
            "第三段😀"
        ]
    );
}

#[test]
fn structural_html_preserves_unknown_subtrees_and_active_partition_bytes() {
    let source = "<div><p>before <unknown><b> raw </b></unknown> after <img src='x.png'></p><p>other</p></div>";
    assert_eq!(
        structural_projection(source, None),
        [
            "before <unknown><b> raw </b></unknown> after <img src='x.png'>",
            "other"
        ]
    );
    let active = TextRange::empty(ByteOffset::new(source.find("before").expect("text") as u64));
    assert_eq!(
        structural_projection(source, Some(active)),
        [
            "<div><p>before <unknown><b> raw </b></unknown> after <img src='x.png'></p>",
            "other"
        ]
    );
}

#[test]
fn production_html_blocks_share_projection_outline_and_source_mode_history() {
    use yu_editor::{EditorCommand, EditorDocument};
    let source = "<div><h2>标题 <em>中文</em></h2><ol start='3'><li>one</li><li>two</li></ol><p>end</p></div>\n";
    assert_eq!(projected(source, None), "标题 中文onetwoend");
    let mut document = EditorDocument::new(source);
    let mut cache = DecorationCache::default();
    let blocks: Vec<_> = document.markdown().blocks().iter().collect();
    assert_eq!(blocks.len(), 4);
    let heading = cache
        .decorate(document.markdown(), blocks[0], None)
        .expect("heading");
    assert!(
        heading
            .line_ornaments()
            .iter()
            .any(|(_, o)| matches!(o, yu_markdown::BlockOrnament::Heading { level: 2 }))
    );
    for (index, number) in [(1, "3."), (2, "4.")] {
        let decoration = cache
            .decorate(document.markdown(), blocks[index], None)
            .expect("list");
        assert!(decoration.line_ornaments().iter().any(|(_, o)| matches!(o, yu_markdown::BlockOrnament::Marker(marker) if marker.text() == number)));
    }
    document.set_source_mode(true).expect("source");
    let raw = document.markdown().blocks().get(0).expect("raw block");
    let decoration = cache
        .decorate(document.markdown(), raw, None)
        .expect("literal");
    assert_eq!(
        VisualText::new(
            document.markdown().source(),
            raw.range(),
            decoration.set().clone()
        )
        .expect("raw text")
        .text(),
        source
    );
    document
        .execute(EditorCommand::insert_text("prefix\n\n"))
        .expect("edit");
    document.set_source_mode(false).expect("preview");
    document.undo().expect("undo across modes");
    assert_eq!(document.markdown().source().as_str(), source);
    assert_eq!(document.markdown().blocks().len(), 4);
    document.redo().expect("redo");
    assert_eq!(
        document.markdown().source().as_str(),
        format!("prefix\n\n{source}")
    );
}

#[test]
fn crlf_blank_line_ends_html_before_following_markdown_and_math() {
    for newline in ["\n", "\r\n"] {
        let source = "<div>\n<h2>HTML title</h2>\n<p>body</p>\n</div>\n\n# Markdown title\n\n```math\nx^2\n```\n".replace('\n', newline);
        let document = yu_markdown::parse(&TextBuffer::new(&source).snapshot());
        let headings: Vec<_> = document
            .blocks()
            .iter()
            .filter_map(|block| match block.kind() {
                yu_markdown::BlockKind::Heading { level } => Some(level),
                _ => None,
            })
            .collect();
        assert_eq!(headings, [2, 1], "{newline:?}");
        let html_end = document.html_regions().regions[0].source.end().get() as usize;
        assert!(html_end <= source.find("# Markdown").expect("markdown"));
        assert_eq!(document.source().as_str(), source);
    }
}

#[test]
fn structural_html_links_project_labels_and_keep_decoded_source_backed_targets() {
    let source = "<div><p>Read <a href='https://example.com/中文?q=1&amp;x=2'><b>文档</b> now</a>.</p><p><a href='#section'>Jump</a> <a id='section'>Anchor</a></p></div>";
    assert_eq!(projected(source, None), "Read 文档 now.Jump Anchor");
    let snapshot = TextBuffer::new(source).snapshot();
    let document = yu_markdown::parse(&snapshot);
    let model = document.html_regions().regions[0]
        .model
        .as_ref()
        .expect("model");
    let links = model.links();
    assert_eq!(links.len(), 2);
    assert_eq!(
        links[0].destination_text,
        "https://example.com/中文?q=1&x=2"
    );
    assert_eq!(
        &source[links[0].destination.start().get() as usize
            ..links[0].destination.end().get() as usize],
        "https://example.com/中文?q=1&amp;x=2"
    );
    assert_eq!(
        &source[links[0].label.start().get() as usize..links[0].label.end().get() as usize],
        "<b>文档</b> now"
    );
    assert_eq!(links[1].destination_text, "#section");
    let mut cache = DecorationCache::default();
    let decorations = cache
        .decorate(
            &document,
            document.blocks().get(0).expect("paragraph"),
            None,
        )
        .expect("decoration");
    assert!(
        decorations
            .styles()
            .iter()
            .any(|attrs| attrs.role() == yu_core::TextRole::Link)
    );
    let active = TextRange::empty(links[0].label.start());
    assert!(
        projected(source, Some(active)).contains("href='https://example.com/中文?q=1&amp;x=2'")
    );
}

#[test]
fn unresolved_html_links_stay_source_and_have_no_target_metadata() {
    for source in [
        "<div><p><a href='x' onclick='run()'>text</a></p></div>",
        "<div class='unknown'><a href='x'>text</a></div>",
        "<div><a href='x' HREF='y'>text</a></div>",
        "<div><a href='x'>unclosed</div>",
    ] {
        let document = yu_markdown::parse(&TextBuffer::new(source).snapshot());
        let model = document.html_regions().regions[0]
            .model
            .as_ref()
            .expect("model");
        assert!(model.links().is_empty(), "{source}");
        assert!(projected(source, None).contains("href='x'"));
    }
}

#[test]
fn inline_html_link_uses_shared_finite_attribute_rules_and_active_source() {
    let source = "Before <a href='https://example.com/?a=1&amp;b=2'><b>中文</b> link</a> after.";
    assert_eq!(projected(source, None), "Before 中文 link after.");
    assert!(
        projected(
            source,
            Some(TextRange::empty(ByteOffset::new(
                source.find("中文").expect("label") as u64
            )))
        )
        .contains("href='https://example.com/?a=1&amp;b=2'")
    );
    let unknown = "Before <a href='x' onclick='run()'>label</a> after.";
    assert_eq!(projected(unknown, None), unknown);
    assert_eq!(
        projected("Before <a id='part'>anchor</a> after", None),
        "Before anchor after"
    );
}

#[test]
fn html_text_lines_compose_with_font_traits_in_paragraphs_and_tables() {
    use yu_editor::{BlockLayoutInput, LayoutConfig, MonospaceMetrics};
    use yu_layout::StyleTable;
    for source in [
        "Before <u><del><b>中文X</b></del></u> after",
        "<p>Before <u><s><b>中文X</b></s></u> after</p>",
        "<table><tr><td><u><strike><b>中文X</b></strike></u></td><td>plain</td></tr></table>",
    ] {
        let snapshot = TextBuffer::new(source).snapshot();
        let document = yu_markdown::parse(&snapshot);
        let block = document.blocks().get(0).expect("block");
        let decorations = DecorationCache::default()
            .decorate(&document, block, None)
            .expect("decoration");
        let visual =
            VisualText::new(&snapshot, block.range(), decorations.set().clone()).expect("visual");
        assert!(!visual.text().contains('<'));
        let input = BlockLayoutInput::from_decorations(
            block.kind(),
            &decorations,
            &visual,
            LayoutConfig::new(320.0, 26.0),
            &MonospaceMetrics::new(8.0),
        )
        .expect("layout input");
        let attrs = input
            .layout_input()
            .runs()
            .iter()
            .map(|run| input.styles().attrs(run.style()).expect("style"))
            .collect::<Vec<_>>();
        let marked = attrs
            .iter()
            .filter(|attrs| attrs.underlined() || attrs.struck())
            .collect::<Vec<_>>();
        assert!(!marked.is_empty());
        assert!(
            marked
                .iter()
                .all(|attrs| attrs.underlined() && attrs.struck() && attrs.style().is_strong())
        );
        assert!(
            attrs
                .iter()
                .any(|attrs| !attrs.underlined() && !attrs.struck())
        );
        assert_eq!(snapshot.as_str(), source);
    }
    assert_eq!(
        projected("Before <u class='unknown'>keep</u> after", None),
        "Before <u class='unknown'>keep</u> after"
    );
}
