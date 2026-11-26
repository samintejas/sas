//! The session: clips in slots, launched from the clip launcher beside the
//! arrangement (a column per scene, a slot per track row). Launching is
//! quantised to the next bar; a track that has launched a clip follows the
//! session, overriding its arrangement clips (its automation still plays)
//! until the set goes back to the arrangement.

use std::collections::{HashMap, HashSet};

use sat_doc::sat_time::Ticks;
use sat_doc::*;
use sat_theme::{Rgba, Theme};
use vello::kurbo::Rect;

use crate::arrange::TransportView;
use crate::icons::{self, Icon};
use crate::paint::{Painter, TextStyle};

/// Width of a cell's launch triangle area.
pub const PLAY_W: f64 = 22.0;

/// What the session is doing, as the app has asked the engine.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SessionView {
    /// The clip each track is playing.
    pub playing: HashMap<TrackId, ClipId>,
    /// Launches (a clip, or a stop) waiting for their bar.
    pub queued: HashMap<TrackId, (Option<ClipId>, Ticks)>,
    /// Tracks whose clips come from the launcher instead of the arrangement.
    pub following: HashSet<TrackId>,
    /// Tracks whose automation comes from the launcher (the arrangement's
    /// lanes are ignored) instead of the arrangement.
    pub auto: HashSet<TrackId>,
    /// When each playing clip started.
    pub started: HashMap<TrackId, Ticks>,
}

impl SessionView {
    /// Promotes launches whose bar has come; returns true if anything changed.
    /// A one-shot clip that has played out stops.
    pub fn advance(&mut self, doc: &Document, playhead: Ticks) -> bool {
        let due: Vec<TrackId> = self.queued.iter().filter(|(_, (_, at))| *at <= playhead).map(|(t, _)| *t).collect();
        let mut changed = !due.is_empty();
        for t in due {
            let (clip, at) = self.queued.remove(&t).unwrap();
            self.following.insert(t);
            match clip {
                Some(c) => {
                    self.playing.insert(t, c);
                    self.started.insert(t, at);
                }
                None => {
                    self.playing.remove(&t);
                }
            }
        }
        let ended: Vec<TrackId> = self
            .playing
            .iter()
            .filter(|(t, c)| {
                let Some(clip) = doc.clips.get(c) else { return true };
                !clip.looped && self.started.get(t).is_some_and(|s| playhead >= *s + clip.length)
            })
            .map(|(t, _)| *t)
            .collect();
        changed |= !ended.is_empty();
        for t in ended {
            self.playing.remove(&t);
        }
        changed
    }

    /// Whether any track takes its clips or automation from the launcher.
    pub fn any_launcher(&self) -> bool {
        !self.following.is_empty() || !self.auto.is_empty() || !self.queued.is_empty()
    }

    /// Back to the arrangement: nothing plays or waits.
    pub fn clear(&mut self) {
        self.playing.clear();
        self.queued.clear();
        self.following.clear();
        self.auto.clear();
        self.started.clear();
    }
}

/// The tick a launch made at `playhead` waits for: the next bar line, kept
/// inside the loop when it is on (a bar past the loop end is the loop start).
pub fn launch_tick(doc: &Document, playhead: Ticks, playing: bool) -> Ticks {
    if !playing {
        return playhead;
    }
    let Ok(m) = doc.meter_map() else { return playhead };
    let next = m.bar_start(m.bar_beat(playhead).bar + 1);
    let l = doc.loop_range;
    if l.enabled && playhead < l.end && next >= l.end { l.start } else { next }
}

/// Where "back to arrangement" sits in the arrange toolbar of the main
/// area `r`, left of the snap control (shown while any track takes its
/// clips or automation from the launcher): the global switch back.
pub fn back_button(r: Rect) -> Rect {
    let s = crate::arrange::snap_button(r);
    Rect::new(s.x0 - 52.0 - s.height() * 1.4, s.y0, s.x0 - 52.0, s.y1)
}

/// Draws "back to arrangement" in the main area `r`.
pub fn draw_back(p: &mut Painter<'_>, th: &Theme, r: Rect) {
    let c = &th.c;
    let b = back_button(r);
    p.rrect(b, 4.0, c.bg_base);
    p.rrect(b, 4.0, c.accent_primary.alpha(0.18));
    p.rrect_stroke(b.inset(-0.5), 4.0, 1.0, c.accent_primary);
    crate::icons::draw(p, crate::icons::Icon::BackToArrangement, b.inset(-4.0), c.accent_primary);
}

/// What a track takes from the launcher or the arrangement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Part {
    /// Its notes and audio.
    Clips,
    /// Its automation.
    Automation,
}

/// What is under a point in the clip launcher.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SessionHit {
    /// A scene's launch cell.
    Scene(SceneId),
    /// A clip's launch triangle.
    Launch(TrackId, ClipId),
    /// A clip's body: select it.
    Select(ClipId),
    /// A group's cell in a scene: launch its tracks' clips there.
    Group(TrackId, SceneId),
    /// An empty slot or a stop button: stop that track (or group).
    Stop(TrackId),
    /// Stop every clip.
    StopAll,
    /// Every track plays the arrangement again.
    BackToArrangement,
    /// Add a scene.
    AddScene,
    /// Nothing.
    Nothing,
}

/// The tracks a column launches: itself, or a group's tracks (all depths).
pub fn tracks_of(doc: &Document, col: TrackId) -> Vec<TrackId> {
    match doc.tracks.get(&col) {
        Some(t) if t.kind == TrackKind::Group => doc
            .tracks_in_order()
            .into_iter()
            .filter(|x| *x != col && doc.tracks[x].kind == TrackKind::Track && doc.is_within(*x, col))
            .collect(),
        Some(_) => vec![col],
        None => Vec::new(),
    }
}

/// One session slot to draw: its track, the clip in it, whether the track
/// follows the session and the queued-launch blink phase.
#[derive(Debug, Clone, Copy)]
pub struct Slot<'d> {
    /// The track.
    pub track: TrackId,
    /// The clip in the slot.
    pub clip: Option<&'d Clip>,
    /// The track follows the session.
    pub follows: bool,
    /// Blink phase for queued launches.
    pub blink: bool,
}

/// Draws a session slot in `cell`: a clip with its launch triangle, play
/// progress and queued/selected outlines, or an empty slot's stop square.
/// The session view and the launcher beside the arrangement both use it.
pub fn draw_slot(
    p: &mut Painter<'_>,
    th: &Theme,
    view: &SessionView,
    selected_clip: Option<ClipId>,
    transport: &TransportView,
    slot: &Slot<'_>,
    cell: Rect,
) {
    let c = &th.c;
    let (track, blink, follows) = (slot.track, slot.blink, slot.follows);
    match slot.clip {
        Some(clip) => {
            let ccol = th.track(clip.color.0);
            let playing = view.playing.get(&track) == Some(&clip.id);
            let queued = view.queued.get(&track).is_some_and(|(q, _)| *q == Some(clip.id));
            let sel = selected_clip == Some(clip.id);
            p.rrect(cell, 1.0, if playing { ccol.alpha(0.85) } else { ccol.alpha(0.28) });
            let play_w = PLAY_W.min(cell.height().max(16.0));
            let play_r = Rect::new(cell.x0, cell.y0, cell.x0 + play_w, cell.y1);
            p.rrect(play_r, 1.0, Rgba::new(0, 0, 0, if playing { 60 } else { 40 }));
            let tri = Rect::new(
                play_r.center().x - 5.0,
                cell.center().y - 5.0,
                play_r.center().x + 5.0,
                cell.center().y + 5.0,
            );
            let tcol = if playing {
                c.text_max
            } else if queued && blink {
                c.accent_primary
            } else {
                c.text_secondary
            };
            icons::draw(p, Icon::Play, tri, tcol);
            let ink = if playing { Rgba::hex(0x0b0b0b) } else { c.text_strong };
            // a clip carrying automation shows a small envelope at its right
            let env_w = if clip.envelopes.is_empty() { 0.0 } else { 16.0 };
            p.text_fit(
                cell.x0 + play_w + 6.0,
                cell.center().y,
                &clip.name,
                &TextStyle::mono(11.0, ink),
                cell.width() - play_w - 10.0 - env_w,
            );
            if env_w > 0.0 {
                let r = Rect::new(cell.x1 - 17.0, cell.center().y - 6.0, cell.x1 - 5.0, cell.center().y + 6.0);
                icons::draw(p, Icon::Envelope, r, ink.alpha(0.8));
            }
            if playing && let Some(start) = view.started.get(&track) {
                // how far through its loop the clip is
                let len = clip.length.0.max(1) as f64;
                let f = ((transport.playhead.0 - start.0).max(0) as f64 % len) / len;
                let bar = Rect::new(
                    cell.x0 + play_w,
                    cell.y1 - 3.0,
                    cell.x0 + play_w + (cell.width() - play_w) * f,
                    cell.y1 - 1.0,
                );
                p.rect(bar, Rgba::hex(0x0b0b0b).alpha(0.6));
            }
            if queued {
                p.rrect_stroke(cell.inset(-0.5), 3.0, 1.0, if blink { c.accent_primary } else { c.border_default });
            }
            if sel {
                p.rrect_stroke(cell.inset(-0.5), 1.0, 1.5, c.text_max);
            }
        }
        None => {
            p.rrect(cell, 1.0, c.bg_surface.alpha(0.45));
            // empty slots stop the track
            let sq =
                Rect::new(cell.center().x - 3.5, cell.center().y - 3.5, cell.center().x + 3.5, cell.center().y + 3.5);
            p.rect(sq, if follows { c.text_secondary.alpha(0.7) } else { c.border_default });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launches_wait_for_the_next_bar_inside_the_loop() {
        let mut doc = Document::new(1);
        let bar = Ticks::quarters(4);
        assert_eq!(launch_tick(&doc, Ticks(10), true), bar);
        assert_eq!(launch_tick(&doc, Ticks(10), false), Ticks(10), "stopped: now");
        doc.loop_range = LoopRange { start: bar, end: Ticks(bar.0 * 3), enabled: true };
        assert_eq!(launch_tick(&doc, Ticks(bar.0 * 2 + 5), true), bar, "wraps to the loop start");
    }

    #[test]
    fn queued_launches_play_on_their_bar() {
        let d = crate::demo::demo();
        let doc = &d.doc;
        let mut v = SessionView::default();
        let s1 = doc.scenes[0].id;
        let drums = doc.tracks.values().find(|t| t.name == "drums").unwrap().id;
        let clip = doc.slots[&(s1, drums)];
        v.queued.insert(drums, (Some(clip), Ticks::quarters(4)));
        assert!(!v.advance(doc, Ticks::quarters(2)));
        assert!(v.advance(doc, Ticks::quarters(4)));
        assert_eq!(v.playing.get(&drums), Some(&clip));
        assert!(v.following.contains(&drums));
        let rhythm = doc.tracks.values().find(|t| t.kind == TrackKind::Group).unwrap().id;
        assert!(tracks_of(doc, rhythm).contains(&drums));
        v.clear();
        assert!(v.playing.is_empty() && v.following.is_empty());
    }
}
