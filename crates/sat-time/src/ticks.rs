use core::ops::{Add, AddAssign, Neg, Sub, SubAssign};

/// Ticks per quarter note.
pub const PPQ: i64 = 960;

/// A musical position or duration in ticks ([`PPQ`] per quarter note).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct Ticks(pub i64);

impl Ticks {
    /// Tick zero.
    pub const ZERO: Ticks = Ticks(0);
    /// One quarter note.
    pub const QUARTER: Ticks = Ticks(PPQ);

    /// `n` quarter notes.
    pub const fn quarters(n: i64) -> Ticks {
        Ticks(n * PPQ)
    }

    /// `n` sixteenth notes.
    pub const fn sixteenths(n: i64) -> Ticks {
        Ticks(n * PPQ / 4)
    }

    /// This position in quarter notes, as a float.
    pub fn as_quarters(self) -> f64 {
        self.0 as f64 / PPQ as f64
    }

    /// The smaller of two positions.
    pub fn min(self, o: Ticks) -> Ticks {
        Ticks(self.0.min(o.0))
    }

    /// The larger of two positions.
    pub fn max(self, o: Ticks) -> Ticks {
        Ticks(self.0.max(o.0))
    }
}

impl Add for Ticks {
    type Output = Ticks;
    fn add(self, o: Ticks) -> Ticks {
        Ticks(self.0 + o.0)
    }
}

impl Sub for Ticks {
    type Output = Ticks;
    fn sub(self, o: Ticks) -> Ticks {
        Ticks(self.0 - o.0)
    }
}

impl AddAssign for Ticks {
    fn add_assign(&mut self, o: Ticks) {
        self.0 += o.0;
    }
}

impl SubAssign for Ticks {
    fn sub_assign(&mut self, o: Ticks) {
        self.0 -= o.0;
    }
}

impl Neg for Ticks {
    type Output = Ticks;
    fn neg(self) -> Ticks {
        Ticks(-self.0)
    }
}
