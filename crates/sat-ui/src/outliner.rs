//! The outliner: the project's track tree at a glance, in the manner of
//! Blender's. Groups fold (the same folding as the arrangement's), every row
//! shows what the track plays and mutes or solos it, a click selects and a
//! double-click opens the track's grid.

use std::collections::BTreeSet;

use sat_doc::*;
use sat_theme::{Rgba, Theme};
use vello::kurbo::{Point, Rect};

use crate::icons::{self, Icon};
use crate::paint::{Align, Painter, TextStyle};
use crate::panel::TABS_H;

/// Height of a row.
pub const ROW_H: f64 = 26.0;
/// Indent per tree level.
pub const INDENT: f64 = 14.0;

/// One track's row.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OutlineRow {
    /// The track.
    pub track: TrackId,
    /// The row.
    pub rect: Rect,
    /// Depth in the tree.
    pub depth: usize,
    /// Its fold chevron (groups).
    pub fold: Option<Rect>,
    /// Its mute toggle.
    pub mute: Rect,
    /// Its solo toggle (not on master).
    pub solo: Option<Rect>,
}

/// What is under a point in the outliner.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OutlineHit {
    /// A row: select the track (a double-click opens it).
    Row(TrackId),
    /// A group's chevron: fold or unfold it.
    Fold(TrackId),
    /// A mute toggle.
    Mute(TrackId),
    /// A solo toggle.
    Solo(TrackId),
    /// Nothing.
    Nothing,
}

/// The outliner's geometry.
#[derive(Debug, Clone, PartialEq)]
pub struct OutlineLayout {
    /// The scrolling area under the header.
    pub body: Rect,
    /// Visible rows, top to bottom.
    pub rows: Vec<OutlineRow>,
    /// How far it can scroll.
    pub max_scroll: f64,
}

fn depth(doc: &Document, t: TrackId) -> usize {
    let mut d = 0;
    let mut at = doc.tracks.get(&t).and_then(|t| t.parent);
    while let Some(p) = at {
        d += 1;
        at = doc.tracks.get(&p).and_then(|t| t.parent);
    }
    d
}

impl OutlineLayout {
    /// Lays out the outliner in the area `r`, with the groups in `folded`
    /// shut and the list scrolled down by `scroll`.
    pub fn new(doc: &Document, folded: &BTreeSet<TrackId>, scroll: f64, r: Rect) -> OutlineLayout {
        let body = Rect::new(r.x0, r.y0 + TABS_H, r.x1, r.y1.max(r.y0 + TABS_H));
        let visible: Vec<TrackId> = doc
            .tracks_in_order()
            .into_iter()
            .filter(|t| {
                // hidden when any group above it is folded
                let mut at = doc.tracks.get(t).and_then(|t| t.parent);
                while let Some(p) = at {
                    if folded.contains(&p) {
                        return false;
                    }
                    at = doc.tracks.get(&p).and_then(|t| t.parent);
                }
                true
            })
            .collect();
        let content = visible.len() as f64 * ROW_H + 12.0;
        let max_scroll = (content - body.height()).max(0.0);
        let y0 = body.y0 + 6.0 - scroll.clamp(0.0, max_scroll);
        let rows = visible
            .into_iter()
            .enumerate()
            .map(|(i, t)| {
                let y = y0 + i as f64 * ROW_H;
                let rect = Rect::new(body.x0 + 4.0, y, body.x1 - 4.0, y + ROW_H);
                let d = depth(doc, t);
                let tr = &doc.tracks[&t];
                let cy = rect.center().y;
                let fx = rect.x0 + 6.0 + d as f64 * INDENT;
                let fold = (tr.kind == TrackKind::Group).then(|| Rect::new(fx, cy - 8.0, fx + 16.0, cy + 8.0));
                let b = |x1: f64| Rect::new(x1 - 22.0, cy - 9.0, x1, cy + 9.0);
                let solo = (tr.kind != TrackKind::Master).then(|| b(rect.x1 - 4.0));
                let mute = b(rect.x1 - 30.0);
                OutlineRow { track: t, rect, depth: d, fold, mute, solo }
            })
            .collect();
        OutlineLayout { body, rows, max_scroll }
    }

    /// What is under `pt`.
    pub fn hit(&self, pt: Point) -> OutlineHit {
        if !self.body.contains(pt) {
            return OutlineHit::Nothing;
        }
        for r in &self.rows {
            if !r.rect.contains(pt) {
                continue;
            }
            if r.fold.is_some_and(|f| f.contains(pt)) {
                return OutlineHit::Fold(r.track);
            }
            if r.mute.contains(pt) {
                return OutlineHit::Mute(r.track);
            }
            if r.solo.is_some_and(|s| s.contains(pt)) {
                return OutlineHit::Solo(r.track);
            }
            return OutlineHit::Row(r.track);
        }
        OutlineHit::Nothing
    }
}

/// What a track plays, in a word or two.
fn what(doc: &Document, t: &Track) -> String {
    match t.kind {
        TrackKind::Group => format!("{} tracks", t.children.len()),
        TrackKind::Return => "return".into(),
        TrackKind::Master => "master".into(),
        TrackKind::Track => t
            .instrument_grid
            .and_then(|g| sat_modules::instrument::label(doc, g))
            .unwrap_or_else(|| if t.frozen.is_some() { "frozen".into() } else { "audio".into() }),
    }
}

fn toggle(p: &mut Painter<'_>, th: &Theme, r: Rect, label: &str, on: Option<Rgba>) {
    let c = &th.c;
    match on {
        Some(col) => p.rrect(r, 4.0, col),
        None => {
            p.rrect(r, 4.0, c.bg_control);
            p.rrect_stroke(r.inset(-0.5), 4.0, 1.0, c.border_subtle);
        }
    }
    let tc = if on.is_some() { c.on_accent_primary } else { c.text_secondary };
    p.text(r.center().x, r.center().y, label, &TextStyle::mono(10.5, tc), Align::Center);
}

/// Draws the outliner in the area `r` and returns its layout.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    p: &mut Painter<'_>,
    th: &Theme,
    doc: &Document,
    folded: &BTreeSet<TrackId>,
    selected: Option<TrackId>,
    scroll: f64,
    r: Rect,
) -> OutlineLayout {
    let c = &th.c;
    let l = OutlineLayout::new(doc, folded, scroll, r);
    let n = doc.tracks.len();
    let end = crate::dock::close_button(r).x0 - 6.0;
    crate::dock::side_header(p, th, r, crate::area::EditorType::Outliner, None, "", &format!("{n} tracks"), end);
    p.clip(l.body, |p| {
        for row in &l.rows {
            let Some(t) = doc.tracks.get(&row.track) else { continue };
            let sel = selected == Some(row.track);
            if sel {
                p.rrect(row.rect, 4.0, c.accent_primary_subtle);
            }
            let cy = row.rect.center().y;
            // tree guides
            for k in 0..row.depth {
                let x = row.rect.x0 + 14.0 + k as f64 * INDENT;
                p.vline(x, row.rect.y0, row.rect.y1, c.border_subtle);
            }
            let mut x = row.rect.x0 + 6.0 + row.depth as f64 * INDENT;
            if let Some(f) = row.fold {
                let ic = if folded.contains(&row.track) { Icon::ChevronRight } else { Icon::ChevronDown };
                icons::draw(p, ic, f.inset(-3.0), c.text_secondary);
            }
            x += 18.0;
            p.rrect(Rect::new(x, cy - 5.0, x + 10.0, cy + 5.0), 2.0, th.track(t.color.0));
            x += 16.0;
            let st = TextStyle::sans(12.0, if sel { c.text_max } else { c.text_strong }).medium();
            let name = if t.kind == TrackKind::Master { "master" } else { t.name.as_str() };
            let nw = (p.fonts.measure(name, &st) as f64).min(row.mute.x0 - 8.0 - x);
            p.text_fit(x, cy, name, &st, row.mute.x0 - 8.0 - x);
            let wx = x + nw + 8.0;
            if row.mute.x0 - 8.0 - wx > 24.0 {
                p.text_fit(wx, cy, &what(doc, t), &TextStyle::mono(10.5, c.text_secondary), row.mute.x0 - 8.0 - wx);
            }
            toggle(p, th, row.mute, "m", t.mixer.mute.then_some(c.accent_warning));
            if let Some(s) = row.solo {
                toggle(p, th, s, "s", t.mixer.solo.then_some(c.accent_success));
            }
        }
    });
    l
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_follow_the_tree_and_fold() {
        let doc = crate::demo::demo().doc;
        let r = Rect::new(0.0, 0.0, 300.0, 2000.0);
        let open = OutlineLayout::new(&doc, &BTreeSet::new(), 0.0, r);
        assert_eq!(open.rows.len(), doc.tracks.len(), "every track has a row");
        let group = open.rows.iter().find(|r| r.fold.is_some()).unwrap();
        let kid = open.rows.iter().find(|r| doc.tracks[&r.track].parent == Some(group.track)).unwrap();
        assert_eq!(kid.depth, group.depth + 1);
        assert_eq!(open.hit(group.fold.unwrap().center()), OutlineHit::Fold(group.track));
        assert_eq!(open.hit(kid.mute.center()), OutlineHit::Mute(kid.track));
        assert_eq!(open.hit(kid.rect.center()), OutlineHit::Row(kid.track));
        let shut = OutlineLayout::new(&doc, &BTreeSet::from([group.track]), 0.0, r);
        let kids = doc.tracks[&group.track].children.len();
        assert_eq!(shut.rows.len(), open.rows.len() - kids);
    }
}
