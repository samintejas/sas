//! Note processors: source modules with a notes input that reshape the notes
//! of whatever feeds them before they reach a clip (or another processor).
//!
//! A clip fed through a chain of processors plays the chain's output, while
//! its editor edits the notes at the head of the chain, the way a clip runs
//! through note effects. Processors work on one cycle of the clip at a time,
//! so they follow a reseeding generator upstream.

use sat_doc::sat_time::{PPQ, Ticks};
use sat_doc::{Module, Note};
use sat_dsp::scale;

use crate::spec::{ParamSpec, RATE_LABELS, Unit, p, rate_ticks};
use crate::{PortKind, PortSpec};

/// Shifts pitch by semitones and octaves.
pub const TRANSPOSE: &str = "transpose";
/// Snaps notes to a scale, and moves them along it.
pub const SCALE: &str = "scale";
/// Turns each note into a chord.
pub const CHORD: &str = "chord";
/// Plays held notes one at a time.
pub const ARP: &str = "arp";
/// Lets each note through with a probability, new each time round.
pub const CHANCE: &str = "chance";
/// Scales, offsets and randomises velocity.
pub const VELOCITY: &str = "velocity";
/// Adds two note streams together.
pub const MERGE_NOTES: &str = "merge_notes";

/// Note processors that can be added to a source grid: kind and label.
pub const MODULES: [(&str, &str); 7] = [
    (TRANSPOSE, "transpose"),
    (SCALE, "scale"),
    (CHORD, "chord"),
    (ARP, "arp"),
    (CHANCE, "chance"),
    (VELOCITY, "velocity"),
    (MERGE_NOTES, "merge"),
];

/// Is `kind` a note processor?
pub fn is_processor(kind: &str) -> bool {
    MODULES.iter().any(|(k, _)| *k == kind)
}

const fn notes_in(key: &'static str) -> PortSpec {
    PortSpec { key, kind: PortKind::Notes, input: true }
}

/// The ports of a processor.
pub fn ports(kind: &str) -> &'static [PortSpec] {
    const ONE: [PortSpec; 2] = [notes_in("in"), crate::NOTES_OUT];
    const TWO: [PortSpec; 3] = [notes_in("a"), notes_in("b"), crate::NOTES_OUT];
    match kind {
        MERGE_NOTES => &TWO,
        k if is_processor(k) => &ONE,
        _ => &[],
    }
}

/// Chord shapes, in choice order.
pub const CHORDS: &[&str] =
    &["octave", "fifth", "major", "minor", "sus2", "sus4", "maj7", "min7", "dom7", "dim", "triad"];
const SHAPES: [&[i32]; 10] = [
    &[0, 12],
    &[0, 7],
    &[0, 4, 7],
    &[0, 3, 7],
    &[0, 2, 7],
    &[0, 5, 7],
    &[0, 4, 7, 11],
    &[0, 3, 7, 10],
    &[0, 4, 7, 10],
    &[0, 3, 6],
];

/// Arp orders, in choice order.
pub const ARP_MODES: &[&str] = &["up", "down", "up-down", "random", "as played"];

/// Strum offsets between chord notes, in choice order.
pub const STRUMS: &[&str] = &["off", "1/64", "1/32", "1/16"];

/// The parameters of a processor, in knob order.
pub fn params(kind: &str) -> &'static [ParamSpec] {
    const TRANSPOSE_P: [ParamSpec; 2] =
        [p("semis", "semis", -24.0, 24.0, 0.0, Unit::Semis), p("octave", "octave", -3.0, 3.0, 0.0, Unit::Int)];
    const SCALE_P: [ParamSpec; 3] = [
        p("root", "root", 0.0, 11.0, 0.0, Unit::Choice(scale::ROOTS)),
        p("scale", "scale", 0.0, 13.0, 1.0, Unit::Choice(scale::NAMES)),
        p("shift", "degrees", -14.0, 14.0, 0.0, Unit::Int),
    ];
    const CHORD_P: [ParamSpec; 5] = [
        p("chord", "chord", 0.0, 10.0, 2.0, Unit::Choice(CHORDS)),
        p("inversion", "inversion", 0.0, 3.0, 0.0, Unit::Int),
        p("strum", "strum", 0.0, 3.0, 0.0, Unit::Choice(STRUMS)),
        p("root", "key root", 0.0, 11.0, 0.0, Unit::Choice(scale::ROOTS)),
        p("scale", "key", 0.0, 13.0, 1.0, Unit::Choice(scale::NAMES)),
    ];
    const ARP_P: [ParamSpec; 4] = [
        p("rate", "rate", 0.0, 3.0, 1.0, Unit::Choice(&RATE_LABELS)),
        p("mode", "mode", 0.0, 4.0, 0.0, Unit::Choice(ARP_MODES)),
        p("octaves", "octaves", 1.0, 4.0, 1.0, Unit::Int),
        p("gate", "gate", 0.05, 1.0, 0.5, Unit::Percent),
    ];
    const CHANCE_P: [ParamSpec; 1] = [p("chance", "chance", 0.0, 1.0, 0.5, Unit::Percent)];
    const VELOCITY_P: [ParamSpec; 3] = [
        p("scale", "scale", 0.0, 2.0, 1.0, Unit::Percent),
        p("offset", "offset", -1.0, 1.0, 0.0, Unit::Bipolar),
        p("random", "random", 0.0, 1.0, 0.0, Unit::Percent),
    ];
    match kind {
        TRANSPOSE => &TRANSPOSE_P,
        SCALE => &SCALE_P,
        CHORD => &CHORD_P,
        ARP => &ARP_P,
        CHANCE => &CHANCE_P,
        VELOCITY => &VELOCITY_P,
        _ => &[],
    }
}

fn get(m: &Module, key: &str) -> f32 {
    let s = params(&m.kind).iter().find(|s| s.key == key);
    s.map_or(0.0, |s| s.clamp(m.params.get(key).copied().unwrap_or(s.default)))
}

fn pitch(p: i32) -> u8 {
    p.clamp(0, 127) as u8
}

/// Sorts by start then pitch and keeps one note per (start, pitch).
fn tidy(mut v: Vec<Note>) -> Vec<Note> {
    v.sort_by(|a, b| a.start.cmp(&b.start).then(a.pitch.cmp(&b.pitch)));
    v.dedup_by(|b, a| a.start == b.start && a.pitch == b.pitch);
    v
}

/// A random number in 0–1 from a seed and a counter.
fn unit(seed: u64, i: u64) -> f32 {
    (crate::turing::mix(seed ^ crate::turing::mix(i)) >> 40) as f32 / (1u64 << 24) as f32
}

/// The notes processor `m` (with macros resolved) makes from `inputs` (one
/// list per input port, in port order) for a clip of length `len`. `seed`
/// is the clip's seed for this cycle.
pub fn process(m: &Module, inputs: &[Vec<Note>], len: Ticks, seed: u64) -> Vec<Note> {
    let input = inputs.first().cloned().unwrap_or_default();
    match m.kind.as_str() {
        TRANSPOSE => {
            let d = get(m, "semis") as i32 + 12 * get(m, "octave") as i32;
            tidy(input.into_iter().map(|n| Note { pitch: pitch(n.pitch as i32 + d), ..n }).collect())
        }
        SCALE => {
            let (root, sc, shift) = (get(m, "root") as i32, get(m, "scale") as usize, get(m, "shift") as i32);
            tidy(
                input
                    .into_iter()
                    .map(|n| Note { pitch: pitch(scale::step(n.pitch as i32, shift, root, sc)), ..n })
                    .collect(),
            )
        }
        CHORD => {
            let shape = get(m, "chord") as usize;
            let strum = [0, PPQ / 16, PPQ / 8, PPQ / 4][get(m, "strum") as usize];
            let (root, sc) = (get(m, "root") as i32, get(m, "scale") as usize);
            let inv = get(m, "inversion") as usize;
            let mut out = Vec::new();
            for n in input {
                let mut iv: Vec<i32> = if shape < SHAPES.len() {
                    SHAPES[shape].to_vec()
                } else {
                    // a triad built from the key: root, third and fifth degrees
                    [0, 2, 4].iter().map(|s| scale::step(n.pitch as i32, *s, root, sc) - n.pitch as i32).collect()
                };
                // each inversion lifts the lowest note an octave
                for _ in 0..inv.min(iv.len() - 1) {
                    iv[0] += 12;
                    iv.sort();
                }
                // an inverted chord drops an octave, so it stays around the played note
                let drop = if inv > 0 { 12 } else { 0 };
                for (k, d) in iv.iter().enumerate() {
                    let start = Ticks((n.start.0 + strum * k as i64).min(len.0 - 1).max(0));
                    let len_k = Ticks((n.len.0 - (start.0 - n.start.0)).max(1));
                    let p = n.pitch as i32 + d - drop;
                    out.push(Note { start, len: len_k, pitch: pitch(p), ..n });
                }
            }
            tidy(out)
        }
        ARP => arp(m, &input, len, seed),
        CHANCE => {
            let c = get(m, "chance");
            input.into_iter().map(|n| Note { chance: n.chance * c, ..n }).collect()
        }
        VELOCITY => {
            let (k, off, rnd) = (get(m, "scale"), get(m, "offset"), get(m, "random"));
            input
                .into_iter()
                .enumerate()
                .map(|(i, n)| {
                    let jitter = (unit(seed, i as u64) * 2.0 - 1.0) * rnd;
                    Note { velocity: (n.velocity * k + off + jitter).clamp(0.01, 1.0), ..n }
                })
                .collect()
        }
        MERGE_NOTES => tidy(inputs.iter().flatten().copied().collect()),
        _ => input,
    }
}

fn arp(m: &Module, input: &[Note], len: Ticks, seed: u64) -> Vec<Note> {
    let rate = rate_ticks(get(m, "rate"));
    let mode = get(m, "mode") as usize;
    let octaves = get(m, "octaves") as i32;
    let note_len = Ticks(((rate as f32 * get(m, "gate")).round() as i64).max(1));
    let mut out = Vec::new();
    let mut i = 0usize;
    let mut t = 0;
    while t < len.0 {
        let held: Vec<&Note> = input.iter().filter(|n| n.start.0 <= t && t < n.start.0 + n.len.0).collect();
        if held.is_empty() {
            i = 0;
            t += rate;
            continue;
        }
        // as played: by start; otherwise low to high, then repeated up the octaves
        let mut base: Vec<&Note> = held.clone();
        if mode != 4 {
            base.sort_by_key(|n| n.pitch);
        }
        let seq: Vec<(i32, f32)> =
            (0..octaves).flat_map(|o| base.iter().map(move |n| (n.pitch as i32 + 12 * o, n.velocity))).collect();
        let n = seq.len();
        let k = match mode {
            1 => n - 1 - i % n,
            2 if n > 1 => {
                let period = 2 * n - 2;
                let j = i % period;
                if j < n { j } else { period - j }
            }
            3 => (unit(seed, t as u64) * n as f32) as usize % n,
            _ => i % n,
        };
        let (p, vel) = seq[k];
        out.push(Note { start: Ticks(t), len: note_len, pitch: pitch(p), velocity: vel, chance: 1.0 });
        i += 1;
        t += rate;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use sat_doc::ModuleId;

    fn module(kind: &str, set: &[(&str, f32)]) -> Module {
        let mut params = crate::default_params(kind);
        for (k, v) in set {
            params.insert(k.to_string(), *v);
        }
        Module {
            id: ModuleId::new(),
            grid: sat_doc::GridId::new(),
            kind: kind.into(),
            name: kind.into(),
            params,
            pos: (0.0, 0.0),
            audio: None,
            plugin: None,
        }
    }

    fn n(start16: i64, len16: i64, pitch: u8) -> Note {
        Note { start: Ticks::sixteenths(start16), len: Ticks::sixteenths(len16), pitch, velocity: 0.8, chance: 1.0 }
    }

    fn pitches(v: &[Note]) -> Vec<u8> {
        v.iter().map(|n| n.pitch).collect()
    }

    const BAR: Ticks = Ticks(PPQ * 4);

    #[test]
    fn transpose_and_scale_move_pitches() {
        let t = module(TRANSPOSE, &[("semis", 2.0), ("octave", -1.0)]);
        assert_eq!(pitches(&process(&t, &[vec![n(0, 1, 60)]], BAR, 0)), vec![50]);
        // c# and f# into c major, then up two degrees
        let s = module(SCALE, &[("shift", 2.0)]);
        assert_eq!(pitches(&process(&s, &[vec![n(0, 1, 61), n(1, 1, 66)]], BAR, 0)), vec![64, 69]);
        // pitches stay in range
        let t = module(TRANSPOSE, &[("octave", 3.0)]);
        assert_eq!(pitches(&process(&t, &[vec![n(0, 1, 120)]], BAR, 0)), vec![127]);
    }

    #[test]
    fn chords_stack_invert_and_strum() {
        let c = module(CHORD, &[]);
        assert_eq!(pitches(&process(&c, &[vec![n(0, 4, 60)]], BAR, 0)), vec![60, 64, 67]);
        let c = module(CHORD, &[("inversion", 1.0)]);
        assert_eq!(pitches(&process(&c, &[vec![n(0, 4, 60)]], BAR, 0)), vec![52, 55, 60]);
        // a key triad on d in c major is d minor
        let c = module(CHORD, &[("chord", 10.0)]);
        assert_eq!(pitches(&process(&c, &[vec![n(0, 4, 62)]], BAR, 0)), vec![62, 65, 69]);
        let c = module(CHORD, &[("strum", 3.0)]);
        let v = process(&c, &[vec![n(0, 4, 60)]], BAR, 0);
        assert_eq!(v.iter().map(|n| n.start.0).collect::<Vec<_>>(), vec![0, 240, 480]);
        assert_eq!(v[2].len, Ticks(480), "strummed notes end together");
    }

    #[test]
    fn the_arp_steps_through_held_notes() {
        let chord = [vec![n(0, 8, 64), n(0, 8, 60), n(0, 8, 67)]];
        let a = module(ARP, &[]);
        let v = process(&a, &chord, BAR, 0);
        assert_eq!(pitches(&v), vec![60, 64, 67, 60, 64, 67, 60, 64], "up, 1/16, only while held");
        assert_eq!(v[1].start, Ticks::sixteenths(1));
        let a = module(ARP, &[("mode", 2.0), ("octaves", 2.0)]);
        assert_eq!(pitches(&process(&a, &chord, BAR, 0)), vec![60, 64, 67, 72, 76, 79, 76, 72]);
        let a = module(ARP, &[("mode", 1.0), ("rate", 2.0)]);
        assert_eq!(pitches(&process(&a, &chord, BAR, 0)), vec![67, 64, 60, 67]);
        let a = module(ARP, &[("mode", 3.0)]);
        let r1 = process(&a, &chord, BAR, 1);
        assert_eq!(r1, process(&a, &chord, BAR, 1), "seeded");
        assert!(r1.iter().all(|n| [60, 64, 67].contains(&n.pitch)));
    }

    #[test]
    fn chance_velocity_and_merge() {
        let c = module(CHANCE, &[("chance", 0.5)]);
        let v = process(&c, &[vec![Note { chance: 0.5, ..n(0, 1, 60) }]], BAR, 0);
        assert_eq!(v[0].chance, 0.25);
        let vel = module(VELOCITY, &[("scale", 0.5), ("offset", 0.1)]);
        assert!((process(&vel, &[vec![n(0, 1, 60)]], BAR, 0)[0].velocity - 0.5).abs() < 1e-6);
        let vel = module(VELOCITY, &[("random", 0.3)]);
        let v = process(&vel, &[(0..16).map(|i| n(i, 1, 60)).collect()], BAR, 7);
        assert!(v.iter().any(|n| n.velocity != 0.8) && v.iter().all(|n| (n.velocity - 0.8).abs() <= 0.3 + 1e-6));
        let m = module(MERGE_NOTES, &[]);
        let v = process(&m, &[vec![n(0, 1, 60), n(2, 1, 62)], vec![n(0, 1, 60), n(1, 1, 64)]], BAR, 0);
        assert_eq!(pitches(&v), vec![60, 64, 62]);
    }
}
