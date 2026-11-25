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
        assert_eq!(kinds, vec![Group, Child, Child, Child, Group, Child, Group, Child, Return, Return, Master]);
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

mod steps {
    use sat_doc::sat_time::Ticks;
    use sat_doc::*;
    use sat_modules::{Editor, drum};
    use sat_ui::demo::demo;
    use sat_ui::steps::{PAGE, StepHit, StepView, geometry, source};
    use vello::kurbo::Rect;

    fn panel() -> Rect {
        Rect::new(4.0, 594.0, 1436.0, 896.0)
    }

    fn clip(d: &Document, name: &str) -> ClipId {
        d.clips.values().find(|c| c.name == name).unwrap().id
    }

    #[test]
    fn piano_roll_clips_get_no_step_editor() {
        let d = demo();
        assert_eq!(source(&d.doc, d.selected).unwrap().1, Editor::PianoRoll);
        assert!(geometry(&d.doc, Some(d.selected), panel(), &StepView::default()).is_none());
    }

    #[test]
    fn drum_cells_pages_and_rows_hit_where_laid_out() {
        let d = demo();
        let groove = clip(&d.doc, "groove");
        let g = geometry(&d.doc, Some(groove), panel(), &StepView::default()).unwrap();
        assert_eq!(g.editor, Editor::Drums);
        assert_eq!((g.steps, g.pages.len()), (32, 2), "two bars of sixteenths");
        assert_eq!(g.cells.len(), PAGE * drum::KIT.len());
        let c = g.cells[PAGE + 5];
        assert_eq!(c.pitch, drum::KIT[1].1);
        assert_eq!(g.hit(c.rect.center()), StepHit::Cell { step: 5, pitch: drum::KIT[1].1 });
        assert_eq!(g.hit(g.pages[1].center()), StepHit::Page(1));
        // the second page shows steps 16..32, and an out-of-range page clamps
        let g2 = geometry(&d.doc, Some(groove), panel(), &StepView { page: 7 }).unwrap();
        assert_eq!(g2.page, 1);
        assert_eq!(g2.cells[0].step, 16);
        assert_eq!(g2.tick_of(16), Ticks::sixteenths(16));
        // cells never overlap and stay inside the panel
        for w in g.cells.windows(2).filter(|w| w[0].pitch == w[1].pitch) {
            assert!(w[0].rect.x1 < w[1].rect.x0);
        }
        assert!(g.cells.iter().all(|c| panel().contains(c.rect.origin()) && c.rect.y1 <= panel().y1));
    }

    #[test]
    fn euclid_knobs_buttons_and_trigs_hit_where_laid_out() {
        let d = demo();
        let eu = clip(&d.doc, "euclid");
        let g = geometry(&d.doc, Some(eu), panel(), &StepView::default()).unwrap();
        assert_eq!(g.editor, Editor::Euclid);
        assert_eq!((g.steps, g.cells.len()), (16, 16));
        assert!(g.pages.is_empty());
        assert!(g.cells.iter().all(|c| c.pitch == 72));
        let keys: Vec<&str> = g.knobs.iter().map(|k| k.0).collect();
        assert_eq!(keys, ["steps", "hits", "rot", "acc", "note", "rate", "gate", "vel"]);
        assert_eq!(g.hit(g.knobs[1].1.center()), StepHit::Knob("hits"));
        assert_eq!(g.hit(g.bake.unwrap().center()), StepHit::Bake);
        assert_eq!(g.hit(g.clear.unwrap().center()), StepHit::Clear);
        assert_eq!(g.hit(g.mode.unwrap().center()), StepHit::Mode);
        assert_eq!(g.hit(g.cells[9].rect.center()), StepHit::Cell { step: 9, pitch: 72 });
        // the demo's hand edits: step 3 added, step 9 removed
        let on: Vec<i64> = sat_modules::effective_notes(&d.doc, g.module, eu).iter().map(|n| n.start.0 / 240).collect();
        assert_eq!(on, vec![0, 2, 3, 5, 7, 12, 14]);
    }
}

mod grid {
    use sat_ui::demo::demo;
    use sat_ui::grid::{GridHit, GridView, free_pos, layout, to_pos, to_screen};
    use vello::kurbo::{Point, Rect, Vec2};

    fn area() -> Rect {
        Rect::new(4.0, 44.0, 1436.0, 590.0)
    }

    #[test]
    fn cards_ports_inlets_and_wires_line_up() {
        let d = demo();
        let track = d.doc.clips[&d.selected].track;
        let grid = d.doc.tracks[&track].source_grid.unwrap();
        let l = layout(&d.doc, grid, area(), &GridView::default()).unwrap();
        assert_eq!(l.cards.len(), 2, "piano roll and an unwired euclid");
        assert_eq!(l.rows.len(), 3);
        assert_eq!(l.wires.len(), 3, "the piano roll feeds every bass clip");
        let pr = &l.cards[0];
        let out = pr.outs[0].1;
        assert!(l.wires.iter().all(|w| w.from == out));
        for (w, r) in l.wires.iter().zip(&l.rows) {
            assert!(l.rows.iter().any(|row| row.inlet == w.to), "{r:?}");
        }
        assert_eq!(l.hit(out), GridHit::Out(pr.module, "notes"));
        assert_eq!(l.hit(Point::new(out.x + 4.0, out.y + 3.0)), GridHit::Out(pr.module, "notes"));
        assert_eq!(l.hit(pr.header.center()), GridHit::Header(pr.module));
        let eu = &l.cards[1];
        assert_eq!(l.hit(eu.params[1].1.center()), GridHit::Param(eu.module, "hits"));
        assert_eq!(l.hit(eu.preview.unwrap().center()), GridHit::Module(eu.module));
        let row = l.rows[1];
        assert_eq!(l.hit(row.inlet), GridHit::Inlet(row.clip));
        assert_eq!(l.hit(row.rect.center()), GridHit::Row(row.clip));
        assert_eq!(l.inlet_near(Point::new(row.inlet.x - 12.0, row.inlet.y + 2.0)), Some(row.clip));
        assert_eq!(l.inlet_near(l.canvas.center()), None);
        assert_eq!(l.hit(l.back.center()), GridHit::Back);
        assert_eq!(l.hit(l.add_module.center()), GridHit::AddModule);
        assert_eq!(l.hit(l.add_clip.center()), GridHit::AddClip);
        assert_eq!(l.hit(Point::new(l.canvas.x1 - 40.0, l.canvas.y1 - 60.0)), GridHit::Canvas);
        // the menu lists the source modules
        let open = layout(&d.doc, grid, area(), &GridView { menu: true, ..Default::default() }).unwrap();
        assert_eq!(open.hit(open.menu[2].1.center()), GridHit::MenuItem(sat_modules::EUCLID));
    }

    #[test]
    fn panning_moves_cards_and_positions_round_trip() {
        let d = demo();
        let track = d.doc.clips[&d.selected].track;
        let grid = d.doc.tracks[&track].source_grid.unwrap();
        let v = GridView { scroll: Vec2::new(30.0, 10.0), ..Default::default() };
        let a = layout(&d.doc, grid, area(), &GridView::default()).unwrap();
        let b = layout(&d.doc, grid, area(), &v).unwrap();
        assert_eq!(a.cards[0].rect.origin() - b.cards[0].rect.origin(), Vec2::new(30.0, 10.0));
        let pt = to_screen(b.canvas, v.scroll, (120.0, 64.0));
        assert_eq!(to_pos(b.canvas, v.scroll, pt), (120.0, 64.0));
        let (x, y) = free_pos(&d.doc, grid);
        let low = d.doc.modules_on(grid).map(|m| m.pos.1 + sat_ui::grid::card_h(m) as f32).fold(0.0, f32::max);
        assert!(x == 0.0 && y > low, "below the lowest card");
        // an unknown grid has no grid view
        assert!(layout(&d.doc, sat_doc::GridId::new(), area(), &GridView::default()).is_none());
    }

    #[test]
    fn fx_grids_show_the_chain_with_bypass_switches() {
        use sat_modules::fx;
        let mut d = demo();
        let grid = d.doc.tracks[&d.doc.master].fx_grids[0];
        let l = layout(&d.doc, grid, area(), &GridView::default()).unwrap();
        assert_eq!(l.kind, sat_doc::GridKind::Fx);
        assert_eq!(l.cards.len(), 4, "in, comp, limiter, out");
        assert_eq!(l.wires.len(), 3);
        let card = |k: &str| l.cards.iter().find(|c| d.doc.modules[&c.module].kind == k).unwrap().clone();
        assert!(card(fx::FX_IN).power.is_none() && card(fx::FX_OUT).power.is_none());
        let comp = card(fx::COMP);
        let power = comp.power.unwrap();
        assert!(comp.header.contains(power.center()));
        assert_eq!(l.hit(power.center()), GridHit::Bypass(comp.module));
        assert_eq!(l.hit(comp.header.center()), GridHit::Header(comp.module));
        assert_eq!(comp.ins.iter().map(|(k, _)| *k).collect::<Vec<_>>(), vec!["in", "side"]);
        // the menu lists fx modules
        let open = layout(&d.doc, grid, area(), &GridView { menu: true, ..Default::default() }).unwrap();
        assert_eq!(open.menu.len(), fx::MODULES.len());
        // a bypassed module reads as bypassed
        let target = sat_doc::ParamTarget::Module { module: comp.module, param: fx::BYPASS.into() };
        sat_doc::apply(&mut d.doc, sat_doc::Op::SetValue { target, value: 1.0 }).unwrap();
        assert!(fx::bypassed(&d.doc.modules[&comp.module]));
    }

    #[test]
    fn instrument_grids_wire_ports_to_ports() {
        use sat_modules::instrument as ins;
        let d = demo();
        let track = d.doc.clips[&d.selected].track;
        let grid = d.doc.tracks[&track].instrument_grid.unwrap();
        let l = layout(&d.doc, grid, area(), &GridView::default()).unwrap();
        assert_eq!(l.kind, sat_doc::GridKind::Instrument);
        assert!(l.rows.is_empty() && l.dock.is_zero_area(), "no clip dock");
        assert_eq!(l.canvas.x1, area().x1, "the canvas takes the full width");
        assert_eq!(l.cards.len(), 6, "the default voice patch");
        assert_eq!(l.wires.len(), 7);
        // every cable ends on an input dot and starts on an output dot
        for w in &l.wires {
            assert!(l.cards.iter().any(|c| c.outs.iter().any(|(_, p)| *p == w.from)));
            assert!(l.cards.iter().any(|c| c.ins.iter().any(|(_, p)| *p == w.to)));
        }
        let kind = |k: &str| l.cards.iter().find(|c| d.doc.modules[&c.module].kind == k).unwrap();
        let filter = kind(ins::FILTER);
        assert!(filter.preview.is_none());
        assert_eq!(filter.params.len(), 5);
        let (key, at) = filter.ins[1];
        assert_eq!(key, "cutoff");
        assert_eq!(at.x, filter.rect.x0, "inputs on the left");
        assert_eq!(l.hit(at), GridHit::In(filter.module, "cutoff"));
        assert_eq!(l.input_near(Point::new(at.x - 10.0, at.y + 4.0)), Some((filter.module, "cutoff")));
        assert_eq!(l.input_near(l.canvas.center() + Vec2::new(0.0, 200.0)), None);
        assert_eq!(l.hit(filter.params[1].1.center()), GridHit::Param(filter.module, "cutoff"));
        assert_eq!(l.hit(filter.outs[0].1), GridHit::Out(filter.module, "out"));
        // the menu lists instrument modules
        let open = layout(&d.doc, grid, area(), &GridView { menu: true, ..Default::default() }).unwrap();
        assert_eq!(open.menu.len(), ins::MODULES.len());
        assert_eq!(open.hit(open.menu[1].1.center()), GridHit::MenuItem(ins::VCO));
        // a new module goes right of the patch, along the top
        let (x, y) = free_pos(&d.doc, grid);
        assert!(y == 0.0 && l.cards.iter().all(|c| to_screen(l.canvas, Vec2::ZERO, (x, y)).x > c.rect.x1));
        // cards in the default patch do not overlap
        for (i, a) in l.cards.iter().enumerate() {
            for b in &l.cards[i + 1..] {
                assert!(a.rect.intersect(b.rect).is_zero_area(), "{:?} {:?}", a.rect, b.rect);
            }
        }
    }
}

mod macros {
    use sat_ui::demo::demo;
    use sat_ui::macros::{MacroHit, Section, grids_for, layout};
    use vello::kurbo::Rect;

    #[test]
    fn the_page_shows_a_tracks_grids_and_its_fader() {
        let d = demo();
        let track = d.doc.clips[&d.selected].track;
        let tr = &d.doc.tracks[&track];
        let panel = Rect::new(4.0, 594.0, 1436.0, 900.0);
        let l = layout(&d.doc, Some(track), None, panel);
        let grids: Vec<_> = l.sections.iter().map(|s| s.grid).collect();
        assert_eq!(grids, vec![tr.source_grid.unwrap(), tr.instrument_grid.unwrap()]);
        let ins = &l.sections[1];
        assert_eq!(ins.knobs.len(), 16);
        let (i, cell) = ins.knobs[2];
        assert_eq!(l.hit(Section::knob_center(cell)), MacroHit::Knob(ins.grid, i));
        assert_eq!(sat_modules::macros::readout(&d.doc, ins.grid, 2).unwrap().0, "pluck");
        assert_eq!(l.hit(l.learn.center()), MacroHit::Learn);
        assert_eq!(l.hit(ins.open.center()), MacroHit::Open(ins.grid));
        assert_eq!(l.hit(l.fader.center()), MacroHit::Fader(track));
        assert!(l.sections.iter().all(|s| s.rect.x1 <= l.track_card.x0), "sections sit left of the track card");
        // with an fx grid on screen, the page shows that grid; a bus shows its fx grid
        let fx = tr.fx_grids[0];
        assert_eq!(grids_for(&d.doc, Some(track), Some(fx)), vec![fx]);
        let master = &d.doc.tracks[&d.doc.master];
        assert_eq!(grids_for(&d.doc, Some(d.doc.master), None), vec![master.fx_grids[0]]);
        assert!(grids_for(&d.doc, None, None).is_empty());
    }
}

#[test]
fn audio_clips_show_take_lanes_and_fade_handles() {
    let (d, l) = layout();
    let vox = d.doc.tracks.values().find(|t| t.name == "vox").unwrap().id;
    assert_eq!(sat_ui::arrange::take_lanes(&d.doc, vox), 3);
    let row = l.rows.iter().find(|r| r.track == vox).unwrap();
    assert_eq!(row.h, 52.0 + 3.0 * sat_ui::arrange::TAKE_H + 4.0, "taller, for its take lanes");
    let verse = l.takes.iter().find(|k| k.take == 1 && k.origin == bars(4)).unwrap();
    // a point a bar into the clip on take 2's lane
    let x = l.x_of(bars(5));
    match l.hit(Point::new(x, verse.rect.center().y)) {
        Hit::Take { take, at, .. } => {
            assert_eq!(take, 1);
            assert!((at.0 - bars(1).0).abs() <= 20, "{at:?}");
        }
        h => panic!("{h:?}"),
    }
    let cb = l.clips.iter().find(|c| c.placement == verse.placement).unwrap();
    let (fin, fout) = cb.fades.unwrap();
    assert!(matches!(l.hit(fin), Hit::Fade { out: false, .. }));
    assert!(matches!(l.hit(fout), Hit::Fade { out: true, .. }));
    assert!(cb.rect.y1 <= verse.rect.y0, "lanes sit under the clip");
    // note clips have no fades
    assert!(l.clips.iter().filter(|c| d.doc.clips[&c.clip].audio.is_none()).all(|c| c.fades.is_none()));
}
