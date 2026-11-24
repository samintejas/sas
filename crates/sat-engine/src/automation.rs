//! Breakpoint automation evaluated at sample positions.

/// Segment shape from one point to the next.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AutoCurve {
    /// Hold, then jump.
    Hold,
    /// Straight line.
    Linear,
    /// Bent line, tension -1 to 1: positive starts slow and ends fast.
    Bend(f32),
}

/// One breakpoint at a timeline sample.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AutoPoint {
    /// Timeline sample.
    pub at: i64,
    /// Value.
    pub value: f32,
    /// Shape towards the next point.
    pub curve: AutoCurve,
}

/// A compiled automation lane: points in samples, sorted.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Automation {
    points: Vec<AutoPoint>,
}

impl Automation {
    /// Builds a lane; points are sorted by position.
    pub fn new(mut points: Vec<AutoPoint>) -> Automation {
        points.sort_by_key(|p| p.at);
        Automation { points }
    }

    /// The value at `pos`, or `None` outside the lane's data (the value then
    /// falls back to the next layer down: snapshot or base).
    pub fn value_at(&self, pos: i64) -> Option<f32> {
        let first = self.points.first()?;
        let last = self.points.last()?;
        if pos < first.at || pos > last.at {
            return None;
        }
        let i = self.points.partition_point(|p| p.at <= pos) - 1;
        let a = self.points[i];
        let Some(b) = self.points.get(i + 1) else { return Some(a.value) };
        let x = (pos - a.at) as f32 / (b.at - a.at).max(1) as f32;
        let shaped = match a.curve {
            AutoCurve::Hold => return Some(a.value),
            AutoCurve::Linear => x,
            AutoCurve::Bend(t) => x.powf(2f32.powf(3.0 * t.clamp(-1.0, 1.0))),
        };
        Some(a.value + (b.value - a.value) * shaped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evaluates() {
        let l = Automation::new(vec![
            AutoPoint { at: 100, value: 0.0, curve: AutoCurve::Linear },
            AutoPoint { at: 200, value: 1.0, curve: AutoCurve::Hold },
            AutoPoint { at: 300, value: 0.5, curve: AutoCurve::Linear },
        ]);
        assert_eq!(l.value_at(99), None);
        assert_eq!(l.value_at(150), Some(0.5));
        assert_eq!(l.value_at(250), Some(1.0));
        assert_eq!(l.value_at(300), Some(0.5));
        assert_eq!(l.value_at(301), None);
        let bent = Automation::new(vec![
            AutoPoint { at: 0, value: 0.0, curve: AutoCurve::Bend(1.0) },
            AutoPoint { at: 100, value: 1.0, curve: AutoCurve::Linear },
        ]);
        assert!(bent.value_at(50).unwrap() < 0.5);
    }
}
