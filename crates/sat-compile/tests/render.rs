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

#[test]
fn strip_meters_follow_the_signal() {
    let mut s = song();
    s.h.apply("mute", [Op::SetTrackFlag { id: s.drums, flag: TrackFlag::Mute, on: true }]).unwrap();
    let d = s.h.doc();
    let c = compile(d, SR, BLOCK).unwrap();
    let meter = |t| c.meter(t).expect("every strip has a meter").clone();
    let (bass, drums, master) = (meter(s.bass), meter(s.drums), meter(d.master));
    let (out, _) = sat_engine::render(c.schedule.clone(), 0, 24_000, BLOCK);
    let peak = out.iter().fold(0f32, |m, v| m.max(v.abs()));
    assert!(bass.take().0 > 0.0, "bass plays from the start");
    assert_eq!(drums.take(), (0.0, 0.0), "muted strips read silent");
    // master's meter is the output peak
    assert!((master.take().0 - peak).abs() < 1e-6);
}

#[test]
fn euclid_sources_play_their_steps_and_hand_edits() {
    let mut s = song();
    let d = s.h.doc();
    let grid = d.tracks[&s.bass].source_grid.unwrap();
    let old = d.inlet_cable(s.bass_clip).unwrap().id;
    let before = render(d, 24_000);
    let eu = ModuleId::new();
    let mut params = sat_modules::default_params(sat_modules::EUCLID);
    params.insert("hits".into(), 3.0);
    params.insert("steps".into(), 8.0);
    s.h.apply(
        "euclid",
        [
            Op::AddModule {
                id: eu,
                grid,
                kind: sat_modules::EUCLID.into(),
                name: "euclid".into(),
                params,
                pos: (0.0, 0.0),
            },
            Op::Disconnect { id: old },
            Op::Connect { id: CableId::new(), from: PortRef::new(eu, "notes"), to: Endpoint::Inlet(s.bass_clip) },
        ],
    )
    .unwrap();
    let starts = |s: &Song| -> Vec<i64> {
        let d = s.h.doc();
        let p = d.placements.values().find(|p| p.clip == s.bass_clip).unwrap();
        let first_bar: Vec<i64> =
            placement_notes(d, p).iter().map(|n| (n.start.0 - p.start.0) / 240).filter(|t| *t < 16).collect();
        first_bar
    };
    // 3 hits over 8 steps, twice per bar
    assert_eq!(starts(&s), vec![0, 3, 6, 8, 11, 14]);
    let note = sat_modules::euclid::Params::of(&s.h.doc().modules[&eu]);
    let edit = sat_modules::step_note(Ticks::sixteenths(3), note.note_len(), note.note, 0.7);
    let ops = sat_modules::toggle_step(s.h.doc(), eu, s.bass_clip, edit);
    s.h.apply("edit", ops).unwrap();
    assert_eq!(starts(&s), vec![0, 6, 8, 11, 14]);
    let after = render(s.h.doc(), 24_000);
    assert!(rms(&after.0) > 0.001);
    assert_ne!(before, after, "the bass now plays the euclid");
}

#[test]
fn instrument_grids_play_patches_and_drums() {
    use sat_modules::instrument as ins;
    let mut s = song();
    let d = s.h.doc();
    let before = render(d, 24_000);
    let grid = d.tracks[&s.bass].instrument_grid.unwrap();
    let dgrid = d.tracks[&s.drums].instrument_grid.unwrap();
    let (mut ops, ids) = ins::default_patch_ops(grid, &[(ins::FILTER, "cutoff", 900.0)]);
    let drum = ModuleId::new();
    ops.push(Op::AddModule {
        id: drum,
        grid: dgrid,
        kind: ins::DRUM_SYNTH.into(),
        name: "drum synth".into(),
        params: sat_modules::default_params(ins::DRUM_SYNTH),
        pos: (0.0, 0.0),
    });
    s.h.apply("patch", ops).unwrap();
    let c = sat_compile::compile(s.h.doc(), 48_000.0, 512).unwrap();
    let (patch, slots) = sat_compile::build_patch(s.h.doc(), grid).unwrap();
    // note in comes first and voice out after the vca it reads
    let pos = |id: ModuleId| slots.iter().position(|x| *x == id).unwrap();
    assert_eq!(pos(ids[0]), 0);
    assert!(pos(ids[3]) < pos(ids[4]) && pos(ids[5]) < pos(ids[3]));
    assert_eq!(patch.mods.len(), 6);
    // a live knob reaches the patch without a recompile
    let target = ParamTarget::Module { module: ids[2], param: "cutoff".into() };
    assert!(matches!(c.param_command(&target, 400.0), Some(Command::SetParam { .. })));
    let drum_target = ParamTarget::Module { module: drum, param: "decay".into() };
    assert!(c.param_command(&drum_target, 2.0).is_some());
    let after = render(s.h.doc(), 24_000);
    assert!(rms(&after.0) > 0.01, "rms {}", rms(&after.0));
    assert_ne!(before, after, "bass and drums now play through the grid");
    assert_eq!(after, render(s.h.doc(), 24_000), "deterministic");
    // breaking the patch (no voice out) falls back to the test synth
    s.h.apply("rm", [Op::RemoveModule { id: ids[4] }]).unwrap();
    assert!(sat_compile::build_patch(s.h.doc(), grid).is_none());
}

#[test]
fn fx_grids_process_tracks_and_returns() {
    use sat_modules::fx;
    let mut s = song();
    let before = render(s.h.doc(), 48_000);
    // a grid with only a module and no in/out passes audio through
    let lead_fx = s.h.doc().tracks[&s.lead].fx_grids[0];
    let (_, op) = sat_modules::add_module(s.h.doc(), lead_fx, fx::DRIVE, (0.0, 0.0));
    s.h.apply("lonely drive", [op]).unwrap();
    assert!(sat_compile::build_fx(s.h.doc(), lead_fx).is_none());
    assert_eq!(render(s.h.doc(), 48_000), before, "no in and out: passes through");
    // a reverb on the return, a limiter on the master
    let ret_fx = s.h.doc().tracks[&s.ret].fx_grids[0];
    let (verb, mut ops) = fx::insert(s.h.doc(), ret_fx, fx::REVERB);
    ops.push(Op::SetValue { target: ParamTarget::Module { module: verb, param: "mix".into() }, value: 1.0 });
    s.h.apply("reverb", ops).unwrap();
    let master_fx = s.h.doc().tracks[&s.h.doc().master].fx_grids[0];
    let (lim, ops) = fx::insert(s.h.doc(), master_fx, fx::LIMITER);
    s.h.apply("limiter", ops).unwrap();
    let ceiling = ParamTarget::Module { module: lim, param: "ceiling".into() };
    s.h.apply("ceiling", [Op::SetValue { target: ceiling.clone(), value: -12.0 }]).unwrap();
    let after = render(s.h.doc(), 48_000);
    assert_ne!(before, after);
    let ceil = 10f32.powf(-12.0 / 20.0);
    assert!(after.0.iter().chain(&after.1).all(|x| x.abs() <= ceil + 1e-5), "the master limiter holds -12 db");
    assert!(rms(&after.0) > 0.01);
    assert_eq!(after, render(s.h.doc(), 48_000), "deterministic");
    // knobs and the bypass switch change live
    let c = sat_compile::compile(s.h.doc(), SR, BLOCK).unwrap();
    assert!(c.param_command(&ceiling, -3.0).is_some());
    let bypass = ParamTarget::Module { module: verb, param: fx::BYPASS.into() };
    assert!(c.param_command(&bypass, 1.0).is_some());
    // bypassing the limiter lifts the ceiling
    s.h.apply(
        "bypass",
        [Op::SetValue { target: ParamTarget::Module { module: lim, param: fx::BYPASS.into() }, value: 1.0 }],
    )
    .unwrap();
    let open = render(s.h.doc(), 48_000);
    assert!(open.0.iter().chain(&open.1).any(|x| x.abs() > ceil + 0.01));
}

#[test]
fn macros_drive_their_parameters() {
    use sat_modules::{instrument as ins, macros};
    let mut s = song();
    let grid = s.h.doc().tracks[&s.bass].instrument_grid.unwrap();
    let (ops, ids) = ins::default_patch_ops(grid, &[]);
    s.h.apply("patch", ops).unwrap();
    let filter = ids[2];
    let (index, ops) = macros::learn(s.h.doc(), filter, "cutoff").unwrap();
    s.h.apply("learn", ops).unwrap();
    let before = render(s.h.doc(), 24_000);
    let target = ParamTarget::Macro { grid, index };
    // a macro is a live value: one command per parameter it drives
    let c = sat_compile::compile(s.h.doc(), SR, BLOCK).unwrap();
    let cmds = c.param_commands(&target, 0.1);
    assert_eq!(cmds.len(), 1);
    let Command::SetParam { value, .. } = cmds[0] else { panic!() };
    let spec = sat_modules::spec(ins::FILTER, "cutoff").unwrap();
    assert!((value - spec.at(0.1)).abs() < 1e-3);
    // the parameter itself no longer answers to its own knob
    let own = ParamTarget::Module { module: filter, param: "cutoff".into() };
    assert!(c.param_commands(&own, 5000.0).is_empty());
    // and a recompile reads the macro, not the stored value
    s.h.apply("macro", [Op::SetValue { target: target.clone(), value: 0.1 }]).unwrap();
    let after = render(s.h.doc(), 24_000);
    assert_ne!(before, after);
    s.h.apply("own", [Op::SetValue { target: own, value: 15_000.0 }]).unwrap();
    assert_eq!(render(s.h.doc(), 24_000), after, "the macro decides");
    // a macro on a generator reshapes the notes
    let src = s.h.doc().tracks[&s.bass].source_grid.unwrap();
    let (eu, op) = sat_modules::add_module(s.h.doc(), src, sat_modules::EUCLID, (0.0, 0.0));
    s.h.apply("eu", [op]).unwrap();
    s.h.apply("wire", sat_modules::wire(s.h.doc(), eu, s.bass_clip)).unwrap();
    let (j, ops) = macros::learn(s.h.doc(), eu, "hits").unwrap();
    s.h.apply("learn hits", ops).unwrap();
    let hits = ParamTarget::Macro { grid: src, index: j };
    assert!(sat_modules::shapes_notes(s.h.doc(), &hits));
    let n = |s: &Song| sat_modules::effective_notes(s.h.doc(), eu, s.bass_clip).len();
    let few = n(&s);
    s.h.apply("more hits", [Op::SetValue { target: hits, value: 0.25 }]).unwrap();
    assert!(n(&s) > few, "{} > {few}", n(&s));
}

#[test]
fn samplers_play_pool_sounds_and_go_silent_without_one() {
    use sat_modules::instrument as ins;
    use std::sync::Arc;
    let mut s = song();
    let grid = s.h.doc().tracks[&s.bass].instrument_grid.unwrap();
    let (smp, op) = sat_modules::add_module(s.h.doc(), grid, ins::SAMPLER, (0.0, 0.0));
    let audio = AudioId::new();
    let file =
        AudioFile { id: audio, name: "saw".into(), file: "saw.wav".into(), rate: 44_100, channels: 1, frames: 44_100 };
    s.h.apply("sampler", [op, Op::AddAudio { file }]).unwrap();
    // removing the test synth leaves the sampler as the bass's instrument
    let synth = s.bass_synth;
    s.h.apply("rm synth", [Op::RemoveModule { id: synth }]).unwrap();
    let saw: Vec<f32> = (0..44_100).map(|i| (i % 100) as f32 / 50.0 - 1.0).collect();
    let mut sounds = sat_compile::Sounds::new();
    sounds.insert(audio, Arc::new(sat_engine::Sample::from_interleaved(44_100.0, 1, &saw)));
    let play = |d: &Document, sounds: &sat_compile::Sounds| {
        let c = sat_compile::compile_with(d, SR, BLOCK, sounds).unwrap();
        sat_engine::render(c.schedule, 0, 96_000, BLOCK)
    };
    assert_eq!(rms(&play(s.h.doc(), &sounds).0), rms(&render(s.h.doc(), 96_000).0), "no sound set yet");
    s.h.apply("set sound", [Op::SetModuleAudio { module: smp, audio: Some(audio) }]).unwrap();
    let with = play(s.h.doc(), &sounds);
    assert!(rms(&with.0) > rms(&render(s.h.doc(), 96_000).0) + 0.005, "the sampler sounds");
    assert_eq!(ins::label(s.h.doc(), grid).as_deref(), Some("sampler · saw"));
    let c = sat_compile::compile_with(s.h.doc(), SR, BLOCK, &sounds).unwrap();
    let tune = ParamTarget::Module { module: smp, param: "tune".into() };
    assert!(c.param_command(&tune, 7.0).is_some(), "knobs are live");
    // removing the sound from the pool unsets it
    s.h.apply("rm sound", [Op::RemoveAudio { id: audio }]).unwrap();
    assert_eq!(s.h.doc().modules[&smp].audio, None);
    assert!(s.h.apply("bad", [Op::SetModuleAudio { module: smp, audio: Some(audio) }]).is_err());
}

#[test]
fn session_clips_launch_over_the_arrangement() {
    let mut s = song();
    s.h.apply("solo", [Op::SetTrackFlag { id: s.lead, flag: TrackFlag::Solo, on: true }]).unwrap();
    let d = s.h.doc();
    let clip = d.clips_on(s.lead).next().unwrap().id;
    let play = |cmds: Vec<Command>, frames: usize| {
        let c = compile(d, SR, BLOCK).unwrap();
        let (mut e, mut h) = engine(c.schedule, 16);
        for cmd in cmds {
            h.send(cmd).unwrap();
        }
        h.send(Command::Play).unwrap();
        let mut out = vec![0.0f32; frames];
        let mut r = vec![0.0f32; frames];
        for (l, r) in out.chunks_mut(BLOCK).zip(r.chunks_mut(BLOCK)) {
            e.process(l, r);
        }
        out
    };
    let c = compile(d, SR, BLOCK).unwrap();
    // the lead's arrangement starts at beat 2; launched at sample 0 it plays from the start
    assert_eq!(rms(&play(vec![], 96_000)[..48_000]), 0.0);
    let launch = c.launch(s.lead, Some(clip), 0).unwrap();
    let launched = play(vec![launch], 96_000);
    assert!(rms(&launched[..48_000]) > 0.001);
    // a stop at beat 1 silences the track, the arrangement included
    let stop = c.launch(s.lead, None, 24_000).unwrap();
    let stopped = play(vec![c.launch(s.lead, Some(clip), 0).unwrap(), stop], 96_000);
    assert!(rms(&stopped[40_000..]) < 1e-4, "{}", rms(&stopped[40_000..]));
    // back to the arrangement
    let back = play(vec![c.launch(s.lead, None, 0).unwrap(), Command::BackToArrangement], 96_000);
    assert_eq!(back, play(vec![], 96_000));
    assert!(c.launch(s.ret, Some(clip), 0).is_none(), "returns launch nothing");
}

#[test]
fn a_launched_clips_envelope_takes_over_the_tracks_automation() {
    let mut s = song();
    s.h.apply("solo", [Op::SetTrackFlag { id: s.lead, flag: TrackFlag::Solo, on: true }]).unwrap();
    let clip = s.h.doc().clips_on(s.lead).next().unwrap().id;
    let len = s.h.doc().clips[&clip].length;
    let left = |tick| Breakpoint { tick, value: -1.0, curve: Curve::Linear };
    s.h.apply(
        "env",
        [Op::SetClipEnvelope {
            clip,
            target: ParamTarget::Pan(s.lead),
            points: im::Vector::from(vec![left(Ticks::ZERO), left(len)]),
        }],
    )
    .unwrap();
    let d = s.h.doc();
    let c = compile(d, SR, BLOCK).unwrap();
    let play = |cmds: Vec<Command>| {
        let c = compile(d, SR, BLOCK).unwrap();
        let (mut e, mut h) = engine(c.schedule, 16);
        for cmd in cmds {
            h.send(cmd).unwrap();
        }
        h.send(Command::Play).unwrap();
        let (mut l, mut r) = (vec![0.0f32; 48_000], vec![0.0f32; 48_000]);
        for (l, r) in l.chunks_mut(BLOCK).zip(r.chunks_mut(BLOCK)) {
            e.process(l, r);
        }
        (rms(&l), rms(&r))
    };
    let launch = || c.launch(s.lead, Some(clip), 0).unwrap();
    // launched with takeover, the clip's envelope pans the track hard left
    let (l, r) = play(vec![launch(), c.launch_envelopes(s.lead, Some(clip), 0, true).unwrap()]);
    // (a little right leaks in while the gains glide from centre)
    assert!(l > 0.001 && r < l * 0.02, "{l} {r}");
    // without takeover the track's own (centre) pan plays
    let (l, r) = play(vec![launch(), c.launch_envelopes(s.lead, Some(clip), 0, false).unwrap()]);
    assert!(l > 0.001 && (l - r).abs() < l * 0.05, "{l} {r}");
}

#[test]
fn audio_clips_play_their_comp_with_fades_and_crossfades() {
    use std::sync::Arc;
    let mut h = History::new(Document::new(1), 10);
    let (t, op) = Op::add_track("vox", None, 0);
    let clip = ClipId::new();
    let (a, b) = (AudioId::new(), AudioId::new());
    let file = |id, n: &str| AudioFile {
        id,
        name: n.into(),
        file: format!("{n}.wav"),
        rate: 48_000,
        channels: 1,
        frames: 96_000,
    };
    h.apply(
        "clip",
        [
            op,
            Op::AddClip {
                id: clip,
                track: t,
                name: "vox".into(),
                color: Color(4),
                length: Ticks::quarters(4),
                looped: false,
                seed: 1,
            },
            Op::AddAudio { file: file(a, "a") },
            Op::AddAudio { file: file(b, "b") },
            Op::AddPlacement {
                id: PlacementId::new(),
                clip,
                start: Ticks::quarters(1),
                offset: Ticks::ZERO,
                length: Ticks::quarters(4),
            },
        ],
    )
    .unwrap();
    let take = |audio, n: u32| Take { audio, name: format!("take {n}"), offset: 0 };
    let mut ac = AudioClip::single(take(a, 1));
    ac.takes.push_back(take(b, 2));
    // take 2 over beats 2–3 of the clip (timeline beats 3–4)
    let ac = ac.comped(Ticks::quarters(2), Ticks::quarters(3), 1, Ticks::quarters(4));
    h.apply("comp", [Op::SetClipAudio { clip, audio: Some(ac) }]).unwrap();
    let mut sounds = sat_compile::Sounds::new();
    sounds.insert(a, Arc::new(sat_engine::Sample { rate: 48_000.0, l: vec![0.2; 96_000], r: vec![0.2; 96_000] }));
    sounds.insert(b, Arc::new(sat_engine::Sample { rate: 48_000.0, l: vec![0.4; 96_000], r: vec![0.4; 96_000] }));
    let c = sat_compile::compile_with(h.doc(), SR, BLOCK, &sounds).unwrap();
    let (l, _) = sat_engine::render(c.schedule, 0, 6 * 24_000, BLOCK);
    let at = |beat: f64| l[(beat * 24_000.0) as usize];
    assert_eq!(rms(&l[..24_000]), 0.0, "silent before the placement");
    let (lo, hi) = (at(1.5), at(3.5));
    assert!(lo > 0.0 && (hi / lo - 2.0).abs() < 1e-3, "take 2 plays twice as loud: {lo} {hi}");
    assert!((at(4.5) - lo).abs() < 1e-6, "and take 1 again after it");
    // the 20 ms crossfades never dip below the quieter take
    let xf = 480;
    for (i, v) in l.iter().enumerate().take(3 * 24_000 + xf).skip(3 * 24_000 - xf) {
        assert!(*v >= lo * 0.999 && *v <= hi * 1.42, "{i}: {v}");
    }
    assert_eq!(rms(&l[5 * 24_000 + 10..]), 0.0, "silent after the clip");
    // a placement trimmed into the clip starts the sound at its offset, declicked
    let p = h.doc().placements.keys().next().copied().unwrap();
    h.apply(
        "trim",
        [Op::UpdatePlacement {
            id: p,
            start: Ticks::quarters(1),
            offset: Ticks::quarters(2),
            length: Ticks::quarters(2),
        }],
    )
    .unwrap();
    let segs = sat_compile::audio_segments(h.doc(), t, &h.doc().tempo_map().unwrap(), SR, &sounds);
    assert_eq!(segs[0].at, 24_000);
    assert_eq!(segs[0].src, 48_000.0, "plays from two beats in");
    assert_eq!(segs[0].fade_in, 96, "a 2 ms declick at the cut");
    // without its sound a take is silent
    let c = sat_compile::compile_with(h.doc(), SR, BLOCK, &sat_compile::Sounds::new()).unwrap();
    assert_eq!(rms(&sat_engine::render(c.schedule, 0, 4 * 24_000, BLOCK).0), 0.0);
}

#[test]
fn armed_tracks_monitor_their_input_through_their_strip() {
    let mut s = song();
    let run = |d: &Document, play: bool| {
        let c = compile(d, SR, BLOCK).unwrap();
        let (mut e, mut h) = engine(c.schedule, 4);
        if play {
            h.send(Command::Locate(10_000_000)).unwrap();
            h.send(Command::Play).unwrap();
        }
        let input = vec![0.25f32; BLOCK];
        let (mut l, mut r) = (vec![0.0; BLOCK], vec![0.0; BLOCK]);
        for _ in 0..4 {
            e.process_with_input(&input, &input, &mut l, &mut r);
        }
        rms(&l)
    };
    assert_eq!(run(s.h.doc(), false), 0.0, "nothing armed: the input is not heard");
    s.h.apply("arm", [Op::SetTrackFlag { id: s.lead, flag: TrackFlag::Arm, on: true }]).unwrap();
    let heard = run(s.h.doc(), false);
    assert!(heard > 0.05, "auto: heard while stopped ({heard})");
    assert_eq!(run(s.h.doc(), true), 0.0, "auto: not while playing back (past the song's end)");
    s.h.apply("in", [Op::SetMonitor { id: s.lead, monitor: Monitor::In }]).unwrap();
    assert!(run(s.h.doc(), true) > 0.05, "in: always");
    // through the strip: the fader turns it down
    s.h.apply("down", [Op::SetValue { target: ParamTarget::Gain(s.lead), value: -20.0 }]).unwrap();
    assert!(run(s.h.doc(), true) < heard * 0.2);
    s.h.apply("off", [Op::SetMonitor { id: s.lead, monitor: Monitor::Off }]).unwrap();
    assert_eq!(run(s.h.doc(), false), 0.0);
}
