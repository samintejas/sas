//! Instrument nodes: a voice patch that every voice runs, a drum synth, or
//! a sampler.
//!
//! A [`Patch`] is a list of modules in dependency order. Each module reads
//! its inputs from, and writes its outputs to, a small per-voice register
//! file, so running a patch is a loop over modules per voice per sample with
//! no allocation. Unconnected inputs fall back to sensible defaults: an
//! oscillator's pitch follows the voice, a vca's cv is 1, an envelope's gate
//! is the voice's gate.

use sat_dsp::drums::{self, Kit};
use sat_dsp::{Adsr, AdsrParams, Mode, Noise, Osc, Shape, Svf, db_gain, midi_hz, pan_gains};

use std::sync::Arc;

use crate::sampler::{Sample, Sampler};
use crate::sequencer::{Cue, Sequencer, SessionClip};
use crate::synth::NoteEvent;

/// Most modules in a patch.
pub const MAX_MODS: usize = 24;
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
    /// Out: -1–1 scaled by depth. Params: rate (Hz), wave, depth.
    Lfo,
    /// In: audio. Params: level (dB), pan.
    VoiceOut,
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
                VKind::Lfo => (st.osc.next(Shape::from_index(p[1]), p[0] / sr, 0.0) * p[2], 0.0, 0.0),
                VKind::VoiceOut => {
                    let s = ins(0, 0.0) * db_gain(p[0]) * v.fade.max(0.0);
                    let (gl, gr) = pan_gains(p[1]);
                    l += s * gl;
                    r += s * gr;
                    (0.0, 0.0, 0.0)
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
