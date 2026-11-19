//! Colours, spacing and egui visuals for both themes. The app follows the
//! system theme (macOS / GNOME dark or light).

use eframe::egui::{
    self, Color32, CornerRadius, FontFamily, FontId, Shadow, Stroke, TextStyle, Theme, Vec2,
};

/// Design tokens. "Soft" colours are pre-blended over `surface` so they are
/// opaque and their contrast can be tested.
#[derive(Debug)]
pub struct Palette {
    /// Window background behind content.
    pub bg: Color32,
    /// Top / bottom bars and side panels.
    pub chrome: Color32,
    /// Tiles and cards.
    pub surface: Color32,
    /// Hovered controls.
    pub raised: Color32,
    /// Resting buttons.
    pub control: Color32,
    /// Menus and popups.
    pub popup: Color32,
    pub border: Color32,
    pub border_strong: Color32,
    pub text: Color32,
    pub text2: Color32,
    pub text3: Color32,
    pub accent: Color32,
    pub on_accent: Color32,
    /// Accent used as text on `accent_soft`.
    pub accent_text: Color32,
    pub accent_soft: Color32,
    pub live: Color32,
    pub live_soft: Color32,
    pub polled: Color32,
    pub polled_soft: Color32,
    pub stopped: Color32,
    pub stopped_soft: Color32,
    pub danger: Color32,
    /// Edge of the black phone frame.
    pub bezel_edge: Color32,
}

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

pub const DARK: Palette = Palette {
    bg: hex(0x0D0F12),
    chrome: hex(0x121519),
    surface: hex(0x171B21),
    raised: hex(0x2A313C),
    control: hex(0x1E232B),
    popup: hex(0x1B2027),
    border: hex(0x272D36),
    border_strong: hex(0x3A424E),
    text: hex(0xE7EAEE),
    text2: hex(0xA3ACB8),
    text3: hex(0x848E9B),
    accent: hex(0x7EA6FF),
    on_accent: hex(0x0B1220),
    accent_text: hex(0xA9C2FF),
    accent_soft: hex(0x252E40),
    live: hex(0x4ADE80),
    live_soft: hex(0x1D322C),
    polled: hex(0xF5B342),
    polled_soft: hex(0x342F25),
    stopped: hex(0xA3ACB8),
    stopped_soft: hex(0x25292F),
    danger: hex(0xFF8A8A),
    bezel_edge: hex(0x2C323B),
};

pub const LIGHT: Palette = Palette {
    bg: hex(0xF4F5F7),
    chrome: hex(0xFBFBFC),
    surface: hex(0xFFFFFF),
    raised: hex(0xE6E9EE),
    control: hex(0xF0F2F5),
    popup: hex(0xFFFFFF),
    border: hex(0xE2E5EA),
    border_strong: hex(0xC3C9D1),
    text: hex(0x15181D),
    text2: hex(0x4F5966),
    text3: hex(0x636D79),
    accent: hex(0x2F5FD8),
    on_accent: hex(0xFFFFFF),
    accent_text: hex(0x2449B0),
    accent_soft: hex(0xEAEFFB),
    live: hex(0x116B33),
    live_soft: hex(0xE8F2EC),
    polled: hex(0x9A5506),
    polled_soft: hex(0xF8EEE6),
    stopped: hex(0x59626E),
    stopped_soft: hex(0xF0F1F3),
    danger: hex(0xC62828),
    bezel_edge: hex(0x1B1F26),
};

/// Black phone frame around every screen.
pub const BEZEL: Color32 = hex(0x050607);

/// Spacing and radii (points).
pub const RADIUS_CONTROL: u8 = 8;
pub const RADIUS_CARD: u8 = 14;

pub fn palette(ctx: &egui::Context) -> &'static Palette {
    match ctx.theme() {
        Theme::Dark => &DARK,
        Theme::Light => &LIGHT,
    }
}

/// What a status dot or pill means. Always shown with a word, never colour alone.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Tone {
    /// Live gRPC stream.
    Live,
    /// Polled PNG (~2 fps).
    Polled,
    /// Starting / booting / not ready.
    Busy,
    Stopped,
}

impl Tone {
    /// (text / dot colour, background).
    pub fn colors(self, p: &Palette) -> (Color32, Color32) {
        match self {
            Tone::Live => (p.live, p.live_soft),
            Tone::Polled => (p.polled, p.polled_soft),
            Tone::Busy => (p.accent_text, p.accent_soft),
            Tone::Stopped => (p.stopped, p.stopped_soft),
        }
    }
}

/// Install both themes' styles. The theme preference is left at its
/// default (follow the system).
pub fn install(ctx: &egui::Context) {
    ctx.set_style_of(Theme::Dark, style(Theme::Dark, &DARK));
    ctx.set_style_of(Theme::Light, style(Theme::Light, &LIGHT));
}

fn style(theme: Theme, p: &Palette) -> egui::Style {
    let mut style = egui::Style {
        visuals: visuals(theme, p),
        ..Default::default()
    };
    use FontFamily::{Monospace, Proportional};
    style.text_styles = [
        (TextStyle::Heading, FontId::new(17.0, Proportional)),
        (TextStyle::Body, FontId::new(13.0, Proportional)),
        (TextStyle::Button, FontId::new(13.0, Proportional)),
        (TextStyle::Small, FontId::new(11.0, Proportional)),
        (TextStyle::Monospace, FontId::new(12.0, Monospace)),
    ]
    .into();
    let s = &mut style.spacing;
    s.button_padding = Vec2::new(10.0, 6.0);
    s.item_spacing = Vec2::new(8.0, 8.0);
    s.interact_size.y = 30.0;
    s.menu_margin = egui::Margin::same(6);
    style
}

fn visuals(theme: Theme, p: &Palette) -> egui::Visuals {
    let mut v = match theme {
        Theme::Dark => egui::Visuals::dark(),
        Theme::Light => egui::Visuals::light(),
    };
    let radius = CornerRadius::same(RADIUS_CONTROL);
    v.panel_fill = p.chrome;
    v.window_fill = p.popup;
    v.window_stroke = Stroke::new(1.0, p.border_strong);
    v.window_corner_radius = CornerRadius::same(10);
    v.menu_corner_radius = CornerRadius::same(10);
    let shadow = Shadow {
        offset: [0, 8],
        blur: 24,
        spread: 0,
        color: Color32::from_black_alpha(if theme == Theme::Dark { 110 } else { 40 }),
    };
    v.window_shadow = shadow;
    v.popup_shadow = shadow;
    v.extreme_bg_color = p.bg;
    v.faint_bg_color = p.surface;
    v.code_bg_color = p.control;
    v.hyperlink_color = p.accent;
    v.warn_fg_color = p.polled;
    v.error_fg_color = p.danger;
    v.selection.bg_fill = p.accent_soft;
    v.selection.stroke = Stroke::new(1.0, p.accent);

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.surface;
    w.noninteractive.weak_bg_fill = p.surface;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text2);
    w.noninteractive.corner_radius = radius;

    w.inactive.bg_fill = p.control;
    w.inactive.weak_bg_fill = p.control;
    w.inactive.bg_stroke = Stroke::new(1.0, p.border_strong);
    w.inactive.fg_stroke = Stroke::new(1.0, p.text);
    w.inactive.corner_radius = radius;

    w.hovered.bg_fill = p.raised;
    w.hovered.weak_bg_fill = p.raised;
    w.hovered.bg_stroke = Stroke::new(1.0, p.border_strong);
    w.hovered.fg_stroke = Stroke::new(1.5, p.text);
    w.hovered.corner_radius = radius;
    w.hovered.expansion = 0.0;

    w.active.bg_fill = p.raised;
    w.active.weak_bg_fill = p.raised;
    w.active.bg_stroke = Stroke::new(1.0, p.accent);
    w.active.fg_stroke = Stroke::new(2.0, p.text);
    w.active.corner_radius = radius;
    w.active.expansion = 0.0;

    w.open.bg_fill = p.raised;
    w.open.weak_bg_fill = p.raised;
    w.open.bg_stroke = Stroke::new(1.0, p.border_strong);
    w.open.fg_stroke = Stroke::new(1.0, p.text);
    w.open.corner_radius = radius;
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn luminance(c: Color32) -> f32 {
        let lin = |v: u8| {
            let v = v as f32 / 255.0;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(c.r()) + 0.7152 * lin(c.g()) + 0.0722 * lin(c.b())
    }

    fn contrast(a: Color32, b: Color32) -> f32 {
        let (la, lb) = (luminance(a), luminance(b));
        (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
    }

    /// Every text colour is readable (WCAG AA, 4.5:1) on every surface it sits on.
    #[test]
    fn text_meets_contrast() {
        for (name, p) in [("dark", &DARK), ("light", &LIGHT)] {
            for (fg_name, fg) in [("text", p.text), ("text2", p.text2), ("text3", p.text3)] {
                for (bg_name, bg) in [
                    ("bg", p.bg),
                    ("chrome", p.chrome),
                    ("surface", p.surface),
                    ("control", p.control),
                ] {
                    let c = contrast(fg, bg);
                    assert!(c >= 4.5, "{name}: {fg_name} on {bg_name} is {c:.2}");
                }
            }
            for tone in [Tone::Live, Tone::Polled, Tone::Busy, Tone::Stopped] {
                let (fg, soft) = tone.colors(p);
                let c = contrast(fg, soft);
                assert!(c >= 4.5, "{name}: {tone:?} pill is {c:.2}");
            }
            let c = contrast(p.on_accent, p.accent);
            assert!(c >= 4.5, "{name}: on_accent is {c:.2}");
            let c = contrast(p.danger, p.surface);
            assert!(c >= 4.5, "{name}: danger is {c:.2}");
        }
    }
}
