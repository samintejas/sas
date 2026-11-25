use sat_doc::sat_time::Ticks;
use sat_doc::*;

fn take(audio: AudioId, n: u32) -> Take {
    Take { audio, name: format!("take {n}"), offset: 0 }
}

const L: Ticks = Ticks(1000);

fn starts(a: &AudioClip) -> Vec<(i64, u16)> {
    a.comp.iter().map(|s| (s.start.0, s.take)).collect()
}

#[test]
fn comping_swaps_a_range_and_merges_neighbours() {
    let id = AudioId::new();
    let mut a = AudioClip::single(take(id, 1));
    a.takes.push_back(take(id, 2));
    a.takes.push_back(take(id, 3));
    let b = a.comped(Ticks(100), Ticks(200), 1, L);
    assert_eq!(starts(&b), vec![(0, 0), (100, 1), (200, 0)]);
    assert_eq!(b.take_at(Ticks(150)), Some(1));
    assert_eq!(b.take_at(Ticks(250)), Some(0));
    // a range over a boundary keeps what follows it
    let c = b.comped(Ticks(150), Ticks(300), 2, L);
    assert_eq!(starts(&c), vec![(0, 0), (100, 1), (150, 2), (300, 0)]);
    // dragged right to left, and covering everything
    let d = c.comped(Ticks(400), Ticks(0), 1, L);
    assert_eq!(starts(&d), vec![(0, 1), (400, 0)]);
    // back to the take around it merges the segments
    let e = b.comped(Ticks(100), Ticks(200), 0, L);
    assert_eq!(starts(&e), vec![(0, 0)]);
    assert_eq!(a.comped(Ticks(5), Ticks(5), 1, L), a, "an empty range changes nothing");
    assert_eq!(a.comped(Ticks(0), Ticks(9), 7, L), a, "no such take");
    // a range to the clip end leaves nothing after it
    assert_eq!(starts(&a.comped(Ticks(500), Ticks(1200), 2, L)), vec![(0, 0), (500, 2)]);
}

#[test]
fn audio_clips_are_validated_saved_and_keep_their_sounds() {
    let mut h = History::new(Document::new(1), 10);
    let (t, op) = Op::add_track("vox", None, 0);
    let clip = ClipId::new();
    let audio = AudioId::new();
    let file =
        AudioFile { id: audio, name: "v".into(), file: "v.wav".into(), rate: 48_000, channels: 1, frames: 96_000 };
    h.apply(
        "clip",
        [
            op,
            Op::AddClip {
                id: clip,
                track: t,
                name: "take".into(),
                color: Color(4),
                length: Ticks::quarters(8),
                looped: false,
                seed: 1,
            },
        ],
    )
    .unwrap();
    let a = AudioClip::single(take(audio, 1));
    assert!(h.apply("no sound", [Op::SetClipAudio { clip, audio: Some(a.clone()) }]).is_err());
    h.apply("sound", [Op::AddAudio { file }, Op::SetClipAudio { clip, audio: Some(a.clone()) }]).unwrap();
    let mut bad = a.clone();
    bad.comp.push_back(CompSegment { start: Ticks(10), take: 3 });
    assert!(h.apply("bad take", [Op::SetClipAudio { clip, audio: Some(bad) }]).is_err());
    let mut bad = a.clone();
    bad.comp[0].start = Ticks(5);
    assert!(h.apply("bad start", [Op::SetClipAudio { clip, audio: Some(bad) }]).is_err());
    assert!(h.apply("in use", [Op::RemoveAudio { id: audio }]).is_err(), "a take still plays it");
    let bytes = file::to_bytes(h.doc()).unwrap();
    assert_eq!(file::from_bytes(&bytes).unwrap().clips[&clip].audio, Some(a));
}
