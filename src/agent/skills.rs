//! Local Skills are copied into Mochi's directory; only enabled files reach Pi.
use super::configuration::Paths;
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
};

const MAX_FILES: usize = 4096;
const MAX_FILE: u64 = 32 * 1024 * 1024;
const MAX_TOTAL: u64 = 128 * 1024 * 1024;

#[derive(Serialize)]
pub struct Skill {
    pub id: String,
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub path: PathBuf,
    pub error: Option<String>,
}

fn regular(metadata: &fs::Metadata) -> bool {
    if metadata.is_symlink() {
        return false;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return false;
        }
    }
    true
}

fn metadata(file: &Path, fallback: Option<&str>) -> Result<(String, String), String> {
    let info = fs::symlink_metadata(file).map_err(|_| "无法读取 SKILL.md")?;
    if !regular(&info) || !info.is_file() || info.len() > 1024 * 1024 {
        return Err("SKILL.md 必须是小于 1 MiB 的普通文件".into());
    }
    let text = fs::read_to_string(file).map_err(|_| "SKILL.md 必须为 UTF-8 文本")?;
    let mut lines = text.trim_start_matches('\u{feff}').lines();
    if lines.next().map(str::trim) != Some("---") {
        return Err("SKILL.md 需要 YAML frontmatter".into());
    }
    let mut header = String::new();
    let mut closed = false;
    for line in lines {
        if line.trim() == "---" {
            closed = true;
            break;
        }
        header.push_str(line);
        header.push('\n');
    }
    if !closed {
        return Err("SKILL.md frontmatter 未闭合".into());
    }
    let value: Value = serde_yaml_ng::from_str(&header).map_err(|_| "SKILL.md frontmatter 无效")?;
    let directory_name = file
        .parent()
        .and_then(Path::file_name)
        .and_then(|s| s.to_str())
        .unwrap_or("");
    let name = value["name"]
        .as_str()
        .unwrap_or(fallback.unwrap_or(directory_name))
        .trim();
    if name.is_empty()
        || name.len() > 64
        || name.starts_with('-')
        || name.ends_with('-')
        || name.contains("--")
        || !name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return Err("Skill 名称需为 1–64 个小写字母、数字或连字符".into());
    }
    let description = value["description"].as_str().unwrap_or("").trim();
    if description.is_empty() || description.chars().count() > 1024 {
        return Err("Skill description 需为 1–1024 字文本".into());
    }
    Ok((name.into(), description.into()))
}

pub fn list(paths: &Paths) -> Result<Vec<Skill>, String> {
    if !paths.skills().exists() {
        return Ok(vec![]);
    }
    let disabled = paths.read("skills.json")?["disabled"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let root = paths
        .skills()
        .canonicalize()
        .map_err(|_| "无法读取 Skills 目录")?;
    let mut queue = vec![root.clone()];
    let mut files = vec![];
    let mut count = 0;
    while let Some(directory) = queue.pop() {
        for entry in fs::read_dir(&directory).map_err(|_| "无法读取 Skills 目录")? {
            let entry = entry.map_err(|_| "无法读取 Skill")?;
            count += 1;
            if count > MAX_FILES {
                return Err("Skills 目录文件过多".into());
            }
            let info = fs::symlink_metadata(entry.path()).map_err(|_| "无法读取 Skill")?;
            if !regular(&info) {
                continue;
            }
            if info.is_dir()
                && !entry.file_name().to_string_lossy().starts_with('.')
                && entry.file_name() != "node_modules"
            {
                let skill_file = entry.path().join("SKILL.md");
                if skill_file.is_file() {
                    files.push((entry.path(), skill_file));
                } else {
                    queue.push(entry.path());
                }
            } else if directory == root
                && info.is_file()
                && entry.path().extension().is_some_and(|e| e == "md")
            {
                files.push((entry.path(), entry.path()));
            }
        }
    }
    files.sort();
    let mut names = HashSet::new();
    Ok(files
        .into_iter()
        .map(|(location, file)| {
            let id = location
                .strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            let (name, description, mut error) = match metadata(&file, None) {
                Ok((name, description)) => (name, description, None),
                Err(error) => (
                    location
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                    String::new(),
                    Some(error),
                ),
            };
            if !names.insert(name.clone()) {
                error = Some("Skill 名称重复，请卸载重复项".into());
            }
            Skill {
                enabled: !disabled.contains(&json!(id)) && error.is_none(),
                id,
                name,
                description,
                path: file,
                error,
            }
        })
        .collect())
}

pub fn enabled(paths: &Paths) -> Result<Vec<PathBuf>, String> {
    Ok(list(paths)?
        .into_iter()
        .filter(|s| s.enabled)
        .map(|s| s.path)
        .collect())
}

pub fn toggle(paths: &Paths, id: &str, enabled: bool) -> Result<(), String> {
    let skill = list(paths)?
        .into_iter()
        .find(|s| s.id == id)
        .ok_or("Skill 不存在")?;
    if enabled && let Some(error) = skill.error {
        return Err(error);
    }
    let mut value = paths.read("skills.json")?;
    let mut disabled = value["disabled"].as_array().cloned().unwrap_or_default();
    disabled.retain(|item| item != id);
    if !enabled {
        disabled.push(json!(id));
    }
    value["disabled"] = json!(disabled);
    paths.save("skills.json", &value)
}

pub fn uninstall(paths: &Paths, id: &str) -> Result<(), String> {
    list(paths)?
        .into_iter()
        .find(|s| s.id == id)
        .ok_or("Skill 不存在")?;
    let root = paths
        .skills()
        .canonicalize()
        .map_err(|_| "Skills 目录不存在")?;
    let target = root.join(id).canonicalize().map_err(|_| "Skill 不存在")?;
    if target == root || !target.starts_with(&root) {
        return Err("Skill 路径越界".into());
    }
    if target.is_dir() {
        fs::remove_dir_all(target)
    } else {
        fs::remove_file(target)
    }
    .map_err(|_| "无法卸载 Skill，请关闭占用文件的程序")?;
    let mut value = paths.read("skills.json")?;
    if let Some(disabled) = value["disabled"].as_array_mut() {
        disabled.retain(|item| item != id);
        paths.save("skills.json", &value)?;
    }
    Ok(())
}

fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.len() > 512
        || name.contains(['\\', ':', '\0'])
        || name.chars().any(char::is_control)
        || Path::new(name)
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
        || name
            .split('/')
            .any(|c| c.is_empty() || c.ends_with(['.', ' ']))
    {
        return Err("Skill 包含不安全的文件路径".into());
    }
    for part in name.split('/') {
        let base = part.split('.').next().unwrap_or("").to_ascii_uppercase();
        if matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$")
            || base.len() == 4
                && (base.starts_with("COM") || base.starts_with("LPT"))
                && base.as_bytes()[3].is_ascii_digit()
        {
            return Err("Skill 文件名包含保留名称".into());
        }
    }
    Ok(())
}

pub fn install(paths: &Paths, source: &Path) -> Result<String, String> {
    paths.ensure()?;
    let source = source.canonicalize().map_err(|_| "安装来源不存在")?;
    let info = fs::symlink_metadata(&source).map_err(|_| "安装来源不可读")?;
    let mut files = BTreeMap::<String, (Vec<u8>, u32)>::new();
    let mut total = 0;
    let mut add = |name: String, bytes: Vec<u8>, mode: u32| -> Result<(), String> {
        validate_name(&name)?;
        total += bytes.len() as u64;
        if bytes.len() as u64 > MAX_FILE || total > MAX_TOTAL || files.len() >= MAX_FILES {
            return Err("Skill 超过大小限制（单文件 32 MiB，总计 128 MiB，4096 个文件）".into());
        }
        if files.keys().any(|key| key.eq_ignore_ascii_case(&name)) {
            return Err("Skill 存在重复文件路径".into());
        }
        files.insert(name, (bytes, mode));
        Ok(())
    };
    if info.is_dir() {
        let mut queue = vec![source.clone()];
        let mut count = 0;
        while let Some(directory) = queue.pop() {
            for entry in fs::read_dir(directory).map_err(|_| "无法读取 Skill 文件夹")? {
                let entry = entry.map_err(|_| "无法读取 Skill 文件")?;
                count += 1;
                if count > MAX_FILES {
                    return Err("Skill 文件过多".into());
                }
                if entry.file_name() == ".git" || entry.file_name() == "node_modules" {
                    continue;
                }
                let info = fs::symlink_metadata(entry.path()).map_err(|_| "Skill 文件不可读")?;
                if !regular(&info) {
                    return Err("Skill 不允许符号链接或目录联接".into());
                }
                if info.is_dir() {
                    queue.push(entry.path());
                } else if info.is_file() {
                    let name = entry
                        .path()
                        .strip_prefix(&source)
                        .map_err(|_| "Skill 路径越界")?
                        .to_string_lossy()
                        .replace('\\', "/");
                    let mut bytes = vec![];
                    fs::File::open(entry.path())
                        .map_err(|_| "Skill 文件不可读")?
                        .take(MAX_FILE + 1)
                        .read_to_end(&mut bytes)
                        .map_err(|_| "Skill 文件不可读")?;
                    #[cfg(unix)]
                    let mode = {
                        use std::os::unix::fs::PermissionsExt;
                        info.permissions().mode()
                    };
                    #[cfg(not(unix))]
                    let mode = 0o644;
                    add(name, bytes, mode)?;
                } else {
                    return Err("Skill 只支持普通文件".into());
                }
            }
        }
    } else {
        if source
            .extension()
            .is_none_or(|e| !e.eq_ignore_ascii_case("zip"))
            || info.len() > MAX_TOTAL
        {
            return Err("请选择 Skill 文件夹或小于 128 MiB 的 ZIP".into());
        }
        let mut archive =
            zip::ZipArchive::new(fs::File::open(&source).map_err(|_| "无法读取 ZIP")?)
                .map_err(|_| "ZIP 无效")?;
        if archive.len() > MAX_FILES {
            return Err("Skill ZIP 文件过多".into());
        }
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index).map_err(|_| "ZIP 条目无效")?;
            let name = entry.name().map_err(|_| "ZIP 文件名无效")?.into_owned();
            validate_name(name.trim_end_matches('/'))?;
            if entry.is_symlink() {
                return Err("Skill ZIP 不允许符号链接".into());
            }
            if entry.is_dir() {
                continue;
            }
            if entry.size() > MAX_FILE {
                return Err("Skill 文件超过 32 MiB".into());
            }
            let mut bytes = vec![];
            (&mut entry)
                .take(MAX_FILE + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| "ZIP 解压失败")?;
            if bytes.len() as u64 != entry.size() {
                return Err("ZIP 文件长度无效".into());
            }
            add(name, bytes, entry.unix_mode().unwrap_or(0o644))?;
        }
    }
    let roots: Vec<_> = files
        .keys()
        .filter(|name| Path::new(name).file_name().is_some_and(|s| s == "SKILL.md"))
        .map(|name| Path::new(name).parent().unwrap().to_path_buf())
        .collect();
    // Nested SKILL.md files belong to the enclosing Skill (for example subskills/).
    let roots: Vec<_> = roots
        .iter()
        .filter(|root| {
            !roots
                .iter()
                .any(|other| other != *root && root.starts_with(other))
        })
        .collect();
    if roots.is_empty() {
        return Err("未找到 SKILL.md，请选择 Skill 文件夹或 ZIP".into());
    }
    if roots.len() != 1 {
        return Err(
            "发现多个并列的 Skill，请每次选择一个主 Skill 文件夹或 ZIP（可包含子 Skill）".into(),
        );
    }
    let prefix = &roots[0];
    let stage = paths.root.join(format!(
        ".skill-install-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| "系统时钟不可用")?
            .as_nanos()
    ));
    fs::create_dir(&stage).map_err(|_| "无法创建 Skill 安装目录")?;
    let result = (|| {
        for (name, (bytes, mode)) in files {
            let Ok(relative) = Path::new(&name).strip_prefix(prefix) else {
                continue;
            };
            let file = stage.join(relative);
            fs::create_dir_all(file.parent().ok_or("Skill 路径无效")?)
                .map_err(|_| "无法创建 Skill 目录")?;
            fs::write(&file, bytes).map_err(|_| "无法写入 Skill 文件")?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&file, fs::Permissions::from_mode(0o600 | (mode & 0o111)))
                    .map_err(|_| "无法设置 Skill 文件权限")?;
            }
            #[cfg(not(unix))]
            let _ = mode;
        }
        let fallback = prefix
            .file_name()
            .or_else(|| {
                if info.is_dir() {
                    source.file_name()
                } else {
                    source.file_stem()
                }
            })
            .and_then(|s| s.to_str());
        let (name, _) = metadata(&stage.join("SKILL.md"), fallback)?;
        if list(paths)?.iter().any(|s| s.name == name) || paths.skills().join(&name).exists() {
            return Err("已安装同名 Skill，请先卸载旧版本".into());
        }
        fs::rename(&stage, paths.skills().join(&name)).map_err(|_| "无法完成 Skill 安装")?;
        Ok(name)
    })();
    if stage.exists() {
        let _ = fs::remove_dir_all(stage);
    }
    result
}
