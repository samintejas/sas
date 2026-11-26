//! The settings window: a floating panel over the app, in the manner of
//! Blender's preferences. Sections down the left, their settings on the
//! right. Changes take effect at once and are kept between runs.

use sat_doc::sat_time::{PPQ, Ticks};
use sat_theme::Theme;
use vello::kurbo::{Point, Rect};

use crate::icons::{self, Icon};
use crate::paint::{Align, Painter, TextStyle};

/// Width of the window.
pub const W: f64 = 680.0;
/// Height of the window.
pub const H: f64 = 440.0;
const NAV_W: f64 = 168.0;
const ROW_H: f64 = 40.0;
const TITLE_H: f64 = 44.0;

/// The window's sections.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Section {
    /// Scale and layout.
    #[default]
    Interface,
    /// The output device.
    Audio,
    /// Snapping and note lengths.
    Editing,
    /// The keyboard shortcuts.
    Keys,
}

impl Section {
    /// Every section, in order.
    pub const ALL: [Section; 4] = [Section::Interface, Section::Audio, Section::Editing, Section::Keys];

    /// Its name.
    pub fn label(self) -> &'static str {
        match self {
            Section::Interface => "interface",
            Section::Audio => "audio",
            Section::Editing => "editing",
            Section::Keys => "keys",
        }
    }

    fn icon(self) -> Icon {
        match self {
            Section::Interface => Icon::Panel,
            Section::Audio => Icon::Pulse,
            Section::Editing => Icon::Pencil,
            Section::Keys => Icon::Keys,
        }
    }
}

/// Interface scales offered as presets.
pub const SCALES: [f64; 5] = [0.8, 0.9, 1.0, 1.1, 1.25];

/// Arrangement snaps offered (`None` is off).
pub fn snaps() -> [(Option<Ticks>, &'static str); 5] {
    [
        (None, "off"),
        (Some(Ticks(PPQ / 4)), "1/16"),
        (Some(Ticks(PPQ)), "beat"),
        (Some(Ticks(PPQ * 4)), "bar"),
        (Some(Ticks(PPQ * 8)), "2 bars"),
    ]
}

/// Note lengths offered for drawing.
pub fn note_lengths() -> [(Ticks, &'static str); 4] {
    [(Ticks(PPQ / 8), "1/32"), (Ticks(PPQ / 4), "1/16"), (Ticks(PPQ / 2), "1/8"), (Ticks(PPQ), "1/4")]
}

/// What the window shows: its section and the values of every setting.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingsView {
    /// The open section.
    pub section: Section,
    /// The interface scale.
    pub ui_scale: f64,
    /// The arrangement's snap.
    pub snap: Option<Ticks>,
    /// The length a drawn note starts at.
    pub note_len: Ticks,
    /// The output device's name ("" when there is none).
    pub device: String,
    /// The output's sample rate in Hz.
    pub sample_rate: f64,
    /// The output's latency in ms.
    pub latency_ms: f64,
}

/// What is under a point in the settings window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SettingsHit {
    /// A section in the list.
    Section(Section),
    /// An interface scale preset.
    Scale(f64),
    /// One scale step down.
    ScaleDown,
    /// One scale step up.
    ScaleUp,
    /// Put the sidebars and the panel back as they were.
    ResetLayout,
    /// An arrangement snap.
    Snap(Option<Ticks>),
    /// A note length for drawing.
    NoteLen(Ticks),
    /// The close button.
    Close,
    /// Somewhere in the window, on nothing.
    Inside,
    /// Outside the window: it closes.
    Outside,
}

/// The settings window's geometry.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingsLayout {
    /// The window.
    pub rect: Rect,
    /// Its close button.
    pub close: Rect,
    /// Its sections.
    pub nav: Vec<(Section, Rect)>,
    /// Every control and what it does.
    pub controls: Vec<(SettingsHit, Rect)>,
    /// Rows: label, value note, y centre.
    rows: Vec<(&'static str, &'static str, f64)>,
}

/// One row of segmented buttons, right-aligned, each `w` wide.
fn segments<T: Copy>(x1: f64, cy: f64, w: f64, items: &[T]) -> Vec<(T, Rect)> {
    let n = items.len() as f64;
    let x0 = x1 - n * w;
    items
        .iter()
        .enumerate()
        .map(|(i, t)| (*t, Rect::new(x0 + i as f64 * w, cy - 12.0, x0 + (i as f64 + 1.0) * w, cy + 12.0)))
        .collect()
}

impl SettingsLayout {
    /// Lays the window out in the middle of `window`.
    pub fn new(view: &SettingsView, window: Rect) -> SettingsLayout {
        let c = window.center();
        let rect = Rect::new(c.x - W / 2.0, c.y - H / 2.0, c.x + W / 2.0, c.y + H / 2.0);
        let close = Rect::new(rect.x1 - 34.0, rect.y0 + 10.0, rect.x1 - 10.0, rect.y0 + 34.0);
        let nav = Section::ALL
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let y = rect.y0 + TITLE_H + 8.0 + i as f64 * 30.0;
                (*s, Rect::new(rect.x0 + 8.0, y, rect.x0 + NAV_W - 8.0, y + 26.0))
            })
            .collect();
        let x1 = rect.x1 - 24.0;
        let row = |i: usize| rect.y0 + TITLE_H + 8.0 + ROW_H / 2.0 + i as f64 * ROW_H;
        let mut controls = Vec::new();
        let mut rows = Vec::new();
        match view.section {
            Section::Interface => {
                rows.push(("interface scale", "ctrl = / ctrl - / ctrl 0", row(0)));
                let presets = segments(x1, row(0), 52.0, &SCALES);
                let x0 = presets[0].1.x0;
                for (s, r) in presets {
                    controls.push((SettingsHit::Scale(s), r));
                }
                let cy = row(0);
                controls.push((SettingsHit::ScaleUp, Rect::new(x0 - 32.0, cy - 12.0, x0 - 8.0, cy + 12.0)));
                controls.push((SettingsHit::ScaleDown, Rect::new(x0 - 60.0, cy - 12.0, x0 - 36.0, cy + 12.0)));
                rows.push(("layout", "sidebars, panel and sizes", row(1)));
                controls.push((SettingsHit::ResetLayout, Rect::new(x1 - 112.0, row(1) - 12.0, x1, row(1) + 12.0)));
            }
            Section::Audio => {
                rows.push(("output device", "", row(0)));
                rows.push(("sample rate", "", row(1)));
                rows.push(("output latency", "", row(2)));
                rows.push(("", "", row(3)));
            }
            Section::Editing => {
                rows.push(("arrangement snap", "", row(0)));
                for (s, r) in segments(x1, row(0), 58.0, &snaps().map(|(s, _)| s)) {
                    controls.push((SettingsHit::Snap(s), r));
                }
                rows.push(("drawn note length", "until you touch a note", row(1)));
                for (l, r) in segments(x1, row(1), 58.0, &note_lengths().map(|(l, _)| l)) {
                    controls.push((SettingsHit::NoteLen(l), r));
                }
            }
            Section::Keys => {}
        }
        SettingsLayout { rect, close, nav, controls, rows }
    }

    /// What is under `pt`.
    pub fn hit(&self, pt: Point) -> SettingsHit {
        if !self.rect.contains(pt) {
            return SettingsHit::Outside;
        }
        if self.close.contains(pt) {
            return SettingsHit::Close;
        }
        if let Some((s, _)) = self.nav.iter().find(|(_, r)| r.contains(pt)) {
            return SettingsHit::Section(*s);
        }
        self.controls.iter().find(|(_, r)| r.contains(pt)).map_or(SettingsHit::Inside, |(h, _)| *h)
    }
}

/// The shortcuts the keys section lists.
pub const KEYS: [(&str, &str); 22] = [
    ("space", "play / stop"),
    ("r", "record"),
    ("l", "loop on / off"),
    ("ctrl k", "command palette"),
    ("ctrl 1 … 5", "session, arrange, grids"),
    ("[  ]", "previous / next track"),
    ("tab", "clip editor / mixer"),
    ("ctrl j", "show / hide the panel"),
    ("ctrl b", "left sidebar"),
    ("ctrl alt b", "right sidebar"),
    ("ctrl space", "maximize the area"),
    ("ctrl =  -  0", "interface scale"),
    ("ctrl ,", "settings"),
    ("ctrl z", "undo (shift: redo)"),
    ("ctrl s", "save (shift: save as)"),
    ("ctrl o", "open"),
    ("ctrl n", "new project"),
    ("ctrl e", "export"),
    ("ctrl d", "duplicate"),
    ("m  s", "mute / solo the track"),
    ("delete", "remove the selection"),
    ("esc", "close menus and windows"),
];

fn label(view: &SettingsView, h: SettingsHit) -> (String, bool) {
    match h {
        SettingsHit::Scale(s) => (format!("{:.0}%", s * 100.0), (view.ui_scale - s).abs() < 0.005),
        SettingsHit::ScaleDown => ("−".into(), false),
        SettingsHit::ScaleUp => ("+".into(), false),
        SettingsHit::ResetLayout => ("reset layout".into(), false),
        SettingsHit::Snap(s) => {
            let l = snaps().iter().find(|(x, _)| *x == s).map_or("", |(_, l)| *l);
            (l.into(), view.snap == s)
        }
        SettingsHit::NoteLen(n) => {
            let l = note_lengths().iter().find(|(x, _)| *x == n).map_or("", |(_, l)| *l);
            (l.into(), view.note_len == n)
        }
        _ => (String::new(), false),
    }
}

/// Draws the window over everything (the app dims behind it) and returns
/// its layout.
pub fn draw(p: &mut Painter<'_>, th: &Theme, view: &SettingsView, window: Rect) -> SettingsLayout {
    let c = &th.c;
    let l = SettingsLayout::new(view, window);
    p.rect(window, sat_theme::Rgba::new(0, 0, 0, 110));
    let r = l.rect;
    p.rrect(r.inflate(1.0, 2.0), 10.0, sat_theme::Rgba::new(0, 0, 0, 60));
    p.rrect(r, 9.0, c.bg_surface);
    // the section list, a step darker
    let nav = Rect::new(r.x0, r.y0, r.x0 + NAV_W, r.y1);
    p.clip(r, |p| p.rect(nav, c.bg_base.mix(c.bg_surface, 0.5)));
    p.vline(nav.x1, r.y0, r.y1, c.border_subtle);
    p.rrect_stroke(r.inset(-0.5), 9.0, 1.0, c.border_strong);
    let cy = r.y0 + TITLE_H / 2.0;
    icons::draw(p, Icon::Gear, Rect::new(r.x0 + 14.0, cy - 7.0, r.x0 + 28.0, cy + 7.0), c.accent_primary);
    p.text(r.x0 + 36.0, cy, "settings", &TextStyle::sans(13.0, c.text_max).medium(), Align::Left);
    p.text(nav.x1 + 24.0, cy, view.section.label(), &TextStyle::sans(13.0, c.text_max).medium(), Align::Left);
    crate::area::icon_button(p, th, l.close, Icon::Close);
    p.hline(nav.x1, r.x1, r.y0 + TITLE_H, c.border_subtle);
    for (s, nr) in &l.nav {
        let on = *s == view.section;
        if on {
            p.rrect(*nr, crate::area::WIDGET_R, c.accent_primary_subtle);
        }
        let col = if on { c.accent_primary } else { c.text_strong };
        let ny = nr.center().y;
        icons::draw(p, s.icon(), Rect::new(nr.x0 + 8.0, ny - 6.0, nr.x0 + 20.0, ny + 6.0), col);
        p.text(nr.x0 + 30.0, ny, s.label(), &TextStyle::sans(12.0, col), Align::Left);
    }
    let x0 = nav.x1 + 24.0;
    let name = TextStyle::sans(12.0, c.text_strong);
    let small = TextStyle::sans(11.0, c.text_secondary);
    for (i, (lab, note, y)) in l.rows.iter().enumerate() {
        if i > 0 {
            p.hline(x0, r.x1 - 24.0, y - ROW_H / 2.0, c.border_subtle);
        }
        if note.is_empty() {
            p.text(x0, *y, lab, &name, Align::Left);
        } else {
            p.text(x0, y - 7.0, lab, &name, Align::Left);
            p.text(x0, y + 8.0, note, &small, Align::Left);
        }
    }
    let value = TextStyle::mono(11.5, c.text_max);
    let rx = r.x1 - 24.0;
    match view.section {
        Section::Audio => {
            let ys: Vec<f64> = l.rows.iter().map(|(_, _, y)| *y).collect();
            let dev = if view.device.is_empty() { "no output" } else { view.device.as_str() };
            let dw = (p.fonts.measure(dev, &value) as f64).min(280.0);
            p.text_fit(rx - dw, ys[0], dev, &value, 280.0);
            let sr = if view.sample_rate > 0.0 { format!("{:.0} Hz", view.sample_rate) } else { "-".into() };
            p.text(rx, ys[1], &sr, &value, Align::Right);
            let lat = if view.latency_ms > 0.0 { format!("{:.1} ms", view.latency_ms) } else { "-".into() };
            p.text(rx, ys[2], &lat, &value, Align::Right);
            p.text(x0, ys[3], "the system's default output is used", &small, Align::Left);
        }
        Section::Keys => {
            let top = r.y0 + TITLE_H + 16.0;
            let col_w = (r.x1 - 24.0 - x0) / 2.0;
            let per = KEYS.len().div_ceil(2);
            for (i, (k, what)) in KEYS.iter().enumerate() {
                let x = x0 + (i / per) as f64 * col_w;
                let y = top + (i % per) as f64 * 31.0 + 10.0;
                let kw = (k.chars().count() as f64 * 6.9 + 14.0).max(30.0);
                let kr = Rect::new(x, y - 10.0, x + kw, y + 10.0);
                crate::area::widget(p, th, kr, false);
                p.text(kr.center().x, y, k, &TextStyle::mono(10.5, c.text_max), Align::Center);
                p.text_fit(x + kw + 10.0, y, what, &name, col_w - kw - 18.0);
            }
        }
        _ => {}
    }
    for (h, cr) in &l.controls {
        let (text, on) = label(view, *h);
        crate::area::button(p, th, *cr, &text, on);
    }
    l
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(section: Section) -> SettingsView {
        SettingsView {
            section,
            ui_scale: 0.9,
            snap: Some(Ticks(PPQ)),
            note_len: Ticks(PPQ / 4),
            device: "out".into(),
            sample_rate: 48000.0,
            latency_ms: 5.0,
        }
    }

    #[test]
    fn every_control_hits_and_the_outside_closes() {
        let window = Rect::new(0.0, 0.0, 1440.0, 900.0);
        for s in Section::ALL {
            let l = SettingsLayout::new(&view(s), window);
            assert_eq!(l.hit(l.nav[1].1.center()), SettingsHit::Section(Section::Audio));
            for (h, r) in &l.controls {
                assert_eq!(l.hit(r.center()), *h, "{s:?}");
                assert!(l.rect.contains(r.origin()), "{h:?} inside the window");
            }
            assert_eq!(l.hit(Point::new(5.0, 5.0)), SettingsHit::Outside);
            assert_eq!(l.hit(l.close.center()), SettingsHit::Close);
        }
        let ui = SettingsLayout::new(&view(Section::Interface), window);
        assert!(ui.controls.iter().any(|(h, _)| *h == SettingsHit::Scale(1.25)));
    }
}
