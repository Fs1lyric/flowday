//! The Claude panel: the saved conversation, and a box to ask. Each request runs on
//! a worker thread with its own store connection, so the window never blocks.

use crate::{secrets, theme};
use eframe::egui::{self, RichText};
use flowday_ai::{tools, Step};
use flowday_core::model::ChatRow;
use flowday_core::store::Store;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};

enum Msg {
    Step(Step),
    Done(Result<String, String>),
}

pub struct Chat {
    pub open: bool,
    input: String,
    rx: Option<Receiver<Msg>>,
    live: Vec<String>,
    error: Option<String>,
    history: Vec<ChatRow>,
    stale: bool,
    focus: bool,
}

/// One line of the transcript as it should read.
pub enum Line {
    User(String),
    Claude(String),
    Did(String),
}

/// Turns saved messages into readable lines: the bracketed timestamp comes off user
/// messages, tool results are hidden, tool calls become one-line summaries.
pub fn lines(rows: &[ChatRow]) -> Vec<Line> {
    let mut out = Vec::new();
    for row in rows {
        let content: Value = serde_json::from_str(&row.content).unwrap_or(Value::Null);
        let blocks = match &content {
            Value::Array(b) => b.clone(),
            Value::String(s) => vec![serde_json::json!({"type": "text", "text": s})],
            _ => continue,
        };
        for b in blocks {
            match (row.role.as_str(), b["type"].as_str()) {
                ("user", Some("text")) => {
                    let t = b["text"].as_str().unwrap_or_default();
                    let t = match t.strip_prefix('[').and_then(|r| r.split_once("] ")) {
                        Some((_, rest)) => rest,
                        None => t,
                    };
                    out.push(Line::User(t.to_string()));
                }
                ("assistant", Some("text")) => out.push(Line::Claude(b["text"].as_str().unwrap_or_default().to_string())),
                ("assistant", Some("tool_use")) => {
                    let name = b["name"].as_str().unwrap_or_default();
                    if tools::writes(name) {
                        out.push(Line::Did(tools::describe(name, &b["input"])));
                    }
                }
                _ => {}
            }
        }
    }
    out
}

impl Chat {
    pub fn new() -> Self {
        Self { open: false, input: String::new(), rx: None, live: Vec::new(), error: None, history: Vec::new(), stale: true, focus: false }
    }

    pub fn busy(&self) -> bool {
        self.rx.is_some()
    }

    /// Opens the panel and sends `text` right away.
    pub fn ask(&mut self, ctx: &egui::Context, store: &Store, db: PathBuf, text: String) {
        self.open = true;
        if self.busy() || text.trim().is_empty() {
            return;
        }
        let Some(key) = secrets::get(secrets::ANTHROPIC) else {
            self.error = Some("Add your Claude API key in Settings first.".into());
            return;
        };
        let model = store.setting("model").ok().flatten().filter(|m| !m.trim().is_empty()).unwrap_or(flowday_ai::DEFAULT_MODEL.into());
        let (tx, rx) = channel();
        self.rx = Some(rx);
        self.error = None;
        self.live.clear();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = Store::open(&db).map_err(|e| e.to_string()).and_then(|store| {
                let http = flowday_ai::Http { api_key: key };
                flowday_ai::turn(&http, &store, &model, &text, crate::now(), |step| {
                    let _ = tx.send(Msg::Step(step));
                    ctx.request_repaint();
                })
            });
            let _ = tx.send(Msg::Done(result));
            ctx.request_repaint();
        });
        self.stale = true;
    }

    /// Drains worker messages. Returns true when the store changed.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        let Some(rx) = &self.rx else { return false };
        while let Ok(msg) = rx.try_recv() {
            match msg {
                Msg::Step(Step::Tool { summary, wrote }) => {
                    self.live.push(summary);
                    changed |= wrote;
                    self.stale = true;
                }
                Msg::Done(r) => {
                    if let Err(e) = r {
                        self.error = Some(e);
                    }
                    self.rx = None;
                    self.live.clear();
                    self.stale = true;
                    changed = true;
                    break;
                }
            }
        }
        changed
    }

    pub fn focus(&mut self) {
        self.open = true;
        self.focus = true;
    }

    pub fn show(&mut self, ui: &mut egui::Ui, store: &Store, db: PathBuf) {
        if self.stale {
            self.history = store.chat_history().unwrap_or_default();
            self.stale = false;
        }
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("✨ Claude").strong().size(16.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("✕").clicked() {
                    self.open = false;
                }
                if !self.busy() && ui.small_button("New chat").clicked() {
                    let _ = store.clear_chat();
                    self.stale = true;
                }
            });
        });
        ui.separator();

        let input_h = 90.0;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .max_height(ui.available_height() - input_h)
            .stick_to_bottom(true)
            .show(ui, |ui| {
                let lines = lines(&self.history);
                if lines.is_empty() {
                    ui.label(
                        RichText::new("Ask me to plan your day, add tasks (\"add dentist next tue 3pm\"), break a task down, or find time for something.")
                            .color(theme::INK_3),
                    );
                }
                for line in lines {
                    match line {
                        Line::User(t) => {
                            ui.add_space(6.0);
                            egui::Frame::new().fill(theme::SURFACE_2).corner_radius(8.0).inner_margin(8.0).show(ui, |ui| {
                                ui.label(t);
                            });
                        }
                        Line::Claude(t) => {
                            ui.add_space(4.0);
                            ui.label(t);
                        }
                        Line::Did(t) => {
                            ui.label(RichText::new(format!("• {t}")).size(12.0).color(theme::ACCENT));
                        }
                    }
                }
                for l in &self.live {
                    ui.label(RichText::new(format!("• {l}")).size(12.0).color(theme::ACCENT));
                }
                if self.busy() {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(RichText::new("Thinking…").color(theme::INK_3));
                    });
                }
                if let Some(e) = &self.error {
                    ui.label(RichText::new(e).color(theme::DANGER));
                }
            });

        ui.separator();
        let edit = ui.add_enabled(
            !self.busy(),
            egui::TextEdit::multiline(&mut self.input)
                .hint_text("Ask Claude… (Enter to send, Shift+Enter for a new line)")
                .desired_rows(2)
                .desired_width(f32::INFINITY),
        );
        if std::mem::take(&mut self.focus) {
            edit.request_focus();
        }
        let send = edit.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift);
        if send {
            let text = std::mem::take(&mut self.input).trim().to_string();
            self.ask(ui.ctx(), store, db, text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(role: &str, content: Value) -> ChatRow {
        ChatRow { id: 0, role: role.into(), content: content.to_string() }
    }

    #[test]
    fn transcript_hides_results_and_timestamps() {
        let rows = [
            row("user", serde_json::json!([{"type": "text", "text": "[Friday 2026-10-16 08:00] plan my day"}])),
            row("assistant", serde_json::json!([
                {"type": "thinking", "thinking": ""},
                {"type": "tool_use", "id": "a", "name": "list_tasks", "input": {}},
                {"type": "tool_use", "id": "b", "name": "create_task", "input": {"title": "Gym"}}
            ])),
            row("user", serde_json::json!([{"type": "tool_result", "tool_use_id": "a", "content": "[]"}])),
            row("assistant", serde_json::json!([{"type": "text", "text": "Done."}])),
        ];
        let l = lines(&rows);
        assert!(matches!(&l[0], Line::User(t) if t == "plan my day"));
        assert!(matches!(&l[1], Line::Did(t) if t == "Added “Gym”"));
        assert!(matches!(&l[2], Line::Claude(t) if t == "Done."));
        assert_eq!(l.len(), 3);
    }
}
