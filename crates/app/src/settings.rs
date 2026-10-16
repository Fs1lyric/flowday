//! Settings: Claude, work hours, and the integrations. Keys go to the keyring;
//! everything else to the store's settings table.

use crate::{secrets, theme};
use chrono::NaiveTime;
use eframe::egui::{self, Id, RichText};
use flowday_core::store::Store;
use flowday_sync::{google, Config};
use std::sync::mpsc::{channel, Receiver};

pub struct Settings {
    anthropic: String,
    model: String,
    start: String,
    end: String,
    buffer: String,
    github: String,
    linear: String,
    notion: String,
    notion_dbs: String,
    google_id: String,
    google_secret: String,
    google_connected: bool,
    gmail: bool,
    push: bool,
    connecting: Option<Receiver<Result<String, String>>>,
    message: Option<(bool, String)>,
}

fn setting(store: &Store, key: &str) -> String {
    store.setting(key).ok().flatten().unwrap_or_default()
}

/// The sync configuration from the keyring and settings.
pub fn sync_config(store: &Store) -> Config {
    let google = match (setting(store, "google_client_id"), secrets::get(secrets::GOOGLE_SECRET), secrets::get(secrets::GOOGLE_REFRESH)) {
        (id, Some(secret), Some(refresh)) if !id.is_empty() => {
            Some(google::Credentials { client_id: id, client_secret: secret, refresh_token: refresh })
        }
        _ => None,
    };
    let github_on = setting(store, "github_enabled") == "1";
    Config {
        github_token: secrets::get(secrets::GITHUB).or_else(|| if github_on { flowday_sync::github::gh_cli_token() } else { None }),
        linear_key: secrets::get(secrets::LINEAR),
        notion_token: secrets::get(secrets::NOTION),
        notion_databases: setting(store, "notion_databases").split([',', '\n']).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect(),
        google,
        gmail_starred: setting(store, "gmail_starred") != "0",
        push_blocks: setting(store, "push_blocks") != "0",
    }
}

pub fn any_integration(store: &Store) -> bool {
    setting(store, "github_enabled") == "1"
        || ["linear_enabled", "notion_enabled", "google_enabled"].iter().any(|k| setting(store, k) == "1")
}

impl Settings {
    pub fn new(store: &Store) -> Self {
        let hours = flowday_core::plan::hours(store);
        let secret = |n| secrets::get(n).unwrap_or_default();
        Self {
            anthropic: secret(secrets::ANTHROPIC),
            model: store.setting("model").ok().flatten().unwrap_or_else(|| flowday_ai::DEFAULT_MODEL.into()),
            start: hours.start.format("%H:%M").to_string(),
            end: hours.end.format("%H:%M").to_string(),
            buffer: flowday_core::plan::buffer(store).to_string(),
            github: secret(secrets::GITHUB),
            linear: secret(secrets::LINEAR),
            notion: secret(secrets::NOTION),
            notion_dbs: setting(store, "notion_databases"),
            google_id: setting(store, "google_client_id"),
            google_secret: secret(secrets::GOOGLE_SECRET),
            google_connected: secrets::get(secrets::GOOGLE_REFRESH).is_some(),
            gmail: setting(store, "gmail_starred") != "0",
            push: setting(store, "push_blocks") != "0",
            connecting: None,
            message: None,
        }
    }

    fn save(&mut self, store: &Store) -> Result<(), String> {
        let t = |s: &str| NaiveTime::parse_from_str(s.trim(), "%H:%M").map_err(|_| format!("work hours: can't read {s:?} (use HH:MM)"));
        let (a, b) = (t(&self.start)?, t(&self.end)?);
        if a >= b {
            return Err("work hours must end after they start".into());
        }
        let buffer: u32 = self.buffer.trim().parse().map_err(|_| "buffer must be a number of minutes")?;
        let e = |e: flowday_core::store::Error| e.to_string();
        store.set_setting("hours", &format!("{}-{}", a.format("%H:%M"), b.format("%H:%M"))).map_err(e)?;
        store.set_setting("buffer_min", &buffer.to_string()).map_err(e)?;
        store.set_setting("model", self.model.trim()).map_err(e)?;
        store.set_setting("notion_databases", self.notion_dbs.trim()).map_err(e)?;
        store.set_setting("google_client_id", self.google_id.trim()).map_err(e)?;
        store.set_setting("gmail_starred", if self.gmail { "1" } else { "0" }).map_err(e)?;
        store.set_setting("push_blocks", if self.push { "1" } else { "0" }).map_err(e)?;
        for (name, value) in [
            (secrets::ANTHROPIC, &self.anthropic),
            (secrets::GITHUB, &self.github),
            (secrets::LINEAR, &self.linear),
            (secrets::NOTION, &self.notion),
            (secrets::GOOGLE_SECRET, &self.google_secret),
        ] {
            // The env var is only a fallback; don't copy it into the keyring.
            if name == secrets::ANTHROPIC && std::env::var("ANTHROPIC_API_KEY").ok().as_ref() == Some(value) {
                continue;
            }
            secrets::set(name, value.trim())?;
        }
        let on = |b: bool| if b { "1" } else { "0" };
        store.set_setting("linear_enabled", on(!self.linear.trim().is_empty())).map_err(e)?;
        store.set_setting("notion_enabled", on(!self.notion.trim().is_empty())).map_err(e)?;
        store.set_setting("google_enabled", on(self.google_connected)).map_err(e)?;
        Ok(())
    }

    /// Returns true when it should close.
    pub fn show(&mut self, ctx: &egui::Context, store: &Store) -> bool {
        if let Some(rx) = &self.connecting {
            if let Ok(r) = rx.try_recv() {
                self.connecting = None;
                self.message = Some(match r.and_then(|token| secrets::set(secrets::GOOGLE_REFRESH, &token)) {
                    Ok(()) => {
                        self.google_connected = true;
                        let _ = store.set_setting("google_enabled", "1");
                        (true, "Connected to Google.".into())
                    }
                    Err(e) => (false, e),
                });
            }
        }
        let mut close = false;
        let modal = egui::Modal::new(Id::new("settings"))
            .frame(egui::Frame::window(&ctx.global_style()).inner_margin(20.0))
            .show(ctx, |ui| {
                ui.set_width(560.0);
                ui.label(RichText::new("Settings").size(20.0).strong());
                egui::ScrollArea::vertical().max_height(ctx.content_rect().height() - 160.0).show(ui, |ui| self.body(ui, store));
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                    if ui.button(RichText::new("Save").color(theme::ACCENT)).clicked() {
                        match self.save(store) {
                            Ok(()) => close = true,
                            Err(e) => self.message = Some((false, e)),
                        }
                    }
                    if let Some((ok, m)) = &self.message {
                        ui.label(RichText::new(m).color(if *ok { theme::OK } else { theme::DANGER }));
                    }
                });
            });
        close || (modal.should_close() && self.connecting.is_none())
    }

    fn body(&mut self, ui: &mut egui::Ui, store: &Store) {
        let head = |ui: &mut egui::Ui, s: &str| {
            ui.add_space(10.0);
            ui.label(RichText::new(s).strong().color(theme::ACCENT));
        };
        let hint = |ui: &mut egui::Ui, s: &str| {
            ui.label(RichText::new(s).size(11.5).color(theme::INK_3));
        };
        let secret = |ui: &mut egui::Ui, s: &mut String| {
            ui.add(egui::TextEdit::singleline(s).password(true).desired_width(300.0));
        };

        head(ui, "Claude");
        egui::Grid::new("claude").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
            ui.label("API key");
            secret(ui, &mut self.anthropic);
            ui.end_row();
            ui.label("Model");
            ui.add(egui::TextEdit::singleline(&mut self.model).desired_width(300.0));
            ui.end_row();
        });
        hint(ui, "From console.anthropic.com. Kept in your system keyring.");

        head(ui, "Your day");
        ui.horizontal(|ui| {
            ui.label("Work hours");
            ui.add(egui::TextEdit::singleline(&mut self.start).desired_width(56.0));
            ui.label("to");
            ui.add(egui::TextEdit::singleline(&mut self.end).desired_width(56.0));
            ui.label("Buffer between blocks");
            ui.add(egui::TextEdit::singleline(&mut self.buffer).desired_width(36.0));
            ui.label("min");
        });

        head(ui, "GitHub");
        let mut github_on = setting(store, "github_enabled") == "1";
        if ui.checkbox(&mut github_on, "Pull issues and PRs assigned to me, and PRs waiting for my review").changed() {
            let _ = store.set_setting("github_enabled", if github_on { "1" } else { "0" });
        }
        ui.horizontal(|ui| {
            ui.label("Token");
            secret(ui, &mut self.github);
        });
        hint(ui, "Leave empty to use the GitHub CLI's login (gh auth token).");

        head(ui, "Linear");
        ui.horizontal(|ui| {
            ui.label("API key");
            secret(ui, &mut self.linear);
        });
        hint(ui, "Settings → Security & access → Personal API keys. Pulls issues assigned to you.");

        head(ui, "Notion");
        ui.horizontal(|ui| {
            ui.label("Token");
            secret(ui, &mut self.notion);
        });
        ui.horizontal(|ui| {
            ui.label("Databases");
            ui.add(egui::TextEdit::singleline(&mut self.notion_dbs).hint_text("database links or ids, comma separated").desired_width(360.0));
        });
        hint(ui, "Make an internal integration at notion.so/my-integrations and share the databases with it.");

        head(ui, "Google Calendar and Gmail");
        egui::Grid::new("google").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
            ui.label("Client id");
            ui.add(egui::TextEdit::singleline(&mut self.google_id).desired_width(300.0));
            ui.end_row();
            ui.label("Client secret");
            secret(ui, &mut self.google_secret);
            ui.end_row();
        });
        hint(ui, "In Google Cloud: enable the Calendar and Gmail APIs, then create an OAuth client of type \"Desktop app\".");
        ui.horizontal(|ui| {
            if self.connecting.is_some() {
                ui.spinner();
                ui.label("Waiting for you to sign in in the browser…");
            } else {
                let label = if self.google_connected { "Reconnect Google" } else { "Connect Google" };
                let ready = !self.google_id.trim().is_empty() && !self.google_secret.trim().is_empty();
                if ui.add_enabled(ready, egui::Button::new(label)).clicked() {
                    let (id, secret) = (self.google_id.trim().to_string(), self.google_secret.trim().to_string());
                    let _ = store.set_setting("google_client_id", &id);
                    let _ = secrets::set(secrets::GOOGLE_SECRET, &secret);
                    let (tx, rx) = channel();
                    let ctx = ui.ctx().clone();
                    std::thread::spawn(move || {
                        let _ = tx.send(google::authorize(&id, &secret));
                        ctx.request_repaint();
                    });
                    self.connecting = Some(rx);
                }
                if self.google_connected {
                    ui.label(RichText::new("Connected").color(theme::OK));
                    if ui.small_button("Disconnect").clicked() {
                        let _ = secrets::set(secrets::GOOGLE_REFRESH, "");
                        let _ = store.set_setting("google_enabled", "0");
                        self.google_connected = false;
                    }
                }
            }
        });
        ui.checkbox(&mut self.push, "Put my time blocks on Google Calendar");
        ui.checkbox(&mut self.gmail, "Bring starred emails into the inbox");
    }
}
