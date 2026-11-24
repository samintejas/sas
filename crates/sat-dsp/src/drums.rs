//! Synthesised drum voices, picked by General MIDI pitch, so a drum track
//! sounds like drums without any samples.

use crate::{Mode, Noise, Osc, Shape, Svf, midi_hz};

/// The kind of drum a pitch plays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Kind {
    /// Bass drum (35, 36).
    #[default]
    Kick,
    /// Side stick (37).
    Rim,
    /// Snare (38, 40).
    Snare,
    /// Hand clap (39).
    Clap,
    /// Closed or pedal hi-hat (42, 44).
    ClosedHat,
    /// Open hi-hat (46).
    OpenHat,
    /// Toms (41, 43, 45, 47, 48, 50).
    Tom,
    /// Crash, ride, splash and china cymbals.
    Cymbal,
    /// Anything else: a short pitched blip.
    Blip,
}

impl Kind {
    /// The drum a General MIDI pitch plays.
    pub fn of(pitch: u8) -> Kind {
        match pitch {
            35 | 36 => Kind::Kick,
            37 => Kind::Rim,
            38 | 40 => Kind::Snare,
            39 => Kind::Clap,
            42 | 44 => Kind::ClosedHat,
            46 => Kind::OpenHat,
            41 | 43 | 45 | 47 | 48 | 50 => Kind::Tom,
            49 | 51 | 52 | 53 | 55 | 57 | 59 => Kind::Cymbal,
            _ => Kind::Blip,
        }
    }

    /// Base length in seconds (before the decay control).
    pub fn length(self) -> f32 {
        match self {
            Kind::Kick => 0.45,
            Kind::Rim => 0.06,
            Kind::Snare => 0.25,
            Kind::Clap => 0.3,
            Kind::ClosedHat => 0.08,
            Kind::OpenHat => 0.5,
            Kind::Tom => 0.4,
            Kind::Cymbal => 1.6,
            Kind::Blip => 0.2,
        }
    }
}

/// One sounding drum hit.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Voice {
    /// What it plays.
    pub kind: Kind,
    /// Samples since the hit.
    pub t: u32,
    /// Velocity, 0–1.
    pub vel: f32,
    /// Base frequency for tonal parts.
    pub hz: f32,
    osc: Osc,
    noise: Noise,
    f1: Svf,
    f2: Svf,
}

/// Controls shared by all hits of a kit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Kit {
    /// Tuning in semitones.
    pub tune: f32,
    /// Length multiplier (0.25–4).
    pub decay: f32,
}

impl Default for Kit {
    fn default() -> Self {
        Kit { tune: 0.0, decay: 1.0 }
    }
}

fn env(t: f32, len: f32) -> f32 {
    // exponential fall to about -60 dB at `len`
    (-6.9 * t / len.max(1e-3)).exp()
}

impl Voice {
    /// A hit of `pitch` at velocity `vel`, with noise seeded by `seed`.
    pub fn new(pitch: u8, vel: f32, seed: u32) -> Voice {
        let kind = Kind::of(pitch);
        let hz = match kind {
            Kind::Kick => 52.0,
            Kind::Snare => 185.0,
            Kind::Rim => 1700.0,
            Kind::Tom => midi_hz(pitch as f32 - 2.0),
            _ => midi_hz(pitch as f32),
        };
        Voice { kind, t: 0, vel, hz, noise: Noise::new(seed.wrapping_mul(2_654_435_761) | 1), ..Voice::default() }
    }

    /// Finished?
    pub fn done(&self, kit: &Kit, sr: f32) -> bool {
        self.t as f32 > self.kind.length() * kit.decay.clamp(0.25, 4.0) * sr
    }

    /// The next sample.
    pub fn next(&mut self, kit: &Kit, sr: f32) -> f32 {
        let t = self.t as f32 / sr;
        self.t = self.t.saturating_add(1);
        let len = self.kind.length() * kit.decay.clamp(0.25, 4.0);
        let tune = 2f32.powf(kit.tune / 12.0);
        let hz = self.hz * tune;
        let y = match self.kind {
            Kind::Kick => {
                let sweep = hz * (1.0 + 2.2 * (-t / 0.03).exp());
                let body = self.osc.next(Shape::Sine, sweep / sr, 0.0) * env(t, len);
                let click = self.noise.sample() * env(t, 0.004) * 0.5;
                (body + click) * 1.1
            }
            Kind::Rim => {
                let tone = self.osc.next(Shape::Triangle, hz / sr, 0.0) * 0.6;
                let n = self.f1.process(self.noise.sample(), 2400.0 * tune, 0.6, sr, Mode::BandPass);
                (tone + n) * env(t, len)
            }
            Kind::Snare => {
                let tone = self.osc.next(Shape::Triangle, hz * (1.0 + 0.5 * (-t / 0.01).exp()) / sr, 0.0);
                let n = self.f1.process(self.noise.sample(), 1800.0 * tune, 0.1, sr, Mode::HighPass);
                tone * env(t, len * 0.5) * 0.7 + n * env(t, len) * 0.8
            }
            Kind::Clap => {
                // three quick bursts, then a tail
                let burst =
                    [0.0, 0.011, 0.022].iter().map(|s| if t >= *s { env(t - s, 0.012) } else { 0.0 }).sum::<f32>();
                let shape = burst + 0.6 * env(t, len) * if t > 0.022 { 1.0 } else { 0.0 };
                self.f1.process(self.noise.sample(), 1200.0 * tune, 0.5, sr, Mode::BandPass) * shape * 1.6
            }
            Kind::ClosedHat | Kind::OpenHat => {
                let n = self.f1.process(self.noise.sample(), 7500.0 * tune, 0.2, sr, Mode::HighPass);
                n * env(t, len) * 0.7
            }
            Kind::Tom => {
                let sweep = hz * (1.0 + 0.6 * (-t / 0.05).exp());
                self.osc.next(Shape::Sine, sweep / sr, 0.0) * env(t, len)
            }
            Kind::Cymbal => {
                let hi = self.f1.process(self.noise.sample(), 5200.0 * tune, 0.3, sr, Mode::HighPass);
                let ring = self.f2.process(hi, 9000.0 * tune, 0.5, sr, Mode::BandPass);
                (hi * 0.5 + ring * 0.5) * env(t, len) * 0.6
            }
            Kind::Blip => self.osc.next(Shape::Sine, hz / sr, 0.0) * env(t, len) * 0.6,
        };
        y * self.vel
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_sounds_and_ends() {
        let sr = 48_000.0;
        let kit = Kit::default();
        for pitch in [36u8, 37, 38, 39, 42, 46, 45, 49, 60] {
            let mut v = Voice::new(pitch, 1.0, 3);
            let mut peak = 0.0f32;
            let mut n = 0;
            while !v.done(&kit, sr) {
                peak = peak.max(v.next(&kit, sr).abs());
                n += 1;
            }
            assert!(peak > 0.05 && peak < 2.5, "{pitch}: {peak}");
            assert!(n as f32 <= Kind::of(pitch).length() * sr + 2.0);
        }
        assert_eq!(Kind::of(44), Kind::ClosedHat);
        // longer decay, longer hit
        let long = Kit { decay: 2.0, ..kit };
        let v = Voice { t: (0.6 * sr) as u32, ..Voice::new(36, 1.0, 1) };
        assert!(v.done(&kit, sr) && !v.done(&long, sr));
    }
}
