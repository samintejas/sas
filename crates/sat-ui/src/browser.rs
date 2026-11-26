//! The browser: everything you can add to the project, in one searchable
//! list, in the manner of Blender's asset browser. New tracks, sources and
//! note processors for the source grid, instrument modules and effects. A
//! click selects an item, a double-click (or its +) adds it to the selected
//! track, into the grid where it belongs, and opens that grid.

use sat_doc::{GridKind, TrackKind};
use sat_theme::Theme;
use vello::kurbo::{Point, Rect, Vec2};

use crate::area::{self, EditorType, HEADER_H};
use crate::icons::{self, Icon};
use crate::paint::{Align, Painter, TextStyle};

/// Height of the search row.
pub const SEARCH_H: f64 = 34.0;
/// Height of a category heading.
pub const HEAD_H: f64 = 26.0;
/// Height of an item.
pub const ROW_H: f64 = 24.0;

/// The browser's categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    /// New tracks, groups and returns.
    Tracks,
    /// Sources for the source grid.
    Sources,
    /// Note processors for the source grid.
    NoteFx,
    /// Instrument modules.
    Instruments,
    /// Effects for the fx chain.
    Effects,
}

impl Category {
    /// Every category, in order.
    pub const ALL: [Category; 5] =
        [Category::Tracks, Category::Sources, Category::NoteFx, Category::Instruments, Category::Effects];

    /// Its name.
    pub fn label(self) -> &'static str {
        match self {
            Category::Tracks => "tracks",
            Category::Sources => "sources",
            Category::NoteFx => "note fx",
            Category::Instruments => "instruments",
            Category::Effects => "effects",
        }
    }

    /// Where its items go.
    pub fn goes_to(self) -> &'static str {
        match self {
            Category::Tracks => "the project",
            Category::Sources | Category::NoteFx => "source grid",
            Category::Instruments => "instrument",
            Category::Effects => "fx chain",
        }
    }

    /// The grid its items go into (none for tracks).
    pub fn grid(self) -> Option<GridKind> {
        match self {
            Category::Tracks => None,
            Category::Sources | Category::NoteFx => Some(GridKind::Source),
            Category::Instruments => Some(GridKind::Instrument),
            Category::Effects => Some(GridKind::Fx),
        }
    }

    fn bit(self) -> u8 {
        1 << Category::ALL.iter().position(|c| *c == self).unwrap()
    }

    fn color(self) -> u8 {
        match self {
            Category::Tracks => 6,
            Category::Sources => 0,
            Category::NoteFx => 4,
            Category::Instruments => 1,
            Category::Effects => 2,
        }
    }

    /// Its items: kind and label.
    pub fn items(self) -> Vec<(&'static str, &'static str)> {
        use sat_modules::{fx, instrument, notefx};
        match self {
            Category::Tracks => TRACKS.to_vec(),
            Category::Sources => {
                sat_modules::SOURCES.iter().filter(|(k, _)| !notefx::is_processor(k)).copied().collect()
            }
            Category::NoteFx => notefx::MODULES.to_vec(),
            Category::Instruments => instrument::MODULES
                .iter()
                .filter(|(k, _)| *k != instrument::NOTE_IN && *k != instrument::VOICE_OUT)
                .copied()
                .collect(),
            Category::Effects => {
                fx::MODULES.iter().filter(|(k, _)| *k != fx::FX_IN && *k != fx::FX_OUT).copied().collect()
            }
        }
    }
}

/// A synth track: a track with the default voice patch.
pub const SYNTH_TRACK: &str = "track:synth";
/// A group.
pub const GROUP: &str = "track:group";
/// A return.
pub const RETURN: &str = "track:return";
const TRACKS: [(&str, &str); 3] = [(SYNTH_TRACK, "synth track"), (GROUP, "group"), (RETURN, "return")];

/// The kind of track a track item adds.
pub fn track_kind(item: &str) -> Option<TrackKind> {
    match item {
        SYNTH_TRACK => Some(TrackKind::Track),
        GROUP => Some(TrackKind::Group),
        RETURN => Some(TrackKind::Return),
        _ => None,
    }
}

/// The browser's state.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BrowserView {
    /// The search text.
    pub query: String,
    /// The search field takes the keys.
    pub typing: bool,
    /// Folded categories, one bit each.
    pub folded: u8,
    /// The selected item: its kind.
    pub selected: Option<&'static str>,
    /// Vertical scroll.
    pub scroll: f64,
}

impl BrowserView {
    /// Folds or unfolds a category.
    pub fn toggle(&mut self, c: Category) {
        self.folded ^= c.bit();
    }

    /// Whether a category is folded (a search shows every match).
    pub fn is_folded(&self, c: Category) -> bool {
        self.query.is_empty() && self.folded & c.bit() != 0
    }
}

/// What is under a point in the browser.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BrowserHit {
    /// The search field.
    Search,
    /// Clear the search.
    Clear,
    /// A category's heading: fold it.
    Fold(Category),
    /// An item: select it (a double-click adds it).
    Item(Category, &'static str),
    /// An item's +: add it.
    Add(Category, &'static str),
    /// Nothing.
    Nothing,
}

/// One item's row.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ItemRow {
    /// Its category.
    pub cat: Category,
    /// Its kind.
    pub kind: &'static str,
    /// Its label.
    pub label: &'static str,
    /// The row.
    pub rect: Rect,
    /// Its + button.
    pub add: Rect,
}

/// The browser's geometry.
#[derive(Debug, Clone, PartialEq)]
pub struct BrowserLayout {
    /// The search field.
    pub search: Rect,
    /// The scrolling list.
    pub body: Rect,
    /// Category headings: category, row, matches.
    pub heads: Vec<(Category, Rect, usize)>,
    /// Visible items.
    pub items: Vec<ItemRow>,
    /// How far the list scrolls.
    pub max_scroll: f64,
}

fn matches(q: &str, label: &str, cat: Category) -> bool {
    q.split_whitespace().all(|w| label.contains(w) || cat.label().contains(w))
}

impl BrowserLayout {
    /// Lays out the browser in the area `r`.
    pub fn new(view: &BrowserView, r: Rect) -> BrowserLayout {
        let y = r.y0 + HEADER_H;
        let search = Rect::new(r.x0 + 8.0, y + 6.0, r.x1 - 8.0, y + SEARCH_H - 4.0);
        let body = Rect::new(r.x0, y + SEARCH_H, r.x1, r.y1.max(y + SEARCH_H));
        let q = view.query.to_lowercase();
        let mut heads = Vec::new();
        let mut items = Vec::new();
        let mut at = 0.0;
        let x0 = body.x0 + 6.0;
        let x1 = body.x1 - 6.0;
        for cat in Category::ALL {
            let found: Vec<_> = cat.items().into_iter().filter(|(_, l)| matches(&q, l, cat)).collect();
            if found.is_empty() {
                continue;
            }
            heads.push((cat, Rect::new(x0, at, x1, at + HEAD_H), found.len()));
            at += HEAD_H;
            if !view.is_folded(cat) {
                for (kind, label) in found {
                    let rect = Rect::new(x0, at, x1, at + ROW_H);
                    let cy = rect.center().y;
                    let add = Rect::new(x1 - 24.0, cy - 9.0, x1 - 4.0, cy + 9.0);
                    items.push(ItemRow { cat, kind, label, rect, add });
                    at += ROW_H;
                }
            }
            at += 6.0;
        }
        let max_scroll = (at + 6.0 - body.height()).max(0.0);
        let dy = body.y0 + 4.0 - view.scroll.clamp(0.0, max_scroll);
        for (_, h, _) in &mut heads {
            *h = *h + Vec2::new(0.0, dy);
        }
        for it in &mut items {
            it.rect = it.rect + Vec2::new(0.0, dy);
            it.add = it.add + Vec2::new(0.0, dy);
        }
        BrowserLayout { search, body, heads, items, max_scroll }
    }

    /// The search field's clear button.
    pub fn clear(&self) -> Rect {
        let cy = self.search.center().y;
        Rect::new(self.search.x1 - 22.0, cy - 8.0, self.search.x1 - 6.0, cy + 8.0)
    }

    /// What is under `pt`.
    pub fn hit(&self, view: &BrowserView, pt: Point) -> BrowserHit {
        if self.search.contains(pt) {
            return if !view.query.is_empty() && self.clear().contains(pt) {
                BrowserHit::Clear
            } else {
                BrowserHit::Search
            };
        }
        if !self.body.contains(pt) {
            return BrowserHit::Nothing;
        }
        if let Some((c, _, _)) = self.heads.iter().find(|(_, r, _)| r.contains(pt)) {
            return BrowserHit::Fold(*c);
        }
        match self.items.iter().find(|i| i.rect.contains(pt)) {
            Some(i) if i.add.contains(pt) => BrowserHit::Add(i.cat, i.kind),
            Some(i) => BrowserHit::Item(i.cat, i.kind),
            None => BrowserHit::Nothing,
        }
    }
}

/// Draws the browser in the area `r` and returns its layout. `track` names
/// the selected track, where items go.
pub fn draw(p: &mut Painter<'_>, th: &Theme, view: &BrowserView, track: Option<&str>, r: Rect) -> BrowserLayout {
    let c = &th.c;
    let l = BrowserLayout::new(view, r);
    area::region(p, th, r);
    let end = crate::dock::close_button(r).x0 - 6.0;
    let (bar, hx) = area::header(p, th, r, EditorType::Browser);
    let note = match track {
        Some(t) => format!("adds to {t}"),
        None => "select a track".into(),
    };
    area::note(p, th, hx, bar.center().y, &note, end - hx);
    // search
    let s = l.search;
    p.rrect(s, area::WIDGET_R, c.bg_base.mix(c.bg_surface, 0.4));
    let line = if view.typing { c.accent_primary.alpha(0.7) } else { area::widget_line(th) };
    p.rrect_stroke(s.inset(-0.5), area::WIDGET_R, 1.0, line);
    let cy = s.center().y;
    icons::draw(p, Icon::Search, Rect::new(s.x0 + 8.0, cy - 6.0, s.x0 + 20.0, cy + 6.0), c.text_secondary);
    let st = TextStyle::sans(12.0, c.text_max);
    let tx = s.x0 + 28.0;
    if view.query.is_empty() {
        let hint = if view.typing { "" } else { "search modules and tracks" };
        p.text(tx, cy, hint, &TextStyle::sans(12.0, c.text_secondary), Align::Left);
    } else {
        p.text_fit(tx, cy, &view.query, &st, s.width() - 60.0);
        icons::draw(p, Icon::Close, l.clear().inset(-4.0), c.text_secondary);
    }
    if view.typing {
        let w = if view.query.is_empty() { 0.0 } else { p.fonts.measure(&view.query, &st) as f64 };
        let x = (tx + w + 1.0).min(s.x1 - 30.0);
        p.vline(x, cy - 7.0, cy + 7.0, c.accent_primary);
    }
    p.clip(l.body, |p| {
        for (cat, h, n) in &l.heads {
            let folded = view.is_folded(*cat);
            let hy = h.center().y;
            let ic = if folded { Icon::ChevronRight } else { Icon::ChevronDown };
            icons::draw(p, ic, Rect::new(h.x0 + 2.0, hy - 5.0, h.x0 + 12.0, hy + 5.0), c.text_secondary);
            let name = TextStyle::sans(12.0, c.text_strong).medium();
            let w = p.text(h.x0 + 18.0, hy, cat.label(), &name, Align::Left);
            let note = format!("→ {}", cat.goes_to());
            p.text_fit(h.x0 + 26.0 + w, hy, &note, &TextStyle::sans(11.0, c.text_secondary), h.width() - w - 70.0);
            p.text(h.x1 - 6.0, hy, &n.to_string(), &TextStyle::mono(10.5, c.text_secondary), Align::Right);
        }
        for it in &l.items {
            let sel = view.selected == Some(it.kind);
            if sel {
                p.rrect(it.rect, area::WIDGET_R, c.accent_primary_subtle);
            }
            let iy = it.rect.center().y;
            let dot = th.track(it.cat.color());
            let ix = it.rect.x0 + 18.0;
            if it.cat == Category::Tracks {
                p.rrect(Rect::new(ix, iy - 5.0, ix + 10.0, iy + 5.0), 2.0, dot);
            } else {
                p.rrect_stroke(Rect::new(ix + 0.5, iy - 4.5, ix + 9.5, iy + 4.5), 2.0, 1.0, dot);
                p.rect(Rect::new(ix + 0.5, iy - 4.5, ix + 9.5, iy - 1.5), dot);
            }
            let col = if sel { c.text_max } else { c.text_primary };
            p.text_fit(ix + 18.0, iy, it.label, &TextStyle::sans(12.0, col), it.add.x0 - ix - 24.0);
            if sel {
                area::icon_button(p, th, it.add, Icon::Plus);
            } else {
                icons::draw(p, Icon::Plus, Rect::from_center_size(it.add.center(), (9.0, 9.0)), c.text_secondary);
            }
        }
        if l.heads.is_empty() {
            let t = TextStyle::sans(12.0, c.text_secondary);
            p.text(l.body.center().x, l.body.y0 + 30.0, "nothing matches", &t, Align::Center);
        }
    });
    l
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_fold_and_filter() {
        let r = Rect::new(0.0, 0.0, 280.0, 3000.0);
        let mut v = BrowserView::default();
        let l = BrowserLayout::new(&v, r);
        assert_eq!(l.heads.len(), Category::ALL.len());
        let total: usize = Category::ALL.iter().map(|c| c.items().len()).sum();
        assert_eq!(l.items.len(), total);
        assert!(!l.items.iter().any(|i| i.kind == sat_modules::fx::FX_IN), "chain ends are not offered");
        let reverb = l.items.iter().find(|i| i.label == "reverb").unwrap();
        assert_eq!(l.hit(&v, reverb.rect.center()), BrowserHit::Item(Category::Effects, reverb.kind));
        assert_eq!(l.hit(&v, reverb.add.center()), BrowserHit::Add(Category::Effects, reverb.kind));
        assert_eq!(l.hit(&v, l.heads[1].1.center()), BrowserHit::Fold(Category::Sources));
        assert_eq!(l.hit(&v, l.search.center()), BrowserHit::Search);
        v.toggle(Category::Effects);
        let shut = BrowserLayout::new(&v, r);
        assert!(!shut.items.iter().any(|i| i.cat == Category::Effects));
        // a search shows matches in folded categories too
        v.query = "rev".into();
        let found = BrowserLayout::new(&v, r);
        assert_eq!(found.items.len(), 1);
        assert_eq!(found.items[0].label, "reverb");
        v.query = "effects".into();
        assert!(BrowserLayout::new(&v, r).items.len() > 5, "a category name matches its items");
    }
}
