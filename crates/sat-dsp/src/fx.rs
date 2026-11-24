//! Stereo effects for fx grids. Each processes a block in place; buffers are
//! allocated once in `new` and never again.

use core::f32::consts::PI;

use crate::{Mode, Svf, db_gain};

/// Flushes values too small to matter (denormals) to zero.
fn flush(x: f32) -> f32 {
    if x.abs() < 1e-20 { 0.0 } else { x }
}

/// One-pole smoothing coefficient for a time constant in seconds.
fn coef(secs: f32, sr: f32) -> f32 {
    if secs <= 0.0 { 0.0 } else { (-1.0 / (secs * sr)).exp() }
}

/// A biquad filter (transposed direct form II).
#[derive(Debug, Clone, Copy, Default)]
pub struct Biquad {
    b: [f32; 3],
    a: [f32; 2],
    z: [f32; 2],
}

impl Biquad {
    fn set(&mut self, b0: f32, b1: f32, b2: f32, a0: f32, a1: f32, a2: f32) {
        self.b = [b0 / a0, b1 / a0, b2 / a0];
        self.a = [a1 / a0, a2 / a0];
    }

    fn w(hz: f32, sr: f32) -> (f32, f32) {
        let w = 2.0 * PI * hz.clamp(10.0, sr * 0.49) / sr;
        (w.cos(), w.sin())
    }

    /// A low shelf at `hz` with `db` of gain.
    pub fn low_shelf(&mut self, hz: f32, db: f32, sr: f32) {
        let a = 10f32.powf(db / 40.0);
        let (c, s) = Biquad::w(hz, sr);
        let alpha = s / 2.0 * 2f32.sqrt();
        let k = 2.0 * a.sqrt() * alpha;
        self.set(
            a * ((a + 1.0) - (a - 1.0) * c + k),
            2.0 * a * ((a - 1.0) - (a + 1.0) * c),
            a * ((a + 1.0) - (a - 1.0) * c - k),
            (a + 1.0) + (a - 1.0) * c + k,
            -2.0 * ((a - 1.0) + (a + 1.0) * c),
            (a + 1.0) + (a - 1.0) * c - k,
        );
    }

    /// A high shelf at `hz` with `db` of gain.
    pub fn high_shelf(&mut self, hz: f32, db: f32, sr: f32) {
        let a = 10f32.powf(db / 40.0);
        let (c, s) = Biquad::w(hz, sr);
        let alpha = s / 2.0 * 2f32.sqrt();
        let k = 2.0 * a.sqrt() * alpha;
        self.set(
            a * ((a + 1.0) + (a - 1.0) * c + k),
            -2.0 * a * ((a - 1.0) + (a + 1.0) * c),
            a * ((a + 1.0) + (a - 1.0) * c - k),
            (a + 1.0) - (a - 1.0) * c + k,
            2.0 * ((a - 1.0) - (a + 1.0) * c),
            (a + 1.0) - (a - 1.0) * c - k,
        );
    }

    /// A peaking band at `hz` with `db` of gain and quality `q`.
    pub fn peak(&mut self, hz: f32, db: f32, q: f32, sr: f32) {
        let a = 10f32.powf(db / 40.0);
        let (c, s) = Biquad::w(hz, sr);
        let alpha = s / (2.0 * q);
        self.set(1.0 + alpha * a, -2.0 * c, 1.0 - alpha * a, 1.0 + alpha / a, -2.0 * c, 1.0 - alpha / a);
    }

    /// A high-pass at `hz` (Butterworth).
    pub fn high_pass(&mut self, hz: f32, sr: f32) {
        let (c, s) = Biquad::w(hz, sr);
        let alpha = s / 2f32.sqrt();
        self.set((1.0 + c) / 2.0, -(1.0 + c), (1.0 + c) / 2.0, 1.0 + alpha, -2.0 * c, 1.0 - alpha);
    }

    /// A low-pass at `hz` (Butterworth).
    pub fn low_pass(&mut self, hz: f32, sr: f32) {
        let (c, s) = Biquad::w(hz, sr);
        let alpha = s / 2f32.sqrt();
        self.set((1.0 - c) / 2.0, 1.0 - c, (1.0 - c) / 2.0, 1.0 + alpha, -2.0 * c, 1.0 - alpha);
    }

    /// Filters one sample.
    pub fn process(&mut self, x: f32) -> f32 {
        let y = self.b[0] * x + self.z[0];
        self.z[0] = flush(self.b[1] * x - self.a[0] * y + self.z[1]);
        self.z[1] = flush(self.b[2] * x - self.a[1] * y);
        y
    }
}

/// Three-band eq: low shelf (120 Hz), a peaking mid at a chosen frequency,
/// high shelf (8 kHz). Params: low, mid, high (dB), freq (Hz).
#[derive(Debug, Clone, Default)]
pub struct Eq {
    f: [[Biquad; 2]; 3],
}

impl Eq {
    /// Processes a block.
    pub fn process(&mut self, l: &mut [f32], r: &mut [f32], p: &[f32], sr: f32) {
        let (low, mid, high, freq) = (p[0], p[1], p[2], p[3]);
        for ch in 0..2 {
            self.f[0][ch].low_shelf(120.0, low, sr);
            self.f[1][ch].peak(freq, mid, 0.9, sr);
            self.f[2][ch].high_shelf(8000.0, high, sr);
        }
        for (ch, buf) in [l, r].into_iter().enumerate() {
            for x in buf.iter_mut() {
                let a = self.f[0][ch].process(*x);
                let b = self.f[1][ch].process(a);
                *x = self.f[2][ch].process(b);
            }
        }
    }
}

/// A stereo-linked compressor. Params: thresh (dB), ratio, attack (s),
/// release (s), makeup (dB). `side`, when given, drives the detector.
#[derive(Debug, Clone, Default)]
pub struct Comp {
    gr: f32,
    /// The deepest gain reduction (dB, positive) in the last block.
    pub meter: f32,
}

impl Comp {
    /// Processes a block.
    pub fn process(&mut self, l: &mut [f32], r: &mut [f32], side: Option<(&[f32], &[f32])>, p: &[f32], sr: f32) {
        let (thr, ratio, att, rel, makeup) = (p[0], p[1].max(1.0), p[2], p[3], p[4]);
        let (ca, cr) = (coef(att, sr), coef(rel, sr));
        let slope = 1.0 - 1.0 / ratio;
        let mut deepest = 0.0f32;
        for i in 0..l.len() {
            let lvl = match side {
                Some((sl, sr_)) => sl[i].abs().max(sr_[i].abs()),
                None => l[i].abs().max(r[i].abs()),
            };
            let db = 20.0 * lvl.max(1e-6).log10();
            let target = ((db - thr) * slope).max(0.0);
            let c = if target > self.gr { ca } else { cr };
            self.gr = flush(target + c * (self.gr - target));
            deepest = deepest.max(self.gr);
            let g = db_gain(makeup - self.gr);
            l[i] *= g;
            r[i] *= g;
        }
        self.meter = deepest;
    }
}

/// A stereo delay with filtered feedback. Params: time (s), feedback, tone
/// (Hz, low-pass in the loop), mix.
#[derive(Debug, Clone)]
pub struct Delay {
    buf: [Vec<f32>; 2],
    pos: usize,
    time: f32,
    tone: [Biquad; 2],
}

impl Delay {
    /// Longest delay time in seconds.
    pub const MAX_SECS: f32 = 2.0;

    /// A delay for sample rate `sr`.
    pub fn new(sr: f32) -> Delay {
        let n = (Delay::MAX_SECS * sr) as usize + 4;
        Delay { buf: [vec![0.0; n], vec![0.0; n]], pos: 0, time: -1.0, tone: Default::default() }
    }

    /// Processes a block.
    pub fn process(&mut self, l: &mut [f32], r: &mut [f32], p: &[f32], sr: f32) {
        let n = self.buf[0].len();
        let want = (p[0].clamp(0.001, Delay::MAX_SECS) * sr).min((n - 3) as f32);
        if self.time < 0.0 {
            self.time = want;
        }
        let (fb, mix) = (p[1].clamp(0.0, 0.98), p[3].clamp(0.0, 1.0));
        for t in &mut self.tone {
            t.low_pass(p[2], sr);
        }
        let glide = coef(0.05, sr);
        for i in 0..l.len() {
            self.time = want + glide * (self.time - want);
            let d = self.time;
            let k = d.floor() as usize;
            let frac = d - k as f32;
            for (ch, x) in [&mut l[i], &mut r[i]].into_iter().enumerate() {
                let b = &self.buf[ch];
                let a = b[(self.pos + n - k) % n];
                let c = b[(self.pos + n - k - 1) % n];
                let wet = a + (c - a) * frac;
                let back = self.tone[ch].process(wet) * fb;
                self.buf[ch][self.pos] = flush(*x + back);
                *x = *x * (1.0 - mix) + wet * mix;
            }
            self.pos = (self.pos + 1) % n;
        }
    }
}

const COMBS: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASSES: [usize; 4] = [556, 441, 341, 225];
const SPREAD: usize = 23;

#[derive(Debug, Clone)]
struct Line {
    buf: Vec<f32>,
    pos: usize,
    store: f32,
}

impl Line {
    fn new(n: usize) -> Line {
        Line { buf: vec![0.0; n.max(1)], pos: 0, store: 0.0 }
    }

    /// Comb with damping, using the first `len` samples of the buffer.
    fn comb(&mut self, x: f32, len: usize, g: f32, damp: f32) -> f32 {
        let len = len.clamp(1, self.buf.len());
        if self.pos >= len {
            self.pos = 0;
        }
        let y = self.buf[self.pos];
        self.store = flush(y * (1.0 - damp) + self.store * damp);
        self.buf[self.pos] = flush(x + self.store * g);
        self.pos += 1;
        y
    }

    fn allpass(&mut self, x: f32) -> f32 {
        let y = self.buf[self.pos];
        self.buf[self.pos] = flush(x + y * 0.5);
        self.pos = (self.pos + 1) % self.buf.len();
        y - x
    }
}

/// A Schroeder-Moorer reverb (eight damped combs and four all-passes per
/// side). Params: size, decay (s), pre (s), lowcut (Hz), mix.
#[derive(Debug, Clone)]
pub struct Reverb {
    combs: [Vec<Line>; 2],
    alls: [Vec<Line>; 2],
    pre: Vec<f32>,
    pre_pos: usize,
    lowcut: Biquad,
    scale: f32,
}

impl Reverb {
    /// Longest pre-delay in seconds.
    pub const MAX_PRE: f32 = 0.25;

    /// A reverb for sample rate `sr`.
    pub fn new(sr: f32) -> Reverb {
        let scale = sr / 44_100.0;
        let side = |off: usize| -> (Vec<Line>, Vec<Line>) {
            let c = COMBS.iter().map(|n| Line::new((((n + off) as f32) * scale * 1.5) as usize)).collect();
            let a = ALLPASSES.iter().map(|n| Line::new((((n + off) as f32) * scale) as usize)).collect();
            (c, a)
        };
        let ((cl, al), (cr, ar)) = (side(0), side(SPREAD));
        Reverb {
            combs: [cl, cr],
            alls: [al, ar],
            pre: vec![0.0; (Reverb::MAX_PRE * sr) as usize + 2],
            pre_pos: 0,
            lowcut: Biquad::default(),
            scale,
        }
    }

    /// Processes a block.
    pub fn process(&mut self, l: &mut [f32], r: &mut [f32], p: &[f32], sr: f32) {
        let (size, decay, pre, lowcut, mix) =
            (p[0].clamp(0.0, 1.0), p[1].max(0.05), p[2].clamp(0.0, Reverb::MAX_PRE), p[3], p[4].clamp(0.0, 1.0));
        let stretch = 0.5 + size;
        self.lowcut.high_pass(lowcut, sr);
        let n = self.pre.len();
        let pd = ((pre * sr) as usize).min(n - 1);
        let damp = 0.25;
        // per-comb feedback for a 60 dB decay over `decay` seconds
        let mut lens = [[0usize; 8]; 2];
        let mut gains = [[0f32; 8]; 2];
        for ch in 0..2 {
            for (k, base) in COMBS.iter().enumerate() {
                let len = (((base + ch * SPREAD) as f32) * self.scale * stretch) as usize;
                lens[ch][k] = len;
                gains[ch][k] = 10f32.powf(-3.0 * len as f32 / (decay * sr)).min(0.98);
            }
        }
        for i in 0..l.len() {
            let dry = (l[i], r[i]);
            let x = self.lowcut.process((dry.0 + dry.1) * 0.5);
            self.pre[self.pre_pos] = x;
            let input = self.pre[(self.pre_pos + n - pd) % n] * 0.03;
            self.pre_pos = (self.pre_pos + 1) % n;
            let mut out = [0.0f32; 2];
            for ch in 0..2 {
                let mut acc = 0.0;
                for (k, c) in self.combs[ch].iter_mut().enumerate() {
                    acc += c.comb(input, lens[ch][k], gains[ch][k], damp);
                }
                for a in self.alls[ch].iter_mut() {
                    acc = a.allpass(acc);
                }
                out[ch] = acc;
            }
            l[i] = dry.0 * (1.0 - mix) + out[0] * mix;
            r[i] = dry.1 * (1.0 - mix) + out[1] * mix;
        }
    }
}

/// A saturator. Params: drive (dB), bias, tone (Hz), level (dB), mix.
#[derive(Debug, Clone, Default)]
pub struct Drive {
    tone: [Biquad; 2],
    dc: [Biquad; 2],
}

impl Drive {
    /// Processes a block.
    pub fn process(&mut self, l: &mut [f32], r: &mut [f32], p: &[f32], sr: f32) {
        let (g, bias, level, mix) = (db_gain(p[0]), p[1].clamp(-1.0, 1.0) * 0.5, db_gain(p[3]), p[4].clamp(0.0, 1.0));
        let norm = 1.0 / g.sqrt();
        for ch in 0..2 {
            self.tone[ch].low_pass(p[2], sr);
            self.dc[ch].high_pass(15.0, sr);
        }
        for (ch, buf) in [l, r].into_iter().enumerate() {
            for x in buf.iter_mut() {
                let wet = ((*x * g + bias).tanh() - bias.tanh()) * norm;
                let wet = self.dc[ch].process(self.tone[ch].process(wet));
                *x = (*x * (1.0 - mix) + wet * mix) * level;
            }
        }
    }
}

/// A look-ahead peak limiter. Params: ceiling (dB), release (s).
#[derive(Debug, Clone)]
pub struct Limiter {
    buf: [Vec<f32>; 2],
    pos: usize,
    gain: f32,
    /// The deepest gain reduction (dB, positive) in the last block.
    pub meter: f32,
}

impl Limiter {
    /// Look-ahead in seconds.
    pub const AHEAD: f32 = 0.0015;

    /// A limiter for sample rate `sr`.
    pub fn new(sr: f32) -> Limiter {
        let n = ((Limiter::AHEAD * sr) as usize).max(1);
        Limiter { buf: [vec![0.0; n], vec![0.0; n]], pos: 0, gain: 1.0, meter: 0.0 }
    }

    /// Processes a block.
    pub fn process(&mut self, l: &mut [f32], r: &mut [f32], p: &[f32], sr: f32) {
        let ceil = db_gain(p[0].min(0.0));
        let n = self.buf[0].len();
        let ca = coef(Limiter::AHEAD / 3.0, sr);
        let cr = coef(p[1].max(0.001), sr);
        let mut lowest = 1.0f32;
        for i in 0..l.len() {
            let peak = l[i].abs().max(r[i].abs());
            let target = if peak > ceil { ceil / peak } else { 1.0 };
            let c = if target < self.gain { ca } else { cr };
            self.gain = target + c * (self.gain - target);
            lowest = lowest.min(self.gain);
            let (dl, dr) = (self.buf[0][self.pos], self.buf[1][self.pos]);
            self.buf[0][self.pos] = l[i];
            self.buf[1][self.pos] = r[i];
            self.pos = (self.pos + 1) % n;
            l[i] = (dl * self.gain).clamp(-ceil, ceil);
            r[i] = (dr * self.gain).clamp(-ceil, ceil);
        }
        self.meter = -20.0 * lowest.max(1e-6).log10();
    }
}

/// A stereo state-variable filter. Params: mode, cutoff (Hz), res.
#[derive(Debug, Clone, Default)]
pub struct Filter {
    f: [Svf; 2],
}

impl Filter {
    /// Processes a block.
    pub fn process(&mut self, l: &mut [f32], r: &mut [f32], p: &[f32], sr: f32) {
        let mode = Mode::from_index(p[0]);
        for (ch, buf) in [l, r].into_iter().enumerate() {
            for x in buf.iter_mut() {
                *x = self.f[ch].process(*x, p[1], p[2], sr, mode);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn sine(hz: f32, n: usize, amp: f32) -> Vec<f32> {
        (0..n).map(|i| (2.0 * PI * hz * i as f32 / SR).sin() * amp).collect()
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    #[test]
    fn eq_boosts_and_cuts_its_bands() {
        let gain = |hz: f32, p: [f32; 4]| {
            let mut e = Eq::default();
            let (mut l, mut r) = (sine(hz, 9600, 0.5), sine(hz, 9600, 0.5));
            e.process(&mut l, &mut r, &p, SR);
            20.0 * (rms(&l[4800..]) / rms(&sine(hz, 9600, 0.5)[4800..])).log10()
        };
        assert!(gain(1000.0, [0.0; 4]).abs() < 0.1, "flat is flat");
        assert!((gain(40.0, [6.0, 0.0, 0.0, 1000.0]) - 6.0).abs() < 0.7);
        assert!((gain(1000.0, [0.0, -6.0, 0.0, 1000.0]) + 6.0).abs() < 0.5);
        assert!((gain(16_000.0, [0.0, 0.0, 4.0, 1000.0]) - 4.0).abs() < 0.7);
    }

    #[test]
    fn comp_reduces_loud_signals_by_the_ratio() {
        let mut c = Comp::default();
        // a 0 dBFS sine peaks at 0 dB: 18 dB over a -18 threshold, 4:1 leaves 4.5
        let (mut l, mut r) = (sine(1000.0, 48_000, 1.0), sine(1000.0, 48_000, 1.0));
        c.process(&mut l, &mut r, None, &[-18.0, 4.0, 0.001, 0.05, 0.0], SR);
        assert!((c.meter - 13.5).abs() < 1.0, "gr {}", c.meter);
        let out = 20.0 * (rms(&l[24_000..]) / rms(&sine(1000.0, 24_000, 1.0))).log10();
        assert!(out < -11.0 && out > -15.0, "{out}");
        // quiet signals pass untouched
        let (mut l, mut r) = (sine(1000.0, 4800, 0.01), sine(1000.0, 4800, 0.01));
        let mut c = Comp::default();
        c.process(&mut l, &mut r, None, &[-18.0, 4.0, 0.001, 0.05, 0.0], SR);
        assert_eq!(l, sine(1000.0, 4800, 0.01));
    }

    #[test]
    fn delay_repeats_after_its_time() {
        let mut d = Delay::new(SR);
        let (mut l, mut r) = (vec![0.0f32; 48_000], vec![0.0f32; 48_000]);
        l[0] = 1.0;
        r[0] = 1.0;
        d.process(&mut l, &mut r, &[0.25, 0.5, 20_000.0, 1.0], SR);
        let peak = l.iter().enumerate().skip(1).max_by(|a, b| a.1.abs().total_cmp(&b.1.abs())).unwrap().0;
        assert!((peak as i32 - 12_000).abs() <= 2, "first echo at {peak}");
        assert!(l[24_000].abs() > 0.2 && l[24_000].abs() < l[12_000].abs(), "second echo is quieter");
    }

    #[test]
    fn reverb_rings_and_decays() {
        let mut v = Reverb::new(SR);
        let n = 48_000 * 3;
        let (mut l, mut r) = (vec![0.0f32; n], vec![0.0f32; n]);
        l[0] = 1.0;
        r[0] = 1.0;
        v.process(&mut l, &mut r, &[0.6, 1.0, 0.02, 100.0, 1.0], SR);
        let early = rms(&l[2400..12_000]);
        let late = rms(&l[96_000..]);
        assert!(early > 1e-3, "tail {early}");
        assert!(late < early * 0.01, "decays: {late} vs {early}");
        assert!(l.iter().chain(&r).all(|x| x.is_finite() && x.abs() < 1.0));
        assert_ne!(l, r, "stereo");
    }

    #[test]
    fn limiter_holds_the_ceiling_and_drive_saturates() {
        let mut lim = Limiter::new(SR);
        let (mut l, mut r) = (sine(100.0, 48_000, 2.0), sine(100.0, 48_000, 2.0));
        lim.process(&mut l, &mut r, &[-1.0, 0.05], SR);
        let ceil = db_gain(-1.0);
        assert!(l.iter().all(|x| x.abs() <= ceil + 1e-6));
        assert!(lim.meter > 6.0);
        let mut d = Drive::default();
        let (mut l, mut r) = (sine(200.0, 9600, 0.9), sine(200.0, 9600, 0.9));
        d.process(&mut l, &mut r, &[24.0, 0.0, 20_000.0, 0.0, 1.0], SR);
        assert!(l.iter().all(|x| x.abs() < 1.0));
        // a squashed sine has a higher rms-to-peak ratio than a sine
        let peak = l[4800..].iter().fold(0f32, |a, x| a.max(x.abs()));
        assert!(rms(&l[4800..]) / peak > 0.8, "crest {}", rms(&l[4800..]) / peak);
    }
}
