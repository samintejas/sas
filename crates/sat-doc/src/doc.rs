//! The project document: one immutable value, cheap to clone.

use im::{OrdMap, Vector};
use sat_time::{MeterChange, MeterMap, TempoMap, TempoPoint, TempoRamp, Ticks, TimeError, TimeSig};
use serde::{Deserialize, Serialize};

use crate::id::*;
use crate::model::*;

/// Tempo of a new project.
pub const DEFAULT_BPM: f32 = 120.0;

/// A whole project.
///
/// Every table is a persistent map, so a clone shares almost everything with
/// the original: the UI can take a copy every frame, and undo keeps old
/// versions around for the price of the parts that changed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Document {
    /// Project seed; generators derive theirs from it and the clip id.
    pub seed: u64,
    /// The master track.
    pub master: TrackId,
    /// The tempo lane on master.
    pub tempo_lane: LaneId,
    /// Time-signature changes on master.
    pub meter: Vector<MeterChange>,
    /// Top-level tracks, groups and returns in display order (master excluded).
    pub root: Vector<TrackId>,
    /// All track nodes.
    pub tracks: OrdMap<TrackId, Track>,
    /// All grids.
    pub grids: OrdMap<GridId, Grid>,
    /// All modules.
    pub modules: OrdMap<ModuleId, Module>,
    /// All cables.
    pub cables: OrdMap<CableId, Cable>,
    /// All clips.
    pub clips: OrdMap<ClipId, Clip>,
    /// Patterns keyed by (source module, clip).
    #[serde(with = "entries")]
    pub patterns: OrdMap<(ModuleId, ClipId), Pattern>,
    /// Arrangement placements.
    pub placements: OrdMap<PlacementId, Placement>,
    /// Session scenes in order.
    pub scenes: Vector<Scene>,
    /// Session slots keyed by (scene, track).
    #[serde(with = "entries")]
    pub slots: OrdMap<(SceneId, TrackId), ClipId>,
    /// Automation lanes.
    pub lanes: OrdMap<LaneId, Lane>,
    /// Loop range.
    pub loop_range: LoopRange,
    /// The audio pool: sound files the project uses.
    #[serde(default)]
    pub audio: OrdMap<AudioId, AudioFile>,
    /// The project's key (C major in projects saved before it existed).
    #[serde(default)]
    pub key: Key,
}

impl Document {
    /// An empty project: master with one fx grid, a tempo lane at 120 bpm, 4/4.
    pub fn new(seed: u64) -> Document {
        Document::with_ids(seed, TrackId::new(), GridId::new(), LaneId::new())
    }

    /// Like [`Document::new`] with fixed ids, for tests and reproducible files.
    pub fn with_ids(seed: u64, master: TrackId, master_fx: GridId, tempo_lane: LaneId) -> Document {
        let master_track = Track {
            id: master,
            name: "master".into(),
            kind: TrackKind::Master,
            parent: None,
            children: Vector::new(),
            color: Color(0),
            mixer: Mixer::default(),
            sends: Vector::new(),
            source_grid: None,
            instrument_grid: None,
            fx_grids: Vector::unit(master_fx),
            armed: false,
            monitor: Monitor::Off,
            frozen: None,
        };
        let fx = Grid::new(master_fx, master, GridKind::Fx);
        let tempo = Lane {
            id: tempo_lane,
            target: ParamTarget::Tempo,
            points: Vector::unit(Breakpoint { tick: Ticks::ZERO, value: DEFAULT_BPM, curve: Curve::Hold }),
        };
        Document {
            seed,
            master,
            tempo_lane,
            meter: Vector::unit(MeterChange { tick: Ticks::ZERO, sig: TimeSig::FOUR_FOUR }),
            root: Vector::new(),
            tracks: OrdMap::unit(master, master_track),
            grids: OrdMap::unit(master_fx, fx),
            modules: OrdMap::new(),
            cables: OrdMap::new(),
            clips: OrdMap::new(),
            patterns: OrdMap::new(),
            placements: OrdMap::new(),
            scenes: Vector::new(),
            slots: OrdMap::new(),
            lanes: OrdMap::unit(tempo_lane, tempo),
            loop_range: LoopRange { start: Ticks::ZERO, end: Ticks::quarters(16), enabled: false },
            audio: OrdMap::new(),
            key: Key::default(),
        }
    }

    /// The master track.
    pub fn master_track(&self) -> &Track {
        &self.tracks[&self.master]
    }

    /// The children of `parent` (`None` = top level), in order.
    pub fn children_of(&self, parent: Option<TrackId>) -> &Vector<TrackId> {
        match parent {
            None => &self.root,
            Some(p) => &self.tracks[&p].children,
        }
    }

    /// Every track node in display order, depth first, master last.
    pub fn tracks_in_order(&self) -> Vec<TrackId> {
        fn walk(d: &Document, ids: &Vector<TrackId>, out: &mut Vec<TrackId>) {
            for id in ids {
                out.push(*id);
                walk(d, &d.tracks[id].children, out);
            }
        }
        let mut out = Vec::with_capacity(self.tracks.len());
        walk(self, &self.root, &mut out);
        out.push(self.master);
        out
    }

    /// Whether `node` is `ancestor` or sits somewhere under it.
    pub fn is_within(&self, node: TrackId, ancestor: TrackId) -> bool {
        let mut cur = Some(node);
        while let Some(c) = cur {
            if c == ancestor {
                return true;
            }
            cur = self.tracks.get(&c).and_then(|t| t.parent);
        }
        false
    }

    /// The modules on a grid.
    pub fn modules_on(&self, grid: GridId) -> impl Iterator<Item = &Module> {
        self.modules.values().filter(move |m| m.grid == grid)
    }

    /// The cables on a grid.
    pub fn cables_on(&self, grid: GridId) -> impl Iterator<Item = &Cable> {
        self.cables.values().filter(move |c| c.grid == grid)
    }

    /// The clips on a track.
    pub fn clips_on(&self, track: TrackId) -> impl Iterator<Item = &Clip> {
        self.clips.values().filter(move |c| c.track == track)
    }

    /// The cable feeding a clip's dock inlet, if it is wired.
    pub fn inlet_cable(&self, clip: ClipId) -> Option<&Cable> {
        self.cables.values().find(|c| c.to == Endpoint::Inlet(clip))
    }

    /// The pattern a source module plays for a clip.
    pub fn pattern(&self, module: ModuleId, clip: ClipId) -> Option<&Pattern> {
        self.patterns.get(&(module, clip))
    }

    /// The base value of a parameter (no snapshot, automation or modulation).
    pub fn base_value(&self, target: &ParamTarget) -> Option<f32> {
        match target {
            ParamTarget::Module { module, param } => self.modules.get(module)?.params.get(param).copied(),
            ParamTarget::Macro { grid, index } => self.grids.get(grid)?.macros.get(*index as usize).copied(),
            ParamTarget::Gain(t) => Some(self.tracks.get(t)?.mixer.gain_db),
            ParamTarget::Pan(t) => Some(self.tracks.get(t)?.mixer.pan),
            ParamTarget::SendLevel { track, target } => {
                self.tracks.get(track)?.sends.iter().find(|s| s.target == *target).map(|s| s.level_db)
            }
            ParamTarget::Tempo => self.tempo_lane().points.front().map(|p| p.value),
        }
    }

    /// The tempo lane.
    pub fn tempo_lane(&self) -> &Lane {
        &self.lanes[&self.tempo_lane]
    }

    /// Compiles master's tempo lane into a tempo map.
    pub fn tempo_map(&self) -> Result<TempoMap, TimeError> {
        let pts: Vec<TempoPoint> = self
            .tempo_lane()
            .points
            .iter()
            .map(|p| TempoPoint {
                tick: p.tick,
                bpm: p.value as f64,
                ramp: match p.curve {
                    Curve::Hold => TempoRamp::Step,
                    Curve::Linear | Curve::Bend(_) => TempoRamp::Linear,
                },
            })
            .collect();
        TempoMap::new(&pts)
    }

    /// Compiles master's time-signature changes into a meter map.
    pub fn meter_map(&self) -> Result<MeterMap, TimeError> {
        let v: Vec<MeterChange> = self.meter.iter().copied().collect();
        MeterMap::new(&v)
    }
}
