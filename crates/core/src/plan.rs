//! Planning a day against the store: work hours and buffer from settings, busy time
//! from events and existing blocks, and where today's unscheduled tasks would go.

use chrono::NaiveDateTime;
use chrono::NaiveTime;

use crate::model::View;
use crate::schedule::{auto_schedule, free_slots, Hours, Slot, Want};
use crate::store::Store;

/// Work hours from the `hours` setting (`09:00-18:00`).
pub fn hours(store: &Store) -> Hours {
    let parse = |s: &str| NaiveTime::parse_from_str(s.trim(), "%H:%M").ok();
    store
        .setting("hours")
        .ok()
        .flatten()
        .and_then(|v| {
            let (a, b) = v.split_once('-')?;
            Some(Hours { start: parse(a)?, end: parse(b)? })
        })
        .unwrap_or_default()
}

pub fn buffer(store: &Store) -> u32 {
    store.setting("buffer_min").ok().flatten().and_then(|v| v.parse().ok()).unwrap_or(5)
}

/// Where today's unscheduled tasks would go: free time is work hours minus events
/// and blocks already on the calendar, from now on.
pub fn propose(store: &Store, now: NaiveDateTime) -> crate::store::Result<Vec<(i64, NaiveDateTime)>> {
    let day = now.date();
    let mut busy: Vec<Slot> = store.events_on(day)?.iter().filter(|e| !e.all_day).map(|e| Slot::new(e.start, e.end)).collect();
    let today = store.list(&View::Today, day)?;
    for t in store.blocks_on(day)? {
        if let Some(start) = t.start() {
            busy.push(Slot::new(start, start + chrono::Duration::minutes(i64::from(t.duration_min))));
        }
    }
    let free = free_slots(day, hours(store), &busy, Some(now));
    let wants: Vec<Want> = today
        .iter()
        .filter(|t| t.time.is_none())
        .map(|t| Want { id: t.id, minutes: t.duration_min, priority: t.priority, deadline: t.deadline })
        .collect();
    Ok(auto_schedule(&wants, &free, buffer(store)))
}

