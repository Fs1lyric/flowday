//! The daily rituals: planning in the morning (pick today's tasks, then let the
//! scheduler block them into free time) and the shutdown at night (move what is
//! left, write a line about the day).

use crate::{list, theme};
use chrono::{Days, NaiveDate, NaiveDateTime, Timelike};
use eframe::egui::{self, Id, RichText};
use flowday_core::model::{Status, Task, TaskPatch, View};
use flowday_core::plan::{hours, propose};
use flowday_core::store::Store;
use std::collections::BTreeSet;

pub enum Ritual {
    Plan { step: PlanStep, picked: BTreeSet<i64>, candidates: Vec<Task>, proposal: Vec<(i64, NaiveDateTime)> },
    Shutdown { note: String },
}

#[derive(PartialEq, Eq)]
pub enum PlanStep {
    Pick,
    Schedule,
}

pub enum Outcome {
    Open,
    Changed,
    Close,
}

impl Ritual {
    pub fn plan(store: &Store, today: NaiveDate) -> Self {
        let mut candidates = Vec::new();
        for v in [View::Inbox, View::Upcoming] {
            candidates.extend(store.list(&v, today).unwrap_or_default());
        }
        // Upcoming only within the week, and the deadline-close ones first.
        let week = today + Days::new(7);
        candidates.retain(|t| t.date.is_none_or(|d| d <= week) || t.deadline.is_some_and(|d| d <= week));
        candidates.sort_by_key(|t| (t.deadline.unwrap_or(NaiveDate::MAX), std::cmp::Reverse(t.priority)));
        Ritual::Plan { step: PlanStep::Pick, picked: BTreeSet::new(), candidates, proposal: Vec::new() }
    }

    pub fn shutdown(store: &Store, today: NaiveDate) -> Self {
        let note = store.setting(&format!("shutdown:{today}")).ok().flatten().unwrap_or_default();
        Ritual::Shutdown { note }
    }

    pub fn show(&mut self, ctx: &egui::Context, store: &Store, now: NaiveDateTime) -> Outcome {
        let mut out = Outcome::Open;
        let modal = egui::Modal::new(Id::new("ritual"))
            .frame(egui::Frame::window(&ctx.global_style()).inner_margin(20.0))
            .show(ctx, |ui| {
                ui.set_width(540.0);
                out = match self {
                    Ritual::Plan { step, picked, candidates, proposal } => plan_ui(ui, store, now, step, picked, candidates, proposal),
                    Ritual::Shutdown { note } => shutdown_ui(ui, store, now.date(), note),
                };
            });
        if matches!(out, Outcome::Open) && modal.should_close() {
            out = Outcome::Close;
        }
        out
    }
}

fn plan_ui(
    ui: &mut egui::Ui,
    store: &Store,
    now: NaiveDateTime,
    step: &mut PlanStep,
    picked: &mut BTreeSet<i64>,
    candidates: &[Task],
    proposal: &mut Vec<(i64, NaiveDateTime)>,
) -> Outcome {
    let today = now.date();
    let mut out = Outcome::Open;
    ui.label(RichText::new("Plan my day").size(20.0).strong());
    match step {
        PlanStep::Pick => {
            let planned = store.list(&View::Today, today).unwrap_or_default();
            let minutes: u32 = planned.iter().map(|t| t.duration_min).sum();
            ui.label(
                RichText::new(format!("{} tasks already on today ({}). Pick more from your inbox and this week:", planned.len(), list::duration(minutes)))
                    .color(theme::INK_3),
            );
            ui.add_space(6.0);
            egui::ScrollArea::vertical().max_height(340.0).show(ui, |ui| {
                if candidates.is_empty() {
                    ui.label(RichText::new("Nothing waiting.").color(theme::INK_3));
                }
                for t in candidates {
                    let mut on = picked.contains(&t.id);
                    let mut text = t.title.clone();
                    let meta = list::meta(t, None, today, true);
                    if !meta.is_empty() {
                        text = format!("{text}   ({meta})");
                    }
                    if ui.checkbox(&mut on, text).changed() {
                        if on {
                            picked.insert(t.id);
                        } else {
                            picked.remove(&t.id);
                        }
                    }
                }
            });
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    out = Outcome::Close;
                }
                if ui.button(RichText::new("Next: block out time  →").color(theme::ACCENT)).clicked() {
                    for &id in picked.iter() {
                        let _ = store.plan(id, Some(today));
                    }
                    *proposal = propose(store, now).unwrap_or_default();
                    *step = PlanStep::Schedule;
                    out = Outcome::Changed;
                }
            });
        }
        PlanStep::Schedule => {
            let h = hours(store);
            ui.label(
                RichText::new(format!("Free time between {} and {} (change it in Settings):", h.start.format("%H:%M"), h.end.format("%H:%M")))
                    .color(theme::INK_3),
            );
            ui.add_space(6.0);
            if proposal.is_empty() {
                ui.label("Nothing left to block out, or no free time left today.");
            }
            for (id, start) in proposal.iter() {
                if let Ok(t) = store.task(*id) {
                    let end = *start + chrono::Duration::minutes(i64::from(t.duration_min));
                    ui.label(format!("{:02}:{:02} – {}   {}", start.hour(), start.minute(), end.format("%H:%M"), t.title));
                }
            }
            let unplaced = store
                .list(&View::Today, today)
                .unwrap_or_default()
                .into_iter()
                .filter(|t| t.time.is_none() && !proposal.iter().any(|p| p.0 == t.id))
                .count();
            if unplaced > 0 {
                ui.label(RichText::new(format!("{unplaced} task(s) don't fit today. Consider moving them.")).color(theme::WARN));
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button("Skip").clicked() {
                    out = Outcome::Close;
                }
                if !proposal.is_empty() && ui.button(RichText::new("Put these on my calendar").color(theme::ACCENT)).clicked() {
                    for (id, start) in proposal.iter() {
                        if let Ok(t) = store.task(*id) {
                            let _ = store.schedule(*id, *start, t.duration_min);
                        }
                    }
                    out = Outcome::Close;
                }
            });
        }
    }
    out
}

fn shutdown_ui(ui: &mut egui::Ui, store: &Store, today: NaiveDate, note: &mut String) -> Outcome {
    let mut out = Outcome::Open;
    ui.label(RichText::new("Shutdown").size(20.0).strong());
    let done_today: Vec<Task> = store
        .list(&View::Done, today)
        .unwrap_or_default()
        .into_iter()
        .filter(|t| t.done_at.and_then(|s| chrono::DateTime::from_timestamp(s, 0)).map(|d| d.with_timezone(&chrono::Local).date_naive()) == Some(today))
        .collect();
    ui.label(RichText::new(format!("You finished {} task(s) today.", done_today.len())).color(theme::OK));
    for t in done_today.iter().take(8) {
        ui.label(RichText::new(format!("✔ {}", t.title)).color(theme::INK_3));
    }
    ui.add_space(8.0);
    let left: Vec<Task> = store.list(&View::Today, today).unwrap_or_default().into_iter().filter(|t| t.status == Status::Open).collect();
    if !left.is_empty() {
        ui.label("Still open. Where should these go?");
        egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
            for t in &left {
                ui.horizontal(|ui| {
                    ui.label(&t.title);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("Inbox").clicked() {
                            let _ = store.update(t.id, &TaskPatch { date: Some(None), time: Some(None), ..Default::default() });
                            out = Outcome::Changed;
                        }
                        if ui.small_button("Someday").clicked() {
                            let p = TaskPatch { date: Some(None), time: Some(None), someday: Some(true), ..Default::default() };
                            let _ = store.update(t.id, &p);
                            out = Outcome::Changed;
                        }
                        if ui.small_button("Tomorrow").clicked() {
                            let _ = store.update(t.id, &TaskPatch { date: Some(Some(today + Days::new(1))), time: Some(None), ..Default::default() });
                            out = Outcome::Changed;
                        }
                    });
                });
            }
        });
        if ui.button("Move all to tomorrow").clicked() {
            for t in &left {
                let _ = store.update(t.id, &TaskPatch { date: Some(Some(today + Days::new(1))), time: Some(None), ..Default::default() });
            }
            out = Outcome::Changed;
        }
    }
    ui.add_space(8.0);
    ui.label("How did today go?");
    ui.add(egui::TextEdit::multiline(note).desired_rows(3).desired_width(f32::INFINITY));
    ui.add_space(8.0);
    if ui.button(RichText::new("Done for the day").color(theme::ACCENT)).clicked() {
        let _ = store.set_setting(&format!("shutdown:{today}"), note);
        out = Outcome::Close;
    }
    out
}
