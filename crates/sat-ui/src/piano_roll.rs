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
pub const TOOLS_H: f64 = 36.0;
/// Width of the keyboard.
pub const KEYS_W: f64 = 64.0;
/// Width of the inspector on the right.
pub const INSPECTOR_W: f64 = 296.0;
/// Height of the beat ruler.
pub const BEATS_H: f64 = 20.0;
/// Height of the velocity lane.
pub const VEL_H: f64 = 44.0;

fn is_black(p: u8) -> bool {
    matches!(p % 12, 1 | 3 | 6 | 8 | 10)
}

fn note_name(p: u8) -> String {
    const N: [&str; 12] = ["c", "c#", "d", "d#", "e", "f", "f#", "g", "g#", "a", "a#", "b"];
    format!("{}{}", N[(p % 12) as usize], p as i32 / 12 - 1)
}

/// Geometry of the roll for one clip.
#[derive(Debug, Clone, Copy)]
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
    /// Clip length shown.
    pub length: Ticks,
}

impl RollLayout {
    /// Lays out the roll inside `r` for `clip` with `lo` as the lowest visible
    /// pitch, showing as many rows as fit at >= 9 px each.
    pub fn at(r: Rect, clip: &Clip, lo: u8) -> RollLayout {
        let rows_fit = ((r.height() - BEATS_H - VEL_H) / 9.0).floor().max(12.0) as u8;
        let lo = lo.min(128 - rows_fit);
        let hi = (lo as u16 + rows_fit as u16 - 1).min(127) as u8;
        let keys = Rect::new(r.x0, r.y0 + BEATS_H, r.x0 + KEYS_W, r.y1 - VEL_H);
        let grid = Rect::new(r.x0 + KEYS_W, r.y0 + BEATS_H, r.x1, r.y1 - VEL_H);
        let vel = Rect::new(r.x0 + KEYS_W, r.y1 - VEL_H, r.x1, r.y1);
        RollLayout { keys, grid, vel, lo, hi, length: clip.length }
    }

    /// Lays out the roll inside `r` for `clip`, fitting its notes' range.
    pub fn new(r: Rect, clip: &Clip, notes: &[Note]) -> RollLayout {
        let (mut lo, mut hi) = notes.iter().fold((127u8, 0u8), |(a, b), n| (a.min(n.pitch), b.max(n.pitch)));
        if notes.is_empty() {
            (lo, hi) = (48, 72);
        }
        // pad to whole rows that fill the height at >= 9 px per row
        let rows_fit = ((r.height() - BEATS_H - VEL_H) / 9.0).floor().max(12.0) as u8;
        let span = hi.saturating_sub(lo) + 1;
        if span < rows_fit {
            let extra = rows_fit - span;
            lo = lo.saturating_sub(extra / 2);
            hi = (lo as u16 + rows_fit as u16 - 1).min(127) as u8;
        }
        let keys = Rect::new(r.x0, r.y0 + BEATS_H, r.x0 + KEYS_W, r.y1 - VEL_H);
        let grid = Rect::new(r.x0 + KEYS_W, r.y0 + BEATS_H, r.x1, r.y1 - VEL_H);
        let vel = Rect::new(r.x0 + KEYS_W, r.y1 - VEL_H, r.x1, r.y1);
        RollLayout { keys, grid, vel, lo, hi, length: clip.length }
    }

    /// Row height.
    pub fn row_h(&self) -> f64 {
        self.grid.height() / (self.hi - self.lo + 1) as f64
    }

    /// Top y of a pitch row.
    pub fn y_of(&self, pitch: u8) -> f64 {
        self.grid.y1 - (pitch as f64 - self.lo as f64 + 1.0) * self.row_h()
    }

    /// x of a clip-relative tick.
    pub fn x_of(&self, t: Ticks) -> f64 {
        self.grid.x0 + t.0 as f64 / self.length.0.max(1) as f64 * self.grid.width()
    }

    /// Clip-relative tick at screen x (not clamped).
    pub fn tick_at(&self, x: f64) -> Ticks {
        Ticks(((x - self.grid.x0) / self.grid.width().max(1.0) * self.length.0 as f64).floor() as i64)
    }

    /// Pitch of the row at screen y, clamped to the visible rows.
    pub fn pitch_at(&self, y: f64) -> u8 {
        let k = ((self.grid.y1 - y) / self.row_h()).floor() as i64;
        (self.lo as i64 + k).clamp(self.lo as i64, self.hi as i64) as u8
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
        Rect::new(self.x_of(n.start) + 0.5, y + 1.0, self.x_of(n.start + n.len) - 0.5, y + self.row_h() - 1.0)
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
}

impl Default for RollView {
    fn default() -> Self {
        RollView { selected: Vec::new(), tool: Tool::Draw, snap: Some(Ticks(PPQ / 4)), lo: None, rubber: None }
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
}

/// The notes the clip panel edits for `clip`.
pub fn clip_notes(doc: &Document, clip: ClipId) -> (Option<ModuleId>, Vec<Note>) {
    let module = doc.inlet_cable(clip).map(|k| k.from.module);
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
    let tg = Rect::new(r.x0 + 12.0, ty - 13.0, r.x0 + 140.0, ty + 13.0);
    let tool_rect = |i: f64| Rect::new(tg.x0 + 2.0 + i * 32.0, tg.y0 + 2.0, tg.x0 + 30.0 + i * 32.0, tg.y1 - 2.0);
    let roll_right = r.x1 - INSPECTOR_W;
    let area = Rect::new(r.x0, tools_y0 + TOOLS_H, roll_right, r.y1);
    let layout = match view.lo {
        Some(lo) => RollLayout::at(area, clip, lo),
        None => RollLayout::new(area, clip, &notes),
    };
    Some(RollGeom {
        clip: clip.id,
        module,
        layout,
        area,
        tools: [(Tool::Draw, tool_rect(0.0)), (Tool::Select, tool_rect(1.0))],
        snap: Rect::new(roll_right - 300.0, ty - 13.0, roll_right - 196.0, ty + 13.0),
        quantise: Rect::new(roll_right - 96.0, ty - 13.0, roll_right - 12.0, ty + 13.0),
    })
}

impl RollGeom {
    /// What is under `pt`, given the clip's `notes`.
    pub fn hit(&self, notes: &[Note], pt: Point) -> RollHit {
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

fn button(p: &mut Painter<'_>, th: &Theme, r: Rect, label: &str) {
    p.rrect(r, 5.0, th.c.bg_control);
    p.rrect_stroke(r.inset(-0.5), 5.0, 1.0, th.c.border_strong.alpha(0.6));
    p.text(r.center().x, r.center().y, label, &TextStyle::mono(12.0, th.c.text_strong), Align::Center);
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
    let feed = doc.inlet_cable(clip.id).and_then(|k| doc.modules.get(&k.from.module));
    if let Some(m) = feed {
        x += tab(p, th, x, tabs, Some(Icon::Pencil), &m.name, true);
    }
    tab(p, th, x, tabs, Some(Icon::ChevronRight), "output", false);
    let open = Rect::new(r.x1 - 168.0, cy - 13.0, r.x1 - 40.0, cy + 13.0);
    button(p, th, open, "open source grid");

    Some(clip)
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
    let tg = Rect::new(r.x0 + 12.0, ty - 13.0, r.x0 + 140.0, ty + 13.0);
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

    // roll
    let (_, notes) = clip_notes(doc, clip.id);
    let area = geom.area;
    let rl = geom.layout;
    p.rect(Rect::new(rl.grid.x0, area.y0, rl.grid.x1, rl.vel.y1), Rgba::hex(0x0c0c0c));
    p.clip(Rect::new(area.x0, area.y0, area.x1, area.y1), |p| {
        let rh = rl.row_h();
        for pitch in rl.lo..=rl.hi {
            let y = rl.y_of(pitch);
            let row = Rect::new(rl.grid.x0, y, rl.grid.x1, y + rh);
            if is_black(pitch) {
                p.rect(row, Rgba::hex(0x090909));
            }
            p.hline(rl.grid.x0, rl.grid.x1, y + rh - 1.0, Rgba::new(255, 255, 255, 8));
            // keyboard
            let key = Rect::new(rl.keys.x0, y, rl.keys.x1, y + rh);
            if is_black(pitch) {
                p.rect(key, Rgba::hex(0x151515));
                p.rect(Rect::new(key.x0, y, key.x0 + 40.0, y + rh - 1.0), Rgba::hex(0x0a0a0a));
            } else {
                p.rect(key, Rgba::hex(0xdcdcdc));
                p.hline(key.x0, key.x1, y + rh - 1.0, Rgba::hex(0x9e9e9e));
            }
            if pitch % 12 == 0 {
                p.text(
                    key.x1 - 6.0,
                    y + rh / 2.0,
                    &note_name(pitch),
                    &TextStyle::mono(9.5, Rgba::hex(0x444444)),
                    Align::Right,
                );
            }
        }
        // beat ruler and grid lines
        let beats = Rect::new(rl.grid.x0, area.y0, rl.grid.x1, area.y0 + BEATS_H);
        p.rect(beats, c.bg_surface);
        let sixteenth = PPQ / 4;
        let steps = rl.length.0 / sixteenth;
        for s in 0..=steps {
            let x = rl.x_of(Ticks(s * sixteenth));
            let (bar, beat, six) = (s / 16, (s / 4) % 4, s % 4);
            let line = if six != 0 {
                Rgba::new(255, 255, 255, 5)
            } else if beat != 0 {
                Rgba::new(255, 255, 255, 12)
            } else {
                c.border_default
            };
            p.vline(x, rl.grid.y0, rl.vel.y1, line);
            if six == 0 {
                p.vline(x, beats.y1 - if beat == 0 { 10.0 } else { 5.0 }, beats.y1, c.border_default);
                p.text(
                    x + 4.0,
                    beats.center().y,
                    &format!("{}.{}", bar + 1, beat + 1),
                    &TextStyle::mono(11.0, c.text_secondary),
                    Align::Left,
                );
            }
        }
        p.hline(beats.x0, beats.x1, beats.y1 - 1.0, c.border_default);
        // notes
        let nc = c.accent_warning;
        for (i, n) in notes.iter().enumerate() {
            if n.pitch < rl.lo || n.pitch > rl.hi {
                continue;
            }
            let nr = rl.note_rect(n);
            let sel = view.selected.contains(&i);
            p.rrect(nr, 1.5, nc.mix(Rgba::hex(0x111111), 0.35 + 0.65 * n.velocity));
            if sel {
                p.rrect_stroke(nr.inset(-0.5), 1.5, 1.5, c.text_max);
            } else {
                p.rrect_stroke(nr.inset(-0.5), 1.5, 1.0, nc.alpha(0.8));
            }
        }
        if let Some(b) = view.rubber {
            p.rect(b, c.accent_primary.alpha(0.12));
            p.rrect_stroke(b, 0.0, 1.0, c.accent_primary.alpha(0.7));
        }
        // velocity lane
        p.rect(rl.vel, Rgba::hex(0x0e0e0e));
        p.hline(rl.vel.x0, rl.vel.x1, rl.vel.y0, c.border_default);
        p.text(area.x0 + 10.0, rl.vel.center().y, "vel", &TextStyle::mono(11.0, c.text_secondary), Align::Left);
        for (i, n) in notes.iter().enumerate() {
            let x = rl.x_of(n.start) + 1.0;
            let top = rl.vel_y(n.velocity);
            let col = if view.selected.contains(&i) { c.text_max } else { nc };
            p.vline(x, top, rl.vel.y1 - 4.0, col);
            p.circle(Point::new(x + 0.5, top), 2.5, col);
        }
        // playhead, if inside a placement of this clip
        if let Some(rel) = clip_playhead(doc, clip, tr) {
            p.vline(rl.x_of(rel), area.y0, rl.vel.y1, c.text_max);
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
    Some(geom)
}
