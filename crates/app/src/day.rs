//! The day calendar beside the list: Google Calendar events and time-blocked tasks
//! on one column of hours. Tasks dragged from the list drop onto it as blocks, and
//! blocks drag to move.

use crate::list::{self, drag_id};
use crate::theme;
use chrono::{Duration, NaiveDate, NaiveDateTime, NaiveTime, Timelike};
use eframe::egui::{self, Align2, CornerRadius, FontId, Id, Pos2, Rect, Sense, Stroke, StrokeKind, Ui};
use flowday_core::model::{Event, Status, Task};

pub const HOUR_PX: f32 = 52.0;
const GUTTER: f32 = 46.0;
const SNAP: i64 = 15;

pub enum Action {
    Select(i64),
    /// Put this task on the calendar at `start`.
    Schedule { id: i64, start: NaiveDateTime },
    /// Change a block's length.
    Resize { id: i64, minutes: u32 },
}

#[derive(Clone, Copy)]
enum Drag {
    Move { id: i64, grab: i64 },
    Resize { id: i64 },
}

fn font(size: f32) -> FontId {
    FontId::proportional(size)
}

/// Minutes from midnight under a y offset, snapped down.
pub fn minute_at(y: f32) -> i64 {
    let m = (y / HOUR_PX * 60.0) as i64;
    (m.div_euclid(SNAP) * SNAP).clamp(0, 24 * 60 - SNAP)
}

/// Overlapping spans share the width: each gets a column and the column count of
/// its cluster.
pub fn pack(spans: &[(i64, i64)]) -> Vec<(usize, usize)> {
    let mut order: Vec<usize> = (0..spans.len()).collect();
    order.sort_by_key(|&i| spans[i]);
    let mut out = vec![(0, 1); spans.len()];
    let mut cluster: Vec<usize> = Vec::new();
    let mut ends: Vec<i64> = Vec::new();
    let mut cluster_end = i64::MIN;
    let close = |cluster: &mut Vec<usize>, ends: &mut Vec<i64>, out: &mut Vec<(usize, usize)>| {
        for &i in cluster.iter() {
            out[i].1 = ends.len();
        }
        cluster.clear();
        ends.clear();
    };
    for i in order {
        let (a, b) = spans[i];
        if a >= cluster_end {
            close(&mut cluster, &mut ends, &mut out);
        }
        let col = match ends.iter().position(|&e| e <= a) {
            Some(c) => c,
            None => {
                ends.push(0);
                ends.len() - 1
            }
        };
        ends[col] = b;
        out[i].0 = col;
        cluster.push(i);
        cluster_end = cluster_end.max(b);
    }
    close(&mut cluster, &mut ends, &mut out);
    out
}

pub fn show(
    ui: &mut Ui,
    day: NaiveDate,
    events: &[Event],
    blocks: &[Task],
    selected: Option<i64>,
    now: NaiveDateTime,
    scroll_to: Option<f32>,
) -> Option<Action> {
    let mut action = None;
    let midnight = day.and_time(NaiveTime::MIN);

    // All-day events above the hours.
    for e in events.iter().filter(|e| e.all_day) {
        let (r, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 22.0), Sense::hover());
        ui.painter().rect_filled(r, CornerRadius::same(4), theme::soft(theme::EVENT));
        ui.painter().text(r.left_center() + egui::vec2(6.0, 0.0), Align2::LEFT_CENTER, &e.title, font(12.0), theme::INK);
    }

    let mut area = egui::ScrollArea::vertical().id_salt("day").auto_shrink([false, false]);
    if let Some(y) = scroll_to {
        area = area.vertical_scroll_offset(y);
    }
    area.show(ui, |ui| {
        let width = ui.available_width();
        let (body, _) = ui.allocate_exact_size(egui::vec2(width, 24.0 * HOUR_PX), Sense::hover());
        let lane = Rect::from_min_max(Pos2::new(body.left() + GUTTER, body.top()), body.right_bottom());
        let p = ui.painter().clone();
        let y_of = |m: i64| body.top() + m as f32 / 60.0 * HOUR_PX;
        for h in 0..24 {
            let y = y_of(h * 60);
            p.line_segment([Pos2::new(lane.left(), y), Pos2::new(lane.right(), y)], Stroke::new(0.5, theme::LINE));
            if h > 0 {
                p.text(Pos2::new(lane.left() - 8.0, y), Align2::RIGHT_CENTER, format!("{h:02}:00"), font(10.5), theme::INK_3);
            }
        }

        // Everything timed, packed side by side.
        enum Item<'a> {
            Ev(&'a Event),
            Block(&'a Task),
        }
        let mut items: Vec<(Item, (i64, i64))> = Vec::new();
        for e in events.iter().filter(|e| !e.all_day) {
            let a = (e.start.max(midnight) - midnight).num_minutes();
            let b = (e.end.min(midnight + Duration::days(1)) - midnight).num_minutes();
            if b > a {
                items.push((Item::Ev(e), (a, b)));
            }
        }
        for t in blocks {
            if let Some(tm) = t.time {
                let a = i64::from(tm.hour() * 60 + tm.minute());
                items.push((Item::Block(t), (a, a + i64::from(t.duration_min.max(15)))));
            }
        }
        let placed = pack(&items.iter().map(|i| i.1).collect::<Vec<_>>());
        let pointer = ui.ctx().pointer_interact_pos();
        let drag: Option<Drag> = ui.data(|d| d.get_temp(Id::new("block-drag")));

        for ((item, (a, b)), (col, cols)) in items.iter().zip(&placed) {
            let w = lane.width() / *cols as f32;
            let r = Rect::from_min_size(
                Pos2::new(lane.left() + w * *col as f32 + 1.0, y_of(*a) + 1.0),
                egui::vec2(w - 4.0, ((b - a) as f32 / 60.0 * HOUR_PX - 2.0).max(16.0)),
            );
            let clip = p.with_clip_rect(r.intersect(ui.clip_rect()));
            match item {
                Item::Ev(e) => {
                    clip.rect_filled(r, CornerRadius::same(4), theme::soft(theme::EVENT));
                    clip.rect_filled(Rect::from_min_size(r.min, egui::vec2(3.0, r.height())), CornerRadius::same(2), theme::EVENT);
                    clip.text(r.left_top() + egui::vec2(7.0, 3.0), Align2::LEFT_TOP, &e.title, font(12.0), theme::INK);
                    ui.interact(r, Id::new(("event", &e.id)), Sense::hover())
                        .on_hover_text(format!("{}\n{} – {}", e.title, e.start.format("%H:%M"), e.end.format("%H:%M")));
                }
                Item::Block(t) => {
                    let done = t.status == Status::Done;
                    let moving = matches!(drag, Some(Drag::Move { id, .. }) if id == t.id);
                    let fill = if done { theme::SURFACE } else { theme::soft(theme::ACCENT) };
                    clip.rect_filled(r, CornerRadius::same(4), fill);
                    if Some(t.id) == selected {
                        clip.rect_stroke(r, CornerRadius::same(4), Stroke::new(1.5, theme::ACCENT), StrokeKind::Inside);
                    }
                    let ink = if done || moving { theme::INK_3 } else { theme::INK };
                    clip.text(r.left_top() + egui::vec2(7.0, 3.0), Align2::LEFT_TOP, &t.title, font(12.0), ink);
                    if r.height() > 34.0 {
                        let end = t.time.unwrap() + Duration::minutes(i64::from(t.duration_min));
                        let span = format!("{} – {}", t.time.unwrap().format("%H:%M"), end.format("%H:%M"));
                        clip.text(r.left_top() + egui::vec2(7.0, 19.0), Align2::LEFT_TOP, span, font(10.5), theme::INK_2);
                    }
                    let edge = Rect::from_min_max(Pos2::new(r.left(), r.bottom() - 5.0), r.right_bottom());
                    let resp = ui.interact(r, Id::new(("block", t.id)), Sense::click_and_drag());
                    let edge_resp = ui.interact(edge, Id::new(("block-edge", t.id)), Sense::drag());
                    if edge_resp.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
                    }
                    if resp.clicked() {
                        action = Some(Action::Select(t.id));
                    }
                    if edge_resp.drag_started() {
                        ui.data_mut(|d| d.insert_temp(Id::new("block-drag"), Drag::Resize { id: t.id }));
                    } else if resp.drag_started() {
                        if let Some(pos) = resp.interact_pointer_pos() {
                            let grab = minute_at(pos.y - body.top()) - a;
                            ui.data_mut(|d| d.insert_temp(Id::new("block-drag"), Drag::Move { id: t.id, grab }));
                        }
                    }
                }
            }
        }

        let released = ui.input(|i| i.pointer.any_released());
        let ghost = |start: i64, minutes: i64, label: String| {
            let r = Rect::from_min_size(
                Pos2::new(lane.left() + 1.0, y_of(start)),
                egui::vec2(lane.width() - 4.0, (minutes as f32 / 60.0 * HOUR_PX).max(16.0)),
            );
            p.rect_stroke(r, CornerRadius::same(4), Stroke::new(1.5, theme::ACCENT), StrokeKind::Inside);
            p.text(r.left_top() + egui::vec2(7.0, 3.0), Align2::LEFT_TOP, label, font(11.0), theme::INK);
        };

        // A block being moved or stretched.
        if let (Some(d), Some(pos)) = (drag, pointer) {
            match d {
                Drag::Move { id, grab } => {
                    let t = blocks.iter().find(|t| t.id == id);
                    let minutes = t.map_or(30, |t| i64::from(t.duration_min));
                    let start = (minute_at(pos.y - body.top()) - grab).clamp(0, 24 * 60 - SNAP);
                    ghost(start, minutes, list::hm(start));
                    if released {
                        if lane.contains(pos) {
                            action = Some(Action::Schedule { id, start: midnight + Duration::minutes(start) });
                        }
                        ui.data_mut(|d| d.remove::<Drag>(Id::new("block-drag")));
                    }
                }
                Drag::Resize { id } => {
                    if let Some(t) = blocks.iter().find(|t| t.id == id) {
                        let a = i64::from(t.time.map_or(0, |tm| tm.hour() * 60 + tm.minute()));
                        let end = (minute_at(pos.y - body.top()) + SNAP).max(a + SNAP);
                        ghost(a, end - a, list::duration((end - a) as u32));
                        if released {
                            action = Some(Action::Resize { id, minutes: (end - a) as u32 });
                        }
                    }
                    if released {
                        ui.data_mut(|d| d.remove::<Drag>(Id::new("block-drag")));
                    }
                }
            }
        }

        // A task dragged in from the list.
        if let (Some(id), Some(pos)) = (ui.data(|d| d.get_temp::<i64>(drag_id())), pointer) {
            if lane.contains(pos) && ui.clip_rect().contains(pos) {
                let start = minute_at(pos.y - body.top());
                ghost(start, 30, list::hm(start));
                if released {
                    action = Some(Action::Schedule { id, start: midnight + Duration::minutes(start) });
                }
            }
        }

        if now.date() == day {
            let y = y_of(i64::from(now.hour() * 60 + now.minute()));
            p.line_segment([Pos2::new(lane.left(), y), Pos2::new(lane.right(), y)], Stroke::new(1.5, theme::DANGER));
            p.circle_filled(Pos2::new(lane.left(), y), 3.5, theme::DANGER);
        }
    });
    action
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minutes_snap_to_quarter_hours() {
        assert_eq!(minute_at(HOUR_PX * 9.0), 540);
        assert_eq!(minute_at(HOUR_PX * 9.4), 555);
        assert_eq!(minute_at(-5.0), 0);
    }

    #[test]
    fn overlaps_share_the_width() {
        let placed = pack(&[(540, 600), (570, 630), (700, 730)]);
        assert_eq!(placed, vec![(0, 2), (1, 2), (0, 1)]);
    }
}
