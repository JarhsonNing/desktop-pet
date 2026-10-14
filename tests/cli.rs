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

#[test]
fn skin_pack_check_and_unpack_do_not_start_the_pet() {
    let directory = directory("skins");
    fs::create_dir_all(&directory).unwrap();
    let package = directory.join("default.mochi");
    let data = directory.join("unused-data");
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("skins/default");
    let executable = env!("CARGO_BIN_EXE_desktop-pet");
    let packed = Command::new(executable)
        .arg("--pack-skin")
        .arg(source)
        .arg("--output")
        .arg(&package)
        .arg("--data-dir")
        .arg(&data)
        .output()
        .unwrap();
    assert!(
        packed.status.success(),
        "{}",
        String::from_utf8_lossy(&packed.stderr)
    );
    let checked = Command::new(executable)
        .arg("--check-skin")
        .arg(&package)
        .arg("--data-dir")
        .arg(&data)
        .output()
        .unwrap();
    assert!(checked.status.success());
    let unpacked = directory.join("unpacked");
    let extracted = Command::new(executable)
        .arg("--unpack-skin")
        .arg(&package)
        .arg("--output")
        .arg(&unpacked)
        .arg("--data-dir")
        .arg(&data)
        .output()
        .unwrap();
    assert!(extracted.status.success());
    assert!(unpacked.join("manifest.json").is_file());
    assert!(unpacked.join("pet.json").is_file());
    assert!(!data.exists());
    fs::remove_dir_all(directory).unwrap();
}
