//! Claude inside Flowday: a raw-HTTP client for the Messages API (there is no
//! official Rust SDK) and the tool loop that lets it act on the store.

pub mod tools;

use chrono::NaiveDateTime;
use flowday_core::store::Store;
use serde_json::{json, Value};

pub const DEFAULT_MODEL: &str = "claude-opus-5-5";
const API_URL: &str = "https://api.anthropic.com/v1/messages";
/// Tool rounds per user message before giving up.
const MAX_ROUNDS: usize = 16;

const SYSTEM: &str = "You are the assistant built into Flowday, the user's personal planner (in the style of Akiflow). \
You manage their tasks and calendar with the tools: add and update tasks, plan days, time-block work, and break big tasks into subtasks. \
Each user message starts with the current local date and time in brackets. \
Act on clear requests without asking for confirmation, except before deleting. \
When planning a day, look at the calendar and free time first, put important and deadline-bound work early, and leave buffers. \
Reply briefly in plain text (no markdown headings or tables): say what you changed, or answer the question.";

/// Sends one Messages API request body and returns the response body.
pub trait Transport {
    fn send(&self, body: &Value) -> Result<Value, String>;
}

pub struct Http {
    pub api_key: String,
}

impl Transport for Http {
    fn send(&self, body: &Value) -> Result<Value, String> {
        let resp = ureq::post(API_URL)
            .timeout(std::time::Duration::from_secs(600))
            .set("x-api-key", &self.api_key)
            .set("anthropic-version", "2023-06-01")
            .set("anthropic-beta", "server-side-fallback-2026-07-01")
            .set("content-type", "application/json")
            .send_json(body.clone());
        match resp {
            Ok(r) => r.into_json::<Value>().map_err(|e| format!("bad response from Claude: {e}")),
            Err(ureq::Error::Status(code, r)) => {
                let body: Value = r.into_json().unwrap_or(Value::Null);
                let msg = body["error"]["message"].as_str().unwrap_or("no details");
                Err(match code {
                    401 => "Claude rejected the API key. Check it in Settings.".into(),
                    429 => format!("Claude is rate limiting requests; try again shortly ({msg})"),
                    529 => "Claude is overloaded right now; try again shortly.".into(),
                    _ => format!("Claude returned HTTP {code}: {msg}"),
                })
            }
            Err(e) => Err(format!("can't reach Claude: {e}")),
        }
    }
}

/// What happened during a turn, for the UI.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// A tool ran; `wrote` is true when it changed data.
    Tool { summary: String, wrote: bool },
}

/// The request body for the conversation so far.
pub fn request(model: &str, messages: &[Value]) -> Value {
    json!({
        "model": model,
        "max_tokens": 16000,
        "system": SYSTEM,
        "tools": tools::definitions(),
        "thinking": {"type": "adaptive"},
        "output_config": {"effort": "medium"},
        "fallbacks": "default",
        "cache_control": {"type": "ephemeral"},
        "messages": messages,
    })
}

/// The text blocks of a message's content, joined.
pub fn text_of(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter(|b| b["type"] == "text")
            .filter_map(|b| b["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// Runs one user message to completion: sends it, runs every tool Claude asks
/// for, and loops until Claude stops. Each message is saved to the store's chat
/// as it happens (the full content, thinking blocks included, so the next request
/// replays it unchanged). Returns Claude's final text.
pub fn turn(
    transport: &dyn Transport,
    store: &Store,
    model: &str,
    user_text: &str,
    now: NaiveDateTime,
    mut on_step: impl FnMut(Step),
) -> Result<String, String> {
    let mut messages: Vec<Value> = store
        .chat_history()
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|row| json!({"role": row.role, "content": serde_json::from_str::<Value>(&row.content).unwrap_or(Value::Null)}))
        .collect();
    let user = json!([{"type": "text", "text": format!("[{}] {}", now.format("%A %Y-%m-%d %H:%M"), user_text)}]);
    store.push_chat("user", &user.to_string()).map_err(|e| e.to_string())?;
    messages.push(json!({"role": "user", "content": user}));

    for _ in 0..MAX_ROUNDS {
        let resp = transport.send(&request(model, &messages))?;
        let stop = resp["stop_reason"].as_str().unwrap_or("");
        if stop == "refusal" {
            let note = json!([{"type": "text", "text": "(Claude declined this request.)"}]);
            store.push_chat("assistant", &note.to_string()).map_err(|e| e.to_string())?;
            return Err("Claude declined this request.".into());
        }
        let content = resp["content"].clone();
        store.push_chat("assistant", &content.to_string()).map_err(|e| e.to_string())?;
        messages.push(json!({"role": "assistant", "content": content.clone()}));

        if stop != "tool_use" {
            let text = text_of(&content);
            if stop == "max_tokens" {
                return Ok(format!("{text}\n(The reply was cut off.)"));
            }
            return Ok(text);
        }
        let mut results = Vec::new();
        for block in content.as_array().into_iter().flatten().filter(|b| b["type"] == "tool_use") {
            let name = block["name"].as_str().unwrap_or_default();
            let input = &block["input"];
            let result = tools::run(store, name, input, now);
            on_step(Step::Tool { summary: tools::describe(name, input), wrote: tools::writes(name) && result.is_ok() });
            results.push(match result {
                Ok(v) => json!({"type": "tool_result", "tool_use_id": block["id"], "content": v.to_string()}),
                Err(e) => json!({"type": "tool_result", "tool_use_id": block["id"], "content": e, "is_error": true}),
            });
        }
        let results = Value::Array(results);
        store.push_chat("user", &results.to_string()).map_err(|e| e.to_string())?;
        messages.push(json!({"role": "user", "content": results}));
    }
    Err("Claude kept calling tools without finishing; stopped.".into())
}
