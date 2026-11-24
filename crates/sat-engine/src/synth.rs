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
    events: Vec<NoteEvent>,
    cursor: usize,
    /// The timeline sample the next block is expected to start at; anything
    /// else is a jump (locate, loop) and reseeks the cursor.
    expect: Option<i64>,
    voices: [Voice; VOICES],
    clock: u64,
    level: f32,
    attack_s: f32,
    release_s: f32,
    wave: Wave,
}

fn pitch_to_hz(p: u8) -> f32 {
    440.0 * 2f32.powf((p as f32 - 69.0) / 12.0)
}

impl Synth {
    /// A synth that plays `events` (sorted here by start).
    pub fn new(mut events: Vec<NoteEvent>, wave: Wave, level: f32) -> Synth {
        events.sort_by(|a, b| a.at.cmp(&b.at).then(a.pitch.cmp(&b.pitch)));
        Synth {
            events,
            cursor: 0,
            expect: None,
            voices: [Voice::default(); VOICES],
            clock: 0,
            level,
            attack_s: 0.002,
            release_s: 0.05,
            wave,
        }
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
        // the event list may have changed: reseek on the next block
        self.expect = None;
    }

    pub(crate) fn release_all(&mut self) {
        for v in &mut self.voices {
            if v.active && v.remaining >= 0 {
                v.remaining = -1;
            }
        }
        self.expect = None;
    }

    fn start(&mut self, e: &NoteEvent, sr: f64) {
        self.clock += 1;
        let slot = match self.voices.iter().position(|v| !v.active) {
            Some(i) => i,
            None => (0..VOICES).min_by_key(|&i| self.voices[i].age).unwrap_or(0),
        };
        self.voices[slot] = Voice {
            active: true,
            phase: 0.0,
            inc: pitch_to_hz(e.pitch) / sr as f32,
            remaining: e.len.max(1),
            env: 0.0,
            vel: e.velocity,
            age: self.clock,
        };
    }

    pub(crate) fn process(&mut self, out: &mut crate::Buf, off: usize, len: usize, pos: Option<i64>, sr: f64) {
        if let Some(t0) = pos
            && self.expect != Some(t0)
        {
            if self.expect.is_some() {
                // a jump: cut what was playing
                self.release_all();
            }
            self.cursor = self.events.partition_point(|e| e.at < t0);
        }
        self.expect = pos.map(|p| p + len as i64);
        let att = 1.0 / (self.attack_s * sr as f32).max(1.0);
        let rel = 1.0 / (self.release_s * sr as f32).max(1.0);
        for k in 0..len {
            if let Some(t0) = pos {
                let t = t0 + k as i64;
                while let Some(e) = self.events.get(self.cursor).copied() {
                    if e.at > t {
                        break;
                    }
                    if e.at == t {
                        self.start(&e, sr);
                    }
                    self.cursor += 1;
                }
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
