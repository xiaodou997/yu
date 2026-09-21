use super::text::measure_label_with_font_size;
use super::{ErRowLayout, ErTableLayout};
use crate::{config::LayoutConfig, theme::Theme};

pub(super) fn measure_table(source: &str, theme: &Theme, config: &LayoutConfig) -> ErTableLayout {
    let (title, body) = source.split_once("\n---\n").unwrap_or((source, ""));
    let measure = |text: &str| {
        measure_label_with_font_size(text, theme.font_size, config, true, &theme.font_family)
    };
    let title = measure(title);
    let padding = (theme.font_size * 0.8).max(10.0);
    let header_height = title.height + padding;
    let mut rows = Vec::new();
    let mut columns = [0.0f32; 4];
    let mut top = header_height;
    for source in body.lines().filter(|line| !line.trim().is_empty()) {
        // Strict parsing rejects invalid source. Manually constructed or permissive
        // graphs retain that source visibly instead of silently dropping a row.
        let attr = crate::er::parse_attribute(source).unwrap_or_else(|_| crate::er::Attribute {
            name: source.to_string(),
            data_type: String::new(),
            keys: Vec::new(),
            comment: "Invalid ER attribute".into(),
        });
        let name = measure(&attr.name);
        let data_type = measure(&attr.data_type);
        let comment = measure(&attr.comment);
        let keys: Vec<_> = attr
            .keys
            .into_iter()
            .map(|key| {
                let label = measure_label_with_font_size(
                    &key,
                    theme.font_size * 0.72,
                    config,
                    false,
                    &theme.font_family,
                );
                let width = label.width + (theme.font_size * 0.45).max(4.0) * 2.0;
                (key, width)
            })
            .collect();
        let key_width = keys.iter().map(|(_, width)| width).sum::<f32>()
            + keys.len().saturating_sub(1) as f32 * theme.font_size * 0.4;
        for (column, width) in columns.iter_mut().zip([
            name.width,
            data_type.width,
            key_width,
            if attr.comment.is_empty() {
                0.0
            } else {
                comment.width
            },
        ]) {
            if width > 0.0 {
                *column = column.max(width + padding * 2.0);
            }
        }
        let height = name.height.max(data_type.height).max(comment.height) + padding;
        rows.push(ErRowLayout {
            name,
            data_type,
            comment,
            keys,
            top,
            height,
        });
        top += height;
    }
    let width = columns.iter().sum::<f32>().max(title.width + padding * 2.0);
    if !rows.is_empty() {
        columns[0] += width - columns.iter().sum::<f32>();
    }
    ErTableLayout {
        title,
        rows,
        columns,
        padding,
        header_height,
        width,
        height: top,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wrapped_comment_keeps_one_semantic_row_and_all_keys() {
        let mut config = LayoutConfig::default();
        config.max_label_width_chars = 12;
        let table = measure_table(
            "CUSTOMER\n---\nstring id PK, FK, UK \"closing } remains text with a long comment\"\nstring name",
            &Theme::modern(),
            &config,
        );
        assert_eq!(table.rows.len(), 2);
        assert_eq!(table.rows[0].keys.len(), 3);
        assert!(table.rows[0].comment.lines.len() > 1);
        assert!(table.rows[0].height > table.rows[1].height);
        assert_eq!(
            table.rows[1].top,
            table.header_height + table.rows[0].height
        );
        assert_eq!(table.height, table.rows[1].top + table.rows[1].height);
        assert_eq!(table.width, table.columns.iter().sum::<f32>());
    }
}
