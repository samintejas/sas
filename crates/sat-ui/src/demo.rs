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

/// What plays a demo track.
enum Ins {
    /// The default voice patch with (module kind, parameter, value) tweaks.
    Patch(&'static [(&'static str, &'static str, f32)]),
    /// The drum synth.
    Drums,
    /// The built-in test synth with a name and a wave.
    Test(&'static str, f32),
}

impl B {
    fn group(&mut self, name: &str, color: u8, index: usize) -> TrackId {
        let (id, op) = Op::add_bus(TrackKind::Group, name, None, index);
        self.ops.push(op);
        self.ops.push(Op::SetTrackColor { id, color: Color(color) });
        id
    }

    fn track(&mut self, name: &str, color: u8, parent: Option<TrackId>, ins: Ins) -> (TrackId, ModuleId) {
        self.track_with(name, color, parent, ins, sat_modules::PIANO_ROLL, "piano roll")
    }

    /// The fx grid of a track added by these ops (or the master's).
    fn fx_grid(&self, doc: &Document, track: TrackId) -> GridId {
        self.ops
            .iter()
            .find_map(|op| match op {
                Op::AddTrack { id, grids, .. } if *id == track => Some(grids.fx),
                _ => None,
            })
            .unwrap_or_else(|| doc.tracks[&track].fx_grids[0])
    }

    /// Fills a track's fx grid with `effects` in series.
    #[allow(clippy::type_complexity)]
    fn fx(&mut self, doc: &Document, track: TrackId, effects: &[(&str, &[(&str, f32)])]) {
        let grid = self.fx_grid(doc, track);
        self.ops.extend(sat_modules::fx::chain(grid, effects));
    }

    /// The grid a module was added to.
    fn grid_of(&self, module: ModuleId) -> GridId {
        self.ops
            .iter()
            .find_map(|op| match op {
                Op::AddModule { id, grid, .. } if *id == module => Some(*grid),
                _ => None,
            })
            .expect("module added")
    }

    fn track_with(
        &mut self,
        name: &str,
        color: u8,
        parent: Option<TrackId>,
        ins_kind: Ins,
        kind: &str,
        module: &str,
    ) -> (TrackId, ModuleId) {
        let (id, op) = Op::add_track(name, parent, 99);
        let Op::AddTrack { grids, .. } = &op else { unreachable!() };
        let (src, ins) = (grids.source.unwrap(), grids.instrument.unwrap());
        self.ops.push(op);
        self.ops.push(Op::SetTrackColor { id, color: Color(color) });
        let seq = ModuleId::new();
        match ins_kind {
            Ins::Patch(tweaks) => self.ops.extend(sat_modules::instrument::default_patch_ops(ins, tweaks).0),
            Ins::Drums => {
                let kind = sat_modules::instrument::DRUM_SYNTH;
                self.ops.push(Op::AddModule {
                    id: ModuleId::new(),
                    grid: ins,
                    kind: kind.into(),
                    name: sat_modules::label_of(kind).into(),
                    params: sat_modules::default_params(kind),
                    pos: (0.0, 0.0),
                })
            }
            Ins::Test(synth, wave) => self.ops.push(Op::AddModule {
                id: ModuleId::new(),
                grid: ins,
                kind: "test_synth".into(),
                name: synth.into(),
                params: OrdMap::from(vec![("wave".to_string(), wave), ("level".to_string(), 0.2)]),
                pos: (0.0, 0.0),
            }),
        }
        self.ops.push(Op::AddModule {
            id: seq,
            grid: src,
            kind: kind.into(),
            name: module.into(),
            params: sat_modules::default_params(kind),
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

use sat_modules::instrument::{ADSR, FILTER, NOTE_IN, VCO, VOICE_OUT};

/// A plucky saw bass.
const BASS: &[(&str, &str, f32)] = &[
    (NOTE_IN, "voices", 4.0),
    (FILTER, "cutoff", 420.0),
    (FILTER, "res", 0.35),
    (FILTER, "amount", 0.55),
    (ADSR, "d", 0.25),
    (ADSR, "s", 0.35),
    (VOICE_OUT, "level", -4.0),
];
/// A short sine blip.
const BLIP: &[(&str, &str, f32)] =
    &[(VCO, "wave", 0.0), (FILTER, "cutoff", 3000.0), (ADSR, "d", 0.09), (ADSR, "s", 0.0), (ADSR, "r", 0.06)];
/// Soft saw pads.
const PADS: &[(&str, &str, f32)] = &[
    (FILTER, "cutoff", 1400.0),
    (FILTER, "amount", 0.15),
    (ADSR, "a", 0.35),
    (ADSR, "s", 0.8),
    (ADSR, "r", 0.9),
    (VOICE_OUT, "level", -12.0),
];

/// Builds the demo project.
pub fn demo() -> Demo {
    let mut doc = Document::new(42);
    let mut b = B { ops: Vec::new() };
    let rhythm = b.group("rhythm", 0, 0);
    let (drums, dseq) = b.track_with("drums", 0, Some(rhythm), Ins::Drums, sat_modules::DRUM_SEQ, "drum seq");
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

    let (bass, bseq) = b.track("bass", 1, Some(rhythm), Ins::Patch(BASS));
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
    // a second source on the bass grid, not wired to a clip yet
    let (_, op) = {
        let grid = b.grid_of(bseq);
        let id = ModuleId::new();
        let mut params = sat_modules::default_params(sat_modules::EUCLID);
        params.insert("hits".into(), 5.0);
        params.insert("note".into(), 45.0);
        let name = "euclid".to_string();
        (id, Op::AddModule { id, grid, kind: sat_modules::EUCLID.into(), name, params, pos: (220.0, 70.0) })
    };
    b.ops.push(op);
    let bass_c = vec![n(0, 2, 45, 0.8), n(2, 2, 52, 0.6), n(6, 2, 48, 0.7), n(10, 4, 43, 0.8)];
    b.clip(bass, bseq, "bass c", 1, 1, bass_c, 12, 4);

    let (perc, eseq) = b.track_with("perc", 2, Some(rhythm), Ins::Patch(BLIP), sat_modules::EUCLID, "euclid 1");
    for (k, v) in [("hits", 7.0), ("rot", 2.0), ("acc", 2.0), ("note", 72.0), ("gate", 0.3), ("vel", 0.55)] {
        b.ops.push(Op::SetValue { target: ParamTarget::Module { module: eseq, param: k.into() }, value: v });
    }
    let eclip = b.clip(perc, eseq, "euclid", 2, 1, Vec::new(), 4, 8);
    // hand edits over the generator: one hit taken out, one added
    b.ops.push(Op::SetOverrides {
        module: eseq,
        clip: eclip,
        overrides: Overrides {
            added: Vector::unit(Note { len: Ticks(72), ..n(3, 1, 72, 0.55) }),
            removed: [(Ticks::sixteenths(9), 72u8)].into_iter().collect(),
        },
    });

    let tops = b.group("tops", 6, 1);
    let (keys, kseq) = b.track("keys", 6, Some(tops), Ins::Patch(PADS));
    let pads = |root: u8| -> Vec<Note> {
        (0..4).flat_map(|c| [0u8, 3, 7, 10].map(|i| n(c * 16, 15, root + i + [0, 5, 3, 7][c as usize], 0.5))).collect()
    };
    b.clip(keys, kseq, "pads", 6, 4, pads(57), 0, 8);
    b.clip(keys, kseq, "pads 2", 6, 4, pads(60), 8, 8);

    let vocals = b.group("vocals", 4, 2);
    let (vox, vseq) = b.track("vox", 4, Some(vocals), Ins::Test("audio in", 0.0));
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
        (perc, -12.0),
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
    // fx: glue on the drums, grit on the bass, a hall and an echo on the
    // returns, glue and a limiter on the master
    use sat_modules::fx::{COMP, DELAY, DRIVE, EQ, LIMITER, REVERB};
    b.fx(
        &doc,
        drums,
        &[(COMP, &[("thresh", -20.0), ("ratio", 3.0), ("attack", 0.005), ("release", 0.1), ("makeup", 3.0)])],
    );
    b.fx(
        &doc,
        bass,
        &[
            (DRIVE, &[("drive", 9.0), ("tone", 6000.0), ("level", -3.0), ("mix", 0.6)]),
            (EQ, &[("low", 2.0), ("high", -2.0)]),
        ],
    );
    b.fx(&doc, keys, &[(EQ, &[("low", -4.0), ("high", 2.0)])]);
    b.fx(&doc, hall, &[(REVERB, &[("decay", 2.8), ("lowcut", 250.0), ("mix", 1.0)])]);
    b.fx(&doc, echo, &[(DELAY, &[("time", 0.363), ("feedback", 0.4), ("tone", 4000.0), ("mix", 1.0)])]);
    b.fx(
        &doc,
        doc.master,
        &[(COMP, &[("thresh", -14.0), ("ratio", 2.0), ("release", 0.2), ("makeup", 2.0)]), (LIMITER, &[])],
    );
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
