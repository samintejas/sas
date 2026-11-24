//! Golden renders: reference projects rendered offline and null-tested
//! against checked-in output.
//!
//! Regenerate after an intentional sound change with
//! `SAT_BLESS=1 cargo test -p sat-compile --test golden`, and listen before
//! committing the new file.

mod common;

use std::path::PathBuf;

use common::*;

/// Every `DECIMATE`th frame is stored, which keeps the files small and still
/// catches timing, level and pitch regressions.
const DECIMATE: usize = 8;
/// Allowed difference per sample. Not zero because `sin` and `powf` may differ
/// in the last bits between platforms' maths libraries.
const TOLERANCE: f32 = 1e-4;

fn golden_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden").join(format!("{name}.f32"))
}

fn check(name: &str, l: &[f32], r: &[f32]) {
    let got: Vec<f32> = l.iter().zip(r).step_by(DECIMATE).flat_map(|(a, b)| [*a, *b]).collect();
    let path = golden_path(name);
    if std::env::var_os("SAT_BLESS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let bytes: Vec<u8> = got.iter().flat_map(|v| v.to_le_bytes()).collect();
        std::fs::write(&path, bytes).unwrap();
        return;
    }
    let bytes = std::fs::read(&path).unwrap_or_else(|_| panic!("missing {}; run with SAT_BLESS=1", path.display()));
    let want: Vec<f32> = bytes.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
    assert_eq!(want.len(), got.len(), "{name}: length changed");
    let (worst, at) = want
        .iter()
        .zip(&got)
        .enumerate()
        .map(|(i, (w, g))| ((w - g).abs(), i))
        .fold((0.0f32, 0), |m, x| if x.0 > m.0 { x } else { m });
    assert!(
        worst <= TOLERANCE,
        "{name}: null test failed, worst difference {worst} at frame {} ({})",
        (at / 2) * DECIMATE,
        if at % 2 == 0 { "left" } else { "right" }
    );
}

#[test]
fn reference_song() {
    let s = song();
    let (l, r) = render(s.h.doc(), 4 * 24_000);
    check("reference_song", &l, &r);
}

#[test]
fn reference_song_at_44100_in_odd_blocks() {
    let s = song();
    let c = sat_compile::compile(s.h.doc(), 44_100.0, 512).unwrap();
    let (l, r) = sat_engine::render(c.schedule, 0, 4 * 22_050, 300);
    check("reference_song_44k", &l, &r);
}
