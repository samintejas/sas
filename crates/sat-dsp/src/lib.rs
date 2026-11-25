//! Signal building blocks for sat's built-in modules: band-limited
//! oscillators, a state-variable filter, envelopes, noise and drum voices.
//!
//! Everything here is plain state plus a per-sample `next`/`process` call:
//! no allocation, no locks, deterministic for the same inputs.

#![warn(missing_docs)]

pub mod drums;
pub mod fx;
pub mod scale;

use core::f32::consts::{PI, TAU};

/// The frequency of a (fractional) MIDI pitch.
pub fn midi_hz(p: f32) -> f32 {
    440.0 * 2f32.powf((p - 69.0) / 12.0)
}

/// Decibels to linear gain.
pub fn db_gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// Oscillator shapes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Shape {
    /// Sine.
    #[default]
    Sine,
    /// Triangle.
    Triangle,
    /// Band-limited saw.
    Saw,
    /// Band-limited square.
    Square,
}

impl Shape {
    /// The shape for a choice index (0 sine, 1 triangle, 2 saw, 3 square).
    pub fn from_index(v: f32) -> Shape {
        match v.round() as i32 {
            1 => Shape::Triangle,
            2 => Shape::Saw,
            3 => Shape::Square,
            _ => Shape::Sine,
        }
    }
}

/// PolyBLEP residual for a discontinuity at phase 0, `dt` = phase increment.
fn blep(t: f32, dt: f32) -> f32 {
    if dt <= 0.0 {
        0.0
    } else if t < dt {
        let x = t / dt;
        x + x - x * x - 1.0
    } else if t > 1.0 - dt {
        let x = (t - 1.0) / dt;
        x * x + x + x + 1.0
    } else {
        0.0
    }
}

/// A phase-accumulating oscillator.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Osc {
    /// Phase, 0–1.
    pub phase: f32,
}

impl Osc {
    /// The next sample of `shape` at `inc` cycles per sample (`hz / sr`),
    /// with `pm` added to the phase (phase modulation, in cycles).
    pub fn next(&mut self, shape: Shape, inc: f32, pm: f32) -> f32 {
        let inc = inc.clamp(0.0, 0.49);
        let t = (self.phase + pm).rem_euclid(1.0);
        let y = match shape {
            Shape::Sine => (t * TAU).sin(),
            Shape::Triangle => 1.0 - 4.0 * (t - 0.5).abs(),
            Shape::Saw => 2.0 * t - 1.0 - blep(t, inc),
            Shape::Square => {
                let sq = if t < 0.5 { 1.0 } else { -1.0 };
                sq + blep(t, inc) - blep((t + 0.5).rem_euclid(1.0), inc)
            }
        };
        self.phase += inc;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        y
    }
}

/// Filter modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// Low-pass.
    #[default]
    LowPass,
    /// Band-pass.
    BandPass,
    /// High-pass.
    HighPass,
}

impl Mode {
    /// The mode for a choice index (0 lp, 1 bp, 2 hp).
    pub fn from_index(v: f32) -> Mode {
        match v.round() as i32 {
            1 => Mode::BandPass,
            2 => Mode::HighPass,
            _ => Mode::LowPass,
        }
    }
}

/// A trapezoidal state-variable filter (Simper / Zavalishin), stable under
/// fast modulation.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Svf {
    ic1: f32,
    ic2: f32,
}

impl Svf {
    /// Filters one sample with cutoff `hz` and resonance `res` (0–1).
    pub fn process(&mut self, x: f32, hz: f32, res: f32, sr: f32, mode: Mode) -> f32 {
        let hz = hz.clamp(10.0, sr * 0.45);
        let g = (PI * hz / sr).tan();
        let k = 2.0 - 1.98 * res.clamp(0.0, 1.0);
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;
        let v3 = x - self.ic2;
        let v1 = a1 * self.ic1 + a2 * v3;
        let v2 = self.ic2 + a2 * self.ic1 + a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        if !self.ic1.is_finite() || !self.ic2.is_finite() {
            *self = Svf::default();
            return 0.0;
        }
        match mode {
            Mode::LowPass => v2,
            Mode::BandPass => v1,
            Mode::HighPass => x - k * v1 - v2,
        }
    }
}

/// Envelope stages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Stage {
    /// Silent, finished.
    #[default]
    Idle,
    /// Rising.
    Attack,
    /// Falling to sustain.
    Decay,
    /// Holding.
    Sustain,
    /// Falling to zero.
    Release,
}

/// An ADSR envelope: linear attack, exponential decay and release.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Adsr {
    /// Current stage.
    pub stage: Stage,
    /// Current level, 0–1.
    pub level: f32,
    gate: bool,
}

/// Times and level for [`Adsr::next`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AdsrParams {
    /// Attack, seconds.
    pub a: f32,
    /// Decay, seconds.
    pub d: f32,
    /// Sustain level, 0–1.
    pub s: f32,
    /// Release, seconds.
    pub r: f32,
}

fn coef(seconds: f32, sr: f32) -> f32 {
    // time to fall to about -60 dB
    (-6.9 / (seconds.max(0.0005) * sr)).exp()
}

impl Adsr {
    /// Advances one sample with `gate` held or not; rising edges retrigger.
    pub fn next(&mut self, gate: bool, p: &AdsrParams, sr: f32) -> f32 {
        if gate && !self.gate {
            self.stage = Stage::Attack;
        } else if !gate && self.gate && self.stage != Stage::Idle {
            self.stage = Stage::Release;
        }
        self.gate = gate;
        let s = p.s.clamp(0.0, 1.0);
        match self.stage {
            Stage::Idle => self.level = 0.0,
            Stage::Attack => {
                self.level += 1.0 / (p.a.max(0.0005) * sr);
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.stage = Stage::Decay;
                }
            }
            Stage::Decay => {
                let c = coef(p.d, sr);
                self.level = s + (self.level - s) * c;
                if (self.level - s).abs() < 1e-4 {
                    self.level = s;
                    self.stage = Stage::Sustain;
                }
            }
            Stage::Sustain => self.level = s,
            Stage::Release => {
                self.level *= coef(p.r, sr);
                if self.level < 1e-4 {
                    self.level = 0.0;
                    self.stage = Stage::Idle;
                }
            }
        }
        self.level
    }

    /// Finished (idle with the gate off).
    pub fn done(&self) -> bool {
        self.stage == Stage::Idle && !self.gate
    }
}

/// A small deterministic noise source (xorshift32).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Noise {
    state: u32,
}

impl Default for Noise {
    fn default() -> Self {
        Noise { state: 0x9e37_79b9 }
    }
}

impl Noise {
    /// A source seeded with `seed` (0 is replaced).
    pub fn new(seed: u32) -> Noise {
        Noise { state: if seed == 0 { 0x9e37_79b9 } else { seed } }
    }

    /// White noise in -1–1.
    pub fn sample(&mut self) -> f32 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.state = x;
        (x >> 8) as f32 / (1u32 << 23) as f32 - 1.0
    }
}

/// Equal-power pan gains (left, right) for `pan` in -1–1, unity at centre.
pub fn pan_gains(pan: f32) -> (f32, f32) {
    let a = (pan.clamp(-1.0, 1.0) + 1.0) * core::f32::consts::FRAC_PI_4;
    (a.cos() * core::f32::consts::SQRT_2, a.sin() * core::f32::consts::SQRT_2)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rms(v: &[f32]) -> f32 {
        (v.iter().map(|x| x * x).sum::<f32>() / v.len() as f32).sqrt()
    }

    #[test]
    fn oscillators_stay_bounded_and_hit_their_pitch() {
        let sr = 48_000.0;
        for shape in [Shape::Sine, Shape::Triangle, Shape::Saw, Shape::Square] {
            let mut o = Osc::default();
            let v: Vec<f32> = (0..4800).map(|_| o.next(shape, 440.0 / sr, 0.0)).collect();
            assert!(v.iter().all(|x| x.abs() <= 1.2), "{shape:?}");
            // count rising zero crossings over 0.1 s: 44 cycles
            let ups = v.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count();
            assert!((43..=45).contains(&ups), "{shape:?} {ups}");
        }
        assert!((midi_hz(69.0) - 440.0).abs() < 1e-3);
        assert!((midi_hz(81.0) - 880.0).abs() < 1e-2);
    }

    #[test]
    fn the_filter_passes_lows_and_cuts_highs() {
        let sr = 48_000.0;
        let tone = |hz: f32, mode: Mode| {
            let (mut o, mut f) = (Osc::default(), Svf::default());
            let v: Vec<f32> =
                (0..9600).map(|_| f.process(o.next(Shape::Sine, hz / sr, 0.0), 1000.0, 0.0, sr, mode)).collect();
            rms(&v[4800..])
        };
        assert!(tone(100.0, Mode::LowPass) > 0.6);
        assert!(tone(8000.0, Mode::LowPass) < 0.05);
        assert!(tone(8000.0, Mode::HighPass) > 0.6);
        assert!(tone(100.0, Mode::HighPass) < 0.05);
        assert!(tone(1000.0, Mode::BandPass) > tone(8000.0, Mode::BandPass) * 4.0);
    }

    #[test]
    fn envelopes_run_their_stages() {
        let sr = 1000.0;
        let p = AdsrParams { a: 0.01, d: 0.05, s: 0.5, r: 0.05 };
        let mut e = Adsr::default();
        let up: Vec<f32> = (0..10).map(|_| e.next(true, &p, sr)).collect();
        assert!((up[9] - 1.0).abs() < 1e-6);
        for _ in 0..200 {
            e.next(true, &p, sr);
        }
        assert_eq!(e.stage, Stage::Sustain);
        assert!((e.level - 0.5).abs() < 1e-3);
        for _ in 0..200 {
            e.next(false, &p, sr);
        }
        assert!(e.done());
    }

    #[test]
    fn noise_is_deterministic_and_centred() {
        let (mut a, mut b) = (Noise::new(7), Noise::new(7));
        let v: Vec<f32> = (0..10_000).map(|_| a.sample()).collect();
        assert!(v.iter().zip((0..10_000).map(|_| b.sample())).all(|(x, y)| *x == y));
        let mean = v.iter().sum::<f32>() / v.len() as f32;
        assert!(mean.abs() < 0.05 && v.iter().all(|x| x.abs() <= 1.0));
        let (l, r) = pan_gains(0.0);
        assert!((l - 1.0).abs() < 1e-6 && (r - 1.0).abs() < 1e-6);
        assert!(pan_gains(-1.0).1.abs() < 1e-6);
    }
}
