//! The sat desktop app.
//!
//! Opens a window with the arrange view and bottom panel, plays the project
//! through the default audio device, and lets you mix it:
//!
//! - space: play / stop; home: back to the start; l: loop on / off
//! - click a clip to edit it in the bottom panel; click the ruler to move the playhead
//! - drag faders and knobs (shift for fine steps, double-click to reset);
//!   click m / s / arm buttons; tab switches the bottom panel between clip and mixer
//! - m / s: mute / solo the selected track; ctrl+z / ctrl+shift+z: undo / redo
//! - ctrl+o open (or drop a .sat folder on the window), ctrl+s save, ctrl+shift+s save as,
//!   ctrl+n new, ctrl+q quit
//! - wheel: scroll; shift+wheel: scroll time; ctrl+wheel: zoom
//!
//! `sat [project.sat]` opens a project bundle; without one it opens a demo.

mod audio;
mod project;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use sat_compile::Compiled;
use sat_doc::sat_time::Ticks;
use sat_doc::{Document, History, Op, ParamTarget, TrackFlag, TrackId, TrackKind};
use sat_engine::{Command, EngineHandle};
use sat_ui::arrange::{ArrangeLayout, ArrangeState, Hit, TransportView, header_controls};
use sat_ui::controls::{self, SILENT_DB};
use sat_ui::mixer::{Knob, Levels, MixHit, MixerLayout};
use sat_ui::paint::{Fonts, color};
use sat_ui::panel::{self, PanelView, Tab};
use sat_ui::shell::{self, Frame, PANEL_H, ShellLayout};
use vello::kurbo::Point;
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
    _output: Option<audio::Output>,
    playing: bool,
    /// Used when there is no audio device: a wall clock drives the playhead.
    clock: Option<(Instant, Ticks)>,
    status: String,
}

impl Playback {
    fn start(doc: &Document) -> Playback {
        let rate = audio::device_rate().unwrap_or(FALLBACK_RATE);
        let compiled = sat_compile::compile(doc, rate, MAX_BLOCK).expect("project compiles");
        let (engine, mut handle) = sat_engine::engine(compiled.schedule.clone(), 256);
        let _ = handle.send(Command::SetLoop(compiled.loop_samples(doc)));
        match audio::start(engine, MAX_BLOCK) {
            Ok(out) => Playback {
                status: format!("{} @ {} Hz", out.name, out.sample_rate),
                handle: Some(handle),
                compiled,
                _output: Some(out),
                playing: false,
                clock: None,
            },
            Err(e) => Playback {
                status: format!("no audio ({e})"),
                handle: None,
                compiled,
                _output: None,
                playing: false,
                clock: None,
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
        match sat_compile::compile(doc, rate, MAX_BLOCK) {
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
        _ => 0.0,
    }
}

/// The value a control position sets.
fn value_at(target: &ParamTarget, pos: f64) -> f32 {
    match target {
        ParamTarget::Pan(_) => controls::pan_at(pos),
        _ => controls::fader_db(pos),
    }
}

/// The value double-click resets a control to.
fn default_value(target: &ParamTarget) -> f32 {
    match target {
        ParamTarget::SendLevel { .. } => SILENT_DB,
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
    last_click: Option<(Instant, Point)>,
    last_frame: Instant,
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
        if structural {
            self.playback.recompile(self.history.doc());
        } else {
            for ch in changes {
                if let sat_doc::Change::Value { target, value } = ch
                    && let Some(cmd) = self.playback.compiled.param_command(target, *value)
                {
                    self.playback.send(cmd);
                }
            }
        }
    }

    /// Replaces the document (after open or new) and resets the view.
    fn load(&mut self, doc: Document) {
        self.playback.stop();
        let first = doc.clips.keys().next().copied();
        self.arrange = ArrangeState {
            selected_clip: first,
            selected_track: first.and_then(|c| doc.clips.get(&c)).map(|c| c.track),
            ..Default::default()
        };
        self.mixer_scroll = 0.0;
        self.levels.clear();
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
        if self.project.autosave(self.history.doc(), v, true) {
            eprintln!("unsaved changes kept in {}", self.project.autosave_dir().join("backups").display());
        }
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
                self.load(doc);
            }
            Err(e) => eprintln!("cannot open {}: {e}", path.display()),
        }
    }

    fn new_project(&mut self) {
        self.keep_unsaved();
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
        self.project = Project::new(None, "untitled".into(), 0);
        self.load(Document::new(seed as u64));
    }

    fn save(&mut self, choose: bool) {
        let v = self.history.version();
        let res = if self.project.path.is_some() && !choose {
            self.project.save(self.history.doc(), v)
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
            self.project.save_as(&path, self.history.doc(), v)
        };
        match res {
            Ok(p) => eprintln!("saved {}", p.display()),
            Err(e) => eprintln!("save failed: {e}"),
        }
        self.update_title();
    }

    fn redraw(&mut self) {
        let Some(g) = &mut self.gfx else { return };
        let size = g.window.inner_size();
        if size.width == 0 || size.height == 0 {
            return;
        }
        let dt = self.last_frame.elapsed().as_secs_f32().min(0.25);
        self.playback.update_levels(self.history.doc(), &mut self.levels, dt);
        let scale = g.window.scale_factor();
        let (w, h) = (size.width as f64 / scale, size.height as f64 / scale);
        let frame = Frame {
            doc: self.history.doc(),
            name: &self.project.name,
            arrange: &self.arrange,
            transport: TransportView {
                playhead: self.playback.playhead(),
                playing: self.playback.playing,
                recording: false,
            },
            panel_h: PANEL_H,
            panel: self.panel,
            mixer_scroll: self.mixer_scroll,
            levels: &self.levels,
            edited: self.project.edited(self.history.version()),
        };
        let mut scene = Scene::new();
        let drawn = shell::draw(&mut scene, &mut self.fonts, &self.theme, &frame, w, h);
        self.shell = Some(drawn.shell);
        self.layout = Some(drawn.arrange);
        if let Some(m) = &drawn.mixer {
            self.mixer_scroll = self.mixer_scroll.min(m.max_scroll);
        }
        self.mixer = drawn.mixer;
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
            self.edit("reset", vec![Op::SetValue { target: target.clone(), value: default_value(&target) }]);
            return;
        }
        self.history.begin_gesture(match target {
            ParamTarget::Gain(_) => "fader",
            ParamTarget::Pan(_) => "pan",
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
        let value = value_at(&d.target, pos);
        if (value - value_at(&d.target, position_of(self.history.doc(), &d.target))).abs() > f32::EPSILON {
            self.edit("drag", vec![Op::SetValue { target: d.target, value }]);
        }
    }

    fn end_drag(&mut self) {
        if self.drag.take().is_some() {
            self.history.end_gesture();
        }
    }

    fn press(&mut self) {
        let now = Instant::now();
        let pt = self.cursor;
        let double =
            self.last_click.is_some_and(|(t, p)| now.duration_since(t) < DOUBLE_CLICK && (p - pt).hypot() < 4.0);
        self.last_click = Some((now, pt));
        if let Some(sl) = self.shell
            && sl.panel.contains(pt)
        {
            return self.press_panel(sl, pt, double);
        }
        let Some(layout) = &self.layout else { return };
        match layout.hit(pt) {
            Hit::Clip { clip, .. } => {
                self.arrange.selected_clip = Some(clip);
                self.arrange.selected_track = self.history.doc().clips.get(&clip).map(|c| c.track);
            }
            Hit::Mute(t) => self.toggle_flag(t, TrackFlag::Mute),
            Hit::Solo(t) => self.toggle_flag(t, TrackFlag::Solo),
            Hit::Arm(t) => self.toggle_flag(t, TrackFlag::Arm),
            Hit::Fader(t) => {
                let row = layout.rows.iter().find(|r| r.track == t).copied();
                if let Some(f) = row.and_then(|r| header_controls(&r, layout.headers).fader) {
                    self.begin_drag(ParamTarget::Gain(t), Axis::Horizontal { travel: f.x1 - f.x0 }, double);
                }
            }
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
            Hit::Lane(..) | Hit::Nothing => {}
        }
    }

    fn press_panel(&mut self, sl: ShellLayout, pt: Point, double: bool) {
        match panel::hit_tab(sl.panel, pt) {
            Some(Tab::Clip) => return self.panel = PanelView::Clip,
            Some(Tab::Mixer) => return self.panel = PanelView::Mixer,
            Some(Tab::Macros) => return,
            None => {}
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
            MixHit::Strip(t) => self.arrange.selected_track = Some(t),
            MixHit::Nothing => {}
        }
    }

    fn key(&mut self, key: &Key) {
        let ctrl = self.mods.control_key() || self.mods.super_key();
        let shift = self.mods.shift_key();
        match key {
            Key::Named(NamedKey::Space) => self.playback.toggle(),
            Key::Named(NamedKey::Home) => self.playback.locate(Ticks::ZERO),
            Key::Named(NamedKey::Tab) => {
                self.panel = if self.panel == PanelView::Mixer { PanelView::Clip } else { PanelView::Mixer };
            }
            Key::Character(c) if ctrl && c.eq_ignore_ascii_case("z") => {
                self.end_drag();
                let did = if shift { self.history.redo() } else { self.history.undo() };
                if did.is_some() {
                    self.playback.recompile(self.history.doc());
                }
            }
            Key::Character(c) if ctrl && c.eq_ignore_ascii_case("s") => self.save(shift),
            Key::Character(c) if ctrl && c.eq_ignore_ascii_case("o") => self.open(),
            Key::Character(c) if ctrl && c.eq_ignore_ascii_case("n") => self.new_project(),
            Key::Character(c) if ctrl && c.eq_ignore_ascii_case("q") => self.quit = true,
            Key::Character(c) if !ctrl && c == "l" => {
                let mut r = self.history.doc().loop_range;
                r.enabled = !r.enabled;
                self.edit("loop", vec![Op::SetLoop { range: r }]);
            }
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
        if let Some(sl) = self.shell
            && sl.panel.contains(self.cursor)
        {
            if self.panel == PanelView::Mixer {
                let d = if dx.abs() > dy.abs() { dx } else { dy };
                let max = self.mixer.as_ref().map(|m| m.max_scroll).unwrap_or(0.0);
                self.mixer_scroll = (self.mixer_scroll - d).clamp(0.0, max);
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
        self.levels.values().any(|(l, r)| *l > 0.0 || *r > 0.0)
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
                dirty = self.drag.is_some();
                self.drag_to();
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
            WindowEvent::DroppedFile(path) => self.open_path(&path),
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
        if self.drag.is_none() && self.project.autosave(self.history.doc(), v, false) {
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

fn main() {
    let arg = std::env::args().nth(1).map(PathBuf::from);
    let (doc, project, selected) = match &arg {
        Some(p) => {
            let mut project = Project::new(None, project::name_of(p), 0);
            let doc = project.open(p, 0).unwrap_or_else(|e| {
                eprintln!("cannot open {}: {e}", p.display());
                std::process::exit(1);
            });
            let first = doc.clips.keys().next().copied();
            (doc, project, first)
        }
        None => {
            let d = sat_ui::demo::demo();
            (d.doc, Project::new(None, "demo set".into(), 0), Some(d.selected))
        }
    };
    let playback = Playback::start(&doc);
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
        last_click: None,
        last_frame: Instant::now(),
        quit: false,
    };
    let el = EventLoop::new().expect("event loop");
    el.run_app(&mut app).expect("run");
}
