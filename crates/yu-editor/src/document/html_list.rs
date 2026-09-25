//! Source-preserving list-item splits inside native HTML cells.
use super::*;
use yu_markdown::html::{HtmlNodeKind, HtmlTag};

mod selection;

fn opening_without_id(source: &str, tag: &HtmlTag) -> String {
    let mut result = String::new();
    let mut cursor = tag.source.start().get() as usize;
    for attribute in &tag.attributes {
        if attribute.name == "id" {
            result.push_str(&source[cursor..attribute.source.start().get() as usize]);
            cursor = attribute.source.end().get() as usize;
        }
    }
    result.push_str(&source[cursor..tag.source.end().get() as usize]);
    result
}

/// Remove byte-identical context from a structural rewrite. This retains
/// real source coordinates for other carets in unchanged sibling items.
fn trim_list_plan(source: &str, plan: (TextRange, String, usize)) -> (TextRange, String, usize) {
    let (range, replacement, caret) = plan;
    let original = &source[range.start().get() as usize..range.end().get() as usize];
    let prefix = original
        .chars()
        .zip(replacement.chars())
        .take_while(|(a, b)| a == b)
        .map(|(ch, _)| ch.len_utf8())
        .sum::<usize>()
        .min(caret);
    let suffix = original[prefix..]
        .chars()
        .rev()
        .zip(replacement[prefix..].chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(ch, _)| ch.len_utf8())
        .sum::<usize>()
        .min(replacement.len() - caret);
    let reduced = TextRange::new(
        ByteOffset::new(range.start().get() + prefix as u64),
        ByteOffset::new(range.end().get() - suffix as u64),
    )
    .expect("trimmed source range");
    (
        reduced,
        replacement[prefix..replacement.len() - suffix].to_owned(),
        caret - prefix,
    )
}

fn compact_list_exits(source: &str, plans: &mut [(TextRange, String, usize)], exits: &[bool]) {
    // Adjacent exits each reopen/close the same list boundary. Remove
    // that newly generated empty segment, without touching user content.
    for i in 1..plans.len() {
        if !exits[i - 1] || !exits[i] {
            continue;
        }
        let (left, right) = plans.split_at_mut(i);
        let previous = &mut left[i - 1];
        let next = &mut right[0];
        if previous.0.end() > next.0.start() {
            continue;
        }
        let gap = &source[previous.0.end().get() as usize..next.0.start().get() as usize];
        let Some(inner) = gap.strip_prefix('>').and_then(|s| s.strip_suffix('<')) else {
            continue;
        };
        let trivia_only = yu_markdown::html::HtmlFragment::parse(inner, ByteOffset::new(0))
            .is_ok_and(|fragment| {
                fragment.nodes.iter().all(|node| match &node.kind {
                    HtmlNodeKind::Comment => true,
                    HtmlNodeKind::Text => inner
                        [node.source.start().get() as usize..node.source.end().get() as usize]
                        .trim()
                        .is_empty(),
                    HtmlNodeKind::Element { .. } => false,
                })
            });
        if !trivia_only {
            continue;
        }
        let Some(open_at) = previous.1.rfind('<') else {
            continue;
        };
        let opening = &previous.1[open_at..];
        let tag = if opening == "<ul" || opening.starts_with("<ul ") {
            "ul"
        } else if opening == "<ol" || opening.starts_with("<ol ") {
            "ol"
        } else {
            continue;
        };
        let closing = format!("/{tag}>");
        if opening.contains('>')
            || !next.1.starts_with(&closing)
            || previous.2 > open_at
            || next.2 < closing.len()
        {
            continue;
        }
        previous.1.truncate(open_at);
        previous.1.push_str(inner);
        previous.0 = TextRange::new(previous.0.start(), next.0.start()).expect("adjacent exits");
        next.1.drain(..closing.len());
        next.2 -= closing.len();
    }
}

pub(super) struct HtmlIndentGroup {
    range: TextRange,
    merge_range: TextRange,
    text: String,
    members: Vec<(usize, EditorSelection)>,
    // Each endpoint is relative to this replacement, not to the primary caret.
    offsets: Vec<(usize, usize)>,
}

impl EditorDocument {
    pub(super) fn split_html_list_item(
        &mut self,
    ) -> Result<Option<CommandResult>, EditorDocumentError> {
        if let Some((range, replacement, caret)) = self.html_list_outdent_plan(true) {
            return self
                .apply_html_list_plan(range, replacement, caret)
                .map(Some);
        }
        self.split_html_list_items(true)
    }

    fn split_html_list_items(
        &mut self,
        include_empty: bool,
    ) -> Result<Option<CommandResult>, EditorDocumentError> {
        let snapshot = self.snapshot();
        let mut plans = Vec::with_capacity(self.presentation.selections.len());
        let mut structural = false;
        let mut exits = Vec::new();
        for selection in self.presentation.selections.as_slice() {
            let range = selection.ordered_range();
            if selection.is_empty() {
                if include_empty
                    && let Some(plan) = self.html_list_outdent_at(selection.focus(), true)
                {
                    structural = true;
                    plans.push(trim_list_plan(snapshot.as_str(), plan));
                    exits.push(true);
                    continue;
                }
                if let Some(markup) = self.html_list_split_markup(selection.focus()) {
                    structural = true;
                    let target = markup.len();
                    plans.push((range, markup, target));
                    exits.push(false);
                    continue;
                }
            }
            let line = source_line(&snapshot, range.start())?;
            let replacement = self
                .html_table_cell_input(range, line.insertion_terminator())
                .unwrap_or_else(|| line.insertion_terminator().to_owned());
            let target = self
                .html_table_input_cursor(range, replacement.len())
                .unwrap_or(replacement.len());
            plans.push((range, replacement, target));
            exits.push(false);
        }
        compact_list_exits(snapshot.as_str(), &mut plans, &exits);
        if !structural {
            return Ok(None);
        }
        if plans
            .windows(2)
            .any(|pair| pair[0].0.end() > pair[1].0.start())
        {
            // Overlapping structural exits need a combined container plan.
            // Preserve the established per-caret split/newline behavior meanwhile.
            return if include_empty {
                self.split_html_list_items(false)
            } else {
                Ok(None)
            };
        }
        let primary = self.presentation.selections.primary_index();
        let mut delta = 0_i128;
        let mut targets = Vec::with_capacity(plans.len());
        for (range, replacement, target) in &plans {
            targets.push((i128::from(range.start().get()) + delta + *target as i128) as u64);
            delta += replacement.len() as i128 - i128::from(range.len());
        }
        self.state.history.break_group();
        self.apply_selection_edits(
            plans
                .into_iter()
                .map(|(range, text, _)| (range, text.into()))
                .collect(),
            HistoryGroup::ListEditing,
            CollapseTo::End,
        )?;
        let snapshot = self.snapshot();
        let cursors = targets
            .into_iter()
            .map(|at| {
                EditorSelection::cursor(
                    &snapshot,
                    ByteOffset::new(at),
                    crate::CaretAffinity::Downstream,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.presentation.selections = Selections::new(&snapshot, cursors, primary)?;
        self.state
            .history
            .finish_selection(self.presentation.selections.clone());
        self.state.history.break_group();
        Ok(Some(self.command_result(true)))
    }

    fn apply_html_list_plan(
        &mut self,
        range: TextRange,
        replacement: String,
        caret: usize,
    ) -> Result<CommandResult, EditorDocumentError> {
        self.state.history.break_group();
        self.apply_transaction_with_group(
            &Transaction::new(self.revision(), [yu_text::Edit::new(range, replacement)]),
            HistoryGroup::ListEditing,
        )?;
        let selection = EditorSelection::cursor(
            &self.snapshot(),
            ByteOffset::new(range.start().get() + caret as u64),
            crate::CaretAffinity::Downstream,
        )?;
        self.set_edit_selection(selection);
        self.state.history.break_group();
        Ok(self.command_result(true))
    }

    pub(super) fn change_html_list_indent(
        &mut self,
        indent: bool,
    ) -> Result<Option<CommandResult>, EditorDocumentError> {
        let groups = if self.presentation.selections.is_multiple() {
            self.combined_html_list_groups(indent)
        } else {
            self.single_html_list_group(indent).map(|group| vec![group])
        };
        match groups {
            Some(groups) => self.apply_html_indent_groups(groups).map(Some),
            // A recognized but unsupported HTML selection must not fall through
            // to a partial primary-only Markdown edit.
            None if self.has_html_list_selection() => Ok(Some(self.command_result(false))),
            None => Ok(None),
        }
    }

    pub(super) fn multiple_html_indent_plan(&self) -> Option<Vec<HtmlIndentGroup>> {
        self.presentation
            .selections
            .is_multiple()
            .then(|| self.combined_html_list_groups(true))
            .flatten()
    }

    pub(super) fn multiple_html_outdent_plan(&self) -> Option<Vec<HtmlIndentGroup>> {
        self.presentation
            .selections
            .is_multiple()
            .then(|| self.combined_html_list_groups(false))
            .flatten()
    }

    pub(super) fn html_list_indent_plan(&self) -> Option<(TextRange, String, usize)> {
        if self.presentation.selections.is_multiple() {
            return None;
        }
        self.html_list_indent_for(self.selection())
    }

    fn html_list_indent_for(
        &self,
        selection: EditorSelection,
    ) -> Option<(TextRange, String, usize)> {
        let caret = selection.focus();
        let span = self.list_selection_span(selection)?;
        let nodes = &span.model.fragment.nodes;
        let id = span.first;
        let item = &nodes[id];
        let last = &nodes[span.last];
        let list = &nodes[span.list];
        let (list_open, _) = element_tags(list)?;
        let previous_id = list
            .children
            .iter()
            .copied()
            .take_while(|child| *child != id)
            .filter(|child| element_tags(&nodes[*child]).is_some_and(|(open, _)| open.name == "li"))
            .last()?;
        let previous = &nodes[previous_id];
        let (_, previous_close) = element_tags(previous)?;
        let source = self.markdown.source().as_str();
        let slice = |a: ByteOffset, b: ByteOffset| &source[a.get() as usize..b.get() as usize];
        let existing = previous.children.iter().rev().find_map(|child| {
            let node = &nodes[*child];
            let (open, close) = element_tags(node)?;
            (open.name == list_open.name
                && slice(node.source.end(), previous_close.source.start())
                    .trim()
                    .is_empty())
            .then_some(close.source.start())
        });
        let insertion = existing.unwrap_or(previous_close.source.start());
        let mut replacement = String::new();
        if existing.is_none() {
            replacement.push_str(&format!("<{}>", list_open.name));
        }
        replacement.push_str(slice(previous_close.source.end(), item.source.start()));
        let target = replacement.len() + (caret.get() - item.source.start().get()) as usize;
        replacement.push_str(slice(item.source.start(), last.source.end()));
        if existing.is_none() {
            replacement.push_str(&format!("</{}>", list_open.name));
        }
        replacement.push_str(slice(insertion, previous_close.source.end()));
        Some((
            TextRange::new(insertion, last.source.end())?,
            replacement,
            target,
        ))
    }

    fn html_list_split_markup(&self, caret: ByteOffset) -> Option<String> {
        let block = self
            .block_index_for_offset(caret)
            .and_then(|i| self.markdown.blocks().get(i))?;
        self.html_table_grid(block)?;
        let index = self.markdown.html_regions();
        let model = index.region_for(block.range())?.model.as_ref().ok()?;
        let nodes = &model.fragment.nodes;
        let (item_id, item_open, item_close) = nodes
            .iter()
            .enumerate()
            .filter_map(|(id, node)| {
                let HtmlNodeKind::Element {
                    opening,
                    closing: Some(closing),
                } = &node.kind
                else {
                    return None;
                };
                (opening.name == "li"
                    && opening.source.end() <= caret
                    && caret <= closing.source.start())
                .then_some((id, opening, closing))
            })
            .min_by_key(|(id, _, _)| nodes[*id].source.len())?;
        let source = self.markdown.source().as_str();
        // Empty items need a separate exit/outdent transaction, not another item.
        let meaningful = nodes.iter().any(|node| {
            node.source.start() >= item_open.source.end()
                && node.source.end() <= item_close.source.start()
                && match &node.kind {
                    HtmlNodeKind::Text => !source
                        [node.source.start().get() as usize..node.source.end().get() as usize]
                        .trim()
                        .is_empty(),
                    HtmlNodeKind::Element { opening, .. } => {
                        matches!(opening.name.as_str(), "img" | "details" | "ul" | "ol")
                    }
                    HtmlNodeKind::Comment => false,
                }
        });
        if !meaningful {
            return None;
        }
        let mut wrappers = Vec::new();
        for (id, node) in nodes.iter().enumerate() {
            if id == item_id
                || node.source.start() < item_open.source.end()
                || node.source.end() > item_close.source.start()
            {
                continue;
            }
            let HtmlNodeKind::Element { opening, closing } = &node.kind else {
                continue;
            };
            // Never inject structural tags into a hidden tag token.
            if opening.source.start() < caret && caret < opening.source.end()
                || closing
                    .as_ref()
                    .is_some_and(|tag| tag.source.start() < caret && caret < tag.source.end())
            {
                return None;
            }
            if let Some(closing) = closing
                && opening.source.end() <= caret
                && caret <= closing.source.start()
            {
                if matches!(opening.name.as_str(), "details" | "summary" | "ul" | "ol") {
                    return None;
                }
                wrappers.push((opening, closing));
            }
        }
        wrappers.sort_by_key(|(opening, _)| opening.source.start());
        let slice =
            |range: TextRange| &source[range.start().get() as usize..range.end().get() as usize];
        let mut markup = String::new();
        for (_, closing) in wrappers.iter().rev() {
            markup.push_str(slice(closing.source));
        }
        markup.push_str(slice(item_close.source));
        markup.push_str(&opening_without_id(source, item_open));
        for (opening, _) in wrappers {
            markup.push_str(&opening_without_id(source, opening));
        }
        Some(markup)
    }
}

// Only div wrappers can be split while promoting an item. Disclosure and
// other semantic containers need their own editing policy.
fn list_parent_through_divs(
    nodes: &[yu_markdown::html::HtmlNode],
    list: usize,
) -> Option<(usize, Vec<usize>)> {
    let mut id = nodes[list].parent?;
    let mut wrappers = Vec::new();
    loop {
        let (open, _) = element_tags(&nodes[id])?;
        match open.name.as_str() {
            "li" => return Some((id, wrappers)),
            "div" => wrappers.push(id),
            _ => return None,
        }
        id = nodes[id].parent?;
    }
}

fn element_tags(node: &yu_markdown::html::HtmlNode) -> Option<(&HtmlTag, &HtmlTag)> {
    match &node.kind {
        HtmlNodeKind::Element {
            opening,
            closing: Some(closing),
        } => Some((opening, closing)),
        _ => None,
    }
}

fn continued_list_open(
    source: &str,
    tag: &HtmlTag,
    remove_id: bool,
    number: Option<u64>,
) -> String {
    let start = tag.source.start().get() as usize;
    let mut value = source[start..tag.source.end().get() as usize].to_owned();
    let mut edits = Vec::new();
    for attr in &tag.attributes {
        if remove_id && attr.name == "id" {
            edits.push((
                attr.source.start().get() as usize - start,
                attr.source.end().get() as usize - start,
                String::new(),
            ));
        } else if attr.name == "start"
            && let (Some(number), Some(range)) = (number, attr.value)
        {
            edits.push((
                range.start().get() as usize - start,
                range.end().get() as usize - start,
                number.to_string(),
            ));
        }
    }
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.0));
    for (start, end, text) in edits {
        value.replace_range(start..end, &text);
    }
    if let Some(number) = number
        && !tag.attributes.iter().any(|attr| attr.name == "start")
    {
        value.insert_str(value.len() - 1, &format!(" start=\"{number}\""));
    }
    value
}

impl EditorDocument {
    pub(super) fn root_html_list_outdent_plan(&self) -> Option<(TextRange, String, usize, usize)> {
        if self.presentation.selections.is_multiple() {
            return None;
        }
        self.root_html_list_outdent_for(self.selection())
    }

    fn root_html_list_outdent_for(
        &self,
        selection: EditorSelection,
    ) -> Option<(TextRange, String, usize, usize)> {
        let (range, text, points) = self.root_html_list_outdent_with_points(
            selection,
            &[selection.anchor(), selection.focus()],
        )?;
        Some((range, text, points[0], points[1]))
    }

    fn root_html_list_outdent_with_points(
        &self,
        selection: EditorSelection,
        points: &[ByteOffset],
    ) -> Option<(TextRange, String, Vec<usize>)> {
        if selection.is_empty() {
            return None;
        }
        let span = self.list_selection_span(selection)?;
        let model = span.model;
        let nodes = &model.fragment.nodes;
        let first_id = span.first;
        let last_id = span.last;
        let first = &nodes[first_id];
        let last = &nodes[last_id];
        let list_id = span.list;
        let list = &nodes[list_id];
        let (open, close) = element_tags(list)?;
        // A list behind a disclosure boundary is not a root list either.
        let mut ancestor = list.parent;
        while let Some(id) = ancestor {
            if element_tags(&nodes[id]).is_some_and(|(tag, _)| tag.name == "li") {
                return None;
            }
            ancestor = nodes[id].parent;
        }
        let siblings: Vec<_> = list
            .children
            .iter()
            .copied()
            .filter(|id| element_tags(&nodes[*id]).is_some_and(|(tag, _)| tag.name == "li"))
            .collect();
        let a = siblings.iter().position(|id| *id == first_id)?;
        let b = siblings.iter().position(|id| *id == last_id)?;
        let source = self.markdown.source().as_str();
        let slice = |a: ByteOffset, b: ByteOffset| &source[a.get() as usize..b.get() as usize];
        let mut output = String::new();
        if a > 0 {
            output.push_str(slice(open.source.start(), open.source.end()));
        }
        output.push_str(slice(open.source.end(), first.source.start()));
        if a > 0 {
            output.push_str(slice(close.source.start(), close.source.end()));
        }
        let mut mapped = vec![None; points.len()];
        let mut previous = first.source.start();
        for id in &siblings[a..=b] {
            let item = &nodes[*id];
            let (tag, end) = element_tags(item)?;
            output.push_str(slice(previous, item.source.start()));
            let block_body = item.children.iter().any(|child| {
                element_tags(&nodes[*child]).is_some_and(|(o, _)| {
                    matches!(
                        o.name.as_str(),
                        "p" | "div"
                            | "ul"
                            | "ol"
                            | "details"
                            | "h1"
                            | "h2"
                            | "h3"
                            | "h4"
                            | "h5"
                            | "h6"
                    )
                })
            });
            let wrapper = if block_body { "div" } else { "p" };
            let mut opening = slice(tag.source.start(), tag.source.end()).to_owned();
            opening.replace_range(1..3, wrapper);
            output.push_str(&opening);
            for (index, point) in points.iter().enumerate() {
                if tag.source.end() <= *point && *point <= end.source.start() {
                    mapped[index] =
                        Some(output.len() + (point.get() - tag.source.end().get()) as usize);
                }
            }
            output.push_str(slice(tag.source.end(), end.source.start()));
            output.push_str(&format!("</{wrapper}>"));
            previous = item.source.end();
        }
        if b + 1 < siblings.len() {
            let number = (open.name == "ol").then(|| {
                model.resolution.elements[list_id]
                    .as_ref()
                    .and_then(|e| e.attributes.start)
                    .unwrap_or(1)
                    .saturating_add(b as u64 + 1)
            });
            output.push_str(&continued_list_open(source, open, a > 0, number));
        }
        output.push_str(slice(last.source.end(), close.source.start()));
        if b + 1 < siblings.len() {
            output.push_str(slice(close.source.start(), close.source.end()));
        }
        Some((
            list.source,
            output,
            mapped.into_iter().collect::<Option<Vec<_>>>()?,
        ))
    }

    fn selected_html_list_outdent_plan(&self) -> Option<(TextRange, String, usize)> {
        self.selected_html_list_outdent_for(self.selection())
    }

    fn selected_html_list_outdent_for(
        &self,
        selection: EditorSelection,
    ) -> Option<(TextRange, String, usize)> {
        let span = self.list_selection_span(selection)?;
        let model = span.model;
        let nodes = &model.fragment.nodes;
        let first_id = span.first;
        let last_id = span.last;
        let first = &nodes[first_id];
        let last = &nodes[last_id];
        let list_id = span.list;
        let list = &nodes[list_id];
        let (open, close) = element_tags(list)?;
        let (parent_id, wrappers) = list_parent_through_divs(nodes, list_id)?;
        let parent = &nodes[parent_id];
        let (_, parent_close) = element_tags(parent)?;
        let siblings: Vec<_> = list
            .children
            .iter()
            .copied()
            .filter(|id| element_tags(&nodes[*id]).is_some_and(|(tag, _)| tag.name == "li"))
            .collect();
        let first_pos = siblings.iter().position(|id| *id == first_id)?;
        let last_pos = siblings.iter().position(|id| *id == last_id)?;
        let source = self.markdown.source().as_str();
        let slice = |a: ByteOffset, b: ByteOffset| &source[a.get() as usize..b.get() as usize];
        let mut replacement = slice(parent.source.start(), list.source.start()).to_owned();
        if first_pos > 0 {
            replacement.push_str(slice(open.source.start(), open.source.end()));
        }
        replacement.push_str(slice(open.source.end(), first.source.start()));
        if first_pos > 0 {
            replacement.push_str(slice(close.source.start(), close.source.end()));
        }
        for id in &wrappers {
            let (_, close) = element_tags(&nodes[*id])?;
            replacement.push_str(slice(close.source.start(), close.source.end()));
        }
        replacement.push_str(slice(
            parent_close.source.start(),
            parent_close.source.end(),
        ));
        let target =
            replacement.len() + (selection.focus().get() - first.source.start().get()) as usize;
        let (_, last_close) = element_tags(last)?;
        replacement.push_str(slice(first.source.start(), last_close.source.start()));
        for id in wrappers.iter().rev() {
            let (open, _) = element_tags(&nodes[*id])?;
            replacement.push_str(&opening_without_id(source, open));
        }
        if last_pos + 1 < siblings.len() {
            let number = (open.name == "ol").then(|| {
                model.resolution.elements[list_id]
                    .as_ref()
                    .and_then(|e| e.attributes.start)
                    .unwrap_or(1)
                    .saturating_add(last_pos as u64 + 1)
            });
            replacement.push_str(&continued_list_open(source, open, first_pos > 0, number));
            replacement.push_str(slice(last.source.end(), close.source.start()));
            replacement.push_str(slice(close.source.start(), close.source.end()));
        } else {
            replacement.push_str(slice(last.source.end(), close.source.start()));
        }
        replacement.push_str(slice(list.source.end(), parent_close.source.start()));
        replacement.push_str(slice(last_close.source.start(), last_close.source.end()));
        Some((parent.source, replacement, target))
    }

    pub(super) fn html_list_outdent_plan(
        &self,
        require_empty: bool,
    ) -> Option<(TextRange, String, usize)> {
        if self.presentation.selections.is_multiple() {
            return None;
        }
        if !self.selection().is_empty() {
            return if require_empty {
                None
            } else {
                self.selected_html_list_outdent_plan()
            };
        }
        self.html_list_outdent_at(self.selection().focus(), require_empty)
    }

    fn html_list_outdent_at(
        &self,
        caret: ByteOffset,
        require_empty: bool,
    ) -> Option<(TextRange, String, usize)> {
        let model = if require_empty {
            let block = self
                .block_index_for_offset(caret)
                .and_then(|i| self.markdown.blocks().get(i))?;
            self.html_table_grid(block)?;
            self.markdown
                .html_regions()
                .region_for(block.range())?
                .model
                .as_ref()
                .ok()?
        } else {
            self.native_list_model(caret)?
        };
        let nodes = &model.fragment.nodes;
        let (item_id, item) = nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| {
                element_tags(node).is_some_and(|(open, close)| {
                    open.name == "li" && open.source.end() <= caret && caret <= close.source.start()
                })
            })
            .min_by_key(|(_, node)| node.source.len())?;
        let (item_open, item_close) = element_tags(item)?;
        let list_id = item.parent?;
        let list = &nodes[list_id];
        let (list_open, list_close) = element_tags(list)?;
        if !matches!(list_open.name.as_str(), "ul" | "ol") {
            return None;
        }
        let source = self.markdown.source().as_str();
        let slice = |a: ByteOffset, b: ByteOffset| &source[a.get() as usize..b.get() as usize];
        let mut block_body = false;
        let mut paragraph_body = false;
        for node in nodes.iter().filter(|node| {
            node.source.start() >= item_open.source.end()
                && node.source.end() <= item_close.source.start()
        }) {
            match &node.kind {
                HtmlNodeKind::Text
                    if require_empty
                        && !slice(node.source.start(), node.source.end())
                            .trim()
                            .is_empty() =>
                {
                    return None;
                }
                HtmlNodeKind::Element { opening, closing } => {
                    if require_empty
                        && matches!(opening.name.as_str(), "img" | "details" | "ul" | "ol")
                    {
                        return None;
                    }
                    if opening.source.start() < caret && caret < opening.source.end()
                        || closing.as_ref().is_some_and(|tag| {
                            tag.source.start() < caret && caret < tag.source.end()
                        })
                    {
                        return None;
                    }
                    paragraph_body |= matches!(
                        opening.name.as_str(),
                        "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
                    );
                    block_body |= matches!(
                        opening.name.as_str(),
                        "p" | "div"
                            | "ul"
                            | "ol"
                            | "details"
                            | "h1"
                            | "h2"
                            | "h3"
                            | "h4"
                            | "h5"
                            | "h6"
                    );
                }
                HtmlNodeKind::Comment
                    if node.source.start() < caret && caret < node.source.end() =>
                {
                    return None;
                }
                _ => {}
            }
        }
        let item_siblings: Vec<_> = list
            .children
            .iter()
            .copied()
            .filter(|id| element_tags(&nodes[*id]).is_some_and(|(open, _)| open.name == "li"))
            .collect();
        let position = item_siblings.iter().position(|id| *id == item_id)?;
        let has_left = position > 0;
        let has_right = position + 1 < item_siblings.len();
        let mut left = String::new();
        if has_left {
            left.push_str(slice(list_open.source.start(), list_open.source.end()));
        }
        left.push_str(slice(list_open.source.end(), item.source.start()));
        if has_left {
            left.push_str(slice(list_close.source.start(), list_close.source.end()));
        }
        let mut right = String::new();
        if has_right {
            let number = (list_open.name == "ol").then(|| {
                model.resolution.elements[list_id]
                    .as_ref()
                    .and_then(|e| e.attributes.start)
                    .unwrap_or(1)
                    .saturating_add(position as u64 + 1)
            });
            right.push_str(&continued_list_open(source, list_open, has_left, number));
        }
        right.push_str(slice(item.source.end(), list_close.source.start()));
        if has_right {
            right.push_str(slice(list_close.source.start(), list_close.source.end()));
        }
        let body = slice(item_open.source.end(), item_close.source.start());
        let body_caret = (caret.get() - item_open.source.end().get()) as usize;
        if let Some((parent_id, wrappers)) = list_parent_through_divs(nodes, list_id) {
            let parent = &nodes[parent_id];
            let (_, parent_close) = element_tags(parent)?;
            let mut replacement = slice(parent.source.start(), list.source.start()).to_owned();
            replacement.push_str(&left);
            for id in &wrappers {
                let (_, close) = element_tags(&nodes[*id])?;
                replacement.push_str(slice(close.source.start(), close.source.end()));
            }
            replacement.push_str(slice(
                parent_close.source.start(),
                parent_close.source.end(),
            ));
            replacement.push_str(slice(item_open.source.start(), item_open.source.end()));
            let target = replacement.len() + body_caret;
            replacement.push_str(body);
            for id in wrappers.iter().rev() {
                let (open, _) = element_tags(&nodes[*id])?;
                replacement.push_str(&opening_without_id(source, open));
            }
            replacement.push_str(&right);
            replacement.push_str(slice(list.source.end(), parent_close.source.start()));
            replacement.push_str(slice(item_close.source.start(), item_close.source.end()));
            return Some((parent.source, replacement, target));
        }
        let mut replacement = left;
        // Transfer the item's attributes to its replacement paragraph/container.
        let wrapper = if block_body { "div" } else { "p" };
        let in_table = nodes.iter().any(|node| {
            element_tags(node).is_some_and(|(open, _)| open.name == "table")
                && node.source.start() <= caret
                && caret <= node.source.end()
        });
        let wrap = !block_body || !item_open.attributes.is_empty() || !in_table;
        if wrap {
            let mut opening = slice(item_open.source.start(), item_open.source.end()).to_owned();
            opening.replace_range(1..3, wrapper);
            replacement.push_str(&opening);
        }
        let mut target = replacement.len() + body_caret;
        replacement.push_str(body);
        // Empty containers alone have no paragraph geometry. Keep their bytes,
        // and give the exited caret a real editable paragraph.
        if require_empty && block_body && !paragraph_body {
            replacement.push_str("<p>");
            target = replacement.len();
            replacement.push_str("</p>");
        }
        if wrap {
            replacement.push_str(&format!("</{wrapper}>"));
        }
        replacement.push_str(&right);
        Some((list.source, replacement, target))
    }
}

impl EditorDocument {
    /// At the first editable position, Backspace removes one list level rather
    /// than walking through hidden tags into the previous item's text.
    pub(super) fn backspace_html_list_start(
        &mut self,
    ) -> Result<Option<CommandResult>, EditorDocumentError> {
        if !self.at_html_list_content_start() {
            return Ok(None);
        }
        self.change_html_list_indent(false)
    }

    fn at_html_list_content_start(&self) -> bool {
        if self.presentation.selections.is_multiple() || !self.selection().is_empty() {
            return false;
        }
        let caret = self.selection().focus();
        let Some(block) = self
            .block_index_for_offset(caret)
            .and_then(|i| self.markdown.blocks().get(i))
        else {
            return false;
        };
        if self.html_table_grid(block).is_none() {
            return false;
        }
        let index = self.markdown.html_regions();
        let Some(model) = index
            .region_for(block.range())
            .and_then(|region| region.model.as_ref().ok())
        else {
            return false;
        };
        let Some((open, close)) = model
            .fragment
            .nodes
            .iter()
            .filter_map(element_tags)
            .filter(|(open, close)| {
                open.name == "li" && open.source.end() <= caret && caret <= close.source.start()
            })
            .min_by_key(|(open, close)| close.source.end().get() - open.source.start().get())
        else {
            return false;
        };
        let source = self.markdown.source().as_str();
        for node in &model.fragment.nodes {
            if node.source.start() < open.source.end()
                || node.source.end() > close.source.start()
                || node.source.start() >= caret
            {
                continue;
            }
            match &node.kind {
                HtmlNodeKind::Text => {
                    let end = node.source.end().min(caret);
                    if !source[node.source.start().get() as usize..end.get() as usize]
                        .trim()
                        .is_empty()
                    {
                        return false;
                    }
                }
                HtmlNodeKind::Comment => {
                    if caret < node.source.end() {
                        return false;
                    }
                }
                HtmlNodeKind::Element { opening, closing } => {
                    if caret < opening.source.end()
                        || matches!(
                            opening.name.as_str(),
                            "img" | "br" | "details" | "ul" | "ol"
                        )
                    {
                        return false;
                    }
                    if let Some(closing) = closing {
                        if closing.source.start() < caret && caret < closing.source.end() {
                            return false;
                        }
                        if closing.source.end() <= caret
                            && matches!(
                                opening.name.as_str(),
                                "p" | "div" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
                            )
                        {
                            return false;
                        }
                    }
                }
            }
        }
        true
    }
}
