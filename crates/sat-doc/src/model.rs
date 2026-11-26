//! The entities a project is made of.
//!
//! These mirror the "Entities" table of the architecture doc. They are plain
//! data; every change goes through an [`Op`](crate::Op).

use im::{OrdMap, OrdSet, Vector};
use sat_time::Ticks;
use serde::{Deserialize, Serialize};

use crate::id::*;

/// What a track node is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackKind {
    /// A track with a source grid, an instrument grid and fx grids.
    Track,
    /// A group; holds child tracks and groups, and has fx grids.
    Group,
    /// A return; fed by sends, has fx grids.
    Return,
    /// The master bus. There is exactly one.
    Master,
}

impl TrackKind {
    /// Whether this kind owns a source grid, an instrument grid and clips.
    pub fn has_clips(self) -> bool {
        self == TrackKind::Track
    }
}

/// A palette index into the theme's track colours.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Color(pub u8);

/// Input monitoring, as on the track header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Monitor {
    /// Always monitor input.
    In,
    /// Monitor while armed and not playing back.
    #[default]
    Auto,
    /// Never monitor.
    Off,
}

/// A track's mixer strip.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Mixer {
    /// Fader gain in dB.
    pub gain_db: f32,
    /// Pan, -1 (left) to 1 (right).
    pub pan: f32,
    /// Muted.
    pub mute: bool,
    /// Soloed.
    pub solo: bool,
}

impl Default for Mixer {
    fn default() -> Self {
        Mixer { gain_db: 0.0, pan: 0.0, mute: false, solo: false }
    }
}

/// A send from a track to a return.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Send {
    /// The return track it feeds.
    pub target: TrackId,
    /// Send level in dB.
    pub level_db: f32,
    /// Taken before the fader.
    pub pre_fader: bool,
}

/// A node in the track tree.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Track {
    /// Its id.
    pub id: TrackId,
    /// Display name.
    pub name: String,
    /// Track, group, return or master.
    pub kind: TrackKind,
    /// The group it sits in; `None` for top level (and for master).
    pub parent: Option<TrackId>,
    /// Child tracks in order (groups only).
    pub children: Vector<TrackId>,
    /// Colour.
    pub color: Color,
    /// Mixer strip.
    pub mixer: Mixer,
    /// Sends to returns.
    pub sends: Vector<Send>,
    /// The source grid (tracks only).
    pub source_grid: Option<GridId>,
    /// The instrument grid (tracks only).
    pub instrument_grid: Option<GridId>,
    /// Fx grids in signal order.
    pub fx_grids: Vector<GridId>,
    /// Armed for recording.
    pub armed: bool,
    /// Input monitoring.
    pub monitor: Monitor,
    /// A frozen render that plays in place of the track's sources and fx.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frozen: Option<Frozen>,
}

/// A track's frozen render: its sources through its fx, before the strip,
/// rendered from the song start. The patch is kept, so unfreezing brings
/// the live sound back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Frozen {
    /// The render, in the audio pool.
    pub audio: AudioId,
}

/// The three kinds of grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GridKind {
    /// Note and audio sources, ending in the clip dock.
    Source,
    /// Turns notes into audio.
    Instrument,
    /// Processes audio.
    Fx,
}

/// Number of macros on every grid.
pub const MACROS: usize = 16;

/// A patching surface: modules, cables and 16 macros.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Grid {
    /// Its id.
    pub id: GridId,
    /// The track that owns it.
    pub track: TrackId,
    /// Source, instrument or fx.
    pub kind: GridKind,
    /// Macro values, normalised 0–1.
    pub macros: [f32; MACROS],
    /// What each assigned macro is called and drives, by index.
    #[serde(default)]
    pub slots: OrdMap<u8, MacroSlot>,
}

impl Grid {
    /// A grid with every macro at 0 and none assigned.
    pub fn new(id: GridId, track: TrackId, kind: GridKind) -> Grid {
        Grid { id, track, kind, macros: [0.0; MACROS], slots: OrdMap::new() }
    }
}

/// An assigned macro: its name and the parameters it drives.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MacroSlot {
    /// Shown under the knob.
    pub name: String,
    /// The parameters it drives.
    pub binds: Vector<MacroBind>,
}

/// One parameter a macro drives: as the macro goes 0 to 1 the parameter's
/// knob position goes `lo` to `hi`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MacroBind {
    /// The module, on the macro's own grid.
    pub module: ModuleId,
    /// Parameter key.
    pub param: String,
    /// Knob position (0–1) at macro 0.
    pub lo: f32,
    /// Knob position (0–1) at macro 1.
    pub hi: f32,
}

/// A module on a grid.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Module {
    /// Its id.
    pub id: ModuleId,
    /// The grid it sits on.
    pub grid: GridId,
    /// Module type key, such as `"euclid"` or `"piano_roll"`.
    pub kind: String,
    /// Display name.
    pub name: String,
    /// Parameter base values by parameter key.
    pub params: OrdMap<String, f32>,
    /// Position on the grid canvas.
    pub pos: (f32, f32),
    /// The sound a module plays (a sampler's sample), from the audio pool.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<AudioId>,
    /// The plugin a plugin module hosts: a `.clap` file's path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin: Option<String>,
}

/// A sound file in the project's audio pool. The samples live in the
/// bundle's `audio/` folder; the document only records what they are.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioFile {
    /// Its id.
    pub id: AudioId,
    /// Display name, usually the imported file's name without extension.
    pub name: String,
    /// File name inside the bundle's `audio/` folder.
    pub file: String,
    /// Sample rate in Hz.
    pub rate: u32,
    /// Channel count (1 or 2).
    pub channels: u16,
    /// Length in sample frames.
    pub frames: u64,
}

/// One end of a cable on a module.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PortRef {
    /// The module.
    pub module: ModuleId,
    /// The port key on that module.
    pub port: String,
}

impl PortRef {
    /// Shorthand constructor.
    pub fn new(module: ModuleId, port: impl Into<String>) -> PortRef {
        PortRef { module, port: port.into() }
    }
}

/// Where a cable goes.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Endpoint {
    /// An input port on a module.
    Port(PortRef),
    /// A dock inlet; one inlet is one clip.
    Inlet(ClipId),
}

/// A cable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cable {
    /// Its id.
    pub id: CableId,
    /// The grid it is on.
    pub grid: GridId,
    /// The output port it leaves.
    pub from: PortRef,
    /// Where it arrives.
    pub to: Endpoint,
}

/// Anything with a value that can be snapshotted, automated or modulated.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParamTarget {
    /// A module parameter.
    Module {
        /// The module.
        module: ModuleId,
        /// Parameter key.
        param: String,
    },
    /// One of a grid's 16 macros.
    Macro {
        /// The grid.
        grid: GridId,
        /// 0–15.
        index: u8,
    },
    /// A track's fader, in dB.
    Gain(TrackId),
    /// A track's pan.
    Pan(TrackId),
    /// The level of a send, in dB.
    SendLevel {
        /// The sending track.
        track: TrackId,
        /// The return it feeds.
        target: TrackId,
    },
    /// Master tempo in bpm (automation only).
    Tempo,
}

/// A note in a pattern.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Note {
    /// Start, relative to the clip.
    pub start: Ticks,
    /// Length.
    pub len: Ticks,
    /// MIDI pitch, 0–127.
    pub pitch: u8,
    /// Velocity, 0–1.
    pub velocity: f32,
    /// Probability of playing, 0–1.
    pub chance: f32,
}

/// Hand edits on top of a generator's output.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Overrides {
    /// Notes added by hand.
    pub added: Vector<Note>,
    /// Generated notes removed by hand, by (start, pitch).
    pub removed: OrdSet<(Ticks, u8)>,
}

impl Overrides {
    /// No hand edits.
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty()
    }
}

/// The notes a source module plays for one clip.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Pattern {
    /// Notes, sorted by start then pitch.
    pub notes: Vector<Note>,
    /// Hand-edit layer over a generator.
    #[serde(default, skip_serializing_if = "Overrides::is_empty")]
    pub overrides: Overrides,
    /// A chord progression (the chord builder's), sorted by start.
    #[serde(default, skip_serializing_if = "Vector::is_empty")]
    pub chords: Vector<ChordSlot>,
}

/// One chord of a progression: where it sits in the clip and which chord it
/// is, relative to the key, so changing the key transposes it. A diatonic
/// chord (`stack` > 0) is named by scale degree and follows the key's scale
/// too; a fixed one (a borrowed or altered chord) keeps its quality.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChordSlot {
    /// Start, relative to the clip.
    pub start: Ticks,
    /// Length.
    pub len: Ticks,
    /// Root, in semitones above the key's root (0–11); the scale degree
    /// (0–6) for a diatonic chord.
    pub root: u8,
    /// Chord quality, an index into the chord builder's table (unused for a
    /// diatonic chord).
    pub quality: u8,
    /// Thirds stacked on the degree for a diatonic chord (3 a triad, 4 a
    /// seventh, 5 a ninth), or 0 for a fixed chord.
    #[serde(default)]
    pub stack: u8,
    /// Inversion: how many of the lowest notes move up an octave (0–5).
    #[serde(default)]
    pub inversion: u8,
    /// Octave shift from the builder's octave (-3–3).
    #[serde(default)]
    pub octave: i8,
}

/// A clip: one slot in a track's clip dock.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    /// Its id.
    pub id: ClipId,
    /// The track it belongs to.
    pub track: TrackId,
    /// Display name.
    pub name: String,
    /// Colour.
    pub color: Color,
    /// Length.
    pub length: Ticks,
    /// Loops when played past its length.
    pub looped: bool,
    /// Start values applied when the clip starts.
    #[serde(with = "entries")]
    pub snapshot: OrdMap<ParamTarget, f32>,
    /// Seed for generators in this clip.
    pub seed: u64,
    /// Recorded or imported audio: an audio clip plays this instead of notes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<AudioClip>,
    /// Automation the clip carries for its track's gain and pan, at
    /// clip-relative ticks. Launched, it loops with the clip and takes over
    /// the track's automation from the arrangement's lanes.
    #[serde(default, with = "entries", skip_serializing_if = "OrdMap::is_empty")]
    pub envelopes: OrdMap<ParamTarget, Vector<Breakpoint>>,
}

/// The audio of a clip: takes of the same passage, and a comp that picks
/// which take plays where.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioClip {
    /// Takes, oldest first.
    pub takes: Vector<Take>,
    /// Comp segments sorted by start, the first at the clip start: each
    /// plays its take until the next one starts.
    pub comp: Vector<CompSegment>,
    /// Gain in dB.
    pub gain_db: f32,
    /// Fade in at the clip start.
    pub fade_in: Ticks,
    /// Fade out at the clip end.
    pub fade_out: Ticks,
    /// Crossfade at each comp boundary, in milliseconds.
    pub xfade_ms: f32,
}

impl AudioClip {
    /// A clip of one take, played whole.
    pub fn single(take: Take) -> AudioClip {
        AudioClip {
            takes: Vector::unit(take),
            comp: Vector::unit(CompSegment { start: Ticks::ZERO, take: 0 }),
            gain_db: 0.0,
            fade_in: Ticks(0),
            fade_out: Ticks(0),
            xfade_ms: 20.0,
        }
    }

    /// The take playing at clip time `t`.
    pub fn take_at(&self, t: Ticks) -> Option<u16> {
        let i = self.comp.iter().take_while(|s| s.start <= t).count();
        self.comp.get(i.checked_sub(1)?).map(|s| s.take)
    }

    /// The comp with take `take` playing over clip time `from..to` of a
    /// clip `len` long, and the takes around it unchanged. Neighbouring
    /// segments of one take merge.
    pub fn comped(&self, from: Ticks, to: Ticks, take: u16, len: Ticks) -> AudioClip {
        let (from, to) = (from.min(to).max(Ticks::ZERO), from.max(to).min(len));
        let mut out = self.clone();
        if to <= from || take as usize >= self.takes.len() {
            return out;
        }
        let after = self.take_at(to).filter(|_| to < len);
        let mut segs: Vec<CompSegment> = self.comp.iter().filter(|s| s.start < from || s.start > to).copied().collect();
        segs.push(CompSegment { start: from, take });
        if let Some(a) = after {
            segs.push(CompSegment { start: to, take: a });
        }
        segs.sort_by_key(|s| s.start);
        segs.dedup_by(|b, a| a.take == b.take || a.start == b.start);
        out.comp = segs.into_iter().collect();
        out
    }
}

/// One recording of a clip's passage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Take {
    /// The sound, from the audio pool.
    pub audio: AudioId,
    /// Display name, such as `take 2`.
    pub name: String,
    /// The frame of the sound heard at the clip start.
    pub offset: u64,
}

/// Where a take starts playing in a comp.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompSegment {
    /// Clip time it starts at.
    pub start: Ticks,
    /// Index into the takes.
    pub take: u16,
}

/// A clip placed on the arrangement.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Placement {
    /// Its id.
    pub id: PlacementId,
    /// The clip it plays (never a copy).
    pub clip: ClipId,
    /// Start on the timeline.
    pub start: Ticks,
    /// Where in the clip playback starts.
    pub offset: Ticks,
    /// How long it plays.
    pub length: Ticks,
}

/// A session scene.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scene {
    /// Its id.
    pub id: SceneId,
    /// Display name.
    pub name: String,
}

/// The shape of an automation segment, from one breakpoint to the next.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Curve {
    /// Hold the value, then jump.
    Hold,
    /// Straight line.
    #[default]
    Linear,
    /// Bent line; tension -1 to 1 (0 = linear).
    Bend(f32),
}

/// One automation breakpoint.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Breakpoint {
    /// Position on the timeline.
    pub tick: Ticks,
    /// Value (bpm for tempo, dB for gain, else normalised).
    pub value: f32,
    /// Shape towards the next point.
    pub curve: Curve,
}

/// An automation lane.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Lane {
    /// Its id.
    pub id: LaneId,
    /// What it drives.
    pub target: ParamTarget,
    /// Breakpoints sorted by tick.
    pub points: Vector<Breakpoint>,
}

/// The project's key: a root pitch class and a scale, indexing
/// `sat_dsp::scale::ROOTS` and `sat_dsp::scale::NAMES`. The piano roll
/// highlights and snaps to it; sources can follow it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Key {
    /// Root pitch class, 0 (c) to 11 (b).
    pub root: u8,
    /// Scale index (0 chromatic, 1 major, 2 minor, ...).
    pub scale: u8,
}

impl Default for Key {
    /// C major.
    fn default() -> Self {
        Key { root: 0, scale: 1 }
    }
}

/// Loop and punch state saved with the project.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LoopRange {
    /// Loop start.
    pub start: Ticks,
    /// Loop end.
    pub end: Ticks,
    /// Loop on.
    pub enabled: bool,
}

/// Serialises a map as a list of `[key, value]` pairs, so non-string keys
/// work in JSON.
pub(crate) mod entries {
    use im::OrdMap;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<K, V, S>(m: &OrdMap<K, V>, s: S) -> Result<S::Ok, S::Error>
    where
        K: Serialize + Ord + Clone,
        V: Serialize + Clone,
        S: Serializer,
    {
        s.collect_seq(m.iter())
    }

    pub fn deserialize<'de, K, V, D>(d: D) -> Result<OrdMap<K, V>, D::Error>
    where
        K: Deserialize<'de> + Ord + Clone,
        V: Deserialize<'de> + Clone,
        D: Deserializer<'de>,
    {
        let v: Vec<(K, V)> = Vec::deserialize(d)?;
        Ok(v.into_iter().collect())
    }
}
