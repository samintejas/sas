//! Macros: each grid has 16, and an assigned macro drives parameters of
//! modules on its grid. A bound parameter follows its macro: as the macro
//! goes 0 to 1 the parameter's knob goes from the bind's `lo` to `hi`.

use sat_doc::{Document, GridId, MACROS, MacroBind, MacroSlot, Module, ModuleId, Op, ParamTarget};

use crate::spec::ParamSpec;

/// The macro (index and bind) driving `key` on `module`, if any.
pub fn bound(doc: &Document, module: ModuleId, key: &str) -> Option<(u8, MacroBind)> {
    let m = doc.modules.get(&module)?;
    let g = doc.grids.get(&m.grid)?;
    g.slots
        .iter()
        .find_map(|(i, s)| s.binds.iter().find(|b| b.module == module && b.param == key).map(|b| (*i, b.clone())))
}

/// The value a bind gives its parameter at macro value `v`.
pub fn value_at(spec: &ParamSpec, bind: &MacroBind, v: f32) -> f32 {
    let pos = bind.lo + (bind.hi - bind.lo) * v.clamp(0.0, 1.0);
    spec.at(pos as f64)
}

/// `m` with every macro-bound parameter set to the value its macro gives it.
pub fn resolved(doc: &Document, m: &Module) -> Module {
    let Some(g) = doc.grids.get(&m.grid) else { return m.clone() };
    let mut out = m.clone();
    for (i, s) in g.slots.iter() {
        let v = g.macros[*i as usize];
        for b in s.binds.iter().filter(|b| b.module == m.id) {
            if let Some(spec) = crate::spec(&m.kind, &b.param) {
                out.params.insert(b.param.clone(), value_at(spec, b, v));
            }
        }
    }
    out
}

/// A parameter's effective value: its macro's when bound, else its own.
pub fn value(doc: &Document, module: ModuleId, key: &str) -> Option<f32> {
    let m = doc.modules.get(&module)?;
    let spec = crate::spec(&m.kind, key)?;
    Some(match bound(doc, module, key) {
        Some((i, b)) => value_at(spec, &b, doc.grids[&m.grid].macros[i as usize]),
        None => spec.clamp(m.params.get(key).copied().unwrap_or(spec.default)),
    })
}

/// The first unassigned macro of `grid`.
pub fn free_slot(doc: &Document, grid: GridId) -> Option<u8> {
    let g = doc.grids.get(&grid)?;
    (0..MACROS as u8).find(|i| !g.slots.contains_key(i))
}

/// Ops that bind `key` on `module` to the first free macro of its grid,
/// named after the parameter, over the parameter's full range, with the
/// macro set where the parameter is now (so nothing jumps). `None` when the
/// parameter is already bound, unknown, or the grid has no free macro.
pub fn learn(doc: &Document, module: ModuleId, key: &str) -> Option<(u8, Vec<Op>)> {
    let m = doc.modules.get(&module)?;
    let spec = crate::spec(&m.kind, key)?;
    if bound(doc, module, key).is_some() {
        return None;
    }
    let index = free_slot(doc, m.grid)?;
    let pos = spec.pos(m.params.get(key).copied().unwrap_or(spec.default)) as f32;
    let bind = MacroBind { module, param: key.into(), lo: 0.0, hi: 1.0 };
    let slot = MacroSlot { name: spec.label.into(), binds: im::Vector::unit(bind) };
    Some((
        index,
        vec![
            Op::SetMacroSlot { grid: m.grid, index, slot: Some(slot) },
            Op::SetValue { target: ParamTarget::Macro { grid: m.grid, index }, value: pos },
        ],
    ))
}

/// Does macro `index` of `grid` drive a parameter that shapes notes?
pub fn drives_notes(doc: &Document, grid: GridId, index: u8) -> bool {
    doc.grids.get(&grid).and_then(|g| g.slots.get(&index)).is_some_and(|s| {
        s.binds.iter().any(|b| doc.modules.get(&b.module).is_some_and(|m| crate::is_generator(&m.kind)))
    })
}

/// What a macro knob shows: its name and its first bound parameter's value
/// as that parameter reads (`1.20k`), or `None` when unassigned.
pub fn readout(doc: &Document, grid: GridId, index: u8) -> Option<(String, String)> {
    let g = doc.grids.get(&grid)?;
    let s = g.slots.get(&index)?;
    let shown = s
        .binds
        .iter()
        .find_map(|b| {
            let m = doc.modules.get(&b.module)?;
            let spec = crate::spec(&m.kind, &b.param)?;
            Some(spec.show(value_at(spec, b, g.macros[index as usize])))
        })
        .unwrap_or_else(|| "unbound".into());
    Some((s.name.clone(), shown))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instrument as ins;
    use sat_doc::apply;

    #[test]
    fn learning_binds_without_a_jump_and_macros_drive_values() {
        let mut doc = Document::new(1);
        let (t, op) = Op::add_track("t", None, 0);
        apply(&mut doc, op).unwrap();
        let grid = doc.tracks[&t].instrument_grid.unwrap();
        let (ops, ids) = ins::default_patch_ops(grid, &[(ins::FILTER, "cutoff", 1200.0)]);
        for op in ops {
            apply(&mut doc, op).unwrap();
        }
        let filter = ids[2];
        let before = value(&doc, filter, "cutoff").unwrap();
        let (i, ops) = learn(&doc, filter, "cutoff").unwrap();
        assert_eq!(i, 0);
        for op in ops {
            apply(&mut doc, op).unwrap();
        }
        assert!((value(&doc, filter, "cutoff").unwrap() - before).abs() / before < 0.01, "no jump");
        assert!(learn(&doc, filter, "cutoff").is_none(), "already bound");
        assert_eq!(readout(&doc, grid, 0).unwrap().0, "cutoff");
        // the macro now drives the cutoff, whatever the module's own value says
        let target = ParamTarget::Macro { grid, index: 0 };
        apply(&mut doc, Op::SetValue { target: target.clone(), value: 1.0 }).unwrap();
        assert_eq!(value(&doc, filter, "cutoff").unwrap(), 20_000.0);
        assert_eq!(resolved(&doc, &doc.modules[&filter]).params["cutoff"], 20_000.0);
        assert_eq!(readout(&doc, grid, 0).unwrap().1, "20.00k");
        // a second learn takes the next macro
        let (j, _) = learn(&doc, filter, "res").unwrap();
        assert_eq!(j, 1);
        assert!(!drives_notes(&doc, grid, 0));
    }
}
