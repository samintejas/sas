//! The session view: a clip launcher. Scenes are rows, tracks (and groups)
//! are columns; a cell holds the clip in that session slot. Launching is
//! quantised to the next bar; a track that has launched a clip follows the
//! session until the set goes back to the arrangement.

use std::collections::{HashMap, HashSet};

use sat_doc::sat_time::Ticks;
use sat_doc::*;
use sat_theme::{Rgba, Theme};
use vello::kurbo::{Point, Rect};

use crate::arrange::TransportView;
use crate::icons::{self, Icon};
use crate::paint::{Align, Painter, TextStyle};

/// Width of the scene column.
pub const SCENE_W: f64 = 132.0;
/// Width of a track column.
pub const COL_W: f64 = 112.0;
/// Height of the header row.
pub const HEAD_H: f64 = 34.0;
/// Height of a scene row.
pub const ROW_H: f64 = 30.0;
/// Gap between cells.
pub const GAP: f64 = 4.0;
/// Width of a cell's launch triangle area.
pub const PLAY_W: f64 = 22.0;

/// What the session is doing, as the app has asked the engine.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SessionView {
    /// The clip each track is playing.
    pub playing: HashMap<TrackId, ClipId>,
    /// Launches (a clip, or a stop) waiting for their bar.
    pub queued: HashMap<TrackId, (Option<ClipId>, Ticks)>,
    /// Tracks following the session instead of the arrangement.
    pub following: HashSet<TrackId>,
    /// When each playing clip started.
    pub started: HashMap<TrackId, Ticks>,
    /// Horizontal scroll, in px.
    pub scroll: f64,
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

    /// Back to the arrangement: nothing plays or waits.
    pub fn clear(&mut self) {
        self.playing.clear();
        self.queued.clear();
        self.following.clear();
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

/// Where "back to arrangement" sits in the main area `r` (both views show
/// it while some track follows the session).
pub fn back_button(r: Rect) -> Rect {
    Rect::new(r.x1 - 196.0, r.y1 - 40.0, r.x1 - 12.0, r.y1 - 12.0)
}

/// Draws "back to arrangement" in the main area `r`.
pub fn draw_back(p: &mut Painter<'_>, th: &Theme, r: Rect) {
    let c = &th.c;
    let b = back_button(r);
    p.rrect(b, 5.0, c.bg_base);
    p.rrect(b, 5.0, c.accent_primary.alpha(0.18));
    p.rrect_stroke(b.inset(-0.5), 5.0, 1.0, c.accent_primary);
    p.text(b.center().x, b.center().y, "back to arrangement", &TextStyle::mono(11.5, c.accent_primary), Align::Center);
}

/// A column: a track or a group.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Column {
    /// The track or group.
    pub track: TrackId,
    /// Its header.
    pub head: Rect,
    /// Whether it is a group (its cells launch its tracks).
    pub group: bool,
}

/// What is under a point in the session view.
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
    /// A column header: select the track.
    Track(TrackId),
    /// Stop every clip.
    StopAll,
    /// Every track plays the arrangement again.
    BackToArrangement,
    /// Nothing.
    Nothing,
}

/// The session view's geometry.
#[derive(Debug, Clone)]
pub struct SessionLayout {
    /// The whole view.
    pub area: Rect,
    /// Columns, left to right.
    pub cols: Vec<Column>,
    /// Scene rows: id and the scene cell.
    pub scenes: Vec<(SceneId, Rect)>,
    /// The stop row's y range.
    pub stop_row: (f64, f64),
    /// "stop all".
    pub stop_all: Rect,
    /// "back to arrangement" (shown when some track follows the session).
    pub back: Rect,
}

impl SessionLayout {
    /// Lays out the session view of `doc` in `r`.
    pub fn new(doc: &Document, view: &SessionView, r: Rect) -> SessionLayout {
        let x0 = r.x0 + 8.0 + SCENE_W + GAP - view.scroll;
        let cols = doc
            .tracks_in_order()
            .into_iter()
            .filter(|t| matches!(doc.tracks[t].kind, TrackKind::Track | TrackKind::Group))
            .enumerate()
            .map(|(i, t)| {
                let x = x0 + i as f64 * (COL_W + GAP);
                Column {
                    track: t,
                    head: Rect::new(x, r.y0 + 8.0, x + COL_W, r.y0 + 8.0 + HEAD_H),
                    group: doc.tracks[&t].kind == TrackKind::Group,
                }
            })
            .collect();
        let y0 = r.y0 + 8.0 + HEAD_H + GAP;
        let scenes: Vec<(SceneId, Rect)> = doc
            .scenes
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let y = y0 + i as f64 * (ROW_H + GAP);
                (s.id, Rect::new(r.x0 + 8.0, y, r.x0 + 8.0 + SCENE_W, y + ROW_H))
            })
            .collect();
        let sy = y0 + scenes.len() as f64 * (ROW_H + GAP);
        let stop_all = Rect::new(r.x0 + 8.0, sy, r.x0 + 8.0 + SCENE_W, sy + ROW_H);
        let back = back_button(r);
        SessionLayout { area: r, cols, scenes, stop_row: (sy, sy + ROW_H), stop_all, back }
    }

    /// The cell of column `col` in the row spanning `y` (top, bottom).
    pub fn cell(&self, col: &Column, row: Rect) -> Rect {
        Rect::new(col.head.x0, row.y0, col.head.x1, row.y1)
    }

    /// What is under `pt`.
    pub fn hit(&self, doc: &Document, view: &SessionView, pt: Point) -> SessionHit {
        if !view.following.is_empty() && self.back.contains(pt) {
            return SessionHit::BackToArrangement;
        }
        if self.stop_all.contains(pt) {
            return SessionHit::StopAll;
        }
        if let Some((s, _)) = self.scenes.iter().find(|(_, r)| r.contains(pt)) {
            return SessionHit::Scene(*s);
        }
        if pt.x < self.stop_all.x1 + GAP {
            return SessionHit::Nothing;
        }
        let Some(col) = self.cols.iter().find(|c| (c.head.x0..c.head.x1).contains(&pt.x)) else {
            return SessionHit::Nothing;
        };
        if col.head.contains(pt) {
            return SessionHit::Track(col.track);
        }
        if (self.stop_row.0..self.stop_row.1).contains(&pt.y) {
            return SessionHit::Stop(col.track);
        }
        let Some((scene, row)) = self.scenes.iter().find(|(_, r)| (r.y0..r.y1).contains(&pt.y)) else {
            return SessionHit::Nothing;
        };
        if col.group {
            return SessionHit::Group(col.track, *scene);
        }
        match doc.slots.get(&(*scene, col.track)) {
            Some(c) if pt.x < self.cell(col, *row).x0 + PLAY_W => SessionHit::Launch(col.track, *c),
            Some(c) => SessionHit::Select(*c),
            None => SessionHit::Stop(col.track),
        }
    }
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

/// Draws the session view and returns its layout.
pub fn draw(
    p: &mut Painter<'_>,
    th: &Theme,
    doc: &Document,
    view: &SessionView,
    selected_clip: Option<ClipId>,
    transport: &TransportView,
    r: Rect,
) -> SessionLayout {
    let l = SessionLayout::new(doc, view, r);
    let c = &th.c;
    p.rect(r, c.bg_base);
    let small = TextStyle::mono(11.0, c.text_secondary);
    let label = TextStyle::mono(11.5, c.text_strong);
    // a slow blink for queued launches, from the playhead
    let blink = (transport.playhead.0 / (sat_doc::sat_time::PPQ / 2)) % 2 == 0;
    let cells = Rect::new(l.stop_all.x1 + GAP, r.y0, r.x1, r.y1);
    p.clip(cells, |p| {
        for col in &l.cols {
            let tr = &doc.tracks[&col.track];
            let tcol = th.track(tr.color.0);
            // header
            p.rrect(col.head, 4.0, c.bg_surface);
            p.rect(Rect::new(col.head.x0, col.head.y1 - 3.0, col.head.x1, col.head.y1), tcol);
            p.text_fit(col.head.x0 + 8.0, col.head.center().y - 1.0, &tr.name, &label, COL_W - 50.0);
            if col.group {
                p.text(col.head.x1 - 8.0, col.head.center().y - 1.0, "group", &small, Align::Right);
            }
            let follows = tracks_of(doc, col.track).iter().any(|t| view.following.contains(t));
            for (scene, row) in &l.scenes {
                let cell = l.cell(col, *row);
                if col.group {
                    // a summary of its tracks' clips in this scene
                    p.rrect(cell, 3.0, c.bg_surface.alpha(0.6));
                    let kids: Vec<TrackId> = tracks_of(doc, col.track);
                    let n = kids.len().max(1) as f64;
                    let w = (cell.width() - 30.0) / n;
                    for (i, k) in kids.iter().enumerate() {
                        let x = cell.x0 + 24.0 + i as f64 * w;
                        if let Some(clip) = doc.slots.get(&(*scene, *k)).and_then(|c| doc.clips.get(c)) {
                            let on = view.playing.get(k) == Some(&clip.id);
                            let col = th.track(clip.color.0).alpha(if on { 1.0 } else { 0.55 });
                            p.rect(Rect::new(x, cell.center().y - 3.0, x + w - 2.0, cell.center().y + 3.0), col);
                        }
                    }
                    icons::draw(
                        p,
                        Icon::Play,
                        Rect::new(cell.x0 + 7.0, cell.center().y - 5.0, cell.x0 + 17.0, cell.center().y + 5.0),
                        c.text_secondary,
                    );
                    continue;
                }
                match doc.slots.get(&(*scene, col.track)).and_then(|id| doc.clips.get(id)) {
                    Some(clip) => {
                        let ccol = th.track(clip.color.0);
                        let playing = view.playing.get(&col.track) == Some(&clip.id);
                        let queued = view.queued.get(&col.track).is_some_and(|(q, _)| *q == Some(clip.id));
                        let sel = selected_clip == Some(clip.id);
                        p.rrect(cell, 3.0, if playing { ccol.alpha(0.85) } else { ccol.alpha(0.28) });
                        let play_r = Rect::new(cell.x0, cell.y0, cell.x0 + PLAY_W, cell.y1);
                        p.rrect(play_r, 3.0, Rgba::new(0, 0, 0, if playing { 60 } else { 40 }));
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
                        p.text_fit(
                            cell.x0 + PLAY_W + 6.0,
                            cell.center().y,
                            &clip.name,
                            &TextStyle::mono(11.0, ink),
                            COL_W - PLAY_W - 10.0,
                        );
                        if playing && let Some(start) = view.started.get(&col.track) {
                            // how far through its loop the clip is
                            let len = clip.length.0.max(1) as f64;
                            let f = ((transport.playhead.0 - start.0).max(0) as f64 % len) / len;
                            let bar = Rect::new(
                                cell.x0 + PLAY_W,
                                cell.y1 - 3.0,
                                cell.x0 + PLAY_W + (cell.width() - PLAY_W) * f,
                                cell.y1 - 1.0,
                            );
                            p.rect(bar, Rgba::hex(0x0b0b0b).alpha(0.6));
                        }
                        if queued {
                            p.rrect_stroke(
                                cell.inset(-0.5),
                                3.0,
                                1.0,
                                if blink { c.accent_primary } else { c.border_default },
                            );
                        }
                        if sel {
                            p.rrect_stroke(cell.inset(-0.5), 3.0, 1.5, c.text_max);
                        }
                    }
                    None => {
                        p.rrect(cell, 3.0, c.bg_surface.alpha(0.45));
                        // empty slots stop the track
                        let sq = Rect::new(
                            cell.center().x - 3.5,
                            cell.center().y - 3.5,
                            cell.center().x + 3.5,
                            cell.center().y + 3.5,
                        );
                        p.rect(sq, if follows { c.text_secondary.alpha(0.7) } else { c.border_default });
                    }
                }
            }
            // stop button
            let stop = Rect::new(col.head.x0, l.stop_row.0, col.head.x1, l.stop_row.1);
            let stopping =
                tracks_of(doc, col.track).iter().any(|t| view.queued.get(t).is_some_and(|(q, _)| q.is_none()));
            p.rrect(stop, 3.0, c.bg_surface.alpha(0.6));
            let sq =
                Rect::new(stop.center().x - 4.5, stop.center().y - 4.5, stop.center().x + 4.5, stop.center().y + 4.5);
            let any_playing = tracks_of(doc, col.track).iter().any(|t| view.playing.contains_key(t));
            let scol = if stopping && blink {
                c.accent_primary
            } else if any_playing {
                c.text_max
            } else {
                c.text_secondary.alpha(0.6)
            };
            p.rect(sq, scol);
        }
    });
    // scene column
    for (i, (id, cell)) in l.scenes.iter().enumerate() {
        let scene = doc.scenes.iter().find(|s| s.id == *id).map_or("", |s| s.name.as_str());
        // a scene reads as playing when every clip it holds is playing
        let clips: Vec<(TrackId, ClipId)> =
            l.cols.iter().filter_map(|col| doc.slots.get(&(*id, col.track)).map(|c| (col.track, *c))).collect();
        let on = !clips.is_empty() && clips.iter().all(|(t, c)| view.playing.get(t) == Some(c));
        p.rrect(*cell, 3.0, if on { c.bg_control } else { c.bg_surface });
        let tri = Rect::new(cell.x0 + 8.0, cell.center().y - 5.0, cell.x0 + 18.0, cell.center().y + 5.0);
        icons::draw(p, Icon::Play, tri, if on { c.accent_primary } else { c.text_secondary });
        p.text_fit(cell.x0 + 28.0, cell.center().y, scene, &label, SCENE_W - 52.0);
        p.text(cell.x1 - 8.0, cell.center().y, &(i + 1).to_string(), &small, Align::Right);
    }
    p.rrect(l.stop_all, 3.0, c.bg_surface);
    let sq =
        Rect::new(l.stop_all.x0 + 9.0, l.stop_all.center().y - 4.0, l.stop_all.x0 + 17.0, l.stop_all.center().y + 4.0);
    p.rect(sq, c.text_secondary);
    p.text(l.stop_all.x0 + 28.0, l.stop_all.center().y, "stop all", &label, Align::Left);
    if doc.scenes.is_empty() {
        p.text(r.center().x, r.center().y, "no scenes yet", &small, Align::Center);
    }
    if !view.following.is_empty() {
        p.rrect(l.back, 5.0, c.accent_primary.alpha(0.18));
        p.rrect_stroke(l.back.inset(-0.5), 5.0, 1.0, c.accent_primary);
        p.text(
            l.back.center().x,
            l.back.center().y,
            "back to arrangement",
            &TextStyle::mono(11.5, c.accent_primary),
            Align::Center,
        );
    }
    l
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
    fn hits_and_queue_promotion() {
        let d = crate::demo::demo();
        let doc = &d.doc;
        let mut v = SessionView::default();
        let l = SessionLayout::new(doc, &v, Rect::new(0.0, 0.0, 1400.0, 600.0));
        let (s1, row) = l.scenes[0];
        assert_eq!(l.hit(doc, &v, row.center()), SessionHit::Scene(s1));
        assert_eq!(l.hit(doc, &v, l.stop_all.center()), SessionHit::StopAll);
        let drums = doc.tracks.values().find(|t| t.name == "drums").unwrap().id;
        let col = *l.cols.iter().find(|c| c.track == drums).unwrap();
        let cell = l.cell(&col, row);
        let clip = doc.slots[&(s1, drums)];
        assert_eq!(l.hit(doc, &v, Point::new(cell.x0 + 5.0, cell.center().y)), SessionHit::Launch(drums, clip));
        assert_eq!(l.hit(doc, &v, Point::new(cell.x1 - 5.0, cell.center().y)), SessionHit::Select(clip));
        assert_eq!(l.hit(doc, &v, Point::new(cell.center().x, l.stop_row.0 + 2.0)), SessionHit::Stop(drums));
        let rhythm = l.cols.iter().find(|c| c.group).unwrap();
        assert_eq!(l.hit(doc, &v, l.cell(rhythm, row).center()), SessionHit::Group(rhythm.track, s1));
        assert!(tracks_of(doc, rhythm.track).contains(&drums));
        // a queued launch plays once its bar comes
        v.queued.insert(drums, (Some(clip), Ticks::quarters(4)));
        assert!(!v.advance(doc, Ticks::quarters(2)));
        assert!(v.advance(doc, Ticks::quarters(4)));
        assert_eq!(v.playing.get(&drums), Some(&clip));
        assert!(v.following.contains(&drums));
        assert_eq!(l.hit(doc, &v, l.back.center()), SessionHit::BackToArrangement);
        v.clear();
        assert!(v.playing.is_empty() && v.following.is_empty());
    }
}
