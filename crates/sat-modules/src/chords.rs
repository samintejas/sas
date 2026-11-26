//! The chord builder: a source that plays a chord progression in a key.
//!
//! Each clip holds its own progression (the pattern's chord slots), stored
//! relative to the key: a slot names its root in semitones above the key's
//! root and its quality, so changing the key root transposes every clip.
//! The module's knobs decide how the chords are played: the key (the
//! project's or its own), octave, voicing, voice leading, a bass note, a
//! rhythm (held, pulsed, syncopated or arpeggiated), strum, gate and
//! velocity. The editor offers the key's diatonic triads and sevenths,
//! chords borrowed from the parallel key and common progressions, and plays
//! any of them live from a single key.

use im::Vector;
use sat_doc::sat_time::{PPQ, Ticks};
use sat_doc::{ChordSlot, Document, Module, Note};
use sat_dsp::scale;

use crate::spec::{ParamSpec, Unit, p};

/// Module kind of the chord builder.
pub const CHORD_BUILDER: &str = "chord_builder";

/// The scales a chord builder harmonises (seven-note scales): label and
/// index into [`sat_dsp::scale::NAMES`].
pub const SCALES: [(&str, usize); 9] = [
    ("major", 1),
    ("minor", 2),
    ("dorian", 3),
    ("phrygian", 4),
    ("lydian", 5),
    ("mixolydian", 6),
    ("locrian", 7),
    ("harm min", 8),
    ("mel min", 9),
];
const SCALE_LABELS: [&str; 9] =
    ["major", "minor", "dorian", "phrygian", "lydian", "mixolydian", "locrian", "harm min", "mel min"];

/// A chord quality: its name as a suffix (`m7`) and its intervals above the
/// root, low to high.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Quality {
    /// Suffix after the root's name ("" for major).
    pub suffix: &'static str,
    /// Semitones above the root.
    pub intervals: &'static [i32],
}

const fn q(suffix: &'static str, intervals: &'static [i32]) -> Quality {
    Quality { suffix, intervals }
}

/// Every chord quality, in the order slots index them. Never reorder: saved
/// progressions refer to these by index.
pub const QUALITIES: [Quality; 28] = [
    q("", &[0, 4, 7]),
    q("m", &[0, 3, 7]),
    q("dim", &[0, 3, 6]),
    q("aug", &[0, 4, 8]),
    q("sus2", &[0, 2, 7]),
    q("sus4", &[0, 5, 7]),
    q("maj7", &[0, 4, 7, 11]),
    q("m7", &[0, 3, 7, 10]),
    q("7", &[0, 4, 7, 10]),
    q("m7b5", &[0, 3, 6, 10]),
    q("dim7", &[0, 3, 6, 9]),
    q("mmaj7", &[0, 3, 7, 11]),
    q("augmaj7", &[0, 4, 8, 11]),
    q("6", &[0, 4, 7, 9]),
    q("m6", &[0, 3, 7, 9]),
    q("add9", &[0, 4, 7, 14]),
    q("madd9", &[0, 3, 7, 14]),
    q("7sus4", &[0, 5, 7, 10]),
    q("maj9", &[0, 4, 7, 11, 14]),
    q("m9", &[0, 3, 7, 10, 14]),
    q("9", &[0, 4, 7, 10, 14]),
    q("7b9", &[0, 4, 7, 10, 13]),
    q("m7b9", &[0, 3, 7, 10, 13]),
    q("m9b5", &[0, 3, 6, 10, 14]),
    q("m7b5b9", &[0, 3, 6, 10, 13]),
    q("mmaj9", &[0, 3, 7, 11, 14]),
    q("augmaj9", &[0, 4, 8, 11, 14]),
    q("5", &[0, 7]),
];

/// Quality indices by name, for code that builds chords.
pub mod quality {
    /// Major triad.
    pub const MAJ: u8 = 0;
    /// Minor triad.
    pub const MIN: u8 = 1;
    /// Diminished triad.
    pub const DIM: u8 = 2;
    /// Suspended second.
    pub const SUS2: u8 = 4;
    /// Suspended fourth.
    pub const SUS4: u8 = 5;
    /// Major seventh.
    pub const MAJ7: u8 = 6;
    /// Minor seventh.
    pub const MIN7: u8 = 7;
    /// Dominant seventh.
    pub const DOM7: u8 = 8;
    /// Major sixth.
    pub const SIX: u8 = 13;
    /// Added ninth.
    pub const ADD9: u8 = 15;
    /// Dominant ninth.
    pub const NINE: u8 = 20;
}

/// How the notes of a chord are spread.
pub const VOICINGS: [&str; 5] = ["close", "drop 2", "drop 3", "spread", "open"];
/// How chords are played over their length.
pub const RHYTHMS: [&str; 10] =
    ["hold", "1/2", "1/4", "1/8", "offbeat", "charleston", "push", "arp up", "arp down", "arp up-down"];
const KEY_FROM: [&str; 2] = ["project", "own"];
const OFF_ON: [&str; 2] = ["off", "on"];
const BASS: [&str; 3] = ["off", "root", "low root"];
const STRUMS: [&str; 4] = ["off", "1/64", "1/32", "1/16"];

/// The chord builder's parameters, in knob order.
pub const PARAMS: [ParamSpec; 12] = [
    p("key", "key", 0.0, 1.0, 0.0, Unit::Choice(&KEY_FROM)),
    p("root", "root", 0.0, 11.0, 0.0, Unit::Choice(scale::ROOTS)),
    p("scale", "scale", 0.0, 8.0, 0.0, Unit::Choice(&SCALE_LABELS)),
    p("octave", "octave", 1.0, 6.0, 4.0, Unit::Int),
    p("voicing", "voicing", 0.0, 4.0, 0.0, Unit::Choice(&VOICINGS)),
    p("lead", "lead", 0.0, 1.0, 1.0, Unit::Choice(&OFF_ON)),
    p("bass", "bass", 0.0, 2.0, 0.0, Unit::Choice(&BASS)),
    p("rhythm", "rhythm", 0.0, 9.0, 0.0, Unit::Choice(&RHYTHMS)),
    p("strum", "strum", 0.0, 3.0, 0.0, Unit::Choice(&STRUMS)),
    p("gate", "gate", 0.05, 1.0, 0.9, Unit::Percent),
    p("vel", "vel", 0.0, 1.0, 0.7, Unit::Percent),
    p("swing", "swing", 0.0, 0.6, 0.0, Unit::Percent),
];

fn spec(key: &str) -> &'static ParamSpec {
    PARAMS.iter().find(|p| p.key == key).expect("known parameter")
}

/// A key: root pitch class and an index into [`SCALES`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Key {
    /// Root pitch class, 0–11.
    pub root: u8,
    /// Index into [`SCALES`].
    pub scale: usize,
}

impl Key {
    /// The chord builder's key for the project key: scales that are not
    /// seven-note modes map to the closest one (major pentatonic and whole
    /// tone to major, minor pentatonic and blues to minor).
    pub fn of_project(k: sat_doc::Key) -> Key {
        let scale = match k.scale as usize {
            i @ 1..=9 => i - 1,
            11 | 12 => 1,
            _ => 0,
        };
        Key { root: k.root.min(11), scale }
    }

    /// The semitones of the scale above the root.
    pub fn degrees(&self) -> &'static [u8] {
        scale::degrees(SCALES[self.scale.min(SCALES.len() - 1)].1)
    }

    /// Is this a minor-ish key (a minor third over the root)?
    pub fn minor(&self) -> bool {
        self.degrees()[2] == 3
    }

    /// The parallel key used for borrowed chords: minor for a major-third
    /// key, major for a minor-third one.
    pub fn parallel(&self) -> Key {
        Key { root: self.root, scale: if self.minor() { 0 } else { 1 } }
    }

    /// The label, such as "a minor".
    pub fn label(&self) -> String {
        format!("{} {}", scale::ROOTS[self.root as usize % 12], SCALES[self.scale.min(SCALES.len() - 1)].0)
    }
}

/// A chord builder's parameters, read from its module with defaults filled in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Params {
    /// The key chords are relative to.
    pub key: Key,
    /// The octave chord roots sit in (4: middle c).
    pub octave: i32,
    /// Index into [`VOICINGS`].
    pub voicing: usize,
    /// Each chord takes the inversion nearest the one before.
    pub lead: bool,
    /// 0 none, 1 root an octave down, 2 root two octaves down.
    pub bass: u8,
    /// Index into [`RHYTHMS`].
    pub rhythm: usize,
    /// Ticks between strummed notes.
    pub strum: i64,
    /// Note length as a fraction of a hit.
    pub gate: f32,
    /// Velocity.
    pub vel: f32,
    /// Delay of the off-beat sixteenths, as a fraction of a sixteenth.
    pub swing: f32,
}

impl Params {
    /// Reads the parameters of `m`, taking the key from `project` when the
    /// module follows it.
    pub fn of(m: &Module, project: sat_doc::Key) -> Params {
        let get = |k: &str| {
            let s = spec(k);
            s.clamp(m.params.get(k).copied().unwrap_or(s.default))
        };
        let key = if get("key") < 0.5 {
            Key::of_project(project)
        } else {
            Key { root: get("root") as u8, scale: get("scale") as usize }
        };
        Params {
            key,
            octave: get("octave") as i32,
            voicing: get("voicing") as usize,
            lead: get("lead") >= 0.5,
            bass: get("bass") as u8,
            rhythm: get("rhythm") as usize,
            strum: [0, PPQ / 16, PPQ / 8, PPQ / 4][(get("strum") as usize).min(3)],
            gate: get("gate"),
            vel: get("vel"),
            swing: get("swing"),
        }
    }

    /// The parameters of a chord builder in `doc`, macros applied.
    pub fn in_doc(doc: &Document, m: &Module) -> Params {
        Params::of(&crate::macros::resolved(doc, m), doc.key)
    }
}

/// A chord before it is placed in time: root above the key root and quality.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chord {
    /// Root, semitones above the key root (0–11).
    pub root: u8,
    /// Index into [`QUALITIES`].
    pub quality: u8,
}

/// A chord as a progression names it: on a degree of the key (built from
/// that many thirds of its scale, so it follows the scale), or a fixed chord
/// (root above the key root, quality) for chords from outside the key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Scale degree (0 the tonic) and size (3 triad, 4 seventh, 5 ninth).
    Degree(u8, u8),
    /// A fixed chord: root above the key root and quality.
    Fixed(u8, u8),
}

impl Step {
    /// The chord it is in `key`.
    pub fn resolve(self, key: Key) -> Chord {
        match self {
            Step::Degree(d, size) => diatonic(key, d as usize, size as usize),
            Step::Fixed(root, quality) => Chord { root: root % 12, quality },
        }
    }

    /// The step a slot names.
    pub fn of(s: &ChordSlot) -> Step {
        if s.stack > 0 { Step::Degree(s.root, s.stack) } else { Step::Fixed(s.root, s.quality) }
    }

    /// Writes the step into a slot (root, quality and stack).
    pub fn set(self, s: &mut ChordSlot) {
        match self {
            Step::Degree(d, size) => {
                s.root = d;
                s.stack = size;
                s.quality = 0;
            }
            Step::Fixed(root, quality) => {
                s.root = root % 12;
                s.quality = quality;
                s.stack = 0;
            }
        }
    }

    /// A slot holding this step.
    pub fn slot(self, start: Ticks, len: Ticks) -> ChordSlot {
        let mut s = ChordSlot { start, len, root: 0, quality: 0, stack: 0, inversion: 0, octave: 0 };
        self.set(&mut s);
        s
    }
}

/// The quality whose intervals are exactly `iv`, if one is.
pub fn quality_of(iv: &[i32]) -> Option<u8> {
    QUALITIES.iter().position(|q| q.intervals == iv).map(|i| i as u8)
}

/// The chord built on scale degree `degree` (0 for the tonic) of `key` by
/// stacking `size` thirds of the scale (3 a triad, 4 a seventh, 5 a ninth).
/// A stack with no name in [`QUALITIES`] drops its top notes until it has one.
pub fn diatonic(key: Key, degree: usize, size: usize) -> Chord {
    let deg = key.degrees();
    let n = deg.len();
    let d = degree % n;
    let at = |i: usize| deg[(d + i) % n] as i32 + 12 * ((d + i) / n) as i32;
    let root = at(0);
    let mut iv: Vec<i32> = (0..size.clamp(2, 5)).map(|k| at(2 * k) - root).collect();
    loop {
        if let Some(qi) = quality_of(&iv) {
            return Chord { root: root.rem_euclid(12) as u8, quality: qi };
        }
        iv.pop();
        if iv.len() < 3 {
            return Chord { root: root.rem_euclid(12) as u8, quality: quality::MAJ };
        }
    }
}

/// The key's chords on each degree: `size` 3 for triads, 4 for sevenths.
pub fn diatonic_row(key: Key, size: usize) -> Vec<Chord> {
    (0..key.degrees().len()).map(|d| diatonic(key, d, size)).collect()
}

/// Chords borrowed from the parallel key: its triads that are not already
/// in this key, on degrees in order.
pub fn borrowed_row(key: Key) -> Vec<Chord> {
    let own = diatonic_row(key, 3);
    let par = key.parallel();
    diatonic_row(par, 3).into_iter().filter(|c| !own.contains(c)).collect()
}

/// The pads the editor offers, row by row: the key's triads and sevenths on
/// each degree (diatonic steps, which follow the scale), and the chords
/// borrowed from the parallel key (fixed).
pub fn pad_rows(key: Key) -> [Vec<Step>; 3] {
    let n = key.degrees().len() as u8;
    [
        (0..n).map(|d| Step::Degree(d, 3)).collect(),
        (0..n).map(|d| Step::Degree(d, 4)).collect(),
        borrowed_row(key).into_iter().map(|c| Step::Fixed(c.root, c.quality)).collect(),
    ]
}

/// The chord's name in `key`, such as "am7" or "f#dim".
pub fn name(key: Key, c: Chord) -> String {
    let pc = (key.root as usize + c.root as usize) % 12;
    let root = scale::ROOTS[pc];
    let mut s = root.to_uppercase();
    s.push_str(QUALITIES[(c.quality as usize).min(QUALITIES.len() - 1)].suffix);
    s
}

/// The chord's roman numeral in `key`: upper case for a major third, lower
/// for a minor one. A root in the scale takes its degree; any other is
/// numbered as in the parallel major, with a flat when it is not in that
/// either, so borrowed chords read the usual way ("bVII", "iv" in a major
/// key; "IV", "iii" from the major in a minor one).
pub fn roman(key: Key, c: Chord) -> String {
    const NUM: [&str; 7] = ["i", "ii", "iii", "iv", "v", "vi", "vii"];
    const MAJOR: [u8; 7] = [0, 2, 4, 5, 7, 9, 11];
    let deg = key.degrees();
    let (prefix, d) = match deg.iter().position(|x| *x == c.root) {
        Some(i) => ("", i),
        None => match MAJOR.iter().position(|x| *x == c.root) {
            Some(i) => ("", i),
            None => ("b", MAJOR.iter().position(|x| *x == (c.root + 1) % 12).unwrap_or(0)),
        },
    };
    let qual = QUALITIES[(c.quality as usize).min(QUALITIES.len() - 1)];
    let major = qual.intervals.get(1).is_none_or(|i| *i != 3);
    let mut s = String::from(prefix);
    let n = NUM[d.min(6)];
    s.push_str(&if major { n.to_uppercase() } else { n.to_string() });
    s.push_str(match qual.suffix {
        "" | "m" => "",
        "dim" => "°",
        "aug" => "+",
        "m7b5" => "ø7",
        "dim7" => "°7",
        x if !major && x.starts_with('m') && !x.starts_with("maj") => &x[1..],
        x => x,
    });
    s
}

/// The pitches of `c` played at `octave`, inverted `inversion` times and
/// spread by `voicing` (an index into [`VOICINGS`]), low to high.
pub fn voice(key: Key, c: Chord, octave: i32, inversion: u8, voicing: usize) -> Vec<i32> {
    let q = QUALITIES[(c.quality as usize).min(QUALITIES.len() - 1)];
    let base = 12 * (octave + 1) + key.root as i32 + c.root as i32;
    let mut v: Vec<i32> = q.intervals.iter().map(|i| base + i).collect();
    for _ in 0..inversion as usize % v.len().max(1) {
        v[0] += 12;
        v.sort_unstable();
    }
    spread(&mut v, voicing);
    v
}

/// Spreads a close chord (sorted) by voicing.
fn spread(v: &mut [i32], voicing: usize) {
    let n = v.len();
    match voicing {
        // drop 2: the second voice from the top goes down an octave
        1 if n >= 3 => v[n - 2] -= 12,
        // drop 3: the third from the top
        2 if n >= 4 => v[n - 3] -= 12,
        2 if n == 3 => v[n - 2] -= 12,
        // spread: every other voice up an octave
        3 => {
            for (i, x) in v.iter_mut().enumerate() {
                if i % 2 == 1 {
                    *x += 12;
                }
            }
        }
        // open: the lowest voice down an octave
        4 if n >= 2 => v[0] -= 12,
        _ => {}
    }
    v.sort_unstable();
}

/// How far apart two chords' voices are: each voice of `b` to the nearest
/// voice of `a`, summed.
fn distance(a: &[i32], b: &[i32]) -> i32 {
    b.iter().map(|x| a.iter().map(|y| (x - y).abs()).min().unwrap_or(0)).sum()
}

/// The voicing of `c` nearest `prev` over every inversion and the octaves
/// around `octave`, as voice leading picks it.
pub fn lead_from(prev: &[i32], key: Key, c: Chord, octave: i32, voicing: usize) -> Vec<i32> {
    let n = QUALITIES[(c.quality as usize).min(QUALITIES.len() - 1)].intervals.len();
    let centre = 12 * (octave + 1) + key.root as i32 + 6;
    let mut best: Option<(i32, Vec<i32>)> = None;
    for oct in [octave - 1, octave, octave + 1] {
        for inv in 0..n as u8 {
            let v = voice(key, c, oct, inv, voicing);
            // keep near the register asked for, then move as little as possible
            let mid = (v[0] + v[v.len() - 1]) / 2;
            let cost = distance(prev, &v) * 4 + (mid - centre).abs();
            if best.as_ref().is_none_or(|(b, _)| cost < *b) {
                best = Some((cost, v));
            }
        }
    }
    best.map(|(_, v)| v).unwrap_or_default()
}

/// The pitches each slot of a progression plays, in order, with voice
/// leading applied when it is on, and the bass note when there is one.
pub fn voiced(p: &Params, chords: &[ChordSlot]) -> Vec<(Vec<i32>, Option<i32>)> {
    let mut prev: Option<Vec<i32>> = None;
    chords
        .iter()
        .map(|s| {
            let c = Step::of(s).resolve(p.key);
            let oct = p.octave + s.octave as i32;
            let v = match (&prev, p.lead) {
                (Some(pv), true) => lead_from(pv, p.key, c, oct, p.voicing),
                _ => voice(p.key, c, oct, s.inversion, p.voicing),
            };
            prev = Some(v.clone());
            let bass = (p.bass > 0).then(|| 12 * (oct + 1) + p.key.root as i32 + c.root as i32 - 12 * p.bass as i32);
            (v, bass)
        })
        .collect()
}

/// The pitches a chord plays live (for auditioning), voiced like the first
/// chord of a progression.
pub fn live_pitches(p: &Params, c: Chord, inversion: u8) -> Vec<u8> {
    let mut slot = Step::Fixed(c.root, c.quality).slot(Ticks::ZERO, Ticks(PPQ));
    slot.inversion = inversion;
    let (v, bass) = voiced(&Params { lead: false, ..*p }, &[slot]).remove(0);
    bass.into_iter().chain(v).map(|x| x.clamp(0, 127) as u8).collect()
}

const SIXTEENTH: i64 = PPQ / 4;

/// The hits of a rhythm within one bar of 4/4, in sixteenths from the bar
/// start, with their length in sixteenths; `None` for hold and the arps.
fn hits(rhythm: usize) -> Option<&'static [(i64, i64)]> {
    Some(match rhythm {
        1 => &[(0, 8), (8, 8)],
        2 => &[(0, 4), (4, 4), (8, 4), (12, 4)],
        3 => &[(0, 2), (2, 2), (4, 2), (6, 2), (8, 2), (10, 2), (12, 2), (14, 2)],
        4 => &[(2, 2), (6, 2), (10, 2), (14, 2)],
        5 => &[(0, 3), (6, 2), (8, 3), (14, 2)],
        6 => &[(0, 3), (3, 3), (6, 4), (10, 2), (12, 2), (14, 2)],
        _ => return None,
    })
}

/// The notes a progression plays over a clip of length `len`. Chords past
/// the clip end are dropped and the last one stops at it.
pub fn generate(p: &Params, chords: &[ChordSlot], len: Ticks) -> Vec<Note> {
    let mut out = Vec::new();
    if len.0 <= 0 {
        return out;
    }
    let vel = p.vel.clamp(0.0, 1.0);
    let swing = |t: i64| if (t / SIXTEENTH) % 2 == 1 { (p.swing * SIXTEENTH as f32) as i64 } else { 0 };
    let mut push = |start: i64, dur: i64, pitch: i32, end: i64| {
        if start >= end || start >= len.0 || !(0..=127).contains(&pitch) {
            return;
        }
        let l = dur.min(end - start).min(len.0 - start).max(1);
        out.push(Note { start: Ticks(start), len: Ticks(l), pitch: pitch as u8, velocity: vel, chance: 1.0 });
    };
    let voices = voiced(p, chords);
    for (s, (v, bass)) in chords.iter().zip(voices) {
        let (c0, c1) = (s.start.0, (s.start.0 + s.len.0).min(len.0));
        if c0 >= c1 {
            continue;
        }
        let gate = |d: i64| ((d as f32 * p.gate).round() as i64).max(1);
        match p.rhythm {
            7..=9 => {
                // arps: sixteenths through the voices; the bass holds
                let mut order: Vec<i32> = v.clone();
                if p.rhythm == 8 {
                    order.reverse();
                }
                if p.rhythm == 9 && order.len() > 2 {
                    let down: Vec<i32> = order[1..order.len() - 1].iter().rev().copied().collect();
                    order.extend(down);
                }
                let mut t = c0;
                let mut i = 0;
                while t < c1 {
                    push(t + swing(t - c0), gate(SIXTEENTH), order[i % order.len()], c1);
                    t += SIXTEENTH;
                    i += 1;
                }
                if let Some(b) = bass {
                    push(c0, gate(c1 - c0), b, c1);
                }
            }
            r => {
                let starts: Vec<(i64, i64)> = match hits(r) {
                    None => vec![(c0, c1 - c0)],
                    Some(bar) => {
                        let bar_len = 16 * SIXTEENTH;
                        let mut v = Vec::new();
                        let mut b = c0;
                        while b < c1 {
                            for (at, d) in bar {
                                let t = b + at * SIXTEENTH;
                                if t < c1 {
                                    v.push((t + swing(t - c0), d * SIXTEENTH));
                                }
                            }
                            b += bar_len;
                        }
                        v
                    }
                };
                for (t, d) in starts {
                    for (i, pitch) in v.iter().enumerate() {
                        let off = i as i64 * p.strum;
                        push(t + off, gate(d) - off.min(gate(d) - 1), *pitch, c1);
                    }
                    if let Some(b) = bass {
                        push(t, gate(d), b, c1);
                    }
                }
            }
        }
    }
    out.sort_by(|a, b| a.start.cmp(&b.start).then(a.pitch.cmp(&b.pitch)));
    out.dedup_by(|a, b| a.start == b.start && a.pitch == b.pitch);
    out
}

use Step::{Degree as D, Fixed as F};

/// Common progressions: name, the mood it suits (major or minor keys), and
/// its chords, one per bar (or shared out over the clip).
pub const PROGRESSIONS: [(&str, &[Step]); 16] = [
    ("pop  I V vi IV", &[D(0, 3), D(4, 3), D(5, 3), D(3, 3)]),
    ("doo-wop  I vi IV V", &[D(0, 3), D(5, 3), D(3, 3), D(4, 3)]),
    ("sensitive  vi IV I V", &[D(5, 3), D(3, 3), D(0, 3), D(4, 3)]),
    ("ii V I", &[D(1, 4), D(4, 4), D(0, 4), D(0, 4)]),
    ("canon", &[D(0, 3), D(4, 3), D(5, 3), D(2, 3), D(3, 3), D(0, 3), D(3, 3), D(4, 3)]),
    ("royal road", &[D(3, 4), D(4, 4), D(2, 4), D(5, 4)]),
    ("soul  ii iii IV V", &[D(1, 4), D(2, 4), D(3, 4), D(4, 4)]),
    ("rock  I bVII IV I", &[D(0, 3), F(10, quality::MAJ), D(3, 3), D(0, 3)]),
    (
        "blues 12 bar",
        &[
            F(0, quality::DOM7),
            F(0, quality::DOM7),
            F(0, quality::DOM7),
            F(0, quality::DOM7),
            F(5, quality::DOM7),
            F(5, quality::DOM7),
            F(0, quality::DOM7),
            F(0, quality::DOM7),
            F(7, quality::DOM7),
            F(5, quality::DOM7),
            F(0, quality::DOM7),
            F(7, quality::DOM7),
        ],
    ),
    ("minor  i VI III VII", &[D(0, 3), D(5, 3), D(2, 3), D(6, 3)]),
    ("andalusian", &[F(0, quality::MIN), F(10, quality::MAJ), F(8, quality::MAJ), F(7, quality::MAJ)]),
    ("epic  i bVI bVII i", &[F(0, quality::MIN), F(8, quality::MAJ), F(10, quality::MAJ), F(0, quality::MIN)]),
    ("dark  i iv v i", &[F(0, quality::MIN), F(5, quality::MIN), F(7, quality::MIN), F(0, quality::MIN)]),
    ("jazz  I vi ii V", &[D(0, 4), D(5, 4), D(1, 4), D(4, 4)]),
    ("lift  IV V iii vi", &[D(3, 3), D(4, 3), D(2, 3), D(5, 3)]),
    ("lofi  ii9 V9 Imaj9", &[D(1, 5), D(4, 5), D(0, 5), D(0, 5)]),
];

/// The chords of preset `index`.
pub fn preset(index: usize) -> &'static [Step] {
    PROGRESSIONS[index % PROGRESSIONS.len()].1
}

const BAR: i64 = PPQ * 4;

/// Lays `chords` out over a clip of length `len`: a bar each when they fit
/// (repeating to fill the clip), otherwise shared out evenly on beats.
pub fn lay_out(chords: &[Step], len: Ticks) -> Vector<ChordSlot> {
    if chords.is_empty() || len.0 <= 0 {
        return Vector::new();
    }
    let n = chords.len() as i64;
    let each = if n * BAR <= len.0 { BAR } else { ((len.0 / n) / PPQ).max(1) * PPQ };
    let mut out = Vector::new();
    let mut t = 0;
    let mut i = 0;
    while t < len.0 {
        let c = chords[i % chords.len()];
        out.push_back(c.slot(Ticks(t), Ticks(each.min(len.0 - t))));
        t += each;
        i += 1;
        if each != BAR && i >= n as usize {
            break;
        }
    }
    out
}

/// A clip's progression from its chord builder (empty if none).
pub fn progression(doc: &Document, module: sat_doc::ModuleId, clip: sat_doc::ClipId) -> Vec<ChordSlot> {
    doc.pattern(module, clip).map(|p| p.chords.iter().copied().collect()).unwrap_or_default()
}

/// The progression with `c` written in: replacing slot `at` when given,
/// otherwise appended a bar long after the last chord (shorter at the clip
/// end). `None` when there is no room left in the clip.
pub fn put(chords: &[ChordSlot], at: Option<usize>, c: Step, len: Ticks) -> Option<(Vec<ChordSlot>, usize)> {
    let mut v = chords.to_vec();
    if let Some(i) = at.filter(|i| *i < v.len()) {
        c.set(&mut v[i]);
        return Some((v, i));
    }
    let start = v.iter().map(|s| s.start.0 + s.len.0).max().unwrap_or(0);
    if start >= len.0 {
        return None;
    }
    v.push(c.slot(Ticks(start), Ticks(BAR.min(len.0 - start))));
    let i = v.len() - 1;
    Some((v, i))
}

/// The progression without slot `i`; the chords after it close the gap.
pub fn remove(chords: &[ChordSlot], i: usize) -> Vec<ChordSlot> {
    let mut v = chords.to_vec();
    if i >= v.len() {
        return v;
    }
    let gone = v.remove(i);
    for s in v.iter_mut().skip(i) {
        s.start = Ticks((s.start.0 - gone.len.0).max(0));
    }
    v
}

/// The progression with slot `i` `by` beats longer (or shorter), moving the
/// chords after it, never below one beat or past the clip end.
pub fn stretch(chords: &[ChordSlot], i: usize, by: i64, len: Ticks) -> Vec<ChordSlot> {
    let mut v = chords.to_vec();
    let Some(s) = v.get(i).copied() else { return v };
    let new = (s.len.0 + by * PPQ).max(PPQ);
    let tail: i64 = v[i + 1..].iter().map(|x| x.len.0).sum();
    let new = new.min((len.0 - s.start.0 - tail).max(PPQ));
    let d = new - s.len.0;
    v[i].len = Ticks(new);
    for x in v.iter_mut().skip(i + 1) {
        x.start = Ticks(x.start.0 + d);
    }
    v.retain(|x| x.start.0 < len.0);
    v
}

/// The op that sets a clip's progression.
pub fn write(module: sat_doc::ModuleId, clip: sat_doc::ClipId, chords: Vec<ChordSlot>) -> sat_doc::Op {
    sat_doc::Op::WriteChords { module, clip, chords: Vector::from(chords) }
}

/// Ops that drop what the chord builder plays for `clip` into a piano roll
/// on the same grid as notes to edit by hand, and feed the clip from it: the
/// grid's first piano roll when it holds nothing for the clip yet, otherwise
/// a new one beside the chord builder. Returns the piano roll too.
pub fn to_piano_roll(
    doc: &Document,
    module: sat_doc::ModuleId,
    clip: sat_doc::ClipId,
) -> Option<(sat_doc::ModuleId, Vec<sat_doc::Op>)> {
    let m = doc.modules.get(&module)?;
    doc.clips.get(&clip)?;
    let notes = crate::effective_notes(doc, module, clip);
    let free = doc
        .modules_on(m.grid)
        .find(|r| r.kind == crate::PIANO_ROLL && doc.pattern(r.id, clip).is_none_or(|p| p.notes.is_empty()))
        .map(|r| r.id);
    let mut ops = Vec::new();
    let roll = match free {
        Some(r) => r,
        None => {
            let (id, op) = crate::add_module(doc, m.grid, crate::PIANO_ROLL, (m.pos.0, m.pos.1 + 220.0));
            ops.push(op);
            id
        }
    };
    ops.push(sat_doc::Op::WritePattern { module: roll, clip, notes: Vector::from(notes) });
    // wire after the module exists: build the cable by hand when it is new
    let from = sat_doc::PortRef::new(roll, crate::NOTES_OUT.key);
    if let Some(old) = doc.inlet_cable(clip) {
        if old.from == from {
            return Some((roll, ops));
        }
        ops.push(sat_doc::Op::Disconnect { id: old.id });
    }
    ops.push(sat_doc::Op::Connect { id: sat_doc::CableId::new(), from, to: sat_doc::Endpoint::Inlet(clip) });
    Some((roll, ops))
}

#[cfg(test)]
mod tests {
    use super::*;

    const C_MAJ: Key = Key { root: 0, scale: 0 };
    const A_MIN: Key = Key { root: 9, scale: 1 };

    fn names(key: Key, v: &[Chord]) -> Vec<String> {
        v.iter().map(|c| name(key, *c)).collect()
    }

    #[test]
    fn diatonic_chords_of_major_and_minor() {
        assert_eq!(names(C_MAJ, &diatonic_row(C_MAJ, 3)), ["C", "Dm", "Em", "F", "G", "Am", "Bdim"]);
        assert_eq!(names(C_MAJ, &diatonic_row(C_MAJ, 4)), ["Cmaj7", "Dm7", "Em7", "Fmaj7", "G7", "Am7", "Bm7b5"]);
        assert_eq!(names(A_MIN, &diatonic_row(A_MIN, 3)), ["Am", "Bdim", "C", "Dm", "Em", "F", "G"]);
        let romans: Vec<String> = diatonic_row(C_MAJ, 3).iter().map(|c| roman(C_MAJ, *c)).collect();
        assert_eq!(romans, ["I", "ii", "iii", "IV", "V", "vi", "vii°"]);
        let romans: Vec<String> = diatonic_row(C_MAJ, 4).iter().map(|c| roman(C_MAJ, *c)).collect();
        assert_eq!(romans, ["Imaj7", "ii7", "iii7", "IVmaj7", "V7", "vi7", "viiø7"]);
        // harmonic minor has a major v and an augmented III
        let hm = Key { root: 9, scale: 7 };
        assert_eq!(names(hm, &diatonic_row(hm, 3)), ["Am", "Bdim", "Caug", "Dm", "E", "F", "G#dim"]);
        assert_eq!(names(hm, &diatonic_row(hm, 4))[6], "G#dim7");
        // ninths, with b9s named in phrygian
        assert_eq!(names(C_MAJ, &[diatonic(C_MAJ, 4, 5), diatonic(C_MAJ, 1, 5)]), ["G9", "Dm9"]);
        let ph = Key { root: 4, scale: 3 };
        assert_eq!(name(ph, diatonic(ph, 0, 5)), "Em7b9");
    }

    #[test]
    fn borrowed_chords_come_from_the_parallel_key() {
        let b = borrowed_row(C_MAJ);
        assert_eq!(names(C_MAJ, &b), ["Cm", "Ddim", "D#", "Fm", "Gm", "G#", "A#"]);
        let romans: Vec<String> = b.iter().map(|c| roman(C_MAJ, *c)).collect();
        assert_eq!(romans, ["i", "ii°", "bIII", "iv", "v", "bVI", "bVII"]);
        let b = borrowed_row(A_MIN);
        assert_eq!(names(A_MIN, &b), ["A", "Bm", "C#m", "D", "E", "F#m", "G#dim"]);
        let romans: Vec<String> = b.iter().map(|c| roman(A_MIN, *c)).collect();
        assert_eq!(romans, ["I", "ii", "iii", "IV", "V", "vi", "vii°"]);
        assert_eq!(roman(C_MAJ, Chord { root: 1, quality: quality::MAJ }), "bII");
    }

    #[test]
    fn voicings_and_inversions() {
        let am7 = diatonic(C_MAJ, 5, 4);
        assert_eq!(voice(C_MAJ, am7, 4, 0, 0), [69, 72, 76, 79]);
        assert_eq!(voice(C_MAJ, am7, 4, 1, 0), [72, 76, 79, 81]);
        assert_eq!(voice(C_MAJ, am7, 4, 0, 1), [64, 69, 72, 79], "drop 2");
        assert_eq!(voice(C_MAJ, am7, 4, 0, 2), [60, 69, 76, 79], "drop 3");
        assert_eq!(voice(C_MAJ, am7, 4, 0, 3), [69, 76, 84, 91], "spread");
        assert_eq!(voice(C_MAJ, am7, 4, 0, 4), [57, 72, 76, 79], "open");
    }

    #[test]
    fn voice_leading_moves_little() {
        let p = Params {
            key: C_MAJ,
            octave: 4,
            voicing: 0,
            lead: true,
            bass: 1,
            rhythm: 0,
            strum: 0,
            gate: 1.0,
            vel: 0.7,
            swing: 0.0,
        };
        let slots = lay_out(preset(0), Ticks(BAR * 4));
        let v = voiced(&p, &slots.iter().copied().collect::<Vec<_>>());
        // C (60 64 67) then G led: b d g (59 62 67) instead of g b d up a fifth
        assert_eq!(v[0].0, [60, 64, 67]);
        assert_eq!(v[1].0, [59, 62, 67]);
        for w in v.windows(2) {
            assert!(distance(&w[0].0, &w[1].0) <= 5, "{w:?}");
        }
        // bass is the root an octave under the chord's octave
        assert_eq!(v[1].1, Some(55));
        let off = voiced(&Params { lead: false, ..p }, &slots.iter().copied().collect::<Vec<_>>());
        assert_eq!(off[1].0, [67, 71, 74]);
    }

    fn params() -> Params {
        Params {
            key: C_MAJ,
            octave: 4,
            voicing: 0,
            lead: false,
            bass: 0,
            rhythm: 0,
            strum: 0,
            gate: 1.0,
            vel: 0.7,
            swing: 0.0,
        }
    }

    #[test]
    fn rhythms_fill_each_chord() {
        let slots: Vec<ChordSlot> = lay_out(preset(0), Ticks(BAR * 4)).into_iter().collect();
        assert_eq!(slots.len(), 4);
        let hold = generate(&params(), &slots, Ticks(BAR * 4));
        assert_eq!(hold.len(), 12);
        assert!(hold.iter().all(|n| n.len.0 == BAR));
        let quarters = generate(&Params { rhythm: 2, ..params() }, &slots, Ticks(BAR * 4));
        assert_eq!(quarters.len(), 4 * 4 * 3);
        let arp = generate(&Params { rhythm: 7, ..params() }, &slots, Ticks(BAR));
        let pitches: Vec<u8> = arp.iter().take(4).map(|n| n.pitch).collect();
        assert_eq!(pitches, [60, 64, 67, 60]);
        assert_eq!(arp.len(), 16);
        let updown = generate(&Params { rhythm: 9, ..params() }, &slots, Ticks(BAR));
        let pitches: Vec<u8> = updown.iter().take(5).map(|n| n.pitch).collect();
        assert_eq!(pitches, [60, 64, 67, 64, 60]);
        // a clip shorter than the progression cuts it off
        let short = generate(&params(), &slots, Ticks(BAR + PPQ));
        assert_eq!(short.len(), 6);
        assert!(short[3..].iter().all(|n| n.len.0 == PPQ));
    }

    #[test]
    fn strum_and_swing_shift_notes() {
        let slots: Vec<ChordSlot> = lay_out(preset(0), Ticks(BAR)).into_iter().collect();
        let s = generate(&Params { strum: PPQ / 8, ..params() }, &slots, Ticks(BAR));
        let starts: Vec<i64> = s.iter().take(3).map(|n| n.start.0).collect();
        assert_eq!(starts, [0, 120, 240], "a short clip shares out the chords a beat each");
        assert!(s.iter().take(3).all(|n| n.start.0 + n.len.0 == PPQ), "strummed notes end together");
        let sw = generate(&Params { rhythm: 3, swing: 0.5, ..params() }, &slots, Ticks(BAR));
        let starts: Vec<i64> = sw.iter().step_by(3).take(3).map(|n| n.start.0).collect();
        assert_eq!(starts, [0, 480, 960], "eighths do not swing");
    }

    #[test]
    fn building_a_progression() {
        let len = Ticks(BAR * 2);
        let g = Step::Degree(4, 3);
        let (v, i) = put(&[], None, Step::Degree(0, 3), len).unwrap();
        let (v, _) = put(&v, None, g, len).unwrap();
        assert_eq!(i, 0);
        assert!(put(&v, None, g, len).is_none(), "the clip is full");
        let (v, _) = put(&v, Some(0), Step::Degree(5, 3), len).unwrap();
        let resolved = |v: &[ChordSlot], k: Key| v.iter().map(|s| Step::of(s).resolve(k)).collect::<Vec<_>>();
        assert_eq!(names(C_MAJ, &resolved(&v, C_MAJ)), ["Am", "G"]);
        // diatonic chords follow the scale, fixed ones keep their quality
        let (v2, _) = put(&v, Some(1), Step::Fixed(10, quality::MAJ), len).unwrap();
        assert_eq!(names(A_MIN, &resolved(&v2, A_MIN)), ["F", "G"]);
        let v = stretch(&v, 0, -2, len);
        assert_eq!((v[0].len.0, v[1].start.0), (BAR / 2, BAR / 2));
        let v = stretch(&v, 0, 8, len);
        assert_eq!(v[0].len.0, BAR, "the next chord keeps its length inside the clip");
        let v = remove(&v, 0);
        assert_eq!((v.len(), v[0].start.0), (1, 0));
        // presets lay out a bar each, repeating over a long clip
        let laid = lay_out(preset(9), Ticks(BAR * 8));
        assert_eq!(laid.len(), 8);
        assert_eq!(name(A_MIN, Step::of(&laid[4]).resolve(A_MIN)), "Am");
        // and share a short clip out on beats
        let laid = lay_out(preset(4), Ticks(BAR * 2));
        assert_eq!(laid.len(), 8);
        assert!(laid.iter().all(|s| s.len.0 == PPQ));
    }

    #[test]
    fn a_clip_plays_its_progression_and_drops_into_a_piano_roll() {
        use sat_doc::*;
        let mut h = History::new(Document::new(1), 50);
        let (track, op) = Op::add_track("t", None, 0);
        let Op::AddTrack { grids, .. } = &op else { unreachable!() };
        let grid = grids.source.unwrap();
        h.apply("t", [op]).unwrap();
        let (cb, op) = crate::add_module(h.doc(), grid, CHORD_BUILDER, (0.0, 0.0));
        let clip = ClipId::new();
        h.apply(
            "m",
            [
                op,
                Op::AddClip {
                    id: clip,
                    track,
                    name: "c".into(),
                    color: Color(0),
                    length: Ticks(BAR * 2),
                    looped: true,
                    seed: 0,
                },
            ],
        )
        .unwrap();
        h.apply("w", crate::wire(h.doc(), cb, clip)).unwrap();
        assert!(crate::effective_notes(h.doc(), cb, clip).is_empty(), "no chords yet");
        let chords = lay_out(preset(0), Ticks(BAR * 2));
        h.apply("p", [write(cb, clip, chords.into_iter().collect())]).unwrap();
        let pitches = |h: &History| -> Vec<u8> {
            crate::effective_notes(h.doc(), cb, clip).iter().filter(|n| n.start.0 == 0).map(|n| n.pitch).collect()
        };
        assert_eq!(pitches(&h), [60, 64, 67]);
        // following the project key: a minor moves the progression to a
        h.apply("k", [Op::SetKey { key: sat_doc::Key { root: 9, scale: 2 } }]).unwrap();
        assert_eq!(pitches(&h), [69, 72, 76], "the tonic chord is now a minor");
        // its own key when set to
        let set =
            |k: &str, v: f32| Op::SetValue { target: ParamTarget::Module { module: cb, param: k.into() }, value: v };
        h.apply("own", [set("key", 1.0), set("root", 2.0)]).unwrap();
        assert_eq!(pitches(&h), [62, 66, 69]);
        assert!(crate::shapes_notes(h.doc(), &ParamTarget::Module { module: cb, param: "voicing".into() }));
        assert_eq!(crate::editor_of(CHORD_BUILDER), crate::Editor::Chords);
        // drop into a piano roll: the clip now plays the roll with the same notes
        let before = crate::effective_notes(h.doc(), cb, clip);
        let (roll, ops) = to_piano_roll(h.doc(), cb, clip).unwrap();
        h.apply("drop", ops).unwrap();
        assert_eq!(crate::editing_source(h.doc(), clip), Some(roll));
        assert_eq!(crate::effective_notes(h.doc(), roll, clip), before);
        assert_eq!(h.doc().modules[&roll].kind, crate::PIANO_ROLL);
        // the progression stays with the chord builder; a second drop makes a new roll
        assert_eq!(progression(h.doc(), cb, clip).len(), 4);
        h.apply("w", crate::wire(h.doc(), cb, clip)).unwrap();
        let (roll2, ops) = to_piano_roll(h.doc(), cb, clip).unwrap();
        h.apply("drop", ops).unwrap();
        assert_ne!(roll, roll2);
        // the project file keeps progressions
        let back = sat_doc::file::from_bytes(&sat_doc::file::to_bytes(h.doc()).unwrap()).unwrap();
        assert_eq!(progression(&back, cb, clip).len(), 4);
    }

    #[test]
    fn project_keys_map_to_modes() {
        assert_eq!(Key::of_project(sat_doc::Key { root: 9, scale: 2 }), A_MIN);
        assert_eq!(Key::of_project(sat_doc::Key { root: 2, scale: 11 }).scale, 1);
        assert_eq!(Key::of_project(sat_doc::Key { root: 2, scale: 0 }).scale, 0);
        assert_eq!(C_MAJ.label(), "c major");
        assert!(PROGRESSIONS.iter().all(|(n, s)| !n.is_empty() && !s.is_empty()));
    }
}
