//! Audio clip playback: a track's audio as segments on the timeline.
//!
//! The compiler turns clips, comps and fades into [`AudioSeg`]s: a piece of
//! one sound placed at a timeline sample, with its own fade in and out.
//! Crossfades are two segments overlapping with opposite fades, both
//! equal-power, so a comp boundary keeps its loudness. Playback is
//! stateless: any block at any position renders the same samples.

use std::sync::Arc;

use crate::sampler::Sample;

/// A piece of a sound on the timeline.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioSeg {
    /// Timeline sample where it starts.
    pub at: i64,
    /// Length in timeline samples.
    pub len: i64,
    /// The sound.
    pub sound: Arc<Sample>,
    /// Frame of the sound heard at `at`.
    pub src: f64,
    /// Fade in, in samples from `at`.
    pub fade_in: i64,
    /// Fade out, in samples before the end.
    pub fade_out: i64,
    /// Linear gain.
    pub gain: f32,
}

/// The node playing a track's audio segments.
#[derive(Debug, Clone)]
pub struct AudioPlayer {
    segs: Vec<AudioSeg>,
    /// The timeline sample the next block would start at while playing, and
    /// how far a stop's fade-out has got.
    stopping: Option<(i64, usize)>,
}

/// Seconds a sound fades out over when the transport stops in it.
const STOP_FADE_S: f64 = 0.005;

/// Equal-power fade gain for `x` from 0 (silent) to 1 (full).
fn fade(x: f64) -> f32 {
    (x.clamp(0.0, 1.0) * core::f64::consts::FRAC_PI_2).sin() as f32
}

impl AudioPlayer {
    /// A player for `segs` (sorted here by start).
    pub fn new(mut segs: Vec<AudioSeg>) -> AudioPlayer {
        segs.retain(|s| s.len > 0);
        segs.sort_by_key(|s| s.at);
        AudioPlayer { segs, stopping: None }
    }

    /// The segments, by start.
    pub fn segments(&self) -> &[AudioSeg] {
        &self.segs
    }

    pub(crate) fn process(&mut self, out: &mut crate::Buf, off: usize, len: usize, pos: Option<i64>, sr: f64) {
        match pos {
            Some(t0) => {
                self.add(out, off, t0, len, sr, |_| 1.0);
                self.stopping = Some((t0 + len as i64, 0));
            }
            // stopped mid-sound: let it fade rather than click
            None => {
                let Some((t0, done)) = self.stopping else { return };
                let total = ((STOP_FADE_S * sr) as usize).max(1);
                let n = len.min(total - done);
                self.add(out, off, t0, n, sr, |k| 1.0 - (done + k) as f32 / total as f32);
                self.stopping = (done + n < total).then_some((t0 + n as i64, done + n));
            }
        }
    }

    /// Adds `len` frames from timeline sample `t0`, scaled by `env` (by
    /// frame of the block).
    fn add(&self, out: &mut crate::Buf, off: usize, t0: i64, len: usize, sr: f64, env: impl Fn(usize) -> f32) {
        let t1 = t0 + len as i64;
        // segments starting after this block are all later in the list
        let end = self.segs.partition_point(|s| s.at < t1);
        for s in &self.segs[..end] {
            let (a, b) = (t0.max(s.at), t1.min(s.at + s.len));
            if a >= b {
                continue;
            }
            let ratio = s.sound.rate as f64 / sr;
            for t in a..b {
                let rel = t - s.at;
                let mut g = s.gain;
                if rel < s.fade_in {
                    g *= fade((rel as f64 + 0.5) / s.fade_in as f64);
                }
                let left = s.len - rel;
                if left <= s.fade_out {
                    g *= fade((left as f64 - 0.5) / s.fade_out as f64);
                }
                let (l, r) = s.sound.at(s.src + rel as f64 * ratio);
                let i = off + (t - t0) as usize;
                let g = g * env((t - t0) as usize);
                out.l[i] += l * g;
                out.r[i] += r * g;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Buf;

    fn dc(v: f32, frames: usize) -> Arc<Sample> {
        Arc::new(Sample { rate: 48_000.0, l: vec![v; frames], r: vec![v; frames] })
    }

    fn render(p: &mut AudioPlayer, from: i64, frames: usize) -> Vec<f32> {
        let mut out = Vec::new();
        let mut b = Buf { l: vec![0.0; 64], r: vec![0.0; 64] };
        let mut t = from;
        while out.len() < frames {
            b.l.fill(0.0);
            b.r.fill(0.0);
            p.process(&mut b, 0, 64, Some(t), 48_000.0);
            out.extend_from_slice(&b.l);
            t += 64;
        }
        out
    }

    #[test]
    fn segments_play_in_place_with_fades_and_crossfades() {
        let seg = |at, len, v, fi, fo| AudioSeg {
            at,
            len,
            sound: dc(v, 10_000),
            src: 0.0,
            fade_in: fi,
            fade_out: fo,
            gain: 1.0,
        };
        // two takes of the same level crossfaded over 200 samples: no dip in power
        let mut p = AudioPlayer::new(vec![seg(1100, 1000, 0.5, 200, 0), seg(100, 1200, 0.5, 0, 200)]);
        let v = render(&mut p, 0, 2_400);
        assert!(v[..100].iter().all(|x| *x == 0.0));
        assert_eq!(v[500], 0.5);
        for x in &v[1100..1300] {
            // equal power: amplitudes sum above, never below, the level
            assert!((0.5..=0.71).contains(x), "{x}");
        }
        assert_eq!(v[1500], 0.5);
        assert!(v[2100..].iter().all(|x| *x == 0.0));
        // stateless: the same samples from any start
        assert_eq!(render(&mut p, 1024, 512), v[1024..1536].to_vec());
    }

    #[test]
    fn sources_play_from_their_offset_at_their_own_rate() {
        let ramp: Vec<f32> = (0..1000).map(|i| i as f32).collect();
        let sound = Arc::new(Sample { rate: 24_000.0, l: ramp.clone(), r: ramp });
        let mut p =
            AudioPlayer::new(vec![AudioSeg { at: 0, len: 100, sound, src: 10.0, fade_in: 0, fade_out: 0, gain: 1.0 }]);
        let v = render(&mut p, 0, 128);
        // at half the engine rate the sound moves half a frame per sample
        assert_eq!(v[0], 10.0);
        assert_eq!(v[40], 30.0);
        assert_eq!(v[100], 0.0);
    }

    #[test]
    fn stopping_mid_sound_fades_out_instead_of_cutting() {
        let seg = AudioSeg { at: 0, len: 48_000, sound: dc(1.0, 48_000), src: 0.0, fade_in: 0, fade_out: 0, gain: 1.0 };
        let mut p = AudioPlayer::new(vec![seg]);
        assert_eq!(render(&mut p, 0, 640)[639], 1.0);
        let mut out = Vec::new();
        let mut b = Buf { l: vec![0.0; 64], r: vec![0.0; 64] };
        for _ in 0..8 {
            b.l.fill(0.0);
            p.process(&mut b, 0, 64, None, 48_000.0);
            out.extend_from_slice(&b.l);
        }
        // 5 ms at 48 kHz: 240 samples down to silence, no step bigger than a ramp's
        assert!(out[0] > 0.99);
        assert!(out.windows(2).all(|w| (w[0] - w[1]).abs() < 0.01));
        assert!(out[240..].iter().all(|v| *v == 0.0));
    }
}
