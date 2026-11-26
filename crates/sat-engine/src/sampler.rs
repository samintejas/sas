//! The sampler: plays one sound from the audio pool, repitched by note.
//!
//! The sample is shared ([`Arc`]) with the app's audio pool, so swapping a
//! schedule never frees sample memory on the audio thread. Parameters, in
//! order: level (dB), root note, tune (semitones), start (0–1 of the sound),
//! mode (gate, one shot, loop), attack (s), release (s), velocity
//! sensitivity (0–1), loop start (0–1 of the sound).
//!
//! A loop runs from the loop start to the end of the sound. Its last few
//! milliseconds crossfade into the frames just after the loop start, so
//! the seam does not click whatever the sound.

use std::sync::Arc;

use sat_dsp::{Adsr, AdsrParams, db_gain};

use crate::instrument::{MAX_PARAMS, MAX_VOICES};

/// A decoded sound: stereo, at its own sample rate.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Sample {
    /// Sample rate in Hz.
    pub rate: f32,
    /// Left channel (mono sounds have the same data on both channels).
    pub l: Vec<f32>,
    /// Right channel, as long as `l`.
    pub r: Vec<f32>,
}

impl Sample {
    /// A sound from interleaved frames of 1 or 2 channels (extra channels are dropped).
    pub fn from_interleaved(rate: f32, channels: usize, data: &[f32]) -> Sample {
        let ch = channels.max(1);
        let l: Vec<f32> = data.chunks_exact(ch).map(|f| f[0]).collect();
        let r: Vec<f32> = if ch == 1 { l.clone() } else { data.chunks_exact(ch).map(|f| f[1]).collect() };
        Sample { rate, l, r }
    }

    /// Length in frames.
    pub fn frames(&self) -> usize {
        self.l.len().min(self.r.len())
    }

    pub(crate) fn at(&self, pos: f64) -> (f32, f32) {
        let n = self.frames();
        if pos < 0.0 || n == 0 {
            return (0.0, 0.0);
        }
        let i = pos as usize;
        if i >= n {
            return (0.0, 0.0);
        }
        let t = (pos - i as f64) as f32;
        let j = (i + 1).min(n - 1);
        (self.l[i] + (self.l[j] - self.l[i]) * t, self.r[i] + (self.r[j] - self.r[i]) * t)
    }
}

/// How a note plays the sound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayMode {
    /// While the note is held, then release.
    Gate,
    /// To the end of the sound, whatever the note length.
    OneShot,
    /// Looping from start to the end of the sound while the note is held.
    Loop,
}

impl PlayMode {
    /// From a parameter value (0, 1, 2).
    pub fn from_index(v: f32) -> PlayMode {
        match v.round() as i32 {
            1 => PlayMode::OneShot,
            2 => PlayMode::Loop,
            _ => PlayMode::Gate,
        }
    }
}

/// Longest loop crossfade, in frames of the sound.
const XFADE: f64 = 1024.0;

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SVoice {
    active: bool,
    /// Samples of gate left; negative once released.
    remaining: i64,
    pitch: f32,
    vel: f32,
    age: u64,
    pos: f64,
    step: f64,
    env: Adsr,
}

/// The sampler's state inside an instrument node.
#[derive(Debug, Clone)]
pub(crate) struct Sampler {
    pub(crate) sample: Option<Arc<Sample>>,
    pub(crate) params: [f32; MAX_PARAMS],
    pub(crate) voices: Box<[SVoice; MAX_VOICES]>,
}

impl Sampler {
    pub(crate) fn new(sample: Option<Arc<Sample>>, params: [f32; MAX_PARAMS]) -> Sampler {
        Sampler { sample, params, voices: Box::new([SVoice::default(); MAX_VOICES]) }
    }

    fn start_frame(&self) -> f64 {
        let n = self.sample.as_ref().map_or(0, |s| s.frames());
        (self.params[3].clamp(0.0, 1.0) as f64 * n as f64).floor().min(n.saturating_sub(1) as f64)
    }

    /// Playback speed of a voice at `pitch` played at `sr`.
    fn step(&self, pitch: f32, sr: f32) -> f64 {
        let rate = self.sample.as_ref().map_or(sr, |s| s.rate);
        let semis = pitch - self.params[1] + self.params[2];
        2f64.powf(semis as f64 / 12.0) * rate as f64 / sr as f64
    }

    pub(crate) fn same_sound(&self, other: &Sampler) -> bool {
        match (&self.sample, &other.sample) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        }
    }

    pub(crate) fn release_all(&mut self) {
        for v in self.voices.iter_mut().filter(|v| v.active && v.remaining >= 0) {
            v.remaining = -1;
        }
    }

    /// Releases the live-held voices at `pitch`.
    pub(crate) fn release_live(&mut self, pitch: u8) {
        for v in self.voices.iter_mut() {
            if v.active && v.pitch == pitch as f32 && v.remaining > crate::LIVE_HOLD / 2 {
                v.remaining = -1;
            }
        }
    }

    pub(crate) fn start(&mut self, pitch: u8, vel: f32, len: i64, clock: u64, sr: f32) {
        if self.sample.as_ref().is_none_or(|s| s.frames() == 0) {
            return;
        }
        let slot = (0..MAX_VOICES)
            .find(|&i| !self.voices[i].active)
            .unwrap_or_else(|| (0..MAX_VOICES).min_by_key(|&i| self.voices[i].age).unwrap_or(0));
        let step = self.step(pitch as f32, sr);
        let pos = self.start_frame();
        self.voices[slot] = SVoice {
            active: true,
            remaining: len.max(1),
            pitch: pitch as f32,
            vel,
            age: clock,
            pos,
            step,
            env: Adsr::default(),
        };
    }

    /// Recomputes every voice's speed (tune and root change live).
    pub(crate) fn refresh(&mut self, sr: f32) {
        for i in 0..MAX_VOICES {
            if self.voices[i].active {
                self.voices[i].step = self.step(self.voices[i].pitch, sr);
            }
        }
    }

    /// One sample of every voice; returns the stereo sum.
    pub(crate) fn next(&mut self, sr: f32) -> (f32, f32) {
        let Some(s) = &self.sample else { return (0.0, 0.0) };
        let n = s.frames() as f64;
        let p = &self.params;
        let mode = PlayMode::from_index(p[4]);
        let level = db_gain(p[0]);
        let sens = p[7].clamp(0.0, 1.0);
        let ap = AdsrParams { a: p[5].max(0.0005), d: 0.001, s: 1.0, r: p[6].max(0.002) };
        let ls = (p[8].clamp(0.0, 1.0) as f64 * n).floor().min(n - 1.0);
        let xf = XFADE.min(((n - ls) / 4.0).floor()).max(0.0);
        let (mut l, mut r) = (0.0, 0.0);
        for v in self.voices.iter_mut().filter(|v| v.active) {
            let held = v.remaining >= 0;
            if held {
                v.remaining -= 1;
            }
            let gate = match mode {
                PlayMode::OneShot => v.pos < n,
                _ => held,
            };
            let e = v.env.next(gate, &ap, sr);
            let (mut a, mut b) = s.at(v.pos);
            if mode == PlayMode::Loop && xf > 0.0 && v.pos >= n - xf {
                // crossfade the end into the frames just after the loop start
                let t = ((v.pos - (n - xf)) / xf) as f32;
                let (c, d) = s.at(ls + (v.pos - (n - xf)));
                a = a * (1.0 - t) + c * t;
                b = b * (1.0 - t) + d * t;
            }
            let g = e * level * (1.0 - sens * (1.0 - v.vel));
            l += a * g;
            r += b * g;
            v.pos += v.step;
            if mode == PlayMode::Loop && held && v.pos >= n {
                v.pos = ls + xf + (v.pos - n);
            }
            if v.pos >= n || (v.env.done() && !gate) {
                v.active = false;
            }
        }
        (l, r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Buf, Instrument, NoteEvent};

    /// One second of a 100 Hz sine at 48 kHz.
    fn sine() -> Arc<Sample> {
        let d: Vec<f32> = (0..48_000).map(|i| (i as f32 * 100.0 / 48_000.0 * std::f32::consts::TAU).sin()).collect();
        Arc::new(Sample::from_interleaved(48_000.0, 1, &d))
    }

    fn params(mode: f32) -> [f32; MAX_PARAMS] {
        [0.0, 60.0, 0.0, 0.0, mode, 0.001, 0.01, 0.0, 0.0, 0.0]
    }

    fn render(i: &mut Instrument, frames: usize) -> Vec<f32> {
        let mut b = Buf { l: vec![0.0; 256], r: vec![0.0; 256] };
        let mut v = Vec::new();
        let mut pos = 0;
        while v.len() < frames {
            b.l.fill(0.0);
            b.r.fill(0.0);
            i.process(&mut b, 0, 256, Some(pos), 48_000.0);
            v.extend_from_slice(&b.l);
            pos += 256;
        }
        v
    }

    fn crossings(v: &[f32]) -> usize {
        v.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count()
    }

    #[test]
    fn notes_repitch_from_the_root_and_gate_mode_stops_on_release() {
        let ev = vec![NoteEvent { at: 0, len: 24_000, pitch: 72, velocity: 1.0 }];
        let mut i = Instrument::sampler(ev, Some(sine()), params(0.0));
        let v = render(&mut i, 48_000);
        // an octave up plays the sine at 200 Hz, twice as fast
        assert!((40..=43).contains(&crossings(&v[1000..11_000])), "{}", crossings(&v[1000..11_000]));
        assert!(v[25_000..].iter().all(|x| x.abs() < 1e-4), "released");
    }

    #[test]
    fn one_shots_play_out_and_loops_hold() {
        let short = vec![NoteEvent { at: 0, len: 100, pitch: 60, velocity: 1.0 }];
        let mut one = Instrument::sampler(short, Some(sine()), params(1.0));
        let v = render(&mut one, 60_000);
        assert!(v[40_000..47_000].iter().any(|x| x.abs() > 0.5), "plays to the end");
        assert!(v[48_100..].iter().all(|x| *x == 0.0), "and stops there");

        let mut p = params(2.0);
        p[3] = 0.1; // start a little in
        p[8] = 0.503; // and loop the second half, from a point that does not line up with the end
        let long = vec![NoteEvent { at: 0, len: 96_000, pitch: 60, velocity: 1.0 }];
        let mut lp = Instrument::sampler(long, Some(sine()), p);
        let v = render(&mut lp, 100_000);
        assert!(v[80_000..90_000].iter().any(|x| x.abs() > 0.5), "still sounding past the sound's end");
        let jump = v[1..96_000].iter().zip(&v[..95_999]).fold(0.0f32, |a, (x, y)| a.max((x - y).abs()));
        assert!(jump < 0.05, "loops without clicks: {jump}");
        assert!(v[97_000..].iter().all(|x| x.abs() < 1e-4));
    }

    #[test]
    fn velocity_and_live_params() {
        let ev = vec![NoteEvent { at: 0, len: 4_800, pitch: 60, velocity: 0.5 }];
        let mut p = params(0.0);
        p[7] = 1.0;
        let mut i = Instrument::sampler(ev, Some(sine()), p);
        let v = render(&mut i, 2_400);
        let peak = v[500..].iter().fold(0.0f32, |a, x| a.max(x.abs()));
        assert!((0.45..0.55).contains(&peak), "half velocity is half as loud: {peak}");
        assert!(i.set_param(0, 0, -120.0));
        assert!(!i.set_param(1, 0, 0.0));
        let v = render(&mut i, 1_024);
        assert!(v.iter().all(|x| x.abs() < 1e-4));
        // no sound loaded: silent, no panic
        let mut none = Instrument::sampler(vec![NoteEvent { at: 0, len: 10, pitch: 60, velocity: 1.0 }], None, p);
        assert!(render(&mut none, 512).iter().all(|x| *x == 0.0));
    }
}
