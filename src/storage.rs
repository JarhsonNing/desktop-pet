//! User-selected storage roots and non-destructive, staged directory copies.
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub version: u32,
    pub content: Option<PathBuf>,
    pub explorer: Option<PathBuf>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            content: None,
            explorer: None,
        }
    }
}
impl Settings {
    pub fn load(data: &Path) -> io::Result<Self> {
        match fs::read(data.join("storage.json")) {
            Ok(bytes) => {
                let settings: Self = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
                if settings.version != 1 {
                    return Err(io::Error::other("存储配置版本较新，未覆盖原文件"));
                }
                for path in [&settings.content, &settings.explorer]
                    .into_iter()
                    .flatten()
                {
                    if !path.is_absolute() {
                        return Err(io::Error::other("存储配置必须使用绝对路径"));
                    }
                }
                Ok(settings)
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error),
        }
    }
    pub fn content(&self, data: &Path) -> PathBuf {
        self.content.clone().unwrap_or_else(|| data.join("content"))
    }
    pub fn explorer(&self, data: &Path) -> PathBuf {
        self.explorer
            .clone()
            .unwrap_or_else(|| data.join("Mochi 文件箱"))
    }
    pub fn save(&self, data: &Path) -> io::Result<()> {
        crate::config::atomic_json(&data.join("storage.json"), self)
    }
    pub fn validate(&self, data: &Path) -> io::Result<()> {
        let content = normalized(&self.content(data))?;
        let explorer = normalized(&self.explorer(data))?;
        let data = normalized(data)?;
        if overlaps(&content, &explorer)
            || data.starts_with(&content)
            || data.starts_with(&explorer)
        {
            return Err(io::Error::other(
                "内容库、文件箱不能互相包含，也不能覆盖应用数据目录",
            ));
        }
        Ok(())
    }
}
pub fn normalized(path: &Path) -> io::Result<PathBuf> {
    let path = std::path::absolute(path)?;
    if path.exists() {
        return path.canonicalize();
    }
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("不能选择磁盘根目录"))?;
    Ok(normalized(parent)?.join(
        path.file_name()
            .ok_or_else(|| io::Error::other("目录名无效"))?,
    ))
}
pub fn display(path: &Path) -> String {
    let value = path.to_string_lossy();
    if let Some(unc) = value.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc}")
    } else {
        value.strip_prefix(r"\\?\").unwrap_or(&value).into()
    }
}
pub fn overlaps(a: &Path, b: &Path) -> bool {
    a.starts_with(b) || b.starts_with(a)
}
pub fn plain(path: &Path) -> io::Result<fs::Metadata> {
    let metadata = fs::symlink_metadata(path)?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(io::Error::other("迁移不跟随链接或重解析点"));
        }
    }
    if metadata.file_type().is_symlink() {
        return Err(io::Error::other("迁移不跟随符号链接"));
    }
    Ok(metadata)
}
static SERIAL: AtomicU64 = AtomicU64::new(0);
pub struct Staging {
    pub path: PathBuf,
    target: PathBuf,
    parent: PathBuf,
}
impl Staging {
    pub fn new(source: &Path, target: &Path) -> io::Result<Self> {
        if !target.is_absolute() {
            return Err(io::Error::other("请选择绝对路径"));
        }
        if target.symlink_metadata().is_ok() {
            plain(target)?;
        }
        let source = normalized(source)?;
        let target = normalized(target)?;
        if overlaps(&source, &target) {
            return Err(io::Error::other("新旧目录不能相同或互相包含"));
        }
        if target.exists() && (!target.is_dir() || fs::read_dir(&target)?.next().is_some()) {
            return Err(io::Error::other("请选择一个空文件夹，已有内容不会被覆盖"));
        }
        let parent = target
            .parent()
            .ok_or_else(|| io::Error::other("不能选择磁盘根目录"))?
            .to_owned();
        fs::create_dir_all(&parent)?;
        let path = parent.join(format!(
            ".mochi-storage-{}-{}-{}",
            std::process::id(),
            crate::safety::timestamp(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path)?;
        Ok(Self {
            path,
            target,
            parent,
        })
    }
    pub fn publish(&mut self) -> io::Result<PathBuf> {
        // remove_dir only removes an empty directory. A newly added user file
        // makes the commit fail instead of being overwritten or removed.
        if self.target.exists() {
            fs::remove_dir(&self.target)?;
        }
        fs::rename(&self.path, &self.target)?;
        Ok(self.target.clone())
    }
}
impl Drop for Staging {
    fn drop(&mut self) {
        // Only clean our own unpublished temporary tree, never either user's root.
        if self.path.parent() == Some(self.parent.as_path())
            && self
                .path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with(".mochi-storage-"))
            && self
                .path
                .canonicalize()
                .is_ok_and(|path| path.parent() == Some(self.parent.as_path()))
            && validate_tree(&self.path, 0).is_ok()
        {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
fn validate_tree(path: &Path, depth: usize) -> io::Result<()> {
    if depth > 64 {
        return Err(io::Error::other("目录层级过深"));
    }
    if plain(path)?.is_dir() {
        for entry in fs::read_dir(path)? {
            validate_tree(&entry?.path(), depth + 1)?;
        }
    }
    Ok(())
}
pub fn copy_tree(
    source: &Path,
    target: &Path,
    cancel: &AtomicBool,
    bytes: &mut u64,
    progress: &mut dyn FnMut(&str, u64),
) -> io::Result<()> {
    fn copy(
        source: &Path,
        target: &Path,
        cancel: &AtomicBool,
        bytes: &mut u64,
        progress: &mut dyn FnMut(&str, u64),
        depth: usize,
    ) -> io::Result<()> {
        if cancel.load(Ordering::Acquire) {
            return Err(io::Error::other("目录切换已取消，继续使用原目录"));
        }
        if depth > 64 {
            return Err(io::Error::other("目录层级过深"));
        }
        if plain(source)?.is_dir() {
            fs::create_dir(target)?;
            for entry in fs::read_dir(source)? {
                let entry = entry?;
                copy(
                    &entry.path(),
                    &target.join(entry.file_name()),
                    cancel,
                    bytes,
                    progress,
                    depth + 1,
                )?;
            }
        } else {
            let mut input = fs::File::open(source)?;
            let mut output = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(target)?;
            let mut buffer = [0; 65536];
            loop {
                if cancel.load(Ordering::Acquire) {
                    return Err(io::Error::other("目录切换已取消，继续使用原目录"));
                }
                let count = input.read(&mut buffer)?;
                if count == 0 {
                    break;
                }
                output.write_all(&buffer[..count])?;
                *bytes = bytes.saturating_add(count as u64);
                progress("正在复制存储目录", *bytes);
            }
            output.sync_all()?;
        }
        Ok(())
    }
    copy(source, target, cancel, bytes, progress, 0)
}
