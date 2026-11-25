//! Renders the shell offscreen and checks pixels.
//!
//! Needs a hardware GPU adapter, or any adapter when `SAT_REQUIRE_GPU` is set
//! (ubuntu CI sets it and uses Mesa lavapipe). Otherwise the test is skipped.

use sat_doc::sat_time::Ticks;
use sat_ui::arrange::{ArrangeState, TransportView};
use sat_ui::demo::{bars, demo};
use sat_ui::gfx::Gpu;
use sat_ui::mixer::Levels;
use sat_ui::paint::{Fonts, color};
use sat_ui::panel::PanelView;
use sat_ui::piano_roll::RollView;
use sat_ui::shell::{self, Frame, PANEL_H};
use vello::wgpu;

fn gpu() -> Option<Gpu> {
    let required = std::env::var_os("SAT_REQUIRE_GPU").is_some_and(|v| !v.is_empty());
    let skip = |why: String| {
        assert!(!required, "{why}");
        eprintln!("skipping: {why}");
        None
    };
    let adapter = match Gpu::headless_adapter() {
        Ok(a) => a,
        Err(e) => return skip(e.to_string()),
    };
    // Unknown software rasterisers (e.g. on GPU-less Windows CI runners) can
    // crash the process inside the driver, which a test cannot catch. They
    // run only where required; ubuntu CI requires Mesa lavapipe.
    let info = adapter.get_info();
    if info.device_type == wgpu::DeviceType::Cpu && !required {
        return skip(format!("software adapter {} ({:?})", info.name, info.backend));
    }
    match Gpu::from_adapter(&adapter) {
        Ok(g) => Some(g),
        Err(e) => skip(e.to_string()),
    }
}

#[test]
fn renders_the_arrange_view() {
    let Some(mut gpu) = gpu() else { return };
    let d = demo();
    let theme = sat_theme::Theme::dark();
    let state = ArrangeState { selected_clip: Some(d.selected), ..Default::default() };
    let playhead = bars(6) + Ticks::QUARTER;
    let levels = Levels::new();
    let frame = Frame {
        doc: &d.doc,
        name: "test",
        arrange: &state,
        transport: TransportView { playhead, playing: true, recording: false },
        panel_h: PANEL_H,
        panel: PanelView::Clip,
        mixer_scroll: 0.0,
        levels: &levels,
        edited: false,
        roll: &RollView::default(),
        steps: &sat_ui::steps::StepView::default(),
        view: sat_ui::shell::MainView::Arrange,
        grid: &sat_ui::grid::GridView::default(),
        learn: false,
        sounds: &d.sounds,
        session: &sat_ui::session::SessionView::default(),
        palette: None,
    };
    let (w, h) = (1280u32, 800u32);
    let mut scene = vello::Scene::new();
    let mut fonts = Fonts::new();
    let al = shell::draw(&mut scene, &mut fonts, &theme, &frame, w as f64, h as f64).arrange;
    let img = gpu.render_rgba(&scene, w, h, color(theme.c.bg_base)).unwrap();
    if let Some(dir) = std::env::var_os("SAT_SCREENSHOT_DIR") {
        img.save_png(&std::path::Path::new(&dir).join("arrange.png")).unwrap();
    }
    let near = |a: [u8; 4], b: sat_theme::Rgba| {
        (a[0] as i32 - b.r as i32).abs() < 12
            && (a[1] as i32 - b.g as i32).abs() < 12
            && (a[2] as i32 - b.b as i32).abs() < 12
    };
    // the playhead is a bright vertical line through the timeline
    let px = al.x_of(playhead).floor() as u32;
    let mid = al.timeline.center().y as u32;
    assert!(near(img.pixel(px, mid), theme.c.text_max), "playhead pixel {:?}", img.pixel(px, mid));
    // the selected clip's header is filled with its track colour
    let cb = al.clips.iter().find(|c| c.clip == d.selected).unwrap();
    let col = theme.track(d.doc.clips[&d.selected].color.0);
    let probe = (cb.rect.x0 as u32 + 3, cb.rect.y0 as u32 + 12);
    assert!(near(img.pixel(probe.0, probe.1), col), "clip header {:?} vs {col:?}", img.pixel(probe.0, probe.1));
    // and it has the accent selection ring just outside its edge
    let ring = img.pixel(cb.rect.center().x as u32, cb.rect.y0 as u32);
    assert!(near(ring, theme.c.accent_primary), "ring {ring:?}");
    // the window background is the base colour
    assert!(near(img.pixel(1, h - 1), theme.c.bg_base));
}

#[test]
fn renders_the_mixer() {
    let Some(mut gpu) = gpu() else { return };
    let mut d = demo();
    let drums = d.doc.tracks.values().find(|t| t.name == "drums").unwrap().id;
    sat_doc::apply(&mut d.doc, sat_doc::Op::SetTrackFlag { id: drums, flag: sat_doc::TrackFlag::Mute, on: true })
        .unwrap();
    let theme = sat_theme::Theme::dark();
    let state = ArrangeState::default();
    let levels = sat_ui::demo::levels(&d.doc);
    let frame = Frame {
        doc: &d.doc,
        name: "test",
        arrange: &state,
        transport: TransportView::default(),
        panel_h: PANEL_H,
        panel: PanelView::Mixer,
        mixer_scroll: 0.0,
        levels: &levels,
        edited: true,
        roll: &RollView::default(),
        steps: &sat_ui::steps::StepView::default(),
        view: sat_ui::shell::MainView::Arrange,
        grid: &sat_ui::grid::GridView::default(),
        learn: false,
        sounds: &d.sounds,
        session: &sat_ui::session::SessionView::default(),
        palette: None,
    };
    let (w, h) = (1280u32, 800u32);
    let mut scene = vello::Scene::new();
    let mut fonts = Fonts::new();
    let drawn = shell::draw(&mut scene, &mut fonts, &theme, &frame, w as f64, h as f64);
    let img = gpu.render_rgba(&scene, w, h, color(theme.c.bg_base)).unwrap();
    if let Some(dir) = std::env::var_os("SAT_SCREENSHOT_DIR") {
        img.save_png(&std::path::Path::new(&dir).join("mixer.png")).unwrap();
    }
    let near = |a: [u8; 4], b: sat_theme::Rgba| {
        (a[0] as i32 - b.r as i32).abs() < 16
            && (a[1] as i32 - b.g as i32).abs() < 16
            && (a[2] as i32 - b.b as i32).abs() < 16
    };
    let ml = drawn.mixer.expect("the panel shows the mixer");
    // master's fader cap sits at its level
    let master = ml.strip(d.doc.master).unwrap();
    let cap = master.cap(sat_ui::controls::fader_pos(-1.5));
    let at = |x: f64, y: f64| img.pixel(x as u32, y as u32);
    assert!(
        near(at(cap.x0 + 2.0, cap.y0 + 2.0), sat_theme::Rgba::hex(0xc8c8c8)),
        "cap {:?}",
        at(cap.x0 + 2.0, cap.y0 + 2.0)
    );
    // its meter is lit green at the bottom
    let m = master.meter;
    let px = at(m.x0 + 1.0, m.y1 - 2.0);
    assert!(px[1] > 120 && px[1] > px[0] && px[1] > px[2], "meter {px:?}");
    // the muted strip's mute button is solid amber
    let mute = ml.strip(drums).unwrap().mute.unwrap();
    assert!(
        near(at(mute.x0 + 3.0, mute.y0 + 3.0), sat_theme::Rgba::hex(0xf0b14a)),
        "mute {:?}",
        at(mute.x0 + 3.0, mute.y0 + 3.0)
    );
}
