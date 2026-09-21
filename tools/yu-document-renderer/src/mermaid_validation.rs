//! Host-side checks for syntax silently discarded by the native parser.

pub(crate) fn validate(source: &str) -> Result<(), String> {
    let prepared = mermaid_rs_renderer::parser::prepare_frontmatter_source(source)
        .map_err(|error| error.to_string())?;
    let normalized = mermaid_rs_renderer::parser::normalize_flowchart_multiline(&prepared)
        .map_err(|error| error.to_string())?;
    let source = normalized.as_ref();
    let mut lines = source.lines().enumerate().filter_map(|(index, line)| {
        let line = line.trim();
        (!line.is_empty() && !line.starts_with("%%")).then_some((index + 1, line))
    });
    let Some((mut header_line, mut header)) = lines.next() else {
        return Ok(());
    };
    if header == "---" {
        if !lines.by_ref().any(|(_, line)| line == "---") {
            return Err("Unterminated Mermaid frontmatter".into());
        }
        let Some((diagram_line, diagram_header)) = lines.next() else {
            return Err("Mermaid frontmatter must be followed by a diagram".into());
        };
        header = diagram_header;
        header_line = diagram_line;
    }
    if header.starts_with("flowchart ") || header.starts_with("graph ") {
        for (line_number, line) in std::iter::once((header_line, header)).chain(lines) {
            for statement in flowchart_statements(line, line_number)? {
                if statement.starts_with("click ") || statement.starts_with("click\t") {
                    return Err(format!(
                        "Interactive Mermaid click directives are not supported by native document diagrams (line {line_number})"
                    ));
                }
            }
        }
        return Ok(());
    }
    let Some(rest) = header.strip_prefix("pie") else {
        return Ok(());
    };
    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        return Ok(());
    }
    let rest = rest.trim();
    let rest = rest.strip_prefix("showData").unwrap_or(rest).trim();
    if !rest.is_empty() && !rest.starts_with("title ") {
        return Err("Unsupported pie header; expected pie, showData, or title".into());
    }
    let mut entries = 0;
    for (line_number, line) in lines {
        if line.starts_with("title ") {
            continue;
        }
        if mermaid_rs_renderer::parser::parse_quoted_pie_entry(line).is_none() {
            return Err(format!(
                "Invalid pie entry on line {line_number}: expected a quoted label and finite nonnegative number"
            ));
        }
        entries += 1;
    }
    if entries == 0 {
        return Err("Pie diagram requires at least one numeric entry".into());
    }
    Ok(())
}

// Follow the native parser's statement boundaries: semicolons inside node shapes
// or quoted labels are text, not statement separators.
fn flowchart_statements(line: &str, line_number: usize) -> Result<Vec<&str>, String> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut depth = 0usize;
    let mut quote = None;
    let mut escaped = false;
    let mut end = line.len();
    for (index, ch) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if let Some(active) = quote {
            if ch == active {
                quote = None;
            }
            continue;
        }
        if ch == '%' && line[index..].starts_with("%%") {
            end = index;
            break;
        }
        match ch {
            '"' | '\'' => quote = Some(ch),
            '[' | '(' | '{' => depth += 1,
            ']' | ')' | '}' => depth = depth.saturating_sub(1),
            ';' if depth == 0 => {
                result.push(line[start..index].trim());
                start = index + 1;
            }
            _ => {}
        }
    }
    if depth != 0 || quote.is_some() {
        return Err(format!(
            "Unclosed flowchart node or quoted label on line {line_number}"
        ));
    }
    result.push(line[start..end].trim());
    Ok(result)
}
