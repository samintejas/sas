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

#[test]
fn header_controls_are_hit_where_they_are_drawn() {
    use sat_ui::arrange::header_controls;
    let (d, l) = layout();
    let row = *l.rows.iter().find(|r| r.kind == RowKind::Track).unwrap();
    let hc = header_controls(&row, l.headers);
    let t = row.track;
    assert_eq!(l.hit(hc.mute.unwrap().center()), Hit::Mute(t));
    assert_eq!(l.hit(hc.solo.unwrap().center()), Hit::Solo(t));
    assert_eq!(l.hit(hc.arm.unwrap().center()), Hit::Arm(t));
    let f = hc.fader.unwrap();
    assert_eq!(l.hit(Point::new((f.x0 + f.x1) / 2.0, f.y + 2.0)), Hit::Fader(t));
    assert!((f.pos_at(f.x_of(0.3)) - 0.3).abs() < 1e-9);
    // the name area is still the header
    assert_eq!(l.hit(Point::new(l.headers.x0 + 20.0, row.y + 17.0)), Hit::Header(t));
    // returns and master have only a mute button
    let master = *l.rows.last().unwrap();
    let hm = header_controls(&master, l.headers);
    assert!(hm.solo.is_none() && hm.fader.is_none());
    assert_eq!(l.hit(hm.mute.unwrap().center()), Hit::Mute(d.doc.master));
}

mod mixer {
    use sat_doc::*;
    use sat_ui::demo::demo;
    use sat_ui::mixer::{Knob, MixHit, MixerLayout, StripKind, returns};
    use vello::kurbo::{Point, Rect};

    fn body() -> Rect {
        Rect::new(4.0, 500.0, 1436.0, 770.0)
    }

    #[test]
    fn strips_follow_the_tree_with_returns_then_master_on_the_right() {
        let d = demo();
        let l = MixerLayout::new(&d.doc, body(), 0.0);
        let kinds: Vec<StripKind> = l.strips.iter().map(|s| s.kind).collect();
        use StripKind::*;
        assert_eq!(kinds, vec![Group, Child, Child, Group, Child, Group, Child, Return, Return, Master]);
        let master = l.strips.last().unwrap();
        assert_eq!(master.track, d.doc.master);
        assert!((master.rect.x1 - (body().x1 - 12.0)).abs() < 1e-9, "master is pinned right");
        // returns sit after a separator
        let sep = l.separator.unwrap();
        let first_ret = l.strips.iter().find(|s| s.kind == Return).unwrap();
        assert!(sep < first_ret.rect.x0);
        // tracks and groups get send knobs a, b and pan; returns and master only pan
        let rets = returns(&d.doc);
        let knobs: Vec<Knob> = l.strips[0].knobs.iter().map(|k| k.1).collect();
        assert_eq!(knobs, vec![Knob::Send(rets[0]), Knob::Send(rets[1]), Knob::Pan]);
        assert_eq!(first_ret.knobs.len(), 1);
        assert!(master.mute.is_none());
        assert_eq!(l.max_scroll, 0.0, "the demo fits");
    }

    #[test]
    fn controls_hit_where_drawn() {
        let d = demo();
        let l = MixerLayout::new(&d.doc, body(), 0.0);
        let s = &l.strips[1];
        let t = s.track;
        assert_eq!(l.hit(s.mute.unwrap().center()), MixHit::Mute(t));
        assert_eq!(l.hit(s.solo.unwrap().center()), MixHit::Solo(t));
        assert_eq!(l.hit(s.knobs[2].0.center()), MixHit::Knob(t, Knob::Pan));
        assert_eq!(l.hit(s.cap(0.5).center()), MixHit::Fader(t));
        assert_eq!(l.hit(Point::new(s.rect.x0 + 6.0, s.rect.y0 + 8.0)), MixHit::Strip(t));
        assert_eq!(l.hit(Point::new(s.rect.x1 + 1.5, s.rect.center().y)), MixHit::Nothing);
        for pos in [0.0, 0.25, 0.83, 1.0] {
            assert!((s.fader_pos_at(s.fader_y(pos)) - pos).abs() < 1e-9);
        }
    }

    #[test]
    fn scrolling_slides_strips_under_a_fixed_master() {
        let mut d = demo();
        // enough tracks to overflow
        for i in 0..12 {
            let (_, op) = Op::add_track(format!("t{i}"), None, 0);
            apply(&mut d.doc, op).unwrap();
        }
        let l0 = MixerLayout::new(&d.doc, body(), 0.0);
        assert!(l0.max_scroll > 0.0);
        let l1 = MixerLayout::new(&d.doc, body(), 1e9);
        let (a, b) = (&l0.strips[0], &l1.strips[0]);
        assert!((a.rect.x0 - b.rect.x0 - l0.max_scroll).abs() < 1e-9, "scroll clamps to the max");
        assert_eq!(l0.strips.last().unwrap().rect, l1.strips.last().unwrap().rect);
        // the strip scrolled under master's left edge is not hit there; master is
        let m = l1.strips.last().unwrap();
        let name_row = Point::new(m.rect.x0 + 20.0, m.rect.y0 + 8.0);
        assert_eq!(l1.hit(name_row), MixHit::Strip(m.track));
        let under = Point::new(m.rect.x0 - 2.0, m.rect.y0 + 8.0);
        assert_eq!(l1.hit(under), MixHit::Nothing);
    }
}

mod roll {
    use sat_doc::sat_time::Ticks;
    use sat_ui::demo::demo;
    use sat_ui::piano_roll::{RollHit, RollView, SNAPS, Tool, clip_notes, geometry, next_snap, snap_label};
    use vello::kurbo::{Point, Rect};

    fn panel() -> Rect {
        Rect::new(4.0, 594.0, 1436.0, 896.0)
    }

    #[test]
    fn notes_grid_velocity_and_buttons_hit_where_drawn() {
        let d = demo();
        let g = geometry(&d.doc, Some(d.selected), panel(), &RollView::default()).unwrap();
        let (module, notes) = clip_notes(&d.doc, d.selected);
        assert_eq!(g.module, module);
        let l = &g.layout;
        let n = notes[3];
        let r = l.note_rect(&n);
        assert_eq!(g.hit(&notes, Point::new(r.x0 + 2.0, r.center().y)), RollHit::Note { index: 3, edge: false });
        assert_eq!(g.hit(&notes, Point::new(r.x1 - 1.0, r.center().y)), RollHit::Note { index: 3, edge: true });
        // an empty cell maps back to its tick and pitch
        let (t, p) = (Ticks(240 * 31), l.lo);
        assert!(!notes.iter().any(|n| n.pitch == p && n.start <= t && n.start + n.len > t));
        let pt = Point::new(l.x_of(t) + 0.05, l.y_of(p) + l.row_h() / 2.0);
        assert_eq!(g.hit(&notes, pt), RollHit::Grid { tick: t, pitch: p });
        // the velocity lane finds the nearest stem and reads the value
        let vx = l.x_of(notes[0].start) + 1.0;
        let RollHit::Velocity { index, value } = g.hit(&notes, Point::new(vx, l.vel_y(0.5))) else { panic!() };
        assert_eq!(index, Some(0));
        assert!((value - 0.5).abs() < 0.01);
        assert_eq!(g.hit(&notes, g.tools[1].1.center()), RollHit::Tool(Tool::Select));
        assert_eq!(g.hit(&notes, g.snap.center()), RollHit::Snap);
        assert_eq!(g.hit(&notes, g.quantise.center()), RollHit::Quantise);
    }

    #[test]
    fn a_fixed_bottom_pitch_holds_the_rows_still() {
        let d = demo();
        let view = RollView { lo: Some(30), ..Default::default() };
        let g = geometry(&d.doc, Some(d.selected), panel(), &view).unwrap();
        assert_eq!(g.layout.lo, 30);
        let top = RollView { lo: Some(127), ..Default::default() };
        let g = geometry(&d.doc, Some(d.selected), panel(), &top).unwrap();
        assert_eq!(g.layout.hi, 127, "scrolling up stops with the top row at 127");
    }

    #[test]
    fn grid_choices_cycle() {
        assert_eq!(snap_label(Some(Ticks(240))), "snap 1/16");
        assert_eq!(snap_label(None), "snap off");
        let mut s = SNAPS[0];
        for _ in 0..SNAPS.len() {
            s = next_snap(s);
        }
        assert_eq!(s, SNAPS[0]);
    }
}
