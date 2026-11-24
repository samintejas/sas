//! Value laws and small widgets shared by the arrange headers and the mixer.
//!
//! Faders, send knobs and meters all use one dB law ([`fader_pos`]), so a
//! meter reading lines up with the fader cap at the same level.

use sat_theme::{Rgba, Theme};
use vello::kurbo::{Point, Rect};

use crate::paint::{Align, Painter, TextStyle};

/// The top of fader travel, in dB.
pub const FADER_MAX_DB: f32 = 6.0;
/// The lowest level a fader shows as a number; below it reads "-inf".
pub const FADER_MIN_DB: f32 = -60.0;
/// The level a fader at the very bottom sets: silent, but finite.
pub const SILENT_DB: f32 = -120.0;

/// Fader position (0 bottom, 1 top) for a level in dB.
pub fn fader_pos(db: f32) -> f64 {
    if db <= FADER_MIN_DB {
        return 0.0;
    }
    let k = ((db - FADER_MIN_DB) / (FADER_MAX_DB - FADER_MIN_DB)).clamp(0.0, 1.0) as f64;
    k * k
}

/// The level in dB for a fader position; the inverse of [`fader_pos`].
/// The very bottom of travel is [`SILENT_DB`].
pub fn fader_db(pos: f64) -> f32 {
    if pos <= 1e-6 {
        return SILENT_DB;
    }
    let k = pos.clamp(0.0, 1.0).sqrt() as f32;
    // round to 0.1 dB so dragged values read cleanly
    ((FADER_MIN_DB + k * (FADER_MAX_DB - FADER_MIN_DB)) * 10.0).round() / 10.0
}

/// A level as the mixer shows it: one decimal with a sign above 0 dB, or "-inf".
pub fn fmt_db(db: f32) -> String {
    let r = (db * 10.0).round() / 10.0;
    if db <= FADER_MIN_DB {
        "-inf".into()
    } else if r > 0.0 {
        format!("+{r:.1}")
    } else if r == 0.0 {
        "0.0".into()
    } else {
        format!("{r:.1}")
    }
}

/// Meter height (0–1) for a linear peak, on the fader law.
pub fn meter_pos(peak: f32) -> f64 {
    if peak <= 0.0 { 0.0 } else { fader_pos(20.0 * peak.log10()) }
}

/// Knob position (0–1) for a pan value (-1–1).
pub fn pan_pos(pan: f32) -> f64 {
    ((pan.clamp(-1.0, 1.0) + 1.0) / 2.0) as f64
}

/// Pan value for a knob position, snapping to centre near the middle.
pub fn pan_at(pos: f64) -> f32 {
    let v = (pos.clamp(0.0, 1.0) * 2.0 - 1.0) as f32;
    if v.abs() < 0.02 { 0.0 } else { (v * 100.0).round() / 100.0 }
}

/// A rotary knob of diameter `size` centred at `c`, showing `value` (0–1)
/// as an arc in `color`.
pub fn knob(p: &mut Painter<'_>, c: Point, size: f64, value: f64, color: Rgba) {
    use std::f64::consts::PI;
    let r = size * 11.0 / 30.0;
    let start = 0.75 * PI; // 7:30, clockwise through 12 to 4:30
    let sweep = 1.5 * PI;
    p.arc(c, r, start, sweep, 2.2, Rgba::hex(0x2c2c2c));
    p.arc(c, r, start, sweep * value.clamp(0.0, 1.0), 2.2, color);
    let cap = size * 8.0 / 30.0;
    p.circle(c, cap, Rgba::hex(0x2a2a2a));
    p.circle_stroke(c, cap, 1.0, Rgba::hex(0x050505));
    let a = start + sweep * value.clamp(0.0, 1.0);
    let (s, co) = a.sin_cos();
    let (r0, r1) = (cap * 0.42, cap * 0.92);
    p.line(Point::new(c.x + co * r0, c.y + s * r0), Point::new(c.x + co * r1, c.y + s * r1), 1.6, Rgba::hex(0xececec));
}

/// A two-bar stereo meter filling `r` (bars side by side, 2 px apart).
pub fn meter(p: &mut Painter<'_>, r: Rect, l: f64, rr: f64) {
    let bw = ((r.width() - 2.0) / 2.0).max(1.0);
    let stops = [(0.0, Rgba::hex(0x4caf50)), (0.75, Rgba::hex(0xc9c040)), (1.0, Rgba::hex(0xe05a5a))];
    for (i, v) in [l, rr].into_iter().enumerate() {
        let x = r.x0 + i as f64 * (bw + 2.0);
        let full = Rect::new(x, r.y0, x + bw, r.y1);
        p.rrect(full, 1.0, Rgba::hex(0x161616));
        let h = r.height() * v.clamp(0.0, 1.0);
        p.vgradient(Rect::new(x, r.y1 - h, x + bw, r.y1), full, 1.0, &stops);
    }
}

/// How a toggle button looks when it is on.
#[derive(Debug, Clone, Copy)]
pub enum On {
    /// Solid fill, dark label (mixer mute and solo).
    Solid(Rgba),
    /// Tinted fill and border, coloured label (arrange headers).
    Tint(Rgba),
}

/// A small labelled toggle button.
pub fn toggle(p: &mut Painter<'_>, th: &Theme, r: Rect, label: &str, on: Option<On>) {
    let cy = r.center().y;
    match on {
        Some(On::Solid(c)) => {
            p.rrect(r, 4.0, c);
            p.text(r.center().x, cy, label, &TextStyle::mono(11.0, Rgba::hex(0x0a0a0a)).medium(), Align::Center);
        }
        Some(On::Tint(c)) => {
            p.rrect(r, 3.0, c.alpha(0.16));
            p.rrect_stroke(r.inset(-0.5), 3.0, 1.0, c.alpha(0.6));
            p.text(r.center().x, cy, label, &TextStyle::mono(11.0, c), Align::Center);
        }
        None => {
            p.rrect(r, 4.0, Rgba::hex(0x1f1f1f));
            p.rrect_stroke(r.inset(-0.5), 4.0, 1.0, Rgba::new(255, 255, 255, 36));
            p.text(r.center().x, cy, label, &TextStyle::mono(11.0, th.c.text_secondary).medium(), Align::Center);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fader_law_round_trips_and_is_monotonic() {
        for db in [-59.0f32, -24.0, -12.0, -6.0, -3.0, 0.0, 3.5, 6.0] {
            assert!((fader_db(fader_pos(db)) - db).abs() <= 0.05, "{db}");
        }
        let mut last = -1.0;
        for i in 0..=100 {
            let p = fader_pos(-70.0 + i as f32 * 0.8);
            assert!(p >= last);
            last = p;
        }
        assert_eq!(fader_db(0.0), SILENT_DB);
        assert_eq!(fader_pos(SILENT_DB), 0.0);
        assert_eq!(fader_pos(FADER_MAX_DB), 1.0);
        // 0 dB sits a little above three quarters of the travel
        assert!((0.8..0.85).contains(&fader_pos(0.0)));
    }

    #[test]
    fn meters_use_the_fader_law() {
        assert_eq!(meter_pos(0.0), 0.0);
        assert!((meter_pos(1.0) - fader_pos(0.0)).abs() < 1e-9);
        assert!((meter_pos(0.5) - fader_pos(-6.0206)).abs() < 1e-4);
    }

    #[test]
    fn pan_snaps_to_centre() {
        assert_eq!(pan_at(0.505), 0.0);
        assert_eq!(pan_at(0.0), -1.0);
        assert_eq!(pan_at(1.0), 1.0);
        assert_eq!(pan_pos(0.0), 0.5);
        assert_eq!(fmt_db(-80.0), "-inf");
        assert_eq!(fmt_db(-3.0), "-3.0");
        assert_eq!(fmt_db(1.4), "+1.4");
        assert_eq!(fmt_db(0.0), "0.0");
        assert_eq!(fmt_db(-0.01), "0.0");
    }
}
