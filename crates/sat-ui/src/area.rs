//! Areas: the one frame every editor lives in, in the manner of Blender.
//!
//! The window is split into areas (the main view, the bottom panel, the two
//! sidebars). Each is a rounded tile on the window's dark gutter with the same
//! header: an editor-type button on the left (it opens a menu to swap what the
//! area shows), then the editor's context, then its tools, then the area's own
//! controls on the right. The widgets in headers and editors come from here
//! too, so every editor looks and behaves alike.

use sat_theme::{Rgba, Theme};
use vello::kurbo::{BezPath, Point, Rect};

use crate::icons::{self, Icon};
use crate::paint::{Align, Painter, TextStyle};

/// Height of every area's header.
pub const HEADER_H: f64 = 30.0;
/// Corner radius of an area.
pub const RADIUS: f64 = 6.0;
/// Height of a row in the editor-type menu.
pub const MENU_ROW: f64 = 24.0;

/// The areas of the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Slot {
    /// The main view.
    #[default]
    Main,
    /// The bottom panel.
    Panel,
    /// The left sidebar.
    Left,
    /// The right sidebar.
    Right,
}

/// What an area shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorType {
    /// The arrangement.
    Arrange,
    /// The session's clip launcher.
    Session,
    /// The selected track's source grid.
    SourceGrid,
    /// The selected track's instrument grid.
    InstrumentGrid,
    /// The selected track's fx grid.
    FxGrid,
    /// The selected clip's editor (piano roll, step editor or audio).
    Clip,
    /// The mixer.
    Mixer,
    /// The clip bank.
    Bank,
    /// The selected track's macros, fader and pan.
    Inspector,
    /// The track tree.
    Outliner,
}

impl EditorType {
    /// Its name in headers and menus.
    pub fn label(self) -> &'static str {
        match self {
            EditorType::Arrange => "arrange",
            EditorType::Session => "session",
            EditorType::SourceGrid => "source grid",
            EditorType::InstrumentGrid => "instrument",
            EditorType::FxGrid => "fx grid",
            EditorType::Clip => "clip",
            EditorType::Mixer => "mixer",
            EditorType::Bank => "clip bank",
            EditorType::Inspector => "inspector",
            EditorType::Outliner => "outliner",
        }
    }

    /// Its icon.
    pub fn icon(self) -> Icon {
        match self {
            EditorType::Arrange => Icon::Lines,
            EditorType::Session => Icon::Grid,
            EditorType::SourceGrid => Icon::Panel,
            EditorType::InstrumentGrid => Icon::Gear,
            EditorType::FxGrid => Icon::Sliders,
            EditorType::Clip => Icon::Keys,
            EditorType::Mixer => Icon::Sliders,
            EditorType::Bank => Icon::Cards,
            EditorType::Inspector => Icon::Knob,
            EditorType::Outliner => Icon::Tree,
        }
    }

    /// The keyboard shortcut shown in the menu.
    pub fn hint(self) -> &'static str {
        match self {
            EditorType::Session => "ctrl 1",
            EditorType::Arrange => "ctrl 2",
            EditorType::SourceGrid => "ctrl 3",
            EditorType::InstrumentGrid => "ctrl 4",
            EditorType::FxGrid => "ctrl 5",
            EditorType::Clip | EditorType::Mixer => "tab",
            EditorType::Bank | EditorType::Inspector | EditorType::Outliner => "",
        }
    }

    /// What an area can show.
    pub fn choices(slot: Slot) -> &'static [EditorType] {
        use EditorType::*;
        match slot {
            Slot::Main => &[Arrange, Session, SourceGrid, InstrumentGrid, FxGrid],
            Slot::Panel => &[Clip, Mixer],
            Slot::Left | Slot::Right => &[Bank, Outliner, Inspector],
        }
    }
}

/// Corner radius of buttons.
pub const WIDGET_R: f64 = 4.0;

/// The editor-type button in the header of an area at `r`: the editor's
/// icon in a small box with a drop arrow, then its name. Its width comes
/// from the name (Geist advances are about 0.56 em at 12 px), so hit testing
/// needs no fonts.
pub fn pill_rect(r: Rect, ty: EditorType) -> Rect {
    let w = 36.0 + 8.0 + ty.label().chars().count() as f64 * 6.9;
    let cy = r.y0 + HEADER_H / 2.0;
    Rect::new(r.x0 + 5.0, cy - 10.0, r.x0 + 5.0 + w, cy + 10.0)
}

/// Draws an area's region background.
pub fn region(p: &mut Painter<'_>, th: &Theme, r: Rect) {
    p.rect(r, th.c.bg_surface);
}

/// The header's colour: a step above the region.
pub fn header_color(th: &Theme) -> Rgba {
    th.c.text_max.mix(th.c.bg_surface, 0.045)
}

/// A button's fill.
pub fn widget_bg(th: &Theme) -> Rgba {
    th.c.text_max.mix(th.c.bg_surface, 0.095)
}

/// A button's outline.
pub fn widget_line(th: &Theme) -> Rgba {
    th.c.text_max.mix(th.c.bg_surface, 0.15)
}

/// Fills and outlines a button's box, lit when on.
pub fn widget(p: &mut Painter<'_>, th: &Theme, r: Rect, on: bool) {
    let c = &th.c;
    if on {
        p.rrect(r, WIDGET_R, c.accent_primary.mix(widget_bg(th), 0.22));
        p.rrect_stroke(r.inset(-0.5), WIDGET_R, 1.0, c.accent_primary.alpha(0.55));
    } else {
        p.rrect(r, WIDGET_R, widget_bg(th));
        p.rrect_stroke(r.inset(-0.5), WIDGET_R, 1.0, widget_line(th));
        // a faint top light, as on a raised key
        p.hline(r.x0 + WIDGET_R, r.x1 - WIDGET_R, r.y0 + 1.0, Rgba::new(255, 255, 255, 10));
    }
}

/// The label style of buttons and headers.
pub fn label_style(th: &Theme, on: bool) -> TextStyle {
    TextStyle::sans(11.5, if on { th.c.accent_primary } else { th.c.text_strong })
}

/// Draws the header of an area at `r` with its editor-type button; returns
/// the header bar and the x where the editor's own header content starts.
pub fn header(p: &mut Painter<'_>, th: &Theme, r: Rect, ty: EditorType) -> (Rect, f64) {
    let c = &th.c;
    let bar = Rect::new(r.x0, r.y0, r.x1, r.y0 + HEADER_H);
    p.rect(bar, header_color(th));
    p.hline(bar.x0, bar.x1, bar.y1 - 1.0, c.bg_base);
    let pill = pill_rect(r, ty);
    let boxed = Rect::new(pill.x0, pill.y0, pill.x0 + 36.0, pill.y1);
    widget(p, th, boxed, false);
    let cy = pill.center().y;
    icons::draw(p, ty.icon(), Rect::new(boxed.x0 + 6.0, cy - 6.0, boxed.x0 + 18.0, cy + 6.0), c.accent_primary);
    icons::draw(p, Icon::ChevronDown, Rect::new(boxed.x1 - 13.0, cy - 4.0, boxed.x1 - 5.0, cy + 4.0), c.text_secondary);
    p.text(boxed.x1 + 8.0, cy, ty.label(), &TextStyle::sans(12.0, c.text_max).medium(), Align::Left);
    (bar, pill.x1 + 14.0)
}

/// Lightens the header of the area with focus (where the last press was),
/// so it is clear where keys go.
pub fn focus(p: &mut Painter<'_>, th: &Theme, r: Rect) {
    let bar = Rect::new(r.x0, r.y0, r.x1, r.y0 + HEADER_H - 1.0);
    p.rect(bar, Rgba::new(255, 255, 255, 6));
    p.rect(Rect::new(r.x0 + RADIUS, r.y0, r.x1 - RADIUS, r.y0 + 1.0), th.c.accent_primary.alpha(0.5));
}

/// Rounds an area's corners onto the gutter and outlines it. Drawn after the
/// editor, so editors can paint edge to edge.
pub fn finish(p: &mut Painter<'_>, th: &Theme, r: Rect) {
    if r.width() < 2.0 * RADIUS || r.height() < 2.0 * RADIUS {
        return;
    }
    let k = 0.5523 * RADIUS;
    let gutter = th.c.bg_base;
    // each corner: the square minus the quarter circle
    for (cx, cy, sx, sy) in
        [(r.x0, r.y0, 1.0, 1.0), (r.x1, r.y0, -1.0, 1.0), (r.x0, r.y1, 1.0, -1.0), (r.x1, r.y1, -1.0, -1.0)]
    {
        let mut b = BezPath::new();
        b.move_to(Point::new(cx, cy));
        b.line_to(Point::new(cx + sx * RADIUS, cy));
        b.curve_to(
            Point::new(cx + sx * (RADIUS - k), cy),
            Point::new(cx, cy + sy * (RADIUS - k)),
            Point::new(cx, cy + sy * RADIUS),
        );
        b.close_path();
        p.path(&b, gutter);
    }
    p.rrect_stroke(r.inset(-0.5), RADIUS - 0.5, 1.0, th.c.border_subtle);
}

/// A text button, lit when on.
pub fn button(p: &mut Painter<'_>, th: &Theme, r: Rect, label: &str, on: bool) {
    widget(p, th, r, on);
    p.text(r.center().x, r.center().y, label, &label_style(th, on), Align::Center);
}

/// A button with an optional icon before its label.
pub fn chip(p: &mut Painter<'_>, th: &Theme, r: Rect, label: &str, icon: Option<Icon>) {
    let c = &th.c;
    widget(p, th, r, false);
    let mut x = r.x0 + 9.0;
    if let Some(ic) = icon {
        icons::draw(p, ic, Rect::new(x, r.center().y - 5.5, x + 11.0, r.center().y + 5.5), c.text_primary);
        x += 17.0;
    }
    p.text(x, r.center().y, label, &label_style(th, false), Align::Left);
}

/// An icon button.
pub fn icon_button(p: &mut Painter<'_>, th: &Theme, r: Rect, icon: Icon) {
    widget(p, th, r, false);
    icons::draw(p, icon, Rect::from_center_size(r.center(), (11.0, 11.0)), th.c.text_strong);
}

/// A dimmer note in a header.
pub fn note(p: &mut Painter<'_>, th: &Theme, x: f64, cy: f64, text: &str, max_w: f64) {
    p.text_fit(x, cy, text, &TextStyle::sans(11.5, th.c.text_secondary), max_w);
}

/// The editor-type menu, open under an area's button.
#[derive(Debug, Clone, PartialEq)]
pub struct MenuLayout {
    /// The area it changes.
    pub slot: Slot,
    /// The menu's box.
    pub rect: Rect,
    /// Its rows.
    pub items: Vec<(EditorType, Rect)>,
}

impl MenuLayout {
    /// The menu for `slot` under its button `pill`, kept inside `window`.
    pub fn new(slot: Slot, pill: Rect, window: Rect) -> MenuLayout {
        let choices = EditorType::choices(slot);
        let w = 210.0;
        let h = choices.len() as f64 * MENU_ROW + 8.0;
        let x0 = pill.x0.min(window.x1 - w - 4.0).max(window.x0 + 4.0);
        // below the button, or above it when it would run off the bottom
        let y0 = if pill.y1 + 4.0 + h > window.y1 { pill.y0 - 4.0 - h } else { pill.y1 + 4.0 };
        let rect = Rect::new(x0, y0, x0 + w, y0 + h);
        let items = choices
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let y = rect.y0 + 4.0 + i as f64 * MENU_ROW;
                (*t, Rect::new(rect.x0 + 4.0, y, rect.x1 - 4.0, y + MENU_ROW))
            })
            .collect();
        MenuLayout { slot, rect, items }
    }

    /// The row under `pt`.
    pub fn hit(&self, pt: Point) -> Option<EditorType> {
        self.items.iter().find(|(_, r)| r.contains(pt)).map(|(t, _)| *t)
    }
}

/// Draws the editor-type menu, the current editor marked.
pub fn draw_menu(p: &mut Painter<'_>, th: &Theme, m: &MenuLayout, current: EditorType) {
    let c = &th.c;
    p.rrect(m.rect, 7.0, c.bg_overlay);
    p.rrect_stroke(m.rect.inset(-0.5), 7.0, 1.0, c.border_strong);
    for (t, r) in &m.items {
        let on = *t == current;
        if on {
            p.rrect(*r, WIDGET_R, c.accent_primary_subtle);
        }
        let cy = r.center().y;
        let col = if on { c.accent_primary } else { c.text_strong };
        icons::draw(p, t.icon(), Rect::new(r.x0 + 8.0, cy - 6.0, r.x0 + 20.0, cy + 6.0), col);
        p.text(r.x0 + 28.0, cy, t.label(), &TextStyle::sans(12.0, col), Align::Left);
        p.text(r.x1 - 8.0, cy, t.hint(), &TextStyle::sans(10.5, c.text_secondary), Align::Right);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menus_list_an_areas_editors_and_stay_on_screen() {
        let window = Rect::new(0.0, 0.0, 1440.0, 900.0);
        let pill = pill_rect(Rect::new(4.0, 600.0, 1000.0, 896.0), EditorType::Clip);
        let m = MenuLayout::new(Slot::Panel, pill, window);
        assert_eq!(m.items.iter().map(|(t, _)| *t).collect::<Vec<_>>(), vec![EditorType::Clip, EditorType::Mixer]);
        assert_eq!(m.hit(m.items[1].1.center()), Some(EditorType::Mixer));
        // near the bottom it opens upwards
        let low = MenuLayout::new(Slot::Main, Rect::new(10.0, 870.0, 100.0, 896.0), window);
        assert!(low.rect.y1 <= 870.0 && low.rect.y0 >= 0.0);
        // the button grows with its label
        let a = pill_rect(window, EditorType::Clip);
        let b = pill_rect(window, EditorType::InstrumentGrid);
        assert!(b.width() > a.width());
    }
}
