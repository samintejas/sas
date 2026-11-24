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
