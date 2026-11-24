//! Compiles a sat [`Document`] into an engine [`Schedule`].
//!
//! The compiler walks the track tree bottom-up: each track becomes a source
//! of note events feeding its instrument, then a mixer strip that feeds its
//! group's bus (or master); groups and returns are a bus plus a strip;
//! master's strip is the output. Node ids are derived from document ids, so
//! state carries over when a recompiled schedule replaces the running one.
//!
//! A track's instrument is the voice patch on its instrument grid (see
//! [`build_patch`]), else its drum synth, else its sampler (playing a sound
//! from the [`Sounds`] the app has loaded), else the old built-in test synth
//! (a module of kind [`TEST_SYNTH`]). A clip plays the pattern
//! of the source module wired straight into its dock inlet (a generator's
//! output plus the clip's hand edits; see `sat_modules`). Every track's fx
//! grid (see [`build_fx`]) runs between its sound and its strip; pre-fader
//! sends tap after it. Session launching arrives with a later phase.

#![warn(missing_docs)]

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use sat_doc::sat_time::{TempoMap, Ticks};
use sat_doc::*;
use sat_engine::{
    AutoCurve, AutoPoint, Automation, Command, FX_PARAMS, Fx, FxKind, FxModule, Instrument, MAX_FX, MAX_FX_REGS,
    MAX_MODS, MAX_PARAMS, MAX_REGS, Meter, NONE, NodeId, NodeKind, NoteEvent, Param, Patch, Sample, Schedule,
    ScheduleBuilder, Strip, Synth, SynthParam, VKind, VModule, Wave, db_to_gain,
};
use sat_modules::instrument as ins;

/// Decoded sounds of the audio pool, by id. The compiler shares them into
/// the schedule; it never loads files itself.
pub type Sounds = HashMap<AudioId, Arc<Sample>>;

/// Module kind of the built-in test synth.
pub const TEST_SYNTH: &str = "test_synth";

/// Why a document could not be compiled.
#[derive(Debug)]
pub enum CompileError {
    /// The tempo lane does not make a valid tempo map.
    Tempo(sat_time::TimeError),
    /// The schedule graph is invalid (a compiler bug).
    Schedule(sat_engine::ScheduleError),
}

impl core::fmt::Display for CompileError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            CompileError::Tempo(e) => write!(f, "tempo: {e}"),
            CompileError::Schedule(e) => write!(f, "schedule: {e}"),
        }
    }
}

impl std::error::Error for CompileError {}

/// How a parameter's value reaches the engine.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Conv {
    Same,
    DbToLinear,
}

/// A compiled document.
pub struct Compiled {
    /// The schedule to run (or swap in).
    pub schedule: Schedule,
    /// The output node (master's strip).
    pub output: NodeId,
    /// The tempo map the schedule was built with.
    pub tempo: TempoMap,
    params: HashMap<ParamTarget, (NodeId, Param, Conv)>,
    /// What each assigned macro drives: (node, param, spec, bind).
    #[allow(clippy::type_complexity)]
    macros: HashMap<ParamTarget, Vec<(NodeId, Param, &'static sat_modules::ParamSpec, MacroBind)>>,
    meters: HashMap<TrackId, Arc<Meter>>,
}

impl Compiled {
    /// The post-fader peak meter of a track's strip.
    pub fn meter(&self, track: TrackId) -> Option<&Arc<Meter>> {
        self.meters.get(&track)
    }

    /// The engine commands for a value change: one for a plain parameter,
    /// one per driven parameter for a macro, none for a parameter a macro
    /// drives (its macro decides its value).
    pub fn param_commands(&self, target: &ParamTarget, value: f32) -> Vec<Command> {
        if let Some(driven) = self.macros.get(target) {
            return driven
                .iter()
                .map(|(node, param, spec, bind)| Command::SetParam {
                    node: *node,
                    param: *param,
                    value: sat_modules::macros::value_at(spec, bind, value),
                })
                .collect();
        }
        if matches!(target, ParamTarget::Module { .. }) && self.macros.values().flatten().any(|d| {
            matches!(target, ParamTarget::Module { module, param } if *module == d.3.module && *param == d.3.param)
        }) {
            return Vec::new();
        }
        self.param_command(target, value).into_iter().collect()
    }

    /// Turns a value change from the document into an engine command, or
    /// `None` if the target has no live node (a recompile covers it).
    pub fn param_command(&self, target: &ParamTarget, value: f32) -> Option<Command> {
        let (node, param, conv) = *self.params.get(target)?;
        let value = match conv {
            Conv::Same => value,
            Conv::DbToLinear => db_to_gain(value),
        };
        Some(Command::SetParam { node, param, value })
    }

    /// Converts the document's loop range to engine samples.
    pub fn loop_samples(&self, doc: &Document) -> Option<(i64, i64)> {
        let r = doc.loop_range;
        r.enabled.then(|| (self.sample(r.start), self.sample(r.end)))
    }

    /// A tick position in timeline samples.
    pub fn sample(&self, t: Ticks) -> i64 {
        self.tempo.tick_to_sample(t, self.schedule.sample_rate()).round() as i64
    }
}

const SALT_SYNTH: u128 = 0x5a7_0001;
const SALT_STRIP: u128 = 0x5a7_0002;
const SALT_BUS: u128 = 0x5a7_0003;
const SALT_FX: u128 = 0x5a7_0004;

fn nid(t: TrackId, salt: u128) -> NodeId {
    // salts sit in the middle bits, clear of ids that differ only in their low bits
    NodeId(t.0.as_u128() ^ (salt << 64))
}

/// Deterministic hash for note chance (splitmix64).
fn mix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

/// A note placed on the timeline, in ticks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimelineNote {
    /// Start on the timeline.
    pub start: Ticks,
    /// End on the timeline.
    pub end: Ticks,
    /// Pitch.
    pub pitch: u8,
    /// Velocity.
    pub velocity: f32,
}

/// Expands one placement of a clip into timeline notes, looping the clip if
/// it loops, honouring offset, length and note chance.
pub fn placement_notes(doc: &Document, p: &Placement) -> Vec<TimelineNote> {
    let Some(clip) = doc.clips.get(&p.clip) else { return Vec::new() };
    let Some(feed) = doc.inlet_cable(clip.id) else { return Vec::new() };
    let notes = sat_modules::effective_notes(doc, feed.from.module, clip.id);
    let len = clip.length.0;
    let (win_start, win_end) = (p.offset.0, p.offset.0 + p.length.0);
    let cycles = if clip.looped { ((win_end + len - 1) / len).max(1) } else { 1 };
    let clip_salt = (clip.id.0.as_u128() as u64) ^ clip.seed ^ doc.seed;
    let mut out = Vec::new();
    for k in 0..cycles {
        for (i, n) in notes.iter().enumerate() {
            if n.start.0 >= len {
                continue;
            }
            let ct = k * len + n.start.0;
            if ct < win_start || ct >= win_end {
                continue;
            }
            if n.chance < 1.0 {
                let h = mix(clip_salt ^ mix(k as u64) ^ (i as u64).wrapping_mul(0x1000_0001));
                if (h >> 40) as f32 / (1u64 << 24) as f32 >= n.chance {
                    continue;
                }
            }
            let start = p.start.0 + ct - win_start;
            // a note stops at the end of the clip cycle and of the placement
            let end = (start + n.len.0).min(p.start.0 + (k * len + len) - win_start).min(p.start.0 + p.length.0);
            out.push(TimelineNote { start: Ticks(start), end: Ticks(end), pitch: n.pitch, velocity: n.velocity });
        }
    }
    out
}

fn lane_automation(doc: &Document, target: &ParamTarget, tempo: &TempoMap, sr: f64) -> Option<Automation> {
    let lane = doc.lanes.values().find(|l| &l.target == target)?;
    if lane.points.is_empty() {
        return None;
    }
    Some(Automation::new(
        lane.points
            .iter()
            .map(|p| AutoPoint {
                at: tempo.tick_to_sample(p.tick, sr).round() as i64,
                value: p.value,
                curve: match p.curve {
                    Curve::Hold => AutoCurve::Hold,
                    Curve::Linear => AutoCurve::Linear,
                    Curve::Bend(b) => AutoCurve::Bend(b),
                },
            })
            .collect(),
    ))
}

/// Which tracks solo leaves audible; `None` when nothing is soloed.
fn solo_audible(doc: &Document) -> Option<BTreeSet<TrackId>> {
    let soloed: Vec<TrackId> = doc.tracks.values().filter(|t| t.mixer.solo).map(|t| t.id).collect();
    if soloed.is_empty() {
        return None;
    }
    let audible = doc
        .tracks
        .values()
        .filter(|t| {
            t.kind == TrackKind::Master
                || t.kind == TrackKind::Return
                || soloed.iter().any(|s| doc.is_within(t.id, *s) || doc.is_within(*s, t.id))
        })
        .map(|t| t.id)
        .collect();
    Some(audible)
}

fn vkind(kind: &str) -> Option<VKind> {
    Some(match kind {
        ins::NOTE_IN => VKind::NoteIn,
        ins::VCO => VKind::Vco,
        ins::NOISE => VKind::Noise,
        ins::MIXER => VKind::Mixer,
        ins::FILTER => VKind::Filter,
        ins::ADSR => VKind::Adsr,
        ins::VCA => VKind::Vca,
        ins::LFO => VKind::Lfo,
        ins::VOICE_OUT => VKind::VoiceOut,
        _ => return None,
    })
}

/// A module's parameter values in engine order, clamped to their ranges.
fn param_values(doc: &Document, m: &Module) -> [f32; MAX_PARAMS] {
    let m = &sat_modules::macros::resolved(doc, m);
    let mut v = [0.0; MAX_PARAMS];
    for (i, s) in sat_modules::params_of(&m.kind).iter().take(MAX_PARAMS).enumerate() {
        v[i] = s.clamp(m.params.get(s.key).copied().unwrap_or(s.default));
    }
    v
}

/// The modules of a grid that `keep` accepts, in dependency order (modules
/// caught in a cycle are left out), with a register for each connected
/// output: (module, input registers, output registers) in port order.
#[allow(clippy::type_complexity)]
fn graph(
    doc: &Document,
    grid: GridId,
    keep: impl Fn(&str) -> bool,
    max_mods: usize,
    max_regs: usize,
) -> Vec<(&Module, Vec<u8>, Vec<u8>)> {
    let mods: Vec<&Module> = doc.modules_on(grid).filter(|m| keep(&m.kind)).collect();
    let idx = |id: ModuleId| mods.iter().position(|m| m.id == id);
    // edges: (from module, from port) -> (to module, to port)
    let edges: Vec<(usize, &str, usize, &str)> = doc
        .cables_on(grid)
        .filter_map(|k| match &k.to {
            Endpoint::Port(p) => Some((idx(k.from.module)?, k.from.port.as_str(), idx(p.module)?, p.port.as_str())),
            Endpoint::Inlet(_) => None,
        })
        .collect();
    // Kahn's algorithm, lowest index first, for a stable order
    let mut indeg = vec![0usize; mods.len()];
    for (a, _, b, _) in &edges {
        if a != b {
            indeg[*b] += 1;
        }
    }
    let mut ready: BTreeSet<usize> = (0..mods.len()).filter(|&i| indeg[i] == 0).collect();
    let mut order = Vec::new();
    while let Some(i) = ready.pop_first() {
        order.push(i);
        for (a, _, b, _) in &edges {
            if *a == i && *b != i {
                indeg[*b] -= 1;
                if indeg[*b] == 0 {
                    ready.insert(*b);
                }
            }
        }
    }
    order.truncate(max_mods);
    let mut regs: HashMap<(usize, &str), u8> = HashMap::new();
    for (a, pa, b, _) in &edges {
        if order.contains(a) && order.contains(b) && !regs.contains_key(&(*a, *pa)) && regs.len() < max_regs {
            let r = regs.len() as u8;
            regs.insert((*a, *pa), r);
        }
    }
    order
        .iter()
        .map(|&i| {
            let m = mods[i];
            let ports = sat_modules::ports(&m.kind);
            let ins = ports
                .iter()
                .filter(|p| p.input)
                .map(|p| {
                    edges
                        .iter()
                        .find(|(a, _, b, pb)| *b == i && *pb == p.key && order.contains(a))
                        .and_then(|(a, pa, _, _)| regs.get(&(*a, *pa)).copied())
                        .unwrap_or(NONE)
                })
                .collect();
            let outs =
                ports.iter().filter(|p| !p.input).map(|p| regs.get(&(i, p.key)).copied().unwrap_or(NONE)).collect();
            (m, ins, outs)
        })
        .collect()
}

/// Builds a voice patch from the patch modules on `grid`, in dependency
/// order (modules caught in a cycle are left out), with each module's slot.
pub fn build_patch(doc: &Document, grid: GridId) -> Option<(Patch, Vec<ModuleId>)> {
    if !doc.modules_on(grid).any(|m| m.kind == ins::VOICE_OUT) {
        return None;
    }
    let mut out = Vec::new();
    let mut slots = Vec::new();
    for (m, i, o) in graph(doc, grid, |k| vkind(k).is_some(), MAX_MODS, MAX_REGS) {
        let mut vm = VModule::new(vkind(&m.kind)?);
        for (j, r) in i.iter().take(4).enumerate() {
            vm.ins[j] = *r;
        }
        for (j, r) in o.iter().take(3).enumerate() {
            vm.outs[j] = *r;
        }
        vm.params = param_values(doc, m);
        out.push(vm);
        slots.push(m.id);
    }
    Some((Patch { mods: out }, slots))
}

fn fx_kind(kind: &str) -> Option<FxKind> {
    use sat_modules::fx as f;
    Some(match kind {
        f::FX_IN => FxKind::In,
        f::FX_OUT => FxKind::Out,
        f::EQ => FxKind::Eq,
        f::COMP => FxKind::Comp,
        f::DELAY => FxKind::Delay,
        f::REVERB => FxKind::Reverb,
        f::DRIVE => FxKind::Drive,
        f::LIMITER => FxKind::Limiter,
        f::FX_FILTER => FxKind::Filter,
        f::SPLIT => FxKind::Split,
        f::MERGE => FxKind::Merge,
        _ => return None,
    })
}

/// Builds the fx chain of `grid`, in dependency order, with each module's
/// slot. `None` (audio passes through) unless the grid has both an `in` and
/// an `out`.
pub fn build_fx(doc: &Document, grid: GridId) -> Option<(Vec<FxModule>, Vec<ModuleId>)> {
    use sat_modules::fx as f;
    let has = |k: &str| doc.modules_on(grid).any(|m| m.kind == k);
    if !has(f::FX_IN) || !has(f::FX_OUT) {
        return None;
    }
    let mut out = Vec::new();
    let mut slots = Vec::new();
    for (m, i, o) in graph(doc, grid, |k| fx_kind(k).is_some(), MAX_FX, MAX_FX_REGS) {
        let mut fm = FxModule::new(fx_kind(&m.kind)?, m.id.0.as_u128() as u64);
        for (j, r) in i.iter().take(2).enumerate() {
            fm.ins[j] = *r;
        }
        for (j, r) in o.iter().take(2).enumerate() {
            fm.outs[j] = *r;
        }
        let v = param_values(doc, m);
        fm.params[..FX_PARAMS - 1].copy_from_slice(&v[..FX_PARAMS - 1]);
        fm.params[FX_PARAMS - 1] = if f::bypassed(m) { 1.0 } else { 0.0 };
        out.push(fm);
        slots.push(m.id);
    }
    Some((out, slots))
}

/// Live parameter targets of the modules in `slots`: every spec parameter,
/// plus the bypass switch of fx modules.
fn module_targets(doc: &Document, slots: &[ModuleId]) -> Vec<(ParamTarget, Param)> {
    let mut targets = Vec::new();
    for (slot, id) in slots.iter().enumerate() {
        let m = &doc.modules[id];
        let slot = slot as u8;
        for (index, s) in sat_modules::params_of(&m.kind).iter().take(MAX_PARAMS).enumerate() {
            let target = ParamTarget::Module { module: *id, param: s.key.into() };
            targets.push((target, Param::Module { slot, index: index as u8 }));
        }
        if sat_modules::fx::bypassable(&m.kind) {
            let target = ParamTarget::Module { module: *id, param: sat_modules::fx::BYPASS.into() };
            targets.push((target, Param::Module { slot, index: sat_modules::fx::BYPASS_INDEX }));
        }
    }
    targets
}

/// The instrument a track plays its notes through, and where each module
/// parameter lands in it.
fn build_instrument(
    doc: &Document,
    track: &Track,
    events: Vec<NoteEvent>,
    sounds: &Sounds,
) -> Option<(NodeKind, Vec<(ParamTarget, Param)>)> {
    let g = track.instrument_grid?;
    if let Some((patch, slots)) = build_patch(doc, g) {
        let targets = module_targets(doc, &slots);
        return Some((NodeKind::Instrument(Box::new(Instrument::patch(events, patch))), targets));
    }
    if let Some(m) = doc.modules_on(g).find(|m| m.kind == ins::DRUM_SYNTH) {
        let v = param_values(doc, m);
        let i = Instrument::drums(events, v[0], v[1], v[2]);
        return Some((NodeKind::Instrument(Box::new(i)), module_targets(doc, &[m.id])));
    }
    if let Some(m) = doc.modules_on(g).find(|m| m.kind == ins::SAMPLER) {
        let sound = m.audio.and_then(|a| sounds.get(&a)).cloned();
        let i = Instrument::sampler(events, sound, param_values(doc, m));
        return Some((NodeKind::Instrument(Box::new(i)), module_targets(doc, &[m.id])));
    }
    let m = doc.modules_on(g).find(|m| m.kind == TEST_SYNTH)?;
    let wave = if m.params.get("wave").copied().unwrap_or(0.0) >= 0.5 { Wave::Saw } else { Wave::Sine };
    let mut s = Synth::new(events, wave, m.params.get("level").copied().unwrap_or(0.25));
    for (key, sp) in [("attack", SynthParam::Attack), ("release", SynthParam::Release)] {
        if let Some(v) = m.params.get(key) {
            s.set_param(sp, *v);
        }
    }
    let targets = [
        ("level", SynthParam::Level),
        ("attack", SynthParam::Attack),
        ("release", SynthParam::Release),
        ("wave", SynthParam::Wave),
    ]
    .into_iter()
    .map(|(key, sp)| (ParamTarget::Module { module: m.id, param: key.into() }, Param::Synth(sp)))
    .collect();
    Some((NodeKind::Synth(Box::new(s)), targets))
}

/// Compiles `doc` for `sample_rate`, processing at most `max_block` frames
/// per run, with no sounds loaded (samplers are silent).
pub fn compile(doc: &Document, sample_rate: f64, max_block: usize) -> Result<Compiled, CompileError> {
    compile_with(doc, sample_rate, max_block, &Sounds::new())
}

/// Like [`compile`], with samplers playing from `sounds`.
pub fn compile_with(
    doc: &Document,
    sample_rate: f64,
    max_block: usize,
    sounds: &Sounds,
) -> Result<Compiled, CompileError> {
    let tempo = doc.tempo_map().map_err(CompileError::Tempo)?;
    let audible = solo_audible(doc);
    let mut b = ScheduleBuilder::new(sample_rate, max_block);
    let mut params: HashMap<ParamTarget, (NodeId, Param, Conv)> = HashMap::new();
    let mut meters: HashMap<TrackId, Arc<Meter>> = HashMap::new();
    // inputs of every bus: track strips, group strips and sends
    #[allow(clippy::type_complexity)]
    let mut bus_inputs: HashMap<TrackId, Vec<(NodeId, f32, Option<TrackId>)>> = HashMap::new();
    let tick_to_sample = |t: Ticks| tempo.tick_to_sample(t, sample_rate).round() as i64;

    for t in doc.tracks_in_order() {
        let tr = &doc.tracks[&t];
        let strip_id = nid(t, SALT_STRIP);
        // the node feeding the strip (and pre-fader sends)
        let pre: Option<NodeId> = match tr.kind {
            TrackKind::Track => {
                let mut events = Vec::new();
                for p in doc.placements.values().filter(|p| doc.clips.get(&p.clip).is_some_and(|c| c.track == t)) {
                    for n in placement_notes(doc, p) {
                        let at = tick_to_sample(n.start);
                        let len = (tick_to_sample(n.end) - at).max(1);
                        events.push(NoteEvent { at, len, pitch: n.pitch, velocity: n.velocity });
                    }
                }
                build_instrument(doc, tr, events, sounds).map(|(kind, targets)| {
                    let id = nid(t, SALT_SYNTH);
                    for (target, param) in targets {
                        params.insert(target, (id, param, Conv::Same));
                    }
                    b.node(id, kind, vec![]);
                    id
                })
            }
            TrackKind::Group | TrackKind::Return | TrackKind::Master => Some(nid(t, SALT_BUS)),
        };
        // the fx grid, between the track's sound and its strip
        let pre = match (pre, tr.fx_grids.front().and_then(|g| build_fx(doc, *g))) {
            (Some(src), Some((chain, slots))) => {
                let id = nid(t, SALT_FX);
                for (target, param) in module_targets(doc, &slots) {
                    params.insert(target, (id, param, Conv::Same));
                }
                b.node(id, NodeKind::Fx(Box::new(Fx::new(chain, sample_rate, max_block))), vec![(src, 1.0)]);
                Some(id)
            }
            (pre, _) => pre,
        };
        let solo_muted = audible.as_ref().is_some_and(|a| !a.contains(&t));
        let meter = meters.entry(t).or_default().clone();
        let strip = Strip::new(
            tr.mixer.gain_db,
            tr.mixer.pan,
            tr.mixer.mute || solo_muted,
            lane_automation(doc, &ParamTarget::Gain(t), &tempo, sample_rate),
        )
        .with_meter(meter);
        b.node(strip_id, NodeKind::Strip(Box::new(strip)), pre.map(|p| vec![(p, 1.0)]).unwrap_or_default());
        params.insert(ParamTarget::Gain(t), (strip_id, Param::GainDb, Conv::Same));
        params.insert(ParamTarget::Pan(t), (strip_id, Param::Pan, Conv::Same));

        if tr.kind != TrackKind::Master {
            let dest = match (tr.kind, tr.parent) {
                (TrackKind::Return, _) | (_, None) => doc.master,
                (_, Some(g)) => g,
            };
            bus_inputs.entry(dest).or_default().push((strip_id, 1.0, None));
        }
        for s in tr.sends.iter() {
            let src = if s.pre_fader { pre } else { Some(strip_id) };
            if let Some(src) = src {
                bus_inputs.entry(s.target).or_default().push((src, db_to_gain(s.level_db), Some(t)));
            }
        }
    }
    // buses, now that every input is known; a send's input slot becomes its level parameter
    for t in doc.tracks_in_order() {
        let tr = &doc.tracks[&t];
        if tr.kind == TrackKind::Track {
            continue;
        }
        let inputs = bus_inputs.remove(&t).unwrap_or_default();
        for (k, (_, _, sender)) in inputs.iter().enumerate() {
            if let Some(sender) = sender {
                params.insert(
                    ParamTarget::SendLevel { track: *sender, target: t },
                    (nid(t, SALT_BUS), Param::InputGain(k as u16), Conv::DbToLinear),
                );
            }
        }
        b.node(nid(t, SALT_BUS), NodeKind::Sum, inputs.into_iter().map(|(n, g, _)| (n, g)).collect());
    }
    // macros fan out to the parameters they drive
    let mut macros: HashMap<ParamTarget, Vec<_>> = HashMap::new();
    for g in doc.grids.values() {
        for (i, slot) in g.slots.iter() {
            for b in slot.binds.iter() {
                let target = ParamTarget::Module { module: b.module, param: b.param.clone() };
                let spec = doc.modules.get(&b.module).and_then(|m| sat_modules::spec(&m.kind, &b.param));
                if let (Some((node, param, _)), Some(spec)) = (params.get(&target), spec) {
                    macros.entry(ParamTarget::Macro { grid: g.id, index: *i }).or_default().push((
                        *node,
                        *param,
                        spec,
                        b.clone(),
                    ));
                }
            }
        }
    }
    let output = nid(doc.master, SALT_STRIP);
    let schedule = b.build(output).map_err(CompileError::Schedule)?;
    Ok(Compiled { schedule, output, tempo, params, macros, meters })
}
