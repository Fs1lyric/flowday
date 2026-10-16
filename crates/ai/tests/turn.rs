use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use flowday_ai::{request, tools, turn, Step, Transport};
use flowday_core::model::{NewTask, View};
use flowday_core::store::Store;
use serde_json::{json, Value};
use std::cell::RefCell;

fn now() -> NaiveDateTime {
    NaiveDate::from_ymd_opt(2026, 10, 16).unwrap().and_time(NaiveTime::from_hms_opt(8, 0, 0).unwrap())
}

/// Replays canned responses and records what was sent.
struct Script {
    replies: RefCell<Vec<Value>>,
    sent: RefCell<Vec<Value>>,
}

impl Transport for Script {
    fn send(&self, body: &Value) -> Result<Value, String> {
        self.sent.borrow_mut().push(body.clone());
        Ok(self.replies.borrow_mut().remove(0))
    }
}

#[test]
fn a_tool_call_runs_against_the_store_and_the_loop_continues() {
    let store = Store::open_in_memory().unwrap();
    let script = Script {
        replies: RefCell::new(vec![
            json!({"stop_reason": "tool_use", "content": [
                {"type": "thinking", "thinking": "", "signature": "sig"},
                {"type": "tool_use", "id": "tu1", "name": "create_task",
                 "input": {"title": "Call mom", "date": "2026-10-16", "time": "17:00", "project": "Family"}}
            ]}),
            json!({"stop_reason": "end_turn", "content": [{"type": "text", "text": "Added it at 5pm."}]}),
        ]),
        sent: RefCell::new(Vec::new()),
    };
    let mut steps = Vec::new();
    let reply = turn(&script, &store, "claude-opus-5-5", "remind me to call mom at 5", now(), |s| steps.push(s)).unwrap();
    assert_eq!(reply, "Added it at 5pm.");
    assert_eq!(steps, vec![Step::Tool { summary: "Added “Call mom”".into(), wrote: true }]);

    let today = store.list(&View::Today, now().date()).unwrap();
    assert_eq!(today.len(), 1);
    assert_eq!(today[0].time, NaiveTime::from_hms_opt(17, 0, 0));

    // The second request replays the assistant turn unchanged (thinking included)
    // and answers the tool call.
    let sent = script.sent.borrow();
    let msgs = sent[1]["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 3);
    assert_eq!(msgs[1]["content"][0]["signature"], "sig");
    assert_eq!(msgs[2]["content"][0]["tool_use_id"], "tu1");
    assert!(msgs[0]["content"][0]["text"].as_str().unwrap().starts_with("[Friday 2026-10-16 08:00]"));

    // Everything is in the saved chat for next time.
    assert_eq!(store.chat_history().unwrap().len(), 4);
}

#[test]
fn a_failing_tool_reports_an_error_result_instead_of_stopping() {
    let store = Store::open_in_memory().unwrap();
    let script = Script {
        replies: RefCell::new(vec![
            json!({"stop_reason": "tool_use", "content": [
                {"type": "tool_use", "id": "tu1", "name": "complete_task", "input": {"id": 999}}
            ]}),
            json!({"stop_reason": "end_turn", "content": [{"type": "text", "text": "That task doesn't exist."}]}),
        ]),
        sent: RefCell::new(Vec::new()),
    };
    turn(&script, &store, "m", "finish 999", now(), |_| {}).unwrap();
    let sent = script.sent.borrow();
    let result = &sent[1]["messages"][2]["content"][0];
    assert_eq!(result["is_error"], true);
}

#[test]
fn a_refusal_is_reported() {
    let store = Store::open_in_memory().unwrap();
    let script = Script {
        replies: RefCell::new(vec![json!({"stop_reason": "refusal", "content": []})]),
        sent: RefCell::new(Vec::new()),
    };
    assert!(turn(&script, &store, "m", "hi", now(), |_| {}).is_err());
}

#[test]
fn requests_use_adaptive_thinking_and_fallbacks() {
    let body = request("claude-opus-5-5", &[]);
    assert_eq!(body["thinking"]["type"], "adaptive");
    assert_eq!(body["fallbacks"], "default");
    assert!(body.get("tool_choice").is_none());
}

#[test]
fn auto_schedule_previews_then_applies() {
    let store = Store::open_in_memory().unwrap();
    store.set_setting("buffer_min", "0").unwrap();
    let day = now().date();
    store.add_task(&NewTask { title: "write".into(), date: Some(day), duration_min: Some(60), ..Default::default() }).unwrap();
    let preview = tools::run(&store, "auto_schedule_today", &json!({"apply": false}), now()).unwrap();
    assert_eq!(preview["blocks"][0]["start"], "09:00");
    assert!(store.blocks_on(day).unwrap().is_empty());
    tools::run(&store, "auto_schedule_today", &json!({"apply": true}), now()).unwrap();
    assert_eq!(store.blocks_on(day).unwrap().len(), 1);
}

#[test]
fn update_task_clears_with_empty_strings_and_finds_free_time() {
    let store = Store::open_in_memory().unwrap();
    let day = now().date();
    let t = store.add_task(&NewTask { title: "x".into(), date: Some(day), ..Default::default() }).unwrap();
    let out = tools::run(&store, "update_task", &json!({"id": t.id, "date": "", "priority": 3}), now()).unwrap();
    assert_eq!(out["date"], Value::Null);
    assert_eq!(out["priority"], 3);
    let free = tools::run(&store, "find_free_time", &json!({"date": "2026-10-16"}), now()).unwrap();
    assert_eq!(free[0]["start"], "09:00");
    assert!(tools::run(&store, "schedule_task", &json!({"id": t.id, "start": "tomorrow"}), now()).is_err());
}
