//! Instrument-grid modules: a voice patch (note in, oscillators, noise, mixer,
//! filter, envelope, vca, lfo, voice out) that every voice runs, or a drum
//! synth that plays General MIDI drums by pitch.
//!
//! Parameter order here is the order the engine reads them in.

use crate::spec::{ParamSpec, Unit, p};
use crate::{PortKind, PortSpec};

/// Note in: pitch, gate and velocity of the voice.
pub const NOTE_IN: &str = "note_in";
/// Oscillator.
pub const VCO: &str = "vco";
/// Noise source.
pub const NOISE: &str = "noise";
/// Three-input audio mixer.
pub const MIXER: &str = "mixer";
/// State-variable filter.
pub const FILTER: &str = "filter";
/// ADSR envelope.
pub const ADSR: &str = "adsr";
/// Voltage-controlled amplifier.
pub const VCA: &str = "vca";
/// Low-frequency oscillator.
pub const LFO: &str = "lfo";
/// Voice out: where each voice's audio leaves the patch.
pub const VOICE_OUT: &str = "voice_out";
/// A self-contained drum synth (no patch needed).
pub const DRUM_SYNTH: &str = "drum_synth";

const fn input(key: &'static str, kind: PortKind) -> PortSpec {
    PortSpec { key, kind, input: true }
}

const fn output(key: &'static str, kind: PortKind) -> PortSpec {
    PortSpec { key, kind, input: false }
}

const WAVES: &[&str] = &["sine", "tri", "saw", "square"];

/// Instrument modules that can be added: kind and label.
pub const MODULES: [(&str, &str); 10] = [
    (NOTE_IN, "note in"),
    (VCO, "vco"),
    (NOISE, "noise"),
    (MIXER, "mixer"),
    (FILTER, "filter"),
    (ADSR, "adsr"),
    (VCA, "vca"),
    (LFO, "lfo"),
    (VOICE_OUT, "voice out"),
    (DRUM_SYNTH, "drum synth"),
];

/// Ports of an instrument module kind.
pub fn ports(kind: &str) -> &'static [PortSpec] {
    use PortKind::*;
    const NOTE_IN_PORTS: [PortSpec; 3] = [output("pitch", Mod), output("gate", Mod), output("vel", Mod)];
    const VCO_PORTS: [PortSpec; 3] = [input("pitch", Mod), input("fm", Mod), output("out", Audio)];
    const OUT_ONLY: [PortSpec; 1] = [output("out", Audio)];
    const MIXER_PORTS: [PortSpec; 4] = [input("1", Audio), input("2", Audio), input("3", Audio), output("out", Audio)];
    const FILTER_PORTS: [PortSpec; 3] = [input("in", Audio), input("cutoff", Mod), output("out", Audio)];
    const ADSR_PORTS: [PortSpec; 2] = [input("gate", Mod), output("env", Mod)];
    const VCA_PORTS: [PortSpec; 3] = [input("in", Audio), input("cv", Mod), output("out", Audio)];
    const LFO_PORTS: [PortSpec; 1] = [output("out", Mod)];
    const VOICE_OUT_PORTS: [PortSpec; 1] = [input("in", Audio)];
    match kind {
        NOTE_IN => &NOTE_IN_PORTS,
        VCO => &VCO_PORTS,
        NOISE => &OUT_ONLY,
        MIXER => &MIXER_PORTS,
        FILTER => &FILTER_PORTS,
        ADSR => &ADSR_PORTS,
        VCA => &VCA_PORTS,
        LFO => &LFO_PORTS,
        VOICE_OUT => &VOICE_OUT_PORTS,
        _ => &[],
    }
}

/// Parameters of an instrument module kind, in engine order.
pub fn params(kind: &str) -> &'static [ParamSpec] {
    const NOTE_IN_P: [ParamSpec; 2] =
        [p("voices", "voices", 1.0, 16.0, 8.0, Unit::Int), p("bend", "transpose", -24.0, 24.0, 0.0, Unit::Semis)];
    const VCO_P: [ParamSpec; 5] = [
        p("wave", "wave", 0.0, 3.0, 2.0, Unit::Choice(WAVES)),
        p("tune", "tune", -24.0, 24.0, 0.0, Unit::Semis),
        p("fine", "fine", -100.0, 100.0, 0.0, Unit::Cents),
        p("fm", "fm", 0.0, 1.0, 0.0, Unit::Percent),
        p("level", "level", 0.0, 1.0, 0.8, Unit::Percent),
    ];
    const NOISE_P: [ParamSpec; 1] = [p("level", "level", 0.0, 1.0, 0.3, Unit::Percent)];
    const MIXER_P: [ParamSpec; 3] = [
        p("g1", "1", -60.0, 6.0, 0.0, Unit::Db),
        p("g2", "2", -60.0, 6.0, -6.0, Unit::Db),
        p("g3", "3", -60.0, 6.0, -12.0, Unit::Db),
    ];
    const FILTER_P: [ParamSpec; 5] = [
        p("mode", "mode", 0.0, 2.0, 0.0, Unit::Choice(&["lp", "bp", "hp"])),
        p("cutoff", "cutoff", 20.0, 20_000.0, 1200.0, Unit::Hz),
        p("res", "res", 0.0, 1.0, 0.2, Unit::Percent),
        p("amount", "env amt", -1.0, 1.0, 0.4, Unit::Bipolar),
        p("track", "key trk", 0.0, 1.0, 0.5, Unit::Percent),
    ];
    const ADSR_P: [ParamSpec; 4] = [
        p("a", "a", 0.001, 5.0, 0.005, Unit::Seconds),
        p("d", "d", 0.001, 5.0, 0.2, Unit::Seconds),
        p("s", "s", 0.0, 1.0, 0.6, Unit::Percent),
        p("r", "r", 0.001, 8.0, 0.15, Unit::Seconds),
    ];
    const VCA_P: [ParamSpec; 1] = [p("level", "level", -60.0, 6.0, 0.0, Unit::Db)];
    const LFO_P: [ParamSpec; 3] = [
        p("rate", "rate", 0.05, 30.0, 2.0, Unit::Hz),
        p("wave", "wave", 0.0, 3.0, 0.0, Unit::Choice(WAVES)),
        p("depth", "depth", 0.0, 1.0, 0.3, Unit::Percent),
    ];
    const VOICE_OUT_P: [ParamSpec; 2] =
        [p("level", "level", -60.0, 6.0, -6.0, Unit::Db), p("pan", "pan", -1.0, 1.0, 0.0, Unit::Pan)];
    const DRUM_P: [ParamSpec; 3] = [
        p("level", "level", -60.0, 6.0, -3.0, Unit::Db),
        p("tune", "tune", -12.0, 12.0, 0.0, Unit::Semis),
        p("decay", "decay", 0.25, 4.0, 1.0, Unit::Percent),
    ];
    match kind {
        NOTE_IN => &NOTE_IN_P,
        VCO => &VCO_P,
        NOISE => &NOISE_P,
        MIXER => &MIXER_P,
        FILTER => &FILTER_P,
        ADSR => &ADSR_P,
        VCA => &VCA_P,
        LFO => &LFO_P,
        VOICE_OUT => &VOICE_OUT_P,
        DRUM_SYNTH => &DRUM_P,
        _ => &[],
    }
}

/// Is `kind` an instrument-grid module?
pub fn is_instrument(kind: &str) -> bool {
    MODULES.iter().any(|(k, _)| *k == kind)
}

/// How an instrument grid reads in a track header: the drum synth's name,
/// "synth patch" for a voice patch, or the first module's name.
pub fn label(doc: &sat_doc::Document, grid: sat_doc::GridId) -> Option<String> {
    let mods: Vec<&sat_doc::Module> = doc.modules_on(grid).collect();
    if let Some(m) = mods.iter().find(|m| m.kind == DRUM_SYNTH) {
        return Some(m.name.clone());
    }
    if mods.iter().any(|m| m.kind == VOICE_OUT) {
        return Some("synth patch".into());
    }
    mods.first().map(|m| m.name.clone())
}

/// A starting patch: (kind, name, position, param overrides) per module and
/// cables as (from module index, port, to module index, port).
#[allow(clippy::type_complexity)]
pub fn default_patch() -> (Vec<(&'static str, (f32, f32))>, Vec<(usize, &'static str, usize, &'static str)>) {
    let mods = vec![
        (NOTE_IN, (0.0, 0.0)),
        (VCO, (196.0, 0.0)),
        (FILTER, (392.0, 0.0)),
        (VCA, (588.0, 0.0)),
        (VOICE_OUT, (784.0, 0.0)),
        (ADSR, (196.0, 230.0)),
    ];
    let cables = vec![
        (0, "pitch", 1, "pitch"),
        (1, "out", 2, "in"),
        (2, "out", 3, "in"),
        (3, "out", 4, "in"),
        (0, "gate", 5, "gate"),
        (5, "env", 3, "cv"),
        (5, "env", 2, "cutoff"),
    ];
    (mods, cables)
}

/// Ops that add [`default_patch`] to `grid`, with `overrides` applied to
/// matching (kind, parameter) pairs. Returns the ops and the new module ids,
/// in [`default_patch`] order.
pub fn default_patch_ops(
    grid: sat_doc::GridId,
    overrides: &[(&str, &str, f32)],
) -> (Vec<sat_doc::Op>, Vec<sat_doc::ModuleId>) {
    use sat_doc::{CableId, Endpoint, ModuleId, Op, PortRef};
    let (mods, cables) = default_patch();
    let ids: Vec<ModuleId> = mods.iter().map(|_| ModuleId::new()).collect();
    let mut ops: Vec<Op> = mods
        .iter()
        .zip(&ids)
        .map(|((kind, pos), id)| {
            let mut params = crate::default_params(kind);
            for (k, key, v) in overrides {
                if k == kind {
                    params.insert(key.to_string(), *v);
                }
            }
            Op::AddModule {
                id: *id,
                grid,
                kind: kind.to_string(),
                name: crate::label_of(kind).to_string(),
                params,
                pos: *pos,
            }
        })
        .collect();
    for (a, pa, b, pb) in cables {
        ops.push(Op::Connect {
            id: CableId::new(),
            from: PortRef::new(ids[a], pa),
            to: Endpoint::Port(PortRef::new(ids[b], pb)),
        });
    }
    (ops, ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_module_has_ports_or_params_and_unique_keys() {
        for (k, _) in MODULES {
            assert!(!ports(k).is_empty() || !params(k).is_empty(), "{k}");
            let mut keys: Vec<&str> = ports(k).iter().map(|p| p.key).collect();
            keys.sort();
            let n = keys.len();
            keys.dedup();
            assert_eq!(n, keys.len(), "{k} ports");
            for s in params(k) {
                assert!(s.min < s.max && (s.min..=s.max).contains(&s.default), "{k}.{}", s.key);
            }
        }
        let (mods, cables) = default_patch();
        for (a, pa, b, pb) in cables {
            assert!(ports(mods[a].0).iter().any(|p| p.key == pa && !p.input));
            assert!(ports(mods[b].0).iter().any(|p| p.key == pb && p.input));
        }
    }
}
