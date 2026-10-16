//! The offline quick-add parser: `Ship release #Work @deep !1 tomorrow 10am 1h`.
//!
//! Every token it recognises is pulled out of the title; anything else stays in the
//! title in order. Quote a word (`'tomorrow'`) to keep it literally.

use chrono::{Datelike, Days, Months, NaiveDate, NaiveTime, Weekday};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct QuickAdd {
    pub title: String,
    pub date: Option<NaiveDate>,
    pub time: Option<NaiveTime>,
    pub duration_min: Option<u32>,
    pub project: Option<String>,
    pub labels: Vec<String>,
    /// 0 = none, 3 = highest (`!1`).
    pub priority: u8,
    pub someday: bool,
    pub rrule: Option<String>,
}

struct Token {
    text: String,
    literal: bool,
}

fn tokenize(input: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut quoted: Option<(char, Vec<String>)> = None;
    for word in input.split_whitespace() {
        if let Some((q, mut words)) = quoted.take() {
            if let Some(stripped) = word.strip_suffix(q) {
                words.push(stripped.to_string());
                out.push(Token { text: words.join(" "), literal: true });
            } else {
                words.push(word.to_string());
                quoted = Some((q, words));
            }
            continue;
        }
        let first = word.chars().next().unwrap();
        if (first == '\'' || first == '"') && word.len() > 1 {
            let rest = &word[1..];
            if let Some(inner) = rest.strip_suffix(first) {
                out.push(Token { text: inner.to_string(), literal: true });
            } else {
                quoted = Some((first, vec![rest.to_string()]));
            }
            continue;
        }
        out.push(Token { text: word.to_string(), literal: false });
    }
    if let Some((q, words)) = quoted {
        out.push(Token { text: format!("{q}{}", words.join(" ")), literal: true });
    }
    out
}

fn weekday(s: &str) -> Option<Weekday> {
    Some(match s {
        "mon" | "monday" => Weekday::Mon,
        "tue" | "tues" | "tuesday" => Weekday::Tue,
        "wed" | "weds" | "wednesday" => Weekday::Wed,
        "thu" | "thur" | "thurs" | "thursday" => Weekday::Thu,
        "fri" | "friday" => Weekday::Fri,
        "sat" | "saturday" => Weekday::Sat,
        "sun" | "sunday" => Weekday::Sun,
        _ => return None,
    })
}

fn month(s: &str) -> Option<u32> {
    const NAMES: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
    let s = if s == "sept" { "sep" } else { s };
    let idx = NAMES.iter().position(|m| s.len() >= 3 && (s == *m || full_month(m).is_some_and(|f| f == s)))?;
    Some(idx as u32 + 1)
}

fn full_month(abbr: &str) -> Option<&'static str> {
    Some(match abbr {
        "jan" => "january",
        "feb" => "february",
        "mar" => "march",
        "apr" => "april",
        "may" => "may",
        "jun" => "june",
        "jul" => "july",
        "aug" => "august",
        "sep" => "september",
        "oct" => "october",
        "nov" => "november",
        "dec" => "december",
        _ => return None,
    })
}

fn day_of_month(s: &str) -> Option<u32> {
    let digits = s.trim_end_matches(|c: char| c.is_ascii_alphabetic());
    let suffix = &s[digits.len()..];
    if !matches!(suffix, "" | "st" | "nd" | "rd" | "th") {
        return None;
    }
    digits.parse().ok().filter(|d| (1..=31).contains(d))
}

/// The next `wd` strictly after `from`.
pub fn next_weekday(from: NaiveDate, wd: Weekday) -> NaiveDate {
    let ahead = (7 + wd.num_days_from_monday() as i64 - from.weekday().num_days_from_monday() as i64) % 7;
    from + Days::new(if ahead == 0 { 7 } else { ahead as u64 })
}

fn rrule_day(wd: Weekday) -> &'static str {
    ["MO", "TU", "WE", "TH", "FR", "SA", "SU"][wd.num_days_from_monday() as usize]
}

/// `5pm`, `12:30pm`, `9:30`, `14:05`. A bare number is not a time.
fn time(s: &str) -> Option<NaiveTime> {
    let (body, meridiem) = if let Some(b) = s.strip_suffix("am") {
        (b, Some(false))
    } else if let Some(b) = s.strip_suffix("pm") {
        (b, Some(true))
    } else {
        (s, None)
    };
    let (h, m) = match body.split_once(':') {
        Some((h, m)) if m.len() == 2 => (h.parse::<u32>().ok()?, m.parse::<u32>().ok()?),
        Some(_) => return None,
        None if meridiem.is_some() => (body.parse::<u32>().ok()?, 0),
        None => return None,
    };
    let h = match meridiem {
        Some(pm) if (1..=12).contains(&h) => h % 12 + if pm { 12 } else { 0 },
        Some(_) => return None,
        None => h,
    };
    NaiveTime::from_hms_opt(h, m, 0)
}

/// `2h`, `90m`, `45min`, `1h30m`, `2hrs`.
fn duration(s: &str) -> Option<u32> {
    let mut total = 0u32;
    let mut rest = s;
    let mut matched = false;
    while !rest.is_empty() {
        let digits = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
        if digits == 0 {
            return None;
        }
        let n: u32 = rest[..digits].parse().ok()?;
        rest = &rest[digits..];
        let unit = rest.find(|c: char| c.is_ascii_digit()).unwrap_or(rest.len());
        total += match &rest[..unit] {
            "h" | "hr" | "hrs" | "hour" | "hours" => n * 60,
            "m" | "min" | "mins" | "minute" | "minutes" => n,
            _ => return None,
        };
        rest = &rest[unit..];
        matched = true;
    }
    (matched && total > 0).then_some(total)
}

fn month_day(m: u32, d: u32, today: NaiveDate) -> Option<NaiveDate> {
    let date = NaiveDate::from_ymd_opt(today.year(), m, d)?;
    if date < today {
        NaiveDate::from_ymd_opt(today.year() + 1, m, d)
    } else {
        Some(date)
    }
}

pub fn parse(input: &str, today: NaiveDate) -> QuickAdd {
    let tokens = tokenize(input);
    let lower: Vec<String> = tokens.iter().map(|t| t.text.to_lowercase()).collect();
    let word = |i: usize| -> Option<&str> {
        tokens.get(i).filter(|t| !t.literal).map(|_| lower[i].as_str())
    };
    let mut q = QuickAdd::default();
    let mut title: Vec<&str> = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let tok = &tokens[i];
        if tok.literal {
            title.push(&tok.text);
            i += 1;
            continue;
        }
        let w = lower[i].as_str();
        let next = word(i + 1);
        let consumed = if let Some(p) = tok.text.strip_prefix('#').filter(|p| !p.is_empty()) {
            q.project = Some(p.to_string());
            1
        } else if let Some(l) = tok.text.strip_prefix('@').filter(|l| !l.is_empty()) {
            q.labels.push(l.to_string());
            1
        } else if let Some(p @ ("1" | "2" | "3")) = w.strip_prefix('!') {
            q.priority = 4 - p.parse::<u8>().unwrap();
            1
        } else if w == "today" || w == "tod" {
            q.date = Some(today);
            1
        } else if matches!(w, "tomorrow" | "tmr" | "tmrw" | "tom") {
            q.date = Some(today + Days::new(1));
            1
        } else if w == "someday" {
            q.someday = true;
            1
        } else if w == "next" && next == Some("week") {
            q.date = Some(next_weekday(today, Weekday::Mon));
            2
        } else if w == "next" && next.and_then(weekday).is_some() {
            q.date = Some(next_weekday(today, weekday(next.unwrap()).unwrap()));
            2
        } else if let Some(wd) = weekday(w) {
            q.date = Some(next_weekday(today, wd));
            1
        } else if w == "in" && next.and_then(|n| n.parse::<u64>().ok()).is_some()
            && matches!(word(i + 2), Some("day" | "days" | "week" | "weeks" | "month" | "months"))
        {
            let n: u64 = next.unwrap().parse().unwrap();
            q.date = match word(i + 2).unwrap() {
                "day" | "days" => today.checked_add_days(Days::new(n)),
                "week" | "weeks" => today.checked_add_days(Days::new(n * 7)),
                _ => today.checked_add_months(Months::new(n as u32)),
            };
            3
        } else if let Ok(d) = NaiveDate::parse_from_str(w, "%Y-%m-%d") {
            q.date = Some(d);
            1
        } else if let (Some(m), Some(d)) = (month(w), next.and_then(day_of_month)) {
            q.date = month_day(m, d, today);
            2
        } else if let (Some(d), Some(m)) = (day_of_month(w), next.and_then(month)) {
            q.date = month_day(m, d, today);
            2
        } else if let (true, Some(n)) = (w == "every", next) {
            let rule = match n {
                "day" => Some("FREQ=DAILY".to_string()),
                "weekday" => Some("FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR".to_string()),
                "week" => Some("FREQ=WEEKLY".to_string()),
                "month" => Some("FREQ=MONTHLY".to_string()),
                "year" => Some("FREQ=YEARLY".to_string()),
                _ => weekday(n).map(|wd| {
                    q.date.get_or_insert(next_weekday(today, wd));
                    format!("FREQ=WEEKLY;BYDAY={}", rrule_day(wd))
                }),
            };
            match rule {
                Some(r) => {
                    q.date.get_or_insert(today);
                    q.rrule = Some(r);
                    2
                }
                None => 0,
            }
        } else if w == "at" && next.and_then(time).is_some() {
            q.time = next.and_then(time);
            2
        } else if let Some(t) = time(w) {
            q.time = Some(t);
            1
        } else if w == "for" && next.and_then(duration).is_some() {
            q.duration_min = next.and_then(duration);
            2
        } else if let Some(d) = duration(w) {
            q.duration_min = Some(d);
            1
        } else {
            0
        };
        if consumed == 0 {
            title.push(&tok.text);
            i += 1;
        } else {
            i += consumed;
        }
    }
    if q.time.is_some() && q.date.is_none() {
        q.date = Some(today);
    }
    if q.someday {
        q.date = None;
        q.time = None;
    }
    q.title = title.join(" ");
    q
}
