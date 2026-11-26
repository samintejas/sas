//! Instrument nodes: a voice patch that every voice runs, a drum synth, or
//! a sampler.
//!
//! A [`Patch`] is a list of modules in dependency order. Each module reads
//! its inputs from, and writes its outputs to, a small per-voice register
//! file, so running a patch is a loop over modules per voice per sample with
//! no allocation. Unconnected inputs fall back to sensible defaults: an
//! oscillator's pitch follows the voice, a vca's cv is 1, an envelope's gate
//! is the voice's gate.
//!
//! Control signals are plain numbers: pitch in MIDI notes, gates and
//! envelopes 0–1, lfos and random -1–1 (scaled by depth). A signal is "high"
//! (for gates, triggers and logic) while it is above 0.

use sat_dsp::drums::{self, Kit};
use sat_dsp::{Adsr, AdsrParams, Mode, Noise, Osc, Shape, Svf, db_gain, midi_hz, pan_gains, scale};

use std::sync::Arc;

use crate::sampler::{Sample, Sampler};
use crate::sequencer::{Cue, Sequencer, SessionClip};
use crate::synth::NoteEvent;

/// Most modules in a patch.
pub const MAX_MODS: usize = 32;
/// Most registers (connected outputs) in a patch.
pub const MAX_REGS: usize = 64;
/// Most parameters per module.
pub const MAX_PARAMS: usize = 10;
/// Most voices.
pub const MAX_VOICES: usize = 16;
/// An unconnected input.
pub const NONE: u8 = u8::MAX;

/// What a patch module does. Inputs, outputs and parameters are listed in
/// the order they appear in [`VModule`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VKind {
    /// Outputs pitch (MIDI, fractional), gate (0/1), velocity. Params: voices, transpose.
    NoteIn,
    /// In: pitch, fm. Out: audio. Params: wave, tune (st), fine (cents), fm amount, level.
    Vco,
    /// Out: audio. Params: level.
    Noise,
    /// In: 1, 2, 3. Out: audio. Params: gain 1, 2, 3 (dB).
    Mixer,
    /// In: audio, cutoff mod. Out: audio. Params: mode, cutoff (Hz), res, mod amount (±1 = ±5 oct), key tracking.
    Filter,
    /// In: gate. Out: env. Params: a, d, s, r.
    Adsr,
    /// In: audio, cv. Out: audio. Params: level (dB).
    Vca,
    /// In: rate mod (±1 = ±4 oct), sync (restarts on a rising edge). Out:
    /// -1–1 scaled by depth, 0–1 scaled by depth. Params: rate (Hz), wave, depth.
    Lfo,
    /// In: audio. Params: level (dB), pan.
    VoiceOut,
    /// In: 1, 2, 3. Out: their sum. Params: gain 1, 2, 3 (-1–1), offset.
    CvMix,
    /// In: signal, cv (default 1). Out: signal × amount × cv + offset.
    /// Params: amount (-1–1), offset.
    Atten,
    /// In: pitch (default the voice's), cv. Out: pitch snapped to a scale, a
    /// trigger whenever the note changes. Params: root, scale, transpose (st),
    /// cv range (st).
    Quantize,
    /// In: signal (default noise), trigger (default the gate). Out: the
    /// signal as it was at the last rising edge.
    SampleHold,
    /// In: signal. Out: the signal, smoothed. Params: rise (s), fall (s).
    Slew,
    /// Out: a new random value `rate` times a second (stepped and smoothly
    /// joined). Params: rate (Hz), depth.
    Random,
    /// In: a, b (default 0). Out: 1 while a > b + threshold, its inverse.
    /// Params: threshold.
    Compare,
    /// In: a, b. Out: and, or, xor.
    Logic,
    /// In: clock, reset. Out: high with every `div`th clock pulse. Params: div.
    ClockDiv,
    /// In: audio, carrier (audio). Out: audio × carrier, blended. Params: mix.
    Ring,
    /// In: audio, fold mod. Out: folded audio. Params: fold, bias, mod amount.
    Fold,
}

/// One module of a patch.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VModule {
    /// What it does.
    pub kind: VKind,
    /// Input registers ([`NONE`] if unconnected).
    pub ins: [u8; 4],
    /// Output registers ([`NONE`] if unused).
    pub outs: [u8; 3],
    /// Parameter values.
    pub params: [f32; MAX_PARAMS],
}

impl VModule {
    /// A module with no connections and all parameters zero.
    pub fn new(kind: VKind) -> VModule {
        VModule { kind, ins: [NONE; 4], outs: [NONE; 3], params: [0.0; MAX_PARAMS] }
    }
}

/// Per-voice state of one module.
#[derive(Debug, Clone, Copy, Default)]
struct ModState {
    osc: Osc,
    svf: Svf,
    adsr: Adsr,
    noise: Noise,
    /// Held or smoothed values.
    a: f32,
    b: f32,
    /// A counter (clock div pulses, trigger samples left, started flag).
    n: u32,
    /// The last input was high (for rising edges).
    hi: bool,
    hi2: bool,
}

/// Is a control signal high?
fn high(x: f32) -> bool {
    x > 0.0
}

/// One-pole smoothing coefficient that gets about 99% of the way in `secs`.
fn smooth_coef(secs: f32, sr: f32) -> f32 {
    (-4.6 / (secs.max(1e-5) * sr)).exp()
}

#[derive(Debug, Clone, Copy)]
struct PVoice {
    active: bool,
    /// Samples of gate left; negative once released.
    remaining: i64,
    pitch: f32,
    vel: f32,
    age: u64,
    /// Declick fade for patches without an envelope, 0–1.
    fade: f32,
    regs: [f32; MAX_REGS],
    st: [ModState; MAX_MODS],
}

impl Default for PVoice {
    fn default() -> Self {
        PVoice {
            active: false,
            remaining: 0,
            pitch: 60.0,
            vel: 0.0,
            age: 0,
            fade: 0.0,
            regs: [0.0; MAX_REGS],
            st: [ModState::default(); MAX_MODS],
        }
    }
}

/// A voice patch.
#[derive(Debug, Clone, PartialEq)]
pub struct Patch {
    /// Modules in dependency order (at most [`MAX_MODS`]).
    pub mods: Vec<VModule>,
}

impl Patch {
    fn voices(&self) -> usize {
        self.mods
            .iter()
            .find(|m| m.kind == VKind::NoteIn)
            .map_or(8, |m| m.params[0].round() as usize)
            .clamp(1, MAX_VOICES)
    }

    fn transpose(&self) -> f32 {
        self.mods.iter().find(|m| m.kind == VKind::NoteIn).map_or(0.0, |m| m.params[1])
    }

    fn has_env(&self) -> bool {
        self.mods.iter().any(|m| m.kind == VKind::Adsr)
    }
}

#[derive(Debug, Clone)]
enum Engine {
    Patch {
        patch: Patch,
        voices: Box<[PVoice; MAX_VOICES]>,
    },
    Drums {
        kit: Kit,
        level: f32,
        voices: Box<[drums::Voice; MAX_VOICES]>,
        on: [bool; MAX_VOICES],
        ages: [u64; MAX_VOICES],
    },
    Sampler(Sampler),
}

/// An instrument node: plays note events through a patch or the drum synth.
#[derive(Debug, Clone)]
pub struct Instrument {
    seq: Sequencer,
    clock: u64,
    engine: Engine,
}

fn reg(r: &[f32; MAX_REGS], i: u8, default: f32) -> f32 {
    if i == NONE { default } else { r[i as usize] }
}

impl Instrument {
    fn with(events: Vec<NoteEvent>, engine: Engine) -> Instrument {
        Instrument { seq: Sequencer::new(events), clock: 0, engine }
    }

    /// The same instrument with session clips it can launch.
    pub fn with_clips(mut self, clips: Vec<SessionClip>) -> Instrument {
        self.seq.set_clips(clips);
        self
    }

    pub(crate) fn seq_mut(&mut self) -> &mut Sequencer {
        &mut self.seq
    }

    /// The key of the session clip playing, if any.
    pub fn playing_clip(&self) -> Option<u64> {
        self.seq.playing()
    }

    /// An instrument running `patch` (truncated to [`MAX_MODS`] modules;
    /// registers past [`MAX_REGS`] are treated as unconnected).
    pub fn patch(events: Vec<NoteEvent>, mut patch: Patch) -> Instrument {
        patch.mods.truncate(MAX_MODS);
        for m in &mut patch.mods {
            for r in m.ins.iter_mut().chain(m.outs.iter_mut()) {
                if *r != NONE && *r as usize >= MAX_REGS {
                    *r = NONE;
                }
            }
        }
        Instrument::with(events, Engine::Patch { patch, voices: Box::new([PVoice::default(); MAX_VOICES]) })
    }

    /// The drum synth, with `level` in dB, `tune` in semitones and `decay` as a length multiplier.
    pub fn drums(events: Vec<NoteEvent>, level_db: f32, tune: f32, decay: f32) -> Instrument {
        Instrument::with(
            events,
            Engine::Drums {
                kit: Kit { tune, decay },
                level: db_gain(level_db),
                voices: Box::new([drums::Voice::default(); MAX_VOICES]),
                on: [false; MAX_VOICES],
                ages: [0; MAX_VOICES],
            },
        )
    }

    /// The sampler playing `sample` (silent without one). Parameters: level
    /// (dB), root note, tune (semitones), start (0–1), mode ([`crate::PlayMode`]
    /// index), attack (s), release (s), velocity sensitivity (0–1), loop start (0–1).
    pub fn sampler(events: Vec<NoteEvent>, sample: Option<Arc<Sample>>, params: [f32; MAX_PARAMS]) -> Instrument {
        Instrument::with(events, Engine::Sampler(Sampler::new(sample, params)))
    }

    /// Sets parameter `index` of module `slot` (the drum synth and the
    /// sampler are slot 0; the drum synth's parameters are level dB, tune,
    /// decay). Returns false if there is no such parameter.
    pub fn set_param(&mut self, slot: u8, index: u8, v: f32) -> bool {
        match &mut self.engine {
            Engine::Patch { patch, .. } => match patch.mods.get_mut(slot as usize) {
                Some(m) if (index as usize) < MAX_PARAMS => {
                    m.params[index as usize] = v;
                    true
                }
                _ => false,
            },
            Engine::Drums { kit, level, .. } => match (slot, index) {
                (0, 0) => {
                    *level = db_gain(v);
                    true
                }
                (0, 1) => {
                    kit.tune = v;
                    true
                }
                (0, 2) => {
                    kit.decay = v;
                    true
                }
                _ => false,
            },
            Engine::Sampler(s) => match (slot, index as usize) {
                (0, i) if i < MAX_PARAMS => {
                    s.params[i] = v;
                    true
                }
                _ => false,
            },
        }
    }

    pub(crate) fn adopt(&mut self, old: &Instrument) {
        self.clock = old.clock;
        self.seq.adopt(&old.seq);
        match (&mut self.engine, &old.engine) {
            // voices carry over only if the patch kept its shape
            (Engine::Patch { patch, voices }, Engine::Patch { patch: op, voices: ov })
                if patch.mods.len() == op.mods.len()
                    && patch.mods.iter().zip(&op.mods).all(|(a, b)| a.kind == b.kind && a.outs == b.outs) =>
            {
                **voices = **ov;
            }
            (Engine::Drums { voices, on, ages, .. }, Engine::Drums { voices: v2, on: o2, ages: a2, .. }) => {
                **voices = **v2;
                *on = *o2;
                *ages = *a2;
            }
            (Engine::Sampler(s), Engine::Sampler(o)) if s.same_sound(o) => {
                *s.voices = *o.voices;
            }
            _ => {}
        }
    }

    pub(crate) fn release_all(&mut self) {
        release(&mut self.engine);
        self.seq.invalidate();
    }

    /// Starts (`on`) or releases a note played live, held until released.
    pub(crate) fn live(&mut self, pitch: u8, velocity: f32, on: bool, sr: f64) {
        if on {
            let e = NoteEvent { at: 0, len: crate::LIVE_HOLD, pitch, velocity };
            start(&mut self.engine, &mut self.clock, &e, sr as f32);
            return;
        }
        match &mut self.engine {
            Engine::Patch { patch, voices } => {
                let p = pitch as f32 + patch.transpose();
                for v in voices.iter_mut() {
                    if v.active && v.pitch == p && v.remaining > crate::LIVE_HOLD / 2 {
                        v.remaining = -1;
                    }
                }
            }
            Engine::Sampler(s) => s.release_live(pitch),
            Engine::Drums { .. } => {}
        }
    }
}

/// Releases every held note.
fn release(engine: &mut Engine) {
    match engine {
        Engine::Patch { voices, .. } => {
            for v in voices.iter_mut().filter(|v| v.active && v.remaining >= 0) {
                v.remaining = -1;
            }
        }
        Engine::Sampler(s) => s.release_all(),
        Engine::Drums { .. } => {}
    }
}

/// Starts a note on a free (or the oldest) voice.
fn start(engine: &mut Engine, clock: &mut u64, e: &NoteEvent, sr: f32) {
    *clock += 1;
    let clock = *clock;
    match engine {
        Engine::Patch { patch, voices } => {
            let n = patch.voices();
            let slot = (0..n)
                .find(|&i| !voices[i].active)
                .unwrap_or_else(|| (0..n).min_by_key(|&i| voices[i].age).unwrap_or(0));
            let v = &mut voices[slot];
            let fresh = !v.active;
            v.active = true;
            v.remaining = e.len.max(1);
            v.pitch = e.pitch as f32 + patch.transpose();
            v.vel = e.velocity;
            v.age = clock;
            v.fade = if fresh { 0.0 } else { v.fade };
            if fresh {
                v.regs = [0.0; MAX_REGS];
                for (i, s) in v.st.iter_mut().enumerate() {
                    *s = ModState {
                        noise: Noise::new((clock as u32).wrapping_mul(31).wrapping_add(i as u32)),
                        ..ModState::default()
                    };
                }
            }
        }
        Engine::Drums { voices, on, ages, .. } => {
            let kind = drums::Kind::of(e.pitch);
            // a closed hat chokes an open one
            if kind == drums::Kind::ClosedHat {
                for i in 0..MAX_VOICES {
                    if on[i] && voices[i].kind == drums::Kind::OpenHat {
                        on[i] = false;
                    }
                }
            }
            let slot = (0..MAX_VOICES)
                .find(|&i| !on[i])
                .unwrap_or_else(|| (0..MAX_VOICES).min_by_key(|&i| ages[i]).unwrap_or(0));
            voices[slot] = drums::Voice::new(e.pitch, e.velocity, clock as u32);
            on[slot] = true;
            ages[slot] = clock;
        }
        Engine::Sampler(s) => s.start(e.pitch, e.velocity, e.len, clock, sr),
    }
}

impl Instrument {
    pub(crate) fn process(&mut self, out: &mut crate::Buf, off: usize, len: usize, pos: Option<i64>, sr: f64) {
        if self.seq.begin(pos, len) {
            self.release_all();
        }
        let sr = sr as f32;
        if let Engine::Sampler(s) = &mut self.engine {
            s.refresh(sr);
        }
        for k in 0..len {
            if let Some(t0) = pos {
                let t = t0 + k as i64;
                let Instrument { seq, clock, engine } = self;
                seq.step(t, |c| match c {
                    Cue::Cut => release(engine),
                    Cue::Start(e) => start(engine, clock, e, sr),
                });
            }
            let (l, r) = match &mut self.engine {
                Engine::Patch { patch, voices } => run_patch(patch, voices, sr),
                Engine::Drums { kit, level, voices, on, .. } => {
                    let mut s = 0.0;
                    for i in 0..MAX_VOICES {
                        if on[i] {
                            s += voices[i].next(kit, sr);
                            if voices[i].done(kit, sr) {
                                on[i] = false;
                            }
                        }
                    }
                    (s * *level, s * *level)
                }
                Engine::Sampler(s) => s.next(sr),
            };
            out.l[off + k] += l;
            out.r[off + k] += r;
        }
    }
}

/// One sample of every active voice; returns the stereo sum.
fn run_patch(patch: &Patch, voices: &mut [PVoice; MAX_VOICES], sr: f32) -> (f32, f32) {
    let env = patch.has_env();
    let (mut l, mut r) = (0.0, 0.0);
    for v in voices.iter_mut().filter(|v| v.active) {
        let gate = v.remaining >= 0;
        if gate {
            v.remaining -= 1;
            v.fade = (v.fade + 1.0 / (0.002 * sr)).min(1.0);
        } else if !env {
            v.fade -= 1.0 / (0.01 * sr);
        }
        let g = if gate { 1.0 } else { 0.0 };
        let mut alive = false;
        for (i, m) in patch.mods.iter().enumerate() {
            let p = &m.params;
            let st = &mut v.st[i];
            let ins = |j: usize, d: f32| reg(&v.regs, m.ins[j], d);
            let (o0, o1, o2) = match m.kind {
                VKind::NoteIn => (v.pitch, g, v.vel),
                VKind::Vco => {
                    let pitch = ins(0, v.pitch) + p[1] + p[2] / 100.0;
                    let pm = ins(1, 0.0) * p[3];
                    (st.osc.next(Shape::from_index(p[0]), midi_hz(pitch) / sr, pm) * p[4], 0.0, 0.0)
                }
                VKind::Noise => (st.noise.sample() * p[0], 0.0, 0.0),
                VKind::Mixer => {
                    (ins(0, 0.0) * db_gain(p[0]) + ins(1, 0.0) * db_gain(p[1]) + ins(2, 0.0) * db_gain(p[2]), 0.0, 0.0)
                }
                VKind::Filter => {
                    let oct = ins(1, 0.0) * p[3] * 5.0 + (v.pitch - 60.0) / 12.0 * p[4];
                    let hz = p[1] * 2f32.powf(oct);
                    (st.svf.process(ins(0, 0.0), hz, p[2], sr, Mode::from_index(p[0])), 0.0, 0.0)
                }
                VKind::Adsr => {
                    let ap = AdsrParams { a: p[0], d: p[1], s: p[2], r: p[3] };
                    let e = st.adsr.next(ins(0, g) > 0.5, &ap, sr);
                    alive |= !st.adsr.done();
                    (e, 0.0, 0.0)
                }
                VKind::Vca => (ins(0, 0.0) * ins(1, 1.0) * db_gain(p[0]), 0.0, 0.0),
                VKind::Lfo => {
                    let sync = high(ins(1, 0.0));
                    if sync && !st.hi {
                        st.osc.phase = 0.0;
                    }
                    st.hi = sync;
                    let hz = p[0] * 2f32.powf(ins(0, 0.0).clamp(-2.0, 2.0) * 4.0);
                    let y = st.osc.next(Shape::from_index(p[1]), hz / sr, 0.0);
                    (y * p[2], (y + 1.0) * 0.5 * p[2], 0.0)
                }
                VKind::VoiceOut => {
                    let s = ins(0, 0.0) * db_gain(p[0]) * v.fade.max(0.0);
                    let (gl, gr) = pan_gains(p[1]);
                    l += s * gl;
                    r += s * gr;
                    (0.0, 0.0, 0.0)
                }
                VKind::CvMix => (ins(0, 0.0) * p[0] + ins(1, 0.0) * p[1] + ins(2, 0.0) * p[2] + p[3], 0.0, 0.0),
                VKind::Atten => (ins(0, 0.0) * p[0] * ins(1, 1.0) + p[1], 0.0, 0.0),
                VKind::Quantize => {
                    let x = ins(0, v.pitch) + ins(1, 0.0) * p[3] + p[2];
                    let q = scale::quantize(x, p[0].round() as i32, p[1].round().max(0.0) as usize);
                    // a 1 ms trigger when the note changes (and on the first sample)
                    if st.n == 0 || q != st.a {
                        st.a = q;
                        st.b = (0.001 * sr).max(1.0);
                    }
                    st.n = 1;
                    let trig = if st.b > 0.0 {
                        st.b -= 1.0;
                        1.0
                    } else {
                        0.0
                    };
                    (q, trig, 0.0)
                }
                VKind::SampleHold => {
                    let x = if m.ins[0] == NONE { st.noise.sample() } else { ins(0, 0.0) };
                    let t = high(ins(1, g));
                    if t && !st.hi {
                        st.a = x;
                    }
                    st.hi = t;
                    (st.a, 0.0, 0.0)
                }
                VKind::Slew => {
                    let x = ins(0, 0.0);
                    if st.n == 0 {
                        st.a = x;
                        st.n = 1;
                    }
                    let c = smooth_coef(if x > st.a { p[0] } else { p[1] }, sr);
                    st.a = x + (st.a - x) * c;
                    (st.a, 0.0, 0.0)
                }
                VKind::Random => {
                    if st.n == 0 {
                        st.a = st.noise.sample();
                        st.b = st.noise.sample();
                        st.n = 1;
                    }
                    st.osc.phase += p[0].max(0.0) / sr;
                    if st.osc.phase >= 1.0 {
                        st.osc.phase -= st.osc.phase.floor();
                        st.a = st.b;
                        st.b = st.noise.sample();
                    }
                    let t = st.osc.phase;
                    let k = t * t * (3.0 - 2.0 * t);
                    (st.b * p[1], (st.a + (st.b - st.a) * k) * p[1], 0.0)
                }
                VKind::Compare => {
                    let gt = ins(0, 0.0) > ins(1, 0.0) + p[0];
                    (if gt { 1.0 } else { 0.0 }, if gt { 0.0 } else { 1.0 }, 0.0)
                }
                VKind::Logic => {
                    let (a, b) = (high(ins(0, 0.0)), high(ins(1, 0.0)));
                    let f = |x: bool| if x { 1.0 } else { 0.0 };
                    (f(a && b), f(a || b), f(a ^ b))
                }
                VKind::ClockDiv => {
                    let reset = high(ins(1, 0.0));
                    if reset && !st.hi2 {
                        st.n = 0;
                    }
                    st.hi2 = reset;
                    let clk = high(ins(0, 0.0));
                    let div = p[0].round().max(1.0) as u32;
                    if clk && !st.hi {
                        // the first pulse after a reset passes
                        st.a = if st.n % div == 0 { 1.0 } else { 0.0 };
                        st.n = st.n.wrapping_add(1);
                    }
                    st.hi = clk;
                    (if clk { st.a } else { 0.0 }, 0.0, 0.0)
                }
                VKind::Ring => {
                    let a = ins(0, 0.0);
                    let mix = p[0].clamp(0.0, 1.0);
                    (a * (1.0 - mix) + a * ins(1, 0.0) * mix, 0.0, 0.0)
                }
                VKind::Fold => {
                    let gain = 1.0 + 7.0 * (p[0] + ins(1, 0.0) * p[2]).clamp(0.0, 1.0);
                    let y = (core::f32::consts::FRAC_PI_2 * (ins(0, 0.0) * gain + p[1])).sin();
                    (y, 0.0, 0.0)
                }
            };
            for (j, o) in [o0, o1, o2].into_iter().enumerate() {
                if m.outs[j] != NONE {
                    v.regs[m.outs[j] as usize] = o;
                }
            }
        }
        let done = if env { !gate && !alive } else { !gate && v.fade <= 0.0 };
        if done {
            v.active = false;
        }
    }
    (l, r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Buf;

    fn simple_patch() -> Patch {
        // note in -> vco -> vca <- adsr ; vca -> voice out
        let mut note = VModule::new(VKind::NoteIn);
        note.outs = [0, 1, 2];
        note.params[0] = 4.0;
        let mut vco = VModule::new(VKind::Vco);
        vco.ins[0] = 0;
        vco.outs[0] = 3;
        vco.params[..5].copy_from_slice(&[0.0, 0.0, 0.0, 0.0, 1.0]);
        let mut env = VModule::new(VKind::Adsr);
        env.ins[0] = 1;
        env.outs[0] = 4;
        env.params[..4].copy_from_slice(&[0.001, 0.05, 0.5, 0.02]);
        let mut vca = VModule::new(VKind::Vca);
        vca.ins = [3, 4, NONE, NONE];
        vca.outs[0] = 5;
        let mut out = VModule::new(VKind::VoiceOut);
        out.ins[0] = 5;
        Patch { mods: vec![note, vco, env, vca, out] }
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

    #[test]
    fn a_patch_plays_its_notes_and_its_voices_end() {
        let ev = vec![NoteEvent { at: 100, len: 4800, pitch: 69, velocity: 1.0 }];
        let mut i = Instrument::patch(ev, simple_patch());
        let v = render(&mut i, 48_000);
        assert!(v[..100].iter().all(|x| *x == 0.0));
        let held = &v[2400..4800];
        let peak = held.iter().fold(0.0f32, |a, x| a.max(x.abs()));
        assert!((0.3..0.8).contains(&peak), "sustain ~0.5: {peak}");
        let ups = held.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count();
        assert!((21..=23).contains(&ups), "440 Hz over 50 ms: {ups}");
        assert!(v[20_000..].iter().all(|x| x.abs() < 1e-6), "released and silent");
        let Engine::Patch { voices, .. } = &i.engine else { panic!() };
        assert!(voices.iter().all(|v| !v.active));
    }

    #[test]
    fn voices_are_limited_and_params_change_live() {
        let ev: Vec<NoteEvent> =
            (0..6).map(|k| NoteEvent { at: 0, len: 48_000, pitch: 60 + k, velocity: 0.5 }).collect();
        let mut i = Instrument::patch(ev, simple_patch());
        render(&mut i, 1024);
        let Engine::Patch { voices, .. } = &i.engine else { panic!() };
        assert_eq!(voices.iter().filter(|v| v.active).count(), 4, "voices param is 4");
        // vca level to -inf-ish silences the output
        assert!(i.set_param(3, 0, -120.0));
        assert!(!i.set_param(40, 0, 0.0));
        let v = render(&mut i, 1024);
        assert!(v.iter().all(|x| x.abs() < 1e-4));
    }

    /// Plays one note of `pitch` held for a second through `mods`, with a
    /// voice out reading register `probe`; returns the left channel.
    fn probe(mut mods: Vec<VModule>, probe: u8, pitch: u8, frames: usize) -> Vec<f32> {
        let mut out = VModule::new(VKind::VoiceOut);
        out.ins[0] = probe;
        mods.push(out);
        let ev = vec![NoteEvent { at: 0, len: 48_000, pitch, velocity: 1.0 }];
        render(&mut Instrument::patch(ev, Patch { mods }), frames)
    }

    fn note_in() -> VModule {
        let mut n = VModule::new(VKind::NoteIn);
        n.outs = [0, 1, 2];
        n.params[0] = 1.0;
        n
    }

    fn rising(v: &[f32]) -> usize {
        v.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count()
    }

    #[test]
    fn the_quantizer_snaps_pitch_and_triggers_on_change() {
        let mut q = VModule::new(VKind::Quantize);
        q.ins[0] = 0;
        q.outs = [3, 4, NONE];
        q.params[..4].copy_from_slice(&[0.0, 1.0, 0.0, 12.0]); // c major
        let v = probe(vec![note_in(), q], 3, 61, 512);
        assert!((v[300] - 60.0).abs() < 1e-4, "c# to c: {}", v[300]);
        // the cv input adds up to the range: +0.5 of 12 st from an atten offset
        let mut a = VModule::new(VKind::Atten);
        a.outs[0] = 5;
        a.params[..2].copy_from_slice(&[1.0, 0.5]);
        let mut q2 = q;
        q2.ins[1] = 5;
        let v = probe(vec![note_in(), a, q2], 3, 60, 512);
        assert!((v[300] - 65.0).abs() < 1e-4, "c + 6 st to f: {}", v[300]);
        // the trigger fires once, for about a millisecond
        let t = probe(vec![note_in(), q], 4, 61, 4096);
        assert!(t[20] > 0.0 && t[2000] == 0.0);
    }

    #[test]
    fn sample_and_hold_holds_noise_until_the_next_trigger() {
        // an lfo square as the trigger, 100 Hz
        let mut lfo = VModule::new(VKind::Lfo);
        lfo.outs = [3, NONE, NONE];
        lfo.params[..3].copy_from_slice(&[100.0, 3.0, 1.0]);
        let mut sh = VModule::new(VKind::SampleHold);
        sh.ins[1] = 3;
        sh.outs[0] = 4;
        let v = probe(vec![note_in(), lfo, sh], 4, 60, 9600);
        let mut levels = v[200..].to_vec();
        levels.dedup();
        // 0.2 s of 100 Hz: about 20 held values, each held for 480 samples
        assert!((15..=25).contains(&levels.len()), "{}", levels.len());
        assert!(v.iter().all(|x| x.abs() <= 1.0));
    }

    #[test]
    fn slew_glides_and_random_steps_at_its_rate() {
        let mut sl = VModule::new(VKind::Slew);
        sl.ins[0] = 1; // the gate, 0 to 1
        sl.outs[0] = 3;
        sl.params[..2].copy_from_slice(&[0.01, 0.01]);
        let v = probe(vec![note_in(), sl], 3, 60, 4096);
        // starts where its input starts (the gate, 1)
        assert!((v[1000] - 1.0).abs() < 1e-3);
        let mut sl2 = sl;
        sl2.ins[0] = 5;
        let mut lfo = VModule::new(VKind::Lfo);
        lfo.outs = [NONE, 5, NONE];
        lfo.params[..3].copy_from_slice(&[10.0, 3.0, 1.0]); // square, 0–1
        let v = probe(vec![note_in(), lfo, sl2], 3, 60, 4800);
        // the square drops at 2400; half the fall time later the slew is about 10% of the way down
        assert!((v[2399] - 1.0).abs() < 1e-3);
        assert!(v[2400 + 240] > 0.05 && v[2400 + 240] < 0.2, "{}", v[2640]);
        let mut rnd = VModule::new(VKind::Random);
        rnd.outs = [3, 4, NONE];
        rnd.params[..2].copy_from_slice(&[10.0, 1.0]);
        let step = probe(vec![note_in(), rnd], 3, 60, 48_000);
        let mut held = step[200..47_000].to_vec();
        held.dedup();
        assert!((9..=11).contains(&held.len()), "10 hz: {}", held.len());
        let smooth = probe(vec![note_in(), rnd], 4, 60, 48_000);
        let jump = smooth[200..47_000].windows(2).fold(0f32, |a, w| a.max((w[1] - w[0]).abs()));
        assert!(jump < 0.01, "smooth: {jump}");
    }

    #[test]
    fn clock_division_and_logic() {
        let mut lfo = VModule::new(VKind::Lfo);
        lfo.outs = [3, NONE, NONE];
        lfo.params[..3].copy_from_slice(&[100.0, 3.0, 1.0]);
        let mut div = VModule::new(VKind::ClockDiv);
        div.ins[0] = 3;
        div.outs[0] = 4;
        div.params[0] = 4.0;
        let clock = probe(vec![note_in(), lfo, div], 3, 60, 48_000);
        let divided = probe(vec![note_in(), lfo, div], 4, 60, 48_000);
        let (c, d) = (rising(&clock), rising(&divided));
        assert!((99..=101).contains(&c) && (24..=26).contains(&d), "{c} {d}");
        // clock and divided: and is the divided pulses, xor the rest
        let mut lg = VModule::new(VKind::Logic);
        lg.ins = [3, 4, NONE, NONE];
        lg.outs = [5, 6, 7];
        let and = probe(vec![note_in(), lfo, div, lg], 5, 60, 48_000);
        let xor = probe(vec![note_in(), lfo, div, lg], 7, 60, 48_000);
        assert_eq!(rising(&and), d);
        assert_eq!(rising(&xor), c - d);
        // cv mix adds with gains and an offset: gate·0.5 + vel·0.25 + 0.1
        let mut mx = VModule::new(VKind::CvMix);
        mx.ins = [1, 2, NONE, NONE];
        mx.outs[0] = 3;
        mx.params[..4].copy_from_slice(&[0.5, 0.25, 1.0, 0.1]);
        let v = probe(vec![note_in(), mx], 3, 60, 512);
        assert!((v[300] - 0.85).abs() < 1e-5, "{}", v[300]);
    }

    #[test]
    fn ring_mod_and_folder_shape_audio() {
        let mut a = VModule::new(VKind::Vco);
        a.ins[0] = 0;
        a.outs[0] = 3;
        a.params[..5].copy_from_slice(&[0.0, 0.0, 0.0, 0.0, 1.0]);
        let mut b = a;
        b.outs[0] = 4;
        b.params[1] = 7.0;
        let mut ring = VModule::new(VKind::Ring);
        ring.ins = [3, 4, NONE, NONE];
        ring.outs[0] = 5;
        ring.params[0] = 1.0;
        let v = probe(vec![note_in(), a, b, ring], 5, 69, 4800);
        let peak = v[480..].iter().fold(0f32, |m, x| m.max(x.abs()));
        assert!(peak > 0.5 && peak <= 1.0, "{peak}");
        let mut f = VModule::new(VKind::Fold);
        f.ins[0] = 3;
        f.outs[0] = 6;
        f.params[..3].copy_from_slice(&[1.0, 0.0, 0.0]);
        let folded = probe(vec![note_in(), a, f], 6, 69, 4800);
        let plain = probe(vec![note_in(), a], 3, 69, 4800);
        // folding adds zero crossings (harmonics) and stays bounded
        assert!(rising(&folded[480..]) > rising(&plain[480..]) * 2);
        assert!(folded.iter().all(|x| x.abs() <= 1.0));
    }

    #[test]
    fn drums_sound_and_hats_choke() {
        let ev = vec![
            NoteEvent { at: 0, len: 10, pitch: 36, velocity: 1.0 },
            NoteEvent { at: 0, len: 10, pitch: 46, velocity: 1.0 },
            NoteEvent { at: 2000, len: 10, pitch: 42, velocity: 1.0 },
        ];
        let mut i = Instrument::drums(ev, 0.0, 0.0, 1.0);
        let v = render(&mut i, 4096);
        assert!(v.iter().fold(0.0f32, |a, x| a.max(x.abs())) > 0.3);
        let Engine::Drums { on, voices, .. } = &i.engine else { panic!() };
        assert!(!(0..MAX_VOICES).any(|k| on[k] && voices[k].kind == drums::Kind::OpenHat), "open hat choked");
        assert!(i.set_param(0, 2, 2.0));
    }
}
