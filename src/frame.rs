//! A phone-shaped frame around a device screen, and the small line icons
//! used in the toolbar. Everything is drawn with the painter: no image
//! assets, no icon fonts.

use eframe::egui::{
    self, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2, epaint,
};
use std::f32::consts::{PI, TAU};

/// Where the phone body and its screen go inside a pane.
#[derive(Clone, Copy, Debug)]
pub struct Layout {
    /// The black body around the screen.
    pub shell: Rect,
    /// The device screen itself.
    pub screen: Rect,
    pub outer_radius: f32,
    pub inner_radius: f32,
    /// How far the side buttons stick out of the body.
    pub button: f32,
}

/// Bezel, side-button room and side-button thickness for a screen whose
/// short side is `short` points.
fn chrome(short: f32) -> (f32, f32, f32) {
    let bezel = (short * 0.021).clamp(3.0, 15.0);
    let outset = (short * 0.012).clamp(2.0, 7.0);
    let button = (short * 0.01).clamp(1.5, 6.0);
    (bezel, outset, button)
}

/// Fit a screen with `aspect` = height / width, plus its frame, centred in
/// `pane`.
pub fn fit(pane: Rect, aspect: f32) -> Layout {
    let aspect = aspect.max(0.05);
    let fit_screen = |w: f32, h: f32| {
        let (w, h) = (w.max(1.0), h.max(1.0));
        if h / w > aspect {
            Vec2::new(w, w * aspect)
        } else {
            Vec2::new(h / aspect, h)
        }
    };
    // The frame's size depends on the screen's, so settle it in a few rounds.
    let mut size = fit_screen(pane.width(), pane.height());
    for _ in 0..4 {
        let (bezel, outset, _) = chrome(size.min_elem());
        size = fit_screen(
            pane.width() - 2.0 * (bezel + outset),
            pane.height() - 2.0 * bezel,
        );
    }
    let short = size.min_elem();
    let (bezel, _, button) = chrome(short);
    let outer_radius = (short * 0.135).clamp(6.0, 92.0);
    let screen = Rect::from_center_size(pane.center(), size);
    Layout {
        shell: screen.expand(bezel),
        screen,
        outer_radius,
        inner_radius: (outer_radius - bezel).max(2.0),
        button,
    }
}

fn radius(r: f32) -> CornerRadius {
    CornerRadius::same(r.round().clamp(0.0, 255.0) as u8)
}

/// Draw the phone body (shadow, black shell, side buttons). The screen is
/// drawn on top by the caller, clipped to `inner_radius`.
pub fn paint_shell(painter: &egui::Painter, l: &Layout, dark: bool) {
    let shadow = epaint::Shadow {
        offset: [0, 6],
        blur: 24,
        spread: 0,
        color: Color32::from_black_alpha(if dark { 110 } else { 60 }),
    };
    painter.add(shadow.as_shape(l.shell, radius(l.outer_radius)));
    let edge = Color32::from_gray(if dark { 64 } else { 30 });
    let body = Color32::from_rgb(8, 8, 10);

    // Side buttons, only when upright: one short and two volume keys on the
    // left, power on the right.
    if l.shell.height() > l.shell.width() {
        let h = l.shell.height();
        let t = l.button;
        let action = (h * 0.04).clamp(10.0, 34.0);
        let volume = (h * 0.08).clamp(16.0, 64.0);
        let power = (h * 0.095).clamp(20.0, 76.0);
        let left = |top: f32, len: f32| {
            Rect::from_min_size(
                Pos2::new(l.shell.left() - t, l.shell.top() + h * top),
                Vec2::new(t + 2.0, len),
            )
        };
        let right = Rect::from_min_size(
            Pos2::new(l.shell.right() - 2.0, l.shell.top() + h * 0.24),
            Vec2::new(t + 2.0, power),
        );
        for r in [
            left(0.16, action),
            left(0.24, volume),
            left(0.33, volume),
            right,
        ] {
            painter.rect(
                r,
                radius(t * 0.6),
                body,
                Stroke::new(1.0, edge),
                StrokeKind::Outside,
            );
        }
    }
    painter.rect(
        l.shell,
        radius(l.outer_radius),
        body,
        Stroke::new(1.0, edge),
        StrokeKind::Outside,
    );
}

/// The device screen, rounded to fit the frame.
pub fn paint_screen(ui: &egui::Ui, l: &Layout, tex: &egui::TextureHandle) {
    egui::Image::from_texture((tex.id(), l.screen.size()))
        .corner_radius(radius(l.inner_radius))
        .paint_at(ui, l.screen);
}

/// An empty (black) screen while waiting for the first frame.
pub fn paint_blank(painter: &egui::Painter, l: &Layout) {
    painter.rect_filled(l.screen, radius(l.inner_radius), Color32::from_gray(16));
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Icon {
    ArrowLeft,
    Phone,
    Back,
    Home,
    Recents,
    Rotate,
    Power,
    Camera,
    More,
    Check,
    VolumeDown,
    VolumeUp,
}

fn arc(c: Pos2, r: f32, from: f32, to: f32) -> Vec<Pos2> {
    let n = ((to - from).abs() / TAU * 48.0).ceil().max(4.0) as usize;
    (0..=n)
        .map(|i| {
            let a = from + (to - from) * i as f32 / n as f32;
            c + r * Vec2::new(a.cos(), a.sin())
        })
        .collect()
}

/// Draw `icon` in the square `rect` (lucide-style, 1.5 pt strokes).
pub fn paint_icon(painter: &egui::Painter, icon: Icon, rect: Rect, color: Color32) {
    let s = rect.width().min(rect.height());
    let c = rect.center();
    let stroke = Stroke::new((s / 11.0).clamp(1.2, 2.0), color);
    let p = |x: f32, y: f32| c + Vec2::new(x, y) * s;
    match icon {
        Icon::ArrowLeft => {
            painter.line_segment([p(0.32, 0.0), p(-0.32, 0.0)], stroke);
            painter.line(vec![p(-0.05, -0.28), p(-0.33, 0.0), p(-0.05, 0.28)], stroke);
        }
        Icon::Phone => {
            let r = Rect::from_center_size(c, Vec2::new(0.52, 0.84) * s);
            painter.rect_stroke(r, radius(s * 0.12), stroke, StrokeKind::Middle);
            painter.line_segment([p(-0.06, 0.29), p(0.06, 0.29)], stroke);
        }
        Icon::Back => {
            let pts = vec![p(0.22, -0.3), p(-0.28, 0.0), p(0.22, 0.3), p(0.22, -0.3)];
            painter.add(egui::Shape::closed_line(pts, stroke));
        }
        Icon::Home => {
            painter.line(vec![p(-0.36, -0.02), p(0.0, -0.36), p(0.36, -0.02)], stroke);
            painter.line(
                vec![p(-0.26, -0.1), p(-0.26, 0.34), p(0.26, 0.34), p(0.26, -0.1)],
                stroke,
            );
            painter.line(
                vec![p(-0.08, 0.34), p(-0.08, 0.1), p(0.08, 0.1), p(0.08, 0.34)],
                stroke,
            );
        }
        Icon::Recents => {
            let r = Rect::from_center_size(c, Vec2::splat(0.58 * s));
            painter.rect_stroke(r, radius(s * 0.08), stroke, StrokeKind::Middle);
        }
        Icon::Rotate => {
            let r = 0.32 * s;
            let end = -0.28 * PI;
            painter.line(arc(c, r, end - 1.65 * PI, end), stroke);
            // Arrow head pointing along the clockwise direction at `end`.
            let tip = c + r * Vec2::new(end.cos(), end.sin());
            let along = Vec2::new(-end.sin(), end.cos());
            let out = Vec2::new(end.cos(), end.sin());
            let k = 0.2 * s;
            painter.line(
                vec![
                    tip - along * k + out * k * 0.15,
                    tip,
                    tip - out * k + along * k * 0.15,
                ],
                stroke,
            );
        }
        Icon::Power => {
            let r = 0.32 * s;
            painter.line(arc(c, r, -0.32 * PI, 1.32 * PI), stroke);
            painter.line_segment([p(0.0, -0.42), p(0.0, -0.04)], stroke);
        }
        Icon::Camera => {
            let body = Rect::from_center_size(p(0.0, 0.05), Vec2::new(0.8, 0.56) * s);
            painter.rect_stroke(body, radius(s * 0.1), stroke, StrokeKind::Middle);
            painter.line(
                vec![
                    p(-0.16, -0.23),
                    p(-0.1, -0.33),
                    p(0.1, -0.33),
                    p(0.16, -0.23),
                ],
                stroke,
            );
            painter.circle_stroke(p(0.0, 0.06), 0.15 * s, stroke);
        }
        Icon::More => {
            for x in [-0.28, 0.0, 0.28] {
                painter.circle_filled(p(x, 0.0), 0.06 * s, color);
            }
        }
        Icon::Check => {
            painter.line(vec![p(-0.3, 0.02), p(-0.08, 0.24), p(0.32, -0.22)], stroke);
        }
        Icon::VolumeDown | Icon::VolumeUp => {
            let pts = vec![
                p(-0.38, -0.11),
                p(-0.22, -0.11),
                p(-0.02, -0.3),
                p(-0.02, 0.3),
                p(-0.22, 0.11),
                p(-0.38, 0.11),
            ];
            painter.add(egui::Shape::closed_line(pts, stroke));
            painter.line(arc(p(-0.02, 0.0), 0.16 * s, -0.3 * PI, 0.3 * PI), stroke);
            if icon == Icon::VolumeUp {
                painter.line(arc(p(-0.02, 0.0), 0.32 * s, -0.3 * PI, 0.3 * PI), stroke);
            }
        }
    }
}

/// A toolbar button: an icon, optionally followed by a short label.
/// `danger` turns it red on hover (for shutting a device down).
pub fn button(ui: &mut egui::Ui, icon: Icon, label: Option<&str>, danger: bool) -> egui::Response {
    let h = ui.spacing().interact_size.y.max(26.0);
    let icon_size = 15.0;
    let galley = label.map(|t| {
        ui.painter().layout_no_wrap(
            t.to_string(),
            FontId::proportional(13.0),
            Color32::PLACEHOLDER,
        )
    });
    let width = match &galley {
        Some(g) => 10.0 + icon_size + 6.0 + g.size().x + 10.0,
        None => h,
    };
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, h), Sense::click());
    if ui.is_rect_visible(rect) {
        let v = ui.style().interact(&resp);
        ui.painter().rect(
            rect,
            radius(6.0),
            v.weak_bg_fill,
            v.bg_stroke,
            StrokeKind::Inside,
        );
        let mut color = v.text_color();
        if danger && resp.hovered() {
            color = ui.visuals().error_fg_color;
        }
        if !resp.enabled() {
            color = color.gamma_multiply(0.4);
        }
        let icon_rect = match &galley {
            Some(_) => Rect::from_center_size(
                Pos2::new(rect.left() + 10.0 + icon_size / 2.0, rect.center().y),
                Vec2::splat(icon_size),
            ),
            None => Rect::from_center_size(rect.center(), Vec2::splat(icon_size)),
        };
        paint_icon(ui.painter(), icon, icon_rect, color);
        if let Some(g) = galley {
            let pos = Pos2::new(icon_rect.right() + 6.0, rect.center().y - g.size().y / 2.0);
            ui.painter().galley(pos, g, color);
        }
    }
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.02 * b.max(1.0)
    }

    #[test]
    fn phone_fits_inside_the_pane_and_keeps_its_shape() {
        for (pane, aspect) in [
            (
                Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 650.0)),
                2400.0 / 1080.0,
            ),
            (
                Rect::from_min_size(Pos2::new(10.0, 40.0), Vec2::new(300.0, 900.0)),
                2.0,
            ),
            (
                Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 400.0)),
                1080.0 / 2400.0,
            ),
            (
                Rect::from_min_size(Pos2::ZERO, Vec2::new(120.0, 200.0)),
                2.2,
            ),
        ] {
            let l = fit(pane, aspect);
            assert!(close(l.screen.height() / l.screen.width(), aspect), "{l:?}");
            assert!(l.shell.contains_rect(l.screen));
            let with_buttons = l.shell.expand2(Vec2::new(l.button, 0.0));
            assert!(
                pane.expand(0.5).contains_rect(with_buttons),
                "{pane:?} {l:?}"
            );
            assert!(close(l.screen.center().x, pane.center().x));
            assert!(close(l.screen.center().y, pane.center().y));
            // It fills the pane along one side, less the frame.
            let used = (l.shell.width() / pane.width()).max(l.shell.height() / pane.height());
            assert!(used > 0.95, "{used} {l:?}");
            assert!(l.inner_radius < l.outer_radius);
        }
    }
}
