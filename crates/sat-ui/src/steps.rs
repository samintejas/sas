//! Step editors for the bottom panel: the drum sequencer (a row of steps per
//! drum) and the generators (euclid, turing: knobs over one row of trigs,
//! each showing its pitch, with hand edits marked).

use sat_doc::sat_time::{PPQ, Ticks};
use sat_doc::*;
use sat_modules::spec::note_name;
use sat_modules::{Editor, drum};
use sat_theme::{Rgba, Theme};
use vello::kurbo::{Point, Rect};

use crate::arrange::TransportView;
use crate::controls;
use crate::paint::{Align, Painter, TextStyle};
use crate::panel::TABS_H;
use crate::piano_roll::{TOOLS_H, clip_bar, clip_playhead};

/// Steps shown per page.
pub const PAGE: usize = 16;
/// Width of the drum pads column.
pub const PAD_W: f64 = 104.0;
/// Width of a generator's card.
pub const GEN_W: f64 = 300.0;
/// Step length of the drum sequencer.
pub const DRUM_RATE: Ticks = Ticks(PPQ / 4);

/// What the step editors show besides the document.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StepView {
    /// The page of steps shown.
    pub page: usize,
}

/// One step cell.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cell {
    /// Step index from the start of the clip.
    pub step: usize,
    /// The pitch it plays.
    pub pitch: u8,
    /// Where it is drawn.
    pub rect: Rect,
}

/// What is under a point in a step editor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StepHit {
    /// A step.
    Cell {
        /// Step index from the start of the clip.
        step: usize,
        /// Pitch.
        pitch: u8,
    },
    /// A page button.
    Page(usize),
    /// A generator knob, by parameter key.
    Knob(&'static str),
    /// The generative / fixed switch.
    Mode,
    /// Clear hand edits.
    Clear,
    /// Bake the generator and edits into fixed steps.
    Bake,
    /// Nothing editable.
    Nothing,
}

/// A step editor's geometry for one clip.
#[derive(Debug, Clone)]
pub struct StepGeom {
    /// The clip.
    pub clip: ClipId,
    /// The source module edited.
    pub module: ModuleId,
    /// Drums or a generator.
    pub editor: Editor,
    /// Step length.
    pub rate: Ticks,
    /// Steps across the clip.
    pub steps: usize,
    /// The page shown (clamped).
    pub page: usize,
    /// Page buttons.
    pub pages: Vec<Rect>,
    /// Visible cells.
    pub cells: Vec<Cell>,
    /// Generator knobs.
    pub knobs: Vec<(&'static str, Rect)>,
    /// Generative / fixed switch.
    pub mode: Option<Rect>,
    /// Clear button.
    pub clear: Option<Rect>,
    /// Bake button.
    pub bake: Option<Rect>,
}

impl StepGeom {
    /// What is under `pt`.
    pub fn hit(&self, pt: Point) -> StepHit {
        if let Some(c) = self.cells.iter().find(|c| c.rect.contains(pt)) {
            return StepHit::Cell { step: c.step, pitch: c.pitch };
        }
        if let Some(i) = self.pages.iter().position(|r| r.contains(pt)) {
            return StepHit::Page(i);
        }
        if let Some((k, _)) = self.knobs.iter().find(|(_, r)| r.contains(pt)) {
            return StepHit::Knob(k);
        }
        let on = |r: Option<Rect>| r.is_some_and(|r| r.contains(pt));
        if on(self.mode) {
            StepHit::Mode
        } else if on(self.clear) {
            StepHit::Clear
        } else if on(self.bake) {
            StepHit::Bake
        } else {
            StepHit::Nothing
        }
    }

    /// The clip-relative tick a step starts at.
    pub fn tick_of(&self, step: usize) -> Ticks {
        Ticks(step as i64 * self.rate.0)
    }
}

/// The source module whose notes `clip`'s editor edits (see
/// [`sat_modules::editing_source`]) and the editor it opens.
pub fn source(doc: &Document, clip: ClipId) -> Option<(&Module, Editor)> {
    let m = doc.modules.get(&sat_modules::editing_source(doc, clip)?)?;
    Some((m, sat_modules::editor_of(&m.kind)))
}

/// Lays out `n` cells of a page across `x0..x1`, with a wider gap every beat.
fn cell_xs(x0: f64, x1: f64, n: usize) -> Vec<(f64, f64)> {
    let (gap, beat_gap) = (3.0, 8.0);
    let beats = (n.saturating_sub(1) / 4) as f64;
    let w = ((x1 - x0) - gap * (n.saturating_sub(1)) as f64 - (beat_gap - gap) * beats) / n.max(1) as f64;
    let mut x = x0;
    (0..n)
        .map(|i| {
            if i > 0 {
                x += if i % 4 == 0 { beat_gap } else { gap };
            }
            let r = (x, x + w);
            x += w;
            r
        })
        .collect()
}

/// Height of a drum row when the editor fits its content.
pub const FIT_ROW_H: f64 = 26.0;
/// Height of a generator's card when the editor fits its content.
pub const FIT_CARD_H: f64 = 190.0;

/// How tall the step editor for `clip` is when it shows everything and no
/// more (the bottom panel fits it), or `None` when the clip has none.
pub fn fit_height(doc: &Document, clip: ClipId) -> Option<f64> {
    let (m, editor) = source(doc, clip)?;
    let body = match editor {
        Editor::Drums => {
            let notes = sat_modules::effective_notes(doc, m.id, clip);
            12.0 + drum::rows(&notes).len().max(1) as f64 * FIT_ROW_H + 6.0
        }
        Editor::Euclid | Editor::Turing => FIT_CARD_H + 16.0,
        Editor::PianoRoll => return None,
    };
    Some(TABS_H + TOOLS_H + body)
}

/// Lays out the step editor at `r` (the whole bottom panel) for `clip`.
pub fn geometry(doc: &Document, clip: Option<ClipId>, r: Rect, view: &StepView) -> Option<StepGeom> {
    let clip = doc.clips.get(&clip?)?;
    let (m, editor) = source(doc, clip.id)?;
    let rate = match editor {
        Editor::Euclid | Editor::Turing => sat_modules::gen_steps(doc, m.id, clip.id)?.rate,
        Editor::Drums => DRUM_RATE,
        Editor::PianoRoll => return None,
    };
    let steps = ((clip.length.0 + rate.0 - 1) / rate.0).max(1) as usize;
    let npages = steps.div_ceil(PAGE);
    let page = view.page.min(npages - 1);
    let tools = Rect::new(r.x0, r.y0 + TABS_H, r.x1, r.y0 + TABS_H + TOOLS_H);
    let ty = tools.center().y;
    let pages = if npages > 1 {
        (0..npages)
            .map(|i| {
                let x1 = r.x1 - 12.0 - (npages - 1 - i) as f64 * 30.0;
                Rect::new(x1 - 28.0, ty - 12.0, x1, ty + 12.0)
            })
            .collect()
    } else {
        Vec::new()
    };
    let body = Rect::new(r.x0, tools.y1, r.x1, r.y1);
    let first = page * PAGE;
    let n = (steps - first).min(PAGE);
    let mut g = StepGeom {
        clip: clip.id,
        module: m.id,
        editor,
        rate,
        steps,
        page,
        pages,
        cells: Vec::new(),
        knobs: Vec::new(),
        mode: None,
        clear: None,
        bake: None,
    };
    match editor {
        Editor::Drums => {
            let notes = sat_modules::effective_notes(doc, m.id, clip.id);
            let rows = drum::rows(&notes);
            let rh = ((body.height() - 12.0) / rows.len() as f64).clamp(10.0, 30.0);
            let xs = cell_xs(body.x0 + PAD_W + 12.0, body.x1 - 16.0, PAGE);
            for (ri, row) in rows.iter().enumerate() {
                let y = body.y0 + 6.0 + ri as f64 * rh;
                for (i, (a, b)) in xs.iter().take(n).enumerate() {
                    let rect = Rect::new(*a, y + 2.0, *b, y + rh - 2.0);
                    g.cells.push(Cell { step: first + i, pitch: row.pitch, rect });
                }
            }
        }
        Editor::Euclid | Editor::Turing => {
            let card = Rect::new(body.x0 + 8.0, body.y0 + 8.0, body.x0 + GEN_W, body.y1 - 8.0);
            let params = sat_modules::params_of(&m.kind);
            let cols = params.len().div_ceil(2).max(1);
            let kw = (card.width() - 16.0) / cols as f64;
            let kh = ((card.height() - 64.0) / 2.0).clamp(40.0, 64.0);
            for (i, p) in params.iter().enumerate() {
                let (col, row) = ((i % cols) as f64, (i / cols) as f64);
                let x = card.x0 + 8.0 + col * kw;
                let y = card.y0 + 30.0 + row * kh;
                g.knobs.push((p.key, Rect::new(x, y, x + kw, y + kh)));
            }
            let ey = card.y1 - 16.0;
            g.clear = Some(Rect::new(card.x1 - 104.0, ey - 11.0, card.x1 - 60.0, ey + 11.0));
            g.bake = Some(Rect::new(card.x1 - 54.0, ey - 11.0, card.x1 - 8.0, ey + 11.0));
            g.mode = Some(Rect::new(r.x0 + 12.0, ty - 12.0, r.x0 + 180.0, ty + 12.0));
            let pitches = sat_modules::gen_steps(doc, m.id, clip.id).map(|s| s.pitches).unwrap_or_default();
            let xs = cell_xs(card.x1 + 64.0, body.x1 - 16.0, PAGE);
            let y0 = body.y0 + 12.0;
            let h = (body.height() * 0.42).clamp(32.0, 64.0);
            for (i, (a, b)) in xs.iter().take(n).enumerate() {
                let pitch = pitches.get(first + i).copied().unwrap_or(60);
                g.cells.push(Cell { step: first + i, pitch, rect: Rect::new(*a, y0, *b, y0 + h) });
            }
        }
        Editor::PianoRoll => {}
    }
    Some(g)
}

fn button(p: &mut Painter<'_>, th: &Theme, r: Rect, label: &str, on: bool) {
    crate::area::button(p, th, r, label, on);
}

fn label(p: &mut Painter<'_>, th: &Theme, x: f64, y: f64, k: &str, v: &str) -> f64 {
    let w = p.text(x, y, k, &TextStyle::mono(11.0, th.c.text_secondary), Align::Left);
    w + 6.0 + p.text(x + w + 6.0, y, v, &TextStyle::mono(12.0, th.c.text_strong), Align::Left) + 18.0
}

/// How a step looks.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Look {
    on: bool,
    velocity: f32,
    added: bool,
    removed: bool,
}

/// Draws the step editor for the selected clip and returns its geometry.
pub fn draw(
    p: &mut Painter<'_>,
    th: &Theme,
    doc: &Document,
    clip: Option<ClipId>,
    view: &StepView,
    tr: &TransportView,
    r: Rect,
) -> Option<StepGeom> {
    let c = &th.c;
    let clip_ref = clip_bar(p, th, doc, clip, r)?;
    let g = geometry(doc, clip, r, view)?;
    let m = &doc.modules[&g.module];
    let tools = Rect::new(r.x0, r.y0 + TABS_H, r.x1, r.y0 + TABS_H + TOOLS_H);
    let ty = tools.center().y;
    p.hline(tools.x0, tools.x1, tools.y1 - 1.0, c.border_subtle);
    let rate = format!("1/{}", PPQ * 4 / g.rate.0);
    let cur = clip_playhead(doc, clip_ref, tr).map(|t| (t.0 / g.rate.0) as usize);
    let notes = sat_modules::effective_notes(doc, g.module, g.clip);
    let pat = doc.pattern(g.module, g.clip);
    for (i, pr) in g.pages.iter().enumerate() {
        button(p, th, *pr, &(i + 1).to_string(), i == g.page);
    }
    if !g.pages.is_empty() {
        let x = g.pages[0].x0 - 8.0;
        p.text(x, ty, "page", &TextStyle::mono(11.0, c.text_secondary), Align::Right);
    }
    let look = |step: usize, pitch: u8| -> Look {
        let t = g.tick_of(step);
        let n = notes.iter().find(|n| n.start == t && n.pitch == pitch);
        let o = pat.map(|p| &p.overrides);
        Look {
            on: n.is_some(),
            velocity: n.map_or(0.0, |n| n.velocity),
            added: o.is_some_and(|o| o.added.iter().any(|a| a.start == t && a.pitch == pitch)),
            removed: o.is_some_and(|o| o.removed.contains(&(t, pitch))),
        }
    };
    match g.editor {
        Editor::Drums => {
            let mut x = r.x0 + 16.0;
            x += label(p, th, x, ty, "kit", "gm drums");
            x += label(p, th, x, ty, "length", &g.steps.to_string());
            label(p, th, x, ty, "rate", &rate);
            let rows = drum::rows(&notes);
            for (ri, row) in rows.iter().enumerate() {
                let col = th.track(ri as u8);
                let cells: Vec<&Cell> = g.cells.iter().filter(|c| c.pitch == row.pitch).collect();
                let Some(first) = cells.first() else { continue };
                let (y0, y1) = (first.rect.y0, first.rect.y1);
                let pad = Rect::new(r.x0 + 12.0, y0, r.x0 + 12.0 + PAD_W - 8.0, y1);
                p.rrect(pad, 4.0, c.bg_control);
                p.rrect(Rect::new(pad.x0 + 7.0, pad.center().y - 3.0, pad.x0 + 13.0, pad.center().y + 3.0), 1.5, col);
                let size = if pad.height() < 16.0 { 10.0 } else { 11.5 };
                p.text(pad.x0 + 20.0, pad.center().y, &row.name, &TextStyle::mono(size, c.text_strong), Align::Left);
                for cell in cells {
                    let l = look(cell.step, cell.pitch);
                    step_cell(p, th, cell, l, col, cur == Some(cell.step));
                }
            }
        }
        Editor::Euclid | Editor::Turing => {
            let generating = sat_modules::generating(m);
            let mode = g.mode.unwrap();
            let half = Rect::new(mode.x0, mode.y0, mode.center().x, mode.y1);
            button(p, th, half, "generative", generating);
            button(p, th, Rect::new(mode.center().x, mode.y0, mode.x1, mode.y1), "fixed", !generating);
            let mut x = mode.x1 + 18.0;
            x += label(p, th, x, ty, "length", &g.steps.to_string());
            label(p, th, x, ty, "rate", &rate);
            // generator card
            let card = Rect::new(r.x0 + 8.0, tools.y1 + 8.0, r.x0 + GEN_W, r.y1 - 8.0);
            p.rrect(card, 6.0, c.bg_field);
            p.rrect_stroke(card.inset(-0.5), 6.0, 1.0, c.border_default);
            p.text(card.x0 + 12.0, card.y0 + 16.0, &m.name, &TextStyle::mono(12.0, c.text_max).medium(), Align::Left);
            let sub = if generating { "generator + edits" } else { "fixed steps" };
            p.text(card.x1 - 12.0, card.y0 + 16.0, sub, &TextStyle::mono(11.0, c.text_secondary), Align::Right);
            let col = th.track(clip_ref.color.0);
            for (key, kr) in &g.knobs {
                let spec = sat_modules::spec(&m.kind, key).unwrap();
                let v = sat_modules::macros::value(doc, m.id, key).unwrap_or(spec.default);
                let kc = if generating { col } else { c.text_secondary };
                p.text(kr.center().x, kr.y0 + 6.0, spec.label, &TextStyle::mono(10.5, c.text_secondary), Align::Center);
                let size = (kr.height() - 30.0).clamp(18.0, 30.0);
                controls::knob(p, Point::new(kr.center().x, kr.y0 + 14.0 + size / 2.0), size, spec.pos(v), kc);
                let vy = (kr.y0 + 20.0 + size).min(kr.y1 - 4.0);
                p.text(kr.center().x, vy, &spec.show(v), &TextStyle::mono(11.0, c.text_strong), Align::Center);
            }
            let (added, removed) = sat_modules::edit_counts(doc, g.module, g.clip);
            let edits = added + removed;
            let ey = g.bake.unwrap().center().y;
            if edits > 0 {
                p.circle(Point::new(card.x0 + 16.0, ey), 3.0, c.accent_warning);
            }
            let text = match edits {
                0 => "no hand edits".to_string(),
                1 => "1 hand edit".to_string(),
                n => format!("{n} hand edits"),
            };
            p.text(card.x0 + 26.0, ey, &text, &TextStyle::mono(11.0, c.text_secondary), Align::Left);
            button(p, th, g.clear.unwrap(), "clear", false);
            button(p, th, g.bake.unwrap(), "bake", false);
            // trigs
            if let Some(first) = g.cells.first() {
                let y = first.rect.center().y;
                p.text(card.x1 + 16.0, y, "trigs", &TextStyle::mono(11.0, c.text_secondary), Align::Left);
                let ny = first.rect.y1 + 16.0;
                p.text(card.x1 + 16.0, ny, "note", &TextStyle::mono(11.0, c.text_secondary), Align::Left);
                // the turing's steps each have a pitch: a tick shows how high
                let (lo, hi) = g.cells.iter().fold((127u8, 0u8), |(a, b), c| (a.min(c.pitch), b.max(c.pitch)));
                for cell in &g.cells {
                    let l = look(cell.step, cell.pitch);
                    step_cell(p, th, cell, l, col, cur == Some(cell.step));
                    if g.editor == Editor::Turing && hi > lo {
                        let k = (cell.pitch - lo) as f64 / (hi - lo) as f64;
                        let y = cell.rect.y1 - 6.0 - k * (cell.rect.height() - 12.0);
                        let tick = if l.on { c.text_max.alpha(0.85) } else { c.text_secondary.alpha(0.45) };
                        p.rect(Rect::new(cell.rect.x0 + 5.0, y - 1.0, cell.rect.x1 - 5.0, y + 1.0), tick);
                    }
                    let st = TextStyle::mono(10.5, if l.on { c.text_strong } else { c.text_secondary.alpha(0.5) });
                    p.text(cell.rect.center().x, ny, &note_name(cell.pitch), &st, Align::Center);
                }
                // legend
                let ly = r.y1 - 18.0;
                let mut lx = card.x1 + 64.0;
                for (k, colr) in
                    [("hit", col), ("accent", c.text_max), ("added", c.accent_warning), ("removed", c.text_secondary)]
                {
                    p.rrect(Rect::new(lx, ly - 4.0, lx + 8.0, ly + 4.0), 2.0, colr);
                    lx += 14.0 + p.text(lx + 14.0, ly, k, &TextStyle::mono(10.5, c.text_secondary), Align::Left) + 16.0;
                }
            }
        }
        Editor::PianoRoll => {}
    }
    Some(g)
}

fn step_cell(p: &mut Painter<'_>, th: &Theme, cell: &Cell, l: Look, col: Rgba, current: bool) {
    let c = &th.c;
    let r = cell.rect;
    let beat = cell.step % 4 == 0;
    let base = if current {
        Rgba::hex(0x303030)
    } else if beat {
        Rgba::hex(0x1d1d1d)
    } else {
        Rgba::hex(0x161616)
    };
    p.rrect(r, 3.0, base);
    if l.on {
        let k = 0.35 + 0.65 * l.velocity.clamp(0.0, 1.0);
        p.rrect(r, 3.0, col.mix(Rgba::hex(0x111111), k));
        if l.velocity >= 0.99 {
            p.rrect_stroke(r.inset(-1.0), 2.5, 1.0, c.text_max.alpha(0.8));
        }
    } else {
        p.rrect_stroke(r.inset(-0.5), 3.0, 1.0, Rgba::new(255, 255, 255, 14));
    }
    if l.added {
        p.circle(Point::new(r.x1 - 5.0, r.y0 + 5.0), 2.5, c.accent_warning);
    }
    if l.removed {
        p.line(Point::new(r.x0 + 4.0, r.y1 - 4.0), Point::new(r.x1 - 4.0, r.y0 + 4.0), 1.2, c.text_secondary);
    }
    if current && l.on {
        p.rrect_stroke(r.inset(0.5), 3.0, 1.5, c.text_max);
    }
}
