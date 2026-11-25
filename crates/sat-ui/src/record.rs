//! Turning a recording into takes.
//!
//! A recording is the input captured from the moment record was pressed.
//! With the loop on, the transport wraps while recording, so the capture is
//! split into passes: the first from where it started to the loop end, then
//! one per time round the loop. Every pass becomes a take of an audio clip
//! on each armed track: the clip it overlaps there, else a new one (the
//! loop range when looping, else the pass itself). A take's sound covers
//! its whole clip, silent where the pass did not reach, so takes line up
//! frame for frame, and the comp plays the new take where it was recorded.

use std::collections::HashMap;

use sat_doc::sat_time::{TempoMap, Ticks};
use sat_doc::*;

/// Passes shorter than this are dropped (a stray click of the record button).
pub const MIN_PASS_SECS: f64 = 0.25;

/// One pass of a recording.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pass {
    /// Timeline seconds where it starts.
    pub at: f64,
    /// Seconds into the recording where it starts.
    pub from: f64,
    /// Length in seconds.
    pub len: f64,
}

/// Splits a recording of `len` seconds that started at timeline second
/// `start` into passes, wrapping at the loop (`(start, end)` seconds) if
/// the transport loops.
pub fn passes(start: f64, len: f64, looping: Option<(f64, f64)>) -> Vec<Pass> {
    let mut out = Vec::new();
    match looping {
        Some((a, b)) if b > a && start < b && start + len > b => {
            let mut at = start;
            let mut from = 0.0;
            while from < len {
                let n = (b - at).min(len - from);
                out.push(Pass { at, from, len: n });
                from += n;
                at = a;
            }
        }
        _ => out.push(Pass { at: start, from: 0.0, len }),
    }
    out.retain(|p| p.len >= MIN_PASS_SECS);
    out
}

fn tick(tempo: &TempoMap, secs: f64) -> Ticks {
    Ticks(tempo.secs_to_tick(secs).round() as i64)
}

/// The edits that add `rec` (captured from timeline second `start`) as takes
/// on `tracks`. The takes' sounds go into `pool` (named after the track
/// and take); the edits add them to the document.
pub fn record(
    doc: &Document,
    pool: &mut sat_disk::Pool,
    tracks: &[TrackId],
    rec: &sat_disk::Sample,
    start: f64,
    looping: Option<(f64, f64)>,
) -> Vec<Op> {
    let Ok(tempo) = doc.tempo_map() else { return Vec::new() };
    let rate = rec.rate as f64;
    let len = rec.frames() as f64 / rate;
    let mut ops = Vec::new();
    for &t in tracks {
        let Some(track) = doc.tracks.get(&t).filter(|tr| tr.kind == TrackKind::Track) else { continue };
        // clips this recording has touched: their audio so far, origin and length
        let mut touched: HashMap<ClipId, (AudioClip, Ticks, Ticks)> = HashMap::new();
        for p in passes(start, len, looping) {
            let (t0, t1) = (tick(&tempo, p.at), tick(&tempo, p.at + p.len));
            // the clip to record into, made if there is none
            let found =
                touched.iter().find(|(_, (_, o, l))| *o < t1 && *o + *l > t0).map(|(c, (_, o, l))| (*c, *o, *l));
            let found = found.or_else(|| {
                doc.placements.values().find_map(|pl| {
                    let c = doc.clips.get(&pl.clip).filter(|c| c.track == t && c.audio.is_some())?;
                    (pl.start < t1 && pl.start + pl.length > t0).then_some((c.id, pl.start - pl.offset, c.length))
                })
            });
            let (clip, origin, clip_len) = match found {
                Some(f) => f,
                None => {
                    let (o, e) = match looping {
                        Some((a, b)) if p.at >= a - 1e-9 && p.at + p.len <= b + 1e-9 => {
                            (tick(&tempo, a), tick(&tempo, b))
                        }
                        _ => (t0, t1),
                    };
                    let id = ClipId::new();
                    ops.push(Op::AddClip {
                        id,
                        track: t,
                        name: track.name.clone(),
                        color: track.color,
                        length: e - o,
                        looped: false,
                        seed: 1,
                    });
                    ops.push(Op::AddPlacement {
                        id: PlacementId::new(),
                        clip: id,
                        start: o,
                        offset: Ticks::ZERO,
                        length: e - o,
                    });
                    (id, o, e - o)
                }
            };
            let audio = touched
                .get(&clip)
                .map(|(a, _, _)| a.clone())
                .or_else(|| doc.clips.get(&clip).and_then(|c| c.audio.clone()));
            let n = audio.as_ref().map_or(0, |a| a.takes.len());
            // the take's sound: the whole clip, the pass where it was sung
            let clip_secs = tempo.tick_to_secs(origin.0 as f64);
            let frames = ((tempo.tick_to_secs((origin + clip_len).0 as f64) - clip_secs) * rate).round() as usize;
            let shift = ((p.at - clip_secs) * rate).round() as i64;
            let from = (p.from * rate).round() as i64;
            let count = (p.len * rate).round() as i64;
            let (mut l, mut r) = (vec![0.0f32; frames], vec![0.0f32; frames]);
            for k in 0..count {
                let (dst, src) = (shift + k, (from + k) as usize);
                if dst >= 0 && (dst as usize) < frames && src < rec.frames() {
                    l[dst as usize] = rec.l[src];
                    r[dst as usize] = rec.r[src];
                }
            }
            let take_name = format!("take {}", n + 1);
            let file = pool.add(&format!("{} {take_name}", track.name), sat_disk::Sample { rate: rec.rate, l, r });
            let take = Take { audio: file.id, name: take_name, offset: 0 };
            let a = match audio {
                Some(mut a) => {
                    a.takes.push_back(take);
                    a.comped(t0 - origin, t1 - origin, n as u16, clip_len)
                }
                None => AudioClip::single(take),
            };
            touched.insert(clip, (a, origin, clip_len));
            ops.push(Op::AddAudio { file });
        }
        for (clip, (a, _, _)) in touched {
            ops.push(Op::SetClipAudio { clip, audio: Some(a) });
        }
    }
    ops
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recordings_split_at_the_loop() {
        assert_eq!(passes(1.0, 3.0, None), vec![Pass { at: 1.0, from: 0.0, len: 3.0 }]);
        let p = passes(3.0, 9.5, Some((2.0, 6.0)));
        assert_eq!(
            p,
            vec![
                Pass { at: 3.0, from: 0.0, len: 3.0 },
                Pass { at: 2.0, from: 3.0, len: 4.0 },
                Pass { at: 2.0, from: 7.0, len: 2.5 },
            ]
        );
        // stopped just after a wrap: the stub is dropped
        assert_eq!(passes(2.0, 4.1, Some((2.0, 6.0))).len(), 1);
        // started after the loop: it never wraps
        assert_eq!(passes(7.0, 9.0, Some((2.0, 6.0))).len(), 1);
    }
}

#[cfg(test)]
mod record_tests {
    use super::*;
    use sat_doc::History;

    /// 120 bpm: a bar is 2 s. Input at 8 kHz, each second its own level.
    fn setup() -> (History, TrackId, sat_disk::Sample) {
        let mut h = History::new(Document::new(1), 10);
        let (t, op) = Op::add_track("vox", None, 0);
        h.apply("track", [op]).unwrap();
        let l: Vec<f32> = (0..8_000 * 10).map(|i| (i / 8_000) as f32 / 10.0 + 0.05).collect();
        (h, t, sat_disk::Sample { rate: 8_000.0, r: l.clone(), l })
    }

    fn apply(h: &mut History, ops: Vec<Op>) {
        h.apply("record", ops).unwrap();
    }

    #[test]
    fn a_pass_makes_a_clip_and_a_second_one_a_take_comped_in_where_it_was_sung() {
        let (mut h, t, rec) = setup();
        let mut pool = sat_disk::Pool::new();
        // 3 s from second 1: a new clip from beat 2 to beat 8
        let first = sat_disk::Sample { rate: 8_000.0, l: rec.l[..24_000].to_vec(), r: rec.r[..24_000].to_vec() };
        let ops = record(h.doc(), &mut pool, &[t], &first, 1.0, None);
        apply(&mut h, ops);
        let c = h.doc().clips_on(t).next().unwrap().clone();
        assert_eq!(c.length, Ticks::quarters(6));
        let p = h.doc().placements.values().next().unwrap();
        assert_eq!(p.start, Ticks::quarters(2));
        let a = c.audio.clone().unwrap();
        assert_eq!(a.takes[0].name, "take 1");
        assert_eq!(pool.get(a.takes[0].audio).unwrap().frames(), 24_000);
        // a second pass over its middle second is take 2, comped in just there
        let mid = sat_disk::Sample { rate: 8_000.0, l: rec.l[..8_000].to_vec(), r: rec.r[..8_000].to_vec() };
        let ops = record(h.doc(), &mut pool, &[t], &mid, 2.0, None);
        apply(&mut h, ops);
        let a = h.doc().clips[&c.id].audio.clone().unwrap();
        assert_eq!(a.takes.len(), 2);
        let starts: Vec<(i64, u16)> = a.comp.iter().map(|s| (s.start.0, s.take)).collect();
        assert_eq!(starts, vec![(0, 0), (Ticks::quarters(2).0, 1), (Ticks::quarters(4).0, 0)]);
        // its sound lines up with the clip: silent for the first second
        let s = pool.get(a.takes[1].audio).unwrap();
        assert_eq!((s.frames(), s.l[7_999], s.l[8_000], s.l[16_000]), (24_000, 0.0, 0.05, 0.0));
    }

    #[test]
    fn loop_recording_stacks_one_take_per_pass() {
        let (mut h, t, rec) = setup();
        h.apply(
            "loop",
            [Op::SetLoop { range: LoopRange { start: Ticks::quarters(4), end: Ticks::quarters(12), enabled: true } }],
        )
        .unwrap();
        let mut pool = sat_disk::Pool::new();
        // from second 3 (mid loop) for 10 s: 3 s, then 4 s, then 3 s passes
        let ops = record(h.doc(), &mut pool, &[t], &rec, 3.0, Some((2.0, 6.0)));
        apply(&mut h, ops);
        let clips: Vec<_> = h.doc().clips_on(t).collect();
        assert_eq!(clips.len(), 1, "one clip over the loop");
        assert_eq!(clips[0].length, Ticks::quarters(8));
        let a = clips[0].audio.clone().unwrap();
        assert_eq!(a.takes.len(), 3);
        // the last pass ran out a second before the loop end, so take 2 plays there
        let starts: Vec<(i64, u16)> = a.comp.iter().map(|s| (s.start.0, s.take)).collect();
        assert_eq!(starts, vec![(0, 2), (Ticks::quarters(6).0, 1)]);
        // take 1 began a second into the loop
        let s1 = pool.get(a.takes[0].audio).unwrap();
        assert_eq!((s1.l[7_999], s1.l[8_000]), (0.0, 0.05));
        let s2 = pool.get(a.takes[1].audio).unwrap();
        assert!((s2.l[0] - 0.35).abs() < 1e-6, "take 2 is seconds 3 to 7 of the recording");
        assert!(record(h.doc(), &mut pool, &[], &rec, 0.0, None).is_empty());
    }
}
