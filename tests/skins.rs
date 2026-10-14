use desktop_pet::{
    engine::Mood,
    skin::{self, Skin},
};
use std::{
    fs,
    io::{Cursor, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

struct Temporary(PathBuf);
impl Temporary {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "mochi-skin-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temporary {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn source(root: &Path) -> PathBuf {
    let source = root.join("source");
    fs::create_dir(&source).unwrap();
    fs::write(
        source.join("manifest.json"),
        include_bytes!("../skins/default/manifest.json"),
    )
    .unwrap();
    fs::write(
        source.join("pet.json"),
        include_bytes!("../skins/default/pet.json"),
    )
    .unwrap();
    source
}

fn edit_ui(source: &Path, edit: impl FnOnce(&mut serde_json::Value)) {
    let path = source.join("pet.json");
    let mut ui: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    edit(&mut ui);
    fs::write(path, serde_json::to_vec(&ui).unwrap()).unwrap();
}

#[test]
fn default_definition_preserves_pet_silhouette_and_motion() {
    let skin = Skin::builtin();
    assert!(skin.animation_interval(Mood::Idle).is_none());
    assert!(!skin.hit_test([0.0, 0.0], 1.0, Mood::Idle, 0.0, [0.0; 2]));
    assert!(skin.hit_test([130.0, 160.0], 1.0, Mood::Idle, 0.0, [0.0; 2]));
    assert!(skin.hit_test([195.0, 240.0], 1.5, Mood::Idle, 0.0, [0.0; 2]));
    assert!(!skin.hit_test([130.0, 100.0], 1.0, Mood::Idle, 0.0, [0.0; 2]));
    assert!(skin.hit_test([85.0, 103.0], 1.0, Mood::Idle, 0.0, [0.0; 2]));
    assert!(skin.hit_test([196.0, 146.0], 1.0, Mood::Happy, 0.0, [0.0; 2]));
    assert!(!skin.hit_test([196.0, 146.0], 1.0, Mood::Idle, 0.0, [0.0; 2]));
    assert!(!skin.hit_test([130.0, 115.0], 1.0, Mood::Sleep, 0.0, [0.0; 2]));
    assert!(skin.hit_test([130.0, 115.0], 1.0, Mood::Dragged, 0.0, [0.0; 2]));
}

#[test]
fn directory_custom_package_and_zip_share_the_same_definition() {
    let temporary = Temporary::new();
    let source = source(&temporary.0);
    let expected = skin::load(&source).unwrap();
    for extension in ["mochi", "zip"] {
        let path = temporary.0.join(format!("skin.{extension}"));
        skin::pack(&source, &path).unwrap();
        let loaded = skin::load(&path).unwrap();
        assert_eq!(loaded.manifest.id, expected.manifest.id);
        assert_eq!(
            serde_json::to_value(&loaded.ui).unwrap(),
            serde_json::to_value(&expected.ui).unwrap()
        );
        if extension == "mochi" {
            assert!(fs::read(&path).unwrap().starts_with(b"MOCHISK\0\x01\0\0\0"));
        }
        let unpacked = temporary.0.join(format!("unpacked-{extension}"));
        skin::unpack(&path, &unpacked).unwrap();
        assert_eq!(
            skin::load(&unpacked).unwrap().manifest.id,
            expected.manifest.id
        );
        assert!(skin::unpack(&path, &unpacked).is_err());
        assert!(skin::pack(&source, &path).is_err());
        assert!(skin::load(&path).is_ok());
    }
}

#[test]
fn invalid_schema_versions_and_unusable_recovery_layouts_are_rejected() {
    let temporary = Temporary::new();
    let source = source(&temporary.0);
    edit_ui(&source, |ui| ui["canvas"][0] = 0.into());
    assert!(skin::load(&source).is_err());
    assert!(skin::pack(&source, &temporary.0.join("invalid.mochi")).is_err());
    assert!(!temporary.0.join("invalid.mochi").exists());
    fs::write(
        source.join("pet.json"),
        include_bytes!("../skins/default/pet.json"),
    )
    .unwrap();
    edit_ui(&source, |ui| {
        ui["bubble"]["restore_button"] = serde_json::json!([0, 0, 10, 10])
    });
    assert!(skin::load(&source).is_err());
    fs::write(
        source.join("pet.json"),
        include_bytes!("../skins/default/pet.json"),
    )
    .unwrap();
    let manifest = source.join("manifest.json");
    let mut definition: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    definition["version"] = 999.into();
    fs::write(manifest, serde_json::to_vec(&definition).unwrap()).unwrap();
    assert!(skin::load(&source).is_err());
}

#[test]
fn traversal_duplicate_assets_and_future_package_versions_are_rejected() {
    let temporary = Temporary::new();
    for entries in [
        vec!["../outside.json"],
        vec!["/absolute.json"],
        vec!["C:/absolute.json"],
        vec!["a\\b.json"],
        vec!["CON.png"],
        vec!["pet.json", "PET.json"],
    ] {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for entry in entries {
            writer
                .start_file(entry, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(b"{}").unwrap();
        }
        let path = temporary.0.join("unsafe.zip");
        fs::write(&path, writer.finish().unwrap().into_inner()).unwrap();
        assert!(skin::load(&path).is_err());
    }
    let path = temporary.0.join("future.mochi");
    fs::write(&path, b"MOCHISK\0\x02\0\0\0").unwrap();
    assert!(skin::load(&path).is_err());
    assert!(!temporary.0.join("outside.json").exists());
}

#[test]
fn png_frame_alpha_controls_hit_testing_at_the_current_frame() {
    let temporary = Temporary::new();
    let source = source(&temporary.0);
    for (name, pixel) in [
        ("one.png", [20, 80, 120, 255]),
        ("two.png", [20, 80, 120, 0]),
    ] {
        let mut image = image::RgbaImage::new(2, 2);
        image.put_pixel(0, 0, image::Rgba(pixel));
        image.save(source.join(name)).unwrap();
    }
    edit_ui(&source, |ui| {
        ui["states"] = serde_json::json!({"idle": {}});
        ui["layers"] = serde_json::json!([{"id":"sprite", "node":{"interactive":true,"shape": {
            "type":"image", "origin":[60,90], "size":[100,100], "frames":["one.png","two.png"], "fps":1
        }}}]);
        let mut happy = ui["layers"][0]["node"].clone();
        happy["shape"]["fps"] = 24.into();
        let mut sleep = ui["layers"][0]["node"].clone();
        sleep["shape"]["frames"] = serde_json::json!(["one.png", "one.png"]);
        ui["layers"][0]["states"] = serde_json::json!({"happy": happy, "sleep": sleep});
        ui["layers"].as_array_mut().unwrap().push(serde_json::json!({
            "id":"invisible-fast-sprite", "node":{"visible":false,"shape": {
                "type":"image", "origin":[60,90], "size":[100,100], "frames":["one.png","two.png"], "fps":30
            }}
        }));
    });
    let skin = skin::load(&source).unwrap();
    assert_eq!(skin.animation_interval(Mood::Idle).unwrap().as_secs(), 1);
    assert_eq!(
        skin.animation_interval(Mood::Happy).unwrap(),
        std::time::Duration::from_secs_f64(1.0 / 24.0)
    );
    assert!(skin.animation_interval(Mood::Sleep).is_none());
    assert!(skin.hit_test([70.0, 100.0], 1.0, Mood::Idle, 0.0, [0.0; 2]));
    assert!(!skin.hit_test([70.0, 100.0], 1.0, Mood::Idle, 1.0, [0.0; 2]));
    assert!(!skin.hit_test([150.0, 180.0], 1.0, Mood::Idle, 0.0, [0.0; 2]));
    assert!(skin.hit_test([70.0, 100.0], 1.0, Mood::Happy, 2.0, [0.0; 2]));
    fs::remove_file(source.join("two.png")).unwrap();
    assert!(skin::load(&source).is_err());
}

#[test]
fn invisible_vector_fills_do_not_block_clicks() {
    let temporary = Temporary::new();
    let source = source(&temporary.0);
    edit_ui(&source, |ui| {
        ui["states"] = serde_json::json!({"idle": {}});
        ui["layers"] = serde_json::json!([{"id":"ring", "node":{"interactive":true,"stroke":{"width":4,"color":"ink"},"shape": {
            "type":"ellipse", "center":[130,160], "radius":[40,40]
        }}}]);
    });
    let skin = skin::load(&source).unwrap();
    assert!(!skin.hit_test([130.0, 160.0], 1.0, Mood::Idle, 0.0, [0.0; 2]));
    assert!(skin.hit_test([170.0, 160.0], 1.0, Mood::Idle, 0.0, [0.0; 2]));
}

#[test]
fn expanded_size_limits_prevent_zip_bombs() {
    let temporary = Temporary::new();
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .start_file(
            "large.json",
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated),
        )
        .unwrap();
    writer.write_all(&vec![0; 8 * 1024 * 1024 + 1]).unwrap();
    let path = temporary.0.join("large.zip");
    fs::write(&path, writer.finish().unwrap().into_inner()).unwrap();
    assert!(skin::load(&path).is_err());
}
