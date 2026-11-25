//! Export matches playback sample for sample, whatever block sizes the
//! device asks for.

mod common;

use common::*;
use sat_engine::{Command, engine};

/// Plays `frames` from the start in device callbacks of `cb` frames, like
/// the app's output stream (schedule built for blocks of `max`).
fn playback(doc: &sat_doc::Document, max: usize, cb: usize, frames: usize) -> Vec<f32> {
    let c = sat_compile::compile(doc, SR, max).unwrap();
    let (mut e, mut h) = engine(c.schedule, 8);
    h.send(Command::SetLoop(None)).unwrap();
    h.send(Command::Play).unwrap();
    let mut out = vec![0.0f32; frames];
    let mut r = vec![0.0f32; frames];
    let mut i = 0;
    while i < frames {
        let n = cb.min(frames - i);
        let (a, b) = (&mut out[i..i + n], &mut r[i..i + n]);
        // the callback splits into blocks no larger than the schedule's
        let mut k = 0;
        while k < n {
            let m = max.min(n - k);
            e.process(&mut a[k..k + m], &mut b[k..k + m]);
            k += m;
        }
        i += n;
    }
    out
}

#[test]
fn export_matches_playback_for_any_callback_size() {
    let s = song();
    let frames = 4 * 48_000;
    let export = playback(s.h.doc(), 1024, 1024, frames);
    assert!(rms(&export) > 0.01);
    for cb in [240, 441, 64, 1, 1000] {
        let p = playback(s.h.doc(), 1024, cb, frames);
        let first = p.iter().zip(&export).position(|(a, b)| a != b);
        assert_eq!(first, None, "callbacks of {cb}: first difference at sample {first:?}");
    }
}
