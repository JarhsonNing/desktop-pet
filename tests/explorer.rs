use desktop_pet::{
    content::{Payload, Policy, Source, Store},
    explorer,
};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};
static SERIAL: AtomicUsize = AtomicUsize::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "mochi-view-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn publish(
    store: &Store,
    data: &std::path::Path,
    force: bool,
    quota: u64,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    explorer::publish(
        data,
        &store.snapshot()?,
        quota,
        force,
        &AtomicBool::new(false),
        |entry| store.view_sources(entry),
    )
}
fn group(root: &std::path::Path, category: &str) -> PathBuf {
    fs::read_dir(root.join(category))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path()
}

#[test]
fn projection_uses_independent_real_files_and_preserves_local_edits_on_refresh() {
    let f = Fixture::new();
    let mut store = Store::open(&f.0, Policy::default()).unwrap();
    let original = f.0.join("原件.txt");
    fs::write(&original, "原始内容").unwrap();
    store
        .import(
            "files",
            Source::Transfer,
            vec![Payload::Files(vec![original.clone()])],
            &AtomicBool::new(false),
            &mut |_, _| {},
        )
        .unwrap();
    store.favorite("files", true).unwrap();
    let root = publish(&store, &f.0, false, 1_000_000).unwrap();
    let output = group(&root, "中转站").join("原件.txt");
    assert_eq!(fs::read_to_string(&output).unwrap(), "原始内容");
    assert!(group(&root, "我的收藏").join("原件.txt").exists());
    fs::write(&output, "本地改动").unwrap();
    publish(&store, &f.0, true, 1_000_000).unwrap();
    assert_eq!(fs::read_to_string(&output).unwrap(), "原始内容");
    assert_eq!(
        fs::read_to_string(group(&root, "本地改动保留").join("原件.txt")).unwrap(),
        "本地改动"
    );
    assert_eq!(fs::read_to_string(&original).unwrap(), "原始内容");
    store.favorite("files", false).unwrap();
    publish(&store, &f.0, false, 1_000_000).unwrap();
    assert_eq!(fs::read_dir(root.join("我的收藏")).unwrap().count(), 0);
    fs::write(root.join("用户另放的文件.txt"), "不要删除").unwrap();
    store.delete(&["files".into()]).unwrap();
    publish(&store, &f.0, false, 1_000_000).unwrap();
    assert!(root.join("用户另放的文件.txt").exists());
    assert!(original.exists());
}

#[test]
fn text_image_and_favorite_original_paths_are_real_file_dialog_items() {
    let f = Fixture::new();
    let mut store = Store::open(&f.0, Policy::default()).unwrap();
    store
        .import(
            "text",
            Source::Transfer,
            vec![Payload::text("中文\n完整正文")],
            &AtomicBool::new(false),
            &mut |_, _| {},
        )
        .unwrap();
    let original = f.0.join("收藏.txt");
    fs::write(&original, "收藏的原路径").unwrap();
    store
        .import(
            "reference",
            Source::Clipboard,
            vec![Payload::Files(vec![original.clone()])],
            &AtomicBool::new(false),
            &mut |_, _| {},
        )
        .unwrap();
    store.favorite("reference", true).unwrap();
    fs::remove_file(original).unwrap();
    let root = publish(&store, &f.0, false, 1_000_000).unwrap();
    assert_eq!(
        fs::read_to_string(group(&root, "中转站").join("文本-1.txt")).unwrap(),
        "中文\n完整正文"
    );
    assert!(
        group(&root, "我的收藏")
            .join("收藏.txt.失效说明.txt")
            .exists()
    );
    let image = image::RgbaImage::from_pixel(3, 2, image::Rgba([100, 150, 200, 255]));
    let mut png = std::io::Cursor::new(Vec::new());
    image.write_to(&mut png, image::ImageFormat::Png).unwrap();
    store
        .import(
            "image",
            Source::Transfer,
            vec![Payload::Image(png.into_inner())],
            &AtomicBool::new(false),
            &mut |_, _| {},
        )
        .unwrap();
    publish(&store, &f.0, false, 1_000_000).unwrap();
    let output = fs::read_dir(root.join("中转站"))
        .unwrap()
        .map(|folder| folder.unwrap().path().join("图片-1.png"))
        .find(|file| file.exists())
        .unwrap();
    assert_eq!(image::open(output).unwrap().into_rgba8(), image);
}

#[cfg(windows)]
#[test]
fn windows_this_pc_namespace_resolves_real_folders_and_uninstalls() {
    use std::os::windows::process::CommandExt;
    let f = Fixture::new();
    let store = Store::open(&f.0, Policy::default()).unwrap();
    let root = publish(&store, &f.0, false, 1_000_000).unwrap();
    struct Registered(PathBuf);
    impl Drop for Registered {
        fn drop(&mut self) {
            let _ = explorer::registration::uninstall(&self.0);
        }
    }
    let registered = Registered(root.clone());
    explorer::registration::install(&root).unwrap();
    let id = explorer::registration::class_id(&root.canonicalize().unwrap());
    let marker = f.0.file_name().unwrap().to_string_lossy();
    let script = format!(
        r#"$ErrorActionPreference='Stop'; [Console]::OutputEncoding=[Text.Encoding]::UTF8; $s=New-Object -ComObject Shell.Application; $all=@($s.NameSpace(17).Items()); $items=@($all | Where-Object {{$_.Path -like '*{id}*' -or $_.Path -like '*{marker}*'}}); if($items.Count -ne 1){{$all | ForEach-Object {{Write-Output $_.Path}}; throw 'This PC item missing'}}; $folder=$items[0].GetFolder; if($null -eq $folder){{throw 'Cannot resolve folder'}}; if(@($folder.Items()).Count -lt 2){{throw 'Children missing'}}; 'OK'"#
    );
    let output = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(0x08000000)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("OK"));
    drop(registered);
    let status = std::process::Command::new("reg.exe")
        .args(["query", &format!("HKCU\\Software\\Classes\\CLSID\\{id}")])
        .creation_flags(0x08000000)
        .output()
        .unwrap();
    assert!(!status.status.success());
}

#[test]
fn unknown_or_tampered_projection_manifest_never_deletes_outside_files() {
    let f = Fixture::new();
    let store = Store::open(&f.0, Policy::default()).unwrap();
    let root = publish(&store, &f.0, false, 1_000_000).unwrap();
    let protected = f.0.join("protected");
    fs::create_dir(&protected).unwrap();
    fs::write(protected.join("keep"), "keep").unwrap();
    fs::write(
        root.join(".mochi-view.json"),
        r#"{"version":1,"records":{"bad":{"folder":"../protected","fingerprint":"","digest":""}}}"#,
    )
    .unwrap();
    assert!(publish(&store, &f.0, false, 1_000_000).is_err());
    assert!(protected.join("keep").exists());
    fs::write(
        root.join(".mochi-view.json"),
        r#"{"version":999,"records":{}}"#,
    )
    .unwrap();
    assert!(publish(&store, &f.0, false, 1_000_000).is_err());
    assert!(
        fs::read_to_string(root.join(".mochi-view.json"))
            .unwrap()
            .contains("999")
    );
}

#[test]
fn cancellation_and_quota_fail_without_changing_library_or_existing_projection() {
    let f = Fixture::new();
    let mut store = Store::open(&f.0, Policy::default()).unwrap();
    store
        .import(
            "text",
            Source::Transfer,
            vec![Payload::text("重要资料".repeat(1000))],
            &AtomicBool::new(false),
            &mut |_, _| {},
        )
        .unwrap();
    assert!(publish(&store, &f.0, false, 1).is_err());
    assert!(store.get("text").is_ok());
    let root = publish(&store, &f.0, false, 1_000_000).unwrap();
    assert!(
        explorer::publish(
            &f.0,
            &store.snapshot().unwrap(),
            1_000_000,
            true,
            &AtomicBool::new(true),
            |entry| store.view_sources(entry)
        )
        .is_err()
    );
    assert!(group(&root, "中转站").join("文本-1.txt").exists());
}

#[test]
fn partial_record_export_can_select_one_file_from_a_multi_file_group() {
    let f = Fixture::new();
    let mut store = Store::open(&f.0, Policy::default()).unwrap();
    let files = [f.0.join("one.txt"), f.0.join("two.txt")];
    for file in &files {
        fs::write(file, file.to_string_lossy().as_bytes()).unwrap();
    }
    let entry = store
        .import(
            "multi",
            Source::Transfer,
            vec![Payload::Files(files.to_vec())],
            &AtomicBool::new(false),
            &mut |_, _| {},
        )
        .unwrap();
    let picks = desktop_pet::delivery::entry_picks(&entry);
    let (paths, _) = store
        .export_picks(&[picks[1].clone()], &AtomicBool::new(false), &mut |_, _| {})
        .unwrap();
    assert_eq!(paths.len(), 1);
    assert_eq!(paths[0].file_name().unwrap(), "two.txt");
}

#[test]
fn file_names_cannot_overwrite_generated_text_in_a_mixed_delivery() {
    let f = Fixture::new();
    let mut store = Store::open(&f.0, Policy::default()).unwrap();
    let file = f.0.join("文本-2.txt");
    fs::write(&file, "文件原文").unwrap();
    store
        .import(
            "mixed",
            Source::Transfer,
            vec![Payload::Files(vec![file]), Payload::text("文字正文")],
            &AtomicBool::new(false),
            &mut |_, _| {},
        )
        .unwrap();
    let (files, _) = store
        .export(&["mixed".into()], &AtomicBool::new(false))
        .unwrap();
    assert_ne!(files[0], files[1]);
    assert_eq!(fs::read_to_string(&files[0]).unwrap(), "文件原文");
    assert_eq!(fs::read_to_string(&files[1]).unwrap(), "文字正文");
}

#[test]
fn projection_preserves_duplicate_contents_and_long_name_extensions() {
    let f = Fixture::new();
    let mut store = Store::open(&f.0, Policy::default()).unwrap();
    let mut files = Vec::new();
    for (index, name) in [
        "3-同名.txt".into(),
        "同名.txt".into(),
        "同名.txt".into(),
        format!("{}.pdf", "名".repeat(80)),
    ]
    .iter()
    .enumerate()
    {
        let parent = f.0.join(index.to_string());
        fs::create_dir(&parent).unwrap();
        let file = parent.join(name);
        fs::write(&file, index.to_string()).unwrap();
        files.push(file);
    }
    store
        .import(
            "group",
            Source::Transfer,
            vec![Payload::Files(files)],
            &AtomicBool::new(false),
            &mut |_, _| {},
        )
        .unwrap();
    let root = publish(&store, &f.0, false, 1_000_000).unwrap();
    let files = fs::read_dir(group(&root, "中转站"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    let mut contents = files
        .iter()
        .map(|file| fs::read_to_string(file).unwrap())
        .collect::<Vec<_>>();
    contents.sort();
    assert_eq!(contents, ["0", "1", "2", "3"]);
    assert!(
        files
            .iter()
            .any(|file| file.extension().is_some_and(|ext| ext == "pdf"))
    );
}
