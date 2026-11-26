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
//! `SAT_DOCK=outliner` puts the outliner in the left sidebar; `SAT_FOLDS=<mask>`
//! folds inspector sections; `SAT_UI_SCALE` sets the interface scale (0.9).
//! `SAT_LAUNCHER=1` shows the clip launcher beside the arrangement;
//! `SAT_TRACK_H` and `SAT_PX_PER_BAR` set the arrange zoom.
//! `SAT_KEY=<root>,<scale>` sets the project key (indexes into the scale
//! lists); `SAT_ROLL=<words>` sets the piano roll: `fold`, `inkey`, `menu`
//! (key chooser open), `zoom=<px per beat>`, `x0=<ticks>`, `row=<px>`.

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
    // SAT_SETTINGS=interface|audio|editing|keys draws the settings window open
    let settings = std::env::var("SAT_SETTINGS").ok().map(|s| sat_ui::settings::SettingsView {
        section: sat_ui::settings::Section::ALL.into_iter().find(|x| x.label() == s).unwrap_or_default(),
        ui_scale: 0.9,
        snap: Some(sat_doc::sat_time::Ticks(sat_doc::sat_time::PPQ * 4)),
        note_len: sat_doc::sat_time::Ticks(sat_doc::sat_time::PPQ / 4),
        device: "built-in output".into(),
        sample_rate: 48000.0,
        latency_ms: 10.7,
    });
    let dock = Dock {
        left_open: !has("noleft"),
        left_shows: if has("outliner") {
            sat_ui::area::EditorType::Outliner
        } else if has("browser") {
            sat_ui::area::EditorType::Browser
        } else {
            sat_ui::area::EditorType::Bank
        },
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
    if let Ok(k) = std::env::var("SAT_KEY") {
        let mut it = k.split(',').filter_map(|v| v.trim().parse::<u8>().ok());
        demo.doc.key = sat_doc::Key { root: it.next().unwrap_or(0), scale: it.next().unwrap_or(1) };
    }
    let roll_words = std::env::var("SAT_ROLL").unwrap_or_default();
    let roll_val = |k: &str| {
        roll_words
            .split(',')
            .find_map(|w| w.trim().strip_prefix(k).and_then(|v| v.strip_prefix('=')).map(str::to_owned))
    };
    let roll_has = |w: &str| roll_words.split(',').any(|x| x.trim() == w);
    let roll = RollView {
        fold: roll_has("fold"),
        in_key: roll_has("inkey"),
        key_menu: roll_has("menu"),
        zoom: roll_val("zoom").and_then(|v| v.parse().ok()),
        x0: Ticks(roll_val("x0").and_then(|v| v.parse().ok()).unwrap_or(0)),
        row_h: roll_val("row").and_then(|v| v.parse().ok()).unwrap_or(sat_ui::piano_roll::ROW_H),
        ..Default::default()
    };
    let theme =
        if std::env::var_os("SAT_LIGHT").is_some() { sat_theme::Theme::light() } else { sat_theme::Theme::dark() };
    let mut fonts = Fonts::new();
    let mut scene = vello::Scene::new();
    // SAT_LAUNCHER=1 shows the clip launcher beside the arrangement (with
    // the verse playing); SAT_TRACK_H sets the track height
    let launcher = std::env::var_os("SAT_LAUNCHER").is_some();
    let state = ArrangeState {
        selected_clip: Some(demo.selected),
        selected_track: Some(demo.doc.clips[&demo.selected].track),
        launcher,
        track_h: std::env::var("SAT_TRACK_H").ok().and_then(|v| v.parse().ok()).unwrap_or(sat_ui::arrange::TRACK_H),
        px_per_bar: std::env::var("SAT_PX_PER_BAR").ok().and_then(|v| v.parse().ok()).unwrap_or(76.0),
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
    if view == MainView::Session || launcher {
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
    // SAT_UI_SCALE: the app's interface scale (0.9 by default), so a
    // screenshot shows as much as the window does
    let scale_ui: f64 = std::env::var("SAT_UI_SCALE").ok().and_then(|v| v.parse().ok()).unwrap_or(0.9);
    let (w, h) = (1440.0 / scale_ui, 900.0 / scale_ui);
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
        outline_scroll: 0.0,
        macro_folds: std::env::var("SAT_FOLDS").ok().and_then(|v| v.parse().ok()).unwrap_or(0),
        pointer: vello::kurbo::Point::new(700.0, 300.0),
        status: "built-in output @ 48000 Hz",
        ui_scale: scale_ui,
        panel,
        mixer_scroll: 0.0,
        levels: &levels,
        edited: false,
        roll: &roll,
        steps: &StepView::default(),
        chords: &sat_ui::chords::ChordView {
            selected: std::env::var("SAT_CHORD_SEL").ok().and_then(|v| v.parse().ok()),
            held: std::env::var_os("SAT_CHORD_HELD").map(|_| sat_modules::chords::Step::Degree(5, 3)),
            ..Default::default()
        },
        view,
        grid: &sat_ui::grid::GridView { menu: std::env::var_os("SAT_GRID_MENU").is_some(), ..Default::default() },
        learn: false,
        sounds: &demo.sounds,
        plugins: &Default::default(),
        session: &session,
        palette: palette.as_ref(),
        bank: &sat_ui::bank::BankView::default(),
        browser: &sat_ui::browser::BrowserView {
            query: std::env::var("SAT_BROWSER_QUERY").unwrap_or_default(),
            typing: std::env::var_os("SAT_BROWSER_QUERY").is_some(),
            selected: Some(sat_ui::browser::SYNTH_TRACK),
            ..Default::default()
        },
        settings: settings.as_ref(),
        drag_clip: None,
    };
    let t = std::time::Instant::now();
    shell::draw(&mut scene, &mut fonts, &theme, &frame, w, h);
    let build = t.elapsed();
    let mut gpu = Gpu::headless()?;
    let t = std::time::Instant::now();
    let px = scale * scale_ui;
    let img =
        gpu.render_rgba(&scaled(&scene, px), (w * px).round() as u32, (h * px).round() as u32, color(theme.c.bg_base))?;
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
