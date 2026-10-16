//! The Ctrl K command bar: jump to a view, run a command, find a task, or add one.

use crate::theme;
use eframe::egui::{self, Id, Key, RichText};
use flowday_core::model::{Project, Task, View};

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Go(View),
    Open(i64),
    Add(String),
    PlanDay,
    Shutdown,
    Chat,
    Sync,
    Settings,
    NewProject(String),
}

#[derive(Default)]
pub struct Palette {
    query: String,
    cursor: usize,
}

/// The commands that match a query, best first. `found` are tasks from a search.
pub fn matches(query: &str, projects: &[Project], labels: &[String], found: &[Task]) -> Vec<(String, Command)> {
    let q = query.trim().to_lowercase();
    let mut fixed: Vec<(String, Command)> = vec![
        ("Go to Inbox".into(), Command::Go(View::Inbox)),
        ("Go to Today".into(), Command::Go(View::Today)),
        ("Go to Upcoming".into(), Command::Go(View::Upcoming)),
        ("Go to Someday".into(), Command::Go(View::Someday)),
        ("Go to Done".into(), Command::Go(View::Done)),
        ("Plan my day".into(), Command::PlanDay),
        ("Shutdown: wrap up the day".into(), Command::Shutdown),
        ("Ask Claude".into(), Command::Chat),
        ("Sync integrations now".into(), Command::Sync),
        ("Settings".into(), Command::Settings),
    ];
    fixed.extend(projects.iter().map(|p| (format!("Project: {}", p.name), Command::Go(View::Project(p.id)))));
    fixed.extend(labels.iter().map(|l| (format!("Label: @{l}"), Command::Go(View::Label(l.clone())))));

    let mut out: Vec<(String, Command)> = fixed.into_iter().filter(|(name, _)| q.is_empty() || fuzzy(&name.to_lowercase(), &q)).collect();
    if !q.is_empty() {
        out.extend(found.iter().take(8).map(|t| (format!("Task: {}", t.title), Command::Open(t.id))));
        out.push((format!("Add task: {}", query.trim()), Command::Add(query.trim().to_string())));
        if let Some(name) = query.trim().strip_prefix("project ") {
            out.push((format!("New project: {name}"), Command::NewProject(name.trim().to_string())));
        }
    }
    out
}

/// Every character of `needle` appears in `hay` in order.
fn fuzzy(hay: &str, needle: &str) -> bool {
    let mut chars = hay.chars();
    needle.chars().all(|c| c == ' ' || chars.any(|h| h == c))
}

impl Palette {
    /// Returns the chosen command, or `Some(None)` when it should close.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        projects: &[Project],
        labels: &[String],
        search: impl Fn(&str) -> Vec<Task>,
    ) -> Option<Option<Command>> {
        let mut out = None;
        let found = if self.query.trim().len() >= 2 { search(self.query.trim()) } else { Vec::new() };
        let items = matches(&self.query, projects, labels, &found);
        self.cursor = self.cursor.min(items.len().saturating_sub(1));
        let modal = egui::Modal::new(Id::new("palette"))
            .frame(egui::Frame::window(&ctx.global_style()).inner_margin(12.0))
            .show(ctx, |ui| {
                ui.set_width(520.0);
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut self.query)
                        .hint_text("Type a command, search, or add a task…")
                        .font(egui::FontId::proportional(17.0))
                        .frame(egui::Frame::NONE)
                        .desired_width(f32::INFINITY),
                );
                edit.request_focus();
                if edit.changed() {
                    self.cursor = 0;
                }
                ui.separator();
                let (down, up, enter) = ui.input_mut(|i| {
                    (
                        i.consume_key(egui::Modifiers::NONE, Key::ArrowDown),
                        i.consume_key(egui::Modifiers::NONE, Key::ArrowUp),
                        i.key_pressed(Key::Enter),
                    )
                });
                if down {
                    self.cursor = (self.cursor + 1).min(items.len().saturating_sub(1));
                }
                if up {
                    self.cursor = self.cursor.saturating_sub(1);
                }
                egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                    for (i, (name, cmd)) in items.iter().enumerate() {
                        let text = if i == self.cursor { RichText::new(name).color(theme::ACCENT) } else { RichText::new(name) };
                        let r = ui.selectable_label(i == self.cursor, text);
                        if i == self.cursor && (down || up) {
                            r.scroll_to_me(None);
                        }
                        if r.clicked() {
                            out = Some(Some(cmd.clone()));
                        }
                    }
                });
                if enter {
                    if let Some((_, cmd)) = items.get(self.cursor) {
                        out = Some(Some(cmd.clone()));
                    }
                }
            });
        if out.is_none() && modal.should_close() {
            out = Some(None);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_query_lists_the_fixed_commands() {
        let items = matches("", &[], &[], &[]);
        assert_eq!(items[0].1, Command::Go(View::Inbox));
        assert!(!items.iter().any(|i| matches!(i.1, Command::Add(_))));
    }

    #[test]
    fn typing_filters_fuzzily_and_offers_to_add() {
        let p = Project { id: 7, name: "Work".into(), color: "#9b87f5".into() };
        let items = matches("wrk", &[p], &[], &[]);
        assert_eq!(items[0].1, Command::Go(View::Project(7)));
        assert_eq!(items.last().unwrap().1, Command::Add("wrk".into()));
        assert!(matches("plan", &[], &[], &[]).iter().any(|i| i.1 == Command::PlanDay));
    }
}
