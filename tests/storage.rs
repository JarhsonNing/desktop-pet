use desktop_pet::{
    content::{Event, Payload, Policy, Service, Source, Store},
    explorer,
    storage::Settings,
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    time::Duration,
};
static SERIAL: AtomicUsize = AtomicUsize::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "mochi-storage-test-{}-{}",
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
fn import(store: &mut Store, id: &str, source: Source, payloads: Vec<Payload>) {
    store
        .import(
            id,
            source,
            payloads,
            &AtomicBool::new(false),
            &mut |_, _| {},
        )
        .unwrap();
}
fn move_library(store: &mut Store, data: &Path, destination: &Path) {
    store
        .relocate(
            destination,
            &AtomicBool::new(false),
            &mut |_, _| {},
            |root| {
                let mut settings = Settings::load(data)?;
                settings.content = Some(root.to_owned());
                settings.save(data)?;
                Ok(())
            },
        )
        .unwrap();
}

#[test]
fn library_copy_preserves_database_favorites_files_and_restart_independent_of_old_root() {
    let f = Fixture::new();
    let data = f.0.join("data");
    let mut store = Store::open(&data, Policy::default()).unwrap();
    let original = f.0.join("中文目录");
    fs::create_dir(&original).unwrap();
    fs::write(original.join("资料.txt"), "完整内容").unwrap();
    import(
        &mut store,
        "group",
        Source::Transfer,
        vec![Payload::Files(vec![original.clone()])],
    );
    import(
        &mut store,
        "history",
        Source::Clipboard,
        vec![Payload::Text {
            plain: "中文\n历史记录".into(),
            html: Some(b"<b>rich</b>".to_vec()),
            rtf: None,
        }],
    );
    store.favorite("group", true).unwrap();
    let old_root = store.root.clone();
    let target = f.0.join("selected library");
    fs::create_dir(&target).unwrap();
    move_library(&mut store, &data, &target);
    assert_eq!(store.root, target.canonicalize().unwrap());
    assert!(old_root.join("index.sqlite").exists());
    assert!(store.get("group").unwrap().favorite);
    assert_eq!(store.snapshot().unwrap().clipboard.len(), 1);
    assert!(
        Store::open_at(&store.root, Policy::default()).is_err(),
        "Selected libraries are exclusively locked"
    );
    fs::remove_dir_all(original).unwrap();
    // Both locations are inside this disposable fixture, and the old DB is closed.
    assert!(old_root.starts_with(f.0.canonicalize().unwrap()));
    fs::remove_dir_all(&old_root).unwrap();
    let (paths, _) = store
        .export(&["group".into()], &AtomicBool::new(false))
        .unwrap();
    assert_eq!(
        fs::read_to_string(paths[0].join("资料.txt")).unwrap(),
        "完整内容"
    );
    let root = store.root.clone();
    drop(store);
    assert_eq!(Settings::load(&data).unwrap().content(&data), root);
    let store = Store::open_at(&root, Policy::default()).unwrap();
    assert_eq!(
        store.search(Source::Clipboard, "历史记录").unwrap().len(),
        1
    );
    assert!(store.get("group").unwrap().favorite);
}

#[test]
fn migration_cancel_collision_overlap_and_commit_failure_keep_original_active() {
    let f = Fixture::new();
    let data = f.0.join("data");
    let mut store = Store::open(&data, Policy::default()).unwrap();
    import(
        &mut store,
        "keep",
        Source::Transfer,
        vec![Payload::text("不能丢失")],
    );
    let old = store.root.clone();
    let occupied = f.0.join("occupied");
    fs::create_dir(&occupied).unwrap();
    fs::write(occupied.join("user.txt"), "untouched").unwrap();
    assert!(
        store
            .relocate(&occupied, &AtomicBool::new(false), &mut |_, _| {}, |_| Ok(
                ()
            ))
            .is_err()
    );
    assert_eq!(
        fs::read_to_string(occupied.join("user.txt")).unwrap(),
        "untouched"
    );
    assert!(
        store
            .relocate(
                &old.join("nested"),
                &AtomicBool::new(false),
                &mut |_, _| {},
                |_| Ok(())
            )
            .is_err()
    );
    let cancelled = f.0.join("cancelled");
    assert!(
        store
            .relocate(
                &cancelled,
                &AtomicBool::new(true),
                &mut |_, _| {},
                |_| panic!("cancelled migration must not commit")
            )
            .is_err()
    );
    assert!(!cancelled.exists());
    let during_copy = AtomicBool::new(false);
    assert!(
        store
            .relocate(
                &f.0.join("cancel during copy"),
                &during_copy,
                &mut |_, _| during_copy.store(true, Ordering::Release),
                |_| panic!("partial copy must not commit")
            )
            .is_err()
    );
    assert!(
        store
            .relocate(
                &f.0.join("commit failure"),
                &AtomicBool::new(false),
                &mut |_, _| {},
                |_| Err("Injected settings write failure".into())
            )
            .is_err()
    );
    assert_eq!(store.root, old);
    assert!(store.get("keep").is_ok());
    assert!(!data.join("storage.json").exists());
    assert!(!fs::read_dir(&f.0).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".mochi-storage-")
    }));
}

#[test]
fn attachments_started_before_switch_and_new_attachments_both_land_in_new_library() {
    let f = Fixture::new();
    let data = f.0.join("data");
    let mut store = Store::open(&data, Policy::default()).unwrap();
    let old_job = store.root.join("incoming/old-job");
    fs::create_dir(&old_job).unwrap();
    fs::write(old_job.join("附件.txt"), "old stream").unwrap();
    move_library(&mut store, &data, &f.0.join("new library"));
    let new_job = store.root.join("incoming/new-job");
    fs::create_dir(&new_job).unwrap();
    fs::write(new_job.join("附件.txt"), "new stream").unwrap();
    for (id, directory) in [("old", old_job), ("new", new_job)] {
        import(
            &mut store,
            id,
            Source::Transfer,
            vec![Payload::TemporaryFiles {
                paths: vec![directory.join("附件.txt")],
                directory: directory.clone(),
                errors: vec![],
            }],
        );
        assert!(!directory.exists());
    }
    assert_eq!(store.snapshot().unwrap().transfer.len(), 2);
    let (files, _) = store
        .export(&["old".into(), "new".into()], &AtomicBool::new(false))
        .unwrap();
    assert_eq!(fs::read_to_string(&files[0]).unwrap(), "old stream");
    assert_eq!(fs::read_to_string(&files[1]).unwrap(), "new stream");
}

#[test]
fn filebox_selection_keeps_local_edits_and_versions_and_rejects_nested_roots() {
    let f = Fixture::new();
    let data = f.0.join("data");
    let mut store = Store::open(&data, Policy::default()).unwrap();
    import(
        &mut store,
        "text",
        Source::Transfer,
        vec![Payload::text("收纳内容")],
    );
    let old = explorer::publish(
        &data,
        &store.snapshot().unwrap(),
        1_000_000,
        false,
        &AtomicBool::new(false),
        |entry| store.view_sources(entry),
    )
    .unwrap();
    fs::write(old.join("自行加入.txt"), "保留用户编辑").unwrap();
    let target = f.0.join("selected view");
    let settings = explorer::relocate(
        &data,
        &Settings::default(),
        &target,
        false,
        &AtomicBool::new(false),
        &mut |_, _| {},
    )
    .unwrap();
    assert_eq!(
        fs::read_to_string(target.join("自行加入.txt")).unwrap(),
        "保留用户编辑"
    );
    assert!(old.join("自行加入.txt").exists());
    assert_eq!(
        Settings::load(&data).unwrap().explorer(&data),
        target.canonicalize().unwrap()
    );
    assert!(
        explorer::relocate(
            &data,
            &settings,
            &data.join("content/inside"),
            false,
            &AtomicBool::new(false),
            &mut |_, _| {}
        )
        .is_err()
    );
    fs::write(data.join("storage.json"), r#"{"version":999}"#).unwrap();
    assert!(Settings::load(&data).is_err());
    assert_eq!(
        fs::read_to_string(data.join("storage.json")).unwrap(),
        r#"{"version":999}"#
    );
}

#[test]
fn service_switch_persists_and_new_imports_work_after_restart() {
    let f = Fixture::new();
    let data = f.0.join("data");
    let target = f.0.join("chosen");
    let mut service = Service::start(data.clone(), Policy::default(), || {});
    service.move_storage("content", target.clone()).unwrap();
    loop {
        if let Event::Storage {
            completed: true,
            error,
            ..
        } = service
            .receiver
            .recv_timeout(Duration::from_secs(15))
            .unwrap()
        {
            assert_eq!(error, None);
            break;
        }
    }
    let id = service.import(Source::Transfer, vec![Payload::text("迁移之后")]);
    loop {
        if let Event::Completed { id: done, .. } = service
            .receiver
            .recv_timeout(Duration::from_secs(15))
            .unwrap()
            && done == id
        {
            break;
        }
    }
    service.stop();
    let mut service = Service::start(data, Policy::default(), || {});
    loop {
        if let Event::Snapshot(snapshot) = service
            .receiver
            .recv_timeout(Duration::from_secs(15))
            .unwrap()
        {
            assert!(snapshot.transfer.iter().any(|entry| entry.id == id));
            break;
        }
    }
    service.stop();
    assert!(target.join("index.sqlite").exists());
}

#[test]
fn missing_selected_library_reports_unavailable_without_creating_empty_replacement() {
    let f = Fixture::new();
    let data = f.0.join("data");
    fs::create_dir(&data).unwrap();
    let missing = f.0.join("disconnected library");
    let settings = Settings {
        content: Some(missing.clone()),
        ..Settings::default()
    };
    settings.save(&data).unwrap();
    let mut service = Service::start(data.clone(), Policy::default(), || {});
    loop {
        if let Event::Storage {
            error: Some(error), ..
        } = service
            .receiver
            .recv_timeout(Duration::from_secs(10))
            .unwrap()
        {
            assert!(error.contains("不可用"));
            break;
        }
    }
    service.stop();
    assert!(!missing.exists());
    assert_eq!(Settings::load(&data).unwrap().content, Some(missing));
}

#[cfg(windows)]
#[test]
fn filebox_registration_moves_and_crash_journal_recovers_only_inactive_entry() {
    use std::os::windows::process::CommandExt;
    let f = Fixture::new();
    let data = f.0.join("data");
    fs::create_dir(&data).unwrap();
    let old = data.join("Mochi 文件箱");
    let new = f.0.join("new view");
    fs::create_dir(&old).unwrap();
    struct Registration(Vec<PathBuf>);
    impl Drop for Registration {
        fn drop(&mut self) {
            for path in &self.0 {
                let _ = explorer::registration::uninstall(path);
            }
        }
    }
    let _registration = Registration(vec![old.clone(), new.clone()]);
    let settings = explorer::relocate(
        &data,
        &Settings::default(),
        &new,
        true,
        &AtomicBool::new(false),
        &mut |_, _| {},
    )
    .unwrap();
    let registered = |path: &Path| {
        std::process::Command::new("reg.exe")
            .args([
                "query",
                &format!(
                    "HKCU\\Software\\Classes\\CLSID\\{}",
                    explorer::registration::class_id(path)
                ),
            ])
            .creation_flags(0x08000000)
            .output()
            .unwrap()
            .status
            .success()
    };
    assert!(registered(&new));
    assert!(!registered(&old));
    // Simulate interruption between publishing the destination and committing its setting.
    for active in [Settings::default(), settings] {
        active.save(&data).unwrap();
        explorer::registration::install(&old).unwrap();
        explorer::registration::install(&new).unwrap();
        desktop_pet::config::atomic_json(
            &data.join("storage-moving.json"),
            &serde_json::json!({"version":1,"old":old,"new":new,"enabled":true}),
        )
        .unwrap();
        explorer::recover_storage(&data, &active).unwrap();
        let active_is_old = active.explorer.is_none();
        assert_eq!(registered(&old), active_is_old);
        assert_eq!(registered(&new), !active_is_old);
        assert!(!data.join("storage-moving.json").exists());
    }
    assert!(old.exists() && new.exists());
}
