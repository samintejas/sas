//! The command palette (⌘K): type part of a track's name and what to open,
//! such as "bass fx", and press enter. It reaches every view and every
//! track's grids from anywhere.

use sat_doc::{Document, GridKind, TrackId, TrackKind};
use sat_theme::Theme;
use vello::kurbo::{Point, Rect};

use crate::icons::{self, Icon};
use crate::paint::{Align, Painter, TextStyle};
use crate::panel::PanelView;

/// Most rows the palette lists.
pub const MAX_ROWS: usize = 9;
const ROW_H: f64 = 30.0;
const W: f64 = 520.0;

/// What a palette entry does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Show the arrangement.
    Arrange,
    /// Show or hide the clip launcher beside the arrangement.
    Launcher,
    /// Show one of a track's grids.
    Grid(TrackId, GridKind),
    /// Select a track.
    Select(TrackId),
    /// Show a bottom panel page.
    Panel(PanelView),
    /// Show or hide a docked area.
    Dock(crate::dock::DockButton),
    /// Maximize the bottom panel, or restore it.
    Maximize,
    /// Zoom the main view to the whole window, or restore.
    Focus,
    /// Play or stop.
    PlayStop,
    /// Loop on or off.
    Loop,
    /// Undo.
    Undo,
    /// Redo.
    Redo,
    /// Save.
    Save,
    /// Freeze a track, or unfreeze it.
    Freeze(TrackId),
}

/// One palette entry.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// What it reads.
    pub label: String,
    /// A hint on the right (its shortcut or what it is).
    pub hint: &'static str,
    /// What it does.
    pub action: Action,
}

/// The palette's state while it is open.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PaletteView {
    /// What has been typed.
    pub query: String,
    /// The highlighted row.
    pub index: usize,
}

fn kind_name(k: GridKind) -> &'static str {
    match k {
        GridKind::Source => "source grid",
        GridKind::Instrument => "instrument",
        GridKind::Fx => "fx",
    }
}

/// Every entry, before filtering.
pub fn entries(doc: &Document) -> Vec<Entry> {
    let mut out = vec![
        Entry { label: "arrange".into(), hint: "ctrl 2", action: Action::Arrange },
        Entry { label: "clip launcher beside arrangement".into(), hint: "ctrl 1", action: Action::Launcher },
    ];
    for t in doc.tracks_in_order() {
        let tr = &doc.tracks[&t];
        let name = if tr.kind == TrackKind::Master { "master".to_string() } else { tr.name.clone() };
        for (k, _) in crate::grid::grid_kinds(doc, t) {
            let hint = match k {
                GridKind::Source => "ctrl 3",
                GridKind::Instrument => "ctrl 4",
                GridKind::Fx => "ctrl 5",
            };
            out.push(Entry { label: format!("{name} · {}", kind_name(k)), hint, action: Action::Grid(t, k) });
        }
        out.push(Entry { label: format!("{name} · select"), hint: "[ ]", action: Action::Select(t) });
        if tr.kind == TrackKind::Track {
            let what = if tr.frozen.is_some() { "unfreeze" } else { "freeze" };
            out.push(Entry { label: format!("{name} · {what}"), hint: "ctrl f", action: Action::Freeze(t) });
        }
    }
    out.extend([
        Entry { label: "clip editor".into(), hint: "tab", action: Action::Panel(PanelView::Clip) },
        Entry { label: "mixer".into(), hint: "tab", action: Action::Panel(PanelView::Mixer) },
        Entry {
            label: "clip bank · left sidebar".into(),
            hint: "ctrl b",
            action: Action::Dock(crate::dock::DockButton::Left),
        },
        Entry {
            label: "bottom panel · collapse / open".into(),
            hint: "ctrl j",
            action: Action::Dock(crate::dock::DockButton::Panel),
        },
        Entry { label: "bottom panel · maximize".into(), hint: "ctrl space", action: Action::Maximize },
        Entry { label: "focus main view · hide panels".into(), hint: "ctrl space", action: Action::Focus },
        Entry { label: "play / stop".into(), hint: "space", action: Action::PlayStop },
        Entry { label: "loop on / off".into(), hint: "l", action: Action::Loop },
        Entry { label: "undo".into(), hint: "ctrl z", action: Action::Undo },
        Entry { label: "redo".into(), hint: "ctrl shift z", action: Action::Redo },
        Entry { label: "save".into(), hint: "ctrl s", action: Action::Save },
    ]);
    out
}

/// How well `word` matches `label`: a word start beats a substring, which
/// beats the letters in order; `None` when it does not match.
fn word_score(label: &str, word: &str) -> Option<u32> {
    if let Some(i) = label.find(word) {
        let at_start = i == 0 || !label.as_bytes()[i - 1].is_ascii_alphanumeric();
        return Some(if at_start { 0 } else { 1 });
    }
    let mut chars = label.chars();
    word.chars().all(|w| chars.any(|c| c == w)).then_some(3)
}

/// The entries matching `query`, best first. Every word of the query must
/// match; an empty query lists everything in order.
pub fn matches(doc: &Document, query: &str) -> Vec<Entry> {
    let q = query.to_lowercase();
    let words: Vec<&str> = q.split_whitespace().collect();
    let mut scored: Vec<(u32, usize, Entry)> = entries(doc)
        .into_iter()
        .enumerate()
        .filter_map(|(i, e)| {
            let label = e.label.to_lowercase();
            let mut total = 0;
            for w in &words {
                total += word_score(&label, w)?;
            }
            Some((total, i, e))
        })
        .collect();
    scored.sort_by_key(|(s, i, _)| (*s, *i));
    scored.into_iter().map(|(_, _, e)| e).collect()
}

/// The palette's geometry.
#[derive(Debug, Clone)]
pub struct PaletteLayout {
    /// The whole box.
    pub rect: Rect,
    /// Listed rows and their entries.
    pub rows: Vec<(Rect, Entry)>,
}

impl PaletteLayout {
    /// The entry under `pt`.
    pub fn hit(&self, pt: Point) -> Option<&Entry> {
        self.rows.iter().find(|(r, _)| r.contains(pt)).map(|(_, e)| e)
    }
}

/// The rows shown for a highlighted index: a window of [`MAX_ROWS`] around it.
fn window(n: usize, index: usize) -> std::ops::Range<usize> {
    let start = index.saturating_sub(MAX_ROWS - 1).min(n.saturating_sub(MAX_ROWS));
    start..(start + MAX_ROWS).min(n)
}

/// Lays out the palette over `area`.
pub fn layout(doc: &Document, v: &PaletteView, area: Rect) -> PaletteLayout {
    let found = matches(doc, &v.query);
    let index = v.index.min(found.len().saturating_sub(1));
    let shown = window(found.len(), index);
    let x0 = area.center().x - W / 2.0;
    let y0 = area.y0 + 60.0;
    let list_y = y0 + 48.0;
    let n = shown.len().max(1) as f64;
    let rect = Rect::new(x0, y0, x0 + W, list_y + n * ROW_H + 8.0);
    let rows = found[shown.clone()]
        .iter()
        .enumerate()
        .map(|(k, e)| {
            let y = list_y + k as f64 * ROW_H;
            (Rect::new(x0 + 6.0, y, x0 + W - 6.0, y + ROW_H), e.clone())
        })
        .collect();
    PaletteLayout { rect, rows }
}

/// Draws the palette over `area`.
pub fn draw(p: &mut Painter<'_>, th: &Theme, doc: &Document, v: &PaletteView, area: Rect) -> PaletteLayout {
    let c = &th.c;
    let l = layout(doc, v, area);
    p.rect(area, sat_theme::Rgba::new(0, 0, 0, 110));
    p.rrect(l.rect, 8.0, c.bg_surface);
    p.rrect_stroke(l.rect.inset(-0.5), 8.0, 1.0, c.border_strong);
    let fy = l.rect.y0 + 24.0;
    icons::draw(p, Icon::Search, Rect::new(l.rect.x0 + 14.0, fy - 6.0, l.rect.x0 + 26.0, fy + 6.0), c.text_secondary);
    let (text, col) = if v.query.is_empty() {
        ("open a track's grid, a view or a command…", c.text_secondary)
    } else {
        (v.query.as_str(), c.text_max)
    };
    let w = p.text(l.rect.x0 + 36.0, fy, text, &TextStyle::mono(13.0, col), Align::Left);
    let cx = l.rect.x0 + 36.0 + if v.query.is_empty() { 0.0 } else { w + 1.0 };
    p.rect(Rect::new(cx, fy - 8.0, cx + 1.5, fy + 8.0), c.accent_primary);
    p.hline(l.rect.x0, l.rect.x1, l.rect.y0 + 46.0, c.border_default);
    let found = matches(doc, &v.query).len();
    let index = v.index.min(found.saturating_sub(1));
    let first = window(found, index).start;
    if l.rows.is_empty() {
        p.text(
            l.rect.x0 + 20.0,
            l.rect.y0 + 48.0 + ROW_H / 2.0,
            "nothing matches",
            &TextStyle::mono(12.0, c.text_secondary),
            Align::Left,
        );
    }
    for (k, (r, e)) in l.rows.iter().enumerate() {
        let on = first + k == index;
        if on {
            p.rrect(*r, 5.0, c.bg_raised);
            p.rect(Rect::new(r.x0, r.y0 + 6.0, r.x0 + 2.0, r.y1 - 6.0), c.accent_primary);
        }
        let col = if on { c.text_max } else { c.text_strong };
        p.text_fit(r.x0 + 14.0, r.center().y, &e.label, &TextStyle::mono(12.5, col), r.width() - 130.0);
        p.text(r.x1 - 12.0, r.center().y, e.hint, &TextStyle::mono(11.0, c.text_secondary), Align::Right);
    }
    l
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_find_a_tracks_grid() {
        let doc = crate::demo::demo().doc;
        let found = matches(&doc, "bass fx");
        let first = found.first().expect("a match");
        let Action::Grid(t, GridKind::Fx) = first.action else { panic!("{first:?}") };
        assert!(doc.tracks[&t].name.to_lowercase().contains("bass"));
        assert!(matches(&doc, "zzzz").is_empty());
        assert_eq!(matches(&doc, "").len(), entries(&doc).len());
    }

    #[test]
    fn the_window_keeps_the_highlight_in_view() {
        assert_eq!(window(3, 0), 0..3);
        assert_eq!(window(20, 0), 0..MAX_ROWS);
        assert_eq!(window(20, 12), 4..13);
        assert_eq!(window(20, 19), 11..20);
    }
}
