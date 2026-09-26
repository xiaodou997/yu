//! Shared, case-sensitive Gantt duration grammar. Times are measured in days.
pub(crate) fn duration_days(value: &str) -> Option<f64> {
    let value = value.trim();
    let (number, factor) = if let Some(number) = value.strip_suffix("ms") {
        (number, 1.0 / 86_400_000.0)
    } else {
        let unit = value.chars().last()?;
        let factor = match unit {
            's' => 1.0 / 86_400.0,
            'm' => 1.0 / 1_440.0,
            'h' => 1.0 / 24.0,
            'd' => 1.0,
            'w' => 7.0,
            'M' => 30.0,
            'y' => 365.0,
            _ => return None,
        };
        (&value[..value.len() - unit.len_utf8()], factor)
    };
    if number.is_empty() || !number.bytes().all(|c| c.is_ascii_digit() || c == b'.') {
        return None;
    }
    let result = number.parse::<f64>().ok()? * factor;
    (result.is_finite() && result >= 0.0).then_some(result)
}

/// Calendar months and years preserve the day where possible, clamping at
/// month end. Fixed units retain sub-day precision.
pub(crate) fn duration_at(value: &str, start: f64) -> Option<f64> {
    let value = value.trim();
    let months = if let Some(number) = value.strip_suffix('M') {
        number.parse::<f64>().ok()?
    } else if let Some(number) = value.strip_suffix('y') {
        number.parse::<f64>().ok()? * 12.0
    } else {
        return duration_days(value);
    };
    if !months.is_finite() || months < 0.0 || months > 120_000.0 {
        return None;
    }
    let (year, month, day) = civil_from_days(start.floor() as i32);
    let absolute_month = i64::from(year) * 12 + i64::from(month) - 1 + months.round() as i64;
    let year = i32::try_from(absolute_month.div_euclid(12)).ok()?;
    let month = (absolute_month.rem_euclid(12) + 1) as u32;
    let next = if month == 12 {
        days_from_civil(year + 1, 1, 1)
    } else {
        days_from_civil(year, month + 1, 1)
    };
    let length = (next - days_from_civil(year, month, 1)) as u32;
    let end = f64::from(days_from_civil(year, month, day.min(length))) + (start - start.floor());
    Some(end - start)
}

pub(crate) fn days_from_civil(year: i32, month: u32, day: u32) -> i32 {
    let y = year - (month <= 2) as i32;
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = month as i32;
    let d = day as i32;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

pub(crate) fn civil_from_days(days: i32) -> (i32, u32, u32) {
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let year = y + (m <= 2) as i32;
    (year, m as u32, d as u32)
}

#[path = "gantt_axis.rs"]
pub(crate) mod axis;
#[path = "gantt_date.rs"]
mod date;
pub(crate) use date::{canonical_date, formatted_date, parse_timestamp, supports_date_format};

/// Validate a Gregorian date without normalizing impossible days into next month.
pub(crate) fn parse_date(value: &str) -> Option<i32> {
    let parts: Vec<_> = value.split(['-', '/', '.']).collect();
    if parts.len() != 3 {
        return None;
    }
    let year: i32 = parts[0].parse().ok()?;
    let month: u32 = parts[1].parse().ok()?;
    let day: u32 = parts[2].parse().ok()?;
    if !(1..=9999).contains(&year) || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let days = days_from_civil(year, month, day);
    (civil_from_days(days) == (year, month, day)).then_some(days)
}

/// Resolve separate start/end dependencies in topological order. `until` uses
/// a start node, whereas `after` uses end nodes; conflating them rejects valid
/// graphs or permits circular schedules.
pub(crate) struct Schedule {
    pub times: Vec<(f64, f64)>,
    pub render_ends: Vec<f64>,
}
pub(crate) fn resolve(
    tasks: &[crate::ir::GanttTask],
    calendar: &crate::ir::GanttCalendar,
    reference_day: Option<i32>,
    inclusive_end_dates: bool,
) -> anyhow::Result<Schedule> {
    use anyhow::{anyhow, bail};
    use std::collections::{HashMap, VecDeque};
    let mut ids = HashMap::new();
    for (i, task) in tasks.iter().enumerate() {
        if ids.insert(task.id.as_str(), i).is_some() {
            bail!("Duplicate Gantt task ID: {}", task.id);
        }
    }
    let index = |id: &str| {
        ids.get(id)
            .copied()
            .ok_or_else(|| anyhow!("Unknown Gantt dependency: {id}"))
    };
    let mut prerequisites = vec![Vec::new(); tasks.len() * 2];
    let mut dependents = vec![Vec::new(); tasks.len() * 2];
    let mut values = vec![None; tasks.len() * 2];
    let mut render_ends = vec![0.0; tasks.len()];
    let origin = reference_day
        .map(f64::from)
        .or_else(|| {
            tasks
                .iter()
                .filter_map(|t| t.start.as_deref().and_then(parse_timestamp))
                .reduce(f64::min)
                .map(f64::floor)
        })
        .unwrap_or(0.0);
    for (i, task) in tasks.iter().enumerate() {
        let start = i * 2;
        let end = start + 1;
        if let Some(date) = task.start.as_deref() {
            values[start] = Some(
                parse_timestamp(date).ok_or_else(|| anyhow!("Invalid Gantt start date: {date}"))?,
            );
        } else if let Some(after) = task.after.as_deref() {
            for id in after.split_whitespace() {
                prerequisites[start].push(index(id)? * 2 + 1);
            }
            if prerequisites[start].is_empty() {
                bail!("Empty Gantt after dependency");
            }
        } else if i > 0 {
            prerequisites[start].push(start - 1);
        } else {
            values[start] = Some(origin);
        }
        if let Some(date) = task.end.as_deref() {
            values[end] = Some(
                parse_timestamp(date).ok_or_else(|| anyhow!("Invalid Gantt end date: {date}"))?
                    + if inclusive_end_dates { 1.0 } else { 0.0 },
            );
        } else if let Some(until) = task.until.as_deref() {
            for id in until.split_whitespace() {
                prerequisites[end].push(index(id)? * 2);
            }
            if prerequisites[end].is_empty() {
                bail!("Empty Gantt until dependency");
            }
        } else {
            prerequisites[end].push(start);
            if task.duration.is_none() {
                bail!("Missing Gantt end or duration: {}", task.id);
            }
        }
    }
    let mut remaining: Vec<_> = prerequisites.iter().map(Vec::len).collect();
    for (node, parents) in prerequisites.iter().enumerate() {
        for &parent in parents {
            dependents[parent].push(node);
        }
    }
    let mut queue: VecDeque<_> = remaining
        .iter()
        .enumerate()
        .filter_map(|(i, count)| (*count == 0).then_some(i))
        .collect();
    let mut completed = 0;
    while let Some(node) = queue.pop_front() {
        let task = &tasks[node / 2];
        if values[node].is_none() {
            let parents = prerequisites[node]
                .iter()
                .map(|i| values[*i].expect("topological parent"));
            let base = if node % 2 == 1 && task.until.is_some() {
                parents.fold(f64::INFINITY, f64::min)
            } else {
                parents.fold(f64::NEG_INFINITY, f64::max)
            };
            values[node] = Some(if node % 2 == 1 && task.until.is_none() {
                let duration =
                    duration_at(task.duration.as_deref().expect("validated duration"), base)
                        .ok_or_else(|| anyhow!("Invalid Gantt duration: {}", task.id))?;
                base + duration
            } else {
                base
            });
        }
        let value = values[node].expect("resolved time");
        if !value.is_finite() || !(-719162.0..2932897.0).contains(&value) {
            bail!("Gantt time exceeds native calendar bounds: {}", task.id);
        }
        if node % 2 == 1 {
            let end = values[node].expect("resolved end");
            render_ends[node / 2] = end;
            if calendar.is_active() && task.duration.is_some() {
                let start = values[node - 1].expect("duration needs start");
                let (end, visible_end) = extend_for_calendar(start, end, calendar)?;
                values[node] = Some(end);
                render_ends[node / 2] = visible_end;
            }
        }
        completed += 1;
        for &child in &dependents[node] {
            remaining[child] -= 1;
            if remaining[child] == 0 {
                queue.push_back(child);
            }
        }
    }
    if completed != values.len() {
        bail!("Circular Gantt task dependency");
    }
    let mut result = Vec::with_capacity(tasks.len());
    for (i, pair) in values.chunks_exact(2).enumerate() {
        let start = pair[0].expect("resolved start");
        let end = pair[1].expect("resolved end");
        if !start.is_finite()
            || !end.is_finite()
            || !(-719162.0..2932897.0).contains(&end)
            || end < start
            || (end - start) > 3_652_500.0
        {
            bail!("Invalid or oversized Gantt task interval: {}", tasks[i].id);
        }
        result.push((start, end));
    }
    if calendar.is_active() && !result.is_empty() {
        let min = result.iter().map(|t| t.0).fold(f64::INFINITY, f64::min);
        let max = result.iter().map(|t| t.1).fold(f64::NEG_INFINITY, f64::max);
        if max - min > 10_000.0 {
            bail!("Gantt exclusion calendar exceeds the 10000-day resource limit");
        }
    }
    Ok(Schedule {
        times: result,
        render_ends,
    })
}

fn extend_for_calendar(
    start: f64,
    original_end: f64,
    calendar: &crate::ir::GanttCalendar,
) -> anyhow::Result<(f64, f64)> {
    let mut day = start + 1.0;
    let mut end = original_end;
    let mut visible_end = original_end;
    let mut previous_excluded = false;
    let mut scanned = 0;
    while day <= end {
        if scanned >= 10_000 {
            anyhow::bail!("Gantt exclusion calendar exceeds the 10000-day resource limit");
        }
        if !previous_excluded {
            visible_end = end;
        }
        previous_excluded = calendar.is_excluded(day.floor() as i32);
        if previous_excluded {
            end += 1.0;
        }
        day += 1.0;
        scanned += 1;
    }
    Ok((end, visible_end))
}
