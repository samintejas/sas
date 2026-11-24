//! The bottom panel's tab bar, shared by the clip editor and the mixer.

use sat_theme::Theme;
use vello::kurbo::{Point, Rect};

use crate::icons::{self, Icon};
use crate::paint::{Align, Painter, TextStyle};

/// Height of the panel's tab bar.
pub const TABS_H: f64 = 36.0;

/// What the bottom panel shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PanelView {
    /// The selected clip in the piano roll.
    #[default]
    Clip,
    /// The mixer.
    Mixer,
    /// The selected track's macros.
    Macros,
}

/// The tabs on the left of the bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    /// Clip editor.
    Clip,
    /// Macros.
    Macros,
    /// Mixer.
    Mixer,
}

const TABS: [(Tab, Icon, &str); 3] =
    [(Tab::Clip, Icon::Panel, "clip"), (Tab::Macros, Icon::Grid, "macros"), (Tab::Mixer, Icon::Sliders, "mixer")];

/// Width of a tab with an icon and a mono-12 label. Geist Mono advances are
/// 0.6 em, so this needs no font access and hit testing can use it.
pub fn tab_width(label: &str, icon: bool) -> f64 {
    label.chars().count() as f64 * 7.2 + 28.0 + if icon { 18.0 } else { 0.0 }
}

/// Rectangles of the view tabs in a panel at `r`.
pub fn tab_rects(r: Rect) -> [(Tab, Rect); 3] {
    let mut x = r.x0 + 4.0;
    TABS.map(|(t, _, label)| {
        let w = tab_width(label, true);
        let rect = Rect::new(x, r.y0, x + w, r.y0 + TABS_H);
        x += w;
        (t, rect)
    })
}

/// The tab under `pt`, if any.
pub fn hit_tab(r: Rect, pt: Point) -> Option<Tab> {
    tab_rects(r).into_iter().find(|(_, rect)| rect.contains(pt)).map(|(t, _)| t)
}

/// Draws one tab at `x` in the bar `r`; returns its width.
pub fn tab(p: &mut Painter<'_>, th: &Theme, x: f64, r: Rect, icon: Option<Icon>, label: &str, on: bool) -> f64 {
    let c = &th.c;
    let st = TextStyle::mono(12.0, if on { c.text_max } else { c.text_secondary });
    let w = tab_width(label, icon.is_some());
    let iw = if icon.is_some() { 18.0 } else { 0.0 };
    let cy = r.center().y;
    if let Some(i) = icon {
        icons::draw(p, i, Rect::new(x + 12.0, cy - 6.0, x + 24.0, cy + 6.0), st.color);
    }
    p.text(x + 14.0 + iw, cy, label, &st, Align::Left);
    if on {
        p.rect(Rect::new(x, r.y1 - 2.0, x + w, r.y1), c.accent_primary);
    }
    w
}

/// Draws the panel background and tab bar; returns the bar's rect and the x
/// where view-specific content can start.
pub fn draw_bar(p: &mut Painter<'_>, th: &Theme, r: Rect, view: PanelView) -> (Rect, f64) {
    let c = &th.c;
    p.rect(r, c.bg_surface);
    p.hline(r.x0, r.x1, r.y0, c.border_default);
    let bar = Rect::new(r.x0, r.y0, r.x1, r.y0 + TABS_H);
    p.hline(r.x0, r.x1, bar.y1 - 1.0, c.border_subtle);
    let mut x = r.x0 + 4.0;
    for (t, icon, label) in TABS {
        let on = matches!(
            (t, view),
            (Tab::Clip, PanelView::Clip) | (Tab::Mixer, PanelView::Mixer) | (Tab::Macros, PanelView::Macros)
        );
        x += tab(p, th, x, bar, Some(icon), label, on);
    }
    p.vline(x + 4.0, bar.y0 + 8.0, bar.y1 - 8.0, c.border_default);
    (bar, x + 16.0)
}
