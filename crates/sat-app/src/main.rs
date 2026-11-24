//! The sat desktop app.
//!
//! Opens a window with the arrange view and bottom panel, plays the project
//! through the default audio device, and handles the first interactions:
//!
//! - space: play / stop; home: back to the start; l: loop on / off
//! - click a clip to edit it in the bottom panel; click the ruler to move the playhead
//! - m / s: mute / solo the selected track; ctrl+z / ctrl+shift+z: undo / redo
//! - wheel: scroll; shift+wheel: scroll time; ctrl+wheel: zoom
//!
//! `sat [project.sat]` opens a project bundle; without one it opens a demo.

mod audio;

use std::sync::Arc;
use std::time::{Duration, Instant};

use sat_compile::Compiled;
use sat_doc::sat_time::Ticks;
use sat_doc::{Document, History, Op, TrackFlag, file};
use sat_engine::{Command, EngineHandle};
use sat_ui::arrange::{ArrangeLayout, ArrangeState, Hit, TransportView};
use sat_ui::paint::{Fonts, color};
use sat_ui::shell::{self, Frame, PANEL_H};
use vello::util::{RenderContext, RenderSurface};
use vello::{AaConfig, RenderParams, Renderer, RendererOptions, Scene, wgpu};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Window, WindowId};

const MAX_BLOCK: usize = 1024;
const FALLBACK_RATE: f64 = 48_000.0;

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

    fn locate(&mut self, t: Ticks) {
        self.clock = Some((Instant::now(), t));
        let s = self.compiled.sample(t);
        self.send(Command::Locate(s));
    }
}

struct Gfx {
    window: Arc<Window>,
    surface: RenderSurface<'static>,
    renderer: Renderer,
}

struct App {
    name: String,
    history: History,
    arrange: ArrangeState,
    playback: Playback,
    ctx: RenderContext,
    gfx: Option<Gfx>,
    fonts: Fonts,
    theme: sat_theme::Theme,
    scene: Scene,
    cursor: (f64, f64),
    mods: ModifiersState,
    layout: Option<ArrangeLayout>,
    last_frame: Instant,
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

    fn redraw(&mut self) {
        let Some(g) = &mut self.gfx else { return };
        let size = g.window.inner_size();
        if size.width == 0 || size.height == 0 {
            return;
        }
        let scale = g.window.scale_factor();
        let (w, h) = (size.width as f64 / scale, size.height as f64 / scale);
        let frame = Frame {
            doc: self.history.doc(),
            name: &self.name,
            arrange: &self.arrange,
            transport: TransportView {
                playhead: self.playback.playhead(),
                playing: self.playback.playing,
                recording: false,
            },
            panel_h: PANEL_H,
        };
        let mut scene = Scene::new();
        let (_, layout) = shell::draw(&mut scene, &mut self.fonts, &self.theme, &frame, w, h);
        self.layout = Some(layout);
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

    fn click(&mut self) {
        let Some(layout) = &self.layout else { return };
        let pt = vello::kurbo::Point::new(self.cursor.0, self.cursor.1);
        match layout.hit(pt) {
            Hit::Clip { clip, .. } => {
                self.arrange.selected_clip = Some(clip);
                self.arrange.selected_track = self.history.doc().clips.get(&clip).map(|c| c.track);
            }
            Hit::Header(t) => {
                self.arrange.selected_track = Some(t);
                let row = layout.rows.iter().find(|r| r.track == t);
                let is_group = self.history.doc().tracks[&t].kind == sat_doc::TrackKind::Group;
                // clicking a group's chevron area folds it
                if is_group
                    && row.is_some_and(|r| self.cursor.0 < layout.headers.x0 + 40.0 && self.cursor.1 < r.y + 28.0)
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

    fn key(&mut self, key: &Key) {
        let ctrl = self.mods.control_key() || self.mods.super_key();
        match key {
            Key::Named(NamedKey::Space) => self.playback.toggle(),
            Key::Named(NamedKey::Home) => self.playback.locate(Ticks::ZERO),
            Key::Character(c) if ctrl && c.eq_ignore_ascii_case("z") => {
                let did = if self.mods.shift_key() { self.history.redo() } else { self.history.undo() };
                if did.is_some() {
                    self.playback.recompile(self.history.doc());
                }
            }
            Key::Character(c) if c == "l" => {
                let mut r = self.history.doc().loop_range;
                r.enabled = !r.enabled;
                self.edit("loop", vec![Op::SetLoop { range: r }]);
            }
            Key::Character(c) if c == "m" || c == "s" => {
                if let Some(t) = self.arrange.selected_track {
                    let tr = &self.history.doc().tracks[&t];
                    let (flag, on) =
                        if c == "m" { (TrackFlag::Mute, !tr.mixer.mute) } else { (TrackFlag::Solo, !tr.mixer.solo) };
                    if tr.kind != sat_doc::TrackKind::Master || flag == TrackFlag::Mute {
                        self.edit(if c == "m" { "mute" } else { "solo" }, vec![Op::SetTrackFlag { id: t, flag, on }]);
                    }
                }
            }
            _ => {}
        }
    }

    fn scroll(&mut self, dx: f64, dy: f64) {
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
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.gfx.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title(format!("sat · {}", self.name))
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
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::RedrawRequested => {
                self.redraw();
                dirty = false;
            }
            WindowEvent::Resized(_) => {}
            WindowEvent::ModifiersChanged(m) => self.mods = m.state(),
            WindowEvent::CursorMoved { position, .. } => {
                let scale = self.gfx.as_ref().map(|g| g.window.scale_factor()).unwrap_or(1.0);
                self.cursor = (position.x / scale, position.y / scale);
                dirty = false;
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => self.click(),
            WindowEvent::MouseWheel { delta, .. } => {
                let (dx, dy) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (x as f64 * 40.0, y as f64 * 40.0),
                    MouseScrollDelta::PixelDelta(p) => (p.x, p.y),
                };
                self.scroll(dx, dy);
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
        // while playing, redraw at display rate for the playhead; otherwise sleep until input
        if self.playback.playing {
            if let Some(g) = &self.gfx {
                g.window.request_redraw();
            }
            el.set_control_flow(ControlFlow::WaitUntil(self.last_frame + Duration::from_millis(8)));
        } else {
            el.set_control_flow(ControlFlow::Wait);
        }
    }
}

fn main() {
    let arg = std::env::args().nth(1);
    let (doc, name, selected) = match &arg {
        Some(path) => {
            let p = std::path::Path::new(path);
            let doc = file::load_bundle(p).unwrap_or_else(|e| {
                eprintln!("cannot open {path}: {e}");
                std::process::exit(1);
            });
            let name = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "project".into());
            let first = doc.clips.keys().next().copied();
            (doc, name, first)
        }
        None => {
            let d = sat_ui::demo::demo();
            (d.doc, "demo set".to_string(), Some(d.selected))
        }
    };
    let playback = Playback::start(&doc);
    let arrange = ArrangeState {
        selected_clip: selected,
        selected_track: selected.and_then(|c| doc.clips.get(&c)).map(|c| c.track),
        ..Default::default()
    };
    let mut app = App {
        name,
        history: History::new(doc, 500),
        arrange,
        playback,
        ctx: RenderContext::new(),
        gfx: None,
        fonts: Fonts::new(),
        theme: sat_theme::Theme::dark(),
        scene: Scene::new(),
        cursor: (0.0, 0.0),
        mods: ModifiersState::empty(),
        layout: None,
        last_frame: Instant::now(),
    };
    let el = EventLoop::new().expect("event loop");
    el.run_app(&mut app).expect("run");
}
