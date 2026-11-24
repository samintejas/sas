//! The arrange view: track headers on the left, the timeline on the right.

use std::collections::BTreeSet;

use sat_doc::sat_time::{PPQ, Ticks};
use sat_doc::*;
use sat_theme::{Rgba, Theme};
use vello::kurbo::{Point, Rect};

use crate::icons::{self, Icon};
use crate::paint::{Align, Painter, TextStyle};

/// Width of the track header column.
pub const HEADER_W: f64 = 220.0;
/// Height of the ruler.
pub const RULER_H: f64 = 44.0;
/// Height of the arrange toolbar row.
pub const TOOLBAR_H: f64 = 36.0;

/// Scroll, zoom and selection of the arrange view.
#[derive(Debug, Clone, PartialEq)]
pub struct ArrangeState {
    /// Tick at the left edge of the timeline.
    pub origin: Ticks,
    /// Zoom: pixels per bar (4/4).
    pub px_per_bar: f64,
    /// Vertical scroll in px.
    pub scroll_y: f64,
    /// Selected clip, shown in the bottom panel.
    pub selected_clip: Option<ClipId>,
    /// Selected track.
    pub selected_track: Option<TrackId>,
    /// Collapsed groups.
    pub collapsed: BTreeSet<TrackId>,
}

impl Default for ArrangeState {
    fn default() -> Self {
        ArrangeState {
            origin: Ticks::ZERO,
            px_per_bar: 76.0,
            scroll_y: 0.0,
            selected_clip: None,
            selected_track: None,
            collapsed: BTreeSet::new(),
        }
    }
}

/// Transport state the views show.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TransportView {
    /// Playhead.
    pub playhead: Ticks,
    /// Playing.
    pub playing: bool,
    /// Recording.
    pub recording: bool,
}

/// What a row in the arrange view shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    /// A track with clips.
    Track,
    /// A group with a summary of its children.
    Group,
    /// A return with its automation.
    Return,
    /// Master.
    Master,
    /// An automation lane under a track.
    Lane(LaneId),
}

/// One laid-out row.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Row {
    /// The track it belongs to.
    pub track: TrackId,
    /// What it shows.
    pub kind: RowKind,
    /// Top edge (screen px).
    pub y: f64,
    /// Height.
    pub h: f64,
    /// Nesting depth in the group tree.
    pub depth: usize,
}

/// One clip placement on screen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClipBox {
    /// The placement.
    pub placement: PlacementId,
    /// Its clip.
    pub clip: ClipId,
    /// Where it is drawn.
    pub rect: Rect,
}

/// What is under a point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Hit {
    /// A clip placement.
    Clip {
        /// The placement.
        placement: PlacementId,
        /// Its clip.
        clip: ClipId,
    },
    /// A track header.
    Header(TrackId),
    /// Empty timeline on a row, at a tick.
    Lane(TrackId, Ticks),
    /// The ruler, at a tick.
    Ruler(Ticks),
    /// Nothing.
    Nothing,
}

/// The arrange view laid out in a rectangle. Pure geometry: no drawing, so
/// hit testing and tests work without a GPU.
#[derive(Debug, Clone)]
pub struct ArrangeLayout {
    /// The whole tile.
    pub bounds: Rect,
    /// The track header column (below the ruler).
    pub headers: Rect,
    /// The ruler.
    pub ruler: Rect,
    /// The timeline (below the ruler).
    pub timeline: Rect,
    /// Rows, top to bottom.
    pub rows: Vec<Row>,
    /// Visible clip placements.
    pub clips: Vec<ClipBox>,
    origin: Ticks,
    px_per_tick: f64,
}

fn row_height(kind: RowKind) -> f64 {
    match kind {
        RowKind::Track => 52.0,
        RowKind::Group => 42.0,
        RowKind::Return | RowKind::Master => 30.0,
        RowKind::Lane(_) => 30.0,
    }
}

impl ArrangeLayout {
    /// Lays out `doc` in `bounds`.
    pub fn new(doc: &Document, st: &ArrangeState, bounds: Rect) -> ArrangeLayout {
        let top = bounds.y0 + TOOLBAR_H;
        let ruler = Rect::new(bounds.x0 + HEADER_W, top, bounds.x1, top + RULER_H);
        let headers = Rect::new(bounds.x0, top + RULER_H, bounds.x0 + HEADER_W, bounds.y1);
        let timeline = Rect::new(bounds.x0 + HEADER_W, top + RULER_H, bounds.x1, bounds.y1);
        let px_per_tick = st.px_per_bar / (PPQ * 4) as f64;
        let mut rows = Vec::new();
        let mut y = timeline.y0 - st.scroll_y;
        let depth = |t: TrackId| {
            let mut d = 0;
            let mut p = doc.tracks[&t].parent;
            while let Some(q) = p {
                d += 1;
                p = doc.tracks[&q].parent;
            }
            d
        };
        let hidden = |t: TrackId| {
            let mut p = doc.tracks[&t].parent;
            while let Some(q) = p {
                if st.collapsed.contains(&q) {
                    return true;
                }
                p = doc.tracks[&q].parent;
            }
            false
        };
        let order = doc.tracks_in_order();
        let main = order.iter().filter(|t| !matches!(doc.tracks[t].kind, TrackKind::Return | TrackKind::Master));
        let tail = order.iter().filter(|t| doc.tracks[t].kind == TrackKind::Return).chain([&doc.master]);
        for &t in main.chain(tail) {
            if hidden(t) {
                continue;
            }
            let kind = match doc.tracks[&t].kind {
                TrackKind::Track => RowKind::Track,
                TrackKind::Group => RowKind::Group,
                TrackKind::Return => RowKind::Return,
                TrackKind::Master => RowKind::Master,
            };
            let h = row_height(kind);
            rows.push(Row { track: t, kind, y, h, depth: depth(t) });
            y += h;
            if kind == RowKind::Track {
                for l in doc.lanes.values().filter(|l| l.target == ParamTarget::Gain(t)) {
                    let k = RowKind::Lane(l.id);
                    rows.push(Row { track: t, kind: k, y, h: row_height(k), depth: depth(t) });
                    y += row_height(k);
                }
            }
        }
        let mut layout =
            ArrangeLayout { bounds, headers, ruler, timeline, rows, clips: Vec::new(), origin: st.origin, px_per_tick };
        let mut clips = Vec::new();
        for p in doc.placements.values() {
            let Some(c) = doc.clips.get(&p.clip) else { continue };
            let Some(row) = layout.rows.iter().find(|r| r.track == c.track && r.kind == RowKind::Track) else {
                continue;
            };
            let x0 = layout.x_of(p.start);
            let x1 = layout.x_of(p.start + p.length);
            if x1 <= timeline.x0 || x0 >= timeline.x1 {
                continue;
            }
            clips.push(ClipBox {
                placement: p.id,
                clip: c.id,
                rect: Rect::new(x0 + 1.0, row.y + 4.0, x1 - 1.0, row.y + row.h - 4.0),
            });
        }
        clips.sort_by(|a, b| a.rect.x0.total_cmp(&b.rect.x0));
        layout.clips = clips;
        layout
    }

    /// Screen x of a tick.
    pub fn x_of(&self, t: Ticks) -> f64 {
        self.timeline.x0 + (t.0 - self.origin.0) as f64 * self.px_per_tick
    }

    /// Tick at a screen x.
    pub fn tick_at(&self, x: f64) -> Ticks {
        Ticks(self.origin.0 + ((x - self.timeline.x0) / self.px_per_tick).round() as i64)
    }

    /// What is under `pt`.
    pub fn hit(&self, pt: Point) -> Hit {
        if self.ruler.contains(pt) {
            return Hit::Ruler(self.tick_at(pt.x));
        }
        if let Some(c) = self.clips.iter().rev().find(|c| c.rect.contains(pt)) {
            return Hit::Clip { placement: c.placement, clip: c.clip };
        }
        let Some(row) = self.rows.iter().find(|r| pt.y >= r.y && pt.y < r.y + r.h) else { return Hit::Nothing };
        if self.headers.contains(pt) {
            return Hit::Header(row.track);
        }
        if self.timeline.contains(pt) {
            return Hit::Lane(row.track, self.tick_at(pt.x));
        }
        Hit::Nothing
    }
}

fn track_label(doc: &Document, t: &Track) -> String {
    match t.kind {
        TrackKind::Track => {
            let ins = t.instrument_grid.and_then(|g| doc.modules_on(g).next()).map(|m| m.name.clone());
            ins.unwrap_or_else(|| "empty".into())
        }
        _ => String::new(),
    }
}

/// Draws a small square toggle button ("m", "s") and returns its rect.
fn small_button(p: &mut Painter<'_>, th: &Theme, x: f64, cy: f64, label: &str, on: Option<Rgba>) -> Rect {
    let r = Rect::new(x, cy - 11.0, x + 22.0, cy + 11.0);
    match on {
        Some(c) => {
            p.rrect(r, 3.0, c.alpha(0.16));
            p.rrect_stroke(r.inset(-0.5), 3.0, 1.0, c.alpha(0.6));
            p.text(r.center().x, cy, label, &TextStyle::mono(11.0, c), Align::Center);
        }
        None => {
            p.rrect(r, 3.0, th.c.bg_control);
            p.rrect_stroke(r.inset(-0.5), 3.0, 1.0, th.c.border_strong.alpha(0.55));
            p.text(r.center().x, cy, label, &TextStyle::mono(11.0, th.c.text_primary), Align::Center);
        }
    }
    r
}

fn fader(p: &mut Painter<'_>, th: &Theme, x0: f64, x1: f64, y: f64, gain_db: f32) {
    let track = Rect::new(x0, y - 1.5, x1, y + 1.5);
    p.rrect(track, 1.5, Rgba::hex(0x262626));
    // 0 dB sits at 75% of the travel, -60 dB at the left
    let k = ((gain_db as f64 + 60.0) / 60.0 * 0.75).clamp(0.0, 1.0);
    let tx = x0 + (x1 - x0) * k;
    p.rrect(Rect::new(x0, y - 1.5, tx, y + 1.5), 1.5, th.c.text_secondary.alpha(0.6));
    p.circle(Point::new(tx, y), 5.0, Rgba::hex(0xd8d8d8));
    p.circle_stroke(Point::new(tx, y), 5.0, 1.0, Rgba::hex(0x050505));
}

fn meter(p: &mut Painter<'_>, th: &Theme, x: f64, y0: f64, y1: f64, level: f64) {
    let h = y1 - y0;
    p.rect(Rect::new(x, y0, x + 3.0, y1), Rgba::hex(0x1a1a1a));
    p.rect(Rect::new(x + 4.0, y0, x + 7.0, y1), Rgba::hex(0x1a1a1a));
    let fill = th.c.accent_success;
    p.rect(Rect::new(x, y1 - h * level, x + 3.0, y1), fill);
    p.rect(Rect::new(x + 4.0, y1 - h * level * 0.92, x + 7.0, y1), fill);
}

/// Draws the arrange view.
pub fn draw(p: &mut Painter<'_>, th: &Theme, doc: &Document, st: &ArrangeState, tr: &TransportView, l: &ArrangeLayout) {
    let c = &th.c;
    let b = l.bounds;
    p.rect(b, c.bg_surface);

    // toolbar: snap and zoom
    let ty = b.y0 + TOOLBAR_H / 2.0;
    p.hline(b.x0, b.x1, b.y0 + TOOLBAR_H - 1.0, c.border_subtle);
    let mut x = b.x1 - 12.0;
    for icon in [Icon::Plus, Icon::Search] {
        let r = Rect::new(x - 26.0, ty - 13.0, x, ty + 13.0);
        icons::draw(p, icon, r.inset(-7.0), c.text_primary);
        x -= 32.0;
    }
    let snap = Rect::new(x - 74.0, ty - 12.0, x - 8.0, ty + 12.0);
    p.rrect(snap, 5.0, c.bg_control);
    p.rrect_stroke(snap.inset(-0.5), 5.0, 1.0, c.border_strong.alpha(0.6));
    p.text(snap.x0 + 8.0, ty, "1 bar", &TextStyle::mono(12.0, c.text_strong), Align::Left);
    icons::draw(p, Icon::ChevronDown, Rect::new(snap.x1 - 20.0, ty - 6.0, snap.x1 - 8.0, ty + 6.0), c.text_secondary);
    p.text(snap.x0 - 10.0, ty, "snap", &TextStyle::mono(12.0, c.text_secondary), Align::Right);

    // header column above the ruler
    let hr = Rect::new(b.x0, l.ruler.y0, b.x0 + HEADER_W, l.ruler.y1);
    p.rect(hr, c.bg_surface);
    icons::draw(p, Icon::Plus, Rect::new(hr.x1 - 26.0, hr.y1 - 26.0, hr.x1 - 12.0, hr.y1 - 12.0), c.text_secondary);

    // timeline background, grid
    p.rect(l.timeline, c.bg_base);
    let bar_ticks = PPQ * 4;
    let first_bar = (l.tick_at(l.timeline.x0).0.div_euclid(bar_ticks)).max(0);
    let last_bar = l.tick_at(l.timeline.x1).0.div_euclid(bar_ticks) + 1;
    p.clip(Rect::new(l.ruler.x0, l.ruler.y0, l.timeline.x1, l.timeline.y1), |p| {
        let lr = doc.loop_range;
        // ruler
        p.rect(l.ruler, c.bg_surface);
        p.hline(l.ruler.x0, l.ruler.x1, l.ruler.y1 - 1.0, c.border_default);
        let strip = Rect::new(l.ruler.x0, l.ruler.y0 + 3.0, l.ruler.x1, l.ruler.y0 + 15.0);
        if lr.enabled {
            let (x0, x1) = (l.x_of(lr.start), l.x_of(lr.end));
            let band = Rect::new(x0, strip.y0, x1, strip.y1);
            p.rect(band, Rgba::hex(0x3a3a3a));
            p.rect(Rect::new(x0, strip.y0, x0 + 2.0, strip.y1 + 3.0), c.text_strong);
            p.rect(Rect::new(x1 - 2.0, strip.y0, x1, strip.y1 + 3.0), c.text_strong);
            let bars = (lr.end.0 - lr.start.0) / bar_ticks;
            p.text(
                x0 + 8.0,
                band.center().y,
                &format!("loop {bars} bars"),
                &TextStyle::mono(10.0, c.text_max),
                Align::Left,
            );
        }
        for bar in first_bar..=last_bar {
            let x = l.x_of(Ticks(bar * bar_ticks));
            p.vline(x, l.ruler.y1 - 22.0, l.ruler.y1, c.border_default);
            p.text(
                x + 5.0,
                l.ruler.y1 - 12.0,
                &(bar + 1).to_string(),
                &TextStyle::mono(11.0, c.text_primary),
                Align::Left,
            );
            for beat in 1..4 {
                let bx = l.x_of(Ticks(bar * bar_ticks + beat * PPQ));
                p.vline(bx, l.ruler.y1 - 5.0, l.ruler.y1, c.border_subtle);
                p.vline(bx, l.timeline.y0, l.timeline.y1, Rgba::new(255, 255, 255, 6));
            }
            p.vline(x, l.timeline.y0, l.timeline.y1, c.border_subtle);
        }
        // dim outside the loop
        if lr.enabled {
            let (x0, x1) = (l.x_of(lr.start), l.x_of(lr.end));
            for r in [
                Rect::new(l.timeline.x0, l.timeline.y0, x0, l.timeline.y1),
                Rect::new(x1, l.timeline.y0, l.timeline.x1, l.timeline.y1),
            ] {
                p.rect(r, Rgba::new(0, 0, 0, 40));
            }
            for x in [x0, x1] {
                for k in 0..((l.timeline.height() / 6.0) as i64) {
                    let y = l.timeline.y0 + k as f64 * 6.0;
                    p.rect(Rect::new(x.floor(), y, x.floor() + 1.0, y + 3.0), c.text_secondary.alpha(0.5));
                }
            }
        }
    });

    // rows
    let rows_clip = Rect::new(b.x0, l.timeline.y0, b.x1, b.y1);
    p.clip(rows_clip, |p| {
        for row in &l.rows {
            if row.y + row.h < l.timeline.y0 || row.y > b.y1 {
                continue;
            }
            draw_row(p, th, doc, st, l, row);
        }
        for cb in &l.clips {
            draw_clip(p, th, doc, st, l, cb);
        }
        // playhead
        let x = l.x_of(tr.playhead);
        if x >= l.timeline.x0 && x <= l.timeline.x1 {
            p.vline(x, l.timeline.y0, l.timeline.y1, c.text_max);
        }
    });
    let x = l.x_of(tr.playhead);
    if x >= l.timeline.x0 && x <= l.timeline.x1 {
        p.vline(x, l.ruler.y0 + 16.0, l.ruler.y1, c.text_max);
        let mut tri = vello::kurbo::BezPath::new();
        tri.move_to((x - 5.0, l.ruler.y1 - 12.0));
        tri.line_to((x + 6.0, l.ruler.y1 - 12.0));
        tri.line_to((x + 0.5, l.ruler.y1 - 4.0));
        tri.close_path();
        p.path(&tri, c.text_max);
    }
    p.vline(l.headers.x1, l.ruler.y0, b.y1, c.border_default);
}

fn draw_row(p: &mut Painter<'_>, th: &Theme, doc: &Document, st: &ArrangeState, l: &ArrangeLayout, row: &Row) {
    let c = &th.c;
    let t = &doc.tracks[&row.track];
    let tc = th.track(t.color.0);
    let hr = Rect::new(l.headers.x0, row.y, l.headers.x1, row.y + row.h);
    let lane = Rect::new(l.timeline.x0, row.y, l.timeline.x1, row.y + row.h);
    let selected = st.selected_track == Some(row.track) && row.kind == RowKind::Track;
    let indent = 14.0 + 10.0 * row.depth.saturating_sub(0) as f64;
    match row.kind {
        RowKind::Track | RowKind::Group => {
            p.rect(hr, if selected { c.bg_raised } else { c.bg_surface });
            p.rect(Rect::new(hr.x0, hr.y0, hr.x0 + 4.0, hr.y1), tc);
            if row.kind == RowKind::Group {
                p.rect(lane, Rgba::new(0, 0, 0, 0));
                let cy = hr.y0 + 17.0;
                let open = !st.collapsed.contains(&row.track);
                icons::draw(
                    p,
                    if open { Icon::ChevronDown } else { Icon::ChevronRight },
                    Rect::new(hr.x0 + indent - 2.0, cy - 5.0, hr.x0 + indent + 8.0, cy + 5.0),
                    c.text_secondary,
                );
                let w = p.text(
                    hr.x0 + indent + 16.0,
                    cy,
                    &t.name,
                    &TextStyle::sans(13.0, c.text_strong).medium(),
                    Align::Left,
                );
                let badge = Rect::new(hr.x0 + indent + 24.0 + w, cy - 8.0, hr.x0 + indent + 40.0 + w, cy + 8.0);
                p.rrect(badge, 8.0, c.bg_fill);
                p.text(
                    badge.center().x,
                    cy,
                    &t.children.len().to_string(),
                    &TextStyle::mono(10.0, c.text_secondary),
                    Align::Center,
                );
                small_button(p, th, hr.x1 - 98.0, cy, "m", t.mixer.mute.then_some(c.accent_warning));
                small_button(p, th, hr.x1 - 72.0, cy, "s", t.mixer.solo.then_some(c.accent_info));
                fader(p, th, hr.x0 + indent, hr.x1 - 50.0, hr.y0 + 33.0, t.mixer.gain_db);
                meter(p, th, hr.x1 - 16.0, hr.y0 + 12.0, hr.y1 - 8.0, 0.55);
                // children summary: one bar per child track's placements
                let kids: Vec<&Track> = t.children.iter().map(|k| &doc.tracks[k]).collect();
                let n = kids.len().max(1) as f64;
                let band = (row.h - 16.0) / n;
                for (i, k) in kids.iter().enumerate() {
                    let y = row.y + 8.0 + band * i as f64 + (band - 4.0) / 2.0;
                    for pl in doc.placements.values() {
                        if doc.clips.get(&pl.clip).is_some_and(|cl| cl.track == k.id) {
                            let (x0, x1) = (l.x_of(pl.start), l.x_of(pl.start + pl.length));
                            p.rrect(Rect::new(x0 + 1.0, y, x1 - 1.0, y + 4.0), 2.0, th.track(k.color.0).alpha(0.85));
                        }
                    }
                }
            } else {
                let cy = hr.y0 + 17.0;
                p.text_fit(hr.x0 + indent, cy, &t.name, &TextStyle::sans(13.0, c.text_strong).medium(), 80.0);
                p.text_fit(
                    hr.x0 + indent,
                    hr.y1 - 13.0,
                    &track_label(doc, t),
                    &TextStyle::sans(11.5, c.text_secondary),
                    hr.width() - indent - 112.0,
                );
                small_button(p, th, hr.x1 - 98.0, cy, "m", t.mixer.mute.then_some(c.accent_warning));
                small_button(p, th, hr.x1 - 72.0, cy, "s", t.mixer.solo.then_some(c.accent_info));
                let arm = small_button(p, th, hr.x1 - 46.0, cy, "", t.armed.then_some(c.accent_danger));
                if t.armed {
                    p.rrect(arm, 3.0, c.accent_danger);
                    p.circle(arm.center(), 3.5, c.on_accent_danger);
                } else {
                    p.circle_stroke(arm.center(), 4.0, 1.0, c.text_primary);
                }
                fader(p, th, hr.x1 - 98.0, hr.x1 - 30.0, hr.y1 - 13.0, t.mixer.gain_db);
                meter(p, th, hr.x1 - 16.0, hr.y0 + 10.0, hr.y1 - 6.0, if t.armed { 0.35 } else { 0.6 });
            }
            if selected {
                p.rrect_stroke(hr.inset(-1.0), 0.0, 1.0, c.accent_primary_border);
            }
            p.hline(hr.x0, lane.x1, hr.y1 - 1.0, c.border_default);
        }
        RowKind::Return | RowKind::Master | RowKind::Lane(_) => {
            let auto_bg = Rgba::hex(0x0e0e0e);
            p.rect(hr, auto_bg);
            p.rect(Rect::new(hr.x0, hr.y0, hr.x0 + 4.0, hr.y1), tc.alpha(0.45));
            p.rect(lane, Rgba::new(0, 0, 0, 64));
            let cy = hr.center().y;
            let target = match row.kind {
                RowKind::Lane(id) => doc.lanes.get(&id).map(|l| l.target.clone()),
                _ => Some(ParamTarget::Gain(row.track)),
            };
            let label = match row.kind {
                RowKind::Return => {
                    let letter = (b'a'
                        + doc
                            .tracks_in_order()
                            .iter()
                            .filter(|x| doc.tracks[x].kind == TrackKind::Return)
                            .position(|x| *x == row.track)
                            .unwrap_or(0) as u8) as char;
                    let badge = Rect::new(hr.x0 + 12.0, cy - 8.0, hr.x0 + 26.0, cy + 8.0);
                    p.rrect(badge, 3.0, c.bg_fill);
                    p.text(
                        badge.center().x,
                        cy,
                        &letter.to_string(),
                        &TextStyle::mono(10.0, c.text_secondary),
                        Align::Center,
                    );
                    p.text(hr.x0 + 34.0, cy, &t.name, &TextStyle::sans(12.5, c.text_strong).medium(), Align::Left);
                    "gain"
                }
                RowKind::Master => {
                    icons::draw(
                        p,
                        Icon::Pulse,
                        Rect::new(hr.x0 + 12.0, cy - 6.0, hr.x0 + 24.0, cy + 6.0),
                        c.text_secondary,
                    );
                    p.text(hr.x0 + 32.0, cy, "master", &TextStyle::sans(12.5, c.text_strong).medium(), Align::Left);
                    "level"
                }
                _ => {
                    p.text(hr.x0 + indent + 10.0, cy, "gain", &TextStyle::mono(11.0, c.text_secondary), Align::Left);
                    ""
                }
            };
            if !label.is_empty() {
                let field = Rect::new(hr.x1 - 120.0, cy - 10.0, hr.x1 - 40.0, cy + 10.0);
                p.rrect(field, 5.0, c.bg_control);
                p.rrect_stroke(field.inset(-0.5), 5.0, 1.0, c.border_default);
                p.text(field.x0 + 8.0, cy, label, &TextStyle::mono(11.0, c.text_primary), Align::Left);
                icons::draw(
                    p,
                    Icon::ChevronDown,
                    Rect::new(field.x1 - 16.0, cy - 4.0, field.x1 - 6.0, cy + 4.0),
                    c.text_secondary,
                );
                small_button(p, th, hr.x1 - 32.0, cy, "m", t.mixer.mute.then_some(c.accent_warning));
            }
            if let Some(target) = target
                && let Some(lane_data) = doc.lanes.values().find(|x| x.target == target)
            {
                draw_automation(p, th, l, lane_data, lane);
            }
            p.hline(hr.x0, lane.x1, hr.y1 - 1.0, c.border_subtle);
        }
    }
}

fn draw_automation(p: &mut Painter<'_>, th: &Theme, l: &ArrangeLayout, lane: &Lane, r: Rect) {
    if lane.points.is_empty() {
        return;
    }
    let (lo, hi) = lane.points.iter().fold((f32::MAX, f32::MIN), |(a, b), q| (a.min(q.value), b.max(q.value)));
    let (lo, hi) = if hi - lo < 1e-3 { (lo - 1.0, hi + 1.0) } else { (lo, hi) };
    let y_of = |v: f32| r.y1 - 6.0 - (r.height() - 12.0) * ((v - lo) / (hi - lo)) as f64;
    let mut pts = vec![Point::new(r.x0, y_of(lane.points[0].value))];
    for (i, q) in lane.points.iter().enumerate() {
        let x = l.x_of(q.tick);
        if i > 0 && lane.points[i - 1].curve == Curve::Hold {
            pts.push(Point::new(x, pts.last().unwrap().y));
        }
        pts.push(Point::new(x, y_of(q.value)));
    }
    pts.push(Point::new(r.x1, pts.last().unwrap().y));
    p.polyline(&pts, 1.25, th.c.text_secondary);
    for q in lane.points.iter() {
        let pt = Point::new(l.x_of(q.tick), y_of(q.value));
        p.circle(pt, 3.0, th.c.bg_base);
        p.circle_stroke(pt, 3.0, 1.2, th.c.text_strong);
    }
}

fn draw_clip(p: &mut Painter<'_>, th: &Theme, doc: &Document, st: &ArrangeState, l: &ArrangeLayout, cb: &ClipBox) {
    let c = &th.c;
    let Some(clip) = doc.clips.get(&cb.clip) else { return };
    let Some(pl) = doc.placements.get(&cb.placement) else { return };
    let col = th.track(clip.color.0);
    let muted = doc.tracks[&clip.track].mixer.mute;
    let col = if muted { col.mix(Rgba::hex(0x333333), 0.45) } else { col };
    let r = cb.rect;
    let header_h = 17.0;
    p.rrect(r, 4.0, col.mix(Rgba::hex(0x111111), 0.16));
    p.clip(r, |p| {
        p.rect(Rect::new(r.x0, r.y0, r.x1, r.y0 + header_h), col);
        p.text_fit(
            r.x0 + 6.0,
            r.y0 + header_h / 2.0,
            &clip.name,
            &TextStyle::sans(11.5, c.on_accent_primary).medium(),
            r.width() - 12.0,
        );
        // loop seams
        let len = clip.length.0;
        let mut k = (pl.offset.0 / len + 1) * len;
        while k < pl.offset.0 + pl.length.0 {
            let x = l.x_of(pl.start + Ticks(k - pl.offset.0));
            p.vline(x, r.y0 + header_h, r.y1, col.alpha(0.35));
            k += len;
        }
        // note thumbnail
        let body = Rect::new(r.x0 + 2.0, r.y0 + header_h + 3.0, r.x1 - 2.0, r.y1 - 3.0);
        let feed = doc.inlet_cable(clip.id).map(|k| k.from.module);
        if let Some(pat) = feed.and_then(|m| doc.pattern(m, clip.id))
            && !pat.notes.is_empty()
        {
            let (lo, hi) = pat.notes.iter().fold((127u8, 0u8), |(a, b), n| (a.min(n.pitch), b.max(n.pitch)));
            let span = (hi - lo).max(6) as f64;
            let nh = (body.height() / (span + 1.0)).clamp(1.5, 3.0);
            let mut k = 0;
            while k * len < pl.offset.0 + pl.length.0 {
                for n in pat.notes.iter() {
                    let ct = k * len + n.start.0;
                    if ct < pl.offset.0 || ct >= pl.offset.0 + pl.length.0 || n.start.0 >= len {
                        continue;
                    }
                    let t0 = pl.start + Ticks(ct - pl.offset.0);
                    let x0 = l.x_of(t0);
                    let x1 = l.x_of(t0 + n.len).max(x0 + 2.0);
                    let y = body.y1 - nh - (n.pitch - lo) as f64 / span * (body.height() - nh);
                    p.rect(Rect::new(x0, y, x1 - 0.5, y + nh), col.alpha(0.9));
                }
                k += 1;
                if !clip.looped {
                    break;
                }
            }
        }
    });
    p.rrect_stroke(r.inset(-0.5), 4.0, 1.0, col.alpha(0.7));
    if st.selected_clip == Some(clip.id) {
        p.rrect_stroke(r.inset(1.0), 5.0, 2.0, c.accent_primary);
    }
}
