use chrono::{NaiveDate, NaiveTime};
use flowday_core::model::{NewTask, SourceItem, View};
use flowday_core::store::Store;
use flowday_sync::{apply_items, github, google, linear, notion};
use serde_json::json;

#[test]
fn github_issues_and_prs_become_items() {
    let resp = json!({"items": [
        {"html_url": "https://github.com/acme/app/issues/12", "title": "Crash on start", "body": "trace"},
        {"html_url": "https://github.com/acme/app/pull/13", "title": "Fix crash", "pull_request": {}, "body": null}
    ]});
    let items = github::parse(&resp);
    assert_eq!(items[0].source_id, "acme/app#12");
    assert_eq!(items[0].title, "Crash on start");
    assert!(items[1].notes.starts_with("acme/app PR #13"));
}

#[test]
fn linear_issues_keep_their_identifier_and_due_date() {
    let resp = json!({"data": {"viewer": {"assignedIssues": {"nodes": [
        {"id": "abc", "identifier": "ENG-4", "title": "Ship it", "url": "https://linear.app/x/ENG-4", "dueDate": "2026-10-20", "description": null}
    ]}}}});
    let items = linear::parse(&resp);
    assert_eq!(items[0].title, "ENG-4 Ship it");
    assert_eq!(items[0].deadline, NaiveDate::from_ymd_opt(2026, 10, 20));
}

#[test]
fn notion_pages_skip_done_ones() {
    let page = |id: &str, title: &str, status: &str| {
        json!({"id": id, "url": "https://notion.so/p", "properties": {
            "Name": {"type": "title", "title": [{"plain_text": title}]},
            "Status": {"type": "status", "status": {"name": status}},
            "Due": {"type": "date", "date": {"start": "2026-10-21T10:00:00.000+00:00"}}
        }})
    };
    let resp = json!({"results": [page("1", "Write post", "In progress"), page("2", "Old", "Done")]});
    let items = notion::parse(&resp);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].deadline, NaiveDate::from_ymd_opt(2026, 10, 21));
    assert_eq!(notion::database_id("https://www.notion.so/me/Tasks-0123456789abcdef0123456789abcdef?v=1"), "0123456789abcdef0123456789abcdef");
}

#[test]
fn items_that_disappear_are_closed_and_known_ones_are_not_duplicated() {
    let store = Store::open_in_memory().unwrap();
    let item = |id: &str| SourceItem {
        source: "github".into(),
        source_id: id.into(),
        title: format!("issue {id}"),
        url: String::new(),
        notes: String::new(),
        deadline: None,
    };
    assert_eq!(apply_items(&store, "github", &[item("a"), item("b")]).unwrap(), (2, 0));
    assert_eq!(apply_items(&store, "github", &[item("a")]).unwrap(), (0, 1));
    let today = NaiveDate::from_ymd_opt(2026, 10, 16).unwrap();
    assert_eq!(store.list(&View::Inbox, today).unwrap().len(), 1);
}

#[test]
fn calendar_events_split_from_our_own_blocks() {
    let page = json!({"items": [
        {"id": "e1", "summary": "Standup", "start": {"dateTime": "2026-10-16T09:00:00Z"}, "end": {"dateTime": "2026-10-16T09:15:00Z"}},
        {"id": "e2", "summary": "Holiday", "start": {"date": "2026-10-17"}, "end": {"date": "2026-10-18"}},
        {"id": "e3", "status": "cancelled"},
        {"id": "b1", "summary": "Write", "start": {"dateTime": "2026-10-16T10:00:00Z"}, "end": {"dateTime": "2026-10-16T11:00:00Z"},
         "extendedProperties": {"private": {"flowday_task": "7"}}}
    ]});
    let (events, pushed) = google::parse_events(&page);
    assert_eq!(events.len(), 2);
    assert!(events[1].all_day);
    assert_eq!(pushed[0].task_id, 7);
    assert_eq!((pushed[0].end - pushed[0].start).num_minutes(), 60);
}

#[test]
fn block_diff_creates_updates_and_deletes() {
    let store = Store::open_in_memory().unwrap();
    let day = NaiveDate::from_ymd_opt(2026, 10, 16).unwrap();
    let at = |h| day.and_time(NaiveTime::from_hms_opt(h, 0, 0).unwrap());
    let mk = |title: &str, h| {
        let t = store.add_task(&NewTask { title: title.into(), date: Some(day), ..Default::default() }).unwrap();
        store.schedule(t.id, at(h), 60).unwrap()
    };
    let (same, moved, new) = (mk("same", 9), mk("moved", 11), mk("new", 14));
    let pushed = vec![
        google::Pushed { event_id: "p1".into(), task_id: same.id, title: "same".into(), start: at(9), end: at(10) },
        google::Pushed { event_id: "p2".into(), task_id: moved.id, title: "moved".into(), start: at(10), end: at(11) },
        google::Pushed { event_id: "p3".into(), task_id: 999, title: "gone".into(), start: at(8), end: at(9) },
    ];
    let blocks = vec![same, moved.clone(), new.clone()];
    let changes = google::diff(&blocks, &pushed);
    assert_eq!(changes, vec![google::Change::Update("p2", &moved), google::Change::Create(&new), google::Change::Delete("p3")]);
}

#[test]
fn oauth_redirect_gives_the_code() {
    assert_eq!(google::code_from_request("GET /?code=4%2F0Ab&scope=x HTTP/1.1").unwrap(), "4/0Ab");
    assert!(google::code_from_request("GET /?error=access_denied HTTP/1.1").is_err());
}

#[test]
fn starred_mail_uses_subject_and_thread() {
    let m = json!({"id": "m1", "threadId": "t1", "snippet": "hi", "payload": {"headers": [
        {"name": "Subject", "value": "Invoice"}, {"name": "From", "value": "a@b.c"}
    ]}});
    let item = google::parse_message(&m).unwrap();
    assert_eq!((item.title.as_str(), item.source_id.as_str()), ("Invoice", "t1"));
}
