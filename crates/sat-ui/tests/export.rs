//! The demo song exports exactly as it plays: patches, drums, samplers,
//! comped audio clips, fx, sends and automation, whatever block sizes the
//! audio device asks for.

use sat_doc::sat_time::Ticks;
use sat_engine::{Command, engine};
use sat_ui::demo::{bars, demo};

const SR: f64 = 48_000.0;

/// Plays from `from` like the app does: a schedule for blocks of 1024,
/// device callbacks of `cb` frames, the loop off.
fn playback(d: &sat_ui::demo::Demo, from: Ticks, cb: usize, frames: usize) -> Vec<f32> {
    let c = sat_compile::compile_with(&d.doc, SR, 1024, d.sounds.sounds()).unwrap();
    let start = c.sample(from);
    let (mut e, mut h) = engine(c.schedule, 8);
    h.send(Command::Locate(start)).unwrap();
    h.send(Command::Play).unwrap();
    let (mut l, mut r) = (vec![0.0f32; frames], vec![0.0f32; frames]);
    let mut i = 0;
    while i < frames {
        let n = cb.min(frames - i);
        let mut k = 0;
        while k < n {
            let m = 1024.min(n - k);
            e.process(&mut l[i + k..i + k + m], &mut r[i + k..i + k + m]);
            k += m;
        }
        i += n;
    }
    l
}

#[test]
fn the_demo_exports_sample_for_sample_as_it_plays() {
    let d = demo();
    // bars 5–6: the verse, with the comped vocal and its crossfades
    let (from, to) = (bars(4), bars(6));
    let (l, r) = sat_compile::render_range(&d.doc, d.sounds.sounds(), SR, from, to, false).unwrap();
    assert_eq!(l.len(), r.len());
    let frames = l.len();
    assert!(l.iter().fold(0.0f32, |a, x| a.max(x.abs())) > 0.05, "it sounds");
    for cb in [240, 441] {
        let p = playback(&d, from, cb, frames);
        let first = p.iter().zip(&l).position(|(a, b)| a != b);
        assert_eq!(first, None, "callbacks of {cb}: first difference at {first:?}");
    }
}

#[test]
fn a_song_export_runs_on_until_its_tail_falls_silent() {
    let d = demo();
    let end = sat_compile::song_end(&d.doc);
    assert_eq!(end, bars(16));
    // the last bar, with the reverb and echo ringing out after it
    let (l, _) = sat_compile::render_range(&d.doc, d.sounds.sounds(), SR, bars(15), end, true).unwrap();
    let bar = (60.0 / 124.0 * 4.0 * SR) as usize;
    assert!(l.len() > bar + 4_800, "a tail: {} frames past the bar", l.len() as i64 - bar as i64);
    assert!(l.len() <= bar + (sat_compile::MAX_TAIL_SECS * SR) as usize);
    assert!(l[l.len() - 100..].iter().all(|x| x.abs() < 1e-3), "it ends quiet");
}
