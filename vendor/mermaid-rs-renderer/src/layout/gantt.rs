use super::*;
use crate::gantt_time::{civil_from_days, days_from_civil};

fn gantt_palette(theme: &Theme) -> Vec<String> {
    vec![
        theme.primary_border_color.clone(),
        "#0ea5e9".to_string(), // sky-500
        "#10b981".to_string(), // emerald-500
        "#6366f1".to_string(), // indigo-500
        "#f97316".to_string(), // orange-500
    ]
}

fn hsl_color(h: f32, s: f32, l: f32) -> String {
    format!("hsl({:.10}, {:.10}%, {:.10}%)", h, s, l)
}

fn shift_color(color: &str, target_s: f32, target_l: f32, strength: f32) -> String {
    let Some((_h, s, l)) = parse_color_to_hsl(color) else {
        return color.to_string();
    };
    let delta_s = (target_s - s) * strength;
    let delta_l = (target_l - l) * strength;
    adjust_color(color, 0.0, delta_s, delta_l)
}

fn gantt_section_palette(theme: &Theme, sections: &[String]) -> HashMap<String, String> {
    let mut map = HashMap::new();
    if sections.is_empty() {
        return map;
    }
    let base = theme.primary_border_color.as_str();
    let step = 360.0 / sections.len().max(1) as f32;
    for (idx, name) in sections.iter().enumerate() {
        let hue_shift = step * idx as f32;
        let mut color = adjust_color(base, hue_shift, 0.0, 0.0);
        color = shift_color(&color, 60.0, 55.0, 0.4);
        map.insert(name.clone(), color);
    }
    map
}

fn gantt_task_color(status: Option<crate::ir::GanttStatus>, base: &str, fallback: &str) -> String {
    let base = if parse_color_to_hsl(base).is_some() {
        base.to_string()
    } else {
        fallback.to_string()
    };
    let status = status.unwrap_or_default();
    // Completion wins over active when contradictory tags are supplied;
    // critical remains an independent outline and milestone remains a shape.
    if status.done { return shift_color(&base, 30.0, 80.0, 0.7); }
    if status.active { return shift_color(&base, 70.0, 52.0, 0.6); }
    if status.milestone {
        if let Some((_, saturation, lightness)) = parse_color_to_hsl(&base) {
            return hsl_color(45.0, saturation.max(65.0), lightness.clamp(50.0, 65.0));
        }
        return "#f59e0b".into();
    }
    if status.critical {
        if let Some((_, saturation, lightness)) = parse_color_to_hsl(&base) {
            return hsl_color(0.0, saturation.max(65.0), lightness.clamp(45.0, 60.0));
        }
        return "#ef4444".into();
    }
    base
}

fn format_gantt_date(days: i32) -> String {
    let (year, month, day) = civil_from_days(days);
    format!("{:04}-{:02}-{:02}", year, month, day)
}

pub(super) fn compute_gantt_layout(graph: &Graph, theme: &Theme, config: &LayoutConfig) -> Layout {
    let padding = theme.font_size * 1.25;
    let row_height = (theme.font_size * 1.5).max(theme.font_size + 8.0);
    let label_gap = theme.font_size * 1.05;

    let title = graph
        .gantt_title
        .as_ref()
        .map(|t| measure_label(t, theme, config));
    let title_height = title.as_ref().map(|t| t.height + padding).unwrap_or(0.0);
    let title_y = padding + title.as_ref().map(|t| t.height * 0.5).unwrap_or(0.0);

    let mut task_label_width = 0.0_f32;
    let mut section_label_width = 0.0_f32;
    for task in &graph.gantt_tasks {
        let label = measure_label(&task.label, theme, config);
        task_label_width = task_label_width.max(label.width);
        if let Some(section) = task.section.as_ref() {
            let section_label = measure_label(section, theme, config);
            section_label_width = section_label_width.max(section_label.width);
        }
    }
    task_label_width = task_label_width.max(theme.font_size * 6.5);

    let label_x = padding;
    let section_task_gap = if section_label_width > 0.0 {
        theme.font_size * 0.8
    } else {
        0.0
    };
    let label_width = section_label_width + section_task_gap + task_label_width;
    let section_label_x = label_x;
    let task_label_x = label_x + section_label_width + section_task_gap;
    let chart_x = padding + label_width + label_gap;
    let top_axis_height = if graph.gantt_top_axis { row_height * 0.9 + theme.font_size } else { 0.0 };
    let chart_y = title_height + padding + top_axis_height;
    let top_axis_y = graph.gantt_top_axis.then_some(chart_y - row_height * 0.6);
    let chart_width = theme.font_size * 26.0;

    assert_eq!(graph.gantt_tasks.len(), graph.gantt_schedule.len(), "Gantt schedule must be resolved before layout");
    assert_eq!(graph.gantt_tasks.len(), graph.gantt_render_ends.len(), "Gantt visual endpoints must be resolved before layout");
    let has_dates = graph.gantt_reference_day.is_some() || graph.gantt_tasks.iter().any(|task| task.start.is_some() || task.end.is_some());
    let mut time_start = f64::MAX;
    let mut time_end = f64::MIN;
    let mut computed = Vec::with_capacity(graph.gantt_tasks.len());
    for ((task, &(start, end)), &visible_end) in graph.gantt_tasks.iter().zip(&graph.gantt_schedule).zip(&graph.gantt_render_ends) {
        time_start = time_start.min(start);
        time_end = time_end.max(end);
        // Milestones use the scheduled midpoint. Ordinary bars may trim a
        // trailing excluded-day span without changing dependency scheduling.
        let display_end = if task.status.is_some_and(|flags| flags.milestone) { end } else { visible_end };
        computed.push((task.label.clone(), start, display_end - start, task.status, task.section.clone()));
    }
    if !time_start.is_finite() || !time_end.is_finite() {
        time_start = 0.0;
        time_end = 1.0;
    }
    if time_end <= time_start {
        time_end = time_start + 1.0;
    }
    let time_span = (time_end - time_start).max(f64::EPSILON);
    let time_scale = f64::from(chart_width) / time_span;
    let today_marker = graph.gantt_reference_day.and_then(|day| {
        let day = f64::from(day);
        if day < time_start || day > time_end { return None; }
        graph.gantt_today_marker.clone().map(|style| (chart_x + ((day - time_start) * time_scale) as f32, style))
    });
    let mut excluded_spans: Vec<(f32, f32)> = Vec::new();
    if graph.gantt_calendar.is_active() {
        for day in (time_start.floor() as i32)..(time_end.ceil() as i32) {
            if !graph.gantt_calendar.is_excluded(day) { continue; }
            let x = chart_x + ((f64::from(day).max(time_start) - time_start) * time_scale) as f32;
            let end = chart_x + ((f64::from(day + 1).min(time_end) - time_start) * time_scale) as f32;
            if let Some(last) = excluded_spans.last_mut() && (last.0 + last.1 - x).abs() < 0.01 { last.1 = end - last.0; }
            else { excluded_spans.push((x, end - x)); }
        }
    }


    let mut ticks: Vec<GanttTick> = Vec::new();
    let tick_times: Vec<f64> = if let Some(days) = graph.gantt_tick_days {
        let step = f64::from(days);
        let mut first = time_start.ceil();
        if graph.gantt_tick_weekly && has_dates {
            // D3 week intervals count from the week containing the Unix epoch.
            let anchor = -4.0 + f64::from(graph.gantt_weekday);
            first = anchor + ((first - anchor) / step).ceil() * step;
        }
        let mut times = Vec::new();
        let mut t = first;
        while t <= time_end && times.len() <= 2048 {
            if has_dates && !graph.gantt_tick_weekly {
                // D3 day.every(n) restarts its day-of-month field each month.
                let (year, month, day) = civil_from_days(t as i32);
                let next_month = if month == 12 { days_from_civil(year + 1, 1, 1) }
                    else { days_from_civil(year, month + 1, 1) };
                let remainder = (day - 1) % days;
                if remainder != 0 {
                    t = (t + f64::from(days - remainder)).min(f64::from(next_month));
                    continue;
                }
                times.push(t);
                t = (t + step).min(f64::from(next_month));
            } else {
                times.push(t);
                t += step;
            }
        }
        times
    } else if has_dates {
        // Calendar ticks use whole days; short charts must not repeat dates.
        let step = (time_span / 4.0).ceil().max(1.0);
        (0..=4).map(|i| time_start + step * f64::from(i))
            .take_while(|t| *t <= time_end).collect()
    } else {
        (0..=4).map(|i| time_start + time_span * f64::from(i) / 4.0).collect()
    };
    for t in tick_times {
        let x = chart_x + ((t - time_start) * time_scale) as f32;
        let label = if has_dates {
            if let Some(format) = graph.gantt_axis_format.as_deref() {
                let (year, month, day) = civil_from_days(t.round() as i32);
                let mut result = String::new();
                let mut chars = format.chars();
                while let Some(ch) = chars.next() {
                    if ch != '%' { result.push(ch); continue; }
                    match chars.next() {
                        Some('Y') => result.push_str(&format!("{year:04}")),
                        Some('m') => result.push_str(&format!("{month:02}")),
                        Some('d') => result.push_str(&format!("{day:02}")),
                        Some('%') => result.push('%'),
                        _ => {}, // Strict parsing rejects unsupported tokens.
                    }
                }
                result
            } else { format_gantt_date(t.round() as i32) }
        } else { format!("{:.0}", t - time_start) };
        ticks.push(GanttTick { x, label });
    }

    let compact = graph
        .gantt_display_mode
        .as_deref()
        .map(|m| m.eq_ignore_ascii_case("compact"))
        .unwrap_or(false);

    let palette = gantt_palette(theme);
    let section_palette = gantt_section_palette(theme, &graph.gantt_sections);
    let mut current_section: Option<String> = None;
    let mut current_section_idx: Option<usize> = None;
    let mut sections: Vec<GanttSectionLayout> = Vec::new();
    let mut tasks: Vec<GanttTaskLayout> = Vec::new();
    let mut y = chart_y;

    // Compact mode: pack non-overlapping tasks into the same row.
    // Each lane is y_position, end_time
    let mut lanes: Vec<(f32, f64)> = Vec::new();

    for (idx, (label, start, duration, status, section)) in computed.iter().enumerate() {
        if section != &current_section {
            if let Some(sec) = section.as_ref() {
                if let Some(prev_idx) = current_section_idx {
                    let height = (y - sections[prev_idx].y).max(row_height);
                    sections[prev_idx].height = height;
                }
                lanes.clear();
                let base_color = section_palette
                    .get(sec)
                    .cloned()
                    .unwrap_or_else(|| palette[idx % palette.len()].clone());
                let band_color = shift_color(&base_color, 20.0, 92.0, 0.7);
                sections.push(GanttSectionLayout {
                    label: measure_label(sec, theme, config),
                    y,
                    height: 0.0,
                    color: base_color,
                    band_color,
                });
                current_section_idx = Some(sections.len() - 1);
            } else if let Some(prev_idx) = current_section_idx {
                let height = (y - sections[prev_idx].y).max(row_height);
                sections[prev_idx].height = height;
                current_section_idx = None;
                lanes.clear();
            }
            current_section = section.clone();
        }

        let milestone = status.is_some_and(|flags| flags.milestone);
        let display_start = if milestone { start + duration * 0.5 } else { *start };
        let bar_x = chart_x + ((display_start - time_start) * time_scale) as f32;
        let bar_width = (duration * time_scale) as f32;
        let base_color = if let Some(sec) = section.as_ref() {
            section_palette
                .get(sec)
                .cloned()
                .unwrap_or_else(|| palette[idx % palette.len()].clone())
        } else {
            palette[idx % palette.len()].clone()
        };
        let color = gantt_task_color(*status, &base_color, &palette[0]);
        let critical = status.is_some_and(|flags| flags.critical);
        let border_color = if critical { "#df4b52".to_owned() } else { theme.primary_border_color.clone() };
        let border_width = if critical { 2.5 } else { 1.0 };


        let task_end = start + duration;
        let task_y = if compact {
            if let Some(lane) = lanes.iter_mut().find(|(_, end)| *start >= *end) {
                let ly = lane.0;
                lane.1 = task_end;
                ly
            } else {
                let ly = y;
                lanes.push((ly, task_end));
                y += row_height;
                ly
            }
        } else {
            let ly = y;
            y += row_height;
            ly
        };

        tasks.push(GanttTaskLayout {
            label: measure_label(label, theme, config),
            x: bar_x,
            y: task_y,
            width: bar_width,
            height: row_height,
            color,
            border_color,
            border_width,
            start: *start,
            duration: *duration,
            status: *status,
        });
    }
    if let Some(prev_idx) = current_section_idx {
        let height = (y - sections[prev_idx].y).max(row_height);
        sections[prev_idx].height = height;
    }

    let tick_font = theme.font_size * 0.8;
    let max_tick_half_width = ticks
        .iter()
        .map(|tick| {
            measure_label_with_font_size(
                tick.label.as_str(),
                tick_font,
                config,
                false,
                theme.font_family.as_str(),
            )
            .width
                / 2.0
        })
        .fold(0.0_f32, f32::max);
    let axis_pad = row_height * 0.9 + theme.font_size;
    let height = y + padding + axis_pad;
    let label_overflow = if compact {
        padding + task_label_width
    } else {
        0.0
    };
    let right_padding = (max_tick_half_width + padding * 0.4)
        .max(label_overflow)
        .max(padding);
    let width = chart_x + chart_width + right_padding;

    Layout {
        frontmatter_title: None,
        kind: graph.kind,
        nodes: BTreeMap::new(),
        edges: Vec::new(),
        subgraphs: Vec::new(),
        diagram: DiagramData::Gantt(GanttLayout {
            top_axis_y,
            title,
            sections,
            tasks,
            time_start,
            time_end,
            chart_x,
            chart_y,
            chart_width,
            chart_height: y - chart_y,
            row_height,
            label_x,
            label_width,
            section_label_x,
            section_label_width,
            task_label_x,
            task_label_width,
            title_y,
            ticks,
            excluded_spans,
            today_marker,
            compact,
        }),
        width,
        height,
    }
}

#[cfg(test)]
mod tests {
    use crate::ir::{DiagramKind, GanttStatus, GanttTask, Graph};
    use crate::layout::LayoutConfig;
    use crate::layout::types::DiagramData;
    use crate::theme::Theme;

    use super::compute_gantt_layout;

    #[test]
    fn multiline_title_stays_above_task_area() {
        let theme = Theme::mermaid_default();
        let config = LayoutConfig::default();
        for count in [1, 2, 4] {
            let mut graph = make_graph(None, vec![task("Task", "Work", "2026-01-01", "1d")]);
            graph.gantt_title = Some(vec!["中文 title"; count].join("\n"));
            let layout = compute_gantt_layout(&graph, &theme, &config);
            let DiagramData::Gantt(gantt) = layout.diagram else { panic!("Gantt layout") };
            let title = gantt.title.as_ref().expect("title");
            assert_eq!(title.lines.len(), count);
            assert!(gantt.title_y - title.height * 0.5 >= 0.0);
            assert!(gantt.title_y + title.height * 0.5 + theme.font_size <= gantt.chart_y);
        }
    }

    fn make_graph(display_mode: Option<&str>, tasks: Vec<GanttTask>) -> Graph {
        let mut graph = Graph::new();
        graph.kind = DiagramKind::Gantt;
        graph.gantt_display_mode = display_mode.map(|s| s.to_string());
        let mut sections = Vec::new();
        for t in &tasks {
            if let Some(sec) = &t.section
                && !sections.contains(sec)
            {
                sections.push(sec.clone());
            }
        }
        graph.gantt_sections = sections;
        graph.gantt_tasks = tasks;
        let schedule = crate::gantt_time::resolve(&graph.gantt_tasks, &graph.gantt_calendar, graph.gantt_reference_day, graph.gantt_inclusive_end_dates).expect("test schedule");
        graph.gantt_schedule = schedule.times;
        graph.gantt_render_ends = schedule.render_ends;
        graph
    }

    fn task(id: &str, section: &str, start: &str, dur: &str) -> GanttTask {
        GanttTask {
            id: id.to_string(),
            label: id.to_string(),
            start: Some(start.to_string()),
            duration: Some(dur.to_string()),
            after: None,
            end: None,
            until: None,
            section: Some(section.to_string()),
            status: None,
        }
    }

    fn milestone(id: &str, section: &str, start: &str) -> GanttTask {
        GanttTask {
            id: id.to_string(),
            label: id.to_string(),
            start: Some(start.to_string()),
            duration: Some("0d".to_string()),
            after: None,
            end: None,
            until: None,
            section: Some(section.to_string()),
            status: Some(GanttStatus { milestone: true, ..GanttStatus::default() }),
        }
    }

    fn extract_task_ys(graph: &Graph) -> Vec<f32> {
        let theme = Theme::modern();
        let config = LayoutConfig::default();
        let layout = compute_gantt_layout(graph, &theme, &config);
        match &layout.diagram {
            DiagramData::Gantt(g) => g.tasks.iter().map(|t| t.y).collect(),
            _ => panic!("expected Gantt layout"),
        }
    }

    #[test]
    fn compact_non_overlapping_tasks_share_row() {
        let graph = make_graph(
            Some("compact"),
            vec![
                milestone("m1", "MacOS", "2025-09-01"),
                milestone("m2", "MacOS", "2026-09-01"),
                milestone("m3", "MacOS", "2027-09-01"),
            ],
        );
        let ys = extract_task_ys(&graph);
        assert_eq!(ys[0], ys[1], "m1 and m2 should share a row");
        assert_eq!(ys[1], ys[2], "m2 and m3 should share a row");
    }

    #[test]
    fn compact_overlapping_tasks_get_separate_rows() {
        let graph = make_graph(
            Some("compact"),
            vec![
                task("hw", "support", "2025-09-01", "1y"),
                task("sw", "support", "2025-09-01", "2y"),
            ],
        );
        let ys = extract_task_ys(&graph);
        assert_ne!(ys[0], ys[1], "overlapping tasks must be on different rows");
    }

    #[test]
    fn non_compact_always_separate_rows() {
        let graph = make_graph(
            None,
            vec![
                milestone("m1", "MacOS", "2025-09-01"),
                milestone("m2", "MacOS", "2026-09-01"),
                milestone("m3", "MacOS", "2027-09-01"),
            ],
        );
        let ys = extract_task_ys(&graph);
        assert_ne!(ys[0], ys[1]);
        assert_ne!(ys[1], ys[2]);
    }
}
