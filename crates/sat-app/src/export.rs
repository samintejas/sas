//! Export: renders the song (or the loop) offline, exactly as it plays, and
//! writes it as a WAV file.

use std::path::Path;

use sat_disk::Sounds;
use sat_doc::Document;
use sat_doc::sat_time::Ticks;

/// What to export.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Span {
    /// From the start to the last clip's end, then the tail until it falls silent.
    Song,
    /// The loop range, with no tail, so the file loops seamlessly.
    Loop,
}

/// Renders `span` of `doc` at `rate` and writes it to `path`. Returns the
/// length written, in seconds.
pub fn export(doc: &Document, sounds: &Sounds, rate: f64, span: Span, path: &Path) -> Result<f64, String> {
    let (from, to, tail) = match span {
        Span::Song => (Ticks::ZERO, sat_compile::song_end(doc), true),
        Span::Loop => (doc.loop_range.start, doc.loop_range.end, false),
    };
    if to <= from {
        return Err(match span {
            Span::Song => "the song is empty".into(),
            Span::Loop => "the loop range is empty".into(),
        });
    }
    let (l, r) = sat_compile::render_range(doc, sounds, rate, from, to, tail).map_err(|e| e.to_string())?;
    let secs = l.len() as f64 / rate;
    let sample = sat_disk::Sample { rate: rate as f32, l, r };
    sat_disk::encode(path, &sample).map_err(|e| e.to_string())?;
    Ok(secs)
}
