//! The euclidean generator: `hits` onsets spread as evenly as possible over
//! `steps`, rotated, with an evenly spread subset of them accented.

use sat_doc::sat_time::{PPQ, Ticks};
use sat_doc::{Module, Note};

pub use crate::spec::{ParamSpec, note_name};
use crate::spec::{RATE_LABELS, Unit, p, rate_ticks};

/// Step lengths the rate parameter picks from: 1/32, 1/16, 1/8, 1/4.
pub const RATES: [Ticks; 4] = [Ticks(PPQ / 8), Ticks(PPQ / 4), Ticks(PPQ / 2), Ticks(PPQ)];

/// The euclid's parameters, in knob order.
pub const PARAMS: [ParamSpec; 8] = [
    p("steps", "steps", 1.0, 64.0, 16.0, Unit::Int),
    p("hits", "hits", 0.0, 64.0, 4.0, Unit::Int),
    p("rot", "rot", 0.0, 63.0, 0.0, Unit::Int),
    p("acc", "acc", 0.0, 64.0, 0.0, Unit::Int),
    p("note", "note", 0.0, 127.0, 48.0, Unit::Note),
    p("rate", "rate", 0.0, 3.0, 1.0, Unit::Choice(&RATE_LABELS)),
    p("gate", "gate", 0.05, 1.0, 0.5, Unit::Percent),
    p("vel", "vel", 0.0, 1.0, 0.7, Unit::Percent),
];

/// The spec for a parameter key.
pub fn spec(key: &str) -> Option<&'static ParamSpec> {
    PARAMS.iter().find(|p| p.key == key)
}

/// `hits` onsets over `steps`, as evenly as possible, starting on a hit.
pub fn rhythm(steps: usize, hits: usize) -> Vec<bool> {
    let hits = hits.min(steps);
    (0..steps).map(|i| hits > 0 && (i * hits) % steps < hits).collect()
}

/// [`rhythm`] rotated right by `rot` steps.
pub fn rotated(steps: usize, hits: usize, rot: usize) -> Vec<bool> {
    let base = rhythm(steps, hits);
    if steps == 0 {
        return base;
    }
    (0..steps).map(|i| base[(i + steps - rot % steps) % steps]).collect()
}

/// A euclid's parameters, read from its module with defaults filled in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Params {
    /// Steps in one cycle.
    pub steps: usize,
    /// Onsets per cycle.
    pub hits: usize,
    /// Rotation in steps.
    pub rot: usize,
    /// Accented onsets per cycle.
    pub acc: usize,
    /// Pitch.
    pub note: u8,
    /// Step length.
    pub rate: Ticks,
    /// Note length as a fraction of a step.
    pub gate: f32,
    /// Velocity of unaccented notes; accents play at full velocity.
    pub vel: f32,
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
            hits: get("hits") as usize,
            rot: get("rot") as usize,
            acc: get("acc") as usize,
            note: get("note") as u8,
            rate: Ticks(rate_ticks(get("rate"))),
            gate: get("gate"),
            vel: get("vel"),
        }
    }

    /// One cycle: (hit, accent) per step.
    pub fn cycle(&self) -> Vec<(bool, bool)> {
        let hits = rotated(self.steps, self.hits, self.rot);
        let n = hits.iter().filter(|h| **h).count();
        let acc = rhythm(n, self.acc);
        let mut k = 0;
        hits.into_iter()
            .map(|h| {
                if !h {
                    return (false, false);
                }
                k += 1;
                (true, acc[k - 1])
            })
            .collect()
    }

    /// Note length.
    pub fn note_len(&self) -> Ticks {
        Ticks(((self.rate.0 as f32 * self.gate).round() as i64).max(1))
    }

    /// The notes for a clip of length `len`: the cycle repeats from the start
    /// of the clip until its end.
    pub fn generate(&self, len: Ticks) -> Vec<Note> {
        let cycle = self.cycle();
        if cycle.is_empty() || self.rate.0 <= 0 {
            return Vec::new();
        }
        let total = (len.0 + self.rate.0 - 1) / self.rate.0;
        let nl = self.note_len();
        (0..total)
            .filter_map(|i| {
                let (hit, acc) = cycle[i as usize % cycle.len()];
                let start = i * self.rate.0;
                hit.then(|| Note {
                    start: Ticks(start),
                    len: Ticks(nl.0.min(len.0 - start).max(1)),
                    pitch: self.note,
                    velocity: if acc { 1.0 } else { self.vel },
                    chance: 1.0,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[bool]) -> String {
        v.iter().map(|b| if *b { 'x' } else { '.' }).collect()
    }

    #[test]
    fn rhythms_spread_evenly() {
        assert_eq!(s(&rhythm(8, 3)), "x..x..x.");
        assert_eq!(s(&rhythm(16, 4)), "x...x...x...x...");
        assert_eq!(s(&rhythm(5, 0)), ".....");
        assert_eq!(s(&rhythm(4, 9)), "xxxx");
        assert_eq!(s(&rotated(8, 3, 2)), "x.x..x..");
        assert_eq!(s(&rotated(8, 3, 10)), s(&rotated(8, 3, 2)));
        assert_eq!(rhythm(16, 7).iter().filter(|b| **b).count(), 7);
    }

    #[test]
    fn generate_repeats_the_cycle_over_the_clip() {
        let p = Params { steps: 3, hits: 1, rot: 0, acc: 1, note: 40, rate: RATES[1], gate: 0.5, vel: 0.6 };
        let notes = p.generate(Ticks(PPQ * 4));
        let starts: Vec<i64> = notes.iter().map(|n| n.start.0 / 240).collect();
        assert_eq!(starts, vec![0, 3, 6, 9, 12, 15]);
        assert!(notes.iter().all(|n| n.len == Ticks(120) && n.pitch == 40 && n.velocity == 1.0));
        // two accents over four hits fall on the first and third
        let p = Params { steps: 16, hits: 4, acc: 2, ..p };
        let v: Vec<f32> = p.generate(Ticks(PPQ * 4)).iter().map(|n| n.velocity).collect();
        assert_eq!(v, vec![1.0, 0.6, 1.0, 0.6]);
    }

    #[test]
    fn specs_clamp_and_show() {
        let rate = spec("rate").unwrap();
        assert_eq!(rate.show(1.0), "1/16");
        assert_eq!(rate.at(1.0), 3.0);
        assert_eq!(spec("note").unwrap().show(48.0), "c3");
        assert_eq!(spec("gate").unwrap().show(0.5), "50%");
        assert_eq!(spec("hits").unwrap().clamp(70.4), 64.0);
        let steps = spec("steps").unwrap();
        assert_eq!(steps.at(steps.pos(16.0)), 16.0);
    }
}
