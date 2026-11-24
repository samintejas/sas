//! A demo project shaped like the design prototype's arrange view, used by
//! the screenshot tool, the tests and the app until projects can be opened.

use im::{OrdMap, Vector};
use sat_doc::sat_time::Ticks;
use sat_doc::*;

/// Bars → ticks in 4/4.
pub fn bars(n: i64) -> Ticks {
    Ticks::quarters(n * 4)
}

fn n(start16: i64, len16: i64, pitch: u8, vel: f32) -> Note {
    Note { start: Ticks::sixteenths(start16), len: Ticks::sixteenths(len16), pitch, velocity: vel, chance: 1.0 }
}

struct B {
    ops: Vec<Op>,
}

impl B {
    fn group(&mut self, name: &str, color: u8, index: usize) -> TrackId {
        let (id, op) = Op::add_bus(TrackKind::Group, name, None, index);
        self.ops.push(op);
        self.ops.push(Op::SetTrackColor { id, color: Color(color) });
        id
    }

    fn track(&mut self, name: &str, color: u8, parent: Option<TrackId>, synth: &str, wave: f32) -> (TrackId, ModuleId) {
        let (id, op) = Op::add_track(name, parent, 99);
        let Op::AddTrack { grids, .. } = &op else { unreachable!() };
        let (src, ins) = (grids.source.unwrap(), grids.instrument.unwrap());
        self.ops.push(op);
        self.ops.push(Op::SetTrackColor { id, color: Color(color) });
        let seq = ModuleId::new();
        self.ops.push(Op::AddModule {
            id: ModuleId::new(),
            grid: ins,
            kind: "test_synth".into(),
            name: synth.into(),
            params: OrdMap::from(vec![("wave".to_string(), wave), ("level".to_string(), 0.2)]),
            pos: (0.0, 0.0),
        });
        self.ops.push(Op::AddModule {
            id: seq,
            grid: src,
            kind: "piano_roll".into(),
            name: "piano roll".into(),
            params: OrdMap::new(),
            pos: (0.0, 0.0),
        });
        (id, seq)
    }

    #[allow(clippy::too_many_arguments)]
    fn clip(
        &mut self,
        track: TrackId,
        seq: ModuleId,
        name: &str,
        color: u8,
        len_bars: i64,
        notes: Vec<Note>,
        start_bar: i64,
        play_bars: i64,
    ) -> ClipId {
        let id = ClipId::new();
        self.ops.extend([
            Op::AddClip {
                id,
                track,
                name: name.into(),
                color: Color(color),
                length: bars(len_bars),
                looped: true,
                seed: 1,
            },
            Op::Connect { id: CableId::new(), from: PortRef::new(seq, "notes"), to: Endpoint::Inlet(id) },
            Op::WritePattern { module: seq, clip: id, notes: Vector::from(notes) },
            Op::AddPlacement {
                id: PlacementId::new(),
                clip: id,
                start: bars(start_bar),
                offset: Ticks::ZERO,
                length: bars(play_bars),
            },
        ]);
        id
    }
}

/// The demo project and the clip that starts selected.
pub struct Demo {
    /// The document.
    pub doc: Document,
    /// The clip shown in the bottom panel.
    pub selected: ClipId,
}

/// Plausible meter levels for screenshots: each strip reads a little below
/// its fader, armed and silent tracks read nothing.
pub fn levels(doc: &Document) -> crate::mixer::Levels {
    doc.tracks
        .values()
        .filter(|t| !t.armed && !t.mixer.mute)
        .map(|t| {
            let peak = 10f32.powf((t.mixer.gain_db - 3.0) / 20.0);
            (t.id, (peak, peak * 0.84))
        })
        .collect()
}

/// Builds the demo project.
pub fn demo() -> Demo {
    let mut doc = Document::new(42);
    let mut b = B { ops: Vec::new() };
    let rhythm = b.group("rhythm", 0, 0);
    let (drums, dseq) = b.track("drums", 0, Some(rhythm), "drum rack", 0.0);
    let kick = |bars: i64| -> Vec<Note> {
        (0..bars * 4)
            .flat_map(|q| {
                let mut v = vec![n(q * 4, 1, 36, 1.0)];
                if q % 2 == 1 {
                    v.push(n(q * 4, 1, 38, 0.8));
                }
                v.push(n(q * 4 + 2, 1, 42, 0.5));
                v
            })
            .collect()
    };
    b.clip(drums, dseq, "intro", 0, 1, kick(1), 0, 4);
    b.clip(drums, dseq, "groove", 0, 2, kick(2), 4, 8);
    let fill: Vec<Note> = (0..16).map(|s| n(s, 1, 38 + (s % 3) as u8, 0.4 + 0.03 * s as f32)).collect();
    b.clip(drums, dseq, "fill", 0, 1, fill, 12, 4);

    let (bass, bseq) = b.track("bass", 1, Some(rhythm), "mixed · instrument", 1.0);
    let bass_a = vec![n(0, 3, 45, 0.9), n(4, 2, 48, 0.7), n(8, 2, 45, 0.8), n(11, 3, 43, 0.7), n(14, 2, 40, 0.6)];
    b.clip(bass, bseq, "bass a", 1, 1, bass_a, 0, 4);
    let bass_b = vec![
        n(0, 3, 45, 0.9),
        n(3, 1, 57, 0.6),
        n(4, 2, 48, 0.8),
        n(6, 1, 52, 0.5),
        n(8, 3, 45, 0.9),
        n(11, 2, 50, 0.7),
        n(13, 1, 48, 0.6),
        n(14, 2, 43, 0.7),
        n(16, 3, 41, 0.9),
        n(20, 2, 45, 0.7),
        n(24, 3, 40, 0.8),
        n(28, 2, 43, 0.6),
        n(30, 2, 47, 0.6),
    ];
    let selected = b.clip(bass, bseq, "bass b", 1, 2, bass_b, 4, 8);
    let bass_c = vec![n(0, 2, 45, 0.8), n(2, 2, 52, 0.6), n(6, 2, 48, 0.7), n(10, 4, 43, 0.8)];
    b.clip(bass, bseq, "bass c", 1, 1, bass_c, 12, 4);

    let tops = b.group("tops", 6, 1);
    let (keys, kseq) = b.track("keys", 6, Some(tops), "piano roll · instrument", 0.0);
    let pads = |root: u8| -> Vec<Note> {
        (0..4).flat_map(|c| [0u8, 3, 7, 10].map(|i| n(c * 16, 15, root + i + [0, 5, 3, 7][c as usize], 0.5))).collect()
    };
    b.clip(keys, kseq, "pads", 6, 4, pads(57), 0, 8);
    b.clip(keys, kseq, "pads 2", 6, 4, pads(60), 8, 8);

    let vocals = b.group("vocals", 4, 2);
    let (vox, vseq) = b.track("vox", 4, Some(vocals), "audio in", 0.0);
    let line = vec![n(0, 6, 69, 0.7), n(8, 4, 72, 0.7), n(12, 4, 71, 0.6), n(16, 12, 67, 0.7)];
    b.clip(vox, vseq, "take 2", 4, 2, line.clone(), 4, 4);
    b.clip(vox, vseq, "take 3", 4, 2, line, 12, 4);

    let (hall, op) = Op::add_bus(TrackKind::Return, "hall", None, 99);
    b.ops.push(op);
    let (echo, op) = Op::add_bus(TrackKind::Return, "echo", None, 99);
    b.ops.push(op);
    for t in [keys, vox] {
        b.ops.push(Op::AddSend { track: t, target: hall, level_db: -10.0, pre_fader: false });
    }
    b.ops.push(Op::AddSend { track: vox, target: echo, level_db: -14.0, pre_fader: false });
    b.ops.push(Op::SetTrackFlag { id: vox, flag: TrackFlag::Arm, on: true });
    b.ops.push(Op::AddSend { track: rhythm, target: hall, level_db: -24.0, pre_fader: false });
    b.ops.push(Op::AddSend { track: drums, target: hall, level_db: -18.0, pre_fader: false });
    // fader levels as in the design's mixer
    for (t, db) in [
        (rhythm, -3.0),
        (drums, -6.0),
        (bass, -8.2),
        (tops, -4.0),
        (keys, -10.5),
        (vocals, -2.0),
        (vox, -7.0),
        (hall, -12.0),
        (echo, -14.0),
        (doc.master, -1.5),
    ] {
        b.ops.push(Op::SetValue { target: ParamTarget::Gain(t), value: db });
    }

    let lane = LaneId::new();
    b.ops.push(Op::AddLane { id: lane, target: ParamTarget::Gain(hall) });
    b.ops.push(Op::SetLanePoints {
        id: lane,
        points: Vector::from(vec![
            Breakpoint { tick: bars(4), value: -12.0, curve: Curve::Hold },
            Breakpoint { tick: bars(12), value: -6.0, curve: Curve::Linear },
        ]),
    });
    let lane = LaneId::new();
    b.ops.push(Op::AddLane { id: lane, target: ParamTarget::Gain(echo) });
    b.ops.push(Op::SetLanePoints {
        id: lane,
        points: Vector::from(vec![
            Breakpoint { tick: bars(6), value: -18.0, curve: Curve::Linear },
            Breakpoint { tick: bars(8), value: -9.0, curve: Curve::Linear },
            Breakpoint { tick: bars(10), value: -9.0, curve: Curve::Linear },
            Breakpoint { tick: bars(12), value: -18.0, curve: Curve::Linear },
        ]),
    });
    let tempo = doc.tempo_lane;
    b.ops.push(Op::SetLanePoints {
        id: tempo,
        points: Vector::unit(Breakpoint { tick: Ticks::ZERO, value: 124.0, curve: Curve::Hold }),
    });
    b.ops.push(Op::SetLoop { range: LoopRange { start: bars(4), end: bars(12), enabled: true } });
    for op in b.ops {
        apply(&mut doc, op).expect("demo ops are valid");
    }
    Demo { doc, selected }
}
