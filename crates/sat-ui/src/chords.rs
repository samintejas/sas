//! The chord builder's editor in the bottom panel: its knobs, the key's
//! chords as pads (triads, sevenths, borrowed), common progressions, and the
//! clip's progression as a lane of chord blocks.
//!
//! A pad plays its chord while it is held; with "write" on it also goes into
//! the progression, replacing the selected chord or added after the last.
//! Keys 1–7 play the triads (with shift the sevenths, with alt the borrowed
//! chords) without writing.

use sat_doc::sat_time::PPQ;
use sat_doc::*;
use sat_modules::chords::{self, Step};
use sat_theme::{Rgba, Theme};
use vello::kurbo::{Point, Rect};

use crate::arrange::TransportView;
use crate::controls;
use crate::paint::{Align, Painter, TextStyle};
use crate::panel::TABS_H;
use crate::piano_roll::{TOOLS_H, clip_bar, clip_playhead};

/// Width of the knob card.
pub const CARD_W: f64 = 330.0;
/// Height of the body when the editor fits its content.
pub const FIT_BODY_H: f64 = 252.0;
/// Width of a pad row's label column.
const ROW_LABEL_W: f64 = 84.0;
/// Height of the progression lane.
const LANE_H: f64 = 46.0;

/// The pad rows' labels.
pub const ROWS: [&str; 3] = ["triads", "sevenths", "borrowed"];

/// What the chord editor shows besides the document.
#[derive(Debug, Clone, PartialEq)]
pub struct ChordView {
    /// The selected chord of the progression.
    pub selected: Option<usize>,
    /// The progression preset on show.
    pub preset: usize,
    /// Pads write into the progression (otherwise they only play).
    pub write: bool,
    /// The pad held down, lit while it plays.
    pub held: Option<Step>,
}

impl Default for ChordView {
    fn default() -> Self {
        ChordView { selected: None, preset: 0, write: true, held: None }
    }
}

/// What is under a point in the chord editor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ChordHit {
    /// A knob, by parameter key.
    Knob(&'static str),
    /// A chord pad.
    Pad(Step),
    /// A chord of the progression.
    Slot(usize),
    /// The lane where there is no chord.
    Lane,
    /// Previous / next progression preset.
    Preset(i32),
    /// Lay the preset out over the clip.
    Apply,
    /// The write switch.
    Write,
    /// Empty the progression.
    Clear,
    /// Drop the clip's chords into a piano roll.
    ToRoll,
    /// Selected chord: inversion up.
    Invert,
    /// Selected chord: an octave up (+1) or down (-1).
    Octave(i8),
    /// Selected chord: a beat longer (+1) or shorter (-1).
    Length(i64),
    /// Remove the selected chord.
    Delete,
    /// Nothing.
    Nothing,
}

/// The chord editor's geometry for one clip.
#[derive(Debug, Clone)]
pub struct ChordGeom {
    /// The clip.
    pub clip: ClipId,
    /// The chord builder.
    pub module: ModuleId,
    /// Knobs, by parameter key.
    pub knobs: Vec<(&'static str, Rect)>,
    /// Pads, row by row.
    pub pads: Vec<(Step, usize, Rect)>,
    /// The progression lane.
    pub lane: Rect,
    /// The chords on the lane.
    pub slots: Vec<Rect>,
    /// Buttons.
    pub buttons: Vec<(ChordHit, Rect)>,
}

impl ChordGeom {
    /// What is under `pt`.
    pub fn hit(&self, pt: Point) -> ChordHit {
        if let Some((k, _)) = self.knobs.iter().find(|(_, r)| r.contains(pt)) {
            return ChordHit::Knob(k);
        }
        if let Some((s, _, _)) = self.pads.iter().find(|(_, _, r)| r.contains(pt)) {
            return ChordHit::Pad(*s);
        }
        if let Some((h, _)) = self.buttons.iter().find(|(_, r)| r.contains(pt)) {
            return *h;
        }
        if let Some(i) = self.slots.iter().position(|r| r.contains(pt)) {
            return ChordHit::Slot(i);
        }
        if self.lane.contains(pt) {
            return ChordHit::Lane;
        }
        ChordHit::Nothing
    }

    /// The pad for key `digit` (1–7) in `row` (0 triads, 1 sevenths, 2 borrowed).
    pub fn pad_for_key(&self, row: usize, digit: usize) -> Option<Step> {
        self.pads.iter().filter(|(_, r, _)| *r == row).nth(digit.checked_sub(1)?).map(|(s, _, _)| *s)
    }
}

/// The chord builder feeding `clip`'s editor, if that is what it edits.
pub fn builder(doc: &Document, clip: ClipId) -> Option<&Module> {
    let m = doc.modules.get(&sat_modules::editing_source(doc, clip)?)?;
    (m.kind == chords::CHORD_BUILDER).then_some(m)
}

/// How tall the editor is when it shows everything.
pub fn fit_height() -> f64 {
    TABS_H + TOOLS_H + FIT_BODY_H
}

fn tool_buttons(r: Rect, ty: f64) -> Vec<(ChordHit, Rect)> {
    let b = |x0: f64, w: f64| Rect::new(x0, ty - 10.0, x0 + w, ty + 10.0);
    let x = r.x0 + 12.0;
    let mut v = vec![
        (ChordHit::Write, b(x, 54.0)),
        (ChordHit::Preset(-1), b(x + 210.0, 22.0)),
        (ChordHit::Preset(1), b(x + 410.0, 22.0)),
        (ChordHit::Apply, b(x + 438.0, 48.0)),
    ];
    let mut rx = r.x1 - 12.0;
    for (h, w) in [(ChordHit::ToRoll, 96.0), (ChordHit::Clear, 46.0)] {
        v.push((h, b(rx - w, w)));
        rx -= w + 6.0;
    }
    v
}

/// Lays out the editor at `r` (the whole bottom panel) for `clip`.
pub fn geometry(doc: &Document, clip: Option<ClipId>, r: Rect, view: &ChordView) -> Option<ChordGeom> {
    let clip = doc.clips.get(&clip?)?;
    let m = builder(doc, clip.id)?;
    let p = chords::Params::in_doc(doc, m);
    let tools = Rect::new(r.x0, r.y0 + TABS_H, r.x1, r.y0 + TABS_H + TOOLS_H);
    let body = Rect::new(r.x0, tools.y1, r.x1, r.y1);
    let mut g = ChordGeom {
        clip: clip.id,
        module: m.id,
        knobs: Vec::new(),
        pads: Vec::new(),
        lane: Rect::ZERO,
        slots: Vec::new(),
        buttons: tool_buttons(r, tools.center().y),
    };
    // knob card
    let card = Rect::new(body.x0 + 8.0, body.y0 + 8.0, body.x0 + CARD_W, body.y1 - 8.0);
    let params = sat_modules::params_of(&m.kind);
    let cols = params.len().div_ceil(2).max(1);
    let kw = (card.width() - 16.0) / cols as f64;
    let kh = ((card.height() - 40.0) / 2.0).clamp(40.0, 72.0);
    for (i, ps) in params.iter().enumerate() {
        let (col, row) = ((i % cols) as f64, (i / cols) as f64);
        let x = card.x0 + 8.0 + col * kw;
        let y = card.y0 + 30.0 + row * kh;
        g.knobs.push((ps.key, Rect::new(x, y, x + kw, y + kh)));
    }
    // pads
    let area = Rect::new(card.x1 + 12.0, body.y0 + 8.0, body.x1 - 12.0, body.y1 - 8.0);
    let lane_top = area.y1 - LANE_H - 12.0;
    let rows = chords::pad_rows(p.key);
    let pads_h = (lane_top - 30.0 - area.y0).max(60.0);
    let rh = (pads_h / 3.0).min(46.0);
    let px0 = area.x0 + ROW_LABEL_W;
    let pw = ((area.x1 - px0) - 6.0 * 6.0) / 7.0;
    for (ri, row) in rows.iter().enumerate() {
        let y = area.y0 + ri as f64 * rh;
        for (i, s) in row.iter().enumerate() {
            let x = px0 + i as f64 * (pw + 6.0);
            g.pads.push((*s, ri, Rect::new(x, y + 2.0, x + pw, y + rh - 3.0)));
        }
    }
    // the selected chord's tools, over the lane
    let ty = lane_top - 13.0;
    let mut x = area.x1;
    for (h, w) in [
        (ChordHit::Delete, 26.0),
        (ChordHit::Length(1), 26.0),
        (ChordHit::Length(-1), 26.0),
        (ChordHit::Octave(1), 26.0),
        (ChordHit::Octave(-1), 26.0),
        (ChordHit::Invert, 40.0),
    ] {
        if view.selected.is_some() {
            g.buttons.push((h, Rect::new(x - w, ty - 10.0, x, ty + 10.0)));
        }
        x -= w + 4.0;
    }
    // the progression lane
    g.lane = Rect::new(px0, lane_top, area.x1, lane_top + LANE_H);
    let len = clip.length.0.max(1) as f64;
    let xt = |t: i64| g.lane.x0 + t as f64 / len * g.lane.width();
    for s in chords::progression(doc, m.id, clip.id) {
        let (a, b) = (xt(s.start.0), xt((s.start.0 + s.len.0).min(clip.length.0)));
        g.slots.push(Rect::new(a + 1.0, g.lane.y0 + 3.0, (b - 1.0).max(a + 4.0), g.lane.y1 - 3.0));
    }
    Some(g)
}

fn button(p: &mut Painter<'_>, th: &Theme, r: Rect, label: &str, on: bool) {
    crate::area::button(p, th, r, label, on);
}

/// Draws the chord editor for the selected clip and returns its geometry.
pub fn draw(
    p: &mut Painter<'_>,
    th: &Theme,
    doc: &Document,
    clip: Option<ClipId>,
    view: &ChordView,
    tr: &TransportView,
    r: Rect,
) -> Option<ChordGeom> {
    let c = &th.c;
    let clip_ref = clip_bar(p, th, doc, clip, r)?;
    let g = geometry(doc, clip, r, view)?;
    let m = &doc.modules[&g.module];
    let params = chords::Params::in_doc(doc, m);
    let key = params.key;
    let col = th.track(clip_ref.color.0);
    let tools = Rect::new(r.x0, r.y0 + TABS_H, r.x1, r.y0 + TABS_H + TOOLS_H);
    let ty = tools.center().y;
    p.hline(tools.x0, tools.x1, tools.y1 - 1.0, c.border_subtle);
    let small = |col: Rgba| TextStyle::mono(11.0, col);
    let progression = chords::progression(doc, g.module, g.clip);
    for (h, br) in &g.buttons {
        match h {
            ChordHit::Write => button(p, th, *br, "write", view.write),
            ChordHit::Preset(d) => button(p, th, *br, if *d < 0 { "‹" } else { "›" }, false),
            ChordHit::Apply => button(p, th, *br, "apply", false),
            ChordHit::Clear => button(p, th, *br, "clear", false),
            ChordHit::ToRoll => button(p, th, *br, "to piano roll", false),
            ChordHit::Invert => button(p, th, *br, "inv", false),
            ChordHit::Octave(d) => button(p, th, *br, if *d < 0 { "o-" } else { "o+" }, false),
            ChordHit::Length(d) => button(p, th, *br, if *d < 0 { "-" } else { "+" }, false),
            ChordHit::Delete => button(p, th, *br, "×", false),
            _ => {}
        }
    }
    // key and preset
    let write = g.buttons.iter().find(|(h, _)| *h == ChordHit::Write).map(|(_, r)| *r).unwrap();
    let from = if m.params.get("key").copied().unwrap_or(0.0) < 0.5 { "project" } else { "own" };
    let kx = write.x1 + 14.0;
    let w = p.text(kx, ty, "key", &small(c.text_secondary), Align::Left);
    let w2 = p.text(kx + w + 6.0, ty, &key.label(), &TextStyle::mono(12.0, c.text_strong), Align::Left);
    p.text(kx + w + w2 + 12.0, ty, from, &small(c.text_secondary.alpha(0.7)), Align::Left);
    if let (Some((_, a)), Some((_, b))) = (
        g.buttons.iter().find(|(h, _)| *h == ChordHit::Preset(-1)),
        g.buttons.iter().find(|(h, _)| *h == ChordHit::Preset(1)),
    ) {
        let (name, _) = chords::PROGRESSIONS[view.preset % chords::PROGRESSIONS.len()];
        let mid = Rect::new(a.x1 + 4.0, a.y0, b.x0 - 4.0, a.y1);
        p.rrect(mid, 4.0, c.bg_field);
        p.text_fit(mid.x0 + 8.0, ty, name, &TextStyle::mono(11.5, c.text_strong), mid.width() - 12.0);
    }
    // knob card
    let card = Rect::new(r.x0 + 8.0, tools.y1 + 8.0, r.x0 + CARD_W, r.y1 - 8.0);
    p.rrect(card, 6.0, c.bg_field);
    p.rrect_stroke(card.inset(-0.5), 6.0, 1.0, c.border_default);
    p.text(card.x0 + 12.0, card.y0 + 16.0, &m.name, &TextStyle::mono(12.0, c.text_max).medium(), Align::Left);
    let n = progression.len();
    let sub = match n {
        0 => "no chords yet".to_string(),
        1 => "1 chord".to_string(),
        n => format!("{n} chords"),
    };
    p.text(card.x1 - 12.0, card.y0 + 16.0, &sub, &small(c.text_secondary), Align::Right);
    let follows = from == "project";
    for (k, kr) in &g.knobs {
        let spec = sat_modules::spec(&m.kind, k).unwrap();
        let v = sat_modules::macros::value(doc, m.id, k).unwrap_or(spec.default);
        let dim = follows && (*k == "root" || *k == "scale");
        let kc = if dim { c.text_secondary } else { col };
        p.text(kr.center().x, kr.y0 + 6.0, spec.label, &TextStyle::mono(10.5, c.text_secondary), Align::Center);
        let size = (kr.height() - 30.0).clamp(18.0, 30.0);
        controls::knob(p, Point::new(kr.center().x, kr.y0 + 14.0 + size / 2.0), size, spec.pos(v), kc);
        let vy = (kr.y0 + 20.0 + size).min(kr.y1 - 4.0);
        let shown = if dim { "–".to_string() } else { spec.show(v) };
        p.text_fit(kr.x0 + 2.0, vy, &shown, &TextStyle::mono(11.0, c.text_strong), kr.width() - 4.0);
    }
    // pad rows
    let selected_step = view.selected.and_then(|i| progression.get(i)).map(Step::of);
    for (ri, label) in ROWS.iter().enumerate() {
        if let Some((_, _, first)) = g.pads.iter().find(|(_, r, _)| *r == ri) {
            let hint = match ri {
                0 => "1-7",
                1 => "shift",
                _ => "alt",
            };
            p.text(first.x0 - ROW_LABEL_W, first.center().y - 6.0, label, &small(c.text_secondary), Align::Left);
            p.text(
                first.x0 - ROW_LABEL_W,
                first.center().y + 7.0,
                hint,
                &TextStyle::mono(10.0, c.text_secondary.alpha(0.6)),
                Align::Left,
            );
        }
    }
    for (i, (s, ri, pr)) in g.pads.iter().enumerate() {
        let chord = s.resolve(key);
        let held = view.held == Some(*s);
        let used = progression.iter().any(|x| Step::of(x).resolve(key) == chord);
        let base = if held { col } else { Rgba::hex(0x1b1b1b).mix(col, if used { 0.82 } else { 1.0 }) };
        p.rrect(*pr, 5.0, base);
        let edge = if selected_step == Some(*s) { c.text_max.alpha(0.9) } else { Rgba::new(255, 255, 255, 18) };
        p.rrect_stroke(pr.inset(-0.5), 5.0, 1.0, edge);
        let fg = if held { Rgba::hex(0x111111) } else { c.text_max };
        let sub = if held { Rgba::hex(0x111111).alpha(0.75) } else { c.text_secondary };
        let roman = chords::roman(key, chord);
        let name = chords::name(key, chord);
        let two = pr.height() >= 30.0;
        if two {
            p.text_fit(
                pr.x0 + 8.0,
                pr.center().y - 7.0,
                &roman,
                &TextStyle::sans(13.0, fg).medium(),
                pr.width() - 22.0,
            );
            p.text_fit(pr.x0 + 8.0, pr.center().y + 8.0, &name, &TextStyle::mono(10.5, sub), pr.width() - 10.0);
        } else {
            p.text_fit(
                pr.x0 + 6.0,
                pr.center().y,
                &format!("{roman} {name}"),
                &TextStyle::mono(10.5, fg),
                pr.width() - 8.0,
            );
        }
        let idx = g.pads[..i].iter().filter(|(_, r, _)| r == ri).count();
        if idx < 7 {
            p.text(
                pr.x1 - 6.0,
                pr.y0 + 8.0,
                &(idx + 1).to_string(),
                &TextStyle::mono(9.5, sub.alpha(0.6)),
                Align::Right,
            );
        }
    }
    // progression lane
    let lane = g.lane;
    p.text(lane.x0 - ROW_LABEL_W, lane.y0 - 13.0, "progression", &small(c.text_secondary), Align::Left);
    let hint = if view.selected.is_some() {
        "a pad replaces the selected chord"
    } else if view.write {
        "click a pad to add it · click a chord to select it"
    } else {
        "pads play only · turn write on to build"
    };
    p.text(lane.x0, lane.y0 - 13.0, hint, &TextStyle::mono(10.5, c.text_secondary.alpha(0.7)), Align::Left);
    p.rrect(lane, 5.0, Rgba::hex(0x141414));
    let len = clip_ref.length.0.max(1);
    let bars = (len + PPQ * 4 - 1) / (PPQ * 4);
    for b in 0..=bars {
        let x = lane.x0 + (b * PPQ * 4) as f64 / len as f64 * lane.width();
        if x <= lane.x1 + 0.5 {
            p.vline(x, lane.y0, lane.y1, Rgba::new(255, 255, 255, 16));
            if b < bars {
                p.text(
                    x + 3.0,
                    lane.y1 + 8.0,
                    &(b + 1).to_string(),
                    &TextStyle::mono(9.5, c.text_secondary.alpha(0.6)),
                    Align::Left,
                );
            }
        }
    }
    let voiced = chords::voiced(&params, &progression);
    for (i, (s, sr)) in progression.iter().zip(&g.slots).enumerate() {
        let chord = Step::of(s).resolve(key);
        let sel = view.selected == Some(i);
        p.rrect(*sr, 4.0, col.mix(Rgba::hex(0x111111), if sel { 1.0 } else { 0.7 }));
        if sel {
            p.rrect_stroke(sr.inset(-1.0), 4.0, 1.5, c.text_max);
        }
        let fg = if sel { Rgba::hex(0x101010) } else { c.text_max };
        let mut label = chords::name(key, chord);
        if s.inversion > 0 && !params.lead {
            label.push_str(&format!(" /{}", s.inversion));
        }
        p.text_fit(sr.x0 + 6.0, sr.y0 + 11.0, &label, &TextStyle::mono(11.5, fg).medium(), sr.width() - 8.0);
        let roman = chords::roman(key, chord);
        let sub = if sel { Rgba::hex(0x101010).alpha(0.7) } else { c.text_secondary };
        p.text_fit(sr.x0 + 6.0, sr.y0 + 25.0, &roman, &TextStyle::mono(10.0, sub), sr.width() - 8.0);
        // a sketch of the voicing: each note a tick by pitch
        if let Some((v, _)) = voiced.get(i) {
            let (lo, hi) = (36.0, 96.0);
            for pitch in v {
                let k = ((*pitch as f64 - lo) / (hi - lo)).clamp(0.0, 1.0);
                let y = sr.y1 - 3.0 - k * (sr.height() - 6.0);
                p.rect(Rect::new(sr.x1 - 16.0, y - 0.75, sr.x1 - 6.0, y + 0.75), fg.alpha(0.55));
            }
        }
    }
    if progression.is_empty() {
        p.text(
            lane.center().x,
            lane.center().y,
            "no chords: click pads, or pick a progression and apply",
            &small(c.text_secondary.alpha(0.6)),
            Align::Center,
        );
    }
    if let Some(t) = clip_playhead(doc, clip_ref, tr) {
        let x = lane.x0 + t.0 as f64 / len as f64 * lane.width();
        p.vline(x, lane.y0, lane.y1, c.text_max);
    }
    Some(g)
}
