//! The bottom panel for an audio clip: its comp drawn large, with fields
//! for gain, fades and crossfade time.
//!
//! Fades are stored in ticks but shown and edited in milliseconds, measured
//! on the clip's first placement (audio does not follow the tempo).

use sat_doc::sat_time::{PPQ, TempoMap, Ticks};
use sat_doc::*;
use sat_theme::{Rgba, Theme};
use vello::kurbo::{Point, Rect};

use crate::arrange::{clip_secs, clip_tick_at};
use crate::paint::{Align, Painter, TextStyle};
use crate::panel::TABS_H;
use crate::piano_roll::{TOOLS_H, clip_bar};

/// An editable value of an audio clip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioField {
    /// Gain in dB.
    Gain,
    /// Fade in, in ms.
    FadeIn,
    /// Fade out, in ms.
    FadeOut,
    /// Crossfade at comp boundaries, in ms.
    Xfade,
}

impl AudioField {
    /// Every field, in panel order.
    pub const ALL: [AudioField; 4] = [AudioField::Gain, AudioField::FadeIn, AudioField::FadeOut, AudioField::Xfade];

    /// How far one pixel of vertical drag moves the value.
    pub fn per_px(self) -> f64 {
        match self {
            AudioField::Gain => 0.1,
            AudioField::FadeIn | AudioField::FadeOut => 5.0,
            AudioField::Xfade => 1.0,
        }
    }

    /// The value a double click resets to.
    pub fn default_value(self) -> f64 {
        match self {
            AudioField::Xfade => 20.0,
            _ => 0.0,
        }
    }

    fn label(self) -> &'static str {
        match self {
            AudioField::Gain => "gain",
            AudioField::FadeIn => "fade in",
            AudioField::FadeOut => "fade out",
            AudioField::Xfade => "xfade",
        }
    }
}

/// The panel laid out.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioEditLayout {
    /// The fields.
    pub fields: Vec<(AudioField, Rect)>,
    /// The waveform.
    pub wave: Rect,
}

impl AudioEditLayout {
    /// The panel in `r` (the whole bottom panel).
    pub fn new(r: Rect) -> AudioEditLayout {
        let cy = r.y0 + TABS_H + TOOLS_H / 2.0;
        let mut x = r.x0 + 12.0;
        let fields = AudioField::ALL
            .map(|f| {
                let w = if f == AudioField::Gain { 118.0 } else { 136.0 };
                let rect = Rect::new(x, cy - 11.0, x + w, cy + 11.0);
                x += w + 8.0;
                (f, rect)
            })
            .to_vec();
        let wave = Rect::new(r.x0 + 12.0, r.y0 + TABS_H + TOOLS_H + 8.0, r.x1 - 12.0, r.y1 - 12.0);
        AudioEditLayout { fields, wave }
    }

    /// The field under `pt`.
    pub fn hit(&self, pt: Point) -> Option<AudioField> {
        self.fields.iter().find(|(_, r)| r.contains(pt)).map(|(f, _)| *f)
    }
}

/// The placement fades are measured on: the clip's first, else one at 0.
fn placement(doc: &Document, clip: &Clip) -> Placement {
    doc.placements.values().filter(|p| p.clip == clip.id).min_by_key(|p| p.start).cloned().unwrap_or(Placement {
        id: PlacementId::from_u128(0),
        clip: clip.id,
        start: Ticks::ZERO,
        offset: Ticks::ZERO,
        length: clip.length,
    })
}

/// A field's value (dB or ms).
pub fn value(doc: &Document, clip: ClipId, f: AudioField) -> Option<f64> {
    let c = doc.clips.get(&clip)?;
    let a = c.audio.as_ref()?;
    let tempo = doc.tempo_map().ok()?;
    let p = placement(doc, c);
    let ms = |from: i64, to: i64| (clip_secs(&tempo, &p, to as f64) - clip_secs(&tempo, &p, from as f64)) * 1000.0;
    Some(match f {
        AudioField::Gain => a.gain_db as f64,
        AudioField::FadeIn => ms(0, a.fade_in.0),
        AudioField::FadeOut => ms(c.length.0 - a.fade_out.0, c.length.0),
        AudioField::Xfade => a.xfade_ms as f64,
    })
}

/// The clip's audio with field `f` set to `v` (clamped to what it allows).
pub fn with_value(doc: &Document, clip: ClipId, f: AudioField, v: f64) -> Option<AudioClip> {
    let c = doc.clips.get(&clip)?;
    let mut a = c.audio.clone()?;
    let tempo = doc.tempo_map().ok()?;
    let p = placement(doc, c);
    let len = c.length.0;
    match f {
        AudioField::Gain => a.gain_db = v.clamp(-48.0, 12.0) as f32,
        AudioField::FadeIn => {
            let t = clip_tick_at(&tempo, &p, v.max(0.0) / 1000.0).round() as i64;
            a.fade_in = Ticks(t.clamp(0, len - a.fade_out.0));
        }
        AudioField::FadeOut => {
            let end = clip_secs(&tempo, &p, len as f64);
            let t = clip_tick_at(&tempo, &p, end - v.max(0.0) / 1000.0).round() as i64;
            a.fade_out = Ticks((len - t).clamp(0, len - a.fade_in.0));
        }
        AudioField::Xfade => a.xfade_ms = v.clamp(0.0, 500.0) as f32,
    }
    Some(a)
}

fn field_text(f: AudioField, v: f64) -> String {
    match f {
        AudioField::Gain => format!("{:+.1} db", v),
        _ => format!("{:.0} ms", v),
    }
}

/// Draws the panel for audio clip `clip`, with its sounds from `sounds`.
pub fn draw(
    p: &mut Painter<'_>,
    th: &Theme,
    doc: &Document,
    clip: ClipId,
    sounds: &sat_disk::Pool,
    r: Rect,
) -> Option<AudioEditLayout> {
    let c = &th.c;
    let cl = clip_bar(p, th, doc, Some(clip), r)?;
    let a = cl.audio.as_ref()?;
    let tempo = doc.tempo_map().ok()?;
    let l = AudioEditLayout::new(r);
    for (f, fr) in &l.fields {
        p.rrect(*fr, 5.0, c.bg_control);
        p.rrect_stroke(fr.inset(-0.5), 5.0, 1.0, c.border_strong.alpha(0.6));
        let cy = fr.center().y;
        p.text(fr.x0 + 10.0, cy, f.label(), &TextStyle::mono(12.0, c.text_secondary), Align::Left);
        let v = value(doc, clip, *f).unwrap_or(0.0);
        p.text(fr.x1 - 10.0, cy, &field_text(*f, v), &TextStyle::mono(12.0, c.text_max), Align::Right);
    }
    let cy = l.fields[0].1.center().y;
    let secs = a
        .takes
        .iter()
        .filter_map(|t| doc.audio.get(&t.audio))
        .map(|f| f.frames as f64 / f.rate.max(1) as f64)
        .fold(0.0, f64::max);
    let secs = secs.round() as u64;
    let info = format!("{} takes · {}:{:02}", a.takes.len(), secs / 60, secs % 60);
    p.text(r.x1 - 16.0, cy, &info, &TextStyle::mono(12.0, c.text_secondary), Align::Right);
    draw_wave(p, th, doc, cl, a, &tempo, sounds, l.wave);
    Some(l)
}

#[allow(clippy::too_many_arguments)]
fn draw_wave(
    p: &mut Painter<'_>,
    th: &Theme,
    doc: &Document,
    cl: &Clip,
    a: &AudioClip,
    tempo: &TempoMap,
    sounds: &sat_disk::Pool,
    w: Rect,
) {
    let c = &th.c;
    let col = th.track(cl.color.0);
    let pl = placement(doc, cl);
    let len = cl.length.0.max(1) as f64;
    let x_of = |ct: f64| w.x0 + ct / len * w.width();
    let ct_of = |x: f64| (x - w.x0) / w.width() * len;
    p.rect(w, Rgba::hex(0x0a0a0a));
    p.clip(w, |p| {
        // beats and bars of the clip
        let mut k = 0;
        while (k * PPQ) as f64 <= len {
            let x = x_of((k * PPQ) as f64);
            let bar = k % 4 == 0;
            p.vline(x, w.y0, w.y1, if bar { c.border_default } else { Rgba::new(255, 255, 255, 8) });
            if bar {
                p.text(
                    x + 5.0,
                    w.y0 + 10.0,
                    &(k / 4 + 1).to_string(),
                    &TextStyle::mono(11.0, c.text_primary),
                    Align::Left,
                );
            }
            k += 1;
        }
        let gain = 10f32.powf(a.gain_db / 20.0);
        let (cy, half) = (w.center().y + 6.0, (w.height() - 32.0) / 2.0);
        let mut x = w.x0;
        while x < w.x1 {
            let (ct0, ct1) = (ct_of(x), ct_of(x + 1.0));
            let take = a.take_at(Ticks(ct0.floor() as i64)).and_then(|i| a.takes.get(i as usize));
            let peak = take.and_then(|t| {
                let ov = sounds.overview(t.audio)?;
                let rate = sounds.get(t.audio)?.rate as f64;
                let f = |ct: f64| t.offset as f64 + clip_secs(tempo, &pl, ct) * rate;
                Some(sat_disk::overview_range(ov, f(ct0), f(ct1)))
            });
            if let Some((lo, hi)) = peak {
                let (lo, hi) = ((lo * gain).max(-1.0) as f64, (hi * gain).min(1.0) as f64);
                p.rect(Rect::new(x, (cy - hi * half).min(cy - 0.5), x + 1.0, (cy - lo * half).max(cy + 0.5)), col);
            }
            x += 1.0;
        }
        // comp boundaries and take names
        for (k, s) in a.comp.iter().enumerate() {
            let x = x_of(s.start.0 as f64);
            if k > 0 {
                p.vline(x, w.y0 + 20.0, w.y1, c.text_max.alpha(0.6));
            }
            if let Some(t) = a.takes.get(s.take as usize) {
                let tag = Rect::new(x + 4.0, w.y1 - 22.0, x + 60.0, w.y1 - 6.0);
                p.rrect(tag, 3.0, col.mix(Rgba::hex(0x111111), 0.35));
                p.text(tag.x0 + 6.0, tag.center().y, &t.name, &TextStyle::mono(10.5, c.text_max), Align::Left);
            }
        }
        // fades
        let (top, bot) = (w.y0 + 22.0, w.y1 - 1.0);
        let curve = |p: &mut Painter<'_>, a0: Point, b0: Point| {
            let pts: Vec<Point> = (0..=24)
                .map(|k| {
                    let u = k as f64 / 24.0;
                    let g = (u * core::f64::consts::FRAC_PI_2).sin();
                    Point::new(a0.x + (b0.x - a0.x) * u, a0.y + (b0.y - a0.y) * g)
                })
                .collect();
            p.polyline(&pts, 1.5, c.text_max);
        };
        let fin = x_of(a.fade_in.0 as f64);
        let fout = x_of((cl.length.0 - a.fade_out.0) as f64);
        if a.fade_in.0 > 0 {
            curve(p, Point::new(w.x0, bot), Point::new(fin, top));
        }
        if a.fade_out.0 > 0 {
            curve(p, Point::new(w.x1, bot), Point::new(fout, top));
        }
        for x in [fin, fout] {
            p.rect(Rect::new(x - 4.0, top - 4.0, x + 4.0, top + 4.0), c.text_max);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_read_and_write_in_ms_and_db() {
        let d = crate::demo::demo();
        let vox = d.doc.clips.values().find(|c| c.name == "vox").unwrap().id;
        // 124 bpm: a sixteenth is about 121 ms
        let fin = value(&d.doc, vox, AudioField::FadeIn).unwrap();
        assert!((fin - 120.97).abs() < 0.1, "{fin}");
        let a = with_value(&d.doc, vox, AudioField::FadeIn, 500.0).unwrap();
        let ms = a.fade_in.0 as f64 / 960.0 * 60_000.0 / 124.0;
        assert!((ms - 500.0).abs() < 1.0, "{ms}");
        let a = with_value(&d.doc, vox, AudioField::Gain, 99.0).unwrap();
        assert_eq!(a.gain_db, 12.0, "clamped");
        let a = with_value(&d.doc, vox, AudioField::Xfade, -5.0).unwrap();
        assert_eq!(a.xfade_ms, 0.0);
        // fades never overlap
        let a = with_value(&d.doc, vox, AudioField::FadeOut, 1e6).unwrap();
        assert_eq!(a.fade_out.0 + a.fade_in.0, d.doc.clips[&vox].length.0);
        let l = AudioEditLayout::new(Rect::new(0.0, 0.0, 1400.0, 300.0));
        assert_eq!(l.hit(l.fields[2].1.center()), Some(AudioField::FadeOut));
        assert_eq!(l.hit(l.wave.center()), None);
    }
}
