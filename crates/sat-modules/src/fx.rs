//! Fx-grid modules: the track's audio comes in at `in`, runs through effects
//! wired in any order (splits and mixes included) and leaves at `out`, ahead
//! of the mixer strip. A grid without both ends passes audio through.
//!
//! Parameter order here is the order the engine reads them in; every effect
//! also has a `bypass` switch, read as engine parameter [`BYPASS_INDEX`].

use sat_doc::{CableId, Document, Endpoint, GridId, ModuleId, Op, PortRef};

use crate::spec::{ParamSpec, Unit, p};
use crate::{PortKind, PortSpec};

/// Where the track's audio enters.
pub const FX_IN: &str = "fx_in";
/// Where the audio leaves for the mixer strip.
pub const FX_OUT: &str = "fx_out";
/// Three-band eq.
pub const EQ: &str = "eq";
/// Compressor with a side-chain input.
pub const COMP: &str = "comp";
/// Stereo delay.
pub const DELAY: &str = "delay";
/// Reverb.
pub const REVERB: &str = "reverb";
/// Saturation.
pub const DRIVE: &str = "drive";
/// Peak limiter.
pub const LIMITER: &str = "limiter";
/// Stereo filter.
pub const FX_FILTER: &str = "fx_filter";
/// One input to two outputs.
pub const SPLIT: &str = "split";
/// Two inputs to one output.
pub const MERGE: &str = "merge";
/// A hosted CLAP plugin (its path is the module's `plugin`). Its seven
/// knobs drive the plugin's first seven parameters, 0–1 across each one's
/// range.
pub const PLUGIN: &str = "clap";

/// The parameter key of the bypass switch.
pub const BYPASS: &str = "bypass";
/// The engine parameter index the bypass switch is sent to.
pub const BYPASS_INDEX: u8 = 7;

/// Fx modules that can be added: kind and label.
pub const MODULES: [(&str, &str); 12] = [
    (FX_IN, "in"),
    (EQ, "eq"),
    (COMP, "comp"),
    (DRIVE, "drive"),
    (FX_FILTER, "filter"),
    (DELAY, "delay"),
    (REVERB, "reverb"),
    (LIMITER, "limiter"),
    (SPLIT, "split"),
    (MERGE, "mix"),
    (PLUGIN, "plugin"),
    (FX_OUT, "out"),
];

const fn input(key: &'static str) -> PortSpec {
    PortSpec { key, kind: PortKind::Audio, input: true }
}

const fn output(key: &'static str) -> PortSpec {
    PortSpec { key, kind: PortKind::Audio, input: false }
}

/// Ports of an fx module kind.
pub fn ports(kind: &str) -> &'static [PortSpec] {
    const IN_PORTS: [PortSpec; 1] = [output("out")];
    const OUT_PORTS: [PortSpec; 1] = [input("in")];
    const THROUGH: [PortSpec; 2] = [input("in"), output("out")];
    const COMP_PORTS: [PortSpec; 3] = [input("in"), input("side"), output("out")];
    const SPLIT_PORTS: [PortSpec; 3] = [input("in"), output("a"), output("b")];
    const MERGE_PORTS: [PortSpec; 3] = [input("1"), input("2"), output("out")];
    match kind {
        FX_IN => &IN_PORTS,
        FX_OUT => &OUT_PORTS,
        EQ | DELAY | REVERB | DRIVE | LIMITER | FX_FILTER | PLUGIN => &THROUGH,
        COMP => &COMP_PORTS,
        SPLIT => &SPLIT_PORTS,
        MERGE => &MERGE_PORTS,
        _ => &[],
    }
}

/// Parameters of an fx module kind, in engine order.
pub fn params(kind: &str) -> &'static [ParamSpec] {
    const IN_P: [ParamSpec; 2] =
        [p("trim", "trim", -24.0, 24.0, 0.0, Unit::Db), p("width", "width", 0.0, 2.0, 1.0, Unit::Percent)];
    const OUT_P: [ParamSpec; 1] = [p("level", "level", -24.0, 6.0, 0.0, Unit::Db)];
    const EQ_P: [ParamSpec; 4] = [
        p("low", "low", -15.0, 15.0, 0.0, Unit::Db),
        p("mid", "mid", -15.0, 15.0, 0.0, Unit::Db),
        p("high", "high", -15.0, 15.0, 0.0, Unit::Db),
        p("freq", "freq", 200.0, 8000.0, 1000.0, Unit::Hz),
    ];
    const COMP_P: [ParamSpec; 5] = [
        p("thresh", "thresh", -50.0, 0.0, -18.0, Unit::Db),
        p("ratio", "ratio", 1.0, 20.0, 4.0, Unit::Ratio),
        p("attack", "attack", 0.0005, 0.1, 0.01, Unit::Seconds),
        p("release", "release", 0.01, 2.0, 0.15, Unit::Seconds),
        p("makeup", "makeup", 0.0, 24.0, 0.0, Unit::Db),
    ];
    const DELAY_P: [ParamSpec; 4] = [
        p("time", "time", 0.01, 2.0, 0.375, Unit::Seconds),
        p("feedback", "feedback", 0.0, 0.95, 0.35, Unit::Percent),
        p("tone", "tone", 500.0, 20_000.0, 6000.0, Unit::Hz),
        p("mix", "mix", 0.0, 1.0, 0.3, Unit::Percent),
    ];
    const REVERB_P: [ParamSpec; 5] = [
        p("size", "size", 0.0, 1.0, 0.6, Unit::Percent),
        p("decay", "decay", 0.2, 10.0, 2.4, Unit::Seconds),
        p("pre", "pre", 0.001, 0.25, 0.02, Unit::Seconds),
        p("lowcut", "lowcut", 20.0, 1000.0, 300.0, Unit::Hz),
        p("mix", "mix", 0.0, 1.0, 0.3, Unit::Percent),
    ];
    const DRIVE_P: [ParamSpec; 5] = [
        p("drive", "drive", 0.0, 36.0, 6.0, Unit::Db),
        p("bias", "bias", -1.0, 1.0, 0.0, Unit::Bipolar),
        p("tone", "tone", 500.0, 20_000.0, 12_000.0, Unit::Hz),
        p("level", "level", -24.0, 6.0, 0.0, Unit::Db),
        p("mix", "mix", 0.0, 1.0, 1.0, Unit::Percent),
    ];
    const LIMITER_P: [ParamSpec; 2] =
        [p("ceiling", "ceiling", -12.0, 0.0, -0.3, Unit::Db), p("release", "release", 0.01, 1.0, 0.08, Unit::Seconds)];
    const FILTER_P: [ParamSpec; 3] = [
        p("mode", "mode", 0.0, 2.0, 0.0, Unit::Choice(&["lp", "bp", "hp"])),
        p("cutoff", "cutoff", 20.0, 20_000.0, 8000.0, Unit::Hz),
        p("res", "res", 0.0, 1.0, 0.1, Unit::Percent),
    ];
    const SPLIT_P: [ParamSpec; 1] = [p("balance", "a · b", 0.0, 1.0, 0.5, Unit::Percent)];
    const MERGE_P: [ParamSpec; 2] = [p("g1", "1", -60.0, 6.0, 0.0, Unit::Db), p("g2", "2", -60.0, 6.0, 0.0, Unit::Db)];
    const PLUGIN_P: [ParamSpec; 7] = [
        p("p1", "p1", 0.0, 1.0, 0.5, Unit::Percent),
        p("p2", "p2", 0.0, 1.0, 0.5, Unit::Percent),
        p("p3", "p3", 0.0, 1.0, 0.5, Unit::Percent),
        p("p4", "p4", 0.0, 1.0, 0.5, Unit::Percent),
        p("p5", "p5", 0.0, 1.0, 0.5, Unit::Percent),
        p("p6", "p6", 0.0, 1.0, 0.5, Unit::Percent),
        p("p7", "p7", 0.0, 1.0, 0.5, Unit::Percent),
    ];
    match kind {
        FX_IN => &IN_P,
        FX_OUT => &OUT_P,
        EQ => &EQ_P,
        COMP => &COMP_P,
        DELAY => &DELAY_P,
        REVERB => &REVERB_P,
        DRIVE => &DRIVE_P,
        LIMITER => &LIMITER_P,
        FX_FILTER => &FILTER_P,
        SPLIT => &SPLIT_P,
        MERGE => &MERGE_P,
        PLUGIN => &PLUGIN_P,
        _ => &[],
    }
}

/// Is `kind` an fx-grid module?
pub fn is_fx(kind: &str) -> bool {
    MODULES.iter().any(|(k, _)| *k == kind)
}

/// Can `kind` be bypassed (every fx module but the two ends)?
pub fn bypassable(kind: &str) -> bool {
    is_fx(kind) && kind != FX_IN && kind != FX_OUT
}

/// Is the module bypassed?
pub fn bypassed(m: &sat_doc::Module) -> bool {
    m.params.get(BYPASS).is_some_and(|v| *v > 0.5)
}

/// Card width plus the gap between cards, for laying out chains.
const STEP: f32 = 196.0;

/// Ops that add an fx module of `kind` to `grid` and wire it into the chain
/// just before `out`. An empty grid first gets its `in` and `out`. Returns
/// the new module's id.
pub fn insert(doc: &Document, grid: GridId, kind: &str) -> (ModuleId, Vec<Op>) {
    let mut ops = Vec::new();
    let add = |ops: &mut Vec<Op>, kind: &str, pos: (f32, f32)| -> ModuleId {
        let id = ModuleId::new();
        let name = crate::label_of(kind).to_string();
        ops.push(Op::AddModule { id, grid, kind: kind.into(), name, params: crate::default_params(kind), pos });
        id
    };
    let find = |k: &str| doc.modules_on(grid).find(|m| m.kind == k).map(|m| (m.id, m.pos));
    let (fin, fout) = (find(FX_IN), find(FX_OUT));
    if kind == FX_IN || kind == FX_OUT || (fin.is_none() && fout.is_some()) {
        // an end of its own, or a grid whose input was removed: just add it
        let x = doc.modules_on(grid).map(|m| m.pos.0 + STEP).fold(0.0f32, f32::max);
        let id = add(&mut ops, kind, (x, 0.0));
        return (id, ops);
    }
    let (fin, fin_new) = match fin {
        Some((id, _)) => (id, false),
        None => (add(&mut ops, FX_IN, (0.0, 0.0)), true),
    };
    let (out, out_pos) = match fout {
        Some(f) => f,
        None => {
            let id = add(&mut ops, FX_OUT, (STEP * 2.0, 0.0));
            (id, (STEP, 0.0))
        }
    };
    let id = add(&mut ops, kind, out_pos);
    let new_out = (out_pos.0 + STEP, out_pos.1);
    if fout.is_some() {
        ops.push(Op::MoveModule { id: out, pos: new_out });
    } else if let Some(Op::AddModule { pos, .. }) =
        ops.iter_mut().find(|o| matches!(o, Op::AddModule { id: i, .. } if *i == out))
    {
        *pos = new_out;
    }
    let first = |input: bool| ports(kind).iter().find(|p| p.input == input).map(|p| p.key).unwrap_or("in");
    let into_out = Endpoint::Port(PortRef::new(out, "in"));
    let feed = if fin_new || fout.is_none() {
        None
    } else {
        doc.cables_on(grid).find(|k| k.to == into_out).map(|k| (k.id, k.from.clone()))
    };
    let from = match feed {
        Some((cable, from)) => {
            ops.push(Op::Disconnect { id: cable });
            from
        }
        None => PortRef::new(fin, "out"),
    };
    ops.push(Op::Connect { id: CableId::new(), from, to: Endpoint::Port(PortRef::new(id, first(true))) });
    ops.push(Op::Connect { id: CableId::new(), from: PortRef::new(id, first(false)), to: into_out });
    (id, ops)
}

/// Ops that fill an empty fx `grid` with `in`, then `effects` (kind and
/// parameter overrides) in series, then `out`, laid out left to right.
#[allow(clippy::type_complexity)]
pub fn chain(grid: GridId, effects: &[(&str, &[(&str, f32)])]) -> Vec<Op> {
    let mut ops = Vec::new();
    let mut prev: Option<ModuleId> = None;
    let all =
        std::iter::once((FX_IN, &[][..])).chain(effects.iter().copied()).chain(std::iter::once((FX_OUT, &[][..])));
    for (i, (kind, overrides)) in all.enumerate() {
        let id = ModuleId::new();
        let mut params = crate::default_params(kind);
        for (k, v) in overrides {
            params.insert(k.to_string(), *v);
        }
        let name = crate::label_of(kind).to_string();
        ops.push(Op::AddModule { id, grid, kind: kind.into(), name, params, pos: (i as f32 * STEP, 0.0) });
        if let Some(p) = prev {
            let from_port = if prev_kind_is_split(&ops, p) { "a" } else { "out" };
            let to_port = ports(kind).iter().find(|p| p.input).map_or("in", |p| p.key);
            ops.push(Op::Connect {
                id: CableId::new(),
                from: PortRef::new(p, from_port),
                to: Endpoint::Port(PortRef::new(id, to_port)),
            });
        }
        prev = Some(id);
    }
    ops
}

fn prev_kind_is_split(ops: &[Op], id: ModuleId) -> bool {
    ops.iter().any(|o| matches!(o, Op::AddModule { id: i, kind, .. } if *i == id && kind == SPLIT))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sat_doc::{TrackKind, apply};

    fn names(doc: &Document, grid: GridId) -> Vec<(String, String)> {
        let n = |id: ModuleId| doc.modules[&id].name.clone();
        let mut v: Vec<_> = doc
            .cables_on(grid)
            .filter_map(|k| match &k.to {
                Endpoint::Port(p) => Some((n(k.from.module), n(p.module))),
                _ => None,
            })
            .collect();
        v.sort();
        v
    }

    #[test]
    fn chains_wire_in_series() {
        let mut doc = Document::new(1);
        let grid = doc.tracks[&doc.master].fx_grids[0];
        for op in chain(grid, &[(COMP, &[("ratio", 2.0)]), (LIMITER, &[])]) {
            apply(&mut doc, op).unwrap();
        }
        let s = |a: &str, b: &str| (a.to_string(), b.to_string());
        assert_eq!(names(&doc, grid), vec![s("comp", "limiter"), s("in", "comp"), s("limiter", "out")]);
        let comp = doc.modules_on(grid).find(|m| m.kind == COMP).unwrap();
        assert_eq!(comp.params["ratio"], 2.0);
    }

    #[test]
    fn inserting_builds_a_chain_before_out() {
        let mut doc = Document::new(1);
        let (t, op) = Op::add_bus(TrackKind::Return, "hall", None, 0);
        apply(&mut doc, op).unwrap();
        let grid = doc.tracks[&t].fx_grids[0];
        let (eq, ops) = insert(&doc, grid, EQ);
        for op in ops {
            apply(&mut doc, op).unwrap();
        }
        let s = |a: &str, b: &str| (a.to_string(), b.to_string());
        assert_eq!(names(&doc, grid), vec![s("eq", "out"), s("in", "eq")]);
        let (_, ops) = insert(&doc, grid, REVERB);
        for op in ops {
            apply(&mut doc, op).unwrap();
        }
        assert_eq!(names(&doc, grid), vec![s("eq", "reverb"), s("in", "eq"), s("reverb", "out")]);
        // left to right, no overlaps
        let x = |k: &str| doc.modules_on(grid).find(|m| m.kind == k).unwrap().pos.0;
        assert!(x(FX_IN) < x(EQ) && x(EQ) < x(REVERB) && x(REVERB) < x(FX_OUT));
        assert_eq!(doc.modules[&eq].pos.0, STEP);
        // a comp goes in through its main input, not the side chain
        let (c, ops) = insert(&doc, grid, COMP);
        for op in ops {
            apply(&mut doc, op).unwrap();
        }
        assert!(doc.cables.values().any(|k| k.to == Endpoint::Port(PortRef::new(c, "in"))));
        for (k, _) in MODULES {
            for s in params(k) {
                assert!(s.min < s.max && (s.min..=s.max).contains(&s.default), "{k}.{}", s.key);
            }
            assert!(params(k).len() <= BYPASS_INDEX as usize, "{k}");
            assert!(!ports(k).is_empty(), "{k}");
        }
    }
}
