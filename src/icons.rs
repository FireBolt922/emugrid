//! Line icons painted with egui shapes: no icon font, no extra bytes.
//! Each icon is drawn on a 24×24 grid scaled into the target rect.

use eframe::egui::{Color32, CornerRadius, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, Vec2};
use std::f32::consts::PI;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Icon {
    Grid,
    List,
    Refresh,
    ChevronLeft,
    ChevronDown,
    /// Android back (triangle).
    Back,
    /// Android home (circle).
    Home,
    /// Android recents (square).
    Recents,
    VolumeDown,
    VolumeUp,
    Power,
    Rotate,
    Camera,
    Play,
    Stop,
    Upload,
    Keyboard,
    Phone,
    Warning,
}

impl Icon {
    #[cfg(test)]
    pub const ALL: [Icon; 19] = [
        Icon::Grid,
        Icon::List,
        Icon::Refresh,
        Icon::ChevronLeft,
        Icon::ChevronDown,
        Icon::Back,
        Icon::Home,
        Icon::Recents,
        Icon::VolumeDown,
        Icon::VolumeUp,
        Icon::Power,
        Icon::Rotate,
        Icon::Camera,
        Icon::Play,
        Icon::Stop,
        Icon::Upload,
        Icon::Keyboard,
        Icon::Phone,
        Icon::Warning,
    ];
}

pub fn paint(painter: &Painter, rect: Rect, icon: Icon, color: Color32) {
    painter.extend(shapes(icon, rect, color));
}

/// Maps 24-grid coordinates into a square centred in `rect`.
struct Grid {
    origin: Pos2,
    scale: f32,
    stroke: Stroke,
    color: Color32,
}

impl Grid {
    fn new(rect: Rect, color: Color32) -> Self {
        let size = rect.width().min(rect.height());
        let origin = rect.center() - Vec2::splat(size / 2.0);
        let scale = size / 24.0;
        Self {
            origin,
            scale,
            stroke: Stroke::new((scale * 2.0).max(1.2), color),
            color,
        }
    }

    fn p(&self, x: f32, y: f32) -> Pos2 {
        self.origin + Vec2::new(x, y) * self.scale
    }

    fn pts(&self, xy: &[(f32, f32)]) -> Vec<Pos2> {
        xy.iter().map(|&(x, y)| self.p(x, y)).collect()
    }

    fn line(&self, xy: &[(f32, f32)]) -> Shape {
        Shape::line(self.pts(xy), self.stroke)
    }

    fn closed(&self, xy: &[(f32, f32)]) -> Shape {
        Shape::closed_line(self.pts(xy), self.stroke)
    }

    fn circle(&self, x: f32, y: f32, r: f32) -> Shape {
        Shape::circle_stroke(self.p(x, y), r * self.scale, self.stroke)
    }

    fn dot(&self, x: f32, y: f32, r: f32) -> Shape {
        Shape::circle_filled(self.p(x, y), r * self.scale, self.color)
    }

    fn rect(&self, x0: f32, y0: f32, x1: f32, y1: f32, r: f32) -> Shape {
        Shape::rect_stroke(
            Rect::from_min_max(self.p(x0, y0), self.p(x1, y1)),
            CornerRadius::same((r * self.scale).round() as u8),
            self.stroke,
            StrokeKind::Middle,
        )
    }

    fn filled_rect(&self, x0: f32, y0: f32, x1: f32, y1: f32, r: f32) -> Shape {
        Shape::rect_filled(
            Rect::from_min_max(self.p(x0, y0), self.p(x1, y1)),
            CornerRadius::same((r * self.scale).round() as u8),
            self.color,
        )
    }

    /// Arc around (cx, cy) from angle `a0` to `a1` in degrees, y pointing down
    /// (0° = right, 90° = bottom).
    fn arc(&self, cx: f32, cy: f32, r: f32, a0: f32, a1: f32) -> Shape {
        let steps = (((a1 - a0).abs() / 12.0).ceil() as usize).max(2);
        let pts = (0..=steps)
            .map(|i| {
                let a = (a0 + (a1 - a0) * i as f32 / steps as f32) * PI / 180.0;
                self.p(cx + r * a.cos(), cy + r * a.sin())
            })
            .collect();
        Shape::line(pts, self.stroke)
    }
}

pub fn shapes(icon: Icon, rect: Rect, color: Color32) -> Vec<Shape> {
    let g = Grid::new(rect, color);
    match icon {
        Icon::Grid => vec![
            g.rect(3.0, 3.0, 10.0, 10.0, 1.5),
            g.rect(14.0, 3.0, 21.0, 10.0, 1.5),
            g.rect(3.0, 14.0, 10.0, 21.0, 1.5),
            g.rect(14.0, 14.0, 21.0, 21.0, 1.5),
        ],
        Icon::List => vec![
            g.line(&[(8.0, 6.0), (21.0, 6.0)]),
            g.line(&[(8.0, 12.0), (21.0, 12.0)]),
            g.line(&[(8.0, 18.0), (21.0, 18.0)]),
            g.dot(3.5, 6.0, 1.3),
            g.dot(3.5, 12.0, 1.3),
            g.dot(3.5, 18.0, 1.3),
        ],
        Icon::Refresh => vec![
            g.arc(12.0, 12.0, 8.0, 0.0, 315.0),
            g.line(&[(17.66, 6.34), (20.0, 8.5)]),
            g.line(&[(20.0, 3.5), (20.0, 8.5), (15.0, 8.5)]),
        ],
        Icon::ChevronLeft => vec![g.line(&[(15.0, 18.0), (9.0, 12.0), (15.0, 6.0)])],
        Icon::ChevronDown => vec![g.line(&[(6.0, 9.0), (12.0, 15.0), (18.0, 9.0)])],
        Icon::Back => vec![g.closed(&[(17.0, 5.0), (17.0, 19.0), (6.0, 12.0)])],
        Icon::Home => vec![g.circle(12.0, 12.0, 7.0)],
        Icon::Recents => vec![g.rect(5.5, 5.5, 18.5, 18.5, 2.0)],
        Icon::VolumeDown | Icon::VolumeUp => {
            let mut v = vec![
                g.closed(&[
                    (11.0, 5.0),
                    (6.0, 9.0),
                    (3.0, 9.0),
                    (3.0, 15.0),
                    (6.0, 15.0),
                    (11.0, 19.0),
                ]),
                g.arc(13.05, 12.0, 3.5, -45.6, 45.6),
            ];
            if icon == Icon::VolumeUp {
                v.push(g.arc(12.7, 12.0, 8.0, -43.4, 43.4));
            }
            v
        }
        Icon::Power => vec![
            g.line(&[(12.0, 3.0), (12.0, 12.0)]),
            g.arc(12.0, 12.0, 8.0, -45.0, 225.0),
        ],
        Icon::Rotate => vec![
            g.arc(12.0, 12.0, 8.0, 180.0, 315.0),
            g.line(&[(17.66, 6.34), (20.0, 8.5)]),
            g.line(&[(20.0, 3.5), (20.0, 8.5), (15.0, 8.5)]),
            g.arc(12.0, 12.0, 8.0, 0.0, 135.0),
            g.line(&[(6.34, 17.66), (4.0, 15.5)]),
            g.line(&[(4.0, 20.5), (4.0, 15.5), (9.0, 15.5)]),
        ],
        Icon::Camera => vec![
            g.closed(&[
                (4.0, 8.0),
                (7.0, 8.0),
                (9.0, 5.0),
                (15.0, 5.0),
                (17.0, 8.0),
                (20.0, 8.0),
                (20.0, 19.0),
                (4.0, 19.0),
            ]),
            g.circle(12.0, 13.0, 3.3),
        ],
        Icon::Play => vec![Shape::convex_polygon(
            g.pts(&[(7.0, 4.5), (19.5, 12.0), (7.0, 19.5)]),
            color,
            Stroke::NONE,
        )],
        Icon::Stop => vec![g.filled_rect(5.0, 5.0, 19.0, 19.0, 2.5)],
        Icon::Upload => vec![
            g.line(&[(12.0, 15.0), (12.0, 3.0)]),
            g.line(&[(7.0, 8.0), (12.0, 3.0), (17.0, 8.0)]),
            g.line(&[(5.0, 21.0), (19.0, 21.0)]),
        ],
        Icon::Keyboard => vec![
            g.rect(2.5, 6.0, 21.5, 18.0, 2.0),
            g.dot(6.5, 10.0, 1.0),
            g.dot(10.0, 10.0, 1.0),
            g.dot(14.0, 10.0, 1.0),
            g.dot(17.5, 10.0, 1.0),
            g.line(&[(7.5, 14.0), (16.5, 14.0)]),
        ],
        Icon::Phone => vec![
            g.rect(6.0, 2.5, 18.0, 21.5, 2.5),
            g.line(&[(11.0, 18.5), (13.0, 18.5)]),
        ],
        Icon::Warning => vec![
            g.closed(&[(12.0, 3.5), (21.5, 20.0), (2.5, 20.0)]),
            g.line(&[(12.0, 10.0), (12.0, 14.5)]),
            g.dot(12.0, 17.3, 1.1),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every icon draws something, and stays inside its box.
    #[test]
    fn every_icon_paints_inside_its_rect() {
        let rect = Rect::from_min_size(Pos2::new(10.0, 20.0), Vec2::splat(24.0));
        for icon in Icon::ALL {
            let shapes = shapes(icon, rect, Color32::WHITE);
            assert!(!shapes.is_empty(), "{icon:?} draws nothing");
            for s in &shapes {
                let b = s.visual_bounding_rect();
                assert!(
                    rect.expand(2.0).contains_rect(b),
                    "{icon:?} spills out: {b:?}"
                );
            }
        }
    }
}
