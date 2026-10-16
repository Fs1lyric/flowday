//! The daily rituals: planning in the morning (pick today's tasks, then let the
//! scheduler block them into free time) and the shutdown at night (move what is
//! left, write a line about the day).

use crate::{list, theme};
use chrono::{Days, NaiveDate, NaiveDateTime, NaiveTime, Timelike};
use eframe::egui::{self, Id, RichText};
use flowday_core::model::{Status, Task, TaskPatch, View};
use flowday_core::schedule::{auto_schedule, free_slots, Hours, Slot, Want};
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

/// Work hours from the `hours` setting (`09:00-18:00`).
pub fn hours(store: &Store) -> Hours {
    let parse = |s: &str| NaiveTime::parse_from_str(s.trim(), "%H:%M").ok();
    store
        .setting("hours")
        .ok()
        .flatten()
        .and_then(|v| {
            let (a, b) = v.split_once('-')?;
            Some(Hours { start: parse(a)?, end: parse(b)? })
        })
        .unwrap_or_default()
}

pub fn buffer(store: &Store) -> u32 {
    store.setting("buffer_min").ok().flatten().and_then(|v| v.parse().ok()).unwrap_or(5)
}

/// Where today's unscheduled tasks would go: free time is work hours minus events
/// and blocks already on the calendar, from now on.
pub fn propose(store: &Store, now: NaiveDateTime) -> flowday_core::store::Result<Vec<(i64, NaiveDateTime)>> {
    let day = now.date();
    let mut busy: Vec<Slot> = store.events_on(day)?.iter().filter(|e| !e.all_day).map(|e| Slot::new(e.start, e.end)).collect();
    let today = store.list(&View::Today, day)?;
    for t in store.blocks_on(day)? {
        if let Some(start) = t.start() {
            busy.push(Slot::new(start, start + chrono::Duration::minutes(i64::from(t.duration_min))));
        }
    }
    let free = free_slots(day, hours(store), &busy, Some(now));
    let wants: Vec<Want> = today
        .iter()
        .filter(|t| t.time.is_none())
        .map(|t| Want { id: t.id, minutes: t.duration_min, priority: t.priority, deadline: t.deadline })
        .collect();
    Ok(auto_schedule(&wants, &free, buffer(store)))
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

#[cfg(test)]
mod tests {
    use super::*;
    use flowday_core::model::NewTask;

    #[test]
    fn proposal_fills_free_time_around_events_and_blocks() {
        let store = Store::open_in_memory().unwrap();
        store.set_setting("hours", "09:00-12:00").unwrap();
        store.set_setting("buffer_min", "0").unwrap();
        let day = NaiveDate::from_ymd_opt(2026, 10, 16).unwrap();
        let at = |h, m| day.and_time(NaiveTime::from_hms_opt(h, m, 0).unwrap());
        let blocked = store.add_task(&NewTask { title: "standup".into(), date: Some(day), ..Default::default() }).unwrap();
        store.schedule(blocked.id, at(9, 0), 30).unwrap();
        let a = store.add_task(&NewTask { title: "a".into(), date: Some(day), duration_min: Some(60), ..Default::default() }).unwrap();
        let b = store.add_task(&NewTask { title: "b".into(), date: Some(day), priority: 3, ..Default::default() }).unwrap();
        let plan = propose(&store, at(8, 0)).unwrap();
        assert_eq!(plan, vec![(b.id, at(9, 30)), (a.id, at(10, 0))]);
    }

    #[test]
    fn hours_fall_back_to_nine_to_six() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(hours(&store), Hours::default());
        store.set_setting("hours", "08:30-16:00").unwrap();
        assert_eq!(hours(&store).start, NaiveTime::from_hms_opt(8, 30, 0).unwrap());
    }
}
