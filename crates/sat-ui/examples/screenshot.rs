//! Renders the demo project to a PNG without a window.
//!
//! `cargo run -p sat-ui --example screenshot -- out.png [scale] [clip|mixer|macros|<clip name>] [grid|instrument|fx|session]`
//!
//! A clip name selects that clip and shows it in the bottom panel. A fourth
//! argument `grid` shows the selected clip's source grid instead of the arrangement.
//! `SAT_PALETTE=<query>` draws the command palette open with that query.
//! `SAT_GRID_MENU=1` draws a grid view with its add-module menu open.
//! `SAT_DOCK=<words>` sets the docked areas: `noleft`, `right` (the inspector;
//! `macros` also opens it), `collapsed`, `max` (panel maximized), `focus`
//! (main view zoomed), `tall` (a 480 px panel). The panel is drawn at the
//! height of the editor it shows (`tall` sets the piano roll's).
//! `SAT_MENU=main|panel|left` draws that area's editor-type menu open.

use sat_doc::sat_time::Ticks;
use sat_ui::arrange::{ArrangeState, TransportView};
use sat_ui::dock::{Dock, Zoom};
use sat_ui::gfx::{Gpu, scaled};
use sat_ui::paint::{Fonts, color};
use sat_ui::panel::PanelView;
use sat_ui::piano_roll::RollView;
use sat_ui::shell::{self, Frame, MainView};
use sat_ui::steps::StepView;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let out = args.next().unwrap_or_else(|| "arrange.png".into());
    let scale: f64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(1.0);
    let view = args.next().unwrap_or_default();
    let panel = match view.as_str() {
        "mixer" => PanelView::Mixer,
        _ => PanelView::Clip,
    };
    let words = std::env::var("SAT_DOCK").unwrap_or_default();
    let has = |w: &str| words.split(',').any(|x| x.trim() == w);
    let dock = Dock {
        left_open: !has("noleft"),
        right_open: has("right") || view == "macros",
        panel_open: !has("collapsed"),
        roll_h: if has("tall") { 480.0 } else { sat_ui::dock::PANEL_H },
        zoom: if has("max") {
            Some(Zoom::Panel)
        } else if has("focus") {
            Some(Zoom::Main)
        } else {
            None
        },
        ..Default::default()
    };
    let mut demo = sat_ui::demo::demo();
    if let Some(c) = demo.doc.clips.values().find(|c| c.name == view) {
        demo.selected = c.id;
    }
    let theme = sat_theme::Theme::dark();
    let mut fonts = Fonts::new();
    let mut scene = vello::Scene::new();
    let state = ArrangeState {
        selected_clip: Some(demo.selected),
        selected_track: Some(demo.doc.clips[&demo.selected].track),
        ..Default::default()
    };
    let view = match args.next().as_deref() {
        Some("grid") => MainView::Grid(demo.doc.tracks[&demo.doc.clips[&demo.selected].track].source_grid.unwrap()),
        Some("fx") => MainView::Grid(demo.doc.tracks[&demo.doc.clips[&demo.selected].track].fx_grids[0]),
        Some("session") => MainView::Session,
        Some("instrument") => {
            MainView::Grid(demo.doc.tracks[&demo.doc.clips[&demo.selected].track].instrument_grid.unwrap())
        }
        _ => MainView::Arrange,
    };
    // in the session view, the verse playing and the break queued on the drums
    let mut session = sat_ui::session::SessionView::default();
    if view == MainView::Session {
        let verse = demo.doc.scenes[1].id;
        for ((scene, track), clip) in demo.doc.slots.iter() {
            if *scene == verse {
                session.playing.insert(*track, *clip);
                session.started.insert(*track, sat_ui::demo::bars(4));
                session.following.insert(*track);
            }
        }
        let brk = demo.doc.scenes[2].id;
        if let Some(((_, t), c)) = demo.doc.slots.iter().find(|((s, _), _)| *s == brk) {
            session.queued.insert(*t, (Some(*c), sat_ui::demo::bars(8)));
        }
    }
    // SAT_PALETTE=<query> draws the command palette open with that query
    let palette = std::env::var("SAT_PALETTE").ok().map(|query| sat_ui::palette::PaletteView { query, index: 0 });
    let (w, h) = (1440.0, 900.0);
    let levels = sat_ui::demo::levels(&demo.doc);
    let mut dock = dock;
    dock.panel_h = dock.target(shell::panel_kind(&demo.doc, panel, state.selected_clip));
    let frame = Frame {
        doc: &demo.doc,
        name: "untitled set",
        arrange: &state,
        transport: TransportView { playhead: Ticks(6 * 3840 + 960 + 240), playing: true, recording: false },
        dock,
        macro_scroll: 0.0,
        focus: sat_ui::area::Slot::Panel,
        menu: match std::env::var("SAT_MENU").as_deref() {
            Ok("main") => Some(sat_ui::area::Slot::Main),
            Ok("panel") => Some(sat_ui::area::Slot::Panel),
            Ok("left") => Some(sat_ui::area::Slot::Left),
            _ => None,
        },
        panel,
        mixer_scroll: 0.0,
        levels: &levels,
        edited: false,
        roll: &RollView::default(),
        steps: &StepView::default(),
        view,
        grid: &sat_ui::grid::GridView { menu: std::env::var_os("SAT_GRID_MENU").is_some(), ..Default::default() },
        learn: false,
        sounds: &demo.sounds,
        plugins: &Default::default(),
        session: &session,
        palette: palette.as_ref(),
        bank: &sat_ui::bank::BankView::default(),
        drag_clip: None,
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
