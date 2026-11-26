//! The bottom panel: clip tabs, the piano roll and the note inspector.

use sat_doc::sat_time::{PPQ, Ticks};
use sat_doc::*;
use sat_theme::{Rgba, Theme};
use vello::kurbo::{Point, Rect};

use crate::arrange::TransportView;
use crate::icons::{self, Icon};
use crate::paint::{Align, Painter, TextStyle};
use crate::panel::{self, PanelView, tab};

pub use crate::panel::TABS_H;
/// Height of the editor toolbar.
pub const TOOLS_H: f64 = 30.0;
/// Width of the keyboard.
pub const KEYS_W: f64 = 64.0;
/// Width of the inspector on the right.
pub const INSPECTOR_W: f64 = 296.0;
/// Height of the beat ruler.
pub const BEATS_H: f64 = 20.0;
/// Height of the velocity lane.
pub const VEL_H: f64 = 60.0;

fn is_black(p: u8) -> bool {
    matches!(p % 12, 1 | 3 | 6 | 8 | 10)
}

fn note_name(p: u8) -> String {
    const N: [&str; 12] = ["c", "c#", "d", "d#", "e", "f", "f#", "g", "g#", "a", "a#", "b"];
    format!("{}{}", N[(p % 12) as usize], p as i32 / 12 - 1)
}

/// Smallest and largest row height (vertical zoom), in px.
pub const ROW_H_RANGE: (f64, f64) = (6.0, 32.0);
/// Default row height.
pub const ROW_H: f64 = 12.0;
/// Most px per beat (horizontal zoom in).
pub const MAX_PX_PER_BEAT: f64 = 960.0;

/// Is `pitch` in `key`?
pub fn in_key(pitch: u8, key: Key) -> bool {
    sat_dsp::scale::contains(pitch as i32, key.root as i32, key.scale as usize)
}

/// How the toolbar names a key, like "c major".
pub fn key_label(key: Key) -> String {
    let root = sat_dsp::scale::ROOTS[key.root as usize % 12];
    let name = sat_dsp::scale::NAMES[(key.scale as usize).min(sat_dsp::scale::NAMES.len() - 1)];
    format!("{root} {name}")
}

/// `pitch` moved `steps` degrees along `key` (snapped into it first);
/// chromatic keys move by semitones.
pub fn key_step(pitch: u8, steps: i32, key: Key) -> u8 {
    sat_dsp::scale::step(pitch as i32, steps, key.root as i32, key.scale as usize).clamp(0, 127) as u8
}

/// `pitch` snapped to the nearest note of `key`.
pub fn key_snap(pitch: u8, key: Key) -> u8 {
    sat_dsp::scale::quantize(pitch as f32, key.root as i32, key.scale as usize).clamp(0.0, 127.0) as u8
}

/// Geometry of the roll for one clip.
#[derive(Debug, Clone, PartialEq)]
pub struct RollLayout {
    /// Keyboard column.
    pub keys: Rect,
    /// Note grid.
    pub grid: Rect,
    /// Velocity lane.
    pub vel: Rect,
    /// Lowest visible pitch.
    pub lo: u8,
    /// Highest visible pitch.
    pub hi: u8,
    /// Clip length.
    pub length: Ticks,
    /// Clip-relative tick at the grid's left edge.
    pub origin: Ticks,
    /// Horizontal zoom: px per tick.
    pub px_per_tick: f64,
    /// The project key the rows are shaded for.
    pub key: Key,
    /// Only the key's rows are shown.
    pub fold: bool,
    /// Every pitch a row can show, lowest first (all 128, or the key's when folded).
    pub all: Vec<u8>,
    /// The visible rows' pitches, lowest first.
    pub rows: Vec<u8>,
    row_h: f64,
}

impl RollLayout {
    /// Lays out the roll inside `r` for `clip`: `lo` is the lowest visible
    /// pitch (`None` centres the clip's notes), `row_h` the row height,
    /// `zoom` px per beat (`None` fits the clip) with `x0` the tick at the
    /// left edge.
    #[allow(clippy::too_many_arguments)]
    pub fn with(
        r: Rect,
        clip: &Clip,
        notes: &[Note],
        lo: Option<u8>,
        row_h: f64,
        zoom: Option<f64>,
        x0: Ticks,
        key: Key,
        fold: bool,
    ) -> RollLayout {
        let keys = Rect::new(r.x0, r.y0 + BEATS_H, r.x0 + KEYS_W, r.y1 - VEL_H);
        let grid = Rect::new(r.x0 + KEYS_W, r.y0 + BEATS_H, r.x1, r.y1 - VEL_H);
        let vel = Rect::new(r.x0 + KEYS_W, r.y1 - VEL_H, r.x1, r.y1);
        let fold = fold && key.scale != 0;
        let all: Vec<u8> = (0..=127u8).filter(|p| !fold || in_key(*p, key)).collect();
        let row_h = row_h.clamp(ROW_H_RANGE.0, ROW_H_RANGE.1);
        let fit = ((grid.height() / row_h).floor().max(4.0) as usize).min(all.len());
        let first = match lo {
            Some(lo) => all.partition_point(|&q| q < lo),
            None => {
                let (a, b) = notes.iter().fold((127u8, 0u8), |(a, b), n| (a.min(n.pitch), b.max(n.pitch)));
                let centre = if notes.is_empty() { 60 } else { ((a as u16 + b as u16) / 2) as u8 };
                all.partition_point(|&q| q < centre).saturating_sub(fit / 2)
            }
        }
        .min(all.len() - fit);
        let rows = all[first..first + fit].to_vec();
        // stretch the rows a little so they fill the grid exactly
        let row_h = grid.height() / fit as f64;
        let length = Ticks(clip.length.0.max(1));
        let fit_ppt = grid.width().max(1.0) / length.0 as f64;
        let ppt = zoom.map_or(fit_ppt, |z| (z / PPQ as f64).max(fit_ppt));
        let visible = (grid.width() / ppt) as i64;
        let origin = Ticks(x0.0.clamp(0, (length.0 - visible).max(0)));
        RollLayout {
            keys,
            grid,
            vel,
            lo: rows[0],
            hi: *rows.last().unwrap(),
            length,
            origin,
            px_per_tick: ppt,
            key,
            fold,
            all,
            rows,
            row_h,
        }
    }

    /// Lays out the roll inside `r` for `clip` with `lo` as the lowest visible
    /// pitch, whole clip in view.
    pub fn at(r: Rect, clip: &Clip, lo: u8) -> RollLayout {
        RollLayout::with(r, clip, &[], Some(lo), ROW_H, None, Ticks::ZERO, Key { root: 0, scale: 0 }, false)
    }

    /// Lays out the roll inside `r` for `clip`, centred on its notes.
    pub fn new(r: Rect, clip: &Clip, notes: &[Note]) -> RollLayout {
        RollLayout::with(r, clip, notes, None, ROW_H, None, Ticks::ZERO, Key { root: 0, scale: 0 }, false)
    }

    /// Row height.
    pub fn row_h(&self) -> f64 {
        self.row_h
    }

    /// Whether the whole clip is in view horizontally.
    pub fn fits(&self) -> bool {
        self.px_per_tick <= self.grid.width().max(1.0) / self.length.0 as f64 + 1e-9
    }

    /// Px per beat.
    pub fn px_per_beat(&self) -> f64 {
        self.px_per_tick * PPQ as f64
    }

    /// Top y of a pitch row. A pitch folded out of view sits on the row of
    /// the key note above it.
    pub fn y_of(&self, pitch: u8) -> f64 {
        let i = if pitch < self.lo {
            -((self.lo - pitch) as f64)
        } else if pitch > self.hi {
            (self.rows.len() + (pitch - self.hi) as usize - 1) as f64
        } else {
            self.rows.partition_point(|&q| q < pitch) as f64
        };
        self.grid.y1 - (i + 1.0) * self.row_h
    }

    /// x of a clip-relative tick.
    pub fn x_of(&self, t: Ticks) -> f64 {
        self.grid.x0 + (t.0 - self.origin.0) as f64 * self.px_per_tick
    }

    /// Clip-relative tick at screen x (not clamped).
    pub fn tick_at(&self, x: f64) -> Ticks {
        Ticks(self.origin.0 + ((x - self.grid.x0) / self.px_per_tick).floor() as i64)
    }

    /// The last clip-relative tick in view.
    pub fn end(&self) -> Ticks {
        Ticks((self.origin.0 + (self.grid.width() / self.px_per_tick).ceil() as i64).min(self.length.0))
    }

    /// Pitch of the row at screen y, clamped to the visible rows.
    pub fn pitch_at(&self, y: f64) -> u8 {
        let k = ((self.grid.y1 - y) / self.row_h).floor() as i64;
        self.rows[k.clamp(0, self.rows.len() as i64 - 1) as usize]
    }

    /// The row index of `pitch` among every row the roll can show.
    pub fn index_of(&self, pitch: u8) -> usize {
        self.all.partition_point(|&q| q < pitch).min(self.all.len() - 1)
    }

    /// The lowest visible pitch after scrolling `rows` rows up (down if negative).
    pub fn scrolled(&self, rows: i32) -> u8 {
        let max = self.all.len() - self.rows.len();
        let i = (self.index_of(self.lo) as i32 + rows).clamp(0, max as i32) as usize;
        self.all[i]
    }

    /// Px per beat and left edge after zooming by `k` around screen x `x`,
    /// or `None` once the whole clip fits again.
    pub fn zoomed(&self, k: f64, x: f64) -> Option<(f64, Ticks)> {
        let fit = self.grid.width().max(1.0) / self.length.0 as f64;
        let ppt = (self.px_per_tick * k).clamp(fit, MAX_PX_PER_BEAT / PPQ as f64);
        if ppt <= fit * 1.0001 {
            return None;
        }
        let at = self.origin.0 as f64 + (x - self.grid.x0) / self.px_per_tick;
        let origin = (at - (x - self.grid.x0) / ppt).round() as i64;
        Some((ppt * PPQ as f64, Ticks(origin.max(0))))
    }

    /// Screen y of a velocity's dot in the velocity lane.
    pub fn vel_y(&self, v: f32) -> f64 {
        self.vel.y1 - 6.0 - (self.vel.height() - 12.0) * v.clamp(0.0, 1.0) as f64
    }

    /// Velocity at a screen y in the velocity lane.
    pub fn vel_at(&self, y: f64) -> f32 {
        (((self.vel.y1 - 6.0 - y) / (self.vel.height() - 12.0).max(1.0)) as f32).clamp(0.0, 1.0)
    }

    /// A note's rectangle.
    pub fn note_rect(&self, n: &Note) -> Rect {
        let y = self.y_of(n.pitch);
        Rect::new(self.x_of(n.start) + 0.5, y + 1.0, self.x_of(n.start + n.len) - 0.5, y + self.row_h - 1.0)
    }
}

/// The piano roll's editing tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tool {
    /// Click empty space to draw a note; drag notes to move or resize.
    #[default]
    Draw,
    /// Drag empty space to select a box of notes; drag notes to move or resize.
    Select,
}

/// Grid choices, finest first; `None` is off.
pub const SNAPS: [Option<Ticks>; 5] =
    [Some(Ticks(PPQ / 8)), Some(Ticks(PPQ / 4)), Some(Ticks(PPQ / 2)), Some(Ticks(PPQ)), None];

/// The label for a grid choice.
pub fn snap_label(snap: Option<Ticks>) -> String {
    match snap {
        Some(t) if t.0 > 0 => format!("snap 1/{}", PPQ * 4 / t.0),
        _ => "snap off".into(),
    }
}

/// The next grid choice after `snap`, wrapping.
pub fn next_snap(snap: Option<Ticks>) -> Option<Ticks> {
    let i = SNAPS.iter().position(|s| *s == snap).unwrap_or(0);
    SNAPS[(i + 1) % SNAPS.len()]
}

/// What the piano roll shows besides the document: selection, tool, grid
/// and vertical position.
#[derive(Debug, Clone, PartialEq)]
pub struct RollView {
    /// Selected notes, as indices into the clip's pattern.
    pub selected: Vec<usize>,
    /// Editing tool.
    pub tool: Tool,
    /// Grid.
    pub snap: Option<Ticks>,
    /// Lowest visible pitch; `None` fits the clip's notes.
    pub lo: Option<u8>,
    /// A box selection being dragged.
    pub rubber: Option<Rect>,
    /// Row height (vertical zoom), in px.
    pub row_h: f64,
    /// Horizontal zoom in px per beat; `None` fits the clip.
    pub zoom: Option<f64>,
    /// Clip-relative tick at the left edge when zoomed in.
    pub x0: Ticks,
    /// Show only the key's rows.
    pub fold: bool,
    /// Drawn and moved notes stay in the key.
    pub in_key: bool,
    /// The key chooser is open.
    pub key_menu: bool,
    /// What the lane under the notes edits.
    pub lane: RollLane,
}

/// What the lane under the piano roll's notes edits: note velocities, or
/// one of the clip's envelopes (which play when the clip is launched).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RollLane {
    /// Note velocities.
    #[default]
    Velocity,
    /// The clip's gain envelope.
    Gain,
    /// The clip's pan envelope.
    Pan,
}

impl RollLane {
    /// Every lane, in chooser order.
    pub const ALL: [RollLane; 3] = [RollLane::Velocity, RollLane::Gain, RollLane::Pan];

    /// Its label in the chooser.
    pub fn label(self) -> &'static str {
        match self {
            RollLane::Velocity => "vel",
            RollLane::Gain => "gain",
            RollLane::Pan => "pan",
        }
    }

    /// The envelope target on `track`, for an envelope lane.
    pub fn target(self, track: TrackId) -> Option<ParamTarget> {
        match self {
            RollLane::Velocity => None,
            RollLane::Gain => Some(ParamTarget::Gain(track)),
            RollLane::Pan => Some(ParamTarget::Pan(track)),
        }
    }
}

impl Default for RollView {
    fn default() -> Self {
        RollView {
            selected: Vec::new(),
            tool: Tool::Draw,
            snap: Some(Ticks(PPQ / 4)),
            lo: None,
            rubber: None,
            row_h: ROW_H,
            zoom: None,
            x0: Ticks::ZERO,
            fold: false,
            in_key: false,
            key_menu: false,
            lane: RollLane::Velocity,
        }
    }
}

/// What is under a point in the clip panel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RollHit {
    /// A note; `edge` is its right edge (resize).
    Note {
        /// Index into the pattern.
        index: usize,
        /// On the resize edge.
        edge: bool,
    },
    /// Empty grid at a tick and pitch.
    Grid {
        /// Clip-relative tick.
        tick: Ticks,
        /// Pitch.
        pitch: u8,
    },
    /// The velocity lane: the note whose stem is nearest (if any is close), and the value at the point.
    Velocity {
        /// Nearest note.
        index: Option<usize>,
        /// Velocity at the point.
        value: f32,
    },
    /// A key on the keyboard.
    Key(u8),
    /// A tool button.
    Tool(Tool),
    /// The grid (snap) button.
    Snap,
    /// The quantise button.
    Quantise,
    /// A lane in the chooser left of the lane under the notes.
    Lane(RollLane),
    /// The envelope lane: the point under the pointer (if any is close),
    /// the clip-relative tick and the height (0–1) at the point.
    Envelope {
        /// The point under the pointer.
        index: Option<usize>,
        /// Clip-relative tick.
        tick: Ticks,
        /// Height in the lane, 0 (bottom) to 1 (top).
        pos: f64,
    },
    /// The key button: open or close the key chooser.
    KeyButton,
    /// A root in the key chooser (0–11).
    Root(u8),
    /// A scale in the key chooser.
    Scale(u8),
    /// The fold button.
    Fold,
    /// The "in key" button.
    InKey,
    /// Zoom out, or in.
    Zoom(bool),
    /// Fit the clip in view.
    ZoomFit,
    /// Nothing editable.
    Nothing,
}

/// The clip panel's geometry for one clip.
#[derive(Debug, Clone)]
pub struct RollGeom {
    /// The clip.
    pub clip: ClipId,
    /// The source module whose pattern is shown.
    pub module: Option<ModuleId>,
    /// The roll.
    pub layout: RollLayout,
    /// Roll, keyboard and velocity lane together.
    pub area: Rect,
    /// Tool buttons.
    pub tools: [(Tool, Rect); 2],
    /// The grid button.
    pub snap: Rect,
    /// The quantise button.
    pub quantise: Rect,
    /// The key button.
    pub key: Rect,
    /// The fold button.
    pub fold: Rect,
    /// The "in key" button.
    pub in_key: Rect,
    /// Zoom out, zoom in, fit.
    pub zoom: [Rect; 3],
    /// The key chooser, when open: its panel, its roots and its scales.
    pub key_menu: Option<KeyMenu>,
    /// The lane chooser, left of the lane under the notes.
    pub lanes: [(RollLane, Rect); 3],
    /// The envelope the lane edits, with its points on screen.
    pub envelope: Option<(ParamTarget, Vec<Point>)>,
}

/// The key chooser under the key button.
#[derive(Debug, Clone, PartialEq)]
pub struct KeyMenu {
    /// The whole panel.
    pub rect: Rect,
    /// A chip per root, c to b.
    pub roots: Vec<Rect>,
    /// A chip per scale, in `sat_dsp::scale::NAMES` order.
    pub scales: Vec<Rect>,
}

impl KeyMenu {
    fn new(below: Rect) -> KeyMenu {
        let (cw, ch, gap) = (30.0, 22.0, 4.0);
        let x0 = below.x0;
        let y0 = below.y1 + 6.0;
        let roots: Vec<Rect> = (0..12)
            .map(|i| {
                let x = x0 + 10.0 + i as f64 * (cw + gap);
                Rect::new(x, y0 + 10.0, x + cw, y0 + 10.0 + ch)
            })
            .collect();
        let sw = (12.0 * (cw + gap) - gap - 3.0 * gap) / 4.0;
        let scales: Vec<Rect> = (0..sat_dsp::scale::NAMES.len())
            .map(|i| {
                let (col, row) = ((i % 4) as f64, (i / 4) as f64);
                let x = x0 + 10.0 + col * (sw + gap);
                let y = y0 + 10.0 + ch + 10.0 + row * (ch + gap);
                Rect::new(x, y, x + sw, y + ch)
            })
            .collect();
        let bottom = scales.last().map_or(y0 + 60.0, |r| r.y1) + 10.0;
        let rect = Rect::new(x0, y0, x0 + 20.0 + 12.0 * (cw + gap) - gap, bottom);
        KeyMenu { rect, roots, scales }
    }
}

/// The notes the clip panel edits for `clip`: the pattern of the module that
/// feeds it, or of the head of the note-processor chain that does.
pub fn clip_notes(doc: &Document, clip: ClipId) -> (Option<ModuleId>, Vec<Note>) {
    let module = sat_modules::editing_source(doc, clip);
    let notes =
        module.and_then(|m| doc.pattern(m, clip)).map(|pt| pt.notes.iter().copied().collect()).unwrap_or_default();
    (module, notes)
}

/// Lays out the clip panel at `r` for the selected clip.
pub fn geometry(doc: &Document, clip: Option<ClipId>, r: Rect, view: &RollView) -> Option<RollGeom> {
    let clip = doc.clips.get(&clip?)?;
    let (module, notes) = clip_notes(doc, clip.id);
    let tools_y0 = r.y0 + TABS_H;
    let ty = tools_y0 + TOOLS_H / 2.0;
    let tg = Rect::new(r.x0 + 12.0, ty - 11.0, r.x0 + 140.0, ty + 11.0);
    let tool_rect = |i: f64| Rect::new(tg.x0 + 2.0 + i * 32.0, tg.y0 + 2.0, tg.x0 + 30.0 + i * 32.0, tg.y1 - 2.0);
    let roll_right = r.x1 - INSPECTOR_W;
    let area = Rect::new(r.x0, tools_y0 + TOOLS_H, roll_right, r.y1);
    let layout = RollLayout::with(area, clip, &notes, view.lo, view.row_h, view.zoom, view.x0, doc.key, view.fold);
    let key = Rect::new(tg.x1 + 12.0, ty - 11.0, tg.x1 + 124.0, ty + 11.0);
    let fold = Rect::new(key.x1 + 4.0, key.y0, key.x1 + 52.0, key.y1);
    let in_key = Rect::new(fold.x1 + 4.0, key.y0, fold.x1 + 64.0, key.y1);
    let zx = roll_right - 300.0 - 12.0;
    let zoom = [
        Rect::new(zx - 104.0, ty - 11.0, zx - 80.0, ty + 11.0),
        Rect::new(zx - 78.0, ty - 11.0, zx - 54.0, ty + 11.0),
        Rect::new(zx - 52.0, ty - 11.0, zx, ty + 11.0),
    ];
    let lanes = lane_chooser(&layout, area);
    let envelope = view.lane.target(clip.track).map(|t| {
        let pts = clip.envelopes.get(&t).map_or_else(Vec::new, |ps| {
            ps.iter()
                .map(|b| Point::new(layout.x_of(b.tick), env_y(&layout, crate::automation::pos_of(doc, &t, b.value))))
                .collect()
        });
        (t, pts)
    });
    Some(RollGeom {
        clip: clip.id,
        module,
        layout,
        area,
        tools: [(Tool::Draw, tool_rect(0.0)), (Tool::Select, tool_rect(1.0))],
        snap: Rect::new(roll_right - 300.0, ty - 11.0, roll_right - 196.0, ty + 11.0),
        quantise: Rect::new(roll_right - 96.0, ty - 11.0, roll_right - 12.0, ty + 11.0),
        key,
        fold,
        in_key,
        zoom,
        key_menu: view.key_menu.then(|| KeyMenu::new(key)),
        lanes,
        envelope,
    })
}

/// The lane chooser's buttons, stacked in the keyboard column beside the
/// lane under the notes.
fn lane_chooser(l: &RollLayout, area: Rect) -> [(RollLane, Rect); 3] {
    let h = (l.vel.height() - 6.0) / 3.0;
    RollLane::ALL.map(|k| {
        let i = RollLane::ALL.iter().position(|x| *x == k).unwrap() as f64;
        let y = l.vel.y0 + 3.0 + i * h;
        (k, Rect::new(area.x0 + 6.0, y + 1.0, l.vel.x0 - 6.0, y + h - 1.0))
    })
}

/// Screen y of height `pos` (0–1) in the envelope lane.
pub fn env_y(l: &RollLayout, pos: f64) -> f64 {
    crate::automation::y_of(l.vel, pos)
}

/// Height (0–1) at screen y in the envelope lane.
pub fn env_pos(l: &RollLayout, y: f64) -> f64 {
    crate::automation::pos_at(l.vel, y)
}

impl RollGeom {
    /// What is under `pt`, given the clip's `notes`.
    pub fn hit(&self, notes: &[Note], pt: Point) -> RollHit {
        if let Some(m) = &self.key_menu {
            if let Some(i) = m.roots.iter().position(|r| r.contains(pt)) {
                return RollHit::Root(i as u8);
            }
            if let Some(i) = m.scales.iter().position(|r| r.contains(pt)) {
                return RollHit::Scale(i as u8);
            }
            if m.rect.contains(pt) {
                return RollHit::Nothing;
            }
        }
        if self.key.contains(pt) {
            return RollHit::KeyButton;
        }
        if self.fold.contains(pt) {
            return RollHit::Fold;
        }
        if self.in_key.contains(pt) {
            return RollHit::InKey;
        }
        if let Some((k, _)) = self.lanes.iter().find(|(_, r)| r.contains(pt)) {
            return RollHit::Lane(*k);
        }
        if let Some(i) = self.zoom.iter().position(|r| r.contains(pt)) {
            return match i {
                0 => RollHit::Zoom(false),
                1 => RollHit::Zoom(true),
                _ => RollHit::ZoomFit,
            };
        }
        if let Some((t, _)) = self.tools.iter().find(|(_, r)| r.contains(pt)) {
            return RollHit::Tool(*t);
        }
        if self.snap.contains(pt) {
            return RollHit::Snap;
        }
        if self.quantise.contains(pt) {
            return RollHit::Quantise;
        }
        let l = &self.layout;
        if l.grid.contains(pt) {
            // topmost (last drawn) note wins
            if let Some(i) = (0..notes.len()).rev().find(|&i| l.note_rect(&notes[i]).inflate(0.0, 0.5).contains(pt)) {
                let nr = l.note_rect(&notes[i]);
                let grip = (nr.width() * 0.3).clamp(2.0, 6.0);
                return RollHit::Note { index: i, edge: pt.x >= nr.x1 - grip };
            }
            return RollHit::Grid { tick: l.tick_at(pt.x), pitch: l.pitch_at(pt.y) };
        }
        if let (true, Some((_, pts))) = (l.vel.contains(pt), &self.envelope) {
            let index = pts
                .iter()
                .enumerate()
                .map(|(i, q)| (i, q.distance(pt)))
                .filter(|(_, d)| *d <= 6.0)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(i, _)| i);
            return RollHit::Envelope { index, tick: l.tick_at(pt.x), pos: env_pos(l, pt.y) };
        }
        if l.vel.contains(pt) {
            let near = notes
                .iter()
                .enumerate()
                .map(|(i, n)| (i, (l.x_of(n.start) + 1.0 - pt.x).abs()))
                .filter(|(_, d)| *d <= 5.0)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(i, _)| i);
            return RollHit::Velocity { index: near, value: l.vel_at(pt.y) };
        }
        if l.keys.contains(pt) {
            return RollHit::Key(l.pitch_at(pt.y));
        }
        RollHit::Nothing
    }
}

/// Draws a clip envelope in the lane under the notes: the track's own
/// value as a dim line, then the curve through the points (held flat
/// before the first and after the last).
fn draw_envelope(
    p: &mut Painter<'_>,
    th: &Theme,
    doc: &Document,
    rl: &RollLayout,
    target: &ParamTarget,
    pts: &[Point],
    col: Rgba,
) {
    let c = &th.c;
    let r = rl.vel;
    if let Some(base) = doc.base_value(target) {
        let y = env_y(rl, crate::automation::pos_of(doc, target, base));
        p.hline(r.x0, r.x1, y, c.text_secondary.alpha(0.35));
    }
    if pts.is_empty() {
        let hint = match target {
            ParamTarget::Pan(_) => "click to add pan points that play when the clip is launched",
            _ => "click to add gain points that play when the clip is launched",
        };
        p.text(r.x0 + 10.0, r.center().y, hint, &TextStyle::mono(10.0, c.text_secondary), Align::Left);
        return;
    }
    p.clip(r, |p| {
        let mut line = vec![Point::new(r.x0, pts[0].y)];
        line.extend_from_slice(pts);
        line.push(Point::new(r.x1, pts[pts.len() - 1].y));
        p.polyline(&line, 1.5, col);
        for q in pts {
            p.circle(*q, 3.0, col);
            p.circle(*q, 1.5, Rgba::hex(0x0e0e0e));
        }
    });
}

fn button(p: &mut Painter<'_>, th: &Theme, r: Rect, label: &str) {
    crate::area::button(p, th, r, label, false);
}

/// Draws the clip panel's tab bar with the clip's name, track, bars and
/// source, and returns the clip (or `None`, saying so, when none is selected).
pub fn clip_bar<'d>(
    p: &mut Painter<'_>,
    th: &Theme,
    doc: &'d Document,
    clip: Option<ClipId>,
    r: Rect,
) -> Option<&'d Clip> {
    let c = &th.c;
    let (tabs, mut x) = panel::draw_bar(p, th, r, PanelView::Clip);
    let Some(clip) = clip.and_then(|id| doc.clips.get(&id)) else {
        p.text(x, tabs.center().y, "no clip selected", &TextStyle::mono(12.0, c.text_secondary), Align::Left);
        return None;
    };
    let col = th.track(clip.color.0);
    let cy = tabs.center().y;
    p.rrect(Rect::new(x, cy - 5.0, x + 10.0, cy + 5.0), 2.0, col);
    x += 18.0;
    x += p.text(x, cy, &clip.name, &TextStyle::mono(12.0, c.text_max).medium(), Align::Left) + 10.0;
    let track = &doc.tracks[&clip.track];
    let bars = doc
        .placements
        .values()
        .filter(|pl| pl.clip == clip.id)
        .map(|pl| (pl.start.0 / (PPQ * 4) + 1, (pl.start.0 + pl.length.0) / (PPQ * 4)))
        .min();
    let info = match bars {
        Some((a, b)) => format!("{} · bars {a}–{b}", track.name),
        None => track.name.clone(),
    };
    x += p.text(x, cy, &info, &TextStyle::mono(12.0, c.text_secondary), Align::Left) + 24.0;
    let feed = sat_modules::editing_source(doc, clip.id).and_then(|m| doc.modules.get(&m));
    if let Some(m) = feed {
        x += tab(p, th, x, tabs, Some(Icon::Pencil), &m.name, true);
    }
    tab(p, th, x, tabs, Some(Icon::ChevronRight), "output", false);
    button(p, th, open_grid_button(r), "open source grid");

    Some(clip)
}

/// The clip bar's "open source grid" button, for the bottom panel at `r`
/// (left of the panel's own maximize and collapse buttons).
pub fn open_grid_button(r: Rect) -> Rect {
    let cy = r.y0 + TABS_H / 2.0;
    Rect::new(r.x1 - 204.0, cy - 11.0, r.x1 - 76.0, cy + 11.0)
}

/// Where the playhead is inside `clip`, if it is inside a placement of it.
pub fn clip_playhead(doc: &Document, clip: &Clip, tr: &TransportView) -> Option<Ticks> {
    let pl = doc
        .placements
        .values()
        .find(|pl| pl.clip == clip.id && tr.playhead >= pl.start && tr.playhead < pl.start + pl.length)?;
    Some(Ticks((tr.playhead.0 - pl.start.0 + pl.offset.0).rem_euclid(clip.length.0)))
}

/// Draws the bottom panel for the selected clip and returns its geometry.
pub fn draw(
    p: &mut Painter<'_>,
    th: &Theme,
    doc: &Document,
    clip: Option<ClipId>,
    view: &RollView,
    tr: &TransportView,
    r: Rect,
) -> Option<RollGeom> {
    let c = &th.c;
    let (Some(geom), Some(clip)) = (geometry(doc, clip, r, view), clip_bar(p, th, doc, clip, r)) else { return None };
    // toolbar
    let tools = Rect::new(r.x0, r.y0 + TABS_H, r.x1, r.y0 + TABS_H + TOOLS_H);
    let ty = tools.center().y;
    let tg = Rect::new(r.x0 + 12.0, ty - 11.0, r.x0 + 140.0, ty + 11.0);
    p.rrect_stroke(tg.inset(-0.5), 5.0, 1.0, c.border_default);
    for (i, ic) in [Icon::Pencil, Icon::Pointer, Icon::Plus, Icon::ChevronDown].iter().enumerate() {
        let br = Rect::new(tg.x0 + 2.0 + i as f64 * 32.0, tg.y0 + 2.0, tg.x0 + 30.0 + i as f64 * 32.0, tg.y1 - 2.0);
        let on = geom.tools.get(i).is_some_and(|(t, _)| *t == view.tool);
        if on {
            p.rrect(br, 4.0, c.accent_primary_subtle);
        }
        icons::draw(p, *ic, br.inset(-7.0), if on { c.accent_primary } else { c.text_primary });
    }
    let roll_right = r.x1 - INSPECTOR_W;
    button(p, th, geom.quantise, "quantise");
    button(p, th, geom.snap, &snap_label(view.snap));
    crate::area::button(p, th, geom.key, &key_label(doc.key), view.key_menu);
    crate::area::button(p, th, geom.fold, "fold", view.fold && doc.key.scale != 0);
    crate::area::button(p, th, geom.in_key, "in key", view.in_key);
    for (r, label) in geom.zoom.iter().zip(["−", "+", "fit"]) {
        button(p, th, *r, label);
    }

    // roll
    let (_, notes) = clip_notes(doc, clip.id);
    let area = geom.area;
    let rl = &geom.layout;
    let key = rl.key;
    let shade = key.scale != 0;
    p.rect(Rect::new(rl.grid.x0, area.y0, rl.grid.x1, rl.vel.y1), Rgba::hex(0x0c0c0c));
    p.clip(Rect::new(area.x0, area.y0, area.x1, area.y1), |p| {
        let rh = rl.row_h();
        let label_all = rl.fold && rh >= 11.0;
        for &pitch in &rl.rows {
            let y = rl.y_of(pitch);
            let row = Rect::new(rl.grid.x0, y, rl.grid.x1, y + rh);
            let pc = pitch % 12;
            let root = shade && pc == key.root;
            if shade {
                // key rows are lit, the others dark; the root is tinted
                if root {
                    p.rect(row, c.accent_primary.alpha(0.10));
                } else if in_key(pitch, key) {
                    p.rect(row, Rgba::new(255, 255, 255, 7));
                } else {
                    p.rect(row, Rgba::hex(0x070707));
                }
            } else if is_black(pitch) {
                p.rect(row, Rgba::hex(0x090909));
            }
            let octave_line = if shade { pc == key.root } else { pc == 0 };
            let line = if octave_line { Rgba::new(255, 255, 255, 22) } else { Rgba::new(255, 255, 255, 8) };
            p.hline(rl.grid.x0, rl.grid.x1, y + rh - 1.0, line);
            // keyboard
            let k = Rect::new(rl.keys.x0, y, rl.keys.x1, y + rh);
            if is_black(pitch) {
                p.rect(k, Rgba::hex(0x151515));
                p.rect(Rect::new(k.x0, y, k.x0 + 40.0, y + rh - 1.0), Rgba::hex(0x0a0a0a));
            } else {
                p.rect(k, Rgba::hex(0xdcdcdc));
                p.hline(k.x0, k.x1, y + rh - 1.0, Rgba::hex(0x9e9e9e));
            }
            if shade && in_key(pitch, key) {
                // a mark on the key's notes
                let col = if root { c.accent_primary } else { c.accent_primary.alpha(0.45) };
                p.rect(Rect::new(k.x1 - 4.0, y + 1.0, k.x1 - 1.0, y + rh - 1.0), col);
            }
            if (pc == 0 || root || label_all) && rh >= 7.0 {
                let col = if is_black(pitch) { Rgba::hex(0x8a8a8a) } else { Rgba::hex(0x444444) };
                let col = if root { c.accent_primary.mix(Rgba::hex(0x222222), 0.7) } else { col };
                p.text(k.x1 - 7.0, y + rh / 2.0, &note_name(pitch), &TextStyle::mono(9.5, col), Align::Right);
            }
        }
        // beat ruler and grid lines, as fine as the zoom allows
        let beats = Rect::new(rl.grid.x0, area.y0, rl.grid.x1, area.y0 + BEATS_H);
        p.rect(beats, c.bg_surface);
        let beat_px = rl.px_per_beat();
        let six_px = beat_px / 4.0;
        let bar_px = beat_px * 4.0;
        let label_every = if beat_px >= 36.0 {
            1
        } else {
            // whole bars, spaced so labels do not collide
            let mut n = 4;
            while (n as f64 / 4.0) * bar_px < 34.0 {
                n *= 2;
            }
            n
        };
        let sixteenth = PPQ / 4;
        let (s0, s1) = (rl.origin.0 / sixteenth, rl.end().0 / sixteenth);
        for s in s0..=s1 {
            let x = rl.x_of(Ticks(s * sixteenth));
            let (bar, beat, six) = (s / 16, (s / 4) % 4, s % 4);
            let line = if six != 0 {
                if six_px < 5.0 {
                    continue;
                }
                Rgba::new(255, 255, 255, 5)
            } else if beat != 0 {
                if beat_px < 6.0 {
                    continue;
                }
                Rgba::new(255, 255, 255, 12)
            } else {
                c.border_default
            };
            p.vline(x, rl.grid.y0, rl.vel.y1, line);
            if six == 0 && (beat_px >= 6.0 || beat == 0) {
                p.vline(x, beats.y1 - if beat == 0 { 10.0 } else { 5.0 }, beats.y1, c.border_default);
            }
            let labelled =
                if label_every == 1 { six == 0 } else { six == 0 && beat == 0 && (bar * 4) % label_every == 0 };
            if labelled {
                let text = if label_every == 1 { format!("{}.{}", bar + 1, beat + 1) } else { format!("{}", bar + 1) };
                p.text(x + 4.0, beats.center().y, &text, &TextStyle::mono(11.0, c.text_secondary), Align::Left);
            }
        }
        // where the view is in the clip, when zoomed in
        if !rl.fits() {
            let w = beats.width();
            let len = rl.length.0 as f64;
            let t0 = beats.x0 + rl.origin.0 as f64 / len * w;
            let t1 = beats.x0 + rl.end().0 as f64 / len * w;
            p.rrect(Rect::new(t0, beats.y0 + 1.0, t1.max(t0 + 8.0), beats.y0 + 4.0), 1.5, c.text_secondary.alpha(0.6));
        }
        p.hline(beats.x0, beats.x1, beats.y1 - 1.0, c.border_default);
        // notes; ones outside the key get a warning edge when the key is shaded
        let nc = c.accent_warning;
        for (i, n) in notes.iter().enumerate() {
            if n.pitch < rl.lo || n.pitch > rl.hi {
                continue;
            }
            let nr = rl.note_rect(n);
            if nr.x1 < rl.grid.x0 || nr.x0 > rl.grid.x1 {
                continue;
            }
            let sel = view.selected.contains(&i);
            p.rrect(nr, 1.5, nc.mix(Rgba::hex(0x111111), 0.35 + 0.65 * n.velocity));
            if sel {
                p.rrect_stroke(nr.inset(-0.5), 1.5, 1.5, c.text_max);
            } else if shade && !in_key(n.pitch, key) {
                p.rrect_stroke(nr.inset(-0.5), 1.5, 1.5, c.accent_danger);
            } else {
                p.rrect_stroke(nr.inset(-0.5), 1.5, 1.0, nc.alpha(0.8));
            }
        }
        if let Some(b) = view.rubber {
            p.rect(b, c.accent_primary.alpha(0.12));
            p.rrect_stroke(b, 0.0, 1.0, c.accent_primary.alpha(0.7));
        }
        // the lane under the notes: velocity, or one of the clip's envelopes
        p.rect(rl.vel, Rgba::hex(0x0e0e0e));
        p.hline(rl.vel.x0, rl.vel.x1, rl.vel.y0, c.border_default);
        for (k, r) in &geom.lanes {
            let on = *k == view.lane;
            let has = k.target(clip.track).is_some_and(|t| clip.envelopes.contains_key(&t));
            if on {
                p.rrect(*r, 3.0, c.accent_primary_subtle);
            }
            let col = if on { c.accent_primary } else { c.text_secondary };
            p.text(r.x0 + 6.0, r.center().y, k.label(), &TextStyle::mono(10.0, col), Align::Left);
            if has {
                // a dot marks an envelope the clip carries
                p.circle(Point::new(r.x1 - 6.0, r.center().y), 2.0, c.accent_primary);
            }
        }
        if let Some((t, pts)) = &geom.envelope {
            draw_envelope(p, th, doc, rl, t, pts, nc);
        }
        p.clip(rl.vel, |p| {
            if geom.envelope.is_some() {
                return;
            }
            for (i, n) in notes.iter().enumerate() {
                let x = rl.x_of(n.start) + 1.0;
                let top = rl.vel_y(n.velocity);
                let col = if view.selected.contains(&i) { c.text_max } else { nc };
                p.vline(x, top, rl.vel.y1 - 4.0, col);
                p.circle(Point::new(x + 0.5, top), 2.5, col);
            }
        });
        // playhead, if inside a placement of this clip
        if let Some(rel) = clip_playhead(doc, clip, tr) {
            let x = rl.x_of(rel);
            if x >= rl.grid.x0 {
                p.vline(x, area.y0, rl.vel.y1, c.text_max);
            }
        }
    });
    p.vline(rl.keys.x1, area.y0, area.y1, c.border_default);

    // inspector
    let ins = Rect::new(roll_right, tools.y0, r.x1, r.y1);
    p.rect(ins, c.bg_surface);
    p.vline(ins.x0, ins.y0, ins.y1, c.border_default);
    let mut y = ins.y0 + 18.0;
    let mut tx = ins.x0 + 12.0;
    for (i, t) in ["notes", "clip", "groove"].iter().enumerate() {
        let st = TextStyle::mono(12.0, if i == 0 { c.text_max } else { c.text_secondary });
        let w = p.fonts.measure(t, &st) as f64;
        if i == 0 {
            p.rrect(Rect::new(tx - 6.0, y - 11.0, tx + w + 6.0, y + 11.0), 4.0, c.bg_fill);
        }
        p.text(tx, y, t, &st, Align::Left);
        tx += w + 24.0;
    }
    y += 26.0;
    let (lo, hi) = notes.iter().fold((127u8, 0u8), |(a, b), n| (a.min(n.pitch), b.max(n.pitch)));
    let rows: [(&str, String); 6] = [
        ("notes", notes.len().to_string()),
        ("range", if notes.is_empty() { "–".into() } else { format!("{} · {}", note_name(lo), note_name(hi)) }),
        ("length", format!("{} bars", clip.length.0 as f64 / (PPQ * 4) as f64)),
        ("loop", if clip.looped { "on".into() } else { "off".into() }),
        ("seed", clip.seed.to_string()),
        ("snapshot", format!("{} values", clip.snapshot.len())),
    ];
    for (k, v) in rows {
        let row = Rect::new(ins.x0 + 8.0, y - 11.0, ins.x1 - 8.0, y + 11.0);
        p.rrect(row, 4.0, c.bg_field);
        p.text(row.x0 + 8.0, y, k, &TextStyle::mono(11.5, c.text_secondary), Align::Left);
        p.text(row.x1 - 8.0, y, &v, &TextStyle::mono(11.5, c.text_strong), Align::Right);
        y += 26.0;
    }
    if let Some(m) = &geom.key_menu {
        draw_key_menu(p, th, m, doc.key);
    }
    Some(geom)
}

fn draw_key_menu(p: &mut Painter<'_>, th: &Theme, m: &KeyMenu, key: Key) {
    let c = &th.c;
    p.rrect(m.rect.inflate(1.0, 1.0), 7.0, Rgba::new(0, 0, 0, 120));
    p.rrect(m.rect, 6.0, c.bg_raised);
    p.rrect_stroke(m.rect.inset(-0.5), 6.0, 1.0, c.border_strong);
    for (i, r) in m.roots.iter().enumerate() {
        let on = key.root as usize == i;
        crate::area::button(p, th, *r, sat_dsp::scale::ROOTS[i], on);
    }
    for (i, r) in m.scales.iter().enumerate() {
        let on = key.scale as usize == i;
        crate::area::button(p, th, *r, sat_dsp::scale::NAMES[i], on);
    }
}
