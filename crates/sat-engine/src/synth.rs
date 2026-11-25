//! A small built-in polyphonic synth, so the engine can make sound before
//! the instrument grid exists. Deterministic: same events, same samples.

use core::f32::consts::TAU;

/// A note to play, already placed on the timeline in samples.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NoteEvent {
    /// Timeline sample where it starts.
    pub at: i64,
    /// Length in samples.
    pub len: i64,
    /// MIDI pitch.
    pub pitch: u8,
    /// Velocity 0–1.
    pub velocity: f32,
}

/// Oscillator shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wave {
    /// Sine.
    Sine,
    /// Naive saw.
    Saw,
}

/// Synth parameters reachable by commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SynthParam {
    /// Output level, linear.
    Level,
    /// Attack time in seconds.
    Attack,
    /// Release time in seconds.
    Release,
    /// 0 = sine, 1 = saw.
    Wave,
}

const VOICES: usize = 16;

#[derive(Debug, Clone, Copy, Default)]
struct Voice {
    active: bool,
    phase: f32,
    inc: f32,
    /// Samples until release starts; negative once releasing.
    remaining: i64,
    env: f32,
    vel: f32,
    age: u64,
}

/// The synth node.
#[derive(Debug, Clone)]
pub struct Synth {
    seq: crate::sequencer::Sequencer,
    voices: [Voice; VOICES],
    clock: u64,
    level: f32,
    attack_s: f32,
    release_s: f32,
    wave: Wave,
}

fn release(voices: &mut [Voice; VOICES]) {
    for v in voices.iter_mut().filter(|v| v.active && v.remaining >= 0) {
        v.remaining = -1;
    }
}

fn start(voices: &mut [Voice; VOICES], clock: &mut u64, e: &NoteEvent, sr: f64) {
    *clock += 1;
    let slot = match voices.iter().position(|v| !v.active) {
        Some(i) => i,
        None => (0..VOICES).min_by_key(|&i| voices[i].age).unwrap_or(0),
    };
    voices[slot] = Voice {
        active: true,
        phase: 0.0,
        inc: pitch_to_hz(e.pitch) / sr as f32,
        remaining: e.len.max(1),
        env: 0.0,
        vel: e.velocity,
        age: *clock,
    };
}

fn pitch_to_hz(p: u8) -> f32 {
    440.0 * 2f32.powf((p as f32 - 69.0) / 12.0)
}

impl Synth {
    /// A synth that plays `events` (sorted here by start).
    pub fn new(events: Vec<NoteEvent>, wave: Wave, level: f32) -> Synth {
        Synth {
            seq: crate::sequencer::Sequencer::new(events),
            voices: [Voice::default(); VOICES],
            clock: 0,
            level,
            attack_s: 0.002,
            release_s: 0.05,
            wave,
        }
    }

    /// The same synth with session clips it can launch.
    pub fn with_clips(mut self, clips: Vec<crate::SessionClip>) -> Synth {
        self.seq.set_clips(clips);
        self
    }

    pub(crate) fn seq_mut(&mut self) -> &mut crate::sequencer::Sequencer {
        &mut self.seq
    }

    /// Sets a parameter directly (off the audio thread, before the synth runs).
    pub fn set_param(&mut self, p: SynthParam, v: f32) {
        match p {
            SynthParam::Level => self.level = v,
            SynthParam::Attack => self.attack_s = v.max(0.0),
            SynthParam::Release => self.release_s = v.max(0.0),
            SynthParam::Wave => self.wave = if v >= 0.5 { Wave::Saw } else { Wave::Sine },
        }
    }

    pub(crate) fn adopt(&mut self, old: &Synth) {
        self.voices = old.voices;
        self.clock = old.clock;
        self.seq.adopt(&old.seq);
    }

    pub(crate) fn release_all(&mut self) {
        for v in &mut self.voices {
            if v.active && v.remaining >= 0 {
                v.remaining = -1;
            }
        }
        self.seq.invalidate();
    }

    pub(crate) fn process(&mut self, out: &mut crate::Buf, off: usize, len: usize, pos: Option<i64>, sr: f64) {
        if self.seq.begin(pos, len) {
            // a jump: cut what was playing
            self.release_all();
        }
        let att = 1.0 / (self.attack_s * sr as f32).max(1.0);
        let rel = 1.0 / (self.release_s * sr as f32).max(1.0);
        for k in 0..len {
            if let Some(t0) = pos {
                let t = t0 + k as i64;
                let (voices, clock) = (&mut self.voices, &mut self.clock);
                self.seq.step(t, |c| match c {
                    crate::sequencer::Cue::Cut => release(voices),
                    crate::sequencer::Cue::Start(e) => start(voices, clock, e, sr),
                });
            }
            let mut s = 0.0;
            for v in self.voices.iter_mut().filter(|v| v.active) {
                if v.remaining >= 0 {
                    v.env = (v.env + att).min(1.0);
                    v.remaining -= 1;
                } else {
                    v.env -= rel;
                    if v.env <= 0.0 {
                        v.active = false;
                        continue;
                    }
                }
                let osc = match self.wave {
                    Wave::Sine => (v.phase * TAU).sin(),
                    Wave::Saw => 2.0 * v.phase - 1.0,
                };
                v.phase += v.inc;
                if v.phase >= 1.0 {
                    v.phase -= 1.0;
                }
                s += osc * v.env * v.vel;
            }
            let i = off + k;
            out.l[i] += s * self.level;
            out.r[i] += s * self.level;
        }
    }
}
