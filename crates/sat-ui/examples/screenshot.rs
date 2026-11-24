//! Renders the demo project to a PNG without a window.
//!
//! `cargo run -p sat-ui --example screenshot -- out.png [scale] [clip|mixer]`

use sat_doc::sat_time::Ticks;
use sat_ui::arrange::{ArrangeState, TransportView};
use sat_ui::gfx::{Gpu, scaled};
use sat_ui::paint::{Fonts, color};
use sat_ui::panel::PanelView;
use sat_ui::shell::{self, Frame, PANEL_H};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let out = args.next().unwrap_or_else(|| "arrange.png".into());
    let scale: f64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(1.0);
    let panel = if args.next().as_deref() == Some("mixer") { PanelView::Mixer } else { PanelView::Clip };
    let demo = sat_ui::demo::demo();
    let theme = sat_theme::Theme::dark();
    let mut fonts = Fonts::new();
    let mut scene = vello::Scene::new();
    let state = ArrangeState {
        selected_clip: Some(demo.selected),
        selected_track: Some(demo.doc.clips[&demo.selected].track),
        ..Default::default()
    };
    let (w, h) = (1440.0, 900.0);
    let levels = sat_ui::demo::levels(&demo.doc);
    let frame = Frame {
        doc: &demo.doc,
        name: "untitled set",
        arrange: &state,
        transport: TransportView { playhead: Ticks(6 * 3840 + 960 + 240), playing: true, recording: false },
        panel_h: PANEL_H,
        panel,
        mixer_scroll: 0.0,
        levels: &levels,
        edited: false,
    };
    let t = std::time::Instant::now();
    shell::draw(&mut scene, &mut fonts, &theme, &frame, w, h);
    let build = t.elapsed();
    let mut gpu = Gpu::headless()?;
    let t = std::time::Instant::now();
    let img =
        gpu.render_rgba(&scaled(&scene, scale), (w * scale) as u32, (h * scale) as u32, color(theme.c.bg_base))?;
    println!(
        "{}: scene {:.1} ms, render {:.1} ms",
        gpu.adapter_name,
        build.as_secs_f64() * 1e3,
        t.elapsed().as_secs_f64() * 1e3
    );
    img.save_png(std::path::Path::new(&out))?;
    println!("wrote {out}");
    Ok(())
}
