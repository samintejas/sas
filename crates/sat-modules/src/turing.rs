//! The turing generator: a looping shift register of random bits. Each step
//! the register turns by one and, with probability `change`, the bit coming
//! round is flipped, so at 0 the phrase repeats every `steps` steps and at 1
//! it repeats every `2 × steps`; in between it slowly mutates. The low eight
//! bits pick a pitch from a scale over `range` semitones above `low`, and
//! (scrambled) decide whether the step plays, `dens` of the time.
//!
//! Everything comes from a seed, so a clip sounds the same every time it is
//! played or exported. With `reseed` on, each time round a looping clip
//! plays a fresh phrase from (clip seed, loop count).

use sat_doc::sat_time::Ticks;
use sat_doc::{Module, Note};

use crate::spec::{ParamSpec, RATE_LABELS, Unit, p, rate_ticks};

/// The scales `scale` picks from: name and the semitones it holds.
pub const SCALES: [(&str, &[u8]); 5] = [
    ("chromatic", &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]),
    ("major", &[0, 2, 4, 5, 7, 9, 11]),
    ("minor", &[0, 2, 3, 5, 7, 8, 10]),
    ("penta", &[0, 3, 5, 7, 10]),
    ("dorian", &[0, 2, 3, 5, 7, 9, 10]),
];

const SCALE_LABELS: [&str; 5] = ["chromatic", "major", "minor", "penta", "dorian"];
const OFF_ON: [&str; 2] = ["off", "on"];

/// The turing's parameters, in knob order.
pub const PARAMS: [ParamSpec; 10] = [
    p("steps", "steps", 2.0, 32.0, 8.0, Unit::Int),
    p("change", "change", 0.0, 1.0, 0.1, Unit::Percent),
    p("low", "low", 0.0, 120.0, 48.0, Unit::Note),
    p("range", "range", 1.0, 48.0, 12.0, Unit::Int),
    p("scale", "scale", 0.0, 4.0, 3.0, Unit::Choice(&SCALE_LABELS)),
    p("dens", "dens", 0.0, 1.0, 0.75, Unit::Percent),
    p("rate", "rate", 0.0, 3.0, 1.0, Unit::Choice(&RATE_LABELS)),
    p("gate", "gate", 0.05, 1.0, 0.5, Unit::Percent),
    p("vel", "vel", 0.0, 1.0, 0.7, Unit::Percent),
    p("reseed", "reseed", 0.0, 1.0, 0.0, Unit::Choice(&OFF_ON)),
];

/// The spec for a parameter key.
pub fn spec(key: &str) -> Option<&'static ParamSpec> {
    PARAMS.iter().find(|p| p.key == key)
}

/// A turing's parameters, read from its module with defaults filled in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Params {
    /// Register length in steps.
    pub steps: usize,
    /// Chance of flipping the bit that comes round, per step.
    pub change: f32,
    /// Lowest pitch.
    pub low: u8,
    /// Semitones above `low` the pitches spread over.
    pub range: u8,
    /// Index into [`SCALES`].
    pub scale: usize,
    /// Share of steps that play.
    pub density: f32,
    /// Step length.
    pub rate: Ticks,
    /// Note length as a fraction of a step.
    pub gate: f32,
    /// Velocity.
    pub vel: f32,
    /// A fresh phrase each time round a looping clip.
    pub reseed: bool,
}

/// One step of a phrase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Step {
    /// The pitch the register gives this step (whether it plays or not).
    pub pitch: u8,
    /// Does it play?
    pub on: bool,
}

/// splitmix64: the generator's random numbers.
pub(crate) fn mix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

impl Params {
    /// Reads the parameters of `m`.
    pub fn of(m: &Module) -> Params {
        let get = |k: &str| {
            let s = spec(k).expect("known parameter");
            s.clamp(m.params.get(k).copied().unwrap_or(s.default))
        };
        Params {
            steps: get("steps") as usize,
            change: get("change"),
            low: get("low") as u8,
            range: get("range") as u8,
            scale: get("scale") as usize,
            density: get("dens"),
            rate: Ticks(rate_ticks(get("rate"))),
            gate: get("gate"),
            vel: get("vel"),
            reseed: get("reseed") >= 0.5,
        }
    }

    /// Note length.
    pub fn note_len(&self) -> Ticks {
        Ticks(((self.rate.0 as f32 * self.gate).round() as i64).max(1))
    }

    /// The pitches the register can pick, low to high.
    pub fn pitches(&self) -> Vec<u8> {
        let (_, degrees) = SCALES[self.scale.min(SCALES.len() - 1)];
        (0..=self.range as u16)
            .filter(|s| degrees.contains(&((s % 12) as u8)))
            .map(|s| (self.low as u16 + s).min(127) as u8)
            .collect()
    }

    /// `n` steps of the phrase that `seed` starts.
    pub fn sequence(&self, n: usize, seed: u64) -> Vec<Step> {
        let len = self.steps.clamp(2, 32);
        let mask = if len == 32 { u32::MAX } else { (1u32 << len) - 1 };
        let mut rng = mix(seed);
        let mut reg = (rng as u32) & mask;
        let pitches = self.pitches();
        let dens = (self.density.clamp(0.0, 1.0) * 256.0).round() as u32;
        (0..n)
            .map(|_| {
                // the low eight bits, wrapping round a short register
                let v = (0..8).fold(0u32, |v, j| v | (((reg >> (j % len)) & 1) << j));
                let pitch = pitches[(v as usize * pitches.len()) >> 8];
                // a scrambled copy decides the gate, so it does not follow pitch
                let on = (v.wrapping_mul(167).wrapping_add(91) & 0xff) < dens;
                // turn the register, maybe flipping the bit that comes round
                let top = (reg >> (len - 1)) & 1;
                rng = mix(rng);
                let flip = ((rng >> 40) as f32 / (1u64 << 24) as f32) < self.change;
                reg = ((reg << 1) | (top ^ flip as u32)) & mask;
                Step { pitch, on }
            })
            .collect()
    }

    /// The notes for a clip of length `len` from `seed`.
    pub fn generate(&self, len: Ticks, seed: u64) -> Vec<Note> {
        if self.rate.0 <= 0 || len.0 <= 0 {
            return Vec::new();
        }
        let n = ((len.0 + self.rate.0 - 1) / self.rate.0) as usize;
        let nl = self.note_len();
        self.sequence(n, seed)
            .into_iter()
            .enumerate()
            .filter(|(_, s)| s.on)
            .map(|(i, s)| {
                let start = i as i64 * self.rate.0;
                Note {
                    start: Ticks(start),
                    len: Ticks(nl.0.min(len.0 - start).max(1)),
                    pitch: s.pitch,
                    velocity: self.vel,
                    chance: 1.0,
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::euclid::RATES;

    fn params() -> Params {
        Params {
            steps: 8,
            change: 0.0,
            low: 48,
            range: 12,
            scale: 3,
            density: 1.0,
            rate: RATES[1],
            gate: 0.5,
            vel: 0.7,
            reseed: false,
        }
    }

    #[test]
    fn a_locked_register_repeats_every_length() {
        let p = params();
        let s = p.sequence(32, 7);
        assert_eq!(s[..8], s[8..16]);
        assert_eq!(s[..8], s[24..32]);
        // the same seed gives the same phrase, another seed another one
        assert_eq!(s, p.sequence(32, 7));
        assert_ne!(s, p.sequence(32, 8));
    }

    #[test]
    fn full_change_inverts_each_time_round() {
        let p = Params { change: 1.0, ..params() };
        let s = p.sequence(32, 3);
        assert_ne!(s[..8], s[8..16]);
        assert_eq!(s[..8], s[16..24]);
    }

    #[test]
    fn some_change_mutates_slowly() {
        let p = Params { change: 0.1, steps: 16, ..params() };
        let s = p.sequence(16 * 20, 11);
        let same = (16..s.len()).filter(|&i| s[i] == s[i - 16]).count() as f32 / (s.len() - 16) as f32;
        assert!(same > 0.3 && same < 0.9, "{same}");
    }

    #[test]
    fn pitches_stay_in_the_scale_and_range() {
        let p = Params { scale: 1, low: 60, range: 24, ..params() };
        assert_eq!(p.pitches().first(), Some(&60));
        assert_eq!(p.pitches().last(), Some(&84));
        let major = [0, 2, 4, 5, 7, 9, 11];
        for s in p.sequence(200, 5) {
            assert!((60..=84).contains(&s.pitch) && major.contains(&((s.pitch - 60) % 12)));
        }
        // the register reaches across the range
        let seen: std::collections::BTreeSet<u8> = (0..20).flat_map(|k| p.sequence(16, k)).map(|s| s.pitch).collect();
        assert!(seen.len() >= 10, "{seen:?}");
    }

    #[test]
    fn density_sets_how_many_steps_play() {
        let count = |d: f32| {
            (0..10).map(|k| Params { density: d, ..params() }.generate(Ticks(960 * 16), k).len()).sum::<usize>()
        };
        assert_eq!(count(0.0), 0);
        assert_eq!(count(1.0), 640);
        let half = count(0.5);
        assert!((200..440).contains(&half), "{half}");
    }

    #[test]
    fn notes_sit_on_the_steps_and_stop_at_the_clip_end() {
        let p = params();
        let notes = p.generate(Ticks(240 * 5 + 60), 1);
        assert_eq!(notes.len(), 6);
        assert!(notes.iter().enumerate().all(|(i, n)| n.start.0 == i as i64 * 240));
        assert_eq!(notes[5].len, Ticks(60));
        assert!(notes[..5].iter().all(|n| n.len == Ticks(120) && n.velocity == 0.7));
    }
}
