//! Strict numeric civil date/time grammar. No locale or hidden system clock.
use super::{civil_from_days, days_from_civil};

const DAY_MS: i64 = 86_400_000;
#[derive(Clone, Copy)]
enum Field {
    Year,
    ShortYear,
    Month,
    Day,
    Hour,
    Hour12,
    Minute,
    Second,
    Fraction,
    Meridiem,
}
#[derive(Clone)]
enum Part {
    Number(Field, usize),
    Period(bool),
    Literal(String),
}
struct Pattern {
    parts: Vec<Part>,
    clock: bool,
    dated: bool,
}

fn pattern(format: &str) -> Option<Pattern> {
    let mut rest = format;
    let mut parts = Vec::new();
    let mut seen = 0u16;
    let mut twelve = false;
    while !rest.is_empty() {
        if let Some(literal) = rest.strip_prefix('[') {
            let end = literal.find(']')?;
            let value = &literal[..end];
            if value.contains(['[', ',', '\r', '\n', '\0']) {
                return None;
            }
            parts.push(Part::Literal(value.to_owned()));
            rest = &literal[end + 1..];
            continue;
        }
        let matched = [
            ("YYYY", Field::Year, 4, 1),
            ("YY", Field::ShortYear, 2, 1),
            ("MM", Field::Month, 2, 2),
            ("M", Field::Month, 0, 2),
            ("DD", Field::Day, 2, 4),
            ("D", Field::Day, 0, 4),
            ("HH", Field::Hour, 2, 8),
            ("H", Field::Hour, 0, 8),
            ("hh", Field::Hour12, 2, 8),
            ("h", Field::Hour12, 0, 8),
            ("mm", Field::Minute, 2, 16),
            ("m", Field::Minute, 0, 16),
            ("ss", Field::Second, 2, 32),
            ("s", Field::Second, 0, 32),
            ("SSS", Field::Fraction, 3, 64),
            ("SS", Field::Fraction, 2, 64),
            ("S", Field::Fraction, 1, 64),
            ("A", Field::Meridiem, 0, 128),
            ("a", Field::Meridiem, 0, 128),
        ]
        .into_iter()
        .find(|(token, _, _, _)| rest.starts_with(token));
        if let Some((token, field, width, bit)) = matched {
            if seen & bit != 0 {
                return None;
            }
            seen |= bit;
            twelve |= matches!(field, Field::Hour12);
            parts.push(if matches!(field, Field::Meridiem) {
                Part::Period(token == "A")
            } else {
                Part::Number(field, width)
            });
            rest = &rest[token.len()..];
        } else {
            let ch = rest.chars().next()?;
            // T is the conventional ISO separator. Other literal letters need [].
            if (ch.is_ascii_alphanumeric() && ch != 'T')
                || matches!(ch, ']' | ',' | '\r' | '\n' | '\0')
            {
                return None;
            }
            parts.push(Part::Literal(ch.to_string()));
            rest = &rest[ch.len_utf8()..];
        }
    }
    let date = seen & 7;
    let clock = seen & 248 != 0;
    if !matches!(date, 0 | 1 | 3 | 7)
        || (date == 0 && !clock)
        || (clock && seen & 8 == 0)
        || (seen & 32 != 0 && seen & 16 == 0)
        || (seen & 64 != 0 && seen & 32 == 0)
        || twelve != (seen & 128 != 0)
    {
        return None;
    }
    Some(Pattern {
        parts,
        clock,
        dated: date != 0,
    })
}

struct Value {
    year: i32,
    month: u32,
    day: u32,
    clock_ms: u32,
    clock: bool,
}
fn parse(value: &str, format: &str, reference_day: Option<i32>) -> Option<Value> {
    let p = pattern(format)?;
    let (mut year, mut month, mut day) = if p.dated {
        (1970, 1, 1)
    } else {
        civil_from_days(reference_day.unwrap_or(0))
    };
    let (mut hour, mut minute, mut second, mut millis) = (0, 0, 0, 0);
    let mut pm = None;
    let mut twelve = false;
    let mut rest = value;
    for part in p.parts {
        match part {
            Part::Literal(literal) => rest = rest.strip_prefix(&literal)?,
            Part::Period(upper) => {
                let period = rest.get(..2)?;
                pm = Some(match (upper, period) {
                    (true, "AM") | (false, "am") => false,
                    (true, "PM") | (false, "pm") => true,
                    _ => return None,
                });
                rest = &rest[2..];
            }
            Part::Number(field, width) => {
                let count = if width == 0 {
                    rest.bytes().take(2).take_while(u8::is_ascii_digit).count()
                } else {
                    width
                };
                if count == 0 {
                    return None;
                }
                let digits = rest.get(..count)?;
                if !digits.bytes().all(|b| b.is_ascii_digit()) {
                    return None;
                }
                let number: u32 = digits.parse().ok()?;
                rest = &rest[count..];
                match field {
                    Field::Year => year = number as i32,
                    Field::ShortYear => {
                        year = number as i32 + if number > 68 { 1900 } else { 2000 }
                    }
                    Field::Month => month = number,
                    Field::Day => day = number,
                    Field::Hour => hour = number,
                    Field::Hour12 => {
                        hour = number;
                        twelve = true;
                    }
                    Field::Minute => minute = number,
                    Field::Second => second = number,
                    Field::Fraction => millis = number * 10u32.pow(3 - width as u32),
                    Field::Meridiem => return None,
                }
            }
        }
    }
    if !rest.is_empty()
        || !(1..=9999).contains(&year)
        || !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || minute > 59
        || second > 59
        || hour > 23
        || (twelve && !(1..=12).contains(&hour))
    {
        return None;
    }
    if civil_from_days(days_from_civil(year, month, day)) != (year, month, day) {
        return None;
    }
    if twelve {
        hour = hour % 12 + if pm? { 12 } else { 0 };
    }
    Some(Value {
        year,
        month,
        day,
        clock_ms: ((hour * 60 + minute) * 60 + second) * 1000 + millis,
        clock: p.clock,
    })
}

pub(crate) fn supports_date_format(format: &str) -> bool {
    pattern(format).is_some()
}

pub(crate) fn formatted_date(value: &str, format: &str) -> Option<i32> {
    let p = pattern(format)?;
    if p.clock || !p.dated {
        return None;
    }
    let v = parse(value, format, None)?;
    Some(days_from_civil(v.year, v.month, v.day))
}

pub(crate) fn canonical_date(
    value: &str,
    format: &str,
    reference_day: Option<i32>,
) -> Option<String> {
    let v = parse(value, format, reference_day)?;
    let date = format!("{:04}-{:02}-{:02}", v.year, v.month, v.day);
    if !v.clock {
        return Some(date);
    }
    let ms = v.clock_ms;
    Some(format!(
        "{date}T{:02}:{:02}:{:02}.{:03}",
        ms / 3_600_000,
        ms / 60_000 % 60,
        ms / 1000 % 60,
        ms % 1000
    ))
}

/// Internal canonical timestamps preserve milliseconds independently of labels.
pub(crate) fn parse_timestamp(value: &str) -> Option<f64> {
    if !value.contains('T') {
        return super::parse_date(value).map(f64::from);
    }
    let v = parse(value, "YYYY-MM-DDTHH:mm:ss.SSS", None)?;
    let ms = i64::from(days_from_civil(v.year, v.month, v.day)) * DAY_MS + i64::from(v.clock_ms);
    Some(ms as f64 / DAY_MS as f64)
}
