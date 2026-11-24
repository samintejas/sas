//! The product rules from the architecture doc, as tests.

use im::{OrdMap, Vector};
use sat_doc::sat_time::{MeterChange, Ticks, TimeSig};
use sat_doc::*;

struct Fx {
    h: History,
    track: TrackId,
    src: GridId,
    seq: ModuleId,
    clip: ClipId,
    cable: CableId,
}

fn note(start: i64, pitch: u8) -> Note {
    Note { start: Ticks::sixteenths(start), len: Ticks::sixteenths(1), pitch, velocity: 0.8, chance: 1.0 }
}

/// A project with one track, a step sequencer on its source grid wired into
/// one clip, and a pattern in that clip.
fn fixture() -> Fx {
    let mut h = History::new(Document::new(7), 100);
    let (track, op) = Op::add_track("bass", None, 0);
    h.apply("add track", [op]).unwrap();
    let src = h.doc().tracks[&track].source_grid.unwrap();
    let seq = ModuleId::new();
    let clip = ClipId::new();
    let cable = CableId::new();
    h.apply(
        "build",
        [
            Op::AddModule {
                id: seq,
                grid: src,
                kind: "step_seq".into(),
                name: "steps".into(),
                params: OrdMap::new(),
                pos: (0.0, 0.0),
            },
            Op::AddClip {
                id: clip,
                track,
                name: "bass a".into(),
                color: Color(3),
                length: Ticks::quarters(4),
                looped: true,
                seed: 1,
            },
            Op::Connect { id: cable, from: PortRef::new(seq, "notes"), to: Endpoint::Inlet(clip) },
            Op::WritePattern { module: seq, clip, notes: Vector::from(vec![note(4, 40), note(0, 36)]) },
        ],
    )
    .unwrap();
    Fx { h, track, src, seq, clip, cable }
}

#[test]
fn a_track_owns_source_instrument_and_fx_grids() {
    let f = fixture();
    let t = &f.h.doc().tracks[&f.track];
    let kinds: Vec<GridKind> = [t.source_grid.unwrap(), t.instrument_grid.unwrap(), t.fx_grids[0]]
        .iter()
        .map(|g| f.h.doc().grids[g].kind)
        .collect();
    assert_eq!(kinds, [GridKind::Source, GridKind::Instrument, GridKind::Fx]);
}

#[test]
fn patterns_are_sorted_and_keyed_by_module_and_clip() {
    let f = fixture();
    let p = f.h.doc().pattern(f.seq, f.clip).unwrap();
    assert_eq!(p.notes.iter().map(|n| n.pitch).collect::<Vec<_>>(), [36, 40]);
}

#[test]
fn one_inlet_is_one_clip() {
    let mut f = fixture();
    let other = ModuleId::new();
    let err =
        f.h.apply(
            "second feed",
            [
                Op::AddModule {
                    id: other,
                    grid: f.src,
                    kind: "euclid".into(),
                    name: "e".into(),
                    params: OrdMap::new(),
                    pos: (1.0, 0.0),
                },
                Op::Connect { id: CableId::new(), from: PortRef::new(other, "notes"), to: Endpoint::Inlet(f.clip) },
            ],
        )
        .unwrap_err();
    assert_eq!(err, OpError::InletTaken);
    // the whole transaction was refused, including the module
    assert!(!f.h.doc().modules.contains_key(&other));
}

#[test]
fn a_copy_is_a_second_wire_from_the_same_path() {
    let mut f = fixture();
    let copy = ClipId::new();
    let modules_before = f.h.doc().modules.len();
    f.h.apply("duplicate", [Op::DuplicateClip { src: f.clip, id: copy, cable: Some(CableId::new()) }]).unwrap();
    let d = f.h.doc();
    // the patch is not duplicated
    assert_eq!(d.modules.len(), modules_before);
    // the pattern is
    assert_eq!(d.pattern(f.seq, copy), d.pattern(f.seq, f.clip));
    // and the new inlet is fed from the same port
    assert_eq!(d.inlet_cable(copy).unwrap().from, d.cables[&f.cable].from);
    // editing the copy does not touch the original
    let mut f2 = f;
    f2.h.apply("edit copy", [Op::WritePattern { module: f2.seq, clip: copy, notes: Vector::new() }]).unwrap();
    assert_eq!(f2.h.doc().pattern(f2.seq, f2.clip).unwrap().notes.len(), 2);
}

#[test]
fn session_and_arrange_share_the_clip() {
    let mut f = fixture();
    let scene = SceneId::new();
    let pl = PlacementId::new();
    f.h.apply(
        "place",
        [
            Op::AddScene { id: scene, name: "1".into(), index: 0 },
            Op::SetSlot { scene, track: f.track, clip: Some(f.clip) },
            Op::AddPlacement {
                id: pl,
                clip: f.clip,
                start: Ticks::ZERO,
                offset: Ticks::ZERO,
                length: Ticks::quarters(8),
            },
        ],
    )
    .unwrap();
    assert_eq!(f.h.doc().slots[&(scene, f.track)], f.h.doc().placements[&pl].clip);
    // removing the clip empties both
    f.h.apply("remove clip", [Op::RemoveClip { id: f.clip }]).unwrap();
    let d = f.h.doc();
    assert!(d.slots.is_empty() && d.placements.is_empty() && d.patterns.is_empty() && d.cables.is_empty());
}

#[test]
fn groups_nest_and_cannot_contain_themselves() {
    let mut h = History::new(Document::new(0), 10);
    let (g1, a) = Op::add_bus(TrackKind::Group, "drums", None, 0);
    let (g2, b) = Op::add_bus(TrackKind::Group, "tops", Some(g1), 0);
    let (t, c) = Op::add_track("hat", Some(g2), 0);
    h.apply("tree", [a, b, c]).unwrap();
    assert_eq!(h.doc().tracks_in_order(), vec![g1, g2, t, h.doc().master]);
    assert_eq!(h.apply("cycle", [Op::MoveTrack { id: g1, parent: Some(g2), index: 0 }]).unwrap_err(), OpError::Cycle);
    // removing the outer group removes everything under it
    h.apply("rm", [Op::RemoveTrack { id: g1 }]).unwrap();
    assert_eq!(h.doc().tracks.len(), 1);
    assert_eq!(h.doc().grids.len(), 1);
}

#[test]
fn sends_go_to_returns_and_vanish_with_them() {
    let mut f = fixture();
    let (ret, op) = Op::add_bus(TrackKind::Return, "verb", None, 1);
    f.h.apply("return", [op]).unwrap();
    f.h.apply("send", [Op::AddSend { track: f.track, target: ret, level_db: -6.0, pre_fader: false }]).unwrap();
    let ch = f
        .h
        .apply("lvl", [Op::SetValue { target: ParamTarget::SendLevel { track: f.track, target: ret }, value: -3.0 }])
        .unwrap();
    assert!(!ch.is_structural());
    f.h.apply("rm", [Op::RemoveTrack { id: ret }]).unwrap();
    assert!(f.h.doc().tracks[&f.track].sends.is_empty());
    let err =
        f.h.apply("bad", [Op::AddSend { track: f.track, target: f.track, level_db: 0.0, pre_fader: false }])
            .unwrap_err();
    assert_eq!(err, OpError::NotAllowed("sends go to returns"));
}

#[test]
fn value_changes_are_not_structural() {
    let mut f = fixture();
    let tx = f.h.apply("knob", [Op::SetValue { target: ParamTarget::Gain(f.track), value: -4.5 }]).unwrap();
    assert_eq!(tx.changes, vec![Change::Value { target: ParamTarget::Gain(f.track), value: -4.5 }]);
    assert!(!tx.is_structural());
    let tx = f.h.apply("wire", [Op::Disconnect { id: f.cable }]).unwrap();
    assert!(tx.is_structural());
}

#[test]
fn snapshot_lives_in_the_clip() {
    let mut f = fixture();
    let grid = f.src;
    f.h.apply(
        "snap",
        [Op::SetSnapshot { clip: f.clip, target: ParamTarget::Macro { grid, index: 2 }, value: Some(0.25) }],
    )
    .unwrap();
    assert_eq!(f.h.doc().clips[&f.clip].snapshot[&ParamTarget::Macro { grid, index: 2 }], 0.25);
    let err = f.h.apply(
        "bad",
        [Op::SetSnapshot { clip: f.clip, target: ParamTarget::Macro { grid, index: 2 }, value: Some(2.0) }],
    );
    assert!(err.is_err());
}

#[test]
fn tempo_lives_on_master() {
    let mut h = History::new(Document::new(0), 10);
    let lane = h.doc().tempo_lane;
    assert_eq!(h.doc().tempo_map().unwrap().bpm_at(Ticks::ZERO), 120.0);
    let pts = Vector::from(vec![
        Breakpoint { tick: Ticks::quarters(4), value: 90.0, curve: Curve::Hold },
        Breakpoint { tick: Ticks::ZERO, value: 140.0, curve: Curve::Hold },
    ]);
    h.apply("tempo", [Op::SetLanePoints { id: lane, points: pts }]).unwrap();
    let m = h.doc().tempo_map().unwrap();
    assert_eq!(m.bpm_at(Ticks::quarters(5)), 90.0);
    // a tempo lane that does not start at 0 is refused
    let bad = Vector::unit(Breakpoint { tick: Ticks(5), value: 100.0, curve: Curve::Hold });
    assert!(h.apply("bad", [Op::SetLanePoints { id: lane, points: bad }]).is_err());
    assert_eq!(h.apply("rm", [Op::RemoveLane { id: lane }]).unwrap_err(), OpError::NotAllowed("the tempo lane stays"));
    // meter
    let ch = Vector::from(vec![
        MeterChange { tick: Ticks::ZERO, sig: TimeSig::FOUR_FOUR },
        MeterChange { tick: Ticks(3840), sig: TimeSig { num: 3, den: 4 } },
    ]);
    h.apply("meter", [Op::SetMeter { changes: ch }]).unwrap();
    assert_eq!(h.doc().meter_map().unwrap().bar_start(2), Ticks(3840 + 2880));
}

#[test]
fn a_gesture_is_one_undo_step() {
    let mut f = fixture();
    let before = f.h.doc().clone();
    f.h.begin_gesture("drag fader");
    for i in 0..10 {
        f.h.apply("fader", [Op::SetValue { target: ParamTarget::Gain(f.track), value: -(i as f32) }]).unwrap();
    }
    f.h.end_gesture();
    assert_eq!(f.h.undo_label(), Some("drag fader"));
    f.h.undo();
    assert_eq!(f.h.doc(), &before);
    f.h.redo();
    assert_eq!(f.h.doc().tracks[&f.track].mixer.gain_db, -9.0);
}

#[test]
fn removing_a_module_cleans_up() {
    let mut f = fixture();
    let lane = LaneId::new();
    let target = ParamTarget::Module { module: f.seq, param: "rate".into() };
    f.h.apply(
        "auto",
        [
            Op::SetValue { target: target.clone(), value: 0.5 },
            Op::AddLane { id: lane, target: target.clone() },
            Op::SetSnapshot { clip: f.clip, target, value: Some(0.1) },
        ],
    )
    .unwrap();
    f.h.apply("rm", [Op::RemoveModule { id: f.seq }]).unwrap();
    let d = f.h.doc();
    assert!(d.cables.is_empty() && d.patterns.is_empty() && !d.lanes.contains_key(&lane));
    assert!(d.clips[&f.clip].snapshot.is_empty());
}

#[test]
fn bundle_round_trip() {
    let f = fixture();
    let dir = tempfile::tempdir().unwrap();
    let bundle = dir.path().join("song.sat");
    file::save_bundle(&bundle, f.h.doc()).unwrap();
    for sub in file::BUNDLE_DIRS {
        assert!(bundle.join(sub).is_dir());
    }
    assert_eq!(&file::load_bundle(&bundle).unwrap(), f.h.doc());
    for i in 0..5 {
        file::write_backup(&bundle, f.h.doc(), &format!("2026112{i}"), 3).unwrap();
    }
    assert_eq!(std::fs::read_dir(bundle.join("backups")).unwrap().count(), 3);
}

#[test]
fn rejects_newer_schema_and_foreign_files() {
    let newer = serde_json::json!({"format": "sat-project", "schema": 999, "doc": {}});
    let bytes = zstd_bytes(&newer);
    assert!(matches!(file::from_bytes(&bytes), Err(file::FileError::TooNew(999))));
    let other = serde_json::json!({"format": "nope", "schema": 1, "doc": {}});
    assert!(matches!(file::from_bytes(&zstd_bytes(&other)), Err(file::FileError::NotAProject)));
}

fn zstd_bytes(v: &serde_json::Value) -> Vec<u8> {
    zstd::encode_all(&serde_json::to_vec(v).unwrap()[..], 1).unwrap()
}

#[test]
fn macros_bind_parameters_on_their_own_grid() {
    let mut f = fixture();
    let ins = f.h.doc().tracks[&f.track].instrument_grid.unwrap();
    let synth = ModuleId::new();
    let add = Op::AddModule {
        id: synth,
        grid: ins,
        kind: "vco".into(),
        name: "vco".into(),
        params: OrdMap::new(),
        pos: (0.0, 0.0),
    };
    f.h.apply("synth", [add]).unwrap();
    let bind = |module, lo, hi| MacroBind { module, param: "tune".into(), lo, hi };
    let slot = |b: MacroBind| Some(MacroSlot { name: "pitch".into(), binds: Vector::unit(b) });
    // a macro drives modules on its own grid only, over knob positions 0–1
    let other = f.h.apply("bad", [Op::SetMacroSlot { grid: f.src, index: 0, slot: slot(bind(synth, 0.0, 1.0)) }]);
    assert!(other.is_err());
    let range = f.h.apply("bad", [Op::SetMacroSlot { grid: ins, index: 0, slot: slot(bind(synth, 0.0, 2.0)) }]);
    assert!(range.is_err());
    let index = f.h.apply("bad", [Op::SetMacroSlot { grid: ins, index: 16, slot: slot(bind(synth, 0.0, 1.0)) }]);
    assert!(index.is_err());
    f.h.apply("map", [Op::SetMacroSlot { grid: ins, index: 2, slot: slot(bind(synth, 0.25, 0.75)) }]).unwrap();
    assert_eq!(f.h.doc().grids[&ins].slots[&2].name, "pitch");
    // saved and loaded
    let back = file::from_bytes(&file::to_bytes(f.h.doc()).unwrap()).unwrap();
    assert_eq!(back.grids[&ins].slots, f.h.doc().grids[&ins].slots);
    // removing the module leaves the macro with nothing to drive; undo brings it back
    f.h.apply("rm", [Op::RemoveModule { id: synth }]).unwrap();
    assert!(f.h.doc().grids[&ins].slots[&2].binds.is_empty());
    f.h.undo();
    assert_eq!(f.h.doc().grids[&ins].slots[&2].binds.len(), 1);
    f.h.apply("clear", [Op::SetMacroSlot { grid: ins, index: 2, slot: None }]).unwrap();
    assert!(f.h.doc().grids[&ins].slots.is_empty());
}
