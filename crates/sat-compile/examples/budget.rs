//! Phase 1 gate check: renders the 64-track reference set at 48 kHz in
//! 128-sample blocks and fails if the average block takes more than 70% of
//! its real-time duration, or a recompile takes more than 10 ms.
//!
//! `cargo run --release -p sat-compile --example budget`

#[path = "../tests/common/mod.rs"]
mod common;

use std::time::Instant;

use sat_engine::{Command, engine};

const SR: f64 = 48_000.0;
const BLOCK: usize = 128;

fn main() {
    if cfg!(debug_assertions) {
        eprintln!("note: debug build; run with --release for meaningful numbers");
    }
    let doc = common::sixty_four();
    let t = Instant::now();
    let compiled = sat_compile::compile(&doc, SR, BLOCK).unwrap();
    let compile_ms = t.elapsed().as_secs_f64() * 1e3;
    let (mut e, mut h) = engine(compiled.schedule, 16);
    h.send(Command::Play).unwrap();
    let mut l = vec![0.0f32; BLOCK];
    let mut r = vec![0.0f32; BLOCK];
    let blocks = 4_000; // ~10.7 s of audio
    let mut worst = 0.0f64;
    let t = Instant::now();
    for _ in 0..blocks {
        let b = Instant::now();
        e.process(&mut l, &mut r);
        worst = worst.max(b.elapsed().as_secs_f64());
    }
    let avg = t.elapsed().as_secs_f64() / blocks as f64;
    let block_s = BLOCK as f64 / SR;
    let load = avg / block_s * 100.0;
    println!("64 tracks @ 48 kHz / {BLOCK}: block budget {:.2} ms", block_s * 1e3);
    println!("  average {:.1} µs ({load:.1}% load), worst {:.1} µs", avg * 1e6, worst * 1e6);
    println!("  compile {compile_ms:.2} ms");
    let ok = load <= 70.0 && compile_ms <= 10.0;
    println!("{}", if ok { "PASS" } else { "FAIL" });
    if !ok && !cfg!(debug_assertions) {
        std::process::exit(1);
    }
}
