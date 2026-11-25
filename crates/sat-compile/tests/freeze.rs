//! Freezing a track swaps its sources and fx for a render of them, and the
//! mix does not change; unfreezing brings the live sound back.

mod common;

use std::sync::Arc;

use common::*;
use sat_compile::{Sounds, render_range, render_track, song_end};
use sat_doc::sat_time::Ticks;
use sat_doc::*;
use sat_engine::Sample;

fn freeze(h: &mut History, sounds: &mut Sounds, track: TrackId) {
    let (l, r) = render_track(h.doc(), sounds, SR, track).unwrap();
    let id = AudioId::new();
    let frames = l.len() as u64;
    sounds.insert(id, Arc::new(Sample { rate: SR as f32, l, r }));
    let file =
        AudioFile { id, name: "frozen".into(), file: "freeze/frozen.wav".into(), rate: SR as u32, channels: 2, frames };
    h.apply("freeze", vec![Op::AddAudio { file }, Op::SetFrozen { id: track, frozen: Some(Frozen { audio: id }) }])
        .unwrap();
}

fn worst(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| (x - y).abs()).fold(0.0, f32::max)
}

#[test]
fn a_frozen_track_sounds_as_it_did_live() {
    let mut s = song();
    let end = song_end(s.h.doc());
    let mut sounds = Sounds::new();
    let (live_l, live_r) = render_range(s.h.doc(), &sounds, SR, Ticks::ZERO, end, false).unwrap();
    // the lead has fx, a send and gain automation; the bass a patch
    for t in [s.lead, s.bass] {
        freeze(&mut s.h, &mut sounds, t);
    }
    let (l, r) = render_range(s.h.doc(), &sounds, SR, Ticks::ZERO, end, false).unwrap();
    assert_eq!(l.len(), live_l.len());
    assert!(rms(&l) > 0.01);
    let w = worst(&l, &live_l).max(worst(&r, &live_r));
    assert!(w < 1e-5, "frozen mix differs by {w}");
}

#[test]
fn a_frozen_track_ignores_its_patch_until_unfrozen() {
    let mut s = song();
    let end = song_end(s.h.doc());
    let mut sounds = Sounds::new();
    freeze(&mut s.h, &mut sounds, s.bass);
    let before = render_range(s.h.doc(), &sounds, SR, Ticks::ZERO, end, false).unwrap().0;
    // emptying its clip changes nothing heard...
    let module = s.h.doc().inlet_cable(s.bass_clip).unwrap().from.module;
    let clip = s.bass_clip;
    s.h.apply("clear", vec![Op::WritePattern { module, clip, notes: Default::default() }]).unwrap();
    let cleared = render_range(s.h.doc(), &sounds, SR, Ticks::ZERO, end, false).unwrap().0;
    assert_eq!(cleared, before);
    // ...but the strip stays live
    s.h.apply("down", vec![Op::SetValue { target: ParamTarget::Gain(s.bass), value: -12.0 }]).unwrap();
    let quieter = render_range(s.h.doc(), &sounds, SR, Ticks::ZERO, end, false).unwrap().0;
    assert!(rms(&quieter) < rms(&before));
    // unfreezing plays the patch again
    s.h.apply("unfreeze", vec![Op::SetFrozen { id: s.bass, frozen: None }]).unwrap();
    let live = render_range(s.h.doc(), &sounds, SR, Ticks::ZERO, end, false).unwrap().0;
    assert!(rms(&live) < rms(&quieter), "the cleared clip is heard once unfrozen");
}

#[test]
fn only_tracks_with_sound_freeze() {
    let s = song();
    let doc = s.h.doc();
    assert!(matches!(render_track(doc, &Sounds::new(), SR, doc.master), Err(sat_compile::CompileError::Silent)));
    let mut h = History::new(doc.clone(), 10);
    let bad = Op::SetFrozen { id: s.bass, frozen: Some(Frozen { audio: AudioId::new() }) };
    assert!(h.apply("freeze", vec![bad]).is_err());
}
