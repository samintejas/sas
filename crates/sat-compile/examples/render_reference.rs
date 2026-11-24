//! Renders the reference song from the tests to a 32-bit float WAV, so you
//! can listen to what the golden tests check.
//!
//! `cargo run -p sat-compile --example render_reference -- out.wav`

#[path = "../tests/common/mod.rs"]
mod common;

use std::io::Write;

fn main() -> std::io::Result<()> {
    let path = std::env::args().nth(1).unwrap_or_else(|| "reference_song.wav".into());
    let s = common::song();
    let (l, r) = common::render(s.h.doc(), 8 * 24_000);
    let data_len = (l.len() * 2 * 4) as u32;
    let mut f = std::io::BufWriter::new(std::fs::File::create(&path)?);
    f.write_all(b"RIFF")?;
    f.write_all(&(36 + data_len).to_le_bytes())?;
    f.write_all(b"WAVEfmt ")?;
    // fmt chunk: IEEE float, stereo
    f.write_all(&16u32.to_le_bytes())?;
    f.write_all(&3u16.to_le_bytes())?;
    f.write_all(&2u16.to_le_bytes())?;
    let sr = common::SR as u32;
    f.write_all(&sr.to_le_bytes())?;
    f.write_all(&(sr * 8).to_le_bytes())?;
    f.write_all(&8u16.to_le_bytes())?;
    f.write_all(&32u16.to_le_bytes())?;
    f.write_all(b"data")?;
    f.write_all(&data_len.to_le_bytes())?;
    for (a, b) in l.iter().zip(&r) {
        f.write_all(&a.to_le_bytes())?;
        f.write_all(&b.to_le_bytes())?;
    }
    println!("wrote {path}");
    Ok(())
}
