mod store;
pub use store::{Store, bytes_label};

use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Source {
    Clipboard,
    Transfer,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Payload {
    Text {
        plain: String,
        html: Option<Vec<u8>>,
        rtf: Option<Vec<u8>>,
    },
    Image(#[serde(with = "bytes_base64")] Vec<u8>),
    Files(Vec<PathBuf>),
    // Native promises are materialized into this owned temporary directory.
    TemporaryFiles {
        paths: Vec<PathBuf>,
        directory: PathBuf,
        errors: Vec<String>,
    },
    Remote {
        url: String,
        name: Option<String>,
    },
    Error(String),
}

mod bytes_base64 {
    use base64::Engine;
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&base64::engine::general_purpose::STANDARD.encode(bytes))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        let value = String::deserialize(deserializer)?;
        base64::engine::general_purpose::STANDARD
            .decode(value)
            .map_err(serde::de::Error::custom)
    }
}

impl Payload {
    pub fn text(value: impl Into<String>) -> Self {
        Self::Text {
            plain: value.into(),
            html: None,
            rtf: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileItem {
    pub name: String,
    pub path: PathBuf,
    pub managed: bool,
    pub directory: bool,
    pub bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Item {
    Text {
        asset: String,
        html: Option<String>,
        rtf: Option<String>,
        link: bool,
    },
    Image {
        asset: String,
        width: u32,
        height: u32,
    },
    Files(Vec<FileItem>),
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum State {
    Saving,
    Ready,
    Partial,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub source: Source,
    pub timestamp: u64,
    pub favorite: bool,
    pub title: String,
    pub preview: String,
    pub kind: String,
    pub state: State,
    pub items: Vec<Item>,
    pub errors: Vec<String>,
    #[serde(default)]
    pub origin: Option<String>,
}

impl Entry {
    fn count(&self) -> usize {
        self.items
            .iter()
            .map(|item| match item {
                Item::Files(files) => files.len(),
                _ => 1,
            })
            .sum()
    }
}

#[derive(Clone, Debug)]
pub struct Policy {
    pub days: u32,
    pub limit: u32,
    pub quota: u64,
    pub download: bool,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            days: 30,
            limit: 500,
            quota: 5 * 1024 * 1024 * 1024,
            download: true,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Snapshot {
    pub clipboard: Vec<Entry>,
    pub transfer: Vec<Entry>,
    pub bytes: u64,
    pub export_bytes: u64,
}

#[derive(Debug)]
pub enum Event {
    Snapshot(Snapshot),
    Progress {
        id: String,
        label: String,
        bytes: u64,
    },
    Completed {
        id: String,
        source: Source,
        count: usize,
    },
    Preview {
        id: String,
        text: Option<String>,
        image: Option<(Vec<u8>, u32, u32)>,
    },
    WebPreview {
        id: String,
        text: Option<String>,
        image: Option<String>,
    },
    Exported {
        id: String,
        files: Vec<PathBuf>,
        payloads: Vec<Payload>,
    },
    Search {
        source: Source,
        query: String,
        entries: Vec<Entry>,
    },
    Saved {
        count: usize,
        errors: Vec<String>,
    },
    Finished(String),
    Error(String),
    Explorer {
        root: PathBuf,
        bytes: u64,
        error: Option<String>,
    },
    Storage {
        settings: crate::storage::Settings,
        error: Option<String>,
        completed: bool,
    },
}

enum Command {
    Import {
        id: String,
        source: Source,
        payloads: Vec<Payload>,
        cancel: Arc<AtomicBool>,
    },
    Collect {
        id: String,
        task_id: String,
        cancel: Arc<AtomicBool>,
    },
    Favorite(String, bool),
    Delete(Vec<String>),
    ClearClipboard,
    Preview(String),
    PreviewWeb(String),
    Export(Vec<String>, Arc<AtomicBool>),
    ExportPicks(String, Vec<crate::delivery::Pick>, Arc<AtomicBool>),
    Save(Vec<PathBuf>, PathBuf, Arc<AtomicBool>),
    ReleaseExports,
    ProtectExports(Vec<PathBuf>),
    Search(Source, String),
    Policy(Policy),
    Explorer(bool, bool),
    MoveStorage(String, PathBuf, Arc<AtomicBool>),
    Stop,
}

pub struct Service {
    sender: mpsc::Sender<Command>,
    pub receiver: mpsc::Receiver<Event>,
    tasks: std::collections::HashMap<String, Arc<AtomicBool>>,
    handle: Option<thread::JoinHandle<()>>,
    serial: u64,
    explorer_cancel: Arc<AtomicBool>,
}

fn progress_callback(id: String, emit: &dyn Fn(Event)) -> impl FnMut(&str, u64) + '_ {
    let mut last = std::time::Instant::now();
    let mut first = true;
    move |label, bytes| {
        if first || bytes == 0 || last.elapsed() >= std::time::Duration::from_millis(100) {
            emit(Event::Progress {
                id: id.clone(),
                label: label.into(),
                bytes,
            });
            last = std::time::Instant::now();
            first = false;
        }
    }
}

impl Service {
    pub fn start(
        directory: PathBuf,
        policy: Policy,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let (sender, commands) = mpsc::channel();
        let (events, receiver) = mpsc::channel();
        let wake = Arc::new(wake);
        let explorer_cancel = Arc::new(AtomicBool::new(false));
        let view_cancel = explorer_cancel.clone();
        let handle = thread::spawn(move || {
            let emit = |event| {
                let _ = events.send(event);
                wake();
            };
            let mut storage =
                match crate::storage::Settings::load(&directory).and_then(|settings| {
                    settings.validate(&directory)?;
                    Ok(settings)
                }) {
                    Ok(settings) => settings,
                    Err(error) => {
                        emit(Event::Storage {
                            settings: Default::default(),
                            error: Some(error.to_string()),
                            completed: false,
                        });
                        emit(Event::Error(format!(
                            "存储配置无法读取，原文件未覆盖：{error}"
                        )));
                        return;
                    }
                };
            if let Err(error) = crate::explorer::recover_storage(&directory, &storage) {
                emit(Event::Error(format!("文件箱入口恢复未完成：{error}")));
            }
            emit(Event::Storage {
                settings: storage.clone(),
                error: None,
                completed: false,
            });
            let root = storage.content(&directory);
            if storage.content.is_some() && !root.join("index.sqlite").is_file() {
                emit(Event::Storage {
                    settings: storage.clone(),
                    error: Some("所选内容库不可用，请连接原磁盘后重启".into()),
                    completed: false,
                });
                emit(Event::Error(
                    "所选内容库不可用，请连接原磁盘后重启；未创建空库或更改存储配置".into(),
                ));
                return;
            }
            let mut store = match Store::open_at(&root, policy) {
                Ok(s) => s,
                Err(e) => {
                    emit(Event::Storage {
                        settings: storage.clone(),
                        error: Some(format!("内容库无法打开：{e}")),
                        completed: false,
                    });
                    emit(Event::Error(format!("内容库无法打开：{e}")));
                    return;
                }
            };
            let refresh = |store: &Store| match store.snapshot() {
                Ok(snapshot) => emit(Event::Snapshot(snapshot)),
                Err(e) => emit(Event::Error(e.to_string())),
            };
            refresh(&store);
            let mut explorer_enabled = false;
            let mut explorer_registered = false;
            loop {
                let command = match commands.recv_timeout(std::time::Duration::from_secs(60)) {
                    Ok(c) => c,
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        if let Err(e) = store.cleanup(crate::safety::timestamp()) {
                            emit(Event::Error(e.to_string()));
                        }
                        refresh(&store);
                        continue;
                    }
                    Err(_) => break,
                };
                if matches!(&command, Command::Stop) {
                    break;
                }
                let refresh_after = !matches!(
                    &command,
                    Command::Search(..) | Command::Preview(..) | Command::PreviewWeb(..)
                );
                let view_after = matches!(
                    &command,
                    Command::Import { .. }
                        | Command::Collect { .. }
                        | Command::Favorite(..)
                        | Command::Delete(..)
                        | Command::ClearClipboard
                        | Command::Explorer(true, _)
                        | Command::MoveStorage(..)
                );
                let view_force = matches!(&command, Command::Explorer(true, true));
                let task_id = match &command {
                    Command::Import { id, .. } => Some(id.clone()),
                    Command::Collect { task_id, .. } => Some(task_id.clone()),
                    Command::Export(ids, _) => Some(ids.join("|")),
                    Command::ExportPicks(key, ..) => Some(key.clone()),
                    Command::Save(..) => Some("save-export".into()),
                    Command::MoveStorage(..) => Some("storage-move".into()),
                    _ => None,
                };
                let result = (|| -> Result<(), Box<dyn std::error::Error>> {
                    match command {
                        Command::Stop => return Ok(()),
                        Command::Import {
                            id,
                            source,
                            payloads,
                            cancel,
                        } => {
                            let mut progress = progress_callback(id.clone(), &emit);
                            let entry =
                                store.import(&id, source, payloads, &cancel, &mut progress)?;
                            let count = entry.count();
                            emit(Event::Completed { id, source, count });
                        }
                        Command::Collect {
                            id,
                            task_id,
                            cancel,
                        } => {
                            let payloads = store.payloads(&id)?;
                            let mut progress = progress_callback(task_id.clone(), &emit);
                            let entry = store.import(
                                &task_id,
                                Source::Transfer,
                                payloads,
                                &cancel,
                                &mut progress,
                            )?;
                            emit(Event::Completed {
                                id: task_id,
                                source: Source::Transfer,
                                count: entry.count(),
                            });
                        }
                        Command::Favorite(id, favorite) => store.favorite(&id, favorite)?,
                        Command::Delete(ids) => store.delete(&ids)?,
                        Command::ClearClipboard => store.clear_clipboard()?,
                        Command::Preview(id) => {
                            let (text, image) = store.preview(&id)?;
                            emit(Event::Preview { id, text, image });
                        }
                        Command::PreviewWeb(id) => {
                            use base64::Engine;
                            let (text, image) = store.preview(&id)?;
                            let image = image
                                .map(|(rgba, width, height)| {
                                    let image = image::RgbaImage::from_raw(width, height, rgba)
                                        .ok_or("Invalid preview pixels")?;
                                    let mut png = std::io::Cursor::new(Vec::new());
                                    image.write_to(&mut png, image::ImageFormat::Png)?;
                                    Ok::<_, Box<dyn std::error::Error>>(format!(
                                        "data:image/png;base64,{}",
                                        base64::engine::general_purpose::STANDARD
                                            .encode(png.into_inner())
                                    ))
                                })
                                .transpose()?;
                            emit(Event::WebPreview { id, text, image });
                        }
                        Command::Export(ids, cancel) => {
                            let mut progress = progress_callback(ids.join("|"), &emit);
                            let (files, payloads) =
                                store.export_with_progress(&ids, &cancel, &mut progress)?;
                            emit(Event::Exported {
                                id: ids.join("|"),
                                files,
                                payloads,
                            });
                        }
                        Command::ExportPicks(id, picks, cancel) => {
                            let mut progress = progress_callback(id.clone(), &emit);
                            let (files, payloads) =
                                match store.export_picks(&picks, &cancel, &mut progress) {
                                    Ok(export) => export,
                                    // Changing the selection supersedes its old prefetch.
                                    // Do not fail unrelated panel requests or show a toast.
                                    Err(_) if cancel.load(Ordering::Acquire) => return Ok(()),
                                    Err(error) => return Err(error),
                                };
                            emit(Event::Exported {
                                id,
                                files,
                                payloads,
                            });
                        }
                        Command::Save(files, directory, cancel) => {
                            let mut progress = progress_callback("save-export".into(), &emit);
                            let (count, errors) = store.save_exports_with_progress(
                                &files,
                                &directory,
                                &cancel,
                                &mut progress,
                            );
                            emit(Event::Saved { count, errors });
                        }
                        Command::ReleaseExports => store.release_exports()?,
                        Command::ProtectExports(files) => store.protect_exports(&files)?,
                        Command::Search(source, query) => emit(Event::Search {
                            source,
                            entries: store.search(source, &query)?,
                            query,
                        }),
                        Command::Policy(policy) => {
                            store.policy = policy;
                            store.cleanup(crate::safety::timestamp())?;
                        }
                        Command::Explorer(enabled, _) => {
                            explorer_enabled = enabled;
                            if !enabled {
                                #[cfg(windows)]
                                crate::explorer::registration::uninstall(
                                    &storage.explorer(&directory),
                                )?;
                                explorer_registered = false;
                            }
                        }
                        Command::MoveStorage(kind, target, cancel) => {
                            let mut progress = progress_callback("storage-move".into(), &emit);
                            let result = (|| -> Result<(), Box<dyn std::error::Error>> {
                                let current = if kind == "content" {
                                    storage.content(&directory)
                                } else {
                                    storage.explorer(&directory)
                                };
                                if crate::storage::normalized(&current)?
                                    == crate::storage::normalized(&target)?
                                {
                                    return Ok(());
                                }
                                if kind == "content" {
                                    let mut next = storage.clone();
                                    next.content = Some(crate::storage::normalized(&target)?);
                                    next.validate(&directory)?;
                                    store.relocate(&target, &cancel, &mut progress, |root| {
                                        next.content = Some(root.to_owned());
                                        next.save(&directory)?;
                                        Ok(())
                                    })?;
                                    storage = next;
                                } else if kind == "explorer" {
                                    storage = crate::explorer::relocate(
                                        &directory,
                                        &storage,
                                        &target,
                                        explorer_enabled,
                                        &cancel,
                                        &mut progress,
                                    )?;
                                    explorer_registered = explorer_enabled;
                                } else {
                                    return Err("未知存储目录类型".into());
                                }
                                Ok(())
                            })();
                            emit(Event::Storage {
                                settings: storage.clone(),
                                error: result.err().map(|error| error.to_string()),
                                completed: true,
                            });
                        }
                    }
                    Ok(())
                })();
                if let Err(e) = result {
                    emit(Event::Error(e.to_string()));
                }
                if let Some(id) = task_id {
                    emit(Event::Finished(id));
                }
                if refresh_after {
                    refresh(&store);
                }
                if view_after && explorer_enabled {
                    emit(Event::Progress {
                        id: "explorer-view".into(),
                        label: "正在更新文件箱展示".into(),
                        bytes: 0,
                    });
                    let root = storage.explorer(&directory);
                    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
                        let snapshot = store.snapshot()?;
                        crate::explorer::publish_at(
                            &root,
                            &snapshot,
                            store.policy.quota,
                            view_force,
                            &view_cancel,
                            |entry| store.view_sources(entry),
                        )?;
                        if !explorer_registered {
                            #[cfg(windows)]
                            crate::explorer::registration::install(&root)?;
                            explorer_registered = true;
                        }
                        Ok(())
                    })();
                    emit(Event::Explorer {
                        bytes: crate::explorer::bytes(&root).unwrap_or(0),
                        root,
                        error: result.err().map(|error| error.to_string()),
                    });
                    emit(Event::Finished("explorer-view".into()));
                }
            }
        });
        Self {
            sender,
            receiver,
            tasks: Default::default(),
            handle: Some(handle),
            serial: 0,
            explorer_cancel,
        }
    }
    fn token(&mut self, id: &str) -> Arc<AtomicBool> {
        let token = Arc::new(AtomicBool::new(false));
        self.tasks.insert(id.into(), token.clone());
        token
    }
    pub fn import(&mut self, source: Source, payloads: Vec<Payload>) -> String {
        self.serial += 1;
        let id = format!(
            "{}-{}-{}",
            crate::safety::timestamp(),
            std::process::id(),
            self.serial
        );
        let cancel = self.token(&id);
        let _ = self.sender.send(Command::Import {
            id: id.clone(),
            source,
            payloads,
            cancel,
        });
        id
    }
    pub fn collect(&mut self, id: &str) {
        self.serial += 1;
        let task_id = format!(
            "collect-{}-{}-{}",
            crate::safety::timestamp(),
            std::process::id(),
            self.serial
        );
        let cancel = self.token(&task_id);
        let _ = self.sender.send(Command::Collect {
            id: id.into(),
            task_id,
            cancel,
        });
    }
    pub fn cancel(&self, id: &str) {
        if id == "explorer-view" {
            self.explorer_cancel.store(true, Ordering::Release);
        }
        if let Some(token) = self.tasks.get(id) {
            token.store(true, Ordering::Release);
        }
    }
    pub fn completed(&mut self, id: &str) {
        self.tasks.remove(id);
    }
    pub fn favorite(&self, id: &str, value: bool) {
        let _ = self.sender.send(Command::Favorite(id.into(), value));
    }
    pub fn delete(&self, ids: Vec<String>) {
        let _ = self.sender.send(Command::Delete(ids));
    }
    pub fn clear_clipboard(&self) {
        let _ = self.sender.send(Command::ClearClipboard);
    }
    pub fn preview(&self, id: &str) {
        let _ = self.sender.send(Command::Preview(id.into()));
    }
    pub fn preview_web(&self, id: &str) {
        let _ = self.sender.send(Command::PreviewWeb(id.into()));
    }
    pub fn export(&mut self, ids: Vec<String>) {
        let key = ids.join("|");
        let cancel = self.token(&key);
        let _ = self.sender.send(Command::Export(ids, cancel));
    }
    pub fn save_exports(&mut self, files: Vec<PathBuf>, directory: PathBuf) {
        let cancel = self.token("save-export");
        let _ = self.sender.send(Command::Save(files, directory, cancel));
    }
    pub fn export_picks(&mut self, key: String, picks: Vec<crate::delivery::Pick>) {
        let cancel = self.token(&key);
        let _ = self.sender.send(Command::ExportPicks(key, picks, cancel));
    }
    pub fn release_exports(&self) {
        let _ = self.sender.send(Command::ReleaseExports);
    }
    pub fn protect_exports(&self, files: Vec<PathBuf>) {
        let _ = self.sender.send(Command::ProtectExports(files));
    }
    pub fn search(&self, source: Source, query: &str) {
        let _ = self.sender.send(Command::Search(source, query.into()));
    }
    pub fn policy(&self, policy: Policy) {
        let _ = self.sender.send(Command::Policy(policy));
    }
    pub fn stop(&mut self) {
        self.explorer_cancel.store(true, Ordering::Release);
        for token in self.tasks.values() {
            token.store(true, Ordering::Release);
        }
        let _ = self.sender.send(Command::Stop);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
    pub fn explorer(&self, enabled: bool, force: bool) {
        self.explorer_cancel.store(!enabled, Ordering::Release);
        let _ = self.sender.send(Command::Explorer(enabled, force));
    }
    pub fn move_storage(&mut self, kind: &str, target: PathBuf) -> Result<(), String> {
        let cancel = self.token("storage-move");
        self.sender
            .send(Command::MoveStorage(kind.into(), target, cancel))
            .map_err(|_| "内容服务不可用，请检查已选择的存储目录并重启".into())
    }
}
impl Drop for Service {
    fn drop(&mut self) {
        self.stop();
    }
}
