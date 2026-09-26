//! Bounded civil-time ticks and axis formatting, shared by parse and layout.
use super::{civil_from_days, days_from_civil};
use crate::ir::{GanttTickUnit as Unit, Graph};
const DAY_MS: i64 = 86_400_000;
const MAX_TICKS: usize = 2048;

pub(crate) fn interval(text: &str) -> Option<(u32, Unit)> {
    let split = text.bytes().position(|b| !b.is_ascii_digit())?;
    let (number, suffix) = text.split_at(split);
    if number.is_empty() || number.starts_with('0') {
        return None;
    }
    let count = number.parse::<u32>().ok()?;
    let unit = match suffix {
        "millisecond" => Unit::Millisecond,
        "second" => Unit::Second,
        "minute" => Unit::Minute,
        "hour" => Unit::Hour,
        "day" => Unit::Day,
        "week" => Unit::Week,
        "month" => Unit::Month,
        _ => return None,
    };
    Some((count, unit))
}

pub(crate) fn supports_format(format: &str) -> bool {
    if format.is_empty() {
        return false;
    }
    let mut chars = format.chars();
    while let Some(ch) = chars.next() {
        if ch == '%'
            && !matches!(
                chars.next(),
                Some(
                    'Y' | 'y'
                        | 'm'
                        | 'd'
                        | 'e'
                        | 'H'
                        | 'I'
                        | 'M'
                        | 'S'
                        | 'L'
                        | 'p'
                        | 'j'
                        | 'w'
                        | 'a'
                        | 'A'
                        | 'b'
                        | 'B'
                        | 'x'
                        | 'X'
                        | '%'
                )
            )
        {
            return false;
        }
    }
    true
}

pub(crate) fn format_time(t: f64, format: &str) -> String {
    let ms = (t * DAY_MS as f64).round() as i64;
    let days = ms.div_euclid(DAY_MS) as i32;
    let within = ms.rem_euclid(DAY_MS);
    let (year, month, day) = civil_from_days(days);
    let hour = within / 3_600_000;
    let minute = within / 60_000 % 60;
    let second = within / 1000 % 60;
    let weekday = (days + 4).rem_euclid(7) as usize;
    let weekdays = [
        "Sunday",
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
    ];
    let months = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    let mut result = String::new();
    let mut chars = format.chars();
    while let Some(ch) = chars.next() {
        if ch != '%' {
            result.push(ch);
            continue;
        }
        let value = match chars.next() {
            Some('Y') => format!("{year:04}"),
            Some('y') => format!("{:02}", year.rem_euclid(100)),
            Some('m') => format!("{month:02}"),
            Some('d') => format!("{day:02}"),
            Some('e') => format!("{day:2}"),
            Some('H') => format!("{hour:02}"),
            Some('I') => format!("{:02}", (hour + 11) % 12 + 1),
            Some('M') => format!("{minute:02}"),
            Some('S') => format!("{second:02}"),
            Some('L') => format!("{:03}", within % 1000),
            Some('p') => if hour < 12 { "AM" } else { "PM" }.into(),
            Some('j') => format!("{:03}", days - days_from_civil(year, 1, 1) + 1),
            Some('w') => weekday.to_string(),
            Some('a') => weekdays[weekday][..3].into(),
            Some('A') => weekdays[weekday].into(),
            Some('b') => months[month as usize - 1][..3].into(),
            Some('B') => months[month as usize - 1].into(),
            Some('x') => format!("{month:02}/{day:02}/{year:04}"),
            Some('X') => format!("{hour:02}:{minute:02}:{second:02}"),
            Some('%') => "%".into(),
            _ => String::new(), // The parser rejects unknown tokens before layout.
        };
        result.push_str(&value);
    }
    result
}

// Day fractions can land just below an exact millisecond after f64 arithmetic.
// Snap only within its representational error, not genuine sub-ms durations.
fn precise_millis(days: f64) -> f64 {
    let value = days * DAY_MS as f64;
    let nearest = value.round();
    if (value - nearest).abs() <= value.abs().max(1.0) * f64::EPSILON * 2.0 {
        nearest
    } else {
        value
    }
}

pub(crate) fn ticks(graph: &Graph) -> anyhow::Result<Vec<f64>> {
    let mut start = graph
        .gantt_schedule
        .iter()
        .map(|v| v.0)
        .reduce(f64::min)
        .unwrap_or(0.0);
    let mut end = graph
        .gantt_schedule
        .iter()
        .map(|v| v.1)
        .reduce(f64::max)
        .unwrap_or(1.0);
    if !start.is_finite() || !end.is_finite() {
        start = 0.0;
        end = 1.0;
    }
    if end <= start {
        end = start + 1.0;
    }
    let mut times = Vec::new();
    let mut push = |t: f64| -> anyhow::Result<()> {
        if times.len() == MAX_TICKS {
            anyhow::bail!("Gantt tickInterval exceeds the 2048-tick resource limit");
        }
        times.push(t);
        Ok(())
    };
    let dated = graph.gantt_reference_day.is_some()
        || graph
            .gantt_tasks
            .iter()
            .any(|t| t.start.is_some() || t.end.is_some());
    let Some((count, unit)) = graph.gantt_tick_interval else {
        if dated && end - start < 1.0 {
            let wanted = ((end - start) * DAY_MS as f64 / 4.0).max(1.0);
            let step = [
                1, 5, 10, 50, 100, 250, 500, 1000, 5000, 15000, 30000, 60000, 300000, 900000,
                1800000, 3600000, 10800000, 21600000, 43200000, 86400000,
            ]
            .into_iter()
            .find(|n| *n as f64 >= wanted)
            .unwrap_or(DAY_MS);
            let first = ((precise_millis(start) / step as f64) - 1e-7).ceil() as i64 * step;
            for i in 0..=5 {
                let t = (first + i * step) as f64 / DAY_MS as f64;
                if t <= end + 1e-12 {
                    push(t)?;
                }
            }
        } else if dated {
            let step = ((end - start) / 4.0).ceil().max(1.0);
            for i in 0..=4 {
                let t = start.ceil() + step * f64::from(i);
                if t <= end {
                    push(t)?;
                }
            }
        } else {
            for i in 0..=4 {
                push(start + (end - start) * f64::from(i) / 4.0)?;
            }
        }
        return Ok(times);
    };
    let count = i64::from(count);
    match unit {
        Unit::Millisecond | Unit::Second | Unit::Minute | Unit::Hour => {
            let (size, cycle) = match unit {
                Unit::Millisecond => (1, None),
                Unit::Second => (1000, Some(60)),
                Unit::Minute => (60_000, Some(60)),
                _ => (3_600_000, Some(24)),
            };
            let mut field = (precise_millis(start) / size as f64 - 1e-7).ceil() as i64;
            let last = (precise_millis(end) / size as f64 + 1e-7).floor() as i64;
            while field <= last {
                let remainder = cycle
                    .map_or(field, |cycle| field.rem_euclid(cycle))
                    .rem_euclid(count);
                if remainder != 0 {
                    let advance = count - remainder;
                    field += cycle.map_or(advance, |cycle| {
                        advance.min(cycle - field.rem_euclid(cycle))
                    });
                    continue;
                }
                push((field * size) as f64 / DAY_MS as f64)?;
                field += cycle.map_or(count, |cycle| count.min(cycle - field.rem_euclid(cycle)));
            }
        }
        Unit::Day => {
            let mut t = start.ceil() as i64;
            while t as f64 <= end {
                let (year, month, day) = civil_from_days(t as i32);
                let next = i64::from(if month == 12 {
                    days_from_civil(year + 1, 1, 1)
                } else {
                    days_from_civil(year, month + 1, 1)
                });
                let remainder = i64::from(day - 1) % count;
                if remainder == 0 {
                    push(t as f64)?;
                }
                t = (t + if remainder == 0 {
                    count
                } else {
                    count - remainder
                })
                .min(next);
            }
        }
        Unit::Week => {
            let step = count * 7;
            let anchor = -4 + i64::from(graph.gantt_weekday);
            let mut t = anchor + ((start - anchor as f64) / step as f64).ceil() as i64 * step;
            while t as f64 <= end {
                push(t as f64)?;
                t += step;
            }
        }
        Unit::Month => {
            let (year, month, _) = civil_from_days(start.floor() as i32);
            let mut y = year;
            let mut m = month;
            loop {
                let t = f64::from(days_from_civil(y, m, 1));
                if t > end {
                    break;
                }
                if t >= start && i64::from(m - 1) % count == 0 {
                    push(t)?;
                }
                if m == 12 {
                    m = 1;
                    y += 1;
                } else {
                    m += 1;
                }
                if y > 9999 {
                    break;
                }
            }
        }
    }
    Ok(times)
}
