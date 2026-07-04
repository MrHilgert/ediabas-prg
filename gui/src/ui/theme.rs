//! Design tokens + egui `Visuals`, ported 1:1 from the Chassis Select handoff.
//!
//! One palette per theme. `Colors` carries every token the screens paint with
//! directly (card fills, status dots, accents); `visuals()` maps the subset egui
//! needs for its built-in widgets/selection.

use egui::{Color32, Rounding, Stroke, Visuals};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    Dark,
    Light,
}

impl Theme {
    pub fn toggled(self) -> Self {
        match self {
            Theme::Dark => Theme::Light,
            Theme::Light => Theme::Dark,
        }
    }
}

/// Every design token as a concrete `Color32`.
#[derive(Clone, Copy)]
pub struct Colors {
    pub bg: Color32,
    pub panel: Color32,
    pub panel2: Color32,
    pub elev: Color32,
    pub stroke: Color32,
    pub stroke2: Color32,
    pub fg: Color32,
    pub fg_dim: Color32,
    pub fg_faint: Color32,
    pub accent: Color32,
    pub accent_fg: Color32,
    pub accent_tint: Color32,
    pub ok: Color32,
    pub warn: Color32,
    pub err: Color32,
    pub faint_dot: Color32,
}

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}
const fn rgba(hex: u32, a: u8) -> Color32 {
    Color32::from_rgba_premultiplied((hex >> 16) as u8, (hex >> 8) as u8, hex as u8, a)
}

pub fn palette(theme: Theme) -> Colors {
    match theme {
        Theme::Dark => Colors {
            bg: rgb(0x0F1216),
            panel: rgb(0x171A1F),
            panel2: rgb(0x1D2128),
            elev: rgb(0x23272F),
            stroke: rgb(0x2A2F38),
            stroke2: rgb(0x3A414C),
            fg: rgb(0xD7DBE1),
            fg_dim: rgb(0x868D98),
            fg_faint: rgb(0x565C66),
            accent: rgb(0x2F81F7),
            accent_fg: rgb(0xFFFFFF),
            accent_tint: rgba(0x2F81F7, 33), // ~0.13 alpha
            ok: rgb(0x3FB950),
            warn: rgb(0xD29922),
            err: rgb(0xF85149),
            faint_dot: rgb(0x565C66),
        },
        Theme::Light => Colors {
            bg: rgb(0xE7E9EC),
            panel: rgb(0xF5F6F8),
            panel2: rgb(0xFFFFFF),
            elev: rgb(0xFFFFFF),
            stroke: rgb(0xD4D8DE),
            stroke2: rgb(0xBCC3CC),
            fg: rgb(0x1A1E25),
            fg_dim: rgb(0x5B626C),
            fg_faint: rgb(0x8B929C),
            accent: rgb(0x1F6FE5),
            accent_fg: rgb(0xFFFFFF),
            accent_tint: rgba(0x1F6FE5, 26), // ~0.10 alpha
            ok: rgb(0x1A7F37),
            warn: rgb(0x9A6700),
            err: rgb(0xCF222E),
            faint_dot: rgb(0x9AA1AB),
        },
    }
}

/// Flat, shadowless egui visuals matching the "engineering terminal" look.
pub fn visuals(theme: Theme) -> Visuals {
    let c = palette(theme);
    let mut v = match theme {
        Theme::Dark => Visuals::dark(),
        Theme::Light => Visuals::light(),
    };
    let r = Rounding::same(3.0);

    v.window_fill = c.bg;
    v.panel_fill = c.panel;
    v.faint_bg_color = c.panel2;
    v.extreme_bg_color = c.panel2;
    v.override_text_color = Some(c.fg);
    v.window_stroke = Stroke::new(1.0, c.stroke);
    v.window_rounding = Rounding::ZERO;
    v.window_shadow = egui::epaint::Shadow::NONE;
    v.popup_shadow = egui::epaint::Shadow::NONE;

    v.selection.bg_fill = c.accent_tint;
    v.selection.stroke = Stroke::new(1.0, c.accent);

    let w = &mut v.widgets;
    for ws in [
        &mut w.noninteractive,
        &mut w.inactive,
        &mut w.hovered,
        &mut w.active,
        &mut w.open,
    ] {
        ws.rounding = r;
        ws.bg_stroke = Stroke::new(1.0, c.stroke);
        ws.fg_stroke = Stroke::new(1.0, c.fg);
    }
    w.noninteractive.bg_fill = c.panel;
    w.noninteractive.weak_bg_fill = c.panel;
    w.noninteractive.fg_stroke = Stroke::new(1.0, c.fg_dim);
    w.inactive.bg_fill = c.panel2;
    w.inactive.weak_bg_fill = c.panel2;
    w.hovered.bg_fill = c.elev;
    w.hovered.weak_bg_fill = c.elev;
    w.hovered.bg_stroke = Stroke::new(1.0, c.stroke2);
    w.active.bg_fill = c.elev;
    w.active.weak_bg_fill = c.elev;
    w.active.bg_stroke = Stroke::new(1.0, c.accent);

    v
}
