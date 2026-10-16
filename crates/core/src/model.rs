//! The records Flowday keeps. Dates and times are local wall-clock values: this is a
//! one-person app, and the calendar sync converts to and from UTC at its edge.

use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Open,
    Done,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Task {
    pub id: i64,
    pub title: String,
    pub notes: String,
    pub status: Status,
    pub project_id: Option<i64>,
    /// 0 = none, 3 = highest.
    pub priority: u8,
    /// The day it is planned for.
    pub date: Option<NaiveDate>,
    /// Set when it is time-blocked on the calendar.
    pub time: Option<NaiveTime>,
    pub duration_min: u32,
    pub deadline: Option<NaiveDate>,
    pub someday: bool,
    pub snoozed_until: Option<NaiveDate>,
    pub rrule: Option<String>,
    pub parent_id: Option<i64>,
    pub labels: Vec<String>,
    /// Where an integration found it: "github", "linear", "notion", "gmail".
    pub source: Option<String>,
    pub source_id: Option<String>,
    pub source_url: Option<String>,
    pub sort: f64,
    pub created_at: i64,
    pub done_at: Option<i64>,
}

impl Task {
    pub fn start(&self) -> Option<NaiveDateTime> {
        Some(self.date?.and_time(self.time?))
    }
}

pub const DEFAULT_DURATION_MIN: u32 = 30;

#[derive(Debug, Clone, Default)]
pub struct NewTask {
    pub title: String,
    pub notes: String,
    pub project_id: Option<i64>,
    pub priority: u8,
    pub date: Option<NaiveDate>,
    pub time: Option<NaiveTime>,
    pub duration_min: Option<u32>,
    pub deadline: Option<NaiveDate>,
    pub someday: bool,
    pub rrule: Option<String>,
    pub parent_id: Option<i64>,
    pub labels: Vec<String>,
}

/// A partial update: `None` leaves a field alone; `Some(None)` clears an optional one.
#[derive(Debug, Clone, Default)]
pub struct TaskPatch {
    pub title: Option<String>,
    pub notes: Option<String>,
    pub project_id: Option<Option<i64>>,
    pub priority: Option<u8>,
    pub date: Option<Option<NaiveDate>>,
    pub time: Option<Option<NaiveTime>>,
    pub duration_min: Option<u32>,
    pub deadline: Option<Option<NaiveDate>>,
    pub someday: Option<bool>,
    pub rrule: Option<Option<String>>,
    pub labels: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Project {
    pub id: i64,
    pub name: String,
    pub color: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum View {
    Inbox,
    Today,
    Upcoming,
    Someday,
    Project(i64),
    Label(String),
    Done,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    pub inbox: usize,
    pub today: usize,
    pub upcoming: usize,
    pub someday: usize,
}

/// An open item an integration reports; it becomes (or refreshes) an inbox task.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceItem {
    pub source: String,
    pub source_id: String,
    pub title: String,
    pub url: String,
    pub notes: String,
    pub deadline: Option<NaiveDate>,
}

/// A calendar event cached from Google Calendar.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Event {
    pub id: String,
    pub calendar: String,
    pub title: String,
    pub start: NaiveDateTime,
    pub end: NaiveDateTime,
    pub all_day: bool,
    pub location: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChatRow {
    pub id: i64,
    pub role: String,
    /// The message content as Messages-API JSON (a string or an array of blocks).
    pub content: String,
}
