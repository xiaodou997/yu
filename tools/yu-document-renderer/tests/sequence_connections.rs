//! Production parser/layout/helper regressions; not macOS real-window evidence.
use mermaid_rs_renderer::ir::{Graph, NodeShape, SequenceActivationKind, SequenceConnection};
use mermaid_rs_renderer::layout::{DiagramData, Layout, SequenceData};
use mermaid_rs_renderer::{LayoutConfig, Theme, compute_layout, parse_mermaid_strict};
use yu_document_renderer::{Kind, RenderStyle, render, render_styled};

fn parsed(source: &str) -> Graph {
    parse_mermaid_strict(source)
        .expect("valid sequence fixture")
        .graph
}
fn layout(graph: &Graph) -> Layout {
    compute_layout(graph, &Theme::modern(), &LayoutConfig::default())
}
fn sequence(layout: &Layout) -> &SequenceData {
    let DiagramData::Sequence(data) = &layout.diagram else {
        panic!("sequence layout expected")
    };
    data
}
fn near(actual: f32, expected: f32) {
    assert!((actual - expected).abs() < 0.002, "{actual} != {expected}");
}
fn both_themes(source: &str, circles: usize) {
    for dark in [false, true] {
        let svg = render_styled(
            Kind::Mermaid,
            source,
            RenderStyle {
                dark,
                foreground: if dark { 0xffffffff } else { 0x000000ff },
                ..RenderStyle::default()
            },
        )
        .expect("native sequence rendering");
        assert_eq!(
            svg.svg
                .matches("class=\"sequence-central-connection\"")
                .count(),
            circles
        );
        assert!(!svg.svg.contains("NaN") && !svg.svg.contains("Infinity"));
        assert!(svg.width > 0 && svg.height > 0);
    }
}

#[test]
fn central_endpoints_preserve_all_existing_arrow_kinds_directions_and_styles() {
    // Test ordinary metadata as the control, independent of central markers.
    let arrows = [
        "<<-->>", "<<->>", r"--|\", "--|/", r"--\\", "--//", "/|--", r"\|--", "//--", r"\\--",
        "-->>", r"-|\", "-|/", r"-\\", "-//", "/|-", r"\|-", "//-", r"\\-", "->>", "--x", "-x",
        "--)", "-)", "-->", "->", "<--", "<-",
    ];
    for arrow in arrows {
        let reverse = arrow.starts_with(['/', '\\']) || matches!(arrow, "<-" | "<--");
        for order in [
            "participant A\nparticipant B",
            "participant B\nparticipant A",
        ] {
            let control = parsed(&format!("sequenceDiagram\n{order}\nA{arrow}B: 消息🙂"));
            for (left, right) in [(false, true), (true, false), (true, true)] {
                let source = format!(
                    "sequenceDiagram\n{order}\nA{}{arrow}{}B: 消息🙂",
                    if left { "()" } else { "" },
                    if right { "()" } else { "" }
                );
                let graph = parsed(&source);
                let edge = &graph.edges[0];
                let before = &control.edges[0];
                assert_eq!(graph.nodes.len(), 2, "{source}");
                assert_eq!((&edge.from, &edge.to), (&before.from, &before.to));
                assert_eq!(
                    (
                        edge.arrow_start,
                        edge.arrow_end,
                        edge.arrow_start_kind,
                        edge.arrow_end_kind,
                        edge.start_decoration,
                        edge.end_decoration,
                        edge.style
                    ),
                    (
                        before.arrow_start,
                        before.arrow_end,
                        before.arrow_start_kind,
                        before.arrow_end_kind,
                        before.start_decoration,
                        before.end_decoration,
                        before.style
                    )
                );
                assert_eq!(
                    graph.sequence_connections[&0],
                    if reverse {
                        SequenceConnection {
                            from: right,
                            to: left,
                        }
                    } else {
                        SequenceConnection {
                            from: left,
                            to: right,
                        }
                    }
                );
                assert!(graph.sequence_activations.is_empty());
                let laid = layout(&graph);
                for circle in &sequence(&laid).connections {
                    let node = &laid.nodes[&circle.participant];
                    near(circle.x, node.x + node.width / 2.0);
                    let edge = &laid.edges[circle.message];
                    let tip = if circle.at_start {
                        edge.points.first()
                    } else {
                        edge.points.last()
                    }
                    .expect("endpoint");
                    near((tip.0 - circle.x).hypot(tip.1 - circle.y), circle.radius);
                    assert!(
                        circle.x - circle.radius >= 0.0 && circle.x + circle.radius <= laid.width
                    );
                    assert!(
                        circle.y - circle.radius >= 0.0 && circle.y + circle.radius <= laid.height
                    );
                }
                both_themes(&source, usize::from(left) + usize::from(right));
            }
        }
    }
}

#[test]
fn fixed_acceptance_samples_have_explicit_success_and_diagnostic_expectations() {
    let source = include_str!("fixtures/sequence-central.mmd");
    for ending in ["\n", "\r\n"] {
        let input = source.replace('\n', ending);
        let before = input.as_bytes().to_vec();
        let graph = parsed(&input);
        assert_eq!(graph.sequence_created["Worker"], 3);
        assert_eq!(graph.sequence_destroyed["Worker"], 5);
        both_themes(&input, 11);
        assert_eq!(input.as_bytes(), before);
        let reopened = String::from_utf8(before).expect("saved UTF-8");
        assert_eq!(
            parsed(&reopened).sequence_connections,
            graph.sequence_connections
        );
    }
    let manual =
        include_str!("../../../platform/macos/yu-shell-macos/Fixtures/group4-sequence-central.md");
    let blocks = manual
        .split("```mermaid\n")
        .skip(1)
        .map(|tail| tail.split_once("```").expect("closed diagram").0)
        .collect::<Vec<_>>();
    assert_eq!(blocks.len(), 5);
    assert_eq!(blocks[0], source);
    for (index, block) in blocks.iter().enumerate() {
        for dark in [false, true] {
            let outcome = render_styled(
                Kind::Mermaid,
                block,
                RenderStyle {
                    dark,
                    ..RenderStyle::default()
                },
            );
            assert_eq!(outcome.is_ok(), index < 3, "sample {index}: {outcome:?}");
        }
    }
}

#[test]
fn connection_bounds_and_number_clearance_scale_with_font_and_theme() {
    let source = "sequenceDiagram\nautonumber 1.25 0.25\nparticipant A\nparticipant B\nA()->>()B: 一\nB()->>()A: 二\nA()->>()A: 三";
    let graph = parsed(source);
    for mut theme in [Theme::modern(), Theme::dark()] {
        for font in [8.0, 16.0, 24.0, 48.0, 96.0] {
            theme.font_size = font;
            let laid = compute_layout(&graph, &theme, &LayoutConfig::default());
            mermaid_rs_renderer::layout::validate_layout_invariants(&laid)
                .expect("finite bounded sequence layout");
            let data = sequence(&laid);
            for circle in &data.connections {
                for number in &data.numbers {
                    assert!(
                        (circle.x - number.x).abs() > circle.radius + number.width / 2.0
                            || (circle.y - number.y).abs() > circle.radius + number.height / 2.0,
                        "font {font}: number overlaps connection"
                    );
                }
            }
        }
    }
}

#[test]
fn official_three_forms_keep_numbers_and_do_not_implicitly_activate() {
    let source = "sequenceDiagram\nautonumber 1.25 0.25\nparticipant Alice\nparticipant John\nAlice->>()John: Hello\nAlice()->>John: How are you?\nJohn()->>()Alice: Great!";
    let graph = parsed(source);
    assert_eq!(graph.sequence_connections.len(), 3);
    assert_eq!(
        graph
            .sequence_message_numbers
            .iter()
            .map(|n| n.expect("number").to_string())
            .collect::<Vec<_>>(),
        ["1.25", "1.5", "1.75"]
    );
    assert!(graph.sequence_activations.is_empty());
    let laid = layout(&graph);
    let data = sequence(&laid);
    assert!(data.activations.is_empty());
    assert_eq!(data.numbers.len(), 3);
    for circle in &data.connections {
        for number in &data.numbers {
            let separated = (circle.x - number.x).abs() > circle.radius + number.width / 2.0
                || (circle.y - number.y).abs() > circle.radius + number.height / 2.0;
            assert!(separated, "number must not cover central marker");
        }
    }
    both_themes(source, 4);
}

#[test]
fn whitespace_unicode_aliases_and_message_parentheses_do_not_create_fake_participants() {
    let source = "sequenceDiagram\nparticipant 客户 as 中文🙂\nparticipant 服务 as 服务端\n客户 () -->> () 服务: 文本 () + - ->>() 不属于语法\n服务 ->> 客户: ordinary ()";
    let graph = parsed(source);
    assert_eq!(graph.nodes.len(), 2);
    assert_eq!(graph.nodes["客户"].label, "中文🙂");
    assert_eq!(graph.sequence_connections.len(), 1);
    assert_eq!(
        graph.edges[0].label.as_deref(),
        Some("文本 () + - ->>() 不属于语法")
    );
    both_themes(source, 2);
}

#[test]
fn malformed_central_tokens_and_mixed_activation_suffixes_are_rejected_then_recover() {
    for arrow in [
        "->>()()", "()()->>", "( )->>", "->>( )", "()->>+", "()->>-", "->>+()", "->>-()", "->>()+",
        "->>()-", "()->>()+", "->>>( )", "->>()>", "->>++", "->>--",
    ] {
        let source = format!("sequenceDiagram\nparticipant A\nparticipant B\nA{arrow}B: invalid");
        assert!(render(Kind::Mermaid, &source).is_err(), "{arrow}");
        both_themes("sequenceDiagram\nA()->>()B: recovery", 2);
    }
}

#[test]
fn self_central_connections_use_send_and_return_endpoints_and_keep_next_content_below() {
    for arrow in ["()->>", "->>()", "()->>()", "()-->>()", "()<<->>()"] {
        let source = format!(
            "sequenceDiagram\nparticipant A\nA{arrow}A: 自调用🙂\nNote over A: After self\nA->>A: next"
        );
        let graph = parsed(&source);
        let laid = layout(&graph);
        let data = sequence(&laid);
        let edge = &laid.edges[0];
        for circle in &data.connections {
            near(
                circle.y,
                if circle.at_start {
                    edge.points[0].1
                } else {
                    edge.points.last().expect("return").1
                },
            );
            assert!(data.notes[0].y > circle.y + circle.radius);
        }
        if data.connections.len() == 2 {
            assert!(data.connections[1].y > data.connections[0].y);
        }
        both_themes(&source, data.connections.len());
    }
}

#[test]
fn creation_central_markers_preserve_actor_shape_alias_and_header_attachment() {
    for kind in ["actor", "participant"] {
        for receiver_left in [false, true] {
            let source = format!(
                "sequenceDiagram\nparticipant A\ncreate {kind} B as 新建🙂\nA()->>()B: Create\nactivate B\nB()->>()B: Work\ndeactivate B\ndestroy B\nB()->>A: Finish"
            );
            let mut graph = parsed(&source);
            // Exercise the layout contract in both spatial orders without
            // pretending a dynamically created participant was predeclared.
            if receiver_left {
                graph.sequence_participants.reverse();
            }
            assert_eq!(graph.sequence_created["B"], 0);
            assert_eq!(
                graph.nodes["B"].shape,
                if kind == "actor" {
                    NodeShape::Actor
                } else {
                    NodeShape::ActorBox
                }
            );
            let laid = layout(&graph);
            let data = sequence(&laid);
            let circle = data
                .connections
                .iter()
                .find(|c| c.message == 0 && !c.at_start)
                .expect("creation receiver");
            let node = &laid.nodes["B"];
            near(circle.y, node.y + node.height / 2.0);
            near(
                circle.x,
                if kind == "actor" {
                    node.x + node.width / 2.0
                } else if receiver_left {
                    node.x + node.width
                } else {
                    node.x
                },
            );
            let life = data
                .lifelines
                .iter()
                .find(|life| life.id == "B")
                .expect("created lifetime");
            assert!(!data.footboxes.iter().any(|box_| box_.id == "B"));
            for bar in data.activations.iter().filter(|bar| bar.participant == "B") {
                assert!(bar.y >= life.y1 && bar.y + bar.height <= life.y2 + 0.001);
            }
            both_themes(&source, 5);
        }
    }
}

#[test]
fn destruction_of_sender_receiver_and_self_clips_all_nested_activation_bars() {
    for (message, destroyed) in [
        ("B()->>A: Finish", "B"),
        ("A->>()B: Finish", "B"),
        ("B()->>()B: Finish", "B"),
    ] {
        let source = format!(
            "sequenceDiagram\nparticipant A\nparticipant B\nA->>+B: Start\nB->>+B: Nested\ndestroy {destroyed}\n{message}\nA->>A: Continue"
        );
        let graph = parsed(&source);
        let laid = layout(&graph);
        let data = sequence(&laid);
        let life = data
            .lifelines
            .iter()
            .find(|life| life.id == destroyed)
            .expect("destroyed life");
        let circle = data
            .connections
            .iter()
            .rev()
            .find(|c| c.participant == destroyed)
            .expect("destruction connection");
        near(life.y2, circle.y);
        assert_eq!(data.activations.len(), 2);
        for bar in &data.activations {
            near(bar.y + bar.height, life.y2);
        }
        both_themes(
            &source,
            if message.starts_with("B()->>()B") {
                2
            } else {
                1
            },
        );
        let svg = render(Kind::Mermaid, &source).expect("destruction SVG").svg;
        assert_eq!(svg.matches("class=\"sequence-destruction\"").count(), 1);
    }
}

#[test]
fn standalone_activation_closes_at_previous_message_not_the_next_message() {
    let source = "sequenceDiagram\nparticipant A\nparticipant B\nA->>B: Call\nactivate B\nB()->>()B: Self\ndeactivate B\nA->>B: Later";
    let graph = parsed(source);
    assert!(!graph.sequence_activations[0].at_message);
    let laid = layout(&graph);
    let bar = &sequence(&laid).activations[0];
    near(bar.y, laid.edges[0].points.last().expect("call arrival").1);
    near(
        bar.y + bar.height,
        laid.edges[1].points.last().expect("self return").1,
    );
    assert!(bar.y + bar.height < laid.edges[2].points[0].1);
    both_themes(source, 2);
}

#[test]
fn standalone_activation_order_around_notes_and_inline_self_return_is_preserved() {
    let source = "sequenceDiagram\nparticipant A\nparticipant B\nA->>B: Call\nNote over B: Before activation\nactivate B\nB()->>()B: Work\nNote over B: Before deactivation\ndeactivate B\nA->>B: Later";
    let graph = parsed(source);
    let laid = layout(&graph);
    let data = sequence(&laid);
    let bar = &data.activations[0];
    near(bar.y, data.notes[0].y + data.notes[0].height);
    near(bar.y + bar.height, data.notes[1].y + data.notes[1].height);
    let self_source = "sequenceDiagram\nA->>+A: Start\nA->>-A: End\nA->>A: Later";
    let self_laid = layout(&parsed(self_source));
    let self_bar = &sequence(&self_laid).activations[0];
    near(
        self_bar.y,
        self_laid.edges[0].points.last().expect("start arrival").1,
    );
    near(
        self_bar.y + self_bar.height,
        self_laid.edges[1].points.last().expect("end return").1,
    );
    both_themes(source, 2);
}

#[test]
fn inactive_or_destroyed_activation_and_pending_lifecycle_errors_are_diagnostic() {
    for body in [
        "deactivate A",
        "A-->>-B: inactive",
        "A->>()B: circle\ndeactivate B",
        "destroy B\nA->>+B: impossible",
        "destroy B\nB->>A: end\nactivate B",
        "create participant C",
        "destroy B",
        "create actor C\nC()->>A: wrong receiver",
        "create participant C\ndestroy C\nA->>()C: zero lifetime",
        "destroy B\nA->>()A: unrelated",
        "create actor C\nNote over A: intervening\nA->>()C: late",
        "destroy B\nB->>()A: end\nB()->>A: stale",
        "create actor C\nA->>()C: create\ncreate actor C\nA->>()C: duplicate",
    ] {
        let source = format!("sequenceDiagram\nparticipant A\nparticipant B\n{body}");
        assert!(render(Kind::Mermaid, &source).is_err(), "{body}");
    }
    both_themes("sequenceDiagram\nA()->>()B: intact", 2);
}

#[test]
fn one_message_can_create_receiver_and_destroy_sender_without_swapping_identities() {
    for directives in [
        "create actor B as 新工作者\ndestroy A",
        "destroy A\ncreate actor B as 新工作者",
    ] {
        let source = format!(
            "sequenceDiagram\nparticipant A\nactivate A\n{directives}\nA()->>()B: Transfer\nB()->>()B: Work"
        );
        let graph = parsed(&source);
        assert_eq!(graph.sequence_created["B"], 0);
        assert_eq!(graph.sequence_destroyed["A"], 0);
        let laid = layout(&graph);
        assert!(sequence(&laid).footboxes.iter().all(|node| node.id != "A"));
        assert!(sequence(&laid).footboxes.iter().any(|node| node.id == "B"));
        both_themes(&source, 4);
    }
}

#[test]
fn nested_frames_and_central_messages_keep_lifecycle_ownership() {
    let source = "sequenceDiagram\nautonumber\nparticipant A\nparticipant B\nA->>+B: Start\nloop Work\nalt primary\nB()->>()A: Primary\nelse secondary\nB-->>()A: Secondary\nend\npar first\nA()->>B: One\nand second\nB()->>()B: Two\nend\ncritical Commit\nB->>()A: Commit\noption Recover\nB()->>A: Recover\nend\nend\ndestroy B\nB-->>-A: Done\nA()->>A: After";
    let graph = parsed(source);
    assert_eq!(graph.sequence_frames.len(), 4);
    assert_eq!(graph.sequence_activations.len(), 2);
    assert_eq!(
        graph.sequence_activations[1].kind,
        SequenceActivationKind::Deactivate
    );
    assert_eq!(graph.sequence_activations[1].participant, "B");
    let laid = layout(&graph);
    let data = sequence(&laid);
    assert_eq!(data.frames.len(), 4);
    for frame in &data.frames {
        assert!(frame.y + frame.height < laid.edges.last().expect("after frames").points[0].1);
    }
    both_themes(source, 9);
}
