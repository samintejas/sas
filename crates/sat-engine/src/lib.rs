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
mod schedule;
mod synth;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, Ordering};

pub use automation::{AutoCurve, AutoPoint, Automation};
pub use schedule::{Buf, Meter, NodeId, NodeKind, Param, Schedule, ScheduleBuilder, ScheduleError, Strip, db_to_gain};
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
}

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
    let e = Engine {
        sched: Box::new(sched),
        rx,
        garbage: gtx,
        pending_garbage: None,
        playing: false,
        pos: 0,
        loop_range: None,
        tel: tel.clone(),
    };
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
            }
        }
        if let Some(p) = self.pending_garbage.take()
            && let Err(rtrb::PushError::Full(p)) = self.garbage.push(p)
        {
            self.pending_garbage = Some(p);
        }
    }

    /// Fills `left` and `right` (equal lengths) with the next frames.
    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        debug_assert_eq!(left.len(), right.len());
        self.handle_commands();
        let max = self.sched.max_block();
        let mut done = 0;
        let (mut pl, mut pr) = (0f32, 0f32);
        while done < left.len() {
            let n = (left.len() - done).min(max);
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
            self.sched.run(0, n, None);
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
            self.sched.run(off, len, Some(self.pos));
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
