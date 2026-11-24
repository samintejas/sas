//! Property tests: random op sequences always undo back to the start, and
//! every intermediate document survives save and load unchanged.

use im::{OrdMap, Vector};
use proptest::prelude::*;
use sat_doc::sat_time::Ticks;
use sat_doc::*;

/// An abstract action; indices pick existing entities modulo their count.
#[derive(Debug, Clone)]
enum Act {
    AddTrack(u8),
    AddGroup(u8),
    AddReturn,
    RemoveTrack(u8),
    MoveTrack(u8, u8, u8),
    AddModule(u8),
    RemoveModule(u8),
    Connect(u8, u8),
    ConnectInlet(u8, u8),
    Disconnect(u8),
    AddClip(u8, u8),
    RemoveClip(u8),
    DuplicateClip(u8),
    WritePattern(u8, u8, Vec<(u8, u8)>),
    SetGain(u8, i8),
    SetMacro(u8, u8, u8),
    AddSend(u8, u8),
    AddScene,
    SetSlot(u8, u8, u8),
    Place(u8, u16, u8),
    AddLane(u8),
    Tempo(u8, u8),
    Undo,
    Redo,
}

fn act() -> impl Strategy<Value = Act> {
    prop_oneof![
        3 => any::<u8>().prop_map(Act::AddTrack),
        1 => any::<u8>().prop_map(Act::AddGroup),
        1 => Just(Act::AddReturn),
        1 => any::<u8>().prop_map(Act::RemoveTrack),
        1 => (any::<u8>(), any::<u8>(), any::<u8>()).prop_map(|(a, b, c)| Act::MoveTrack(a, b, c)),
        3 => any::<u8>().prop_map(Act::AddModule),
        1 => any::<u8>().prop_map(Act::RemoveModule),
        2 => (any::<u8>(), any::<u8>()).prop_map(|(a, b)| Act::Connect(a, b)),
        2 => (any::<u8>(), any::<u8>()).prop_map(|(a, b)| Act::ConnectInlet(a, b)),
        1 => any::<u8>().prop_map(Act::Disconnect),
        3 => (any::<u8>(), any::<u8>()).prop_map(|(a, b)| Act::AddClip(a, b)),
        1 => any::<u8>().prop_map(Act::RemoveClip),
        1 => any::<u8>().prop_map(Act::DuplicateClip),
        2 => (any::<u8>(), any::<u8>(), prop::collection::vec((0u8..64, 0u8..128), 0..8))
            .prop_map(|(a, b, n)| Act::WritePattern(a, b, n)),
        2 => (any::<u8>(), any::<i8>()).prop_map(|(a, b)| Act::SetGain(a, b)),
        1 => (any::<u8>(), 0u8..16, any::<u8>()).prop_map(|(a, b, c)| Act::SetMacro(a, b, c)),
        1 => (any::<u8>(), any::<u8>()).prop_map(|(a, b)| Act::AddSend(a, b)),
        1 => Just(Act::AddScene),
        1 => (any::<u8>(), any::<u8>(), any::<u8>()).prop_map(|(a, b, c)| Act::SetSlot(a, b, c)),
        1 => (any::<u8>(), any::<u16>(), 1u8..32).prop_map(|(a, b, c)| Act::Place(a, b, c)),
        1 => any::<u8>().prop_map(Act::AddLane),
        1 => (any::<u8>(), 40u8..240).prop_map(|(a, b)| Act::Tempo(a, b)),
        1 => Just(Act::Undo),
        1 => Just(Act::Redo),
    ]
}

fn pick<T: Clone>(v: &[T], i: u8) -> Option<T> {
    if v.is_empty() { None } else { Some(v[i as usize % v.len()].clone()) }
}

/// Turns an abstract action into a concrete op against the current document.
fn concretise(d: &Document, a: &Act) -> Option<Op> {
    let all_tracks: Vec<TrackId> = d.tracks.keys().copied().collect();
    let tracks: Vec<TrackId> = d.tracks.values().filter(|t| t.kind == TrackKind::Track).map(|t| t.id).collect();
    let groups: Vec<TrackId> = d.tracks.values().filter(|t| t.kind == TrackKind::Group).map(|t| t.id).collect();
    let returns: Vec<TrackId> = d.tracks.values().filter(|t| t.kind == TrackKind::Return).map(|t| t.id).collect();
    let grids: Vec<GridId> = d.grids.keys().copied().collect();
    let modules: Vec<ModuleId> = d.modules.keys().copied().collect();
    let clips: Vec<ClipId> = d.clips.keys().copied().collect();
    let cables: Vec<CableId> = d.cables.keys().copied().collect();
    let parent = |i: u8| if i % 3 == 0 { None } else { pick(&groups, i) };
    Some(match a {
        Act::AddTrack(p) => Op::add_track("t", parent(*p), *p as usize).1,
        Act::AddGroup(p) => Op::add_bus(TrackKind::Group, "g", parent(*p), *p as usize).1,
        Act::AddReturn => Op::add_bus(TrackKind::Return, "r", None, 99).1,
        Act::RemoveTrack(i) => Op::RemoveTrack { id: pick(&all_tracks, *i)? },
        Act::MoveTrack(i, p, x) => Op::MoveTrack { id: pick(&all_tracks, *i)?, parent: parent(*p), index: *x as usize },
        Act::AddModule(g) => Op::AddModule {
            id: ModuleId::new(),
            grid: if g % 2 == 0 {
                let srcs: Vec<GridId> = d.grids.values().filter(|x| x.kind == GridKind::Source).map(|x| x.id).collect();
                pick(&srcs, *g / 2).or_else(|| pick(&grids, *g))?
            } else {
                pick(&grids, *g)?
            },
            kind: "osc".into(),
            name: "m".into(),
            params: OrdMap::unit("level".to_string(), 0.5),
            pos: (*g as f32, 1.0),
        },
        Act::RemoveModule(i) => Op::RemoveModule { id: pick(&modules, *i)? },
        Act::Connect(a, b) => {
            let from = pick(&modules, *a)?;
            let g = d.modules[&from].grid;
            let peers: Vec<ModuleId> = d.modules_on(g).map(|m| m.id).collect();
            Op::Connect {
                id: CableId::new(),
                from: PortRef::new(from, "out"),
                to: Endpoint::Port(PortRef::new(pick(&peers, *b)?, "in")),
            }
        }
        Act::ConnectInlet(a, b) => {
            let clip = pick(&clips, *b)?;
            let src = d.tracks[&d.clips[&clip].track].source_grid?;
            let on: Vec<ModuleId> = d.modules_on(src).map(|m| m.id).collect();
            Op::Connect { id: CableId::new(), from: PortRef::new(pick(&on, *a)?, "notes"), to: Endpoint::Inlet(clip) }
        }
        Act::Disconnect(i) => Op::Disconnect { id: pick(&cables, *i)? },
        Act::AddClip(t, l) => Op::AddClip {
            id: ClipId::new(),
            track: pick(&tracks, *t)?,
            name: "c".into(),
            color: Color(*l),
            length: Ticks::quarters(*l as i64 % 16 + 1),
            looped: l % 2 == 0,
            seed: *l as u64,
        },
        Act::RemoveClip(i) => Op::RemoveClip { id: pick(&clips, *i)? },
        Act::DuplicateClip(i) => {
            Op::DuplicateClip { src: pick(&clips, *i)?, id: ClipId::new(), cable: Some(CableId::new()) }
        }
        Act::WritePattern(m, c, notes) => Op::WritePattern {
            clip: pick(&clips, *c)?,
            module: {
                let src = d.tracks[&d.clips[&pick(&clips, *c)?].track].source_grid?;
                let on: Vec<ModuleId> = d.modules_on(src).map(|m| m.id).collect();
                pick(&on, *m)?
            },
            notes: notes
                .iter()
                .map(|(s, p)| Note {
                    start: Ticks::sixteenths(*s as i64),
                    len: Ticks::sixteenths(1),
                    pitch: *p,
                    velocity: 0.5,
                    chance: 1.0,
                })
                .collect(),
        },
        Act::SetGain(t, v) => {
            Op::SetValue { target: ParamTarget::Gain(pick(&all_tracks, *t)?), value: *v as f32 / 4.0 }
        }
        Act::SetMacro(g, i, v) => {
            Op::SetValue { target: ParamTarget::Macro { grid: pick(&grids, *g)?, index: *i }, value: *v as f32 / 255.0 }
        }
        Act::AddSend(t, r) => {
            Op::AddSend { track: pick(&all_tracks, *t)?, target: pick(&returns, *r)?, level_db: -6.0, pre_fader: false }
        }
        Act::AddScene => Op::AddScene { id: SceneId::new(), name: "s".into(), index: 0 },
        Act::SetSlot(s, t, c) => {
            let scenes: Vec<SceneId> = d.scenes.iter().map(|s| s.id).collect();
            let clip = pick(&clips, *c)?;
            let track = if t % 4 == 0 { pick(&all_tracks, *t)? } else { d.clips[&clip].track };
            Op::SetSlot { scene: pick(&scenes, *s)?, track, clip: Some(clip) }
        }
        Act::Place(c, s, l) => Op::AddPlacement {
            id: PlacementId::new(),
            clip: pick(&clips, *c)?,
            start: Ticks(*s as i64 * 10),
            offset: Ticks::ZERO,
            length: Ticks::quarters(*l as i64),
        },
        Act::AddLane(t) => Op::AddLane { id: LaneId::new(), target: ParamTarget::Gain(pick(&all_tracks, *t)?) },
        Act::Tempo(at, bpm) => {
            let mut pts: Vector<Breakpoint> = d.tempo_lane().points.clone();
            pts.push_back(Breakpoint { tick: Ticks::quarters(*at as i64), value: *bpm as f32, curve: Curve::Linear });
            Op::SetLanePoints { id: d.tempo_lane, points: pts }
        }
        Act::Undo | Act::Redo => return None,
    })
}

fn check_invariants(d: &Document) {
    // the track tree and the parent links agree, and every node is reachable once
    let order = d.tracks_in_order();
    assert_eq!(order.len(), d.tracks.len());
    for t in d.tracks.values() {
        assert!(d.children_of(t.parent).contains(&t.id) || t.kind == TrackKind::Master);
        for g in t.source_grid.iter().chain(t.instrument_grid.iter()).chain(t.fx_grids.iter()) {
            assert_eq!(d.grids[g].track, t.id);
        }
    }
    // no dangling references
    for m in d.modules.values() {
        assert!(d.grids.contains_key(&m.grid));
    }
    for c in d.cables.values() {
        assert!(d.modules.contains_key(&c.from.module));
        match &c.to {
            Endpoint::Port(p) => assert!(d.modules.contains_key(&p.module)),
            Endpoint::Inlet(k) => assert!(d.clips.contains_key(k)),
        }
    }
    for ((m, c), _) in d.patterns.iter() {
        assert!(d.modules.contains_key(m) && d.clips.contains_key(c));
    }
    for p in d.placements.values() {
        assert!(d.clips.contains_key(&p.clip));
    }
    for ((_, t), c) in d.slots.iter() {
        assert_eq!(d.clips[c].track, *t);
    }
    for c in d.clips.values() {
        assert!(d.tracks.contains_key(&c.track));
    }
    for t in d.tracks.values() {
        for s in t.sends.iter() {
            assert_eq!(d.tracks[&s.target].kind, TrackKind::Return);
        }
    }
    for l in d.lanes.values() {
        assert!(d.base_value(&l.target).is_some(), "dangling lane {:?}", l.target);
    }
    // inlets take one cable
    let mut inlets: Vec<ClipId> =
        d.cables.values().filter_map(|c| if let Endpoint::Inlet(k) = c.to { Some(k) } else { None }).collect();
    let n = inlets.len();
    inlets.sort();
    inlets.dedup();
    assert_eq!(n, inlets.len());
    d.tempo_map().unwrap();
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    #[test]
    fn random_ops_undo_to_start_and_round_trip(acts in prop::collection::vec(act(), 1..60), group in 1usize..4) {
        let start = Document::new(42);
        let mut h = History::new(start.clone(), 10_000);
        let mut batch = Vec::new();
        for a in &acts {
            match a {
                Act::Undo => { h.undo(); }
                Act::Redo => { h.redo(); }
                _ => {
                    // concretise against the doc as it will be when the batch runs
                    let mut preview = h.doc().clone();
                    for op in &batch { let _ = apply(&mut preview, Clone::clone(op)); }
                    if let Some(op) = concretise(&preview, a) { batch.push(op); }
                    if batch.len() >= group {
                        let before = h.doc().clone();
                        if h.apply("batch", batch.drain(..)).is_err() {
                            // a refused transaction changes nothing
                            prop_assert_eq!(h.doc(), &before);
                        }
                    }
                }
            }
            check_invariants(h.doc());
            let back = file::from_bytes(&file::to_bytes(h.doc()).unwrap()).unwrap();
            prop_assert_eq!(&back, h.doc());
        }
        while h.undo().is_some() {}
        prop_assert_eq!(h.doc(), &start);
    }
}
