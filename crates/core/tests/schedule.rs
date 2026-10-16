use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use flowday_core::schedule::{auto_schedule, free_slots, Hours, Slot, Want};

fn day() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, 16).unwrap()
}
fn at(h: u32, m: u32) -> NaiveDateTime {
    day().and_time(NaiveTime::from_hms_opt(h, m, 0).unwrap())
}
fn hours() -> Hours {
    Hours { start: NaiveTime::from_hms_opt(9, 0, 0).unwrap(), end: NaiveTime::from_hms_opt(17, 0, 0).unwrap() }
}

#[test]
fn free_time_is_work_hours_minus_busy() {
    let busy = [Slot::new(at(10, 0), at(11, 0)), Slot::new(at(10, 30), at(12, 0)), Slot::new(at(16, 30), at(18, 0))];
    let free = free_slots(day(), hours(), &busy, None);
    assert_eq!(free, vec![Slot::new(at(9, 0), at(10, 0)), Slot::new(at(12, 0), at(16, 30))]);
}

#[test]
fn free_time_starts_after_now_rounded_up_to_15_min() {
    let free = free_slots(day(), hours(), &[], Some(at(13, 7)));
    assert_eq!(free, vec![Slot::new(at(13, 15), at(17, 0))]);
    assert!(free_slots(day(), hours(), &[], Some(at(18, 0))).is_empty());
}

#[test]
fn auto_schedule_puts_important_work_first_and_skips_what_does_not_fit() {
    let free = vec![Slot::new(at(9, 0), at(10, 0)), Slot::new(at(12, 0), at(14, 0))];
    let wants = [
        Want { id: 1, minutes: 30, priority: 0, deadline: None },
        Want { id: 2, minutes: 90, priority: 3, deadline: None },
        Want { id: 3, minutes: 45, priority: 1, deadline: None },
        Want { id: 4, minutes: 180, priority: 2, deadline: None },
    ];
    let plan = auto_schedule(&wants, &free, 0);
    // 2 (prio 3, 90m) can't fit 9-10 so goes to 12:00; 3 (prio 1) at 9:00; 1 at 9:45? no: 9:45+30 > 10, so 13:30
    assert_eq!(plan, vec![(3, at(9, 0)), (2, at(12, 0)), (1, at(13, 30))]);
}

#[test]
fn auto_schedule_leaves_a_buffer_and_prefers_near_deadlines() {
    let free = vec![Slot::new(at(9, 0), at(11, 0))];
    let soon = NaiveDate::from_ymd_opt(2026, 10, 17);
    let wants = [
        Want { id: 1, minutes: 30, priority: 0, deadline: None },
        Want { id: 2, minutes: 30, priority: 0, deadline: soon },
    ];
    assert_eq!(auto_schedule(&wants, &free, 10), vec![(2, at(9, 0)), (1, at(9, 40))]);
}
