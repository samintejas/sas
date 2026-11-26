//! The left sidebar's clip bank. Every clip in the project, grouped by track
//! with the cards wrapping to the sidebar's width, and how often the
//! arrangement and the session use each.
//! Clips are dragged from here onto their track's lane or a session slot;
//! a clip deleted from the arrangement stays here, unused, until it is
//! deleted from the bank.

use sat_doc::sat_time::PPQ;
use sat_doc::*;
use sat_theme::{Rgba, Theme};
use vello::kurbo::{Point, Rect};

use crate::icons::{self, Icon};
use crate::paint::{Align, Painter, TextStyle};
use crate::panel::TABS_H;

/// Width of the card dragged out of the bank.
pub const CARD_W: f64 = 132.0;
/// A clip card's height.
pub const CARD_H: f64 = 46.0;
/// Narrowest a card in the bank gets before the grid drops a column.
pub const MIN_CARD_W: f64 = 104.0;
/// Height of a track's heading.
pub const HEAD_H: f64 = 26.0;
/// Height of the hint line at the bottom.
pub const FOOT_H: f64 = 26.0;
const GAP: f64 = 6.0;

/// Scroll and selection of the clip bank.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BankView {
    /// Vertical scroll, px.
    pub scroll: f64,
    /// The selected clip (Delete removes it from the project).
    pub selected: Option<ClipId>,
}

/// How a clip is used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Uses {
    /// Placements in the arrangement.
    pub placements: usize,
    /// Session slots.
    pub slots: usize,
}

impl Uses {
    /// Used nowhere.
    pub fn unused(&self) -> bool {
        self.placements == 0 && self.slots == 0
    }
}

/// How `clip` is used.
pub fn uses(doc: &Document, clip: ClipId) -> Uses {
    Uses {
        placements: doc.placements.values().filter(|p| p.clip == clip).count(),
        slots: doc.slots.values().filter(|c| **c == clip).count(),
    }
}

/// A clip card on screen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CardBox {
    /// The clip.
    pub clip: ClipId,
    /// The card.
    pub rect: Rect,
    /// Its delete button (unused clips only).
    pub delete: Option<Rect>,
}

/// One track's group: a heading and its cards.
#[derive(Debug, Clone, PartialEq)]
pub struct BankRow {
    /// The track.
    pub track: TrackId,
    /// The group, heading included.
    pub rect: Rect,
    /// Its clips.
    pub cards: Vec<CardBox>,
    /// The add-clip button in the heading.
    pub add: Rect,
}

/// What is under a point in the bank.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BankHit {
    /// A clip card: select it, drag it to place it.
    Card(ClipId),
    /// A card's delete button.
    Delete(ClipId),
    /// A row's add button: a new clip for the track, in the bank.
    Add(TrackId),
    /// Nothing.
    Nothing,
}

/// The clip bank's geometry.
#[derive(Debug, Clone, PartialEq)]
pub struct BankLayout {
    /// The scrolling area between the header and the hint line.
    pub body: Rect,
    /// Cards per line.
    pub cols: usize,
    /// Track rows, top to bottom.
    pub rows: Vec<BankRow>,
    /// How far it can scroll.
    pub max_scroll: f64,
}

impl BankLayout {
    /// Lays out the bank of `doc` in the sidebar `r`.
    pub fn new(doc: &Document, view: &BankView, r: Rect) -> BankLayout {
        let body = Rect::new(r.x0, r.y0 + TABS_H, r.x1, (r.y1 - FOOT_H).max(r.y0 + TABS_H));
        let tracks: Vec<TrackId> =
            doc.tracks_in_order().into_iter().filter(|t| doc.tracks[t].kind == TrackKind::Track).collect();
        let avail = (body.width() - 20.0).max(MIN_CARD_W);
        let cols = (((avail + GAP) / (MIN_CARD_W + GAP)).floor() as usize).max(1);
        let cw = (avail - GAP * (cols - 1) as f64) / cols as f64;
        let height = |n: usize| HEAD_H + n.div_ceil(cols).max(1) as f64 * (CARD_H + GAP) + 8.0;
        let content: f64 = tracks.iter().map(|t| height(doc.clips_on(*t).count())).sum::<f64>() + 8.0;
        let max_scroll = (content - body.height()).max(0.0);
        let scroll = view.scroll.clamp(0.0, max_scroll);
        let mut y = body.y0 + 6.0 - scroll;
        let rows = tracks
            .into_iter()
            .map(|t| {
                let n = doc.clips_on(t).count();
                let rect = Rect::new(body.x0 + 6.0, y, body.x1 - 6.0, y + height(n));
                let x0 = body.x0 + 10.0;
                let cards = doc
                    .clips_on(t)
                    .enumerate()
                    .map(|(i, c)| {
                        let (col, line) = ((i % cols) as f64, (i / cols) as f64);
                        let cx = x0 + col * (cw + GAP);
                        let cy = y + HEAD_H + line * (CARD_H + GAP);
                        let card = Rect::new(cx, cy, cx + cw, cy + CARD_H);
                        let delete = uses(doc, c.id)
                            .unused()
                            .then(|| Rect::new(card.x1 - 18.0, card.y0 + 1.0, card.x1 - 2.0, card.y0 + 15.0));
                        CardBox { clip: c.id, rect: card, delete }
                    })
                    .collect();
                let hy = y + HEAD_H / 2.0;
                let add = Rect::new(rect.x1 - 26.0, hy - 10.0, rect.x1 - 4.0, hy + 10.0);
                y = rect.y1;
                BankRow { track: t, rect, cards, add }
            })
            .collect();
        BankLayout { body, cols, rows, max_scroll }
    }

    /// What is under `pt`.
    pub fn hit(&self, pt: Point) -> BankHit {
        if !self.body.contains(pt) {
            return BankHit::Nothing;
        }
        for row in &self.rows {
            for c in &row.cards {
                if c.delete.is_some_and(|d| d.contains(pt)) {
                    return BankHit::Delete(c.clip);
                }
                if c.rect.contains(pt) {
                    return BankHit::Card(c.clip);
                }
            }
            if row.add.contains(pt) {
                return BankHit::Add(row.track);
            }
        }
        BankHit::Nothing
    }
}

/// How a card describes a clip's uses.
pub fn uses_label(u: Uses) -> String {
    match (u.placements, u.slots) {
        (0, 0) => "unused".into(),
        (p, 0) => format!("{p}× arrange"),
        (0, s) => format!("{s}× session"),
        (p, s) => format!("{p}× arr · {s}× ses"),
    }
}

/// Draws a clip card at `r` (also the ghost dragged out of the bank).
pub fn draw_card(p: &mut Painter<'_>, th: &Theme, doc: &Document, clip: ClipId, r: Rect, selected: bool) {
    let c = &th.c;
    let Some(cl) = doc.clips.get(&clip) else { return };
    let col = th.track(cl.color.0);
    let u = uses(doc, clip);
    let faded = if u.unused() { 0.55 } else { 1.0 };
    p.rrect(r, 4.0, col.mix(Rgba::hex(0x111111), 0.16));
    p.clip(r, |p| {
        p.rect(Rect::new(r.x0, r.y0, r.x1, r.y0 + 16.0), col.alpha(faded));
        p.text_fit(
            r.x0 + 6.0,
            r.y0 + 8.0,
            &cl.name,
            &TextStyle::sans(11.0, c.on_accent_primary).medium(),
            r.width() - 28.0,
        );
        // a thumbnail of its notes, or its takes
        let body = Rect::new(r.x0 + 4.0, r.y0 + 19.0, r.x1 - 4.0, r.y1 - 13.0);
        if let Some(a) = &cl.audio {
            let label = format!("audio · {} take{}", a.takes.len(), if a.takes.len() == 1 { "" } else { "s" });
            p.text(body.x0 + 2.0, body.center().y, &label, &TextStyle::mono(10.0, col), Align::Left);
        } else if let Some(m) = doc.inlet_cable(clip).map(|k| k.from.module) {
            let notes = sat_modules::effective_notes(doc, m, clip);
            if !notes.is_empty() {
                let (lo, hi) = notes.iter().fold((127u8, 0u8), |(a, b), n| (a.min(n.pitch), b.max(n.pitch)));
                let span = (hi - lo).max(6) as f64;
                let len = cl.length.0.max(1) as f64;
                for n in &notes {
                    let x0 = body.x0 + body.width() * n.start.0 as f64 / len;
                    let x1 = (body.x0 + body.width() * (n.start.0 + n.len.0) as f64 / len).max(x0 + 1.5);
                    let y = body.y1 - 2.0 - (n.pitch - lo) as f64 / span * (body.height() - 2.0);
                    p.rect(Rect::new(x0, y, x1.min(body.x1), y + 2.0), col.alpha(0.9 * faded));
                }
            }
        } else {
            p.text(body.x0 + 2.0, body.center().y, "not wired", &TextStyle::mono(10.0, c.text_secondary), Align::Left);
        }
        let bars = cl.length.0 as f64 / (PPQ * 4) as f64;
        let ucol = if u.unused() { c.accent_warning } else { c.text_secondary };
        p.text(r.x0 + 6.0, r.y1 - 7.0, &uses_label(u), &TextStyle::mono(9.5, ucol), Align::Left);
        p.text(r.x1 - 6.0, r.y1 - 7.0, &format!("{bars}b"), &TextStyle::mono(9.5, c.text_secondary), Align::Right);
    });
    p.rrect_stroke(r.inset(-0.5), 4.0, 1.0, col.alpha(0.6 * faded));
    if selected {
        p.rrect_stroke(r.inset(1.0), 5.0, 2.0, c.accent_primary);
    }
}

/// Draws the clip bank in the sidebar `r` and returns its layout.
pub fn draw(p: &mut Painter<'_>, th: &Theme, doc: &Document, view: &BankView, r: Rect) -> BankLayout {
    let c = &th.c;
    let l = BankLayout::new(doc, view, r);
    let n = doc.clips.len();
    let unused = doc.clips.keys().filter(|k| uses(doc, **k).unused()).count();
    let note = if unused > 0 { format!("{n} · {unused} unused") } else { format!("{n}") };
    crate::dock::side_header(p, th, r, None, "clip bank", &note, crate::dock::close_button(r).x0 - 6.0);
    p.clip(l.body, |p| {
        for row in &l.rows {
            let Some(t) = doc.tracks.get(&row.track) else { continue };
            let tc = th.track(t.color.0);
            let hy = row.rect.y0 + HEAD_H / 2.0;
            p.rrect(Rect::new(row.rect.x0 + 4.0, hy - 5.0, row.rect.x0 + 7.0, hy + 5.0), 1.5, tc);
            let k = doc.clips_on(row.track).count();
            let count = format!("{k}");
            let cw = p.text(row.add.x0 - 8.0, hy, &count, &TextStyle::mono(10.5, c.text_secondary), Align::Right);
            p.text_fit(
                row.rect.x0 + 14.0,
                hy,
                &t.name,
                &TextStyle::sans(12.5, c.text_strong).medium(),
                (row.add.x0 - 16.0 - cw - row.rect.x0 - 14.0).max(0.0),
            );
            p.rrect(row.add, 4.0, c.bg_control);
            p.rrect_stroke(row.add.inset(-0.5), 4.0, 1.0, c.border_default);
            icons::draw(p, Icon::Plus, row.add.inset(-6.0), c.text_secondary);
            for card in &row.cards {
                draw_card(p, th, doc, card.clip, card.rect, view.selected == Some(card.clip));
                if let Some(d) = card.delete {
                    icons::draw(p, Icon::Close, d.inset(-4.0), c.on_accent_primary);
                }
            }
            p.hline(row.rect.x0, row.rect.x1, row.rect.y1 - 1.0, c.border_subtle);
        }
    });
    if l.max_scroll > 0.0 {
        let track = Rect::new(r.x1 - 4.0, l.body.y0 + 2.0, r.x1 - 2.0, l.body.y1 - 2.0);
        let frac = l.body.height() / (l.body.height() + l.max_scroll);
        let at = view.scroll.clamp(0.0, l.max_scroll) / l.max_scroll;
        let th_h = (track.height() * frac).max(24.0);
        let y = track.y0 + (track.height() - th_h) * at;
        p.rrect(Rect::new(track.x0, y, track.x1, y + th_h), 1.0, c.border_strong.alpha(0.6));
    }
    let foot = Rect::new(r.x0, l.body.y1, r.x1, r.y1);
    p.hline(foot.x0, foot.x1, foot.y0, c.border_subtle);
    p.clip(foot, |p| {
        let hint = "drag onto a lane · double-click places";
        p.text(foot.x0 + 12.0, foot.center().y, hint, &TextStyle::mono(10.0, c.text_secondary), Align::Left);
    });
    l
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_clip_has_a_card_and_hits_find_them() {
        let doc = crate::demo::demo().doc;
        let l = BankLayout::new(&doc, &BankView::default(), Rect::new(0.0, 0.0, 264.0, 4000.0));
        assert_eq!(l.cols, 2, "two cards a line in a default sidebar");
        let cards: usize = l.rows.iter().map(|r| r.cards.len()).sum();
        let on_tracks = doc.clips.values().filter(|c| doc.tracks[&c.track].kind == TrackKind::Track).count();
        assert_eq!(cards, on_tracks);
        let first = l.rows.iter().find_map(|r| r.cards.first()).unwrap();
        assert_eq!(l.hit(first.rect.center()), BankHit::Card(first.clip));
        assert_eq!(l.hit(l.rows[0].add.center()), BankHit::Add(l.rows[0].track));
        for row in &l.rows {
            assert!(row.cards.iter().all(|c| c.rect.x1 <= 264.0 && c.rect.y0 >= row.rect.y0 + HEAD_H));
        }
        // wider, more to a line
        let wide = BankLayout::new(&doc, &BankView::default(), Rect::new(0.0, 0.0, 600.0, 4000.0));
        assert_eq!(wide.cols, 5);
    }

    #[test]
    fn uses_count_placements_and_slots() {
        let doc = crate::demo::demo().doc;
        let c = doc.placements.values().next().unwrap().clip;
        assert!(uses(&doc, c).placements >= 1);
        assert_eq!(uses_label(Uses::default()), "unused");
        assert_eq!(uses_label(Uses { placements: 2, slots: 1 }), "2× arr · 1× ses");
    }
}
