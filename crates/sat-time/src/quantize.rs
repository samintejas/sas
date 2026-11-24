use crate::{PPQ, Ticks};

/// A quantisation grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Grid {
    /// No quantisation.
    Off,
    /// A fixed step in ticks.
    Step(Ticks),
}

impl Grid {
    /// A grid of 1/`n` notes (4 = quarters, 16 = sixteenths). `n` must divide 4·[`PPQ`].
    pub const fn note(n: i64) -> Grid {
        Grid::Step(Ticks(PPQ * 4 / n))
    }

    /// A grid of 1/`n` note triplets.
    pub const fn triplet(n: i64) -> Grid {
        Grid::Step(Ticks(PPQ * 4 * 2 / (n * 3)))
    }
}

/// How [`quantize`] picks a grid line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rounding {
    /// The nearest line; halfway rounds later.
    Nearest,
    /// The line at or before.
    Floor,
    /// The line at or after.
    Ceil,
}

/// Moves `t` onto `grid`, measured from `origin`.
pub fn quantize(t: Ticks, grid: Grid, origin: Ticks, rounding: Rounding) -> Ticks {
    let step = match grid {
        Grid::Off => return t,
        Grid::Step(s) if s.0 <= 0 => return t,
        Grid::Step(s) => s.0,
    };
    let rel = t.0 - origin.0;
    let floor = rel.div_euclid(step) * step;
    let q = match rounding {
        Rounding::Floor => floor,
        Rounding::Ceil => {
            if floor == rel {
                floor
            } else {
                floor + step
            }
        }
        Rounding::Nearest => {
            if (rel - floor) * 2 >= step {
                floor + step
            } else {
                floor
            }
        }
    };
    Ticks(origin.0 + q)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grids() {
        assert_eq!(Grid::note(4), Grid::Step(Ticks(960)));
        assert_eq!(Grid::note(16), Grid::Step(Ticks(240)));
        assert_eq!(Grid::triplet(8), Grid::Step(Ticks(320)));
    }

    #[test]
    fn rounding() {
        let g = Grid::note(16);
        assert_eq!(quantize(Ticks(119), g, Ticks::ZERO, Rounding::Nearest), Ticks(0));
        assert_eq!(quantize(Ticks(120), g, Ticks::ZERO, Rounding::Nearest), Ticks(240));
        assert_eq!(quantize(Ticks(239), g, Ticks::ZERO, Rounding::Floor), Ticks(0));
        assert_eq!(quantize(Ticks(1), g, Ticks::ZERO, Rounding::Ceil), Ticks(240));
        assert_eq!(quantize(Ticks(240), g, Ticks::ZERO, Rounding::Ceil), Ticks(240));
        assert_eq!(quantize(Ticks(-1), g, Ticks::ZERO, Rounding::Floor), Ticks(-240));
        assert_eq!(quantize(Ticks(130), g, Ticks(10), Rounding::Floor), Ticks(10));
        assert_eq!(quantize(Ticks(77), Grid::Off, Ticks::ZERO, Rounding::Floor), Ticks(77));
    }
}
