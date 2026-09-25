//! Cross-level item ownership, exact endpoint mapping and live history contracts.
use yu_core::ByteOffset;
use yu_editor::{CaretAffinity, EditorCommand, EditorDocument, EditorSelection, Selections};

fn marked(text: &str) -> (String, Vec<(usize, usize)>) {
    let mut output = String::new();
    let mut endpoints = std::collections::BTreeMap::<usize, (Option<usize>, Option<usize>)>::new();
    let mut remaining = text;
    while let Some(start) = remaining.find('«') {
        output.push_str(&remaining[..start]);
        let tail = &remaining[start + '«'.len_utf8()..];
        let end = tail.find('»').expect("closed endpoint marker");
        let marker = &tail[..end];
        let id = marker[1..].parse::<usize>().expect("selection index");
        let pair = endpoints.entry(id).or_default();
        let slot = match &marker[..1] {
            "A" => &mut pair.0,
            "F" => &mut pair.1,
            _ => panic!("endpoint kind"),
        };
        assert!(slot.replace(output.len()).is_none(), "duplicate endpoint");
        remaining = &tail[end + '»'.len_utf8()..];
    }
    output.push_str(remaining);
    let count = endpoints.len();
    let pairs = (0..count)
        .map(|id| {
            let (a, f) = endpoints.remove(&id).expect("contiguous selection ids");
            (a.expect("anchor"), f.expect("focus"))
        })
        .collect();
    (output, pairs)
}

fn selections(
    doc: &EditorDocument,
    pairs: &[(usize, usize)],
    reverse: bool,
    primary: usize,
) -> Selections {
    let snapshot = doc.snapshot();
    Selections::new(
        &snapshot,
        pairs.iter().enumerate().map(|(i, (a, f))| {
            let (a, f) = if reverse { (*f, *a) } else { (*a, *f) };
            EditorSelection::range(
                &snapshot,
                ByteOffset::new(a as u64),
                ByteOffset::new(f as u64),
                if i % 2 == 0 {
                    CaretAffinity::Upstream
                } else {
                    CaretAffinity::Downstream
                },
            )
            .expect("source endpoints")
        }),
        primary,
    )
    .expect("selection set")
}

fn seed(source: &str) -> EditorDocument {
    let mut doc = EditorDocument::new(source);
    for text in [" HISTORY-A", " HISTORY-B"] {
        let snapshot = doc.snapshot();
        doc.set_selection(
            EditorSelection::cursor(&snapshot, snapshot.len_bytes(), CaretAffinity::Downstream)
                .expect("end"),
        )
        .expect("separate history group");
        doc.execute(EditorCommand::insert_text(text))
            .expect("seed edit");
    }
    doc.undo().expect("live redo branch");
    assert_eq!(doc.history_stats().undo_entries(), 1);
    assert_eq!(doc.history_stats().redo_entries(), 1);
    doc
}

fn matrix(fixture: &str, indent: bool, supported: bool, complete_document: bool) {
    let normalized = fixture.replace("\r\n", "\n");
    let (before, after) = normalized
        .trim_end_matches('\n')
        .split_once("\n@@AFTER@@\n")
        .expect("before/after fixture");
    for cell in [false, true] {
        if cell && complete_document {
            continue;
        }
        for ending in ["\n", "\r\n"] {
            for bom in ["", "\u{feff}"] {
                let wrap = |text: &str| {
                    let text = if cell {
                        format!("<table><tr><td>{text}</td><td>NEIGHBOR</td></tr></table>")
                    } else {
                        text.to_owned()
                    };
                    format!("{bom}PREFIX中文🙂\n\n{text}\n\nTAIL").replace('\n', ending)
                };
                let (source, old_points) = marked(&wrap(before));
                let (expected, new_points) = marked(&wrap(after));
                assert_eq!(old_points.len(), new_points.len());
                for reverse in [false, true] {
                    for primary in [0, old_points.len() - 1] {
                        eprintln!(
                            "cell={cell}, ending={ending:?}, bom={}, reverse={reverse}, primary={primary}",
                            !bom.is_empty()
                        );
                        let mut doc = seed(&source);
                        let old = selections(&doc, &old_points, reverse, primary);
                        doc.set_selections(old.as_slice().iter().copied(), old.primary_index())
                            .expect("select");
                        let revision = doc.revision();
                        let history = doc.history_stats();
                        let command = if indent {
                            EditorCommand::IndentList
                        } else {
                            EditorCommand::OutdentList
                        };
                        if !supported {
                            for _ in 0..3 {
                                assert!(!doc.command_available(&command));
                                assert!(
                                    !doc.execute(command.clone()).expect("safe no-op").changed()
                                );
                                assert_eq!(doc.snapshot().as_str(), format!("{source} HISTORY-A"));
                                assert_eq!(doc.revision(), revision);
                                assert_eq!(doc.selections(), &old);
                                assert_eq!(doc.history_stats(), history);
                            }
                            for (command, tail) in [
                                (EditorCommand::Redo, " HISTORY-A HISTORY-B"),
                                (EditorCommand::Undo, " HISTORY-A"),
                                (EditorCommand::Undo, ""),
                                (EditorCommand::Redo, " HISTORY-A"),
                                (EditorCommand::Redo, " HISTORY-A HISTORY-B"),
                            ] {
                                assert!(
                                    doc.execute(command)
                                        .expect("original history replay")
                                        .changed()
                                );
                                assert_eq!(doc.snapshot().as_str(), format!("{source}{tail}"));
                            }
                            continue;
                        }
                        assert!(doc.command_available(&command));
                        assert_eq!(doc.revision(), revision, "availability must be read-only");
                        assert!(
                            doc.execute(command)
                                .expect("cross-level list command")
                                .changed()
                        );
                        assert_eq!(doc.snapshot().as_str(), format!("{expected} HISTORY-A"));
                        assert_eq!(
                            doc.selections(),
                            &selections(&doc, &new_points, reverse, primary)
                        );
                        assert_eq!(doc.history_stats().undo_entries(), 2);
                        assert_eq!(doc.history_stats().redo_entries(), 0);
                        for region in &doc.markdown().html_regions().regions {
                            let model = region.model.as_ref().expect("balanced resulting HTML");
                            assert!(
                                model.resolution.diagnostics.is_empty(),
                                "result must remain finite native HTML"
                            );
                        }
                        assert!(doc.undo().expect("one undo").changed());
                        assert_eq!(doc.snapshot().as_str(), format!("{source} HISTORY-A"));
                        assert_eq!(
                            doc.selections(),
                            &selections(&doc, &old_points, reverse, primary)
                        );
                        assert_eq!(doc.history_stats().undo_entries(), 1);
                        assert!(doc.redo().expect("one redo").changed());
                        assert_eq!(doc.snapshot().as_str(), format!("{expected} HISTORY-A"));
                        assert_eq!(
                            doc.selections(),
                            &selections(&doc, &new_points, reverse, primary)
                        );
                    }
                }
            }
        }
    }
}

macro_rules! case {
    ($name:ident, $file:literal, $indent:expr, $supported:expr, $complete:expr) => {
        #[test]
        fn $name() {
            matrix(
                include_str!(concat!("fixtures/group4-list/", $file)),
                $indent,
                $supported,
                $complete,
            );
        }
    };
}
case!(
    parent_to_descendant_indent_moves_one_subtree,
    "parent-child.indent.case",
    true,
    true,
    false
);
case!(
    descendant_to_neighbor_promotes_common_list_items,
    "child-next.outdent.case",
    false,
    true,
    false
);
case!(
    root_cross_level_outdent_maps_both_endpoints,
    "root-cross.outdent.case",
    false,
    true,
    false
);
case!(
    multiple_parent_child_indent_deduplicates_and_maps_passive_text,
    "multi-parent.indent.case",
    true,
    true,
    false
);
case!(
    multiple_parent_child_outdent_deduplicates,
    "multi-parent.outdent.case",
    false,
    true,
    false
);
case!(
    ancestor_selected_after_two_children_still_dominates,
    "late-ancestor.outdent.case",
    false,
    true,
    false
);
case!(
    repeated_text_maps_by_source_ownership_not_label,
    "repeated-root.outdent.case",
    false,
    true,
    false
);
case!(
    cross_level_promotion_preserves_divs_ids_and_numbering,
    "div-cross.outdent.case",
    false,
    true,
    false
);
case!(
    cross_cell_selection_rejects_all_members_atomically,
    "cross-cells.reject-outdent.case",
    false,
    false,
    true
);
case!(
    disclosure_boundary_is_not_widened_away,
    "cross-disclosure.reject-outdent.case",
    false,
    false,
    false
);
case!(
    hidden_tag_endpoint_rejects_all_members,
    "raw-endpoint.reject-indent.case",
    true,
    false,
    false
);
case!(
    markdown_primary_does_not_bypass_html_rejection,
    "mixed-markdown.reject-outdent.case",
    false,
    false,
    true
);
case!(
    first_ancestor_noop_does_not_indent_its_selected_child,
    "first-ancestor.reject-indent.case",
    true,
    false,
    false
);

#[test]
fn source_mode_never_enables_cross_level_html_editing() {
    let (source, points) = marked("<ul><li>前</li><li>«A0»父<ul><li>子«F0»</li></ul></li></ul>");
    let mut doc = seed(&source);
    doc.set_source_mode(true).expect("literal mode");
    let selected = selections(&doc, &points, false, 0);
    doc.set_selections(selected.as_slice().iter().copied(), 0)
        .expect("select");
    let revision = doc.revision();
    let history = doc.history_stats();
    for command in [EditorCommand::IndentList, EditorCommand::OutdentList] {
        assert!(!doc.command_available(&command));
        assert!(!doc.execute(command).expect("literal no-op").changed());
        assert_eq!(doc.revision(), revision);
        assert_eq!(doc.selections(), &selected);
        assert_eq!(doc.history_stats(), history);
    }
}
