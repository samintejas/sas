//! Phase 1 gate: the 64-track reference set at 48 kHz / 128 samples must
//! use at most 70% of the 2.7 ms block.

use criterion::{Criterion, criterion_group, criterion_main};
use sat_engine::{Command, engine};

#[path = "../tests/common/mod.rs"]
mod common;

const SR: f64 = 48_000.0;
const BLOCK: usize = 128;

fn bench(c: &mut Criterion) {
    let doc = common::sixty_four();
    let compiled = sat_compile::compile(&doc, SR, BLOCK).unwrap();
    let (mut e, mut h) = engine(compiled.schedule, 16);
    h.send(Command::Play).unwrap();
    let mut l = vec![0.0f32; BLOCK];
    let mut r = vec![0.0f32; BLOCK];
    // warm up into the busy part of the pattern
    for _ in 0..200 {
        e.process(&mut l, &mut r);
    }
    c.bench_function("64 tracks, one 128-sample block", |b| b.iter(|| e.process(&mut l, &mut r)));
    c.bench_function("compile 64 tracks", |b| b.iter(|| sat_compile::compile(&doc, SR, BLOCK).unwrap()));
}

criterion_group!(benches, bench);
criterion_main!(benches);
