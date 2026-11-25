//! Clip edits in the arrange view, as pure functions from a placement as it
//! was when the drag began to the span it has now. The app turns spans into
//! `UpdatePlacement` ops, so every drag replays from the same start.

use sat_doc::sat_time::{PPQ, Ticks};
use sat_doc::{Clip, ClipId, Document, Op, Placement, PlacementId};

/// Where a placement plays: start on the timeline, offset into its clip, length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    /// Start on the timeline.
    pub start: Ticks,
    /// Where in the clip playback starts.
    pub offset: Ticks,
    /// How long it plays.
    pub length: Ticks,
}

impl Span {
    /// The span of a placement.
    pub fn of(p: &Placement) -> Span {
        Span { start: p.start, offset: p.offset, length: p.length }
    }

    /// Where it ends.
    pub fn end(&self) -> Ticks {
        self.start + self.length
    }

    /// The op that gives placement `id` this span.
    pub fn op(&self, id: PlacementId) -> Op {
        Op::UpdatePlacement { id, start: self.start, offset: self.offset, length: self.length }
    }
}

/// `t` rounded to the nearest multiple of `snap` (unchanged when off).
pub fn snap_round(t: Ticks, snap: Option<Ticks>) -> Ticks {
    match snap {
        Some(s) if s.0 > 0 => Ticks(((t.0 as f64 / s.0 as f64).round() as i64) * s.0),
        _ => t,
    }
}

/// The shortest a placement can be trimmed to: one snap step, else a sixteenth.
pub fn min_len(snap: Option<Ticks>) -> Ticks {
    snap.filter(|s| s.0 > 0).unwrap_or(Ticks(PPQ / 4))
}

/// `from` moved by `delta` ticks, its start snapped and kept at or after zero.
pub fn moved(from: Span, delta: i64, snap: Option<Ticks>) -> Span {
    let start = snap_round(Ticks(from.start.0 + delta), snap).max(Ticks::ZERO);
    Span { start, ..from }
}

/// `from` with an edge dragged by `delta` ticks, snapped.
///
/// The right edge sets the length; the left edge moves the start and shifts
/// the offset with it, so the music stays where it was on the timeline. A
/// looping clip can grow either way (its offset wraps); a one-shot clip
/// stops at its own start and end.
pub fn trimmed(from: Span, clip: &Clip, right: bool, delta: i64, snap: Option<Ticks>) -> Span {
    let min = min_len(snap).0.min(from.length.0);
    let len = clip.length.0.max(1);
    if right {
        let mut end = snap_round(Ticks(from.end().0 + delta), snap).0;
        if !clip.looped {
            end = end.min(from.start.0 + (len - from.offset.0));
        }
        let length = (end - from.start.0).max(min);
        return Span { length: Ticks(length), ..from };
    }
    let mut start = snap_round(Ticks(from.start.0 + delta), snap).0.max(0);
    start = start.min(from.end().0 - min);
    if !clip.looped {
        // the offset cannot go before the clip's start
        start = start.max(from.start.0 - from.offset.0);
    }
    let shift = start - from.start.0;
    let offset = (from.offset.0 + shift).rem_euclid(if clip.looped { len } else { i64::MAX });
    Span { start: Ticks(start), offset: Ticks(offset), length: Ticks(from.length.0 - shift) }
}

/// Ops that copy placement `p` into a new clip of its own, playing `span`:
/// the copy's id and the ops. The copy plays the same notes or audio but
/// edits to it no longer change the original.
pub fn copy_ops(doc: &Document, p: &Placement, span: Span) -> Option<(ClipId, PlacementId, Vec<Op>)> {
    doc.clips.get(&p.clip)?;
    let id = ClipId::new();
    let cable = doc.inlet_cable(p.clip).map(|_| sat_doc::CableId::new());
    let pid = PlacementId::new();
    let ops = vec![
        Op::DuplicateClip { src: p.clip, id, cable },
        Op::AddPlacement { id: pid, clip: id, start: span.start, offset: span.offset, length: span.length },
    ];
    Some((id, pid, ops))
}

/// Ops that remove placement `p`, and its clip too when nothing else plays
/// it (no other placement, no session slot), so the clip dock stays tidy.
pub fn remove_ops(doc: &Document, p: PlacementId) -> Vec<Op> {
    let Some(pl) = doc.placements.get(&p) else { return Vec::new() };
    let others = doc.placements.values().any(|q| q.id != p && q.clip == pl.clip);
    let slotted = doc.slots.values().any(|c| *c == pl.clip);
    if others || slotted { vec![Op::RemovePlacement { id: p }] } else { vec![Op::RemoveClip { id: pl.clip }] }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(len: i64, looped: bool) -> Clip {
        let doc = crate::demo::demo().doc;
        let mut c = doc.clips.values().next().unwrap().clone();
        c.length = Ticks(len);
        c.looped = looped;
        c
    }

    const BAR: i64 = PPQ * 4;

    fn span(start: i64, offset: i64, length: i64) -> Span {
        Span { start: Ticks(start), offset: Ticks(offset), length: Ticks(length) }
    }

    #[test]
    fn moving_snaps_the_start_and_stops_at_zero() {
        let s = span(BAR * 2, 0, BAR);
        assert_eq!(moved(s, BAR + 100, Some(Ticks(BAR))).start, Ticks(BAR * 3));
        assert_eq!(moved(s, -BAR * 5, Some(Ticks(BAR))).start, Ticks::ZERO);
        assert_eq!(moved(s, 7, None).start, Ticks(BAR * 2 + 7));
    }

    #[test]
    fn a_looping_clip_grows_both_ways() {
        let c = clip(BAR, true);
        let s = span(BAR * 4, 0, BAR * 2);
        assert_eq!(trimmed(s, &c, true, BAR * 3, Some(Ticks(BAR))), span(BAR * 4, 0, BAR * 5));
        // the left edge pulled back a beat: the offset wraps to the last beat of the clip
        assert_eq!(trimmed(s, &c, false, -PPQ, Some(Ticks(PPQ))), span(BAR * 4 - PPQ, BAR - PPQ, BAR * 2 + PPQ));
        // trimmed in from the left by a beat
        assert_eq!(trimmed(s, &c, false, PPQ, Some(Ticks(PPQ))), span(BAR * 4 + PPQ, PPQ, BAR * 2 - PPQ));
    }

    #[test]
    fn a_one_shot_clip_stops_at_its_ends() {
        let c = clip(BAR * 2, false);
        let s = span(BAR, 0, BAR * 2);
        assert_eq!(trimmed(s, &c, true, BAR * 4, None).length, Ticks(BAR * 2));
        assert_eq!(trimmed(s, &c, false, -BAR, None), s);
        let t = trimmed(s, &c, false, PPQ, None);
        assert_eq!(t, span(BAR + PPQ, PPQ, BAR * 2 - PPQ));
    }

    #[test]
    fn trimming_keeps_a_minimum_length() {
        let c = clip(BAR, true);
        let s = span(0, 0, BAR);
        assert_eq!(trimmed(s, &c, true, -BAR * 2, Some(Ticks(PPQ))).length, Ticks(PPQ));
        assert_eq!(trimmed(s, &c, false, BAR * 2, Some(Ticks(PPQ))), span(BAR - PPQ, BAR - PPQ, PPQ));
    }

    #[test]
    fn removing_the_last_placement_removes_the_clip() {
        let doc = crate::demo::demo().doc;
        let lone = doc
            .placements
            .values()
            .find(|p| {
                doc.placements.values().filter(|q| q.clip == p.clip).count() == 1
                    && !doc.slots.values().any(|c| *c == p.clip)
            })
            .expect("a clip placed once");
        assert_eq!(remove_ops(&doc, lone.id), vec![Op::RemoveClip { id: lone.clip }]);
    }

    #[test]
    fn a_copy_is_a_new_clip_that_plays_the_same_notes() {
        let doc = crate::demo::demo().doc;
        let p = *doc.placements.values().find(|p| doc.inlet_cable(p.clip).is_some()).unwrap();
        let to = Span { start: p.start + p.length, ..Span::of(&p) };
        let (id, pid, ops) = copy_ops(&doc, &p, to).unwrap();
        let mut h = sat_doc::History::new(doc, 10);
        h.apply("copy", ops).unwrap();
        let d = h.doc();
        assert_eq!(d.placements[&pid].start, to.start);
        let src = d.inlet_cable(p.clip).unwrap().from.clone();
        assert_eq!(d.inlet_cable(id).unwrap().from, src);
        assert_eq!(
            sat_modules::effective_notes(d, src.module, id),
            sat_modules::effective_notes(d, src.module, p.clip)
        );
    }
}
