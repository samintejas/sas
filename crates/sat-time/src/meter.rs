use crate::{PPQ, Ticks, TimeError};

/// A time signature, such as 4/4 or 7/8.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TimeSig {
    /// Beats per bar.
    pub num: u16,
    /// The note value of one beat (4 = quarter note).
    pub den: u16,
}

impl TimeSig {
    /// Common time.
    pub const FOUR_FOUR: TimeSig = TimeSig { num: 4, den: 4 };

    /// Length of one beat.
    pub fn beat_len(self) -> Ticks {
        Ticks(PPQ * 4 / self.den as i64)
    }

    /// Length of one bar.
    pub fn bar_len(self) -> Ticks {
        Ticks(self.beat_len().0 * self.num as i64)
    }

    fn validate(self) -> Result<(), TimeError> {
        let ok = self.num > 0 && self.den.is_power_of_two() && self.den <= 64;
        if ok { Ok(()) } else { Err(TimeError::BadTimeSig(self)) }
    }
}

/// A time-signature change on master's lane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct MeterChange {
    /// Where the new signature starts; must fall on a bar line.
    pub tick: Ticks,
    /// The signature from here on.
    pub sig: TimeSig,
}

/// A position as bar, beat and remaining ticks, all counted from zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BarBeat {
    /// Bar index (bar 1 in the UI is 0 here).
    pub bar: i64,
    /// Beat within the bar.
    pub beat: u16,
    /// Ticks past the beat.
    pub tick: Ticks,
}

#[derive(Debug, Clone, Copy)]
struct Span {
    tick: Ticks,
    bar: i64,
    sig: TimeSig,
}

/// Converts ticks to bars and beats.
#[derive(Debug, Clone)]
pub struct MeterMap {
    spans: Vec<Span>,
}

impl Default for MeterMap {
    fn default() -> Self {
        MeterMap { spans: vec![Span { tick: Ticks::ZERO, bar: 0, sig: TimeSig::FOUR_FOUR }] }
    }
}

impl MeterMap {
    /// Builds a map. The first change must be at tick 0, ticks strictly
    /// increase, and every change lands on a bar line of the meter before it.
    pub fn new(changes: &[MeterChange]) -> Result<MeterMap, TimeError> {
        if changes.is_empty() || changes[0].tick != Ticks::ZERO {
            return Err(TimeError::BadOrder);
        }
        let mut spans: Vec<Span> = Vec::with_capacity(changes.len());
        for c in changes {
            c.sig.validate()?;
            let bar = match spans.last() {
                None => 0,
                Some(prev) => {
                    let d = c.tick.0 - prev.tick.0;
                    let bl = prev.sig.bar_len().0;
                    if d <= 0 {
                        return Err(TimeError::BadOrder);
                    }
                    if d % bl != 0 {
                        return Err(TimeError::OffBarLine(c.tick));
                    }
                    prev.bar + d / bl
                }
            };
            spans.push(Span { tick: c.tick, bar, sig: c.sig });
        }
        Ok(MeterMap { spans })
    }

    fn span_for_tick(&self, t: Ticks) -> &Span {
        let i = self.spans.partition_point(|s| s.tick <= t).saturating_sub(1);
        &self.spans[i]
    }

    /// The signature in force at `t`.
    pub fn sig_at(&self, t: Ticks) -> TimeSig {
        self.span_for_tick(t).sig
    }

    /// Splits `t` into bar, beat and ticks.
    pub fn bar_beat(&self, t: Ticks) -> BarBeat {
        let s = self.span_for_tick(t);
        let rel = t.0 - s.tick.0;
        let bl = s.sig.bar_len().0;
        let bar_off = rel.div_euclid(bl);
        let in_bar = rel.rem_euclid(bl);
        let beat_len = s.sig.beat_len().0;
        BarBeat { bar: s.bar + bar_off, beat: (in_bar / beat_len) as u16, tick: Ticks(in_bar % beat_len) }
    }

    /// The tick where bar `bar` starts.
    pub fn bar_start(&self, bar: i64) -> Ticks {
        let i = self.spans.partition_point(|s| s.bar <= bar).saturating_sub(1);
        let s = &self.spans[i];
        Ticks(s.tick.0 + (bar - s.bar) * s.sig.bar_len().0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_four() {
        let m = MeterMap::default();
        assert_eq!(m.bar_beat(Ticks(0)), BarBeat { bar: 0, beat: 0, tick: Ticks(0) });
        assert_eq!(m.bar_beat(Ticks(3840 + 960 + 5)), BarBeat { bar: 1, beat: 1, tick: Ticks(5) });
        assert_eq!(m.bar_start(3), Ticks(3 * 3840));
    }

    #[test]
    fn changes() {
        let m = MeterMap::new(&[
            MeterChange { tick: Ticks(0), sig: TimeSig::FOUR_FOUR },
            MeterChange { tick: Ticks(2 * 3840), sig: TimeSig { num: 7, den: 8 } },
        ])
        .unwrap();
        // 7/8 bar = 7 * 480 = 3360 ticks
        assert_eq!(m.bar_start(2), Ticks(7680));
        assert_eq!(m.bar_start(3), Ticks(7680 + 3360));
        assert_eq!(m.bar_beat(Ticks(7680 + 3360 + 480 * 6)), BarBeat { bar: 3, beat: 6, tick: Ticks(0) });
        assert_eq!(m.sig_at(Ticks(7679)), TimeSig::FOUR_FOUR);
    }

    #[test]
    fn rejects_off_bar() {
        let r = MeterMap::new(&[
            MeterChange { tick: Ticks(0), sig: TimeSig::FOUR_FOUR },
            MeterChange { tick: Ticks(960), sig: TimeSig { num: 3, den: 4 } },
        ]);
        assert_eq!(r.unwrap_err(), TimeError::OffBarLine(Ticks(960)));
        let r = MeterMap::new(&[MeterChange { tick: Ticks(0), sig: TimeSig { num: 3, den: 5 } }]);
        assert!(matches!(r, Err(TimeError::BadTimeSig(_))));
    }
}
