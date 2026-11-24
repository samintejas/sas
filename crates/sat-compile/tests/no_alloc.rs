//! Real-time safety: the audio thread never allocates.
//!
//! This binary installs an allocator that aborts if anything allocates inside
//! `assert_no_alloc`, then drives the engine through playback, parameter
//! changes, locate, loop, a schedule swap and stop.

mod common;

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use common::*;
use sat_doc::*;
use sat_engine::{Command, engine};

#[global_allocator]
static A: AllocDisabler = AllocDisabler;

#[test]
fn engine_process_does_not_allocate() {
    let mut s = song();
    let c = sat_compile::compile(s.h.doc(), SR, BLOCK).unwrap();
    let gain = c.param_command(&ParamTarget::Gain(s.bass), -6.0).unwrap();
    let send = c.param_command(&ParamTarget::SendLevel { track: s.lead, target: s.ret }, -3.0).unwrap();
    let (mut e, mut h) = engine(c.schedule, 64);
    let (_, op) = Op::add_track("extra", None, 9);
    s.h.apply("add", [op]).unwrap();
    let next = Box::new(sat_compile::compile(s.h.doc(), SR, BLOCK).unwrap().schedule);

    let mut l = vec![0.0f32; BLOCK];
    let mut r = vec![0.0f32; BLOCK];
    h.send(Command::Play).unwrap();
    h.send(gain).unwrap();
    h.send(send).unwrap();
    h.send(Command::SetLoop(Some((10_000, 70_000)))).unwrap();
    assert_no_alloc(|| {
        for _ in 0..400 {
            e.process(&mut l, &mut r);
        }
    });
    h.send(Command::Swap(next)).unwrap();
    h.send(Command::Locate(30_000)).unwrap();
    assert_no_alloc(|| {
        for _ in 0..200 {
            e.process(&mut l, &mut r);
        }
    });
    h.send(Command::Stop).unwrap();
    assert_no_alloc(|| {
        for _ in 0..50 {
            e.process(&mut l, &mut r);
        }
    });
    assert_eq!(h.collect_garbage(), 1);
}

#[test]
fn sixty_four_tracks_do_not_allocate() {
    let doc = sixty_four();
    let c = sat_compile::compile(&doc, SR, BLOCK).unwrap();
    let (mut e, mut h) = engine(c.schedule, 16);
    h.send(Command::Play).unwrap();
    let mut l = vec![0.0f32; BLOCK];
    let mut r = vec![0.0f32; BLOCK];
    assert_no_alloc(|| {
        for _ in 0..300 {
            e.process(&mut l, &mut r);
        }
    });
    assert!(l.iter().any(|v| *v != 0.0));
}

#[test]
fn instrument_patches_and_drums_do_not_allocate() {
    use sat_modules::instrument as ins;
    let mut s = song();
    let grid = s.h.doc().tracks[&s.bass].instrument_grid.unwrap();
    let dgrid = s.h.doc().tracks[&s.drums].instrument_grid.unwrap();
    let (mut ops, ids) = ins::default_patch_ops(grid, &[]);
    ops.push(Op::AddModule {
        id: ModuleId::new(),
        grid: dgrid,
        kind: ins::DRUM_SYNTH.into(),
        name: "drum synth".into(),
        params: sat_modules::default_params(ins::DRUM_SYNTH),
        pos: (0.0, 0.0),
    });
    s.h.apply("patch", ops).unwrap();
    let c = sat_compile::compile(s.h.doc(), SR, BLOCK).unwrap();
    let cutoff = c.param_command(&ParamTarget::Module { module: ids[2], param: "cutoff".into() }, 300.0).unwrap();
    let (mut e, mut h) = engine(c.schedule, 64);
    // a knob change that recompiles (a new cable) swaps in a schedule that adopts the voices
    let ops = sat_modules::patch(s.h.doc(), &PortRef::new(ids[0], "vel"), &PortRef::new(ids[1], "fm"), None).unwrap();
    s.h.apply("patch", ops).unwrap();
    let next = Box::new(sat_compile::compile(s.h.doc(), SR, BLOCK).unwrap().schedule);
    let mut l = vec![0.0f32; BLOCK];
    let mut r = vec![0.0f32; BLOCK];
    h.send(Command::Play).unwrap();
    h.send(cutoff).unwrap();
    assert_no_alloc(|| {
        for _ in 0..400 {
            e.process(&mut l, &mut r);
        }
    });
    assert!(l.iter().any(|v| *v != 0.0));
    h.send(Command::Swap(next)).unwrap();
    h.send(Command::Locate(30_000)).unwrap();
    assert_no_alloc(|| {
        for _ in 0..200 {
            e.process(&mut l, &mut r);
        }
    });
    h.send(Command::Stop).unwrap();
    assert_no_alloc(|| {
        for _ in 0..50 {
            e.process(&mut l, &mut r);
        }
    });
    assert_eq!(h.collect_garbage(), 1);
}
