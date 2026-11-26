//! The app shell: top bar, the main view, and the docked areas around it
//! (bottom panel, left and right sidebars; see [`crate::dock`]).

use sat_doc::sat_time::{PPQ, Ticks};
use sat_doc::{Document, GridId, GridKind};
use sat_theme::{Rgba, Theme};
use vello::Scene;
use vello::kurbo::{Point, Rect};

use crate::area::{EditorType, Slot};
use crate::arrange::{self, ArrangeLayout, ArrangeState, TransportView};
use crate::dock::{self, Dock, DockButton, GAP, GRAB, Handle, Side, Zoom};
use crate::grid::{self, GridLayout, GridView};
use crate::icons::{self, Icon};
use crate::mixer::{self, Levels, MixerLayout};
use crate::paint::{Align, Fonts, Painter, TextStyle};
use crate::panel::PanelView;
use crate::piano_roll::{self, RollGeom, RollView};
use crate::steps::{self, StepGeom, StepView};

/// Height of the top bar.
pub const TOPBAR_H: f64 = 34.0;
/// Width of the icon strips at the window's left and right edges.
pub const STRIP_W: f64 = 32.0;
/// Height of the status bar along the bottom.
pub const STATUS_H: f64 = 22.0;

/// What an icon in an edge strip opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StripTarget {
    /// A sidebar.
    Side(Side),
    /// The bottom panel.
    Panel,
}

/// The icons in the edge strips: each opens an editor in its space (or
/// closes it when it is already open there), like Blender's editor tabs.
/// The left strip opens editors in the left sidebar and, at its foot, the
/// bottom panel; the right strip opens editors in the right sidebar.
pub fn strip_buttons(sl: &ShellLayout) -> Vec<(StripTarget, EditorType, Rect)> {
    let b = |r: Rect, k: f64, from_bottom: bool| {
        let y = if from_bottom { r.y1 - 6.0 - (k + 1.0) * 30.0 } else { r.y0 + 6.0 + k * 30.0 };
        Rect::new(r.x0 + 4.0, y, r.x1 - 4.0, y + 26.0)
    };
    let sides = [EditorType::Bank, EditorType::Outliner, EditorType::Inspector];
    let mut out = Vec::new();
    for (k, ty) in sides.iter().enumerate() {
        out.push((StripTarget::Side(Side::Left), *ty, b(sl.strip_left, k as f64, false)));
        out.push((StripTarget::Side(Side::Right), *ty, b(sl.strip_right, k as f64, false)));
    }
    for (k, ty) in [EditorType::Mixer, EditorType::Clip].iter().enumerate() {
        out.push((StripTarget::Panel, *ty, b(sl.strip_left, k as f64, true)));
    }
    out
}

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
        (b, Rect::new(x + 5.0, cy - 10.0, x + 29.0, cy + 10.0))
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

/// A clip dragged out of the bank: the clip, the pointer, and where it
/// would land (and whether it can).
pub type DragClip = (sat_doc::ClipId, vello::kurbo::Point, Option<(Rect, bool)>);

/// The top bar's layout toggles: left sidebar, bottom panel, right sidebar.
pub fn dock_buttons(topbar: Rect) -> [(DockButton, Rect); 3] {
    let rx = topbar.x1 - 12.0 - 3.0 * 34.0 - 10.0;
    let cy = topbar.center().y;
    let mut k = 3.0;
    [DockButton::Left, DockButton::Panel, DockButton::Right].map(|b| {
        let x1 = rx - (k - 1.0) * 28.0;
        k -= 1.0;
        (b, Rect::new(x1 - 24.0, cy - 11.0, x1, cy + 11.0))
    })
}

/// The top bar's search box, which opens the command palette.
pub fn search_box(topbar: Rect) -> Rect {
    let rx = dock_buttons(topbar)[0].1.x0 - 10.0;
    let cy = topbar.center().y;
    Rect::new(rx - 72.0, cy - 11.0, rx, cy + 11.0)
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
    /// Which docked areas show, and their sizes.
    pub dock: Dock,
    /// Vertical scroll of the inspector.
    pub macro_scroll: f64,
    /// The area with focus (where the last press was).
    pub focus: Slot,
    /// The area whose editor-type menu is open.
    pub menu: Option<Slot>,
    /// Vertical scroll of the outliner.
    pub outline_scroll: f64,
    /// The inspector's folded sections (see [`crate::macros::Fold`]).
    pub macro_folds: u8,
    /// The pointer, for the status bar's hints.
    pub pointer: Point,
    /// The audio device, for the status bar.
    pub status: &'a str,
    /// The interface scale, for the status bar.
    pub ui_scale: f64,
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
    /// The plugins plugin modules host.
    pub plugins: &'a crate::grid::PluginViews,
    /// What the session is playing.
    pub session: &'a crate::session::SessionView,
    /// The command palette, when open.
    pub palette: Option<&'a crate::palette::PaletteView>,
    /// The clip bank's scroll and selection.
    pub bank: &'a crate::bank::BankView,
    /// A clip dragged out of the bank: the clip, the pointer, and where it
    /// would land (and whether it can).
    pub drag_clip: Option<DragClip>,
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
    /// The inspector's macros, when the right sidebar shows.
    pub macros: Option<crate::macros::MacroLayout>,
    /// The session view, when shown.
    pub session: Option<crate::session::SessionLayout>,
    /// The command palette, when open.
    pub palette: Option<crate::palette::PaletteLayout>,
    /// The editor-type menu, when open.
    pub menu: Option<crate::area::MenuLayout>,
    /// The outliner, when a sidebar shows it.
    pub outline: Option<crate::outliner::OutlineLayout>,
    /// The clip bank, when a sidebar shows it.
    pub bank: Option<crate::bank::BankLayout>,
}

/// Rectangles of the shell's regions.
#[derive(Debug, Clone, Copy)]
pub struct ShellLayout {
    /// Top bar.
    pub topbar: Rect,
    /// The main view (arrange, session or a grid); zero height while the
    /// bottom panel is maximized.
    pub arrange: Rect,
    /// Bottom panel; just its tab bar when collapsed, zero height while the
    /// main view is zoomed.
    pub panel: Rect,
    /// Left sidebar (clip bank), when it shows.
    pub left: Option<Rect>,
    /// Right sidebar (inspector), when it shows.
    pub right: Option<Rect>,
    /// The icon strip at the left edge.
    pub strip_left: Rect,
    /// The icon strip at the right edge.
    pub strip_right: Rect,
    /// The status bar along the bottom.
    pub status: Rect,
    /// The dock the layout was made from.
    pub dock: Dock,
}

impl ShellLayout {
    /// Splits a window of `w`×`h` by `dock`.
    pub fn new(w: f64, h: f64, dock: &Dock) -> ShellLayout {
        let topbar = Rect::new(0.0, 0.0, w, TOPBAR_H);
        let status = Rect::new(0.0, h - STATUS_H, w, h);
        let strip_left = Rect::new(0.0, TOPBAR_H, STRIP_W, status.y0);
        let strip_right = Rect::new(w - STRIP_W, TOPBAR_H, w, status.y0);
        let (y0, y1) = (TOPBAR_H + GAP, status.y0 - GAP);
        let (x0, x1) = (strip_left.x1, strip_right.x0);
        // the main view keeps some width when both sidebars are wide
        let room = (x1 - x0 - 2.0 * GAP - 360.0).max(0.0);
        let mut lw = if dock.side_open(Side::Left) { dock.left_w } else { 0.0 };
        let mut rw = if dock.side_open(Side::Right) { dock.right_w } else { 0.0 };
        if lw + rw > room && lw + rw > 0.0 {
            let k = room / (lw + rw);
            lw *= k;
            rw *= k;
        }
        let left = (lw > 0.0).then(|| Rect::new(x0, y0, x0 + lw, y1));
        let right = (rw > 0.0).then(|| Rect::new(x1 - rw, y0, x1, y1));
        let cx0 = left.map_or(x0, |r| r.x1 + GAP);
        let cx1 = right.map_or(x1, |r| r.x0 - GAP);
        let (arrange, panel) = match dock.zoom {
            Some(Zoom::Main) => (Rect::new(cx0, y0, cx1, y1), Rect::new(cx0, y1, cx1, y1)),
            Some(Zoom::Panel) => (Rect::new(cx0, y0, cx1, y0), Rect::new(cx0, y0, cx1, y1)),
            None => {
                let ph = if dock.panel_open {
                    dock.panel_h.min(y1 - y0 - dock::MAIN_MIN).max(crate::panel::TABS_H)
                } else {
                    crate::panel::TABS_H
                };
                let panel = Rect::new(cx0, y1 - ph, cx1, y1);
                (Rect::new(cx0, y0, cx1, panel.y0 - GAP), panel)
            }
        };
        ShellLayout { topbar, arrange, panel, left, right, strip_left, strip_right, status, dock: *dock }
    }

    /// The rectangle of an area, when it shows.
    pub fn slot_rect(&self, slot: Slot) -> Option<Rect> {
        match slot {
            Slot::Main => self.main_shown().then_some(self.arrange),
            Slot::Panel => (self.panel.height() > 0.0).then_some(self.panel),
            Slot::Left => self.left,
            Slot::Right => self.right,
        }
    }

    /// The area under `pt`.
    pub fn slot_at(&self, pt: Point) -> Option<Slot> {
        [Slot::Left, Slot::Right, Slot::Panel, Slot::Main]
            .into_iter()
            .find(|s| self.slot_rect(*s).is_some_and(|r| r.contains(pt)))
    }

    /// The bottom panel shows its editor (not collapsed or hidden).
    pub fn panel_open(&self) -> bool {
        self.panel.height() > crate::panel::TABS_H
    }

    /// The main view shows.
    pub fn main_shown(&self) -> bool {
        self.arrange.height() > 0.0
    }

    /// The resize handle under `pt`, if any.
    pub fn handle_at(&self, pt: Point) -> Option<Handle> {
        if let Some(l) = self.left
            && (l.x1 - GRAB..l.x1 + GAP + GRAB).contains(&pt.x)
            && (l.y0..l.y1).contains(&pt.y)
        {
            return Some(Handle::Left);
        }
        if let Some(r) = self.right
            && (r.x0 - GAP - GRAB..r.x0 + GRAB).contains(&pt.x)
            && (r.y0..r.y1).contains(&pt.y)
        {
            return Some(Handle::Right);
        }
        let p = self.panel;
        if self.dock.zoom != Some(Zoom::Main)
            && (p.x0..p.x1).contains(&pt.x)
            && (p.y0 - GAP - GRAB..p.y0 + GRAB).contains(&pt.y)
        {
            return Some(Handle::Panel);
        }
        None
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
    p.text(x, cy - 6.5, label, &TextStyle::mono(9.5, c.text_secondary), Align::Left);
    p.text(x, cy + 6.0, value, &TextStyle::mono(12.5, c.accent_primary).medium(), Align::Left);
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
    let group = Rect::new(x, cy - 13.0, x + 136.0, cy + 13.0);
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
    let panel = Rect::new(x, cy - 14.0, x + 344.0, cy + 14.0);
    p.rrect(panel, 6.0, c.bg_surface);
    p.rrect_stroke(panel.inset(-0.5), 6.0, 1.0, c.border_default);
    let mut dx = x + 10.0;
    dx += display(p, th, dx, cy, "pos", &format_pos(f.doc, t.playhead), 96.0);
    p.vline(dx - 6.0, cy - 10.0, cy + 10.0, c.border_default);
    dx += display(p, th, dx, cy, "bpm", &format!("{bpm:.2}"), 90.0);
    p.vline(dx - 6.0, cy - 10.0, cy + 10.0, c.border_default);
    dx += display(p, th, dx, cy, "sig", &format!("{}/{}", sig.num, sig.den), 62.0);
    p.vline(dx - 6.0, cy - 10.0, cy + 10.0, c.border_default);
    display(p, th, dx, cy, "loop", if f.doc.loop_range.enabled { "on" } else { "off" }, 60.0);
    icon_button(p, th, Rect::new(panel.x1 + 12.0, cy - 12.0, panel.x1 + 36.0, cy + 12.0), Icon::Pulse, None);
    // right side
    let mut rx = w - 12.0;
    for ic in [Icon::Gear, Icon::Redo, Icon::Undo] {
        icons::draw(p, ic, Rect::new(rx - 22.0, cy - 8.0, rx - 6.0, cy + 8.0), c.text_secondary);
        rx -= 34.0;
    }
    for (b, br) in dock_buttons(r) {
        let on = match b {
            DockButton::Left => f.dock.side_open(Side::Left),
            DockButton::Right => f.dock.side_open(Side::Right),
            DockButton::Panel => f.dock.zoom != Some(Zoom::Main) && f.dock.panel_open,
        };
        dock::draw_glyph(p, b, br, on, if on { c.text_strong } else { c.text_secondary });
    }
    let search = search_box(r);
    p.rrect(search, 5.0, c.bg_control);
    p.rrect_stroke(search.inset(-0.5), 5.0, 1.0, c.border_default);
    icons::draw(p, Icon::Search, Rect::new(search.x0 + 8.0, cy - 6.0, search.x0 + 20.0, cy + 6.0), c.text_secondary);
    p.text(
        search.x1 - 8.0,
        cy,
        if cfg!(target_os = "macos") { "cmd k" } else { "ctrl k" },
        &TextStyle::mono(10.5, c.text_secondary),
        Align::Right,
    );
}

/// Draws a whole frame of `w`×`h` logical pixels into `scene`.
pub fn draw(scene: &mut Scene, fonts: &mut Fonts, th: &Theme, f: &Frame<'_>, w: f64, h: f64) -> Drawn {
    let mut p = Painter { scene, fonts };
    let sl = ShellLayout::new(w, h, &f.dock);
    p.rect(Rect::new(0.0, 0.0, w, h), th.c.bg_base);
    topbar(&mut p, th, f, sl.topbar);
    let al = ArrangeLayout::new(f.doc, f.arrange, sl.arrange);
    let mut gl = None;
    let mut ssl = None;
    match f.view {
        _ if !sl.main_shown() => {}
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
                gl = grid::draw(
                    p,
                    th,
                    f.doc,
                    g,
                    f.grid,
                    f.arrange.selected_clip,
                    f.sounds,
                    f.plugins,
                    f.levels,
                    sl.arrange,
                )
            });
        }
        MainView::Arrange => {
            p.clip(sl.arrange, |p| arrange::draw(p, th, f.doc, f.arrange, &f.transport, f.levels, f.sounds, &al));
            if !f.session.following.is_empty() {
                crate::session::draw_back(&mut p, th, sl.arrange);
            }
        }
    }
    if sl.main_shown() {
        crate::area::finish(&mut p, th, sl.arrange);
    }
    let mut ml = None;
    let mut rl = None;
    let mut ae = None;
    let mut sg = None;
    let mut mc = None;
    let mut bk = None;
    let mut ol = None;
    let on_screen = match f.view {
        MainView::Grid(g) => Some(g),
        MainView::Arrange | MainView::Session => None,
    };
    if sl.panel.height() > 0.0 && !sl.panel_open() {
        // collapsed: the header, to click and bring it back
        p.clip(sl.panel, |p| {
            let (bar, x) = crate::panel::draw_bar(p, th, sl.panel, f.panel);
            let hint = if cfg!(target_os = "macos") { "cmd j: open" } else { "ctrl j: open" };
            p.text(x, bar.center().y, hint, &TextStyle::mono(11.0, th.c.text_secondary), Align::Left);
            dock::draw_panel_buttons(p, th, sl.panel, &f.dock);
        });
        crate::area::finish(&mut p, th, sl.panel);
    } else if sl.panel.height() > 0.0 {
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
        });
        p.clip(sl.panel, |p| dock::draw_panel_buttons(p, th, sl.panel, &f.dock));
        crate::area::finish(&mut p, th, sl.panel);
    }
    for (side, r) in [(Side::Left, sl.left), (Side::Right, sl.right)] {
        let Some(r) = r else { continue };
        match f.dock.side_shows(side) {
            EditorType::Inspector => {
                let track = f.arrange.selected_track;
                let (scroll, folds) = (f.macro_scroll, f.macro_folds);
                p.clip(r, |p| {
                    mc = Some(crate::macros::draw(p, th, f.doc, track, on_screen, f.learn, scroll, folds, r))
                });
            }
            EditorType::Outliner => {
                let (folded, sel, scroll) = (&f.arrange.collapsed, f.arrange.selected_track, f.outline_scroll);
                p.clip(r, |p| ol = Some(crate::outliner::draw(p, th, f.doc, folded, sel, scroll, r)));
            }
            _ => p.clip(r, |p| bk = Some(crate::bank::draw(p, th, f.doc, f.bank, r))),
        }
        crate::area::finish(&mut p, th, r);
    }
    strips(&mut p, th, f, &sl);
    status_bar(&mut p, th, f, &sl);
    // the area with focus: where keys go
    if let Some(r) = sl.slot_rect(f.focus).filter(|r| r.height() > 0.0) {
        crate::area::focus(&mut p, th, r);
    }
    if let Some((clip, at, target)) = f.drag_clip {
        if let Some((r, ok)) = target {
            let col = if ok { th.c.accent_primary } else { th.c.accent_danger };
            p.rrect(r, 4.0, col.alpha(0.14));
            p.rrect_stroke(r.inset(-0.5), 4.0, 1.5, col);
        }
        let ghost =
            Rect::new(at.x - 20.0, at.y - 12.0, at.x - 20.0 + crate::bank::CARD_W, at.y - 12.0 + crate::bank::CARD_H);
        crate::bank::draw_card(&mut p, th, f.doc, clip, ghost, true);
    }
    let menu = f.menu.and_then(|slot| {
        let r = sl.slot_rect(slot).filter(|r| r.height() > 0.0)?;
        let ty = editor_in(f.doc, f.view, f.panel, &f.dock, slot);
        let m = crate::area::MenuLayout::new(slot, crate::area::pill_rect(r, ty), Rect::new(0.0, 0.0, w, h));
        crate::area::draw_menu(&mut p, th, &m, ty);
        Some(m)
    });
    let pl = f.palette.map(|v| crate::palette::draw(&mut p, th, f.doc, v, Rect::new(0.0, 0.0, w, h)));
    Drawn {
        shell: sl,
        arrange: al,
        mixer: ml,
        roll: rl,
        audio: ae,
        steps: sg,
        grid: gl,
        macros: mc,
        session: ssl,
        palette: pl,
        bank: bk,
        outline: ol,
        menu,
    }
}

/// Draws the edge strips' icons, the open ones lit.
fn strips(p: &mut Painter<'_>, th: &Theme, f: &Frame<'_>, sl: &ShellLayout) {
    let c = &th.c;
    for r in [sl.strip_left, sl.strip_right] {
        p.rect(r, c.bg_base);
    }
    let d = &f.dock;
    for (target, ty, r) in strip_buttons(sl) {
        let on = match target {
            StripTarget::Side(s) => d.side_open(s) && d.side_shows(s) == ty,
            StripTarget::Panel => sl.panel_open() && f.panel.editor() == ty,
        };
        if on {
            p.rrect(r, 6.0, c.bg_raised);
            let bar = match target {
                StripTarget::Side(Side::Right) => Rect::new(r.x1 + 2.0, r.y0 + 6.0, r.x1 + 4.0, r.y1 - 6.0),
                _ => Rect::new(r.x0 - 4.0, r.y0 + 6.0, r.x0 - 2.0, r.y1 - 6.0),
            };
            p.rect(bar, c.accent_primary);
        }
        let col = if on { c.text_max } else { c.text_secondary };
        icons::draw(p, ty.icon(), Rect::from_center_size(r.center(), (15.0, 15.0)), col);
    }
    // a rule between the sidebar editors and the bottom panel's
    let y = strip_buttons(sl)
        .iter()
        .filter(|(t, _, _)| *t == StripTarget::Panel)
        .map(|(_, _, r)| r.y0)
        .fold(f64::MAX, f64::min);
    p.hline(sl.strip_left.x0 + 9.0, sl.strip_left.x1 - 9.0, y - 6.0, c.border_default);
}

/// What the status bar says about the area under the pointer.
fn hint(ty: EditorType) -> &'static str {
    match ty {
        EditorType::Arrange => {
            "click a clip to edit it · drag to move, edges to trim · alt drag copies · double-click a lane draws"
        }
        EditorType::Session => "click a slot to launch it · a scene launches its row · drag clips in from the bank",
        EditorType::SourceGrid | EditorType::InstrumentGrid | EditorType::FxGrid => {
            "drag from an out port to an in port of the same colour · drag a wire off to unplug · delete removes a module"
        }
        EditorType::Clip => {
            "click to draw · drag notes to move, edges to resize · shift arrows: octave · double-click deletes"
        }
        EditorType::Mixer => "drag faders and knobs · shift: fine · double-click resets · src / ins / fx open grids",
        EditorType::Bank => "drag a clip onto its lane or a slot · double-click places it at the playhead · + new clip",
        EditorType::Inspector => "drag a macro · learn maps the next touched parameter · click a heading to fold it",
        EditorType::Outliner => "click selects · double-click opens the track · chevrons fold groups · m / s",
    }
}

/// Draws the status bar: hints for the area under the pointer, then the
/// project and the audio device.
fn status_bar(p: &mut Painter<'_>, th: &Theme, f: &Frame<'_>, sl: &ShellLayout) {
    let c = &th.c;
    let r = sl.status;
    p.rect(r, c.bg_base);
    p.hline(r.x0, r.x1, r.y0, c.border_subtle);
    let cy = r.center().y;
    let st = TextStyle::mono(10.5, c.text_secondary);
    let keys = "ctrl space maximize · ctrl j panel · ctrl k commands";
    let tracks = f.doc.tracks.values().filter(|t| t.kind == sat_doc::TrackKind::Track).count();
    let right = format!("{tracks} tracks · {} clips · {} · {:.0}%", f.doc.clips.len(), f.status, f.ui_scale * 100.0);
    let rw = p.text(r.x1 - 12.0, cy, &right, &st, Align::Right);
    let slot = sl.slot_at(f.pointer).unwrap_or(f.focus);
    let ty = editor_in(f.doc, f.view, f.panel, &f.dock, slot);
    let x = r.x0 + 12.0;
    let lw = p.text(x, cy, ty.label(), &TextStyle::mono(10.5, c.accent_primary), Align::Left);
    let room = r.x1 - 24.0 - rw - (x + lw + 10.0);
    let text = format!("{} · {keys}", hint(ty));
    p.text_fit(x + lw + 10.0, cy, &text, &st, room);
}

/// What the main view shows, as an editor type.
pub fn main_editor(doc: &Document, view: MainView) -> EditorType {
    match view {
        MainView::Arrange => EditorType::Arrange,
        MainView::Session => EditorType::Session,
        MainView::Grid(g) => doc.grids.get(&g).map_or(EditorType::SourceGrid, |g| grid::editor_type(g.kind)),
    }
}

/// What the area `slot` shows.
pub fn editor_in(doc: &Document, view: MainView, panel: PanelView, d: &Dock, slot: Slot) -> EditorType {
    match slot {
        Slot::Main => main_editor(doc, view),
        Slot::Panel => panel.editor(),
        Slot::Left => d.side_shows(Side::Left),
        Slot::Right => d.side_shows(Side::Right),
    }
}

/// Which editor the bottom panel shows, for its height.
pub fn panel_kind(doc: &Document, panel: PanelView, clip: Option<sat_doc::ClipId>) -> dock::PanelKind {
    use dock::PanelKind;
    match panel {
        PanelView::Mixer => PanelKind::Mixer,
        PanelView::Clip => {
            let Some(c) = clip.filter(|c| doc.clips.contains_key(c)) else { return PanelKind::Roll };
            if doc.clips[&c].audio.is_some() {
                PanelKind::Audio
            } else if let Some(h) = steps::fit_height(doc, c) {
                PanelKind::Fit(h)
            } else {
                PanelKind::Roll
            }
        }
    }
}
