mod common;

use common::*;
use im::Vector;
use sat_compile::{compile, placement_notes};
use sat_doc::sat_time::Ticks;
use sat_doc::*;
use sat_engine::{Command, engine};

#[test]
fn looping_placement_with_offset() {
    let s = song();
    let d = s.h.doc();
    let p = d.placements.values().find(|p| p.clip == s.bass_clip).unwrap();
    let mut p = *p;
    p.offset = Ticks::sixteenths(6);
    p.length = Ticks::sixteenths(12);
    let notes = placement_notes(d, &p);
    let starts: Vec<i64> = notes.iter().map(|n| n.start.0 / 240).collect();
    // clip notes at 16ths 0, 6, 10 of a 16-step loop; window starts at step 6
    assert_eq!(starts, vec![0, 4, 10]);
    // the note at clip step 10 lasts 4 steps but is cut at the loop end (step 16 = window step 10)
    assert_eq!(notes[1].end.0 - notes[1].start.0, Ticks::sixteenths(4).0);
    // the second cycle's first note (3 steps from window step 10) is cut by the placement end
    assert_eq!(notes[2].end, p.start + p.length);
    assert_eq!(notes.len(), 3);
}

#[test]
fn renders_sound_deterministically() {
    let s = song();
    let a = render(s.h.doc(), 24_000);
    let b = render(s.h.doc(), 24_000);
    assert!(rms(&a.0) > 0.01, "rms {}", rms(&a.0));
    assert_eq!(a, b);
}

#[test]
fn mute_and_solo() {
    let mut s = song();
    // solo the lead: drums and bass go silent, the lead and its return play
    s.h.apply("solo", [Op::SetTrackFlag { id: s.lead, flag: TrackFlag::Solo, on: true }]).unwrap();
    let (l, _) = render(s.h.doc(), 96_000);
    // first two beats: the lead has not started and everything else is soloed out
    assert_eq!(rms(&l[..48_000]), 0.0);
    assert!(rms(&l[48_000..]) > 0.0);
    s.h.apply("unsolo", [Op::SetTrackFlag { id: s.lead, flag: TrackFlag::Solo, on: false }]).unwrap();
    // muting the group silences both of its tracks
    s.h.apply("mute", [Op::SetTrackFlag { id: s.group, flag: TrackFlag::Mute, on: true }]).unwrap();
    assert_eq!(rms(&render(s.h.doc(), 24_000).0), 0.0);
}

#[test]
fn tempo_moves_events() {
    let mut s = song();
    let one = |d: &Document| {
        let c = compile(d, SR, BLOCK).unwrap();
        c.sample(Ticks::quarters(1))
    };
    assert_eq!(one(s.h.doc()), 24_000);
    let lane = s.h.doc().tempo_lane;
    s.h.apply(
        "tempo",
        [Op::SetLanePoints {
            id: lane,
            points: Vector::unit(Breakpoint { tick: Ticks::ZERO, value: 60.0, curve: Curve::Hold }),
        }],
    )
    .unwrap();
    assert_eq!(one(s.h.doc()), 48_000);
}

#[test]
fn live_parameter_changes_reach_the_engine() {
    let s = song();
    let c = compile(s.h.doc(), SR, BLOCK).unwrap();
    let cmd = c.param_command(&ParamTarget::Gain(s.group), -120.0).unwrap();
    let send = c.param_command(&ParamTarget::SendLevel { track: s.lead, target: s.ret }, -6.0);
    assert!(matches!(send, Some(Command::SetParam { value, .. }) if (value - 0.501).abs() < 0.01));
    let synth = c.param_command(&ParamTarget::Module { module: s.bass_synth, param: "level".into() }, 0.0);
    assert!(synth.is_some());
    let (mut e, mut h) = engine(c.schedule, 16);
    h.send(Command::Play).unwrap();
    h.send(cmd).unwrap();
    // two blocks to let the fader ramp down, then the group is silent
    let mut l = vec![0.0; BLOCK];
    let mut r = vec![0.0; BLOCK];
    e.process(&mut l, &mut r);
    let mut total = 0.0;
    for _ in 0..80 {
        e.process(&mut l, &mut r);
        total += rms(&l);
    }
    // the lead starts on beat 3 (48_000 samples); 81 blocks is ~10k samples
    assert_eq!(total, 0.0);
}

#[test]
fn recompile_and_swap_keeps_voices() {
    let mut s = song();
    let c = compile(s.h.doc(), SR, BLOCK).unwrap();
    let (mut e, mut h) = engine(c.schedule, 16);
    h.send(Command::Play).unwrap();
    let mut l = vec![0.0; BLOCK];
    let mut r = vec![0.0; BLOCK];
    for _ in 0..10 {
        e.process(&mut l, &mut r);
    }
    let before = l[BLOCK - 1];
    // a structural edit elsewhere (a new empty track) recompiles everything
    let (_, op) = Op::add_track("new", None, 9);
    s.h.apply("add", [op]).unwrap();
    let c2 = compile(s.h.doc(), SR, BLOCK).unwrap();
    h.send(Command::Swap(Box::new(c2.schedule))).unwrap();
    e.process(&mut l, &mut r);
    // the kick and bass that were sounding keep sounding: no jump to silence
    assert!(before.abs() > 0.0 && l[0].abs() > 0.0);
    assert!((l[0] - before).abs() < 0.2, "click: {before} -> {}", l[0]);
    assert_eq!(h.collect_garbage(), 1);
}

#[test]
fn gain_automation_shapes_the_lead() {
    let mut s = song();
    // isolate the lead
    s.h.apply("solo", [Op::SetTrackFlag { id: s.lead, flag: TrackFlag::Solo, on: true }]).unwrap();
    s.h.apply("no send", [Op::RemoveSend { track: s.lead, target: s.ret }]).unwrap();
    let (l, _) = render(s.h.doc(), 6 * 24_000);
    // lane ramps -12 dB at beat 2 up to 0 dB at beat 6; the first note (beats 2–4) is quieter than the second (4–6)
    let early = rms(&l[2 * 24_000 + 2000..4 * 24_000 - 2000]);
    let late = rms(&l[4 * 24_000 + 2000..6 * 24_000 - 2000]);
    assert!(late > early * 1.5, "early {early} late {late}");
}

#[test]
fn loop_wraps_inside_a_block() {
    let s = song();
    let c = compile(s.h.doc(), SR, BLOCK).unwrap();
    let (mut e, mut h) = engine(c.schedule, 16);
    h.send(Command::SetLoop(Some((0, 1000)))).unwrap();
    h.send(Command::Play).unwrap();
    let mut l = vec![0.0; BLOCK];
    let mut r = vec![0.0; BLOCK];
    for _ in 0..20 {
        e.process(&mut l, &mut r);
        assert!((0..1000).contains(&e.position()));
    }
}
