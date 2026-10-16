//! Flowday's integrations. Task sources (GitHub, Linear, Notion, starred Gmail) pull
//! open items into the inbox and close the ones that went away; Google Calendar
//! pulls events and pushes time blocks back. Parsing is separate from HTTP so it
//! can be tested on recorded responses.

pub mod github;
pub mod google;
pub mod linear;
pub mod notion;

use chrono::NaiveDateTime;
use flowday_core::model::SourceItem;
use flowday_core::store::Store;

/// Everything the sync needs. A missing token turns that integration off.
#[derive(Debug, Clone, Default)]
pub struct Config {
    pub github_token: Option<String>,
    pub linear_key: Option<String>,
    pub notion_token: Option<String>,
    /// Notion databases to pull pages from, by id.
    pub notion_databases: Vec<String>,
    pub google: Option<google::Credentials>,
    pub gmail_starred: bool,
    pub push_blocks: bool,
}

/// Puts a source's open items in the store and closes the ones it no longer
/// reports. Returns (new, closed).
pub fn apply_items(store: &Store, source: &str, items: &[SourceItem]) -> Result<(usize, usize), String> {
    let mut added = 0;
    for item in items {
        let (_, new) = store.upsert_source_item(item).map_err(|e| e.to_string())?;
        added += usize::from(new);
    }
    let ids: Vec<String> = items.iter().map(|i| i.source_id.clone()).collect();
    let closed = store.close_missing_source_items(source, &ids).map_err(|e| e.to_string())?;
    Ok((added, closed))
}

fn summary((added, closed): (usize, usize)) -> String {
    match (added, closed) {
        (0, 0) => "up to date".into(),
        (a, 0) => format!("{a} new"),
        (0, c) => format!("{c} closed"),
        (a, c) => format!("{a} new, {c} closed"),
    }
}

/// Runs every configured integration once. Each reports on its own, so one
/// failing doesn't stop the rest.
pub fn run_all(store: &Store, cfg: &Config, now: NaiveDateTime) -> Vec<(&'static str, Result<String, String>)> {
    let mut out = Vec::new();
    if let Some(token) = &cfg.github_token {
        let r = github::pull(token).and_then(|items| apply_items(store, "github", &items)).map(summary);
        out.push(("GitHub", r));
    }
    if let Some(key) = &cfg.linear_key {
        let r = linear::pull(key).and_then(|items| apply_items(store, "linear", &items)).map(summary);
        out.push(("Linear", r));
    }
    if let Some(token) = &cfg.notion_token {
        if !cfg.notion_databases.is_empty() {
            let r = notion::pull(token, &cfg.notion_databases).and_then(|items| apply_items(store, "notion", &items)).map(summary);
            out.push(("Notion", r));
        }
    }
    if let Some(creds) = &cfg.google {
        match google::access_token(creds) {
            Ok(token) => {
                out.push(("Google Calendar", google::sync_calendar(store, &token, now, cfg.push_blocks)));
                if cfg.gmail_starred {
                    let r = google::pull_starred(&token).and_then(|items| apply_items(store, "gmail", &items)).map(summary);
                    out.push(("Gmail", r));
                }
            }
            Err(e) => out.push(("Google", Err(e))),
        }
    }
    out
}

/// Turns a failed HTTP call into a readable message.
pub(crate) fn http_error(service: &str, e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(401 | 403, _) => format!("{service} rejected the token; check it in Settings"),
        ureq::Error::Status(code, r) => {
            let body = r.into_string().unwrap_or_default();
            format!("{service} returned HTTP {code}: {}", body.chars().take(200).collect::<String>())
        }
        e => format!("can't reach {service}: {e}"),
    }
}

/// Percent-encodes a query or form value.
pub(crate) fn enc(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}
