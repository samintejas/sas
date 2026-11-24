//! The bottom panel's macros page: the 16 macros of the selected track's
//! source and instrument grids (or of the fx grid on screen, or a bus's fx
//! grid), and the track's fader and pan. Learn mode maps the next touched
//! parameter to a free macro.

use sat_doc::*;
use sat_theme::{Rgba, Theme};
use vello::kurbo::{Point, Rect};

use crate::controls;
use crate::paint::{Align, Painter, TextStyle};
use crate::panel::{self, PanelView, TABS_H};

/// Knob diameter.
pub const KNOB: f64 = 32.0;
/// Width of the track section.
pub const TRACK_W: f64 = 140.0;

/// One grid's 16 macros.
#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    /// The grid.
    pub grid: GridId,
    /// The section's card.
    pub rect: Rect,
    /// The "open" button (shows the grid).
    pub open: Rect,
    /// Each macro's knob cell, by index.
    pub knobs: Vec<(u8, Rect)>,
}

impl Section {
    /// The knob's centre in a cell.
    pub fn knob_center(cell: Rect) -> Point {
        Point::new(cell.center().x, cell.y0 + 6.0 + KNOB / 2.0)
    }
}

/// What is under a point on the macros page.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MacroHit {
    /// A macro's knob.
    Knob(GridId, u8),
    /// A section's open button.
    Open(GridId),
    /// The learn button.
    Learn,
    /// The track's fader.
    Fader(TrackId),
    /// The track's pan knob.
    Pan(TrackId),
    /// Nothing.
    Nothing,
}

/// The macros page's geometry.
#[derive(Debug, Clone, PartialEq)]
pub struct MacroLayout {
    /// The track shown.
    pub track: Option<TrackId>,
    /// The learn button.
    pub learn: Rect,
    /// Grid sections, left to right.
    pub sections: Vec<Section>,
    /// The track section.
    pub track_card: Rect,
    /// The fader slot.
    pub fader: Rect,
    /// The pan knob's cell.
    pub pan: Rect,
}

impl MacroLayout {
    /// What is under `pt`.
    pub fn hit(&self, pt: Point) -> MacroHit {
        if self.learn.contains(pt) {
            return MacroHit::Learn;
        }
        for s in &self.sections {
            if s.open.contains(pt) {
                return MacroHit::Open(s.grid);
            }
            if let Some((i, _)) = s.knobs.iter().find(|(_, r)| r.contains(pt)) {
                return MacroHit::Knob(s.grid, *i);
            }
        }
        if let Some(t) = self.track {
            if self.fader.inset(8.0).contains(pt) {
                return MacroHit::Fader(t);
            }
            if self.pan.contains(pt) {
                return MacroHit::Pan(t);
            }
        }
        MacroHit::Nothing
    }

    /// The fader cap's centre y for a position (0–1).
    pub fn fader_y(&self, pos: f64) -> f64 {
        self.fader.y1 - pos.clamp(0.0, 1.0) * self.fader.height()
    }
}

/// The grids whose macros the page shows: the fx grid on screen if there is
/// one, else the track's source and instrument grids, else its fx grid.
pub fn grids_for(doc: &Document, track: Option<TrackId>, on_screen: Option<GridId>) -> Vec<GridId> {
    if let Some(g) = on_screen.filter(|g| doc.grids.get(g).is_some_and(|g| g.kind == GridKind::Fx)) {
        return vec![g];
    }
    let Some(t) = track.and_then(|t| doc.tracks.get(&t)) else { return Vec::new() };
    let v: Vec<GridId> = t.source_grid.into_iter().chain(t.instrument_grid).collect();
    if v.is_empty() { t.fx_grids.front().copied().into_iter().collect() } else { v }
}

/// Lays out the page in panel `r`.
pub fn layout(doc: &Document, track: Option<TrackId>, on_screen: Option<GridId>, r: Rect) -> MacroLayout {
    let cy = r.y0 + TABS_H / 2.0;
    let learn = Rect::new(r.x1 - 84.0, cy - 12.0, r.x1 - 12.0, cy + 12.0);
    let body = Rect::new(r.x0 + 12.0, r.y0 + TABS_H + 10.0, r.x1 - 12.0, r.y1 - 10.0);
    let track_card = Rect::new(body.x1 - TRACK_W, body.y0, body.x1, body.y1);
    let grids = grids_for(doc, track, on_screen);
    let n = grids.len().max(1) as f64;
    let gap = 10.0;
    let w = ((track_card.x0 - gap - body.x0) - gap * (n - 1.0)) / n;
    let sections = grids
        .iter()
        .enumerate()
        .map(|(k, g)| {
            let x0 = body.x0 + k as f64 * (w + gap);
            let rect = Rect::new(x0, body.y0, x0 + w, body.y1);
            let open = Rect::new(rect.x1 - 78.0, rect.y0 + 10.0, rect.x1 - 12.0, rect.y0 + 34.0);
            let grid_r = Rect::new(rect.x0 + 10.0, rect.y0 + 44.0, rect.x1 - 10.0, rect.y1 - 6.0);
            let (cw, ch) = (grid_r.width() / 8.0, grid_r.height() / 2.0);
            let knobs = (0..MACROS as u8)
                .map(|i| {
                    let (col, row) = ((i % 8) as f64, (i / 8) as f64);
                    let x = grid_r.x0 + col * cw;
                    let y = grid_r.y0 + row * ch;
                    (i, Rect::new(x, y, x + cw, y + ch))
                })
                .collect();
            Section { grid: *g, rect, open, knobs }
        })
        .collect();
    let fader = Rect::new(track_card.x0 + 34.0, track_card.y0 + 46.0, track_card.x0 + 40.0, track_card.y1 - 40.0);
    let pan = Rect::new(track_card.x1 - 64.0, track_card.y1 - 90.0, track_card.x1 - 10.0, track_card.y1 - 10.0);
    MacroLayout { track: track.filter(|t| doc.tracks.contains_key(t)), learn, sections, track_card, fader, pan }
}

/// The colour of a grid kind's macros.
pub fn kind_color(kind: GridKind) -> Rgba {
    match kind {
        GridKind::Source => Rgba::hex(0xf0b14a),
        GridKind::Instrument => Rgba::hex(0x3b8bd2),
        GridKind::Fx => Rgba::hex(0x8a78f0),
    }
}

/// Draws the page in panel `r` and returns its layout.
pub fn draw(
    p: &mut Painter<'_>,
    th: &Theme,
    doc: &Document,
    track: Option<TrackId>,
    on_screen: Option<GridId>,
    learn: bool,
    r: Rect,
) -> MacroLayout {
    let c = &th.c;
    let l = layout(doc, track, on_screen, r);
    let (bar, x) = panel::draw_bar(p, th, r, PanelView::Macros);
    let cy = bar.center().y;
    let mut x = x;
    if let Some(tr) = l.track.and_then(|t| doc.tracks.get(&t)) {
        p.rrect(Rect::new(x, cy - 5.0, x + 10.0, cy + 5.0), 2.0, th.track(tr.color.0));
        x += 18.0;
    }
    x += p.text(x, cy, "macros", &TextStyle::mono(12.0, c.text_max).medium(), Align::Left) + 10.0;
    let sub = match l.track.and_then(|t| doc.tracks.get(&t)) {
        Some(tr) if learn => format!("{} · learn: touch a parameter to map it · click a macro to clear it", tr.name),
        Some(tr) => format!("{} · 16 per grid", tr.name),
        None => "select a track".into(),
    };
    p.text(x, cy, &sub, &TextStyle::mono(11.0, if learn { c.accent_primary } else { c.text_secondary }), Align::Left);
    // learn button
    let lb = l.learn;
    if learn {
        p.rrect(lb, 5.0, c.accent_primary.alpha(0.18));
        p.rrect_stroke(lb.inset(-0.5), 5.0, 1.0, c.accent_primary);
    } else {
        p.rrect(lb, 5.0, c.bg_control);
        p.rrect_stroke(lb.inset(-0.5), 5.0, 1.0, c.border_strong.alpha(0.6));
    }
    let lc = if learn { c.accent_primary } else { c.text_strong };
    p.text(lb.center().x, lb.center().y, "learn", &TextStyle::mono(12.0, lc), Align::Center);

    for s in &l.sections {
        let g = &doc.grids[&s.grid];
        let col = kind_color(g.kind);
        p.rrect(s.rect, 6.0, c.bg_base);
        p.rrect_stroke(s.rect.inset(-0.5), 6.0, 1.0, c.border_default);
        let hy = s.rect.y0 + 22.0;
        p.circle(Point::new(s.rect.x0 + 16.0, hy), 4.0, col);
        let title = match g.kind {
            GridKind::Source => "source grid",
            GridKind::Instrument => "instrument grid",
            GridKind::Fx => "fx grid",
        };
        let w = p.text(s.rect.x0 + 28.0, hy, title, &TextStyle::mono(12.0, c.text_max).medium(), Align::Left);
        let what = match g.kind {
            GridKind::Instrument => sat_modules::instrument::label(doc, s.grid).unwrap_or_else(|| "empty".into()),
            _ => doc.tracks.get(&g.track).map(|t| t.name.clone()).unwrap_or_default(),
        };
        p.text(s.rect.x0 + 36.0 + w, hy, &what, &TextStyle::mono(11.0, c.text_secondary), Align::Left);
        let count = format!("{}/16", g.slots.len());
        p.text(s.open.x0 - 10.0, hy, &count, &TextStyle::mono(11.0, c.text_secondary), Align::Right);
        p.rrect(s.open, 5.0, c.bg_control);
        p.rrect_stroke(s.open.inset(-0.5), 5.0, 1.0, c.border_strong.alpha(0.6));
        p.text(s.open.center().x, s.open.center().y, "open", &TextStyle::mono(12.0, c.text_strong), Align::Center);
        for (i, cell) in &s.knobs {
            let kc = Section::knob_center(*cell);
            let ly = kc.y + KNOB / 2.0 + 12.0;
            match sat_modules::macros::readout(doc, s.grid, *i) {
                Some((name, value)) => {
                    controls::knob(p, kc, KNOB, g.macros[*i as usize] as f64, col);
                    if learn {
                        p.circle_stroke(kc, KNOB / 2.0 + 3.0, 1.0, c.accent_primary.alpha(0.5));
                    }
                    p.text(kc.x, ly, &name, &TextStyle::mono(11.0, c.text_secondary), Align::Center);
                    p.text(kc.x, ly + 15.0, &value, &TextStyle::mono(11.0, c.text_strong), Align::Center);
                }
                None => {
                    // dashed ring with a plus
                    for k in 0..16 {
                        let a0 = k as f64 / 16.0 * std::f64::consts::TAU;
                        let a1 = a0 + std::f64::consts::TAU / 32.0;
                        let rr = KNOB * 0.42;
                        let pt = |a: f64| Point::new(kc.x + rr * a.cos(), kc.y + rr * a.sin());
                        p.line(pt(a0), pt(a1), 1.0, c.text_secondary.alpha(0.7));
                    }
                    p.line(Point::new(kc.x - 5.0, kc.y), Point::new(kc.x + 5.0, kc.y), 1.3, c.text_secondary);
                    p.line(Point::new(kc.x, kc.y - 5.0), Point::new(kc.x, kc.y + 5.0), 1.3, c.text_secondary);
                    let name = format!("macro {}", i + 1);
                    p.text(kc.x, ly, &name, &TextStyle::mono(11.0, c.text_secondary), Align::Center);
                    let what = if learn { "learn" } else { "assign" };
                    p.text(kc.x, ly + 15.0, what, &TextStyle::mono(11.0, c.text_strong), Align::Center);
                }
            }
        }
    }
    if l.sections.is_empty() {
        let at = Rect::new(r.x0, bar.y1, l.track_card.x0, r.y1).center();
        p.text(at.x, at.y, "select a track to see its macros", &TextStyle::mono(12.0, c.text_secondary), Align::Center);
    }
    // track: fader and pan
    let tc = l.track_card;
    p.rrect(tc, 6.0, c.bg_base);
    p.rrect_stroke(tc.inset(-0.5), 6.0, 1.0, c.border_default);
    let hy = tc.y0 + 22.0;
    p.circle(Point::new(tc.x0 + 16.0, hy), 4.0, c.text_secondary);
    p.text(tc.x0 + 28.0, hy, "track", &TextStyle::mono(12.0, c.text_max).medium(), Align::Left);
    if let Some(tr) = l.track.and_then(|t| doc.tracks.get(&t)) {
        p.rrect(l.fader, 3.0, Rgba::hex(0x0a0a0a));
        p.rrect_stroke(l.fader.inset(-0.5), 3.0, 1.0, Rgba::hex(0x262626));
        let pos = controls::fader_pos(tr.mixer.gain_db);
        let fy = l.fader_y(pos);
        let fill = Rect::new(l.fader.x0 + 1.0, fy, l.fader.x1 - 1.0, l.fader.y1 - 1.0);
        p.rrect(fill, 2.0, Rgba::hex(0x4caf50).alpha(0.7));
        let cap = Rect::new(l.fader.center().x - 11.0, fy - 5.0, l.fader.center().x + 11.0, fy + 5.0);
        p.rrect(cap, 2.0, Rgba::hex(0xc8c8c8));
        let fx = l.fader.center().x;
        p.text(fx, tc.y1 - 26.0, "level", &TextStyle::mono(11.0, c.text_secondary), Align::Center);
        let db = controls::fmt_db(tr.mixer.gain_db);
        p.text(fx, tc.y1 - 12.0, &db, &TextStyle::mono(11.0, c.text_strong), Align::Center);
        let kc = Section::knob_center(l.pan);
        controls::knob(p, kc, KNOB, controls::pan_pos(tr.mixer.pan), Rgba::hex(0xd8d8d8));
        p.text(kc.x, kc.y + KNOB / 2.0 + 12.0, "pan", &TextStyle::mono(11.0, c.text_secondary), Align::Center);
        let pan = sat_modules::spec::p("pan", "pan", -1.0, 1.0, 0.0, sat_modules::Unit::Pan).show(tr.mixer.pan);
        p.text(kc.x, kc.y + KNOB / 2.0 + 27.0, &pan, &TextStyle::mono(11.0, c.text_strong), Align::Center);
    }
    l
}
