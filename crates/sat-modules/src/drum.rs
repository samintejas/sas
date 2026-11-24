//! The drum sequencer: one row of steps per drum, General MIDI pitches.

use sat_doc::Note;

/// A drum row: its name and pitch.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// Shown on the pad.
    pub name: String,
    /// The pitch its steps play.
    pub pitch: u8,
}

/// The default kit, top row first.
pub const KIT: [(&str, u8); 8] = [
    ("kick", 36),
    ("rim", 37),
    ("snare", 38),
    ("clap", 39),
    ("ch hat", 42),
    ("low tom", 45),
    ("op hat", 46),
    ("crash", 49),
];

/// Velocities a step cycles through: normal, accent, ghost.
pub const VELOCITIES: [f32; 3] = [0.8, 1.0, 0.45];

/// The velocity after `v` in [`VELOCITIES`].
pub fn next_velocity(v: f32) -> f32 {
    let i = VELOCITIES.iter().position(|x| (x - v).abs() < 0.01).map_or(0, |i| i + 1);
    VELOCITIES[i % VELOCITIES.len()]
}

/// The rows to show for `notes`: the kit, then any other pitch the notes use,
/// highest first.
pub fn rows(notes: &[Note]) -> Vec<Row> {
    let mut v: Vec<Row> = KIT.iter().map(|(n, p)| Row { name: n.to_string(), pitch: *p }).collect();
    let mut extra: Vec<u8> = notes.iter().map(|n| n.pitch).filter(|p| !KIT.iter().any(|(_, k)| k == p)).collect();
    extra.sort_unstable_by(|a, b| b.cmp(a));
    extra.dedup();
    v.extend(extra.into_iter().map(|p| Row { name: crate::euclid::note_name(p), pitch: p }));
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use sat_doc::sat_time::Ticks;

    #[test]
    fn rows_add_unknown_pitches_and_velocities_cycle() {
        let n = |p| Note { start: Ticks(0), len: Ticks(240), pitch: p, velocity: 0.8, chance: 1.0 };
        let r = rows(&[n(36), n(60), n(51), n(60)]);
        assert_eq!(r.len(), KIT.len() + 2);
        assert_eq!((r[8].pitch, r[9].pitch), (60, 51));
        assert_eq!(r[8].name, "c4");
        assert_eq!(next_velocity(0.8), 1.0);
        assert_eq!(next_velocity(0.45), 0.8);
        assert_eq!(next_velocity(0.3), 0.8);
    }
}
