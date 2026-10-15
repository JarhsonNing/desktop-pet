use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
    process::{Child, Command},
};

pub const VERSION: &str = "1.2.0";

pub fn runtime_directory() -> Result<PathBuf, String> {
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let parent = executable.parent().ok_or("应用目录不可用")?;
    #[cfg(target_os = "macos")]
    if parent.file_name().is_some_and(|name| name == "MacOS") {
        return Ok(parent
            .parent()
            .ok_or("应用资源目录不可用")?
            .join("Resources/runtime/pi"));
    }
    Ok(parent.join("runtime/pi"))
}

pub fn verify(directory: &Path) -> Result<PathBuf, String> {
    let invalid = "包内 Pi 缺失或损坏，请重新安装麻薯（开发构建请运行 scripts/prepare_pi.py）";
    let metadata: Value = serde_json::from_slice(
        &fs::read(directory.join("mochi-runtime.json")).map_err(|_| invalid)?,
    )
    .map_err(|_| invalid)?;
    if metadata["version"] != VERSION {
        return Err("包内 Pi 版本不匹配，需要 1.2.0".into());
    }
    let files = metadata["files"].as_object().ok_or(invalid)?;
    let executable = if cfg!(windows) { "pi.exe" } else { "pi" };
    if !files.contains_key(executable)
        || !files.contains_key("package.json")
        || !files.contains_key("LICENSE")
    {
        return Err(invalid.into());
    }
    for (name, expected) in files {
        if Path::new(name)
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
        {
            return Err(invalid.into());
        }
        let mut file = fs::File::open(directory.join(name)).map_err(|_| invalid)?;
        let mut digest = Sha256::new();
        let mut buffer = [0; 65536];
        loop {
            let n = file.read(&mut buffer).map_err(|_| invalid)?;
            if n == 0 {
                break;
            }
            digest.update(&buffer[..n]);
        }
        if format!("{:x}", digest.finalize()) != expected.as_str().unwrap_or("") {
            return Err(invalid.into());
        }
    }
    Ok(directory.join(executable))
}

pub fn hide(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
}

pub struct Tree {
    #[cfg(windows)]
    job: windows_sys::Win32::Foundation::HANDLE,
    #[cfg(unix)]
    pid: i32,
}
// A job handle is owned exclusively by the Agent worker.
#[cfg(windows)]
unsafe impl Send for Tree {}

impl Tree {
    pub fn attach(child: &Child) -> Result<Self, String> {
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::{Foundation::CloseHandle, System::JobObjects::*};
            unsafe {
                let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if job.is_null() {
                    return Err("无法创建 Pi 进程组".into());
                }
                let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                if SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as *const _,
                    std::mem::size_of_val(&info) as u32,
                ) == 0
                    || AssignProcessToJobObject(job, child.as_raw_handle()) == 0
                {
                    CloseHandle(job);
                    return Err("无法隔离 Pi 子进程".into());
                }
                Ok(Self { job })
            }
        }
        #[cfg(unix)]
        {
            Ok(Self {
                pid: child.id() as i32,
            })
        }
        #[cfg(not(any(windows, unix)))]
        {
            let _ = child;
            Ok(Self {})
        }
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        #[cfg(windows)]
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.job);
        }
        #[cfg(unix)]
        unsafe {
            libc::kill(-self.pid, libc::SIGKILL);
        }
    }
}
