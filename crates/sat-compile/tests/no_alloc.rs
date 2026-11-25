//! Real-time safety: the audio thread never allocates.
//!
//! This binary installs an allocator that aborts if anything allocates inside
//! `assert_no_alloc`, then drives the engine through playback, parameter
//! changes, locate, loop, a schedule swap and stop.

mod common;

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use common::*;
use sat_doc::sat_time::Ticks;
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
    let clip = s.h.doc().clips_on(s.lead).next().unwrap().id;
    let launch = c.launch(s.lead, Some(clip), 20_000).unwrap();
    let stop = c.launch(s.bass, None, 40_000).unwrap();
    let (mut e, mut h) = engine(c.schedule, 64);
    let (_, op) = Op::add_track("extra", None, 9);
    s.h.apply("add", [op]).unwrap();
    let next = Box::new(sat_compile::compile(s.h.doc(), SR, BLOCK).unwrap().schedule);

    let mut l = vec![0.0f32; BLOCK];
    let mut r = vec![0.0f32; BLOCK];
    h.send(Command::Play).unwrap();
    h.send(gain).unwrap();
    h.send(send).unwrap();
    h.send(launch).unwrap();
    h.send(stop).unwrap();
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
    h.send(Command::BackToArrangement).unwrap();
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
fn instrument_patches_drums_and_samplers_do_not_allocate() {
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
    // a looping sampler on the lead
    let lgrid = s.h.doc().tracks[&s.lead].instrument_grid.unwrap();
    let (smp, op) = sat_modules::add_module(s.h.doc(), lgrid, ins::SAMPLER, (0.0, 0.0));
    let audio = AudioId::new();
    let file = AudioFile { id: audio, name: "s".into(), file: "s.wav".into(), rate: 44_100, channels: 1, frames: 4410 };
    s.h.apply("sampler", [op, Op::AddAudio { file }, Op::SetModuleAudio { module: smp, audio: Some(audio) }]).unwrap();
    s.h.apply("loop", [Op::SetValue { target: ParamTarget::Module { module: smp, param: "mode".into() }, value: 2.0 }])
        .unwrap();
    let tone: Vec<f32> = (0..4410).map(|i| (i as f32 * 0.05).sin()).collect();
    let mut sounds = sat_compile::Sounds::new();
    sounds.insert(audio, std::sync::Arc::new(sat_engine::Sample::from_interleaved(44_100.0, 1, &tone)));
    // an audio clip of two comped takes on the lead
    let clip = ClipId::new();
    let take = |n: u32| Take { audio, name: format!("take {n}"), offset: 0 };
    let mut ac = AudioClip::single(take(1));
    ac.takes.push_back(take(2));
    let ac = ac.comped(Ticks::sixteenths(2), Ticks::sixteenths(5), 1, Ticks::quarters(2));
    s.h.apply(
        "audio clip",
        [
            Op::AddClip {
                id: clip,
                track: s.lead,
                name: "vox".into(),
                color: Color(4),
                length: Ticks::quarters(2),
                looped: false,
                seed: 1,
            },
            Op::SetClipAudio { clip, audio: Some(ac) },
            Op::AddPlacement {
                id: PlacementId::new(),
                clip,
                start: Ticks::ZERO,
                offset: Ticks::ZERO,
                length: Ticks::quarters(2),
            },
        ],
    )
    .unwrap();
    // the lead monitors its input
    s.h.apply("monitor", [Op::SetMonitor { id: s.lead, monitor: Monitor::In }]).unwrap();
    // and a full fx chain on the bass
    let fx_grid = s.h.doc().tracks[&s.bass].fx_grids[0];
    use sat_modules::fx;
    for k in [fx::EQ, fx::COMP, fx::DRIVE, fx::FX_FILTER, fx::DELAY, fx::REVERB, fx::LIMITER] {
        let (_, ops) = fx::insert(s.h.doc(), fx_grid, k);
        s.h.apply("fx", ops).unwrap();
    }
    assert_eq!(sat_compile::build_fx(s.h.doc(), fx_grid).unwrap().0.len(), 9);
    let c = sat_compile::compile_with(s.h.doc(), SR, BLOCK, &sounds).unwrap();
    let cutoff = c.param_command(&ParamTarget::Module { module: ids[2], param: "cutoff".into() }, 300.0).unwrap();
    let (mut e, mut h) = engine(c.schedule, 64);
    // a knob change that recompiles (a new cable) swaps in a schedule that adopts the voices
    let ops = sat_modules::patch(s.h.doc(), &PortRef::new(ids[0], "vel"), &PortRef::new(ids[1], "fm"), None).unwrap();
    s.h.apply("patch", ops).unwrap();
    let next = Box::new(sat_compile::compile_with(s.h.doc(), SR, BLOCK, &sounds).unwrap().schedule);
    let mut l = vec![0.0f32; BLOCK];
    let mut r = vec![0.0f32; BLOCK];
    h.send(Command::Play).unwrap();
    h.send(cutoff).unwrap();
    h.send(Command::SetRecording(true)).unwrap();
    let input = vec![0.1f32; BLOCK];
    assert_no_alloc(|| {
        for _ in 0..400 {
            e.process_with_input(&input, &input, &mut l, &mut r);
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
