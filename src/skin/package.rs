use super::{Skin, invalid};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{self, Cursor, Read, Write},
    path::Path,
    sync::Arc,
};

const MAGIC: &[u8; 8] = b"MOCHISK\0";
const VERSION: u32 = 1;
const MAX_PACKAGE: u64 = 16 * 1024 * 1024;
const MAX_TOTAL: u64 = 32 * 1024 * 1024;
const MAX_FILE: u64 = 8 * 1024 * 1024;
const MAX_FILES: usize = 256;

pub fn validate_path(name: &str) -> io::Result<()> {
    if name.is_empty()
        || name.len() > 240
        || name.contains(['\\', ':', '\0'])
        || name.chars().any(char::is_control)
        || name
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | "..") || part.ends_with(['.', ' ']))
    {
        return Err(invalid(format!("Invalid skin asset path: {name}")));
    }
    for part in name.split('/') {
        let base = part.split('.').next().unwrap_or("").to_ascii_uppercase();
        if matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$")
            || base.len() == 4
                && (base.starts_with("COM") || base.starts_with("LPT"))
                && base.as_bytes()[3].is_ascii_digit()
        {
            return Err(invalid("Skin paths cannot use reserved device names"));
        }
    }
    Ok(())
}

pub fn load(path: &Path) -> io::Result<Arc<Skin>> {
    Skin::from_files(source_files(path)?).map(Arc::new)
}

fn source_files(path: &Path) -> io::Result<BTreeMap<String, Vec<u8>>> {
    let files = if path.is_dir() {
        directory_files(path)?
    } else {
        let file = fs::File::open(path)?;
        if file.metadata()?.len() > MAX_PACKAGE {
            return Err(invalid("Skin package exceeds 16 MiB"));
        }
        let mut bytes = Vec::new();
        file.take(MAX_PACKAGE + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_PACKAGE {
            return Err(invalid("Skin package exceeds 16 MiB"));
        }
        archive_files(&bytes)?
    };
    let file_names: BTreeSet<String> = files.keys().map(|name| name.to_lowercase()).collect();
    for name in files.keys() {
        let mut parent = Path::new(name).parent();
        while let Some(path) = parent {
            if file_names.contains(&path.to_string_lossy().replace('\\', "/").to_lowercase()) {
                return Err(invalid("A skin asset cannot also be a directory"));
            }
            parent = path.parent();
        }
    }
    Ok(files)
}

fn archive_files(bytes: &[u8]) -> io::Result<BTreeMap<String, Vec<u8>>> {
    let payload = if bytes.starts_with(MAGIC) {
        if bytes.len() < 12 || u32::from_le_bytes(bytes[8..12].try_into().unwrap()) != VERSION {
            return Err(invalid("Unsupported .mochi package version"));
        }
        &bytes[12..]
    } else if bytes.starts_with(b"PK") {
        bytes
    } else {
        return Err(invalid("Expected a .mochi skin package or ZIP archive"));
    };
    let mut archive =
        zip::ZipArchive::new(Cursor::new(payload)).map_err(|error| invalid(error.to_string()))?;
    if archive.len() > MAX_FILES {
        return Err(invalid("Skin package has too many entries"));
    }
    let mut files = BTreeMap::new();
    let mut names = BTreeSet::new();
    let mut total = 0_u64;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| invalid(error.to_string()))?;
        let name = entry
            .name()
            .map_err(|error| invalid(error.to_string()))?
            .into_owned();
        validate_path(name.trim_end_matches('/'))?;
        if entry.is_symlink() {
            return Err(invalid("Skin packages cannot contain symlinks"));
        }
        if entry.is_dir() {
            continue;
        }
        if !names.insert(name.to_lowercase()) {
            return Err(invalid("Duplicate skin asset path"));
        }
        if entry.size() > MAX_FILE {
            return Err(invalid("Skin asset exceeds 8 MiB"));
        }
        total += entry.size();
        if total > MAX_TOTAL {
            return Err(invalid("Skin package expands beyond 32 MiB"));
        }
        let mut data = Vec::new();
        (&mut entry).take(MAX_FILE + 1).read_to_end(&mut data)?;
        if data.len() as u64 > MAX_FILE || data.len() as u64 != entry.size() {
            return Err(invalid("Invalid skin asset length"));
        }
        files.insert(name, data);
    }
    Ok(files)
}

fn directory_files(root: &Path) -> io::Result<BTreeMap<String, Vec<u8>>> {
    let root = root.canonicalize()?;
    let mut pending = vec![root.clone()];
    let mut files = BTreeMap::new();
    let mut names = BTreeSet::new();
    let mut total = 0_u64;
    let mut entries = 0;
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            entries += 1;
            if entries > MAX_FILES {
                return Err(invalid("Skin directory has too many entries"));
            }
            let path = entry.path();
            let name = path
                .strip_prefix(&root)
                .map_err(|_| invalid("Asset is outside skin directory"))?
                .to_str()
                .ok_or_else(|| invalid("Asset name must be Unicode"))?
                .replace('\\', "/");
            validate_path(&name)?;
            let kind = entry.file_type()?;
            if kind.is_symlink() || !path.canonicalize()?.starts_with(&root) {
                return Err(invalid(
                    "Skin directories cannot contain symlinks or external assets",
                ));
            }
            if kind.is_dir() {
                pending.push(path);
                continue;
            }
            if !kind.is_file() {
                return Err(invalid("Skin assets must be regular files"));
            }
            if !names.insert(name.to_lowercase()) {
                return Err(invalid("Duplicate skin asset path"));
            }
            let mut data = Vec::new();
            fs::File::open(path)?
                .take(MAX_FILE + 1)
                .read_to_end(&mut data)?;
            total += data.len() as u64;
            if data.len() as u64 > MAX_FILE || total > MAX_TOTAL {
                return Err(invalid("Skin assets exceed size limits"));
            }
            files.insert(name, data);
        }
    }
    Ok(files)
}

pub fn pack(directory: &Path, output: &Path) -> io::Result<()> {
    if !directory.is_dir() {
        return Err(invalid("Packing requires a skin directory"));
    }
    let extension = output
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !matches!(extension.as_str(), "mochi" | "zip") {
        return Err(invalid("Output must end in .mochi or .zip"));
    }
    let files = source_files(directory)?;
    // Validate the exact files that will be shipped before creating an output file.
    Skin::from_files(files.clone())?;
    let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (name, data) in files {
        archive
            .start_file(name, options)
            .map_err(|error| invalid(error.to_string()))?;
        archive.write_all(&data)?;
    }
    let payload = archive
        .finish()
        .map_err(|error| invalid(error.to_string()))?
        .into_inner();
    if payload.len() as u64 + 12 > MAX_PACKAGE {
        return Err(invalid("Packed skin exceeds 16 MiB"));
    }
    if let Some(parent) = output.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)?;
    if extension == "mochi" {
        file.write_all(MAGIC)?;
        file.write_all(&VERSION.to_le_bytes())?;
    }
    file.write_all(&payload)?;
    file.sync_all()
}

pub fn unpack(package: &Path, output: &Path) -> io::Result<()> {
    if !package.is_file() {
        return Err(invalid("Unpacking requires a .mochi or ZIP file"));
    }
    let files = source_files(package)?;
    Skin::from_files(files.clone())?;
    if let Some(parent) = output.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    // A new destination ensures extraction cannot overwrite files or traverse existing symlinks.
    fs::create_dir(output)?;
    let root = output.canonicalize()?;
    for (name, data) in files {
        let path = root.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        file.write_all(&data)?;
    }
    Ok(())
}
