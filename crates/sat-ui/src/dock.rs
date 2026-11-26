//! Docked panels around the main view, in the manner of VS Code and Blender.
//!
//! - The **bottom panel** holds the editors that gain from room: the clip
//!   editor and the mixer. Drag its top edge to resize it, collapse it to its
//!   tab bar (ctrl j, or its chevron), or maximize it over the main view
//!   (ctrl space over it, or its maximize button).
//! - The **left sidebar** holds the clip bank, a list that reads best tall and
//!   narrow, next to the lanes its clips are dragged onto (ctrl b).
//! - The **right sidebar** is the inspector: the selected track's macros,
//!   fader and pan, following the selection (ctrl alt b).
//!
//! Both sidebars resize from their inner edge and close when dragged narrow.
//! Ctrl space over the main view zooms it to fill the window; again restores.

use sat_theme::{Rgba, Theme};
use vello::kurbo::{Point, Rect};

use crate::icons::{self, Icon};
use crate::paint::{Align, Painter, TextStyle};
use crate::panel::TABS_H;

/// Gap between docked areas; the resize handles sit in it.
pub const GAP: f64 = 4.0;
/// How far either side of a gap a press grabs its handle.
pub const GRAB: f64 = 3.0;
/// Narrowest a sidebar can be; dragged below `SIDE_CLOSE` it closes.
pub const SIDE_MIN: f64 = 220.0;
/// Below this a dragged sidebar closes.
pub const SIDE_CLOSE: f64 = 140.0;
/// Widest a sidebar can be.
pub const SIDE_MAX: f64 = 640.0;
/// Lowest the open bottom panel can be; dragged below `PANEL_CLOSE` it collapses.
pub const PANEL_MIN: f64 = 160.0;
/// Below this a dragged panel collapses to its tab bar.
pub const PANEL_CLOSE: f64 = 90.0;
/// The main view keeps at least this much height under a dragged panel.
pub const MAIN_MIN: f64 = 140.0;
/// Default bottom panel height.
pub const PANEL_H: f64 = 306.0;
/// Default left sidebar width.
pub const LEFT_W: f64 = 264.0;
/// Default right sidebar width.
pub const RIGHT_W: f64 = 316.0;

/// A sidebar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// The clip bank.
    Left,
    /// The inspector (macros).
    Right,
}

/// An area zoomed to fill the space below the top bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zoom {
    /// The bottom panel covers the main view (the sidebars stay).
    Panel,
    /// The main view fills the window; panel and sidebars hide.
    Main,
}

/// A resize handle between areas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handle {
    /// The left sidebar's right edge.
    Left,
    /// The right sidebar's left edge.
    Right,
    /// The bottom panel's top edge.
    Panel,
}

/// Which areas are open and how big they are. Window furniture, not part of
/// the project.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Dock {
    /// The left sidebar (clip bank) shows.
    pub left_open: bool,
    /// Its width.
    pub left_w: f64,
    /// The right sidebar (inspector) shows.
    pub right_open: bool,
    /// Its width.
    pub right_w: f64,
    /// The bottom panel is open; collapsed, only its tab bar shows.
    pub panel_open: bool,
    /// Its height when open.
    pub panel_h: f64,
    /// An area zoomed to fill the window.
    pub zoom: Option<Zoom>,
}

impl Default for Dock {
    fn default() -> Dock {
        Dock {
            left_open: true,
            left_w: LEFT_W,
            right_open: false,
            right_w: RIGHT_W,
            panel_open: true,
            panel_h: PANEL_H,
            zoom: None,
        }
    }
}

impl Dock {
    /// Shows or hides a sidebar. Leaves a zoom of the main view, so the
    /// sidebar asked for appears.
    pub fn toggle_side(&mut self, s: Side) {
        if self.zoom == Some(Zoom::Main) {
            self.zoom = None;
            self.set_side(s, true);
            return;
        }
        let open = !self.side_open(s);
        self.set_side(s, open);
    }

    /// Opens or closes a sidebar.
    pub fn set_side(&mut self, s: Side, open: bool) {
        match s {
            Side::Left => self.left_open = open,
            Side::Right => self.right_open = open,
        }
    }

    /// Whether a sidebar shows (a zoomed main view hides both).
    pub fn side_open(&self, s: Side) -> bool {
        self.zoom != Some(Zoom::Main)
            && match s {
                Side::Left => self.left_open,
                Side::Right => self.right_open,
            }
    }

    /// Collapses the bottom panel to its tab bar, or opens it again.
    pub fn toggle_panel(&mut self) {
        if self.zoom.is_some() {
            // out of either zoom, back to an open panel
            self.zoom = None;
            self.panel_open = true;
            return;
        }
        self.panel_open = !self.panel_open;
    }

    /// Opens the bottom panel (picking a tab does).
    pub fn open_panel(&mut self) {
        if self.zoom == Some(Zoom::Main) {
            self.zoom = None;
        }
        self.panel_open = true;
    }

    /// Zooms an area, or restores the layout when it is already zoomed.
    pub fn toggle_zoom(&mut self, z: Zoom) {
        if self.zoom.is_some() {
            self.zoom = None;
        } else {
            self.zoom = Some(z);
            if z == Zoom::Panel {
                self.panel_open = true;
            }
        }
    }

    /// Sets the size a handle controls from the pointer, for a window of
    /// `w`×`h`; dragged far enough, the area closes.
    pub fn drag(&mut self, handle: Handle, pt: Point, w: f64, h: f64) {
        match handle {
            Handle::Left => {
                let x = pt.x - GAP;
                self.left_open = x >= SIDE_CLOSE;
                if self.left_open {
                    self.left_w = x.clamp(SIDE_MIN, SIDE_MAX.min(w * 0.45));
                }
            }
            Handle::Right => {
                let x = w - GAP - pt.x;
                self.right_open = x >= SIDE_CLOSE;
                if self.right_open {
                    self.right_w = x.clamp(SIDE_MIN, SIDE_MAX.min(w * 0.45));
                }
            }
            Handle::Panel => {
                if self.zoom == Some(Zoom::Panel) {
                    // dragging a maximized panel's edge brings the main view back
                    self.zoom = None;
                }
                let ph = h - GAP - pt.y;
                self.panel_open = ph >= PANEL_CLOSE;
                if self.panel_open {
                    self.panel_h = ph.clamp(PANEL_MIN, (h - crate::shell::TOPBAR_H - MAIN_MIN).max(PANEL_MIN));
                }
            }
        }
    }
}

/// The bottom panel's own buttons, at the right of its tab bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelButton {
    /// Maximize over the main view, or restore.
    Maximize,
    /// Collapse to the tab bar, or open.
    Collapse,
}

/// Where the bottom panel's buttons are, for a panel at `r`.
pub fn panel_buttons(r: Rect) -> [(PanelButton, Rect); 2] {
    let cy = r.y0 + TABS_H / 2.0;
    let b = |x1: f64| Rect::new(x1 - 26.0, cy - 13.0, x1, cy + 13.0);
    [(PanelButton::Maximize, b(r.x1 - 38.0)), (PanelButton::Collapse, b(r.x1 - 8.0))]
}

/// A sidebar's close button, for a sidebar at `r`.
pub fn close_button(r: Rect) -> Rect {
    let cy = r.y0 + TABS_H / 2.0;
    Rect::new(r.x1 - 34.0, cy - 13.0, r.x1 - 8.0, cy + 13.0)
}

/// The layout toggles in the top bar, left of the search box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockButton {
    /// The left sidebar.
    Left,
    /// The bottom panel.
    Panel,
    /// The right sidebar.
    Right,
}

/// Draws a small layout glyph: a window outline with the toggled area
/// filled when it shows.
pub fn draw_glyph(p: &mut Painter<'_>, b: DockButton, r: Rect, on: bool, c: Rgba) {
    let w = Rect::from_center_size(r.center(), (16.0, 13.0));
    p.rrect_stroke(w, 2.5, 1.0, c);
    let part = match b {
        DockButton::Left => Rect::new(w.x0, w.y0, w.x0 + 5.0, w.y1),
        DockButton::Panel => Rect::new(w.x0, w.y1 - 4.5, w.x1, w.y1),
        DockButton::Right => Rect::new(w.x1 - 5.0, w.y0, w.x1, w.y1),
    };
    if on {
        p.rrect(part, 2.0, c);
    } else {
        match b {
            DockButton::Left => p.vline(part.x1, w.y0, w.y1, c),
            DockButton::Panel => p.hline(w.x0, w.x1, part.y0, c),
            DockButton::Right => p.vline(part.x0, w.y0, w.y1, c),
        }
    }
}

/// A small square button with a hover-free outline.
pub fn small_button(p: &mut Painter<'_>, th: &Theme, r: Rect) {
    p.rrect(r, 5.0, th.c.bg_control);
    p.rrect_stroke(r.inset(-0.5), 5.0, 1.0, th.c.border_default);
}

/// Draws the bottom panel's maximize and collapse buttons over its tab bar.
pub fn draw_panel_buttons(p: &mut Painter<'_>, th: &Theme, r: Rect, dock: &Dock) {
    let c = &th.c;
    for (b, br) in panel_buttons(r) {
        small_button(p, th, br);
        let k = br.center();
        match b {
            PanelButton::Maximize if dock.zoom == Some(Zoom::Panel) => {
                // restore: two overlapping windows
                p.rrect_stroke(Rect::new(k.x - 5.0, k.y - 2.0, k.x + 3.0, k.y + 5.0), 1.5, 1.0, c.text_strong);
                p.polyline(
                    &[
                        Point::new(k.x - 2.0, k.y - 3.5),
                        Point::new(k.x - 2.0, k.y - 5.0),
                        Point::new(k.x + 5.5, k.y - 5.0),
                        Point::new(k.x + 5.5, k.y + 2.0),
                        Point::new(k.x + 4.0, k.y + 2.0),
                    ],
                    1.0,
                    c.text_strong,
                );
            }
            PanelButton::Maximize => {
                p.rrect_stroke(Rect::from_center_size(k, (11.0, 10.0)), 1.5, 1.0, c.text_strong);
                p.hline(k.x - 5.5, k.x + 5.5, k.y - 2.5, c.text_strong);
            }
            PanelButton::Collapse => {
                let open = dock.panel_open && dock.zoom != Some(Zoom::Main);
                let s = if open { 1.0 } else { -1.0 };
                p.polyline(
                    &[
                        Point::new(k.x - 5.0, k.y - 2.5 * s),
                        Point::new(k.x, k.y + 2.5 * s),
                        Point::new(k.x + 5.0, k.y - 2.5 * s),
                    ],
                    1.4,
                    c.text_strong,
                );
            }
        }
    }
}

/// Draws a sidebar's background and header (title, a dimmer note, close
/// button); the note is clipped at `note_x1`. Returns the header bar.
pub fn side_header(
    p: &mut Painter<'_>,
    th: &Theme,
    r: Rect,
    swatch: Option<Rgba>,
    title: &str,
    note: &str,
    note_x1: f64,
) -> Rect {
    let c = &th.c;
    p.rect(r, c.bg_surface);
    let bar = Rect::new(r.x0, r.y0, r.x1, r.y0 + TABS_H);
    p.hline(r.x0, r.x1, bar.y1 - 1.0, c.border_subtle);
    let cy = bar.center().y;
    let mut x = r.x0 + 12.0;
    if let Some(col) = swatch {
        p.rrect(Rect::new(x, cy - 5.0, x + 10.0, cy + 5.0), 2.0, col);
        x += 18.0;
    }
    x += p.text(x, cy, title, &TextStyle::mono(12.0, c.text_max).medium(), Align::Left) + 10.0;
    let close = close_button(r);
    if note_x1 > x {
        p.clip(Rect::new(x, bar.y0, note_x1, bar.y1), |p| {
            p.text(x, cy, note, &TextStyle::mono(11.0, c.text_secondary), Align::Left);
        });
    }
    icons::draw(p, Icon::Close, close.inset(-8.0), c.text_secondary);
    bar
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dragging_resizes_and_closes() {
        let mut d = Dock::default();
        d.drag(Handle::Left, Point::new(400.0, 300.0), 1440.0, 900.0);
        assert!(d.left_open);
        assert_eq!(d.left_w, 396.0);
        d.drag(Handle::Left, Point::new(60.0, 300.0), 1440.0, 900.0);
        assert!(!d.left_open, "dragged narrow, it closes");
        assert_eq!(d.left_w, 396.0, "and remembers its width");
        d.drag(Handle::Panel, Point::new(500.0, 900.0 - 4.0 - 400.0), 1440.0, 900.0);
        assert_eq!(d.panel_h, 400.0);
        d.drag(Handle::Panel, Point::new(500.0, 100.0), 1440.0, 900.0);
        assert_eq!(d.panel_h, 900.0 - crate::shell::TOPBAR_H - MAIN_MIN, "the main view keeps some room");
        d.drag(Handle::Panel, Point::new(500.0, 880.0), 1440.0, 900.0);
        assert!(!d.panel_open, "dragged low, it collapses");
    }

    #[test]
    fn zoom_toggles_and_panel_keys_restore() {
        let mut d = Dock::default();
        d.toggle_zoom(Zoom::Main);
        assert!(!d.side_open(Side::Left));
        d.toggle_side(Side::Right);
        assert_eq!(d.zoom, None, "asking for a sidebar leaves the zoom");
        assert!(d.side_open(Side::Right) && d.side_open(Side::Left));
        d.panel_open = false;
        d.toggle_zoom(Zoom::Panel);
        assert!(d.panel_open, "maximizing a collapsed panel opens it");
        d.toggle_panel();
        assert_eq!(d.zoom, None);
        assert!(d.panel_open);
    }
}
