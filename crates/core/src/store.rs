//! The SQLite store. One file, opened by the app; the AI tools and the sync engine
//! both go through this API rather than writing SQL of their own.

use std::path::Path;

use chrono::{Days, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use rusqlite::{params, params_from_iter, Connection, OptionalExtension, Row};

use crate::model::*;
use crate::quickadd::QuickAdd;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Sql(#[from] rusqlite::Error),
    #[error("no task with id {0}")]
    NoTask(i64),
    #[error("bad recurrence rule {rule:?}: {reason}")]
    BadRule { rule: String, reason: String },
}

pub type Result<T> = std::result::Result<T, Error>;

const MIGRATIONS: &[&str] = &[r#"
CREATE TABLE projects (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE COLLATE NOCASE,
    color TEXT NOT NULL DEFAULT '#8b7cf6'
);
CREATE TABLE tasks (
    id INTEGER PRIMARY KEY,
    title TEXT NOT NULL,
    notes TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'open',
    project_id INTEGER REFERENCES projects(id) ON DELETE SET NULL,
    priority INTEGER NOT NULL DEFAULT 0,
    date TEXT,
    time TEXT,
    duration_min INTEGER NOT NULL DEFAULT 30,
    deadline TEXT,
    someday INTEGER NOT NULL DEFAULT 0,
    snoozed_until TEXT,
    rrule TEXT,
    parent_id INTEGER REFERENCES tasks(id) ON DELETE CASCADE,
    source TEXT,
    source_id TEXT,
    source_url TEXT,
    sort REAL NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    done_at INTEGER,
    UNIQUE (source, source_id)
);
CREATE INDEX tasks_date ON tasks(status, date);
CREATE TABLE labels (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE COLLATE NOCASE
);
CREATE TABLE task_labels (
    task_id INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    label_id INTEGER NOT NULL REFERENCES labels(id) ON DELETE CASCADE,
    PRIMARY KEY (task_id, label_id)
);
CREATE TABLE events (
    calendar TEXT NOT NULL,
    id TEXT NOT NULL,
    title TEXT NOT NULL,
    start TEXT NOT NULL,
    end TEXT NOT NULL,
    all_day INTEGER NOT NULL DEFAULT 0,
    location TEXT NOT NULL DEFAULT '',
    url TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (calendar, id)
);
CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE chat (id INTEGER PRIMARY KEY, role TEXT NOT NULL, content TEXT NOT NULL);
"#];

const TASK_COLS: &str = "t.id, t.title, t.notes, t.status, t.project_id, t.priority, t.date, t.time, \
    t.duration_min, t.deadline, t.someday, t.snoozed_until, t.rrule, t.parent_id, t.source, t.source_id, \
    t.source_url, t.sort, t.created_at, t.done_at, \
    (SELECT group_concat(l.name, char(31)) FROM task_labels tl JOIN labels l ON l.id = tl.label_id \
     WHERE tl.task_id = t.id)";

const ORDER: &str = "ORDER BY t.date IS NULL, t.date, t.time IS NULL, t.time, t.priority DESC, t.sort, t.id";

fn day(s: Option<String>) -> Option<NaiveDate> {
    s.and_then(|s| NaiveDate::parse_from_str(&s, "%Y-%m-%d").ok())
}
fn clock(s: Option<String>) -> Option<NaiveTime> {
    s.and_then(|s| NaiveTime::parse_from_str(&s, "%H:%M").ok())
}
fn day_str(d: Option<NaiveDate>) -> Option<String> {
    d.map(|d| d.format("%Y-%m-%d").to_string())
}
fn clock_str(t: Option<NaiveTime>) -> Option<String> {
    t.map(|t| t.format("%H:%M").to_string())
}
fn stamp(dt: NaiveDateTime) -> String {
    dt.format("%Y-%m-%dT%H:%M").to_string()
}

fn task_row(r: &Row) -> rusqlite::Result<Task> {
    let labels: Option<String> = r.get(20)?;
    Ok(Task {
        id: r.get(0)?,
        title: r.get(1)?,
        notes: r.get(2)?,
        status: if r.get::<_, String>(3)? == "done" { Status::Done } else { Status::Open },
        project_id: r.get(4)?,
        priority: r.get(5)?,
        date: day(r.get(6)?),
        time: clock(r.get(7)?),
        duration_min: r.get(8)?,
        deadline: day(r.get(9)?),
        someday: r.get(10)?,
        snoozed_until: day(r.get(11)?),
        rrule: r.get(12)?,
        parent_id: r.get(13)?,
        source: r.get(14)?,
        source_id: r.get(15)?,
        source_url: r.get(16)?,
        sort: r.get(17)?,
        created_at: r.get(18)?,
        done_at: r.get(19)?,
        labels: labels
            .map(|l| l.split('\u{1f}').map(str::to_string).collect())
            .unwrap_or_default(),
    })
}

fn event_row(r: &Row) -> rusqlite::Result<Event> {
    let parse = |s: String| NaiveDateTime::parse_from_str(&s, "%Y-%m-%dT%H:%M").unwrap_or_default();
    Ok(Event {
        calendar: r.get(0)?,
        id: r.get(1)?,
        title: r.get(2)?,
        start: parse(r.get(3)?),
        end: parse(r.get(4)?),
        all_day: r.get(5)?,
        location: r.get(6)?,
        url: r.get(7)?,
    })
}

/// The next date of `rule` strictly after `after`, for a series that started on `start`.
pub fn next_occurrence(rule: &str, start: NaiveDate, after: NaiveDate) -> Result<Option<NaiveDate>> {
    let text = format!("DTSTART:{}T000000Z\nRRULE:{rule}", start.format("%Y%m%d"));
    let set: rrule::RRuleSet = text.parse().map_err(|e: rrule::RRuleError| Error::BadRule {
        rule: rule.to_string(),
        reason: e.to_string(),
    })?;
    let noon = after.and_hms_opt(12, 0, 0).unwrap().and_utc().with_timezone(&rrule::Tz::UTC);
    Ok(set.after(noon).all(1).dates.first().map(|d| d.date_naive()))
}

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "wal")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "foreign_keys", true)?;
        let version: usize = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(version) {
            let tx = conn.transaction()?;
            tx.execute_batch(sql)?;
            tx.pragma_update(None, "user_version", i as i64 + 1)?;
            tx.commit()?;
        }
        Ok(Self { conn })
    }

    // ---- projects and labels ----

    pub fn projects(&self) -> Result<Vec<Project>> {
        let mut st = self.conn.prepare("SELECT id, name, color FROM projects ORDER BY name COLLATE NOCASE")?;
        let rows = st.query_map([], |r| Ok(Project { id: r.get(0)?, name: r.get(1)?, color: r.get(2)? }))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn project_by_name(&self, name: &str) -> Result<Option<Project>> {
        Ok(self
            .conn
            .query_row("SELECT id, name, color FROM projects WHERE name = ?1", [name], |r| {
                Ok(Project { id: r.get(0)?, name: r.get(1)?, color: r.get(2)? })
            })
            .optional()?)
    }

    pub fn create_project(&self, name: &str, color: &str) -> Result<Project> {
        self.conn.execute("INSERT INTO projects (name, color) VALUES (?1, ?2)", params![name, color])?;
        Ok(Project { id: self.conn.last_insert_rowid(), name: name.to_string(), color: color.to_string() })
    }

    /// The project with this name, created with a palette colour if it does not exist.
    pub fn ensure_project(&self, name: &str) -> Result<Project> {
        if let Some(p) = self.project_by_name(name)? {
            return Ok(p);
        }
        const PALETTE: [&str; 8] = ["#8b7cf6", "#f472b6", "#38bdf8", "#34d399", "#fbbf24", "#fb7185", "#a3e635", "#60a5fa"];
        let n: usize = self.conn.query_row("SELECT count(*) FROM projects", [], |r| r.get(0))?;
        self.create_project(name, PALETTE[n % PALETTE.len()])
    }

    pub fn rename_project(&self, id: i64, name: &str, color: &str) -> Result<()> {
        self.conn.execute("UPDATE projects SET name = ?2, color = ?3 WHERE id = ?1", params![id, name, color])?;
        Ok(())
    }

    pub fn delete_project(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM projects WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn labels(&self) -> Result<Vec<String>> {
        let mut st = self.conn.prepare(
            "SELECT name FROM labels WHERE id IN (SELECT label_id FROM task_labels) ORDER BY name COLLATE NOCASE",
        )?;
        let rows = st.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn set_labels(&self, task: i64, labels: &[String]) -> Result<()> {
        self.conn.execute("DELETE FROM task_labels WHERE task_id = ?1", [task])?;
        for name in labels {
            self.conn.execute("INSERT OR IGNORE INTO labels (name) VALUES (?1)", [name])?;
            self.conn.execute(
                "INSERT OR IGNORE INTO task_labels (task_id, label_id) SELECT ?1, id FROM labels WHERE name = ?2",
                params![task, name],
            )?;
        }
        Ok(())
    }

    // ---- tasks ----

    pub fn task(&self, id: i64) -> Result<Task> {
        self.conn
            .query_row(&format!("SELECT {TASK_COLS} FROM tasks t WHERE t.id = ?1"), [id], task_row)
            .optional()?
            .ok_or(Error::NoTask(id))
    }

    pub fn add_task(&self, n: &NewTask) -> Result<Task> {
        let now = Utc::now().timestamp();
        self.conn.execute(
            "INSERT INTO tasks (title, notes, project_id, priority, date, time, duration_min, deadline, someday,
                                rrule, parent_id, created_at, sort)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                     (SELECT coalesce(max(sort), 0) + 1 FROM tasks))",
            params![
                n.title.trim(),
                n.notes,
                n.project_id,
                n.priority.min(3),
                day_str(n.date),
                clock_str(n.time),
                n.duration_min.unwrap_or(DEFAULT_DURATION_MIN),
                day_str(n.deadline),
                n.someday,
                n.rrule,
                n.parent_id,
                now,
            ],
        )?;
        let id = self.conn.last_insert_rowid();
        self.set_labels(id, &n.labels)?;
        self.task(id)
    }

    pub fn add_quick(&self, q: &QuickAdd) -> Result<Task> {
        let project_id = match &q.project {
            Some(name) => Some(self.ensure_project(name)?.id),
            None => None,
        };
        self.add_task(&NewTask {
            title: q.title.clone(),
            project_id,
            priority: q.priority,
            date: q.date,
            time: q.time,
            duration_min: q.duration_min,
            someday: q.someday,
            rrule: q.rrule.clone(),
            labels: q.labels.clone(),
            ..Default::default()
        })
    }

    pub fn update(&self, id: i64, p: &TaskPatch) -> Result<Task> {
        let old = self.task(id)?;
        let date = p.date.unwrap_or(old.date);
        // a task can't keep a time without a day
        let time = if date.is_none() { None } else { p.time.unwrap_or(old.time) };
        self.conn.execute(
            "UPDATE tasks SET title = ?2, notes = ?3, project_id = ?4, priority = ?5, date = ?6, time = ?7,
                    duration_min = ?8, deadline = ?9, someday = ?10, rrule = ?11 WHERE id = ?1",
            params![
                id,
                p.title.as_deref().unwrap_or(&old.title).trim(),
                p.notes.as_deref().unwrap_or(&old.notes),
                p.project_id.unwrap_or(old.project_id),
                p.priority.unwrap_or(old.priority).min(3),
                day_str(date),
                clock_str(time),
                p.duration_min.unwrap_or(old.duration_min).max(5),
                day_str(p.deadline.unwrap_or(old.deadline)),
                if date.is_some() { false } else { p.someday.unwrap_or(old.someday) },
                p.rrule.clone().unwrap_or(old.rrule),
            ],
        )?;
        if let Some(labels) = &p.labels {
            self.set_labels(id, labels)?;
        }
        self.task(id)
    }

    pub fn delete(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM tasks WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Plan a task for a day (or back to unplanned with `None`), dropping any time block.
    pub fn plan(&self, id: i64, date: Option<NaiveDate>) -> Result<Task> {
        self.update(id, &TaskPatch { date: Some(date), time: Some(None), someday: Some(false), ..Default::default() })
    }

    /// Time-block a task on the calendar.
    pub fn schedule(&self, id: i64, start: NaiveDateTime, duration_min: u32) -> Result<Task> {
        self.update(id, &TaskPatch {
            date: Some(Some(start.date())),
            time: Some(Some(start.time())),
            duration_min: Some(duration_min),
            someday: Some(false),
            ..Default::default()
        })
    }

    /// Take a task off the calendar but keep it planned for its day.
    pub fn unschedule(&self, id: i64) -> Result<Task> {
        self.update(id, &TaskPatch { time: Some(None), ..Default::default() })
    }

    pub fn snooze(&self, id: i64, until: NaiveDate) -> Result<()> {
        self.conn.execute("UPDATE tasks SET snoozed_until = ?2 WHERE id = ?1", params![id, day_str(Some(until))])?;
        Ok(())
    }

    pub fn reorder(&self, id: i64, sort: f64) -> Result<()> {
        self.conn.execute("UPDATE tasks SET sort = ?2 WHERE id = ?1", params![id, sort])?;
        Ok(())
    }

    /// Mark a task done. A recurring task gets its next occurrence created, which is returned.
    pub fn complete(&self, id: i64, today: NaiveDate) -> Result<Option<Task>> {
        let task = self.task(id)?;
        self.conn.execute(
            "UPDATE tasks SET status = 'done', done_at = ?2 WHERE id = ?1",
            params![id, Utc::now().timestamp()],
        )?;
        let (Some(rule), Some(date)) = (&task.rrule, task.date) else { return Ok(None) };
        let Some(next) = next_occurrence(rule, date, date.max(today))? else { return Ok(None) };
        let copy = self.add_task(&NewTask {
            title: task.title.clone(),
            notes: task.notes.clone(),
            project_id: task.project_id,
            priority: task.priority,
            date: Some(next),
            time: task.time,
            duration_min: Some(task.duration_min),
            rrule: task.rrule.clone(),
            labels: task.labels.clone(),
            ..Default::default()
        })?;
        Ok(Some(copy))
    }

    pub fn reopen(&self, id: i64) -> Result<()> {
        self.conn.execute("UPDATE tasks SET status = 'open', done_at = NULL WHERE id = ?1", [id])?;
        Ok(())
    }

    fn query_tasks(&self, filter: &str, args: Vec<rusqlite::types::Value>, order: &str) -> Result<Vec<Task>> {
        let sql = format!("SELECT {TASK_COLS} FROM tasks t WHERE {filter} {order}");
        let mut st = self.conn.prepare(&sql)?;
        let rows = st.query_map(params_from_iter(args), task_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn view_filter(view: &View, today: NaiveDate) -> (String, Vec<rusqlite::types::Value>) {
        use rusqlite::types::Value::{Integer, Text};
        let t = today.format("%Y-%m-%d").to_string();
        let open = "t.status = 'open' AND t.parent_id IS NULL";
        match view {
            View::Inbox => (
                format!("{open} AND t.date IS NULL AND t.project_id IS NULL AND t.someday = 0 \
                         AND (t.snoozed_until IS NULL OR t.snoozed_until <= ?1)"),
                vec![Text(t)],
            ),
            View::Today => (format!("{open} AND t.date <= ?1"), vec![Text(t)]),
            View::Upcoming => (format!("{open} AND t.date > ?1"), vec![Text(t)]),
            View::Someday => (format!("{open} AND t.someday = 1"), vec![]),
            View::Project(id) => (format!("{open} AND t.project_id = ?1"), vec![Integer(*id)]),
            View::Label(name) => (
                format!("{open} AND t.id IN (SELECT tl.task_id FROM task_labels tl JOIN labels l \
                         ON l.id = tl.label_id WHERE l.name = ?1)"),
                vec![Text(name.clone())],
            ),
            View::Done => ("t.status = 'done' AND t.parent_id IS NULL".into(), vec![]),
        }
    }

    pub fn list(&self, view: &View, today: NaiveDate) -> Result<Vec<Task>> {
        let (filter, args) = Self::view_filter(view, today);
        let order = if *view == View::Done { "ORDER BY t.done_at DESC, t.id DESC LIMIT 500" } else { ORDER };
        self.query_tasks(&filter, args, order)
    }

    pub fn counts(&self, today: NaiveDate) -> Result<Counts> {
        let count = |v: View| -> Result<usize> {
            let (filter, args) = Self::view_filter(&v, today);
            Ok(self.conn.query_row(
                &format!("SELECT count(*) FROM tasks t WHERE {filter}"),
                params_from_iter(args),
                |r| r.get(0),
            )?)
        };
        Ok(Counts {
            inbox: count(View::Inbox)?,
            today: count(View::Today)?,
            upcoming: count(View::Upcoming)?,
            someday: count(View::Someday)?,
        })
    }

    pub fn subtasks(&self, parent: i64) -> Result<Vec<Task>> {
        use rusqlite::types::Value::Integer;
        self.query_tasks("t.parent_id = ?1", vec![Integer(parent)], "ORDER BY t.sort, t.id")
    }

    /// Open tasks planned between two days inclusive (for the week view and the AI).
    pub fn tasks_between(&self, from: NaiveDate, to: NaiveDate) -> Result<Vec<Task>> {
        use rusqlite::types::Value::Text;
        self.query_tasks(
            "t.status = 'open' AND t.parent_id IS NULL AND t.date >= ?1 AND t.date <= ?2",
            vec![Text(from.to_string()), Text(to.to_string())],
            ORDER,
        )
    }

    /// Time-blocked tasks on a day, open or done, in start order.
    pub fn blocks_on(&self, date: NaiveDate) -> Result<Vec<Task>> {
        use rusqlite::types::Value::Text;
        self.query_tasks("t.date = ?1 AND t.time IS NOT NULL", vec![Text(date.to_string())], "ORDER BY t.time, t.id")
    }

    pub fn search(&self, text: &str) -> Result<Vec<Task>> {
        use rusqlite::types::Value::Text;
        let like = format!("%{}%", text.replace('%', "\\%").replace('_', "\\_"));
        self.query_tasks(
            "(t.title LIKE ?1 ESCAPE '\\' OR t.notes LIKE ?1 ESCAPE '\\')",
            vec![Text(like)],
            "ORDER BY t.status = 'done', t.id DESC LIMIT 50",
        )
    }

    // ---- integrations ----

    /// Insert or refresh a task an integration found. Returns the task and whether it is new.
    pub fn upsert_source_item(&self, item: &SourceItem) -> Result<(Task, bool)> {
        let existing: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM tasks WHERE source = ?1 AND source_id = ?2",
                params![item.source, item.source_id],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(id) = existing {
            self.conn.execute(
                "UPDATE tasks SET title = ?2, source_url = ?3, deadline = coalesce(?4, deadline) WHERE id = ?1",
                params![id, item.title, item.url, day_str(item.deadline)],
            )?;
            return Ok((self.task(id)?, false));
        }
        let task = self.add_task(&NewTask {
            title: item.title.clone(),
            notes: item.notes.clone(),
            deadline: item.deadline,
            ..Default::default()
        })?;
        self.conn.execute(
            "UPDATE tasks SET source = ?2, source_id = ?3, source_url = ?4 WHERE id = ?1",
            params![task.id, item.source, item.source_id, item.url],
        )?;
        Ok((self.task(task.id)?, true))
    }

    /// Complete the open tasks from `source` that its latest pull no longer reports.
    pub fn close_missing_source_items(&self, source: &str, still_open: &[String]) -> Result<usize> {
        let mut st = self.conn.prepare("SELECT id, source_id FROM tasks WHERE source = ?1 AND status = 'open'")?;
        let rows: Vec<(i64, String)> =
            st.query_map([source], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
        let now = Utc::now().timestamp();
        let mut closed = 0;
        for (id, sid) in rows {
            if !still_open.contains(&sid) {
                self.conn.execute("UPDATE tasks SET status = 'done', done_at = ?2 WHERE id = ?1", params![id, now])?;
                closed += 1;
            }
        }
        Ok(closed)
    }

    // ---- calendar events ----

    pub fn upsert_event(&self, e: &Event) -> Result<()> {
        self.conn.execute(
            "INSERT INTO events (calendar, id, title, start, end, all_day, location, url)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT (calendar, id) DO UPDATE SET title = ?3, start = ?4, end = ?5, all_day = ?6,
                 location = ?7, url = ?8",
            params![e.calendar, e.id, e.title, stamp(e.start), stamp(e.end), e.all_day, e.location, e.url],
        )?;
        Ok(())
    }

    /// Events overlapping the days `from..=to`.
    pub fn events_between(&self, from: NaiveDate, to: NaiveDate) -> Result<Vec<Event>> {
        let lo = stamp(from.and_hms_opt(0, 0, 0).unwrap());
        let hi = stamp((to + Days::new(1)).and_hms_opt(0, 0, 0).unwrap());
        let mut st = self.conn.prepare(
            "SELECT calendar, id, title, start, end, all_day, location, url FROM events
             WHERE start < ?2 AND (end > ?1 OR (all_day = 1 AND start >= ?1)) ORDER BY all_day DESC, start",
        )?;
        let rows = st.query_map(params![lo, hi], event_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn events_on(&self, date: NaiveDate) -> Result<Vec<Event>> {
        self.events_between(date, date)
    }

    /// Drop cached events of `calendar` in the window that the latest pull did not return.
    pub fn prune_events(&self, calendar: &str, from: NaiveDate, to: NaiveDate, keep: &[String]) -> Result<()> {
        for e in self.events_between(from, to)? {
            if e.calendar == calendar && !keep.contains(&e.id) {
                self.conn.execute("DELETE FROM events WHERE calendar = ?1 AND id = ?2", params![calendar, e.id])?;
            }
        }
        Ok(())
    }

    // ---- settings and chat ----

    pub fn setting(&self, key: &str) -> Result<Option<String>> {
        Ok(self.conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0)).optional()?)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO UPDATE SET value = ?2",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn push_chat(&self, role: &str, content_json: &str) -> Result<i64> {
        self.conn.execute("INSERT INTO chat (role, content) VALUES (?1, ?2)", params![role, content_json])?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn chat_history(&self) -> Result<Vec<ChatRow>> {
        let mut st = self.conn.prepare("SELECT id, role, content FROM chat ORDER BY id")?;
        let rows = st.query_map([], |r| Ok(ChatRow { id: r.get(0)?, role: r.get(1)?, content: r.get(2)? }))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn clear_chat(&self) -> Result<()> {
        self.conn.execute("DELETE FROM chat", [])?;
        Ok(())
    }
}
