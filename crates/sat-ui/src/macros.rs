//! The right sidebar's inspector: the selected track's fader and pan, and
//! the 16 macros of its source and instrument grids (or of the fx grid on
//! screen, or a bus's fx grid), stacked to the sidebar's width. Learn mode
//! maps the next touched parameter to a free macro.

use sat_doc::*;
use sat_theme::{Rgba, Theme};
use vello::kurbo::{Point, Rect};

use crate::controls;
use crate::paint::{Align, Painter, TextStyle};
use crate::panel::TABS_H;

/// Knob diameter.
pub const KNOB: f64 = 32.0;
/// Height of the track card.
pub const TRACK_H: f64 = 128.0;
/// Height of a macro's cell: knob, name and value.
pub const CELL_H: f64 = 78.0;
/// Narrowest a knob cell gets in an eight-wide grid; narrower, four wide.
pub const CELL_W8: f64 = 62.0;

/// One grid's 16 macros.
#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    /// The grid.
    pub grid: GridId,
    /// The section's card.
    pub rect: Rect,
    /// The "open" button (shows the grid).
    pub open: Rect,
    /// Each macro's knob cell, by index (none while folded).
    pub knobs: Vec<(u8, Rect)>,
    /// Its [`Fold`] bit.
    pub fold: u8,
    /// Folded to its heading.
    pub folded: bool,
}

impl Section {
    /// A section's heading row, which folds it.
    pub fn heading(rect: Rect) -> Rect {
        Rect::new(rect.x0, rect.y0, rect.x1, rect.y0 + FOLDED_H)
    }

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
    /// A section's heading: fold or unfold it (a [`Fold`] bit).
    Fold(u8),
    /// Nothing.
    Nothing,
}

/// The inspector's sections that fold, as bits of a mask.
pub struct Fold;

impl Fold {
    /// The track card.
    pub const TRACK: u8 = 1;
    /// A grid kind's section.
    pub fn grid(kind: GridKind) -> u8 {
        match kind {
            GridKind::Source => 2,
            GridKind::Instrument => 4,
            GridKind::Fx => 8,
        }
    }
}

/// Height of a folded section: just its heading.
pub const FOLDED_H: f64 = 44.0;

/// The macros page's geometry.
#[derive(Debug, Clone, PartialEq)]
pub struct MacroLayout {
    /// The track shown.
    pub track: Option<TrackId>,
    /// The learn button.
    pub learn: Rect,
    /// The scrolling area below the header.
    pub body: Rect,
    /// How far it can scroll.
    pub max_scroll: f64,
    /// Grid sections, top to bottom.
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
        if !self.body.contains(pt) {
            return MacroHit::Nothing;
        }
        for s in &self.sections {
            if s.open.contains(pt) {
                return MacroHit::Open(s.grid);
            }
            if Section::heading(s.rect).contains(pt) {
                return MacroHit::Fold(s.fold);
            }
            if let Some((i, _)) = s.knobs.iter().find(|(_, r)| r.contains(pt)) {
                return MacroHit::Knob(s.grid, *i);
            }
        }
        if Section::heading(self.track_card).contains(pt) {
            return MacroHit::Fold(Fold::TRACK);
        }
        if let Some(t) = self.track.filter(|_| self.fader.width() > 0.0) {
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

/// Lays out the inspector in the sidebar `r`, scrolled down by `scroll`.
/// Sections whose bit is set in `folds` show only their heading.
pub fn layout(
    doc: &Document,
    track: Option<TrackId>,
    on_screen: Option<GridId>,
    r: Rect,
    scroll: f64,
    folds: u8,
) -> MacroLayout {
    let close = crate::dock::close_button(r);
    let cy = r.y0 + TABS_H / 2.0;
    let learn = Rect::new(close.x0 - 70.0, cy - 10.0, close.x0 - 6.0, cy + 10.0);
    let body = Rect::new(r.x0, r.y0 + TABS_H, r.x1, r.y1.max(r.y0 + TABS_H));
    let grids = grids_for(doc, track, on_screen);
    let gap = 10.0;
    let inner = Rect::new(body.x0 + 10.0, body.y0, body.x1 - 10.0, body.y1);
    let cols = if (inner.width() - 20.0) / 8.0 >= CELL_W8 { 8 } else { 4 };
    let rows = MACROS / cols;
    let full_h = 44.0 + rows as f64 * CELL_H + 6.0;
    let fold_of = |g: &GridId| doc.grids.get(g).map_or(0, |g| Fold::grid(g.kind));
    let sec_h = |g: &GridId| if folds & fold_of(g) != 0 { FOLDED_H } else { full_h };
    let track_h = if folds & Fold::TRACK != 0 { FOLDED_H } else { TRACK_H };
    let content = 10.0 + track_h + grids.iter().map(|g| gap + sec_h(g)).sum::<f64>() + 10.0;
    let max_scroll = (content - body.height()).max(0.0);
    let top = body.y0 + 10.0 - scroll.clamp(0.0, max_scroll);
    let track_card = Rect::new(inner.x0, top, inner.x1, top + track_h);
    let mut y = track_card.y1 + gap;
    let sections = grids
        .iter()
        .map(|g| {
            let y0 = y;
            let folded = folds & fold_of(g) != 0;
            y += sec_h(g) + gap;
            let rect = Rect::new(inner.x0, y0, inner.x1, y0 + sec_h(g));
            let open = Rect::new(rect.x1 - 62.0, rect.y0 + 10.0, rect.x1 - 10.0, rect.y0 + 34.0);
            let grid_r = Rect::new(rect.x0 + 10.0, rect.y0 + 44.0, rect.x1 - 10.0, rect.y1 - 6.0);
            let cw = grid_r.width() / cols as f64;
            let knobs = (0..MACROS as u8)
                .map(|i| {
                    let (col, row) = ((i as usize % cols) as f64, (i as usize / cols) as f64);
                    let x = grid_r.x0 + col * cw;
                    let y = grid_r.y0 + row * CELL_H;
                    (i, Rect::new(x, y, x + cw, y + CELL_H))
                })
                .filter(|_| !folded)
                .collect();
            Section { grid: *g, rect, open, knobs, fold: fold_of(g), folded }
        })
        .collect();
    let (fader, pan) = if folds & Fold::TRACK != 0 {
        (Rect::ZERO, Rect::ZERO)
    } else {
        (
            Rect::new(track_card.x0 + 28.0, track_card.y0 + 42.0, track_card.x0 + 34.0, track_card.y1 - 14.0),
            Rect::new(track_card.x1 - 84.0, track_card.y0 + 36.0, track_card.x1 - 14.0, track_card.y1 - 6.0),
        )
    };
    MacroLayout {
        track: track.filter(|t| doc.tracks.contains_key(t)),
        learn,
        body,
        max_scroll,
        sections,
        track_card,
        fader,
        pan,
    }
}

/// The colour of a grid kind's macros.
pub fn kind_color(kind: GridKind) -> Rgba {
    match kind {
        GridKind::Source => Rgba::hex(0xf0b14a),
        GridKind::Instrument => Rgba::hex(0x3b8bd2),
        GridKind::Fx => Rgba::hex(0x8a78f0),
    }
}

/// Draws the inspector in the sidebar `r` and returns its layout.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    p: &mut Painter<'_>,
    th: &Theme,
    doc: &Document,
    track: Option<TrackId>,
    on_screen: Option<GridId>,
    learn: bool,
    scroll: f64,
    folds: u8,
    r: Rect,
) -> MacroLayout {
    let c = &th.c;
    let l = layout(doc, track, on_screen, r, scroll, folds);
    let tr = l.track.and_then(|t| doc.tracks.get(&t));
    let name = tr.map(|t| t.name.as_str()).unwrap_or("no track");
    let swatch = tr.map(|t| th.track(t.color.0));
    let end = l.learn.x0 - 6.0;
    crate::dock::side_header(p, th, r, crate::area::EditorType::Inspector, swatch, name, "", end);
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
    p.clip(l.body, |p| draw_body(p, th, doc, &l, learn));
    if l.max_scroll > 0.0 {
        let track = Rect::new(r.x1 - 4.0, l.body.y0 + 2.0, r.x1 - 2.0, l.body.y1 - 2.0);
        let frac = l.body.height() / (l.body.height() + l.max_scroll);
        let at = (l.body.y0 + 10.0 - l.track_card.y0) / l.max_scroll;
        let h = (track.height() * frac).max(24.0);
        let y = track.y0 + (track.height() - h) * at.clamp(0.0, 1.0);
        p.rrect(Rect::new(track.x0, y, track.x1, y + h), 1.0, c.border_strong.alpha(0.6));
    }
    l
}

/// A section heading's fold marker: a filled triangle, down when open.
fn disclosure(p: &mut Painter<'_>, at: Point, folded: bool, col: Rgba) {
    let mut b = vello::kurbo::BezPath::new();
    if folded {
        b.move_to(Point::new(at.x - 2.5, at.y - 4.5));
        b.line_to(Point::new(at.x + 3.5, at.y));
        b.line_to(Point::new(at.x - 2.5, at.y + 4.5));
    } else {
        b.move_to(Point::new(at.x - 4.5, at.y - 2.5));
        b.line_to(Point::new(at.x + 4.5, at.y - 2.5));
        b.line_to(Point::new(at.x, at.y + 3.5));
    }
    b.close_path();
    p.path(&b, col);
}

fn draw_body(p: &mut Painter<'_>, th: &Theme, doc: &Document, l: &MacroLayout, learn: bool) {
    let c = &th.c;
    for s in &l.sections {
        let g = &doc.grids[&s.grid];
        let col = kind_color(g.kind);
        p.rrect(s.rect, 6.0, c.bg_base);
        p.rrect_stroke(s.rect.inset(-0.5), 6.0, 1.0, c.border_default);
        let hy = s.rect.y0 + 22.0;
        disclosure(p, Point::new(s.rect.x0 + 16.0, hy), s.folded, col);
        let title = match g.kind {
            GridKind::Source => "source",
            GridKind::Instrument => "instrument",
            GridKind::Fx => "fx",
        };
        let w = p.text(s.rect.x0 + 28.0, hy, title, &TextStyle::mono(12.0, c.text_max).medium(), Align::Left);
        let what = match g.kind {
            GridKind::Instrument => sat_modules::instrument::label(doc, s.grid).unwrap_or_else(|| "empty".into()),
            _ => doc.tracks.get(&g.track).map(|t| t.name.clone()).unwrap_or_default(),
        };
        let count = format!("{}/16", g.slots.len());
        let cw = p.text(s.open.x0 - 10.0, hy, &count, &TextStyle::mono(11.0, c.text_secondary), Align::Right);
        let room = s.open.x0 - 20.0 - cw - (s.rect.x0 + 36.0 + w);
        p.text_fit(s.rect.x0 + 36.0 + w, hy, &what, &TextStyle::mono(11.0, c.text_secondary), room.max(0.0));
        p.rrect(s.open, 5.0, c.bg_control);
        p.rrect_stroke(s.open.inset(-0.5), 5.0, 1.0, c.border_strong.alpha(0.6));
        p.text(s.open.center().x, s.open.center().y, "open", &TextStyle::mono(12.0, c.text_strong), Align::Center);
        if s.folded {
            continue;
        }
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
        let at = Rect::new(l.body.x0, l.track_card.y1, l.body.x1, l.body.y1).center();
        p.text(at.x, at.y, "select a track to see its macros", &TextStyle::mono(12.0, c.text_secondary), Align::Center);
    }
    // track: fader and pan
    let tc = l.track_card;
    p.rrect(tc, 6.0, c.bg_base);
    p.rrect_stroke(tc.inset(-0.5), 6.0, 1.0, c.border_default);
    let hy = tc.y0 + 22.0;
    let track_folded = l.fader.width() == 0.0;
    disclosure(p, Point::new(tc.x0 + 16.0, hy), track_folded, c.text_secondary);
    p.text(tc.x0 + 28.0, hy, "track", &TextStyle::mono(12.0, c.text_max).medium(), Align::Left);
    if learn {
        let tip = "touch a parameter to map it";
        p.text(tc.x1 - 12.0, hy, tip, &TextStyle::mono(10.5, c.accent_primary), Align::Right);
    }
    if let Some(tr) = l.track.and_then(|t| doc.tracks.get(&t)).filter(|_| !track_folded) {
        p.rrect(l.fader, 3.0, Rgba::hex(0x0a0a0a));
        p.rrect_stroke(l.fader.inset(-0.5), 3.0, 1.0, Rgba::hex(0x262626));
        let pos = controls::fader_pos(tr.mixer.gain_db);
        let fy = l.fader_y(pos);
        let fill = Rect::new(l.fader.x0 + 1.0, fy, l.fader.x1 - 1.0, l.fader.y1 - 1.0);
        p.rrect(fill, 2.0, Rgba::hex(0x4caf50).alpha(0.7));
        let cap = Rect::new(l.fader.center().x - 11.0, fy - 5.0, l.fader.center().x + 11.0, fy + 5.0);
        p.rrect(cap, 2.0, Rgba::hex(0xc8c8c8));
        let lx = l.fader.x1 + 22.0;
        let my = l.fader.center().y;
        p.text(lx, my - 8.0, "level", &TextStyle::mono(11.0, c.text_secondary), Align::Left);
        let db = controls::fmt_db(tr.mixer.gain_db);
        p.text(lx, my + 8.0, &db, &TextStyle::mono(12.0, c.text_strong), Align::Left);
        let kc = Section::knob_center(l.pan);
        controls::knob(p, kc, KNOB, controls::pan_pos(tr.mixer.pan), Rgba::hex(0xd8d8d8));
        p.text(kc.x, kc.y + KNOB / 2.0 + 12.0, "pan", &TextStyle::mono(11.0, c.text_secondary), Align::Center);
        let pan = sat_modules::spec::p("pan", "pan", -1.0, 1.0, 0.0, sat_modules::Unit::Pan).show(tr.mixer.pan);
        p.text(kc.x, kc.y + KNOB / 2.0 + 27.0, &pan, &TextStyle::mono(11.0, c.text_strong), Align::Center);
    }
}
