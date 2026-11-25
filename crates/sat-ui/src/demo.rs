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
    /// Counts the fixed ids handed out, so clips and sources keep their ids
    /// (and generators their seeds) from one run to the next.
    ids: u128,
}

/// What plays a demo track.
enum Ins {
    /// The default voice patch with (module kind, parameter, value) tweaks.
    Patch(&'static [(&'static str, &'static str, f32)]),
    /// The drum synth.
    Drums,
    /// A sampler playing a pool sound, with (parameter, value) tweaks.
    Sampler(AudioId, &'static [(&'static str, f32)]),
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

    /// The instrument grid of a track added by these ops.
    fn ins_grid(&self, track: TrackId) -> GridId {
        self.ops
            .iter()
            .find_map(|op| match op {
                Op::AddTrack { id, grids, .. } if *id == track => grids.instrument,
                _ => None,
            })
            .expect("track added")
    }

    /// The first module of `kind` added to `grid`, with its parameters.
    fn module_on(&self, grid: GridId, kind: &str) -> (ModuleId, OrdMap<String, f32>) {
        self.ops
            .iter()
            .find_map(|op| match op {
                Op::AddModule { id, grid: g, kind: k, params, .. } if *g == grid && k == kind => {
                    Some((*id, params.clone()))
                }
                _ => None,
            })
            .expect("module added")
    }

    /// Assigns macro `index` of `grid`: `name`, driving (module kind,
    /// parameter, knob range) on that grid, set where the first one sits now.
    fn macro_slot(&mut self, grid: GridId, index: u8, name: &str, binds: &[(&str, &str, f32, f32)]) {
        let mut value = 0.0;
        let mut out = Vector::new();
        for (k, (kind, param, lo, hi)) in binds.iter().enumerate() {
            let (module, params) = self.module_on(grid, kind);
            if k == 0 {
                let spec = sat_modules::spec(kind, param).expect("known parameter");
                let pos = spec.pos(params.get(*param).copied().unwrap_or(spec.default)) as f32;
                value = ((pos - lo) / (hi - lo)).clamp(0.0, 1.0);
            }
            out.push_back(MacroBind { module, param: param.to_string(), lo: *lo, hi: *hi });
        }
        self.ops.push(Op::SetMacroSlot { grid, index, slot: Some(MacroSlot { name: name.into(), binds: out }) });
        self.ops.push(Op::SetValue { target: ParamTarget::Macro { grid, index }, value });
    }

    /// A fixed id, the same every run.
    fn fixed_id(&mut self) -> u128 {
        self.ids += 1;
        0xde70_0000_0000_0000_0000_0000_0000_0000 | self.ids
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
        let seq = ModuleId::from_u128(self.fixed_id());
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
            Ins::Sampler(sound, tweaks) => {
                let kind = sat_modules::instrument::SAMPLER;
                let mut params = sat_modules::default_params(kind);
                for (k, v) in tweaks {
                    params.insert(k.to_string(), *v);
                }
                let module = ModuleId::new();
                self.ops.push(Op::AddModule {
                    id: module,
                    grid: ins,
                    kind: kind.into(),
                    name: sat_modules::label_of(kind).into(),
                    params,
                    pos: (0.0, 0.0),
                });
                self.ops.push(Op::SetModuleAudio { module, audio: Some(sound) });
            }
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
        let id = ClipId::from_u128(self.fixed_id());
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
        ]);
        // no bars to play: a clip for the session only
        if play_bars > 0 {
            self.ops.push(Op::AddPlacement {
                id: PlacementId::new(),
                clip: id,
                start: bars(start_bar),
                offset: Ticks::ZERO,
                length: bars(play_bars),
            });
        }
        id
    }

    /// An audio clip of `len_bars`, placed whole at `start_bar`.
    fn audio_clip(&mut self, track: TrackId, name: &str, color: u8, start_bar: i64, len_bars: i64, audio: AudioClip) {
        let id = ClipId::from_u128(self.fixed_id());
        self.ops.extend([
            Op::AddClip {
                id,
                track,
                name: name.into(),
                color: Color(color),
                length: bars(len_bars),
                looped: false,
                seed: 1,
            },
            Op::SetClipAudio { clip: id, audio: Some(audio) },
            Op::AddPlacement {
                id: PlacementId::new(),
                clip: id,
                start: bars(start_bar),
                offset: Ticks::ZERO,
                length: bars(len_bars),
            },
        ]);
    }
}

/// The demo project and the clip that starts selected.
pub struct Demo {
    /// The document.
    pub doc: Document,
    /// The clip shown in the bottom panel.
    pub selected: ClipId,
    /// The sounds its samplers play (generated, so the demo needs no files).
    pub sounds: sat_disk::Pool,
}

/// A sung "ah" on A4, 1.6 s at 48 kHz: additive harmonics shaped by three
/// vowel formants, with vibrato fading in so a loop over its second half
/// sounds alive.
pub fn vowel() -> sat_disk::Sample {
    const RATE: f32 = 48_000.0;
    const FORMANTS: [(f32, f32, f32); 3] = [(730.0, 90.0, 1.0), (1090.0, 110.0, 0.5), (2440.0, 170.0, 0.22)];
    let frames = (1.6 * RATE) as usize;
    let mut phase = 0.0f64;
    let mut out = Vec::with_capacity(frames);
    for i in 0..frames {
        let t = i as f32 / RATE;
        let depth = ((t - 0.25) / 0.5).clamp(0.0, 1.0) * 0.25;
        let f0 = 440.0 * 2f32.powf(depth * (t * 5.5 * std::f32::consts::TAU).sin() / 12.0);
        phase += f0 as f64 / RATE as f64;
        let mut s = 0.0;
        for k in 1..=30 {
            let f = k as f32 * f0;
            if f > 9_000.0 {
                break;
            }
            let shape: f32 = FORMANTS.iter().map(|(c, bw, g)| g / (1.0 + ((f - c) / (bw * 0.5)).powi(2))).sum();
            s += (shape + 0.02) / (k as f32).sqrt() * ((phase * k as f64).fract() as f32 * std::f32::consts::TAU).sin();
        }
        let env = (t / 0.04).min(1.0) * ((1.6 - t) / 0.05).clamp(0.0, 1.0);
        out.push(s * env);
    }
    let peak = out.iter().fold(0.0f32, |a, x| a.max(x.abs())).max(1e-6);
    let l: Vec<f32> = out.iter().map(|x| x / peak * 0.7).collect();
    sat_disk::Sample { rate: RATE, r: l.clone(), l }
}

/// The demo's sung phrase: (start, length) in sixteenths at 124 bpm, and pitch.
pub const PHRASE: [(f32, f32, f32); 8] = [
    (0.0, 6.0, 69.0),
    (8.0, 4.0, 72.0),
    (12.0, 4.0, 71.0),
    (16.0, 12.0, 67.0),
    (32.0, 6.0, 69.0),
    (40.0, 4.0, 74.0),
    (44.0, 4.0, 72.0),
    (48.0, 14.0, 71.0),
];

/// One take of [`PHRASE`] sung on "ah" (24 kHz, mono): a little early or
/// late, a little louder or softer on each note, and note `flat` sung most
/// of a semitone flat, so takes are worth comping.
pub fn sung(take: u32, flat: usize) -> sat_disk::Sample {
    const RATE: f32 = 24_000.0;
    const SIXTEENTH: f32 = 60.0 / 124.0 / 4.0;
    const FORMANTS: [(f32, f32, f32); 3] = [(730.0, 90.0, 1.0), (1090.0, 110.0, 0.5), (2440.0, 170.0, 0.22)];
    let mut seed = 0x9e37_79b9u32.wrapping_mul(take + 1);
    let mut rnd = || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed as f32 / u32::MAX as f32 * 2.0 - 1.0
    };
    // each note: start, end (s), pitch, level
    let notes: Vec<(f32, f32, f32, f32)> = PHRASE
        .iter()
        .enumerate()
        .map(|(i, (s, l, p))| {
            let start = s * SIXTEENTH + 0.02 + rnd() * 0.02;
            let pitch = p - if i == flat { 0.7 } else { 0.0 };
            (start, start + l * SIXTEENTH - 0.03, pitch, 1.0 + rnd() * 0.15)
        })
        .collect();
    let frames = (64.0 * SIXTEENTH * RATE) as usize;
    let vib_rate = 5.2 + rnd() * 0.4;
    let (mut phase, mut f0) = (0.0f64, 440.0f32);
    let mut out = Vec::with_capacity(frames);
    for i in 0..frames {
        let t = i as f32 / RATE;
        // the note sounding (or the last one, for its release)
        let k = notes.iter().rposition(|n| n.0 <= t).unwrap_or(0);
        let (s, e, pitch, level) = notes[k];
        let env = ((t - s) / 0.04).clamp(0.0, 1.0) * ((e + 0.08 - t) / 0.08).clamp(0.0, 1.0) * level;
        let depth = ((t - s - 0.2) / 0.4).clamp(0.0, 1.0) * 0.3;
        let target = 440.0 * 2f32.powf((pitch - 69.0 + depth * (t * vib_rate * std::f32::consts::TAU).sin()) / 12.0);
        // a quick glide into each note
        f0 += (target - f0) * 0.004;
        phase += f0 as f64 / RATE as f64;
        let mut v = 0.0;
        if env > 0.0 {
            for h in 1..=14 {
                let f = h as f32 * f0;
                if f > 5_000.0 {
                    break;
                }
                let shape: f32 = FORMANTS.iter().map(|(c, bw, g)| g / (1.0 + ((f - c) / (bw * 0.5)).powi(2))).sum();
                v += (shape + 0.02) / (h as f32).sqrt()
                    * ((phase * h as f64).fract() as f32 * std::f32::consts::TAU).sin();
            }
        }
        out.push(v * env);
    }
    let peak = out.iter().fold(0.0f32, |a, x| a.max(x.abs())).max(1e-6);
    let l: Vec<f32> = out.iter().map(|x| x / peak * 0.6).collect();
    sat_disk::Sample { rate: RATE, r: l.clone(), l }
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
/// A short square pluck.
const PLUCK: &[(&str, &str, f32)] = &[
    (VCO, "wave", 2.0),
    (FILTER, "cutoff", 1800.0),
    (FILTER, "amount", 0.4),
    (ADSR, "d", 0.18),
    (ADSR, "s", 0.0),
    (ADSR, "r", 0.12),
    (VOICE_OUT, "level", -14.0),
];
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
    let mut b = B { ops: Vec::new(), ids: 0 };
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
    let d_intro = b.clip(drums, dseq, "intro", 0, 1, kick(1), 0, 4);
    let d_groove = b.clip(drums, dseq, "groove", 0, 2, kick(2), 4, 8);
    let fill: Vec<Note> = (0..16).map(|s| n(s, 1, 38 + (s % 3) as u8, 0.4 + 0.03 * s as f32)).collect();
    let d_fill = b.clip(drums, dseq, "fill", 0, 1, fill, 12, 4);

    let (bass, bseq) = b.track("bass", 1, Some(rhythm), Ins::Patch(BASS));
    let bass_a = vec![n(0, 3, 45, 0.9), n(4, 2, 48, 0.7), n(8, 2, 45, 0.8), n(11, 3, 43, 0.7), n(14, 2, 40, 0.6)];
    let b_a = b.clip(bass, bseq, "bass a", 1, 1, bass_a, 0, 4);
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
        let id = ModuleId::from_u128(b.fixed_id());
        let mut params = sat_modules::default_params(sat_modules::EUCLID);
        params.insert("hits".into(), 5.0);
        params.insert("note".into(), 45.0);
        let name = "euclid".to_string();
        (id, Op::AddModule { id, grid, kind: sat_modules::EUCLID.into(), name, params, pos: (220.0, 70.0) })
    };
    b.ops.push(op);
    let bass_c = vec![n(0, 2, 45, 0.8), n(2, 2, 52, 0.6), n(6, 2, 48, 0.7), n(10, 4, 43, 0.8)];
    let b_c = b.clip(bass, bseq, "bass c", 1, 1, bass_c, 12, 4);

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
    let k_a = b.clip(keys, kseq, "pads", 6, 4, pads(57), 0, 8);
    let k_b = b.clip(keys, kseq, "pads 2", 6, 4, pads(60), 8, 8);

    // a seeded turing phrase over the second half, new each time round
    let (arp, tseq) = b.track_with("turing", 5, Some(tops), Ins::Patch(PLUCK), sat_modules::TURING, "turing 1");
    for (k, v) in
        [("steps", 16.0), ("change", 0.0), ("low", 57.0), ("range", 19.0), ("dens", 0.6), ("vel", 0.5), ("reseed", 1.0)]
    {
        b.ops.push(Op::SetValue { target: ParamTarget::Module { module: tseq, param: k.into() }, value: v });
    }
    let t_a = b.clip(arp, tseq, "turing", 5, 1, Vec::new(), 8, 8);

    let vocals = b.group("vocals", 4, 2);
    let mut sounds = sat_disk::Pool::new();
    let ah = sounds.add_with_id(AudioId::from_u128(0xa4), "ah", vowel());
    b.ops.push(Op::AddAudio { file: ah.clone() });
    const VOX: &[(&str, f32)] = &[
        ("root", 69.0),
        ("mode", 2.0),
        ("loop", 0.5),
        ("attack", 0.03),
        ("release", 0.25),
        ("level", -7.0),
        ("vel", 0.4),
    ];
    let (vox, vseq) = b.track("vox", 4, Some(vocals), Ins::Sampler(ah.id, VOX));
    let line = vec![n(0, 6, 69, 0.7), n(8, 4, 72, 0.7), n(12, 4, 71, 0.6), n(16, 12, 67, 0.7)];
    // the sampler's clips play in the session; the arrangement has the sung takes
    let v_a = b.clip(vox, vseq, "ah", 4, 2, line.clone(), 4, 0);
    let v_b = b.clip(vox, vseq, "ah 2", 4, 2, line, 12, 0);
    let takes: Vec<Take> = [1usize, 3, 7]
        .iter()
        .enumerate()
        .map(|(i, flat)| {
            let name = format!("take {}", i + 1);
            let f =
                sounds.add_with_id(AudioId::from_u128(0xb0 + i as u128), &format!("vox {name}"), sung(i as u32, *flat));
            b.ops.push(Op::AddAudio { file: f.clone() });
            Take { audio: f.id, name, offset: 0 }
        })
        .collect();
    // each take flubs a different note: the comp keeps the good ones
    let mut verse = AudioClip::single(takes[2].clone());
    verse.takes = takes.iter().cloned().collect();
    verse.comp = Vector::unit(CompSegment { start: Ticks::ZERO, take: 2 });
    let verse = verse.comped(Ticks::sixteenths(30), Ticks::sixteenths(47), 0, bars(4)).comped(
        Ticks::sixteenths(47),
        bars(4),
        1,
        bars(4),
    );
    let verse = AudioClip { fade_in: Ticks::sixteenths(1), fade_out: Ticks::sixteenths(2), ..verse };
    b.audio_clip(vox, "vox", 4, 4, 4, verse.clone());
    let drop = AudioClip { comp: Vector::unit(CompSegment { start: Ticks::ZERO, take: 0 }), ..verse };
    let drop = drop.comped(Ticks::sixteenths(16), Ticks::sixteenths(30), 2, bars(4));
    b.audio_clip(vox, "vox 2", 4, 12, 4, drop);

    // the same clips as session scenes
    let scenes: [(&str, Vec<(TrackId, ClipId)>); 3] = [
        ("intro", vec![(drums, d_intro), (bass, b_a), (keys, k_a)]),
        ("verse", vec![(drums, d_groove), (bass, selected), (perc, eclip), (keys, k_a), (vox, v_a)]),
        ("break", vec![(drums, d_fill), (bass, b_c), (keys, k_b), (arp, t_a), (vox, v_b)]),
    ];
    for (i, (name, slots)) in scenes.into_iter().enumerate() {
        let id = SceneId::new();
        b.ops.push(Op::AddScene { id, name: name.into(), index: i });
        for (track, clip) in slots {
            b.ops.push(Op::SetSlot { scene: id, track, clip: Some(clip) });
        }
    }

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
    // macros: the bass's tone and pluck (one macro, two parameters), the
    // pads' envelope, and the spare euclid's density
    let bg = b.ins_grid(bass);
    b.macro_slot(bg, 0, "cutoff", &[(FILTER, "cutoff", 0.25, 0.8)]);
    b.macro_slot(bg, 1, "res", &[(FILTER, "res", 0.0, 0.9)]);
    b.macro_slot(bg, 2, "pluck", &[(ADSR, "d", 0.75, 0.45), (FILTER, "amount", 0.65, 0.95)]);
    let kg = b.ins_grid(keys);
    b.macro_slot(kg, 0, "attack", &[(ADSR, "a", 0.0, 0.8)]);
    b.macro_slot(kg, 1, "release", &[(ADSR, "r", 0.2, 0.9)]);
    let bsrc = b.grid_of(bseq);
    b.macro_slot(bsrc, 0, "density", &[(sat_modules::EUCLID, "hits", 0.02, 0.2)]);
    let tempo = doc.tempo_lane;
    b.ops.push(Op::SetLanePoints {
        id: tempo,
        points: Vector::unit(Breakpoint { tick: Ticks::ZERO, value: 124.0, curve: Curve::Hold }),
    });
    b.ops.push(Op::SetLoop { range: LoopRange { start: bars(4), end: bars(12), enabled: true } });
    for op in b.ops {
        apply(&mut doc, op).expect("demo ops are valid");
    }
    Demo { doc, selected, sounds }
}
