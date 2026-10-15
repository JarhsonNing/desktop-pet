use super::*;
use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, Read, Write},
    path::{Component, Path},
    time::Duration,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
type Preview = (Option<String>, Option<(Vec<u8>, u32, u32)>);

fn export_target(directory: &Path, name: &str, used: &[PathBuf], serial: &mut u64) -> PathBuf {
    let mut path = directory.join(name);
    while used.contains(&path) {
        *serial += 1;
        path = directory.join(format!("{serial}-{name}"));
    }
    path
}

pub fn bytes_label(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    } else if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

pub struct Store {
    pub root: PathBuf,
    db: Connection,
    pub policy: Policy,
    active: Option<String>,
    warnings: Vec<String>,
    incoming: Vec<PathBuf>,
    _lock: fs::File,
}

impl Store {
    pub fn view_sources(&self, entry: &Entry) -> Result<Vec<crate::explorer::ViewSource>> {
        use crate::explorer::ViewSource;
        let mut output = Vec::new();
        for (index, item) in entry.items.iter().enumerate() {
            match item {
                Item::Text { asset, .. } => output.push(ViewSource::Bytes {
                    name: format!("文本-{}.txt", index + 1),
                    bytes: self.read_blob(asset)?,
                }),
                Item::Image { asset, .. } => output.push(ViewSource::Bytes {
                    name: format!("图片-{}.png", index + 1),
                    bytes: self.read_blob(asset)?,
                }),
                Item::Files(files) => {
                    for file in files {
                        let path = if file.managed {
                            self.asset(file.path.to_str().ok_or("文件路径无效")?)?
                        } else {
                            file.path.clone()
                        };
                        if path.exists() {
                            output.push(ViewSource::File {
                                name: file.name.clone(),
                                path,
                            });
                        } else {
                            output.push(ViewSource::Bytes {
                                name: format!("{}.失效说明.txt", file.name),
                                bytes: format!("原文件已不存在：{}", file.path.display())
                                    .into_bytes(),
                            });
                        }
                    }
                }
            }
        }
        Ok(output)
    }
    pub fn open(directory: &Path, policy: Policy) -> Result<Self> {
        Self::open_at(&directory.join("content"), policy)
    }
    pub fn open_at(root: &Path, policy: Policy) -> Result<Self> {
        for folder in ["assets", "tmp", "incoming", "exports"] {
            fs::create_dir_all(root.join(folder))?;
        }
        let root = root.canonicalize()?;
        let lock = crate::safety::lock(&root.join("library.lock"))?;
        let db = Connection::open(root.join("index.sqlite"))?;
        db.busy_timeout(Duration::from_secs(2))?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;
            CREATE TABLE IF NOT EXISTS entries(id TEXT PRIMARY KEY, source TEXT NOT NULL, stamp INTEGER NOT NULL, favorite INTEGER NOT NULL, fingerprint TEXT NOT NULL, search TEXT NOT NULL, data TEXT NOT NULL);
            CREATE INDEX IF NOT EXISTS entries_order ON entries(source,stamp DESC);
            CREATE INDEX IF NOT EXISTS entries_hash ON entries(source,fingerprint);
            CREATE TABLE IF NOT EXISTS assets(hash TEXT PRIMARY KEY, bytes INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS entry_assets(entry TEXT REFERENCES entries(id) ON DELETE CASCADE, asset TEXT REFERENCES assets(hash), PRIMARY KEY(entry,asset));
            CREATE TABLE IF NOT EXISTS exports(name TEXT PRIMARY KEY, stamp INTEGER NOT NULL, protected INTEGER NOT NULL DEFAULT 0);
            PRAGMA user_version=1;")?;
        let mut store = Self {
            root: root.clone(),
            db,
            policy,
            active: None,
            warnings: vec![],
            incoming: vec![root.join("incoming")],
            _lock: lock,
        };
        // Recover interrupted imports before exposing records to the UI.
        for mut entry in store.all()? {
            if entry.state == State::Saving {
                entry.state = State::Failed;
                entry.errors.push("上次保存被中断，请重新拖入内容。".into());
                store.db.execute(
                    "UPDATE entries SET data=? WHERE id=?",
                    params![serde_json::to_string(&entry)?, entry.id],
                )?;
            }
        }
        store.clear_folder("tmp")?;
        store.clear_folder("incoming")?;
        store.cleanup(crate::safety::timestamp())?;
        Ok(store)
    }

    pub fn relocate(
        &mut self,
        target: &Path,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(&str, u64),
        commit: impl FnOnce(&Path) -> Result<()>,
    ) -> Result<PathBuf> {
        let mut staging = crate::storage::Staging::new(&self.root, target)?;
        let mut bytes = 0;
        for name in ["assets", "exports"] {
            crate::storage::copy_tree(
                &self.root.join(name),
                &staging.path.join(name),
                cancel,
                &mut bytes,
                progress,
            )?;
        }
        for name in ["tmp", "incoming"] {
            fs::create_dir(staging.path.join(name))?;
        }
        let database = staging.path.join("index.sqlite");
        self.db.execute(
            "VACUUM main INTO ?1",
            [database.to_str().ok_or("数据库路径无效")?],
        )?;
        if cancel.load(Ordering::Acquire) {
            return Err("目录切换已取消，继续使用原目录".into());
        }
        let root = staging.publish()?;
        let lock = crate::safety::lock(&root.join("library.lock"))?;
        let db = Connection::open(root.join("index.sqlite"))?;
        db.busy_timeout(Duration::from_secs(2))?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        // Until this durable commit succeeds the old database stays authoritative.
        commit(&root)?;
        self.db = db;
        self.root = root.clone();
        self._lock = lock;
        self.incoming.push(root.join("incoming"));
        Ok(root)
    }

    fn clear_folder(&self, folder: &str) -> Result<()> {
        for entry in fs::read_dir(self.root.join(folder))? {
            remove_owned(&entry?.path())?;
        }
        Ok(())
    }
    fn all(&self) -> Result<Vec<Entry>> {
        let mut stmt = self
            .db
            .prepare("SELECT data FROM entries ORDER BY stamp DESC,rowid DESC")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        rows.map(|row| Ok(serde_json::from_str(&row?)?)).collect()
    }
    pub fn get(&self, id: &str) -> Result<Entry> {
        let json: String =
            self.db
                .query_row("SELECT data FROM entries WHERE id=?", [id], |row| {
                    row.get(0)
                })?;
        Ok(serde_json::from_str(&json)?)
    }
    pub fn snapshot(&self) -> Result<Snapshot> {
        let mut snapshot = Snapshot {
            bytes: self.used()?,
            export_bytes: tree_bytes(&self.root.join("exports"))?,
            ..Default::default()
        };
        for entry in self.all()? {
            if entry.source == Source::Clipboard {
                snapshot.clipboard.push(entry);
            } else {
                snapshot.transfer.push(entry);
            }
        }
        Ok(snapshot)
    }
    fn used(&self) -> Result<u64> {
        Ok(self
            .db
            .query_row("SELECT COALESCE(SUM(bytes),0) FROM assets", [], |row| {
                row.get(0)
            })?)
    }
    pub fn search(&self, source: Source, query: &str) -> Result<Vec<Entry>> {
        let term = format!(
            "%{}%",
            query
                .replace('!', "!!")
                .replace('%', "!%")
                .replace('_', "!_")
        );
        let mut stmt = self.db.prepare("SELECT data FROM entries WHERE source=? AND search LIKE ? ESCAPE '!' ORDER BY stamp DESC,rowid DESC")?;
        let rows = stmt.query_map(params![source_key(source), term], |r| r.get::<_, String>(0))?;
        rows.map(|row| Ok(serde_json::from_str(&row?)?)).collect()
    }
    pub fn favorite(&mut self, id: &str, value: bool) -> Result<()> {
        let mut entry = self.get(id)?;
        entry.favorite = value;
        self.db.execute(
            "UPDATE entries SET favorite=?, data=? WHERE id=?",
            params![value, serde_json::to_string(&entry)?, id],
        )?;
        Ok(())
    }
    pub fn delete(&mut self, ids: &[String]) -> Result<()> {
        let tx = self.db.transaction()?;
        for id in ids {
            tx.execute("DELETE FROM entries WHERE id=?", [id])?;
        }
        tx.commit()?;
        self.gc()
    }
    pub fn clear_clipboard(&mut self) -> Result<()> {
        self.db.execute(
            "DELETE FROM entries WHERE source='clipboard' AND favorite=0",
            [],
        )?;
        self.gc()
    }
    pub fn cleanup(&mut self, now: u64) -> Result<()> {
        self.db.execute(
            "DELETE FROM entries WHERE source='clipboard' AND favorite=0 AND stamp<?",
            [now.saturating_sub(self.policy.days as u64 * 86400)],
        )?;
        self.db.execute("DELETE FROM entries WHERE id IN (SELECT id FROM entries WHERE source='clipboard' AND favorite=0 ORDER BY stamp DESC,rowid DESC LIMIT -1 OFFSET ?)", [self.policy.limit])?;
        let mut stmt = self
            .db
            .prepare("SELECT name FROM exports WHERE protected=0 AND stamp<?")?;
        let names = stmt
            .query_map([now.saturating_sub(86400)], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(stmt);
        for name in names {
            let path = self.export_dir(&name)?;
            remove_owned(&path)?;
            self.db
                .execute("DELETE FROM exports WHERE name=?", [name])?;
        }
        self.gc()
    }
    fn gc(&mut self) -> Result<()> {
        let mut stmt = self.db.prepare(
            "SELECT hash FROM assets WHERE hash NOT IN (SELECT asset FROM entry_assets)",
        )?;
        let unused = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(stmt);
        for hash in unused {
            remove_owned(&self.asset(&hash)?)?;
            self.db.execute("DELETE FROM assets WHERE hash=?", [hash])?;
        }
        // Covers a crash between atomic file publication and its database commit.
        for file in fs::read_dir(self.root.join("assets"))? {
            let file = file?;
            let hash = file.file_name().to_string_lossy().into_owned();
            let exists: bool = self.db.query_row(
                "SELECT EXISTS(SELECT 1 FROM assets WHERE hash=?)",
                [&hash],
                |r| r.get(0),
            )?;
            if !exists {
                remove_owned(&file.path())?;
            }
        }
        Ok(())
    }
    fn asset(&self, hash: &str) -> Result<PathBuf> {
        if hash.len() != 64 || !hash.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err("内容库资源标识损坏".into());
        }
        Ok(self.root.join("assets").join(hash))
    }
    fn read_blob(&self, hash: &str) -> Result<Vec<u8>> {
        Ok(fs::read(self.asset(hash)?)?)
    }
    fn export_dir(&self, name: &str) -> Result<PathBuf> {
        if name.is_empty() || !name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-') {
            return Err("导出缓存路径无效".into());
        }
        Ok(self.root.join("exports").join(name))
    }
    fn make_room(&mut self, extra: u64) -> Result<()> {
        while self.used()?.saturating_add(extra) > self.policy.quota {
            let oldest = self.db.query_row("SELECT id FROM entries WHERE source='clipboard' AND favorite=0 AND id<>? ORDER BY stamp,rowid LIMIT 1", [self.active.as_deref().unwrap_or("")], |r| r.get::<_, String>(0));
            match oldest {
                Ok(id) => self.delete(&[id])?,
                Err(rusqlite::Error::QueryReturnedNoRows) => {
                    return Err("内容库空间不足，请清理中转内容或提高空间限额。".into());
                }
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }
    fn blob(&mut self, bytes: &[u8], refs: &mut Vec<String>) -> Result<String> {
        let hash = format!("{:x}", Sha256::digest(bytes));
        let path = self.asset(&hash)?;
        if !path.exists() {
            self.make_room(bytes.len() as u64)?;
            let tmp = self.root.join("tmp").join(&hash);
            let mut file = fs::File::create(&tmp)?;
            file.write_all(bytes)?;
            file.sync_all()?;
            drop(file);
            fs::rename(tmp, &path)?;
            self.db.execute(
                "INSERT OR IGNORE INTO assets(hash,bytes) VALUES(?,?)",
                params![hash, bytes.len() as u64],
            )?;
        }
        self.pin(&hash)?;
        refs.push(hash.clone());
        Ok(hash)
    }
    fn pin(&self, hash: &str) -> Result<()> {
        if let Some(id) = &self.active {
            self.db.execute(
                "INSERT OR IGNORE INTO entry_assets VALUES(?,?)",
                params![id, hash],
            )?;
        }
        Ok(())
    }
    fn copy_file(
        &mut self,
        source: &Path,
        refs: &mut Vec<String>,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(&str, u64),
    ) -> Result<FileItem> {
        let metadata = fs::symlink_metadata(source)?;
        if metadata.file_type().is_symlink() || is_reparse(&metadata) {
            return Err("不递归收纳符号链接或重解析点，请拖入实际文件。".into());
        }
        let name = source
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let temporary = self.root.join("tmp").join(format!("file-{}", refs.len()));
        if temporary.exists() {
            remove_owned(&temporary)?;
        }
        let mut hasher = Sha256::new();
        hasher.update(if metadata.is_dir() {
            b"directory".as_slice()
        } else {
            b"file".as_slice()
        });
        let mut bytes = 0;
        // Reserve known size, then check again while streaming unknown/changing files.
        self.make_room(tree_bytes(source)?)?;
        let budget = self.policy.quota.saturating_sub(self.used()?);
        copy_tree(
            source,
            &temporary,
            source,
            &mut hasher,
            &mut bytes,
            budget,
            cancel,
            progress,
        )?;
        let hash = format!("{:x}", hasher.finalize());
        let target = self.asset(&hash)?;
        if target.exists() {
            remove_owned(&temporary)?;
        } else {
            fs::rename(&temporary, &target)?;
            self.db.execute(
                "INSERT INTO assets(hash,bytes) VALUES(?,?)",
                params![hash, bytes],
            )?;
        }
        self.pin(&hash)?;
        refs.push(hash.clone());
        Ok(FileItem {
            name,
            path: PathBuf::from(hash),
            managed: true,
            directory: metadata.is_dir(),
            bytes,
        })
    }
    pub fn import(
        &mut self,
        id: &str,
        source: Source,
        payloads: Vec<Payload>,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(&str, u64),
    ) -> Result<Entry> {
        let now = crate::safety::timestamp();
        self.cleanup(now)?;
        let mut entry = Entry {
            id: id.into(),
            source,
            timestamp: now,
            favorite: false,
            title: "正在保存…".into(),
            preview: String::new(),
            kind: "files".into(),
            state: State::Saving,
            items: vec![],
            errors: vec![],
            origin: payloads.iter().find_map(|payload| {
                if let Payload::Remote { url, .. } = payload {
                    Some(url.clone())
                } else {
                    None
                }
            }),
        };
        self.db.execute(
            "INSERT INTO entries VALUES(?,?,?,0,'','',?)",
            params![id, source_key(source), now, serde_json::to_string(&entry)?],
        )?;
        self.active = Some(id.into());
        self.warnings.clear();
        let mut refs = Vec::new();
        let mut search = String::new();
        for payload in payloads {
            if cancel.load(Ordering::Acquire) {
                entry.errors.push("保存已取消。".into());
                break;
            }
            let result =
                self.import_payload(source, payload, &mut refs, cancel, progress, &mut search);
            match result {
                Ok(item) => entry.items.push(item),
                Err(e) => entry.errors.push(e.to_string()),
            }
        }
        entry.errors.append(&mut self.warnings);
        entry.state = if entry.items.is_empty() {
            State::Failed
        } else if entry.errors.is_empty() {
            State::Ready
        } else {
            State::Partial
        };
        if let Some(item) = entry.items.first() {
            let (kind, title, preview) = self.describe(item)?;
            entry.kind = if entry.items.len() > 1 {
                "mixed".into()
            } else {
                kind
            };
            entry.title = title;
            entry.preview = preview;
        } else {
            entry.title = "未能保存内容".into();
            entry.preview = entry.errors.join("\n");
        }
        let fingerprint = format!("{:x}", Sha256::digest(serde_json::to_vec(&entry.items)?));
        if source == Source::Clipboard && !entry.items.is_empty()
            && let Ok(existing) = self.db.query_row("SELECT id FROM entries WHERE source='clipboard' AND fingerprint=? AND id<>? LIMIT 1", params![fingerprint,id], |r| r.get::<_,String>(0)) {
            let old = self.get(&existing)?; entry.id = existing; entry.favorite = old.favorite;
            self.db.execute("DELETE FROM entries WHERE id=?", [id])?;
        }
        let tx = self.db.transaction()?;
        tx.execute(
            "INSERT OR REPLACE INTO entries VALUES(?,?,?,?,?,?,?)",
            params![
                entry.id,
                source_key(source),
                now,
                entry.favorite,
                fingerprint,
                search,
                serde_json::to_string(&entry)?
            ],
        )?;
        for hash in item_assets(&entry.items) {
            tx.execute(
                "INSERT OR IGNORE INTO entry_assets VALUES(?,?)",
                params![entry.id, hash],
            )?;
        }
        tx.commit()?;
        self.active = None;
        self.gc()?;
        self.clear_folder("tmp")?;
        self.cleanup(now)?;
        Ok(entry)
    }
    fn import_payload(
        &mut self,
        source: Source,
        payload: Payload,
        refs: &mut Vec<String>,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(&str, u64),
        search: &mut String,
    ) -> Result<Item> {
        match payload {
            Payload::Error(error) => Err(error.into()),
            Payload::Text { plain, html, rtf } => {
                search.push_str(&plain);
                search.push('\n');
                let link = is_url(&plain);
                let asset = self.blob(plain.as_bytes(), refs)?;
                let html = html.map(|data| self.blob(&data, refs)).transpose()?;
                let rtf = rtf.map(|data| self.blob(&data, refs)).transpose()?;
                Ok(Item::Text {
                    asset,
                    html,
                    rtf,
                    link,
                })
            }
            Payload::Image(png) => {
                let reader =
                    image::ImageReader::new(io::Cursor::new(&png)).with_guessed_format()?;
                let (width, height) = reader.into_dimensions()?;
                if width as u64 * height as u64 > 64 * 1024 * 1024 {
                    return Err("图片超过 6400 万像素，无法生成安全预览。".into());
                }
                let asset = self.blob(&png, refs)?;
                search.push_str("图片 image screenshot");
                Ok(Item::Image {
                    asset,
                    width,
                    height,
                })
            }
            Payload::Files(paths) => {
                self.import_paths(source, paths, refs, cancel, progress, search)
            }
            Payload::TemporaryFiles {
                paths,
                directory,
                errors,
            } => {
                self.warnings.extend(errors);
                // Only native-owned incoming folders may be removed.
                let directory = directory.canonicalize()?;
                if !self.incoming.iter().any(|parent| {
                    parent
                        .canonicalize()
                        .is_ok_and(|parent| directory.starts_with(&parent) && directory != parent)
                }) {
                    return Err("临时文件不在接收目录内".into());
                }
                if paths
                    .iter()
                    .any(|p| p.canonicalize().is_ok_and(|p| !p.starts_with(&directory)))
                {
                    return Err("临时附件路径超出接收目录".into());
                }
                let result =
                    self.import_paths(Source::Transfer, paths, refs, cancel, progress, search);
                remove_owned(&directory)?;
                result
            }
            Payload::Remote { url, name } => {
                if !self.policy.download {
                    return self.import_payload(
                        source,
                        Payload::text(url),
                        refs,
                        cancel,
                        progress,
                        search,
                    );
                }
                let result = self.download(&url, name, cancel, progress);
                match result {
                    Ok(path) => {
                        let result = self.import_paths(
                            Source::Transfer,
                            vec![path.clone()],
                            refs,
                            cancel,
                            progress,
                            search,
                        );
                        remove_owned(path.parent().unwrap())?;
                        result
                    }
                    Err(error) => {
                        // Preserve the source URL even when the transfer cannot complete.
                        let asset = self.blob(url.as_bytes(), refs)?;
                        search.push_str(&url);
                        progress(&format!("下载失败：{error}；已保留来源链接"), 0);
                        self.warnings
                            .push(format!("下载失败：{error}；已保留来源链接。"));
                        Ok(Item::Text {
                            asset,
                            html: None,
                            rtf: None,
                            link: true,
                        })
                    }
                }
            }
        }
    }
    fn import_paths(
        &mut self,
        source: Source,
        paths: Vec<PathBuf>,
        refs: &mut Vec<String>,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(&str, u64),
        search: &mut String,
    ) -> Result<Item> {
        let mut files = Vec::new();
        let mut errors = Vec::new();
        for path in paths {
            search.push_str(&path.to_string_lossy());
            search.push('\n');
            let result = (|| -> Result<FileItem> {
                if source == Source::Transfer {
                    self.copy_file(&path, refs, cancel, progress)
                } else {
                    let meta = fs::metadata(&path)?;
                    Ok(FileItem {
                        name: path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned(),
                        directory: meta.is_dir(),
                        bytes: meta.len(),
                        path,
                        managed: false,
                    })
                }
            })();
            match result {
                Ok(file) => files.push(file),
                Err(e) => errors.push(e.to_string()),
            }
        }
        if files.is_empty() {
            return Err(errors.join("\n").into());
        }
        if !errors.is_empty() {
            progress(&format!("部分项目未保存：{}", errors.join("；")), 0);
        }
        self.warnings.extend(errors);
        Ok(Item::Files(files))
    }
    fn describe(&self, item: &Item) -> Result<(String, String, String)> {
        Ok(match item {
            Item::Text { asset, link, .. } => {
                let text = fs::read_to_string(self.asset(asset)?)?;
                let preview = text.chars().take(240).collect::<String>();
                (
                    if *link { "link" } else { "text" }.into(),
                    text.lines()
                        .next()
                        .unwrap_or("空文本")
                        .chars()
                        .take(48)
                        .collect(),
                    preview,
                )
            }
            Item::Image { width, height, .. } => {
                ("image".into(), "图片".into(), format!("{width} × {height}"))
            }
            Item::Files(files) => (
                "files".into(),
                if files.len() == 1 {
                    files[0].name.clone()
                } else {
                    format!("{} 个文件", files.len())
                },
                files
                    .iter()
                    .map(|f| f.name.as_str())
                    .collect::<Vec<_>>()
                    .join(" · "),
            ),
        })
    }
    pub fn payloads(&self, id: &str) -> Result<Vec<Payload>> {
        self.get(id)?
            .items
            .iter()
            .map(|item| {
                Ok(match item {
                    Item::Text {
                        asset, html, rtf, ..
                    } => Payload::Text {
                        plain: fs::read_to_string(self.asset(asset)?)?,
                        html: html.as_ref().map(|a| self.read_blob(a)).transpose()?,
                        rtf: rtf.as_ref().map(|a| self.read_blob(a)).transpose()?,
                    },
                    Item::Image { asset, .. } => Payload::Image(fs::read(self.asset(asset)?)?),
                    Item::Files(files) => Payload::Files(
                        files
                            .iter()
                            .map(|f| {
                                if f.managed {
                                    self.asset(f.path.to_str().unwrap_or_default())
                                } else {
                                    Ok(f.path.clone())
                                }
                            })
                            .collect::<Result<_>>()?,
                    ),
                })
            })
            .collect()
    }
    pub fn preview(&self, id: &str) -> Result<Preview> {
        let mut texts = Vec::new();
        let mut preview = None;
        for item in self.get(id)?.items {
            match item {
                Item::Text { asset, .. } => texts.push(fs::read_to_string(self.asset(&asset)?)?),
                Item::Image {
                    asset,
                    width,
                    height,
                } if preview.is_none() => {
                    let image = image::ImageReader::open(self.asset(&asset)?)?
                        .with_guessed_format()?
                        .decode()?
                        .thumbnail(width.min(900), height.min(900))
                        .into_rgba8();
                    let (w, h) = image.dimensions();
                    preview = Some((image.into_raw(), w, h));
                }
                Item::Files(files) => {
                    for file in files {
                        if preview.is_none() && !file.directory {
                            let path = if file.managed {
                                self.asset(file.path.to_str().unwrap_or_default())?
                            } else {
                                file.path.clone()
                            };
                            if let Ok(reader) = image::ImageReader::open(&path)
                                .and_then(|r| r.with_guessed_format())
                                && let Ok((w, h)) = reader.into_dimensions()
                                && w as u64 * h as u64 <= 64 * 1024 * 1024
                                && let Ok(image) = image::ImageReader::open(&path)?
                                    .with_guessed_format()?
                                    .decode()
                            {
                                let image = image.thumbnail(w.min(900), h.min(900)).into_rgba8();
                                let (w, h) = image.dimensions();
                                preview = Some((image.into_raw(), w, h));
                            }
                        }
                        texts.push(format!(
                            "{} · {}{}",
                            file.name,
                            bytes_label(file.bytes),
                            if !file.managed && !file.path.exists() {
                                " · 原文件已不存在"
                            } else {
                                ""
                            }
                        ));
                        if !file.managed {
                            texts.push(file.path.display().to_string());
                        }
                        if file.directory {
                            let path = if file.managed {
                                self.asset(file.path.to_str().unwrap_or_default())?
                            } else {
                                file.path.clone()
                            };
                            let mut listing = Vec::new();
                            if let Err(error) = folder_listing(&path, &path, &mut listing, 0) {
                                listing.push(format!("目录预览不可用：{error}"));
                            }
                            if listing.len() >= 500 {
                                listing.push("…仅显示前 500 项，导出保留完整目录。".into());
                            }
                            texts.push(listing.join("\n"));
                        }
                    }
                }
                _ => {}
            }
        }
        Ok((Some(texts.join("\n\n")), preview))
    }
    pub fn export(
        &mut self,
        ids: &[String],
        cancel: &AtomicBool,
    ) -> Result<(Vec<PathBuf>, Vec<Payload>)> {
        self.export_with_progress(ids, cancel, &mut |_, _| {})
    }
    pub fn export_with_progress(
        &mut self,
        ids: &[String],
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(&str, u64),
    ) -> Result<(Vec<PathBuf>, Vec<Payload>)> {
        let picks = ids
            .iter()
            .map(|id| {
                self.get(id)
                    .map(|entry| crate::delivery::entry_picks(&entry))
            })
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        self.export_picks(&picks, cancel, progress)
    }
    pub fn export_picks(
        &mut self,
        picks: &[crate::delivery::Pick],
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(&str, u64),
    ) -> Result<(Vec<PathBuf>, Vec<Payload>)> {
        progress("正在准备拖出", 0);
        let mut digest = Sha256::new();
        digest.update(serde_json::to_vec(picks)?);
        let name = format!("{:x}", digest.finalize());
        let directory = self.export_dir(&name)?;
        fs::create_dir_all(&directory)?;
        let mut paths = Vec::new();
        let mut payloads = Vec::new();
        let mut serial = 0;
        for pick in picks {
            if cancel.load(Ordering::Acquire) {
                return Err("导出准备已取消".into());
            }
            let entry = self.get(&pick.entry)?;
            let mut item = entry
                .items
                .get(pick.item)
                .cloned()
                .ok_or("所选内容已失效")?;
            if let Item::Files(files) = &mut item {
                let file = files
                    .get(pick.file.ok_or("请选择具体文件")?)
                    .cloned()
                    .ok_or("所选文件已失效")?;
                *files = vec![file];
            } else if pick.file.is_some() {
                return Err("文件选择类型不匹配".into());
            }
            for item in [item] {
                match item {
                    Item::Text {
                        asset, html, rtf, ..
                    } => {
                        let plain = fs::read_to_string(self.asset(&asset)?)?;
                        let path = export_target(
                            &directory,
                            &format!("文本-{}.txt", paths.len() + 1),
                            &paths,
                            &mut serial,
                        );
                        fs::write(&path, plain.as_bytes())?;
                        paths.push(path);
                        payloads.push(Payload::Text {
                            plain,
                            html: html.map(|a| self.read_blob(&a)).transpose()?,
                            rtf: rtf.map(|a| self.read_blob(&a)).transpose()?,
                        });
                    }
                    Item::Image { asset, .. } => {
                        let path = export_target(
                            &directory,
                            &format!("图片-{}.png", paths.len() + 1),
                            &paths,
                            &mut serial,
                        );
                        fs::copy(self.asset(&asset)?, &path)?;
                        paths.push(path);
                        payloads.push(Payload::Image(fs::read(self.asset(&asset)?)?));
                    }
                    Item::Files(files) => {
                        for file in files {
                            let original = if file.managed {
                                self.asset(file.path.to_str().unwrap_or_default())?
                            } else {
                                file.path.clone()
                            };
                            if !original.exists() {
                                return Err(format!("原文件已不存在：{}", file.name).into());
                            }
                            let safe_name = safe_filename(&file.name);
                            let target = export_target(&directory, &safe_name, &paths, &mut serial);
                            if !target.exists() {
                                let mut hash = Sha256::new();
                                let mut bytes = 0;
                                let temporary = directory.join(format!(".prepare-{}", paths.len()));
                                let result = copy_tree(
                                    &original,
                                    &temporary,
                                    &original,
                                    &mut hash,
                                    &mut bytes,
                                    u64::MAX,
                                    cancel,
                                    progress,
                                );
                                if let Err(e) = result {
                                    remove_owned(&temporary)?;
                                    return Err(e);
                                }
                                fs::rename(temporary, &target)?;
                            }
                            payloads.push(Payload::Files(vec![target.clone()]));
                            paths.push(target);
                        }
                    }
                }
            }
        }
        self.db.execute(
            "INSERT OR REPLACE INTO exports VALUES(?,?,0)",
            params![name, crate::safety::timestamp()],
        )?;
        if paths.is_empty() {
            return Err("记录没有可导出的内容。".into());
        }
        // Protected exports survive cleanup while a native clipboard/data object
        // can still use their paths. Native bridge releases them on ownership loss.
        Ok((paths, payloads))
    }
    pub fn release_exports(&self) -> Result<()> {
        self.db.execute("UPDATE exports SET protected=0", [])?;
        Ok(())
    }
    pub fn protect_exports(&self, files: &[PathBuf]) -> Result<()> {
        for file in files {
            if let Some(name) = file
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
            {
                self.export_dir(name)?;
                self.db
                    .execute("UPDATE exports SET protected=1 WHERE name=?", [name])?;
            }
        }
        Ok(())
    }
    pub fn save_exports(
        &self,
        files: &[PathBuf],
        directory: &Path,
        cancel: &AtomicBool,
    ) -> (usize, Vec<String>) {
        self.save_exports_with_progress(files, directory, cancel, &mut |_, _| {})
    }
    pub fn save_exports_with_progress(
        &self,
        files: &[PathBuf],
        directory: &Path,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(&str, u64),
    ) -> (usize, Vec<String>) {
        progress("正在另存到文件夹", 0);
        let mut count = 0;
        let mut errors = Vec::new();
        for file in files {
            if cancel.load(Ordering::Acquire) {
                errors.push("另存已取消".into());
                break;
            }
            let name = file.file_name().unwrap_or_default();
            let target = directory.join(name);
            if target.exists() {
                errors.push(format!("目标已存在：{}", target.display()));
                continue;
            }
            let temp_root = directory.join(format!(
                ".mochi-saving-{}-{}",
                std::process::id(),
                SAVE_SERIAL.fetch_add(1, Ordering::Relaxed)
            ));
            if let Err(e) = fs::create_dir(&temp_root) {
                errors.push(e.to_string());
                continue;
            }
            let temporary = temp_root.join(name);
            let mut hash = Sha256::new();
            let mut bytes = 0;
            let result = copy_tree(
                file,
                &temporary,
                file,
                &mut hash,
                &mut bytes,
                u64::MAX,
                cancel,
                progress,
            );
            match result {
                Ok(()) if !target.exists() => match publish_no_replace(&temporary, &target) {
                    Ok(()) => count += 1,
                    Err(e) => {
                        errors.push(e.to_string());
                        let _ = remove_owned(&temporary);
                    }
                },
                Ok(()) => {
                    errors.push(format!("目标已存在：{}", target.display()));
                    let _ = remove_owned(&temporary);
                }
                Err(e) => {
                    errors.push(e.to_string());
                    let _ = remove_owned(&temporary);
                }
            }
            let _ = remove_owned(&temp_root);
        }
        (count, errors)
    }
    fn download(
        &mut self,
        url: &str,
        name: Option<String>,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(&str, u64),
    ) -> Result<PathBuf> {
        let parsed = reqwest::Url::parse(url)?;
        if !matches!(parsed.scheme(), "https" | "http") {
            return Err("该地址不能直接下载，请从浏览器保存后拖入实际文件。".into());
        }
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let result = runtime.block_on(async {
            let client = reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .read_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(300))
                .redirect(reqwest::redirect::Policy::limited(5))
                .build()?;
            let mut response = http_wait(client.get(parsed).send(), cancel)
                .await?
                .error_for_status()?;
            let name = name
                .or_else(|| {
                    response
                        .url()
                        .path_segments()
                        .and_then(|mut p| p.next_back())
                        .filter(|p| !p.is_empty())
                        .map(str::to_owned)
                })
                .unwrap_or_else(|| "下载文件".into());
            if response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v.starts_with("text/html"))
            {
                return Err("服务器返回网页或登录页面，未当作文件保存。".into());
            }
            if let Some(size) = response.content_length() {
                self.make_room(size)?;
            }
            let directory = self.root.join("tmp").join("download");
            fs::create_dir_all(&directory)?;
            let path = directory.join(safe_filename(&name));
            let mut file = fs::File::create(&path)?;
            let mut bytes = 0;
            loop {
                if cancel.load(Ordering::Acquire) {
                    return Err("下载已取消".into());
                }
                let Some(chunk) = http_wait(response.chunk(), cancel).await? else {
                    break;
                };
                bytes += chunk.len() as u64;
                self.make_room(bytes)?;
                file.write_all(&chunk)?;
                progress("正在下载", bytes);
            }
            file.sync_all()?;
            Ok(path)
        });
        runtime.shutdown_timeout(Duration::from_millis(100));
        result
    }
}

async fn http_wait<T>(
    future: impl std::future::Future<Output = reqwest::Result<T>>,
    cancel: &AtomicBool,
) -> Result<T> {
    tokio::pin!(future);
    loop {
        if cancel.load(Ordering::Acquire) {
            return Err("下载已取消".into());
        }
        tokio::select! {result=&mut future=>return Ok(result?),_=tokio::time::sleep(Duration::from_millis(100))=>{}}
    }
}
static SAVE_SERIAL: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
fn folder_listing(
    path: &Path,
    root: &Path,
    lines: &mut Vec<String>,
    depth: usize,
) -> io::Result<()> {
    if depth > 64 || lines.len() >= 500 {
        return Ok(());
    }
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || is_reparse(&metadata) {
        return Ok(());
    }
    let mut children = fs::read_dir(path)?.collect::<io::Result<Vec<_>>>()?;
    children.sort_by_key(|child| child.file_name());
    for child in children {
        if lines.len() >= 500 {
            break;
        }
        let path = child.path();
        let metadata = fs::symlink_metadata(&path)?;
        let relative = path.strip_prefix(root).map_err(io::Error::other)?;
        lines.push(format!(
            "{}{} · {}",
            relative.display(),
            if metadata.is_dir() { "/" } else { "" },
            bytes_label(metadata.len())
        ));
        if metadata.is_dir() && !metadata.file_type().is_symlink() && !is_reparse(&metadata) {
            folder_listing(&path, root, lines, depth + 1)?;
        }
    }
    Ok(())
}
fn publish_no_replace(source: &Path, target: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{MOVEFILE_WRITE_THROUGH, MoveFileExW};
        let from = source
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let to = target
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), MOVEFILE_WRITE_THROUGH) } == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::ffi::OsStrExt;
        let from = std::ffi::CString::new(source.as_os_str().as_bytes())?;
        let to = std::ffi::CString::new(target.as_os_str().as_bytes())?;
        if unsafe {
            libc::renameatx_np(
                libc::AT_FDCWD,
                from.as_ptr(),
                libc::AT_FDCWD,
                to.as_ptr(),
                libc::RENAME_EXCL,
            )
        } == 0
        {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        if source.is_file() {
            fs::hard_link(source, target)?;
            fs::remove_file(source)
        } else {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "此平台不支持目录安全导出",
            ))
        }
    }
}
fn source_key(source: Source) -> &'static str {
    if source == Source::Clipboard {
        "clipboard"
    } else {
        "transfer"
    }
}
fn item_assets(items: &[Item]) -> Vec<String> {
    let mut hashes = Vec::new();
    for item in items {
        match item {
            Item::Text {
                asset, html, rtf, ..
            } => {
                hashes.push(asset.clone());
                hashes.extend(html.iter().cloned());
                hashes.extend(rtf.iter().cloned());
            }
            Item::Image { asset, .. } => hashes.push(asset.clone()),
            Item::Files(files) => hashes.extend(
                files
                    .iter()
                    .filter(|f| f.managed)
                    .map(|f| f.path.to_string_lossy().into_owned()),
            ),
        }
    }
    hashes
}
fn is_url(text: &str) -> bool {
    !text.trim().contains(char::is_whitespace) && reqwest::Url::parse(text.trim()).is_ok()
}
fn safe_filename(name: &str) -> String {
    let name = name
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect::<String>();
    let name = name.trim_matches([' ', '.']);
    if name.is_empty() {
        "未命名文件".into()
    } else {
        name.chars().take(180).collect()
    }
}
fn remove_owned(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() && !is_reparse(&meta) => {
            fs::remove_dir_all(path)
        }
        Ok(_) => fs::remove_file(path),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}
#[cfg(windows)]
fn is_reparse(meta: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    meta.file_attributes() & 0x400 != 0
}
#[cfg(not(windows))]
fn is_reparse(_meta: &fs::Metadata) -> bool {
    false
}
fn tree_bytes(path: &Path) -> io::Result<u64> {
    let meta = fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() || is_reparse(&meta) {
        return Ok(0);
    }
    if !meta.is_dir() {
        return Ok(meta.len());
    }
    fs::read_dir(path)?.try_fold(0u64, |sum, e| {
        Ok(sum.saturating_add(tree_bytes(&e?.path())?))
    })
}
#[allow(clippy::too_many_arguments)]
fn copy_tree(
    source: &Path,
    target: &Path,
    root: &Path,
    hash: &mut Sha256,
    bytes: &mut u64,
    budget: u64,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(&str, u64),
) -> Result<()> {
    if cancel.load(Ordering::Acquire) {
        return Err("保存已取消".into());
    }
    let meta = fs::symlink_metadata(source)?;
    if meta.file_type().is_symlink() || is_reparse(&meta) {
        return Err(format!("不跟随符号链接：{}", source.display()).into());
    }
    let relative = source.strip_prefix(root)?;
    if relative
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err("文件相对路径无效".into());
    }
    hash.update(relative.to_string_lossy().as_bytes());
    hash.update([0]);
    hash.update([u8::from(meta.is_dir())]);
    if meta.is_dir() {
        fs::create_dir_all(target)?;
        let mut children = fs::read_dir(source)?.collect::<io::Result<Vec<_>>>()?;
        children.sort_by_key(|c| c.file_name());
        for child in children {
            copy_tree(
                &child.path(),
                &target.join(child.file_name()),
                root,
                hash,
                bytes,
                budget,
                cancel,
                progress,
            )?;
        }
    } else {
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut input = fs::File::open(source)?;
        let mut output = fs::File::create(target)?;
        let mut buffer = vec![0; 64 * 1024];
        loop {
            if cancel.load(Ordering::Acquire) {
                return Err("保存已取消".into());
            }
            let count = input.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            *bytes = bytes.saturating_add(count as u64);
            if *bytes > budget {
                return Err("文件超过内容库剩余空间".into());
            }
            hash.update(&buffer[..count]);
            output.write_all(&buffer[..count])?;
            progress(
                &format!(
                    "正在保存 {}",
                    source.file_name().unwrap_or_default().to_string_lossy()
                ),
                *bytes,
            );
        }
        output.sync_all()?;
    }
    Ok(())
}
