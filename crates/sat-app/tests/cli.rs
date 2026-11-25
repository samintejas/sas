//! The command line export.

use std::process::Command;

fn sat() -> Command {
    Command::new(env!("CARGO_BIN_EXE_sat"))
}

#[test]
fn exports_the_demo_loop_to_a_wav_file() {
    let dir = std::env::temp_dir().join(format!("sat-cli-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let out = dir.join("loop.wav");
    let run = sat()
        .args(["--export".as_ref(), out.as_os_str(), "--loop".as_ref(), "--rate".as_ref(), "8000".as_ref()])
        .output()
        .unwrap();
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    let s = sat_disk::decode(&out).unwrap();
    assert_eq!(s.rate, 8000.0);
    // the demo loop is eight bars at 124 bpm
    let want = 8.0 * 4.0 * 60.0 / 124.0 * 8000.0;
    assert!((s.frames() as f64 - want).abs() <= 1.0, "{} frames, want {want}", s.frames());
    assert!(s.l.iter().any(|v| v.abs() > 0.01), "the export is silent");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn rejects_unknown_options() {
    let run = sat().arg("--nope").output().unwrap();
    assert_eq!(run.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&run.stderr).contains("unknown option --nope"));
}
