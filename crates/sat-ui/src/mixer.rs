//! The mixer: one strip per track, group, return and master, in the bottom panel.
//!
//! Layout is pure geometry ([`MixerLayout`]) so the app can hit-test drags
//! and tests can check it without a GPU.

use std::collections::HashMap;

use sat_doc::*;
use sat_theme::{Rgba, Theme};
use vello::kurbo::{Point, Rect};

use crate::controls::{self, On};
use crate::paint::{Align, Painter, TextStyle};
use crate::panel::{self, PanelView};

/// Width of a strip.
pub const STRIP_W: f64 = 98.0;
/// Width of the master strip.
pub const MASTER_W: f64 = 104.0;
/// Space between strips.
pub const GAP: f64 = 4.0;
/// Knob diameter.
pub const KNOB: f64 = 24.0;
/// How many sends a strip shows (returns a, b, …).
pub const SENDS_SHOWN: usize = 2;

/// Linear peak levels (left, right) per track, as the meters show them.
pub type Levels = HashMap<TrackId, (f32, f32)>;

/// How a strip is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StripKind {
    /// A group.
    Group,
    /// A track or group inside a group.
    Child,
    /// A top-level track.
    Track,
    /// A return.
    Return,
    /// Master.
    Master,
}

/// What a knob on a strip controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Knob {
    /// The send to this return.
    Send(TrackId),
    /// Pan.
    Pan,
}

/// One strip's geometry.
#[derive(Debug, Clone, PartialEq)]
pub struct StripLayout {
    /// The track.
    pub track: TrackId,
    /// How it is drawn.
    pub kind: StripKind,
    /// The whole strip.
    pub rect: Rect,
    /// Knobs, left to right: their circle's bounding box and what they set.
    pub knobs: Vec<(Rect, Knob)>,
    /// The fader slot; the cap's centre travels from its bottom to its top.
    pub fader: Rect,
    /// The stereo meter.
    pub meter: Rect,
    /// The level readout.
    pub readout: Rect,
    /// Mute button (not on master).
    pub mute: Option<Rect>,
    /// Solo button (not on master).
    pub solo: Option<Rect>,
    /// Buttons that open the track's grids (source, instrument, fx: those it has).
    pub grids: Vec<(GridKind, Rect)>,
}

impl StripLayout {
    /// Screen y of the fader cap's centre for a position (0–1).
    pub fn fader_y(&self, pos: f64) -> f64 {
        self.fader.y1 - pos.clamp(0.0, 1.0) * self.fader.height()
    }

    /// Fader position (0–1) at screen y.
    pub fn fader_pos_at(&self, y: f64) -> f64 {
        ((self.fader.y1 - y) / self.fader.height().max(1.0)).clamp(0.0, 1.0)
    }

    /// The fader cap's rectangle for a position.
    pub fn cap(&self, pos: f64) -> Rect {
        let (cx, cy) = (self.fader.center().x, self.fader_y(pos));
        Rect::new(cx - 11.0, cy - 5.0, cx + 11.0, cy + 5.0)
    }
}

/// What is under a point in the mixer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MixHit {
    /// A fader (slot or cap).
    Fader(TrackId),
    /// A knob.
    Knob(TrackId, Knob),
    /// A mute button.
    Mute(TrackId),
    /// A solo button.
    Solo(TrackId),
    /// A button that opens one of the track's grids.
    Grid(TrackId, GridKind),
    /// Elsewhere on a strip.
    Strip(TrackId),
    /// Nothing.
    Nothing,
}

/// The mixer laid out in a panel.
#[derive(Debug, Clone)]
pub struct MixerLayout {
    /// The area below the tab bar.
    pub body: Rect,
    /// Strips, left to right (master last).
    pub strips: Vec<StripLayout>,
    /// The separator between tracks and returns.
    pub separator: Option<f64>,
    /// How far the strips can scroll.
    pub max_scroll: f64,
}

/// Returns in order; the first two are sends a and b.
pub fn returns(doc: &Document) -> Vec<TrackId> {
    doc.tracks_in_order().into_iter().filter(|t| doc.tracks[t].kind == TrackKind::Return).collect()
}

fn strip_layout(doc: &Document, t: TrackId, kind: StripKind, rect: Rect, rets: &[TrackId]) -> StripLayout {
    let inner = rect.inset(-4.0);
    let tr = &doc.tracks[&t];
    let mut knobs = Vec::new();
    let ky = inner.y0 + 18.0;
    let sends = matches!(tr.kind, TrackKind::Track | TrackKind::Group);
    if sends {
        let w = inner.width() / 3.0;
        for (i, k) in rets.iter().take(SENDS_SHOWN).map(|r| Knob::Send(*r)).chain([Knob::Pan]).enumerate() {
            let cx = inner.x0 + w * (i as f64 + 0.5);
            knobs.push((Rect::new(cx - KNOB / 2.0, ky, cx + KNOB / 2.0, ky + KNOB), k));
        }
    } else {
        let cx = inner.center().x;
        knobs.push((Rect::new(cx - KNOB / 2.0, ky, cx + KNOB / 2.0, ky + KNOB), Knob::Pan));
    }
    let buttons_y0 = inner.y1 - 24.0;
    let readout = Rect::new(inner.x0, buttons_y0 - 18.0, inner.x1, buttons_y0 - 4.0);
    // a row of buttons that open the track's grids
    let gy = ky + KNOB + 20.0;
    let kinds = crate::grid::grid_kinds(doc, t);
    let gw = inner.width() / 3.0;
    let grids: Vec<(GridKind, Rect)> = kinds
        .iter()
        .map(|(k, _)| {
            let i = match k {
                GridKind::Source => 0.0,
                GridKind::Instrument => 1.0,
                GridKind::Fx => 2.0,
            };
            (*k, Rect::new(inner.x0 + gw * i + 1.0, gy, inner.x0 + gw * (i + 1.0) - 1.0, gy + 16.0))
        })
        .collect();
    let (f0, f1) = (gy + 22.0, readout.y0 - 4.0);
    let cx = inner.center().x;
    // the slot and the meter sit side by side, centred as a 22 px group
    let fader = Rect::new(cx - 11.0, f0 + 5.0, cx - 5.0, f1 - 5.0);
    let meter = Rect::new(cx + 1.0, f0, cx + 11.0, f1);
    let (mute, solo) = if kind == StripKind::Master {
        (None, None)
    } else {
        let mid = inner.center().x;
        (
            Some(Rect::new(inner.x0, buttons_y0, mid - 1.5, inner.y1)),
            Some(Rect::new(mid + 1.5, buttons_y0, inner.x1, inner.y1)),
        )
    };
    StripLayout { track: t, kind, rect, knobs, fader, meter, readout, mute, solo, grids }
}

impl MixerLayout {
    /// Lays out the mixer for `doc` in the panel body `body`, scrolled by `scroll_x`.
    pub fn new(doc: &Document, body: Rect, scroll_x: f64) -> MixerLayout {
        let rets = returns(doc);
        let area = Rect::new(body.x0 + 12.0, body.y0 + 8.0, body.x1 - 12.0, body.y1 - 10.0);
        let master_rect = Rect::new(area.x1 - MASTER_W, area.y0, area.x1, area.y1);
        let order = doc.tracks_in_order();
        let mut plan: Vec<(TrackId, StripKind)> = order
            .iter()
            .filter(|t| matches!(doc.tracks[t].kind, TrackKind::Track | TrackKind::Group))
            .map(|&t| {
                let tr = &doc.tracks[&t];
                let kind = match (tr.kind, tr.parent) {
                    (_, Some(_)) => StripKind::Child,
                    (TrackKind::Group, None) => StripKind::Group,
                    _ => StripKind::Track,
                };
                (t, kind)
            })
            .collect();
        let n_main = plan.len();
        plan.extend(rets.iter().map(|&r| (r, StripKind::Return)));
        let content_w = plan.len() as f64 * (STRIP_W + GAP) + if rets.is_empty() { 0.0 } else { 9.0 };
        let view_w = (master_rect.x0 - GAP - area.x0).max(0.0);
        let max_scroll = (content_w - view_w).max(0.0);
        let scroll = scroll_x.clamp(0.0, max_scroll);
        let mut x = area.x0 - scroll;
        let mut strips = Vec::with_capacity(plan.len() + 1);
        let mut separator = None;
        for (i, (t, kind)) in plan.into_iter().enumerate() {
            if i == n_main && n_main > 0 {
                separator = Some(x + 4.0);
                x += 9.0;
            }
            let rect = Rect::new(x, area.y0, x + STRIP_W, area.y1);
            strips.push(strip_layout(doc, t, kind, rect, &rets));
            x += STRIP_W + GAP;
        }
        strips.push(strip_layout(doc, doc.master, StripKind::Master, master_rect, &rets));
        MixerLayout { body, strips, separator, max_scroll }
    }

    /// The x range scrolling strips may draw in (left of master).
    pub fn scroll_view(&self) -> Rect {
        let m = self.strips.last().map(|s| s.rect.x0).unwrap_or(self.body.x1);
        Rect::new(self.body.x0, self.body.y0, m - GAP, self.body.y1)
    }

    /// The strip for a track.
    pub fn strip(&self, t: TrackId) -> Option<&StripLayout> {
        self.strips.iter().find(|s| s.track == t)
    }

    /// What is under `pt`.
    pub fn hit(&self, pt: Point) -> MixHit {
        let view = self.scroll_view();
        let Some(s) =
            self.strips.iter().rev().find(|s| s.rect.contains(pt) && (s.kind == StripKind::Master || pt.x < view.x1))
        else {
            return MixHit::Nothing;
        };
        let t = s.track;
        if s.mute.is_some_and(|r| r.contains(pt)) {
            return MixHit::Mute(t);
        }
        if s.solo.is_some_and(|r| r.contains(pt)) {
            return MixHit::Solo(t);
        }
        // knobs are small: accept the label below them too
        if let Some((_, k)) =
            s.knobs.iter().find(|(r, _)| Rect::new(r.x0 - 4.0, r.y0, r.x1 + 4.0, r.y1 + 16.0).contains(pt))
        {
            return MixHit::Knob(t, *k);
        }
        if let Some((k, _)) = s.grids.iter().find(|(_, r)| r.contains(pt)) {
            return MixHit::Grid(t, *k);
        }
        let grab = Rect::new(s.fader.x0 - 10.0, s.fader.y0 - 8.0, s.meter.x1, s.fader.y1 + 8.0);
        if grab.contains(pt) {
            return MixHit::Fader(t);
        }
        MixHit::Strip(t)
    }
}

/// The value a knob shows (0–1).
pub fn knob_value(doc: &Document, t: TrackId, k: Knob) -> f64 {
    let tr = &doc.tracks[&t];
    match k {
        Knob::Pan => controls::pan_pos(tr.mixer.pan),
        Knob::Send(r) => {
            tr.sends.iter().find(|s| s.target == r).map(|s| controls::fader_pos(s.level_db)).unwrap_or(0.0)
        }
    }
}

fn knob_label(rets: &[TrackId], k: Knob) -> String {
    match k {
        Knob::Pan => "pan".into(),
        Knob::Send(r) => {
            let i = rets.iter().position(|x| *x == r).unwrap_or(0);
            ((b'a' + i as u8) as char).to_string()
        }
    }
}

/// Draws the mixer panel at `r` (tab bar included) and returns its layout.
pub fn draw(
    p: &mut Painter<'_>,
    th: &Theme,
    doc: &Document,
    selected: Option<TrackId>,
    levels: &Levels,
    scroll_x: f64,
    r: Rect,
) -> MixerLayout {
    let c = &th.c;
    let (bar, x) = panel::draw_bar(p, th, r, PanelView::Mixer);
    let rets = returns(doc);
    let cy = bar.center().y;
    let letters: Vec<String> =
        (0..rets.len().min(SENDS_SHOWN)).map(|i| ((b'a' + i as u8) as char).to_string()).collect();
    let channels = doc.tracks.len();
    let info = if letters.is_empty() {
        format!("{channels} channels")
    } else {
        format!("{channels} channels · sends {}", letters.join("·"))
    };
    p.text(x, cy, &info, &TextStyle::mono(12.0, c.text_secondary), Align::Left);

    let body = Rect::new(r.x0, bar.y1, r.x1, r.y1);
    let l = MixerLayout::new(doc, body, scroll_x);
    let view = l.scroll_view();
    let (main, master) = l.strips.split_at(l.strips.len() - 1);
    p.clip(view, |p| {
        if let Some(sx) = l.separator {
            p.rect(Rect::new(sx, body.y0 + 8.0, sx + 1.0, body.y1 - 10.0), c.border_default);
        }
        for s in main {
            if s.rect.x1 >= view.x0 && s.rect.x0 <= view.x1 {
                draw_strip(p, th, doc, s, &rets, selected == Some(s.track), levels);
            }
        }
    });
    for s in master {
        draw_strip(p, th, doc, s, &rets, selected == Some(s.track), levels);
    }
    l
}

fn draw_strip(
    p: &mut Painter<'_>,
    th: &Theme,
    doc: &Document,
    s: &StripLayout,
    rets: &[TrackId],
    selected: bool,
    levels: &Levels,
) {
    let c = &th.c;
    let tr = &doc.tracks[&s.track];
    let col = if s.kind == StripKind::Master { Rgba::hex(0xdcdcdc) } else { th.track(tr.color.0) };
    let (bg, border) = match s.kind {
        StripKind::Group => (col.mix(Rgba::hex(0x121212), 0.06), col.alpha(0.45)),
        StripKind::Master => (Rgba::hex(0x161616), c.border_default),
        _ => (Rgba::hex(0x121212), c.border_default),
    };
    let r = s.rect;
    p.rrect(r, 4.0, bg);
    // coloured top edge; children also get a darker foot
    p.clip(r, |p| {
        p.rect(Rect::new(r.x0, r.y0, r.x1, r.y0 + 2.0), col);
        if s.kind == StripKind::Child {
            p.rect(Rect::new(r.x0, r.y1 - 3.0, r.x1, r.y1), Rgba::hex(0x3a3a3a));
        }
    });
    let border = if selected { Rgba::new(240, 160, 220, 153) } else { border };
    p.rrect_stroke(r.inset(-0.5), 4.0, 1.0, border);

    // name
    let inner = r.inset(-4.0);
    let ny = inner.y0 + 8.0;
    p.circle(Point::new(inner.x0 + 5.0, ny), 3.0, col);
    let name = match s.kind {
        StripKind::Return => {
            let i = rets.iter().position(|x| *x == s.track).unwrap_or(0);
            format!("{} · {}", (b'a' + i as u8) as char, tr.name)
        }
        StripKind::Master => "master".into(),
        _ => tr.name.clone(),
    };
    p.text_fit(inner.x0 + 13.0, ny, &name, &TextStyle::sans(11.0, c.text_strong), inner.width() - 14.0);

    // knobs
    for (kr, k) in &s.knobs {
        let v = knob_value(doc, s.track, *k);
        let has = match k {
            Knob::Send(r) => tr.sends.iter().any(|x| x.target == *r),
            Knob::Pan => true,
        };
        controls::knob(p, kr.center(), KNOB, v, if has { col } else { col.alpha(0.35) });
        p.text(
            kr.center().x,
            kr.y1 + 8.0,
            &knob_label(rets, *k),
            &TextStyle::sans(11.0, Rgba::hex(0x9a9a9a)),
            Align::Center,
        );
    }

    // grid buttons
    for (k, gr) in &s.grids {
        p.rrect(*gr, 3.0, c.bg_control);
        p.rrect_stroke(gr.inset(-0.5), 3.0, 1.0, c.border_default);
        let label = match k {
            GridKind::Source => "src",
            GridKind::Instrument => "ins",
            GridKind::Fx => "fx",
        };
        p.text(gr.center().x, gr.center().y, label, &TextStyle::mono(10.0, c.text_primary), Align::Center);
    }

    // fader and meter
    p.rrect(s.fader, 3.0, Rgba::hex(0x0a0a0a));
    p.rrect_stroke(s.fader.inset(-0.5), 3.0, 1.0, Rgba::hex(0x262626));
    let (l, rr) = levels.get(&s.track).copied().unwrap_or((0.0, 0.0));
    controls::meter(p, s.meter, controls::meter_pos(l), controls::meter_pos(rr));
    let pos = controls::fader_pos(tr.mixer.gain_db);
    let cap = s.cap(pos);
    p.rrect(cap, 2.0, Rgba::hex(0xc8c8c8));
    p.rect(Rect::new(cap.x0 + 3.0, cap.center().y - 0.5, cap.x1 - 3.0, cap.center().y + 0.5), Rgba::hex(0x5a5a5a));
    p.text(
        s.readout.center().x,
        s.readout.center().y,
        &controls::fmt_db(tr.mixer.gain_db),
        &TextStyle::mono(11.0, c.text_strong),
        Align::Center,
    );

    // buttons
    if let (Some(m), Some(so)) = (s.mute, s.solo) {
        controls::toggle(p, th, m, "m", tr.mixer.mute.then_some(On::Solid(Rgba::hex(0xf0b14a))));
        controls::toggle(p, th, so, "s", tr.mixer.solo.then_some(On::Solid(Rgba::hex(0x6aa8ff))));
    } else {
        let inner = s.rect.inset(-4.0);
        controls::toggle(p, th, Rect::new(inner.x0, inner.y1 - 24.0, inner.x1, inner.y1), "cue", None);
    }
}
