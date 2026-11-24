//! Parameter descriptors: key, range, default, how a knob maps to the value
//! and how the value reads.

use sat_doc::sat_time::PPQ;

/// What a parameter's number means.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Unit {
    /// A whole number.
    Int,
    /// One of several named choices (the value is the index).
    Choice(&'static [&'static str]),
    /// 0–1 shown as a percentage.
    Percent,
    /// Frequency in Hz (knob is logarithmic).
    Hz,
    /// Level in dB.
    Db,
    /// Time in seconds (knob is logarithmic).
    Seconds,
    /// Semitones.
    Semis,
    /// Cents.
    Cents,
    /// A MIDI pitch.
    Note,
    /// Pan, -1–1.
    Pan,
    /// A signed amount, -1–1, shown as a percentage.
    Bipolar,
}

/// One parameter of a module.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParamSpec {
    /// Parameter key.
    pub key: &'static str,
    /// Short label.
    pub label: &'static str,
    /// Lowest value.
    pub min: f32,
    /// Highest value.
    pub max: f32,
    /// Default.
    pub default: f32,
    /// What the number means.
    pub unit: Unit,
}

/// Shorthand constructor.
pub const fn p(key: &'static str, label: &'static str, min: f32, max: f32, default: f32, unit: Unit) -> ParamSpec {
    ParamSpec { key, label, min, max, default, unit }
}

/// Step lengths as labels for rate choices.
pub const RATE_LABELS: [&str; 4] = ["1/32", "1/16", "1/8", "1/4"];

/// A pitch as a lower-case name and octave, such as `c3`.
pub fn note_name(p: u8) -> String {
    const N: [&str; 12] = ["c", "c#", "d", "d#", "e", "f", "f#", "g", "g#", "a", "a#", "b"];
    format!("{}{}", N[(p % 12) as usize], p as i32 / 12 - 1)
}

impl ParamSpec {
    fn whole(&self) -> bool {
        matches!(self.unit, Unit::Int | Unit::Choice(_) | Unit::Note | Unit::Semis)
    }

    fn log(&self) -> bool {
        matches!(self.unit, Unit::Hz | Unit::Seconds) && self.min > 0.0
    }

    /// `v` clamped to the range (and rounded, for whole-number parameters).
    pub fn clamp(&self, v: f32) -> f32 {
        let v = if v.is_finite() { v.clamp(self.min, self.max) } else { self.default };
        if self.whole() { v.round() } else { v }
    }

    /// Knob position (0–1) for a value.
    pub fn pos(&self, v: f32) -> f64 {
        let v = self.clamp(v);
        let k = if self.log() {
            (v / self.min).ln() / (self.max / self.min).ln()
        } else {
            (v - self.min) / (self.max - self.min)
        };
        k.clamp(0.0, 1.0) as f64
    }

    /// The value at a knob position.
    pub fn at(&self, pos: f64) -> f32 {
        let k = pos.clamp(0.0, 1.0) as f32;
        let v =
            if self.log() { self.min * (self.max / self.min).powf(k) } else { self.min + k * (self.max - self.min) };
        if self.whole() {
            v.round()
        } else if self.log() {
            // three significant digits
            let mag = 10f32.powf(v.abs().log10().floor() - 2.0);
            (v / mag).round() * mag
        } else {
            (v * 100.0).round() / 100.0
        }
    }

    /// The value as an editor shows it.
    pub fn show(&self, v: f32) -> String {
        let v = self.clamp(v);
        match self.unit {
            Unit::Int => format!("{}", v as i64),
            Unit::Choice(names) => names.get(v as usize).copied().unwrap_or("?").to_string(),
            Unit::Percent => format!("{}%", (v * 100.0).round()),
            Unit::Bipolar => format!("{:+}%", (v * 100.0).round()),
            Unit::Hz if v >= 1000.0 => format!("{:.2}k", v / 1000.0),
            Unit::Hz if v < 10.0 => format!("{v:.1}hz"),
            Unit::Hz => format!("{}", v.round()),
            Unit::Db if v <= -60.0 => "-inf".into(),
            Unit::Db => format!("{v:.1} db"),
            Unit::Seconds if v < 1.0 => format!("{}ms", (v * 1000.0).round()),
            Unit::Seconds => format!("{v:.2}s"),
            Unit::Semis => format!("{:+} st", v as i64),
            Unit::Cents => format!("{:+} c", v.round() as i64),
            Unit::Note => note_name(v as u8),
            Unit::Pan if v.abs() < 0.005 => "c".into(),
            Unit::Pan if v < 0.0 => format!("{}l", (-v * 100.0).round()),
            Unit::Pan => format!("{}r", (v * 100.0).round()),
        }
    }
}

/// Ticks per step for a rate choice index.
pub fn rate_ticks(i: f32) -> i64 {
    [PPQ / 8, PPQ / 4, PPQ / 2, PPQ][(i.round().max(0.0) as usize).min(3)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knobs_map_and_values_read() {
        let hz = p("cutoff", "cutoff", 20.0, 20_000.0, 1200.0, Unit::Hz);
        assert!((hz.pos(632.46) - 0.5).abs() < 1e-3, "log knob");
        assert!((hz.at(hz.pos(1200.0)) - 1200.0).abs() < 1.0);
        assert_eq!(hz.show(1200.0), "1.20k");
        assert_eq!(hz.show(440.0), "440");
        let db = p("level", "level", -60.0, 6.0, 0.0, Unit::Db);
        assert_eq!(db.show(-3.0), "-3.0 db");
        assert_eq!(db.show(-60.0), "-inf");
        let s = p("a", "a", 0.001, 10.0, 0.01, Unit::Seconds);
        assert_eq!(s.show(0.01), "10ms");
        assert_eq!(s.show(1.5), "1.50s");
        let w = p("wave", "wave", 0.0, 3.0, 0.0, Unit::Choice(&["sine", "tri", "saw", "sqr"]));
        assert_eq!(w.show(2.2), "saw");
        assert_eq!(w.at(1.0), 3.0);
        let pan = p("pan", "pan", -1.0, 1.0, 0.0, Unit::Pan);
        assert_eq!((pan.show(0.0), pan.show(-0.5), pan.show(0.25)), ("c".into(), "50l".into(), "25r".into()));
        assert_eq!(p("t", "t", -24.0, 24.0, 0.0, Unit::Semis).show(-12.0), "-12 st");
        assert_eq!(rate_ticks(1.0), PPQ / 4);
        assert_eq!(hz.clamp(f32::NAN), 1200.0);
    }
}
