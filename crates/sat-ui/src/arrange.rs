//! The arrange view: track headers on the left, the timeline on the right.

use std::collections::BTreeSet;

use sat_doc::sat_time::{PPQ, Ticks};
use sat_doc::*;
use sat_theme::{Rgba, Theme};
use vello::kurbo::{Point, Rect};

use crate::icons::{self, Icon};
use crate::paint::{Align, Painter, TextStyle};
use crate::session::{SessionHit, SessionView};

/// Width of the track header column.
pub const HEADER_W: f64 = 196.0;
/// Height of the ruler.
pub const RULER_H: f64 = 44.0;
/// Height of the arrange toolbar row.
pub const TOOLBAR_H: f64 = crate::area::HEADER_H;

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
    /// The selected placement (moved, trimmed, duplicated or deleted by keys).
    pub selected_placement: Option<PlacementId>,
    /// The grid clip edits snap to; `None` is off.
    pub snap: Option<Ticks>,
    /// Tracks whose automation lanes are folded away.
    pub hidden_lanes: BTreeSet<TrackId>,
    /// The breakpoint being dragged, shown with its value.
    pub active_point: Option<(LaneId, usize)>,
    /// Vertical zoom: height of a track's clip row, in px.
    pub track_h: f64,
    /// The clip launcher shows beside the timeline (session slots per track row).
    pub launcher: bool,
}

/// Smallest and largest track height (vertical zoom), in px.
pub const TRACK_H_RANGE: (f64, f64) = (28.0, 240.0);
/// Smallest and largest px per bar (horizontal zoom).
pub const PX_PER_BAR_RANGE: (f64, f64) = (8.0, 2000.0);

/// The arrange snap settings the toolbar cycles through.
pub const SNAPS: [Option<Ticks>; 4] = [Some(Ticks(PPQ * 4)), Some(Ticks(PPQ)), Some(Ticks(PPQ / 4)), None];

/// The snap after `s` in [`SNAPS`].
pub fn next_snap(s: Option<Ticks>) -> Option<Ticks> {
    let i = SNAPS.iter().position(|x| *x == s).unwrap_or(0);
    SNAPS[(i + 1) % SNAPS.len()]
}

/// How the toolbar names a snap setting.
pub fn snap_label(s: Option<Ticks>) -> &'static str {
    match s {
        Some(t) if t.0 == PPQ * 4 => "1 bar",
        Some(t) if t.0 == PPQ => "1 beat",
        Some(t) if t.0 == PPQ / 4 => "1/16",
        Some(_) => "grid",
        None => "off",
    }
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
            selected_placement: None,
            snap: Some(Ticks(PPQ * 4)),
            hidden_lanes: BTreeSet::new(),
            active_point: None,
            track_h: TRACK_H,
            launcher: true,
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
    /// Height of the clip part (a track's row above its take lanes).
    pub clip_h: f64,
}

impl Row {
    /// A track row too short for its instrument line: just the name line.
    pub fn compact(&self) -> bool {
        self.kind == RowKind::Track && self.clip_h < 40.0
    }
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
    /// A half of a header's source switch: the track's clips or its
    /// automation, from the launcher or the arrangement.
    Source(TrackId, crate::session::Part),
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
    /// The left or right edge of a clip placement: drag to trim it.
    ClipEdge {
        /// The placement.
        placement: PlacementId,
        /// Its clip.
        clip: ClipId,
        /// The right edge (else the left).
        right: bool,
    },
    /// The toolbar's snap button.
    Snap,
    /// A track header's automation button: show, hide or start its lanes.
    AutoToggle(TrackId),
    /// A lane header's +: add the track's next lane.
    LaneAdd(TrackId),
    /// A lane header's ×: remove the lane.
    LaneRemove(LaneId),
    /// An automation breakpoint.
    AutoPoint {
        /// Its lane.
        lane: LaneId,
        /// Its index in the lane.
        index: usize,
    },
    /// Empty space in an automation lane (a return's or master's row may
    /// have no lane yet: it drives the track's gain).
    AutoLane {
        /// The track.
        track: TrackId,
        /// The lane, if it exists.
        lane: Option<LaneId>,
        /// Tick under the point.
        tick: Ticks,
        /// Height under the point, 0–1.
        pos: f64,
    },
    /// Empty timeline on a row, at a tick.
    Lane(TrackId, Ticks),
    /// The ruler, at a tick.
    Ruler(Ticks),
    /// A slot or scene in the clip launcher beside the timeline.
    Session(SessionHit),
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
    /// The toolbar's snap button.
    pub snap: Rect,
    /// Visible automation lanes.
    pub autos: Vec<AutoBox>,
    /// The clip launcher beside the timeline, when shown (below the ruler).
    pub launcher: Option<Rect>,
    /// The launcher's scene heads, in the ruler band.
    pub scene_heads: Vec<(SceneId, Rect)>,
    /// The launcher's "+ scene" head after the last scene.
    pub add_scene: Option<Rect>,
    /// The launcher's "stop all" button, in the ruler band.
    pub stop_all: Option<Rect>,
    origin: Ticks,
    px_per_tick: f64,
}

/// An automation lane on screen.
#[derive(Debug, Clone, PartialEq)]
pub struct AutoBox {
    /// The track.
    pub track: TrackId,
    /// The lane (a return's or master's row shows one before it exists).
    pub lane: Option<LaneId>,
    /// What it drives.
    pub target: ParamTarget,
    /// The timeline part of its row.
    pub rect: Rect,
    /// Its breakpoints on screen.
    pub points: Vec<Point>,
}

/// How close to a breakpoint a press grabs it, in px.
pub const POINT_GRAB: f64 = 6.0;

/// How close to a clip's edge a press trims it, in px.
pub const EDGE_GRAB: f64 = 6.0;

/// Where the toolbar's snap button is in an arrange tile at `bounds`.
pub fn snap_button(bounds: Rect) -> Rect {
    let ty = bounds.y0 + TOOLBAR_H / 2.0;
    let x = bounds.x1 - 12.0 - 64.0;
    Rect::new(x - 74.0, ty - 10.0, x - 8.0, ty + 10.0)
}

/// Width of a scene column in the launcher beside the timeline.
pub const SLOT_W: f64 = 92.0;

/// Default height of the clip part of a track row.
pub const TRACK_H: f64 = 52.0;

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
        let hx = bounds.x0 + HEADER_W;
        // the launcher takes up to half the width beside the headers
        let lw = if st.launcher {
            ((doc.scenes.len() as f64 + 0.5) * SLOT_W + 8.0).min(((bounds.x1 - hx) * 0.5).max(0.0))
        } else {
            0.0
        };
        let ruler = Rect::new(hx + lw, top, bounds.x1, top + RULER_H);
        let headers = Rect::new(bounds.x0, top + RULER_H, hx, bounds.y1);
        let timeline = Rect::new(hx + lw, top + RULER_H, bounds.x1, bounds.y1);
        let launcher = st.launcher.then(|| Rect::new(hx, top + RULER_H, hx + lw, bounds.y1));
        let scene_heads = if st.launcher {
            doc.scenes
                .iter()
                .enumerate()
                .map(|(i, sc)| {
                    let x = hx + 4.0 + i as f64 * SLOT_W;
                    (sc.id, Rect::new(x, top + RULER_H - 26.0, x + SLOT_W - 4.0, top + RULER_H - 4.0))
                })
                .collect()
        } else {
            Vec::new()
        };
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
            let clip_h = if kind == RowKind::Track {
                st.track_h.clamp(TRACK_H_RANGE.0, TRACK_H_RANGE.1)
            } else {
                row_height(kind)
            };
            let h = clip_h + if lanes > 0 { lanes as f64 * TAKE_H + 4.0 } else { 0.0 };
            rows.push(Row { track: t, kind, y, h, depth: depth(t), clip_h });
            y += h;
            if matches!(kind, RowKind::Track | RowKind::Group) && !st.hidden_lanes.contains(&t) {
                for target in crate::automation::lane_targets(t) {
                    if let Some(l) = crate::automation::lane_of(doc, &target) {
                        let k = RowKind::Lane(l.id);
                        rows.push(Row {
                            track: t,
                            kind: k,
                            y,
                            h: row_height(k),
                            depth: depth(t),
                            clip_h: row_height(k),
                        });
                        y += row_height(k);
                    }
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
            snap: snap_button(bounds),
            autos: Vec::new(),
            launcher,
            add_scene: launcher.map(|_| {
                let x = hx + 4.0 + doc.scenes.len() as f64 * SLOT_W;
                Rect::new(x, top + RULER_H - 26.0, x + SLOT_W * 0.5 - 4.0, top + RULER_H - 4.0)
            }),
            stop_all: launcher.map(|l| Rect::new(l.x1 - 24.0, top + 3.0, l.x1 - 6.0, top + 15.0)),
            scene_heads,
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
            let bottom = if lanes > 0 { row.y + row.clip_h - 3.0 } else { row.y + row.h - 4.0 };
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
                    let y = row.y + row.clip_h + i as f64 * TAKE_H;
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
        layout.autos = layout
            .rows
            .iter()
            .filter_map(|r| {
                let (lane, target) = match r.kind {
                    RowKind::Lane(id) => (Some(id), doc.lanes.get(&id)?.target.clone()),
                    RowKind::Return | RowKind::Master => {
                        let target = ParamTarget::Gain(r.track);
                        (crate::automation::lane_of(doc, &target).map(|l| l.id), target)
                    }
                    _ => return None,
                };
                let rect = Rect::new(timeline.x0, r.y, timeline.x1, r.y + r.h - 1.0);
                let points = lane
                    .and_then(|id| doc.lanes.get(&id))
                    .map(|l| {
                        l.points
                            .iter()
                            .map(|q| {
                                let pos = crate::automation::pos_of(doc, &target, q.value);
                                Point::new(layout.x_of(q.tick), crate::automation::y_of(rect, pos))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                Some(AutoBox { track: r.track, lane, target, rect, points })
            })
            .collect();
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

    /// The row at screen height `y`.
    pub fn row_at(&self, y: f64) -> Option<&Row> {
        self.rows.iter().find(|r| y >= r.y && y < r.y + r.h)
    }

    /// What is under `pt`.
    pub fn hit(&self, pt: Point) -> Hit {
        self.hit_with(None, pt)
    }

    /// The session slot rectangle of scene column `i` on `row`.
    pub fn slot_rect(&self, row: &Row, i: usize) -> Option<Rect> {
        let l = self.launcher?;
        let x = l.x0 + 4.0 + i as f64 * SLOT_W;
        Some(Rect::new(x, row.y + 3.0, x + SLOT_W - 4.0, row.y + row.clip_h - 3.0))
    }

    /// What is under `pt`; with `doc`, the launcher's slots resolve to their clips.
    pub fn hit_with(&self, doc: Option<&Document>, pt: Point) -> Hit {
        if let Some((s, _)) = self.scene_heads.iter().find(|(_, r)| r.contains(pt)) {
            return Hit::Session(SessionHit::Scene(*s));
        }
        if self.add_scene.is_some_and(|r| r.contains(pt)) {
            return Hit::Session(SessionHit::AddScene);
        }
        if self.stop_all.is_some_and(|r| r.contains(pt)) {
            return Hit::Session(SessionHit::StopAll);
        }
        if let (Some(l), Some(doc)) = (self.launcher, doc)
            && l.contains(pt)
        {
            let Some(row) = self.row_at(pt.y) else { return Hit::Nothing };
            let Some((i, (scene, _))) =
                self.scene_heads.iter().enumerate().find(|(_, (_, h))| (h.x0..h.x1).contains(&pt.x))
            else {
                return Hit::Nothing;
            };
            let Some(cell) = self.slot_rect(row, i).filter(|c| c.contains(pt)) else { return Hit::Nothing };
            return Hit::Session(match row.kind {
                RowKind::Group => SessionHit::Group(row.track, *scene),
                RowKind::Track => match doc.slots.get(&(*scene, row.track)) {
                    Some(c) if pt.x < cell.x0 + crate::session::PLAY_W => SessionHit::Launch(row.track, *c),
                    Some(c) => SessionHit::Select(*c),
                    None => SessionHit::Stop(row.track),
                },
                _ => SessionHit::Nothing,
            });
        }
        if self.snap.contains(pt) {
            return Hit::Snap;
        }
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
            // the outer few pixels of a clip trim it; a narrow clip keeps its middle to move it
            let grab = (c.rect.width() / 4.0).min(EDGE_GRAB);
            if pt.x <= c.rect.x0 + grab {
                return Hit::ClipEdge { placement: c.placement, clip: c.clip, right: false };
            }
            if pt.x >= c.rect.x1 - grab {
                return Hit::ClipEdge { placement: c.placement, clip: c.clip, right: true };
            }
            return Hit::Clip { placement: c.placement, clip: c.clip };
        }
        if let Some(k) = self.takes.iter().rev().find(|k| k.rect.contains(pt)) {
            return Hit::Take { placement: k.placement, clip: k.clip, take: k.take, at: self.tick_at(pt.x) - k.origin };
        }
        if let Some(a) = self.autos.iter().find(|a| a.rect.contains(pt)) {
            if let Some(lane) = a.lane
                && let Some(index) = a.points.iter().position(|q| (*q - pt).hypot() <= POINT_GRAB)
            {
                return Hit::AutoPoint { lane, index };
            }
            let pos = crate::automation::pos_at(a.rect, pt.y);
            return Hit::AutoLane { track: a.track, lane: a.lane, tick: self.tick_at(pt.x), pos };
        }
        let Some(row) = self.rows.iter().find(|r| pt.y >= r.y && pt.y < r.y + r.h) else { return Hit::Nothing };
        if self.headers.contains(pt) {
            let hc = header_controls(row, self.headers);
            let t = row.track;
            let inside = |r: Option<Rect>| r.is_some_and(|r| r.contains(pt));
            if inside(hc.auto) {
                return Hit::AutoToggle(t);
            }
            if inside(hc.lane_add) {
                return Hit::LaneAdd(t);
            }
            if let (true, RowKind::Lane(id)) = (inside(hc.lane_remove), row.kind) {
                return Hit::LaneRemove(id);
            }
            if inside(hc.clips_src) {
                return Hit::Source(t, crate::session::Part::Clips);
            }
            if inside(hc.auto_src) {
                return Hit::Source(t, crate::session::Part::Automation);
            }
            return Hit::Header(t);
        }
        if self.timeline.contains(pt) {
            return Hit::Lane(row.track, self.tick_at(pt.x));
        }
        Hit::Nothing
    }
}

/// The controls in a header.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct HeaderControls {
    /// The automation button: show, hide or start the track's lanes.
    pub auto: Option<Rect>,
    /// The source switch's clips half: launcher or arrangement.
    pub clips_src: Option<Rect>,
    /// The source switch's automation half: launcher or arrangement.
    pub auto_src: Option<Rect>,
    /// A lane header's add-lane button.
    pub lane_add: Option<Rect>,
    /// A lane header's remove-lane button.
    pub lane_remove: Option<Rect>,
}

/// The header controls of `row` in the header column `headers`. Mute,
/// solo, arm, monitoring and the fader live in the mixer; a header names
/// the track and says where its clips and automation come from.
pub fn header_controls(row: &Row, headers: Rect) -> HeaderControls {
    let (x1, y0, y1) = (headers.x1, row.y, row.y + row.h);
    match row.kind {
        RowKind::Group => {
            let cy = y0 + 17.0;
            HeaderControls { auto: Some(Rect::new(x1 - 30.0, cy - 9.0, x1 - 10.0, cy + 9.0)), ..Default::default() }
        }
        RowKind::Track => {
            let cy = if row.compact() { y0 + row.clip_h / 2.0 } else { y0 + 17.0 };
            // the source switch is a pair of icons; the lanes button lines
            // up with a group's
            let icon = |x: f64| Some(Rect::new(x, cy - 9.0, x + 20.0, cy + 9.0));
            HeaderControls {
                auto: icon(x1 - 30.0),
                clips_src: icon(x1 - 78.0),
                auto_src: icon(x1 - 57.0),
                ..Default::default()
            }
        }
        RowKind::Return | RowKind::Master => HeaderControls::default(),
        RowKind::Lane(_) => {
            let cy = (y0 + y1) / 2.0;
            let small = |x: f64| Some(Rect::new(x, cy - 9.0, x + 18.0, cy + 9.0));
            HeaderControls { lane_add: small(x1 - 50.0), lane_remove: small(x1 - 28.0), ..Default::default() }
        }
    }
}

fn track_label(doc: &Document, t: &Track) -> String {
    match t.kind {
        TrackKind::Track => {
            let ins = t.instrument_grid.and_then(|g| sat_modules::instrument::label(doc, g));
            let ins = ins.unwrap_or_else(|| "empty".into());
            if t.frozen.is_some() { format!("frozen · {ins}") } else { ins }
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

/// Draws the arrange view.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    p: &mut Painter<'_>,
    th: &Theme,
    doc: &Document,
    st: &ArrangeState,
    tr: &TransportView,
    sounds: &sat_disk::Pool,
    session: &SessionView,
    l: &ArrangeLayout,
) {
    let c = &th.c;
    let b = l.bounds;
    crate::area::region(p, th, b);

    // area header: editor type, what is in the song, snap and zoom
    let (_, hx) = crate::area::header(p, th, b, crate::area::EditorType::Arrange);
    let ty = b.y0 + TOOLBAR_H / 2.0;
    let tracks = doc.tracks.values().filter(|t| t.kind == TrackKind::Track).count();
    let bars = doc.placements.values().map(|pl| pl.start.0 + pl.length.0).max().unwrap_or(0).max(0).div_euclid(PPQ * 4);
    let note_end = if session.any_launcher() { crate::session::back_button(b).x0 } else { snap_button(b).x0 - 52.0 };
    crate::area::note(p, th, hx, ty, &format!("{tracks} tracks · {bars} bars"), note_end - 12.0 - hx);
    let mut x = b.x1 - 12.0;
    for icon in [Icon::Plus, Icon::Search] {
        let r = Rect::new(x - 26.0, ty - 11.0, x, ty + 11.0);
        crate::area::icon_button(p, th, r, icon);
        x -= 32.0;
    }
    let snap = l.snap;
    p.rrect(snap, 5.0, c.neutral_3);
    p.rrect_stroke(snap.inset(-0.5), 5.0, 1.0, c.border_default);
    p.text(snap.x0 + 8.0, ty, snap_label(st.snap), &TextStyle::mono(12.0, c.text_strong), Align::Left);
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
            draw_row(p, th, doc, st, session, l, row);
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
        // a track playing the session greys out its arrangement clips
        for cb in &l.clips {
            if doc.clips.get(&cb.clip).is_some_and(|cl| session.following.contains(&cl.track)) {
                p.rect(cb.rect, Rgba::new(10, 10, 10, 150));
            }
        }
        // playhead
        let x = l.x_of(tr.playhead);
        if x >= l.timeline.x0 && x <= l.timeline.x1 {
            p.vline(x, l.timeline.y0, l.timeline.y1, c.text_max);
        }
    });
    if l.launcher.is_some() {
        draw_launcher(p, th, doc, st, tr, session, l);
    }
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
    session: &SessionView,
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
                draw_auto_button(p, th, doc, st, &hc, t);
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
                let cy = if row.compact() { hr.y0 + row.clip_h / 2.0 } else { hr.y0 + 17.0 };
                let name_w = hc.auto.map_or(hr.width(), |r| r.x0 - 6.0 - hr.x0 - indent);
                p.text_fit(hr.x0 + indent, cy, &t.name, &TextStyle::sans(13.0, c.text_strong).medium(), name_w);
                if !row.compact() {
                    p.text_fit(
                        hr.x0 + indent,
                        hr.y0 + 35.0,
                        &track_label(doc, t),
                        &TextStyle::sans(11.5, c.text_secondary),
                        hr.width() - indent - 12.0,
                    );
                }
                draw_auto_button(p, th, doc, st, &hc, t);
                draw_source_switch(p, th, &hc, session, row.track);
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
                    let name = target.as_ref().map_or("", crate::automation::target_name);
                    p.text(hr.x0 + indent + 10.0, cy, name, &TextStyle::mono(11.0, c.text_secondary), Align::Left);
                    // + only while the track has a lane left to add
                    let more = crate::automation::lane_targets(row.track)
                        .iter()
                        .any(|x| crate::automation::lane_of(doc, x).is_none());
                    let add = hc.lane_add.filter(|_| more);
                    for (r, ic) in [(add, Icon::Plus), (hc.lane_remove, Icon::Close)] {
                        if let Some(r) = r {
                            p.rrect(r, 3.0, c.bg_control);
                            icons::draw(p, ic, r.inset(-5.0), c.text_secondary);
                        }
                    }
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
            }
            if let Some(a) = l.autos.iter().find(|a| (a.rect.y0 - row.y).abs() < 0.5) {
                draw_automation(p, th, doc, st, a);
            }
            p.hline(hr.x0, lane.x1, hr.y1 - 1.0, c.border_subtle);
        }
    }
}

/// The clip launcher beside the timeline: a column per scene, a slot per
/// track row, the same slots the session view shows.
fn draw_launcher(
    p: &mut Painter<'_>,
    th: &Theme,
    doc: &Document,
    st: &ArrangeState,
    tr: &TransportView,
    session: &SessionView,
    l: &ArrangeLayout,
) {
    let Some(lr) = l.launcher else { return };
    let c = &th.c;
    let band = Rect::new(lr.x0, l.ruler.y0, lr.x1, l.ruler.y1);
    let blink = (tr.playhead.0 / (PPQ / 2)) % 2 == 0;
    p.rect(band, c.bg_surface);
    p.hline(band.x0, band.x1, band.y1 - 1.0, c.border_default);
    p.text(band.x0 + 8.0, band.y0 + 9.0, "clip launcher", &TextStyle::mono(10.0, c.text_secondary), Align::Left);
    p.clip(band, |p| {
        for (id, h) in &l.scene_heads {
            let name = doc.scenes.iter().find(|s| s.id == *id).map_or("", |s| s.name.as_str());
            let clips: Vec<(TrackId, ClipId)> =
                doc.slots.iter().filter(|((s, _), _)| s == id).map(|((_, t), c)| (*t, *c)).collect();
            let on = !clips.is_empty() && clips.iter().all(|(t, c)| session.playing.get(t) == Some(c));
            p.rrect(*h, 3.0, if on { c.bg_control } else { c.bg_fill });
            let tri = Rect::new(h.x0 + 6.0, h.center().y - 4.5, h.x0 + 15.0, h.center().y + 4.5);
            icons::draw(p, Icon::Play, tri, if on { c.accent_primary } else { c.text_secondary });
            p.text_fit(h.x0 + 20.0, h.center().y, name, &TextStyle::mono(11.0, c.text_strong), h.width() - 24.0);
        }
        if let Some(a) = l.add_scene {
            p.rrect(a, 3.0, c.bg_surface.alpha(0.7));
            p.rrect_stroke(a.inset(-0.5), 3.0, 1.0, c.border_subtle);
            icons::draw(
                p,
                Icon::Plus,
                Rect::new(a.center().x - 5.0, a.center().y - 5.0, a.center().x + 5.0, a.center().y + 5.0),
                c.text_secondary,
            );
        }
        if let Some(s) = l.stop_all {
            let any = !session.playing.is_empty();
            let sq = Rect::new(s.center().x - 4.0, s.center().y - 4.0, s.center().x + 4.0, s.center().y + 4.0);
            p.rect(sq, if any { c.text_max } else { c.text_secondary.alpha(0.6) });
        }
    });
    let body = Rect::new(lr.x0, lr.y0, lr.x1, lr.y1);
    p.clip(body, |p| {
        p.rect(body, c.bg_base);
        for row in &l.rows {
            if row.y + row.h < lr.y0 || row.y > lr.y1 {
                continue;
            }
            let strip = Rect::new(lr.x0, row.y, lr.x1, row.y + row.h);
            match row.kind {
                RowKind::Track => {
                    let follows = session.following.contains(&row.track);
                    for (i, (scene, _)) in l.scene_heads.iter().enumerate() {
                        let Some(cell) = l.slot_rect(row, i) else { continue };
                        let clip = doc.slots.get(&(*scene, row.track)).and_then(|id| doc.clips.get(id));
                        let slot = crate::session::Slot { track: row.track, clip, follows, blink };
                        crate::session::draw_slot(p, th, session, st.selected_clip, tr, &slot, cell);
                    }
                }
                RowKind::Group => {
                    let kids = crate::session::tracks_of(doc, row.track);
                    for (i, (scene, _)) in l.scene_heads.iter().enumerate() {
                        let Some(cell) = l.slot_rect(row, i) else { continue };
                        let cell = Rect::new(cell.x0, cell.y0, cell.x1, cell.y0 + (row.clip_h - 6.0).min(26.0));
                        p.rrect(cell, 1.0, c.bg_surface.alpha(0.6));
                        let n = kids.len().max(1) as f64;
                        let w = (cell.width() - 26.0) / n;
                        for (k, t) in kids.iter().enumerate() {
                            let x = cell.x0 + 22.0 + k as f64 * w;
                            if let Some(clip) = doc.slots.get(&(*scene, *t)).and_then(|id| doc.clips.get(id)) {
                                let on = session.playing.get(t) == Some(&clip.id);
                                let col = th.track(clip.color.0).alpha(if on { 1.0 } else { 0.55 });
                                p.rect(Rect::new(x, cell.center().y - 2.5, x + w - 2.0, cell.center().y + 2.5), col);
                            }
                        }
                        let tri =
                            Rect::new(cell.x0 + 6.0, cell.center().y - 4.5, cell.x0 + 15.0, cell.center().y + 4.5);
                        icons::draw(p, Icon::Play, tri, c.text_secondary);
                    }
                }
                _ => p.rect(strip, Rgba::new(0, 0, 0, 40)),
            }
            p.hline(strip.x0, strip.x1, strip.y1 - 1.0, c.border_subtle);
        }
    });
    p.vline(lr.x1, l.ruler.y0, lr.y1, c.border_default);
}

/// The two halves of a track's source switch: "clip" and "auto", lit
/// (accent) while that part comes from the launcher, dim while it plays
/// the arrangement.
fn draw_source_switch(p: &mut Painter<'_>, th: &Theme, hc: &HeaderControls, session: &SessionView, t: TrackId) {
    let c = &th.c;
    let (Some(a), Some(b)) = (hc.clips_src, hc.auto_src) else { return };
    // one pill holding both halves; a half lit in the accent plays from the launcher
    let pill = a.union(b);
    p.rrect(pill, 4.0, c.bg_control);
    p.rrect_stroke(pill.inset(-0.5), 4.0, 1.0, c.border_default);
    for (r, icon, on) in
        [(a, Icon::Clip, session.following.contains(&t)), (b, Icon::Envelope, session.auto.contains(&t))]
    {
        if on {
            p.rrect(r, 4.0, c.accent_primary.alpha(0.2));
            p.rrect_stroke(r.inset(-0.5), 4.0, 1.0, c.accent_primary.alpha(0.8));
        }
        icons::draw(p, icon, r.inset(-4.0), if on { c.accent_primary } else { c.text_secondary });
    }
}

/// The automation button: lit while the track's lanes show.
fn draw_auto_button(
    p: &mut Painter<'_>,
    th: &Theme,
    doc: &Document,
    st: &ArrangeState,
    hc: &HeaderControls,
    t: &Track,
) {
    let Some(r) = hc.auto else { return };
    let has = crate::automation::lane_targets(t.id).iter().any(|x| crate::automation::lane_of(doc, x).is_some());
    let on = has && !st.hidden_lanes.contains(&t.id);
    small_button(p, th, r, "", on.then_some(th.c.accent_primary));
    icons::draw(p, Icon::Pulse, r.inset(-5.0), if on { th.c.accent_primary } else { th.c.text_primary });
}

/// Draws a lane: its curve between breakpoints and, outside them, the
/// target's own value (which plays there), dimmed.
fn draw_automation(p: &mut Painter<'_>, th: &Theme, doc: &Document, st: &ArrangeState, a: &AutoBox) {
    use crate::automation::{fmt_value, pos_of, y_of};
    let c = &th.c;
    let r = a.rect;
    let base = doc.base_value(&a.target).map(|v| y_of(r, pos_of(doc, &a.target, v)));
    let lane = a.lane.and_then(|id| doc.lanes.get(&id));
    let points: Vec<&Breakpoint> = lane.map(|l| l.points.iter().collect()).unwrap_or_default();
    let dim = c.text_secondary.alpha(0.7);
    let (Some(first), Some(last)) = (a.points.first(), a.points.last()) else {
        if let Some(y) = base {
            p.polyline(&[Point::new(r.x0, y), Point::new(r.x1, y)], 1.0, dim);
        }
        return;
    };
    if let Some(y) = base {
        p.polyline(&[Point::new(r.x0, y), Point::new(first.x, y)], 1.0, dim);
        p.polyline(&[Point::new(last.x, y), Point::new(r.x1, y)], 1.0, dim);
    }
    let mut line = vec![*first];
    for (i, w) in a.points.windows(2).enumerate() {
        let (q0, q1) = (w[0], w[1]);
        match points.get(i).map(|b| b.curve) {
            Some(Curve::Hold) => line.extend([Point::new(q1.x, q0.y), q1]),
            Some(Curve::Bend(t)) if t != 0.0 => {
                let k = 2f64.powf(3.0 * t as f64);
                line.extend((1..=16).map(|j| {
                    let x = j as f64 / 16.0;
                    Point::new(q0.x + (q1.x - q0.x) * x, q0.y + (q1.y - q0.y) * x.powf(k))
                }));
            }
            _ => line.push(q1),
        }
    }
    p.polyline(&line, 1.25, c.text_strong);
    for (i, q) in a.points.iter().enumerate() {
        let active = st.active_point.is_some_and(|(id, k)| Some(id) == a.lane && k == i);
        let (rad, fill) = if active { (4.0, c.accent_primary) } else { (3.0, c.bg_base) };
        p.circle(*q, rad, fill);
        p.circle_stroke(*q, rad, 1.2, if active { c.accent_primary } else { c.text_strong });
        if active && let Some(b) = points.get(i) {
            let label = fmt_value(&a.target, b.value);
            let y = if q.y - r.y0 > 14.0 { q.y - 10.0 } else { q.y + 10.0 };
            p.text(q.x + 8.0, y, &label, &TextStyle::mono(10.5, c.text_max), Align::Left);
        }
    }
}

fn draw_clip(p: &mut Painter<'_>, th: &Theme, doc: &Document, st: &ArrangeState, l: &ArrangeLayout, cb: &ClipBox) {
    let c = &th.c;
    let Some(clip) = doc.clips.get(&cb.clip) else { return };
    let Some(pl) = doc.placements.get(&cb.placement) else { return };
    let col = th.track(clip.color.0);
    let muted = doc.tracks[&clip.track].mixer.mute;
    let frozen = doc.tracks[&clip.track].frozen.is_some();
    let col = if muted { col.mix(Rgba::hex(0x333333), 0.45) } else { col };
    // a frozen track's clips turn icy: they play a render, not their notes
    let col = if frozen { col.mix(Rgba::hex(0xa9c7d8), 0.45) } else { col };
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
        let mut notes = feed.map(|m| sat_modules::effective_notes(doc, m, clip.id)).unwrap_or_default();
        let reseeds = feed.is_some_and(|m| sat_modules::reseeds(doc, m));
        if !notes.is_empty() {
            let (mut lo, mut hi) = notes.iter().fold((127u8, 0u8), |(a, b), n| (a.min(n.pitch), b.max(n.pitch)));
            // later passes of a reseeding generator reach anywhere in its range
            if reseeds && let Some(m) = feed.and_then(|m| doc.modules.get(&m)) {
                let ps = sat_modules::turing::Params::of(&sat_modules::macros::resolved(doc, m)).pitches();
                lo = lo.min(ps.first().copied().unwrap_or(lo));
                hi = hi.max(ps.last().copied().unwrap_or(hi));
            }
            let span = (hi - lo).max(6) as f64;
            let nh = (body.height() / (span + 1.0)).clamp(1.5, 3.0);
            let mut k = 0;
            while k * len < pl.offset.0 + pl.length.0 {
                // a reseeding generator plays a new phrase each time round
                if reseeds && k > 0 {
                    notes = sat_modules::effective_notes_at(doc, feed.unwrap(), clip.id, k as u64);
                }
                for n in notes.iter() {
                    let ct = k * len + n.start.0;
                    if ct < pl.offset.0 || ct >= pl.offset.0 + pl.length.0 || n.start.0 >= len {
                        continue;
                    }
                    let t0 = pl.start + Ticks(ct - pl.offset.0);
                    let x0 = l.x_of(t0);
                    let x1 = l.x_of(t0 + n.len).max(x0 + 2.0);
                    let y = body.y1 - nh - (n.pitch.clamp(lo, hi) - lo) as f64 / span * (body.height() - nh);
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
    draw_selection(p, th, st, cb);
}

/// Outlines the selected placement, with grips at its trim edges; other
/// placements of the selected clip (which play the same clip) get a faint outline.
fn draw_selection(p: &mut Painter<'_>, th: &Theme, st: &ArrangeState, cb: &ClipBox) {
    let c = &th.c;
    let r = cb.rect;
    if st.selected_placement == Some(cb.placement) {
        p.rrect_stroke(r.inset(1.0), 4.0, 2.0, c.accent_primary);
        if r.width() > 24.0 {
            let cy = r.center().y + 4.0;
            for x in [r.x0 + 3.0, r.x1 - 5.0] {
                p.rrect(Rect::new(x, cy - 7.0, x + 2.0, cy + 7.0), 1.0, c.accent_primary);
            }
        }
    } else if st.selected_clip == Some(cb.clip) {
        let w = if st.selected_placement.is_none() { 2.0 } else { 1.0 };
        let a = if st.selected_placement.is_none() { 1.0 } else { 0.45 };
        p.rrect_stroke(r.inset(1.0), 4.0, w, c.accent_primary.alpha(a));
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
        let y = row.y + row.clip_h + i as f64 * TAKE_H;
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
    draw_selection(p, th, st, cb);
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
