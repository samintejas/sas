//! The crash-test plugin loaded from its library by a real host process,
//! the way any CLAP plugin is.

use std::path::PathBuf;

use sat_plug::PlugHost;

/// The build's output folder (`target/<profile>`), from this test's path.
fn out_dir() -> PathBuf {
    let exe = std::env::current_exe().expect("test path");
    exe.parent().and_then(|d| d.parent()).expect("target dir").to_path_buf()
}

#[test]
fn loads_through_dlopen_and_runs() {
    let dir = out_dir();
    let lib = dir.join(format!("{}sat_testplug{}", std::env::consts::DLL_PREFIX, std::env::consts::DLL_SUFFIX));
    let host = dir.join(format!("sat-plughost{}", std::env::consts::EXE_SUFFIX));
    if !lib.exists() || !host.exists() {
        eprintln!("skipped: build the workspace first ({} or {} missing)", lib.display(), host.display());
        return;
    }
    let h = PlugHost::spawn(&host, &[], lib.to_str().expect("utf-8 path"), 48_000.0, 256).expect("loads");
    assert_eq!(h.hello.name, "sat crash test");
    let mut p = h.processor();
    // half way along gain's -60..12 dB is -24 dB
    let want = 10f32.powf(-24.0 / 20.0);
    let ok = (0..50).any(|_| {
        let (mut l, mut r) = (vec![1.0f32; 256], vec![1.0f32; 256]);
        let ok = p.run(&mut l, &mut r, &[0.5, 0.0, 0.0]) && (l[10] - want).abs() < 1e-4;
        std::thread::sleep(std::time::Duration::from_millis(2));
        ok
    });
    assert!(ok, "audio did not come back through the plugin");
}
