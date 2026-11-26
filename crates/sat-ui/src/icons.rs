//! Small vector icons drawn with paths, sized to a square box.

use sat_theme::Rgba;
use vello::kurbo::{BezPath, Point, Rect};

use crate::paint::Painter;

/// Icon shapes used by the shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    /// Play triangle.
    Play,
    /// Stop square.
    Stop,
    /// Record dot.
    Record,
    /// Loop arrows.
    Loop,
    /// Down chevron.
    ChevronDown,
    /// Right chevron.
    ChevronRight,
    /// Left chevron.
    ChevronLeft,
    /// Plus.
    Plus,
    /// Cross (close, remove).
    Close,
    /// Sidebar panel.
    Panel,
    /// Grid of four squares.
    Grid,
    /// Arrange lines.
    Lines,
    /// Mixer sliders.
    Sliders,
    /// Undo arrow.
    Undo,
    /// Redo arrow.
    Redo,
    /// Search glass.
    Search,
    /// Gear.
    Gear,
    /// Pencil.
    Pencil,
    /// Pointer arrow.
    Pointer,
    /// Waveform pulse.
    Pulse,
    /// Power switch (bypass).
    Power,
    /// A tree of rows (outliner).
    Tree,
    /// Cards (clip bank).
    Cards,
    /// A knob (inspector).
    Knob,
    /// Piano keys (clip editor).
    Keys,
    /// Up chevron.
    ChevronUp,
    /// A folder (browser).
    Folder,
}

/// Draws `icon` centred in `r`.
pub fn draw(p: &mut Painter<'_>, icon: Icon, r: Rect, c: Rgba) {
    let cx = r.center().x;
    let cy = r.center().y;
    let s = r.width().min(r.height()) / 2.0;
    let pt = |x: f64, y: f64| Point::new(cx + x * s, cy + y * s);
    let w = 1.3;
    match icon {
        Icon::Play => {
            let mut b = BezPath::new();
            b.move_to(pt(-0.45, -0.6));
            b.line_to(pt(0.6, 0.0));
            b.line_to(pt(-0.45, 0.6));
            b.close_path();
            p.path(&b, c);
        }
        Icon::Stop => p.rrect(Rect::new(cx - 0.45 * s, cy - 0.45 * s, cx + 0.45 * s, cy + 0.45 * s), 1.5, c),
        Icon::Record => p.circle(r.center(), 0.42 * s, c),
        Icon::Loop => {
            p.polyline(&[pt(-0.7, 0.1), pt(-0.7, -0.35), pt(0.55, -0.35)], w, c);
            p.polyline(&[pt(0.3, -0.6), pt(0.6, -0.35), pt(0.3, -0.1)], w, c);
            p.polyline(&[pt(0.7, -0.1), pt(0.7, 0.35), pt(-0.55, 0.35)], w, c);
            p.polyline(&[pt(-0.3, 0.1), pt(-0.6, 0.35), pt(-0.3, 0.6)], w, c);
        }
        Icon::ChevronDown => p.polyline(&[pt(-0.4, -0.2), pt(0.0, 0.2), pt(0.4, -0.2)], w, c),
        Icon::ChevronRight => p.polyline(&[pt(-0.2, -0.4), pt(0.2, 0.0), pt(-0.2, 0.4)], w, c),
        Icon::ChevronLeft => p.polyline(&[pt(0.2, -0.4), pt(-0.2, 0.0), pt(0.2, 0.4)], w, c),
        Icon::Plus => {
            p.line(pt(-0.5, 0.0), pt(0.5, 0.0), w, c);
            p.line(pt(0.0, -0.5), pt(0.0, 0.5), w, c);
        }
        Icon::Close => {
            p.line(pt(-0.4, -0.4), pt(0.4, 0.4), w, c);
            p.line(pt(-0.4, 0.4), pt(0.4, -0.4), w, c);
        }
        Icon::Panel => {
            p.rrect_stroke(Rect::new(cx - 0.7 * s, cy - 0.6 * s, cx + 0.7 * s, cy + 0.6 * s), 2.0, 1.0, c);
            p.vline(cx - 0.25 * s, cy - 0.6 * s, cy + 0.6 * s, c);
        }
        Icon::Grid => {
            for (x, y) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                let q = Rect::from_center_size(pt(x * 0.35, y * 0.35), (0.5 * s, 0.5 * s));
                p.rrect_stroke(q, 1.5, 1.0, c);
            }
        }
        Icon::Lines => {
            for (i, l) in [0.9, 0.6, 0.75].iter().enumerate() {
                let y = -0.4 + i as f64 * 0.4;
                p.line(pt(-0.6, y), pt(-0.6 + 1.2 * l, y), w, c);
            }
        }
        Icon::Sliders => {
            for (i, k) in [-0.2, 0.3, -0.05].iter().enumerate() {
                let x = -0.5 + i as f64 * 0.5;
                p.line(pt(x, -0.6), pt(x, 0.6), 1.0, c);
                p.line(pt(x - 0.18, *k), pt(x + 0.18, *k), 2.0, c);
            }
        }
        Icon::Undo | Icon::Redo => {
            let f = if icon == Icon::Undo { 1.0 } else { -1.0 };
            p.polyline(&[pt(-0.3 * f, -0.55), pt(-0.6 * f, -0.25), pt(-0.3 * f, 0.05)], w, c);
            let mut b = BezPath::new();
            b.move_to(pt(-0.6 * f, -0.25));
            b.line_to(pt(0.15 * f, -0.25));
            b.quad_to(pt(0.6 * f, -0.25), pt(0.6 * f, 0.15));
            b.quad_to(pt(0.6 * f, 0.55), pt(0.15 * f, 0.55));
            p.scene.stroke(
                &vello::kurbo::Stroke::new(w),
                vello::kurbo::Affine::IDENTITY,
                crate::paint::color(c),
                None,
                &b,
            );
        }
        Icon::Search => {
            p.circle_stroke(pt(-0.1, -0.1), 0.4 * s, w, c);
            p.line(pt(0.2, 0.2), pt(0.6, 0.6), w, c);
        }
        Icon::Gear => {
            p.circle_stroke(r.center(), 0.3 * s, w, c);
            for k in 0..8 {
                let a = k as f64 * std::f64::consts::FRAC_PI_4;
                p.line(pt(0.45 * a.cos(), 0.45 * a.sin()), pt(0.7 * a.cos(), 0.7 * a.sin()), 1.6, c);
            }
        }
        Icon::Pencil => {
            p.line(pt(-0.5, 0.5), pt(0.45, -0.45), 2.0, c);
            p.line(pt(-0.6, 0.6), pt(-0.45, 0.45), 1.0, c);
        }
        Icon::Pointer => {
            let mut b = BezPath::new();
            b.move_to(pt(-0.4, -0.6));
            b.line_to(pt(0.45, 0.15));
            b.line_to(pt(0.0, 0.15));
            b.line_to(pt(-0.2, 0.6));
            b.close_path();
            p.scene.stroke(
                &vello::kurbo::Stroke::new(1.1),
                vello::kurbo::Affine::IDENTITY,
                crate::paint::color(c),
                None,
                &b,
            );
        }
        Icon::Power => {
            // an open ring with a bar through the gap
            let pts: Vec<Point> = (0..=20)
                .map(|k| {
                    let a = -std::f64::consts::FRAC_PI_2 + 0.65 + k as f64 / 20.0 * (std::f64::consts::TAU - 1.3);
                    pt(0.55 * a.cos(), 0.55 * a.sin())
                })
                .collect();
            p.polyline(&pts, w, c);
            p.line(pt(0.0, -0.7), pt(0.0, -0.1), w, c);
        }
        Icon::Tree => {
            for (i, x) in [-0.6, -0.25, -0.25, -0.6].iter().enumerate() {
                let y = -0.55 + i as f64 * 0.37;
                p.circle(pt(*x, y), 0.09 * s, c);
                p.line(pt(x + 0.2, y), pt(0.65, y), w, c);
            }
            p.line(pt(-0.6, -0.45), pt(-0.6, 0.0), 1.0, c);
            p.line(pt(-0.6, -0.18), pt(-0.35, -0.18), 1.0, c);
        }
        Icon::Cards => {
            for (x, y) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                let q = Rect::from_center_size(pt(x * 0.36, y * 0.33), (0.6 * s, 0.5 * s));
                p.rrect_stroke(q, 1.5, 1.0, c);
                p.rect(Rect::new(q.x0, q.y0, q.x1, q.y0 + 0.16 * s), c);
            }
        }
        Icon::Knob => {
            p.circle_stroke(r.center(), 0.55 * s, w, c);
            p.line(r.center(), pt(-0.32, -0.32), 1.6, c);
        }
        Icon::Keys => {
            p.rrect_stroke(Rect::new(cx - 0.7 * s, cy - 0.6 * s, cx + 0.7 * s, cy + 0.6 * s), 1.5, 1.0, c);
            for x in [-0.23, 0.23] {
                p.vline(cx + x * s, cy - 0.6 * s, cy + 0.6 * s, c);
            }
            for x in [-0.23, 0.23] {
                p.rect(Rect::new(cx + (x - 0.1) * s, cy - 0.6 * s, cx + (x + 0.1) * s, cy + 0.05 * s), c);
            }
        }
        Icon::Folder => {
            let mut b = BezPath::new();
            b.move_to(pt(-0.7, -0.5));
            b.line_to(pt(-0.2, -0.5));
            b.line_to(pt(0.0, -0.3));
            b.line_to(pt(0.7, -0.3));
            b.line_to(pt(0.7, 0.55));
            b.line_to(pt(-0.7, 0.55));
            b.close_path();
            p.scene.stroke(
                &vello::kurbo::Stroke::new(1.1),
                vello::kurbo::Affine::IDENTITY,
                crate::paint::color(c),
                None,
                &b,
            );
            p.line(pt(-0.7, -0.1), pt(0.7, -0.1), 1.0, c);
        }
        Icon::ChevronUp => p.polyline(&[pt(-0.4, 0.2), pt(0.0, -0.2), pt(0.4, 0.2)], w, c),
        Icon::Pulse => {
            p.polyline(
                &[pt(-0.7, 0.0), pt(-0.35, 0.0), pt(-0.15, -0.55), pt(0.15, 0.55), pt(0.35, 0.0), pt(0.7, 0.0)],
                w,
                c,
            );
        }
    }
}
