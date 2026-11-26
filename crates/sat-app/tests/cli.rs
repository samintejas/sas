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

/// Saves the demo as a bundle in `dir`, with the crash-test plugin on the
/// keys track at `gain_db` if given.
fn demo_bundle(dir: &std::path::Path, gain_db: Option<f32>) -> std::path::PathBuf {
    use sat_doc::{Op, ParamTarget, apply};
    let mut d = sat_ui::demo::demo();
    if let Some(db) = gain_db {
        let keys = d.doc.tracks.values().find(|t| t.name == "keys").unwrap();
        let grid = *keys.fx_grids.front().unwrap();
        let (m, mut ops) = sat_modules::fx::insert(&d.doc, grid, sat_modules::fx::PLUGIN);
        ops.push(Op::SetModulePlugin { module: m, plugin: Some("builtin:crash-test".into()) });
        for (k, v) in [("p1", (db + 60.0) / 72.0), ("p2", 0.0), ("p3", 0.0)] {
            ops.push(Op::SetValue { target: ParamTarget::Module { module: m, param: k.into() }, value: v });
        }
        for op in ops {
            apply(&mut d.doc, op).unwrap();
        }
    }
    let path = dir.join(format!("{}.sat", gain_db.map_or("dry".into(), |g| format!("plug{g}"))));
    sat_doc::file::save_bundle(&path, &d.doc).unwrap();
    sat_disk::write_sounds(&path, &d.doc, d.sounds.sounds()).unwrap();
    path
}

fn export_loop(project: &std::path::Path) -> sat_disk::Sample {
    let out = project.with_extension("wav");
    let run = sat()
        .args(["--export".as_ref(), out.as_os_str(), "--loop".as_ref(), "--rate".as_ref(), "8000".as_ref()])
        .arg(project)
        .output()
        .unwrap();
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    sat_disk::decode(&out).unwrap()
}

#[test]
fn exports_render_through_plugins() {
    let dir = std::env::temp_dir().join(format!("sat-cli-plug-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let dry = export_loop(&demo_bundle(&dir, None));
    // at 0 dB the plugin is heard exactly as the dry track
    let unity = export_loop(&demo_bundle(&dir, Some(0.0)));
    assert_eq!(dry.frames(), unity.frames());
    let worst = dry.l.iter().zip(&unity.l).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
    assert!(worst < 1e-6, "{worst}");
    // at -60 dB the keys are gone from the export
    let quiet = export_loop(&demo_bundle(&dir, Some(-60.0)));
    let diff = dry.l.iter().zip(&quiet.l).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
    assert!(diff > 0.01, "the plugin was not rendered ({diff})");
    std::fs::remove_dir_all(&dir).ok();
}
