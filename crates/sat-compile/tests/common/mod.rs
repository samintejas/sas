//! A small reference song used by the render tests.

#![allow(dead_code)]

use im::{OrdMap, Vector};
use sat_doc::sat_time::Ticks;
use sat_doc::*;

pub const SR: f64 = 48_000.0;
pub const BLOCK: usize = 128;

pub struct Song {
    pub h: History,
    pub drums: TrackId,
    pub bass: TrackId,
    pub lead: TrackId,
    pub group: TrackId,
    pub ret: TrackId,
    pub bass_clip: ClipId,
    pub bass_synth: ModuleId,
}

fn id(n: u128) -> u128 {
    0x5a7_0000_0000_0000_0000_0000_0000_0000 | n
}

fn add_track(ops: &mut Vec<Op>, n: u128, name: &str, parent: Option<TrackId>) -> TrackId {
    let t = TrackId::from_u128(id(0x100 + n));
    ops.push(Op::AddTrack {
        id: t,
        kind: TrackKind::Track,
        name: name.into(),
        color: Color(n as u8),
        parent,
        index: 99,
        grids: NewGrids {
            source: Some(GridId::from_u128(id(0x1000 + n * 10 + 1))),
            instrument: Some(GridId::from_u128(id(0x1000 + n * 10 + 2))),
            fx: GridId::from_u128(id(0x1000 + n * 10 + 3)),
        },
    });
    t
}

/// Adds a test synth, a sequencer wired to one clip with `notes`, and places
/// the clip at `start` for `length`.
#[allow(clippy::too_many_arguments)]
fn instrument(
    ops: &mut Vec<Op>,
    n: u128,
    track: TrackId,
    wave: f32,
    notes: Vec<Note>,
    clip_len: Ticks,
    start: Ticks,
    length: Ticks,
) -> (ClipId, ModuleId) {
    let synth = ModuleId::from_u128(id(0x10000 + n * 100 + 1));
    let seq = ModuleId::from_u128(id(0x10000 + n * 100 + 2));
    let clip = ClipId::from_u128(id(0x10000 + n * 100 + 3));
    ops.extend([
        Op::AddModule {
            id: synth,
            grid: GridId::from_u128(id(0x1000 + n * 10 + 2)),
            kind: sat_compile::TEST_SYNTH.into(),
            name: "synth".into(),
            params: OrdMap::from(vec![("wave".to_string(), wave), ("level".to_string(), 0.3)]),
            pos: (0.0, 0.0),
        },
        Op::AddModule {
            id: seq,
            grid: GridId::from_u128(id(0x1000 + n * 10 + 1)),
            kind: "piano_roll".into(),
            name: "roll".into(),
            params: OrdMap::new(),
            pos: (0.0, 0.0),
        },
        Op::AddClip { id: clip, track, name: "clip".into(), color: Color(1), length: clip_len, looped: true, seed: 9 },
        Op::Connect {
            id: CableId::from_u128(id(0x10000 + n * 100 + 4)),
            from: PortRef::new(seq, "notes"),
            to: Endpoint::Inlet(clip),
        },
        Op::WritePattern { module: seq, clip, notes: Vector::from(notes) },
        Op::AddPlacement {
            id: PlacementId::from_u128(id(0x10000 + n * 100 + 5)),
            clip,
            start,
            offset: Ticks::ZERO,
            length,
        },
    ]);
    (clip, synth)
}

pub fn note(start16: i64, len16: i64, pitch: u8, velocity: f32) -> Note {
    Note { start: Ticks::sixteenths(start16), len: Ticks::sixteenths(len16), pitch, velocity, chance: 1.0 }
}

/// Two bars at 120 bpm: drums and bass in a group, a lead with a send to a
/// return, gain automation on the lead.
pub fn song() -> Song {
    let doc = Document::with_ids(1234, TrackId::from_u128(id(1)), GridId::from_u128(id(2)), LaneId::from_u128(id(3)));
    let mut h = History::new(doc, 100);
    let mut ops = Vec::new();
    let group = TrackId::from_u128(id(4));
    ops.push(Op::AddTrack {
        id: group,
        kind: TrackKind::Group,
        name: "rhythm".into(),
        color: Color(2),
        parent: None,
        index: 0,
        grids: NewGrids { source: None, instrument: None, fx: GridId::from_u128(id(5)) },
    });
    let ret = TrackId::from_u128(id(6));
    ops.push(Op::AddTrack {
        id: ret,
        kind: TrackKind::Return,
        name: "verb".into(),
        color: Color(3),
        parent: None,
        index: 99,
        grids: NewGrids { source: None, instrument: None, fx: GridId::from_u128(id(7)) },
    });
    let drums = add_track(&mut ops, 2, "drums", Some(group));
    let bass = add_track(&mut ops, 3, "bass", Some(group));
    let lead = add_track(&mut ops, 4, "lead", None);
    let kick: Vec<Note> = (0..4).map(|i| note(i * 4, 1, 36, 1.0)).collect();
    instrument(&mut ops, 2, drums, 0.0, kick, Ticks::quarters(4), Ticks::ZERO, Ticks::quarters(8));
    let bassline = vec![note(0, 3, 40, 0.9), note(6, 2, 43, 0.7), note(10, 4, 38, 0.8)];
    let (bass_clip, bass_synth) =
        instrument(&mut ops, 3, bass, 1.0, bassline, Ticks::quarters(4), Ticks::ZERO, Ticks::quarters(8));
    let lead_notes = vec![note(0, 8, 64, 0.6), note(8, 8, 67, 0.6)];
    instrument(&mut ops, 4, lead, 0.0, lead_notes, Ticks::quarters(4), Ticks::quarters(2), Ticks::quarters(6));
    ops.push(Op::AddSend { track: lead, target: ret, level_db: -6.0, pre_fader: false });
    let lane = LaneId::from_u128(id(8));
    ops.push(Op::AddLane { id: lane, target: ParamTarget::Gain(lead) });
    ops.push(Op::SetLanePoints {
        id: lane,
        points: Vector::from(vec![
            Breakpoint { tick: Ticks::quarters(2), value: -12.0, curve: Curve::Linear },
            Breakpoint { tick: Ticks::quarters(6), value: 0.0, curve: Curve::Hold },
        ]),
    });
    h.apply("song", ops).unwrap();
    Song { h, drums, bass, lead, group, ret, bass_clip, bass_synth }
}

pub fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

pub fn render(doc: &Document, frames: usize) -> (Vec<f32>, Vec<f32>) {
    let c = sat_compile::compile(doc, SR, BLOCK).unwrap();
    sat_engine::render(c.schedule, 0, frames, BLOCK)
}
