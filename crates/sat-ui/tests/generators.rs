//! The demo's turing phrase: seeded, new each time round when it reseeds,
//! and the same every time it is compiled.

use sat_doc::sat_time::Ticks;
use sat_doc::*;

fn turing(doc: &Document) -> (ModuleId, ClipId, Placement) {
    let m = doc.modules.values().find(|m| m.kind == sat_modules::TURING).expect("a turing").id;
    let c = doc.clips.values().find(|c| doc.inlet_cable(c.id).is_some_and(|w| w.from.module == m)).unwrap().id;
    let p = *doc.placements.values().find(|p| p.clip == c).unwrap();
    (m, c, p)
}

fn bar_pitches(notes: &[sat_compile::TimelineNote], start: Ticks, bar: i64) -> Vec<(i64, u8)> {
    let len = Ticks::quarters(4).0;
    let from = start.0 + bar * len;
    notes.iter().filter(|n| n.start.0 >= from && n.start.0 < from + len).map(|n| (n.start.0 - from, n.pitch)).collect()
}

#[test]
fn a_reseeding_turing_plays_a_new_phrase_each_time_round() {
    let d = sat_ui::demo::demo();
    let (_, _, p) = turing(&d.doc);
    let notes = sat_compile::placement_notes(&d.doc, &p);
    let bars: Vec<_> = (0..8).map(|b| bar_pitches(&notes, p.start, b)).collect();
    assert!(bars.iter().all(|b| !b.is_empty()));
    for i in 1..bars.len() {
        assert_ne!(bars[i], bars[i - 1], "bar {i} repeats the one before");
    }
    // compiling again gives the same notes
    assert_eq!(notes, sat_compile::placement_notes(&d.doc, &p));
}

#[test]
fn without_reseed_the_phrase_repeats_and_a_new_clip_seed_changes_it() {
    let d = sat_ui::demo::demo();
    let (m, c, p) = turing(&d.doc);
    let mut h = History::new(d.doc.clone(), 10);
    h.apply(
        "lock",
        vec![Op::SetValue { target: ParamTarget::Module { module: m, param: "reseed".into() }, value: 0.0 }],
    )
    .unwrap();
    let notes = sat_compile::placement_notes(h.doc(), &p);
    let first = bar_pitches(&notes, p.start, 0);
    assert!((1..8).all(|b| bar_pitches(&notes, p.start, b) == first));
    // the first time round is the phrase the step editor shows
    let shown = sat_modules::effective_notes(h.doc(), m, c);
    assert_eq!(first, shown.iter().map(|n| (n.start.0, n.pitch)).collect::<Vec<_>>());
    let steps = sat_modules::gen_steps(h.doc(), m, c).unwrap();
    assert!(shown.iter().all(|n| steps.pitches[(n.start.0 / steps.rate.0) as usize] == n.pitch));
    let mut doc = h.doc().clone();
    doc.clips[&c].seed ^= 0xdead;
    assert_ne!(sat_modules::effective_notes(&doc, m, c), shown);
}

#[test]
fn the_demo_phrase_is_the_same_every_run() {
    let (a, b) = (sat_ui::demo::demo(), sat_ui::demo::demo());
    let (_, _, pa) = turing(&a.doc);
    let (_, _, pb) = turing(&b.doc);
    assert_eq!(sat_compile::placement_notes(&a.doc, &pa), sat_compile::placement_notes(&b.doc, &pb));
}
