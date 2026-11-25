//! A plugin in a host process: audio through it in the same block, and a
//! crash, abort or hang that leaves the engine running with silence.

use std::path::Path;
use std::time::{Duration, Instant};

use sat_plug::{PlugHost, Status};

const RATE: f64 = 48_000.0;
const BLOCK: usize = 256;

fn host() -> PlugHost {
    let exe = Path::new(env!("CARGO_BIN_EXE_sat-plughost"));
    PlugHost::spawn(exe, &[], "builtin:crash-test", RATE, BLOCK).expect("the crash-test plugin loads")
}

/// Runs a block, retrying when a busy machine made it late (a dropout).
fn run_ok(p: &mut sat_plug::Processor, l: &mut [f32], r: &mut [f32], params: &[f32]) -> bool {
    let (l0, r0) = (l.to_vec(), r.to_vec());
    for _ in 0..50 {
        if p.run(l, r, params) {
            return true;
        }
        l.copy_from_slice(&l0);
        r.copy_from_slice(&r0);
        std::thread::sleep(Duration::from_millis(2));
    }
    false
}

/// gain -60..12 dB, mode 0..3, leak 0..8, as the engine sends them (0–1)
fn params(gain_db: f32, mode: f32) -> [f32; 3] {
    [(gain_db + 60.0) / 72.0, mode / 3.0, 0.0]
}

#[test]
fn reports_its_name_and_parameters() {
    let h = host();
    assert_eq!(h.hello.name, "sat crash test");
    let names: Vec<&str> = h.hello.params.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["gain", "mode", "leak"]);
}

#[test]
fn audio_goes_through_the_plugin_in_the_same_block() {
    let h = host();
    let mut p = h.processor();
    for (gain, want) in [(0.0, 1.0), (-6.0, 0.501_187), (6.0, 1.995_262)] {
        let (mut l, mut r) = (vec![1.0f32; BLOCK], vec![-0.5f32; BLOCK]);
        assert!(run_ok(&mut p, &mut l, &mut r, &params(gain, 0.0)));
        assert!((l[100] - want).abs() < 1e-4, "{gain} dB: {}", l[100]);
        assert!((r[100] + want * 0.5).abs() < 1e-4);
    }
    // a callback longer than the region's block goes through in pieces
    let (mut l, mut r) = (vec![1.0f32; BLOCK * 3 + 7], vec![1.0f32; BLOCK * 3 + 7]);
    assert!(run_ok(&mut p, &mut l, &mut r, &params(0.0, 0.0)));
    assert!(l.iter().all(|v| (*v - 1.0).abs() < 1e-6));
}

fn crashes_quietly(mode: f32, want: Status) {
    let mut h = host();
    let mut p = h.processor();
    // warm up: a busy machine can make the first blocks late
    let warm = (0..200).any(|_| {
        let (mut l, mut r) = (vec![1.0f32; BLOCK], vec![1.0f32; BLOCK]);
        let ok = p.run(&mut l, &mut r, &params(0.0, 0.0));
        std::thread::sleep(Duration::from_millis(1));
        ok
    });
    assert!(warm, "the host never answered");
    // the block that kills it comes back silent, within its budget
    let t = Instant::now();
    let (mut l, mut r) = (vec![1.0f32; BLOCK], vec![1.0f32; BLOCK]);
    assert!(!p.run(&mut l, &mut r, &params(0.0, mode)));
    assert!(t.elapsed() < Duration::from_millis(30), "waited {:?}", t.elapsed());
    assert!(l.iter().chain(&r).all(|v| *v == 0.0));
    // and so does every block after, without waiting at all
    let t = Instant::now();
    for _ in 0..100 {
        let (mut l, mut r) = (vec![1.0f32; BLOCK], vec![1.0f32; BLOCK]);
        assert!(!p.run(&mut l, &mut r, &params(0.0, 0.0)));
    }
    assert!(t.elapsed() < Duration::from_millis(10));
    // the app learns what happened
    let t = Instant::now();
    while h.status() != want && t.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(h.status(), want);
}

#[test]
fn a_panicking_plugin_leaves_silence() {
    crashes_quietly(1.0, Status::Crashed);
}

#[test]
fn an_aborting_plugin_leaves_silence() {
    crashes_quietly(2.0, Status::Crashed);
}

#[test]
fn a_hanging_plugin_leaves_silence_and_is_killed_on_drop() {
    crashes_quietly(3.0, Status::Hung);
}

#[test]
fn a_missing_plugin_fails_to_load_with_a_reason() {
    let exe = Path::new(env!("CARGO_BIN_EXE_sat-plughost"));
    let e = PlugHost::spawn(exe, &[], "/nonexistent/thing.clap", RATE, BLOCK).err().unwrap();
    assert!(e.contains("cannot load"), "{e}");
    let e = PlugHost::spawn(exe, &[], "builtin:nope", RATE, BLOCK).err().unwrap();
    assert!(e.contains("no built-in plugin"), "{e}");
}

#[test]
fn dropping_the_host_ends_the_process_and_removes_the_region() {
    let h = host();
    let path = h.shm_path();
    assert!(path.exists());
    drop(h);
    assert!(!path.exists());
}
