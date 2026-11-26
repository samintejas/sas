//! Phase 7 gate: a plugin crash leaves the set playing.
//!
//! The demo's keys track gets the crash-test plugin in its fx grid. At 0 dB
//! it is heard exactly as the dry track (same block, no added latency);
//! when it crashes mid-song its track goes silent, every other track plays
//! on, and no block waits past its budget.

use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

use sat_compile::{NoPlugins, Plugins};
use sat_doc::*;
use sat_engine::Command;
use sat_plug::{PlugHost, Status};

const RATE: f64 = 48_000.0;
const BLOCK: usize = 256;

/// The tests run one at a time: each spins an engine and a host, and on a
/// small machine running them together starves the hosts of CPU.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

struct Hosts(HashMap<ModuleId, PlugHost>);

impl Plugins for Hosts {
    fn processor(&self, module: ModuleId) -> Option<Box<dyn sat_engine::External>> {
        self.0.get(&module).map(|h| Box::new(h.processor()) as Box<dyn sat_engine::External>)
    }
}

/// The demo with the crash-test plugin (at 0 dB) on the keys track.
fn demo_with_plugin() -> (sat_ui::demo::Demo, ModuleId) {
    let mut d = sat_ui::demo::demo();
    let keys = d.doc.tracks.values().find(|t| t.name == "keys").unwrap();
    let grid = *keys.fx_grids.front().unwrap();
    let (m, mut ops) = sat_modules::fx::insert(&d.doc, grid, sat_modules::fx::PLUGIN);
    ops.push(Op::SetModulePlugin { module: m, plugin: Some("builtin:crash-test".into()) });
    // gain 0 dB (of -60..12), mode run, no leak
    for (k, v) in [("p1", 60.0 / 72.0), ("p2", 0.0), ("p3", 0.0)] {
        ops.push(Op::SetValue { target: ParamTarget::Module { module: m, param: k.into() }, value: v });
    }
    for op in ops {
        apply(&mut d.doc, op).unwrap();
    }
    (d, m)
}

/// Renders `blocks` blocks from bar 9 (the pads play there), returning the
/// output and the slowest block.
fn play(
    doc: &Document,
    sounds: &sat_compile::Sounds,
    plugins: &dyn Plugins,
    blocks: usize,
    at: Option<(usize, ParamTarget, f32)>,
) -> (Vec<f32>, Duration) {
    let c = sat_compile::compile_with_plugins(doc, RATE, BLOCK, sounds, plugins).unwrap();
    let start = c.sample(sat_doc::sat_time::Ticks::quarters(32));
    let cmd = at.as_ref().map(|(_, t, v)| c.param_command(t, *v).expect("a live parameter"));
    let (mut e, mut h) = sat_engine::engine(c.schedule, 8);
    h.send(Command::SetLoop(None)).unwrap();
    h.send(Command::Locate(start)).unwrap();
    h.send(Command::Play).unwrap();
    let (mut l, mut r) = ([0.0f32; BLOCK], [0.0f32; BLOCK]);
    let mut out = Vec::new();
    let mut slowest = Duration::ZERO;
    let mut cmd = cmd;
    for b in 0..blocks {
        if at.as_ref().is_some_and(|(k, _, _)| *k == b) {
            h.send(cmd.take().unwrap()).unwrap();
        }
        let t = Instant::now();
        e.process(&mut l, &mut r);
        slowest = slowest.max(t.elapsed());
        out.extend_from_slice(&l);
    }
    assert!(h.telemetry().playhead() > start, "the transport moved on");
    (out, slowest)
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

fn host() -> PlugHost {
    let exe = Path::new(env!("CARGO_BIN_EXE_sat-plughost"));
    PlugHost::spawn(exe, &[], "builtin:crash-test", RATE, BLOCK).unwrap()
}

#[test]
fn a_plugin_at_unity_sounds_exactly_like_the_dry_track() {
    let _s = serial();
    let (d, m) = demo_with_plugin();
    let (dry, _) = play(&d.doc, d.sounds.sounds(), &NoPlugins, 400, None);
    assert!(rms(&dry) > 0.01);
    // a busy test machine can starve the host into a dropout; that is
    // counted, and a run with one is tried again
    for _ in 0..5 {
        let hosts = Hosts(HashMap::from([(m, host())]));
        let (wet, _) = play(&d.doc, d.sounds.sounds(), &hosts, 400, None);
        if hosts.0[&m].dropouts() > 0 {
            continue;
        }
        let worst = wet.iter().zip(&dry).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
        assert!(worst < 1e-6, "differs by {worst}");
        return;
    }
    panic!("every run had a dropout");
}

fn crash_mid_song(mode: f32, want: Status) {
    let _s = serial();
    let (d, m) = demo_with_plugin();
    let mode_target = ParamTarget::Module { module: m, param: "p2".into() };
    // 400 blocks (about 2 s), the plugin dies at block 200. A shared test
    // machine can deschedule the test thread for tens of ms in any block,
    // plugin or not; like a dropout in the unity test, a run with such a
    // stall is tried again with a fresh host
    let mut runs = Vec::new();
    let (mut hosts, out) = loop {
        let hosts = Hosts(HashMap::from([(m, host())]));
        let (out, slowest) = play(&d.doc, d.sounds.sounds(), &hosts, 400, Some((200, mode_target.clone(), mode / 3.0)));
        runs.push(slowest);
        // no block waited past the plugin's budget (half a block, 2.7 ms) by much
        if slowest < Duration::from_millis(25) {
            break (hosts, out);
        }
        assert!(runs.len() < 3, "every run had a slow block: {runs:?}");
    };
    // the set keeps playing after the crash...
    let after = &out[220 * BLOCK..];
    assert!(rms(after) > 0.01, "the set went quiet: {}", rms(after));
    // ...and the keys track is gone from it: compare with the keys muted
    let mut muted = d.doc.clone();
    let keys = muted.tracks.values().find(|t| t.name == "keys").unwrap().id;
    apply(&mut muted, Op::SetTrackFlag { id: keys, flag: TrackFlag::Mute, on: true }).unwrap();
    let (quiet, _) = play(&muted, d.sounds.sounds(), &NoPlugins, 400, None);
    let (full, _) = play(&d.doc, d.sounds.sounds(), &NoPlugins, 400, None);
    let late = 300 * BLOCK..;
    let to = |x: &[f32]| x[late.clone()].iter().zip(&out[late.clone()]).map(|(a, b)| (a - b).abs()).sum::<f32>();
    assert!(to(&quiet) < to(&full), "after the crash it sounds like the set without keys");
    let host = hosts.0.get_mut(&m).unwrap();
    let t = Instant::now();
    while host.status() != want && t.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(host.status(), want);
}

#[test]
fn a_panicking_plugin_leaves_the_set_playing() {
    crash_mid_song(1.0, Status::Crashed);
}

#[test]
fn a_hanging_plugin_leaves_the_set_playing() {
    crash_mid_song(3.0, Status::Hung);
}
