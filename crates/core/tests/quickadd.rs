use chrono::{NaiveDate, NaiveTime};
use flowday_core::quickadd::parse;

fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, 16).unwrap() // a Friday
}
fn t(h: u32, m: u32) -> NaiveTime {
    NaiveTime::from_hms_opt(h, m, 0).unwrap()
}

#[test]
fn plain_title_goes_to_inbox() {
    let q = parse("Buy milk", today());
    assert_eq!(q.title, "Buy milk");
    assert_eq!(q.date, None);
    assert_eq!(q.time, None);
    assert_eq!(q.priority, 0);
}

#[test]
fn today_and_tomorrow() {
    assert_eq!(parse("Email Sam today", today()).date, Some(today()));
    let q = parse("Email Sam tomorrow", today());
    assert_eq!(q.title, "Email Sam");
    assert_eq!(q.date, NaiveDate::from_ymd_opt(2026, 10, 17));
    assert_eq!(parse("tmr gym", today()).date, NaiveDate::from_ymd_opt(2026, 10, 17));
}

#[test]
fn weekday_means_the_next_one() {
    // Friday -> "monday" is the 19th, "friday" is next week's, not today
    assert_eq!(parse("standup monday", today()).date, NaiveDate::from_ymd_opt(2026, 10, 19));
    assert_eq!(parse("review fri", today()).date, NaiveDate::from_ymd_opt(2026, 10, 23));
    assert_eq!(parse("plan next week", today()).date, NaiveDate::from_ymd_opt(2026, 10, 19));
}

#[test]
fn explicit_dates() {
    assert_eq!(parse("rent 2026-11-01", today()).date, NaiveDate::from_ymd_opt(2026, 11, 1));
    assert_eq!(parse("party nov 3", today()).date, NaiveDate::from_ymd_opt(2026, 11, 3));
    assert_eq!(parse("party 3 nov", today()).date, NaiveDate::from_ymd_opt(2026, 11, 3));
    // a month/day already past rolls to next year
    assert_eq!(parse("taxes jan 5", today()).date, NaiveDate::from_ymd_opt(2027, 1, 5));
    assert_eq!(parse("trip in 3 days", today()).date, NaiveDate::from_ymd_opt(2026, 10, 19));
}

#[test]
fn times_imply_today_when_no_date() {
    let q = parse("Call mom 5pm", today());
    assert_eq!(q.title, "Call mom");
    assert_eq!(q.time, Some(t(17, 0)));
    assert_eq!(q.date, Some(today()));
    assert_eq!(parse("sync at 9:30", today()).time, Some(t(9, 30)));
    assert_eq!(parse("sync 14:05 tomorrow", today()).time, Some(t(14, 5)));
    assert_eq!(parse("lunch 12:30pm", today()).time, Some(t(12, 30)));
    assert_eq!(parse("late 12am", today()).time, Some(t(0, 0)));
}

#[test]
fn durations() {
    assert_eq!(parse("deep work 2h", today()).duration_min, Some(120));
    assert_eq!(parse("deep work for 90m", today()).duration_min, Some(90));
    assert_eq!(parse("walk 1h30m", today()).duration_min, Some(90));
    assert_eq!(parse("walk 45min", today()).title, "walk");
}

#[test]
fn project_label_priority() {
    let q = parse("Ship release #Work @urgent @deep !1 tomorrow 10am 1h", today());
    assert_eq!(q.title, "Ship release");
    assert_eq!(q.project.as_deref(), Some("Work"));
    assert_eq!(q.labels, vec!["urgent".to_string(), "deep".to_string()]);
    assert_eq!(q.priority, 3); // !1 is the highest -> priority 3
    assert_eq!(q.date, NaiveDate::from_ymd_opt(2026, 10, 17));
    assert_eq!(q.time, Some(t(10, 0)));
    assert_eq!(q.duration_min, Some(60));
    assert_eq!(parse("x !3", today()).priority, 1);
}

#[test]
fn someday_and_words_that_look_like_tokens() {
    assert!(parse("learn piano someday", today()).someday);
    // "at" and "for" stay when nothing follows them
    assert_eq!(parse("look at it", today()).title, "look at it");
    assert_eq!(parse("pay for lunch", today()).title, "pay for lunch");
    // an escaped word is kept literally
    assert_eq!(parse("watch 'tomorrow' film", today()).title, "watch tomorrow film");
}

#[test]
fn recurrence() {
    assert_eq!(parse("water plants every day", today()).rrule.as_deref(), Some("FREQ=DAILY"));
    assert_eq!(parse("1:1 every monday", today()).rrule.as_deref(), Some("FREQ=WEEKLY;BYDAY=MO"));
    assert_eq!(parse("1:1 every monday", today()).date, NaiveDate::from_ymd_opt(2026, 10, 19));
    assert_eq!(parse("review every weekday", today()).rrule.as_deref(), Some("FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR"));
    assert_eq!(parse("bills every month", today()).rrule.as_deref(), Some("FREQ=MONTHLY"));
}
