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
}

fn button(p: &mut Painter<'_>, th: &Theme, r: Rect, label: &str) {
    p.rrect(r, 5.0, th.c.bg_control);
    p.rrect_stroke(r.inset(-0.5), 5.0, 1.0, th.c.border_strong.alpha(0.6));
    p.text(r.center().x, r.center().y, label, &TextStyle::mono(12.0, th.c.text_strong), Align::Center);
}

/// Draws the bottom panel for the selected clip.
pub fn draw(p: &mut Painter<'_>, th: &Theme, doc: &Document, clip: Option<ClipId>, tr: &TransportView, r: Rect) {
    let c = &th.c;
    let (tabs, mut x) = panel::draw_bar(p, th, r, PanelView::Clip);
    let Some(clip) = clip.and_then(|id| doc.clips.get(&id)) else {
        p.text(x, tabs.center().y, "no clip selected", &TextStyle::mono(12.0, c.text_secondary), Align::Left);
        return;
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

    // toolbar
    let tools = Rect::new(r.x0, tabs.y1, r.x1, tabs.y1 + TOOLS_H);
    let ty = tools.center().y;
    let tg = Rect::new(r.x0 + 12.0, ty - 13.0, r.x0 + 140.0, ty + 13.0);
    p.rrect_stroke(tg.inset(-0.5), 5.0, 1.0, c.border_default);
    for (i, ic) in [Icon::Pencil, Icon::Pointer, Icon::Plus, Icon::ChevronDown].iter().enumerate() {
        let br = Rect::new(tg.x0 + 2.0 + i as f64 * 32.0, tg.y0 + 2.0, tg.x0 + 30.0 + i as f64 * 32.0, tg.y1 - 2.0);
        if i == 0 {
            p.rrect(br, 4.0, c.accent_primary_subtle);
        }
        icons::draw(p, *ic, br.inset(-7.0), if i == 0 { c.accent_primary } else { c.text_primary });
    }
    let roll_right = r.x1 - INSPECTOR_W;
    button(p, th, Rect::new(roll_right - 96.0, ty - 13.0, roll_right - 12.0, ty + 13.0), "quantise");
    button(p, th, Rect::new(roll_right - 300.0, ty - 13.0, roll_right - 196.0, ty + 13.0), "snap 1/16");

    // roll
    let notes: Vec<Note> =
        feed.and_then(|m| doc.pattern(m.id, clip.id)).map(|pt| pt.notes.iter().copied().collect()).unwrap_or_default();
    let area = Rect::new(r.x0, tools.y1, roll_right, r.y1);
    let rl = RollLayout::new(area, clip, &notes);
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
        for n in &notes {
            let x0 = rl.x_of(n.start);
            let x1 = rl.x_of(n.start + n.len);
            let y = rl.y_of(n.pitch);
            let nr = Rect::new(x0 + 0.5, y + 1.0, x1 - 0.5, y + rh - 1.0);
            p.rrect(nr, 1.5, nc.mix(Rgba::hex(0x111111), 0.35 + 0.65 * n.velocity));
            p.rrect_stroke(nr.inset(-0.5), 1.5, 1.0, nc.alpha(0.8));
        }
        // velocity lane
        p.rect(rl.vel, Rgba::hex(0x0e0e0e));
        p.hline(rl.vel.x0, rl.vel.x1, rl.vel.y0, c.border_default);
        p.text(area.x0 + 10.0, rl.vel.center().y, "vel", &TextStyle::mono(11.0, c.text_secondary), Align::Left);
        for n in &notes {
            let x = rl.x_of(n.start) + 1.0;
            let top = rl.vel.y1 - 6.0 - (rl.vel.height() - 12.0) * n.velocity as f64;
            p.vline(x, top, rl.vel.y1 - 4.0, nc);
            p.circle(Point::new(x + 0.5, top), 2.5, nc);
        }
        // playhead, if inside a placement of this clip
        if let Some(pl) = doc
            .placements
            .values()
            .find(|pl| pl.clip == clip.id && tr.playhead >= pl.start && tr.playhead < pl.start + pl.length)
        {
            let rel = Ticks((tr.playhead.0 - pl.start.0 + pl.offset.0).rem_euclid(clip.length.0));
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
}
