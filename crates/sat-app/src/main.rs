//! The sat desktop app.
//!
//! Opens a window with the arrange view and bottom panel, plays the project
//! through the default audio device, and lets you mix it:
//!
//! - space: play / stop; home: back to the start; l: loop on / off
//! - click a clip to edit it in the bottom panel; click the ruler to move the playhead
//! - drag faders and knobs (shift for fine steps, double-click to reset);
//!   click m / s / arm buttons; tab switches the bottom panel between clip and mixer
//! - piano roll: draw notes by clicking (drag to set the length), drag notes to
//!   move them, drag their right edge to resize, drag in the velocity lane;
//!   double-click deletes; the select tool drags a box; delete, arrows
//!   (shift: octave), ctrl+a, escape; alt drags off the grid
//! - drum seq: click a step to toggle it, shift-click cycles its velocity
//!   (normal, accent, ghost); page buttons pick the bar shown
//! - euclid: drag the knobs; click a trig to add or remove a hand edit over the
//!   generator; clear drops the edits, bake fixes everything into steps
//! - source grid (top bar button or "open source grid"): drag module headers to
//!   move them, drag from a notes out to a clip's inlet to wire it, drag a wire
//!   off an inlet to rewire or unplug it; + module, + clip; delete removes the
//!   selected module
//! - m / s: mute / solo the selected track; ctrl+z / ctrl+shift+z: undo / redo
//! - ctrl+o open (or drop a .sat folder on the window), ctrl+s save, ctrl+shift+s save as,
//!   ctrl+n new, ctrl+q quit
//! - ctrl+e export the song to WAV (with its tail); ctrl+shift+e export the loop range
//! - wheel: scroll; shift+wheel: scroll time; ctrl+wheel: zoom
//!
//! `sat [project.sat]` opens a project bundle; without one it opens a demo.
//! `sat --export out.wav [--loop] [--rate hz] [project.sat]` renders the song
//! (or the loop) to a WAV file without opening a window.

mod audio;
mod export;
mod input;
mod project;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use sat_compile::Compiled;
use sat_doc::sat_time::Ticks;
use sat_doc::{
    Breakpoint, CableId, ClipId, Document, Endpoint, GridKind, History, LaneId, ModuleId, Note, Op, ParamTarget,
    PlacementId, PortRef, TrackFlag, TrackId, TrackKind,
};
use sat_engine::{Command, EngineHandle};
use sat_modules::{Editor, PortKind, drum};
use sat_ui::arrange::{ArrangeLayout, ArrangeState, Hit, RowKind, TransportView, header_controls};
use sat_ui::arrange_edit::{self, Span};
use sat_ui::automation;
use sat_ui::controls::{self, SILENT_DB};
use sat_ui::grid::{self, GridHit, GridLayout, GridView};
use sat_ui::mixer::{Knob, Levels, MixHit, MixerLayout};
use sat_ui::paint::{Fonts, color};
use sat_ui::palette::{self, Action, PaletteLayout, PaletteView};
use sat_ui::panel::{self, PanelView, Tab};
use sat_ui::piano_roll::{self, RollGeom, RollHit, RollView, Tool, clip_notes, next_snap};
use sat_ui::roll_edit::{self, Edited};
use sat_ui::session::{SessionHit, SessionLayout, SessionView};
use sat_ui::shell::{self, Frame, MainView, PANEL_H, ShellLayout, TransportButton, ViewButton};
use sat_ui::steps::{StepGeom, StepHit, StepView};
use vello::kurbo::{Point, Rect, Vec2};
use vello::util::{RenderContext, RenderSurface};
use vello::{AaConfig, RenderParams, Renderer, RendererOptions, Scene, wgpu};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Window, WindowId};

use project::Project;

const MAX_BLOCK: usize = 1024;
const FALLBACK_RATE: f64 = 48_000.0;
/// Meter fall-off in dB per second.
const METER_FALL_DB: f32 = 24.0;
/// Pixels of knob travel for its full range.
const KNOB_TRAVEL: f64 = 160.0;
const DOUBLE_CLICK: Duration = Duration::from_millis(350);

/// Audio and transport, tied to one compiled schedule.
struct Playback {
    handle: Option<EngineHandle>,
    compiled: Compiled,
    output: Option<audio::Output>,
    playing: bool,
    /// Used when there is no audio device: a wall clock drives the playhead.
    clock: Option<(Instant, Ticks)>,
    status: String,
    /// The decoded sounds samplers play.
    pool: sat_disk::Pool,
}

impl Playback {
    fn start(doc: &Document, pool: sat_disk::Pool) -> Playback {
        let rate = audio::device_rate().unwrap_or(FALLBACK_RATE);
        let compiled = sat_compile::compile_with(doc, rate, MAX_BLOCK, pool.sounds()).expect("project compiles");
        let (engine, mut handle) = sat_engine::engine(compiled.schedule.clone(), 256);
        let _ = handle.send(Command::SetLoop(compiled.loop_samples(doc)));
        match audio::start(engine, MAX_BLOCK) {
            Ok(out) => Playback {
                status: format!("{} @ {} Hz", out.name, out.sample_rate),
                handle: Some(handle),
                compiled,
                output: Some(out),
                playing: false,
                clock: None,
                pool,
            },
            Err(e) => Playback {
                status: format!("no audio ({e})"),
                handle: None,
                compiled,
                output: None,
                playing: false,
                clock: None,
                pool,
            },
        }
    }

    fn send(&mut self, c: Command) {
        if let Some(h) = &mut self.handle {
            let _ = h.send(c);
            h.collect_garbage();
        }
    }

    /// Recompiles after a structural edit and swaps the new schedule in.
    fn recompile(&mut self, doc: &Document) {
        let rate = self.compiled.schedule.sample_rate();
        match sat_compile::compile_with(doc, rate, MAX_BLOCK, self.pool.sounds()) {
            Ok(c) => {
                self.send(Command::Swap(Box::new(c.schedule.clone())));
                self.send(Command::SetLoop(c.loop_samples(doc)));
                self.compiled = c;
            }
            Err(e) => eprintln!("compile failed: {e}"),
        }
    }

    fn playhead(&self) -> Ticks {
        match (&self.handle, self.clock) {
            (Some(h), _) => {
                let s = h.telemetry().playhead() as f64;
                Ticks(self.compiled.tempo.sample_to_tick(s, self.compiled.schedule.sample_rate()).round() as i64)
            }
            (None, Some((t0, from))) if self.playing => {
                let secs = self.compiled.tempo.tick_to_secs(from.0 as f64) + t0.elapsed().as_secs_f64();
                Ticks(self.compiled.tempo.secs_to_tick(secs) as i64)
            }
            (None, Some((_, from))) => from,
            _ => Ticks::ZERO,
        }
    }

    fn toggle(&mut self) {
        let here = self.playhead();
        self.playing = !self.playing;
        self.clock = Some((Instant::now(), here));
        self.send(if self.playing { Command::Play } else { Command::Stop });
    }

    fn stop(&mut self) {
        if self.playing {
            self.toggle();
        }
    }

    fn locate(&mut self, t: Ticks) {
        self.clock = Some((Instant::now(), t));
        let s = self.compiled.sample(t);
        self.send(Command::Locate(s));
    }

    /// Folds the engine's peaks since the last frame into `levels`, letting
    /// them fall at [`METER_FALL_DB`] per second.
    fn update_levels(&self, doc: &Document, levels: &mut Levels, dt: f32) {
        let fall = 10f32.powf(-METER_FALL_DB * dt / 20.0);
        for t in doc.tracks.keys() {
            let (pl, pr) = self.compiled.meter(*t).map(|m| m.take()).unwrap_or((0.0, 0.0));
            let e = levels.entry(*t).or_insert((0.0, 0.0));
            *e = (pl.max(e.0 * fall), pr.max(e.1 * fall));
            if e.0 < 1e-4 && e.1 < 1e-4 {
                *e = (0.0, 0.0);
            }
        }
        levels.retain(|t, _| doc.tracks.contains_key(t));
    }
}

/// How a drag moves its value.
#[derive(Debug, Clone, Copy)]
enum Axis {
    /// Up raises the value; `travel` px covers the whole range.
    Vertical { travel: f64 },
    /// Right raises the value.
    Horizontal { travel: f64 },
}

/// A fader or knob being dragged.
#[derive(Debug, Clone)]
struct Drag {
    target: ParamTarget,
    axis: Axis,
    from: Point,
    start: f64,
}

/// A piano-roll drag. Each keeps the notes and selection from when it began,
/// so every move recomputes from the same starting point.
#[derive(Debug, Clone)]
enum RollDrag {
    /// Move the selection; `anchor` is the note under the pointer.
    Move { notes: Vec<Note>, sel: Vec<usize>, anchor: Note, from: Point },
    /// Resize the selection by the anchor note's end.
    Resize { notes: Vec<Note>, sel: Vec<usize>, anchor: Note, from: Point },
    /// Set the selection's velocity.
    Velocity { notes: Vec<Note>, sel: Vec<usize> },
    /// Box-select from a point; `keep` is the selection to add to.
    Box { from: Point, keep: Vec<usize> },
}

/// The spec of a module parameter.
fn module_spec(doc: &Document, target: &ParamTarget) -> Option<&'static sat_modules::ParamSpec> {
    let ParamTarget::Module { module, param } = target else { return None };
    sat_modules::spec(&doc.modules.get(module)?.kind, param)
}

/// A drag in a grid.
#[derive(Debug, Clone)]
enum GridDrag {
    /// Moving a module by its header.
    Module { id: ModuleId, from: Point, start: (f32, f32) },
    /// Pulling a cable from an output; `detach` is the cable picked up off an inlet or input.
    Wire { from: PortRef, kind: PortKind, start: Point, detach: Option<CableId> },
    /// Panning the canvas.
    Pan { from: Point, start: Vec2 },
}

/// A recording in progress: the input captured since record was pressed.
struct Recording {
    tracks: Vec<TrackId>,
    /// Timeline seconds where it started.
    start: f64,
    /// The loop, in seconds, if the transport loops.
    looping: Option<(f64, f64)>,
    l: Vec<f32>,
    r: Vec<f32>,
}

/// A drag on an audio clip in the arrange view.
#[derive(Debug, Clone)]
enum AudioDrag {
    /// Swiping a take lane: the take plays over the swiped range (or, for
    /// a click, over the comp segment under the press).
    Comp { clip: ClipId, take: u16, origin: Ticks, from: Ticks, start: sat_doc::AudioClip },
    /// Moving a fade handle.
    Fade { clip: ClipId, origin: Ticks, out: bool },
    /// Dragging a field of the audio clip editor.
    Field { clip: ClipId, field: sat_ui::audio_edit::AudioField, from: f64, start: f64 },
}

/// A drag on a clip placement in the arrange view. Each keeps the span the
/// placement had when the drag began, so every move recomputes from it.
#[derive(Debug, Clone)]
enum ClipDrag {
    /// Moving it along its track; `copy` (alt at the press) moves a copy
    /// instead, made once the pointer has moved.
    Move { placement: PlacementId, from: Span, x: f64, copy: bool },
    /// Dragging its left or right edge.
    Trim { placement: PlacementId, from: Span, x: f64, right: bool },
}

/// The comp with `take` over the segment of `a` holding clip time `at`.
fn comp_segment(a: &sat_doc::AudioClip, at: Ticks, take: u16, len: Ticks) -> sat_doc::AudioClip {
    let i = a.comp.iter().take_while(|s| s.start <= at).count().saturating_sub(1);
    let start = a.comp.get(i).map_or(Ticks::ZERO, |s| s.start);
    let end = a.comp.get(i + 1).map_or(len, |s| s.start);
    a.comped(start, end, take, len)
}

/// Does any track hear its input (so the input should be open)?
fn monitored(doc: &Document) -> bool {
    doc.tracks.values().any(|t| match t.monitor {
        sat_doc::Monitor::In => t.kind == TrackKind::Track,
        sat_doc::Monitor::Auto => t.armed,
        sat_doc::Monitor::Off => false,
    })
}

/// Clip time snapped to the sixteenth it falls in.
fn snap16(t: Ticks) -> Ticks {
    let s = Ticks::sixteenths(1).0;
    Ticks(t.0.div_euclid(s) * s)
}

/// A parameter's position (0–1) on its control.
fn position_of(doc: &Document, target: &ParamTarget) -> f64 {
    match target {
        ParamTarget::Gain(t) => controls::fader_pos(doc.tracks[t].mixer.gain_db),
        ParamTarget::Pan(t) => controls::pan_pos(doc.tracks[t].mixer.pan),
        ParamTarget::SendLevel { track, target } => doc.tracks[track]
            .sends
            .iter()
            .find(|s| s.target == *target)
            .map(|s| controls::fader_pos(s.level_db))
            .unwrap_or(0.0),
        ParamTarget::Macro { .. } => doc.base_value(target).unwrap_or(0.0).clamp(0.0, 1.0) as f64,
        ParamTarget::Module { .. } => match (module_spec(doc, target), doc.base_value(target)) {
            (Some(s), v) => s.pos(s.clamp(v.unwrap_or(s.default))),
            _ => 0.0,
        },
        _ => 0.0,
    }
}

/// The value a control position sets.
fn value_at(doc: &Document, target: &ParamTarget, pos: f64) -> f32 {
    match target {
        ParamTarget::Pan(_) => controls::pan_at(pos),
        ParamTarget::Module { .. } => module_spec(doc, target).map_or(0.0, |s| s.at(pos)),
        ParamTarget::Macro { .. } => ((pos.clamp(0.0, 1.0) * 1000.0).round() / 1000.0) as f32,
        _ => controls::fader_db(pos),
    }
}

/// The value double-click resets a control to.
fn default_value(doc: &Document, target: &ParamTarget) -> f32 {
    match target {
        ParamTarget::SendLevel { .. } => SILENT_DB,
        ParamTarget::Module { .. } => module_spec(doc, target).map_or(0.0, |s| s.default),
        _ => 0.0,
    }
}

struct Gfx {
    window: Arc<Window>,
    surface: RenderSurface<'static>,
    renderer: Renderer,
}

struct App {
    project: Project,
    history: History,
    arrange: ArrangeState,
    panel: PanelView,
    mixer_scroll: f64,
    levels: Levels,
    playback: Playback,
    ctx: RenderContext,
    gfx: Option<Gfx>,
    fonts: Fonts,
    theme: sat_theme::Theme,
    scene: Scene,
    cursor: Point,
    mods: ModifiersState,
    shell: Option<ShellLayout>,
    layout: Option<ArrangeLayout>,
    mixer: Option<MixerLayout>,
    drag: Option<Drag>,
    roll: RollView,
    roll_geom: Option<RollGeom>,
    roll_drag: Option<RollDrag>,
    steps: StepView,
    step_geom: Option<StepGeom>,
    view: MainView,
    /// The view a grid's back button returns to.
    prev_view: MainView,
    /// The command palette, while open.
    palette: Option<PaletteView>,
    palette_layout: Option<PaletteLayout>,
    grid: GridView,
    grid_layout: Option<GridLayout>,
    /// What the session is playing and queued.
    session: SessionView,
    session_layout: Option<SessionLayout>,
    /// Macro learn mode: the next touched parameter maps to a free macro.
    learn: bool,
    macro_layout: Option<sat_ui::macros::MacroLayout>,
    grid_drag: Option<GridDrag>,
    audio_drag: Option<AudioDrag>,
    audio_layout: Option<sat_ui::audio_edit::AudioEditLayout>,
    clip_drag: Option<ClipDrag>,
    /// A breakpoint being dragged: its lane, index, and the lane's points when the drag began.
    auto_drag: Option<(LaneId, usize, Vec<Breakpoint>)>,
    /// The last press was in the bottom panel (keys go to its editor), else
    /// in the main view (keys edit clips there).
    panel_focus: bool,
    /// The pointer shape shown, so it is only set when it changes.
    cursor_icon: winit::window::CursorIcon,
    /// Length of the last drawn or resized note.
    note_len: sat_doc::sat_time::Ticks,
    last_click: Option<(Instant, Point)>,
    last_frame: Instant,
    /// The input, opened the first time something is recorded.
    input: Option<input::Input>,
    /// Opening the input failed; not retried until something is recorded.
    input_failed: bool,
    rec: Option<Recording>,
    quit: bool,
}

impl App {
    fn edit(&mut self, label: &str, ops: Vec<Op>) {
        match self.history.apply(label, ops) {
            Ok(tx) => self.after_change(tx.is_structural(), &tx.changes),
            Err(e) => eprintln!("{label}: {e}"),
        }
    }

    fn after_change(&mut self, structural: bool, changes: &[sat_doc::Change]) {
        // pattern edits change the note events baked into the schedule
        // as do pattern edits and generator knobs
        let doc = self.history.doc();
        let patterns = changes.iter().any(|c| match c {
            sat_doc::Change::Pattern { .. } => true,
            sat_doc::Change::Value { target, .. } => sat_modules::shapes_notes(doc, target),
            _ => false,
        });
        if structural || patterns {
            self.playback.recompile(self.history.doc());
        } else {
            for ch in changes {
                if let sat_doc::Change::Value { target, value } = ch {
                    for cmd in self.playback.compiled.param_commands(target, *value) {
                        self.playback.send(cmd);
                    }
                }
            }
        }
    }

    /// Replaces the document (after open or new) and resets the view.
    fn load(&mut self, doc: Document) {
        self.playback.stop();
        self.session.clear();
        let first = doc.clips.keys().next().copied();
        self.arrange = ArrangeState {
            selected_clip: first,
            selected_track: first.and_then(|c| doc.clips.get(&c)).map(|c| c.track),
            ..Default::default()
        };
        self.mixer_scroll = 0.0;
        self.levels.clear();
        self.roll = RollView { tool: self.roll.tool, snap: self.roll.snap, ..Default::default() };
        self.roll_drag = None;
        self.steps = StepView::default();
        self.view = MainView::Arrange;
        self.grid = GridView::default();
        self.grid_drag = None;
        self.audio_drag = None;
        self.clip_drag = None;
        self.auto_drag = None;
        self.playback.recompile(&doc);
        self.history = History::new(doc, 500);
        self.playback.locate(Ticks::ZERO);
        self.update_title();
    }

    fn update_title(&self) {
        if let Some(g) = &self.gfx {
            let dot = if self.project.edited(self.history.version()) { " •" } else { "" };
            g.window.set_title(&format!("sat · {}{dot}", self.project.name));
        }
    }

    /// Keeps unsaved work in an autosave before the document is replaced.
    fn keep_unsaved(&mut self) {
        let v = self.history.version();
        if self.project.autosave(self.history.doc(), self.playback.pool.sounds(), v, true) {
            eprintln!("unsaved changes kept in {}", self.project.autosave_dir().join("backups").display());
        }
    }

    /// A WAV dropped on the window: into the shown instrument grid's (or the
    /// selected track's) sampler, adding one if the grid has none.
    fn drop_sound(&mut self, path: &Path) {
        let doc = self.history.doc();
        let grid = match self.view {
            MainView::Grid(g) if doc.grids.get(&g).is_some_and(|g| g.kind == GridKind::Instrument) => Some(g),
            _ => self.arrange.selected_track.and_then(|t| doc.tracks.get(&t)).and_then(|t| t.instrument_grid),
        };
        let Some(grid) = grid else {
            eprintln!("select a track to drop a sound on");
            return;
        };
        let target = self
            .grid
            .selected
            .filter(|id| {
                doc.modules.get(id).is_some_and(|m| m.grid == grid && m.kind == sat_modules::instrument::SAMPLER)
            })
            .or_else(|| doc.modules_on(grid).find(|m| m.kind == sat_modules::instrument::SAMPLER).map(|m| m.id));
        self.load_sound(target, grid, path);
    }

    /// Imports a WAV and sets it on sampler `module`, or on a new sampler on `grid`.
    fn load_sound(&mut self, module: Option<ModuleId>, grid: sat_doc::GridId, path: &Path) {
        let file = match self.playback.pool.import(path) {
            Ok(f) => f,
            Err(e) => return eprintln!("cannot load {}: {e}", path.display()),
        };
        let doc = self.history.doc();
        let mut ops = vec![Op::AddAudio { file: file.clone() }];
        let module = match module {
            Some(m) => m,
            None => {
                let pos = grid::free_pos(doc, grid);
                let (id, op) = sat_modules::add_module(doc, grid, sat_modules::instrument::SAMPLER, pos);
                ops.push(op);
                id
            }
        };
        ops.push(Op::SetModuleAudio { module, audio: Some(file.id) });
        eprintln!("loaded {} ({} frames at {} Hz)", file.name, file.frames, file.rate);
        self.edit("load sound", ops);
        self.grid.selected = Some(module);
    }

    fn open(&mut self) {
        let mut dialog = rfd::FileDialog::new().set_title("Open a sat project (.sat folder)");
        if let Some(dir) = self.project.path.as_ref().and_then(|p| p.parent()) {
            dialog = dialog.set_directory(dir);
        }
        if let Some(path) = dialog.pick_folder() {
            self.open_path(&path);
        }
    }

    fn open_path(&mut self, path: &Path) {
        self.keep_unsaved();
        match self.project.open(path, 0) {
            Ok(doc) => {
                eprintln!("opened {}", path.display());
                self.playback.pool = load_sounds(path, &doc);
                self.load(doc);
            }
            Err(e) => eprintln!("cannot open {}: {e}", path.display()),
        }
    }

    fn new_project(&mut self) {
        self.keep_unsaved();
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
        self.project = Project::new(None, "untitled".into(), 0);
        self.playback.pool = sat_disk::Pool::new();
        self.load(Document::new(seed as u64));
    }

    fn save(&mut self, choose: bool) {
        let v = self.history.version();
        let res = if self.project.path.is_some() && !choose {
            self.project.save(self.history.doc(), self.playback.pool.sounds(), v)
        } else {
            let mut dialog = rfd::FileDialog::new().set_title("Save sat project").set_file_name(format!(
                "{}.{}",
                self.project.name,
                project::EXT
            ));
            if let Some(dir) = self.project.path.as_ref().and_then(|p| p.parent()) {
                dialog = dialog.set_directory(dir);
            }
            let Some(path) = dialog.save_file() else { return };
            self.project.save_as(&path, self.history.doc(), self.playback.pool.sounds(), v)
        };
        match res {
            Ok(p) => eprintln!("saved {}", p.display()),
            Err(e) => eprintln!("save failed: {e}"),
        }
        self.update_title();
    }

    fn export(&mut self, span: export::Span) {
        let what = if span == export::Span::Loop { "loop" } else { "song" };
        let mut dialog = rfd::FileDialog::new()
            .set_title(format!("Export the {what} as WAV"))
            .add_filter("WAV audio", &["wav", "WAV"])
            .set_file_name(format!(
                "{}{}.wav",
                self.project.name,
                if span == export::Span::Loop { " loop" } else { "" }
            ));
        if let Some(dir) = self.project.path.as_ref().and_then(|p| p.parent()) {
            dialog = dialog.set_directory(dir);
        }
        let Some(path) = dialog.save_file() else { return };
        let rate = self.playback.compiled.schedule.sample_rate();
        match export::export(self.history.doc(), self.playback.pool.sounds(), rate, span, &path) {
            Ok(secs) => eprintln!("exported {} ({secs:.1} s)", path.display()),
            Err(e) => eprintln!("export failed: {e}"),
        }
    }

    fn redraw(&mut self) {
        self.poll_recording();
        if self.input.is_none() && !self.input_failed && monitored(self.history.doc()) {
            self.ensure_input();
        }
        let Some(g) = &mut self.gfx else { return };
        let size = g.window.inner_size();
        if size.width == 0 || size.height == 0 {
            return;
        }
        let dt = self.last_frame.elapsed().as_secs_f32().min(0.25);
        self.playback.update_levels(self.history.doc(), &mut self.levels, dt);
        let scale = g.window.scale_factor();
        let (w, h) = (size.width as f64 / scale, size.height as f64 / scale);
        self.session.advance(self.history.doc(), self.playback.playhead());
        let frame = Frame {
            doc: self.history.doc(),
            name: &self.project.name,
            arrange: &self.arrange,
            transport: TransportView {
                playhead: self.playback.playhead(),
                playing: self.playback.playing,
                recording: self.rec.is_some(),
            },
            panel_h: PANEL_H,
            panel: self.panel,
            mixer_scroll: self.mixer_scroll,
            levels: &self.levels,
            edited: self.project.edited(self.history.version()),
            roll: &self.roll,
            steps: &self.steps,
            view: self.view,
            grid: &self.grid,
            learn: self.learn,
            sounds: &self.playback.pool,
            session: &self.session,
            palette: self.palette.as_ref(),
        };
        let mut scene = Scene::new();
        let drawn = shell::draw(&mut scene, &mut self.fonts, &self.theme, &frame, w, h);
        self.shell = Some(drawn.shell);
        self.layout = Some(drawn.arrange);
        if let Some(m) = &drawn.mixer {
            self.mixer_scroll = self.mixer_scroll.min(m.max_scroll);
        }
        self.mixer = drawn.mixer;
        if self.roll.lo.is_none()
            && let Some(g) = &drawn.roll
        {
            // hold the rows still once shown, so edits do not re-fit them
            self.roll.lo = Some(g.layout.lo);
        }
        self.roll_geom = drawn.roll;
        self.audio_layout = drawn.audio;
        self.step_geom = drawn.steps;
        if matches!(self.view, MainView::Grid(_)) && drawn.grid.is_none() {
            // the track went away (undo): back to the arrangement
            self.view = MainView::Arrange;
        }
        self.grid_layout = drawn.grid;
        self.macro_layout = drawn.macros;
        self.session_layout = drawn.session;
        self.palette_layout = drawn.palette;
        self.scene.reset();
        self.scene.append(&scene, Some(vello::kurbo::Affine::scale(scale)));
        let dev = &self.ctx.devices[g.surface.dev_id];
        if g.surface.config.width != size.width || g.surface.config.height != size.height {
            self.ctx.resize_surface(&mut g.surface, size.width, size.height);
        }
        let params = RenderParams {
            base_color: color(self.theme.c.bg_base),
            width: size.width,
            height: size.height,
            antialiasing_method: AaConfig::Area,
        };
        if let Err(e) =
            g.renderer.render_to_texture(&dev.device, &dev.queue, &self.scene, &g.surface.target_view, &params)
        {
            eprintln!("render: {e}");
            return;
        }
        let st = match g.surface.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(st) | wgpu::CurrentSurfaceTexture::Suboptimal(st) => st,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                // reconfigure on the next frame
                self.ctx.resize_surface(&mut g.surface, size.width, size.height);
                g.window.request_redraw();
                return;
            }
            _ => return,
        };
        let mut enc = dev.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("blit") });
        let view = st.texture.create_view(&Default::default());
        g.surface.blitter.copy(&dev.device, &mut enc, &g.surface.target_view, &view);
        dev.queue.submit([enc.finish()]);
        g.window.pre_present_notify();
        dev.queue.present(st);
        self.last_frame = Instant::now();
    }

    /// Play or pause; pausing ends a recording.
    fn toggle_play(&mut self) {
        if self.playback.playing {
            self.stop_recording();
        }
        self.playback.toggle();
    }

    /// Starts recording onto the armed tracks (arming the selected track if
    /// none is), starting the transport if it is stopped.
    fn start_recording(&mut self) {
        let doc = self.history.doc();
        let mut tracks: Vec<TrackId> = doc
            .tracks_in_order()
            .into_iter()
            .filter(|t| doc.tracks[t].armed && doc.tracks[t].kind == TrackKind::Track)
            .collect();
        if tracks.is_empty() {
            match self.arrange.selected_track.filter(|t| doc.tracks.get(t).is_some_and(|x| x.kind == TrackKind::Track))
            {
                Some(t) => {
                    self.toggle_flag(t, TrackFlag::Arm);
                    tracks.push(t);
                }
                None => return eprintln!("record: arm a track first"),
            }
        }
        self.input_failed = false;
        if !self.ensure_input() {
            return;
        }
        if let Some(i) = &mut self.input {
            i.clear();
        }
        // what was sung at a moment lines up with what was heard then
        let latency = self.input.as_ref().map_or(0.0, |i| i.latency())
            + self.playback.output.as_ref().map_or(0.0, |o| o.latency());
        let doc = self.history.doc();
        let tempo = &self.playback.compiled.tempo;
        let start = tempo.tick_to_secs(self.playback.playhead().0 as f64) - latency;
        let lr = doc.loop_range;
        let looping = lr.enabled.then(|| (tempo.tick_to_secs(lr.start.0 as f64), tempo.tick_to_secs(lr.end.0 as f64)));
        self.rec = Some(Recording { tracks, start, looping, l: Vec::new(), r: Vec::new() });
        self.playback.send(Command::SetRecording(true));
        if !self.playback.playing {
            self.playback.toggle();
        }
    }

    /// Opens the input if it is not open, and lets the engine monitor it.
    /// False (after saying why) if there is none.
    fn ensure_input(&mut self) -> bool {
        if self.input.is_some() {
            return true;
        }
        if self.input_failed {
            return false;
        }
        match input::Input::open() {
            Ok(mut i) => {
                eprintln!("sat: input {} @ {} Hz", i.name, i.rate);
                if let (Some(o), Some(m)) = (&mut self.playback.output, i.take_monitor()) {
                    o.monitor(m);
                }
                self.input = Some(i);
                true
            }
            Err(e) => {
                eprintln!("input: {e}");
                self.input_failed = true;
                false
            }
        }
    }

    /// Moves newly captured input into the recording.
    fn poll_recording(&mut self) {
        if let (Some(rec), Some(i)) = (&mut self.rec, &mut self.input) {
            i.drain(&mut rec.l, &mut rec.r);
        }
    }

    /// Ends a recording, turning it into takes.
    fn stop_recording(&mut self) {
        self.poll_recording();
        let (Some(rec), Some(i)) = (self.rec.take(), &self.input) else { return };
        self.playback.send(Command::SetRecording(false));
        let sample = sat_disk::Sample { rate: i.rate, l: rec.l, r: rec.r };
        let mut ops = Vec::new();
        for t in &rec.tracks {
            // each track records the input its recorder takes
            let doc = self.history.doc();
            let choice = sat_modules::recorder(doc, *t).and_then(|m| m.params.get("input").copied()).unwrap_or(0.0);
            let s = sat_ui::record::channels(&sample, choice);
            ops.extend(sat_ui::record::record(doc, &mut self.playback.pool, &[*t], &s, rec.start, rec.looping));
        }
        if !ops.is_empty() {
            self.edit("record", ops);
        }
    }

    fn toggle_flag(&mut self, t: TrackId, flag: TrackFlag) {
        let tr = &self.history.doc().tracks[&t];
        let on = match flag {
            TrackFlag::Mute => !tr.mixer.mute,
            TrackFlag::Solo => !tr.mixer.solo,
            TrackFlag::Arm => !tr.armed,
        };
        let label = match flag {
            TrackFlag::Mute => "mute",
            TrackFlag::Solo => "solo",
            TrackFlag::Arm => "arm",
        };
        self.edit(label, vec![Op::SetTrackFlag { id: t, flag, on }]);
    }

    /// Starts dragging `target`; double-click resets it instead.
    fn begin_drag(&mut self, target: ParamTarget, axis: Axis, double: bool) {
        let doc = self.history.doc();
        if let ParamTarget::Module { module, param } = &target {
            // learn mode maps the parameter to a free macro instead
            if self.learn
                && let Some((_, ops)) = sat_modules::macros::learn(doc, *module, param)
            {
                self.edit("map to macro", ops);
                return;
            }
            // a macro-driven parameter is played through its macro
            if let Some((index, _)) = sat_modules::macros::bound(doc, *module, param) {
                let grid = doc.modules[module].grid;
                return self.begin_drag(ParamTarget::Macro { grid, index }, axis, double);
            }
        }
        // a send knob with no send yet creates one, silent, then drags it
        let mut ops = Vec::new();
        if let ParamTarget::SendLevel { track, target: ret } = target
            && !doc.tracks[&track].sends.iter().any(|s| s.target == ret)
        {
            if double {
                return;
            }
            ops.push(Op::AddSend { track, target: ret, level_db: SILENT_DB, pre_fader: false });
        }
        if double {
            self.edit(
                "reset",
                vec![Op::SetValue { target: target.clone(), value: default_value(self.history.doc(), &target) }],
            );
            return;
        }
        self.history.begin_gesture(match target {
            ParamTarget::Gain(_) => "fader",
            ParamTarget::Pan(_) => "pan",
            ParamTarget::Module { .. } => "knob",
            ParamTarget::Macro { .. } => "macro",
            _ => "send",
        });
        if !ops.is_empty() {
            self.edit("send", ops);
        }
        let start = position_of(self.history.doc(), &target);
        self.drag = Some(Drag { target, axis, from: self.cursor, start });
    }

    fn drag_to(&mut self) {
        let Some(d) = self.drag.clone() else { return };
        let fine = if self.mods.shift_key() { 0.25 } else { 1.0 };
        let delta = match d.axis {
            Axis::Vertical { travel } => (d.from.y - self.cursor.y) / travel,
            Axis::Horizontal { travel } => (self.cursor.x - d.from.x) / travel,
        };
        let pos = (d.start + delta * fine).clamp(0.0, 1.0);
        let doc = self.history.doc();
        let value = value_at(doc, &d.target, pos);
        if (value - value_at(doc, &d.target, position_of(doc, &d.target))).abs() > f32::EPSILON {
            self.edit("drag", vec![Op::SetValue { target: d.target, value }]);
        }
    }

    fn end_drag(&mut self) {
        if self.drag.take().is_some() {
            self.history.end_gesture();
        }
        if self.roll_drag.take().is_some() {
            self.roll.rubber = None;
            self.history.end_gesture();
        }
        if let Some(d) = self.grid_drag.take() {
            self.end_grid_drag(d);
        }
        if self.audio_drag.take().is_some() {
            self.history.end_gesture();
        }
        if self.clip_drag.take().is_some() {
            self.history.end_gesture();
        }
        if self.auto_drag.take().is_some() {
            self.arrange.active_point = None;
            self.history.end_gesture();
        }
    }

    fn press_session(&mut self, pt: Point, double: bool) {
        let Some(l) = &self.session_layout else { return };
        let doc = self.history.doc();
        let hit = l.hit(doc, &self.session, pt);
        let columns: Vec<TrackId> = l.cols.iter().filter(|c| !c.group).map(|c| c.track).collect();
        let slot = |scene: sat_doc::SceneId, t: TrackId| doc.slots.get(&(scene, t)).copied();
        let launches: Vec<(TrackId, Option<ClipId>)> = match hit {
            SessionHit::Launch(t, c) => vec![(t, Some(c))],
            SessionHit::Group(g, scene) => {
                sat_ui::session::tracks_of(doc, g).into_iter().map(|t| (t, slot(scene, t))).collect()
            }
            SessionHit::Scene(scene) => columns.iter().map(|t| (*t, slot(scene, *t))).collect(),
            SessionHit::Stop(t) => sat_ui::session::tracks_of(doc, t).into_iter().map(|t| (t, None)).collect(),
            SessionHit::StopAll => {
                self.session.playing.keys().chain(self.session.queued.keys()).map(|t| (*t, None)).collect()
            }
            SessionHit::Select(c) => {
                self.select_clip(c);
                return;
            }
            SessionHit::Track(t) => {
                self.arrange.selected_track = Some(t);
                if double {
                    self.open_track(t);
                }
                return;
            }
            SessionHit::BackToArrangement => return self.back_to_arrangement(),
            SessionHit::Nothing => return,
        };
        if let SessionHit::Launch(_, c) = hit {
            self.select_clip(c);
        }
        self.launch(launches);
    }

    /// Launches clips (or stops, with `None`) on tracks at the next bar,
    /// starting the transport if it is stopped (then they start now).
    fn launch(&mut self, launches: Vec<(TrackId, Option<ClipId>)>) {
        if launches.is_empty() {
            return;
        }
        let was_playing = self.playback.playing;
        let at = sat_ui::session::launch_tick(self.history.doc(), self.playback.playhead(), was_playing);
        let sample = self.playback.compiled.sample(at);
        for (t, clip) in launches {
            if let Some(cmd) = self.playback.compiled.launch(t, clip, sample) {
                self.playback.send(cmd);
                self.session.queued.insert(t, (clip, at));
            }
        }
        if !was_playing && self.session.queued.values().any(|(c, _)| c.is_some()) {
            self.playback.toggle();
        }
    }

    fn back_to_arrangement(&mut self) {
        self.playback.send(Command::BackToArrangement);
        self.session.clear();
    }

    fn select_clip(&mut self, clip: ClipId) {
        if self.arrange.selected_clip != Some(clip) {
            self.steps.page = 0;
            self.roll.selected.clear();
            self.roll.lo = None;
        }
        self.arrange.selected_clip = Some(clip);
        self.arrange.selected_track = self.history.doc().clips.get(&clip).map(|c| c.track);
    }

    /// The edited clip, its source module, its notes and its length.
    fn roll_target(&self) -> Option<(ClipId, sat_doc::ModuleId, Vec<Note>, Ticks)> {
        let clip = self.arrange.selected_clip?;
        let len = self.history.doc().clips.get(&clip)?.length;
        let (module, notes) = clip_notes(self.history.doc(), clip);
        Some((clip, module?, notes, len))
    }

    /// Writes edited notes into the clip's pattern and takes their selection.
    fn write_notes(&mut self, label: &str, e: Edited) {
        let Some((clip, module, notes, _)) = self.roll_target() else { return };
        if e.notes != notes {
            self.edit(label, vec![Op::WritePattern { module, clip, notes: im::Vector::from(e.notes) }]);
        }
        self.roll.selected = e.selected;
    }

    fn press_roll(&mut self, pt: Point, double: bool) {
        let (Some(g), Some((_, _, notes, len))) = (self.roll_geom.clone(), self.roll_target()) else { return };
        let shift = self.mods.shift_key();
        match g.hit(&notes, pt) {
            RollHit::Tool(t) => self.roll.tool = t,
            RollHit::Snap => self.roll.snap = next_snap(self.roll.snap),
            RollHit::Quantise => {
                let snap = self.roll.snap.unwrap_or(Ticks::sixteenths(1));
                self.write_notes("quantise", roll_edit::quantise(&notes, &self.roll.selected, snap, len));
            }
            RollHit::Note { index, .. } if double => {
                self.write_notes("delete note", roll_edit::delete(&notes, &[index]));
            }
            RollHit::Note { index, .. } if shift => {
                if let Some(k) = self.roll.selected.iter().position(|&i| i == index) {
                    self.roll.selected.remove(k);
                } else {
                    self.roll.selected.push(index);
                }
            }
            RollHit::Note { index, edge } => {
                if !self.roll.selected.contains(&index) {
                    self.roll.selected = vec![index];
                }
                let (sel, anchor) = (self.roll.selected.clone(), notes[index]);
                self.history.begin_gesture(if edge { "resize notes" } else { "move notes" });
                self.roll_drag = Some(if edge {
                    RollDrag::Resize { notes, sel, anchor, from: pt }
                } else {
                    RollDrag::Move { notes, sel, anchor, from: pt }
                });
            }
            RollHit::Grid { tick, pitch } if self.roll.tool == Tool::Draw => {
                let start = roll_edit::snap_floor(tick, self.roll.snap);
                let note = Note { start, len: self.note_len, pitch, velocity: 0.8, chance: 1.0 };
                self.history.begin_gesture("draw note");
                let e = roll_edit::add(&notes, note, len);
                let (sel, anchor) = (e.selected.clone(), e.notes[e.selected[0]]);
                let after = e.notes.clone();
                self.write_notes("draw note", e);
                // keep dragging to set its length
                self.roll_drag = Some(RollDrag::Resize { notes: after, sel, anchor, from: pt });
            }
            RollHit::Grid { .. } => {
                let keep = if shift { self.roll.selected.clone() } else { Vec::new() };
                self.roll.selected = keep.clone();
                self.roll_drag = Some(RollDrag::Box { from: pt, keep });
                self.history.begin_gesture("select");
            }
            RollHit::Velocity { index: Some(i), value } => {
                if !self.roll.selected.contains(&i) {
                    self.roll.selected = vec![i];
                }
                let sel = self.roll.selected.clone();
                self.history.begin_gesture("velocity");
                self.write_notes("velocity", roll_edit::set_velocity(&notes, &sel, value));
                self.roll_drag = Some(RollDrag::Velocity { notes, sel });
            }
            RollHit::Key(pitch) => {
                self.roll.selected =
                    notes.iter().enumerate().filter(|(_, n)| n.pitch == pitch).map(|(i, _)| i).collect();
            }
            RollHit::Velocity { index: None, .. } | RollHit::Nothing => {
                if !shift {
                    self.roll.selected.clear();
                }
            }
        }
    }

    fn roll_drag_to(&mut self) {
        let (Some(d), Some(g), Some((_, _, _, len))) =
            (self.roll_drag.clone(), self.roll_geom.clone(), self.roll_target())
        else {
            return;
        };
        let l = &g.layout;
        let snap = if self.mods.alt_key() { None } else { self.roll.snap };
        let pt = self.cursor;
        match d {
            RollDrag::Move { notes, sel, anchor, from } => {
                let raw = l.tick_at(pt.x).0 - l.tick_at(from.x).0;
                let start = roll_edit::snap_round(Ticks(anchor.start.0 + raw), snap);
                let dp = l.pitch_at(pt.y) as i32 - l.pitch_at(from.y) as i32;
                self.write_notes("move notes", roll_edit::move_by(&notes, &sel, start.0 - anchor.start.0, dp, len));
            }
            RollDrag::Resize { notes, sel, anchor, from } => {
                let raw = l.tick_at(pt.x).0 - l.tick_at(from.x).0;
                let end = anchor.start + anchor.len;
                let new_end = roll_edit::snap_round(Ticks(end.0 + raw), snap);
                let min = snap.unwrap_or(Ticks(sat_doc::sat_time::PPQ / 16));
                let e = roll_edit::resize_by(&notes, &sel, new_end.0 - end.0, min, len);
                if let Some(&i) = e.selected.first() {
                    self.note_len = e.notes[i].len;
                }
                self.write_notes("resize notes", e);
            }
            RollDrag::Velocity { notes, sel } => {
                self.write_notes("velocity", roll_edit::set_velocity(&notes, &sel, l.vel_at(pt.y)));
            }
            RollDrag::Box { from, keep } => {
                let notes = self.roll_target().map(|t| t.2).unwrap_or_default();
                let b = Rect::from_points(from, pt).intersect(l.grid);
                let mut sel = roll_edit::in_box(
                    &notes,
                    l.tick_at(b.x0),
                    l.tick_at(b.x1),
                    l.pitch_at(b.y1 - 0.5),
                    l.pitch_at(b.y0 + 0.5),
                );
                for k in keep {
                    if !sel.contains(&k) {
                        sel.push(k);
                    }
                }
                self.roll.rubber = Some(b);
                self.roll.selected = sel;
            }
        }
    }

    /// Keys for the piano roll; returns whether the key was used.
    fn roll_key(&mut self, key: &Key) -> bool {
        if self.panel != PanelView::Clip || self.roll_drag.is_some() || self.mods.alt_key() || !self.panel_focus {
            return false;
        }
        let Some((_, _, notes, len)) = self.roll_target() else { return false };
        let sel = self.roll.selected.clone();
        let ctrl = self.mods.control_key() || self.mods.super_key();
        let step = self.roll.snap.unwrap_or(Ticks::sixteenths(1)).0;
        let octave = if self.mods.shift_key() { 12 } else { 1 };
        match key {
            Key::Named(NamedKey::Delete | NamedKey::Backspace) if !sel.is_empty() => {
                self.write_notes("delete notes", roll_edit::delete(&notes, &sel));
            }
            Key::Named(NamedKey::ArrowUp) if !sel.is_empty() => {
                self.write_notes("transpose", roll_edit::move_by(&notes, &sel, 0, octave, len));
            }
            Key::Named(NamedKey::ArrowDown) if !sel.is_empty() => {
                self.write_notes("transpose", roll_edit::move_by(&notes, &sel, 0, -octave, len));
            }
            Key::Named(NamedKey::ArrowLeft) if !sel.is_empty() => {
                self.write_notes("move notes", roll_edit::move_by(&notes, &sel, -step, 0, len));
            }
            Key::Named(NamedKey::ArrowRight) if !sel.is_empty() => {
                self.write_notes("move notes", roll_edit::move_by(&notes, &sel, step, 0, len));
            }
            Key::Named(NamedKey::Escape) => self.roll.selected.clear(),
            Key::Character(c) if ctrl && c.eq_ignore_ascii_case("a") => self.roll.selected = (0..notes.len()).collect(),
            _ => return false,
        }
        true
    }

    fn press(&mut self) {
        let now = Instant::now();
        let pt = self.cursor;
        let double =
            self.last_click.is_some_and(|(t, p)| now.duration_since(t) < DOUBLE_CLICK && (p - pt).hypot() < 4.0);
        self.last_click = Some((now, pt));
        if self.palette.is_some() {
            let hit = self.palette_layout.as_ref().and_then(|l| l.hit(pt)).map(|e| e.action);
            self.palette = None;
            if let Some(a) = hit {
                self.run(a);
            }
            return;
        }
        self.panel_focus = self.shell.is_some_and(|sl| sl.panel.contains(pt));
        if let Some(sl) = self.shell
            && sl.panel.contains(pt)
        {
            return self.press_panel(sl, pt, double);
        }
        if let Some(sl) = self.shell
            && sl.topbar.contains(pt)
        {
            match shell::transport_buttons(sl.topbar).into_iter().find(|(_, r)| r.contains(pt)).map(|(b, _)| b) {
                Some(TransportButton::Stop) => {
                    self.stop_recording();
                    if self.playback.playing {
                        self.playback.stop();
                    } else {
                        self.playback.locate(Ticks::ZERO);
                    }
                    return;
                }
                Some(TransportButton::Play) => return self.toggle_play(),
                Some(TransportButton::Record) => {
                    return if self.rec.is_some() { self.stop_recording() } else { self.start_recording() };
                }
                Some(TransportButton::Loop) => {
                    let mut range = self.history.doc().loop_range;
                    range.enabled = !range.enabled;
                    return self.edit("loop", vec![Op::SetLoop { range }]);
                }
                None => {}
            }
            match shell::view_buttons(sl.topbar).into_iter().find(|(_, r)| r.contains(pt)).map(|(b, _)| b) {
                Some(ViewButton::Arrange) => self.view = MainView::Arrange,
                Some(ViewButton::Session) => self.view = MainView::Session,
                Some(ViewButton::SourceGrid) => self.open_grid(self.arrange.selected_track, GridKind::Source),
                Some(ViewButton::Instrument) => self.open_grid(self.arrange.selected_track, GridKind::Instrument),
                Some(ViewButton::Fx) => self.open_grid(self.arrange.selected_track, GridKind::Fx),
                _ if shell::search_box(sl.topbar).contains(pt) => self.palette = Some(PaletteView::default()),
                _ => {}
            }
            return;
        }
        if matches!(self.view, MainView::Grid(_)) {
            return self.press_grid(pt, double);
        }
        if let Some(sl) = self.shell
            && !self.session.following.is_empty()
            && sat_ui::session::back_button(sl.arrange).contains(pt)
        {
            return self.back_to_arrangement();
        }
        if self.view == MainView::Session {
            return self.press_session(pt, double);
        }
        let Some(layout) = &self.layout else { return };
        match layout.hit(pt) {
            Hit::Clip { placement, clip } => {
                self.select_clip(clip);
                self.arrange.selected_placement = Some(placement);
                self.begin_clip_drag(placement, None);
            }
            Hit::ClipEdge { placement, clip, right } => {
                self.select_clip(clip);
                self.arrange.selected_placement = Some(placement);
                self.begin_clip_drag(placement, Some(right));
            }
            Hit::AutoToggle(t) => self.toggle_lanes(t),
            Hit::LaneAdd(t) => self.add_lane(t),
            Hit::LaneRemove(id) => self.edit("remove lane", vec![Op::RemoveLane { id }]),
            hit @ (Hit::AutoPoint { .. } | Hit::AutoLane { .. }) => self.press_lane(hit, double),
            Hit::Snap => self.arrange.snap = sat_ui::arrange::next_snap(self.arrange.snap),
            Hit::Lane(t, at) if double && layout.row_at(pt.y).is_some_and(|r| r.kind == RowKind::Track) => {
                self.draw_clip(t, at)
            }
            Hit::Take { placement, clip, take, at } => {
                self.select_clip(clip);
                let doc = self.history.doc();
                let (Some(c), Some(p)) = (doc.clips.get(&clip), doc.placements.get(&placement)) else { return };
                let Some(a) = c.audio.clone() else { return };
                let after = comp_segment(&a, at, take, c.length);
                let origin = p.start - p.offset;
                self.history.begin_gesture("comp");
                if after != a {
                    self.edit("comp", vec![Op::SetClipAudio { clip, audio: Some(after) }]);
                }
                self.audio_drag = Some(AudioDrag::Comp { clip, take, origin, from: snap16(at), start: a });
            }
            Hit::Fade { placement, clip, out } => {
                self.select_clip(clip);
                let Some(p) = self.history.doc().placements.get(&placement) else { return };
                let origin = p.start - p.offset;
                self.history.begin_gesture("fade");
                self.audio_drag = Some(AudioDrag::Fade { clip, origin, out });
            }
            Hit::Mute(t) => self.toggle_flag(t, TrackFlag::Mute),
            Hit::Solo(t) => self.toggle_flag(t, TrackFlag::Solo),
            Hit::Arm(t) => self.toggle_flag(t, TrackFlag::Arm),
            Hit::Monitor(t, monitor) => {
                if self.history.doc().tracks[&t].monitor != monitor {
                    self.input_failed = false;
                    self.edit("monitor", vec![Op::SetMonitor { id: t, monitor }]);
                }
            }
            Hit::Fader(t) => {
                let row = layout.rows.iter().find(|r| r.track == t).copied();
                if let Some(f) = row.and_then(|r| header_controls(&r, layout.headers).fader) {
                    self.begin_drag(ParamTarget::Gain(t), Axis::Horizontal { travel: f.x1 - f.x0 }, double);
                }
            }
            Hit::Header(t) if double && !self.on_group_chevron(t, pt) => self.open_track(t),
            Hit::Header(t) => {
                self.arrange.selected_track = Some(t);
                let row = layout.rows.iter().find(|r| r.track == t);
                let is_group = self.history.doc().tracks[&t].kind == TrackKind::Group;
                // clicking a group's chevron area folds it
                if is_group
                    && row.is_some_and(|r| pt.x < layout.headers.x0 + 40.0 && pt.y < r.y + 28.0)
                    && !self.arrange.collapsed.remove(&t)
                {
                    self.arrange.collapsed.insert(t);
                }
            }
            Hit::Ruler(t) => {
                let snapped = Ticks((t.0 / 960) * 960).max(Ticks::ZERO);
                self.playback.locate(snapped);
            }
            Hit::Lane(..) | Hit::Nothing => self.arrange.selected_placement = None,
        }
    }

    /// The track's lanes: shown or hidden, or a gain lane started when it has none.
    fn toggle_lanes(&mut self, t: TrackId) {
        let doc = self.history.doc();
        let has = automation::lane_targets(t).iter().any(|x| automation::lane_of(doc, x).is_some());
        if has {
            if !self.arrange.hidden_lanes.remove(&t) {
                self.arrange.hidden_lanes.insert(t);
            }
        } else {
            self.edit("add lane", vec![Op::AddLane { id: LaneId::new(), target: ParamTarget::Gain(t) }]);
            self.arrange.hidden_lanes.remove(&t);
        }
    }

    /// Adds the track's next lane that does not exist yet (gain, then pan).
    fn add_lane(&mut self, t: TrackId) {
        let doc = self.history.doc();
        if let Some(target) = automation::lane_targets(t).into_iter().find(|x| automation::lane_of(doc, x).is_none()) {
            self.edit("add lane", vec![Op::AddLane { id: LaneId::new(), target }]);
        }
    }

    /// A press on a lane: on a point, drag it (double-click deletes it); on
    /// empty space, add a point there (creating the lane if needed) and drag it.
    fn press_lane(&mut self, hit: Hit, double: bool) {
        let snap = if self.mods.alt_key() { None } else { Some(Ticks::sixteenths(1)) };
        match hit {
            Hit::AutoPoint { lane, index } => {
                let Some(l) = self.history.doc().lanes.get(&lane) else { return };
                let pts: Vec<Breakpoint> = l.points.iter().copied().collect();
                if double {
                    self.edit(
                        "delete point",
                        vec![Op::SetLanePoints { id: lane, points: automation::remove(&pts, index).into() }],
                    );
                    self.arrange.active_point = None;
                    return;
                }
                self.history.begin_gesture("move point");
                self.arrange.active_point = Some((lane, index));
                self.auto_drag = Some((lane, index, pts));
            }
            Hit::AutoLane { track, lane, tick, pos } => {
                let doc = self.history.doc();
                let mut ops = Vec::new();
                let (id, target, pts) = match lane.and_then(|id| doc.lanes.get(&id)) {
                    Some(l) => (l.id, l.target.clone(), l.points.iter().copied().collect::<Vec<_>>()),
                    None => {
                        let id = LaneId::new();
                        ops.push(Op::AddLane { id, target: ParamTarget::Gain(track) });
                        (id, ParamTarget::Gain(track), Vec::new())
                    }
                };
                let tick = arrange_edit::snap_round(tick, snap).max(Ticks::ZERO);
                let (after, index) = automation::insert(&pts, tick, automation::value_at(doc, &target, pos));
                ops.push(Op::SetLanePoints { id, points: after.clone().into() });
                self.history.begin_gesture("add point");
                self.edit("add point", ops);
                self.arrange.active_point = Some((id, index));
                self.auto_drag = Some((id, index, after));
            }
            _ => {}
        }
    }

    fn auto_drag_to(&mut self) {
        let (Some((lane, index, pts)), Some(l)) = (self.auto_drag.clone(), self.layout.as_ref()) else { return };
        let doc = self.history.doc();
        let Some(target) = doc.lanes.get(&lane).map(|x| x.target.clone()) else { return };
        let Some(a) = l.autos.iter().find(|a| a.lane == Some(lane)) else { return };
        let snap = if self.mods.alt_key() { None } else { Some(Ticks::sixteenths(1)) };
        let tick = arrange_edit::snap_round(l.tick_at(self.cursor.x), snap).max(Ticks::ZERO);
        let value = automation::value_at(doc, &target, automation::pos_at(a.rect, self.cursor.y));
        let after = automation::move_point(&pts, index, tick, value);
        if doc.lanes.get(&lane).is_some_and(|x| x.points.iter().ne(after.iter())) {
            self.edit("move point", vec![Op::SetLanePoints { id: lane, points: after.into() }]);
        }
    }

    /// Starts moving a placement or, given an edge (`Some(right)`), trimming
    /// it. Alt-dragging a clip's body drags a copy and leaves the original.
    fn begin_clip_drag(&mut self, placement: PlacementId, edge: Option<bool>) {
        let Some(p) = self.history.doc().placements.get(&placement).cloned() else { return };
        let from = Span::of(&p);
        let x = self.cursor.x;
        if let Some(right) = edge {
            self.history.begin_gesture("trim clip");
            self.clip_drag = Some(ClipDrag::Trim { placement, from, x, right });
            return;
        }
        let copy = self.mods.alt_key();
        self.history.begin_gesture(if copy { "copy clip" } else { "move clip" });
        self.clip_drag = Some(ClipDrag::Move { placement, from, x, copy });
    }

    fn clip_drag_to(&mut self) {
        let Some(mut d) = self.clip_drag.clone() else { return };
        if let ClipDrag::Move { placement, from, x, copy: true } = d {
            if (self.cursor.x - x).abs() < 3.0 {
                return;
            }
            let doc = self.history.doc();
            let Some(p) = doc.placements.get(&placement) else { return };
            let Some((clip, pid, ops)) = arrange_edit::copy_ops(doc, p, from) else { return };
            self.edit("copy clip", ops);
            self.select_clip(clip);
            self.arrange.selected_placement = Some(pid);
            d = ClipDrag::Move { placement: pid, from, x, copy: false };
            self.clip_drag = Some(d.clone());
        }
        let Some(l) = self.layout.as_ref() else { return };
        // alt trims off the grid (alt at the press already chose a copy for moves)
        let snap = if self.mods.alt_key() && matches!(d, ClipDrag::Trim { .. }) { None } else { self.arrange.snap };
        let doc = self.history.doc();
        let (placement, span) = match d {
            ClipDrag::Move { placement, from, x, .. } => {
                let delta = l.tick_at(self.cursor.x).0 - l.tick_at(x).0;
                (placement, arrange_edit::moved(from, delta, snap))
            }
            ClipDrag::Trim { placement, from, x, right } => {
                let Some(clip) = doc.placements.get(&placement).and_then(|p| doc.clips.get(&p.clip)) else { return };
                let delta = l.tick_at(self.cursor.x).0 - l.tick_at(x).0;
                (placement, arrange_edit::trimmed(from, clip, right, delta, snap))
            }
        };
        if doc.placements.get(&placement).is_some_and(|p| Span::of(p) != span) {
            self.edit("clip", vec![span.op(placement)]);
        }
    }

    /// Double-click on an empty lane: a new looping clip there, a bar (or one
    /// snap step, if longer) long, fed by the track's first source module.
    fn draw_clip(&mut self, track: TrackId, at: Ticks) {
        let doc = self.history.doc();
        let Some(tr) = doc.tracks.get(&track).filter(|t| t.kind == TrackKind::Track) else { return };
        let bar = Ticks::quarters(4);
        let step = self.arrange.snap.unwrap_or(Ticks::sixteenths(1)).0.max(1);
        let start = Ticks(at.0 - at.0.rem_euclid(step));
        let length = bar.max(self.arrange.snap.unwrap_or(bar));
        let feed = tr.source_grid.and_then(|g| {
            doc.modules_on(g).find(|m| sat_modules::ports(&m.kind).iter().any(|p| p.key == "notes" && !p.input))
        });
        let (id, pid) = (ClipId::new(), PlacementId::new());
        let n = doc.clips_on(track).count();
        let mut ops = vec![
            Op::AddClip { id, track, name: format!("clip {}", n + 1), color: tr.color, length, looped: true, seed: 1 },
            Op::AddPlacement { id: pid, clip: id, start, offset: Ticks::ZERO, length },
        ];
        if let Some(m) = feed {
            ops.push(Op::Connect { id: CableId::new(), from: PortRef::new(m.id, "notes"), to: Endpoint::Inlet(id) });
        }
        self.edit("draw clip", ops);
        self.select_clip(id);
        self.arrange.selected_placement = Some(pid);
        self.panel = PanelView::Clip;
    }

    /// Keys for the selected placement while the main view has focus:
    /// delete, ctrl+d (a copy right after it), left / right (nudge by the
    /// snap), ctrl+l (loop over it), escape (deselect).
    fn arrange_key(&mut self, key: &Key) -> bool {
        if self.panel_focus || self.view != MainView::Arrange || self.clip_drag.is_some() {
            return false;
        }
        let doc = self.history.doc();
        let Some(p) = self.arrange.selected_placement.and_then(|p| doc.placements.get(&p)).cloned() else {
            return false;
        };
        let ctrl = self.mods.control_key() || self.mods.super_key();
        let from = Span::of(&p);
        let step = self.arrange.snap.unwrap_or(Ticks::sixteenths(1)).0;
        match key {
            Key::Named(NamedKey::Delete | NamedKey::Backspace) => {
                let ops = arrange_edit::remove_ops(doc, p.id);
                self.edit("delete clip", ops);
                self.arrange.selected_placement = None;
            }
            Key::Character(c) if ctrl && c.eq_ignore_ascii_case("d") => {
                let to = Span { start: from.end(), ..from };
                let Some((clip, pid, ops)) = arrange_edit::copy_ops(doc, &p, to) else { return true };
                self.edit("duplicate clip", ops);
                self.select_clip(clip);
                self.arrange.selected_placement = Some(pid);
            }
            Key::Character(c) if ctrl && c.eq_ignore_ascii_case("l") => {
                let range = sat_doc::LoopRange { start: from.start, end: from.end(), enabled: true };
                self.edit("loop to clip", vec![Op::SetLoop { range }]);
            }
            Key::Named(NamedKey::ArrowLeft | NamedKey::ArrowRight) if !self.mods.alt_key() => {
                let d = if matches!(key, Key::Named(NamedKey::ArrowLeft)) { -step } else { step };
                let to = Span { start: Ticks((from.start.0 + d).max(0)), ..from };
                if to != from {
                    self.edit("nudge clip", vec![to.op(p.id)]);
                }
            }
            Key::Named(NamedKey::Escape) => self.arrange.selected_placement = None,
            _ => return false,
        }
        true
    }

    /// The pointer shape for what is under it: a resize arrow on clip edges.
    fn update_cursor(&mut self) {
        use winit::window::CursorIcon;
        let icon = match &self.clip_drag {
            Some(ClipDrag::Trim { .. }) => CursorIcon::EwResize,
            Some(ClipDrag::Move { .. }) => CursorIcon::Grabbing,
            None if self.view == MainView::Arrange && self.palette.is_none() => {
                match self.layout.as_ref().map(|l| l.hit(self.cursor)) {
                    Some(Hit::ClipEdge { .. }) => CursorIcon::EwResize,
                    _ => CursorIcon::Default,
                }
            }
            None => CursorIcon::Default,
        };
        if icon != self.cursor_icon {
            self.cursor_icon = icon;
            if let Some(g) = &self.gfx {
                g.window.set_cursor(icon);
            }
        }
    }

    /// Whether `pt` is on group `t`'s fold chevron in the arrange headers.
    fn on_group_chevron(&self, t: TrackId, pt: Point) -> bool {
        let Some(layout) = &self.layout else { return false };
        let is_group = self.history.doc().tracks.get(&t).is_some_and(|t| t.kind == TrackKind::Group);
        is_group
            && layout
                .rows
                .iter()
                .find(|r| r.track == t)
                .is_some_and(|r| pt.x < layout.headers.x0 + 40.0 && pt.y < r.y + 28.0)
    }

    fn press_panel(&mut self, sl: ShellLayout, pt: Point, double: bool) {
        match panel::hit_tab(sl.panel, pt) {
            Some(Tab::Clip) => return self.panel = PanelView::Clip,
            Some(Tab::Mixer) => return self.panel = PanelView::Mixer,
            Some(Tab::Macros) => return self.panel = PanelView::Macros,
            None => {}
        }
        if self.panel == PanelView::Clip {
            if piano_roll::open_grid_button(sl.panel).contains(pt) {
                let track = self.arrange.selected_clip.and_then(|c| self.history.doc().clips.get(&c)).map(|c| c.track);
                return self.open_grid(track, GridKind::Source);
            }
            if let (Some(l), Some(clip)) = (&self.audio_layout, self.arrange.selected_clip) {
                use sat_ui::audio_edit as ae;
                let Some(field) = l.hit(pt) else { return };
                let doc = self.history.doc();
                if double {
                    if let Some(a) = ae::with_value(doc, clip, field, field.default_value()) {
                        self.edit("reset", vec![Op::SetClipAudio { clip, audio: Some(a) }]);
                    }
                    return;
                }
                let start = ae::value(doc, clip, field).unwrap_or(0.0);
                self.history.begin_gesture("audio clip");
                self.audio_drag = Some(AudioDrag::Field { clip, field, from: pt.y, start });
                return;
            }
            if self.step_geom.is_some() {
                return self.press_steps(pt, double);
            }
            return self.press_roll(pt, double);
        }
        if self.panel == PanelView::Macros {
            return self.press_macros(pt, double);
        }
        let Some(m) = &self.mixer else { return };
        match m.hit(pt) {
            MixHit::Fader(t) => {
                let travel = m.strip(t).map(|s| s.fader.height()).unwrap_or(100.0);
                self.begin_drag(ParamTarget::Gain(t), Axis::Vertical { travel }, double);
            }
            MixHit::Knob(t, k) => {
                let target = match k {
                    Knob::Pan => ParamTarget::Pan(t),
                    Knob::Send(r) => ParamTarget::SendLevel { track: t, target: r },
                };
                self.begin_drag(target, Axis::Vertical { travel: KNOB_TRAVEL }, double);
            }
            MixHit::Mute(t) => self.toggle_flag(t, TrackFlag::Mute),
            MixHit::Solo(t) => self.toggle_flag(t, TrackFlag::Solo),
            MixHit::Grid(t, kind) => self.open_grid(Some(t), kind),
            MixHit::Strip(t) if double => self.open_track(t),
            MixHit::Strip(t) => self.select_track(t),
            MixHit::Nothing => {}
        }
    }

    fn press_macros(&mut self, pt: Point, double: bool) {
        use sat_ui::macros::MacroHit;
        let Some(l) = self.macro_layout.clone() else { return };
        match l.hit(pt) {
            MacroHit::Learn => self.learn = !self.learn,
            MacroHit::Knob(grid, index) => {
                let assigned = self.history.doc().grids.get(&grid).is_some_and(|g| g.slots.contains_key(&index));
                if !assigned {
                    // an empty macro: learn maps the next touched parameter to one
                    self.learn = true;
                } else if self.learn {
                    self.edit("clear macro", vec![Op::SetMacroSlot { grid, index, slot: None }]);
                } else {
                    self.begin_drag(ParamTarget::Macro { grid, index }, Axis::Vertical { travel: KNOB_TRAVEL }, double);
                }
            }
            MacroHit::Open(grid) => self.show_grid(grid),
            MacroHit::Fader(t) => {
                self.begin_drag(ParamTarget::Gain(t), Axis::Vertical { travel: l.fader.height() }, double)
            }
            MacroHit::Pan(t) => self.begin_drag(ParamTarget::Pan(t), Axis::Vertical { travel: KNOB_TRAVEL }, double),
            MacroHit::Nothing => {}
        }
    }

    fn press_steps(&mut self, pt: Point, double: bool) {
        let Some(g) = self.step_geom.clone() else { return };
        let doc = self.history.doc();
        let Some(m) = doc.modules.get(&g.module) else { return };
        let (module, clip) = (g.module, g.clip);
        match g.hit(pt) {
            StepHit::Cell { step, pitch } => {
                let start = g.tick_of(step);
                let ops = match g.editor {
                    Editor::Euclid | Editor::Turing => {
                        let Some(s) = sat_modules::gen_steps(doc, module, clip) else { return };
                        let note = sat_modules::step_note(start, s.note_len, pitch, s.velocity);
                        sat_modules::toggle_step(doc, module, clip, note)
                    }
                    _ => {
                        let notes = sat_modules::effective_notes(doc, module, clip);
                        let on = notes.iter().find(|n| n.start == start && n.pitch == pitch).copied();
                        match on {
                            // shift-click cycles an existing step's velocity
                            Some(n) if self.mods.shift_key() => {
                                let mut v: Vec<Note> = notes.clone();
                                for x in v.iter_mut().filter(|x| x.start == start && x.pitch == pitch) {
                                    x.velocity = drum::next_velocity(n.velocity);
                                }
                                vec![Op::WritePattern { module, clip, notes: im::Vector::from(v) }]
                            }
                            _ => {
                                let note = sat_modules::step_note(start, g.rate, pitch, drum::VELOCITIES[0]);
                                sat_modules::toggle_step(doc, module, clip, note)
                            }
                        }
                    }
                };
                self.edit("step", ops);
            }
            StepHit::Page(i) => self.steps.page = i,
            StepHit::Knob(param) => {
                let target = ParamTarget::Module { module, param: param.into() };
                self.begin_drag(target, Axis::Vertical { travel: KNOB_TRAVEL }, double);
            }
            StepHit::Mode => {
                let want_fixed = g.mode.is_some_and(|r| pt.x >= r.center().x);
                let ops = match (want_fixed, sat_modules::generating(m)) {
                    (true, true) => sat_modules::bake(doc, module, clip),
                    (false, false) => sat_modules::unbake(module),
                    _ => Vec::new(),
                };
                if !ops.is_empty() {
                    self.edit(if want_fixed { "bake" } else { "generate" }, ops);
                }
            }
            StepHit::Clear => {
                let ops = sat_modules::clear_edits(doc, module, clip);
                if !ops.is_empty() {
                    self.edit("clear edits", ops);
                }
            }
            StepHit::Bake => {
                if sat_modules::generating(m) {
                    self.edit("bake", sat_modules::bake(doc, module, clip));
                }
            }
            StepHit::Nothing => {}
        }
    }

    /// Shows `track`'s grid of `kind`, if it has one.
    fn open_grid(&mut self, track: Option<TrackId>, kind: GridKind) {
        let Some(t) = track else { return };
        let Some((_, g)) = grid::grid_kinds(self.history.doc(), t).into_iter().find(|(k, _)| *k == kind) else {
            return;
        };
        self.show_grid(g);
        self.arrange.selected_track = Some(t);
    }

    /// Shows grid `g`, remembering the view to go back to.
    fn show_grid(&mut self, g: sat_doc::GridId) {
        if !matches!(self.view, MainView::Grid(_)) {
            self.prev_view = self.view;
        }
        if self.view != MainView::Grid(g) {
            self.grid = GridView::default();
        }
        self.grid.back_label = if self.prev_view == MainView::Session { "session" } else { "arrange" };
        self.view = MainView::Grid(g);
    }

    /// Leaves a grid for the view it was opened from.
    fn leave_grid(&mut self) {
        self.view = self.prev_view;
    }

    /// The kind of grid on screen, if one is.
    fn grid_kind(&self) -> Option<GridKind> {
        let MainView::Grid(g) = self.view else { return None };
        self.history.doc().grids.get(&g).map(|g| g.kind)
    }

    /// Selects `t`; a grid on screen follows to `t`'s grid of the same kind.
    fn select_track(&mut self, t: TrackId) {
        self.arrange.selected_track = Some(t);
        if let Some(kind) = self.grid_kind() {
            self.open_grid(Some(t), kind);
        }
    }

    /// Steps the selected track up or down. In a grid, it steps to the
    /// nearest track that has a grid of the same kind and shows that.
    fn step_track(&mut self, down: bool) {
        let doc = self.history.doc();
        let kind = self.grid_kind();
        let order: Vec<TrackId> = doc
            .tracks_in_order()
            .into_iter()
            .filter(|t| match kind {
                Some(k) => grid::grid_kinds(doc, *t).iter().any(|(gk, _)| *gk == k),
                None => doc.tracks[t].kind != TrackKind::Master,
            })
            .collect();
        if order.is_empty() {
            return;
        }
        let here = self.arrange.selected_track.and_then(|t| order.iter().position(|x| *x == t));
        let n = order.len();
        let next = match here {
            Some(i) if down => (i + 1) % n,
            Some(i) => (i + n - 1) % n,
            None => 0,
        };
        self.select_track(order[next]);
    }

    /// The grid a double-click on a track opens: a track's instrument, a bus's fx.
    fn open_track(&mut self, t: TrackId) {
        let kind = match self.history.doc().tracks.get(&t).map(|t| t.kind) {
            Some(TrackKind::Track) => GridKind::Instrument,
            _ => GridKind::Fx,
        };
        self.open_grid(Some(t), kind);
    }

    fn press_grid(&mut self, pt: Point, double: bool) {
        let Some(l) = self.grid_layout.clone() else { return };
        let hit = l.hit(pt);
        if !matches!(hit, GridHit::MenuItem(_) | GridHit::AddModule) {
            self.grid.menu = false;
        }
        match hit {
            GridHit::Back => self.leave_grid(),
            GridHit::PrevTrack => self.step_track(false),
            GridHit::NextTrack => self.step_track(true),
            GridHit::Kind(kind) => self.open_grid(Some(l.track), kind),
            GridHit::Track(t) => {
                // the same kind of grid if the track has one, else its fx
                let has = grid::grid_kinds(self.history.doc(), t).iter().any(|(k, _)| *k == l.kind);
                self.open_grid(Some(t), if has { l.kind } else { GridKind::Fx });
            }
            GridHit::TrackMute(t) => self.toggle_flag(t, TrackFlag::Mute),
            GridHit::TrackSolo(t) => self.toggle_flag(t, TrackFlag::Solo),
            GridHit::AddModule => self.grid.menu = !self.grid.menu,
            GridHit::MenuItem(kind) => {
                let doc = self.history.doc();
                let (id, ops) = if l.kind == GridKind::Fx {
                    // effects go into the chain, just before out
                    sat_modules::fx::insert(doc, l.grid, kind)
                } else {
                    let pos = grid::free_pos(doc, l.grid);
                    let (id, op) = sat_modules::add_module(doc, l.grid, kind, pos);
                    (id, vec![op])
                };
                self.edit("add module", ops);
                self.grid.selected = Some(id);
                self.grid.menu = false;
            }
            GridHit::Header(id) => {
                self.grid.selected = Some(id);
                if let Some(start) = self.history.doc().modules.get(&id).map(|m| m.pos) {
                    self.history.begin_gesture("move module");
                    self.grid_drag = Some(GridDrag::Module { id, from: pt, start });
                }
            }
            GridHit::Out(id, key) => {
                let start = l.card(id).and_then(|c| c.outs.iter().find(|(k, _)| *k == key)).map_or(pt, |(_, p)| *p);
                let kind = grid::port_kind(self.history.doc(), id, key);
                self.grid.wire = Some((start, pt, kind));
                self.grid_drag = Some(GridDrag::Wire { from: PortRef::new(id, key), kind, start, detach: None });
            }
            GridHit::In(id, key) => {
                // pick the cable up off the input; dropping it elsewhere repatches or unplugs it
                let doc = self.history.doc();
                let to = Endpoint::Port(PortRef::new(id, key));
                if let Some(k) = doc.cables.values().find(|k| k.to == to).cloned() {
                    let start = l.wires.iter().find(|w| w.cable == k.id).map_or(pt, |w| w.from);
                    let kind = grid::port_kind(doc, id, key);
                    self.grid.wire = Some((start, pt, kind));
                    self.grid_drag = Some(GridDrag::Wire { from: k.from, kind, start, detach: Some(k.id) });
                } else {
                    self.grid.selected = Some(id);
                }
            }
            GridHit::Param(id, key) => {
                self.grid.selected = Some(id);
                let travel = l.card(id).and_then(|c| c.params.first()).map_or(150.0, |(_, r)| r.width());
                let target = ParamTarget::Module { module: id, param: key.into() };
                self.begin_drag(target, Axis::Horizontal { travel }, double);
            }
            GridHit::Module(id) => self.grid.selected = Some(id),
            GridHit::Sound(id) => {
                self.grid.selected = Some(id);
                let dialog = rfd::FileDialog::new().set_title("Load a sound").add_filter("WAV audio", &["wav", "WAV"]);
                if let Some(path) = dialog.pick_file() {
                    self.load_sound(Some(id), l.grid, &path);
                }
            }
            GridHit::Bypass(id) => {
                let on = self.history.doc().modules.get(&id).is_some_and(sat_modules::fx::bypassed);
                let target = ParamTarget::Module { module: id, param: sat_modules::fx::BYPASS.into() };
                self.edit(
                    if on { "enable" } else { "bypass" },
                    vec![Op::SetValue { target, value: if on { 0.0 } else { 1.0 } }],
                );
            }
            GridHit::Inlet(clip) => {
                // pick the wire up off the inlet; dropping it elsewhere rewires or unplugs it
                if let Some(k) = self.history.doc().inlet_cable(clip).cloned() {
                    let start = l.wires.iter().find(|w| w.cable == k.id).map_or(pt, |w| w.from);
                    self.grid.wire = Some((start, pt, PortKind::Notes));
                    let kind = PortKind::Notes;
                    self.grid_drag = Some(GridDrag::Wire { from: k.from, kind, start, detach: Some(k.id) });
                }
            }
            GridHit::Row(clip) => self.select_clip(clip),
            GridHit::AddClip => self.add_clip(l.track),
            GridHit::Canvas => {
                self.grid.selected = None;
                self.grid_drag = Some(GridDrag::Pan { from: pt, start: self.grid.scroll });
            }
            GridHit::Nothing => {}
        }
    }

    fn audio_drag_to(&mut self) {
        let Some(d) = self.audio_drag.clone() else { return };
        let Some(l) = self.layout.as_ref() else { return };
        let doc = self.history.doc();
        match d {
            AudioDrag::Comp { clip, take, origin, from, start } => {
                let Some(c) = doc.clips.get(&clip) else { return };
                let at = l.tick_at(self.cursor.x) - origin;
                let to = snap16(at);
                // under a sixteenth of travel it is still a click on one segment
                let after = if to == from {
                    comp_segment(&start, at, take, c.length)
                } else {
                    start.comped(from, to, take, c.length)
                };
                if c.audio.as_ref() != Some(&after) {
                    self.edit("comp", vec![Op::SetClipAudio { clip, audio: Some(after) }]);
                }
            }
            AudioDrag::Field { clip, field, from, start } => {
                let fine = if self.mods.shift_key() { 0.1 } else { 1.0 };
                let v = start + (from - self.cursor.y) * field.per_px() * fine;
                if let Some(a) = sat_ui::audio_edit::with_value(doc, clip, field, v)
                    && doc.clips.get(&clip).and_then(|c| c.audio.as_ref()) != Some(&a)
                {
                    self.edit("audio clip", vec![Op::SetClipAudio { clip, audio: Some(a) }]);
                }
            }
            AudioDrag::Fade { clip, origin, out } => {
                let Some(c) = doc.clips.get(&clip) else { return };
                let Some(mut a) = c.audio.clone() else { return };
                let at = (l.tick_at(self.cursor.x) - origin).0;
                let len = c.length.0;
                if out {
                    a.fade_out = Ticks((len - at).clamp(0, len - a.fade_in.0));
                } else {
                    a.fade_in = Ticks(at.clamp(0, len - a.fade_out.0));
                }
                if c.audio.as_ref() != Some(&a) {
                    self.edit("fade", vec![Op::SetClipAudio { clip, audio: Some(a) }]);
                }
            }
        }
    }

    fn grid_drag_to(&mut self) {
        let Some(d) = self.grid_drag.clone() else { return };
        let pt = self.cursor;
        match d {
            GridDrag::Module { id, from, start } => {
                let pos = ((start.0 + (pt.x - from.x) as f32).max(0.0), (start.1 + (pt.y - from.y) as f32).max(0.0));
                if self.history.doc().modules.get(&id).is_some_and(|m| m.pos != pos) {
                    self.edit("move module", vec![Op::MoveModule { id, pos }]);
                }
            }
            GridDrag::Wire { start, kind, .. } => self.grid.wire = Some((start, pt, kind)),
            GridDrag::Pan { from, start } => {
                let s = start - (pt - from);
                self.grid.scroll = Vec2::new(s.x.max(0.0), s.y.max(0.0));
            }
        }
    }

    fn end_grid_drag(&mut self, d: GridDrag) {
        self.grid.wire = None;
        match d {
            GridDrag::Module { .. } => self.history.end_gesture(),
            GridDrag::Wire { from, detach, .. }
                if self.grid_layout.as_ref().is_some_and(|l| l.kind != GridKind::Source) =>
            {
                let target = self.grid_layout.as_ref().and_then(|l| l.input_near(self.cursor));
                let doc = self.history.doc();
                let ops = match target {
                    Some((m, key)) => match sat_modules::patch(doc, &from, &PortRef::new(m, key), detach) {
                        Some(ops) => ops,
                        None => return, // the ports do not fit: leave everything as it was
                    },
                    None => detach.map(|id| Op::Disconnect { id }).into_iter().collect(),
                };
                if !ops.is_empty() {
                    self.edit(if target.is_some() { "patch" } else { "unplug" }, ops);
                }
            }
            GridDrag::Wire { from, detach, .. } => {
                let target = self.grid_layout.as_ref().and_then(|l| l.inlet_near(self.cursor));
                let doc = self.history.doc();
                let mut ops = Vec::new();
                if let Some(id) = detach {
                    if target.is_some_and(|c| doc.inlet_cable(c).is_some_and(|k| k.id == id)) {
                        return; // dropped back where it was
                    }
                    ops.push(Op::Disconnect { id });
                }
                if let Some(clip) = target {
                    if let Some(old) = doc.inlet_cable(clip).filter(|k| Some(k.id) != detach) {
                        ops.push(Op::Disconnect { id: old.id });
                    }
                    ops.push(Op::Connect { id: CableId::new(), from, to: Endpoint::Inlet(clip) });
                }
                if !ops.is_empty() {
                    self.edit(if target.is_some() { "wire" } else { "unplug" }, ops);
                }
            }
            GridDrag::Pan { .. } => {}
        }
    }

    /// Adds a one-bar clip to `track`'s dock, placed after its last placement, and selects it.
    fn add_clip(&mut self, track: TrackId) {
        let doc = self.history.doc();
        let Some(tr) = doc.tracks.get(&track) else { return };
        let n = doc.clips_on(track).count();
        let end = doc
            .placements
            .values()
            .filter(|p| doc.clips.get(&p.clip).is_some_and(|c| c.track == track))
            .map(|p| p.start + p.length)
            .max()
            .unwrap_or(Ticks::ZERO);
        let id = ClipId::new();
        let bar = Ticks::quarters(4);
        let ops = vec![
            Op::AddClip {
                id,
                track,
                name: format!("clip {}", n + 1),
                color: tr.color,
                length: bar,
                looped: true,
                seed: 1,
            },
            Op::AddPlacement {
                id: sat_doc::PlacementId::new(),
                clip: id,
                start: end,
                offset: Ticks::ZERO,
                length: bar,
            },
        ];
        self.edit("add clip", ops);
        self.select_clip(id);
    }

    /// Source grid keys: delete removes the selected module, escape closes the menu or deselects.
    fn grid_key(&mut self, key: &Key) -> bool {
        if !matches!(self.view, MainView::Grid(_)) {
            return false;
        }
        match key {
            Key::Named(NamedKey::Delete | NamedKey::Backspace) => {
                let Some(id) = self.grid.selected.take() else { return false };
                self.edit("remove module", vec![Op::RemoveModule { id }]);
                true
            }
            Key::Named(NamedKey::Escape) if self.grid.menu || self.grid.selected.is_some() => {
                self.grid.menu = false;
                self.grid.selected = None;
                true
            }
            Key::Named(NamedKey::Escape) => {
                self.leave_grid();
                true
            }
            _ => false,
        }
    }

    /// Keys while the palette is open: type to filter, arrows to move,
    /// enter to run, escape to close.
    fn palette_key(&mut self, key: &Key) {
        let Some(v) = &mut self.palette else { return };
        let n = palette::matches(self.history.doc(), &v.query).len();
        match key {
            Key::Named(NamedKey::Escape) => self.palette = None,
            Key::Named(NamedKey::Enter) => {
                let i = v.index.min(n.saturating_sub(1));
                let a = palette::matches(self.history.doc(), &v.query).get(i).map(|e| e.action);
                self.palette = None;
                if let Some(a) = a {
                    self.run(a);
                }
            }
            Key::Named(NamedKey::ArrowDown) => {
                v.index = (v.index.min(n.saturating_sub(1)) + 1).min(n.saturating_sub(1))
            }
            Key::Named(NamedKey::ArrowUp) => v.index = v.index.min(n.saturating_sub(1)).saturating_sub(1),
            Key::Named(NamedKey::Backspace) => {
                v.query.pop();
                v.index = 0;
            }
            Key::Named(NamedKey::Space) => {
                v.query.push(' ');
                v.index = 0;
            }
            Key::Character(c) if !(self.mods.control_key() || self.mods.super_key()) => {
                v.query.push_str(c);
                v.index = 0;
            }
            Key::Character(c) if c.eq_ignore_ascii_case("k") => self.palette = None,
            _ => {}
        }
    }

    /// Runs a palette entry.
    fn run(&mut self, a: Action) {
        match a {
            Action::Session => self.view = MainView::Session,
            Action::Arrange => self.view = MainView::Arrange,
            Action::Grid(t, kind) => self.open_grid(Some(t), kind),
            Action::Select(t) => self.select_track(t),
            Action::Panel(p) => self.panel = p,
            Action::PlayStop => self.playback.toggle(),
            Action::Loop => self.toggle_loop(),
            Action::Undo | Action::Redo => self.undo(a == Action::Redo),
            Action::Save => self.save(false),
        }
        self.update_title();
    }

    fn toggle_loop(&mut self) {
        let mut r = self.history.doc().loop_range;
        r.enabled = !r.enabled;
        self.edit("loop", vec![Op::SetLoop { range: r }]);
    }

    fn undo(&mut self, redo: bool) {
        self.end_drag();
        let did = if redo { self.history.redo() } else { self.history.undo() };
        if did.is_some() {
            self.roll.selected.clear();
            self.playback.recompile(self.history.doc());
        }
    }

    fn key(&mut self, key: &Key) {
        if self.palette.is_some() {
            return self.palette_key(key);
        }
        if (self.mods.control_key() || self.mods.super_key())
            && matches!(key, Key::Character(c) if c.eq_ignore_ascii_case("k"))
        {
            self.palette = Some(PaletteView::default());
            return;
        }
        if self.learn && matches!(key, Key::Named(NamedKey::Escape)) {
            self.learn = false;
            return self.update_title();
        }
        if self.grid_key(key) {
            return self.update_title();
        }
        if self.arrange_key(key) {
            return self.update_title();
        }
        if self.roll_key(key) {
            return self.update_title();
        }
        let ctrl = self.mods.control_key() || self.mods.super_key();
        let shift = self.mods.shift_key();
        match key {
            Key::Named(NamedKey::Space) => self.toggle_play(),
            Key::Named(NamedKey::Home) => self.playback.locate(Ticks::ZERO),
            Key::Named(NamedKey::Tab) => {
                // clip, mixer, macros (shift goes back)
                let order = [PanelView::Clip, PanelView::Mixer, PanelView::Macros];
                let i = order.iter().position(|p| *p == self.panel).unwrap_or(0);
                self.panel = order[(i + if shift { 2 } else { 1 }) % 3];
            }
            Key::Character(c) if ctrl && matches!(c.as_str(), "1" | "2" | "3" | "4" | "5") => match c.as_str() {
                "1" => self.view = MainView::Session,
                "2" => self.view = MainView::Arrange,
                "3" => self.open_grid(self.arrange.selected_track, GridKind::Source),
                "4" => self.open_grid(self.arrange.selected_track, GridKind::Instrument),
                _ => self.open_grid(self.arrange.selected_track, GridKind::Fx),
            },
            Key::Character(c) if !ctrl && (c == "[" || c == "]") => self.step_track(c == "]"),
            Key::Named(NamedKey::ArrowUp | NamedKey::ArrowDown) if self.mods.alt_key() => {
                self.step_track(matches!(key, Key::Named(NamedKey::ArrowDown)));
            }
            Key::Character(c) if ctrl && c.eq_ignore_ascii_case("z") => self.undo(shift),
            Key::Character(c) if ctrl && c.eq_ignore_ascii_case("s") => self.save(shift),
            Key::Character(c) if ctrl && c.eq_ignore_ascii_case("o") => self.open(),
            Key::Character(c) if ctrl && c.eq_ignore_ascii_case("n") => self.new_project(),
            Key::Character(c) if ctrl && c.eq_ignore_ascii_case("e") => {
                self.export(if shift { export::Span::Loop } else { export::Span::Song })
            }
            Key::Character(c) if ctrl && c.eq_ignore_ascii_case("q") => self.quit = true,
            Key::Character(c) if !ctrl && c == "l" => self.toggle_loop(),
            Key::Character(c) if !ctrl && (c == "m" || c == "s") => {
                if let Some(t) = self.arrange.selected_track {
                    let kind = self.history.doc().tracks[&t].kind;
                    let flag = if c == "m" { TrackFlag::Mute } else { TrackFlag::Solo };
                    if kind != TrackKind::Master || flag == TrackFlag::Mute {
                        self.toggle_flag(t, flag);
                    }
                }
            }
            _ => {}
        }
        self.update_title();
    }

    fn scroll(&mut self, dx: f64, dy: f64) {
        if matches!(self.view, MainView::Grid(_)) && self.shell.is_some_and(|sl| sl.arrange.contains(self.cursor)) {
            let (dx, dy) = if self.mods.shift_key() { (dy, dx) } else { (dx, dy) };
            self.grid.scroll = Vec2::new((self.grid.scroll.x - dx).max(0.0), (self.grid.scroll.y - dy).max(0.0));
            return;
        }
        if let Some(sl) = self.shell
            && sl.panel.contains(self.cursor)
        {
            if self.panel == PanelView::Mixer {
                let d = if dx.abs() > dy.abs() { dx } else { dy };
                let max = self.mixer.as_ref().map(|m| m.max_scroll).unwrap_or(0.0);
                self.mixer_scroll = (self.mixer_scroll - d).clamp(0.0, max);
            } else if let (Some(g), Some(lo)) = (&self.roll_geom, self.roll.lo) {
                // wheel up shows higher notes, a row per 20 px
                let rows = (dy / 20.0).round() as i32;
                let max = 127 - (g.layout.hi - g.layout.lo) as i32;
                self.roll.lo = Some((lo as i32 + rows).clamp(0, max.max(0)) as u8);
            }
            return;
        }
        if self.mods.control_key() || self.mods.super_key() {
            let k = (1.0 + dy * 0.0015).clamp(0.5, 2.0);
            self.arrange.px_per_bar = (self.arrange.px_per_bar * k).clamp(8.0, 2000.0);
        } else if self.mods.shift_key() || dx.abs() > dy.abs() {
            let d = if self.mods.shift_key() { dy } else { dx };
            let ticks_per_px = 3840.0 / self.arrange.px_per_bar;
            self.arrange.origin = Ticks((self.arrange.origin.0 as f64 - d * ticks_per_px).max(0.0) as i64);
        } else {
            self.arrange.scroll_y = (self.arrange.scroll_y - dy).max(0.0);
        }
    }

    fn quit(&mut self, el: &ActiveEventLoop) {
        self.end_drag();
        self.keep_unsaved();
        self.project.finish();
        el.exit();
    }

    fn metering(&self) -> bool {
        // a monitored input moves its meters even while stopped
        self.levels.values().any(|(l, r)| *l > 0.0 || *r > 0.0)
            || (self.input.is_some() && monitored(self.history.doc()))
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.gfx.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title(format!("sat · {}", self.project.name))
            .with_inner_size(LogicalSize::new(1440.0, 900.0))
            .with_min_inner_size(LogicalSize::new(900.0, 560.0));
        let window = Arc::new(el.create_window(attrs).expect("window"));
        let size = window.inner_size();
        let surface = pollster::block_on(self.ctx.create_surface(
            window.clone(),
            size.width.max(1),
            size.height.max(1),
            wgpu::PresentMode::AutoVsync,
        ))
        .expect("surface");
        let dev = &self.ctx.devices[surface.dev_id];
        let renderer = Renderer::new(
            &dev.device,
            RendererOptions { antialiasing_support: vello::AaSupport::area_only(), ..Default::default() },
        )
        .expect("renderer");
        eprintln!(
            "sat: {} · audio: {}",
            self.ctx.devices[surface.dev_id].adapter().get_info().name,
            self.playback.status
        );
        window.request_redraw();
        self.gfx = Some(Gfx { window, surface, renderer });
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _: WindowId, ev: WindowEvent) {
        let mut dirty = true;
        match ev {
            WindowEvent::CloseRequested => self.quit(el),
            WindowEvent::RedrawRequested => {
                self.redraw();
                dirty = false;
            }
            WindowEvent::Resized(_) => {}
            WindowEvent::ModifiersChanged(m) => self.mods = m.state(),
            WindowEvent::CursorMoved { position, .. } => {
                let scale = self.gfx.as_ref().map(|g| g.window.scale_factor()).unwrap_or(1.0);
                self.cursor = Point::new(position.x / scale, position.y / scale);
                dirty = self.drag.is_some()
                    || self.roll_drag.is_some()
                    || self.grid_drag.is_some()
                    || self.audio_drag.is_some()
                    || self.clip_drag.is_some()
                    || self.auto_drag.is_some();
                self.drag_to();
                self.clip_drag_to();
                self.auto_drag_to();
                self.update_cursor();
                self.audio_drag_to();
                self.roll_drag_to();
                self.grid_drag_to();
            }
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => match state {
                ElementState::Pressed => self.press(),
                ElementState::Released => {
                    self.end_drag();
                    self.update_title();
                }
            },
            WindowEvent::CursorLeft { .. } => dirty = false,
            WindowEvent::MouseWheel { delta, .. } => {
                let (dx, dy) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (x as f64 * 40.0, y as f64 * 40.0),
                    MouseScrollDelta::PixelDelta(p) => (p.x, p.y),
                };
                self.scroll(dx, dy);
            }
            WindowEvent::DroppedFile(path) => {
                if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("wav")) {
                    self.drop_sound(&path);
                } else {
                    self.open_path(&path);
                }
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed && !event.repeat => {
                self.key(&event.logical_key);
            }
            _ => dirty = false,
        }
        if dirty && let Some(g) = &self.gfx {
            g.window.request_redraw();
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        if self.quit {
            return self.quit(el);
        }
        let v = self.history.version();
        if self.drag.is_none() && self.project.autosave(self.history.doc(), self.playback.pool.sounds(), v, false) {
            self.update_title();
        }
        // while playing or meters are falling, redraw at display rate; otherwise
        // sleep until input or the next autosave
        if self.playback.playing || self.metering() {
            if let Some(g) = &self.gfx {
                g.window.request_redraw();
            }
            el.set_control_flow(ControlFlow::WaitUntil(self.last_frame + Duration::from_millis(8)));
        } else if let Some(at) = self.project.next_autosave(v) {
            el.set_control_flow(ControlFlow::WaitUntil(at));
        } else {
            el.set_control_flow(ControlFlow::Wait);
        }
    }
}

/// The sounds of a project bundle; ones that cannot be read are reported
/// and their samplers stay silent.
fn load_sounds(bundle: &Path, doc: &Document) -> sat_disk::Pool {
    let mut pool = sat_disk::Pool::new();
    for (name, e) in pool.load(bundle, doc) {
        eprintln!("sound {name} is missing: {e}");
    }
    pool
}

/// Command line: an optional project bundle, and export flags.
#[derive(Default)]
struct Args {
    project: Option<PathBuf>,
    export: Option<PathBuf>,
    span: Option<export::Span>,
    rate: Option<f64>,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args::default();
    let mut it = std::env::args_os().skip(1);
    while let Some(arg) = it.next() {
        match arg.to_str() {
            Some("--export") => a.export = Some(it.next().ok_or("--export needs a file")?.into()),
            Some("--loop") => a.span = Some(export::Span::Loop),
            Some("--rate") => {
                let v = it.next().ok_or("--rate needs a number")?;
                let r: f64 = v.to_str().and_then(|v| v.parse().ok()).ok_or("--rate needs a number")?;
                if !(8_000.0..=384_000.0).contains(&r) {
                    return Err(format!("--rate {r} is out of range"));
                }
                a.rate = Some(r);
            }
            Some(f) if f.starts_with("--") => return Err(format!("unknown option {f}")),
            _ if a.project.is_none() => a.project = Some(arg.into()),
            _ => return Err("more than one project given".into()),
        }
    }
    if a.export.is_none() && (a.span.is_some() || a.rate.is_some()) {
        return Err("--loop and --rate go with --export".into());
    }
    Ok(a)
}

fn main() {
    let args = parse_args().unwrap_or_else(|e| {
        eprintln!("sat: {e}\nusage: sat [--export out.wav [--loop] [--rate hz]] [project.sat]");
        std::process::exit(2);
    });
    let arg = args.project.clone();
    let (doc, project, selected, pool) = match &arg {
        Some(p) => {
            let mut project = Project::new(None, project::name_of(p), 0);
            let doc = project.open(p, 0).unwrap_or_else(|e| {
                eprintln!("cannot open {}: {e}", p.display());
                std::process::exit(1);
            });
            let first = doc.clips.keys().next().copied();
            let pool = load_sounds(p, &doc);
            (doc, project, first, pool)
        }
        None => {
            let d = sat_ui::demo::demo();
            (d.doc, Project::new(None, "demo set".into(), 0), Some(d.selected), d.sounds)
        }
    };
    if let Some(out) = &args.export {
        let span = args.span.unwrap_or(export::Span::Song);
        let rate = args.rate.unwrap_or(FALLBACK_RATE);
        match export::export(&doc, pool.sounds(), rate, span, out) {
            Ok(secs) => eprintln!("exported {} ({secs:.1} s at {rate} Hz)", out.display()),
            Err(e) => {
                eprintln!("sat: export failed: {e}");
                std::process::exit(1);
            }
        }
        return;
    }
    let playback = Playback::start(&doc, pool);
    let arrange = ArrangeState {
        selected_clip: selected,
        selected_track: selected.and_then(|c| doc.clips.get(&c)).map(|c| c.track),
        ..Default::default()
    };
    let mut app = App {
        project,
        history: History::new(doc, 500),
        arrange,
        panel: PanelView::Clip,
        mixer_scroll: 0.0,
        levels: HashMap::new(),
        playback,
        ctx: RenderContext::new(),
        gfx: None,
        fonts: Fonts::new(),
        theme: sat_theme::Theme::dark(),
        scene: Scene::new(),
        cursor: Point::ZERO,
        mods: ModifiersState::empty(),
        shell: None,
        layout: None,
        mixer: None,
        drag: None,
        roll: RollView::default(),
        roll_geom: None,
        roll_drag: None,
        steps: StepView::default(),
        step_geom: None,
        view: MainView::Arrange,
        prev_view: MainView::Arrange,
        palette: None,
        palette_layout: None,
        grid: GridView::default(),
        grid_layout: None,
        session: SessionView::default(),
        session_layout: None,
        learn: false,
        macro_layout: None,
        grid_drag: None,
        audio_drag: None,
        audio_layout: None,
        clip_drag: None,
        auto_drag: None,
        panel_focus: false,
        cursor_icon: winit::window::CursorIcon::Default,
        note_len: Ticks::sixteenths(1),
        last_click: None,
        last_frame: Instant::now(),
        input: None,
        input_failed: false,
        rec: None,
        quit: false,
    };
    let el = EventLoop::new().expect("event loop");
    el.run_app(&mut app).expect("run");
}
