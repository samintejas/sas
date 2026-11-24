//! Musical time for sat.
//!
//! Musical positions are integer [`Ticks`] at [`PPQ`] (960) per quarter note.
//! A [`TempoMap`] converts ticks to seconds and samples and back; a
//! [`MeterMap`] converts ticks to bars and beats. Both are built off the audio
//! thread from master's tempo and time-signature lanes, and every lookup is
//! allocation-free so the engine can use them inside the audio callback.

#![cfg_attr(not(test), warn(missing_docs))]

mod meter;
mod quantize;
mod tempo;
mod ticks;

pub use meter::{BarBeat, MeterChange, MeterMap, TimeSig};
pub use quantize::{Grid, Rounding, quantize};
pub use tempo::{TempoCursor, TempoMap, TempoPoint, TempoRamp};
pub use ticks::{PPQ, Ticks};

/// Errors from building time maps.
#[derive(Debug, Clone, PartialEq)]
pub enum TimeError {
    /// A tempo outside the supported range (1–999 bpm) or not finite.
    BadTempo(f64),
    /// Points must be sorted by tick with unique ticks, and the first at tick 0.
    BadOrder,
    /// A time signature with a zero numerator or a denominator that is not a power of two.
    BadTimeSig(TimeSig),
    /// A meter change that does not fall on a bar line of the previous meter.
    OffBarLine(Ticks),
}

impl core::fmt::Display for TimeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            TimeError::BadTempo(b) => write!(f, "tempo {b} bpm is out of range"),
            TimeError::BadOrder => write!(f, "points must start at tick 0 and be strictly increasing"),
            TimeError::BadTimeSig(s) => write!(f, "bad time signature {}/{}", s.num, s.den),
            TimeError::OffBarLine(t) => write!(f, "meter change at tick {} is not on a bar line", t.0),
        }
    }
}

impl std::error::Error for TimeError {}
