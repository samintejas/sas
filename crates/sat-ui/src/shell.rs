//! The app shell: top bar, arrange view and bottom panel.

use sat_doc::Document;
use sat_doc::sat_time::{PPQ, Ticks};
use sat_theme::{Rgba, Theme};
use vello::Scene;
use vello::kurbo::Rect;

use crate::arrange::{self, ArrangeLayout, ArrangeState, TransportView};
use crate::icons::{self, Icon};
use crate::paint::{Align, Fonts, Painter, TextStyle};
use crate::piano_roll;

/// Height of the top bar.
pub const TOPBAR_H: f64 = 40.0;
/// Default height of the bottom panel.
pub const PANEL_H: f64 = 306.0;

/// Everything the shell needs to draw a frame.
pub struct Frame<'a> {
    /// The project.
    pub doc: &'a Document,
    /// Project name.
    pub name: &'a str,
    /// Arrange view state.
    pub arrange: &'a ArrangeState,
    /// Transport.
    pub transport: TransportView,
    /// Bottom panel height (0 hides it).
    pub panel_h: f64,
}

/// Rectangles of the shell's regions.
#[derive(Debug, Clone, Copy)]
pub struct ShellLayout {
    /// Top bar.
    pub topbar: Rect,
    /// Arrange tile.
    pub arrange: Rect,
    /// Bottom panel.
    pub panel: Rect,
}

impl ShellLayout {
    /// Splits a window of `w`×`h`.
    pub fn new(w: f64, h: f64, panel_h: f64) -> ShellLayout {
        let topbar = Rect::new(0.0, 0.0, w, TOPBAR_H);
        let panel_h = panel_h.min(h - TOPBAR_H - 200.0).max(0.0);
        let arrange = Rect::new(4.0, TOPBAR_H + 4.0, w - 4.0, h - panel_h - 4.0);
        let panel = Rect::new(4.0, h - panel_h, w - 4.0, h - 4.0);
        ShellLayout { topbar, arrange, panel }
    }
}

/// Formats a position as bar.beat.sixteenth, 1-based, like the prototype's
/// `007.02.01`.
pub fn format_pos(doc: &Document, t: Ticks) -> String {
    let bb = doc.meter_map().map(|m| m.bar_beat(t)).unwrap_or(sat_doc::sat_time::BarBeat {
        bar: 0,
        beat: 0,
        tick: Ticks::ZERO,
    });
    format!("{:03}.{:02}.{:02}", bb.bar + 1, bb.beat + 1, bb.tick.0 / (PPQ / 4) + 1)
}

fn display(p: &mut Painter<'_>, th: &Theme, x: f64, cy: f64, label: &str, value: &str, w: f64) -> f64 {
    let c = &th.c;
    p.text(x, cy - 8.0, label, &TextStyle::mono(10.5, c.text_secondary), Align::Left);
    p.text(x, cy + 7.0, value, &TextStyle::mono(13.0, c.accent_primary).medium(), Align::Left);
    w
}

fn icon_button(p: &mut Painter<'_>, th: &Theme, r: Rect, icon: Icon, state: Option<Rgba>) {
    match state {
        Some(c) if c == th.c.accent_primary => {
            p.rrect(r, 5.0, c);
            icons::draw(p, icon, r.inset(-6.0), th.c.on_accent_primary);
        }
        Some(c) => {
            p.rrect(r, 5.0, c.alpha(0.16));
            p.rrect_stroke(r.inset(-0.5), 5.0, 1.0, c.alpha(0.5));
            icons::draw(p, icon, r.inset(-6.0), c);
        }
        None => {
            p.rrect(r, 5.0, th.c.bg_control);
            p.rrect_stroke(r.inset(-0.5), 5.0, 1.0, th.c.border_default);
            icons::draw(p, icon, r.inset(-6.0), th.c.text_strong);
        }
    }
}

fn topbar(p: &mut Painter<'_>, th: &Theme, f: &Frame<'_>, r: Rect) {
    let c = &th.c;
    p.rect(r, c.bg_base);
    p.hline(r.x0, r.x1, r.y1 - 1.0, c.border_default);
    let cy = r.center().y;
    icons::draw(p, Icon::Panel, Rect::new(10.0, cy - 7.0, 26.0, cy + 7.0), c.text_primary);
    p.text(44.0, cy, "sat", &TextStyle::sans(15.0, c.text_max).medium(), Align::Left);
    p.text(76.0, cy, f.name, &TextStyle::sans(13.0, c.text_secondary), Align::Left);
    let mut x = 160.0;
    for (i, ic) in [Icon::Grid, Icon::Lines, Icon::Panel, Icon::Gear, Icon::Sliders].iter().enumerate() {
        let br = Rect::new(x, r.y0, x + 40.0, r.y1);
        icons::draw(p, *ic, br.inset(-12.0), if i == 1 { c.text_max } else { c.text_secondary });
        if i == 1 {
            p.rect(Rect::new(br.x0 + 4.0, r.y1 - 2.0, br.x1 - 4.0, r.y1), c.accent_primary);
        }
        x += 40.0;
        if i == 1 {
            p.vline(x + 6.0, cy - 9.0, cy + 9.0, c.border_default);
            x += 12.0;
        }
    }
    // transport
    let w = r.width();
    let mut x = w / 2.0 - 260.0;
    let group = Rect::new(x, cy - 15.0, x + 136.0, cy + 15.0);
    p.rrect(group, 6.0, c.bg_surface);
    p.rrect_stroke(group.inset(-0.5), 6.0, 1.0, c.border_default);
    let t = &f.transport;
    let buttons = [
        (Icon::Stop, None),
        (Icon::Play, Some(c.accent_primary)),
        (Icon::Record, t.recording.then_some(c.accent_danger)),
        (Icon::Loop, f.doc.loop_range.enabled.then_some(Rgba::hex(0xc8c8c8))),
    ];
    for (ic, state) in buttons {
        let br = Rect::new(x + 4.0, cy - 12.0, x + 30.0, cy + 12.0);
        match (ic, state) {
            (Icon::Record, s) => {
                p.rrect(br, 5.0, if s.is_some() { c.accent_danger.alpha(0.18) } else { Rgba::new(0, 0, 0, 0) });
                icons::draw(p, ic, br.inset(-5.0), c.accent_danger);
            }
            (Icon::Stop, _) => {
                p.rrect(br, 5.0, c.bg_control);
                icons::draw(p, ic, br.inset(-6.0), c.text_max);
            }
            (_, s) => icon_button(p, th, br, ic, s),
        }
        x += 33.0;
    }
    x += 14.0;
    let bpm = f.doc.tempo_map().map(|m| m.bpm_at(t.playhead)).unwrap_or(120.0);
    let sig = f.doc.meter_map().map(|m| m.sig_at(t.playhead)).unwrap_or(sat_doc::sat_time::TimeSig::FOUR_FOUR);
    let panel = Rect::new(x, cy - 16.0, x + 344.0, cy + 16.0);
    p.rrect(panel, 6.0, c.bg_surface);
    p.rrect_stroke(panel.inset(-0.5), 6.0, 1.0, c.border_default);
    let mut dx = x + 10.0;
    dx += display(p, th, dx, cy, "pos", &format_pos(f.doc, t.playhead), 96.0);
    p.vline(dx - 6.0, cy - 12.0, cy + 12.0, c.border_default);
    dx += display(p, th, dx, cy, "bpm", &format!("{bpm:.2}"), 90.0);
    p.vline(dx - 6.0, cy - 12.0, cy + 12.0, c.border_default);
    dx += display(p, th, dx, cy, "sig", &format!("{}/{}", sig.num, sig.den), 62.0);
    p.vline(dx - 6.0, cy - 12.0, cy + 12.0, c.border_default);
    display(p, th, dx, cy, "loop", if f.doc.loop_range.enabled { "on" } else { "off" }, 60.0);
    icon_button(p, th, Rect::new(panel.x1 + 12.0, cy - 14.0, panel.x1 + 40.0, cy + 14.0), Icon::Pulse, None);
    // right side
    let mut rx = w - 12.0;
    for ic in [Icon::Gear, Icon::Redo, Icon::Undo] {
        icons::draw(p, ic, Rect::new(rx - 22.0, cy - 8.0, rx - 6.0, cy + 8.0), c.text_secondary);
        rx -= 34.0;
    }
    let search = Rect::new(rx - 78.0, cy - 13.0, rx - 6.0, cy + 13.0);
    p.rrect(search, 5.0, c.bg_control);
    p.rrect_stroke(search.inset(-0.5), 5.0, 1.0, c.border_default);
    icons::draw(p, Icon::Search, Rect::new(search.x0 + 8.0, cy - 6.0, search.x0 + 20.0, cy + 6.0), c.text_secondary);
    p.text(search.x1 - 8.0, cy, "cmd k", &TextStyle::mono(10.5, c.text_secondary), Align::Right);
}

/// Draws a whole frame of `w`×`h` logical pixels into `scene`.
pub fn draw(
    scene: &mut Scene,
    fonts: &mut Fonts,
    th: &Theme,
    f: &Frame<'_>,
    w: f64,
    h: f64,
) -> (ShellLayout, ArrangeLayout) {
    let mut p = Painter { scene, fonts };
    let sl = ShellLayout::new(w, h, f.panel_h);
    p.rect(Rect::new(0.0, 0.0, w, h), th.c.bg_base);
    topbar(&mut p, th, f, sl.topbar);
    let al = ArrangeLayout::new(f.doc, f.arrange, sl.arrange);
    p.clip(sl.arrange, |p| arrange::draw(p, th, f.doc, f.arrange, &f.transport, &al));
    p.rrect_stroke(sl.arrange.inset(0.5), 0.0, 1.0, th.c.border_default);
    if sl.panel.height() > 0.0 {
        p.clip(sl.panel, |p| piano_roll::draw(p, th, f.doc, f.arrange.selected_clip, &f.transport, sl.panel));
        p.rrect_stroke(sl.panel.inset(0.5), 0.0, 1.0, th.c.border_default);
    }
    (sl, al)
}
