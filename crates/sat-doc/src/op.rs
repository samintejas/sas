//! Every change to a project is an [`Op`]. [`apply`] is the reducer.

use im::{OrdMap, Vector};
use sat_time::{MeterChange, Ticks};
use serde::{Deserialize, Serialize};

use crate::doc::Document;
use crate::id::*;
use crate::model::*;

/// Why an op was refused. A refused op leaves the document untouched.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum OpError {
    /// The id is already in use.
    #[error("id already exists: {0}")]
    Exists(String),
    /// The id does not name anything.
    #[error("not found: {0}")]
    NotFound(String),
    /// The op does not apply to this kind of entity.
    #[error("not allowed: {0}")]
    NotAllowed(&'static str),
    /// A value is out of range or not finite.
    #[error("bad value: {0}")]
    BadValue(&'static str),
    /// Moving a group into itself.
    #[error("a group cannot contain itself")]
    Cycle,
    /// A dock inlet already has a path wired to it.
    #[error("clip inlet already wired")]
    InletTaken,
    /// The tempo or meter lane would be invalid.
    #[error(transparent)]
    Time(#[from] sat_time::TimeError),
}

/// What a track flag op sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackFlag {
    /// Mute.
    Mute,
    /// Solo.
    Solo,
    /// Record arm.
    Arm,
}

/// Grid ids for a new track. Tracks need all three; groups and returns only `fx`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct NewGrids {
    /// Source grid (tracks only).
    pub source: Option<GridId>,
    /// Instrument grid (tracks only).
    pub instrument: Option<GridId>,
    /// The first fx grid.
    pub fx: GridId,
}

/// A clip's editable properties; `None` leaves a field as it is.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ClipProps {
    /// New name.
    pub name: Option<String>,
    /// New colour.
    pub color: Option<Color>,
    /// New length.
    pub length: Option<Ticks>,
    /// Loop on or off.
    pub looped: Option<bool>,
    /// New seed.
    pub seed: Option<u64>,
}

/// One change to a project.
///
/// Ops carry every id they create, so a list of ops replays to exactly the
/// same document. Agents, scripts and the UI all edit through these.
#[allow(missing_docs)] // fields are named after what they hold
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    // --- tracks ---
    /// Adds a track, group or return under `parent` at `index` (clamped).
    AddTrack {
        id: TrackId,
        kind: TrackKind,
        name: String,
        color: Color,
        parent: Option<TrackId>,
        index: usize,
        grids: NewGrids,
    },
    /// Removes a track node, everything under it, and everything that refers to it.
    RemoveTrack { id: TrackId },
    /// Moves a node to `parent` at `index` (clamped).
    MoveTrack { id: TrackId, parent: Option<TrackId>, index: usize },
    /// Renames a node.
    RenameTrack { id: TrackId, name: String },
    /// Recolours a node.
    SetTrackColor { id: TrackId, color: Color },
    /// Sets mute, solo or arm.
    SetTrackFlag { id: TrackId, flag: TrackFlag, on: bool },
    /// Sets input monitoring.
    SetMonitor { id: TrackId, monitor: Monitor },
    /// Adds a send to a return.
    AddSend { track: TrackId, target: TrackId, level_db: f32, pre_fader: bool },
    /// Removes a send.
    RemoveSend { track: TrackId, target: TrackId },

    // --- grids and modules ---
    /// Adds an fx grid to a node at `index` in its fx chain (clamped).
    AddFxGrid { id: GridId, track: TrackId, index: usize },
    /// Removes an fx grid and its modules and cables.
    RemoveFxGrid { id: GridId },
    /// Adds a module to a grid.
    AddModule { id: ModuleId, grid: GridId, kind: String, name: String, params: OrdMap<String, f32>, pos: (f32, f32) },
    /// Removes a module, its cables and its patterns.
    RemoveModule { id: ModuleId },
    /// Moves a module on the canvas.
    MoveModule { id: ModuleId, pos: (f32, f32) },
    /// Wires an output port to a port or a dock inlet on the same grid.
    Connect { id: CableId, from: PortRef, to: Endpoint },
    /// Removes a cable.
    Disconnect { id: CableId },
    /// Adds a sound file to the audio pool.
    AddAudio { file: AudioFile },
    /// Removes a sound file from the pool; modules playing it go silent.
    RemoveAudio { id: AudioId },
    /// Sets (or clears) the sound a module plays.
    SetModuleAudio { module: ModuleId, audio: Option<AudioId> },

    // --- values ---
    /// Sets the base value of a parameter, macro, fader, pan or send level.
    SetValue { target: ParamTarget, value: f32 },
    /// Assigns (names and binds) or clears one of a grid's macros.
    SetMacroSlot { grid: GridId, index: u8, slot: Option<MacroSlot> },

    // --- clips and patterns ---
    /// Adds a clip (a dock slot) to a track.
    AddClip { id: ClipId, track: TrackId, name: String, color: Color, length: Ticks, looped: bool, seed: u64 },
    /// Removes a clip, its inlet cable, patterns, placements and slots.
    RemoveClip { id: ClipId },
    /// Changes a clip's properties.
    UpdateClip { id: ClipId, props: ClipProps },
    /// Copies a clip: its patterns and snapshot are duplicated, and a second
    /// cable is wired from the path feeding `src` to the new clip's inlet.
    DuplicateClip { src: ClipId, id: ClipId, cable: Option<CableId> },
    /// Sets (or clears, with `None`) a clip's snapshot value for a target.
    SetSnapshot { clip: ClipId, target: ParamTarget, value: Option<f32> },
    /// Replaces the notes a source module plays for a clip.
    WritePattern { module: ModuleId, clip: ClipId, notes: Vector<Note> },
    /// Replaces the hand-edit layer of a pattern.
    SetOverrides { module: ModuleId, clip: ClipId, overrides: Overrides },

    // --- arrangement and session ---
    /// Places a clip on the arrangement.
    AddPlacement { id: PlacementId, clip: ClipId, start: Ticks, offset: Ticks, length: Ticks },
    /// Moves or trims a placement.
    UpdatePlacement { id: PlacementId, start: Ticks, offset: Ticks, length: Ticks },
    /// Removes a placement (the clip stays).
    RemovePlacement { id: PlacementId },
    /// Adds a scene at `index` (clamped).
    AddScene { id: SceneId, name: String, index: usize },
    /// Removes a scene and its slots.
    RemoveScene { id: SceneId },
    /// Puts a clip in a session slot, or empties it with `None`.
    SetSlot { scene: SceneId, track: TrackId, clip: Option<ClipId> },

    // --- automation and time ---
    /// Adds an automation lane.
    AddLane { id: LaneId, target: ParamTarget },
    /// Replaces a lane's breakpoints (sorted by the reducer).
    SetLanePoints { id: LaneId, points: Vector<Breakpoint> },
    /// Removes a lane (not the tempo lane).
    RemoveLane { id: LaneId },
    /// Replaces master's time-signature changes.
    SetMeter { changes: Vector<MeterChange> },
    /// Sets the loop range.
    SetLoop { range: LoopRange },
}

/// What an applied op changed, so the app knows how to react.
#[allow(missing_docs)]
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    /// A value changed: send it to the engine as a parameter event.
    Value { target: ParamTarget, value: f32 },
    /// A pattern's notes changed: the engine rereads the pattern.
    Pattern { module: ModuleId, clip: ClipId },
    /// The structure changed: recompile the schedule.
    Structure,
    /// Something only the UI cares about (names, colours, positions).
    Cosmetic,
}

/// `retain` and in-place updates for persistent maps, which `im` lacks.
trait MapExt<K, V> {
    fn retain(&mut self, keep: impl FnMut(&K, &V) -> bool);
    fn update_where(&mut self, pred: impl FnMut(&V) -> bool, f: impl FnMut(&mut V));
}

impl<K: Ord + Clone, V: Clone> MapExt<K, V> for OrdMap<K, V> {
    fn retain(&mut self, mut keep: impl FnMut(&K, &V) -> bool) {
        let gone: Vec<K> = self.iter().filter(|(k, v)| !keep(k, v)).map(|(k, _)| k.clone()).collect();
        for k in gone {
            self.remove(&k);
        }
    }

    fn update_where(&mut self, mut pred: impl FnMut(&V) -> bool, mut f: impl FnMut(&mut V)) {
        let hit: Vec<K> = self.iter().filter(|(_, v)| pred(v)).map(|(k, _)| k.clone()).collect();
        for k in hit {
            if let Some(v) = self.get_mut(&k) {
                f(v);
            }
        }
    }
}

fn nf(what: &str, id: impl core::fmt::Display) -> OpError {
    OpError::NotFound(format!("{what} {id}"))
}

fn exists(what: &str, id: impl core::fmt::Display) -> OpError {
    OpError::Exists(format!("{what} {id}"))
}

fn finite(v: f32, what: &'static str) -> Result<(), OpError> {
    if v.is_finite() { Ok(()) } else { Err(OpError::BadValue(what)) }
}

fn track(d: &Document, id: TrackId) -> Result<&Track, OpError> {
    d.tracks.get(&id).ok_or_else(|| nf("track", id))
}

fn insert_at<T: Clone>(v: &mut Vector<T>, index: usize, x: T) {
    let i = index.min(v.len());
    v.insert(i, x);
}

fn remove_from<T: Clone + PartialEq>(v: &mut Vector<T>, x: &T) {
    if let Some(i) = v.index_of(x) {
        v.remove(i);
    }
}

fn children_mut(d: &mut Document, parent: Option<TrackId>) -> &mut Vector<TrackId> {
    match parent {
        None => &mut d.root,
        Some(p) => &mut d.tracks.get_mut(&p).expect("parent checked").children,
    }
}

fn check_parent(d: &Document, parent: Option<TrackId>) -> Result<(), OpError> {
    if let Some(p) = parent
        && track(d, p)?.kind != TrackKind::Group
    {
        return Err(OpError::NotAllowed("parent must be a group"));
    }
    Ok(())
}

fn check_grid_free(d: &Document, id: GridId) -> Result<(), OpError> {
    if d.grids.contains_key(&id) { Err(exists("grid", id)) } else { Ok(()) }
}

fn sort_notes(notes: &mut Vector<Note>) {
    notes.sort_by(|a, b| a.start.cmp(&b.start).then(a.pitch.cmp(&b.pitch)));
}

fn check_note(n: &Note) -> Result<(), OpError> {
    if n.pitch > 127 {
        return Err(OpError::BadValue("pitch above 127"));
    }
    if n.len.0 <= 0 || n.start.0 < 0 {
        return Err(OpError::BadValue("note start or length"));
    }
    if !(0.0..=1.0).contains(&n.velocity) || !(0.0..=1.0).contains(&n.chance) {
        return Err(OpError::BadValue("velocity and chance are 0–1"));
    }
    Ok(())
}

/// Removes a grid and everything on it.
fn drop_grid(d: &mut Document, g: GridId) {
    let mods: Vec<ModuleId> = d.modules_on(g).map(|m| m.id).collect();
    for m in mods {
        drop_module(d, m);
    }
    d.grids.remove(&g);
}

fn drop_module(d: &mut Document, m: ModuleId) {
    d.cables.retain(|_, c| c.from.module != m && !matches!(&c.to, Endpoint::Port(p) if p.module == m));
    d.patterns.retain(|(pm, _), _| *pm != m);
    d.lanes.retain(|_, l| !matches!(&l.target, ParamTarget::Module { module, .. } if *module == m));
    let hit = |t: &ParamTarget| matches!(t, ParamTarget::Module { module, .. } if *module == m);
    d.clips.update_where(|c| c.snapshot.keys().any(hit), |c| c.snapshot.retain(|t, _| !hit(t)));
    // macros stop driving it
    let drives = |s: &MacroSlot| s.binds.iter().any(|b| b.module == m);
    d.grids.update_where(
        |g| g.slots.values().any(drives),
        |g| g.slots.update_where(drives, |s| s.binds.retain(|b| b.module != m)),
    );
    d.modules.remove(&m);
}

fn drop_clip(d: &mut Document, c: ClipId) {
    d.cables.retain(|_, k| k.to != Endpoint::Inlet(c));
    d.patterns.retain(|(_, pc), _| *pc != c);
    d.placements.retain(|_, p| p.clip != c);
    d.slots.retain(|_, s| *s != c);
    d.clips.remove(&c);
}

fn target_mentions_track(t: &ParamTarget, d: &Document, track: TrackId) -> bool {
    match t {
        ParamTarget::Gain(x) | ParamTarget::Pan(x) => *x == track,
        ParamTarget::SendLevel { track: a, target: b } => *a == track || *b == track,
        ParamTarget::Macro { grid, .. } => d.grids.get(grid).is_none_or(|g| g.track == track),
        ParamTarget::Module { module, .. } => {
            d.modules.get(module).is_none_or(|m| d.grids.get(&m.grid).is_none_or(|g| g.track == track))
        }
        ParamTarget::Tempo => false,
    }
}

fn drop_track(d: &mut Document, id: TrackId) {
    let t = d.tracks[&id].clone();
    for c in t.children.iter() {
        drop_track(d, *c);
    }
    let clips: Vec<ClipId> = d.clips_on(id).map(|c| c.id).collect();
    for c in clips {
        drop_clip(d, c);
    }
    let lanes_gone: Vec<LaneId> =
        d.lanes.values().filter(|l| target_mentions_track(&l.target, d, id)).map(|l| l.id).collect();
    for l in lanes_gone {
        d.lanes.remove(&l);
    }
    for g in t.source_grid.into_iter().chain(t.instrument_grid).chain(t.fx_grids.iter().copied()) {
        drop_grid(d, g);
    }
    d.slots.retain(|(_, tr), _| *tr != id);
    d.tracks.update_where(|t| t.sends.iter().any(|s| s.target == id), |t| t.sends.retain(|s| s.target != id));
    remove_from(children_mut(d, t.parent), &id);
    d.tracks.remove(&id);
}

fn validate_value(target: &ParamTarget, value: f32) -> Result<(), OpError> {
    finite(value, "value must be finite")?;
    match target {
        ParamTarget::Macro { .. } if !(0.0..=1.0).contains(&value) => Err(OpError::BadValue("macros are 0–1")),
        ParamTarget::Pan(_) if !(-1.0..=1.0).contains(&value) => Err(OpError::BadValue("pan is -1–1")),
        ParamTarget::Tempo => Err(OpError::NotAllowed("tempo is set through the tempo lane")),
        _ => Ok(()),
    }
}

/// Applies one op to `d`, returning what changed. On error `d` may be partly
/// changed; callers apply transactions to a copy (see [`crate::History`]).
pub fn apply(d: &mut Document, op: Op) -> Result<Change, OpError> {
    use Change::*;
    Ok(match op {
        Op::AddTrack { id, kind, name, color, parent, index, grids } => {
            if d.tracks.contains_key(&id) {
                return Err(exists("track", id));
            }
            if kind == TrackKind::Master {
                return Err(OpError::NotAllowed("there is only one master"));
            }
            check_parent(d, parent)?;
            if kind == TrackKind::Return && parent.is_some() {
                return Err(OpError::NotAllowed("returns live at the top level"));
            }
            let (src, ins) = match (kind.has_clips(), grids.source, grids.instrument) {
                (true, Some(s), Some(i)) => (Some(s), Some(i)),
                (true, _, _) => return Err(OpError::NotAllowed("tracks need source and instrument grids")),
                (false, None, None) => (None, None),
                (false, _, _) => return Err(OpError::NotAllowed("only tracks have source and instrument grids")),
            };
            let mut new_grids = vec![(grids.fx, GridKind::Fx)];
            if let (Some(s), Some(i)) = (src, ins) {
                new_grids.push((s, GridKind::Source));
                new_grids.push((i, GridKind::Instrument));
            }
            for (i, (g, _)) in new_grids.iter().enumerate() {
                check_grid_free(d, *g)?;
                if new_grids[..i].iter().any(|(h, _)| h == g) {
                    return Err(exists("grid", g));
                }
            }
            for (g, k) in new_grids {
                d.grids.insert(g, Grid::new(g, id, k));
            }
            d.tracks.insert(
                id,
                Track {
                    id,
                    name,
                    kind,
                    parent,
                    children: Vector::new(),
                    color,
                    mixer: Mixer::default(),
                    sends: Vector::new(),
                    source_grid: src,
                    instrument_grid: ins,
                    fx_grids: Vector::unit(grids.fx),
                    armed: false,
                    monitor: if kind.has_clips() { Monitor::Auto } else { Monitor::Off },
                },
            );
            insert_at(children_mut(d, parent), index, id);
            Structure
        }
        Op::RemoveTrack { id } => {
            if track(d, id)?.kind == TrackKind::Master {
                return Err(OpError::NotAllowed("master cannot be removed"));
            }
            drop_track(d, id);
            Structure
        }
        Op::MoveTrack { id, parent, index } => {
            let t = track(d, id)?;
            if t.kind == TrackKind::Master {
                return Err(OpError::NotAllowed("master cannot move"));
            }
            if t.kind == TrackKind::Return && parent.is_some() {
                return Err(OpError::NotAllowed("returns live at the top level"));
            }
            check_parent(d, parent)?;
            if let Some(p) = parent
                && d.is_within(p, id)
            {
                return Err(OpError::Cycle);
            }
            let old = t.parent;
            remove_from(children_mut(d, old), &id);
            insert_at(children_mut(d, parent), index, id);
            d.tracks.get_mut(&id).unwrap().parent = parent;
            Structure
        }
        Op::RenameTrack { id, name } => {
            track(d, id)?;
            d.tracks.get_mut(&id).unwrap().name = name;
            Cosmetic
        }
        Op::SetTrackColor { id, color } => {
            track(d, id)?;
            d.tracks.get_mut(&id).unwrap().color = color;
            Cosmetic
        }
        Op::SetTrackFlag { id, flag, on } => {
            let t = track(d, id)?;
            if flag == TrackFlag::Arm && !t.kind.has_clips() {
                return Err(OpError::NotAllowed("only tracks can be armed"));
            }
            if flag == TrackFlag::Solo && t.kind == TrackKind::Master {
                return Err(OpError::NotAllowed("master cannot be soloed"));
            }
            let t = d.tracks.get_mut(&id).unwrap();
            match flag {
                TrackFlag::Mute => t.mixer.mute = on,
                TrackFlag::Solo => t.mixer.solo = on,
                TrackFlag::Arm => t.armed = on,
            }
            Structure
        }
        Op::SetMonitor { id, monitor } => {
            if !track(d, id)?.kind.has_clips() {
                return Err(OpError::NotAllowed("only tracks monitor input"));
            }
            d.tracks.get_mut(&id).unwrap().monitor = monitor;
            Structure
        }
        Op::AddSend { track: from, target, level_db, pre_fader } => {
            finite(level_db, "send level")?;
            let t = track(d, from)?;
            if t.kind == TrackKind::Master || t.kind == TrackKind::Return && from == target {
                return Err(OpError::NotAllowed("cannot send from here"));
            }
            if track(d, target)?.kind != TrackKind::Return {
                return Err(OpError::NotAllowed("sends go to returns"));
            }
            if t.kind == TrackKind::Return {
                return Err(OpError::NotAllowed("returns do not send"));
            }
            if t.sends.iter().any(|s| s.target == target) {
                return Err(exists("send to", target));
            }
            d.tracks.get_mut(&from).unwrap().sends.push_back(Send { target, level_db, pre_fader });
            Structure
        }
        Op::RemoveSend { track: from, target } => {
            let t = track(d, from)?;
            if !t.sends.iter().any(|s| s.target == target) {
                return Err(nf("send to", target));
            }
            d.tracks.get_mut(&from).unwrap().sends.retain(|s| s.target != target);
            d.lanes.retain(|_, l| l.target != ParamTarget::SendLevel { track: from, target });
            Structure
        }
        Op::AddFxGrid { id, track: t, index } => {
            track(d, t)?;
            check_grid_free(d, id)?;
            d.grids.insert(id, Grid::new(id, t, GridKind::Fx));
            insert_at(&mut d.tracks.get_mut(&t).unwrap().fx_grids, index, id);
            Structure
        }
        Op::RemoveFxGrid { id } => {
            let g = d.grids.get(&id).ok_or_else(|| nf("grid", id))?;
            if g.kind != GridKind::Fx {
                return Err(OpError::NotAllowed("only fx grids can be removed"));
            }
            let t = g.track;
            remove_from(&mut d.tracks.get_mut(&t).unwrap().fx_grids, &id);
            d.lanes.retain(|_, l| !matches!(l.target, ParamTarget::Macro { grid, .. } if grid == id));
            let hit = |k: &ParamTarget| matches!(k, ParamTarget::Macro { grid, .. } if *grid == id);
            d.clips.update_where(|c| c.snapshot.keys().any(hit), |c| c.snapshot.retain(|k, _| !hit(k)));
            drop_grid(d, id);
            Structure
        }
        Op::AddModule { id, grid, kind, name, params, pos } => {
            if d.modules.contains_key(&id) {
                return Err(exists("module", id));
            }
            if !d.grids.contains_key(&grid) {
                return Err(nf("grid", grid));
            }
            for v in params.values() {
                finite(*v, "parameter")?;
            }
            d.modules.insert(id, Module { id, grid, kind, name, params, pos, audio: None });
            Structure
        }
        Op::RemoveModule { id } => {
            if !d.modules.contains_key(&id) {
                return Err(nf("module", id));
            }
            drop_module(d, id);
            Structure
        }
        Op::AddAudio { file } => {
            if d.audio.contains_key(&file.id) {
                return Err(exists("audio file", file.id));
            }
            if file.file.is_empty() || file.file.contains(['/', '\\']) || file.file.starts_with('.') {
                return Err(OpError::BadValue("audio file names are plain names inside audio/"));
            }
            if file.rate == 0 || !(1..=2).contains(&file.channels) {
                return Err(OpError::BadValue("audio needs a rate and 1 or 2 channels"));
            }
            d.audio.insert(file.id, file);
            Structure
        }
        Op::RemoveAudio { id } => {
            if d.audio.remove(&id).is_none() {
                return Err(nf("audio file", id));
            }
            d.modules.update_where(|m| m.audio == Some(id), |m| m.audio = None);
            Structure
        }
        Op::SetModuleAudio { module, audio } => {
            if let Some(a) = audio
                && !d.audio.contains_key(&a)
            {
                return Err(nf("audio file", a));
            }
            d.modules.get_mut(&module).ok_or_else(|| nf("module", module))?.audio = audio;
            Structure
        }
        Op::MoveModule { id, pos } => {
            let m = d.modules.get_mut(&id).ok_or_else(|| nf("module", id))?;
            m.pos = pos;
            Cosmetic
        }
        Op::Connect { id, from, to } => {
            if d.cables.contains_key(&id) {
                return Err(exists("cable", id));
            }
            let grid = d.modules.get(&from.module).ok_or_else(|| nf("module", from.module))?.grid;
            match &to {
                Endpoint::Port(p) => {
                    let g2 = d.modules.get(&p.module).ok_or_else(|| nf("module", p.module))?.grid;
                    if g2 != grid {
                        return Err(OpError::NotAllowed("cables stay on one grid"));
                    }
                    if p.module == from.module {
                        return Err(OpError::NotAllowed("a module cannot feed itself"));
                    }
                }
                Endpoint::Inlet(c) => {
                    let clip = d.clips.get(c).ok_or_else(|| nf("clip", c))?;
                    if d.tracks[&clip.track].source_grid != Some(grid) {
                        return Err(OpError::NotAllowed("inlets are on the clip's own source grid"));
                    }
                    if d.inlet_cable(*c).is_some() {
                        return Err(OpError::InletTaken);
                    }
                }
            }
            if d.cables.values().any(|c| c.from == from && c.to == to) {
                return Err(OpError::NotAllowed("already connected"));
            }
            d.cables.insert(id, Cable { id, grid, from, to });
            Structure
        }
        Op::Disconnect { id } => {
            d.cables.remove(&id).ok_or_else(|| nf("cable", id))?;
            Structure
        }
        Op::SetValue { target, value } => {
            validate_value(&target, value)?;
            match &target {
                ParamTarget::Module { module, param } => {
                    let m = d.modules.get_mut(module).ok_or_else(|| nf("module", module))?;
                    m.params.insert(param.clone(), value);
                }
                ParamTarget::Macro { grid, index } => {
                    let g = d.grids.get_mut(grid).ok_or_else(|| nf("grid", grid))?;
                    *g.macros.get_mut(*index as usize).ok_or(OpError::BadValue("macro index"))? = value;
                }
                ParamTarget::Gain(t) => d.tracks.get_mut(t).ok_or_else(|| nf("track", t))?.mixer.gain_db = value,
                ParamTarget::Pan(t) => d.tracks.get_mut(t).ok_or_else(|| nf("track", t))?.mixer.pan = value,
                ParamTarget::SendLevel { track: t, target: r } => {
                    let tr = d.tracks.get_mut(t).ok_or_else(|| nf("track", t))?;
                    let i = tr.sends.iter().position(|s| s.target == *r).ok_or_else(|| nf("send to", r))?;
                    tr.sends[i].level_db = value;
                }
                ParamTarget::Tempo => unreachable!("validated"),
            }
            Value { target, value }
        }
        Op::SetMacroSlot { grid, index, slot } => {
            if index as usize >= MACROS {
                return Err(OpError::BadValue("macro index"));
            }
            if let Some(s) = &slot {
                for b in s.binds.iter() {
                    let m = d.modules.get(&b.module).ok_or_else(|| nf("module", b.module))?;
                    if m.grid != grid {
                        return Err(OpError::NotAllowed("macros drive modules on their own grid"));
                    }
                    finite(b.lo, "macro range must be finite")?;
                    finite(b.hi, "macro range must be finite")?;
                    if !(0.0..=1.0).contains(&b.lo) || !(0.0..=1.0).contains(&b.hi) {
                        return Err(OpError::BadValue("macro ranges are knob positions, 0–1"));
                    }
                }
            }
            let g = d.grids.get_mut(&grid).ok_or_else(|| nf("grid", grid))?;
            match slot {
                Some(s) => {
                    g.slots.insert(index, s);
                }
                None => {
                    g.slots.remove(&index);
                }
            }
            Structure
        }
        Op::AddClip { id, track: t, name, color, length, looped, seed } => {
            if d.clips.contains_key(&id) {
                return Err(exists("clip", id));
            }
            if !track(d, t)?.kind.has_clips() {
                return Err(OpError::NotAllowed("only tracks hold clips"));
            }
            if length.0 <= 0 {
                return Err(OpError::BadValue("clip length"));
            }
            d.clips.insert(id, Clip { id, track: t, name, color, length, looped, snapshot: OrdMap::new(), seed });
            Structure
        }
        Op::RemoveClip { id } => {
            if !d.clips.contains_key(&id) {
                return Err(nf("clip", id));
            }
            drop_clip(d, id);
            Structure
        }
        Op::UpdateClip { id, props } => {
            let c = d.clips.get_mut(&id).ok_or_else(|| nf("clip", id))?;
            let structural = props.length.is_some() || props.looped.is_some() || props.seed.is_some();
            if let Some(l) = props.length {
                if l.0 <= 0 {
                    return Err(OpError::BadValue("clip length"));
                }
                c.length = l;
            }
            if let Some(n) = props.name {
                c.name = n;
            }
            if let Some(col) = props.color {
                c.color = col;
            }
            if let Some(lp) = props.looped {
                c.looped = lp;
            }
            if let Some(s) = props.seed {
                c.seed = s;
            }
            if structural { Structure } else { Cosmetic }
        }
        Op::DuplicateClip { src, id, cable } => {
            if d.clips.contains_key(&id) {
                return Err(exists("clip", id));
            }
            let mut c = d.clips.get(&src).ok_or_else(|| nf("clip", src))?.clone();
            c.id = id;
            c.name = format!("{} copy", c.name);
            let feed = d.inlet_cable(src).cloned();
            match (feed, cable) {
                (Some(f), Some(cid)) => {
                    if d.cables.contains_key(&cid) {
                        return Err(exists("cable", cid));
                    }
                    d.cables.insert(cid, Cable { id: cid, grid: f.grid, from: f.from, to: Endpoint::Inlet(id) });
                }
                (Some(_), None) => return Err(OpError::NotAllowed("a wired clip's copy needs a cable id")),
                (None, _) => {}
            }
            let pats: Vec<(ModuleId, crate::model::Pattern)> =
                d.patterns.iter().filter(|((_, pc), _)| *pc == src).map(|((m, _), p)| (*m, p.clone())).collect();
            for (m, p) in pats {
                d.patterns.insert((m, id), p);
            }
            d.clips.insert(id, c);
            Structure
        }
        Op::SetSnapshot { clip, target, value } => {
            if let Some(v) = value {
                validate_value(&target, v)?;
            }
            let c = d.clips.get_mut(&clip).ok_or_else(|| nf("clip", clip))?;
            match value {
                Some(v) => {
                    c.snapshot.insert(target, v);
                }
                None => {
                    c.snapshot.remove(&target);
                }
            }
            Structure
        }
        Op::WritePattern { module, clip, mut notes } => {
            let m = d.modules.get(&module).ok_or_else(|| nf("module", module))?;
            let c = d.clips.get(&clip).ok_or_else(|| nf("clip", clip))?;
            if d.grids[&m.grid].kind != GridKind::Source || d.tracks[&c.track].source_grid != Some(m.grid) {
                return Err(OpError::NotAllowed("patterns belong to source modules on the clip's track"));
            }
            for n in notes.iter() {
                check_note(n)?;
            }
            sort_notes(&mut notes);
            match d.patterns.get_mut(&(module, clip)) {
                Some(p) => p.notes = notes,
                None => {
                    d.patterns.insert((module, clip), crate::model::Pattern { notes, overrides: Overrides::default() });
                }
            }
            Pattern { module, clip }
        }
        Op::SetOverrides { module, clip, overrides } => {
            for n in overrides.added.iter() {
                check_note(n)?;
            }
            let p = d.patterns.get_mut(&(module, clip)).ok_or_else(|| nf("pattern for clip", clip))?;
            p.overrides = overrides;
            Pattern { module, clip }
        }
        Op::AddPlacement { id, clip, start, offset, length } => {
            if d.placements.contains_key(&id) {
                return Err(exists("placement", id));
            }
            if !d.clips.contains_key(&clip) {
                return Err(nf("clip", clip));
            }
            check_span(start, offset, length)?;
            d.placements.insert(id, Placement { id, clip, start, offset, length });
            Structure
        }
        Op::UpdatePlacement { id, start, offset, length } => {
            check_span(start, offset, length)?;
            let p = d.placements.get_mut(&id).ok_or_else(|| nf("placement", id))?;
            p.start = start;
            p.offset = offset;
            p.length = length;
            Structure
        }
        Op::RemovePlacement { id } => {
            d.placements.remove(&id).ok_or_else(|| nf("placement", id))?;
            Structure
        }
        Op::AddScene { id, name, index } => {
            if d.scenes.iter().any(|s| s.id == id) {
                return Err(exists("scene", id));
            }
            insert_at(&mut d.scenes, index, Scene { id, name });
            Structure
        }
        Op::RemoveScene { id } => {
            let i = d.scenes.iter().position(|s| s.id == id).ok_or_else(|| nf("scene", id))?;
            d.scenes.remove(i);
            d.slots.retain(|(s, _), _| *s != id);
            Structure
        }
        Op::SetSlot { scene, track: t, clip } => {
            if !d.scenes.iter().any(|s| s.id == scene) {
                return Err(nf("scene", scene));
            }
            track(d, t)?;
            match clip {
                Some(c) => {
                    let cl = d.clips.get(&c).ok_or_else(|| nf("clip", c))?;
                    if cl.track != t {
                        return Err(OpError::NotAllowed("a slot holds a clip of its own track"));
                    }
                    d.slots.insert((scene, t), c);
                }
                None => {
                    d.slots.remove(&(scene, t));
                }
            }
            Structure
        }
        Op::AddLane { id, target } => {
            if d.lanes.contains_key(&id) {
                return Err(exists("lane", id));
            }
            if target == ParamTarget::Tempo {
                return Err(OpError::NotAllowed("there is one tempo lane"));
            }
            if d.lanes.values().any(|l| l.target == target) {
                return Err(OpError::NotAllowed("target already has a lane"));
            }
            if d.base_value(&target).is_none() {
                return Err(nf("automation target", format!("{target:?}")));
            }
            d.lanes.insert(id, Lane { id, target, points: Vector::new() });
            Structure
        }
        Op::SetLanePoints { id, mut points } => {
            let lane = d.lanes.get(&id).ok_or_else(|| nf("lane", id))?;
            for p in points.iter() {
                finite(p.value, "breakpoint value")?;
                if let Curve::Bend(b) = p.curve
                    && !(-1.0..=1.0).contains(&b)
                {
                    return Err(OpError::BadValue("bend is -1–1"));
                }
            }
            points.sort_by(|a, b| a.tick.cmp(&b.tick));
            let is_tempo = lane.target == ParamTarget::Tempo;
            d.lanes.get_mut(&id).unwrap().points = points;
            if is_tempo {
                d.tempo_map()?;
            }
            Structure
        }
        Op::RemoveLane { id } => {
            if id == d.tempo_lane {
                return Err(OpError::NotAllowed("the tempo lane stays"));
            }
            d.lanes.remove(&id).ok_or_else(|| nf("lane", id))?;
            Structure
        }
        Op::SetMeter { changes } => {
            let v: Vec<MeterChange> = changes.iter().copied().collect();
            sat_time::MeterMap::new(&v)?;
            d.meter = changes;
            Structure
        }
        Op::SetLoop { range } => {
            if range.end <= range.start {
                return Err(OpError::BadValue("loop end must be after start"));
            }
            d.loop_range = range;
            Structure
        }
    })
}

fn check_span(start: Ticks, offset: Ticks, length: Ticks) -> Result<(), OpError> {
    if start.0 < 0 || offset.0 < 0 || length.0 <= 0 {
        return Err(OpError::BadValue("placement start, offset or length"));
    }
    Ok(())
}

impl Op {
    /// Adds a track with fresh ids for it and its grids.
    pub fn add_track(name: impl Into<String>, parent: Option<TrackId>, index: usize) -> (TrackId, Op) {
        let id = TrackId::new();
        let grids = NewGrids { source: Some(GridId::new()), instrument: Some(GridId::new()), fx: GridId::new() };
        (id, Op::AddTrack { id, kind: TrackKind::Track, name: name.into(), color: Color(0), parent, index, grids })
    }

    /// Adds a group or return with fresh ids.
    pub fn add_bus(kind: TrackKind, name: impl Into<String>, parent: Option<TrackId>, index: usize) -> (TrackId, Op) {
        let id = TrackId::new();
        let grids = NewGrids { source: None, instrument: None, fx: GridId::new() };
        (id, Op::AddTrack { id, kind, name: name.into(), color: Color(0), parent, index, grids })
    }
}
