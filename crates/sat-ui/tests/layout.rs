//! Arrange layout and hit testing, without a GPU.

use sat_doc::sat_time::Ticks;
use sat_doc::*;
use sat_ui::arrange::{ArrangeLayout, ArrangeState, Hit, RowKind};
use sat_ui::demo::{bars, demo};
use vello::kurbo::{Point, Rect};

fn layout() -> (sat_ui::demo::Demo, ArrangeLayout) {
    let d = demo();
    let l = ArrangeLayout::new(&d.doc, &ArrangeState::default(), Rect::new(0.0, 0.0, 1400.0, 700.0));
    (d, l)
}

#[test]
fn rows_follow_the_track_tree_with_returns_and_master_last() {
    let (d, l) = layout();
    let kinds: Vec<RowKind> = l.rows.iter().map(|r| r.kind).collect();
    assert_eq!(kinds.last(), Some(&RowKind::Master));
    assert_eq!(l.rows.last().unwrap().track, d.doc.master);
    assert_eq!(kinds.iter().filter(|k| **k == RowKind::Return).count(), 2);
    // a group comes right before its first child, which is indented one level
    let first = &l.rows[0];
    assert_eq!(first.kind, RowKind::Group);
    assert_eq!(l.rows[1].depth, 1);
    // rows stack without gaps
    for w in l.rows.windows(2) {
        assert_eq!(w[0].y + w[0].h, w[1].y);
    }
}

#[test]
fn time_maps_to_x_and_back() {
    let (_, l) = layout();
    for t in [Ticks::ZERO, bars(3), bars(7) + Ticks::QUARTER] {
        assert_eq!(l.tick_at(l.x_of(t)), t);
    }
}

#[test]
fn clicks_find_clips_headers_and_the_ruler() {
    let (d, l) = layout();
    let cb = l.clips.iter().find(|c| c.clip == d.selected).unwrap();
    assert_eq!(l.hit(cb.rect.center()), Hit::Clip { placement: cb.placement, clip: d.selected });
    let track = d.doc.clips[&d.selected].track;
    let row = l.rows.iter().find(|r| r.track == track && r.kind == RowKind::Track).unwrap();
    assert_eq!(l.hit(Point::new(20.0, row.y + 10.0)), Hit::Header(track));
    let x = l.x_of(bars(2));
    assert_eq!(l.hit(Point::new(x, l.ruler.center().y)), Hit::Ruler(bars(2)));
    // empty space in the bass lane before nothing is placed? every bar has a clip, so use the group lane
    let group_row = &l.rows[0];
    assert!(matches!(l.hit(Point::new(x, group_row.y + 2.0)), Hit::Lane(t, _) if t == group_row.track));
}

#[test]
fn collapsing_a_group_hides_its_children_and_their_clips() {
    let d = demo();
    let group = d.doc.root[0];
    let mut st = ArrangeState::default();
    st.collapsed.insert(group);
    let l = ArrangeLayout::new(&d.doc, &st, Rect::new(0.0, 0.0, 1400.0, 700.0));
    let kids = &d.doc.tracks[&group].children;
    assert!(l.rows.iter().all(|r| !kids.contains(&r.track)));
    assert!(l.clips.iter().all(|c| !kids.contains(&d.doc.clips[&c.clip].track)));
}

#[test]
fn zoom_and_scroll() {
    let d = demo();
    let st = ArrangeState { origin: bars(4), px_per_bar: 200.0, ..Default::default() };
    let l = ArrangeLayout::new(&d.doc, &st, Rect::new(0.0, 0.0, 1400.0, 700.0));
    assert_eq!(l.x_of(bars(4)), l.timeline.x0);
    assert_eq!(l.x_of(bars(5)) - l.x_of(bars(4)), 200.0);
    // clips that end before the origin are culled
    assert!(l.clips.iter().all(|c| c.rect.x1 >= l.timeline.x0));
}

#[test]
fn position_display() {
    let d = demo();
    assert_eq!(sat_ui::shell::format_pos(&d.doc, Ticks::ZERO), "001.01.01");
    assert_eq!(sat_ui::shell::format_pos(&d.doc, bars(6) + Ticks::QUARTER + Ticks::sixteenths(1)), "007.02.02");
    let _ = TrackKind::Track;
}
