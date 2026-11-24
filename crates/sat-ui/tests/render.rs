//! Renders the shell offscreen and checks pixels.
//!
//! Needs a GPU adapter; a software one (Mesa lavapipe) is enough. Without one
//! the test is skipped, unless `SAT_REQUIRE_GPU` is set (CI sets it).

use sat_doc::sat_time::Ticks;
use sat_ui::arrange::{ArrangeState, TransportView};
use sat_ui::demo::{bars, demo};
use sat_ui::gfx::Gpu;
use sat_ui::paint::{Fonts, color};
use sat_ui::shell::{self, Frame, PANEL_H};

fn gpu() -> Option<Gpu> {
    match Gpu::headless() {
        Ok(g) => Some(g),
        Err(e) if std::env::var_os("SAT_REQUIRE_GPU").is_none_or(|v| v.is_empty()) => {
            eprintln!("skipping: {e}");
            None
        }
        Err(e) => panic!("{e}"),
    }
}

#[test]
fn renders_the_arrange_view() {
    let Some(mut gpu) = gpu() else { return };
    let d = demo();
    let theme = sat_theme::Theme::dark();
    let state = ArrangeState { selected_clip: Some(d.selected), ..Default::default() };
    let playhead = bars(6) + Ticks::QUARTER;
    let frame = Frame {
        doc: &d.doc,
        name: "test",
        arrange: &state,
        transport: TransportView { playhead, playing: true, recording: false },
        panel_h: PANEL_H,
    };
    let (w, h) = (1280u32, 800u32);
    let mut scene = vello::Scene::new();
    let mut fonts = Fonts::new();
    let (_, al) = shell::draw(&mut scene, &mut fonts, &theme, &frame, w as f64, h as f64);
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
