//! Resolve item ownership before combining edits; source bytes, not labels, map carets.
use super::*;
use yu_markdown::html::HtmlBlockModel;

pub(super) struct ListSpan<'a> {
    pub model: &'a HtmlBlockModel,
    pub list: usize,
    pub first: usize,
    pub last: usize,
}

impl ListSpan<'_> {
    fn coverage(&self) -> TextRange {
        TextRange::new(
            self.model.fragment.nodes[self.first].source.start(),
            self.model.fragment.nodes[self.last].source.end(),
        )
        .expect("ordered list items")
    }
}

struct SelectionGroup {
    origin: EditorSelection,
    members: Vec<(usize, EditorSelection)>,
}

pub(super) struct PreparedIndent {
    transaction: Transaction,
    endpoints: Vec<(u64, u64, crate::CaretAffinity)>,
}

impl EditorDocument {
    pub(super) fn native_list_model(&self, at: ByteOffset) -> Option<&HtmlBlockModel> {
        if self.source_mode() {
            return None;
        }
        let block = self
            .block_index_for_offset(at)
            .and_then(|i| self.markdown.blocks().get(i))?;
        let model = self
            .markdown
            .html_regions()
            .region_for(block.range())?
            .model
            .as_ref()
            .ok()?;
        // A source-fallback table cannot acquire structural list editing. Outside
        // a table, the same resolved finite HTML tree is the capability boundary.
        if model.fragment.nodes.iter().any(|node| {
            element_tags(node).is_some_and(|(open, _)| open.name == "table")
                && node.source.start() <= at
                && at <= node.source.end()
        }) {
            self.html_table_grid(block)?;
        } else if !model.resolution.diagnostics.is_empty() {
            return None;
        }
        Some(model)
    }

    pub(super) fn list_selection_span(&self, selection: EditorSelection) -> Option<ListSpan<'_>> {
        let selected = selection.ordered_range();
        let model = self.native_list_model(selected.start())?;
        let nodes = &model.fragment.nodes;
        let item_at = |at| {
            // Never interpret an endpoint inside hidden markup as an item body.
            if nodes.iter().any(|node| match &node.kind {
                HtmlNodeKind::Comment => node.source.start() < at && at < node.source.end(),
                HtmlNodeKind::Element { opening, closing } => {
                    (opening.source.start() < at && at < opening.source.end())
                        || closing
                            .as_ref()
                            .is_some_and(|c| c.source.start() < at && at < c.source.end())
                }
                HtmlNodeKind::Text => false,
            }) {
                return None;
            }
            nodes
                .iter()
                .enumerate()
                .filter(|(_, node)| {
                    element_tags(node).is_some_and(|(o, c)| {
                        o.name == "li" && o.source.end() <= at && at <= c.source.start()
                    })
                })
                .min_by_key(|(_, node)| node.source.len())
                .map(|(id, _)| id)
        };
        let chain = |mut item: usize| {
            let mut result = Vec::new();
            while let Some(list) = nodes[item].parent {
                if !element_tags(&nodes[list])
                    .is_some_and(|(o, _)| matches!(o.name.as_str(), "ul" | "ol"))
                {
                    break;
                }
                if model.resolution.elements[item].is_none()
                    || model.resolution.elements[list].is_none()
                {
                    break;
                }
                result.push((list, item));
                // Only transparent div wrappers may be crossed. Cells, details,
                // summary and unrelated semantic containers are not widened away.
                let Some((parent, _)) = list_parent_through_divs(nodes, list) else {
                    break;
                };
                item = parent;
            }
            result
        };
        let left = chain(item_at(selected.start())?);
        let right = chain(item_at(selected.end())?);
        let (list, first, last) = left.into_iter().find_map(|(list, first)| {
            right
                .iter()
                .find(|(other, _)| *other == list)
                .map(|(_, last)| (list, first, *last))
        })?;
        if nodes[first].source.start() > nodes[last].source.start() {
            return None;
        }
        Some(ListSpan {
            model,
            list,
            first,
            last,
        })
    }

    /// Includes unsupported/cross-container selections so a failed HTML plan
    /// cannot fall through to editing just the primary Markdown selection.
    pub(in crate::document) fn has_html_list_selection(&self) -> bool {
        if self.source_mode() {
            return false;
        }
        self.selections()
            .as_slice()
            .iter()
            .any(|selection| self.touches_html_list(*selection))
    }

    fn touches_html_list(&self, selection: EditorSelection) -> bool {
        let range = selection.ordered_range();
        self.markdown.html_regions().regions.iter().any(|region| {
            region.model.as_ref().is_ok_and(|model| {
                model.fragment.nodes.iter().any(|node| {
                    element_tags(node).is_some_and(|(o, _)| o.name == "li")
                        && if range.is_empty() {
                            node.source.start() <= range.start()
                                && range.start() <= node.source.end()
                        } else {
                            node.source.start() < range.end() && range.start() < node.source.end()
                        }
                })
            })
        })
    }

    fn merge_list_origins(
        &self,
        left: EditorSelection,
        right: EditorSelection,
    ) -> Option<EditorSelection> {
        let a = self.list_selection_span(left)?;
        let b = self.list_selection_span(right)?;
        if !std::ptr::eq(a.model, b.model) {
            return None;
        }
        let contains = |a: TextRange, b: TextRange| a.start() <= b.start() && b.end() <= a.end();
        let related = contains(a.coverage(), b.coverage()) || contains(b.coverage(), a.coverage());
        if !related {
            if a.list != b.list {
                return None;
            }
            let nodes = &a.model.fragment.nodes;
            let siblings: Vec<_> = nodes[a.list]
                .children
                .iter()
                .copied()
                .filter(|id| element_tags(&nodes[*id]).is_some_and(|(o, _)| o.name == "li"))
                .collect();
            let ae = siblings.iter().position(|id| *id == a.last)?;
            let bs = siblings.iter().position(|id| *id == b.first)?;
            let be = siblings.iter().position(|id| *id == b.last)?;
            let as_ = siblings.iter().position(|id| *id == a.first)?;
            if bs > ae + 1 || as_ > be + 1 {
                return None;
            }
        }
        EditorSelection::range(
            &self.snapshot(),
            left.ordered_range()
                .start()
                .min(right.ordered_range().start()),
            left.ordered_range().end().max(right.ordered_range().end()),
            crate::CaretAffinity::Downstream,
        )
        .ok()
    }

    fn list_group_plan(&self, group: SelectionGroup, indent: bool) -> Option<HtmlIndentGroup> {
        let span = self.list_selection_span(group.origin)?;
        let nodes = &span.model.fragment.nodes;
        if !indent {
            // A list inside a disclosure nested under an item is not a root list.
            // Do not take the root-exit fallback across that semantic boundary.
            let mut parent = nodes[span.list].parent;
            let mut has_item_parent = false;
            while let Some(id) = parent {
                has_item_parent |= element_tags(&nodes[id]).is_some_and(|(o, _)| o.name == "li");
                parent = nodes[id].parent;
            }
            if has_item_parent && list_parent_through_divs(nodes, span.list).is_none() {
                return None;
            }
        }
        let points: Vec<_> = group
            .members
            .iter()
            .flat_map(|(_, s)| [s.anchor(), s.focus()])
            .collect();
        let (range, text, offsets) = if !indent
            && span.first != span.last
            && let Some(plan) = self.root_html_list_outdent_with_points(group.origin, &points)
        {
            plan
        } else {
            let (plan, reference) = if indent {
                (
                    self.html_list_indent_for(group.origin)?,
                    group.origin.focus(),
                )
            } else if span.first == span.last {
                // An explicitly selected ancestor owns its subtree once, even
                // when the last member's caret is in a deeper descendant.
                let reference = element_tags(&nodes[span.first])?.0.source.end();
                (self.html_list_outdent_at(reference, false)?, reference)
            } else {
                (
                    self.selected_html_list_outdent_for(group.origin)?,
                    group.origin.focus(),
                )
            };
            let (range, text, target) = plan;
            let offsets = points
                .iter()
                .map(|at| {
                    usize::try_from(
                        target as i128 + i128::from(at.get()) - i128::from(reference.get()),
                    )
                    .ok()
                })
                .collect::<Option<Vec<_>>>()?;
            (range, text, offsets)
        };
        let (point_offsets, remainder) = offsets.as_chunks::<2>();
        if !remainder.is_empty() {
            return None;
        }
        trim_group(
            self.markdown.source().as_str(),
            HtmlIndentGroup {
                range,
                merge_range: range,
                text,
                members: group.members,
                offsets: point_offsets.iter().map(|p| (p[0], p[1])).collect(),
            },
        )
    }

    pub(super) fn combined_html_list_groups(&self, indent: bool) -> Option<Vec<HtmlIndentGroup>> {
        if self.source_mode() || self.selections().table_columns().is_some() {
            return None;
        }
        let mut input = Vec::new();
        for (index, selection) in self.selections().as_slice().iter().copied().enumerate() {
            if let Some(span) = self.list_selection_span(selection) {
                input.push((
                    span.coverage(),
                    SelectionGroup {
                        origin: selection,
                        members: vec![(index, selection)],
                    },
                ));
            } else if self.touches_html_list(selection) {
                return None;
            }
        }
        input.sort_by_key(|(range, _)| (range.start(), std::cmp::Reverse(range.end())));
        let mut selected: Vec<(TextRange, SelectionGroup)> = Vec::new();
        for (coverage, group) in input {
            if let Some((previous_range, previous)) = selected.last_mut()
                && previous_range.start() <= coverage.start()
                && coverage.end() <= previous_range.end()
            {
                previous.origin = self.merge_list_origins(previous.origin, group.origin)?;
                previous.members.extend(group.members);
            } else {
                selected.push((coverage, group));
            }
        }
        let mut groups: Vec<HtmlIndentGroup> = Vec::new();
        let mut origins: Vec<EditorSelection> = Vec::new();
        for (_, mut group) in selected {
            let span = self.list_selection_span(group.origin)?;
            if indent {
                let nodes = &span.model.fragment.nodes;
                let first =
                    nodes[span.list].children.iter().copied().find(|id| {
                        element_tags(&nodes[*id]).is_some_and(|(o, _)| o.name == "li")
                    })?;
                // Retain the established no-op for a first item, without letting
                // a selected descendant move a second time on its behalf.
                if first == span.first {
                    continue;
                }
            }
            let mut planned = self.list_group_plan(
                SelectionGroup {
                    origin: group.origin,
                    members: group.members.clone(),
                },
                indent,
            )?;
            while let Some(previous) = groups.last() {
                let previous_origin = *origins.last()?;
                let actual_overlap = planned.range.start() < previous.range.end();
                let raw_overlap = planned.merge_range.start() < previous.merge_range.end();
                let nested_outdent_overlap = !indent
                    && raw_overlap
                    && [previous_origin, group.origin].into_iter().all(|origin| {
                        self.list_selection_span(origin).is_some_and(|span| {
                            list_parent_through_divs(&span.model.fragment.nodes, span.list)
                                .is_some()
                        })
                    });
                if !actual_overlap && !(indent && raw_overlap) && !nested_outdent_overlap {
                    break;
                }
                let Some(origin) = self.merge_list_origins(previous_origin, group.origin) else {
                    if actual_overlap {
                        return None;
                    }
                    break;
                };
                let previous = groups.pop()?;
                origins.pop();
                group.origin = origin;
                group.members.extend(previous.members);
                group.members.sort_by_key(|(index, _)| *index);
                planned = self.list_group_plan(
                    SelectionGroup {
                        origin: group.origin,
                        members: group.members.clone(),
                    },
                    indent,
                )?;
            }
            origins.push(group.origin);
            groups.push(planned);
        }
        if groups.is_empty() {
            return None;
        }
        if !indent {
            let targets: Vec<_> = groups
                .iter()
                .map(|g| {
                    g.offsets
                        .iter()
                        .flat_map(|(a, f)| [*a, *f])
                        .min()
                        .unwrap_or(0)
                })
                .collect();
            let mut plans: Vec<_> = groups
                .iter_mut()
                .zip(&targets)
                .map(|(g, target)| (g.range, std::mem::take(&mut g.text), *target))
                .collect();
            let exits = vec![true; plans.len()];
            compact_list_exits(self.markdown.source().as_str(), &mut plans, &exits);
            for ((group, (range, text, target)), old) in groups.iter_mut().zip(plans).zip(targets) {
                let removed = old.checked_sub(target)?;
                for (a, f) in &mut group.offsets {
                    *a = a.checked_sub(removed)?;
                    *f = f.checked_sub(removed)?;
                }
                group.range = range;
                group.text = text;
            }
        }
        Some(groups)
    }

    pub(super) fn single_html_list_group(&self, indent: bool) -> Option<HtmlIndentGroup> {
        let selection = self.selection();
        // Validate the same semantic boundary used for multi-selection planning.
        let probe = SelectionGroup {
            origin: selection,
            members: vec![(0, selection)],
        };
        let planned = self.list_group_plan(probe, indent)?;
        if !indent
            && !selection.is_empty()
            && let Some((range, text, anchor, focus)) = self.root_html_list_outdent_for(selection)
        {
            return trim_group(
                self.markdown.source().as_str(),
                HtmlIndentGroup {
                    range,
                    merge_range: range,
                    text,
                    members: vec![(0, selection)],
                    offsets: vec![(anchor, focus)],
                },
            );
        }
        Some(planned)
    }

    pub(in crate::document) fn html_list_command_available(&self, indent: bool) -> bool {
        let groups = if self.selections().is_multiple() {
            self.combined_html_list_groups(indent)
        } else {
            self.single_html_list_group(indent).map(|group| vec![group])
        };
        groups.is_some_and(|groups| self.prepare_html_list_edit(&groups).is_some())
    }

    pub(super) fn prepare_html_list_edit(
        &self,
        groups: &[HtmlIndentGroup],
    ) -> Option<PreparedIndent> {
        if groups.is_empty() || self.selections().table_columns().is_some() {
            return None;
        }
        let snapshot = self.snapshot();
        let mut endpoints = vec![None; self.selections().len()];
        let mut delta = 0i128;
        let mut previous_end = None;
        let mut changes_source = false;
        for group in groups {
            snapshot.utf16_offset(group.range.start()).ok()?;
            snapshot.utf16_offset(group.range.end()).ok()?;
            if previous_end.is_some_and(|end| end > group.range.start()) || group.range.is_empty() {
                return None;
            }
            previous_end = Some(group.range.end());
            if group.members.len() != group.offsets.len() {
                return None;
            }
            let original = &snapshot.as_str()
                [group.range.start().get() as usize..group.range.end().get() as usize];
            changes_source |= original != group.text.as_str();
            let base = i128::from(group.range.start().get()) + delta;
            for ((index, original), (anchor, focus)) in group.members.iter().zip(&group.offsets) {
                // Prove UTF-8 boundaries in the actual replacement rather than
                // rebuilding a second full document for every menu query.
                if !group.text.is_char_boundary(*anchor) || !group.text.is_char_boundary(*focus) {
                    return None;
                }
                let slot = endpoints.get_mut(*index)?;
                if slot.is_some() {
                    return None;
                }
                *slot = Some((
                    u64::try_from(base + *anchor as i128).ok()?,
                    u64::try_from(base + *focus as i128).ok()?,
                    original.affinity(),
                ));
            }
            delta += group.text.len() as i128 - i128::from(group.range.len());
        }
        if !changes_source {
            return None;
        }
        let final_len = u64::try_from(i128::from(snapshot.len_bytes().get()) + delta).ok()?;
        let map_passive = |at: ByteOffset| {
            snapshot.utf16_offset(at).ok()?;
            let mut delta = 0i128;
            for group in groups {
                if at <= group.range.start() {
                    break;
                }
                // Passive endpoints in rewritten context cannot be guessed.
                if at < group.range.end() {
                    return None;
                }
                delta += group.text.len() as i128 - i128::from(group.range.len());
            }
            u64::try_from(i128::from(at.get()) + delta).ok()
        };
        for (slot, original) in endpoints.iter_mut().zip(self.selections().as_slice()) {
            if slot.is_none() {
                *slot = Some((
                    map_passive(original.anchor())?,
                    map_passive(original.focus())?,
                    original.affinity(),
                ));
            }
        }
        let endpoints = endpoints.into_iter().collect::<Option<Vec<_>>>()?;
        let mut ordered: Vec<_> = endpoints
            .iter()
            .map(|(a, f, _)| ((*a).min(*f), (*a).max(*f)))
            .collect();
        ordered.sort_unstable();
        if ordered.iter().any(|(_, end)| *end > final_len)
            || ordered.windows(2).any(|p| {
                p[1].0 < p[0].1 || (p[1].0 == p[0].1 && (p[0].0 == p[0].1 || p[1].0 == p[1].1))
            })
        {
            return None;
        }
        Some(PreparedIndent {
            transaction: Transaction::new(
                self.revision(),
                groups
                    .iter()
                    .map(|g| yu_text::Edit::new(g.range, g.text.clone())),
            ),
            endpoints,
        })
    }

    pub(super) fn apply_html_indent_groups(
        &mut self,
        groups: Vec<HtmlIndentGroup>,
    ) -> Result<CommandResult, EditorDocumentError> {
        let Some(PreparedIndent {
            transaction,
            endpoints,
        }) = self.prepare_html_list_edit(&groups)
        else {
            return Ok(self.command_result(false));
        };
        let primary = self.selections().primary_index();
        self.state.history.break_group();
        self.apply_transaction_with_group(&transaction, HistoryGroup::ListEditing)?;
        let snapshot = self.snapshot();
        let ranges = endpoints
            .into_iter()
            .map(|(a, f, affinity)| {
                EditorSelection::range(&snapshot, ByteOffset::new(a), ByteOffset::new(f), affinity)
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.presentation.selections = Selections::new(&snapshot, ranges, primary)?;
        self.state
            .history
            .finish_selection(self.presentation.selections.clone());
        self.state.history.break_group();
        Ok(self.command_result(true))
    }
}

fn trim_group(source: &str, mut group: HtmlIndentGroup) -> Option<HtmlIndentGroup> {
    let original =
        source.get(group.range.start().get() as usize..group.range.end().get() as usize)?;
    if group
        .offsets
        .iter()
        .any(|(a, f)| !group.text.is_char_boundary(*a) || !group.text.is_char_boundary(*f))
    {
        return None;
    }
    let min = group
        .offsets
        .iter()
        .flat_map(|(a, f)| [*a, *f])
        .min()
        .unwrap_or(0);
    let max = group
        .offsets
        .iter()
        .flat_map(|(a, f)| [*a, *f])
        .max()
        .unwrap_or(group.text.len());
    let prefix = original
        .chars()
        .zip(group.text.chars())
        .take_while(|(a, b)| a == b)
        .map(|(ch, _)| ch.len_utf8())
        .sum::<usize>()
        .min(min);
    let suffix = original[prefix..]
        .chars()
        .rev()
        .zip(group.text[prefix..].chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(ch, _)| ch.len_utf8())
        .sum::<usize>()
        .min(group.text.len().saturating_sub(max));
    group.range = TextRange::new(
        ByteOffset::new(group.range.start().get() + prefix as u64),
        ByteOffset::new(group.range.end().get() - suffix as u64),
    )
    .expect("trimmed group");
    group.text = group.text[prefix..group.text.len() - suffix].to_owned();
    for (a, f) in &mut group.offsets {
        *a -= prefix;
        *f -= prefix;
    }
    Some(group)
}
