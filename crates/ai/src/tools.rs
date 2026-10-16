//! The tools Claude can call, and their implementations over the store. Every tool
//! returns JSON text; a failure becomes an `is_error` tool result.

use chrono::{Duration, NaiveDate, NaiveDateTime, NaiveTime};
use flowday_core::model::{NewTask, Task, TaskPatch, View};
use flowday_core::plan;
use flowday_core::schedule::{free_slots, Slot};
use flowday_core::store::Store;
use serde_json::{json, Value};

/// The tool definitions sent with every request. Keep this deterministic so the
/// prompt cache holds.
pub fn definitions() -> Value {
    let task_fields = json!({
        "title": {"type": "string"},
        "notes": {"type": "string"},
        "project": {"type": "string", "description": "Project name; created if new."},
        "priority": {"type": "integer", "minimum": 0, "maximum": 3, "description": "0 none, 3 highest"},
        "date": {"type": "string", "description": "Planned day, YYYY-MM-DD; empty string clears"},
        "time": {"type": "string", "description": "Start time HH:MM, which time-blocks it; empty string clears"},
        "duration_min": {"type": "integer", "minimum": 5},
        "deadline": {"type": "string", "description": "YYYY-MM-DD; empty string clears"},
        "labels": {"type": "array", "items": {"type": "string"}},
        "someday": {"type": "boolean"},
        "repeat": {"type": "string", "description": "An RFC 5545 RRULE such as FREQ=WEEKLY;BYDAY=MO; empty string clears"}
    });
    let mut create_props = task_fields.clone();
    create_props["parent_id"] = json!({"type": "integer", "description": "Make it a subtask of this task"});
    let mut update_props = task_fields;
    update_props["id"] = json!({"type": "integer"});
    json!([
        {
            "name": "list_tasks",
            "description": "List open tasks in a view. Today includes overdue tasks. Returns id, title, date, time, duration, priority, project, labels, deadline.",
            "input_schema": {"type": "object", "properties": {
                "view": {"type": "string", "enum": ["inbox", "today", "upcoming", "someday", "done"]},
                "project": {"type": "string", "description": "List a project instead of a view"}
            }}
        },
        {
            "name": "search_tasks",
            "description": "Find tasks whose title or notes contain the text.",
            "input_schema": {"type": "object", "properties": {"query": {"type": "string"}}, "required": ["query"]}
        },
        {
            "name": "create_task",
            "description": "Create a task. With no date or project it lands in the inbox.",
            "input_schema": {"type": "object", "properties": create_props, "required": ["title"]}
        },
        {
            "name": "update_task",
            "description": "Change fields of a task. Only the fields given change.",
            "input_schema": {"type": "object", "properties": update_props, "required": ["id"]}
        },
        {
            "name": "complete_task",
            "description": "Mark a task done. A repeating task gets its next occurrence.",
            "input_schema": {"type": "object", "properties": {"id": {"type": "integer"}}, "required": ["id"]}
        },
        {
            "name": "delete_task",
            "description": "Delete a task for good. Only when the user asks.",
            "input_schema": {"type": "object", "properties": {"id": {"type": "integer"}}, "required": ["id"]}
        },
        {
            "name": "schedule_task",
            "description": "Time-block a task on the calendar.",
            "input_schema": {"type": "object", "properties": {
                "id": {"type": "integer"},
                "start": {"type": "string", "description": "YYYY-MM-DDTHH:MM local time"},
                "duration_min": {"type": "integer", "minimum": 5}
            }, "required": ["id", "start"]}
        },
        {
            "name": "get_calendar",
            "description": "Calendar events and time-blocked tasks for a range of days.",
            "input_schema": {"type": "object", "properties": {
                "from": {"type": "string", "description": "YYYY-MM-DD"},
                "to": {"type": "string", "description": "YYYY-MM-DD, inclusive"}
            }, "required": ["from", "to"]}
        },
        {
            "name": "find_free_time",
            "description": "Free slots on a day within the user's work hours, after events and blocks. Today only counts time from now.",
            "input_schema": {"type": "object", "properties": {"date": {"type": "string", "description": "YYYY-MM-DD"}}, "required": ["date"]}
        },
        {
            "name": "auto_schedule_today",
            "description": "Block today's unscheduled tasks into free time, most important first. Set apply=false to preview.",
            "input_schema": {"type": "object", "properties": {"apply": {"type": "boolean"}}, "required": ["apply"]}
        },
        {
            "name": "list_projects",
            "description": "All projects and labels.",
            "input_schema": {"type": "object", "properties": {}}
        }
    ])
}

/// A short line for the chat transcript about what a tool did.
pub fn describe(name: &str, input: &Value) -> String {
    let title = input.get("title").and_then(Value::as_str);
    let id = input.get("id").and_then(Value::as_i64);
    match (name, title, id) {
        ("create_task", Some(t), _) => format!("Added “{t}”"),
        ("update_task", _, Some(id)) => format!("Updated task #{id}"),
        ("complete_task", _, Some(id)) => format!("Completed task #{id}"),
        ("delete_task", _, Some(id)) => format!("Deleted task #{id}"),
        ("schedule_task", _, Some(id)) => {
            format!("Blocked task #{id} at {}", input.get("start").and_then(Value::as_str).unwrap_or("?"))
        }
        ("auto_schedule_today", _, _) => "Planned today's calendar".into(),
        (other, _, _) => format!("Looked at {}", other.replace('_', " ")),
    }
}

/// Tools that change data, so the app should reload after them.
pub fn writes(name: &str) -> bool {
    matches!(name, "create_task" | "update_task" | "complete_task" | "delete_task" | "schedule_task")
        || name == "auto_schedule_today"
}

fn date(v: &Value, key: &str) -> Result<Option<Option<NaiveDate>>, String> {
    match v.get(key).and_then(Value::as_str) {
        None => Ok(None),
        Some("") => Ok(Some(None)),
        Some(s) => NaiveDate::parse_from_str(s, "%Y-%m-%d").map(|d| Some(Some(d))).map_err(|_| format!("{key}: expected YYYY-MM-DD, got {s:?}")),
    }
}

fn time(v: &Value) -> Result<Option<Option<NaiveTime>>, String> {
    match v.get("time").and_then(Value::as_str) {
        None => Ok(None),
        Some("") => Ok(Some(None)),
        Some(s) => NaiveTime::parse_from_str(s, "%H:%M").map(|t| Some(Some(t))).map_err(|_| format!("time: expected HH:MM, got {s:?}")),
    }
}

fn opt_text(v: &Value, key: &str) -> Option<Option<String>> {
    v.get(key).and_then(Value::as_str).map(|s| if s.is_empty() { None } else { Some(s.to_string()) })
}

fn labels(v: &Value) -> Option<Vec<String>> {
    v.get("labels").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
}

fn id(v: &Value) -> Result<i64, String> {
    v.get("id").and_then(Value::as_i64).ok_or_else(|| "id is required".to_string())
}

fn project_id(store: &Store, v: &Value) -> Result<Option<Option<i64>>, String> {
    match v.get("project").and_then(Value::as_str) {
        None => Ok(None),
        Some("") => Ok(Some(None)),
        Some(name) => Ok(Some(Some(store.ensure_project(name).map_err(|e| e.to_string())?.id))),
    }
}

pub fn task_json(t: &Task, store: &Store) -> Value {
    let project = t.project_id.and_then(|id| store.projects().ok()?.into_iter().find(|p| p.id == id)).map(|p| p.name);
    json!({
        "id": t.id,
        "title": t.title,
        "status": t.status,
        "date": t.date,
        "time": t.time.map(|t| t.format("%H:%M").to_string()),
        "duration_min": t.duration_min,
        "priority": t.priority,
        "project": project,
        "labels": t.labels,
        "deadline": t.deadline,
        "someday": t.someday,
        "repeat": t.rrule,
        "notes": if t.notes.chars().count() > 300 { format!("{}…", t.notes.chars().take(300).collect::<String>()) } else { t.notes.clone() },
        "source": t.source,
        "source_url": t.source_url,
    })
}

fn tasks_json(ts: &[Task], store: &Store) -> Value {
    Value::Array(ts.iter().map(|t| task_json(t, store)).collect())
}

/// Runs one tool call. `now` is local wall-clock time.
pub fn run(store: &Store, name: &str, input: &Value, now: NaiveDateTime) -> Result<Value, String> {
    let today = now.date();
    let e = |e: flowday_core::store::Error| e.to_string();
    match name {
        "list_tasks" => {
            let view = if let Some(p) = input.get("project").and_then(Value::as_str) {
                let project = store.project_by_name(p).map_err(e)?.ok_or_else(|| format!("no project named {p:?}"))?;
                View::Project(project.id)
            } else {
                match input.get("view").and_then(Value::as_str).unwrap_or("today") {
                    "inbox" => View::Inbox,
                    "upcoming" => View::Upcoming,
                    "someday" => View::Someday,
                    "done" => View::Done,
                    _ => View::Today,
                }
            };
            let mut ts = store.list(&view, today).map_err(e)?;
            ts.truncate(100);
            Ok(tasks_json(&ts, store))
        }
        "search_tasks" => {
            let q = input.get("query").and_then(Value::as_str).unwrap_or_default();
            Ok(tasks_json(&store.search(q).map_err(e)?, store))
        }
        "create_task" => {
            let title = input.get("title").and_then(Value::as_str).filter(|s| !s.trim().is_empty()).ok_or("title is required")?;
            let n = NewTask {
                title: title.trim().to_string(),
                notes: input.get("notes").and_then(Value::as_str).unwrap_or_default().to_string(),
                project_id: project_id(store, input)?.flatten(),
                priority: input.get("priority").and_then(Value::as_u64).unwrap_or(0).min(3) as u8,
                date: date(input, "date")?.flatten(),
                time: time(input)?.flatten(),
                duration_min: input.get("duration_min").and_then(Value::as_u64).map(|m| m as u32),
                deadline: date(input, "deadline")?.flatten(),
                someday: input.get("someday").and_then(Value::as_bool).unwrap_or(false),
                rrule: opt_text(input, "repeat").flatten(),
                parent_id: input.get("parent_id").and_then(Value::as_i64),
                labels: labels(input).unwrap_or_default(),
            };
            let n = NewTask { date: n.date.or(n.time.map(|_| today)), ..n };
            Ok(task_json(&store.add_task(&n).map_err(e)?, store))
        }
        "update_task" => {
            let id = id(input)?;
            let p = TaskPatch {
                title: input.get("title").and_then(Value::as_str).map(str::to_string),
                notes: input.get("notes").and_then(Value::as_str).map(str::to_string),
                project_id: project_id(store, input)?,
                priority: input.get("priority").and_then(Value::as_u64).map(|p| p.min(3) as u8),
                date: date(input, "date")?,
                time: time(input)?,
                duration_min: input.get("duration_min").and_then(Value::as_u64).map(|m| m as u32),
                deadline: date(input, "deadline")?,
                someday: input.get("someday").and_then(Value::as_bool),
                rrule: opt_text(input, "repeat"),
                labels: labels(input),
            };
            Ok(task_json(&store.update(id, &p).map_err(e)?, store))
        }
        "complete_task" => {
            let next = store.complete(id(input)?, today).map_err(e)?;
            Ok(json!({"done": true, "next_occurrence": next.map(|t| task_json(&t, store))}))
        }
        "delete_task" => {
            store.delete(id(input)?).map_err(e)?;
            Ok(json!({"deleted": true}))
        }
        "schedule_task" => {
            let id = id(input)?;
            let s = input.get("start").and_then(Value::as_str).ok_or("start is required")?;
            let start = NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M").map_err(|_| format!("start: expected YYYY-MM-DDTHH:MM, got {s:?}"))?;
            let minutes = match input.get("duration_min").and_then(Value::as_u64) {
                Some(m) => m as u32,
                None => store.task(id).map_err(e)?.duration_min,
            };
            Ok(task_json(&store.schedule(id, start, minutes).map_err(e)?, store))
        }
        "get_calendar" => {
            let from = date(input, "from")?.flatten().ok_or("from is required")?;
            let to = date(input, "to")?.flatten().ok_or("to is required")?;
            if (to - from).num_days() > 62 {
                return Err("ask for at most two months".into());
            }
            let events: Vec<Value> = store
                .events_between(from, to)
                .map_err(e)?
                .iter()
                .map(|ev| json!({"title": ev.title, "start": ev.start, "end": ev.end, "all_day": ev.all_day, "location": ev.location}))
                .collect();
            let blocks: Vec<Value> = store
                .tasks_between(from, to)
                .map_err(e)?
                .iter()
                .filter(|t| t.time.is_some())
                .map(|t| task_json(t, store))
                .collect();
            Ok(json!({"events": events, "blocked_tasks": blocks}))
        }
        "find_free_time" => {
            let day = date(input, "date")?.flatten().ok_or("date is required")?;
            let mut busy: Vec<Slot> =
                store.events_on(day).map_err(e)?.iter().filter(|ev| !ev.all_day).map(|ev| Slot::new(ev.start, ev.end)).collect();
            for t in store.blocks_on(day).map_err(e)? {
                if let Some(s) = t.start() {
                    busy.push(Slot::new(s, s + Duration::minutes(i64::from(t.duration_min))));
                }
            }
            let after = (day == today).then_some(now);
            let free = free_slots(day, plan::hours(store), &busy, after);
            Ok(Value::Array(
                free.iter()
                    .map(|s| json!({"start": s.start.format("%H:%M").to_string(), "end": s.end.format("%H:%M").to_string(), "minutes": s.minutes()}))
                    .collect(),
            ))
        }
        "auto_schedule_today" => {
            let apply = input.get("apply").and_then(Value::as_bool).unwrap_or(false);
            let proposal = plan::propose(store, now).map_err(e)?;
            let mut out = Vec::new();
            for (id, start) in &proposal {
                let t = store.task(*id).map_err(e)?;
                if apply {
                    store.schedule(*id, *start, t.duration_min).map_err(e)?;
                }
                out.push(json!({"id": id, "title": t.title, "start": start.format("%H:%M").to_string(), "duration_min": t.duration_min}));
            }
            let left: Vec<String> = store
                .list(&View::Today, today)
                .map_err(e)?
                .into_iter()
                .filter(|t| t.time.is_none() && !proposal.iter().any(|p| p.0 == t.id))
                .map(|t| t.title)
                .collect();
            Ok(json!({"applied": apply, "blocks": out, "did_not_fit": left}))
        }
        "list_projects" => {
            let projects: Vec<String> = store.projects().map_err(e)?.into_iter().map(|p| p.name).collect();
            Ok(json!({"projects": projects, "labels": store.labels().map_err(e)?}))
        }
        other => Err(format!("unknown tool {other}")),
    }
}
