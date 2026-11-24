//! Built-in source modules: what they are called, which editor the bottom
//! panel opens for them, and how generators turn parameters into notes.
//!
//! A source module holds no notes itself; it reads the pattern stored under
//! (module, clip). For a generator (euclid) the notes come from its
//! parameters instead, and the pattern's override layer (notes added and
//! removed by hand) is applied on top. Baking writes the combined result into
//! the pattern, clears the layer and marks the module fixed, so it plays the
//! pattern from then on; switching back to generative ignores the baked notes.

#![warn(missing_docs)]

pub mod drum;
pub mod euclid;
pub mod fx;
pub mod instrument;
pub mod macros;
pub mod spec;

pub use spec::{ParamSpec, Unit};

use im::Vector;
use sat_doc::sat_time::Ticks;
use sat_doc::*;

/// Module kind of the piano roll.
pub const PIANO_ROLL: &str = "piano_roll";
/// Module kind of the drum sequencer.
pub const DRUM_SEQ: &str = "drum_seq";
/// Module kind of the euclidean generator.
pub const EUCLID: &str = "euclid";

/// The parameter that marks a generator as baked into a fixed pattern.
pub const FIXED: &str = "fixed";

/// What flows through a port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortKind {
    /// Note events.
    Notes,
    /// Audio.
    Audio,
    /// A control signal.
    Mod,
}

/// A port on a module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortSpec {
    /// Port key, as cables name it.
    pub key: &'static str,
    /// What flows through it.
    pub kind: PortKind,
    /// An input (left side) rather than an output (right side).
    pub input: bool,
}

/// The notes output every source module has.
pub const NOTES_OUT: PortSpec = PortSpec { key: "notes", kind: PortKind::Notes, input: false };

/// The ports of a module kind.
pub fn ports(kind: &str) -> &'static [PortSpec] {
    match kind {
        PIANO_ROLL | DRUM_SEQ | EUCLID => &[NOTES_OUT],
        k if fx::is_fx(k) => fx::ports(k),
        _ => instrument::ports(kind),
    }
}

/// The parameters of a module kind, in knob (and engine) order.
pub fn params_of(kind: &str) -> &'static [ParamSpec] {
    match kind {
        EUCLID => &euclid::PARAMS,
        k if fx::is_fx(k) => fx::params(k),
        _ => instrument::params(kind),
    }
}

/// The spec of one parameter of a module kind.
pub fn spec(kind: &str, key: &str) -> Option<&'static ParamSpec> {
    params_of(kind).iter().find(|p| p.key == key)
}

/// The modules that can be added to a grid of `kind`: kind and label.
pub fn catalogue(kind: GridKind) -> &'static [(&'static str, &'static str)] {
    match kind {
        GridKind::Source => &SOURCES,
        GridKind::Instrument => &instrument::MODULES,
        GridKind::Fx => &fx::MODULES,
    }
}

/// Source modules that can be added to a source grid: kind and label.
pub const SOURCES: [(&str, &str); 3] = [(PIANO_ROLL, "piano roll"), (DRUM_SEQ, "drum seq"), (EUCLID, "euclid")];

/// The label of a module kind.
pub fn label_of(kind: &str) -> &str {
    SOURCES
        .iter()
        .chain(instrument::MODULES.iter())
        .chain(fx::MODULES.iter())
        .find(|(k, _)| *k == kind)
        .map_or(kind, |(_, l)| l)
}

/// An op that adds a source module of `kind` to `grid` at `pos`, named after
/// its kind and numbered when the grid already has one ("euclid 2").
pub fn add_module(doc: &Document, grid: GridId, kind: &str, pos: (f32, f32)) -> (ModuleId, Op) {
    let label = label_of(kind);
    let taken: Vec<&str> = doc.modules_on(grid).map(|m| m.name.as_str()).collect();
    let name = if !taken.contains(&label) {
        label.to_string()
    } else {
        (2..).map(|i| format!("{label} {i}")).find(|n| !taken.contains(&n.as_str())).unwrap()
    };
    let id = ModuleId::new();
    (id, Op::AddModule { id, grid, kind: kind.into(), name, params: default_params(kind), pos })
}

/// Ops that wire `module`'s notes output into `clip`'s inlet, replacing
/// whatever fed it.
pub fn wire(doc: &Document, module: ModuleId, clip: ClipId) -> Vec<Op> {
    let from = PortRef::new(module, NOTES_OUT.key);
    let mut ops = Vec::new();
    if let Some(old) = doc.inlet_cable(clip) {
        if old.from == from {
            return ops;
        }
        ops.push(Op::Disconnect { id: old.id });
    }
    ops.push(Op::Connect { id: CableId::new(), from, to: Endpoint::Inlet(clip) });
    ops
}

/// Ops that patch output `from` into input `to` on the same grid, moving the
/// cable `moving` if one is being dragged and replacing whatever fed `to`.
/// `None` when the ports do not fit: an output must feed an input of the
/// same kind on another module, without making a loop.
pub fn patch(doc: &Document, from: &PortRef, to: &PortRef, moving: Option<CableId>) -> Option<Vec<Op>> {
    let (a, b) = (doc.modules.get(&from.module)?, doc.modules.get(&to.module)?);
    let out = ports(&a.kind).iter().find(|p| p.key == from.port && !p.input)?;
    let inp = ports(&b.kind).iter().find(|p| p.key == to.port && p.input)?;
    if a.grid != b.grid || a.id == b.id || out.kind != inp.kind {
        return None;
    }
    let cables: Vec<&Cable> = doc.cables_on(a.grid).filter(|k| Some(k.id) != moving).collect();
    // a loop if `from` is downstream of `to`
    let mut seen = vec![to.module];
    let mut i = 0;
    while i < seen.len() {
        let m = seen[i];
        if m == from.module {
            return None;
        }
        for k in &cables {
            if let Endpoint::Port(p) = &k.to
                && k.from.module == m
                && !seen.contains(&p.module)
            {
                seen.push(p.module);
            }
        }
        i += 1;
    }
    let dest = Endpoint::Port(to.clone());
    let mut ops: Vec<Op> = moving.map(|id| Op::Disconnect { id }).into_iter().collect();
    if cables.iter().any(|k| k.from == *from && k.to == dest) && moving.is_none() {
        return Some(Vec::new());
    }
    for k in cables.iter().filter(|k| k.to == dest) {
        ops.push(Op::Disconnect { id: k.id });
    }
    ops.push(Op::Connect { id: CableId::new(), from: from.clone(), to: dest });
    Some(ops)
}

/// Which editor the bottom panel opens for a source module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Editor {
    /// Notes on a pitch-by-time grid.
    PianoRoll,
    /// One row of steps per drum.
    Drums,
    /// Generator knobs over one row of steps.
    Euclid,
}

/// The editor for a module kind; unknown kinds open the piano roll.
pub fn editor_of(kind: &str) -> Editor {
    match kind {
        DRUM_SEQ => Editor::Drums,
        EUCLID => Editor::Euclid,
        _ => Editor::PianoRoll,
    }
}

/// Does this module kind generate its notes from parameters?
pub fn is_generator(kind: &str) -> bool {
    kind == EUCLID
}

/// Is this module a generator that is currently generating (not baked)?
pub fn generating(m: &Module) -> bool {
    is_generator(&m.kind) && m.params.get(FIXED).copied().unwrap_or(0.0) < 0.5
}

/// Does changing `target` change notes (so the schedule must be recompiled
/// rather than sent a parameter event)?
pub fn shapes_notes(doc: &Document, target: &ParamTarget) -> bool {
    match target {
        ParamTarget::Module { module, .. } => doc.modules.get(module).is_some_and(|m| is_generator(&m.kind)),
        ParamTarget::Macro { grid, index } => macros::drives_notes(doc, *grid, *index),
        _ => false,
    }
}

/// Default parameters for a new module of `kind`.
pub fn default_params(kind: &str) -> im::OrdMap<String, f32> {
    params_of(kind).iter().map(|p| (p.key.to_string(), p.default)).collect()
}

fn sort(v: &mut [Note]) {
    v.sort_by(|a, b| a.start.cmp(&b.start).then(a.pitch.cmp(&b.pitch)));
}

/// The notes a module produces for a clip before hand edits: its generator's
/// output when it is generating, otherwise the stored pattern.
pub fn base_notes(doc: &Document, module: ModuleId, clip: ClipId) -> Vec<Note> {
    let (Some(m), Some(c)) = (doc.modules.get(&module), doc.clips.get(&clip)) else { return Vec::new() };
    if generating(m) {
        return match m.kind.as_str() {
            EUCLID => euclid::Params::of(&macros::resolved(doc, m)).generate(c.length),
            _ => Vec::new(),
        };
    }
    doc.pattern(module, clip).map(|p| p.notes.iter().copied().collect()).unwrap_or_default()
}

/// The notes a clip plays from `module`: [`base_notes`] minus hand-removed
/// notes plus hand-added ones, sorted by start then pitch, one note per slot.
pub fn effective_notes(doc: &Document, module: ModuleId, clip: ClipId) -> Vec<Note> {
    let mut v = base_notes(doc, module, clip);
    if let Some(p) = doc.pattern(module, clip) {
        let o = &p.overrides;
        // a hand-added note replaces a generated one in the same slot
        v.retain(|n| {
            !o.removed.contains(&(n.start, n.pitch))
                && !o.added.iter().any(|a| a.start == n.start && a.pitch == n.pitch)
        });
        v.extend(o.added.iter().copied());
    }
    sort(&mut v);
    v
}

/// The clip's hand edits over a generator: (added, removed).
pub fn edit_counts(doc: &Document, module: ModuleId, clip: ClipId) -> (usize, usize) {
    doc.pattern(module, clip).map(|p| (p.overrides.added.len(), p.overrides.removed.len())).unwrap_or((0, 0))
}

/// Ops that turn the step at (`note.start`, `note.pitch`) on or off.
///
/// For a generating module the edit goes into the override layer: a
/// generated note is removed (or restored), anything else is added (or
/// taken back). For other modules the stored pattern is edited directly.
pub fn toggle_step(doc: &Document, module: ModuleId, clip: ClipId, note: Note) -> Vec<Op> {
    let Some(m) = doc.modules.get(&module) else { return Vec::new() };
    let pat = doc.pattern(module, clip);
    let slot = |n: &Note| n.start == note.start && n.pitch == note.pitch;
    if generating(m) {
        let mut ops = Vec::new();
        if pat.is_none() {
            ops.push(Op::WritePattern { module, clip, notes: Vector::new() });
        }
        let mut o = pat.map(|p| p.overrides.clone()).unwrap_or_default();
        if let Some(i) = o.added.iter().position(slot) {
            o.added.remove(i);
        } else if base_notes(doc, module, clip).iter().any(slot) {
            if o.removed.remove(&(note.start, note.pitch)).is_none() {
                o.removed.insert((note.start, note.pitch));
            }
        } else {
            o.added.push_back(note);
        }
        ops.push(Op::SetOverrides { module, clip, overrides: o });
        return ops;
    }
    let mut notes: Vec<Note> = pat.map(|p| p.notes.iter().copied().collect()).unwrap_or_default();
    match notes.iter().position(slot) {
        Some(i) => {
            notes.remove(i);
        }
        None => notes.push(note),
    }
    sort(&mut notes);
    vec![Op::WritePattern { module, clip, notes: Vector::from(notes) }]
}

/// Ops that clear a clip's hand edits.
pub fn clear_edits(doc: &Document, module: ModuleId, clip: ClipId) -> Vec<Op> {
    match doc.pattern(module, clip) {
        Some(p) if !p.overrides.is_empty() => vec![Op::SetOverrides { module, clip, overrides: Overrides::default() }],
        _ => Vec::new(),
    }
}

/// Ops that bake a generator and its hand edits into a fixed pattern.
pub fn bake(doc: &Document, module: ModuleId, clip: ClipId) -> Vec<Op> {
    let notes = Vector::from(effective_notes(doc, module, clip));
    vec![
        Op::WritePattern { module, clip, notes },
        Op::SetOverrides { module, clip, overrides: Overrides::default() },
        Op::SetValue { target: ParamTarget::Module { module, param: FIXED.into() }, value: 1.0 },
    ]
}

/// Ops that switch a baked generator back to generating.
pub fn unbake(module: ModuleId) -> Vec<Op> {
    vec![Op::SetValue { target: ParamTarget::Module { module, param: FIXED.into() }, value: 0.0 }]
}

/// The note at a step for editors that write one note per step.
pub fn step_note(start: Ticks, len: Ticks, pitch: u8, velocity: f32) -> Note {
    Note { start, len: Ticks(len.0.max(1)), pitch, velocity, chance: 1.0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use im::OrdMap;

    #[test]
    fn patching_checks_kinds_loops_and_replaces() {
        use instrument as ins;
        let mut doc = Document::new(1);
        let (t, op) = Op::add_track("t", None, 0);
        apply(&mut doc, op).unwrap();
        let grid = doc.tracks[&t].instrument_grid.unwrap();
        let (ops, ids) = ins::default_patch_ops(grid, &[]);
        for op in ops {
            apply(&mut doc, op).unwrap();
        }
        let [note, vco, filter, vca, _out, adsr] = ids[..] else { panic!() };
        let pr = |m, k: &str| PortRef::new(m, k);
        // audio into a mod input, input to input, and a module into itself do not fit
        assert!(patch(&doc, &pr(vco, "out"), &pr(filter, "cutoff"), None).is_none());
        assert!(patch(&doc, &pr(vco, "pitch"), &pr(filter, "in"), None).is_none());
        assert!(patch(&doc, &pr(filter, "out"), &pr(filter, "in"), None).is_none());
        // vca out back into the filter would loop
        assert!(patch(&doc, &pr(vca, "out"), &pr(filter, "in"), None).is_none());
        // the same cable again changes nothing
        assert_eq!(patch(&doc, &pr(vco, "out"), &pr(filter, "in"), None), Some(Vec::new()));
        // the envelope into the vco's fm replaces nothing; into the vca cv it replaces
        let ops = patch(&doc, &pr(adsr, "env"), &pr(vco, "fm"), None).unwrap();
        assert_eq!(ops.len(), 1);
        let ops = patch(&doc, &pr(note, "vel"), &pr(vca, "cv"), None).unwrap();
        assert!(matches!(ops[..], [Op::Disconnect { .. }, Op::Connect { .. }]));
        for op in ops {
            apply(&mut doc, op).unwrap();
        }
        let into_cv: Vec<_> =
            doc.cables.values().filter(|k| k.to == Endpoint::Port(pr(vca, "cv"))).map(|k| k.from.clone()).collect();
        assert_eq!(into_cv, vec![pr(note, "vel")]);
    }

    /// A document with one track, a source module of `kind` and a one-bar clip
    /// wired to it.
    fn setup(kind: &str) -> (History, ModuleId, ClipId) {
        let mut h = History::new(Document::new(1), 50);
        let (track, op) = Op::add_track("t", None, 0);
        let Op::AddTrack { grids, .. } = &op else { unreachable!() };
        let src = grids.source.unwrap();
        let module = ModuleId::new();
        let clip = ClipId::new();
        h.apply(
            "setup",
            [
                op,
                Op::AddModule {
                    id: module,
                    grid: src,
                    kind: kind.into(),
                    name: kind.into(),
                    params: default_params(kind),
                    pos: (0.0, 0.0),
                },
                Op::AddClip {
                    id: clip,
                    track,
                    name: "c".into(),
                    color: Color(0),
                    length: Ticks::quarters(4),
                    looped: true,
                    seed: 0,
                },
                Op::Connect { id: CableId::new(), from: PortRef::new(module, "out"), to: Endpoint::Inlet(clip) },
            ],
        )
        .unwrap();
        (h, module, clip)
    }

    fn starts(v: &[Note]) -> Vec<i64> {
        v.iter().map(|n| n.start.0 / 240).collect()
    }

    #[test]
    fn euclid_generates_and_hand_edits_sit_on_top() {
        let (mut h, m, c) = setup(EUCLID);
        assert_eq!(starts(&effective_notes(h.doc(), m, c)), vec![0, 4, 8, 12]);
        // remove a generated hit, add an off-grid one
        let pitch = euclid::Params::of(&h.doc().modules[&m]).note;
        let at = |s: i64| step_note(Ticks::sixteenths(s), Ticks(120), pitch, 0.7);
        h.apply("off", toggle_step(h.doc(), m, c, at(4))).unwrap();
        h.apply("on", toggle_step(h.doc(), m, c, at(6))).unwrap();
        assert_eq!(starts(&effective_notes(h.doc(), m, c)), vec![0, 6, 8, 12]);
        assert_eq!(edit_counts(h.doc(), m, c), (1, 1));
        // the edits survive a parameter change
        h.apply("hits", [Op::SetValue { target: ParamTarget::Module { module: m, param: "hits".into() }, value: 8.0 }])
            .unwrap();
        assert_eq!(starts(&effective_notes(h.doc(), m, c)), vec![0, 2, 6, 8, 10, 12, 14]);
        // toggling again takes both edits back
        h.apply("undo off", toggle_step(h.doc(), m, c, at(4))).unwrap();
        h.apply("undo on", toggle_step(h.doc(), m, c, at(6))).unwrap();
        assert_eq!(edit_counts(h.doc(), m, c), (0, 0));
    }

    #[test]
    fn bake_fixes_the_result_and_unbake_regenerates() {
        let (mut h, m, c) = setup(EUCLID);
        let pitch = euclid::Params::of(&h.doc().modules[&m]).note;
        h.apply("on", toggle_step(h.doc(), m, c, step_note(Ticks::sixteenths(2), Ticks(120), pitch, 0.7))).unwrap();
        let before = effective_notes(h.doc(), m, c);
        h.apply("bake", bake(h.doc(), m, c)).unwrap();
        assert!(!generating(&h.doc().modules[&m]));
        assert_eq!(edit_counts(h.doc(), m, c), (0, 0));
        assert_eq!(effective_notes(h.doc(), m, c), before);
        // a baked generator ignores its knobs
        h.apply("hits", [Op::SetValue { target: ParamTarget::Module { module: m, param: "hits".into() }, value: 1.0 }])
            .unwrap();
        assert_eq!(effective_notes(h.doc(), m, c), before);
        h.apply("gen", unbake(m)).unwrap();
        assert_eq!(starts(&effective_notes(h.doc(), m, c)), vec![0]);
        assert!(clear_edits(h.doc(), m, c).is_empty());
    }

    #[test]
    fn modules_are_numbered_and_wiring_replaces_the_feed() {
        let (mut h, m, c) = setup(PIANO_ROLL);
        let grid = h.doc().modules[&m].grid;
        let (e, op) = add_module(h.doc(), grid, EUCLID, (200.0, 0.0));
        h.apply("add", [op]).unwrap();
        let (e2, op) = add_module(h.doc(), grid, EUCLID, (400.0, 0.0));
        h.apply("add", [op]).unwrap();
        assert_eq!(h.doc().modules[&e].name, "euclid");
        assert_eq!(h.doc().modules[&e2].name, "euclid 2");
        assert_eq!(ports(EUCLID), &[NOTES_OUT]);
        h.apply("wire", wire(h.doc(), e, c)).unwrap();
        assert_eq!(h.doc().inlet_cable(c).unwrap().from.module, e);
        assert_eq!(starts(&effective_notes(h.doc(), e, c)), vec![0, 4, 8, 12]);
        assert!(wire(h.doc(), e, c).is_empty(), "already wired");
        assert_eq!(h.doc().cables.len(), 1);
    }

    #[test]
    fn plain_patterns_toggle_in_place() {
        let (mut h, m, c) = setup(DRUM_SEQ);
        let kick = |s: i64| step_note(Ticks::sixteenths(s), Ticks(240), 36, 1.0);
        h.apply("a", toggle_step(h.doc(), m, c, kick(4))).unwrap();
        h.apply("b", toggle_step(h.doc(), m, c, kick(0))).unwrap();
        assert_eq!(starts(&effective_notes(h.doc(), m, c)), vec![0, 4]);
        h.apply("c", toggle_step(h.doc(), m, c, kick(4))).unwrap();
        assert_eq!(starts(&effective_notes(h.doc(), m, c)), vec![0]);
        assert_eq!(edit_counts(h.doc(), m, c), (0, 0));
        assert!(!shapes_notes(h.doc(), &ParamTarget::Module { module: m, param: "x".into() }));
        assert_eq!(editor_of(DRUM_SEQ), Editor::Drums);
        assert_eq!(editor_of("anything"), Editor::PianoRoll);
        assert_eq!(default_params(PIANO_ROLL), OrdMap::new());
    }
}
