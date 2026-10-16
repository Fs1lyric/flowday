//! The task pane on the right: every field of the selected task, saved as it is
//! edited, plus its subtasks.

use crate::theme;
use chrono::{Days, NaiveDate, NaiveTime};
use eframe::egui::{self, RichText};
use flowday_core::model::{NewTask, Project, Status, Task, TaskPatch};
use flowday_core::store::Store;

pub struct Detail {
    pub id: i64,
    title: String,
    notes: String,
    labels: String,
    date: String,
    time: String,
    deadline: String,
    subtask: String,
    error: Option<String>,
}

pub enum Outcome {
    Nothing,
    /// Hand this request to Claude.
    Ask(String),
    Changed,
    Close,
    Deleted,
}

const REPEATS: [(&str, Option<&str>); 6] = [
    ("Doesn't repeat", None),
    ("Every day", Some("FREQ=DAILY")),
    ("Every weekday", Some("FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR")),
    ("Every week", Some("FREQ=WEEKLY")),
    ("Every month", Some("FREQ=MONTHLY")),
    ("Every year", Some("FREQ=YEARLY")),
];

fn date_text(d: Option<NaiveDate>) -> String {
    d.map(|d| d.to_string()).unwrap_or_default()
}

/// Reads `2026-10-17`, `today`, `tomorrow` or a weekday name; empty clears.
pub fn parse_date(s: &str, today: NaiveDate) -> Result<Option<NaiveDate>, String> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(None);
    }
    if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Ok(Some(d));
    }
    let q = flowday_core::quickadd::parse(&format!("x {s}"), today);
    match q.date {
        Some(d) if q.title == "x" => Ok(Some(d)),
        _ => Err(format!("can't read the date {s:?}")),
    }
}

/// Reads `14:30`, `2pm` or `9`; empty clears.
pub fn parse_time(s: &str) -> Result<Option<NaiveTime>, String> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(None);
    }
    if let Ok(t) = NaiveTime::parse_from_str(s, "%H:%M") {
        return Ok(Some(t));
    }
    let today = NaiveDate::from_ymd_opt(2000, 1, 3).unwrap();
    let q = flowday_core::quickadd::parse(&format!("x {s}"), today);
    match q.time {
        Some(t) if q.title == "x" => Ok(Some(t)),
        _ => Err(format!("can't read the time {s:?}")),
    }
}

impl Detail {
    pub fn new(t: &Task) -> Self {
        Self {
            id: t.id,
            title: t.title.clone(),
            notes: t.notes.clone(),
            labels: t.labels.join(", "),
            date: date_text(t.date),
            time: t.time.map(|t| t.format("%H:%M").to_string()).unwrap_or_default(),
            deadline: date_text(t.deadline),
            subtask: String::new(),
            error: None,
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui, store: &Store, projects: &[Project], today: NaiveDate) -> Outcome {
        let t = match store.task(self.id) {
            Ok(t) => t,
            Err(_) => return Outcome::Close,
        };
        let mut patch = TaskPatch::default();
        let mut out = Outcome::Nothing;

        ui.horizontal(|ui| {
            let done = t.status == Status::Done;
            if ui.button(if done { "Reopen" } else { "✔ Done" }).clicked() {
                let r = if done { store.reopen(t.id) } else { store.complete(t.id, today).map(drop) };
                self.report(r);
                out = Outcome::Changed;
            }
            if ui.button("✨ Break down").on_hover_text("Claude splits it into subtasks").clicked() {
                out = Outcome::Ask(format!("Break task #{} \"{}\" into a few concrete subtasks (create them with parent_id {}) and suggest a total duration.", t.id, t.title, t.id));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("✕").on_hover_text("Close (Esc)").clicked() {
                    out = Outcome::Close;
                }
                if ui.button(RichText::new("Delete").color(theme::DANGER)).clicked() {
                    self.report(store.delete(t.id));
                    out = Outcome::Deleted;
                }
            });
        });
        if matches!(out, Outcome::Deleted | Outcome::Close | Outcome::Ask(_)) {
            return out;
        }
        ui.add_space(6.0);

        let title = ui.add(
            egui::TextEdit::singleline(&mut self.title)
                .font(egui::FontId::proportional(19.0))
                .frame(egui::Frame::NONE)
                .desired_width(f32::INFINITY),
        );
        if title.lost_focus() && self.title.trim() != t.title && !self.title.trim().is_empty() {
            patch.title = Some(self.title.trim().to_string());
        }
        if let Some(url) = &t.source_url {
            ui.hyperlink_to(format!("Open in {}", t.source.as_deref().unwrap_or("source")), url);
        }
        ui.add_space(4.0);

        egui::Grid::new("fields").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
            ui.label(RichText::new("Project").color(theme::INK_3));
            let current = projects.iter().find(|p| Some(p.id) == t.project_id).map_or("Inbox", |p| p.name.as_str());
            egui::ComboBox::from_id_salt("project").selected_text(current).show_ui(ui, |ui| {
                if ui.selectable_label(t.project_id.is_none(), "Inbox").clicked() {
                    patch.project_id = Some(None);
                }
                for p in projects {
                    if ui.selectable_label(t.project_id == Some(p.id), &p.name).clicked() {
                        patch.project_id = Some(Some(p.id));
                    }
                }
            });
            ui.end_row();

            ui.label(RichText::new("Priority").color(theme::INK_3));
            ui.horizontal(|ui| {
                for (p, label) in [(0u8, "None"), (3, "!1"), (2, "!2"), (1, "!3")] {
                    let text = RichText::new(label).color(theme::priority(p).unwrap_or(theme::INK_2));
                    if ui.selectable_label(t.priority == p, text).clicked() {
                        patch.priority = Some(p);
                    }
                }
            });
            ui.end_row();

            ui.label(RichText::new("Date").color(theme::INK_3));
            ui.horizontal(|ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut self.date).hint_text("none").desired_width(96.0));
                if r.lost_focus() {
                    match parse_date(&self.date, today) {
                        Ok(d) if d != t.date => patch.date = Some(d),
                        Ok(_) => {}
                        Err(e) => self.error = Some(e),
                    }
                }
                if ui.small_button("Today").clicked() {
                    patch.date = Some(Some(today));
                }
                if ui.small_button("Tomorrow").clicked() {
                    patch.date = Some(today.checked_add_days(Days::new(1)));
                }
            });
            ui.end_row();

            ui.label(RichText::new("Time").color(theme::INK_3));
            ui.horizontal(|ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut self.time).hint_text("unscheduled").desired_width(96.0));
                if r.lost_focus() {
                    match parse_time(&self.time) {
                        Ok(tm) if tm != t.time => {
                            patch.time = Some(tm);
                            if tm.is_some() && t.date.is_none() {
                                patch.date = Some(Some(today));
                            }
                        }
                        Ok(_) => {}
                        Err(e) => self.error = Some(e),
                    }
                }
                let mut minutes = t.duration_min;
                egui::ComboBox::from_id_salt("duration").selected_text(crate::list::duration(minutes)).width(70.0).show_ui(
                    ui,
                    |ui| {
                        for m in [15, 30, 45, 60, 90, 120, 180, 240] {
                            ui.selectable_value(&mut minutes, m, crate::list::duration(m));
                        }
                    },
                );
                if minutes != t.duration_min {
                    patch.duration_min = Some(minutes);
                }
            });
            ui.end_row();

            ui.label(RichText::new("Deadline").color(theme::INK_3));
            let r = ui.add(egui::TextEdit::singleline(&mut self.deadline).hint_text("none").desired_width(96.0));
            if r.lost_focus() {
                match parse_date(&self.deadline, today) {
                    Ok(d) if d != t.deadline => patch.deadline = Some(d),
                    Ok(_) => {}
                    Err(e) => self.error = Some(e),
                }
            }
            ui.end_row();

            ui.label(RichText::new("Repeat").color(theme::INK_3));
            let current = REPEATS.iter().find(|r| r.1 == t.rrule.as_deref()).map_or("Custom", |r| r.0);
            egui::ComboBox::from_id_salt("repeat").selected_text(current).show_ui(ui, |ui| {
                for (label, rule) in REPEATS {
                    if ui.selectable_label(t.rrule.as_deref() == rule, label).clicked() {
                        patch.rrule = Some(rule.map(str::to_string));
                        if rule.is_some() && t.date.is_none() {
                            patch.date = Some(Some(today));
                        }
                    }
                }
            });
            ui.end_row();

            ui.label(RichText::new("Labels").color(theme::INK_3));
            let r = ui.add(egui::TextEdit::singleline(&mut self.labels).hint_text("deep, errand").desired_width(180.0));
            if r.lost_focus() {
                let labels: Vec<String> =
                    self.labels.split(',').map(|s| s.trim().trim_start_matches('@').to_string()).filter(|s| !s.is_empty()).collect();
                if labels != t.labels {
                    patch.labels = Some(labels);
                }
            }
            ui.end_row();
        });

        ui.horizontal(|ui| {
            let mut someday = t.someday;
            if ui.checkbox(&mut someday, "Someday").changed() {
                patch.someday = Some(someday);
            }
            if t.time.is_some() && ui.button("Remove from calendar").clicked() {
                self.report(store.unschedule(t.id).map(drop));
                out = Outcome::Changed;
            }
            ui.menu_button("Snooze", |ui| {
                for (label, days) in [("Tomorrow", 1), ("In 3 days", 3), ("Next week", 7)] {
                    if ui.button(label).clicked() {
                        self.report(store.snooze(t.id, today + Days::new(days)));
                        out = Outcome::Changed;
                        ui.close();
                    }
                }
            });
        });

        ui.add_space(8.0);
        ui.label(RichText::new("Notes").color(theme::INK_3));
        let r = ui.add(egui::TextEdit::multiline(&mut self.notes).desired_rows(5).desired_width(f32::INFINITY));
        if r.lost_focus() && self.notes != t.notes {
            patch.notes = Some(self.notes.clone());
        }

        ui.add_space(8.0);
        ui.label(RichText::new("Subtasks").color(theme::INK_3));
        match store.subtasks(t.id) {
            Ok(subs) => {
                for s in subs {
                    ui.horizontal(|ui| {
                        let mut done = s.status == Status::Done;
                        if ui.checkbox(&mut done, &s.title).changed() {
                            let r = if done { store.complete(s.id, today).map(drop) } else { store.reopen(s.id) };
                            self.report(r);
                            out = Outcome::Changed;
                        }
                        if ui.small_button("✕").clicked() {
                            self.report(store.delete(s.id));
                            out = Outcome::Changed;
                        }
                    });
                }
            }
            Err(e) => self.error = Some(e.to_string()),
        }
        let r = ui.add(egui::TextEdit::singleline(&mut self.subtask).hint_text("Add a subtask").desired_width(f32::INFINITY));
        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !self.subtask.trim().is_empty() {
            let n = NewTask { title: self.subtask.trim().to_string(), parent_id: Some(t.id), ..Default::default() };
            self.report(store.add_task(&n).map(drop));
            self.subtask.clear();
            r.request_focus();
            out = Outcome::Changed;
        }

        if let Some(e) = &self.error {
            ui.add_space(6.0);
            ui.label(RichText::new(e).color(theme::DANGER));
        }

        let touched = patch.title.is_some()
            || patch.notes.is_some()
            || patch.project_id.is_some()
            || patch.priority.is_some()
            || patch.date.is_some()
            || patch.time.is_some()
            || patch.duration_min.is_some()
            || patch.deadline.is_some()
            || patch.someday.is_some()
            || patch.rrule.is_some()
            || patch.labels.is_some();
        if touched {
            match store.update(t.id, &patch) {
                Ok(new) => {
                    let (notes, title, labels) = (self.notes.clone(), self.title.clone(), self.labels.clone());
                    *self = Detail::new(&new);
                    // Keep what is still being typed in other fields.
                    (self.notes, self.title, self.labels) = (notes, title, labels);
                    out = Outcome::Changed;
                }
                Err(e) => self.error = Some(e.to_string()),
            }
        }
        out
    }

    fn report<T>(&mut self, r: flowday_core::store::Result<T>) {
        if let Err(e) = r {
            self.error = Some(e.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_and_times_read_loosely() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 16).unwrap();
        assert_eq!(parse_date("2026-10-20", today), Ok(NaiveDate::from_ymd_opt(2026, 10, 20)));
        assert_eq!(parse_date("tomorrow", today), Ok(NaiveDate::from_ymd_opt(2026, 10, 17)));
        assert_eq!(parse_date("", today), Ok(None));
        assert!(parse_date("soonish", today).is_err());
        assert_eq!(parse_time("14:30"), Ok(NaiveTime::from_hms_opt(14, 30, 0)));
        assert_eq!(parse_time("2pm"), Ok(NaiveTime::from_hms_opt(14, 0, 0)));
    }
}
