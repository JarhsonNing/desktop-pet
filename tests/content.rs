use desktop_pet::content::{Item, Payload, Policy, Source, State, Store};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

static SERIAL: AtomicUsize = AtomicUsize::new(0);
struct Fixture {
    path: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "mochi-content-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }
    fn store(&self, quota: u64) -> Store {
        Store::open(
            &self.path,
            Policy {
                quota,
                ..Default::default()
            },
        )
        .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
fn save(
    store: &mut Store,
    id: &str,
    source: Source,
    payloads: Vec<Payload>,
) -> desktop_pet::content::Entry {
    store
        .import(
            id,
            source,
            payloads,
            &AtomicBool::new(false),
            &mut |_, _| {},
        )
        .unwrap()
}

#[test]
fn unicode_search_full_text_dedup_and_restart() {
    let f = Fixture::new();
    let mut store = f.store(1024 * 1024);
    let text = format!("曦玥\n{}目标文字 100%_", "前".repeat(300));
    let first = save(
        &mut store,
        "one",
        Source::Clipboard,
        vec![Payload::text(&text)],
    );
    store.favorite(&first.id, true).unwrap();
    save(
        &mut store,
        "different",
        Source::Clipboard,
        vec![Payload::text("另一条")],
    );
    let second = save(
        &mut store,
        "two",
        Source::Clipboard,
        vec![Payload::text(&text)],
    );
    assert_eq!(first.id, second.id);
    assert!(second.favorite);
    assert_eq!(store.snapshot().unwrap().clipboard.len(), 2);
    assert_eq!(store.snapshot().unwrap().clipboard[0].id, first.id);
    assert_eq!(
        store.search(Source::Clipboard, "目标文字").unwrap().len(),
        1
    );
    assert_eq!(store.search(Source::Clipboard, "100%_").unwrap().len(), 1);
    drop(store);
    let store = f.store(1024 * 1024);
    assert_eq!(store.preview(&first.id).unwrap().0.unwrap(), text);
}

#[test]
fn transfer_survives_source_removal_and_never_overwrites_destination() {
    let f = Fixture::new();
    let source = f.path.join("原文件.txt");
    fs::write(&source, "独立副本").unwrap();
    let mut store = f.store(1024 * 1024);
    let entry = save(
        &mut store,
        "transfer",
        Source::Transfer,
        vec![Payload::Files(vec![source.clone()])],
    );
    fs::remove_file(source).unwrap();
    let (files, payloads) = store.export(&[entry.id], &AtomicBool::new(false)).unwrap();
    assert_eq!(files[0].file_name().unwrap(), "原文件.txt");
    assert_eq!(fs::read_to_string(&files[0]).unwrap(), "独立副本");
    assert!(matches!(payloads[0], Payload::Files(_)));
    let destination = f.path.join("destination");
    fs::create_dir(&destination).unwrap();
    fs::write(destination.join("原文件.txt"), "已有文件").unwrap();
    let (count, errors) = store.save_exports(&files, &destination, &AtomicBool::new(false));
    assert_eq!(count, 0);
    assert_eq!(errors.len(), 1);
    assert_eq!(
        fs::read_to_string(destination.join("原文件.txt")).unwrap(),
        "已有文件"
    );
}

#[test]
fn clipboard_file_lists_do_not_create_backups() {
    let f = Fixture::new();
    let path = f.path.join("file.txt");
    fs::write(&path, "references only").unwrap();
    let mut store = f.store(100);
    let entry = save(
        &mut store,
        "clip",
        Source::Clipboard,
        vec![Payload::Files(vec![path.clone()])],
    );
    assert_eq!(store.snapshot().unwrap().bytes, 0);
    fs::remove_file(path).unwrap();
    assert!(
        store
            .preview(&entry.id)
            .unwrap()
            .0
            .unwrap()
            .contains("原文件已不存在")
    );
    assert!(store.export(&[entry.id], &AtomicBool::new(false)).is_err());
}

#[test]
fn clearing_history_preserves_shared_transfer_assets_and_favorites() {
    let f = Fixture::new();
    let mut store = f.store(1000);
    let clip = save(
        &mut store,
        "clip",
        Source::Clipboard,
        vec![Payload::text("共享内容")],
    );
    let payloads = store.payloads(&clip.id).unwrap();
    let stage = save(&mut store, "stage", Source::Transfer, payloads);
    save(
        &mut store,
        "favorite",
        Source::Clipboard,
        vec![Payload::text("收藏")],
    );
    store.favorite("favorite", true).unwrap();
    store.clear_clipboard().unwrap();
    assert_eq!(store.snapshot().unwrap().clipboard.len(), 1);
    assert_eq!(store.preview(&stage.id).unwrap().0.unwrap(), "共享内容");
    store.delete(&[stage.id]).unwrap();
    assert_eq!(store.snapshot().unwrap().transfer.len(), 0);
}

#[test]
fn quota_evicts_only_unpinned_history_and_preserves_in_progress_assets() {
    let f = Fixture::new();
    let mut store = f.store(12);
    save(
        &mut store,
        "old",
        Source::Clipboard,
        vec![Payload::text("old content!")],
    );
    let entry = save(
        &mut store,
        "new",
        Source::Transfer,
        vec![Payload::Text {
            plain: "123456".into(),
            html: Some(b"abcdef".to_vec()),
            rtf: None,
        }],
    );
    assert_eq!(entry.state, State::Ready);
    assert_eq!(store.snapshot().unwrap().bytes, 12);
    assert_eq!(store.snapshot().unwrap().clipboard.len(), 0);
    assert_eq!(store.payloads(&entry.id).unwrap().len(), 1);
    let rejected = save(
        &mut store,
        "full",
        Source::Transfer,
        vec![Payload::text("x")],
    );
    assert_eq!(rejected.state, State::Failed);
    assert_eq!(store.preview("new").unwrap().0.unwrap(), "123456");
}

#[test]
fn retention_counts_and_age_preserve_transfer_and_favorites() {
    let f = Fixture::new();
    let mut store = f.store(1000);
    store.policy.limit = 2;
    save(&mut store, "a", Source::Clipboard, vec![Payload::text("a")]);
    store.favorite("a", true).unwrap();
    for id in ["b", "c", "d"] {
        save(&mut store, id, Source::Clipboard, vec![Payload::text(id)]);
    }
    save(
        &mut store,
        "s",
        Source::Transfer,
        vec![Payload::text("stage")],
    );
    assert_eq!(store.snapshot().unwrap().clipboard.len(), 3);
    store
        .cleanup(desktop_pet::safety::timestamp() + 31 * 86400)
        .unwrap();
    assert_eq!(store.snapshot().unwrap().clipboard.len(), 1);
    assert_eq!(store.snapshot().unwrap().transfer.len(), 1);
}

#[test]
fn folder_structure_same_names_and_partial_failure_are_preserved() {
    let f = Fixture::new();
    let folder = f.path.join("资料");
    fs::create_dir_all(folder.join("子目录")).unwrap();
    fs::write(folder.join("子目录/文件.txt"), "内容").unwrap();
    let mut store = f.store(1000);
    let entry = save(
        &mut store,
        "group",
        Source::Transfer,
        vec![Payload::Files(vec![folder.clone(), f.path.join("missing")])],
    );
    assert_eq!(entry.state, State::Partial);
    assert!(!entry.errors.is_empty());
    assert!(
        store
            .preview(&entry.id)
            .unwrap()
            .0
            .unwrap()
            .contains("文件.txt")
    );
    let (files, _) = store.export(&[entry.id], &AtomicBool::new(false)).unwrap();
    assert_eq!(
        fs::read_to_string(files[0].join("子目录/文件.txt")).unwrap(),
        "内容"
    );
    assert!(folder.exists());
}

#[test]
fn cancellation_and_failed_download_keep_recoverable_records() {
    let f = Fixture::new();
    let mut store = f.store(1000);
    let cancelled = store
        .import(
            "cancel",
            Source::Transfer,
            vec![Payload::text("no")],
            &AtomicBool::new(true),
            &mut |_, _| {},
        )
        .unwrap();
    assert_eq!(cancelled.state, State::Failed);
    assert_eq!(store.snapshot().unwrap().bytes, 0);
    let entry = save(
        &mut store,
        "remote",
        Source::Transfer,
        vec![Payload::Remote {
            url: "blob:https://example.com/unavailable".into(),
            name: None,
        }],
    );
    assert_eq!(entry.state, State::Partial);
    assert!(entry.errors[0].contains("下载失败"));
    assert!(
        store
            .preview(&entry.id)
            .unwrap()
            .0
            .unwrap()
            .starts_with("blob:")
    );
}

#[test]
fn image_and_rich_text_roundtrip_without_execution() {
    let f = Fixture::new();
    let mut store = f.store(1024 * 1024);
    let mut png = std::io::Cursor::new(Vec::new());
    image::RgbaImage::from_pixel(8, 9, image::Rgba([1, 2, 3, 255]))
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let entry = save(
        &mut store,
        "image",
        Source::Clipboard,
        vec![Payload::Image(png.into_inner())],
    );
    assert!(matches!(
        entry.items[0],
        Item::Image {
            width: 8,
            height: 9,
            ..
        }
    ));
    let preview = store.preview(&entry.id).unwrap().1.unwrap();
    assert_eq!((preview.1, preview.2), (8, 9));
    let text = save(
        &mut store,
        "html",
        Source::Clipboard,
        vec![Payload::Text {
            plain: "可见文字".into(),
            html: Some(b"<script>throw 1</script>".to_vec()),
            rtf: Some(b"{\\rtf1 text}".to_vec()),
        }],
    );
    assert_eq!(store.preview(&text.id).unwrap().0.unwrap(), "可见文字");
    let payloads = store.payloads(&text.id).unwrap();
    assert!(matches!(
        &payloads[0],
        Payload::Text {
            html: Some(_),
            rtf: Some(_),
            ..
        }
    ));
}

#[test]
fn temporary_attachments_are_backed_up_then_removed_only_from_owned_directory() {
    let f = Fixture::new();
    let mut store = f.store(1000);
    let directory = store.root.join("incoming/attachment");
    fs::create_dir(&directory).unwrap();
    let path = directory.join("附件.txt");
    fs::write(&path, "attachment").unwrap();
    let entry = save(
        &mut store,
        "attachment",
        Source::Transfer,
        vec![Payload::TemporaryFiles {
            paths: vec![path],
            directory: directory.clone(),
            errors: vec![],
        }],
    );
    assert_eq!(entry.state, State::Ready);
    assert!(!directory.exists());
    let outside = f.path.join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("keep"), "keep").unwrap();
    let rejected = save(
        &mut store,
        "bad",
        Source::Transfer,
        vec![Payload::TemporaryFiles {
            paths: vec![],
            directory: outside.clone(),
            errors: vec![],
        }],
    );
    assert_eq!(rejected.state, State::Failed);
    assert!(outside.join("keep").exists());
}

fn local_response(
    response: Vec<u8>,
    stall: std::time::Duration,
) -> (String, std::thread::JoinHandle<()>) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/sample.bin", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    let worker = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            match listener.accept() {
                Ok((mut socket, _)) => {
                    socket
                        .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                        .unwrap();
                    let mut request = [0; 4096];
                    let _ = socket.read(&mut request);
                    let _ = socket.write_all(&response);
                    std::thread::sleep(stall);
                    break;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "No request reached the local fixture"
                    );
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(e) => panic!("{e}"),
            }
        }
    });
    (url, worker)
}

#[test]
fn http_download_creates_an_independent_file_and_html_or_auth_failures_keep_origin() {
    let f = Fixture::new();
    let mut store = f.store(1024 * 1024);
    let (url, server) = local_response(b"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: 7\r\nConnection: close\r\n\r\npayload".to_vec(), std::time::Duration::ZERO);
    let downloaded = save(
        &mut store,
        "download",
        Source::Transfer,
        vec![Payload::Remote {
            url: url.clone(),
            name: Some("下载内容.bin".into()),
        }],
    );
    server.join().unwrap();
    assert_eq!(downloaded.state, State::Ready);
    assert_eq!(downloaded.origin.as_deref(), Some(url.as_str()));
    let (paths, _) = store
        .export(&[downloaded.id], &AtomicBool::new(false))
        .unwrap();
    assert_eq!(fs::read(&paths[0]).unwrap(), b"payload");
    assert_eq!(paths[0].file_name().unwrap(), "下载内容.bin");
    for (id, status, kind, body) in [
        ("login", "200 OK", "text/html", "<html>Log in</html>"),
        ("auth", "401 Unauthorized", "text/plain", "Unauthorized"),
    ] {
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let (url, server) = local_response(response.into_bytes(), std::time::Duration::ZERO);
        let entry = save(
            &mut store,
            id,
            Source::Transfer,
            vec![Payload::Remote {
                url: url.clone(),
                name: None,
            }],
        );
        server.join().unwrap();
        assert_eq!(entry.state, State::Partial);
        assert_eq!(entry.origin.as_deref(), Some(url.as_str()));
        assert_eq!(store.preview(&entry.id).unwrap().0.unwrap(), url);
        assert!(!entry.errors.is_empty());
    }
}

#[test]
fn cancellation_interrupts_a_stalled_http_body_and_removes_partial_files() {
    let f = Fixture::new();
    let mut store = f.store(1024 * 1024);
    let (url, server) = local_response(
        b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\nConnection: close\r\n\r\nx".to_vec(),
        std::time::Duration::from_millis(500),
    );
    let cancel = std::sync::Arc::new(AtomicBool::new(false));
    let token = cancel.clone();
    let trigger = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(150));
        token.store(true, Ordering::Release);
    });
    let entry = store
        .import(
            "cancel-http",
            Source::Transfer,
            vec![Payload::Remote { url, name: None }],
            &cancel,
            &mut |_, _| {},
        )
        .unwrap();
    trigger.join().unwrap();
    server.join().unwrap();
    assert!(entry.errors.iter().any(|e| e.contains("取消")));
    assert_eq!(fs::read_dir(store.root.join("tmp")).unwrap().count(), 0);
    assert!(entry.items.iter().all(|i| !matches!(i, Item::Files(_))));
}

#[test]
fn protected_exports_survive_expiry_then_are_released_without_deleting_library_content() {
    let f = Fixture::new();
    let mut store = f.store(1024);
    save(
        &mut store,
        "keep",
        Source::Transfer,
        vec![Payload::text("保留的内容")],
    );
    let (files, _) = store
        .export(&["keep".into()], &AtomicBool::new(false))
        .unwrap();
    store.protect_exports(&files).unwrap();
    let later = desktop_pet::safety::timestamp() + 86401;
    store.cleanup(later).unwrap();
    assert!(files[0].exists());
    store.release_exports().unwrap();
    store.cleanup(later).unwrap();
    assert!(!files[0].exists());
    assert_eq!(store.preview("keep").unwrap().0.unwrap(), "保留的内容");
}

#[test]
fn restarting_recovers_interrupted_records_and_removes_unpublished_data() {
    let f = Fixture::new();
    let mut store = f.store(1024);
    save(
        &mut store,
        "ready",
        Source::Transfer,
        vec![Payload::text("完整内容")],
    );
    let mut interrupted = store.get("ready").unwrap();
    interrupted.id = "interrupted".into();
    interrupted.state = State::Saving;
    interrupted.items.clear();
    let root = store.root.clone();
    drop(store);
    let connection = rusqlite::Connection::open(root.join("index.sqlite")).unwrap();
    connection
        .execute(
            "INSERT INTO entries VALUES(?, 'transfer', ?, 0, '', '', ?)",
            rusqlite::params![
                interrupted.id,
                interrupted.timestamp,
                serde_json::to_string(&interrupted).unwrap()
            ],
        )
        .unwrap();
    drop(connection);
    fs::write(root.join("tmp/partial"), "partial").unwrap();
    fs::write(root.join("incoming/attachment"), "partial").unwrap();
    fs::write(root.join("assets/orphan"), "orphan").unwrap();
    let store = f.store(1024);
    assert_eq!(store.get("interrupted").unwrap().state, State::Failed);
    assert!(store.get("interrupted").unwrap().errors[0].contains("中断"));
    assert_eq!(store.preview("ready").unwrap().0.unwrap(), "完整内容");
    for folder in ["tmp", "incoming"] {
        assert_eq!(fs::read_dir(root.join(folder)).unwrap().count(), 0);
    }
    assert!(!root.join("assets/orphan").exists());
}

#[test]
fn service_collects_a_file_group_and_reports_the_real_item_count() {
    use desktop_pet::content::{Event, Service};
    let f = Fixture::new();
    let files = [f.path.join("甲.txt"), f.path.join("乙.txt")];
    for path in &files {
        fs::write(path, "内容").unwrap();
    }
    let mut service = Service::start(f.path.clone(), Policy::default(), || {});
    let id = service.import(Source::Clipboard, vec![Payload::Files(files.to_vec())]);
    loop {
        if let Event::Completed { id: complete, .. } = service
            .receiver
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap()
        {
            assert_eq!(complete, id);
            break;
        }
    }
    service.collect(&id);
    let collected = loop {
        if let Event::Completed {
            id,
            source: Source::Transfer,
            count,
        } = service
            .receiver
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap()
        {
            assert_eq!(count, 2);
            break id;
        }
    };
    service.stop();
    for path in &files {
        fs::remove_file(path).unwrap();
    }
    let mut store = f.store(1024);
    let (paths, _) = store.export(&[collected], &AtomicBool::new(false)).unwrap();
    assert_eq!(paths.len(), 2);
    assert!(
        paths
            .iter()
            .all(|path| fs::read_to_string(path).unwrap() == "内容")
    );
}
