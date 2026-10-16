//! A dark palette in the spirit of Akiflow: near-black greys with a violet accent.

use eframe::egui::{self, Color32, CornerRadius, Stroke, Visuals};

pub const BG: Color32 = Color32::from_rgb(0x12, 0x12, 0x16);
pub const SIDEBAR: Color32 = Color32::from_rgb(0x18, 0x18, 0x1d);
pub const SURFACE: Color32 = Color32::from_rgb(0x1c, 0x1c, 0x22);
pub const SURFACE_2: Color32 = Color32::from_rgb(0x27, 0x27, 0x2f);
pub const LINE: Color32 = Color32::from_rgb(0x2d, 0x2d, 0x36);
pub const INK: Color32 = Color32::from_rgb(0xf3, 0xf3, 0xf6);
pub const INK_2: Color32 = Color32::from_rgb(0xc8, 0xc8, 0xd2);
pub const INK_3: Color32 = Color32::from_rgb(0x88, 0x88, 0x96);
pub const ACCENT: Color32 = Color32::from_rgb(0x9b, 0x87, 0xf5);
pub const EVENT: Color32 = Color32::from_rgb(0x89, 0xb3, 0xf9);
pub const DANGER: Color32 = Color32::from_rgb(0xf8, 0x71, 0x71);
pub const WARN: Color32 = Color32::from_rgb(0xf5, 0xb8, 0x6b);
pub const OK: Color32 = Color32::from_rgb(0x7f, 0xd8, 0xa8);

/// Project colours, stored on the project as `#rrggbb`.
pub const SWATCHES: [&str; 7] = ["#9b87f5", "#f48aab", "#a5e0ca", "#f5dfb4", "#89b3f9", "#96dfcd", "#b2bad1"];

pub fn hex(s: &str) -> Color32 {
    Color32::from_hex(s).unwrap_or(ACCENT)
}

/// A colour faded into the background, for block fills.
pub fn soft(c: Color32) -> Color32 {
    let mix = |a: u8, b: u8| ((a as u16 * 30 + b as u16 * 70) / 100) as u8;
    Color32::from_rgb(mix(c.r(), BG.r()), mix(c.g(), BG.g()), mix(c.b(), BG.b()))
}

/// Priority flag colours, `!1` (3) to `!3` (1).
pub fn priority(p: u8) -> Option<Color32> {
    match p {
        3 => Some(DANGER),
        2 => Some(WARN),
        1 => Some(EVENT),
        _ => None,
    }
}

pub fn apply(ctx: &egui::Context) {
    let mut v = Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = SURFACE;
    v.window_stroke = Stroke::new(1.0, LINE);
    v.window_corner_radius = CornerRadius::same(10);
    v.extreme_bg_color = SURFACE_2;
    v.faint_bg_color = SURFACE;
    v.override_text_color = Some(INK);
    v.hyperlink_color = ACCENT;
    v.selection.bg_fill = soft(ACCENT);
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    let r = CornerRadius::same(6);
    for (w, fill) in [
        (&mut v.widgets.noninteractive, SURFACE),
        (&mut v.widgets.inactive, SURFACE_2),
        (&mut v.widgets.hovered, LINE),
        (&mut v.widgets.active, soft(ACCENT)),
        (&mut v.widgets.open, SURFACE_2),
    ] {
        w.bg_fill = fill;
        w.weak_bg_fill = fill;
        w.corner_radius = r;
    }
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, INK_2);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT);
    ctx.set_visuals(v);
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.button_padding = egui::vec2(10.0, 5.0);
    });
}
