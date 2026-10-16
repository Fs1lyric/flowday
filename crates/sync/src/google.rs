//! Google: OAuth for a desktop app (loopback redirect, your own Cloud client id),
//! Calendar events pulled into the store, Flowday's time blocks pushed back as
//! events tagged with the task id, and starred Gmail threads as inbox items.

use chrono::{DateTime, Days, Duration, Local, NaiveDate, NaiveDateTime, TimeZone};
use flowday_core::model::{Event, SourceItem, Task};
use flowday_core::store::Store;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;

pub const CALENDAR: &str = "google";
const SCOPES: &str = "https://www.googleapis.com/auth/calendar.events https://www.googleapis.com/auth/gmail.readonly";
const API: &str = "https://www.googleapis.com/calendar/v3/calendars/primary/events";

#[derive(Debug, Clone)]
pub struct Credentials {
    pub client_id: String,
    pub client_secret: String,
    pub refresh_token: String,
}

pub fn auth_url(client_id: &str, redirect: &str) -> String {
    format!(
        "https://accounts.google.com/o/oauth2/v2/auth?client_id={}&redirect_uri={}&response_type=code&scope={}&access_type=offline&prompt=consent",
        crate::enc(client_id),
        crate::enc(redirect),
        crate::enc(SCOPES)
    )
}

/// The `code` (or `error`) from the redirect's request line, `GET /?code=…&scope=… HTTP/1.1`.
pub fn code_from_request(line: &str) -> Result<String, String> {
    let path = line.split_whitespace().nth(1).unwrap_or_default();
    let query = path.split_once('?').map(|q| q.1).unwrap_or_default();
    let mut error = None;
    for pair in query.split('&') {
        match pair.split_once('=') {
            Some(("code", v)) => return Ok(decode(v)),
            Some(("error", v)) => error = Some(decode(v)),
            _ => {}
        }
    }
    Err(format!("Google sign-in failed: {}", error.unwrap_or_else(|| "no code in the redirect".into())))
}

fn decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => match std::str::from_utf8(&bytes[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                Some(b) => {
                    out.push(b);
                    i += 3;
                    continue;
                }
                None => out.push(b'%'),
            },
            b'+' => out.push(b' '),
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn token_request(form: &[(&str, &str)]) -> Result<Value, String> {
    ureq::post("https://oauth2.googleapis.com/token")
        .send_form(form)
        .map_err(|e| crate::http_error("Google", e))?
        .into_json()
        .map_err(|e| e.to_string())
}

/// Opens the browser for consent, waits (up to five minutes) for the redirect to
/// a one-shot local server, and returns the refresh token.
pub fn authorize(client_id: &str, client_secret: &str) -> Result<String, String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let redirect = format!("http://127.0.0.1:{}", listener.local_addr().map_err(|e| e.to_string())?.port());
    let url = auth_url(client_id, &redirect);
    if std::process::Command::new("xdg-open").arg(&url).spawn().is_err() {
        return Err(format!("can't open a browser; visit {url}"));
    }
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(300);
    let (stream, _) = loop {
        match listener.accept() {
            Ok(s) => break s,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if std::time::Instant::now() > deadline {
                    return Err("timed out waiting for Google sign-in".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            Err(e) => return Err(e.to_string()),
        }
    };
    stream.set_nonblocking(false).map_err(|e| e.to_string())?;
    let mut line = String::new();
    BufReader::new(&stream).read_line(&mut line).map_err(|e| e.to_string())?;
    let code = code_from_request(&line);
    let page = if code.is_ok() { "Flowday is connected to Google. You can close this tab." } else { "Google sign-in failed. Go back to Flowday." };
    let _ = (&stream).write_all(
        format!("HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}", page.len()).as_bytes(),
    );
    let code = code?;
    let resp = token_request(&[
        ("code", &code),
        ("client_id", client_id),
        ("client_secret", client_secret),
        ("redirect_uri", &redirect),
        ("grant_type", "authorization_code"),
    ])?;
    resp["refresh_token"].as_str().map(str::to_string).ok_or_else(|| "Google didn't return a refresh token".into())
}

pub fn access_token(c: &Credentials) -> Result<String, String> {
    let resp = token_request(&[
        ("client_id", &c.client_id),
        ("client_secret", &c.client_secret),
        ("refresh_token", &c.refresh_token),
        ("grant_type", "refresh_token"),
    ])?;
    resp["access_token"].as_str().map(str::to_string).ok_or_else(|| "Google didn't return an access token; reconnect in Settings".into())
}

fn local(s: &str) -> Option<NaiveDateTime> {
    Some(DateTime::parse_from_rfc3339(s).ok()?.with_timezone(&Local).naive_local())
}

fn rfc3339(t: NaiveDateTime) -> String {
    match Local.from_local_datetime(&t).earliest() {
        Some(dt) => dt.to_rfc3339(),
        None => (t + Duration::hours(1)).and_local_timezone(Local).earliest().map(|d| d.to_rfc3339()).unwrap_or_default(),
    }
}

/// A Flowday block found on Google: which task it shows, and when.
#[derive(Debug, Clone, PartialEq)]
pub struct Pushed {
    pub event_id: String,
    pub task_id: i64,
    pub title: String,
    pub start: NaiveDateTime,
    pub end: NaiveDateTime,
}

/// Splits an events page into ordinary events and Flowday's own blocks.
pub fn parse_events(page: &Value) -> (Vec<Event>, Vec<Pushed>) {
    let mut events = Vec::new();
    let mut pushed = Vec::new();
    for e in page["items"].as_array().into_iter().flatten() {
        if e["status"] == "cancelled" {
            continue;
        }
        let Some(id) = e["id"].as_str() else { continue };
        let title = e["summary"].as_str().unwrap_or("(busy)").to_string();
        let (start, end, all_day) = match (e["start"]["dateTime"].as_str(), e["start"]["date"].as_str()) {
            (Some(s), _) => match (local(s), e["end"]["dateTime"].as_str().and_then(local)) {
                (Some(a), Some(b)) => (a, b, false),
                _ => continue,
            },
            (None, Some(d)) => {
                let a = NaiveDate::parse_from_str(d, "%Y-%m-%d").ok();
                let b = e["end"]["date"].as_str().and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok());
                match (a, b) {
                    (Some(a), Some(b)) => (a.and_hms_opt(0, 0, 0).unwrap(), b.and_hms_opt(0, 0, 0).unwrap(), true),
                    _ => continue,
                }
            }
            _ => continue,
        };
        if let Some(task) = e["extendedProperties"]["private"]["flowday_task"].as_str().and_then(|s| s.parse().ok()) {
            pushed.push(Pushed { event_id: id.into(), task_id: task, title, start, end });
            continue;
        }
        events.push(Event {
            id: id.into(),
            calendar: CALENDAR.into(),
            title,
            start,
            end,
            all_day,
            location: e["location"].as_str().unwrap_or_default().into(),
            url: e["htmlLink"].as_str().unwrap_or_default().into(),
        });
    }
    (events, pushed)
}

/// What to do on Google so it matches the blocks: create, update, delete.
#[derive(Debug, PartialEq)]
pub enum Change<'a> {
    Create(&'a Task),
    Update(&'a str, &'a Task),
    Delete(&'a str),
}

pub fn diff<'a>(blocks: &'a [Task], pushed: &'a [Pushed]) -> Vec<Change<'a>> {
    let span = |t: &Task| t.start().map(|s| (s, s + Duration::minutes(i64::from(t.duration_min))));
    let mut out = Vec::new();
    for t in blocks {
        match pushed.iter().find(|p| p.task_id == t.id) {
            None => out.push(Change::Create(t)),
            Some(p) if span(t) != Some((p.start, p.end)) || p.title != t.title => out.push(Change::Update(&p.event_id, t)),
            Some(_) => {}
        }
    }
    for p in pushed {
        // A block can only appear once; extra copies go too.
        let first = pushed.iter().find(|q| q.task_id == p.task_id).map(|q| &q.event_id) == Some(&p.event_id);
        if !blocks.iter().any(|t| t.id == p.task_id) || !first {
            out.push(Change::Delete(&p.event_id));
        }
    }
    out
}

fn event_body(t: &Task) -> Value {
    let start = t.start().unwrap();
    let end = start + Duration::minutes(i64::from(t.duration_min));
    json!({
        "summary": t.title,
        "description": "Time block from Flowday",
        "start": {"dateTime": rfc3339(start)},
        "end": {"dateTime": rfc3339(end)},
        "transparency": "opaque",
        "extendedProperties": {"private": {"flowday_task": t.id.to_string()}},
    })
}

fn get(token: &str, url: &str) -> Result<Value, String> {
    ureq::get(url)
        .set("Authorization", &format!("Bearer {token}"))
        .call()
        .map_err(|e| crate::http_error("Google", e))?
        .into_json()
        .map_err(|e| e.to_string())
}

/// Pulls a week back to a month ahead, and (with `push`) mirrors the next two
/// weeks of blocks. Returns a one-line report.
pub fn sync_calendar(store: &Store, token: &str, now: NaiveDateTime, push: bool) -> Result<String, String> {
    let today = now.date();
    let (from, to) = (today - Days::new(7), today + Days::new(30));
    let mut events = Vec::new();
    let mut pushed = Vec::new();
    let mut page_token = String::new();
    loop {
        let mut url = format!(
            "{API}?singleEvents=true&orderBy=startTime&maxResults=2500&timeMin={}&timeMax={}",
            crate::enc(&rfc3339(from.and_hms_opt(0, 0, 0).unwrap())),
            crate::enc(&rfc3339((to + Days::new(1)).and_hms_opt(0, 0, 0).unwrap()))
        );
        if !page_token.is_empty() {
            url.push_str(&format!("&pageToken={}", crate::enc(&page_token)));
        }
        let page = get(token, &url)?;
        let (e, p) = parse_events(&page);
        events.extend(e);
        pushed.extend(p);
        match page["nextPageToken"].as_str() {
            Some(t) => page_token = t.to_string(),
            None => break,
        }
    }
    for e in &events {
        store.upsert_event(e).map_err(|e| e.to_string())?;
    }
    let ids: Vec<String> = events.iter().map(|e| e.id.clone()).collect();
    store.prune_events(CALENDAR, from, to, &ids).map_err(|e| e.to_string())?;
    let mut report = format!("{} events", events.len());

    if push {
        let horizon = today + Days::new(14);
        let blocks: Vec<Task> = store.tasks_between(today, horizon).map_err(|e| e.to_string())?.into_iter().filter(|t| t.time.is_some()).collect();
        let pushed: Vec<Pushed> = pushed.into_iter().filter(|p| p.start.date() >= today && p.start.date() <= horizon).collect();
        let auth = format!("Bearer {token}");
        let changes = diff(&blocks, &pushed);
        for c in &changes {
            let r = match c {
                Change::Create(t) => ureq::post(API).set("Authorization", &auth).send_json(event_body(t)).map(drop),
                Change::Update(id, t) => {
                    ureq::request("PATCH", &format!("{API}/{id}")).set("Authorization", &auth).send_json(event_body(t)).map(drop)
                }
                Change::Delete(id) => match ureq::delete(&format!("{API}/{id}")).set("Authorization", &auth).call() {
                    Err(ureq::Error::Status(404 | 410, _)) => Ok(()),
                    r => r.map(drop),
                },
            };
            r.map_err(|e| crate::http_error("Google Calendar", e))?;
        }
        if !changes.is_empty() {
            report.push_str(&format!(", {} block change(s) pushed", changes.len()));
        }
    }
    Ok(report)
}

/// The subject and sender of one message's metadata.
pub fn parse_message(m: &Value) -> Option<SourceItem> {
    let header = |name: &str| {
        m["payload"]["headers"].as_array()?.iter().find(|h| h["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case(name)))?["value"]
            .as_str()
            .map(str::to_string)
    };
    let thread = m["threadId"].as_str()?;
    let subject = header("Subject").filter(|s| !s.trim().is_empty()).unwrap_or_else(|| "(no subject)".into());
    let from = header("From").unwrap_or_default();
    Some(SourceItem {
        source: "gmail".into(),
        source_id: thread.to_string(),
        title: subject,
        url: format!("https://mail.google.com/mail/u/0/#all/{thread}"),
        notes: format!("From {from}\n\n{}", m["snippet"].as_str().unwrap_or_default()),
        deadline: None,
    })
}

pub fn pull_starred(token: &str) -> Result<Vec<SourceItem>, String> {
    let base = "https://gmail.googleapis.com/gmail/v1/users/me/messages";
    let list = get(token, &format!("{base}?q=is%3Astarred&maxResults=50"))?;
    let mut items: Vec<SourceItem> = Vec::new();
    for m in list["messages"].as_array().into_iter().flatten() {
        let Some(id) = m["id"].as_str() else { continue };
        let full = get(token, &format!("{base}/{id}?format=metadata&metadataHeaders=Subject&metadataHeaders=From"))?;
        if let Some(item) = parse_message(&full) {
            if !items.iter().any(|i| i.source_id == item.source_id) {
                items.push(item);
            }
        }
    }
    Ok(items)
}
