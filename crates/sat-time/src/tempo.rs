use crate::{PPQ, Ticks, TimeError};

/// How tempo moves from one point to the next.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TempoRamp {
    /// Hold this tempo until the next point, then jump.
    #[default]
    Step,
    /// Change linearly (in bpm over ticks) to reach the next point's tempo.
    Linear,
}

/// One point on master's tempo lane.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TempoPoint {
    /// Where this tempo starts.
    pub tick: Ticks,
    /// Beats (quarter notes) per minute.
    pub bpm: f64,
    /// How the tempo moves towards the next point.
    pub ramp: TempoRamp,
}

#[derive(Debug, Clone, Copy)]
struct Segment {
    start_tick: f64,
    /// Length in ticks; `f64::INFINITY` for the last segment.
    len: f64,
    start_sec: f64,
    bpm0: f64,
    /// bpm change per tick (0 for constant segments).
    slope: f64,
}

const SEC_PER_TICK_AT_1BPM: f64 = 60.0 / PPQ as f64;

impl Segment {
    fn secs_at(&self, x: f64) -> f64 {
        if self.slope == 0.0 {
            x * SEC_PER_TICK_AT_1BPM / self.bpm0
        } else {
            SEC_PER_TICK_AT_1BPM / self.slope * ((self.bpm0 + self.slope * x) / self.bpm0).ln()
        }
    }

    fn ticks_at(&self, s: f64) -> f64 {
        if self.slope == 0.0 {
            s * self.bpm0 / SEC_PER_TICK_AT_1BPM
        } else {
            self.bpm0 * ((s * self.slope / SEC_PER_TICK_AT_1BPM).exp() - 1.0) / self.slope
        }
    }

    fn bpm_at(&self, x: f64) -> f64 {
        self.bpm0 + self.slope * x
    }

    fn end_sec(&self) -> f64 {
        self.start_sec + self.secs_at(self.len)
    }
}

/// Converts between ticks, seconds and samples.
///
/// Built once from the tempo lane; every lookup after that is allocation-free.
#[derive(Debug, Clone)]
pub struct TempoMap {
    segs: Vec<Segment>,
}

impl TempoMap {
    /// A map with one fixed tempo.
    pub fn constant(bpm: f64) -> Result<TempoMap, TimeError> {
        TempoMap::new(&[TempoPoint { tick: Ticks::ZERO, bpm, ramp: TempoRamp::Step }])
    }

    /// Builds a map from tempo points. The first point must be at tick 0 and
    /// ticks must strictly increase.
    pub fn new(points: &[TempoPoint]) -> Result<TempoMap, TimeError> {
        if points.is_empty() || points[0].tick != Ticks::ZERO {
            return Err(TimeError::BadOrder);
        }
        for p in points {
            if !p.bpm.is_finite() || !(1.0..=999.0).contains(&p.bpm) {
                return Err(TimeError::BadTempo(p.bpm));
            }
        }
        if points.windows(2).any(|w| w[1].tick <= w[0].tick) {
            return Err(TimeError::BadOrder);
        }
        let mut segs = Vec::with_capacity(points.len());
        let mut start_sec = 0.0;
        for (i, p) in points.iter().enumerate() {
            let (len, slope) = match points.get(i + 1) {
                Some(n) => {
                    let len = (n.tick.0 - p.tick.0) as f64;
                    let slope = match p.ramp {
                        TempoRamp::Step => 0.0,
                        TempoRamp::Linear => (n.bpm - p.bpm) / len,
                    };
                    (len, slope)
                }
                None => (f64::INFINITY, 0.0),
            };
            let seg = Segment { start_tick: p.tick.0 as f64, len, start_sec, bpm0: p.bpm, slope };
            if len.is_finite() {
                start_sec = seg.end_sec();
            }
            segs.push(seg);
        }
        Ok(TempoMap { segs })
    }

    fn seg_for_tick(&self, t: f64) -> usize {
        // index of the last segment whose start is <= t (segment 0 also covers t < 0)
        self.segs.partition_point(|s| s.start_tick <= t).saturating_sub(1)
    }

    fn seg_for_sec(&self, s: f64) -> usize {
        self.segs.partition_point(|g| g.start_sec <= s).saturating_sub(1)
    }

    /// The tempo at a position, in bpm.
    pub fn bpm_at(&self, t: Ticks) -> f64 {
        let g = &self.segs[self.seg_for_tick(t.0 as f64)];
        g.bpm_at((t.0 as f64 - g.start_tick).max(0.0))
    }

    /// Seconds from tick 0 to `t` (fractional ticks allowed).
    pub fn tick_to_secs(&self, t: f64) -> f64 {
        let g = &self.segs[self.seg_for_tick(t)];
        g.start_sec + g.secs_at(t - g.start_tick)
    }

    /// The tick (fractional) at `s` seconds from tick 0.
    pub fn secs_to_tick(&self, s: f64) -> f64 {
        let g = &self.segs[self.seg_for_sec(s)];
        g.start_tick + g.ticks_at(s - g.start_sec)
    }

    /// The sample position of `t` at `sample_rate`, as a float.
    pub fn tick_to_sample(&self, t: Ticks, sample_rate: f64) -> f64 {
        self.tick_to_secs(t.0 as f64) * sample_rate
    }

    /// The fractional tick at sample position `n`.
    pub fn sample_to_tick(&self, n: f64, sample_rate: f64) -> f64 {
        self.secs_to_tick(n / sample_rate)
    }

    /// A cursor for fast lookups that move mostly forward, as the engine's do.
    pub fn cursor(&self) -> TempoCursor<'_> {
        TempoCursor { map: self, seg: 0 }
    }
}

/// Remembers the last segment used, so monotonic lookups are O(1).
#[derive(Debug, Clone)]
pub struct TempoCursor<'a> {
    map: &'a TempoMap,
    seg: usize,
}

impl TempoCursor<'_> {
    fn locate(&mut self, t: f64) -> &Segment {
        let segs = &self.map.segs;
        let s = &segs[self.seg];
        let inside = (t >= s.start_tick || self.seg == 0) && t < s.start_tick + s.len;
        if !inside {
            // try the next segment before falling back to a search
            let next = self.seg + 1;
            if next < segs.len() && t >= segs[next].start_tick && t < segs[next].start_tick + segs[next].len {
                self.seg = next;
            } else {
                self.seg = self.map.seg_for_tick(t);
            }
        }
        &segs[self.seg]
    }

    /// Seconds from tick 0 to `t`.
    pub fn tick_to_secs(&mut self, t: f64) -> f64 {
        let g = *self.locate(t);
        g.start_sec + g.secs_at(t - g.start_tick)
    }

    /// The sample position of `t` at `sample_rate`.
    pub fn tick_to_sample(&mut self, t: Ticks, sample_rate: f64) -> f64 {
        self.tick_to_secs(t.0 as f64) * sample_rate
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9 * a.abs().max(b.abs()).max(1.0)
    }

    #[test]
    fn constant_120() {
        let m = TempoMap::constant(120.0).unwrap();
        assert!(close(m.tick_to_secs(960.0), 0.5));
        assert!(close(m.tick_to_sample(Ticks::quarters(4), 48_000.0), 96_000.0));
        assert!(close(m.secs_to_tick(2.0), 3840.0));
        assert!(close(m.tick_to_secs(-960.0), -0.5));
    }

    #[test]
    fn step_change() {
        let m = TempoMap::new(&[
            TempoPoint { tick: Ticks(0), bpm: 120.0, ramp: TempoRamp::Step },
            TempoPoint { tick: Ticks::quarters(4), bpm: 60.0, ramp: TempoRamp::Step },
        ])
        .unwrap();
        assert!(close(m.tick_to_secs(Ticks::quarters(4).0 as f64), 2.0));
        assert!(close(m.tick_to_secs(Ticks::quarters(5).0 as f64), 3.0));
        assert_eq!(m.bpm_at(Ticks::quarters(5)), 60.0);
        assert!(close(m.secs_to_tick(3.0), Ticks::quarters(5).0 as f64));
    }

    #[test]
    fn linear_ramp_round_trips() {
        let m = TempoMap::new(&[
            TempoPoint { tick: Ticks(0), bpm: 100.0, ramp: TempoRamp::Linear },
            TempoPoint { tick: Ticks::quarters(8), bpm: 140.0, ramp: TempoRamp::Step },
        ])
        .unwrap();
        assert!(close(m.bpm_at(Ticks::quarters(4)), 120.0));
        // ramp duration lies between the durations at the two end tempos
        let d = m.tick_to_secs(Ticks::quarters(8).0 as f64);
        assert!(d < 8.0 * 60.0 / 100.0 && d > 8.0 * 60.0 / 140.0);
        for t in [0.0, 1.0, 500.0, 4000.0, 7679.0, 7680.0, 20_000.0] {
            assert!(close(m.secs_to_tick(m.tick_to_secs(t)), t), "t={t}");
        }
    }

    #[test]
    fn cursor_matches_map() {
        let m = TempoMap::new(&[
            TempoPoint { tick: Ticks(0), bpm: 90.0, ramp: TempoRamp::Linear },
            TempoPoint { tick: Ticks(5000), bpm: 150.0, ramp: TempoRamp::Step },
            TempoPoint { tick: Ticks(9000), bpm: 70.0, ramp: TempoRamp::Step },
        ])
        .unwrap();
        let mut c = m.cursor();
        for t in (0..12_000).step_by(37).chain([100, 9500, 0]) {
            assert!(close(c.tick_to_secs(t as f64), m.tick_to_secs(t as f64)));
        }
    }

    #[test]
    fn rejects_bad_input() {
        assert_eq!(TempoMap::new(&[]).unwrap_err(), TimeError::BadOrder);
        assert!(matches!(TempoMap::constant(0.0), Err(TimeError::BadTempo(_))));
        assert!(matches!(TempoMap::constant(f64::NAN), Err(TimeError::BadTempo(_))));
        let p = |t, b| TempoPoint { tick: Ticks(t), bpm: b, ramp: TempoRamp::Step };
        assert_eq!(TempoMap::new(&[p(10, 120.0)]).unwrap_err(), TimeError::BadOrder);
        assert_eq!(TempoMap::new(&[p(0, 120.0), p(0, 100.0)]).unwrap_err(), TimeError::BadOrder);
    }
}
