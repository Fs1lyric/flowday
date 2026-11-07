//! Flowday, the desktop app.

mod chat;
mod day;
mod detail;
mod list;
mod palette;
mod rituals;
mod secrets;
mod settings;
mod theme;

use chrono::{Days, Local, NaiveDate, NaiveDateTime};
use detail::Detail;
use eframe::egui::{self, Key, RichText};
use flowday_core::model::{Counts, Event, Project, Task, View};
use flowday_core::quickadd;
use flowday_core::store::Store;
use palette::{Command, Palette};
use rituals::Ritual;
use settings::Settings;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};

type SyncReport = Vec<(&'static str, Result<String, String>)>;

struct App {
    store: Store,
    view: View,
    tasks: Vec<Task>,
    counts: Counts,
    projects: Vec<Project>,
    labels: Vec<String>,
    /// The day shown on the calendar.
    day: NaiveDate,
    events: Vec<Event>,
    blocks: Vec<Task>,
    detail: Option<Detail>,
    quick: String,
    focus_quick: bool,
    palette: Option<Palette>,
    ritual: Option<Ritual>,
    new_project: Option<String>,
    chat: chat::Chat,
    settings: Option<Settings>,
    sync_rx: Option<Receiver<SyncReport>>,
    sync_now: bool,
    last_sync: Option<std::time::Instant>,
    sync_report: SyncReport,
    stale: bool,
    loaded_at: std::time::Instant,
    scroll_to: Option<f32>,
    error: Option<String>,
}

pub fn now() -> NaiveDateTime {
    Local::now().naive_local()
}

pub fn today() -> NaiveDate {
    now().date()
}

fn data_path() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share"));
    base.join("flowday/flowday.db")
}

fn view_title(view: &View, projects: &[Project]) -> String {
    match view {
        View::Inbox => "Inbox".into(),
        View::Today => "Today".into(),
        View::Upcoming => "Upcoming".into(),
        View::Someday => "Someday".into(),
        View::Done => "Done".into(),
        View::Project(id) => projects.iter().find(|p| p.id == *id).map_or("Project".into(), |p| p.name.clone()),
        View::Label(l) => format!("@{l}"),
    }
}

impl App {
    fn new(store: Store) -> Self {
        let today = today();
        App {
            store,
            view: View::Today,
            tasks: Vec::new(),
            counts: Counts::default(),
            projects: Vec::new(),
            labels: Vec::new(),
            day: today,
            events: Vec::new(),
            blocks: Vec::new(),
            detail: None,
            quick: String::new(),
            focus_quick: false,
            palette: None,
            ritual: None,
            new_project: None,
            chat: chat::Chat::new(),
            settings: None,
            sync_rx: None,
            sync_now: false,
            last_sync: None,
            sync_report: Vec::new(),
            stale: true,
            loaded_at: std::time::Instant::now(),
            scroll_to: Some(8.0 * day::HOUR_PX),
            error: None,
        }
    }

    fn report<T>(&mut self, r: flowday_core::store::Result<T>) -> Option<T> {
        match r {
            Ok(v) => {
                self.stale = true;
                Some(v)
            }
            Err(e) => {
                self.error = Some(e.to_string());
                None
            }
        }
    }

    /// Reloads after a change, and every 20 s so the AI and sync writes show up.
    fn reload(&mut self) {
        if !self.stale && self.loaded_at.elapsed().as_secs() < 20 {
            return;
        }
        let today = today();
        let s = &self.store;
        let result = (|| -> flowday_core::store::Result<()> {
            self.tasks = s.list(&self.view, today)?;
            self.counts = s.counts(today)?;
            self.projects = s.projects()?;
            self.labels = s.labels()?;
            self.events = s.events_on(self.day)?;
            self.blocks = s.blocks_on(self.day)?;
            Ok(())
        })();
        if let Err(e) = result {
            self.error = Some(e.to_string());
        }
        self.stale = false;
        self.loaded_at = std::time::Instant::now();
    }

    fn go(&mut self, view: View) {
        self.view = view;
        self.stale = true;
    }

    fn open(&mut self, id: i64) {
        if let Ok(t) = self.store.task(id) {
            self.detail = Some(Detail::new(&t));
        }
    }

    fn add_quick(&mut self, text: &str) {
        if text.trim().is_empty() {
            return;
        }
        let mut q = quickadd::parse(text, today());
        // New tasks land where you are looking.
        match &self.view {
            View::Today if q.date.is_none() && !q.someday => q.date = Some(today()),
            View::Someday => q.someday = true,
            View::Label(l) if !q.labels.contains(l) => q.labels.push(l.clone()),
            _ => {}
        }
        let project = match &self.view {
            View::Project(id) if q.project.is_none() => Some(*id),
            _ => None,
        };
        if let Some(t) = self.report(self.store.add_quick(&q)) {
            if let Some(pid) = project {
                let patch = flowday_core::model::TaskPatch { project_id: Some(Some(pid)), ..Default::default() };
                self.report(self.store.update(t.id, &patch));
            }
        }
    }

    fn run(&mut self, cmd: Command) {
        match cmd {
            Command::Go(v) => self.go(v),
            Command::Open(id) => self.open(id),
            Command::Add(text) => self.add_quick(&text),
            Command::PlanDay => self.ritual = Some(Ritual::plan(&self.store, today())),
            Command::Shutdown => self.ritual = Some(Ritual::shutdown(&self.store, today())),
            Command::Chat => self.chat.focus(),
            Command::Sync => self.sync_now = true,
            Command::Settings => self.settings = Some(Settings::new(&self.store)),
            Command::NewProject(name) => {
                let color = theme::SWATCHES[self.projects.len() % theme::SWATCHES.len()];
                if let Some(p) = self.report(self.store.create_project(&name, color)) {
                    self.go(View::Project(p.id));
                }
            }
        }
    }

    fn keys(&mut self, ctx: &egui::Context) {
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, Key::K)) {
            self.palette = Some(Palette::default());
        }
        if ctx.input(|i| i.key_pressed(Key::Escape)) && self.palette.is_none() && self.ritual.is_none() {
            self.detail = None;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, Key::J)) {
            if self.chat.open {
                self.chat.open = false;
            } else {
                self.chat.focus();
            }
        }
        if self.palette.is_some() || self.ritual.is_some() || self.settings.is_some() || ctx.memory(|m| m.focused().is_some()) {
            return;
        }
        let pressed = |k| ctx.input(|i| i.key_pressed(k));
        let views = [(Key::Num1, View::Inbox), (Key::Num2, View::Today), (Key::Num3, View::Upcoming), (Key::Num4, View::Someday), (Key::Num5, View::Done)];
        for (k, v) in views {
            if pressed(k) {
                self.go(v);
            }
        }
        if pressed(Key::N) || pressed(Key::Q) {
            self.focus_quick = true;
        }
        if pressed(Key::P) {
            self.run(Command::PlanDay);
        }
        if pressed(Key::ArrowLeft) {
            self.shift_day(-1);
        }
        if pressed(Key::ArrowRight) {
            self.shift_day(1);
        }
        if pressed(Key::T) {
            self.day = today();
            self.stale = true;
        }
    }

    fn shift_day(&mut self, by: i64) {
        self.day = if by < 0 { self.day - Days::new(by.unsigned_abs()) } else { self.day + Days::new(by as u64) };
        self.stale = true;
    }

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(12.0);
        ui.label(RichText::new("Flowday").strong().color(theme::ACCENT).size(18.0));
        ui.add_space(10.0);
        let c = self.counts;
        let items = [
            ("📥  Inbox", View::Inbox, c.inbox),
            ("☀  Today", View::Today, c.today),
            ("📅  Upcoming", View::Upcoming, c.upcoming),
            ("☁  Someday", View::Someday, c.someday),
            ("✔  Done", View::Done, 0),
        ];
        for (name, v, n) in items {
            self.nav(ui, name, v, n);
        }
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("PROJECTS").size(11.0).color(theme::INK_3));
            if ui.small_button("+").on_hover_text("New project").clicked() {
                self.new_project = Some(String::new());
            }
        });
        if let Some(name) = self.new_project.as_mut() {
            let r = ui.add(egui::TextEdit::singleline(name).hint_text("Project name").desired_width(f32::INFINITY));
            r.request_focus();
            if r.lost_focus() {
                let name = std::mem::take(name);
                self.new_project = None;
                if ui.input(|i| i.key_pressed(Key::Enter)) && !name.trim().is_empty() {
                    self.run(Command::NewProject(name.trim().to_string()));
                }
            }
        }
        for p in self.projects.clone() {
            let selected = self.view == View::Project(p.id);
            let r = ui.selectable_label(selected, RichText::new(format!("     {}", p.name)).color(if selected { theme::INK } else { theme::INK_2 }));
            let dot = r.rect.left_center() + egui::vec2(10.0, 0.0);
            ui.painter().circle_filled(dot, 4.0, theme::hex(&p.color));
            if r.clicked() {
                self.go(View::Project(p.id));
            }
            r.context_menu(|ui| {
                if ui.button("Delete project").clicked() {
                    self.report(self.store.delete_project(p.id));
                    if selected {
                        self.go(View::Inbox);
                    }
                    ui.close();
                }
            });
        }
        if !self.labels.is_empty() {
            ui.add_space(12.0);
            ui.label(RichText::new("LABELS").size(11.0).color(theme::INK_3));
            for l in self.labels.clone() {
                if ui.selectable_label(self.view == View::Label(l.clone()), format!("@{l}")).clicked() {
                    self.go(View::Label(l));
                }
            }
        }
        ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
            ui.add_space(10.0);
            ui.label(RichText::new("Ctrl K for everything · Ctrl J for Claude").size(11.0).color(theme::INK_3));
            if self.sync_rx.is_some() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new("Syncing…").size(11.0).color(theme::INK_3));
                });
            } else if !self.sync_report.is_empty() {
                let failed: Vec<String> = self.sync_report.iter().filter_map(|(n, r)| r.as_ref().err().map(|e| format!("{n}: {e}"))).collect();
                let ok: Vec<String> = self.sync_report.iter().filter_map(|(n, r)| r.as_ref().ok().map(|m| format!("{n}: {m}"))).collect();
                let (text, color) = if failed.is_empty() { ("Synced ✔", theme::INK_3) } else { ("Sync problem ⚠", theme::WARN) };
                let r = ui.add(egui::Label::new(RichText::new(text).size(11.0).color(color)).sense(egui::Sense::click()));
                if r.on_hover_text([ok, failed].concat().join("\n")).clicked() {
                    self.sync_now = true;
                }
            }
            if ui.button("⚙  Settings").clicked() {
                self.run(Command::Settings);
            }
            if ui.button("✨  Ask Claude").clicked() {
                self.chat.focus();
            }
            if ui.button("🌙  Shutdown").clicked() {
                self.run(Command::Shutdown);
            }
            if ui.button("🗓  Plan my day").clicked() {
                self.run(Command::PlanDay);
            }
        });
    }

    fn nav(&mut self, ui: &mut egui::Ui, name: &str, v: View, n: usize) {
        let selected = self.view == v;
        let (rect, r) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 30.0), egui::Sense::click());
        if selected || r.hovered() {
            ui.painter().rect_filled(rect, 6.0, if selected { theme::SURFACE_2 } else { theme::SURFACE });
        }
        let ink = if selected { theme::INK } else { theme::INK_2 };
        ui.painter().text(rect.left_center() + egui::vec2(10.0, 0.0), egui::Align2::LEFT_CENTER, name, egui::FontId::proportional(14.0), ink);
        if n > 0 {
            ui.painter().text(rect.right_center() - egui::vec2(10.0, 0.0), egui::Align2::RIGHT_CENTER, n, egui::FontId::proportional(12.0), theme::INK_3);
        }
        if r.clicked() {
            self.go(v);
        }
    }

    fn main_list(&mut self, ui: &mut egui::Ui) {
        let today = today();
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(view_title(&self.view, &self.projects)).size(22.0).strong());
            if self.view == View::Today {
                ui.label(RichText::new(today.format("%A %-d %B").to_string()).color(theme::INK_3));
            }
        });
        ui.add_space(8.0);
        if self.view != View::Done {
            let edit = ui.add(
                egui::TextEdit::singleline(&mut self.quick)
                    .hint_text("+  Add a task: Ship release #Work @deep !1 tomorrow 10am 1h")
                    .desired_width(f32::INFINITY)
                    .margin(egui::vec2(10.0, 8.0)),
            );
            if std::mem::take(&mut self.focus_quick) {
                edit.request_focus();
            }
            if !self.quick.trim().is_empty() && ui.small_button("✨ Let Claude add this").on_hover_text("For anything the quick-add syntax can't say").clicked() {
                let text = std::mem::take(&mut self.quick);
                self.chat.ask(ui.ctx(), &self.store, data_path(), format!("Add this to my tasks: {text}"));
            }
            if edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                let text = std::mem::take(&mut self.quick);
                self.add_quick(&text);
                edit.request_focus();
            }
            if !self.quick.trim().is_empty() {
                let q = quickadd::parse(&self.quick, today);
                let mut bits = Vec::new();
                if let Some(d) = q.date {
                    bits.push(list::day_label(d, today));
                }
                if let Some(t) = q.time {
                    bits.push(t.format("%H:%M").to_string());
                }
                if let Some(m) = q.duration_min {
                    bits.push(list::duration(m));
                }
                if let Some(p) = &q.project {
                    bits.push(format!("#{p}"));
                }
                bits.extend(q.labels.iter().map(|l| format!("@{l}")));
                if q.priority > 0 {
                    bits.push(format!("!{}", 4 - q.priority));
                }
                if q.rrule.is_some() {
                    bits.push("repeats".into());
                }
                if q.someday {
                    bits.push("someday".into());
                }
                if !bits.is_empty() {
                    ui.label(RichText::new(format!("{}  →  {}", q.title, bits.join("  ·  "))).size(12.0).color(theme::ACCENT));
                }
            }
        }
        ui.add_space(6.0);

        let selected = self.detail.as_ref().map(|d| d.id);
        let mut action = None;
        let grouped = matches!(self.view, View::Upcoming | View::Done);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            if self.tasks.is_empty() {
                ui.add_space(30.0);
                let empty = match self.view {
                    View::Inbox => "Inbox zero. Nice.",
                    View::Today => "Nothing planned. Press P to plan your day.",
                    _ => "Nothing here.",
                };
                ui.vertical_centered(|ui| ui.label(RichText::new(empty).color(theme::INK_3)));
            }
            if grouped && self.view == View::Upcoming {
                for (date, run) in list::group_by_date(&self.tasks) {
                    if let Some(d) = date {
                        ui.add_space(8.0);
                        ui.label(RichText::new(list::day_label(d, today)).strong().color(theme::INK_2));
                    }
                    for t in run {
                        let p = self.projects.iter().find(|p| Some(p.id) == t.project_id);
                        if let Some(a) = list::row(ui, t, p, Some(t.id) == selected, today, false) {
                            action = Some(a);
                        }
                    }
                }
            } else {
                let show_date = !matches!(self.view, View::Today);
                for t in &self.tasks {
                    let p = self.projects.iter().find(|p| Some(p.id) == t.project_id);
                    if let Some(a) = list::row(ui, t, p, Some(t.id) == selected, today, show_date) {
                        action = Some(a);
                    }
                }
            }
        });
        match action {
            Some(list::RowAction::Toggle(id)) => {
                let done = self.tasks.iter().any(|t| t.id == id && t.status == flowday_core::model::Status::Done);
                if done {
                    self.report(self.store.reopen(id));
                } else {
                    self.report(self.store.complete(id, today));
                }
            }
            Some(list::RowAction::Select(id)) => self.open(id),
            None => {}
        }
    }

    /// Runs the integrations on a worker every ten minutes, or when asked.
    fn sync(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.sync_rx {
            if let Ok(report) = rx.try_recv() {
                self.sync_report = report;
                self.sync_rx = None;
                self.stale = true;
            }
            return;
        }
        let due = self.last_sync.is_none_or(|t| t.elapsed().as_secs() >= 600);
        if !(std::mem::take(&mut self.sync_now) || due) {
            return;
        }
        self.last_sync = Some(std::time::Instant::now());
        if !settings::any_integration(&self.store) {
            return;
        }
        let cfg = settings::sync_config(&self.store);
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let report = match Store::open(&data_path()) {
                Ok(store) => flowday_sync::run_all(&store, &cfg, now()),
                Err(e) => vec![("Sync", Err(e.to_string()))],
            };
            let _ = tx.send(report);
            ctx.request_repaint();
        });
        self.sync_rx = Some(rx);
    }

    fn calendar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            if ui.button("‹").clicked() {
                self.shift_day(-1);
            }
            if ui.button("Today").clicked() {
                self.day = today();
                self.stale = true;
            }
            if ui.button("›").clicked() {
                self.shift_day(1);
            }
            ui.label(RichText::new(self.day.format("%a %-d %b").to_string()).strong());
        });
        ui.add_space(6.0);
        let selected = self.detail.as_ref().map(|d| d.id);
        let action = day::show(ui, self.day, &self.events, &self.blocks, selected, now(), self.scroll_to.take());
        match action {
            Some(day::Action::Select(id)) => self.open(id),
            Some(day::Action::Schedule { id, start }) => {
                let minutes = self.store.task(id).map(|t| t.duration_min).unwrap_or(30);
                self.report(self.store.schedule(id, start, minutes));
            }
            Some(day::Action::Resize { id, minutes }) => {
                let patch = flowday_core::model::TaskPatch { duration_min: Some(minutes), ..Default::default() };
                self.report(self.store.update(id, &patch));
            }
            None => {}
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.keys(&ctx);
        self.reload();
        ctx.request_repaint_after(std::time::Duration::from_secs(20));

        egui::Panel::left("sidebar")
            .resizable(false)
            .exact_size(210.0)
            .frame(egui::Frame::new().fill(theme::SIDEBAR).inner_margin(egui::Margin::symmetric(12, 0)))
            .show(ui, |ui| self.sidebar(ui));

        if self.chat.poll() {
            self.stale = true;
        }
        if self.chat.open {
            egui::Panel::right("chat")
                .resizable(true)
                .default_size(360.0)
                .frame(egui::Frame::new().fill(theme::SURFACE).inner_margin(egui::Margin::symmetric(12, 0)))
                .show(ui, |ui| self.chat.show(ui, &self.store, data_path()));
        }
        self.sync(&ctx);
        let wide = ctx.content_rect().width() > 1000.0 + if self.chat.open { 360.0 } else { 0.0 };
        if wide || self.detail.is_none() {
            egui::Panel::right("calendar")
                .resizable(true)
                .default_size(300.0)
                .frame(egui::Frame::new().fill(theme::BG).inner_margin(egui::Margin::symmetric(10, 0)))
                .show(ui, |ui| self.calendar(ui));
        }
        if let Some(d) = self.detail.as_mut() {
            let out = egui::Panel::right("detail")
                .resizable(true)
                .default_size(340.0)
                .frame(egui::Frame::new().fill(theme::SURFACE).inner_margin(egui::Margin::same(14)))
                .show(ui, |ui| egui::ScrollArea::vertical().show(ui, |ui| d.show(ui, &self.store, &self.projects, today())).inner)
                .inner;
            match out {
                detail::Outcome::Nothing => {}
                detail::Outcome::Changed => self.stale = true,
                detail::Outcome::Ask(text) => self.chat.ask(&ctx, &self.store, data_path(), text),
                detail::Outcome::Close => self.detail = None,
                detail::Outcome::Deleted => {
                    self.detail = None;
                    self.stale = true;
                }
            }
        }
        if let Some(e) = self.error.clone() {
            egui::Panel::bottom("error").show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(e).color(theme::DANGER));
                    if ui.small_button("Dismiss").clicked() {
                        self.error = None;
                    }
                });
            });
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::BG).inner_margin(egui::Margin::symmetric(18, 0)))
            .show(ui, |ui| self.main_list(ui));

        // A task drag ends wherever the pointer is let go.
        if ctx.input(|i| i.pointer.any_released()) {
            ctx.data_mut(|d| d.remove::<i64>(list::drag_id()));
        }

        if let Some(p) = self.palette.as_mut() {
            let store = &self.store;
            if let Some(chosen) = p.show(&ctx, &self.projects, &self.labels, |q| store.search(q).unwrap_or_default()) {
                self.palette = None;
                if let Some(cmd) = chosen {
                    self.run(cmd);
                }
            }
        }
        if let Some(s) = self.settings.as_mut() {
            if s.show(&ctx, &self.store) {
                self.settings = None;
                self.sync_now = true;
            }
        }
        if let Some(r) = self.ritual.as_mut() {
            match r.show(&ctx, &self.store, now()) {
                rituals::Outcome::Open => {}
                rituals::Outcome::Changed => self.stale = true,
                rituals::Outcome::Close => {
                    self.ritual = None;
                    self.stale = true;
                }
            }
        }
    }
}

const USAGE: &str = "usage: flowday [--plan] [--add <text>]";

fn main() -> eframe::Result {
    let mut plan = false;
    let mut add: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--plan" => plan = true,
            "--add" => add = Some(args.by_ref().collect::<Vec<_>>().join(" ")),
            _ => {
                eprintln!("{USAGE}");
                std::process::exit(2);
            }
        }
        if add.is_some() {
            break;
        }
    }
    let store = match Store::open(&data_path()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("flowday: can't open {}: {e}", data_path().display());
            std::process::exit(1);
        }
    };
    // `flowday --add "Call mom tomorrow 5pm"` adds without opening the window.
    if let Some(text) = add {
        let q = quickadd::parse(&text, today());
        match store.add_quick(&q) {
            Ok(t) => println!("added: {}", t.title),
            Err(e) => {
                eprintln!("flowday: {e}");
                std::process::exit(1);
            }
        }
        return Ok(());
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Flowday")
            .with_app_id("flowday")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([520.0, 400.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Flowday",
        options,
        Box::new(move |cc| {
            theme::apply(&cc.egui_ctx);
            // Bigger text and icons on a 1080p screen; UI_ZOOM=1 gives the old size.
            let zoom = std::env::var("UI_ZOOM").ok().and_then(|z| z.parse::<f32>().ok()).unwrap_or(1.25);
            cc.egui_ctx.set_zoom_factor(zoom.clamp(0.5, 3.0));
            let mut app = App::new(store);
            if plan {
                app.run(Command::PlanDay);
            }
            Ok(Box::new(app))
        }),
    )
}
