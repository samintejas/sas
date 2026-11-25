//! Musical scales and pitch quantizing, shared by the note processors on a
//! source grid and the quantizer in a voice patch.

/// Scale names, in choice order.
pub const NAMES: &[&str] = &[
    "chromatic",
    "major",
    "minor",
    "dorian",
    "phrygian",
    "lydian",
    "mixolydian",
    "locrian",
    "harm min",
    "mel min",
    "pent maj",
    "pent min",
    "blues",
    "whole",
];

/// Pitch-class names, for a root choice.
pub const ROOTS: &[&str] = &["c", "c#", "d", "d#", "e", "f", "f#", "g", "g#", "a", "a#", "b"];

/// The degrees of each scale, as semitones above the root.
const DEGREES: [&[u8]; 14] = [
    &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
    &[0, 2, 4, 5, 7, 9, 11],
    &[0, 2, 3, 5, 7, 8, 10],
    &[0, 2, 3, 5, 7, 9, 10],
    &[0, 1, 3, 5, 7, 8, 10],
    &[0, 2, 4, 6, 7, 9, 11],
    &[0, 2, 4, 5, 7, 9, 10],
    &[0, 1, 3, 5, 6, 8, 10],
    &[0, 2, 3, 5, 7, 8, 11],
    &[0, 2, 3, 5, 7, 9, 11],
    &[0, 2, 4, 7, 9],
    &[0, 3, 5, 7, 10],
    &[0, 3, 5, 6, 7, 10],
    &[0, 2, 4, 6, 8, 10],
];

/// The semitone offsets of scale `index` (clamped to the list).
pub fn degrees(index: usize) -> &'static [u8] {
    DEGREES[index.min(DEGREES.len() - 1)]
}

/// Is pitch class `pc` (0–11) in the scale rooted at `root`?
pub fn contains(pc: i32, root: i32, scale: usize) -> bool {
    degrees(scale).contains(&((pc - root).rem_euclid(12) as u8))
}

/// `pitch` (MIDI, fractional) moved to the nearest note of the scale rooted
/// at pitch class `root`; ties go down.
pub fn quantize(pitch: f32, root: i32, scale: usize) -> f32 {
    if !pitch.is_finite() {
        return 0.0;
    }
    let base = pitch.round() as i32;
    let mut best = base;
    let mut dist = f32::MAX;
    for d in -6..=6 {
        let n = base + d;
        if contains(n, root, scale) {
            let e = (n as f32 - pitch).abs();
            if e < dist - 1e-4 || ((e - dist).abs() <= 1e-4 && n < best) {
                best = n;
                dist = e;
            }
        }
    }
    best as f32
}

/// The note `steps` scale degrees from `pitch` (which is first snapped to the
/// scale), so a step of 2 in major from C is E.
pub fn step(pitch: i32, steps: i32, root: i32, scale: usize) -> i32 {
    let deg = degrees(scale);
    let n = deg.len() as i32;
    let p = quantize(pitch as f32, root, scale) as i32;
    let rel = p - root;
    let oct = rel.div_euclid(12);
    let idx = deg.iter().position(|d| *d as i32 == rel.rem_euclid(12)).unwrap_or(0) as i32 + steps;
    root + (oct + idx.div_euclid(n)) * 12 + deg[idx.rem_euclid(n) as usize] as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantizing_snaps_to_the_nearest_degree() {
        assert_eq!(NAMES.len(), DEGREES.len());
        // c major: c# goes down to c, f# down to f, 61.6 up to d
        assert_eq!(quantize(61.0, 0, 1), 60.0);
        assert_eq!(quantize(66.0, 0, 1), 65.0);
        assert_eq!(quantize(61.6, 0, 1), 62.0);
        // a minor pentatonic from a different root
        assert_eq!(quantize(70.0, 9, 11), 69.0);
        assert_eq!(quantize(65.4, 0, 0), 65.0);
        // negative pitches work
        assert_eq!(quantize(-1.0, 0, 1), -1.0);
        assert_eq!(quantize(f32::NAN, 0, 1), 0.0);
    }

    #[test]
    fn steps_walk_the_scale() {
        assert_eq!(step(60, 2, 0, 1), 64);
        assert_eq!(step(60, 7, 0, 1), 72);
        assert_eq!(step(60, -1, 0, 1), 59);
        assert_eq!(step(61, 0, 0, 1), 60);
        assert_eq!(step(57, 2, 9, 2), 60);
    }
}
