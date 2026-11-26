//! The sat real-time engine.
//!
//! The engine runs a pre-compiled, flat [`Schedule`] of nodes. On the audio
//! thread it never allocates, locks or makes system calls:
//!
//! - commands arrive through an `rtrb` single-producer ring ([`EngineHandle::send`]);
//! - a new schedule is built off-thread and swapped in at a block boundary,
//!   carrying node state over by stable [`NodeId`];
//! - the replaced schedule goes back on a second ring and is dropped by the
//!   control thread ([`EngineHandle::collect_garbage`]);
//! - telemetry (playhead, peaks) is published through atomics.
//!
//! [`render`] runs the same engine offline, faster than real time.

#![warn(missing_docs)]

mod automation;
mod fx;
mod instrument;
mod monitor;
mod player;
mod sampler;
mod schedule;
mod sequencer;
mod synth;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, Ordering};

pub use automation::{AutoCurve, AutoPoint, Automation};
pub use fx::{External, FX_PARAMS, Fx, FxKind, FxModule, MAX_FX, MAX_FX_REGS};
pub use instrument::{Instrument, MAX_MODS, MAX_PARAMS, MAX_REGS, MAX_VOICES, NONE, Patch, VKind, VModule};
pub use monitor::{InputChannel, Monitor, MonitorMode};
pub use player::{AudioPlayer, AudioSeg};
pub use sampler::{PlayMode, Sample};
pub use schedule::{Buf, Meter, NodeId, NodeKind, Param, Schedule, ScheduleBuilder, ScheduleError, Strip, db_to_gain};
pub use sequencer::SessionClip;
pub use synth::{NoteEvent, Synth, SynthParam, Wave};

/// A message to the audio thread.
#[derive(Debug)]
pub enum Command {
    /// Replace the running schedule at the next block boundary.
    Swap(Box<Schedule>),
    /// Set a parameter on a node.
    SetParam {
        /// The node.
        node: NodeId,
        /// The parameter.
        param: Param,
        /// The value.
        value: f32,
    },
    /// Start playback from the current position.
    Play,
    /// Stop playback; sounding notes release.
    Stop,
    /// Move the playhead to a timeline sample.
    Locate(i64),
    /// Set or clear the loop range in timeline samples (start, end).
    SetLoop(Option<(i64, i64)>),
    /// Launch a session clip (by key) on a node at a timeline sample, or
    /// stop the node's clip there with `None`. The node then follows the
    /// session until [`Command::BackToArrangement`].
    Launch {
        /// The instrument node.
        node: NodeId,
        /// The clip's key, or `None` to stop.
        clip: Option<u64>,
        /// When, in timeline samples.
        at: i64,
    },
    /// Every track plays the arrangement again.
    BackToArrangement,
    /// Hands one of a track's nodes (instrument, audio player or strip) to
    /// the launcher from timeline sample `at`, or back to the arrangement.
    Source {
        /// The node.
        node: NodeId,
        /// Launcher (true) or arrangement (false).
        launcher: bool,
        /// When, in timeline samples (the hand-over to the launcher).
        at: i64,
    },
    /// Recording started or stopped (tracks monitoring on auto hear their
    /// input while recording).
    SetRecording(bool),
    /// A note played live on an instrument node (from the chord builder's
    /// pads or the computer keyboard): `on` starts it, held until the same
    /// pitch is released; it sounds whether or not the transport plays.
    Live {
        /// The instrument node.
        node: NodeId,
        /// MIDI pitch.
        pitch: u8,
        /// Velocity 0–1.
        velocity: f32,
        /// Start (true) or release (false).
        on: bool,
    },
}

/// Gate length of a live note: long enough to hold until released.
pub(crate) const LIVE_HOLD: i64 = i64::MAX / 4;

/// Read-only engine state for the UI, updated every block.
#[derive(Debug, Default)]
pub struct Telemetry {
    playhead: AtomicI64,
    playing: AtomicBool,
    peak_l: AtomicU32,
    peak_r: AtomicU32,
    blocks: AtomicU32,
}

impl Telemetry {
    /// Playhead in timeline samples.
    pub fn playhead(&self) -> i64 {
        self.playhead.load(Ordering::Relaxed)
    }

    /// Whether the transport is playing.
    pub fn playing(&self) -> bool {
        self.playing.load(Ordering::Relaxed)
    }

    /// Output peaks (left, right) of the last block, linear.
    pub fn peaks(&self) -> (f32, f32) {
        (f32::from_bits(self.peak_l.load(Ordering::Relaxed)), f32::from_bits(self.peak_r.load(Ordering::Relaxed)))
    }

    /// Blocks processed so far (wraps).
    pub fn blocks(&self) -> u32 {
        self.blocks.load(Ordering::Relaxed)
    }
}

/// The audio-thread side. Call [`Engine::process`] from the device callback.
pub struct Engine {
    sched: Box<Schedule>,
    rx: rtrb::Consumer<Command>,
    garbage: rtrb::Producer<Box<Schedule>>,
    /// A replaced schedule that did not fit on the garbage ring yet.
    pending_garbage: Option<Box<Schedule>>,
    playing: bool,
    pos: i64,
    loop_range: Option<(i64, i64)>,
    recording: bool,
    /// The input of the current block.
    input: Buf,
    tel: Arc<Telemetry>,
}

/// The control-thread side.
pub struct EngineHandle {
    tx: rtrb::Producer<Command>,
    garbage: rtrb::Consumer<Box<Schedule>>,
    tel: Arc<Telemetry>,
}

impl EngineHandle {
    /// Queues a command. Gives it back if the ring is full.
    pub fn send(&mut self, cmd: Command) -> Result<(), Command> {
        self.tx.push(cmd).map_err(|rtrb::PushError::Full(c)| c)
    }

    /// Drops schedules the engine has finished with. Call it regularly off
    /// the audio thread.
    pub fn collect_garbage(&mut self) -> usize {
        let mut n = 0;
        while self.garbage.pop().is_ok() {
            n += 1;
        }
        n
    }

    /// Engine telemetry.
    pub fn telemetry(&self) -> &Arc<Telemetry> {
        &self.tel
    }
}

/// Creates an engine running `sched`, with room for `capacity` queued commands.
pub fn engine(sched: Schedule, capacity: usize) -> (Engine, EngineHandle) {
    let (tx, rx) = rtrb::RingBuffer::new(capacity.max(1));
    let (gtx, grx) = rtrb::RingBuffer::new(16);
    let tel = Arc::new(Telemetry::default());
    let mut e = Engine {
        sched: Box::new(sched),
        rx,
        garbage: gtx,
        pending_garbage: None,
        playing: false,
        pos: 0,
        loop_range: None,
        recording: false,
        input: Buf { l: Vec::new(), r: Vec::new() },
        tel: tel.clone(),
    };
    let n = e.sched.max_block();
    e.input = Buf { l: vec![0.0; n], r: vec![0.0; n] };
    (e, EngineHandle { tx, garbage: grx, tel })
}

impl Engine {
    fn retire(&mut self, old: Box<Schedule>) {
        if let Some(p) = self.pending_garbage.take()
            && let Err(rtrb::PushError::Full(p)) = self.garbage.push(p)
        {
            // Both the ring and the spare slot are full: the control thread
            // is not collecting. Dropping here frees memory on the audio
            // thread, which is the lesser evil compared with leaking.
            drop(p);
        }
        if let Err(rtrb::PushError::Full(old)) = self.garbage.push(old) {
            self.pending_garbage = Some(old);
        }
    }

    fn handle_commands(&mut self) {
        while let Ok(cmd) = self.rx.pop() {
            match cmd {
                Command::Swap(mut new) => {
                    new.adopt_state(&mut self.sched);
                    let old = core::mem::replace(&mut self.sched, new);
                    self.retire(old);
                }
                Command::SetParam { node, param, value } => {
                    self.sched.set_param(node, param, value);
                }
                Command::Play => self.playing = true,
                Command::Stop => {
                    self.playing = false;
                    self.sched.all_notes_off();
                }
                Command::Locate(p) => {
                    self.pos = p;
                    self.sched.all_notes_off();
                }
                Command::SetLoop(r) => self.loop_range = r.filter(|(s, e)| e > s),
                Command::Launch { node, clip, at } => {
                    self.sched.launch(node, clip, at);
                }
                Command::BackToArrangement => self.sched.back_to_arrangement(),
                Command::Source { node, launcher, at } => {
                    self.sched.source(node, launcher, at);
                }
                Command::SetRecording(on) => self.recording = on,
                Command::Live { node, pitch, velocity, on } => {
                    self.sched.live(node, pitch, velocity, on);
                }
            }
        }
        if let Some(p) = self.pending_garbage.take()
            && let Err(rtrb::PushError::Full(p)) = self.garbage.push(p)
        {
            self.pending_garbage = Some(p);
        }
    }

    /// Fills `left` and `right` (equal lengths) with the next frames, with
    /// no input (monitored tracks hear silence).
    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        self.process_with_input(&[], &[], left, right);
    }

    /// Like [`Engine::process`], with the device input for the same frames
    /// (missing frames are silence).
    pub fn process_with_input(&mut self, in_l: &[f32], in_r: &[f32], left: &mut [f32], right: &mut [f32]) {
        debug_assert_eq!(left.len(), right.len());
        self.handle_commands();
        let max = self.sched.max_block().min(self.input.l.len());
        let mut done = 0;
        let (mut pl, mut pr) = (0f32, 0f32);
        while done < left.len() {
            let n = (left.len() - done).min(max);
            for (k, (l, r)) in self.input.l[..n].iter_mut().zip(&mut self.input.r[..n]).enumerate() {
                *l = in_l.get(done + k).copied().unwrap_or(0.0);
                *r = in_r.get(done + k).copied().unwrap_or(0.0);
            }
            self.run_block(n);
            let out = self.sched.output();
            for k in 0..n {
                let (l, r) = (out.l[k], out.r[k]);
                left[done + k] = l;
                right[done + k] = r;
                pl = pl.max(l.abs());
                pr = pr.max(r.abs());
            }
            done += n;
        }
        self.tel.playhead.store(self.pos, Ordering::Relaxed);
        self.tel.playing.store(self.playing, Ordering::Relaxed);
        self.tel.peak_l.store(pl.to_bits(), Ordering::Relaxed);
        self.tel.peak_r.store(pr.to_bits(), Ordering::Relaxed);
        self.tel.blocks.fetch_add(1, Ordering::Relaxed);
    }

    /// Runs one block of `n` frames, splitting it where the loop wraps.
    fn run_block(&mut self, n: usize) {
        if !self.playing {
            self.sched.run(0, n, None, &self.input, self.recording);
            return;
        }
        let mut off = 0;
        while off < n {
            let mut len = n - off;
            if let Some((_, e)) = self.loop_range
                && self.pos < e
            {
                len = len.min((e - self.pos) as usize);
            }
            self.sched.run(off, len, Some(self.pos), &self.input, self.recording);
            self.pos += len as i64;
            off += len;
            if let Some((s, e)) = self.loop_range
                && self.pos >= e
            {
                self.pos = s;
            }
        }
    }

    /// Current playhead.
    pub fn position(&self) -> i64 {
        self.pos
    }
}

/// Renders `frames` of `sched` offline from timeline sample `start`, in
/// blocks of `block` frames. Returns (left, right).
pub fn render(sched: Schedule, start: i64, frames: usize, block: usize) -> (Vec<f32>, Vec<f32>) {
    let (mut e, mut h) = engine(sched, 4);
    h.send(Command::Locate(start)).expect("fresh ring");
    h.send(Command::Play).expect("fresh ring");
    let mut l = vec![0.0; frames];
    let mut r = vec![0.0; frames];
    let block = block.max(1);
    let mut i = 0;
    while i < frames {
        let n = block.min(frames - i);
        e.process(&mut l[i..i + n], &mut r[i..i + n]);
        i += n;
    }
    (l, r)
}

#[cfg(test)]
mod monitor_tests {
    use super::*;

    fn sched(mode: MonitorMode) -> Schedule {
        let mut b = ScheduleBuilder::new(48_000.0, 64);
        let m = NodeId(1);
        b.node(m, NodeKind::Monitor(Box::new(Monitor::new(mode, InputChannel::Left))), vec![]);
        b.build(m).unwrap()
    }

    fn run(e: &mut Engine) -> Vec<f32> {
        let input = vec![0.5f32; 64];
        let (mut l, mut r) = (vec![0.0; 64], vec![0.0; 64]);
        e.process_with_input(&input, &[0.25; 64], &mut l, &mut r);
        assert_eq!(l, r, "the first channel on both sides");
        l
    }

    #[test]
    fn monitors_follow_their_mode_and_ramp() {
        let (mut e, mut h) = engine(sched(MonitorMode::Auto), 8);
        let v = run(&mut e);
        assert!(v[0] > 0.0 && v[0] < 0.1 && v[63] == 0.5, "ramps in while stopped");
        assert_eq!(run(&mut e)[0], 0.5);
        h.send(Command::Play).unwrap();
        assert_eq!(run(&mut e)[63], 0.0, "auto: quiet while playing back");
        h.send(Command::SetRecording(true)).unwrap();
        run(&mut e);
        assert_eq!(run(&mut e)[0], 0.5, "and heard while recording");
        let (mut off, _) = engine(sched(MonitorMode::Off), 8);
        assert!(run(&mut off).iter().all(|x| *x == 0.0));
        // without input the monitor hears silence
        let (mut e, _) = engine(sched(MonitorMode::In), 8);
        let (mut l, mut r) = (vec![1.0; 64], vec![1.0; 64]);
        e.process(&mut l, &mut r);
        assert!(l.iter().all(|x| *x == 0.0));
    }
}

#[cfg(test)]
mod live_tests {
    use super::*;

    fn loud(e: &mut Engine) -> f32 {
        let (mut l, mut r) = (vec![0.0; 256], vec![0.0; 256]);
        e.process(&mut l, &mut r);
        l.iter().fold(0.0f32, |a, x| a.max(x.abs()))
    }

    #[test]
    fn live_notes_hold_until_released_while_stopped() {
        let mut b = ScheduleBuilder::new(48_000.0, 256);
        let s = NodeId(1);
        b.node(s, NodeKind::Synth(Box::new(Synth::new(Vec::new(), Wave::Saw, 0.5))), vec![]);
        let (mut e, mut h) = engine(b.build(s).unwrap(), 16);
        assert_eq!(loud(&mut e), 0.0);
        h.send(Command::Live { node: s, pitch: 60, velocity: 1.0, on: true }).unwrap();
        h.send(Command::Live { node: s, pitch: 64, velocity: 1.0, on: true }).unwrap();
        loud(&mut e);
        // still sounding long after any sequenced note would have ended
        for _ in 0..200 {
            loud(&mut e);
        }
        assert!(loud(&mut e) > 0.05, "held");
        h.send(Command::Live { node: s, pitch: 60, velocity: 0.0, on: false }).unwrap();
        h.send(Command::Live { node: s, pitch: 64, velocity: 0.0, on: false }).unwrap();
        for _ in 0..40 {
            loud(&mut e);
        }
        assert!(loud(&mut e) < 1e-3, "released");
        // a node that plays no notes ignores live notes
        assert!(!e.sched.live(NodeId(99), 60, 1.0, true));
    }
}
