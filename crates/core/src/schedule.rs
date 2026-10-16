//! Free time and auto-scheduling. Pure functions over wall-clock times; the caller
//! gathers the busy slots (events and existing blocks) from the store.

use chrono::{Duration, NaiveDate, NaiveDateTime, NaiveTime, Timelike};

/// Working hours for one day.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hours {
    pub start: NaiveTime,
    pub end: NaiveTime,
}

impl Default for Hours {
    fn default() -> Self {
        Self { start: NaiveTime::from_hms_opt(9, 0, 0).unwrap(), end: NaiveTime::from_hms_opt(18, 0, 0).unwrap() }
    }
}

/// A half-open span of time, `start..end`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slot {
    pub start: NaiveDateTime,
    pub end: NaiveDateTime,
}

impl Slot {
    pub fn new(start: NaiveDateTime, end: NaiveDateTime) -> Self {
        Self { start, end }
    }

    pub fn minutes(&self) -> i64 {
        (self.end - self.start).num_minutes()
    }
}

/// A task that wants a place on the calendar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Want {
    pub id: i64,
    pub minutes: u32,
    /// 0 = none, 3 = highest.
    pub priority: u8,
    pub deadline: Option<NaiveDate>,
}

fn round_up_15(t: NaiveDateTime) -> NaiveDateTime {
    let t = t.with_second(0).unwrap().with_nanosecond(0).unwrap();
    let over = t.minute() % 15;
    if over == 0 {
        t
    } else {
        t + Duration::minutes(i64::from(15 - over))
    }
}

/// The free slots on `day` inside `hours`, minus `busy`. With `now`, time before it
/// (rounded up to the next quarter hour) is not free.
pub fn free_slots(day: NaiveDate, hours: Hours, busy: &[Slot], now: Option<NaiveDateTime>) -> Vec<Slot> {
    let mut start = day.and_time(hours.start);
    let end = day.and_time(hours.end);
    if let Some(now) = now {
        start = start.max(round_up_15(now));
    }
    if start >= end {
        return Vec::new();
    }
    let mut busy: Vec<Slot> = busy.iter().copied().filter(|b| b.end > start && b.start < end).collect();
    busy.sort_by_key(|b| b.start);

    let mut free = Vec::new();
    let mut cursor = start;
    for b in busy {
        if b.start > cursor {
            free.push(Slot::new(cursor, b.start.min(end)));
        }
        cursor = cursor.max(b.end);
        if cursor >= end {
            break;
        }
    }
    if cursor < end {
        free.push(Slot::new(cursor, end));
    }
    free
}

/// Place each want, most important first (priority, then nearest deadline), at the
/// earliest free time it fits, with `buffer_min` minutes after each. Wants that fit
/// nowhere are left out. Returns `(id, start)` in start order.
pub fn auto_schedule(wants: &[Want], free: &[Slot], buffer_min: u32) -> Vec<(i64, NaiveDateTime)> {
    let mut order: Vec<&Want> = wants.iter().collect();
    order.sort_by(|a, b| {
        b.priority
            .cmp(&a.priority)
            .then_with(|| match (a.deadline, b.deadline) {
                (Some(x), Some(y)) => x.cmp(&y),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            })
            .then(a.id.cmp(&b.id))
    });

    let mut free: Vec<Slot> = free.to_vec();
    let mut plan = Vec::new();
    for w in order {
        let need = i64::from(w.minutes);
        if let Some(slot) = free.iter_mut().filter(|s| s.minutes() >= need).min_by_key(|s| s.start) {
            plan.push((w.id, slot.start));
            slot.start = (slot.start + Duration::minutes(need + i64::from(buffer_min))).min(slot.end);
        }
    }
    plan.sort_by_key(|&(_, start)| start);
    plan
}
