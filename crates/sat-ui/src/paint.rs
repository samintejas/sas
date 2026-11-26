//! A thin drawing layer over a vello scene, with text from parley.

use std::sync::Arc;

use parley::{FontContext, FontFamily, FontWeight, Layout, LayoutContext, PositionedLayoutItem, StyleProperty};
use sat_theme::Rgba;
use vello::kurbo::{Affine, Arc as KArc, BezPath, Cap, Circle, Line, Point, Rect, RoundedRect, Stroke};
use vello::peniko::{Blob, Color, ColorStop, Fill, Gradient};
use vello::{Glyph, Scene};

const GEIST: &[u8] = include_bytes!("../../../assets/fonts/Geist-Variable.ttf");
const GEIST_MONO: &[u8] = include_bytes!("../../../assets/fonts/GeistMono-Variable.ttf");

/// Converts a theme colour.
pub fn color(c: Rgba) -> Color {
    Color::from_rgba8(c.r, c.g, c.b, c.a)
}

/// Which face to set text in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Face {
    /// Geist: labels.
    Sans,
    /// Geist Mono: numbers and the dense DAW surface.
    Mono,
}

/// Text style.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextStyle {
    /// Face.
    pub face: Face,
    /// Size in px.
    pub size: f32,
    /// Weight (400 regular, 500 medium).
    pub weight: f32,
    /// Colour.
    pub color: Rgba,
}

impl TextStyle {
    /// Mono at `size` in `color`, regular weight.
    pub fn mono(size: f32, color: Rgba) -> TextStyle {
        TextStyle { face: Face::Mono, size, weight: 400.0, color }
    }

    /// Sans at `size` in `color`, regular weight.
    pub fn sans(size: f32, color: Rgba) -> TextStyle {
        TextStyle { face: Face::Sans, size, weight: 400.0, color }
    }

    /// The same style at medium weight.
    pub fn medium(self) -> TextStyle {
        TextStyle { weight: 500.0, ..self }
    }
}

/// Horizontal text alignment relative to the x you pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    /// x is the left edge.
    Left,
    /// x is the centre.
    Center,
    /// x is the right edge.
    Right,
}

/// Font database and layout scratch space; create once and reuse.
pub struct Fonts {
    font_cx: FontContext,
    layout_cx: LayoutContext<()>,
}

impl Default for Fonts {
    fn default() -> Self {
        Fonts::new()
    }
}

impl Fonts {
    /// Loads the bundled Geist fonts (system fonts are not used, so output is
    /// the same on every machine).
    pub fn new() -> Fonts {
        let mut font_cx = FontContext {
            collection: parley::fontique::Collection::new(parley::fontique::CollectionOptions {
                shared: false,
                system_fonts: false,
            }),
            source_cache: Default::default(),
        };
        for bytes in [GEIST, GEIST_MONO] {
            font_cx.collection.register_fonts(Blob::new(Arc::new(bytes)), None);
        }
        Fonts { font_cx, layout_cx: LayoutContext::new() }
    }

    fn layout(&mut self, text: &str, st: &TextStyle) -> Layout<()> {
        let mut b = self.layout_cx.ranged_builder(&mut self.font_cx, text, 1.0, true);
        let family = match st.face {
            Face::Sans => sat_theme::FONT_SANS,
            Face::Mono => sat_theme::FONT_MONO,
        };
        b.push_default(StyleProperty::FontFamily(FontFamily::named(family)));
        b.push_default(StyleProperty::FontSize(st.size));
        b.push_default(StyleProperty::FontWeight(FontWeight::new(st.weight)));
        let mut l = b.build(text);
        l.break_all_lines(None);
        l
    }

    /// Width of `text` in px.
    pub fn measure(&mut self, text: &str, st: &TextStyle) -> f32 {
        self.layout(text, st).width()
    }
}

/// Draws into a scene.
pub struct Painter<'a> {
    /// The scene being built.
    pub scene: &'a mut Scene,
    /// Fonts.
    pub fonts: &'a mut Fonts,
}

impl Painter<'_> {
    /// Fills a rectangle.
    pub fn rect(&mut self, r: Rect, c: Rgba) {
        if c.a == 0 || r.is_zero_area() {
            return;
        }
        self.scene.fill(Fill::NonZero, Affine::IDENTITY, color(c), None, &r);
    }

    /// Fills a rounded rectangle.
    pub fn rrect(&mut self, r: Rect, radius: f64, c: Rgba) {
        if c.a == 0 {
            return;
        }
        self.scene.fill(Fill::NonZero, Affine::IDENTITY, color(c), None, &RoundedRect::from_rect(r, radius));
    }

    /// A 1 px outline just inside `r`.
    pub fn rect_stroke_px(&mut self, r: Rect, c: Rgba) {
        self.hline(r.x0, r.x1, r.y0 + 0.5, c);
        self.hline(r.x0, r.x1, r.y1 - 0.5, c);
        self.vline(r.x0 + 0.5, r.y0, r.y1, c);
        self.vline(r.x1 - 0.5, r.y0, r.y1, c);
    }

    /// Strokes a rounded rectangle inside `r` (pixel-aligned for 1 px lines).
    pub fn rrect_stroke(&mut self, r: Rect, radius: f64, width: f64, c: Rgba) {
        if c.a == 0 {
            return;
        }
        let r = r.inset(-width / 2.0);
        self.scene.stroke(&Stroke::new(width), Affine::IDENTITY, color(c), None, &RoundedRect::from_rect(r, radius));
    }

    /// A horizontal 1 px line at y covering x0..x1.
    pub fn hline(&mut self, x0: f64, x1: f64, y: f64, c: Rgba) {
        self.rect(Rect::new(x0, y.floor(), x1, y.floor() + 1.0), c);
    }

    /// A vertical 1 px line at x covering y0..y1.
    pub fn vline(&mut self, x: f64, y0: f64, y1: f64, c: Rgba) {
        self.rect(Rect::new(x.floor(), y0, x.floor() + 1.0, y1), c);
    }

    /// A line between two points.
    pub fn line(&mut self, a: Point, b: Point, width: f64, c: Rgba) {
        self.scene.stroke(&Stroke::new(width), Affine::IDENTITY, color(c), None, &Line::new(a, b));
    }

    /// A polyline.
    pub fn polyline(&mut self, pts: &[Point], width: f64, c: Rgba) {
        if pts.len() < 2 {
            return;
        }
        let mut p = BezPath::new();
        p.move_to(pts[0]);
        for q in &pts[1..] {
            p.line_to(*q);
        }
        self.scene.stroke(&Stroke::new(width), Affine::IDENTITY, color(c), None, &p);
    }

    /// A cable from `a` to `b`: a cubic curve that leaves `a` to the right
    /// and arrives at `b` from the left, dashed when `dashed`.
    pub fn cable(&mut self, a: Point, b: Point, width: f64, c: Rgba, dashed: bool) {
        let k = ((b.x - a.x).abs() * 0.5).max(40.0);
        let mut p = BezPath::new();
        p.move_to(a);
        p.curve_to(Point::new(a.x + k, a.y), Point::new(b.x - k, b.y), b);
        let mut st = Stroke::new(width).with_caps(vello::kurbo::Cap::Round);
        if dashed {
            st = st.with_dashes(0.0, [4.0, 4.0]);
        }
        self.scene.stroke(&st, Affine::IDENTITY, color(c), None, &p);
    }

    /// A filled path.
    pub fn path(&mut self, p: &BezPath, c: Rgba) {
        self.scene.fill(Fill::NonZero, Affine::IDENTITY, color(c), None, p);
    }

    /// A filled circle.
    pub fn circle(&mut self, center: Point, r: f64, c: Rgba) {
        self.scene.fill(Fill::NonZero, Affine::IDENTITY, color(c), None, &Circle::new(center, r));
    }

    /// A stroked circle.
    pub fn circle_stroke(&mut self, center: Point, r: f64, width: f64, c: Rgba) {
        self.scene.stroke(&Stroke::new(width), Affine::IDENTITY, color(c), None, &Circle::new(center, r));
    }

    /// An arc of a circle with round caps. Angles are in radians, clockwise
    /// from 3 o'clock (y points down).
    pub fn arc(&mut self, center: Point, r: f64, start: f64, sweep: f64, width: f64, c: Rgba) {
        if sweep.abs() < 1e-6 {
            return;
        }
        let a = KArc::new(center, (r, r), start, sweep, 0.0);
        self.scene.stroke(&Stroke::new(width).with_caps(Cap::Round), Affine::IDENTITY, color(c), None, &a);
    }

    /// A rounded rectangle filled with a vertical gradient. `stops` run from
    /// the bottom (0) to the top (1) of `full`, so a partly filled meter keeps
    /// its colours in place; `r` is the part to fill.
    pub fn vgradient(&mut self, r: Rect, full: Rect, radius: f64, stops: &[(f32, Rgba)]) {
        if r.is_zero_area() {
            return;
        }
        let g = Gradient::new_linear((full.x0, full.y1), (full.x0, full.y0))
            .with_stops(stops.iter().map(|&(o, c)| ColorStop::from((o, color(c)))).collect::<Vec<_>>().as_slice());
        self.scene.fill(Fill::NonZero, Affine::IDENTITY, &g, None, &RoundedRect::from_rect(r, radius));
    }

    /// Clips everything drawn inside `f` to `r`.
    pub fn clip(&mut self, r: Rect, f: impl FnOnce(&mut Painter<'_>)) {
        self.scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &r);
        f(self);
        self.scene.pop_layer();
    }

    /// Draws `text` with its baseline vertically centred on `cy` and aligned to `x`.
    /// Returns the width.
    pub fn text(&mut self, x: f64, cy: f64, text: &str, st: &TextStyle, align: Align) -> f64 {
        if text.is_empty() {
            return 0.0;
        }
        let layout = self.fonts.layout(text, st);
        let w = layout.width() as f64;
        let x0 = match align {
            Align::Left => x,
            Align::Center => x - w / 2.0,
            Align::Right => x - w,
        };
        // centre the cap height (about 0.7 em) on cy
        let baseline_y = cy + st.size as f64 * 0.36;
        let brush = color(st.color);
        for line in layout.lines() {
            let line_baseline = line.metrics().baseline as f64;
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(gr) = item else { continue };
                let run = gr.run();
                let dy = baseline_y - line_baseline;
                let coords: Vec<i16> = run.normalized_coords().iter().map(|c| c.to_bits()).collect();
                self.scene
                    .draw_glyphs(run.font())
                    .font_size(run.font_size())
                    .normalized_coords(&coords)
                    .brush(brush)
                    .draw(
                        Fill::NonZero,
                        gr.positioned_glyphs().map(|g| Glyph {
                            id: g.id,
                            x: (x0 + g.x as f64) as f32,
                            y: (dy + g.y as f64) as f32,
                        }),
                    );
            }
        }
        w
    }

    /// Like [`Painter::text`] but cut with an ellipsis to fit `max_w`.
    pub fn text_fit(&mut self, x: f64, cy: f64, text: &str, st: &TextStyle, max_w: f64) {
        if max_w <= 4.0 {
            return;
        }
        if self.fonts.measure(text, st) as f64 <= max_w {
            self.text(x, cy, text, st, Align::Left);
            return;
        }
        let chars: Vec<char> = text.chars().collect();
        for n in (0..chars.len()).rev() {
            let s: String = chars[..n].iter().collect::<String>() + "…";
            if self.fonts.measure(&s, st) as f64 <= max_w {
                self.text(x, cy, &s, st, Align::Left);
                return;
            }
        }
    }
}
