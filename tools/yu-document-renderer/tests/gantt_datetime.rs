//! Production Gantt parse/layout/render regressions, not real-window evidence.
use mermaid_rs_renderer::layout::{DiagramData, GanttLayout};
use mermaid_rs_renderer::{Graph, LayoutConfig, Theme, compute_layout, parse_mermaid_strict_at};
use yu_document_renderer::{Kind, RenderStyle, render_styled};
const DAY: i32 = 20454; // 2026-01-01, independent of the test machine clock.
const DAY_MS: f64 = 86_400_000.0;
fn graph(source: &str, day: Option<i32>) -> Graph {
    parse_mermaid_strict_at(source, day).expect(source).graph
}
fn layout(g: &Graph) -> GanttLayout {
    let result = compute_layout(g, &Theme::modern(), &LayoutConfig::default());
    let DiagramData::Gantt(data) = result.diagram else {
        panic!("Gantt layout")
    };
    data
}
fn millis(days: f64) -> i64 {
    (days * DAY_MS).round() as i64
}
fn themes(source: &str) {
    for dark in [false, true] {
        let result = render_styled(
            Kind::Mermaid,
            source,
            RenderStyle {
                dark,
                reference_day: Some(DAY),
                ..RenderStyle::default()
            },
        )
        .expect(source);
        assert!(result.width > 0 && result.height > 0);
        assert!(!result.svg.contains("NaN") && !result.svg.contains("Infinity"));
    }
}

#[test]
fn numeric_date_formats_and_literals_preserve_canonical_milliseconds() {
    for (format, value, canonical) in [
        (
            "YYYY-MM-DD HH:mm:ss.SSS",
            "2026-01-02 13:04:05.123",
            "2026-01-02T13:04:05.123",
        ),
        (
            "D/M/YYYY H:m:s.S",
            "2/1/2026 3:4:5.1",
            "2026-01-02T03:04:05.100",
        ),
        (
            "YY-MM-DDTHH:mm:ss.SS",
            "26-01-02T13:04:05.12",
            "2026-01-02T13:04:05.120",
        ),
        (
            "YYYY年MM月DD日[ at ]hh:mm:ss A",
            "2026年01月02日 at 01:04:05 PM",
            "2026-01-02T13:04:05.000",
        ),
        (
            "YYYY-MM-DD HH:mm",
            "2026-01-02 13:04",
            "2026-01-02T13:04:00.000",
        ),
        (
            "YYYYMMDDHHmmss",
            "20260102130405",
            "2026-01-02T13:04:05.000",
        ),
    ] {
        let source =
            format!("gantt\ndateFormat {format}\n任务中文🙂 :a,{value},250ms\n后续 :after a,1s\n");
        for source in [source.clone(), source.replace('\n', "\r\n")] {
            let g = graph(&source, Some(DAY));
            assert_eq!(g.gantt_tasks[0].start.as_deref(), Some(canonical));
            assert_eq!(
                millis(g.gantt_schedule[0].1) - millis(g.gantt_schedule[0].0),
                250
            );
            assert_eq!(g.gantt_schedule[1].0, g.gantt_schedule[0].1);
            assert_eq!(
                millis(g.gantt_schedule[1].1) - millis(g.gantt_schedule[1].0),
                1000
            );
            themes(&source);
        }
    }
}

#[test]
fn time_only_uses_explicit_reference_day_and_no_hidden_clock() {
    let source = "gantt\ndateFormat HH:mm:ss.SSS\nA :a,23:59:59.750,500ms\nB :after a,1s";
    let first = graph(source, Some(DAY));
    let next = graph(source, Some(DAY + 1));
    let offline = graph(source, None);
    assert_eq!(
        first.gantt_tasks[0].start.as_deref(),
        Some("2026-01-01T23:59:59.750")
    );
    assert_eq!(
        offline.gantt_tasks[0].start.as_deref(),
        Some("1970-01-01T23:59:59.750")
    );
    for (a, b) in first.gantt_schedule.iter().zip(&next.gantt_schedule) {
        assert_eq!(millis(b.0) - millis(a.0), DAY_MS as i64);
        assert_eq!(millis(b.1) - millis(a.1), DAY_MS as i64);
    }
    assert!(first.gantt_schedule[0].1 > f64::from(DAY + 1));
    themes(source);
}

#[test]
fn explicit_endpoints_dependencies_and_inclusive_dates_keep_time_of_day() {
    let body = "gantt\ndateFormat YYYY-MM-DD HH:mm:ss.SSS\nA :a,2026-01-01 23:59:59.750,2026-01-02 00:00:00.250\nB :b,after a,250ms\nC :c,2026-01-01 23:59:59.750,until b";
    let a = graph(body, Some(DAY));
    assert_eq!(
        millis(a.gantt_schedule[0].1) - millis(a.gantt_schedule[0].0),
        500
    );
    assert_eq!(a.gantt_schedule[0].1, a.gantt_schedule[1].0);
    assert_eq!(a.gantt_schedule[2].1, a.gantt_schedule[1].0);
    let inclusive = graph(
        &body.replace("gantt\n", "gantt\ninclusiveEndDates\n"),
        Some(DAY),
    );
    assert_eq!(
        millis(inclusive.gantt_schedule[0].1) - millis(a.gantt_schedule[0].1),
        DAY_MS as i64
    );
    assert_eq!(
        millis(inclusive.gantt_schedule[1].1) - millis(inclusive.gantt_schedule[1].0),
        250
    );
    themes(body);
}

#[test]
fn partial_dates_and_short_year_cutoff_are_explicit() {
    for (format, value, expected) in [
        ("YYYY", "2026", "2026-01-01"),
        ("YYYY-MM", "2026-02", "2026-02-01"),
        ("YY-M-D", "68-1-2", "2068-01-02"),
        ("YY-M-D", "69-1-2", "1969-01-02"),
    ] {
        let g = graph(
            &format!("gantt\ndateFormat {format}\nA :a,{value},1d"),
            None,
        );
        assert_eq!(g.gantt_tasks[0].start.as_deref(), Some(expected));
    }
}

#[test]
fn twelve_hour_clock_handles_midnight_noon_and_case_strictly() {
    for (value, hour) in [
        ("12:00 AM", 0),
        ("12:00 PM", 12),
        ("01:00 PM", 13),
        ("11:00 PM", 23),
    ] {
        let g = graph(
            &format!("gantt\ndateFormat hh:mm A\nA :a,{value},1h"),
            Some(DAY),
        );
        assert_eq!(
            millis(g.gantt_schedule[0].0) - i64::from(DAY) * DAY_MS as i64,
            hour * 3_600_000
        );
    }
    let g = graph("gantt\ndateFormat h:m a\nA :a,1:2 pm,1s", Some(DAY));
    assert_eq!(
        g.gantt_tasks[0].start.as_deref(),
        Some("2026-01-01T13:02:00.000")
    );
}

#[test]
fn calendar_months_preserve_fractional_day_including_before_epoch() {
    for (start, end) in [
        ("1969-12-31 12:34", "1970-01-31 12:34"),
        ("2024-01-31 23:59", "2024-02-29 23:59"),
    ] {
        let g = graph(
            &format!("gantt\ndateFormat YYYY-MM-DD HH:mm\nA :a,{start},1M\nB :b,{end},0d"),
            None,
        );
        assert_eq!(g.gantt_schedule[0].1, g.gantt_schedule[1].0);
    }
}

#[test]
fn date_exclusions_keep_existing_day_step_contract_with_clock_fields() {
    let source = "gantt\ndateFormat YYYY-MM-DD HH:mm\nexcludes weekends\nA :a,2026-01-02 09:30,1d\nB :after a,30m";
    let g = graph(source, Some(DAY));
    assert_eq!(
        millis(g.gantt_schedule[0].1) - millis(g.gantt_schedule[0].0),
        3 * DAY_MS as i64
    );
    assert_eq!(
        millis(g.gantt_schedule[1].1) - millis(g.gantt_schedule[1].0),
        1_800_000
    );
    themes(source);
}

#[test]
fn minute_ticks_cross_midnight_and_axis_does_not_round_noon_to_tomorrow() {
    let source = "gantt\ndateFormat YYYY-MM-DD HH:mm\naxisFormat %m-%d %H:%M\ntickInterval 30minute\nA :a,2026-01-01 23:30,1h";
    let labels: Vec<_> = layout(&graph(source, Some(DAY)))
        .ticks
        .into_iter()
        .map(|t| t.label)
        .collect();
    assert_eq!(labels, ["01-01 23:30", "01-02 00:00", "01-02 00:30"]);
    let noon = source.replace("23:30", "13:30");
    assert!(
        layout(&graph(&noon, Some(DAY)))
            .ticks
            .iter()
            .all(|t| t.label.starts_with("01-01"))
    );
    themes(source);
}

#[test]
fn millisecond_second_and_hour_ticks_have_real_positions_and_labels() {
    for (format, start, interval, duration, labels) in [
        (
            "HH:mm:ss.SSS",
            "00:00:00.250",
            "250millisecond",
            "500ms",
            vec!["00:00:00.250", "00:00:00.500", "00:00:00.750"],
        ),
        (
            "HH:mm:ss.SSS",
            "00:00:59.000",
            "1second",
            "2s",
            vec!["00:00:59.000", "00:01:00.000", "00:01:01.000"],
        ),
        (
            "HH:mm:ss.SSS",
            "23:00:00.000",
            "1hour",
            "2h",
            vec!["23:00:00.000", "00:00:00.000", "01:00:00.000"],
        ),
    ] {
        let source = format!(
            "gantt\ndateFormat {format}\naxisFormat %H:%M:%S.%L\ntickInterval {interval}\nA :a,{start},{duration}"
        );
        let data = layout(&graph(&source, Some(DAY)));
        assert_eq!(
            data.ticks
                .iter()
                .map(|t| t.label.as_str())
                .collect::<Vec<_>>(),
            labels
        );
        assert!((data.ticks[1].x - data.ticks[0].x - data.chart_width / 2.0).abs() < 0.05);
        themes(&source);
    }
}

#[test]
fn calendar_month_ticks_reset_at_year_boundary_not_thirty_day_steps() {
    let source = "gantt\naxisFormat %Y-%m-%d\ntickInterval 1month\nA :a,2024-01-31,2024-04-01";
    let data = layout(&graph(source, None));
    assert_eq!(
        data.ticks
            .iter()
            .map(|t| t.label.as_str())
            .collect::<Vec<_>>(),
        ["2024-02-01", "2024-03-01", "2024-04-01"]
    );
    let source = "gantt\naxisFormat %Y-%m-%d\ntickInterval 3month\nA :a,2025-11-01,2026-07-01";
    assert_eq!(
        layout(&graph(source, None))
            .ticks
            .into_iter()
            .map(|t| t.label)
            .collect::<Vec<_>>(),
        ["2026-01-01", "2026-04-01", "2026-07-01"]
    );
}

#[test]
fn axis_calendar_tokens_negative_epoch_and_automatic_subday_ticks() {
    let source = "gantt\ndateFormat YYYY-MM-DD HH:mm:ss.SSS\naxisFormat %Y %y %j %a %A %b %B %I:%M:%S %p %L %%\ntickInterval 1hour\nA :a,1969-12-31 23:00:00.000,1h";
    let data = layout(&graph(source, None));
    assert_eq!(
        data.ticks[0].label,
        "1969 69 365 Wed Wednesday Dec December 11:00:00 PM 000 %"
    );
    let automatic = graph(
        "gantt\ndateFormat HH:mm:ss.SSS\nA :a,17:00:00.000,500ms",
        Some(DAY),
    );
    let labels = layout(&automatic)
        .ticks
        .into_iter()
        .map(|t| t.label)
        .collect::<Vec<_>>();
    assert!(labels.len() >= 2);
    assert!(labels.windows(2).all(|w| w[0] != w[1]));
    themes(source);
}

#[test]
fn fixed_dates_stay_fixed_but_reference_marker_changes_without_source_changes() {
    let source = "gantt\ndateFormat YYYY-MM-DD HH:mm\nA :a,2026-01-01 00:00,4d";
    let a = graph(source, Some(DAY + 1));
    let b = graph(source, Some(DAY + 2));
    assert_eq!(a.gantt_schedule, b.gantt_schedule);
    let a = layout(&a);
    let b = layout(&b);
    assert!(
        (b.today_marker.expect("today").0 - a.today_marker.expect("today").0 - a.chart_width / 4.0)
            .abs()
            < 0.01
    );
}

#[test]
fn invalid_formats_dates_times_and_density_are_explicit_failures_and_recover() {
    for (format, value) in [
        ("YYYY-MM-DD HH:mm", "2026-02-29 12:00"),
        ("HH:mm", "24:00"),
        ("HH:mm", "12:60"),
        ("HH:mm:ss", "12:59:60"),
        ("HH:mm", "1:00"),
        ("HH:mm", "01:00junk"),
        ("hh:mm A", "00:00 AM"),
        ("hh:mm A", "13:00 PM"),
        ("hh:mm A", "01:00 pm"),
        ("HH:mm:ss.SSS", "00:00:00.12"),
        ("YYYY-MM-DD", "0000-01-01"),
        ("HH:mm:ss.SSSS", "00:00:00.1234"),
        ("HH:HH", "12:12"),
        ("hh:mm", "01:00"),
        ("HH:mm A", "13:00 PM"),
        ("mm:ss", "20:00"),
        ("MM-DD", "01-02"),
        ("YYYY-MMM-DD", "2026-Jan-01"),
        ("YYYY-Q", "2026-1"),
        ("X", "1700000000"),
        ("YYYY-MM-DDTHH:mmZ", "2026-01-01T00:00Z"),
    ] {
        let source = format!("gantt\ndateFormat {format}\nA :a,{value},1h");
        assert!(
            parse_mermaid_strict_at(&source, Some(DAY)).is_err(),
            "{source}"
        );
    }
    for source in [
        "gantt\ndateFormat HH:mm\nA :a,23:55,00:05",
        "gantt\ndateFormat HH:mm\nA :a,01:00,until missing",
        "gantt\ntickInterval 1millisecond\nA :a,2026-01-01,2048ms",
        "gantt\ntickInterval 0second\nA :1h",
        "gantt\ntickInterval 01minute\nA :1h",
        "gantt\naxisFormat %Z\nA :1d",
        "gantt\naxisFormat %Q\nA :1d",
    ] {
        assert!(
            parse_mermaid_strict_at(source, Some(DAY)).is_err(),
            "{source}"
        );
    }
    themes("gantt\ndateFormat HH:mm\nA :a,23:55,10m");
}

#[test]
fn long_datetime_tick_labels_expand_geometry_without_removing_ticks() {
    let short = "gantt\ndateFormat HH:mm\naxisFormat %H\ntickInterval 1hour\nA :a,08:00,6h";
    let long = short.replace("%H", "%Y-%m-%d %H:%M:%S.%L");
    let a = layout(&graph(short, Some(DAY)));
    let b = layout(&graph(&long, Some(DAY)));
    assert_eq!(a.ticks.len(), 7);
    assert_eq!(b.ticks.len(), a.ticks.len());
    assert!(b.chart_width > a.chart_width * 1.5);
    for ((x, y), index) in a.ticks.iter().zip(&b.ticks).zip(0..) {
        let fraction_a = (x.x - a.chart_x) / a.chart_width;
        let fraction_b = (y.x - b.chart_x) / b.chart_width;
        assert!((fraction_a - fraction_b).abs() < 0.0001, "tick {index}");
    }
    themes(&long);
}

#[test]
fn native_fixture_has_four_supported_and_two_rejected_charts() {
    let source =
        include_str!("../../../platform/macos/yu-shell-macos/Fixtures/group4-gantt-calendar.md");
    let diagrams: Vec<_> = source
        .split("```mermaid\n")
        .skip(1)
        .map(|part| part.split("```").next().expect("fence"))
        .collect();
    assert_eq!(diagrams.len(), 6);
    for (index, diagram) in diagrams.iter().enumerate() {
        for source in [diagram.to_string(), diagram.replace('\n', "\r\n")] {
            let result = render_styled(
                Kind::Mermaid,
                &source,
                RenderStyle {
                    reference_day: Some(DAY),
                    ..RenderStyle::default()
                },
            );
            assert_eq!(result.is_ok(), index < 4, "fixture {index}");
        }
    }
}

#[test]
fn bounded_tick_limit_does_not_silently_drop_the_tail() {
    let source =
        "gantt\ndateFormat HH:mm:ss.SSS\ntickInterval 1millisecond\nA :a,00:00:00.000,2047ms";
    let g = graph(source, Some(DAY));
    assert_eq!(g.gantt_ticks.len(), 2048);
    assert_eq!(
        millis(*g.gantt_ticks.last().expect("last tick")) - millis(g.gantt_schedule[0].0),
        2047
    );
}
