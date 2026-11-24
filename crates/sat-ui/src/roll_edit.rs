//! Note edits for the piano roll, as pure functions over a pattern's notes.
//!
//! Every edit takes the notes (sorted by start, then pitch, as the document
//! keeps them) and a selection (indices into them), and returns the new notes,
//! sorted the same way, with the selection carried along. Notes that land on
//! the same start and pitch merge: the edited one wins.

use sat_doc::Note;
use sat_doc::sat_time::Ticks;

/// Notes plus a selection, as indices into the notes.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Edited {
    /// Sorted notes.
    pub notes: Vec<Note>,
    /// Selected notes.
    pub selected: Vec<usize>,
}

/// Sorts tagged notes, drops same-slot duplicates (selected ones win) and
/// splits the tags back out.
fn normalize(mut tagged: Vec<(Note, bool)>) -> Edited {
    // selected first within a slot, so they survive the dedup
    tagged.sort_by(|(a, sa), (b, sb)| a.start.cmp(&b.start).then(a.pitch.cmp(&b.pitch)).then(sb.cmp(sa)));
    tagged.dedup_by(|b, a| a.0.start == b.0.start && a.0.pitch == b.0.pitch);
    let mut out = Edited::default();
    for (i, (n, s)) in tagged.into_iter().enumerate() {
        out.notes.push(n);
        if s {
            out.selected.push(i);
        }
    }
    out
}

fn tag(notes: &[Note], selected: &[usize]) -> Vec<(Note, bool)> {
    notes.iter().enumerate().map(|(i, n)| (*n, selected.contains(&i))).collect()
}

/// Rounds `t` down to the grid.
pub fn snap_floor(t: Ticks, snap: Option<Ticks>) -> Ticks {
    match snap {
        Some(s) if s.0 > 0 => Ticks(t.0.div_euclid(s.0) * s.0),
        _ => t,
    }
}

/// Rounds `t` to the nearest grid line.
pub fn snap_round(t: Ticks, snap: Option<Ticks>) -> Ticks {
    match snap {
        Some(s) if s.0 > 0 => Ticks((t.0 + s.0 / 2).div_euclid(s.0) * s.0),
        _ => t,
    }
}

/// Adds `note` (clamped into the clip) and selects only it.
pub fn add(notes: &[Note], note: Note, len: Ticks) -> Edited {
    let mut n = note;
    n.start = Ticks(n.start.0.clamp(0, (len.0 - 1).max(0)));
    n.len = Ticks(n.len.0.clamp(1, (len.0 - n.start.0).max(1)));
    let mut t = tag(notes, &[]);
    t.push((n, true));
    normalize(t)
}

/// Removes the selected notes.
pub fn delete(notes: &[Note], selected: &[usize]) -> Edited {
    let notes = notes.iter().enumerate().filter(|(i, _)| !selected.contains(i)).map(|(_, n)| (*n, false)).collect();
    normalize(notes)
}

/// Moves the selection by `dt` ticks and `dp` semitones, as a group: the
/// move is limited so every selected note stays whole inside the clip and
/// within 0–127.
pub fn move_by(notes: &[Note], selected: &[usize], dt: i64, dp: i32, len: Ticks) -> Edited {
    let sel: Vec<&Note> = selected.iter().filter_map(|&i| notes.get(i)).collect();
    if sel.is_empty() {
        return normalize(tag(notes, selected));
    }
    let min_start = sel.iter().map(|n| n.start.0).min().unwrap();
    let max_end = sel.iter().map(|n| n.start.0 + n.len.0).max().unwrap();
    let lo = sel.iter().map(|n| n.pitch as i32).min().unwrap();
    let hi = sel.iter().map(|n| n.pitch as i32).max().unwrap();
    let dt = dt.clamp(-min_start, (len.0 - max_end).max(-min_start));
    let dp = dp.clamp(-lo, 127 - hi);
    let t = notes
        .iter()
        .enumerate()
        .map(|(i, n)| {
            if selected.contains(&i) {
                let mut m = *n;
                m.start = Ticks(n.start.0 + dt);
                m.pitch = (n.pitch as i32 + dp) as u8;
                (m, true)
            } else {
                (*n, false)
            }
        })
        .collect();
    normalize(t)
}

/// Changes the length of the selected notes by `dl` ticks, keeping each at
/// least `min` long and inside the clip.
pub fn resize_by(notes: &[Note], selected: &[usize], dl: i64, min: Ticks, len: Ticks) -> Edited {
    let t = notes
        .iter()
        .enumerate()
        .map(|(i, n)| {
            if selected.contains(&i) {
                let mut m = *n;
                let max = (len.0 - n.start.0).max(1);
                m.len = Ticks((n.len.0 + dl).clamp(min.0.max(1).min(max), max));
                (m, true)
            } else {
                (*n, false)
            }
        })
        .collect();
    normalize(t)
}

/// Sets the velocity (0–1) of the selected notes.
pub fn set_velocity(notes: &[Note], selected: &[usize], v: f32) -> Edited {
    let v = v.clamp(0.0, 1.0);
    let t = notes
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let s = selected.contains(&i);
            let mut m = *n;
            if s {
                m.velocity = v;
            }
            (m, s)
        })
        .collect();
    normalize(t)
}

/// Snaps the starts of the selected notes (all notes if none are selected)
/// to the nearest grid line, keeping their ends inside the clip.
pub fn quantise(notes: &[Note], selected: &[usize], snap: Ticks, len: Ticks) -> Edited {
    let all = selected.is_empty();
    let t = notes
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let s = selected.contains(&i);
            let mut m = *n;
            if all || s {
                let last = snap_floor(Ticks((len.0 - 1).max(0)), Some(snap)).0;
                let q = snap_round(n.start, Some(snap)).0.clamp(0, last);
                m.start = Ticks(q);
                m.len = Ticks(m.len.0.min(len.0 - q).max(1));
            }
            (m, s)
        })
        .collect();
    normalize(t)
}

/// Indices of notes that overlap the tick range and pitch range (inclusive).
pub fn in_box(notes: &[Note], t0: Ticks, t1: Ticks, p0: u8, p1: u8) -> Vec<usize> {
    let (t0, t1) = (t0.min(t1), t0.max(t1));
    let (p0, p1) = (p0.min(p1), p0.max(p1));
    notes
        .iter()
        .enumerate()
        .filter(|(_, n)| n.start < t1 && n.start + n.len > t0 && n.pitch >= p0 && n.pitch <= p1)
        .map(|(i, _)| i)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(start16: i64, len16: i64, pitch: u8) -> Note {
        Note { start: Ticks::sixteenths(start16), len: Ticks::sixteenths(len16), pitch, velocity: 0.8, chance: 1.0 }
    }

    const BAR: Ticks = Ticks(3840);

    #[test]
    fn add_sorts_selects_and_clamps() {
        let e = add(&[n(0, 1, 60), n(8, 1, 60)], n(4, 20, 62), BAR);
        assert_eq!(e.notes.iter().map(|n| n.start.0 / 240).collect::<Vec<_>>(), vec![0, 4, 8]);
        assert_eq!(e.selected, vec![1]);
        // clamped to end at the clip's end
        assert_eq!(e.notes[1].len, Ticks::sixteenths(12));
    }

    #[test]
    fn move_keeps_the_group_inside_the_clip_and_range() {
        let notes = [n(0, 2, 60), n(14, 2, 126), n(4, 1, 50)];
        let mut sorted = notes.to_vec();
        sorted.sort_by(|a, b| a.start.cmp(&b.start).then(a.pitch.cmp(&b.pitch)));
        // select the notes at 0 and 14 (indices 0 and 2 once sorted)
        let e = move_by(&sorted, &[0, 2], Ticks::sixteenths(5).0, 5, BAR);
        // the note at 14 already ends at the clip end, so the group cannot move
        // right; it moves one semitone up (126 -> 127)
        let starts: Vec<(i64, u8)> = e.selected.iter().map(|&i| (e.notes[i].start.0 / 240, e.notes[i].pitch)).collect();
        assert_eq!(starts, vec![(0, 61), (14, 127)]);
        assert_eq!(e.notes[e.selected[1]].len, Ticks::sixteenths(2), "lengths are kept");
        let e = move_by(&sorted, &[0, 1], Ticks::sixteenths(40).0, 0, BAR);
        assert_eq!(
            e.notes[e.selected[1]].start,
            Ticks::sixteenths(15),
            "stops where the last note ends at the clip end"
        );
        // moving left past zero stops at zero
        let e = move_by(&sorted, &[0], -1000, 0, BAR);
        assert_eq!(e.notes[e.selected[0]].start, Ticks::ZERO);
    }

    #[test]
    fn moving_onto_a_note_replaces_it() {
        let notes = [n(0, 1, 60), n(4, 1, 60)];
        let e = move_by(&notes, &[0], Ticks::sixteenths(4).0, 0, BAR);
        assert_eq!(e.notes.len(), 1);
        assert_eq!(e.selected, vec![0]);
    }

    #[test]
    fn resize_velocity_delete_quantise() {
        let notes = [n(0, 4, 60), n(14, 1, 62)];
        let e = resize_by(&notes, &[0, 1], Ticks::sixteenths(4).0, Ticks::sixteenths(1), BAR);
        assert_eq!(e.notes[0].len, Ticks::sixteenths(8));
        assert_eq!(e.notes[1].len, Ticks::sixteenths(2), "kept inside the clip");
        let e = resize_by(&notes, &[0], -100_000, Ticks::sixteenths(1), BAR);
        assert_eq!(e.notes[0].len, Ticks::sixteenths(1));
        let e = set_velocity(&notes, &[1], 1.5);
        assert_eq!((e.notes[0].velocity, e.notes[1].velocity), (0.8, 1.0));
        assert_eq!(e.selected, vec![1]);
        let e = delete(&notes, &[0]);
        assert_eq!(e.notes, vec![notes[1]]);
        let off = [Note { start: Ticks(250), ..n(0, 1, 60) }, Note { start: Ticks(3780), ..n(0, 4, 61) }];
        let e = quantise(&off, &[], Ticks::sixteenths(1), BAR);
        assert_eq!(e.notes[0].start, Ticks(240));
        // the last note would round to the clip end, so it stays on the last grid line
        assert_eq!(e.notes[1].start, Ticks(3600));
        assert_eq!(e.notes[1].len, Ticks(240));
    }

    #[test]
    fn snapping_and_box_selection() {
        let s = Some(Ticks(240));
        assert_eq!(snap_floor(Ticks(479), s), Ticks(240));
        assert_eq!(snap_round(Ticks(359), s), Ticks(240));
        assert_eq!(snap_round(Ticks(360), s), Ticks(480));
        assert_eq!(snap_floor(Ticks(479), None), Ticks(479));
        let notes = [n(0, 2, 60), n(4, 1, 64), n(8, 1, 70)];
        assert_eq!(in_box(&notes, Ticks(300), Ticks::sixteenths(5), 59, 65), vec![0, 1]);
    }
}
