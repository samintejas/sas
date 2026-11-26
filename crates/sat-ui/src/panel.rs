//! The bottom panel's header, shared by the clip editor and the mixer: the
//! editors that gain from room. The clip bank and the macros live in the
//! sidebars (see [`crate::dock`]).

use sat_theme::Theme;
use vello::kurbo::Rect;

use crate::icons::{self, Icon};
use crate::paint::{Align, Painter, TextStyle};

/// Height of the panel's header (the same as every area's).
pub const TABS_H: f64 = crate::area::HEADER_H;

/// What the bottom panel shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PanelView {
    /// The selected clip in the piano roll.
    #[default]
    Clip,
    /// The mixer.
    Mixer,
}

impl PanelView {
    /// The editor type its header names.
    pub fn editor(self) -> crate::area::EditorType {
        match self {
            PanelView::Clip => crate::area::EditorType::Clip,
            PanelView::Mixer => crate::area::EditorType::Mixer,
        }
    }
}

/// Width of a tab with an icon and a mono-12 label. Geist Mono advances are
/// 0.6 em, so this needs no font access and hit testing can use it.
pub fn tab_width(label: &str, icon: bool) -> f64 {
    label.chars().count() as f64 * 7.2 + 28.0 + if icon { 18.0 } else { 0.0 }
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
        p.rect(Rect::new(x + 6.0, r.y1 - 2.0, x + w - 6.0, r.y1), c.accent_primary);
    }
    w
}

/// Draws the panel background and its area header; returns the header bar
/// and the x where view-specific content can start.
pub fn draw_bar(p: &mut Painter<'_>, th: &Theme, r: Rect, view: PanelView) -> (Rect, f64) {
    crate::area::region(p, th, r);
    crate::area::header(p, th, r, view.editor())
}
