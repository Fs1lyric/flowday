use chrono::{NaiveDate, NaiveTime};
use flowday_core::model::NewTask;
use flowday_core::plan::{hours, propose};
use flowday_core::schedule::Hours;
use flowday_core::store::Store;

#[test]
fn proposal_fills_free_time_around_blocks() {
    let store = Store::open_in_memory().unwrap();
    store.set_setting("hours", "09:00-12:00").unwrap();
    store.set_setting("buffer_min", "0").unwrap();
    let day = NaiveDate::from_ymd_opt(2026, 10, 16).unwrap();
    let at = |h, m| day.and_time(NaiveTime::from_hms_opt(h, m, 0).unwrap());
    let blocked = store.add_task(&NewTask { title: "standup".into(), date: Some(day), ..Default::default() }).unwrap();
    store.schedule(blocked.id, at(9, 0), 30).unwrap();
    let a = store.add_task(&NewTask { title: "a".into(), date: Some(day), duration_min: Some(60), ..Default::default() }).unwrap();
    let b = store.add_task(&NewTask { title: "b".into(), date: Some(day), priority: 3, ..Default::default() }).unwrap();
    let plan = propose(&store, at(8, 0)).unwrap();
    assert_eq!(plan, vec![(b.id, at(9, 30)), (a.id, at(10, 0))]);
}

#[test]
fn hours_fall_back_to_nine_to_six() {
    let store = Store::open_in_memory().unwrap();
    assert_eq!(hours(&store), Hours::default());
    store.set_setting("hours", "08:30-16:00").unwrap();
    assert_eq!(hours(&store).start, NaiveTime::from_hms_opt(8, 30, 0).unwrap());
}
