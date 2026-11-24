//! Which notes a track's instrument starts, sample by sample: the
//! arrangement's notes, or a session clip launched over them.
//!
//! A launched clip starts at a timeline sample (the app quantises it) and
//! loops on its own length from there, so it keeps its phase across
//! transport loops that are a multiple of its length. Once a clip has been
//! launched the track follows the session, even after the clip stops, until
//! [`crate::Command::BackToArrangement`].

use crate::synth::NoteEvent;

/// A clip a track can launch: its notes relative to the clip start.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionClip {
    /// Stable key (from the document's clip id), so launches survive recompiles.
    pub key: u64,
    /// Notes, `at` from 0 to `len`, sorted here.
    pub events: Vec<NoteEvent>,
    /// Length in samples (at least 1).
    pub len: i64,
    /// Loops until stopped; otherwise plays once.
    pub looped: bool,
}

/// What happens at a sample.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Cue<'a> {
    /// A launch or stop took effect: cut the sounding notes (comes first).
    Cut,
    /// A note starts.
    Start(&'a NoteEvent),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Launch {
    Clip(u64),
    Stop,
}

/// The note source of an instrument node.
#[derive(Debug, Clone, Default)]
pub(crate) struct Sequencer {
    events: Vec<NoteEvent>,
    cursor: usize,
    clips: Vec<SessionClip>,
    /// The session has taken over from the arrangement.
    session: bool,
    /// The playing clip (index) and the sample it started at.
    playing: Option<(usize, i64)>,
    /// Next event of the playing clip, and the clip position it is for.
    ccur: usize,
    crel: Option<i64>,
    pending: Option<(Launch, i64)>,
    /// The sample the next block should start at; anything else is a jump.
    expect: Option<i64>,
}

fn sort(events: &mut [NoteEvent]) {
    events.sort_by(|a, b| a.at.cmp(&b.at).then(a.pitch.cmp(&b.pitch)));
}

impl Sequencer {
    pub(crate) fn new(mut events: Vec<NoteEvent>) -> Sequencer {
        sort(&mut events);
        Sequencer { events, ..Sequencer::default() }
    }

    pub(crate) fn set_clips(&mut self, mut clips: Vec<SessionClip>) {
        for c in &mut clips {
            c.len = c.len.max(1);
            sort(&mut c.events);
        }
        self.clips = clips;
    }

    /// Carries the session state over from the node this one replaces.
    pub(crate) fn adopt(&mut self, old: &Sequencer) {
        self.session = old.session;
        self.playing = old.playing.and_then(|(i, s)| {
            let key = old.clips.get(i)?.key;
            Some((self.clips.iter().position(|c| c.key == key)?, s))
        });
        self.pending = old.pending.filter(|(l, _)| match l {
            Launch::Clip(k) => self.clips.iter().any(|c| c.key == *k),
            Launch::Stop => true,
        });
        self.expect = None;
        self.crel = None;
    }

    /// Queues clip `key` (or a stop, with `None`) for sample `at`. Returns
    /// false if the node has no such clip.
    pub(crate) fn launch(&mut self, key: Option<u64>, at: i64) -> bool {
        let l = match key {
            Some(k) if self.clips.iter().any(|c| c.key == k) => Launch::Clip(k),
            Some(_) => return false,
            None => Launch::Stop,
        };
        self.pending = Some((l, at));
        true
    }

    /// Hands the track back to the arrangement. Returns true if notes
    /// should be cut.
    pub(crate) fn back_to_arrangement(&mut self) -> bool {
        let was = self.session;
        self.session = false;
        self.playing = None;
        self.pending = None;
        self.expect = None;
        was
    }

    /// Forgets where the transport was, so the next block reseeks.
    pub(crate) fn invalidate(&mut self) {
        self.expect = None;
        self.crel = None;
    }

    /// The key of the playing clip, if any.
    pub fn playing(&self) -> Option<u64> {
        self.playing.and_then(|(i, _)| self.clips.get(i)).map(|c| c.key)
    }

    /// Starts a block at `pos` of `len` frames. Returns true on a jump
    /// (locate or loop), when sounding notes should be cut.
    pub(crate) fn begin(&mut self, pos: Option<i64>, len: usize) -> bool {
        let mut jumped = false;
        if let Some(t0) = pos
            && self.expect != Some(t0)
        {
            jumped = self.expect.is_some();
            self.cursor = self.events.partition_point(|e| e.at < t0);
            self.crel = None;
        }
        self.expect = pos.map(|p| p + len as i64);
        jumped
    }

    /// Calls `cue` with what happens at sample `t`: a cut when a launch or
    /// stop takes effect, then every note that starts.
    pub(crate) fn step(&mut self, t: i64, mut cue: impl FnMut(Cue<'_>)) {
        if let Some((l, at)) = self.pending
            && t >= at
        {
            self.pending = None;
            self.session = true;
            cue(Cue::Cut);
            self.playing = match l {
                Launch::Clip(k) => self.clips.iter().position(|c| c.key == k).map(|i| (i, at)),
                Launch::Stop => None,
            };
            self.crel = None;
        }
        if !self.session {
            while let Some(e) = self.events.get(self.cursor) {
                if e.at > t {
                    break;
                }
                if e.at == t {
                    cue(Cue::Start(e));
                }
                self.cursor += 1;
            }
            return;
        }
        let Some((i, s0)) = self.playing else { return };
        let c = &self.clips[i];
        let mut rel = t - s0;
        if c.looped {
            rel = rel.rem_euclid(c.len);
        } else if !(0..c.len).contains(&rel) {
            if rel >= c.len {
                self.playing = None;
            }
            return;
        }
        if self.crel != Some(rel) {
            self.ccur = c.events.partition_point(|e| e.at < rel);
        }
        while let Some(e) = c.events.get(self.ccur) {
            if e.at != rel {
                break;
            }
            cue(Cue::Start(e));
            self.ccur += 1;
        }
        self.crel = Some(if c.looped && rel + 1 == c.len { 0 } else { rel + 1 });
        if self.crel == Some(0) {
            self.ccur = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(at: i64, pitch: u8) -> NoteEvent {
        NoteEvent { at, len: 10, pitch, velocity: 1.0 }
    }

    /// Runs samples `from..to` in blocks of 7 and collects (sample, pitch) starts.
    fn run(s: &mut Sequencer, from: i64, to: i64) -> Vec<(i64, u8)> {
        let mut out = Vec::new();
        let mut t = from;
        while t < to {
            let n = 7.min(to - t) as usize;
            s.begin(Some(t), n);
            for k in 0..n as i64 {
                s.step(t + k, |c| {
                    if let Cue::Start(e) = c {
                        out.push((t + k, e.pitch))
                    }
                });
            }
            t += n as i64;
        }
        out
    }

    #[test]
    fn clips_launch_on_time_loop_and_stop() {
        let mut s = Sequencer::new(vec![ev(5, 60), ev(50, 61), ev(120, 62)]);
        s.set_clips(vec![
            SessionClip { key: 1, events: vec![ev(0, 70), ev(10, 71)], len: 20, looped: true },
            SessionClip { key: 2, events: vec![ev(3, 80)], len: 30, looped: false },
        ]);
        assert_eq!(run(&mut s, 0, 40), vec![(5, 60)], "the arrangement plays");
        assert!(s.launch(Some(1), 45));
        assert!(!s.launch(Some(9), 45), "unknown clip");
        assert!(s.launch(Some(1), 45));
        // the arrangement note at 50 is replaced by the clip from 45 on
        assert_eq!(run(&mut s, 40, 90), vec![(45, 70), (55, 71), (65, 70), (75, 71), (85, 70)]);
        assert_eq!(s.playing(), Some(1));
        s.launch(Some(2), 100);
        assert_eq!(run(&mut s, 90, 200), vec![(95, 71), (103, 80)], "a one-shot plays once");
        assert_eq!(s.playing(), None);
        // stopped: the session still owns the track until handed back
        assert!(run(&mut s, 200, 300).is_empty());
        assert!(s.back_to_arrangement());
        assert_eq!(run(&mut s, 100, 130), vec![(120, 62)]);
    }

    #[test]
    fn launches_survive_recompiles_and_jumps() {
        let mut s = Sequencer::new(vec![]);
        let clip = SessionClip { key: 7, events: vec![ev(0, 70)], len: 16, looped: true };
        s.set_clips(vec![clip.clone()]);
        s.launch(Some(7), 0);
        run(&mut s, 0, 20);
        let mut next = Sequencer::new(vec![]);
        next.set_clips(vec![SessionClip { key: 3, events: vec![], len: 4, looped: true }, clip]);
        next.adopt(&s);
        assert_eq!(next.playing(), Some(7));
        assert_eq!(run(&mut next, 20, 40), vec![(32, 70)], "same phase after the swap");
        // a jump back keeps the clip's phase
        assert_eq!(run(&mut next, 8, 20), vec![(16, 70)]);
    }
}
