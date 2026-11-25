//! The app shell: top bar, arrange view and bottom panel.

use sat_doc::sat_time::{PPQ, Ticks};
use sat_doc::{Document, GridId, GridKind};
use sat_theme::{Rgba, Theme};
use vello::Scene;
use vello::kurbo::Rect;

use crate::arrange::{self, ArrangeLayout, ArrangeState, TransportView};
use crate::grid::{self, GridLayout, GridView};
use crate::icons::{self, Icon};
use crate::mixer::{self, Levels, MixerLayout};
use crate::paint::{Align, Fonts, Painter, TextStyle};
use crate::panel::PanelView;
use crate::piano_roll::{self, RollGeom, RollView};
use crate::steps::{self, StepGeom, StepView};

/// Height of the top bar.
pub const TOPBAR_H: f64 = 40.0;
/// Default height of the bottom panel.
pub const PANEL_H: f64 = 306.0;

/// What the main area shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MainView {
    /// The arrangement.
    #[default]
    Arrange,
    /// One of a track's grids.
    Grid(GridId),
    /// The session's clip launcher.
    Session,
}

/// The view buttons in the top bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewButton {
    /// Session view.
    Session,
    /// Arrange view.
    Arrange,
    /// The selected track's source grid.
    SourceGrid,
    /// The selected track's instrument grid.
    Instrument,
    /// Fx grid (later).
    Fx,
}

/// A transport button in the top bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportButton {
    /// Stop (and back to the start when stopped).
    Stop,
    /// Play or pause.
    Play,
    /// Record onto the armed tracks.
    Record,
    /// Loop on or off.
    Loop,
}

/// Where the top bar's transport buttons are.
pub fn transport_buttons(topbar: Rect) -> [(TransportButton, Rect); 4] {
    use TransportButton::*;
    let cy = topbar.center().y;
    let x0 = topbar.x0 + topbar.width() / 2.0 - 260.0;
    let mut k = 0.0;
    [Stop, Play, Record, Loop].map(|b| {
        let x = x0 + 33.0 * k;
        k += 1.0;
        (b, Rect::new(x + 4.0, cy - 12.0, x + 30.0, cy + 12.0))
    })
}

/// Where the top bar's view buttons are.
pub fn view_buttons(topbar: Rect) -> [(ViewButton, Rect); 5] {
    use ViewButton::*;
    let mut x = 160.0;
    [Session, Arrange, SourceGrid, Instrument, Fx].map(|b| {
        let r = Rect::new(x, topbar.y0, x + 40.0, topbar.y1);
        x += if b == Arrange { 52.0 } else { 40.0 };
        (b, r)
    })
}

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
    /// What the bottom panel shows.
    pub panel: PanelView,
    /// Horizontal scroll of the mixer.
    pub mixer_scroll: f64,
    /// Meter levels per track.
    pub levels: &'a Levels,
    /// Unsaved changes.
    pub edited: bool,
    /// Piano roll selection, tool and grid.
    pub roll: &'a RollView,
    /// Step editor page.
    pub steps: &'a StepView,
    /// What the main area shows.
    pub view: MainView,
    /// Source grid pan, selection and menu.
    pub grid: &'a GridView,
    /// Macro learn mode is on.
    pub learn: bool,
    /// The decoded sounds samplers play.
    pub sounds: &'a sat_disk::Pool,
    /// What the session is playing.
    pub session: &'a crate::session::SessionView,
}

/// Layouts of everything drawn, for hit testing.
#[derive(Debug, Clone)]
pub struct Drawn {
    /// The shell's regions.
    pub shell: ShellLayout,
    /// The arrange view.
    pub arrange: ArrangeLayout,
    /// The mixer, when the panel shows it.
    pub mixer: Option<MixerLayout>,
    /// The clip panel, when it shows a clip.
    pub roll: Option<RollGeom>,
    /// The audio clip editor, when the selected clip is an audio clip.
    pub audio: Option<crate::audio_edit::AudioEditLayout>,
    /// The step editor, when the clip's source is a drum seq or euclid.
    pub steps: Option<StepGeom>,
    /// The source grid, when shown.
    pub grid: Option<GridLayout>,
    /// The macros page, when the panel shows it.
    pub macros: Option<crate::macros::MacroLayout>,
    /// The session view, when shown.
    pub session: Option<crate::session::SessionLayout>,
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
    let nw = p.text(76.0, cy, f.name, &TextStyle::sans(13.0, c.text_secondary), Align::Left);
    if f.edited {
        p.circle(vello::kurbo::Point::new(76.0 + nw + 8.0, cy), 3.0, c.text_secondary);
    }
    let active = match f.view {
        MainView::Arrange => ViewButton::Arrange,
        MainView::Session => ViewButton::Session,
        MainView::Grid(g) => match f.doc.grids.get(&g).map(|g| g.kind) {
            Some(GridKind::Instrument) => ViewButton::Instrument,
            Some(GridKind::Fx) => ViewButton::Fx,
            _ => ViewButton::SourceGrid,
        },
    };
    let icons = [Icon::Grid, Icon::Lines, Icon::Panel, Icon::Gear, Icon::Sliders];
    for ((b, br), ic) in view_buttons(r).into_iter().zip(icons) {
        let on = b == active;
        icons::draw(p, ic, br.inset(-12.0), if on { c.text_max } else { c.text_secondary });
        if on {
            p.rect(Rect::new(br.x0 + 4.0, r.y1 - 2.0, br.x1 - 4.0, r.y1), c.accent_primary);
        }
        if b == ViewButton::Arrange {
            p.vline(br.x1 + 6.0, cy - 9.0, cy + 9.0, c.border_default);
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
    for ((ic, state), (_, br)) in buttons.into_iter().zip(transport_buttons(r)) {
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
pub fn draw(scene: &mut Scene, fonts: &mut Fonts, th: &Theme, f: &Frame<'_>, w: f64, h: f64) -> Drawn {
    let mut p = Painter { scene, fonts };
    let sl = ShellLayout::new(w, h, f.panel_h);
    p.rect(Rect::new(0.0, 0.0, w, h), th.c.bg_base);
    topbar(&mut p, th, f, sl.topbar);
    let al = ArrangeLayout::new(f.doc, f.arrange, sl.arrange);
    let mut gl = None;
    let mut ssl = None;
    match f.view {
        MainView::Session => {
            p.clip(sl.arrange, |p| {
                ssl = Some(crate::session::draw(
                    p,
                    th,
                    f.doc,
                    f.session,
                    f.arrange.selected_clip,
                    &f.transport,
                    sl.arrange,
                ))
            });
        }
        MainView::Grid(g) => {
            p.clip(sl.arrange, |p| {
                gl = grid::draw(p, th, f.doc, g, f.grid, f.arrange.selected_clip, f.sounds, sl.arrange)
            });
        }
        MainView::Arrange => {
            p.clip(sl.arrange, |p| arrange::draw(p, th, f.doc, f.arrange, &f.transport, f.levels, f.sounds, &al));
            if !f.session.following.is_empty() {
                crate::session::draw_back(&mut p, th, sl.arrange);
            }
        }
    }
    p.rrect_stroke(sl.arrange.inset(0.5), 0.0, 1.0, th.c.border_default);
    let mut ml = None;
    let mut rl = None;
    let mut ae = None;
    let mut sg = None;
    let mut mc = None;
    let on_screen = match f.view {
        MainView::Grid(g) => Some(g),
        MainView::Arrange | MainView::Session => None,
    };
    if sl.panel.height() > 0.0 {
        p.clip(sl.panel, |p| match f.panel {
            PanelView::Clip => {
                let clip = f.arrange.selected_clip;
                let audio = clip.filter(|c| f.doc.clips.get(c).is_some_and(|c| c.audio.is_some()));
                match clip.and_then(|c| steps::source(f.doc, c)).map(|(_, e)| e) {
                    _ if audio.is_some() => {
                        ae = audio.and_then(|c| crate::audio_edit::draw(p, th, f.doc, c, f.sounds, sl.panel))
                    }
                    Some(sat_modules::Editor::Drums | sat_modules::Editor::Euclid | sat_modules::Editor::Turing) => {
                        sg = steps::draw(p, th, f.doc, clip, f.steps, &f.transport, sl.panel)
                    }
                    _ => rl = piano_roll::draw(p, th, f.doc, clip, f.roll, &f.transport, sl.panel),
                }
            }
            PanelView::Mixer => {
                ml = Some(mixer::draw(p, th, f.doc, f.arrange.selected_track, f.levels, f.mixer_scroll, sl.panel))
            }
            PanelView::Macros => {
                let track = f.arrange.selected_track;
                mc = Some(crate::macros::draw(p, th, f.doc, track, on_screen, f.learn, sl.panel))
            }
        });
        p.rrect_stroke(sl.panel.inset(0.5), 0.0, 1.0, th.c.border_default);
    }
    Drawn { shell: sl, arrange: al, mixer: ml, roll: rl, audio: ae, steps: sg, grid: gl, macros: mc, session: ssl }
}
