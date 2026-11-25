//! The arrange view: track headers on the left, the timeline on the right.

use std::collections::BTreeSet;

use sat_doc::sat_time::{PPQ, Ticks};
use sat_doc::*;
use sat_theme::{Rgba, Theme};
use vello::kurbo::{Point, Rect};

use crate::controls;
use crate::icons::{self, Icon};
use crate::mixer::Levels;
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
    /// The header shows the track's monitor switch (armed, or not on auto).
    pub monitor: bool,
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
    /// An audio clip's fade handles (fade in, fade out), at the top of the clip.
    pub fades: Option<(Point, Point)>,
}

/// One take lane of an audio clip placement on screen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TakeBox {
    /// The placement.
    pub placement: PlacementId,
    /// Its clip.
    pub clip: ClipId,
    /// Index of the take.
    pub take: u16,
    /// Where it is drawn.
    pub rect: Rect,
    /// The timeline tick of the clip's start.
    pub origin: Ticks,
}

/// Height of a take lane under an audio clip.
pub const TAKE_H: f64 = 12.0;
/// Most take lanes shown under a track.
pub const MAX_TAKE_LANES: usize = 4;

/// How many take lanes a track shows: the most takes of its audio clips,
/// when any clip has more than one.
pub fn take_lanes(doc: &Document, t: TrackId) -> usize {
    let n = doc.clips_on(t).filter_map(|c| c.audio.as_ref()).map(|a| a.takes.len()).max().unwrap_or(0);
    if n > 1 { n.min(MAX_TAKE_LANES) } else { 0 }
}

/// Seconds of clip time `ct` of placement `p` after the clip's start, on the
/// timeline (audio plays at its own speed, whatever the tempo).
pub fn clip_secs(tempo: &sat_doc::sat_time::TempoMap, p: &Placement, ct: f64) -> f64 {
    let origin = (p.start.0 - p.offset.0) as f64;
    tempo.tick_to_secs(origin + ct) - tempo.tick_to_secs(origin)
}

/// Clip time (fractional ticks) heard `secs` after an audio clip's start.
pub fn clip_tick_at(tempo: &sat_doc::sat_time::TempoMap, p: &Placement, secs: f64) -> f64 {
    let origin = (p.start.0 - p.offset.0) as f64;
    tempo.secs_to_tick(tempo.tick_to_secs(origin) + secs) - origin
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
    /// A track header (outside its controls).
    Header(TrackId),
    /// A header's mute button.
    Mute(TrackId),
    /// A header's solo button.
    Solo(TrackId),
    /// A header's record-arm button.
    Arm(TrackId),
    /// A part of a header's monitor switch.
    Monitor(TrackId, Monitor),
    /// A header's fader.
    Fader(TrackId),
    /// A take lane of an audio clip, at a clip time.
    Take {
        /// The placement.
        placement: PlacementId,
        /// Its clip.
        clip: ClipId,
        /// Index of the take.
        take: u16,
        /// Clip time under the point.
        at: Ticks,
    },
    /// An audio clip's fade handle.
    Fade {
        /// The placement.
        placement: PlacementId,
        /// Its clip.
        clip: ClipId,
        /// The fade out (else the fade in).
        out: bool,
    },
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
    /// Take lanes of visible audio clip placements.
    pub takes: Vec<TakeBox>,
    origin: Ticks,
    px_per_tick: f64,
}

/// Height of the clip part of a track row.
const TRACK_H: f64 = 52.0;

fn row_height(kind: RowKind) -> f64 {
    match kind {
        RowKind::Track => TRACK_H,
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
            let lanes = if kind == RowKind::Track { take_lanes(doc, t) } else { 0 };
            let h = row_height(kind) + if lanes > 0 { lanes as f64 * TAKE_H + 4.0 } else { 0.0 };
            let tr = &doc.tracks[&t];
            let monitor = kind == RowKind::Track && (tr.armed || tr.monitor != Monitor::Auto);
            rows.push(Row { track: t, kind, y, h, depth: depth(t), monitor });
            y += h;
            if kind == RowKind::Track {
                for l in doc.lanes.values().filter(|l| l.target == ParamTarget::Gain(t)) {
                    let k = RowKind::Lane(l.id);
                    rows.push(Row { track: t, kind: k, y, h: row_height(k), depth: depth(t), monitor: false });
                    y += row_height(k);
                }
            }
        }
        let mut layout = ArrangeLayout {
            bounds,
            headers,
            ruler,
            timeline,
            rows,
            clips: Vec::new(),
            takes: Vec::new(),
            origin: st.origin,
            px_per_tick,
        };
        let mut clips = Vec::new();
        let mut takes = Vec::new();
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
            let lanes = take_lanes(doc, c.track);
            let bottom = if lanes > 0 { row.y + TRACK_H - 3.0 } else { row.y + row.h - 4.0 };
            let rect = Rect::new(x0 + 1.0, row.y + 4.0, x1 - 1.0, bottom);
            let fades = c.audio.as_ref().map(|a| {
                // a fade ends where its length in clip time falls on the timeline
                let fin = layout.x_of(p.start + a.fade_in - p.offset).clamp(rect.x0, rect.x1);
                let fout = layout.x_of(p.start + (c.length - a.fade_out) - p.offset).clamp(rect.x0, rect.x1);
                (Point::new(fin, rect.y0 + 3.0), Point::new(fout, rect.y0 + 3.0))
            });
            clips.push(ClipBox { placement: p.id, clip: c.id, rect, fades });
            if let Some(a) = &c.audio {
                for (i, _) in a.takes.iter().enumerate().take(lanes) {
                    let y = row.y + TRACK_H + i as f64 * TAKE_H;
                    takes.push(TakeBox {
                        placement: p.id,
                        clip: c.id,
                        take: i as u16,
                        rect: Rect::new(rect.x0, y, rect.x1, y + TAKE_H - 1.0),
                        origin: p.start - p.offset,
                    });
                }
            }
        }
        clips.sort_by(|a, b| a.rect.x0.total_cmp(&b.rect.x0));
        layout.clips = clips;
        layout.takes = takes;
        layout
    }

    /// Screen x of a tick.
    pub fn x_of(&self, t: Ticks) -> f64 {
        self.timeline.x0 + (t.0 - self.origin.0) as f64 * self.px_per_tick
    }

    /// Fractional tick at a screen x.
    pub fn tick_at_f(&self, x: f64) -> f64 {
        self.origin.0 as f64 + (x - self.timeline.x0) / self.px_per_tick
    }

    /// Screen x of a fractional tick.
    pub fn x_of_f(&self, t: f64) -> f64 {
        self.timeline.x0 + (t - self.origin.0 as f64) * self.px_per_tick
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
        for c in self.clips.iter().rev() {
            if let Some((fin, fout)) = c.fades {
                let near = |h: Point| (pt.x - h.x).abs() <= 6.0 && (pt.y - h.y).abs() <= 6.0;
                // the fade out wins when both handles sit together at a short clip's end
                if near(fout) {
                    return Hit::Fade { placement: c.placement, clip: c.clip, out: true };
                }
                if near(fin) {
                    return Hit::Fade { placement: c.placement, clip: c.clip, out: false };
                }
            }
        }
        if let Some(c) = self.clips.iter().rev().find(|c| c.rect.contains(pt)) {
            return Hit::Clip { placement: c.placement, clip: c.clip };
        }
        if let Some(k) = self.takes.iter().rev().find(|k| k.rect.contains(pt)) {
            return Hit::Take { placement: k.placement, clip: k.clip, take: k.take, at: self.tick_at(pt.x) - k.origin };
        }
        let Some(row) = self.rows.iter().find(|r| pt.y >= r.y && pt.y < r.y + r.h) else { return Hit::Nothing };
        if self.headers.contains(pt) {
            let hc = header_controls(row, self.headers);
            let t = row.track;
            let inside = |r: Option<Rect>| r.is_some_and(|r| r.contains(pt));
            if inside(hc.mute) {
                return Hit::Mute(t);
            }
            if inside(hc.solo) {
                return Hit::Solo(t);
            }
            if inside(hc.arm) {
                return Hit::Arm(t);
            }
            if let Some((m, _)) = hc.monitor.and_then(|r| monitor_parts(r).into_iter().find(|(_, p)| p.contains(pt))) {
                return Hit::Monitor(t, m);
            }
            if hc.fader.is_some_and(|f| pt.x >= f.x0 - 6.0 && pt.x <= f.x1 + 6.0 && (pt.y - f.y).abs() <= 7.0) {
                return Hit::Fader(t);
            }
            return Hit::Header(t);
        }
        if self.timeline.contains(pt) {
            return Hit::Lane(row.track, self.tick_at(pt.x));
        }
        Hit::Nothing
    }
}

/// A horizontal header fader: travel from `x0` (silent) to `x1` (+6 dB) at height `y`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HFader {
    /// Left end.
    pub x0: f64,
    /// Right end.
    pub x1: f64,
    /// Centre line.
    pub y: f64,
}

impl HFader {
    /// Fader position (0–1) at screen x.
    pub fn pos_at(&self, x: f64) -> f64 {
        ((x - self.x0) / (self.x1 - self.x0).max(1.0)).clamp(0.0, 1.0)
    }

    /// Screen x of a position.
    pub fn x_of(&self, pos: f64) -> f64 {
        self.x0 + (self.x1 - self.x0) * pos.clamp(0.0, 1.0)
    }
}

/// Where a row's header controls are. Drawing and hit testing both use it.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct HeaderControls {
    /// Mute button.
    pub mute: Option<Rect>,
    /// Solo button.
    pub solo: Option<Rect>,
    /// Record-arm button.
    pub arm: Option<Rect>,
    /// Fader.
    pub fader: Option<HFader>,
    /// Meter.
    pub meter: Option<Rect>,
    /// The monitor switch: in, auto, off.
    pub monitor: Option<Rect>,
}

/// The monitor modes, in the order the switch shows them.
pub const MONITOR_MODES: [(Monitor, &str); 3] = [(Monitor::In, "in"), (Monitor::Auto, "auto"), (Monitor::Off, "off")];

/// The three parts of a monitor switch.
pub fn monitor_parts(r: Rect) -> [(Monitor, Rect); 3] {
    let w = r.width() / 3.0;
    let mut k = 0.0;
    MONITOR_MODES.map(|(m, _)| {
        let x = r.x0 + w * k;
        k += 1.0;
        (m, Rect::new(x, r.y0, x + w, r.y1))
    })
}

/// The header controls of `row` in the header column `headers`.
pub fn header_controls(row: &Row, headers: Rect) -> HeaderControls {
    let (x1, y0, y1) = (headers.x1, row.y, row.y + row.h);
    let indent = 14.0 + 10.0 * row.depth as f64;
    let button = |x: f64, cy: f64| Some(Rect::new(x, cy - 11.0, x + 22.0, cy + 11.0));
    match row.kind {
        RowKind::Group => {
            let cy = y0 + 17.0;
            HeaderControls {
                mute: button(x1 - 98.0, cy),
                solo: button(x1 - 72.0, cy),
                arm: None,
                fader: Some(HFader { x0: headers.x0 + indent, x1: x1 - 50.0, y: y0 + 33.0 }),
                meter: Some(Rect::new(x1 - 16.0, y0 + 12.0, x1 - 9.0, y1 - 8.0)),
                monitor: None,
            }
        }
        RowKind::Track => {
            let cy = y0 + 17.0;
            HeaderControls {
                mute: button(x1 - 98.0, cy),
                solo: button(x1 - 72.0, cy),
                arm: button(x1 - 46.0, cy),
                fader: Some(HFader { x0: x1 - 98.0, x1: x1 - 30.0, y: y0 + TRACK_H - 13.0 }),
                meter: Some(Rect::new(x1 - 16.0, y0 + 10.0, x1 - 9.0, y0 + TRACK_H - 6.0)),
                monitor: row.monitor.then(|| {
                    let cy = y0 + TRACK_H - 13.0;
                    Rect::new(headers.x0 + indent - 2.0, cy - 9.0, headers.x0 + indent + 94.0, cy + 9.0)
                }),
            }
        }
        RowKind::Return | RowKind::Master => {
            HeaderControls { mute: button(x1 - 32.0, (y0 + y1) / 2.0), ..Default::default() }
        }
        RowKind::Lane(_) => HeaderControls::default(),
    }
}

fn track_label(doc: &Document, t: &Track) -> String {
    match t.kind {
        TrackKind::Track => {
            let ins = t.instrument_grid.and_then(|g| sat_modules::instrument::label(doc, g));
            ins.unwrap_or_else(|| "empty".into())
        }
        _ => String::new(),
    }
}

/// Draws a small square toggle button ("m", "s") in `r`.
fn small_button(p: &mut Painter<'_>, th: &Theme, r: Rect, label: &str, on: Option<Rgba>) -> Rect {
    let cy = r.center().y;
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

fn fader(p: &mut Painter<'_>, th: &Theme, f: HFader, gain_db: f32) {
    let (x0, y) = (f.x0, f.y);
    p.rrect(Rect::new(x0, y - 1.5, f.x1, y + 1.5), 1.5, Rgba::hex(0x262626));
    let tx = f.x_of(controls::fader_pos(gain_db));
    p.rrect(Rect::new(x0, y - 1.5, tx, y + 1.5), 1.5, th.c.text_secondary.alpha(0.6));
    p.circle(Point::new(tx, y), 5.0, Rgba::hex(0xd8d8d8));
    p.circle_stroke(Point::new(tx, y), 5.0, 1.0, Rgba::hex(0x050505));
}

fn meter(p: &mut Painter<'_>, th: &Theme, r: Rect, (l, rr): (f32, f32)) {
    let h = r.height();
    let (a, b) = (r.x0, r.x0 + 4.0);
    p.rect(Rect::new(a, r.y0, a + 3.0, r.y1), Rgba::hex(0x1a1a1a));
    p.rect(Rect::new(b, r.y0, b + 3.0, r.y1), Rgba::hex(0x1a1a1a));
    let fill = th.c.accent_success;
    p.rect(Rect::new(a, r.y1 - h * controls::meter_pos(l), a + 3.0, r.y1), fill);
    p.rect(Rect::new(b, r.y1 - h * controls::meter_pos(rr), b + 3.0, r.y1), fill);
}

/// Draws the arrange view.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    p: &mut Painter<'_>,
    th: &Theme,
    doc: &Document,
    st: &ArrangeState,
    tr: &TransportView,
    levels: &Levels,
    sounds: &sat_disk::Pool,
    l: &ArrangeLayout,
) {
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
            draw_row(p, th, doc, st, levels, l, row);
        }
        let tempo = doc.tempo_map().ok();
        for cb in &l.clips {
            match (&doc.clips.get(&cb.clip).and_then(|c| c.audio.as_ref()), &tempo) {
                (Some(a), Some(tempo)) => draw_audio_clip(p, th, doc, st, l, cb, a, tempo, sounds),
                _ => draw_clip(p, th, doc, st, l, cb),
            }
        }
        if let Some(tempo) = &tempo {
            for k in &l.takes {
                draw_take(p, th, doc, l, k, tempo, sounds);
            }
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

fn draw_row(
    p: &mut Painter<'_>,
    th: &Theme,
    doc: &Document,
    st: &ArrangeState,
    levels: &Levels,
    l: &ArrangeLayout,
    row: &Row,
) {
    let c = &th.c;
    let t = &doc.tracks[&row.track];
    let tc = th.track(t.color.0);
    let hr = Rect::new(l.headers.x0, row.y, l.headers.x1, row.y + row.h);
    let lane = Rect::new(l.timeline.x0, row.y, l.timeline.x1, row.y + row.h);
    let selected = st.selected_track == Some(row.track) && row.kind == RowKind::Track;
    let indent = 14.0 + 10.0 * row.depth as f64;
    let hc = header_controls(row, l.headers);
    let level = levels.get(&row.track).copied().unwrap_or((0.0, 0.0));
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
                draw_mute_solo(p, th, &hc, t);
                if let (Some(f), Some(m)) = (hc.fader, hc.meter) {
                    fader(p, th, f, t.mixer.gain_db);
                    meter(p, th, m, level);
                }
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
                if let Some(mr) = hc.monitor {
                    p.rrect_stroke(mr.inset(-0.5), 4.0, 1.0, c.border_default);
                    for ((m, part), (_, label)) in monitor_parts(mr).into_iter().zip(MONITOR_MODES) {
                        let on = t.monitor == m;
                        if on {
                            p.rrect(part.inset(-2.0), 3.0, c.bg_fill);
                        }
                        let col = if on { c.accent_primary } else { c.text_secondary };
                        p.text(part.center().x, part.center().y, label, &TextStyle::mono(10.5, col), Align::Center);
                    }
                } else {
                    p.text_fit(
                        hr.x0 + indent,
                        hr.y0 + TRACK_H - 13.0,
                        &track_label(doc, t),
                        &TextStyle::sans(11.5, c.text_secondary),
                        hr.width() - indent - 112.0,
                    );
                }
                draw_mute_solo(p, th, &hc, t);
                if let Some(ar) = hc.arm {
                    let arm = small_button(p, th, ar, "", t.armed.then_some(c.accent_danger));
                    if t.armed {
                        p.rrect(arm, 3.0, c.accent_danger);
                        p.circle(arm.center(), 3.5, c.on_accent_danger);
                    } else {
                        p.circle_stroke(arm.center(), 4.0, 1.0, c.text_primary);
                    }
                }
                if let (Some(f), Some(m)) = (hc.fader, hc.meter) {
                    fader(p, th, f, t.mixer.gain_db);
                    meter(p, th, m, level);
                }
                draw_take_names(p, th, doc, st, row, hr);
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
                draw_mute_solo(p, th, &hc, t);
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

fn draw_mute_solo(p: &mut Painter<'_>, th: &Theme, hc: &HeaderControls, t: &Track) {
    if let Some(r) = hc.mute {
        small_button(p, th, r, "m", t.mixer.mute.then_some(th.c.accent_warning));
    }
    if let Some(r) = hc.solo {
        small_button(p, th, r, "s", t.mixer.solo.then_some(th.c.accent_info));
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
        let notes = feed.map(|m| sat_modules::effective_notes(doc, m, clip.id)).unwrap_or_default();
        if !notes.is_empty() {
            let (lo, hi) = notes.iter().fold((127u8, 0u8), |(a, b), n| (a.min(n.pitch), b.max(n.pitch)));
            let span = (hi - lo).max(6) as f64;
            let nh = (body.height() / (span + 1.0)).clamp(1.5, 3.0);
            let mut k = 0;
            while k * len < pl.offset.0 + pl.length.0 {
                for n in notes.iter() {
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

/// The audio clip whose takes a track's header lists: the selected clip if
/// it is on the track, else the track's first audio clip with takes.
fn takes_clip<'a>(doc: &'a Document, st: &ArrangeState, t: TrackId) -> Option<&'a Clip> {
    let sel = st.selected_clip.and_then(|c| doc.clips.get(&c)).filter(|c| c.track == t && c.audio.is_some());
    sel.or_else(|| doc.clips_on(t).find(|c| c.audio.as_ref().is_some_and(|a| a.takes.len() > 1)))
}

fn draw_take_names(p: &mut Painter<'_>, th: &Theme, doc: &Document, st: &ArrangeState, row: &Row, hr: Rect) {
    let lanes = take_lanes(doc, row.track);
    let Some(a) = takes_clip(doc, st, row.track).and_then(|c| c.audio.as_ref()).filter(|_| lanes > 0) else {
        return;
    };
    let c = &th.c;
    for (i, take) in a.takes.iter().enumerate().take(lanes) {
        let y = row.y + TRACK_H + i as f64 * TAKE_H;
        let lane = Rect::new(hr.x0 + 14.0, y, hr.x1 - 12.0, y + TAKE_H - 1.0);
        let used = a.comp.iter().any(|s| s.take as usize == i);
        if used {
            p.rect(lane, th.track(doc.tracks[&row.track].color.0).alpha(0.16));
        }
        let col = if used { c.text_strong } else { c.text_secondary };
        p.text(lane.x0 + 6.0, lane.center().y, &take.name, &TextStyle::sans(10.5, col), Align::Left);
        if let Some(f) = doc.audio.get(&take.audio) {
            let secs = (f.frames as f64 / f.rate.max(1) as f64).round() as u64;
            let len = format!("{}:{:02}", secs / 60, secs % 60);
            p.text(lane.x1 - 6.0, lane.center().y, &len, &TextStyle::mono(10.0, c.text_secondary), Align::Right);
        }
    }
}

/// Min and max of take `take` over clip time `ct0..ct1` of placement `pl`,
/// or nothing when its sound is not loaded.
fn take_peak(
    tempo: &sat_doc::sat_time::TempoMap,
    pl: &Placement,
    take: &Take,
    sounds: &sat_disk::Pool,
    ct0: f64,
    ct1: f64,
) -> Option<(f32, f32)> {
    let ov = sounds.overview(take.audio)?;
    let rate = sounds.get(take.audio)?.rate as f64;
    let f = |ct: f64| take.offset as f64 + clip_secs(tempo, pl, ct) * rate;
    Some(sat_disk::overview_range(ov, f(ct0), f(ct1)))
}

/// Visible pixel columns of a clip box, with the clip time at each edge.
fn columns<'a>(l: &'a ArrangeLayout, pl: &Placement, r: Rect, step: f64) -> impl Iterator<Item = (f64, f64, f64)> + 'a {
    let origin = (pl.start.0 - pl.offset.0) as f64;
    let (x0, x1) = (r.x0.max(l.timeline.x0).floor(), r.x1.min(l.timeline.x1));
    (0..((x1 - x0) / step).ceil().max(0.0) as usize).map(move |k| {
        let x = x0 + k as f64 * step;
        (x, l.tick_at_f(x) - origin, l.tick_at_f(x + step) - origin)
    })
}

/// Equal-power fade curve from `a` (silent) to `b` (full).
fn fade_curve(a: Point, b: Point) -> Vec<Point> {
    (0..=16)
        .map(|k| {
            let u = k as f64 / 16.0;
            let g = (u * core::f64::consts::FRAC_PI_2).sin();
            Point::new(a.x + (b.x - a.x) * u, a.y + (b.y - a.y) * g)
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn draw_audio_clip(
    p: &mut Painter<'_>,
    th: &Theme,
    doc: &Document,
    st: &ArrangeState,
    l: &ArrangeLayout,
    cb: &ClipBox,
    a: &AudioClip,
    tempo: &sat_doc::sat_time::TempoMap,
    sounds: &sat_disk::Pool,
) {
    let c = &th.c;
    let Some(clip) = doc.clips.get(&cb.clip) else { return };
    let Some(pl) = doc.placements.get(&cb.placement) else { return };
    let col = th.track(clip.color.0);
    let col = if doc.tracks[&clip.track].mixer.mute { col.mix(Rgba::hex(0x333333), 0.45) } else { col };
    let r = cb.rect;
    let origin = (pl.start.0 - pl.offset.0) as f64;
    let gain = 10f32.powf(a.gain_db / 20.0);
    p.rrect(r, 3.0, col.mix(Rgba::hex(0x111111), 0.38));
    p.clip(r, |p| {
        // the comp's waveform, take by take
        let (cy, half) = (r.center().y + 2.0, (r.height() - 8.0) / 2.0);
        for (x, ct0, ct1) in columns(l, pl, r, 1.0) {
            let Some(take) = a.take_at(Ticks(ct0.floor() as i64)).and_then(|i| a.takes.get(i as usize)) else {
                continue;
            };
            let Some((lo, hi)) = take_peak(tempo, pl, take, sounds, ct0, ct1) else { continue };
            let (lo, hi) = ((lo * gain).max(-1.0) as f64, (hi * gain).min(1.0) as f64);
            let (y0, y1) = (cy - hi * half, cy - lo * half);
            p.rect(Rect::new(x, y0.min(cy - 0.5), x + 1.0, y1.max(cy + 0.5)), col.alpha(0.9));
        }
        // comp boundaries: a crossfade, and the take that starts there
        let xf = a.xfade_ms as f64 / 1000.0 / 2.0;
        for (k, seg) in a.comp.iter().enumerate() {
            let b = seg.start.0 as f64;
            let x = l.x_of_f(origin + b);
            if k > 0 {
                let bs = clip_secs(tempo, pl, b);
                let w = 5.0f64;
                let xl = l.x_of_f(origin + clip_tick_at(tempo, pl, bs - xf)).min(x - w);
                let xr = l.x_of_f(origin + clip_tick_at(tempo, pl, bs + xf)).max(x + w);
                let (top, bot) = (r.y0 + 3.0, r.y1 - 1.0);
                let white = c.text_max.alpha(0.85);
                p.polyline(&fade_curve(Point::new(xr, bot), Point::new(xl, top)), 1.2, white);
                p.polyline(&fade_curve(Point::new(xl, bot), Point::new(xr, top)), 1.2, white);
            }
            if let Some(t) = a.takes.get(seg.take as usize) {
                let next = a.comp.get(k + 1).map_or(clip.length.0 as f64, |n| n.start.0 as f64);
                let w = l.x_of_f(origin + next) - x - 14.0;
                if w > 14.0 {
                    p.text_fit(x.max(r.x0) + 6.0, r.y0 + 9.0, &t.name, &TextStyle::sans(10.5, c.text_max), w);
                }
            }
        }
        // fades, with their handles
        if let Some((fin, fout)) = cb.fades {
            let white = c.text_max.alpha(0.9);
            let start = l.x_of(pl.start - pl.offset).max(r.x0);
            let end = l.x_of(pl.start + clip.length - pl.offset).min(r.x1);
            if a.fade_in.0 > 0 {
                p.polyline(&fade_curve(Point::new(start, r.y1), fin), 1.2, white);
            }
            if a.fade_out.0 > 0 {
                p.polyline(&fade_curve(Point::new(end, r.y1), fout), 1.2, white);
            }
            for h in [fin, fout] {
                let sq = Rect::new(h.x - 3.0, h.y - 3.0, h.x + 3.0, h.y + 3.0);
                p.rect(sq, c.text_max);
                p.rrect_stroke(sq, 0.0, 1.0, Rgba::hex(0x111111));
            }
        }
    });
    p.rrect_stroke(r.inset(-0.5), 3.0, 1.0, col.alpha(0.7));
    if st.selected_clip == Some(clip.id) {
        p.rrect_stroke(r.inset(1.0), 4.0, 2.0, c.accent_primary);
    }
}

fn draw_take(
    p: &mut Painter<'_>,
    th: &Theme,
    doc: &Document,
    l: &ArrangeLayout,
    k: &TakeBox,
    tempo: &sat_doc::sat_time::TempoMap,
    sounds: &sat_disk::Pool,
) {
    let Some(clip) = doc.clips.get(&k.clip) else { return };
    let Some(a) = &clip.audio else { return };
    let Some(pl) = doc.placements.get(&k.placement) else { return };
    let Some(take) = a.takes.get(k.take as usize) else { return };
    let col = th.track(clip.color.0);
    let r = k.rect;
    let origin = (pl.start.0 - pl.offset.0) as f64;
    p.rect(r, col.mix(Rgba::hex(0x111111), 0.16));
    // the stretches of the comp this take plays
    let mut segs = a.comp.iter().enumerate().filter(|(_, s)| s.take == k.take).peekable();
    if segs.peek().is_some() {
        for (i, s) in segs {
            let end = a.comp.get(i + 1).map_or(clip.length.0, |n| n.start.0);
            let (x0, x1) = (l.x_of_f(origin + s.start.0 as f64).max(r.x0), l.x_of_f(origin + end as f64).min(r.x1));
            if x1 > x0 {
                p.rect(Rect::new(x0, r.y0, x1, r.y1), col.mix(Rgba::hex(0x111111), 0.42));
            }
        }
    }
    let (cy, half) = (r.center().y, r.height() / 2.0 - 1.0);
    for (x, ct0, ct1) in columns(l, pl, r, 2.0) {
        let Some((lo, hi)) = take_peak(tempo, pl, take, sounds, ct0, ct1) else { continue };
        let on = a.take_at(Ticks(ct0.floor() as i64)) == Some(k.take);
        let (y0, y1) = (cy - hi.min(1.0) as f64 * half, cy - lo.max(-1.0) as f64 * half);
        p.rect(Rect::new(x, y0.min(cy - 0.5), x + 1.0, y1.max(cy + 0.5)), col.alpha(if on { 0.95 } else { 0.35 }));
    }
}
