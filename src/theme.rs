//! Catppuccin Mocha theme — matches the Omarchy desktop look.
//!
//! Instead of egui's stock dark visuals we install the full palette:
//! base/mantle/crust surfaces, rounded corners everywhere, and proper
//! text/widget colors, so the app reads as a native part of the desktop.

use std::collections::BTreeMap;

use eframe::egui::{self, Color32};

// Base surfaces
pub const CRUST: Color32 = Color32::from_rgb(0x11, 0x11, 0x1b);
pub const MANTLE: Color32 = Color32::from_rgb(0x18, 0x18, 0x25);
pub const BASE: Color32 = Color32::from_rgb(0x1e, 0x1e, 0x2e);
pub const SURFACE0: Color32 = Color32::from_rgb(0x31, 0x32, 0x44);
pub const SURFACE1: Color32 = Color32::from_rgb(0x45, 0x47, 0x5a);
pub const SURFACE2: Color32 = Color32::from_rgb(0x58, 0x5b, 0x70);

// Text
pub const OVERLAY0: Color32 = Color32::from_rgb(0x6c, 0x70, 0x86);
pub const OVERLAY1: Color32 = Color32::from_rgb(0x7f, 0x84, 0x9c);
pub const SUBTEXT0: Color32 = Color32::from_rgb(0xa6, 0xad, 0xc8);
pub const SUBTEXT1: Color32 = Color32::from_rgb(0xba, 0xc2, 0xde);
pub const TEXT: Color32 = Color32::from_rgb(0xcd, 0xd6, 0xf4);

// Accents
pub const GREEN: Color32 = Color32::from_rgb(0xa6, 0xe3, 0xa1);
pub const RED: Color32 = Color32::from_rgb(0xf3, 0x8b, 0xa8);
pub const YELLOW: Color32 = Color32::from_rgb(0xf9, 0xe2, 0xaf);
pub const BLUE: Color32 = Color32::from_rgb(0x89, 0xb4, 0xfa);
pub const SAPPHIRE: Color32 = Color32::from_rgb(0x74, 0xc7, 0xec);
pub const PEACH: Color32 = Color32::from_rgb(0xfa, 0xb3, 0x87);

/// Yellow tinted over mantle — background for the certificate warning banner.
pub const YELLOW_DIM: Color32 = Color32::from_rgb(0x33, 0x30, 0x36);

/// Blue blended over base — selection background.
pub const SELECTION: Color32 = Color32::from_rgb(0x39, 0x44, 0x61);

/// Install fonts, spacing and visuals on the egui context. Call once at startup.
pub fn apply(ctx: &egui::Context) {
    let mut style = egui::Style::default();

    style.text_styles = BTreeMap::from([
        (egui::TextStyle::Heading, egui::FontId::new(20.0, egui::FontFamily::Proportional)),
        (egui::TextStyle::Body, egui::FontId::new(13.5, egui::FontFamily::Proportional)),
        (egui::TextStyle::Button, egui::FontId::new(13.5, egui::FontFamily::Proportional)),
        (egui::TextStyle::Monospace, egui::FontId::new(12.5, egui::FontFamily::Monospace)),
        (egui::TextStyle::Small, egui::FontId::new(11.5, egui::FontFamily::Proportional)),
    ]);

    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(12.0, 7.0);

    style.visuals = visuals();
    // eframe keeps separate styles per system theme; the app is dark-only,
    // so install our style on both to avoid a default light fallback.
    ctx.set_style_of(egui::Theme::Dark, style.clone());
    ctx.set_style_of(egui::Theme::Light, style);
}

fn visuals() -> egui::Visuals {
    let mut v = egui::Visuals::dark();

    v.panel_fill = BASE;
    v.window_fill = MANTLE;
    v.faint_bg_color = BASE;
    v.extreme_bg_color = CRUST; // TextEdit background
    v.code_bg_color = CRUST;
    v.window_stroke = egui::Stroke::new(1.0, SURFACE0);
    v.hyperlink_color = SAPPHIRE;
    v.warn_fg_color = YELLOW;
    v.error_fg_color = RED;

    v.widgets = egui::style::Widgets {
        noninteractive: widget(SURFACE0, Color32::TRANSPARENT, TEXT, 8.0),
        inactive: widget(SURFACE1, SURFACE0, TEXT, 8.0),
        hovered: widget(SURFACE2, SURFACE1, TEXT, 8.0),
        active: widget(SURFACE2, SURFACE1, TEXT, 8.0),
        open: widget(SURFACE1, SURFACE0, TEXT, 8.0),
    };

    v.selection = egui::style::Selection {
        bg_fill: SELECTION,
        stroke: egui::Stroke::new(1.0, BLUE),
    };

    v
}

fn widget(bg: Color32, weak_bg: Color32, fg: Color32, radius: f32) -> egui::style::WidgetVisuals {
    egui::style::WidgetVisuals {
        bg_fill: bg,
        weak_bg_fill: weak_bg,
        bg_stroke: egui::Stroke::NONE,
        fg_stroke: egui::Stroke::new(1.0, fg),
        corner_radius: radius.into(),
        expansion: 0.0,
    }
}
