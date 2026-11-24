//! The fx node: a track's fx grid compiled to modules in dependency order
//! over stereo registers, processing the track's audio in place before the
//! mixer strip. Everything is allocated in [`Fx::new`].

use sat_dsp::db_gain;
use sat_dsp::fx::{Comp, Delay, Drive, Eq, Filter, Limiter, Reverb};

use crate::instrument::NONE;
use crate::schedule::Buf;

/// Most modules in one fx grid.
pub const MAX_FX: usize = 24;
/// Most stereo registers (connected outputs) in one fx grid.
pub const MAX_FX_REGS: usize = 32;
/// Parameter slots per fx module; the last is the bypass switch.
pub const FX_PARAMS: usize = 8;

/// What an fx module does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FxKind {
    /// The track's audio.
    In,
    /// Back to the track (summed when there are several).
    Out,
    /// Three-band eq.
    Eq,
    /// Compressor (second input: side chain).
    Comp,
    /// Delay.
    Delay,
    /// Reverb.
    Reverb,
    /// Saturation.
    Drive,
    /// Peak limiter.
    Limiter,
    /// Filter.
    Filter,
    /// One input to two outputs.
    Split,
    /// Two inputs to one output.
    Merge,
}

/// One module of an fx grid: registers it reads and writes, and parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FxModule {
    /// What it does.
    pub kind: FxKind,
    /// A stable key (from the module's id) so state survives recompiles.
    pub key: u64,
    /// Input registers, [`NONE`] when unconnected.
    pub ins: [u8; 2],
    /// Output registers, [`NONE`] when unconnected.
    pub outs: [u8; 2],
    /// Parameter values in spec order; `params[7]` > 0.5 bypasses.
    pub params: [f32; FX_PARAMS],
}

impl FxModule {
    /// An unconnected module with every parameter at 0.
    pub fn new(kind: FxKind, key: u64) -> FxModule {
        FxModule { kind, key, ins: [NONE; 2], outs: [NONE; 2], params: [0.0; FX_PARAMS] }
    }
}

#[derive(Debug, Clone)]
enum State {
    None,
    Eq(Box<Eq>),
    Comp(Comp),
    Delay(Box<Delay>),
    Reverb(Box<Reverb>),
    Drive(Box<Drive>),
    Limiter(Box<Limiter>),
    Filter(Box<Filter>),
}

impl State {
    fn new(kind: FxKind, sr: f32) -> State {
        match kind {
            FxKind::Eq => State::Eq(Box::default()),
            FxKind::Comp => State::Comp(Comp::default()),
            FxKind::Delay => State::Delay(Box::new(Delay::new(sr))),
            FxKind::Reverb => State::Reverb(Box::new(Reverb::new(sr))),
            FxKind::Drive => State::Drive(Box::default()),
            FxKind::Limiter => State::Limiter(Box::new(Limiter::new(sr))),
            FxKind::Filter => State::Filter(Box::default()),
            _ => State::None,
        }
    }
}

fn empty() -> Buf {
    Buf { l: Vec::new(), r: Vec::new() }
}

/// A compiled fx grid.
#[derive(Debug, Clone)]
pub struct Fx {
    mods: Vec<FxModule>,
    states: Vec<State>,
    regs: Vec<Buf>,
    input: Buf,
    zero: Buf,
    sr: f32,
}

impl Fx {
    /// Builds the node for `mods` (in run order) at `sample_rate`, for
    /// blocks of at most `max_block` frames.
    pub fn new(mut mods: Vec<FxModule>, sample_rate: f64, max_block: usize) -> Fx {
        mods.truncate(MAX_FX);
        let sr = sample_rate as f32;
        let nregs = mods
            .iter()
            .flat_map(|m| m.ins.iter().chain(&m.outs))
            .filter(|r| **r != NONE)
            .map(|r| *r as usize + 1)
            .max()
            .unwrap_or(0)
            .min(MAX_FX_REGS);
        for m in &mut mods {
            for r in m.ins.iter_mut().chain(m.outs.iter_mut()) {
                if *r != NONE && *r as usize >= nregs {
                    *r = NONE;
                }
            }
        }
        let buf = || Buf { l: vec![0.0; max_block], r: vec![0.0; max_block] };
        Fx {
            states: mods.iter().map(|m| State::new(m.kind, sr)).collect(),
            regs: (0..nregs).map(|_| buf()).collect(),
            mods,
            input: buf(),
            zero: buf(),
            sr,
        }
    }

    /// Sets parameter `index` of module `slot`.
    pub fn set_param(&mut self, slot: u8, index: u8, v: f32) -> bool {
        match self.mods.get_mut(slot as usize) {
            Some(m) if (index as usize) < FX_PARAMS => {
                m.params[index as usize] = v;
                true
            }
            _ => false,
        }
    }

    /// Takes over tails and envelopes from `old`, module by module (same key
    /// and kind). Swaps buffers rather than copying them; allocation-free.
    pub fn adopt(&mut self, old: &mut Fx) {
        for i in 0..self.mods.len() {
            let (key, kind) = (self.mods[i].key, self.mods[i].kind);
            if let Some(j) = old.mods.iter().position(|m| m.key == key && m.kind == kind)
                && self.sr == old.sr
            {
                core::mem::swap(&mut self.states[i], &mut old.states[j]);
            }
        }
    }

    /// Gain reduction (dB) of a compressor or limiter in the last block.
    pub fn reduction(&self, slot: usize) -> Option<f32> {
        match self.states.get(slot)? {
            State::Comp(c) => Some(c.meter),
            State::Limiter(l) => Some(l.meter),
            _ => None,
        }
    }

    /// Processes `out[off..off + len]` in place.
    pub(crate) fn process(&mut self, out: &mut Buf, off: usize, len: usize) {
        let r = off..off + len;
        self.input.l[r.clone()].copy_from_slice(&out.l[r.clone()]);
        self.input.r[r.clone()].copy_from_slice(&out.r[r.clone()]);
        out.l[r.clone()].fill(0.0);
        out.r[r.clone()].fill(0.0);
        let sr = self.sr;
        for i in 0..self.mods.len() {
            let m = self.mods[i];
            // take this module's outputs out of the register file so its
            // inputs can be read while they are written
            let mut o = [empty(), empty()];
            for (k, reg) in m.outs.iter().enumerate() {
                if *reg != NONE {
                    o[k] = core::mem::replace(&mut self.regs[*reg as usize], empty());
                }
            }
            let input = |k: usize| -> &Buf {
                match m.ins[k] {
                    NONE => &self.zero,
                    reg => &self.regs[reg as usize],
                }
            };
            let p = &m.params;
            let bypass = p[FX_PARAMS - 1] > 0.5;
            match m.kind {
                FxKind::In => {
                    if !o[0].l.is_empty() {
                        let (g, w) = (db_gain(p[0]), p[1]);
                        for k in r.clone() {
                            let (a, b) = (self.input.l[k], self.input.r[k]);
                            let (mid, side) = ((a + b) * 0.5, (a - b) * 0.5 * w);
                            o[0].l[k] = (mid + side) * g;
                            o[0].r[k] = (mid - side) * g;
                        }
                    }
                }
                FxKind::Out => {
                    let (src, g) = (input(0), db_gain(p[0]));
                    for k in r.clone() {
                        out.l[k] += src.l[k] * g;
                        out.r[k] += src.r[k] * g;
                    }
                }
                FxKind::Split => {
                    let src = input(0);
                    let bal = p[0].clamp(0.0, 1.0);
                    let gains = [(2.0 * (1.0 - bal)).min(1.0), (2.0 * bal).min(1.0)];
                    for (dst, g) in o.iter_mut().zip(gains) {
                        if !dst.l.is_empty() {
                            for k in r.clone() {
                                dst.l[k] = src.l[k] * g;
                                dst.r[k] = src.r[k] * g;
                            }
                        }
                    }
                }
                FxKind::Merge => {
                    if !o[0].l.is_empty() {
                        let (a, b) = (input(0), input(1));
                        let (ga, gb) = (db_gain(p[0]), db_gain(p[1]));
                        for k in r.clone() {
                            o[0].l[k] = a.l[k] * ga + b.l[k] * gb;
                            o[0].r[k] = a.r[k] * ga + b.r[k] * gb;
                        }
                    }
                }
                _ => {
                    // effects: copy the input across, then process in place
                    if !o[0].l.is_empty() {
                        let src = input(0);
                        o[0].l[r.clone()].copy_from_slice(&src.l[r.clone()]);
                        o[0].r[r.clone()].copy_from_slice(&src.r[r.clone()]);
                    }
                    let side = (m.ins[1] != NONE).then(|| input(1));
                    let state = &mut self.states[i];
                    if !bypass && !o[0].l.is_empty() {
                        let (l, rr) = (&mut o[0].l[r.clone()], &mut o[0].r[r.clone()]);
                        match state {
                            State::Eq(e) => e.process(l, rr, p, sr),
                            State::Comp(c) => {
                                let side = side.map(|s| (&s.l[r.clone()], &s.r[r.clone()]));
                                c.process(l, rr, side, p, sr)
                            }
                            State::Delay(d) => d.process(l, rr, p, sr),
                            State::Reverb(v) => v.process(l, rr, p, sr),
                            State::Drive(d) => d.process(l, rr, p, sr),
                            State::Limiter(x) => x.process(l, rr, p, sr),
                            State::Filter(f) => f.process(l, rr, p, sr),
                            State::None => {}
                        }
                    }
                }
            }
            for (k, reg) in m.outs.iter().enumerate() {
                if *reg != NONE {
                    self.regs[*reg as usize] = core::mem::replace(&mut o[k], empty());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chain(mid: Option<FxModule>) -> Vec<FxModule> {
        let mut fin = FxModule::new(FxKind::In, 1);
        fin.outs[0] = 0;
        fin.params[0] = 0.0;
        fin.params[1] = 1.0;
        let mut fout = FxModule::new(FxKind::Out, 2);
        let mut v = vec![fin];
        match mid {
            Some(mut m) => {
                m.ins[0] = 0;
                m.outs[0] = 1;
                fout.ins[0] = 1;
                v.push(m);
            }
            None => fout.ins[0] = 0,
        }
        v.push(fout);
        v
    }

    /// Runs `l`/`r` through in 64-frame blocks.
    fn run(fx: &mut Fx, l: &[f32], r: &[f32]) -> Buf {
        let mut out = Buf { l: Vec::new(), r: Vec::new() };
        for (cl, cr) in l.chunks(64).zip(r.chunks(64)) {
            let mut b = Buf { l: vec![0.0; 64], r: vec![0.0; 64] };
            b.l[..cl.len()].copy_from_slice(cl);
            b.r[..cr.len()].copy_from_slice(cr);
            fx.process(&mut b, 0, cl.len());
            out.l.extend_from_slice(&b.l[..cl.len()]);
            out.r.extend_from_slice(&b.r[..cr.len()]);
        }
        out
    }

    #[test]
    fn in_to_out_passes_audio_and_width_narrows_it() {
        let l: Vec<f32> = (0..256).map(|i| (i as f32 * 0.1).sin()).collect();
        let r: Vec<f32> = (0..256).map(|i| (i as f32 * 0.07).cos()).collect();
        let mut fx = Fx::new(chain(None), 48_000.0, 64);
        let out = run(&mut fx, &l, &r);
        assert!(out.l.iter().zip(&l).all(|(a, b)| (a - b).abs() < 1e-6));
        // width 0 is mono
        let mut c = chain(None);
        c[0].params[1] = 0.0;
        let out = run(&mut Fx::new(c, 48_000.0, 64), &l, &r);
        assert_eq!(out.l, out.r);
    }

    #[test]
    fn effects_process_and_bypass_and_state_carries_over() {
        let mut d = FxModule::new(FxKind::Delay, 9);
        d.params[..4].copy_from_slice(&[0.01, 0.0, 20_000.0, 1.0]);
        let mut impulse = vec![0.0f32; 2048];
        impulse[0] = 1.0;
        let mut fx = Fx::new(chain(Some(d)), 48_000.0, 64);
        let out = run(&mut fx, &impulse, &impulse);
        assert!(out.l[0].abs() < 1e-3 && out.l[480].abs() > 0.5, "the delay moved the impulse");
        // bypassed, the impulse goes straight through
        let mut bd = d;
        bd.params[FX_PARAMS - 1] = 1.0;
        let out = run(&mut Fx::new(chain(Some(bd)), 48_000.0, 64), &impulse, &impulse);
        assert_eq!(out.l[0], 1.0);
        // a long delay half-way through its time, handed to a new node, still echoes
        let mut long = d;
        long.params[0] = 0.03;
        let mut a = Fx::new(chain(Some(long)), 48_000.0, 64);
        run(&mut a, &impulse[..256], &impulse[..256]);
        let mut b = Fx::new(chain(Some(long)), 48_000.0, 64);
        b.adopt(&mut a);
        let silence = vec![0.0f32; 2048];
        let out = run(&mut b, &silence, &silence);
        assert!(out.l.iter().any(|x| x.abs() > 0.5), "the echo survived the swap");
        // a live parameter change reaches the module
        assert!(b.set_param(1, 0, 0.02) && !b.set_param(7, 0, 1.0));
    }

    #[test]
    fn the_side_chain_drives_the_compressor() {
        // main: a quiet signal; side: a loud one from a second input register
        let mut fin = FxModule::new(FxKind::In, 1);
        fin.outs[0] = 0;
        fin.params[1] = 1.0;
        let mut s = FxModule::new(FxKind::Split, 2);
        s.ins[0] = 0;
        s.outs = [1, 2];
        s.params[0] = 0.5;
        let mut boost = FxModule::new(FxKind::Merge, 3);
        boost.ins[0] = 2;
        boost.outs[0] = 3;
        boost.params[0] = 6.0; // the side path is louder
        let mut c = FxModule::new(FxKind::Comp, 4);
        c.ins = [1, 3];
        c.outs[0] = 4;
        c.params[..5].copy_from_slice(&[-30.0, 10.0, 0.001, 0.1, 0.0]);
        let mut fout = FxModule::new(FxKind::Out, 5);
        fout.ins[0] = 4;
        let x: Vec<f32> = (0..4800).map(|i| (i as f32 * 0.05).sin() * 0.1).collect();
        let squashed = run(&mut Fx::new(vec![fin, s, boost, c, fout], 48_000.0, 64), &x, &x);
        // without the side chain the same comp (threshold -30 db) does less
        let mut plain = c;
        plain.ins[1] = NONE;
        let mut fout2 = fout;
        fout2.ins[0] = 4;
        let gentle = run(&mut Fx::new(vec![fin, s, plain, fout2], 48_000.0, 64), &x, &x);
        let peak = |b: &Buf| b.l[2400..].iter().fold(0f32, |a, v| a.max(v.abs()));
        assert!(peak(&squashed) < peak(&gentle) * 0.7, "{} vs {}", peak(&squashed), peak(&gentle));
    }

    #[test]
    fn split_and_merge_sum_two_paths() {
        let mut fin = FxModule::new(FxKind::In, 1);
        fin.outs[0] = 0;
        fin.params[1] = 1.0;
        let mut s = FxModule::new(FxKind::Split, 2);
        s.ins[0] = 0;
        s.outs = [1, 2];
        s.params[0] = 0.5;
        let mut m = FxModule::new(FxKind::Merge, 3);
        m.ins = [1, 2];
        m.outs[0] = 3;
        let mut fout = FxModule::new(FxKind::Out, 4);
        fout.ins[0] = 3;
        let mut fx = Fx::new(vec![fin, s, m, fout], 48_000.0, 64);
        let out = run(&mut fx, &[0.25; 64], &[0.25; 64]);
        assert!(out.l.iter().all(|x| (x - 0.5).abs() < 1e-6), "both paths at unity");
    }
}
