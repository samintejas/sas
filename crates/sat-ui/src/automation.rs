//! Automation lanes in the arrange view: which lanes a track shows, how a
//! value maps to a height in its lane, and breakpoint edits as pure functions.

use sat_doc::sat_time::Ticks;
use sat_doc::{Breakpoint, Curve, Document, Lane, ParamTarget, TrackId};
use vello::kurbo::Rect;

use crate::controls;

/// The lanes a track can show under its row, in order.
pub fn lane_targets(t: TrackId) -> [ParamTarget; 2] {
    [ParamTarget::Gain(t), ParamTarget::Pan(t)]
}

/// The lane driving `target`, if there is one.
pub fn lane_of<'a>(doc: &'a Document, target: &ParamTarget) -> Option<&'a Lane> {
    doc.lanes.values().find(|l| &l.target == target)
}

/// What a lane header calls its target.
pub fn target_name(target: &ParamTarget) -> &'static str {
    match target {
        ParamTarget::Gain(_) => "gain",
        ParamTarget::Pan(_) => "pan",
        ParamTarget::SendLevel { .. } => "send",
        ParamTarget::Module { .. } => "param",
        ParamTarget::Macro { .. } => "macro",
        ParamTarget::Tempo => "tempo",
    }
}

/// A value's height in its lane, 0 (bottom) to 1 (top), on the same scale
/// as the target's control (a fader's dB curve, a pan knob).
pub fn pos_of(doc: &Document, target: &ParamTarget, value: f32) -> f64 {
    match target {
        ParamTarget::Gain(_) | ParamTarget::SendLevel { .. } => controls::fader_pos(value),
        ParamTarget::Pan(_) => controls::pan_pos(value),
        ParamTarget::Module { module, param } => doc
            .modules
            .get(module)
            .and_then(|m| sat_modules::spec(&m.kind, param))
            .map_or(0.5, |s| s.pos(s.clamp(value))),
        ParamTarget::Macro { .. } => value.clamp(0.0, 1.0) as f64,
        ParamTarget::Tempo => ((value - 20.0) / 280.0).clamp(0.0, 1.0) as f64,
    }
}

/// The value at height `pos` (0–1) in a lane.
pub fn value_at(doc: &Document, target: &ParamTarget, pos: f64) -> f32 {
    let pos = pos.clamp(0.0, 1.0);
    match target {
        ParamTarget::Gain(_) | ParamTarget::SendLevel { .. } => (controls::fader_db(pos) * 10.0).round() / 10.0,
        ParamTarget::Pan(_) => (controls::pan_at(pos) * 100.0).round() / 100.0,
        ParamTarget::Module { module, param } => {
            doc.modules.get(module).and_then(|m| sat_modules::spec(&m.kind, param)).map_or(0.0, |s| s.at(pos))
        }
        ParamTarget::Macro { .. } => pos as f32,
        ParamTarget::Tempo => 20.0 + pos as f32 * 280.0,
    }
}

/// How a value reads in a lane, for the point being dragged.
pub fn fmt_value(target: &ParamTarget, value: f32) -> String {
    match target {
        ParamTarget::Gain(_) | ParamTarget::SendLevel { .. } => controls::fmt_db(value),
        ParamTarget::Pan(_) if value.abs() < 0.005 => "c".into(),
        ParamTarget::Pan(_) => format!("{}{:.0}", if value < 0.0 { "l" } else { "r" }, value.abs() * 100.0),
        _ => format!("{value:.2}"),
    }
}

/// Vertical inset of a lane's drawing area.
pub const LANE_PAD: f64 = 5.0;

/// Screen y of height `pos` in lane rect `r`.
pub fn y_of(r: Rect, pos: f64) -> f64 {
    r.y1 - LANE_PAD - (r.height() - 2.0 * LANE_PAD) * pos.clamp(0.0, 1.0)
}

/// Height (0–1) at screen y in lane rect `r`.
pub fn pos_at(r: Rect, y: f64) -> f64 {
    ((r.y1 - LANE_PAD - y) / (r.height() - 2.0 * LANE_PAD).max(1.0)).clamp(0.0, 1.0)
}

/// `points` with a new point at `tick`, and its index. A point already at
/// that tick takes the new value instead.
pub fn insert(points: &[Breakpoint], tick: Ticks, value: f32) -> (Vec<Breakpoint>, usize) {
    let mut v = points.to_vec();
    if let Some(i) = v.iter().position(|p| p.tick == tick) {
        v[i].value = value;
        return (v, i);
    }
    let i = v.partition_point(|p| p.tick < tick);
    // the new point takes the curve of the segment it splits
    let curve = i.checked_sub(1).map_or(Curve::Linear, |k| v[k].curve);
    v.insert(i, Breakpoint { tick, value, curve });
    (v, i)
}

/// `points` with point `i` moved to `tick` and `value`, kept between its
/// neighbours so the order (and its index) stays.
pub fn move_point(points: &[Breakpoint], i: usize, tick: Ticks, value: f32) -> Vec<Breakpoint> {
    let mut v = points.to_vec();
    if i >= v.len() {
        return v;
    }
    let lo = i.checked_sub(1).map_or(Ticks::ZERO, |k| v[k].tick + Ticks(1));
    let hi = v.get(i + 1).map_or(Ticks(i64::MAX), |p| p.tick - Ticks(1));
    v[i].tick = tick.max(lo).min(hi.max(lo));
    v[i].value = value;
    v
}

/// `points` without point `i`.
pub fn remove(points: &[Breakpoint], i: usize) -> Vec<Breakpoint> {
    let mut v = points.to_vec();
    if i < v.len() {
        v.remove(i);
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bp(t: i64, v: f32) -> Breakpoint {
        Breakpoint { tick: Ticks(t), value: v, curve: Curve::Linear }
    }

    #[test]
    fn inserting_keeps_ticks_sorted() {
        let pts = [bp(0, 0.0), bp(100, 1.0)];
        let (v, i) = insert(&pts, Ticks(50), 0.5);
        assert_eq!(i, 1);
        assert_eq!(v.iter().map(|p| p.tick.0).collect::<Vec<_>>(), [0, 50, 100]);
        let (v, i) = insert(&v, Ticks(100), -1.0);
        assert_eq!((v.len(), i, v[2].value), (3, 2, -1.0));
    }

    #[test]
    fn a_moved_point_stays_between_its_neighbours() {
        let pts = [bp(0, 0.0), bp(100, 1.0), bp(200, 0.0)];
        assert_eq!(move_point(&pts, 1, Ticks(500), 2.0)[1], bp(199, 2.0));
        assert_eq!(move_point(&pts, 1, Ticks(-5), 2.0)[1], bp(1, 2.0));
        assert_eq!(move_point(&pts, 0, Ticks(-5), 3.0)[0], bp(0, 3.0));
        assert_eq!(remove(&pts, 1), vec![bp(0, 0.0), bp(200, 0.0)]);
    }

    #[test]
    fn heights_round_trip_through_values() {
        let doc = crate::demo::demo().doc;
        let t = doc.master;
        for pos in [0.25, 0.5, 0.8] {
            let pan = ParamTarget::Pan(t);
            assert!((pos_of(&doc, &pan, value_at(&doc, &pan, pos)) - pos).abs() < 0.01);
            let gain = ParamTarget::Gain(t);
            assert!((pos_of(&doc, &gain, value_at(&doc, &gain, pos)) - pos).abs() < 0.01);
        }
        let r = Rect::new(0.0, 100.0, 10.0, 130.0);
        assert!((pos_at(r, y_of(r, 0.3)) - 0.3).abs() < 1e-9);
    }
}
