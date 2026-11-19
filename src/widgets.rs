//! Small painted widgets shared by the views: buttons, pills, segmented
//! control, keyboard chips, phone bezels.

use crate::icons::{self, Icon};
use crate::theme::{self, BEZEL, Palette, RADIUS_CONTROL, Tone};
use eframe::egui::{
    self, Align2, Color32, CornerRadius, CursorIcon, FontId, Painter, Pos2, Rect, Response,
    RichText, Role, Sense, Stroke, StrokeKind, TextureId, Ui, Vec2, WidgetInfo, epaint::RectShape,
};

pub fn pal(ui: &Ui) -> &'static Palette {
    theme::palette(ui.ctx())
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Primary,
    Secondary,
    Ghost,
    Danger,
}

const ICON: f32 = 15.0;
const ICON_GAP: f32 = 7.0;

/// (fill, stroke, text) for a button of `kind`.
fn button_colors(p: &Palette, kind: Kind, hovered: bool) -> (Color32, Stroke, Color32) {
    match (kind, hovered) {
        (Kind::Primary, false) => (p.accent, Stroke::NONE, p.on_accent),
        (Kind::Primary, true) => (
            p.accent.lerp_to_gamma(p.text, 0.12),
            Stroke::NONE,
            p.on_accent,
        ),
        (Kind::Secondary, h) => (
            if h { p.raised } else { p.control },
            Stroke::new(1.0, p.border_strong),
            p.text,
        ),
        (Kind::Ghost, false) => (Color32::TRANSPARENT, Stroke::NONE, p.text2),
        (Kind::Ghost, true) => (p.control, Stroke::NONE, p.text),
        (Kind::Danger, h) => (
            if h {
                p.danger.gamma_multiply(0.14)
            } else {
                Color32::TRANSPARENT
            },
            Stroke::new(1.0, p.danger.gamma_multiply(0.55)),
            p.danger,
        ),
    }
}

/// A text button with an optional leading icon, 32 px tall.
pub fn button(ui: &mut Ui, kind: Kind, icon: Option<Icon>, label: &str) -> Response {
    sized_button(ui, kind, icon, label, 32.0)
}

pub fn sized_button(
    ui: &mut Ui,
    kind: Kind,
    icon: Option<Icon>,
    label: &str,
    height: f32,
) -> Response {
    let p = pal(ui);
    let galley = ui.painter().layout_no_wrap(
        label.to_owned(),
        FontId::proportional(13.0),
        Color32::PLACEHOLDER,
    );
    let icon_w = if icon.is_some() { ICON + ICON_GAP } else { 0.0 };
    let pad = if icon.is_some() { 11.0 } else { 13.0 };
    let size = Vec2::new(galley.size().x + icon_w + pad * 2.0, height);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    resp.widget_info(|| WidgetInfo::labeled(Role::Button, ui.is_enabled(), label));
    if ui.is_rect_visible(rect) {
        let (fill, stroke, fg) = button_colors(p, kind, resp.hovered());
        let painter = ui.painter();
        painter.rect(rect, RADIUS_CONTROL, fill, stroke, StrokeKind::Inside);
        let mut x = rect.left() + pad;
        if let Some(icon) = icon {
            let r = Rect::from_min_size(
                Pos2::new(x, rect.center().y - ICON / 2.0),
                Vec2::splat(ICON),
            );
            let s = if icon == Icon::Stop || icon == Icon::Play {
                r.shrink(2.5)
            } else {
                r
            };
            icons::paint(painter, s, icon, fg);
            x += icon_w;
        }
        let pos = Pos2::new(x, rect.center().y - galley.size().y / 2.0);
        painter.galley(pos, galley, fg);
    }
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// A square icon-only button. `framed` adds a border (toolbar style);
/// unframed buttons only fill on hover (control rail style).
pub fn icon_button(ui: &mut Ui, icon: Icon, tooltip: &str, size: f32, framed: bool) -> Response {
    let p = pal(ui);
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    resp.widget_info(|| WidgetInfo::labeled(Role::Button, ui.is_enabled(), tooltip));
    if ui.is_rect_visible(rect) {
        let hovered = resp.hovered();
        let fill = if hovered {
            p.raised
        } else {
            Color32::TRANSPARENT
        };
        let stroke = if framed {
            Stroke::new(1.0, p.border)
        } else {
            Stroke::NONE
        };
        ui.painter()
            .rect(rect, RADIUS_CONTROL, fill, stroke, StrokeKind::Inside);
        let fg = if hovered { p.text } else { p.text2 };
        let glyph = Rect::from_center_size(rect.center(), Vec2::splat((size * 0.48).round()));
        icons::paint(ui.painter(), glyph, icon, fg);
    }
    resp.on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text(tooltip)
}

/// The Grid / List style toggle. Returns the index clicked, if any.
pub fn segmented(ui: &mut Ui, items: &[(Icon, &str)], selected: usize) -> Option<usize> {
    let p = pal(ui);
    let mut clicked = None;
    egui::Frame::new()
        .fill(p.bg)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(9)
        .inner_margin(3)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.horizontal(|ui| {
                for (i, (icon, label)) in items.iter().enumerate() {
                    let on = i == selected;
                    let galley = ui.painter().layout_no_wrap(
                        (*label).to_owned(),
                        FontId::proportional(13.0),
                        Color32::PLACEHOLDER,
                    );
                    let size = Vec2::new(galley.size().x + 14.0 + 6.0 + 24.0, 28.0);
                    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
                    resp.widget_info(|| WidgetInfo::selected(Role::Button, true, on, *label));
                    if on {
                        ui.painter().rect_filled(rect, 6, p.raised);
                    }
                    let fg = if on || resp.hovered() {
                        p.text
                    } else {
                        p.text2
                    };
                    let icon_rect = Rect::from_min_size(
                        Pos2::new(rect.left() + 12.0, rect.center().y - 7.0),
                        Vec2::splat(14.0),
                    );
                    icons::paint(ui.painter(), icon_rect, *icon, fg);
                    let pos = Pos2::new(
                        icon_rect.right() + 6.0,
                        rect.center().y - galley.size().y / 2.0,
                    );
                    ui.painter().galley(pos, galley, fg);
                    // Clicking the selected one counts too: "Grid" also leaves the focused view.
                    if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                        clicked = Some(i);
                    }
                }
            });
        });
    clicked
}

const PILL_FONT: f32 = 10.5;
const PILL_H: f32 = 20.0;

pub fn pill_size(painter: &Painter, label: &str) -> Vec2 {
    let g = painter.layout_no_wrap(
        label.to_owned(),
        FontId::proportional(PILL_FONT),
        Color32::PLACEHOLDER,
    );
    Vec2::new(g.size().x + 8.0 + 6.0 + 5.0 + 8.0, PILL_H)
}

/// Dot + uppercase word in a soft rounded pill, filling `rect`.
pub fn paint_pill(painter: &Painter, p: &Palette, rect: Rect, label: &str, tone: Tone) {
    let (fg, soft) = tone.colors(p);
    painter.rect_filled(rect, CornerRadius::same(255), soft);
    painter.circle_filled(Pos2::new(rect.left() + 11.0, rect.center().y), 3.0, fg);
    painter.text(
        Pos2::new(rect.left() + 19.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(PILL_FONT),
        fg,
    );
}

pub fn status_pill(ui: &mut Ui, label: &str, tone: Tone) -> Response {
    let size = pill_size(ui.painter(), label);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    paint_pill(ui.painter(), pal(ui), rect, label, tone);
    resp
}

/// A status dot; `hollow` for stopped devices.
pub fn paint_dot(painter: &Painter, center: Pos2, color: Color32, hollow: bool) {
    if hollow {
        painter.circle_stroke(center, 3.25, Stroke::new(1.5, color));
    } else {
        painter.circle_filled(center, 4.0, color);
    }
}

/// A keyboard key chip.
pub fn kbd(ui: &mut Ui, text: &str) -> Response {
    let p = pal(ui);
    let galley = ui.painter().layout_no_wrap(
        text.to_owned(),
        FontId::monospace(10.5),
        Color32::PLACEHOLDER,
    );
    let size = galley.size() + Vec2::new(12.0, 6.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter();
    painter.rect(
        rect,
        5,
        p.control,
        Stroke::new(1.0, p.border),
        StrokeKind::Inside,
    );
    painter.line_segment(
        [
            rect.left_bottom() + Vec2::new(3.0, -0.5),
            rect.right_bottom() + Vec2::new(-3.0, -0.5),
        ],
        Stroke::new(1.0, p.border_strong),
    );
    painter.galley(rect.min + Vec2::new(6.0, 3.0), galley, p.text);
    resp
}

/// Small uppercase heading (e.g. "ANDROID EMULATORS").
pub fn section_label(ui: &mut Ui, text: &str) -> Response {
    let p = pal(ui);
    ui.label(
        RichText::new(text.to_uppercase())
            .size(11.0)
            .color(p.text3)
            .extra_letter_spacing(0.6),
    )
}

/// Size of a bordered tag such as "gRPC".
pub fn tag_size(painter: &Painter, text: &str) -> Vec2 {
    let g = painter.layout_no_wrap(
        text.to_owned(),
        FontId::proportional(10.5),
        Color32::PLACEHOLDER,
    );
    g.size() + Vec2::new(12.0, 5.0)
}

pub fn paint_tag(painter: &Painter, p: &Palette, rect: Rect, text: &str) {
    painter.rect_stroke(rect, 5, Stroke::new(1.0, p.border), StrokeKind::Inside);
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        FontId::proportional(10.5),
        p.text2,
    );
}

/// Width of the black frame for a bezel with this outer corner radius.
pub fn bezel_pad(radius: f32) -> f32 {
    (radius * 0.22).clamp(4.0, 9.0)
}

/// Black phone frame filling `rect`; returns the screen area inside it and
/// that area's corner radius.
pub fn bezel(painter: &Painter, p: &Palette, rect: Rect, radius: f32) -> (Rect, f32) {
    painter.rect(
        rect,
        CornerRadius::same(radius.round() as u8),
        BEZEL,
        Stroke::new(1.0, p.bezel_edge),
        StrokeKind::Inside,
    );
    let pad = bezel_pad(radius);
    (rect.shrink(pad), (radius - pad).max(2.0))
}

/// A texture drawn with rounded corners.
pub fn paint_rounded_texture(painter: &Painter, tex: TextureId, rect: Rect, radius: f32) {
    painter.add(
        RectShape::filled(
            rect,
            CornerRadius::same(radius.round() as u8),
            Color32::WHITE,
        )
        .with_texture(tex, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0))),
    );
}

/// The app mark (2×2 rounded squares) and name.
pub fn wordmark(ui: &mut Ui) {
    let p = pal(ui);
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::hover());
    let cell = 8.0;
    for (i, alpha) in [1.0, 0.55, 0.55, 0.3].into_iter().enumerate() {
        let min = rect.min + Vec2::new(1.0 + (i % 2) as f32 * 10.0, 1.0 + (i / 2) as f32 * 10.0);
        ui.painter().rect_filled(
            Rect::from_min_size(min, Vec2::splat(cell)),
            2,
            p.accent.gamma_multiply(alpha),
        );
    }
    ui.label(
        RichText::new("emugrid")
            .monospace()
            .size(14.0)
            .color(p.text)
            .strong(),
    );
}

/// A primary "Start" button joined to a chevron that opens `menu`.
/// Returns the main part's response.
pub fn split_button(ui: &mut Ui, label: &str, menu: impl FnOnce(&mut Ui)) -> Response {
    let p = pal(ui);
    let r = CornerRadius::same(RADIUS_CONTROL);
    let prev = ui.spacing().item_spacing.x;
    ui.spacing_mut().item_spacing.x = 0.0;

    // In a right-to-left layout allocate the chevron first, so it still ends
    // up on the right of the main part.
    let rtl = ui.layout().prefer_right_to_left();
    let galley = ui.painter().layout_no_wrap(
        label.to_owned(),
        FontId::proportional(13.0),
        Color32::PLACEHOLDER,
    );
    let main_size = Vec2::new(galley.size().x + 11.0 + 11.0 + 10.0 + 6.0, 30.0);
    let caret_size = Vec2::new(26.0, 30.0);
    let (main_rect, main, caret_rect, caret) = if rtl {
        let (cr, c) = ui.allocate_exact_size(caret_size, Sense::click());
        let (mr, m) = ui.allocate_exact_size(main_size, Sense::click());
        (mr, m, cr, c)
    } else {
        let (mr, m) = ui.allocate_exact_size(main_size, Sense::click());
        let (cr, c) = ui.allocate_exact_size(caret_size, Sense::click());
        (mr, m, cr, c)
    };
    ui.spacing_mut().item_spacing.x = prev;
    main.widget_info(|| WidgetInfo::labeled(Role::Button, ui.is_enabled(), label));
    caret.widget_info(|| WidgetInfo::labeled(Role::Button, ui.is_enabled(), "More start options"));

    let painter = ui.painter();
    let fill = |h: bool| {
        if h {
            p.accent.lerp_to_gamma(p.text, 0.12)
        } else {
            p.accent
        }
    };
    painter.rect_filled(
        main_rect,
        CornerRadius {
            nw: r.nw,
            sw: r.sw,
            ne: 0,
            se: 0,
        },
        fill(main.hovered()),
    );
    painter.rect_filled(
        caret_rect,
        CornerRadius {
            nw: 0,
            sw: 0,
            ne: r.ne,
            se: r.se,
        },
        fill(caret.hovered()),
    );
    painter.vline(
        caret_rect.left(),
        caret_rect.y_range().shrink(6.0),
        Stroke::new(1.0, p.on_accent.gamma_multiply(0.3)),
    );
    let play = Rect::from_min_size(
        Pos2::new(main_rect.left() + 11.0, main_rect.center().y - 5.0),
        Vec2::splat(10.0),
    );
    icons::paint(painter, play, Icon::Play, p.on_accent);
    painter.galley(
        Pos2::new(
            play.right() + 6.0,
            main_rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        p.on_accent,
    );
    icons::paint(
        painter,
        Rect::from_center_size(caret_rect.center(), Vec2::splat(13.0)),
        Icon::ChevronDown,
        p.on_accent,
    );

    let caret = caret
        .on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text("More ways to start");
    egui::Popup::menu(&caret)
        .align(egui::RectAlign::BOTTOM_END)
        .gap(4.0)
        .width(240.0)
        .show(menu);
    main.on_hover_cursor(CursorIcon::PointingHand)
}

/// A menu entry with a title and a dimmer description line.
pub fn menu_item(ui: &mut Ui, title: &str, detail: &str, danger: bool) -> Response {
    let p = pal(ui);
    let (fg, dim) = if danger {
        (p.danger, p.danger.gamma_multiply(0.75))
    } else {
        (p.text, p.text2)
    };
    let width = ui.available_width().max(200.0);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 44.0), Sense::click());
    resp.widget_info(|| WidgetInfo::labeled(Role::MenuItem, ui.is_enabled(), title));
    let painter = ui.painter();
    if resp.hovered() {
        painter.rect_filled(rect, 7, p.raised);
    }
    painter.text(
        rect.left_top() + Vec2::new(10.0, 7.0),
        Align2::LEFT_TOP,
        title,
        FontId::proportional(13.0),
        fg,
    );
    painter.text(
        rect.left_top() + Vec2::new(10.0, 25.0),
        Align2::LEFT_TOP,
        detail,
        FontId::proportional(11.5),
        dim,
    );
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// Single-line text cut off with "…" at `max_width`.
pub fn truncated(
    painter: &Painter,
    text: String,
    font: FontId,
    color: Color32,
    max_width: f32,
) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text, font, color);
    job.wrap.max_width = max_width;
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    painter.layout_job(job)
}

/// An icon laid out inline, like a glyph.
pub fn icon(ui: &mut Ui, icon: Icon, size: f32, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    icons::paint(ui.painter(), rect, icon, color);
}
