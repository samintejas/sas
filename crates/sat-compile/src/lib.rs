//! Compiles a sat [`Document`] into an engine [`Schedule`].
//!
//! The compiler walks the track tree bottom-up: each track becomes a source
//! of note events feeding its instrument, then a mixer strip that feeds its
//! group's bus (or master); groups and returns are a bus plus a strip;
//! master's strip is the output. Node ids are derived from document ids, so
//! state carries over when a recompiled schedule replaces the running one.
//!
//! Phase 1 scope: the instrument is the built-in test synth (a module of
//! kind [`TEST_SYNTH`] on the instrument grid) and a clip plays the pattern
//! of the source module wired straight into its dock inlet. Processors, fx
//! modules and session launching arrive with later phases.

#![warn(missing_docs)]

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use sat_doc::sat_time::{TempoMap, Ticks};
use sat_doc::*;
use sat_engine::{
    AutoCurve, AutoPoint, Automation, Command, Meter, NodeId, NodeKind, NoteEvent, Param, Schedule, ScheduleBuilder,
    Strip, Synth, SynthParam, Wave, db_to_gain,
};

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
    meters: HashMap<TrackId, Arc<Meter>>,
}

impl Compiled {
    /// The post-fader peak meter of a track's strip.
    pub fn meter(&self, track: TrackId) -> Option<&Arc<Meter>> {
        self.meters.get(&track)
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

fn nid(t: TrackId, salt: u128) -> NodeId {
    NodeId(t.0.as_u128() ^ salt)
}

/// Deterministic hash for note chance (splitmix64).
fn mix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

/// The notes a clip actually plays: the pattern minus hand-removed notes plus
/// hand-added ones.
fn effective_notes(p: &Pattern) -> Vec<Note> {
    let mut v: Vec<Note> =
        p.notes.iter().filter(|n| !p.overrides.removed.contains(&(n.start, n.pitch))).copied().collect();
    v.extend(p.overrides.added.iter().copied());
    v.sort_by(|a, b| a.start.cmp(&b.start).then(a.pitch.cmp(&b.pitch)));
    v
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
    let Some(pat) = doc.pattern(feed.from.module, clip.id) else { return Vec::new() };
    let notes = effective_notes(pat);
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

fn find_synth<'a>(doc: &'a Document, track: &Track) -> Option<&'a Module> {
    let g = track.instrument_grid?;
    doc.modules_on(g).find(|m| m.kind == TEST_SYNTH)
}

/// Compiles `doc` for `sample_rate`, processing at most `max_block` frames per run.
pub fn compile(doc: &Document, sample_rate: f64, max_block: usize) -> Result<Compiled, CompileError> {
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
            TrackKind::Track => find_synth(doc, tr).map(|m| {
                let mut events = Vec::new();
                for p in doc.placements.values().filter(|p| doc.clips.get(&p.clip).is_some_and(|c| c.track == t)) {
                    for n in placement_notes(doc, p) {
                        let at = tick_to_sample(n.start);
                        let len = (tick_to_sample(n.end) - at).max(1);
                        events.push(NoteEvent { at, len, pitch: n.pitch, velocity: n.velocity });
                    }
                }
                let wave = if m.params.get("wave").copied().unwrap_or(0.0) >= 0.5 { Wave::Saw } else { Wave::Sine };
                let mut s = Synth::new(events, wave, m.params.get("level").copied().unwrap_or(0.25));
                let id = nid(t, SALT_SYNTH);
                for (key, sp) in [("attack", SynthParam::Attack), ("release", SynthParam::Release)] {
                    if let Some(v) = m.params.get(key) {
                        s.set_param(sp, *v);
                    }
                }
                for (key, sp) in [
                    ("level", SynthParam::Level),
                    ("attack", SynthParam::Attack),
                    ("release", SynthParam::Release),
                    ("wave", SynthParam::Wave),
                ] {
                    params.insert(
                        ParamTarget::Module { module: m.id, param: key.into() },
                        (id, Param::Synth(sp), Conv::Same),
                    );
                }
                b.node(id, NodeKind::Synth(Box::new(s)), vec![]);
                id
            }),
            TrackKind::Group | TrackKind::Return | TrackKind::Master => Some(nid(t, SALT_BUS)),
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
    let output = nid(doc.master, SALT_STRIP);
    let schedule = b.build(output).map_err(CompileError::Schedule)?;
    Ok(Compiled { schedule, output, tempo, params, meters })
}
