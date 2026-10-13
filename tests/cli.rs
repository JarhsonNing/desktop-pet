use desktop_pet::safety;
use std::{
    fs,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

fn directory(name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("mochi-cli-{name}-{}-{unique}", std::process::id()))
}

#[test]
fn simulation_does_not_create_a_desktop_lease() {
    let directory = directory("simulation");
    let output = Command::new(env!("CARGO_BIN_EXE_desktop-pet"))
        .args(["--simulate", "--data-dir"])
        .arg(&directory)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("no desktop changes"));
    assert!(!directory.exists());
}

#[test]
fn command_line_recovery_is_idempotent_without_a_lease() {
    let directory = directory("restore");
    for _ in 0..2 {
        let output = Command::new(env!("CARGO_BIN_EXE_desktop-pet"))
            .args(["--restore-desktop", "--data-dir"])
            .arg(&directory)
            .output()
            .unwrap();
        assert!(output.status.success());
        #[cfg(not(target_os = "windows"))]
        assert!(String::from_utf8_lossy(&output.stdout).contains("Restored: false"));
    }
    assert!(!safety::lease_path(&directory).exists());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn a_second_instance_is_rejected_before_opening_a_window() {
    let directory = directory("instance");
    let _instance = safety::lock(&directory.join("instance.lock")).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_desktop-pet"))
        .arg("--data-dir")
        .arg(&directory)
        .output()
        .unwrap();
    assert!(!output.status.success());
    #[cfg(not(target_os = "windows"))]
    assert!(String::from_utf8_lossy(&output.stderr).contains("already running"));
    drop(_instance);
    fs::remove_dir_all(directory).unwrap();
}
