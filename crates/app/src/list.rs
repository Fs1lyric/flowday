//! The task list: rows with a check circle, the title and a line of meta. Rows only
//! report what happened; the app applies it to the store.

use crate::theme;
use chrono::{Datelike, NaiveDate};
use eframe::egui::{self, Align2, CornerRadius, FontId, Id, Pos2, Rect, Sense, Stroke, StrokeKind, Ui};
use flowday_core::model::{Project, Status, Task};

pub const ROW_H: f32 = 40.0;

/// The id of the task being dragged out of a list, kept in egui's temp memory.
pub fn drag_id() -> Id {
    Id::new("task-drag")
}

pub enum RowAction {
    Toggle(i64),
    Select(i64),
}

/// "Today", "Tomorrow", "Mon 19 Oct": how a date reads in meta lines and headings.
pub fn day_label(d: NaiveDate, today: NaiveDate) -> String {
    match (d - today).num_days() {
        0 => "Today".into(),
        1 => "Tomorrow".into(),
        -1 => "Yesterday".into(),
        n if (2..7).contains(&n) => d.format("%A").to_string(),
        _ if d.year() == today.year() => d.format("%a %-d %b").to_string(),
        _ => d.format("%a %-d %b %Y").to_string(),
    }
}

/// Splits tasks (already in date order) into runs that share a date.
pub fn group_by_date(tasks: &[Task]) -> Vec<(Option<NaiveDate>, Vec<&Task>)> {
    let mut out: Vec<(Option<NaiveDate>, Vec<&Task>)> = Vec::new();
    for t in tasks {
        match out.last_mut() {
            Some((d, run)) if *d == t.date => run.push(t),
            _ => out.push((t.date, vec![t])),
        }
    }
    out
}

/// The meta line under a title: when, how long, where it's from, labels.
pub fn meta(t: &Task, project: Option<&Project>, today: NaiveDate, show_date: bool) -> String {
    let mut parts = Vec::new();
    if let Some(p) = project {
        parts.push(format!("# {}", p.name));
    }
    match (t.date, t.time) {
        (Some(d), Some(tm)) if show_date => parts.push(format!("{} {}", day_label(d, today), tm.format("%H:%M"))),
        (_, Some(tm)) => parts.push(tm.format("%H:%M").to_string()),
        (Some(d), None) if show_date || d < today => parts.push(day_label(d, today)),
        _ => {}
    }
    if t.time.is_some() || t.duration_min != flowday_core::model::DEFAULT_DURATION_MIN {
        parts.push(duration(t.duration_min));
    }
    if let Some(dl) = t.deadline {
        parts.push(format!("due {}", day_label(dl, today)));
    }
    if t.rrule.is_some() {
        parts.push("repeats".into());
    }
    if let Some(src) = &t.source {
        parts.push(src.clone());
    }
    for l in &t.labels {
        parts.push(format!("@{l}"));
    }
    parts.join("  ·  ")
}

/// Minutes from midnight as `HH:MM`.
pub fn hm(minutes: i64) -> String {
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

pub fn duration(min: u32) -> String {
    match (min / 60, min % 60) {
        (0, m) => format!("{m}m"),
        (h, 0) => format!("{h}h"),
        (h, m) => format!("{h}h{m}"),
    }
}

/// One row. Dragging it starts a task drag the day calendar can drop.
pub fn row(ui: &mut Ui, t: &Task, project: Option<&Project>, selected: bool, today: NaiveDate, show_date: bool) -> Option<RowAction> {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW_H), Sense::click_and_drag());
    let p = ui.painter().clone();
    let fill = if selected {
        theme::SURFACE_2
    } else if resp.hovered() {
        theme::SURFACE
    } else {
        theme::BG
    };
    p.rect_filled(rect, CornerRadius::same(6), fill);

    let done = t.status == Status::Done;
    let center = Pos2::new(rect.left() + 18.0, rect.center().y);
    let check = Rect::from_center_size(center, egui::vec2(22.0, 22.0));
    let check_resp = ui.interact(check, Id::new(("check", t.id)), Sense::click());
    let ring = theme::priority(t.priority).unwrap_or(theme::INK_3);
    if done {
        p.circle_filled(center, 8.0, theme::ACCENT);
        p.text(center, Align2::CENTER_CENTER, "✔", FontId::proportional(10.0), theme::BG);
    } else {
        p.circle_stroke(center, 8.0, Stroke::new(if check_resp.hovered() { 2.0 } else { 1.4 }, ring));
    }

    let left = rect.left() + 36.0;
    let clip = p.with_clip_rect(Rect::from_min_max(Pos2::new(left, rect.top()), rect.right_bottom() - egui::vec2(8.0, 0.0)));
    let title_color = if done { theme::INK_3 } else { theme::INK };
    let meta = meta(t, project, today, show_date);
    let title_y = if meta.is_empty() { rect.center().y } else { rect.top() + 12.0 };
    clip.text(Pos2::new(left, title_y), Align2::LEFT_CENTER, &t.title, FontId::proportional(14.0), title_color);
    if !meta.is_empty() {
        let color = match t.date {
            Some(d) if d < today && !done => theme::DANGER,
            _ => theme::INK_3,
        };
        clip.text(Pos2::new(left, rect.top() + 29.0), Align2::LEFT_CENTER, meta, FontId::proportional(11.0), color);
    }
    if let Some(proj) = project {
        let dot = Pos2::new(rect.right() - 14.0, rect.center().y);
        p.circle_filled(dot, 4.0, theme::hex(&proj.color));
    }

    if resp.drag_started() {
        ui.data_mut(|d| d.insert_temp(drag_id(), t.id));
    }
    if ui.data(|d| d.get_temp::<i64>(drag_id())) == Some(t.id) {
        p.rect_stroke(rect, CornerRadius::same(6), Stroke::new(1.0, theme::ACCENT), StrokeKind::Inside);
    }
    if check_resp.clicked() {
        Some(RowAction::Toggle(t.id))
    } else if resp.clicked() {
        Some(RowAction::Select(t.id))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, m, day).unwrap()
    }

    #[test]
    fn dates_read_relative_to_today() {
        let today = d(10, 16);
        assert_eq!(day_label(today, today), "Today");
        assert_eq!(day_label(d(10, 17), today), "Tomorrow");
        assert_eq!(day_label(d(10, 19), today), "Monday");
        assert_eq!(day_label(d(11, 3), today), "Tue 3 Nov");
    }

    #[test]
    fn durations_are_short() {
        assert_eq!(duration(30), "30m");
        assert_eq!(duration(60), "1h");
        assert_eq!(duration(90), "1h30");
    }
}
