//! Versioned, disposable Explorer projection. Content records remain authoritative.
//! This module never imports Explorer edits or installs a database/provider DLL.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

pub enum ViewSource {
    File { name: String, path: PathBuf },
    Bytes { name: String, bytes: Vec<u8> },
}
#[derive(Default, Serialize, Deserialize)]
struct Published {
    folder: String,
    fingerprint: String,
    digest: String,
}
#[derive(Serialize, Deserialize)]
struct Manifest {
    version: u32,
    records: BTreeMap<String, Published>,
}
impl Default for Manifest {
    fn default() -> Self {
        Self {
            version: 1,
            records: BTreeMap::new(),
        }
    }
}

pub fn directory(data: &Path) -> PathBuf {
    data.join("Mochi 文件箱")
}
fn plain(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(io::Error::other("展示目录不允许链接或重解析点"));
        }
    }
    if metadata.file_type().is_symlink() {
        return Err(io::Error::other("展示目录不允许符号链接"));
    }
    Ok(())
}
fn fingerprint(path: &Path, hash: &mut Sha256, depth: usize) -> io::Result<()> {
    if depth > 64 {
        return Err(io::Error::other("目录层级过深"));
    }
    plain(path)?;
    if path.is_dir() {
        let mut children = fs::read_dir(path)?.collect::<io::Result<Vec<_>>>()?;
        children.sort_by_key(|entry| entry.file_name());
        for child in children {
            hash.update(child.file_name().to_string_lossy().as_bytes());
            fingerprint(&child.path(), hash, depth + 1)?;
        }
    } else {
        let mut input = fs::File::open(path)?;
        let mut buffer = [0; 65536];
        loop {
            let count = input.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            hash.update(&buffer[..count]);
        }
    }
    Ok(())
}
fn digest(path: &Path) -> io::Result<String> {
    let mut hash = Sha256::new();
    fingerprint(path, &mut hash, 0)?;
    Ok(format!("{:x}", hash.finalize()))
}
fn safe_name(name: &str) -> String {
    let name: String = name
        .chars()
        .filter(|c| !c.is_control() && !"<>:\"/\\|?*".contains(*c))
        .collect();
    let name = name.trim().trim_end_matches('.');
    if name.is_empty() {
        "内容".into()
    } else {
        if name.chars().count() <= 70 {
            return name.into();
        }
        let extension = Path::new(name)
            .extension()
            .and_then(|value| value.to_str())
            .filter(|value| value.chars().count() <= 16)
            .map(|value| format!(".{value}"))
            .unwrap_or_default();
        format!(
            "{}{}",
            name.chars()
                .take(70 - extension.chars().count())
                .collect::<String>(),
            extension
        )
    }
}
fn copy(
    source: &Path,
    target: &Path,
    remaining: &mut u64,
    cancel: &AtomicBool,
    depth: usize,
) -> io::Result<()> {
    if depth > 64 || cancel.load(Ordering::Acquire) {
        return Err(io::Error::other("展示更新已取消或目录层级过深"));
    }
    plain(source)?;
    if source.is_dir() {
        fs::create_dir(target)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            copy(
                &entry.path(),
                &target.join(entry.file_name()),
                remaining,
                cancel,
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
                return Err(io::Error::other("展示更新已取消"));
            }
            let count = input.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            if *remaining < count as u64 {
                return Err(io::Error::other(
                    "展示副本空间不足，请清理文件箱或提高空间上限",
                ));
            }
            *remaining -= count as u64;
            output.write_all(&buffer[..count])?;
        }
        output.sync_all()?;
    }
    Ok(())
}
pub fn bytes(path: &Path) -> io::Result<u64> {
    if !path.exists() {
        return Ok(0);
    }
    plain(path)?;
    if path.is_file() {
        return Ok(path.metadata()?.len());
    }
    fs::read_dir(path)?.try_fold(0u64, |sum, child| {
        Ok(sum.saturating_add(bytes(&child?.path())?))
    })
}
fn retire(root: &Path, path: &Path, expected: &str) -> io::Result<()> {
    // Every removal is confined to our owned projection, never the content store.
    if !path.starts_with(root)
        || path == root
        || !path
            .parent()
            .ok_or_else(|| io::Error::other("无父目录"))?
            .canonicalize()?
            .starts_with(root.canonicalize()?)
    {
        return Err(io::Error::other("展示清理路径越界"));
    }
    plain(path)?;
    if digest(path).is_ok_and(|value| value == expected) {
        fs::remove_dir_all(path)
    } else {
        let archive = root.join("本地改动保留");
        fs::create_dir_all(&archive)?;
        plain(&archive)?;
        let mut serial = 0u64;
        loop {
            let target = archive.join(format!(
                "{}-{serial}",
                path.file_name().unwrap().to_string_lossy()
            ));
            if !target.exists() {
                return fs::rename(path, target);
            }
            serial += 1;
        }
    }
}

pub fn publish(
    data: &Path,
    snapshot: &crate::content::Snapshot,
    quota: u64,
    force: bool,
    cancel: &AtomicBool,
    sources: impl FnMut(&crate::content::Entry) -> Result<Vec<ViewSource>, Box<dyn std::error::Error>>,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    publish_at(&directory(data), snapshot, quota, force, cancel, sources)
}

pub fn publish_at(
    root: &Path,
    snapshot: &crate::content::Snapshot,
    quota: u64,
    force: bool,
    cancel: &AtomicBool,
    mut sources: impl FnMut(
        &crate::content::Entry,
    ) -> Result<Vec<ViewSource>, Box<dyn std::error::Error>>,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let root = root.to_owned();
    if root.exists() {
        plain(&root)?;
    }
    fs::create_dir_all(&root)?;
    let index = root.join(".mochi-view.json");
    for path in [&index, &index.with_extension("tmp")] {
        if path.symlink_metadata().is_ok() {
            plain(path)?;
        }
    }
    if !index.exists() && fs::read_dir(&root)?.next().is_some() {
        return Err("展示目录已存在且不属于 Mochi，未修改其内容".into());
    }
    let mut manifest: Manifest = if index.exists() {
        serde_json::from_slice(&fs::read(&index)?)?
    } else {
        Manifest::default()
    };
    if manifest.version != 1 {
        return Err("展示目录格式较新，当前版本不会覆盖".into());
    }
    for record in manifest.records.values() {
        let parts = Path::new(&record.folder).components().collect::<Vec<_>>();
        if parts.len() != 2
            || parts
                .iter()
                .any(|part| !matches!(part, std::path::Component::Normal(_)))
            || !matches!(parts[0].as_os_str().to_str(), Some("中转站" | "我的收藏"))
        {
            return Err("展示索引路径无效，未修改目录".into());
        }
    }
    crate::config::atomic_json(&index, &manifest)?;
    for name in ["中转站", "我的收藏"] {
        let folder = root.join(name);
        fs::create_dir_all(&folder)?;
        plain(&folder)?;
    }
    let mut wanted = BTreeMap::new();
    for entry in &snapshot.transfer {
        wanted.insert(format!("transfer:{}", entry.id), ("中转站", entry));
    }
    for entry in snapshot
        .transfer
        .iter()
        .chain(&snapshot.clipboard)
        .filter(|entry| entry.favorite)
    {
        wanted.insert(format!("favorite:{}", entry.id), ("我的收藏", entry));
    }
    for key in manifest.records.keys().cloned().collect::<Vec<_>>() {
        if !wanted.contains_key(&key) {
            let old = &manifest.records[&key];
            let path = root.join(&old.folder);
            if path.exists() {
                retire(&root, &path, &old.digest)?;
            }
            manifest.records.remove(&key);
        }
    }
    for (key, (category, entry)) in wanted {
        if cancel.load(Ordering::Acquire) {
            return Err("展示更新已取消".into());
        }
        let signature = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&(&entry.items, &entry.title))?)
        );
        if !force
            && manifest
                .records
                .get(&key)
                .is_some_and(|old| old.fingerprint == signature && root.join(&old.folder).exists())
        {
            continue;
        }
        let hash = format!("{:x}", Sha256::digest(key.as_bytes()));
        let folder = format!("{category}/{}-{}", safe_name(&entry.title), &hash[..16]);
        let target = root.join(&folder);
        if target.exists()
            && !manifest
                .records
                .get(&key)
                .is_some_and(|old| old.folder == folder)
        {
            return Err("展示目录内存在同名本地内容，未覆盖".into());
        }
        let temporary = root.join(format!(".prepare-{hash}"));
        if temporary.exists() {
            plain(&temporary)?;
            return Err("上次展示准备目录仍存在，请手动检查后清理 .prepare 目录".into());
        }
        fs::create_dir(&temporary)?;
        let mut remaining = quota.saturating_sub(bytes(&root)?);
        let result = (|| -> Result<(), Box<dyn std::error::Error>> {
            let items = sources(entry)?;
            if items.is_empty() {
                fs::write(temporary.join("保存状态.txt"), entry.errors.join("\n"))?;
            }
            for (index, item) in items.into_iter().enumerate() {
                let (name, source, content) = match item {
                    ViewSource::File { name, path } => (name, Some(path), None),
                    ViewSource::Bytes { name, bytes } => (name, None, Some(bytes)),
                };
                let mut output = temporary.join(safe_name(&name));
                let mut serial = index + 1;
                while output.exists() {
                    output = temporary.join(format!("{serial}-{}", safe_name(&name)));
                    serial += 1;
                }
                if let Some(source) = source {
                    copy(&source, &output, &mut remaining, cancel, 0)?;
                } else if let Some(content) = content {
                    if content.len() as u64 > remaining {
                        return Err("展示副本空间不足".into());
                    }
                    remaining -= content.len() as u64;
                    fs::write(output, content)?;
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            let hash = digest(&temporary)?;
            retire(&root, &temporary, &hash)?;
            crate::config::atomic_json(&index, &manifest)?;
            return Err(error);
        }
        let sum = digest(&temporary)?;
        if let Some(old) = manifest.records.get(&key) {
            let path = root.join(&old.folder);
            if path.exists() {
                retire(&root, &path, &old.digest)?;
            }
        }
        fs::rename(&temporary, &target)?;
        manifest.records.insert(
            key,
            Published {
                folder,
                fingerprint: signature,
                digest: sum,
            },
        );
        crate::config::atomic_json(&index, &manifest)?;
    }
    crate::config::atomic_json(&index, &manifest)?;
    Ok(root)
}

#[derive(Serialize, Deserialize)]
struct StorageJournal {
    version: u32,
    old: PathBuf,
    new: PathBuf,
    enabled: bool,
}

pub fn recover_storage(
    data: &Path,
    settings: &crate::storage::Settings,
) -> Result<(), Box<dyn std::error::Error>> {
    let path = data.join("storage-moving.json");
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let journal: StorageJournal = serde_json::from_slice(&bytes)?;
    if journal.version != 1 {
        return Err("存储迁移日志版本较新，未覆盖原文件".into());
    }
    let active = crate::storage::normalized(&settings.explorer(data))?;
    let old = crate::storage::normalized(&journal.old)?;
    let new = crate::storage::normalized(&journal.new)?;
    let inactive = if active == old {
        &new
    } else if active == new {
        &old
    } else {
        return Err("存储迁移日志与当前目录不符，请检查 storage-moving.json".into());
    };
    #[cfg(windows)]
    {
        if journal.enabled && active.is_dir() {
            registration::install(&active)?;
        }
        registration::uninstall(inactive)?;
    }
    #[cfg(not(windows))]
    let _ = inactive;
    fs::remove_file(path)?;
    Ok(())
}

pub fn relocate(
    data: &Path,
    settings: &crate::storage::Settings,
    target: &Path,
    enabled: bool,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(&str, u64),
) -> Result<crate::storage::Settings, Box<dyn std::error::Error>> {
    recover_storage(data, settings)?;
    let old = settings.explorer(data);
    let mut next = settings.clone();
    next.explorer = Some(crate::storage::normalized(target)?);
    next.validate(data)?;
    let mut staging = crate::storage::Staging::new(&old, target)?;
    let mut copied = 0;
    if old.exists() {
        plain(&old)?;
        let index = old.join(".mochi-view.json");
        if index.exists() {
            plain(&index)?;
            let manifest: Manifest = serde_json::from_slice(&fs::read(index)?)?;
            if manifest.version != 1 {
                return Err("文件箱格式较新，不能迁移".into());
            }
        } else if fs::read_dir(&old)?.next().is_some() {
            return Err("原文件箱不属于 Mochi，未修改其内容".into());
        }
        for entry in fs::read_dir(&old)? {
            let entry = entry?;
            crate::storage::copy_tree(
                &entry.path(),
                &staging.path.join(entry.file_name()),
                cancel,
                &mut copied,
                progress,
            )?;
        }
    }
    if cancel.load(Ordering::Acquire) {
        return Err("目录切换已取消，继续使用原目录".into());
    }
    next.explorer = Some(staging.publish()?);
    let new = next.explorer(data);
    let journal = data.join("storage-moving.json");
    crate::config::atomic_json(
        &journal,
        &StorageJournal {
            version: 1,
            old: crate::storage::normalized(&old)?,
            new: new.clone(),
            enabled,
        },
    )?;
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        #[cfg(windows)]
        {
            if enabled {
                registration::install(&new)?;
            }
            registration::uninstall(&old)?;
        }
        #[cfg(not(windows))]
        let _ = (enabled, &new);
        next.save(data)?;
        Ok(())
    })();
    if let Err(error) = result {
        let _ = recover_storage(data, settings);
        return Err(error);
    }
    let _ = fs::remove_file(journal);
    Ok(next)
}

#[cfg(windows)]
pub mod registration {
    use super::*;
    use std::os::windows::process::CommandExt;
    fn notify() {
        use windows::Win32::UI::Shell::{SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify};
        unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
    }
    fn shell_path(root: &Path) -> io::Result<PathBuf> {
        let root = root.canonicalize().or_else(|_| std::path::absolute(root))?;
        let value = root.to_string_lossy();
        Ok(if let Some(unc) = value.strip_prefix(r"\\?\UNC\") {
            PathBuf::from(format!(r"\\{unc}"))
        } else if let Some(local) = value.strip_prefix(r"\\?\") {
            PathBuf::from(local)
        } else {
            root
        })
    }
    pub fn class_id(root: &Path) -> String {
        let root = shell_path(root).unwrap_or_else(|_| root.to_owned());
        let hash = format!(
            "{:x}",
            Sha256::digest(root.to_string_lossy().to_lowercase().as_bytes())
        );
        format!(
            "{{{}-{}-{}-{}-{}}}",
            &hash[..8],
            &hash[8..12],
            &hash[12..16],
            &hash[16..20],
            &hash[20..32]
        )
    }
    fn reg(args: &[&str]) -> io::Result<()> {
        let output = std::process::Command::new("reg.exe")
            .args(args)
            .creation_flags(0x08000000)
            .output()?;
        if output.status.success() {
            Ok(())
        } else {
            Err(io::Error::other(
                String::from_utf8_lossy(&output.stderr).into_owned(),
            ))
        }
    }
    pub fn install(root: &Path) -> io::Result<()> {
        let root = shell_path(root)?;
        let id = class_id(&root);
        let class = format!("HKCU\\Software\\Classes\\CLSID\\{id}");
        let entries = [
            ("", "", "REG_SZ", "Mochi 文件箱".to_owned()),
            (
                "",
                "System.IsPinnedToNameSpaceTree",
                "REG_DWORD",
                "1".into(),
            ),
            (
                "DefaultIcon",
                "",
                "REG_EXPAND_SZ",
                "%SystemRoot%\\system32\\imageres.dll,-3".into(),
            ),
            (
                "InProcServer32",
                "",
                "REG_EXPAND_SZ",
                "%SystemRoot%\\system32\\shell32.dll".into(),
            ),
            (
                "Instance",
                "CLSID",
                "REG_SZ",
                "{0E5AAE11-A475-4c5b-AB00-C66DE400274E}".into(),
            ),
            (
                "Instance\\InitPropertyBag",
                "Attributes",
                "REG_DWORD",
                "17".into(),
            ),
            (
                "Instance\\InitPropertyBag",
                "TargetFolderPath",
                "REG_SZ",
                root.to_string_lossy().into_owned(),
            ),
            ("ShellFolder", "FolderValueFlags", "REG_DWORD", "40".into()),
            (
                "ShellFolder",
                "Attributes",
                "REG_DWORD",
                "0xF080004D".into(),
            ),
        ];
        for (suffix, value, kind, data) in entries {
            let path = if suffix.is_empty() {
                class.clone()
            } else {
                format!("{class}\\{suffix}")
            };
            let option = if value.is_empty() { "/ve" } else { "/v" };
            let mut args = vec!["add", &path, option];
            if !value.is_empty() {
                args.push(value);
            }
            args.extend(["/t", kind, "/d", &data, "/f"]);
            reg(&args)?;
        }
        let namespace = format!(
            "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\MyComputer\\NameSpace\\{id}"
        );
        reg(&[
            "add",
            &namespace,
            "/ve",
            "/t",
            "REG_SZ",
            "/d",
            "Mochi 文件箱",
            "/f",
        ])?;
        notify();
        Ok(())
    }
    pub fn uninstall(root: &Path) -> io::Result<()> {
        let root = shell_path(root)?;
        let id = class_id(&root);
        for path in [
            format!(
                "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\MyComputer\\NameSpace\\{id}"
            ),
            format!("HKCU\\Software\\Classes\\CLSID\\{id}"),
        ] {
            let output = std::process::Command::new("reg.exe")
                .args(["query", &path])
                .creation_flags(0x08000000)
                .output()?;
            if output.status.success() {
                reg(&["delete", &path, "/f"])?;
            }
        }
        notify();
        Ok(())
    }
}
