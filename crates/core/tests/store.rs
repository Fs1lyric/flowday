use chrono::{NaiveDate, NaiveTime};
use flowday_core::model::*;
use flowday_core::quickadd::parse;
use flowday_core::store::Store;

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}
fn today() -> NaiveDate {
    d(2026, 10, 16)
}
fn t(h: u32, m: u32) -> NaiveTime {
    NaiveTime::from_hms_opt(h, m, 0).unwrap()
}
fn titles(v: &[Task]) -> Vec<&str> {
    v.iter().map(|t| t.title.as_str()).collect()
}

#[test]
fn quick_add_creates_project_and_labels() {
    let s = Store::open_in_memory().unwrap();
    let task = s.add_quick(&parse("Ship #Work @deep !1 tomorrow 10am 1h", today())).unwrap();
    assert_eq!(task.title, "Ship");
    assert_eq!(task.priority, 3);
    assert_eq!(task.date, Some(d(2026, 10, 17)));
    assert_eq!(task.time, Some(t(10, 0)));
    assert_eq!(task.duration_min, 60);
    assert_eq!(task.labels, vec!["deep".to_string()]);
    let work = s.project_by_name("work").unwrap().expect("project matched case-insensitively");
    assert_eq!(task.project_id, Some(work.id));
    // a second quick add reuses the project
    s.add_quick(&parse("Review #work", today())).unwrap();
    assert_eq!(s.projects().unwrap().len(), 1);
}

#[test]
fn views_split_tasks_like_akiflow() {
    let s = Store::open_in_memory().unwrap();
    for line in [
        "inbox thing",
        "overdue yesterday 2026-10-15",
        "laundry today",
        "later thing tomorrow",
        "piano someday",
        "project only #Home",
    ] {
        s.add_quick(&parse(line, today())).unwrap();
    }
    assert_eq!(titles(&s.list(&View::Inbox, today()).unwrap()), ["inbox thing"]);
    assert_eq!(titles(&s.list(&View::Today, today()).unwrap()), ["overdue yesterday", "laundry"]);
    assert_eq!(titles(&s.list(&View::Upcoming, today()).unwrap()), ["later thing"]);
    assert_eq!(titles(&s.list(&View::Someday, today()).unwrap()), ["piano"]);
    let home = s.project_by_name("Home").unwrap().unwrap();
    assert_eq!(titles(&s.list(&View::Project(home.id), today()).unwrap()), ["project only"]);
    let c = s.counts(today()).unwrap();
    assert_eq!((c.inbox, c.today, c.upcoming, c.someday), (1, 2, 1, 1));
}

#[test]
fn complete_and_reopen() {
    let s = Store::open_in_memory().unwrap();
    let task = s.add_quick(&parse("call bank today", today())).unwrap();
    assert!(s.complete(task.id, today()).unwrap().is_none());
    assert!(s.list(&View::Today, today()).unwrap().is_empty());
    assert_eq!(titles(&s.list(&View::Done, today()).unwrap()), ["call bank"]);
    s.reopen(task.id).unwrap();
    assert_eq!(s.task(task.id).unwrap().status, Status::Open);
}

#[test]
fn completing_a_recurring_task_spawns_the_next_one() {
    let s = Store::open_in_memory().unwrap();
    let task = s.add_quick(&parse("1:1 every monday 10am", today())).unwrap();
    assert_eq!(task.date, Some(d(2026, 10, 19)));
    let next = s.complete(task.id, today()).unwrap().expect("next occurrence");
    assert_eq!(next.date, Some(d(2026, 10, 26)));
    assert_eq!(next.time, Some(t(10, 0)));
    assert_eq!(next.rrule.as_deref(), Some("FREQ=WEEKLY;BYDAY=MO"));
    assert_eq!(s.task(task.id).unwrap().status, Status::Done);
    // daily task done late jumps to the next day after today, not after its old date
    let daily = s.add_quick(&parse("meds every day 2026-10-10", today())).unwrap();
    let next = s.complete(daily.id, today()).unwrap().unwrap();
    assert_eq!(next.date, Some(d(2026, 10, 17)));
}

#[test]
fn time_blocking() {
    let s = Store::open_in_memory().unwrap();
    let task = s.add_quick(&parse("write essay", today())).unwrap();
    s.schedule(task.id, today().and_time(t(14, 0)), 90).unwrap();
    let got = s.task(task.id).unwrap();
    assert_eq!((got.date, got.time, got.duration_min), (Some(today()), Some(t(14, 0)), 90));
    assert!(s.list(&View::Inbox, today()).unwrap().is_empty());
    assert_eq!(titles(&s.blocks_on(today()).unwrap()), ["write essay"]);
    s.unschedule(task.id).unwrap();
    let got = s.task(task.id).unwrap();
    assert_eq!((got.date, got.time), (Some(today()), None));
    assert!(s.blocks_on(today()).unwrap().is_empty());
    s.plan(task.id, None).unwrap();
    assert_eq!(titles(&s.list(&View::Inbox, today()).unwrap()), ["write essay"]);
}

#[test]
fn snooze_hides_until_the_day() {
    let s = Store::open_in_memory().unwrap();
    let task = s.add_quick(&parse("reply to Ana", today())).unwrap();
    s.snooze(task.id, d(2026, 10, 18)).unwrap();
    assert!(s.list(&View::Inbox, today()).unwrap().is_empty());
    assert_eq!(titles(&s.list(&View::Inbox, d(2026, 10, 18)).unwrap()), ["reply to Ana"]);
}

#[test]
fn update_patch_and_delete() {
    let s = Store::open_in_memory().unwrap();
    let task = s.add_quick(&parse("draft", today())).unwrap();
    s.update(task.id, &TaskPatch {
        title: Some("final draft".into()),
        notes: Some("see doc".into()),
        priority: Some(2),
        deadline: Some(Some(d(2026, 10, 30))),
        labels: Some(vec!["writing".into()]),
        ..Default::default()
    })
    .unwrap();
    let got = s.task(task.id).unwrap();
    assert_eq!(got.title, "final draft");
    assert_eq!(got.notes, "see doc");
    assert_eq!(got.priority, 2);
    assert_eq!(got.deadline, Some(d(2026, 10, 30)));
    assert_eq!(titles(&s.list(&View::Label("writing".into()), today()).unwrap()), ["final draft"]);
    s.delete(task.id).unwrap();
    assert!(s.task(task.id).is_err());
}

#[test]
fn subtasks_belong_to_a_parent() {
    let s = Store::open_in_memory().unwrap();
    let parent = s.add_quick(&parse("launch", today())).unwrap();
    let child = s.add_task(&NewTask { title: "write post".into(), parent_id: Some(parent.id), ..Default::default() }).unwrap();
    assert_eq!(titles(&s.subtasks(parent.id).unwrap()), ["write post"]);
    // subtasks are not listed on their own in views
    assert_eq!(titles(&s.list(&View::Inbox, today()).unwrap()), ["launch"]);
    s.delete(parent.id).unwrap();
    assert!(s.task(child.id).is_err());
}

#[test]
fn integrations_upsert_by_source_id() {
    let s = Store::open_in_memory().unwrap();
    let item = SourceItem {
        source: "github".into(),
        source_id: "Fs1lyric/calendi#4".into(),
        title: "Fix tunnel".into(),
        url: "https://github.com/Fs1lyric/calendi/issues/4".into(),
        notes: String::new(),
        deadline: None,
    };
    let (a, created) = s.upsert_source_item(&item).unwrap();
    assert!(created);
    let (b, created) = s.upsert_source_item(&SourceItem { title: "Fix the tunnel".into(), ..item.clone() }).unwrap();
    assert!(!created);
    assert_eq!(a.id, b.id);
    assert_eq!(b.title, "Fix the tunnel");
    assert_eq!(b.source_url.as_deref(), Some("https://github.com/Fs1lyric/calendi/issues/4"));
    assert_eq!(titles(&s.list(&View::Inbox, today()).unwrap()), ["Fix the tunnel"]);
    // items closed upstream get completed
    s.close_missing_source_items("github", &[]).unwrap();
    assert_eq!(s.task(a.id).unwrap().status, Status::Done);
}

#[test]
fn calendar_events_cache() {
    let s = Store::open_in_memory().unwrap();
    let ev = Event {
        id: "g1".into(),
        calendar: "primary".into(),
        title: "Dentist".into(),
        start: today().and_time(t(9, 0)),
        end: today().and_time(t(10, 0)),
        all_day: false,
        location: String::new(),
        url: String::new(),
    };
    s.upsert_event(&ev).unwrap();
    s.upsert_event(&Event { title: "Dentist (moved)".into(), ..ev.clone() }).unwrap();
    let got = s.events_on(today()).unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].title, "Dentist (moved)");
    assert!(s.events_on(d(2026, 10, 17)).unwrap().is_empty());
    s.prune_events("primary", today(), d(2026, 10, 20), &[]).unwrap();
    assert!(s.events_on(today()).unwrap().is_empty());
}

#[test]
fn settings_and_chat_history() {
    let s = Store::open_in_memory().unwrap();
    assert_eq!(s.setting("work_start").unwrap(), None);
    s.set_setting("work_start", "09:00").unwrap();
    assert_eq!(s.setting("work_start").unwrap().as_deref(), Some("09:00"));
    s.push_chat("user", r#""hi""#).unwrap();
    s.push_chat("assistant", r#"[{"type":"text","text":"hello"}]"#).unwrap();
    let h = s.chat_history().unwrap();
    assert_eq!(h.len(), 2);
    assert_eq!(h[1].role, "assistant");
    s.clear_chat().unwrap();
    assert!(s.chat_history().unwrap().is_empty());
}

#[test]
fn reopening_a_file_keeps_data() {
    let dir = std::env::temp_dir().join(format!("flowday-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("f.db");
    {
        let s = Store::open(&path).unwrap();
        s.add_quick(&parse("persist me", today())).unwrap();
    }
    let s = Store::open(&path).unwrap();
    assert_eq!(titles(&s.list(&View::Inbox, today()).unwrap()), ["persist me"]);
    std::fs::remove_dir_all(&dir).unwrap();
}
