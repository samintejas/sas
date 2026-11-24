//! The flat, pre-compiled processing schedule.

use crate::automation::Automation;
use crate::synth::{Synth, SynthParam};

/// A stable node id. The compiler derives it from document ids, so a node
/// keeps its id (and its state: voices, smoothing) across recompiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(pub u128);

/// A parameter a [`Command::SetParam`](crate::Command::SetParam) can reach.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Param {
    /// A strip's fader in dB.
    GainDb,
    /// A strip's pan, -1 to 1.
    Pan,
    /// A strip's mute (value > 0.5 = muted).
    Mute,
    /// The gain of one input, linear (sends).
    InputGain(u16),
    /// A synth parameter.
    Synth(SynthParam),
}

/// A stereo buffer.
#[derive(Debug, Clone)]
pub struct Buf {
    /// Left channel.
    pub l: Vec<f32>,
    /// Right channel.
    pub r: Vec<f32>,
}

impl Buf {
    fn new(n: usize) -> Buf {
        Buf { l: vec![0.0; n], r: vec![0.0; n] }
    }
}

/// A value that ramps linearly to its target across one block, so knob
/// moves do not click.
#[derive(Debug, Clone, Copy)]
pub struct Smooth {
    cur: f32,
    target: f32,
}

impl Smooth {
    /// Starts settled at `v`.
    pub fn new(v: f32) -> Smooth {
        Smooth { cur: v, target: v }
    }

    /// Sets the value to reach by the end of the next block.
    pub fn set(&mut self, v: f32) {
        self.target = v;
    }

    /// Returns (start, per-sample step) for a block of `n` samples and settles.
    fn ramp(&mut self, n: usize) -> (f32, f32) {
        let start = self.cur;
        let step = if n == 0 { 0.0 } else { (self.target - start) / n as f32 };
        self.cur = self.target;
        (start, step)
    }
}

/// One input of a node: another node's output times a gain.
#[derive(Debug, Clone)]
pub struct Input {
    /// Index of the source node; always lower than the reading node's.
    pub(crate) src: usize,
    /// Linear gain.
    pub(crate) gain: Smooth,
}

/// A mixer strip: fader, pan and mute, with an optional gain automation lane.
#[derive(Debug, Clone)]
pub struct Strip {
    /// Fader base value in dB.
    pub gain_db: f32,
    /// Pan.
    pub pan: f32,
    /// Muted (after solo resolution).
    pub mute: bool,
    /// Gain automation in dB; overrides the base value while it has data at the playhead.
    pub gain_lane: Option<Automation>,
    gains: (Smooth, Smooth),
}

impl Strip {
    /// A strip with these settings.
    pub fn new(gain_db: f32, pan: f32, mute: bool, gain_lane: Option<Automation>) -> Strip {
        let mut s = Strip { gain_db, pan, mute, gain_lane, gains: (Smooth::new(0.0), Smooth::new(0.0)) };
        let (l, r) = s.target_gains(None);
        s.gains = (Smooth::new(l), Smooth::new(r));
        s
    }

    fn target_gains(&self, pos: Option<i64>) -> (f32, f32) {
        if self.mute {
            return (0.0, 0.0);
        }
        let db = match (pos, &self.gain_lane) {
            (Some(p), Some(lane)) => lane.value_at(p).unwrap_or(self.gain_db),
            _ => self.gain_db,
        };
        let g = db_to_gain(db);
        // equal-power pan
        let a = (self.pan.clamp(-1.0, 1.0) + 1.0) * core::f32::consts::FRAC_PI_4;
        (g * a.cos() * core::f32::consts::SQRT_2, g * a.sin() * core::f32::consts::SQRT_2)
    }

    fn process(&mut self, b: &mut Buf, off: usize, len: usize, pos: Option<i64>) {
        let (tl, tr) = self.target_gains(pos);
        self.gains.0.set(tl);
        self.gains.1.set(tr);
        let (mut gl, dl) = self.gains.0.ramp(len);
        let (mut gr, dr) = self.gains.1.ramp(len);
        for i in off..off + len {
            gl += dl;
            gr += dr;
            b.l[i] *= gl;
            b.r[i] *= gr;
        }
    }
}

/// Converts decibels to linear gain; -inf below -120 dB.
pub fn db_to_gain(db: f32) -> f32 {
    if db <= -120.0 { 0.0 } else { 10f32.powf(db / 20.0) }
}

/// What a node does after summing its inputs.
#[derive(Debug, Clone)]
pub enum NodeKind {
    /// Sums inputs only (groups, returns, master bus).
    Sum,
    /// A mixer strip.
    Strip(Box<Strip>),
    /// A built-in test synth playing note events.
    Synth(Box<Synth>),
}

/// A node in the schedule.
#[derive(Debug, Clone)]
pub struct Node {
    pub(crate) id: NodeId,
    pub(crate) kind: NodeKind,
    pub(crate) inputs: Vec<Input>,
}

/// Why a schedule could not be built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScheduleError {
    /// Two nodes share an id.
    DuplicateId(NodeId),
    /// An input or the output names a node that does not exist.
    UnknownNode(NodeId),
    /// The nodes form a cycle.
    Cycle,
}

impl core::fmt::Display for ScheduleError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ScheduleError::DuplicateId(id) => write!(f, "duplicate node {:x}", id.0),
            ScheduleError::UnknownNode(id) => write!(f, "unknown node {:x}", id.0),
            ScheduleError::Cycle => write!(f, "schedule has a cycle"),
        }
    }
}

impl std::error::Error for ScheduleError {}

struct Pending {
    id: NodeId,
    kind: NodeKind,
    inputs: Vec<(NodeId, f32)>,
}

/// Collects nodes in any order; [`ScheduleBuilder::build`] sorts them.
pub struct ScheduleBuilder {
    nodes: Vec<Pending>,
    sample_rate: f64,
    max_block: usize,
}

impl ScheduleBuilder {
    /// A builder for a schedule at `sample_rate` processing at most `max_block` frames at once.
    pub fn new(sample_rate: f64, max_block: usize) -> ScheduleBuilder {
        ScheduleBuilder { nodes: Vec::new(), sample_rate, max_block: max_block.max(1) }
    }

    /// The sample rate this schedule is built for.
    pub fn sample_rate(&self) -> f64 {
        self.sample_rate
    }

    /// Adds a node fed by `inputs` (source id, linear gain).
    pub fn node(&mut self, id: NodeId, kind: NodeKind, inputs: Vec<(NodeId, f32)>) -> &mut Self {
        self.nodes.push(Pending { id, kind, inputs });
        self
    }

    /// Sorts nodes into dependency order and allocates every buffer.
    /// `output` is the node whose buffer goes to the device.
    pub fn build(self, output: NodeId) -> Result<Schedule, ScheduleError> {
        let n = self.nodes.len();
        let mut ids: Vec<(NodeId, usize)> = self.nodes.iter().enumerate().map(|(i, p)| (p.id, i)).collect();
        ids.sort();
        if let Some(w) = ids.windows(2).find(|w| w[0].0 == w[1].0) {
            return Err(ScheduleError::DuplicateId(w[0].0));
        }
        let find = |id: NodeId| ids.binary_search_by(|(k, _)| k.cmp(&id)).map(|i| ids[i].1);
        // Kahn's algorithm; ties broken by insertion order for determinism
        let mut indeg = vec![0usize; n];
        let mut users: Vec<Vec<usize>> = vec![Vec::new(); n];
        for (i, p) in self.nodes.iter().enumerate() {
            for (src, _) in &p.inputs {
                let s = find(*src).map_err(|_| ScheduleError::UnknownNode(*src))?;
                users[s].push(i);
                indeg[i] += 1;
            }
        }
        let out_old = find(output).map_err(|_| ScheduleError::UnknownNode(output))?;
        let mut ready: std::collections::BTreeSet<usize> = (0..n).filter(|&i| indeg[i] == 0).collect();
        let mut order = Vec::with_capacity(n);
        while let Some(i) = ready.pop_first() {
            order.push(i);
            for &u in &users[i] {
                indeg[u] -= 1;
                if indeg[u] == 0 {
                    ready.insert(u);
                }
            }
        }
        if order.len() != n {
            return Err(ScheduleError::Cycle);
        }
        let mut new_index = vec![0usize; n];
        for (new, &old) in order.iter().enumerate() {
            new_index[old] = new;
        }
        let mut slots: Vec<Option<Pending>> = self.nodes.into_iter().map(Some).collect();
        let nodes: Vec<Node> = order
            .iter()
            .map(|&old| {
                let p = slots[old].take().expect("each node once");
                let inputs = p
                    .inputs
                    .iter()
                    .map(|(src, g)| Input { src: new_index[find(*src).expect("checked")], gain: Smooth::new(*g) })
                    .collect();
                Node { id: p.id, kind: p.kind, inputs }
            })
            .collect();
        let mut by_id: Vec<(NodeId, u32)> = nodes.iter().enumerate().map(|(i, n)| (n.id, i as u32)).collect();
        by_id.sort();
        let bufs = (0..n).map(|_| Buf::new(self.max_block)).collect();
        Ok(Schedule {
            nodes,
            bufs,
            by_id,
            output: new_index[out_old],
            sample_rate: self.sample_rate,
            max_block: self.max_block,
        })
    }
}

/// A compiled, ready-to-run schedule. Built off the audio thread; running it
/// never allocates.
#[derive(Debug, Clone)]
pub struct Schedule {
    nodes: Vec<Node>,
    bufs: Vec<Buf>,
    by_id: Vec<(NodeId, u32)>,
    output: usize,
    sample_rate: f64,
    max_block: usize,
}

impl Schedule {
    /// The sample rate it was built for.
    pub fn sample_rate(&self) -> f64 {
        self.sample_rate
    }

    /// The largest block it processes in one go.
    pub fn max_block(&self) -> usize {
        self.max_block
    }

    /// Number of nodes.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Whether it has no nodes.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Node ids in processing order.
    pub fn order(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.nodes.iter().map(|n| n.id)
    }

    fn index_of(&self, id: NodeId) -> Option<usize> {
        self.by_id.binary_search_by(|(k, _)| k.cmp(&id)).ok().map(|i| self.by_id[i].1 as usize)
    }

    /// Sets a parameter. Returns false if the node or parameter does not exist.
    pub fn set_param(&mut self, id: NodeId, p: Param, v: f32) -> bool {
        let Some(i) = self.index_of(id) else { return false };
        let node = &mut self.nodes[i];
        match (&mut node.kind, p) {
            (_, Param::InputGain(k)) => match node.inputs.get_mut(k as usize) {
                Some(inp) => inp.gain.set(v),
                None => return false,
            },
            (NodeKind::Strip(s), Param::GainDb) => s.gain_db = v,
            (NodeKind::Strip(s), Param::Pan) => s.pan = v,
            (NodeKind::Strip(s), Param::Mute) => s.mute = v > 0.5,
            (NodeKind::Synth(s), Param::Synth(sp)) => s.set_param(sp, v),
            _ => return false,
        }
        true
    }

    /// Takes over per-node state (voices, smoothed gains) from the schedule
    /// it replaces, matching nodes by id. Allocation-free.
    pub fn adopt_state(&mut self, old: &mut Schedule) {
        let (mut a, mut b) = (0, 0);
        while a < self.by_id.len() && b < old.by_id.len() {
            let (ia, ib) = (self.by_id[a], old.by_id[b]);
            match ia.0.cmp(&ib.0) {
                core::cmp::Ordering::Less => a += 1,
                core::cmp::Ordering::Greater => b += 1,
                core::cmp::Ordering::Equal => {
                    let (n, o) = (&mut self.nodes[ia.1 as usize], &mut old.nodes[ib.1 as usize]);
                    match (&mut n.kind, &mut o.kind) {
                        (NodeKind::Synth(n), NodeKind::Synth(o)) => n.adopt(o),
                        (NodeKind::Strip(n), NodeKind::Strip(o)) => n.gains = o.gains,
                        _ => {}
                    }
                    a += 1;
                    b += 1;
                }
            }
        }
    }

    /// Releases every sounding note.
    pub(crate) fn all_notes_off(&mut self) {
        for n in &mut self.nodes {
            if let NodeKind::Synth(s) = &mut n.kind {
                s.release_all();
            }
        }
    }

    /// Runs every node for `len` frames written at `off` in the buffers.
    /// `pos` is the timeline sample at `off` when playing.
    pub(crate) fn run(&mut self, off: usize, len: usize, pos: Option<i64>) {
        debug_assert!(off + len <= self.max_block);
        for i in 0..self.nodes.len() {
            let (before, rest) = self.bufs.split_at_mut(i);
            let out = &mut rest[0];
            out.l[off..off + len].fill(0.0);
            out.r[off..off + len].fill(0.0);
            let node = &mut self.nodes[i];
            for inp in &mut node.inputs {
                let src = &before[inp.src];
                let (mut g, d) = inp.gain.ramp(len);
                for k in off..off + len {
                    g += d;
                    out.l[k] += src.l[k] * g;
                    out.r[k] += src.r[k] * g;
                }
            }
            match &mut node.kind {
                NodeKind::Sum => {}
                NodeKind::Strip(s) => s.process(out, off, len, pos),
                NodeKind::Synth(s) => s.process(out, off, len, pos, self.sample_rate),
            }
        }
    }

    /// The output buffer.
    pub(crate) fn output(&self) -> &Buf {
        &self.bufs[self.output]
    }
}
